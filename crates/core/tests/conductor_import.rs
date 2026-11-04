//! "Import from Conductor" against a fixture install: real git worktrees in a
//! `conductor/workspaces` tree plus a Conductor-shaped SQLite database.
//! Proves the import never changes Conductor's files, points workspaces at
//! the existing worktrees, carries chats over, and doesn't duplicate.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use suneiro_core::{conductor::Paths, git, Core};

/// Every file, directory and symlink below `root` with its content and mtime.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            let meta = std::fs::symlink_metadata(&p).unwrap();
            let mtime = meta.modified().map(|t| format!("{t:?}")).unwrap_or_default();
            let v = if meta.file_type().is_symlink() {
                format!("link {}", std::fs::read_link(&p).unwrap().display())
            } else if meta.is_dir() {
                stack.push(p.clone());
                format!("dir {mtime}")
            } else {
                format!("file {mtime} {:?}", std::fs::read(&p).unwrap())
            };
            out.insert(p, v);
        }
    }
    out
}

async fn git(dir: &Path, args: &[&str]) {
    git::git(dir, args).await.unwrap();
}

const SCHEMA: &str = r#"
CREATE TABLE repos (id TEXT PRIMARY KEY, name TEXT, root_path TEXT, default_branch TEXT DEFAULT 'main',
    setup_script TEXT, run_script TEXT, archive_script TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE workspaces (local_id TEXT PRIMARY KEY, id TEXT, repository_id TEXT, directory_name TEXT,
    secondary_directory_name TEXT, workspace_path TEXT, branch TEXT, state TEXT DEFAULT 'active',
    workspace_name TEXT, pr_title TEXT, intended_target_branch TEXT, initialization_parent_branch TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE sessions (id TEXT PRIMARY KEY, workspace_id TEXT, agent_type TEXT, claude_session_id TEXT,
    title TEXT DEFAULT 'Untitled', model TEXT, is_hidden INTEGER DEFAULT 0, last_user_message_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));
CREATE TABLE session_messages (id TEXT PRIMARY KEY, session_id TEXT, role TEXT, content TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')));
"#;

struct Fixture {
    _tmp: tempfile::TempDir,
    core: Arc<Core>,
    paths: Paths,
    repo: PathBuf,
    tokyo: PathBuf,
    lima: PathBuf,
}

async fn fixture() -> Fixture {
    std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let repo = base.join("projects/app");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "commit.gpgsign", "false"],
        &["config", "user.email", "test@example.com"],
        &["config", "user.name", "Test"],
        &["commit", "--allow-empty", "-qm", "init"],
    ] {
        git(&repo, args).await;
    }
    let root = base.join("conductor");
    let ws_dir = root.join("workspaces/app");
    std::fs::create_dir_all(&ws_dir).unwrap();
    let tokyo = ws_dir.join("tokyo");
    let lima = ws_dir.join("lima");
    git(&repo, &["worktree", "add", "-q", "-b", "fix-login", tokyo.to_str().unwrap()]).await;
    git(&repo, &["worktree", "add", "-q", "-b", "lima", lima.to_str().unwrap()]).await;
    std::fs::write(tokyo.join("wip.txt"), "uncommitted work").unwrap();
    // Conductor's rename alias, a stale worktree and a leftover folder.
    std::os::unix::fs::symlink(&tokyo, ws_dir.join("fix-login-page")).unwrap();
    std::fs::create_dir_all(ws_dir.join("stale")).unwrap();
    std::fs::write(ws_dir.join("stale/.git"), "gitdir: /nonexistent/.git/worktrees/stale\n").unwrap();
    std::fs::create_dir_all(ws_dir.join("empty")).unwrap();

    let db = base.join("app-data/com.conductor.app/conductor.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO repos (id, name, root_path, setup_script, run_script) VALUES ('r1', 'app', ?1, 'pnpm install', 'pnpm dev')",
            [repo.to_str().unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workspaces (local_id, id, repository_id, directory_name, secondary_directory_name, workspace_path, branch, state, workspace_name, intended_target_branch, created_at, updated_at)
             VALUES ('w1', 'w1', 'r1', 'tokyo', 'fix-login-page', ?1, 'fix-login', 'ready', 'Fix login page', 'origin/main', '2026-02-27 06:24:59', '2026-03-01T10:00:00.000Z')",
            [tokyo.to_str().unwrap()],
        )
        .unwrap();
        // An older, archived row that used the same directory.
        conn.execute(
            "INSERT INTO workspaces (local_id, id, repository_id, directory_name, workspace_path, branch, state, workspace_name, updated_at)
             VALUES ('w0', 'w0', 'r1', 'tokyo', ?1, 'old-branch', 'archived', 'Old task', '2026-01-01 00:00:00')",
            [tokyo.to_str().unwrap()],
        )
        .unwrap();
        conn.execute_batch(
            r#"
            INSERT INTO sessions (id, workspace_id, agent_type, title, last_user_message_at, created_at) VALUES
              ('s1', 'w1', 'claude', 'Fix login', '2026-03-01T09:00:00.000Z', '2026-02-27 06:25:00'),
              ('s2', 'w1', 'codex', 'Untitled', '2026-03-01T10:00:00.000Z', '2026-03-01 09:30:00'),
              ('s3', 'w1', 'claude', 'Empty chat', NULL, '2026-03-01 09:40:00'),
              ('s4', 'w1', 'claude', 'Hidden', NULL, '2026-03-01 09:50:00');
            UPDATE sessions SET is_hidden = 1 WHERE id = 's4';
            INSERT INTO session_messages (id, session_id, role, content, created_at) VALUES
              ('m1', 's1', 'user', 'The login page crashes', '2026-02-27 06:25:01'),
              ('m2', 's1', 'assistant', '{"type":"assistant","message":{"id":"a1","content":[{"type":"text","text":"Found the crash."}]}}', '2026-02-27 06:25:05'),
              ('m3', 's1', 'assistant', '{"type":"result","subtype":"success"}', '2026-02-27 06:25:06'),
              ('m4', 's2', 'user', 'Add a regression test', '2026-03-01T10:00:00.000Z'),
              ('m5', 's4', 'user', 'secret hidden chat', '2026-03-01T10:00:00.000Z');
            "#,
        )
        .unwrap();
    }
    assert!(!db.with_extension("db-wal").exists(), "fixture closed cleanly");

    let core = Core::new(&base.join("data"), Arc::new(|_| {})).unwrap();
    let mut settings = core.settings();
    settings.workspaces_root = base.join("suneiro-ws").to_string_lossy().into();
    core.save_settings(&settings).unwrap();
    Fixture { core, paths: Paths { root, db }, repo, tokyo, lima, _tmp: tmp }
}

