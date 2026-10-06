//! Suneiro core: repos, git worktree workspaces, ACP agent sessions, terminals
//! and GitHub integration. Deliberately free of any UI framework so it can back
//! the desktop app, a CLI or a headless daemon.

pub mod acp;
pub mod agent;
pub mod attachments;
pub mod catalog;
pub mod env;
pub mod events;
pub mod forge;
pub mod git;
pub mod pty;
pub mod recent;
pub mod setup;
pub mod title;
pub mod usage;
pub mod store;
pub mod storage;
pub mod workspace;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use tokio::io::AsyncReadExt;

pub use agent::{builtin_agents, AgentDef, Agents};
pub use events::{Event, Sink};
pub use store::{Repo, Session, Store, Workspace};

use crate::env::{tokio_command, user_shell};
use crate::store::now;
use crate::setup::Settings;
use crate::workspace::{load_file_config, merge_config, pick_name, script_env, RepoConfig};

/// Stash message used to keep a workspace's uncommitted work while archived.
fn archive_marker(workspace_id: &str) -> String {
    format!("runner-archive:{workspace_id}")
}

/// Work in progress that a restart would interrupt (e.g. to install an update).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Activity {
    /// Chats in the middle of a turn (including ones waiting for an answer).
    pub agents: usize,
    /// Workspaces with a running agent, or being set up or archived.
    pub workspaces: usize,
}

impl Activity {
    pub fn is_idle(&self) -> bool {
        self.agents == 0 && self.workspaces == 0
    }
}

/// Marks a workspace busy (setup or archive script, worktree changes) until dropped.
struct WorkspaceJob {
    jobs: Arc<parking_lot::Mutex<std::collections::HashMap<String, usize>>>,
    workspace_id: String,
}

impl WorkspaceJob {
    fn start(jobs: &Arc<parking_lot::Mutex<std::collections::HashMap<String, usize>>>, workspace_id: &str) -> Self {
        *jobs.lock().entry(workspace_id.to_string()).or_default() += 1;
        Self { jobs: jobs.clone(), workspace_id: workspace_id.to_string() }
    }
}

impl Drop for WorkspaceJob {
    fn drop(&mut self) {
        let mut jobs = self.jobs.lock();
        if let Some(n) = jobs.get_mut(&self.workspace_id) {
            *n -= 1;
            if *n == 0 {
                jobs.remove(&self.workspace_id);
            }
        }
    }
}

/// Persists transcript events, then forwards everything to the UI sink.
#[derive(Clone)]
pub struct Emitter {
    store: Arc<Store>,
    sink: Sink,
}

impl Emitter {
    pub fn emit(&self, event: Event) {
        if let Some(session_id) = event.persist_key() {
            if let Ok(v) = serde_json::to_value(&event) {
                self.store.append_event(session_id, v);
            }
        }
        (self.sink)(event);
    }
}

pub struct Core {
    pub store: Arc<Store>,
    data_dir: PathBuf,
    pub agents: Arc<Agents>,
    pub terminals: pty::Terminals,
    emitter: Emitter,
    /// Last PR info sent per workspace, to only emit changes.
    prs: parking_lot::Mutex<std::collections::HashMap<String, Value>>,
    pr_refresh: tokio::sync::Notify,
    /// When each repo's base branch was last fetched, to fetch at most once a minute.
    fetched: parking_lot::Mutex<std::collections::HashMap<String, std::time::Instant>>,
    /// Workspaces with a setup/archive in progress (count per workspace).
    jobs: Arc<parking_lot::Mutex<std::collections::HashMap<String, usize>>>,
}

impl Core {
    pub fn new(data_dir: &Path, sink: Sink) -> Result<Arc<Self>> {
        let store = Arc::new(Store::open(&storage::database_path(data_dir))?);
        let emitter = Emitter { store: store.clone(), sink };
        let agents = Arc::new(Agents::new(store.clone(), emitter.clone()));
        // Warm the login-shell env capture off the UI's critical path.
        std::thread::spawn(|| {
            env::login_env();
        });
        Ok(Arc::new(Self {
            store,
            data_dir: data_dir.to_path_buf(),
            agents,
            terminals: Default::default(),
            emitter,
            prs: Default::default(),
            pr_refresh: tokio::sync::Notify::new(),
            fetched: Default::default(),
            jobs: Default::default(),
        }))
    }

