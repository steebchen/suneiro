//! Agent sessions. Each session owns one ACP agent process running inside the
//! workspace's worktree. Agents authenticate with whatever the user already has
//! set up locally (Claude subscription login, Codex ChatGPT login, OpenCode auth).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::acp::{AcpConnection, Incoming, PROTOCOL_VERSION};
use crate::events::Event;
use crate::store::{now, Session, Store};
use crate::Emitter;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDef {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

pub fn builtin_agents() -> Vec<AgentDef> {
    let def = |id: &str, name: &str, command: &str, args: &[&str]| AgentDef {
        id: id.into(),
        name: name.into(),
        command: command.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
    };
    vec![
        def("claude", "Claude Code", "npx", &["-y", "@agentclientprotocol/claude-agent-acp@0.83.0"]),
        def("codex", "Codex", "npx", &["-y", "@agentclientprotocol/codex-acp@2.0.0"]),
        def("opencode", "OpenCode", "opencode", &["acp"]),
    ]
}


const REPLAY_DONE: &str = "replay-done";

struct LiveSession {
    id: String,
    workspace_id: String,
    agent: AgentDef,
    cwd: PathBuf,
    conn: tokio::sync::Mutex<Option<Arc<AcpConnection>>>,
    acp_session_id: Mutex<Option<String>>,
    /// Our request id -> JSON-RPC id of the agent's pending permission request.
    permissions: Mutex<HashMap<String, Value>>,
    /// Our request id -> JSON-RPC id of a pending question (elicitation).
    questions: Mutex<HashMap<String, Value>>,
    running: AtomicBool,
    replaying: AtomicBool,
    titled: AtomicBool,
    /// Plan mode: the agent plans and asks before acting. Otherwise every
    /// permission request is approved automatically.
    plan: AtomicBool,
    /// While we're changing the agent's mode ourselves, don't treat mode
    /// updates as the agent leaving plan mode.
    applying_mode: AtomicBool,
    config: Mutex<Value>,
    /// The agent accepts `_session/steering` (messages injected mid-turn).
    steering: AtomicBool,
    /// The agent accepts image content in prompts.
    images: AtomicBool,
    /// Latest cumulative cost the agent process reported (Claude and OpenCode
    /// report a running total per process), and this turn's share of it.
    cost_reported: Mutex<Option<f64>>,
    turn_cost: Mutex<Option<f64>>,
    /// Model and effort to apply once a brand-new ACP session exists.
    initial: Mutex<Option<(Option<String>, Option<String>)>>,
    /// Context for the agent that goes along with the next prompt (e.g. that
    /// the user restored the files to an earlier checkpoint).
    notes: Mutex<Vec<String>>,
}

pub struct Agents {
    store: Arc<Store>,
    emitter: Emitter,
    sessions: Mutex<HashMap<String, Arc<LiveSession>>>,
    defs: Mutex<Vec<AgentDef>>,
    file_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Agents {
    pub fn new(store: Arc<Store>, emitter: Emitter) -> Self {
        Self { store, emitter, sessions: Mutex::default(), defs: Mutex::new(builtin_agents()), file_locks: Mutex::default() }
    }

    pub fn defs(&self) -> Vec<AgentDef> {
        self.defs.lock().clone()
    }

    /// Add or replace an agent definition.
    pub fn register(&self, def: AgentDef) {
        let mut defs = self.defs.lock();
        defs.retain(|d| d.id != def.id);
        defs.push(def);
    }

    pub fn def(&self, id: &str) -> Result<AgentDef> {
        self.defs
            .lock()
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown agent `{id}`"))
    }

    fn live(&self, session: &Session, cwd: PathBuf) -> Result<Arc<LiveSession>> {
        if let Some(s) = self.sessions.lock().get(&session.id) {
            return Ok(s.clone());
        }
        let live = Arc::new(LiveSession {
            id: session.id.clone(),
            workspace_id: session.workspace_id.clone(),
            agent: self.def(&session.agent_id)?,
            cwd,
            conn: tokio::sync::Mutex::new(None),
            acp_session_id: Mutex::new(session.acp_session_id.clone()),
            permissions: Mutex::default(),
            questions: Mutex::default(),
            running: AtomicBool::new(false),
            replaying: AtomicBool::new(false),
            titled: AtomicBool::new(!session.title.is_empty()),
            plan: AtomicBool::new(false),
            applying_mode: AtomicBool::new(false),
            config: Mutex::new(json!([])),
            steering: AtomicBool::new(false),
            images: AtomicBool::new(true),
            cost_reported: Mutex::new(None),
            turn_cost: Mutex::new(None),
            initial: Mutex::new(None),
            notes: Mutex::default(),
        });
        Ok(self.sessions.lock().entry(session.id.clone()).or_insert(live).clone())
    }

    fn get(&self, session_id: &str) -> Result<Arc<LiveSession>> {
        if let Some(s) = self.sessions.lock().get(session_id) {
            return Ok(s.clone());
        }
        let session = self.store.session(session_id)?;
        let ws = self.store.workspace(&session.workspace_id)?;
        self.live(&session, PathBuf::from(ws.path))
    }

    fn state(&self, s: &LiveSession, state: &str, error: Option<String>) {
        self.emitter.emit(Event::SessionState {
            session_id: s.id.clone(),
            state: state.into(),
            error,
        });
    }

    /// Start the agent in the background so config options (model, mode) are
    /// ready by the time the user types their first message.
    pub fn warm_up(self: &Arc<Self>, session_id: &str) {
        let this = self.clone();
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            let Ok(s) = this.get(&session_id) else { return };
            if let Err(e) = this.connect(&s).await {
                this.state(&s, "error", Some(format!("{e:#}")));
            }
        });
    }

    async fn connect(self: &Arc<Self>, s: &Arc<LiveSession>) -> Result<Arc<AcpConnection>> {
        let mut guard = s.conn.lock().await;
        if let Some(conn) = guard.as_ref().filter(|c| c.is_alive()) {
            return Ok(conn.clone());
        }
        if !s.running.load(Ordering::SeqCst) {
            self.state(s, "connecting", None);
        }
        // The worktree may still be checking out in the background.
        loop {
            match self.store.workspace(&s.workspace_id)?.status.as_str() {
                "creating" => tokio::time::sleep(std::time::Duration::from_millis(150)).await,
                "failed" => bail!("the workspace's worktree could not be created (see the Setup tab)"),
                _ => break,
            }
        }

        let (conn, rx) = AcpConnection::spawn(&s.agent.command, &s.agent.args, &s.cwd)?;
        // A new agent process starts its running cost total from scratch.
        *s.cost_reported.lock() = None;
        tokio::spawn(self.clone().handle_incoming(s.clone(), conn.clone(), rx));

        let init = conn
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "clientCapabilities": {
                        "fs": {"readTextFile": false, "writeTextFile": false},
                        "terminal": false,
                        // Lets agents ask the user structured questions
                        // (Claude's AskUserQuestion, Codex's request_user_input).
                        "elicitation": {"form": {}}
                    },
                    "clientInfo": {"name": "suneiro", "version": env!("CARGO_PKG_VERSION")}
                }),
            )
            .await?;
        let caps = init.get("agentCapabilities").cloned().unwrap_or(Value::Null);
        s.steering.store(
            init.pointer("/_meta/steering/supported").and_then(|v| v.as_bool()).unwrap_or(false),
            Ordering::SeqCst,
        );
        s.images.store(
            caps.pointer("/promptCapabilities/image").and_then(|v| v.as_bool()).unwrap_or(false),
            Ordering::SeqCst,
        );
        let cwd = s.cwd.to_string_lossy().to_string();

        let existing = s.acp_session_id.lock().clone();
        let mut response = None;
        if let Some(id) = &existing {
            let params = json!({"sessionId": id, "cwd": cwd, "mcpServers": []});
            if caps.pointer("/sessionCapabilities/resume").is_some() {
                response = conn.request("session/resume", params).await.ok();
            } else if caps.get("loadSession").and_then(|v| v.as_bool()) == Some(true) {
                // session/load replays history as updates; we already have it stored.
                s.replaying.store(true, Ordering::SeqCst);
                response = conn.request("session/load", params).await.ok();
                conn.mark(REPLAY_DONE);
            }
        }
        let is_new = response.is_none();
        let response = match response {
            Some(r) => r,
            None => {
                let r = conn.request("session/new", json!({"cwd": cwd, "mcpServers": []})).await?;
                let id = r
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("agent returned no sessionId"))?;
                *s.acp_session_id.lock() = Some(id.to_string());
                self.store.set_acp_session_id(&s.id, id)?;
                r
            }
        };
        if let Some(opts) = response.get("configOptions").filter(|v| v.is_array()) {
            self.set_config_options(s, opts.clone());
        }
        if is_new {
            if let Err(e) = self.apply_initial(s, &conn).await {
                log::warn!("could not apply initial model: {e:#}");
            }
        }
        if let Err(e) = self.apply_mode(s, &conn).await {
            log::warn!("could not apply permission mode: {e:#}");
        }
        if !s.running.load(Ordering::SeqCst) {
            self.state(s, "idle", None);
        }
        *guard = Some(conn.clone());
        Ok(conn)
    }

    async fn handle_incoming(
        self: Arc<Self>,
        s: Arc<LiveSession>,
        conn: Arc<AcpConnection>,
        mut rx: mpsc::UnboundedReceiver<Incoming>,
    ) {
        while let Some(msg) = rx.recv().await {
            match msg {
                Incoming::Marker(REPLAY_DONE) => s.replaying.store(false, Ordering::SeqCst),
                Incoming::Marker(_) => {}
                Incoming::Notification { method, params } if method == "session/update" => {
                    if s.replaying.load(Ordering::SeqCst) {
                        continue;
                    }
                    let Some(update) = params.get("update").cloned() else { continue };
                    self.on_update(&s, update);
                }
                Incoming::Notification { .. } => {}
                Incoming::Request { id, method, params } => match method.as_str() {
                    "session/request_permission" if !s.plan.load(Ordering::SeqCst) => {
                        let options = params.get("options").cloned().unwrap_or(json!([]));
                        let outcome = match auto_allow_option(&options) {
                            Some(option_id) => json!({"outcome": "selected", "optionId": option_id}),
                            None => json!({"outcome": "cancelled"}),
                        };
                        let _ = conn.respond(id, json!({"outcome": outcome})).await;
                    }
                    "session/request_permission" => {
                        let request_id = uuid::Uuid::new_v4().to_string();
                        s.permissions.lock().insert(request_id.clone(), id);
                        self.emitter.emit(Event::PermissionRequest {
                            session_id: s.id.clone(),
                            request_id,
                            tool_call: params.get("toolCall").cloned().unwrap_or(Value::Null),
                            options: params.get("options").cloned().unwrap_or(json!([])),
                        });
                    }
                    "elicitation/create" if params["mode"] == "form" || params["mode"].is_null() => {
                        let request_id = uuid::Uuid::new_v4().to_string();
                        s.questions.lock().insert(request_id.clone(), id);
                        self.emitter.emit(Event::Question {
                            session_id: s.id.clone(),
                            request_id,
                            message: params["message"].as_str().unwrap_or("").to_string(),
                            schema: params.get("requestedSchema").cloned().unwrap_or(json!({})),
                            tool_call_id: params["toolCallId"].as_str().map(str::to_string),
                            auto_resolve_ms: params.pointer("/_meta/codex/autoResolutionMs").and_then(|v| v.as_u64()),
                        });
                    }
                    "elicitation/create" => {
                        // URL mode (e.g. MCP OAuth) isn't supported yet.
                        let _ = conn.respond(id, json!({"action": "decline"})).await;
                    }
                    _ => {
                        let _ = conn.respond_error(id, -32601, "method not supported").await;
                    }
                },
                Incoming::Closed { stderr_tail } => {
                    let pending: Vec<String> = s.permissions.lock().drain().map(|(k, _)| k).collect();
                    for request_id in pending {
                        self.emitter.emit(Event::PermissionResolved {
                            session_id: s.id.clone(),
                            request_id,
                        });
                    }
                    let questions: Vec<String> = s.questions.lock().drain().map(|(k, _)| k).collect();
                    for request_id in questions {
                        self.emitter.emit(Event::QuestionResolved { session_id: s.id.clone(), request_id });
                    }
                    if s.running.load(Ordering::SeqCst) {
                        let tail = stderr_tail.lines().rev().take(8).collect::<Vec<_>>();
                        let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
                        self.state(&s, "error", Some(format!("Agent exited unexpectedly.\n{tail}")));
                    }
                    break;
                }
            }
        }
    }

    fn on_update(self: &Arc<Self>, s: &Arc<LiveSession>, update: Value) {
        let kind = update.get("sessionUpdate").and_then(|k| k.as_str()).unwrap_or("");
        match kind {
            "config_option_update" => {
                if let Some(opts) = update.get("configOptions") {
                    self.set_config_options(s, opts.clone());
                    self.check_plan_exit(s);
                }
            }
            "usage_update" => {
                if let Some(total) = update.pointer("/cost/amount").and_then(|v| v.as_f64()) {
                    if update.pointer("/cost/currency").and_then(|c| c.as_str()).unwrap_or("USD") == "USD" {
                        let mut last = s.cost_reported.lock();
                        // Per-process running total: the increase is this turn's
                        // cost; a smaller value means the total was reset.
                        let delta = match *last {
                            Some(prev) if total + 1e-9 >= prev => total - prev,
                            Some(_) => total,
                            None => 0.0_f64.max(total - self.baseline_after_resume(s, total)),
                        };
                        *last = Some(total);
                        let mut turn = s.turn_cost.lock();
                        *turn = Some(turn.unwrap_or(0.0) + delta);
                    }
                }
            }
            "current_mode_update" => {
                if let Some(mode) = update.get("currentModeId").and_then(|m| m.as_str()) {
                    let mut config = s.config.lock().clone();
                    if let Some(opts) = config.as_array_mut() {
                        for o in opts.iter_mut().filter(|o| o["id"] == "mode") {
                            o["currentValue"] = json!(mode);
                        }
                    }
                    self.set_config_options(s, config);
                    self.check_plan_exit(s);
                }
            }
            "session_info_update" => {
                // Agents often echo the raw prompt here; only use it if we have nothing better.
                if let Some(title) = update.get("title").and_then(|t| t.as_str()) {
                    if !s.titled.load(Ordering::SeqCst) {
                        self.set_title(s, title);
                    }
                }
            }
            _ => {}
        }
        self.emitter.emit(Event::SessionUpdate { session_id: s.id.clone(), update });
    }

    /// First cost report of a freshly connected process. A brand-new session
    /// starts at zero; a resumed one may report its earlier total again, so we
    /// count only what exceeds the cost already recorded for this session.
    fn baseline_after_resume(&self, s: &LiveSession, total: f64) -> f64 {
        if s.acp_session_id.lock().is_none() {
            return 0.0;
        }
        let recorded: f64 = self
            .store
            .usage_for_session(&s.id)
            .map(|rows| rows.iter().filter_map(|u| u.cost_usd).sum())
            .unwrap_or(0.0);
        // If the reported total is below what we recorded, it restarted at zero.
        if total + 1e-9 >= recorded { recorded } else { 0.0 }
    }

    /// Store a finished turn's tokens and cost, and tell the UI.
    fn record_usage(&self, s: &LiveSession, result: &Value) {
        let cost = s.turn_cost.lock().take();
        let usage = &result["usage"];
        let n = |k: &str| usage[k].as_i64().unwrap_or(0);
        let input = n("inputTokens");
        let cached = n("cachedReadTokens") + n("cachedWriteTokens");
        let output = n("outputTokens") + n("thoughtTokens");
        if input + cached + output == 0 && cost.is_none() {
            return;
        }
        let model = crate::catalog::model_option(&s.config.lock())
            .and_then(|o| o["currentValue"].as_str().map(str::to_string))
            .or_else(|| result.pointer("/_meta/quota/model_usage/0/model").and_then(|m| m.as_str()).map(str::to_string))
            .unwrap_or_else(|| "unknown".into());
        let Ok(ws) = self.store.workspace(&s.workspace_id) else { return };
        let record = crate::store::UsageRecord {
            session_id: s.id.clone(),
            workspace_id: ws.id,
            repo_id: ws.repo_id,
            agent: s.agent.id.clone(),
            model,
            ts: now(),
            input_tokens: input,
            cached_tokens: cached,
            output_tokens: output,
            cost_usd: cost,
        };
        if self.store.add_usage(&record).is_ok() {
            let pricing = crate::usage::load_pricing(&self.store);
            self.emitter.emit(Event::Usage {
                session_id: s.id.clone(),
                usage: crate::usage::price(record, &pricing),
            });
        }
    }

    fn set_config_options(&self, s: &LiveSession, opts: Value) {
        if let Some(catalog) = crate::catalog::from_config(&opts) {
            crate::catalog::save(&self.store, &s.agent.id, &catalog);
        }
        let current = |o: Option<&Value>| o.and_then(|o| o["currentValue"].as_str()).map(str::to_string);
        let (model, effort) = (
            current(crate::catalog::model_option(&opts)),
            current(crate::catalog::effort_option(&opts)),
        );
        if model.is_some() || effort.is_some() {
            let _ = self.store.set_session_model(&s.id, model.as_deref(), effort.as_deref());
        }
        *s.config.lock() = opts.clone();
        self.emitter.emit(Event::SessionConfig { session_id: s.id.clone(), config_options: opts });
    }

    /// When the user approves a plan, agents leave plan mode on their own;
    /// follow them back to auto-accept.
    fn check_plan_exit(self: &Arc<Self>, s: &Arc<LiveSession>) {
        if !s.plan.load(Ordering::SeqCst) || s.applying_mode.load(Ordering::SeqCst) {
            return;
        }
        if is_plan(&s.config.lock()) {
            return;
        }
        s.plan.store(false, Ordering::SeqCst);
        self.emitter.emit(Event::SessionMode { session_id: s.id.clone(), plan: false });
        let this = self.clone();
        let s = s.clone();
        tokio::spawn(async move {
            let conn = s.conn.lock().await.clone();
            if let Some(conn) = conn {
                let _ = this.apply_mode(&s, &conn).await;
            }
        });
    }

    /// Push our plan/auto-accept choice into the agent's own config options.
    async fn apply_mode(&self, s: &LiveSession, conn: &AcpConnection) -> Result<()> {
        let changes = mode_changes(&s.config.lock(), s.plan.load(Ordering::SeqCst));
        if changes.is_empty() {
            return Ok(());
        }
        s.applying_mode.store(true, Ordering::SeqCst);
        let acp_id = s.acp_session_id.lock().clone().unwrap_or_default();
        let mut result = Ok(());
        for (config_id, value) in changes {
            match conn
                .request(
                    "session/set_config_option",
                    json!({"sessionId": acp_id, "configId": config_id, "value": value}),
                )
                .await
            {
                Ok(r) => {
                    if let Some(opts) = r.get("configOptions").filter(|v| v.is_array()) {
                        self.set_config_options(s, opts.clone());
                    }
                }
                Err(e) => result = Err(e),
            }
        }
        s.applying_mode.store(false, Ordering::SeqCst);
        result
    }

    /// Choose model, effort and plan mode for a session before it connects.
    pub fn preset(&self, session_id: &str, plan: bool, model: Option<String>, effort: Option<String>) -> Result<()> {
        let s = self.get(session_id)?;
        s.plan.store(plan, Ordering::SeqCst);
        *s.initial.lock() = Some((model, effort));
        if plan {
            self.emitter.emit(Event::SessionMode { session_id: s.id.clone(), plan });
        }
        Ok(())
    }

    async fn apply_initial(&self, s: &LiveSession, conn: &AcpConnection) -> Result<()> {
        let Some((model, effort)) = s.initial.lock().take() else { return Ok(()) };
        let acp_id = s.acp_session_id.lock().clone().unwrap_or_default();
        // Model first: the available effort levels can depend on it.
        for (is_model, want) in [(true, model), (false, effort)] {
            let Some(want) = want else { continue };
            let change = {
                let config = s.config.lock();
                let option = if is_model {
                    crate::catalog::model_option(&config)
                } else {
                    crate::catalog::effort_option(&config)
                };
                option.and_then(|o| {
                    let valid = crate::catalog::choices(o).iter().any(|c| c.value == want);
                    (valid && o["currentValue"] != want.as_str()).then(|| o["id"].as_str().unwrap_or("").to_string())
                })
            };
            let Some(config_id) = change else { continue };
            let r = conn
                .request(
                    "session/set_config_option",
                    json!({"sessionId": acp_id, "configId": config_id, "value": want}),
                )
                .await?;
            if let Some(opts) = r.get("configOptions").filter(|v| v.is_array()) {
                self.set_config_options(s, opts.clone());
            }
        }
        Ok(())
    }

    pub async fn set_plan_mode(self: &Arc<Self>, session_id: &str, plan: bool) -> Result<()> {
        let s = self.get(session_id)?;
        s.plan.store(plan, Ordering::SeqCst);
        self.emitter.emit(Event::SessionMode { session_id: s.id.clone(), plan });
        // Leaving plan mode also approves anything the agent is waiting on.
        if !plan {
            let conn = s.conn.lock().await.clone();
            if let Some(conn) = &conn {
                let pending: Vec<(String, Value)> = s.permissions.lock().drain().collect();
                for (request_id, id) in pending {
                    let _ = conn.respond(id, json!({"outcome": {"outcome": "cancelled"}})).await;
                    self.emitter.emit(Event::PermissionResolved { session_id: s.id.clone(), request_id });
                }
            }
        }
        let conn = s.conn.lock().await.clone();
        match conn {
            Some(conn) => self.apply_mode(&s, &conn).await,
            None => Ok(()), // applied on connect
        }
    }

    fn set_title(&self, s: &LiveSession, title: &str) {
        let title: String = title.lines().next().unwrap_or("").chars().take(60).collect();
        if title.trim().is_empty() {
            return;
        }
        s.titled.store(true, Ordering::SeqCst);
        let _ = self.store.set_session_title(&s.id, &title);
        self.emitter.emit(Event::SessionTitle { session_id: s.id.clone(), title });
    }

    /// Instant heuristic title, upgraded to a Haiku summary when it arrives.
    /// The workspace gets the same title if it doesn't have one yet.
    fn auto_title(self: &Arc<Self>, s: &Arc<LiveSession>, text: &str) {
        let quick = crate::title::quick_title(text);
        self.set_title(s, &quick);
        let title_workspace = self
            .store
            .workspace(&s.workspace_id)
            .map(|w| w.title.is_empty())
            .unwrap_or(false);
        if title_workspace {
            self.set_workspace_title(&s.workspace_id, &quick);
        }
        let this = self.clone();
        let s = s.clone();
        let text = text.to_string();
        tokio::spawn(async move {
            let summary = crate::title::summarize(&text).await;
            if let Some(summary) = &summary {
                this.set_title(&s, summary);
                if title_workspace {
                    this.set_workspace_title(&s.workspace_id, summary);
                }
            }
            if title_workspace {
                this.name_branch(&s.workspace_id, summary.as_deref().unwrap_or(&quick)).await;
            }
        });
    }

    /// Rename a workspace's auto-named branch ("suneiro/tokyo") after its task
    /// ("suneiro/fix-login-bug"), as long as it hasn't been pushed.
    async fn name_branch(&self, workspace_id: &str, title: &str) {
        // The worktree may still be checking out.
        let mut waited = 0;
        let ws = loop {
            let Ok(ws) = self.store.workspace(workspace_id) else { return };
            if ws.status != "creating" {
                break ws;
            }
            if waited > 600 {
                return;
            }
            waited += 1;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        };
        let settings: crate::setup::Settings = self
            .store
            .setting("settings")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let Some(slug) = crate::workspace::branch_slug(title) else { return };
        let path = std::path::Path::new(&ws.path);
        let pushed = crate::git::has_upstream(path).await || crate::git::remote_branch_exists(path, &ws.branch).await;
        if !settings.rename_branches || !ws.branch.ends_with(&ws.name) || pushed {
            return;
        }
        let prefix = &ws.branch[..ws.branch.len() - ws.name.len()];
        let mut branch = format!("{prefix}{slug}");
        let mut n = 2;
        while crate::git::branch_exists(path, &branch).await || crate::git::remote_branch_exists(path, &branch).await {
            branch = format!("{prefix}{slug}-{n}");
            n += 1;
        }
        if let Err(e) = crate::git::git(path, &["branch", "-m", &ws.branch, &branch]).await {
            log::warn!("could not rename branch: {e:#}");
            return;
        }
        let _ = self.store.set_workspace_branch(workspace_id, &branch);
        self.emitter.emit(Event::WorkspaceBranch { workspace_id: workspace_id.into(), branch });
    }

    fn set_workspace_title(&self, workspace_id: &str, title: &str) {
        if title.trim().is_empty() {
            return;
        }
        let _ = self.store.set_workspace_title(workspace_id, title);
        self.emitter.emit(Event::WorkspaceTitle {
            workspace_id: workspace_id.into(),
            title: title.into(),
        });
    }

    pub fn prompt(self: &Arc<Self>, session_id: &str, text: String) -> Result<()> {
        self.prompt_with(session_id, text, Vec::new())
    }

    /// Prompt with attached images (paths of image files).
    pub fn prompt_with(self: &Arc<Self>, session_id: &str, text: String, images: Vec<String>) -> Result<()> {
        let s = self.get(session_id)?;
        if s.running.swap(true, Ordering::SeqCst) {
            bail!("agent is still working");
        }
        if !s.titled.load(Ordering::SeqCst) && !text.trim().is_empty() {
            self.auto_title(&s, &text);
        }
        self.emitter.emit(Event::UserMessage {
            session_id: s.id.clone(),
            text: text.clone(),
            ts: now(),
            images: images.clone(),
        });
        self.state(&s, "running", None);
        self.emitter.emit(Event::WorkspaceStatus {
            workspace_id: s.workspace_id.clone(),
            status: "dirty".into(),
        });

        let this = self.clone();
        tokio::spawn(async move {
            let result = async {
                let conn = this.connect(&s).await?;
                // After connecting: the worktree is ready by then.
                this.checkpoint(&s).await;
                let acp_id = s.acp_session_id.lock().clone().unwrap_or_default();
                let mut blocks = prompt_blocks(&text, &s.cwd);
                if let Some(list) = blocks.as_array_mut() {
                    list.extend(image_blocks(&images, s.images.load(Ordering::SeqCst)));
                }
                let notes: Vec<String> = s.notes.lock().drain(..).collect();
                if let (false, Some(list)) = (notes.is_empty(), blocks.as_array_mut()) {
                    list.push(json!({"type": "text", "text": format!("<system-reminder>\n{}\n</system-reminder>", notes.join("\n"))}));
                }
                let r = conn.request("session/prompt", json!({"sessionId": acp_id, "prompt": blocks})).await;
                if r.is_err() {
                    // Keep them for the next attempt.
                    let mut pending = s.notes.lock();
                    let later = std::mem::take(&mut *pending);
                    *pending = notes.into_iter().chain(later).collect();
                }
                r
            }
            .await;
            s.running.store(false, Ordering::SeqCst);
            if let Ok(r) = &result {
                this.record_usage(&s, r);
            }
            match result {
                Ok(r) => {
                    let stop_reason =
                        r.get("stopReason").and_then(|v| v.as_str()).unwrap_or("end_turn");
                    this.emitter.emit(Event::TurnEnd {
                        session_id: s.id.clone(),
                        stop_reason: stop_reason.into(),
                        ts: now(),
                    });
                    this.state(&s, "idle", None);
                }
                Err(e) => this.state(&s, "error", Some(format!("{e:#}"))),
            }
            this.emitter.emit(Event::WorkspaceStatus {
                workspace_id: s.workspace_id.clone(),
                status: "dirty".into(),
            });
        });
        Ok(())
    }

    /// Snapshot the worktree before a turn so the user can go back to it.
    /// Best-effort: a failure only means there's no restore point.
    async fn checkpoint(&self, s: &LiveSession) {
        let lock = self.workspace_lock(&s.workspace_id);
        let _guard = lock.lock().await;
        let refname = format!("refs/runner/checkpoints/{}/{}", s.id, now());
        match crate::git::checkpoint(&s.cwd, &refname).await {
            Ok(commit) => self.emitter.emit(Event::Checkpoint { session_id: s.id.clone(), commit }),
            Err(e) => log::warn!("checkpoint failed: {e:#}"),
        }
    }

    /// Serializes snapshotting and restoring a workspace's files.
    pub fn workspace_lock(&self, workspace_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.file_locks.lock().entry(workspace_id.to_string()).or_default().clone()
    }

    /// Tell every open chat of a workspace something along with its next prompt.
    pub fn add_note(&self, workspace_id: &str, note: &str) {
        for s in self.sessions.lock().values().filter(|s| s.workspace_id == workspace_id) {
            s.notes.lock().push(note.to_string());
        }
    }

    /// Whether any chat of the workspace is in the middle of a turn.
    pub fn workspace_busy(&self, workspace_id: &str) -> bool {
        self.sessions
            .lock()
            .values()
            .any(|s| s.workspace_id == workspace_id && s.running.load(Ordering::SeqCst))
    }

    /// Deliver a message now. If a turn is running, agents that support
    /// steering get it injected into that turn; others are stopped and then
    /// prompted. With no turn running it's an ordinary prompt.
    /// Returns "injected", "interrupted" or "sent".
    pub async fn steer(self: &Arc<Self>, session_id: &str, text: String) -> Result<&'static str> {
        self.steer_with(session_id, text, Vec::new()).await
    }

    pub async fn steer_with(self: &Arc<Self>, session_id: &str, text: String, images: Vec<String>) -> Result<&'static str> {
        let s = self.get(session_id)?;
        if !s.running.load(Ordering::SeqCst) {
            self.prompt_with(session_id, text, images)?;
            return Ok("sent");
        }
        let conn = s.conn.lock().await.clone();
        if let (true, Some(conn)) = (s.steering.load(Ordering::SeqCst), conn) {
            let acp_id = s.acp_session_id.lock().clone().unwrap_or_default();
            let mut blocks = prompt_blocks(&text, &s.cwd);
            if let Some(list) = blocks.as_array_mut() {
                list.extend(image_blocks(&images, s.images.load(Ordering::SeqCst)));
            }
            let r = conn
                .request(
                    "_session/steering",
                    json!({
                        "sessionId": acp_id,
                        "prompt": blocks,
                        // If the turn just ended, hand the message back so it
                        // goes through a normal, tracked session/prompt.
                        "_meta": {"steering": {"idleBehavior": "promptRequired"}}
                    }),
                )
                .await?;
            return match r["outcome"].as_str().unwrap_or("") {
                "injected" | "startedNewTurn" => {
                    self.emitter.emit(Event::UserMessage { session_id: s.id.clone(), text, ts: now(), images });
                    Ok("injected")
                }
                "promptRequired" => {
                    self.prompt_with(session_id, text, images)?;
                    Ok("sent")
                }
                other => bail!("the agent couldn't take the message right now ({other})"),
            };
        }
        // No steering support: stop the turn, then send.
        self.cancel(session_id).await?;
        for _ in 0..300 {
            if !s.running.load(Ordering::SeqCst) {
                self.prompt_with(session_id, text, images)?;
                return Ok("interrupted");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        bail!("the agent didn't stop in time; the message is still queued")
    }

    pub async fn cancel(&self, session_id: &str) -> Result<()> {
        let s = self.get(session_id)?;
        let conn = s.conn.lock().await.clone();
        let Some(conn) = conn else { return Ok(()) };
        let pending: Vec<(String, Value)> = s.permissions.lock().drain().collect();
        for (request_id, id) in pending {
            let _ = conn.respond(id, json!({"outcome": {"outcome": "cancelled"}})).await;
            self.emitter.emit(Event::PermissionResolved { session_id: s.id.clone(), request_id });
        }
        let questions: Vec<(String, Value)> = s.questions.lock().drain().collect();
        for (request_id, id) in questions {
            let _ = conn.respond(id, json!({"action": "cancel"})).await;
            self.emitter.emit(Event::QuestionResolved { session_id: s.id.clone(), request_id });
        }
        let acp_id = s.acp_session_id.lock().clone().unwrap_or_default();
        conn.notify("session/cancel", json!({"sessionId": acp_id})).await
    }

    pub async fn respond_permission(
        &self,
        session_id: &str,
        request_id: &str,
        option_id: Option<String>,
    ) -> Result<()> {
        let s = self.get(session_id)?;
        let id = s
            .permissions
            .lock()
            .remove(request_id)
            .ok_or_else(|| anyhow!("permission request already resolved"))?;
        let conn = s.conn.lock().await.clone().ok_or_else(|| anyhow!("agent not connected"))?;
        let outcome = match option_id {
            Some(option_id) => json!({"outcome": "selected", "optionId": option_id}),
            None => json!({"outcome": "cancelled"}),
        };
        conn.respond(id, json!({"outcome": outcome})).await?;
        self.emitter.emit(Event::PermissionResolved {
            session_id: s.id.clone(),
            request_id: request_id.into(),
        });
        Ok(())
    }

    /// Answer a question: `response` is `{"action": "accept", "content": {..}}`,
    /// `{"action": "decline"}` (skip) or `{"action": "cancel"}`.
    pub async fn answer_question(&self, session_id: &str, request_id: &str, response: Value) -> Result<()> {
        let s = self.get(session_id)?;
        let id = s
            .questions
            .lock()
            .remove(request_id)
            .ok_or_else(|| anyhow!("question already answered"))?;
        let conn = s.conn.lock().await.clone().ok_or_else(|| anyhow!("agent not connected"))?;
        conn.respond(id, response).await?;
        self.emitter.emit(Event::QuestionResolved { session_id: s.id.clone(), request_id: request_id.into() });
        Ok(())
    }

    pub async fn set_config(
        self: &Arc<Self>,
        session_id: &str,
        config_id: &str,
        value: Value,
    ) -> Result<()> {
        let s = self.get(session_id)?;
        let conn = self.connect(&s).await?;
        let acp_id = s.acp_session_id.lock().clone().unwrap_or_default();
        let mut params = json!({"sessionId": acp_id, "configId": config_id, "value": value});
        if value.is_boolean() {
            params["type"] = json!("boolean");
        }
        let r = conn.request("session/set_config_option", params).await?;
        if let Some(opts) = r.get("configOptions").filter(|v| v.is_array()) {
            self.emitter.emit(Event::SessionConfig {
                session_id: s.id.clone(),
                config_options: opts.clone(),
            });
        }
        Ok(())
    }

    /// Workspace of every chat in the middle of a turn, one entry per chat.
    pub fn running_workspaces(&self) -> Vec<String> {
        self.sessions
            .lock()
            .values()
            .filter(|s| s.running.load(Ordering::SeqCst))
            .map(|s| s.workspace_id.clone())
            .collect()
    }

    pub fn is_running(&self, session_id: &str) -> bool {
        self.sessions
            .lock()
            .get(session_id)
            .is_some_and(|s| s.running.load(Ordering::SeqCst))
    }

    pub fn close(&self, session_id: &str) {
        if let Some(s) = self.sessions.lock().remove(session_id) {
            if let Ok(guard) = s.conn.try_lock() {
                if let Some(conn) = guard.as_ref() {
                    conn.kill();
                }
            }
        }
    }

    pub fn shutdown(&self) {
        let ids: Vec<String> = self.sessions.lock().keys().cloned().collect();
        for id in ids {
            self.close(&id);
        }
    }
}