#[tokio::test]
async fn imports_conductor_worktrees_read_only_and_idempotently() {
    let f = fixture().await;
    let app_data = f.paths.db.parent().unwrap().to_path_buf();
    let before = (snapshot(&f.paths.root), snapshot(&app_data));

    let scan = f.core.conductor_scan_at(f.paths.clone()).await.unwrap();
    assert!(scan.detected);
    assert_eq!(scan.db_error, None);
    let keys: Vec<&str> = scan.workspaces.iter().map(|w| w.key.as_str()).collect();
    assert_eq!(scan.workspaces.len(), 2, "alias, stale and empty folders are skipped: {keys:?}");
    assert_eq!(scan.unavailable.len(), 1, "{:?}", scan.unavailable);
    assert!(scan.unavailable[0].path.ends_with("/stale"));
    assert!(scan.unavailable[0].reason.contains("git worktree repair"));
    let tokyo = scan.workspaces.iter().find(|w| w.name == "tokyo").unwrap();
    assert_eq!(tokyo.path, f.tokyo.to_string_lossy());
    assert_eq!(tokyo.repo_path, f.repo.to_string_lossy());
    assert_eq!(tokyo.repo_name, "app");
    assert_eq!(tokyo.branch, "fix-login");
    assert_eq!(tokyo.title, "Fix login page");
    assert_eq!(tokyo.base_branch.as_deref(), Some("main"));
    assert!(!tokyo.archived_in_conductor, "the active row wins over the archived one");
    assert_eq!(tokyo.chats, 2);
    let lima = scan.workspaces.iter().find(|w| w.name == "lima").unwrap();
    assert_eq!(lima.title, "lima", "no database row: falls back to the branch");
    assert!(lima.last_activity.is_some());
    assert!(scan.workspaces.iter().all(|w| w.imported.is_none()));

    let all: Vec<String> = scan.workspaces.iter().map(|w| w.key.clone()).collect();
    let report = f.core.conductor_import_at(f.paths.clone(), &all, true).await.unwrap();
    assert_eq!(report.imported.len(), 2, "{:?}", report.skipped);
    assert_eq!(report.scripts_for, vec!["app".to_string()]);

    let repos = f.core.store.repos().unwrap();
    assert_eq!(repos.len(), 1, "both grouped under the main checkout");
    let config = f.core.repo_settings(&repos[0].id);
    assert_eq!(config.scripts.setup.as_deref(), Some("pnpm install"));
    assert_eq!(config.scripts.run.as_deref(), Some("pnpm dev"));

    let ws = f.core.store.workspaces().unwrap();
    let t = ws.iter().find(|w| w.name == "tokyo").unwrap();
    assert_eq!(t.path, f.tokyo.to_string_lossy());
    assert_eq!(t.branch, "fix-login");
    assert_eq!(t.status, "ready");
    assert_eq!(t.title, "Fix login page");
    let sessions = f.core.store.sessions(&t.id).unwrap();
    let titles: Vec<&str> = sessions.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Fix login", "Add a regression test"], "empty and hidden chats are skipped");
    assert_eq!(sessions[0].agent_id, "claude");
    assert_eq!(sessions[1].agent_id, "codex");
    assert!(sessions.iter().all(|s| s.acp_session_id.is_none()));
    let events = f.core.store.events(&sessions[0].id).unwrap();
    assert!(events.iter().any(|e| e["type"] == "userMessage" && e["text"] == "The login page crashes"));
    assert!(events.iter().any(|e| e["update"]["content"]["text"] == "Found the crash."));
    let l = ws.iter().find(|w| w.name == "lima").unwrap();
    assert_eq!(f.core.store.sessions(&l.id).unwrap().len(), 1, "a fresh chat when there's no history");

    // Branches and worktrees are untouched.
    let head = git::git(&f.tokyo, &["rev-parse", "--abbrev-ref", "HEAD"]).await.unwrap();
    assert_eq!(head.trim(), "fix-login");
    assert!(f.tokyo.join("wip.txt").exists());

    // Importing again changes nothing.
    let rescan = f.core.conductor_scan_at(f.paths.clone()).await.unwrap();
    assert!(rescan.workspaces.iter().all(|w| w.imported.is_some()));
    let again = f.core.conductor_import_at(f.paths.clone(), &all, true).await.unwrap();
    assert!(again.imported.is_empty());
    assert_eq!(again.skipped.len(), 2);
    assert_eq!(f.core.store.workspaces().unwrap().len(), 2);

    // Archiving an imported workspace only hides it; restoring brings it back.
    f.core.archive_workspace(&t.id).await.unwrap();
    assert!(f.tokyo.join("wip.txt").exists(), "worktree and uncommitted work stay");
    assert_eq!(f.core.store.workspace(&t.id).unwrap().status, "archived");
    let again = f.core.conductor_import_at(f.paths.clone(), &all, true).await.unwrap();
    assert!(again.imported.is_empty(), "archived imports don't come back as duplicates");
    let restored = f.core.restore_workspace(&t.id).await.unwrap();
    assert_eq!(restored.status, "ready");
    assert_eq!(f.core.store.sessions(&t.id).unwrap().len(), 2);

    // Not a single byte or timestamp of Conductor's data changed.
    let after = (snapshot(&f.paths.root), snapshot(&app_data));
    assert_eq!(before.0, after.0, "conductor/ changed");
    assert_eq!(before.1, after.1, "Conductor app data changed (no -wal/-shm files either)");
    let _ = f.lima;
}