    /// What a restart would interrupt right now.
    pub fn activity(&self) -> Activity {
        let mut workspaces: std::collections::HashSet<String> = self.jobs.lock().keys().cloned().collect();
        let running = self.agents.running_workspaces();
        let agents = running.len();
        workspaces.extend(running);
        Activity { agents, workspaces: workspaces.len() }
    }

    pub fn shutdown(&self) {
        self.agents.shutdown();
        self.terminals.kill_all();
    }

    // ---- settings ----

    pub fn settings(&self) -> Settings {
        self.store
            .setting("settings")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.store.set_setting("settings", &serde_json::to_string(settings)?)
    }

    // ---- repos ----

    pub async fn add_repo(&self, path: &str) -> Result<Repo> {
        let root = git::repo_root(Path::new(path))
            .await
            .map_err(|_| anyhow!("{path} is not a git repository"))?;
        if let Some(existing) = self.store.repo_by_path(&root)? {
            return Ok(existing);
        }
        let repo = Repo {
            id: uuid::Uuid::new_v4().to_string(),
            name: Path::new(&root).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            default_branch: git::default_branch(Path::new(&root)).await,
            path: root,
        };
        self.store.add_repo(&repo)?;
        Ok(repo)
    }

    /// Branches to start a workspace from.
    pub async fn branches(&self, repo_id: &str) -> Result<Vec<git::Branch>> {
        let repo = self.store.repo(repo_id)?;
        git::branches(Path::new(&repo.path)).await
    }

    /// Open pull requests to start a workspace from.
    pub async fn open_prs(&self, repo_id: &str) -> Result<Vec<Value>> {
        let repo = self.store.repo(repo_id)?;
        forge::open_prs(Path::new(&repo.path)).await
    }

    /// Repositories the user recently used with coding agents, not yet added.
    pub async fn recent_projects(&self, limit: usize) -> Result<Vec<recent::RecentProject>> {
        let added: Vec<String> = self.store.repos()?.into_iter().map(|r| r.path).collect();
        Ok(tokio::task::spawn_blocking(move || recent::recent_projects(&added, limit)).await?)
    }

    /// Clone `owner/repo` (or a URL) with `gh` into ~/projects (or ~) and add it.
    pub async fn clone_repo(&self, spec: &str) -> Result<Repo> {
        let spec = spec.trim().trim_end_matches(".git");
        let name = spec.rsplit(['/', ':']).next().filter(|n| !n.is_empty()).ok_or_else(|| anyhow!("enter owner/repo or a URL"))?;
        let home = dirs::home_dir().ok_or_else(|| anyhow!("no home directory"))?;
        let parent = if home.join("projects").is_dir() { home.join("projects") } else { home };
        let dest = parent.join(name);
        if !dest.exists() {
            let out = tokio_command("gh")
                .args(["repo", "clone", spec, &dest.to_string_lossy()])
                .output()
                .await
                .map_err(|_| anyhow!("the GitHub CLI (gh) is required to clone"))?;
            if !out.status.success() {
                bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
            }
        }
        self.add_repo(&dest.to_string_lossy()).await
    }

    // ---- workspaces ----

    pub async fn create_workspace(self: &Arc<Self>, repo_id: &str) -> Result<Workspace> {
        self.new_workspace(repo_id, None).await
    }