/// The prompt text plus every `@path` that names a file in the worktree:
/// small text files are embedded (all our agents support embedded context),
/// others are passed as links for the agent to read itself.
fn prompt_blocks(text: &str, cwd: &std::path::Path) -> Value {
    // No empty text block (e.g. images only): the model API rejects those.
    let mut blocks = if text.trim().is_empty() { vec![] } else { vec![json!({"type": "text", "text": text})] };
    let mut seen = std::collections::HashSet::new();
    for word in text.split_whitespace() {
        let Some(path) = word.strip_prefix('@') else { continue };
        let path = path.trim_end_matches([',', '.', ':', ';', ')', '!', '?']);
        let abs = cwd.join(path);
        if path.is_empty() || !abs.is_file() || !seen.insert(path.to_string()) {
            continue;
        }
        let uri = format!("file://{}", abs.display());
        let text = std::fs::metadata(&abs)
            .ok()
            .filter(|m| m.len() <= 200_000)
            .and_then(|_| std::fs::read_to_string(&abs).ok());
        blocks.push(match text {
            Some(text) => json!({"type": "resource", "resource": {"uri": uri, "text": text, "mimeType": "text/plain"}}),
            None => json!({"type": "resource_link", "uri": uri, "name": path}),
        });
    }
    Value::Array(blocks)
}

