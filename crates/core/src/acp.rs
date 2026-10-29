//! Minimal Agent Client Protocol client: newline-delimited JSON-RPC 2.0 over
//! the agent process's stdio.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, oneshot};

use crate::env::tokio_command;

pub const PROTOCOL_VERSION: u64 = 1;

#[derive(Debug)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// Injected by the client (not the agent) to mark a position in the stream.
    Marker(&'static str),
    Barrier(oneshot::Sender<()>),
    Closed {
        stderr_tail: String,
    },
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>;

const CONTROL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const STARTUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

struct Transport {
    pending: Pending,
    alive: AtomicBool,
    error: Mutex<String>,
    stderr: Mutex<String>,
    incoming: mpsc::UnboundedSender<Incoming>,
    closed: tokio::sync::watch::Sender<bool>,
    #[cfg(unix)]
    pid: Option<u32>,
}

impl Transport {
    fn fail(&self, reason: impl Into<String>) {
        let mut pending = self.pending.lock();
        if !self.alive.swap(false, Ordering::SeqCst) {
            return;
        }
        let tail = self.stderr.lock().clone();
        let reason = reason.into();
        let reason = if tail.is_empty() {
            reason
        } else {
            format!("{reason}\nAgent stderr:\n{tail}")
        };
        *self.error.lock() = reason.clone();
        for (_, waiter) in pending.drain() {
            let _ = waiter.send(Err(anyhow!(reason.clone())));
        }
        self.closed.send_replace(true);
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            let _ = crate::env::std_command("kill")
                .args(["-KILL", &format!("-{pid}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.incoming.send(Incoming::Closed {
            stderr_tail: reason,
        });
    }

    fn error(&self) -> anyhow::Error {
        anyhow!(self.error.lock().clone())
    }
}

struct PendingRequest<'a> {
    id: u64,
    transport: &'a Transport,
}

impl Drop for PendingRequest<'_> {
    fn drop(&mut self) {
        self.transport.pending.lock().remove(&self.id);
    }
}

// A canceled write may have sent only part of a JSON frame. Never reuse that stream.
struct Writing<'a> {
    transport: &'a Transport,
    complete: bool,
}

impl Drop for Writing<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.transport
                .fail("ACP write interrupted; reconnect before sending another message");
        }
    }
}

pub struct AcpConnection {
    stdin: tokio::sync::Mutex<ChildStdin>,
    transport: Arc<Transport>,
    next_id: AtomicU64,
}