    /// Start a workspace on an existing branch (local or only on origin) or
    /// on a pull request's branch.
    pub async fn create_workspace_from(self: &Arc<Self>, repo_id: &str, branch: Option<&str>, pr: Option<u64>) -> Result<Workspace> {
        let repo = self.store.repo(repo_id)?;
        let repo_path = PathBuf::from(&repo.path);
        let (branch, base, title) = match (branch, pr) {
            (_, Some(n)) => {
                let head = forge::pr_head(&repo_path, n).await?;
                let base = head["baseRefName"].as_str().unwrap_or(&repo.default_branch).to_string();
                let title = head["title"].as_str().unwrap_or_default().to_string();
                if head["isCrossRepository"].as_bool() == Some(true) {
                    // A fork's branch: fetch the PR head into a local branch.
                    let local = format!("pr-{n}");
                    // Fast-forwards an existing pr-<n> to the PR's latest head.
                    let fetched = git::git(&repo_path, &["fetch", "-q", "origin", &format!("pull/{n}/head:{local}")]).await;
                    if !git::branch_exists(&repo_path, &local).await {
                        fetched?;
                    }
                    (local, base, title)
                } else {
                    let b = head["headRefName"].as_str().ok_or_else(|| anyhow!("PR #{n} has no branch"))?;
                    (b.to_string(), base, title)
                }
            }
            (Some(b), None) => (b.trim().to_string(), repo.default_branch.clone(), String::new()),
            (None, None) => bail!("choose a branch or pull request"),
        };
        if let Some(open) = self.store.workspaces()?.iter().find(|w| w.repo_id == repo.id && w.branch == branch) {
            bail!("`{branch}` is already open in workspace {}", if open.title.is_empty() { &open.name } else { &open.title });
        }
        if let Some(at) = git::checked_out_at(&repo_path, &branch).await {
            bail!("`{branch}` is checked out at {at}; switch that checkout to another branch first");
        }
        self.new_workspace(repo_id, Some((branch, base, title))).await
    }

    /// New workspace on a fresh branch, or on `existing` (branch, base, title).
    async fn new_workspace(self: &Arc<Self>, repo_id: &str, existing: Option<(String, String, String)>) -> Result<Workspace> {
        let repo = self.store.repo(repo_id)?;
        let repo_path = PathBuf::from(&repo.path);
        let settings = self.settings();
        let root = PathBuf::from(&settings.workspaces_root).join(&repo.name);
        let prefix = settings.branch_prefix.trim();
        let taken = self.store.workspace_names(repo_id)?;
        let mut name = pick_name(&taken, now() as u64);
        let mut n = 2;
        while git::git(&repo_path, &["rev-parse", "--verify", "--quiet", &format!("{prefix}{name}")])
            .await
            .is_ok()
            || root.join(&name).exists()
        {
            name = format!("{}-{n}", pick_name(&taken, now() as u64));
            n += 1;
        }
        let path = root.join(&name);
        let new_branch = existing.is_none();
        let (branch, base_branch, title) =
            existing.unwrap_or_else(|| (format!("{prefix}{name}"), repo.default_branch.clone(), String::new()));
        let ws = Workspace {
            id: uuid::Uuid::new_v4().to_string(),
            repo_id: repo.id.clone(),
            branch,
            name,
            base_branch,
            path: path.to_string_lossy().to_string(),
            status: "creating".into(),
            created_at: now(),
            title,
            archived_at: None,
            unread: false,
        };
        self.store.add_workspace(&ws)?;

        // Fetching and checking out can take a while on big repos; return right
        // away and let the UI show progress via WorkspaceStatus events.
        self.spawn_worktree_setup(ws.clone(), repo, new_branch);
        Ok(ws)
    }

    /// Create (or re-create) the worktree in the background, then run setup.
    fn spawn_worktree_setup(self: &Arc<Self>, ws: Workspace, repo: Repo, new_branch: bool) {
        let this = self.clone();
        let job = WorkspaceJob::start(&self.jobs, &ws.id);
        tokio::spawn(async move {
            let _job = job;
            let repo_path = PathBuf::from(&repo.path);
            let status = match this.prepare_worktree(&ws, &repo_path, new_branch).await {
                Ok(Some(setup)) => {
                    this.set_status(&ws.id, "setting_up");
                    let ok = this.run_script(&ws, &repo.path, &setup).await;
                    if ok { "ready" } else { "setup_failed" }
                }
                Ok(None) => "ready",
                Err(e) => {
                    this.emitter.emit(Event::ScriptOutput {
                        workspace_id: ws.id.clone(),
                        data: format!("Failed to create worktree: {e:#}\n"),
                    });
                    "failed"
                }
            };
            this.set_status(&ws.id, status);
        });
    }