/// Image files as ACP image blocks, or as links to the files for agents
/// that don't take images directly (they can still open them).
fn image_blocks(paths: &[String], supported: bool) -> Vec<Value> {
    use base64::Engine;
    paths
        .iter()
        .filter_map(|path| {
            let uri = format!("file://{path}");
            if !supported {
                let name = std::path::Path::new(path).file_name()?.to_string_lossy().to_string();
                return Some(json!({"type": "resource_link", "uri": uri, "name": name}));
            }
            let data = std::fs::read(path).ok()?;
            Some(json!({
                "type": "image",
                "mimeType": crate::attachments::mime_type(path),
                "data": base64::engine::general_purpose::STANDARD.encode(data),
                "uri": uri,
            }))
        })
        .collect()
}

/// Prefer "always" so the agent stops asking for the same tool.
fn auto_allow_option(options: &Value) -> Option<String> {
    let options = options.as_array()?;
    ["allow_always", "allow_once"]
        .iter()
        .find_map(|kind| options.iter().find(|o| o["kind"] == *kind))
        .and_then(|o| o["optionId"].as_str())
        .map(str::to_string)
}

fn option<'a>(config: &'a Value, id: &str) -> Option<&'a Value> {
    config.as_array()?.iter().find(|o| o["id"] == id)
}

