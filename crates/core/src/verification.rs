//! User-configured checks, bound to HEAD, index and every non-ignored worktree file.
use crate::{env, git, Core, Event, WorkspaceJob};
use anyhow::{bail, Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::Path, sync::Arc, time::Duration};
use tokio::{io::AsyncReadExt, sync::watch};

const LOG_LIMIT: usize = 64 * 1024;
const HISTORY_LIMIT: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    pub head: String,
    pub fingerprint: String,
}

/// Content hashes, not mtime/status: editing an already-dirty file invalidates results.
pub async fn revision(path: &Path) -> Result<Revision> {
    let head = git::git(path, &["rev-parse", "HEAD"])
        .await?
        .trim()
        .to_string();
    let out = env::tokio_command("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(path)
        .output()
        .await?;
    if !out.status.success() {
        bail!("cannot enumerate verification inputs");
    }
    let mut files: Vec<_> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| p.to_vec())
        .collect();
    files.sort();
    files.dedup();
    let index = git::git(
        path,
        &[
            "diff",
            "--cached",
            "--binary",
            "--no-ext-diff",
            "--no-textconv",
        ],
    )
    .await?;
    let path = path.to_path_buf();
    let fingerprint = tokio::task::spawn_blocking(move || -> Result<String> {
        use std::io::Read;
        let mut hash = Sha256::new();
        hash.update(index.as_bytes());
        for name in files {
            let name = String::from_utf8(name).context("verification requires UTF-8 file paths")?;
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            let file = path.join(&name);
            match std::fs::symlink_metadata(&file) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => hash.update(b"deleted"),
                Err(e) => return Err(e.into()),
                Ok(meta) if meta.is_dir() => {
                    bail!("verification of submodules requires a separate workspace: {name}")
                }
                Ok(meta) => {
                    if !meta.is_file() && !meta.file_type().is_symlink() {
                        bail!("unsupported verification input: {name}");
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        hash.update(meta.permissions().mode().to_le_bytes());
                    }
                    if meta.file_type().is_symlink() {
                        hash.update(b"link");
                        hash.update(std::fs::read_link(file)?.as_os_str().as_encoded_bytes());
                    } else {
                        hash.update(b"file");
                        hash.update(meta.len().to_le_bytes());
                        let mut f = std::fs::File::open(file)?;
                        let mut buf = [0; 16384];
                        loop {
                            let n = f.read(&mut buf)?;
                            if n == 0 {
                                break;
                            }
                            hash.update(&buf[..n]);
                        }
                    }
                }
            }
        }
        Ok(format!("{:x}", hash.finalize()))
    })
    .await??;
    Ok(Revision { head, fingerprint })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckPreset {
    pub id: String,
    pub command: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckRun {
    pub id: String,
    pub workspace_id: String,
    pub preset_id: String,
    pub command: String,
    pub cwd: String,
    pub revision: Revision,
    pub status: String,
    pub log: String,
    pub exit_code: Option<i32>,
    pub stale: bool,
}
#[derive(Default)]
pub(crate) struct Runner {
    active: Mutex<HashMap<String, (String, watch::Sender<bool>)>>,
    publishing: Mutex<()>,
    processes: Mutex<HashMap<String, u32>>,
    stopped: std::sync::atomic::AtomicBool,
}
impl Runner {
    pub fn busy(&self, workspace: &str) -> bool {
        self.active.lock().contains_key(workspace)
    }
    pub fn cancel_all(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        for (_, tx) in self.active.lock().values() {
            let _ = tx.send(true);
        }
        for pid in self.processes.lock().values() {
            kill_group(*pid);
        }
    }
}

impl Core {
    pub fn verification_presets(&self, repo_id: &str) -> Result<Vec<CheckPreset>> {
        self.store.repo(repo_id)?;
        Ok(self
            .store
            .setting(&format!("verification_presets:{repo_id}"))?
            .map(|v| serde_json::from_str(&v))
            .transpose()?
            .unwrap_or_default())
    }
    pub fn save_verification_presets(
        &self,
        repo_id: &str,
        presets: Vec<CheckPreset>,
    ) -> Result<()> {
        self.store.repo(repo_id)?;
        let mut ids = std::collections::HashSet::new();
        for p in &presets {
            if !["tests", "lint", "typecheck", "build"].contains(&p.id.as_str())
                || !ids.insert(&p.id)
                || p.command.trim().is_empty()
                || p.command.len() > 8192
            {
                bail!("invalid check preset");
            }
        }
        self.store.set_setting(
            &format!("verification_presets:{repo_id}"),
            &serde_json::to_string(&presets)?,
        )
    }
    fn saved_checks(&self, workspace: &str) -> Result<Vec<CheckRun>> {
        Ok(self
            .store
            .setting(&format!("verification_runs:{workspace}"))?
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default())
    }
    fn publish_check(&self, run: &CheckRun) -> Result<()> {
        let _guard = self.verification.publishing.lock();
        let mut runs = self.saved_checks(&run.workspace_id)?;
        let mut next = run.clone();
        if let Some(previous) = runs.iter().find(|r| r.id == run.id) {
            if previous.status != "running" && next.status == "running" {
                next = previous.clone();
            }
            next.stale |= previous.stale || run.stale;
        }
        runs.retain(|r| r.id != run.id);
        runs.push(next.clone());
        if runs.len() > HISTORY_LIMIT {
            runs.drain(..runs.len() - HISTORY_LIMIT);
        }
        self.store.set_setting(
            &format!("verification_runs:{}", run.workspace_id),
            &serde_json::to_string(&runs)?,
        )?;
        self.emitter.emit(Event::VerificationChanged {
            workspace_id: run.workspace_id.clone(),
            run: next,
        });
        Ok(())
    }
    pub async fn verification_status(&self, workspace_id: &str) -> Result<Vec<CheckRun>> {
        let (ws, path) = self.ws_path(workspace_id)?;
        let current = revision(&path).await?;
        let presets = self.verification_presets(&ws.repo_id)?;
        let mut runs = self.saved_checks(workspace_id)?;
        for run in &mut runs {
            let old = run.clone();
            run.stale |= run.revision != current
                || !presets
                    .iter()
                    .any(|p| p.id == run.preset_id && p.command == run.command);
            if run.status == "running" && !self.verification.busy(workspace_id) {
                run.status = "cancelled".into();
                run.log.push_str("\nInterrupted by application shutdown.\n");
            }
            if old.stale != run.stale || old.status != run.status {
                self.publish_check(run)?;
            }
        }
        Ok(runs)
    }
    pub async fn run_check(
        self: &Arc<Self>,
        workspace_id: &str,
        preset_id: &str,
    ) -> Result<CheckRun> {
        let (ws, path) = self.ws_path(workspace_id)?;
        let lock = self.agents.workspace_lock(workspace_id);
        let guard = lock
            .try_lock_owned()
            .map_err(|_| anyhow::anyhow!("workspace operation already running"))?;
        self.ensure_verification_idle(workspace_id)?;
        if ws.status == "archived" || ws.status == "creating" || ws.status == "failed" {
            bail!("workspace is not ready");
        }
        let preset = self
            .verification_presets(&ws.repo_id)?
            .into_iter()
            .find(|p| p.id == preset_id)
            .context("configure this check in repository settings first")?;
        let revision = revision(&path).await?;
        let run = CheckRun {
            id: uuid::Uuid::new_v4().to_string(),
            workspace_id: workspace_id.into(),
            preset_id: preset.id,
            command: preset.command,
            cwd: ws.path.clone(),
            revision,
            status: "running".into(),
            log: String::new(),
            exit_code: None,
            stale: false,
        };
        let (tx, rx) = watch::channel(false);
        self.verification
            .active
            .lock()
            .insert(workspace_id.into(), (run.id.clone(), tx));
        if let Err(e) = self.publish_check(&run) {
            self.verification.active.lock().remove(workspace_id);
            return Err(e);
        }
        let job = WorkspaceJob::start(&self.jobs, workspace_id);
        let core = self.clone();
        let task_run = run.clone();
        tokio::spawn(async move {
            let _guard = guard;
            let _job = job;
            let mut run = task_run;
            if let Err(e) = execute(&core, &mut run, rx).await {
                run.status = "failed".into();
                append_log(&mut run.log, &format!("\n{e:#}\n"));
            }
            match revision_at_end(&run).await {
                Ok(r) => run.stale |= r != run.revision,
                Err(_) => run.stale = true,
            }
            let _ = core.publish_check(&run);
            core.verification.active.lock().remove(&run.workspace_id);
        });
        Ok(run)
    }
    pub fn cancel_check(&self, workspace_id: &str, run_id: &str) -> Result<()> {
        let active = self.verification.active.lock();
        let (id, tx) = active.get(workspace_id).context("check is not running")?;
        if id != run_id {
            bail!("check is no longer running");
        }
        tx.send(true).context("check already finished")
    }
    pub(crate) fn ensure_verification_idle(&self, workspace_id: &str) -> Result<()> {
        if self.agents.workspace_busy(workspace_id)
            || self.verification.busy(workspace_id)
            || self.jobs.lock().contains_key(workspace_id)
        {
            bail!("workspace is busy; stop its agents and checks first");
        }
        Ok(())
    }
}
async fn revision_at_end(run: &CheckRun) -> Result<Revision> {
    revision(Path::new(&run.cwd)).await
}
fn append_log(log: &mut String, text: &str) {
    log.push_str(text);
    if log.len() > LOG_LIMIT {
        let mut n = log.len() - LOG_LIMIT;
        while !log.is_char_boundary(n) {
            n += 1;
        }
        log.drain(..n);
    }
}

fn kill_group(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = env::std_command("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}
struct ProcessGroup<'a> {
    pid: u32,
    workspace_id: &'a str,
    runner: &'a Runner,
}
impl Drop for ProcessGroup<'_> {
    fn drop(&mut self) {
        let mut processes = self.runner.processes.lock();
        kill_group(self.pid);
        processes.remove(self.workspace_id);
    }
}
async fn execute(core: &Core, run: &mut CheckRun, mut cancel: watch::Receiver<bool>) -> Result<()> {
    use std::process::Stdio;
    let mut cmd = env::tokio_command(&env::user_shell());
    cmd.args(["-c", &run.command])
        .current_dir(&run.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = {
        let mut processes = core.verification.processes.lock();
        if core
            .verification
            .stopped
            .load(std::sync::atomic::Ordering::SeqCst)
            || *cancel.borrow()
        {
            run.status = "cancelled".into();
            return Ok(());
        }
        let child = cmd.spawn().context("start configured check")?;
        processes.insert(
            run.workspace_id.clone(),
            child.id().context("missing check pid")?,
        );
        child
    };
    let workspace_id = run.workspace_id.clone();
    let group = ProcessGroup {
        pid: child.id().context("missing check pid")?,
        workspace_id: &workspace_id,
        runner: &core.verification,
    };
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut a = [0; 4096];
    let mut b = [0; 4096];
    let mut out_open = true;
    let mut err_open = true;
    let timeout = tokio::time::sleep(Duration::from_secs(30 * 60));
    tokio::pin!(timeout);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let status = loop {
        tokio::select! {
            biased;
            _ = cancel.changed() => { run.status = "cancelled".into(); break None; }
            _ = &mut timeout => { append_log(&mut run.log, "\nCheck timed out after 30 minutes.\n"); run.status = "failed".into(); break None; }
            status = child.wait() => { break Some(status?); }
            _ = tick.tick() => { core.publish_check(run)?; }
            n = stdout.read(&mut a), if out_open => { let n = n?; out_open = n != 0; append_log(&mut run.log, &String::from_utf8_lossy(&a[..n])); }
            n = stderr.read(&mut b), if err_open => { let n = n?; err_open = n != 0; append_log(&mut run.log, &String::from_utf8_lossy(&b[..n])); }
        }
    };
    drop(group);
    if status.is_none() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    let mut rest = [0; 4096];
    let drain = async {
        for stream in [
            &mut stdout as &mut (dyn tokio::io::AsyncRead + Unpin + Send),
            &mut stderr,
        ] {
            loop {
                let n = stream.read(&mut rest).await?;
                if n == 0 {
                    break;
                }
                append_log(&mut run.log, &String::from_utf8_lossy(&rest[..n]));
            }
        }
        Ok::<_, std::io::Error>(())
    };
    let _ = tokio::time::timeout(Duration::from_secs(1), drain).await;
    if let Some(status) = status {
        run.exit_code = status.code();
        run.status = if status.success() { "passed" } else { "failed" }.into();
    }
    Ok(())
}