    /// Bring an archived workspace back: check its branch out into a fresh
    /// worktree. Chat history is kept, so sessions can continue.
    pub async fn restore_workspace(self: &Arc<Self>, workspace_id: &str) -> Result<Workspace> {
        let mut ws = self.store.workspace(workspace_id)?;
        if ws.status != "archived" {
            bail!("workspace is not archived");
        }
        let repo = self.store.repo(&ws.repo_id)?;
        if !git::branch_exists(Path::new(&repo.path), &ws.branch).await {
            bail!("branch `{}` no longer exists, so this workspace can't be restored", ws.branch);
        }
        if Path::new(&ws.path).exists() {
            bail!("{} already exists; move it away and try again", ws.path);
        }
        self.store.set_workspace_status(&ws.id, "creating")?;
        self.store.set_archived_at(&ws.id, None)?;
        ws.status = "creating".into();
        ws.archived_at = None;
        self.spawn_worktree_setup(ws.clone(), repo, false);
        Ok(ws)
    }

    fn set_status(&self, workspace_id: &str, status: &str) {
        let _ = self.store.set_workspace_status(workspace_id, status);
        self.emitter.emit(Event::WorkspaceStatus { workspace_id: workspace_id.into(), status: status.into() });
    }

    /// Create the worktree and copy configured files. Returns the setup script, if any.
    async fn prepare_worktree(&self, ws: &Workspace, repo_path: &Path, new_branch: bool) -> Result<Option<String>> {
        let path = PathBuf::from(&ws.path);
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        if new_branch {
            git::create_worktree(repo_path, &path, &ws.branch, &ws.base_branch).await?;
        } else {
            git::ensure_local_branch(repo_path, &ws.branch).await?;
            git::add_worktree(repo_path, &path, &ws.branch).await?;
            if let Some(stash) = git::stash_find(repo_path, &archive_marker(&ws.id)).await {
                if let Err(e) = git::stash_pop(&path, &stash).await {
                    self.emitter.emit(Event::ScriptOutput {
                        workspace_id: ws.id.clone(),
                        data: format!("Couldn't re-apply the changes saved when archiving ({e:#}). They're kept in `git stash` as {stash}.\n"),
                    });
                }
            }
        }
        let config = self.config_for(&ws.repo_id, &path);
        for file in &config.copy {
            let (from, to) = (repo_path.join(file), path.join(file));
            if from.exists() && !to.exists() {
                if let Some(dir) = to.parent() {
                    let _ = tokio::fs::create_dir_all(dir).await;
                }
                let _ = tokio::fs::copy(&from, &to).await;
            }
        }
        Ok(config.scripts.setup)
    }