impl AcpConnection {
    pub fn spawn(
        program: &str,
        args: &[String],
        cwd: &Path,
    ) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Incoming>)> {
        let mut cmd = tokio_command(program);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = cmd.spawn().with_context(|| format!("failed to start `{program}`; check that it is installed and available on your login-shell PATH"))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let (tx, rx) = mpsc::unbounded_channel();
        let (closed, _) = tokio::sync::watch::channel(false);
        let transport = Arc::new(Transport {
            pending: Mutex::default(),
            alive: AtomicBool::new(true),
            error: Mutex::new("agent connection closed".into()),
            stderr: Mutex::default(),
            incoming: tx,
            closed,
            #[cfg(unix)]
            pid: child.id(),
        });
        let stderr_task = {
            let transport = transport.clone();
            let mut closed = transport.closed.subscribe();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                loop {
                    let result = tokio::select! {
                        biased;
                        _ = closed.changed() => break,
                        result = stderr.read(&mut buf) => result,
                    };
                    let Ok(n) = result else { break };
                    if n == 0 {
                        break;
                    }
                    let mut tail = transport.stderr.lock();
                    tail.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if tail.len() > 4000 {
                        let cut = (tail.len() - 4000..tail.len())
                            .find(|i| tail.is_char_boundary(*i))
                            .unwrap_or(0);
                        tail.drain(..cut);
                    }
                }
            })
        };
        {
            let transport = transport.clone();
            let mut closed = transport.closed.subscribe();
            tokio::spawn(async move {
                // Read bounded frames incrementally; a peer need not finish a line to exceed the limit.
                let mut stdout = BufReader::new(stdout);
                let mut frame = Vec::new();
                let reason = loop {
                    let chunk = tokio::select! {
                        biased;
                        _ = closed.changed() => return,
                        result = stdout.fill_buf() => match result {
                            Ok(chunk) => chunk,
                            Err(e) => break format!("ACP stdout read failed: {e}"),
                        },
                    };
                    if chunk.is_empty() {
                        break if frame.is_empty() {
                            "Agent closed ACP stdout".into()
                        } else {
                            "Agent closed ACP stdout mid-frame (partial JSON output)".into()
                        };
                    }
                    let n = chunk
                        .iter()
                        .position(|b| *b == b'\n')
                        .map(|i| i + 1)
                        .unwrap_or(chunk.len());
                    let complete = chunk[n - 1] == b'\n';
                    if frame.len() + n > MAX_FRAME_BYTES {
                        break format!("ACP frame exceeds {MAX_FRAME_BYTES} bytes; check the agent's stdout protocol");
                    }
                    frame.extend_from_slice(&chunk[..n]);
                    stdout.consume(n);
                    if !complete {
                        continue;
                    }
                    if !frame.iter().all(u8::is_ascii_whitespace) {
                        let msg = match serde_json::from_slice::<Value>(&frame) {
                            Ok(msg) => msg,
                            Err(e) => break format!("Invalid ACP JSON from agent stdout: {e}; stdout must contain only newline-delimited JSON-RPC"),
                        };
                        if let Err(e) = dispatch(msg, &transport.pending, &transport.incoming) {
                            break format!("Invalid ACP message: {e}");
                        }
                    }
                    frame.clear();
                };
                // Give stderr's final diagnostic a bounded opportunity to arrive.
                let _ =
                    tokio::time::timeout(std::time::Duration::from_millis(100), stderr_task).await;
                transport.fail(reason);
            });
        }
        {
            let transport = transport.clone();
            let mut closed = transport.closed.subscribe();
            tokio::spawn(async move {
                tokio::select! {
                    biased;
                    _ = closed.changed() => {},
                    result = child.wait() => {
                        // stdout may be inherited by a descendant, so EOF alone is not a process monitor.
                        tokio::task::yield_now().await;
                        let reason = match result {
                            Ok(status) => format!("Agent process exited ({status})"),
                            Err(e) => format!("Could not wait for agent process: {e}"),
                        };
                        transport.fail(reason);
                    }
                }
                // Always reap the direct child, including explicit kill and protocol failure.
                let _ = child.kill().await;
                let _ = child.wait().await;
            });
        }
        Ok((
            Arc::new(Self {
                stdin: tokio::sync::Mutex::new(stdin),
                transport,
                next_id: AtomicU64::new(1),
            }),
            rx,
        ))
    }

    pub fn is_alive(&self) -> bool {
        self.transport.alive.load(Ordering::SeqCst)
    }

    async fn write(&self, msg: Value) -> Result<()> {
        let mut line = serde_json::to_vec(&msg)?;
        line.push(b'\n');
        let result = tokio::time::timeout(WRITE_TIMEOUT, async {
            let mut stdin = self.stdin.lock().await;
            if !self.is_alive() {
                return Err(self.transport.error());
            }
            let mut writing = Writing {
                transport: &self.transport,
                complete: false,
            };
            stdin.write_all(&line).await.context("writing ACP frame")?;
            stdin.flush().await.context("flushing ACP frame")?;
            writing.complete = true;
            Ok(())
        })
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                self.transport.fail(format!("ACP write failed: {e:#}"));
                Err(e)
            }
            Err(_) => {
                self.transport
                    .fail("ACP write timed out after 10s; agent is not reading stdin");
                Err(self.transport.error())
            }
        }
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let timeout = match method {
            "session/prompt" => None,
            "initialize" | "session/new" | "session/load" | "session/resume" => {
                Some(STARTUP_TIMEOUT)
            }
            _ => Some(CONTROL_TIMEOUT),
        };
        self.request_with_timeout(method, params, timeout).await
    }

    /// Prompts have no duration limit. Custom bounds are useful for discovery and fault tests.
    pub async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Option<std::time::Duration>,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.transport.pending.lock();
            if !self.is_alive() {
                return Err(self.transport.error());
            }
            pending.insert(id, tx);
        }
        let _pending = PendingRequest {
            id,
            transport: &self.transport,
        };
        let request = async {
            self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
                .await?;
            rx.await.map_err(|_| self.transport.error())?
        };
        let result = match timeout {
            Some(timeout) => match tokio::time::timeout(timeout, request).await {
                Ok(result) => result,
                Err(_) => {
                    self.transport.fail(format!("ACP `{method}` timed out after {:.1}s; check the agent login and restart the session. The request was not replayed", timeout.as_secs_f64()));
                    Err(self.transport.error())
                }
            },
            None => request.await,
        };
        result.with_context(|| format!("ACP `{method}` failed"))
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    pub async fn respond(&self, id: Value, result: Value) -> Result<()> {
        self.write(json!({"jsonrpc": "2.0", "id": id, "result": result}))
            .await
    }

    pub async fn respond_error(&self, id: Value, code: i64, message: &str) -> Result<()> {
        self.write(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
            .await
    }

    pub fn mark(&self, marker: &'static str) {
        let _ = self.transport.incoming.send(Incoming::Marker(marker));
    }

    /// Wait until the consumer has applied all preceding notifications.
    pub async fn drain(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.transport
            .incoming
            .send(Incoming::Barrier(tx))
            .map_err(|_| anyhow!("ACP event consumer closed"))?;
        tokio::time::timeout(WRITE_TIMEOUT, rx)
            .await
            .context("ACP event drain timed out")?
            .context("ACP event consumer closed before drain")
    }

    pub fn kill(&self) {
        self.transport
            .fail("Agent connection stopped; no pending request was replayed");
    }
}

impl Drop for AcpConnection {
    fn drop(&mut self) {
        self.kill();
    }
}