fn has_value(opt: &Value, value: &str) -> bool {
    let flat = |o: &Value| o["value"] == value;
    opt["options"].as_array().is_some_and(|opts| {
        opts.iter().any(|o| flat(o) || o["options"].as_array().is_some_and(|g| g.iter().any(flat)))
    })
}

fn is_plan(config: &Value) -> bool {
    ["mode", "collaboration_mode"]
        .iter()
        .filter_map(|id| option(config, id))
        .any(|o| o["currentValue"] == "plan")
}

/// Config changes that put the agent into plan or auto-accept mode.
///
/// - Claude: `mode` = plan | bypassPermissions
/// - Codex: `collaboration_mode` = plan | default, `mode` = agent-full-access
/// - OpenCode: `mode` = plan | build
fn mode_changes(config: &Value, plan: bool) -> Vec<(String, String)> {
    const AUTO: &[&str] = &["bypassPermissions", "agent-full-access", "build", "acceptEdits", "auto"];
    let mut changes = Vec::new();
    let mut want = |id: &str, value: &str| {
        if let Some(o) = option(config, id) {
            if has_value(o, value) && o["currentValue"] != value {
                changes.push((id.to_string(), value.to_string()));
            }
        }
    };
    let auto_value = option(config, "mode").and_then(|o| AUTO.iter().find(|v| has_value(o, v)).copied());
    let collab_plan = option(config, "collaboration_mode").is_some_and(|o| has_value(o, "plan"));
    if collab_plan {
        want("collaboration_mode", if plan { "plan" } else { "default" });
        if let Some(v) = auto_value {
            want("mode", v);
        }
    } else if plan {
        want("mode", "plan");
    } else if let Some(v) = auto_value {
        want("mode", v);
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn select(id: &str, current: &str, values: &[&str]) -> Value {
        json!({"id": id, "type": "select", "currentValue": current,
               "options": values.iter().map(|v| json!({"value": v, "name": v})).collect::<Vec<_>>()})
    }

    #[test]
    fn claude_mode_changes() {
        let config = json!([select("mode", "auto", &["default", "acceptEdits", "plan", "auto", "bypassPermissions"])]);
        assert_eq!(mode_changes(&config, false), vec![("mode".into(), "bypassPermissions".into())]);
        assert_eq!(mode_changes(&config, true), vec![("mode".into(), "plan".into())]);
    }

    #[test]
    fn codex_mode_changes() {
        let config = json!([
            select("mode", "agent", &["read-only", "workspace-write", "agent", "agent-full-access"]),
            select("collaboration_mode", "default", &["default", "plan"]),
        ]);
        assert_eq!(mode_changes(&config, false), vec![("mode".into(), "agent-full-access".into())]);
        assert_eq!(
            mode_changes(&config, true),
            vec![("collaboration_mode".into(), "plan".into()), ("mode".into(), "agent-full-access".into())]
        );
        assert!(!is_plan(&config));
    }

    #[test]
    fn opencode_mode_changes() {
        let config = json!([select("mode", "build", &["build", "plan"])]);
        assert!(mode_changes(&config, false).is_empty());
        assert_eq!(mode_changes(&config, true), vec![("mode".into(), "plan".into())]);
    }

    #[test]
    fn mentions_become_resource_links() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.ts"), "x").unwrap();
        let blocks = prompt_blocks("look at @src/a.ts, not @missing.ts or @src/a.ts again", dir.path());
        let blocks = blocks.as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1]["type"], "resource");
        assert_eq!(blocks[1]["resource"]["text"], "x");
        assert_eq!(prompt_blocks("  ", dir.path()), json!([]));
        assert!(blocks[1]["resource"]["uri"].as_str().unwrap().ends_with("/src/a.ts"));
    }

    #[test]
    fn picks_allow_always() {
        let opts = json!([{"optionId": "a", "kind": "allow_once"}, {"optionId": "b", "kind": "allow_always"}, {"optionId": "c", "kind": "reject_once"}]);
        assert_eq!(auto_allow_option(&opts).as_deref(), Some("b"));
    }
}