#[tokio::test]
async fn works_without_a_database() {
    let f = fixture().await;
    let paths = Paths { root: f.paths.root.clone(), db: f.paths.db.with_file_name("missing.db") };
    let scan = f.core.conductor_scan_at(paths.clone()).await.unwrap();
    assert_eq!(scan.workspaces.len(), 2);
    let tokyo = scan.workspaces.iter().find(|w| w.name == "tokyo").unwrap();
    assert_eq!(tokyo.title, "fix-login");
    let report = f.core.conductor_import_at(paths, std::slice::from_ref(&tokyo.key), true).await.unwrap();
    assert_eq!(report.imported.len(), 1);
    assert!(report.scripts_for.is_empty());
    let unknown = f.core.conductor_import_at(f.paths.clone(), &["/nope".into()], true).await.unwrap();
    assert_eq!(unknown.skipped.len(), 1);
}

#[tokio::test]
async fn skips_worktrees_already_open_in_suneiro() {
    let f = fixture().await;
    // A Suneiro workspace already points at the lima worktree (e.g. added by hand).
    let repo = f.core.add_repo(f.repo.to_str().unwrap()).await.unwrap();
    let ws = suneiro_core::Workspace {
        id: "existing".into(),
        repo_id: repo.id,
        name: "lima".into(),
        branch: "lima".into(),
        base_branch: "main".into(),
        path: f.lima.to_string_lossy().into(),
        status: "ready".into(),
        created_at: 0,
        title: "Mine".into(),
        archived_at: None,
        unread: false,
    };
    f.core.store.add_workspace(&ws).unwrap();
    let scan = f.core.conductor_scan_at(f.paths.clone()).await.unwrap();
    let lima = scan.workspaces.iter().find(|w| w.name == "lima").unwrap();
    assert_eq!(lima.imported.as_deref(), Some("existing"));
    let report = f.core.conductor_import_at(f.paths.clone(), std::slice::from_ref(&lima.key), false).await.unwrap();
    assert!(report.imported.is_empty());
}