fn safe_error_details(data: &Value) -> Value {
    fn summary(data: &Value, depth: u8) -> Value {
        let mut out = serde_json::Map::new();
        if depth == 0 {
            return Value::Object(out);
        }
        for key in ["code", "type", "name", "status", "statusCode", "retryable"] {
            if let Some(value) = data.get(key) {
                let safe = match value {
                    Value::Number(_) | Value::Bool(_) => true,
                    Value::String(s) => {
                        s.len() <= 80
                            && !s.is_empty()
                            && s.bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                            && !s.starts_with("sk-")
                            && !s.starts_with("sk_")
                            && !s.starts_with("eyJ")
                    }
                    _ => false,
                };
                out.insert(
                    key.into(),
                    if safe {
                        value.clone()
                    } else {
                        json!("[redacted]")
                    },
                );
            }
        }
        for key in ["error", "cause"] {
            if let Some(value) = data.get(key) {
                let nested = summary(value, depth - 1);
                if nested != json!({}) {
                    out.insert(key.into(), nested);
                }
            }
        }
        Value::Object(out)
    }
    // Provider bodies may echo keys, prompts or HTTP headers. Never print arbitrary data.
    summary(data, 4)
}

fn dispatch(msg: Value, pending: &Pending, tx: &mpsc::UnboundedSender<Incoming>) -> Result<()> {
    if msg.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(anyhow!("expected JSON-RPC 2.0 object"));
    }
    if msg.get("method").is_some_and(|method| !method.is_string()) {
        return Err(anyhow!("JSON-RPC method must be a string"));
    }
    let method = msg.get("method").and_then(Value::as_str);
    let id = msg.get("id").filter(|v| !v.is_null());
    if let Some(id) = id {
        if !id.is_string() && !id.is_number() {
            return Err(anyhow!("invalid JSON-RPC id"));
        }
    }
    match (method, id) {
        (Some(method), id) => {
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let incoming = match id {
                Some(id) => Incoming::Request {
                    id: id.clone(),
                    method: method.into(),
                    params,
                },
                None => Incoming::Notification {
                    method: method.into(),
                    params,
                },
            };
            let _ = tx.send(incoming);
        }
        (None, Some(id)) => {
            if msg.get("result").is_some() == msg.get("error").is_some() {
                return Err(anyhow!(
                    "response must contain exactly one of result or error"
                ));
            }
            let result = match msg.get("error") {
                Some(err) => {
                    let message = err
                        .get("message")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("invalid JSON-RPC error message"))?;
                    let code = err
                        .get("code")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| anyhow!("invalid JSON-RPC error code"))?;
                    let details = err
                        .get("data")
                        .map(safe_error_details)
                        .filter(|data| data != &json!({}));
                    Err(match details {
                        Some(details) => {
                            anyhow!("agent error {code}: {message}; details: {details}")
                        }
                        None => anyhow!("agent error {code}: {message}"),
                    })
                }
                None => Ok(msg["result"].clone()),
            };
            // Late responses to canceled requests are valid but have no waiter.
            if let Some(id) = id.as_u64() {
                if let Some(waiter) = pending.lock().remove(&id) {
                    let _ = waiter.send(result);
                }
            }
        }
        (None, None) => return Err(anyhow!("message has neither method nor response id")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spawn(script: &str) -> Arc<AcpConnection> {
        AcpConnection::spawn(
            "node",
            &["-e".into(), script.into()],
            Path::new(env!("CARGO_MANIFEST_DIR")),
        )
        .unwrap()
        .0
    }

    #[tokio::test]
    async fn dropping_request_future_removes_pending_entry() {
        let conn = spawn("process.stdin.resume(); setInterval(()=>{},1000)");
        let task = {
            let conn = conn.clone();
            tokio::spawn(async move { conn.request("session/prompt", json!({})).await })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while conn.transport.pending.lock().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
        assert!(conn.transport.pending.lock().is_empty());
        conn.kill();
    }

    #[tokio::test]
    async fn timeout_and_disconnect_remove_pending_entries() {
        let conn = spawn("process.stdin.resume(); setInterval(()=>{},1000)");
        assert!(conn
            .request_with_timeout("initialize", json!({}), Some(Duration::from_millis(100)))
            .await
            .is_err());
        assert!(conn.transport.pending.lock().is_empty());
        assert!(!conn.is_alive());
        assert!(conn.request("session/prompt", json!({})).await.is_err());
        assert!(conn.transport.pending.lock().is_empty());
    }

    #[tokio::test]
    async fn canceling_partial_write_closes_corrupted_stream() {
        let conn = spawn("process.stdin.pause(); setInterval(()=>{},1000)");
        let mut request = Box::pin(conn.request(
            "session/prompt",
            json!({"text": "x".repeat(8 * 1024 * 1024)}),
        ));
        tokio::select! {
            result = &mut request => panic!("write unexpectedly completed: {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(300)) => {},
        }
        drop(request);
        assert!(!conn.is_alive());
        assert!(conn.transport.pending.lock().is_empty());
    }
}