    /// Run a repo script inside the worktree, streaming output as `ScriptOutput`.
    async fn run_script(&self, ws: &Workspace, repo_path: &str, script: &str) -> bool {
        let emit = |data: String| {
            self.emitter.emit(Event::ScriptOutput { workspace_id: ws.id.clone(), data })
        };
        emit(format!("$ {script}\n"));
        let child = tokio_command(&user_shell())
            .args(["-lc", script])
            .current_dir(&ws.path)
            .envs(script_env(repo_path, &ws.name, &ws.path, &ws.id))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                emit(format!("failed to start: {e}\n"));
                return false;
            }
        };
        let (mut out, mut err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
        let (mut b1, mut b2) = (vec![0u8; 16384], vec![0u8; 16384]);
        let (mut out_done, mut err_done) = (false, false);
        while !(out_done && err_done) {
            tokio::select! {
                r = out.read(&mut b1), if !out_done => match r {
                    Ok(0) | Err(_) => out_done = true,
                    Ok(n) => emit(String::from_utf8_lossy(&b1[..n]).into_owned()),
                },
                r = err.read(&mut b2), if !err_done => match r {
                    Ok(0) | Err(_) => err_done = true,
                    Ok(n) => emit(String::from_utf8_lossy(&b2[..n]).into_owned()),
                },
            }
        }
        let ok = child.wait().await.map(|s| s.success()).unwrap_or(false);
        emit(if ok { "\n✓ done\n".into() } else { "\n✗ script failed\n".into() });
        ok
    }

    pub async fn archive_workspace(&self, workspace_id: &str) -> Result<()> {
        let ws = self.store.workspace(workspace_id)?;
        let repo = self.store.repo(&ws.repo_id)?;
        let _job = WorkspaceJob::start(&self.jobs, &ws.id);
        for s in self.store.sessions(&ws.id)? {
            self.agents.close(&s.id);
        }
        let path = Path::new(&ws.path);
        if path.exists() {
            if let Some(script) = self.config_for(&ws.repo_id, path).scripts.archive {
                self.run_script(&ws, &repo.path, &script).await;
            }
            // Never throw away work: uncommitted changes go to a stash that
            // restore re-applies. If that fails, don't archive.
            git::stash_save(path, &archive_marker(&ws.id)).await?;
            let trash = path.parent().and_then(Path::parent).unwrap_or(path).join(".trash");
            git::discard_worktree(Path::new(&repo.path), path, &trash).await?;
        }
        self.store.set_workspace_status(&ws.id, "archived")?;
        self.store.set_archived_at(&ws.id, Some(now()))?;
        Ok(())
    }

    /// User-chosen title; replaces any auto-generated one.
    pub fn rename_workspace(&self, workspace_id: &str, title: &str) -> Result<()> {
        let title = title.trim();
        self.store.set_workspace_title(workspace_id, title)?;
        self.emitter.emit(Event::WorkspaceTitle { workspace_id: workspace_id.into(), title: title.into() });
        Ok(())
    }

    /// The effective config for a workspace (committed file + app settings).
    pub fn repo_config(&self, workspace_id: &str) -> Result<RepoConfig> {
        let ws = self.store.workspace(workspace_id)?;
        Ok(self.config_for(&ws.repo_id, Path::new(&ws.path)))
    }

    fn config_for(&self, repo_id: &str, checkout: &Path) -> RepoConfig {
        merge_config(load_file_config(checkout).map(|(_, c)| c), self.repo_settings(repo_id))
    }

    /// Scripts and files to copy set in the app for a repository.
    pub fn repo_settings(&self, repo_id: &str) -> RepoConfig {
        self.store
            .setting(&format!("repo_config:{repo_id}"))
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save_repo_settings(&self, repo_id: &str, config: &RepoConfig) -> Result<()> {
        self.store.set_setting(&format!("repo_config:{repo_id}"), &serde_json::to_string(config)?)
    }

    /// The config file committed in the repository's main checkout, if any.
    pub fn repo_file_config(&self, repo_id: &str) -> Result<Option<(String, RepoConfig)>> {
        let repo = self.store.repo(repo_id)?;
        Ok(load_file_config(Path::new(&repo.path)).map(|(n, c)| (n.to_string(), c)))
    }

    // ---- sessions ----

    /// New chat. `model`/`effort` are applied once the agent starts; plan
    /// mode follows the user's default.
    pub fn create_session(
        &self,
        workspace_id: &str,
        agent_id: &str,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<Session> {
        self.agents.def(agent_id)?;
        let session = Session {
            id: uuid::Uuid::new_v4().to_string(),
            workspace_id: workspace_id.into(),
            agent_id: agent_id.into(),
            acp_session_id: None,
            title: String::new(),
            created_at: now(),
            model: model.clone(),
            effort: effort.clone(),
        };
        self.store.add_session(&session)?;
        self.agents.preset(&session.id, self.settings().plan_by_default, model, effort)?;
        self.agents.warm_up(&session.id);
        Ok(session)
    }

    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        self.agents.close(session_id);
        // The chat's checkpoints go with it (in the background; it's only cleanup).
        // Refs are shared by the repo, so this works even when the workspace
        // is archived (no worktree).
        let repo = self.store.session(session_id).and_then(|s| self.store.workspace(&s.workspace_id)).and_then(|w| self.store.repo(&w.repo_id));
        if let Ok(repo) = repo {
            let prefix = format!("refs/runner/checkpoints/{session_id}/");
            std::thread::spawn(move || {
                let git = |args: &[&str]| env::std_command("git").args(args).current_dir(&repo.path).output();
                if let Ok(out) = git(&["for-each-ref", "--format=%(refname)", &prefix]) {
                    for r in String::from_utf8_lossy(&out.stdout).lines().filter(|r| !r.is_empty()) {
                        let _ = git(&["update-ref", "-d", r]);
                    }
                }
            });
        }
        self.store.delete_session(session_id)
    }

    /// Put the workspace's files back to a checkpoint taken before one of
    /// the chat's messages. Returns a checkpoint of the current state, so
    /// the restore can be undone.
    pub async fn restore_checkpoint(&self, session_id: &str, commit: &str) -> Result<String> {
        let session = self.store.session(session_id)?;
        let (_, path) = self.ws_path(&session.workspace_id)?;
        let lock = self.agents.workspace_lock(&session.workspace_id);
        let _guard = lock.lock().await;
        if self.agents.workspace_busy(&session.workspace_id) {
            bail!("an agent is working in this workspace; stop it first");
        }
        git::git(&path, &["cat-file", "-e", &format!("{commit}^{{commit}}")])
            .await
            .map_err(|_| anyhow!("this checkpoint no longer exists"))?;
        let undo = git::checkpoint(&path, &format!("refs/runner/checkpoints/{session_id}/{}", now())).await?;
        git::restore_checkpoint(&path, commit).await?;
        self.agents.add_note(
            &session.workspace_id,
            "The user restored this workspace's files (and git HEAD) to how they were before one of their earlier \
             messages. Changes made after that point are gone; re-read files instead of relying on what you saw before.",
        );
        self.emitter.emit(Event::CheckpointRestored {
            session_id: session_id.into(),
            commit: commit.into(),
            undo: Some(undo.clone()),
            ts: now(),
        });
        self.emitter.emit(Event::WorkspaceStatus { workspace_id: session.workspace_id, status: "dirty".into() });
        Ok(undo)
    }

    // ---- attachments ----

    fn attachments_dir(&self) -> PathBuf {
        self.data_dir.join("attachments")
    }

    /// Store a pasted image (base64). Returns the path to attach to a prompt.
    pub fn save_attachment(&self, mime_type: &str, data: &str) -> Result<String> {
        attachments::save(&self.attachments_dir(), mime_type, data)
    }

    /// Attach an image file by copying it. Returns the copy's path.
    pub fn import_attachment(&self, path: &str) -> Result<String> {
        attachments::import(&self.attachments_dir(), path)
    }

    pub fn attachment_data_url(&self, path: &str) -> Result<String> {
        attachments::data_url(&self.attachments_dir(), path)
    }

    // ---- usage & cost ----

    /// Usage since `since` (ms) with costs reported or estimated.
    pub fn usage(&self, since: i64) -> Result<Vec<usage::PricedUsage>> {
        let pricing = usage::load_pricing(&self.store);
        Ok(self.store.usage_since(since)?.into_iter().map(|u| usage::price(u, &pricing)).collect())
    }

    pub fn pricing(&self) -> usage::Pricing {
        usage::load_pricing(&self.store)
    }

    pub fn save_pricing(&self, pricing: &usage::Pricing) -> Result<()> {
        usage::save_pricing(&self.store, pricing)
    }

    // ---- model catalog ----

    pub fn catalogs(&self) -> std::collections::HashMap<String, catalog::Catalog> {
        self.agents
            .defs()
            .into_iter()
            .filter_map(|d| catalog::load(&self.store, &d.id).map(|c| (d.id, c)))
            .collect()
    }

    /// Discover an agent's models by starting it briefly (no prompt is sent).
    pub async fn refresh_catalog(&self, agent_id: &str) -> Result<catalog::Catalog> {
        let def = self.agents.def(agent_id)?;
        let c = catalog::discover(&def).await?;
        catalog::save(&self.store, agent_id, &c);
        Ok(c)
    }

    // ---- pull requests ----

    /// Poll GitHub for the PRs of every active workspace: once a minute, or
    /// right away after `refresh_prs`. Must be called inside a tokio runtime.
    pub fn start_pr_poller(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                this.poll_prs().await;
                tokio::select! {
                    _ = this.pr_refresh.notified() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => {}
                }
            }
        });
    }

    pub fn refresh_prs(&self) {
        self.pr_refresh.notify_one();
    }

    /// Current PR info for all workspaces (for a UI that just (re)loaded).
    pub fn cached_prs(&self) -> std::collections::HashMap<String, Value> {
        self.prs.lock().clone()
    }

    pub async fn poll_prs(&self) {
        let Ok(workspaces) = self.store.workspaces() else { return };
        let Ok(repos) = self.store.repos() else { return };
        for repo in repos {
            let mine: Vec<&Workspace> = workspaces.iter().filter(|w| w.repo_id == repo.id).collect();
            if mine.is_empty() {
                continue;
            }
            // Not a GitHub repo, gh missing or signed out: just show nothing.
            let Ok(prs) = forge::list_prs(Path::new(&repo.path)).await else { continue };
            for ws in mine {
                let pr = prs
                    .iter()
                    .find(|p| p["headRefName"] == ws.branch.as_str())
                    .cloned()
                    .unwrap_or(Value::Null);
                let changed = self.prs.lock().get(&ws.id) != Some(&pr);
                if changed {
                    self.prs.lock().insert(ws.id.clone(), pr.clone());
                    self.emitter.emit(Event::WorkspacePr { workspace_id: ws.id.clone(), pr });
                }
            }
        }
    }

    // ---- git / review ----

    fn ws_path(&self, workspace_id: &str) -> Result<(Workspace, PathBuf)> {
        let ws = self.store.workspace(workspace_id)?;
        let path = PathBuf::from(&ws.path);
        Ok((ws, path))
    }

    pub async fn changed_files(&self, workspace_id: &str) -> Result<Vec<git::ChangedFile>> {
        let (ws, path) = self.ws_path(workspace_id)?;
        git::changed_files(&path, &ws.base_branch).await
    }

    pub async fn list_files(&self, workspace_id: &str) -> Result<Vec<String>> {
        let (_, path) = self.ws_path(workspace_id)?;
        git::list_files(&path).await
    }

    pub async fn file_diff(&self, workspace_id: &str, file: &str) -> Result<String> {
        let (ws, path) = self.ws_path(workspace_id)?;
        git::file_diff(&path, &ws.base_branch, file).await
    }

    pub async fn revert_file(&self, workspace_id: &str, file: &str) -> Result<()> {
        let (ws, path) = self.ws_path(workspace_id)?;
        git::revert_file(&path, &ws.base_branch, file).await
    }

    pub async fn commit_all(&self, workspace_id: &str, message: &str) -> Result<()> {
        let (_, path) = self.ws_path(workspace_id)?;
        if !git::has_uncommitted(&path).await? {
            bail!("nothing to commit");
        }
        git::commit_all(&path, message).await
    }

    /// Fetch the base branch, unless that happened recently (or `force`).
    async fn fetch_base(&self, ws: &Workspace, path: &Path, force: bool) {
        let key = format!("{}\0{}", ws.repo_id, ws.base_branch);
        let fresh = self.fetched.lock().get(&key).is_some_and(|t| t.elapsed().as_secs() < 60);
        if fresh && !force {
            return;
        }
        let _ = git::git(path, &["fetch", "-q", "origin", &ws.base_branch]).await;
        self.fetched.lock().insert(key, std::time::Instant::now());
    }

    /// Ahead/behind the base branch, and any merge in progress.
    pub async fn sync_status(&self, workspace_id: &str) -> Result<git::SyncStatus> {
        let (ws, path) = self.ws_path(workspace_id)?;
        self.fetch_base(&ws, &path, false).await;
        git::sync_status(&path, &ws.base_branch).await
    }

    /// Merge the latest base branch in. Returns conflicted files, if any.
    pub async fn merge_base_branch(&self, workspace_id: &str) -> Result<Vec<String>> {
        let (ws, path) = self.ws_path(workspace_id)?;
        if self.agents.workspace_busy(workspace_id) {
            bail!("an agent is working in this workspace; wait for it to finish");
        }
        self.fetch_base(&ws, &path, true).await;
        let result = git::merge_base_branch(&path, &ws.base_branch).await;
        self.emitter.emit(Event::WorkspaceStatus { workspace_id: ws.id, status: "dirty".into() });
        result
    }

    pub async fn abort_merge(&self, workspace_id: &str) -> Result<()> {
        let (ws, path) = self.ws_path(workspace_id)?;
        git::abort_merge(&path).await?;
        self.emitter.emit(Event::WorkspaceStatus { workspace_id: ws.id, status: "dirty".into() });
        Ok(())
    }

    pub async fn pr_status(&self, workspace_id: &str) -> Result<Option<Value>> {
        let (ws, path) = self.ws_path(workspace_id)?;
        forge::pr_status(&path, &ws.branch).await
    }

    /// Commit pending changes (using the title as message), push, open a PR.
    pub async fn create_pr(&self, workspace_id: &str, title: &str, body: &str) -> Result<String> {
        let (ws, path) = self.ws_path(workspace_id)?;
        if git::has_uncommitted(&path).await? {
            git::commit_all(&path, title).await?;
        }
        git::push(&path, &ws.branch).await?;
        let url = forge::create_pr(&path, &ws.base_branch, title, body).await?;
        self.refresh_prs();
        Ok(url)
    }

    /// Commit pending changes with a written message (falling back to the
    /// workspace title), then push.
    pub async fn commit_and_push(&self, workspace_id: &str) -> Result<()> {
        let (ws, path) = self.ws_path(workspace_id)?;
        if git::has_uncommitted(&path).await? {
            let summary = git::uncommitted_summary(&path).await?;
            let message = match title::commit_message(&summary).await {
                Some(m) => m,
                None if !ws.title.is_empty() => ws.title.clone(),
                None => "Update".into(),
            };
            git::commit_all(&path, &message).await?;
        }
        self.push(workspace_id).await
    }

    /// A PR title and description for the workspace's changes, written by
    /// Claude Haiku with the user's login.
    pub async fn draft_pr(&self, workspace_id: &str) -> Result<(String, String)> {
        let (ws, path) = self.ws_path(workspace_id)?;
        let mut summary = git::branch_summary(&path, &ws.base_branch).await?;
        if git::has_uncommitted(&path).await? {
            summary = format!("{summary}\n\nNot committed yet:\n{}", git::uncommitted_summary(&path).await?);
        }
        title::pr_text(&summary)
            .await
            .ok_or_else(|| anyhow!("couldn't write a description (this uses Claude Code; is it installed and signed in?)"))
    }

    pub async fn push(&self, workspace_id: &str) -> Result<()> {
        let (ws, path) = self.ws_path(workspace_id)?;
        git::push(&path, &ws.branch).await?;
        self.refresh_prs();
        Ok(())
    }

    pub async fn pr_details(&self, workspace_id: &str) -> Result<Value> {
        let (ws, path) = self.ws_path(workspace_id)?;
        forge::pr_details(&path, &ws.branch).await
    }

    pub async fn merge_pr(&self, workspace_id: &str) -> Result<()> {
        let (ws, path) = self.ws_path(workspace_id)?;
        forge::merge_pr(&path, &ws.branch).await?;
        self.refresh_prs();
        Ok(())
    }

    // ---- terminals ----

    #[allow(clippy::too_many_arguments)]
    pub fn open_terminal(
        &self,
        workspace_id: &str,
        terminal_id: &str,
        cols: u16,
        rows: u16,
        command: Option<&str>,
        on_output: impl Fn(Vec<u8>) + Send + 'static,
        on_exit: impl FnOnce() + Send + 'static,
    ) -> Result<()> {
        let (ws, path) = self.ws_path(workspace_id)?;
        let root = self.store.repo(&ws.repo_id).map(|r| r.path).unwrap_or_default();
        let env = script_env(&root, &ws.name, &ws.path, &ws.id);
        self.terminals.spawn(terminal_id, &path, cols, rows, command, &env, on_output, on_exit)
    }

    /// Terminal outside any workspace (used for agent install/login flows).
    pub fn open_home_terminal(
        &self,
        terminal_id: &str,
        cols: u16,
        rows: u16,
        command: &str,
        on_output: impl Fn(Vec<u8>) + Send + 'static,
        on_exit: impl FnOnce() + Send + 'static,
    ) -> Result<()> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        self.terminals.spawn(terminal_id, &home, cols, rows, Some(command), &[], on_output, on_exit)
    }
}
