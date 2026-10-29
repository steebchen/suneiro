use std::path::Path;
use std::sync::mpsc;

use anyhow::Result;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub default_branch: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub branch: String,
    pub base_branch: String,
    pub path: String,
    pub status: String,
    pub created_at: i64,
    /// Short summary of the task, generated from the first prompt.
    pub title: String,
    pub archived_at: Option<i64>,
    /// Manually marked unread; cleared when the workspace is opened.
    pub unread: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub workspace_id: String,
    pub agent_id: String,
    pub acp_session_id: Option<String>,
    pub title: String,
    pub created_at: i64,
    /// Last known model/effort, so the UI can show them before the agent runs.
    pub model: Option<String>,
    pub effort: Option<String>,
    pub parent_session_id: Option<String>,
}

/// One agent turn's token usage and (if the agent reports it) cost.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    pub session_id: String,
    pub workspace_id: String,
    pub repo_id: String,
    pub agent: String,
    pub model: String,
    pub ts: i64,
    /// Uncached input tokens.
    pub input_tokens: i64,
    /// Input tokens read from (or written to) the prompt cache.
    pub cached_tokens: i64,
    /// Output tokens, including reasoning.
    pub output_tokens: i64,
    pub cost_usd: Option<f64>,
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS repos (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    default_branch TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repos(id),
    name TEXT NOT NULL,
    branch TEXT NOT NULL,
    base_branch TEXT NOT NULL,
    path TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    archived_at INTEGER,
    unread INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    agent_id TEXT NOT NULL,
    acp_session_id TEXT,
    title TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    model TEXT,
    effort TEXT
);
CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    data TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS events_session ON events(session_id, seq);
CREATE TABLE IF NOT EXISTS usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    agent TEXT NOT NULL,
    model TEXT NOT NULL,
    ts INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    cached_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    -- As reported by the agent; NULL when it doesn't report cost (estimated from tokens).
    cost_usd REAL
);
CREATE INDEX IF NOT EXISTS usage_ts ON usage(ts);
CREATE INDEX IF NOT EXISTS usage_session ON usage(session_id);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

pub struct Store { pub(crate) conn: Mutex<Connection>, writer: mpsc::Sender<(String, Value)>, }

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(crate::supervision::SCHEMA)?;
        migrate(&conn)?;
        let writer = spawn_event_writer(Connection::open(path)?);
        Ok(Self { conn: Mutex::new(conn), writer })
    }

    pub fn add_repo(&self, repo: &Repo) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO repos (id, name, path, default_branch, created_at, position)
             VALUES (?1, ?2, ?3, ?4, ?5, (SELECT COALESCE(MAX(position), 0) + 1 FROM repos))",
            params![repo.id, repo.name, repo.path, repo.default_branch, now()],
        )?;
        Ok(())
    }

    pub fn repo_by_path(&self, path: &str) -> Result<Option<Repo>> {
        Ok(self.conn.lock().query_row("SELECT id, name, path, default_branch FROM repos WHERE path = ?1", [path], row_to_repo).optional()?)
    }

    pub fn repo(&self, id: &str) -> Result<Repo> {
        Ok(self.conn.lock().query_row("SELECT id, name, path, default_branch FROM repos WHERE id = ?1", [id], row_to_repo)?)
    }

    pub fn repos(&self) -> Result<Vec<Repo>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT id, name, path, default_branch FROM repos ORDER BY position, name")?;
        let rows = stmt.query_map([], row_to_repo)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Store the sidebar order of repos; `ids` lists them first to last.
    pub fn reorder_repos(&self, ids: &[String]) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute("UPDATE repos SET position = ?1 WHERE id = ?2", params![i as i64 + 1, id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_repo(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM events WHERE session_id IN (SELECT s.id FROM sessions s JOIN workspaces w ON s.workspace_id = w.id WHERE w.repo_id = ?1)",
            [id],
        )?;
        conn.execute("DELETE FROM sessions WHERE workspace_id IN (SELECT id FROM workspaces WHERE repo_id = ?1)", [id])?;
        conn.execute("DELETE FROM workspaces WHERE repo_id = ?1", [id])?;
        conn.execute("DELETE FROM repos WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn add_usage(&self, u: &UsageRecord) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO usage (session_id, workspace_id, repo_id, agent, model, ts, input_tokens, cached_tokens, output_tokens, cost_usd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![u.session_id, u.workspace_id, u.repo_id, u.agent, u.model, u.ts, u.input_tokens, u.cached_tokens, u.output_tokens, u.cost_usd],
        )?;
        Ok(())
    }

    pub fn usage_for_session(&self, session_id: &str) -> Result<Vec<UsageRecord>> {
        Ok(self.usage_since(0)?.into_iter().filter(|u| u.session_id == session_id).collect())
    }

    /// All usage rows since `since` (ms), oldest first.
    pub fn usage_since(&self, since: i64) -> Result<Vec<UsageRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT session_id, workspace_id, repo_id, agent, model, ts, input_tokens, cached_tokens, output_tokens, cost_usd
             FROM usage WHERE ts >= ?1 ORDER BY ts",
        )?;
        let rows = stmt.query_map([since], |r| {
            Ok(UsageRecord {
                session_id: r.get(0)?,
                workspace_id: r.get(1)?,
                repo_id: r.get(2)?,
                agent: r.get(3)?,
                model: r.get(4)?,
                ts: r.get(5)?,
                input_tokens: r.get(6)?,
                cached_tokens: r.get(7)?,
                output_tokens: r.get(8)?,
                cost_usd: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.lock().query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .lock()
            .execute("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])?;
        Ok(())
    }

    /// Atomically consume an unchanged setting (e.g. a single-use approved preview).
    pub fn consume_setting(&self, key: &str, expected: &str) -> Result<bool> {
        Ok(self.conn.lock().execute("DELETE FROM settings WHERE key = ?1 AND value = ?2", params![key, expected])? == 1)
    }

    pub fn add_workspace(&self, ws: &Workspace) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO workspaces (id, repo_id, name, branch, base_branch, path, status, created_at, title) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![ws.id, ws.repo_id, ws.name, ws.branch, ws.base_branch, ws.path, ws.status, ws.created_at, ws.title],
        )?;
        Ok(())
    }

    pub fn set_workspace_status(&self, id: &str, status: &str) -> Result<()> {
        self.conn.lock().execute("UPDATE workspaces SET status = ?2 WHERE id = ?1", params![id, status])?;
        Ok(())
    }

    pub fn set_workspace_branch(&self, id: &str, branch: &str) -> Result<()> {
        self.conn.lock().execute("UPDATE workspaces SET branch = ?2 WHERE id = ?1", params![id, branch])?;
        Ok(())
    }

    pub fn set_workspace_title(&self, id: &str, title: &str) -> Result<()> {
        self.conn.lock().execute("UPDATE workspaces SET title = ?2 WHERE id = ?1", params![id, title])?;
        Ok(())
    }

    pub fn workspace(&self, id: &str) -> Result<Workspace> {
        Ok(self.conn.lock().query_row(
            "SELECT id, repo_id, name, branch, base_branch, path, status, created_at, title, archived_at, unread FROM workspaces WHERE id = ?1",
            [id],
            row_to_workspace,
        )?)
    }

    pub fn workspaces(&self) -> Result<Vec<Workspace>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, name, branch, base_branch, path, status, created_at, title, archived_at, unread FROM workspaces WHERE status != 'archived' ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], row_to_workspace)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Every workspace including archived ones, most recent activity first.
    pub fn all_workspaces(&self) -> Result<Vec<Workspace>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, repo_id, name, branch, base_branch, path, status, created_at, title, archived_at, unread FROM workspaces ORDER BY COALESCE(archived_at, created_at) DESC",
        )?;
        let rows = stmt.query_map([], row_to_workspace)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_workspace_unread(&self, id: &str, unread: bool) -> Result<()> {
        self.conn.lock().execute("UPDATE workspaces SET unread = ?2 WHERE id = ?1", params![id, unread])?;
        Ok(())
    }

    pub fn set_archived_at(&self, id: &str, at: Option<i64>) -> Result<()> {
        self.conn.lock().execute("UPDATE workspaces SET archived_at = ?2 WHERE id = ?1", params![id, at])?;
        Ok(())
    }

    pub fn workspace_names(&self, repo_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT name FROM workspaces WHERE repo_id = ?1")?;
        let rows = stmt.query_map([repo_id], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn add_session(&self, s: &Session) -> Result<()> {
        self.add_session_with_depth(s, 8)
    }

    pub fn add_session_with_depth(&self, s: &Session, max_depth: u8) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        validate_parent(&tx, &s.id, &s.workspace_id, s.parent_session_id.as_deref(), max_depth)?;
        tx.execute(
            "INSERT INTO sessions (id, workspace_id, agent_id, acp_session_id, title, created_at, model, effort, parent_session_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![s.id, s.workspace_id, s.agent_id, s.acp_session_id, s.title, s.created_at, s.model, s.effort, s.parent_session_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn detach_session(&self, id: &str) -> Result<()> {
        let changed = self.conn.lock().execute("UPDATE sessions SET parent_session_id = NULL WHERE id = ?1", [id])?;
        anyhow::ensure!(changed == 1, "chat does not exist");
        Ok(())
    }

    pub fn session(&self, id: &str) -> Result<Session> {
        Ok(self.conn.lock().query_row(
            "SELECT id, workspace_id, agent_id, acp_session_id, title, created_at, model, effort, parent_session_id FROM sessions WHERE id = ?1",
            [id],
            row_to_session,
        )?)
    }

    pub fn sessions(&self, workspace_id: &str) -> Result<Vec<Session>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, agent_id, acp_session_id, title, created_at, model, effort, parent_session_id FROM sessions WHERE workspace_id = ?1 ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([workspace_id], row_to_session)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_acp_session_id(&self, id: &str, acp_id: &str) -> Result<()> {
        self.conn.lock().execute("UPDATE sessions SET acp_session_id = ?2 WHERE id = ?1", params![id, acp_id])?;
        Ok(())
    }

    pub fn set_session_model(&self, id: &str, model: Option<&str>, effort: Option<&str>) -> Result<()> {
        self.conn.lock().execute("UPDATE sessions SET model = COALESCE(?2, model), effort = COALESCE(?3, effort) WHERE id = ?1", params![id, model, effort])?;
        Ok(())
    }

    pub fn set_session_title(&self, id: &str, title: &str) -> Result<()> {
        self.conn.lock().execute("UPDATE sessions SET title = ?2 WHERE id = ?1", params![id, title])?;
        Ok(())
    }

    pub fn delete_session(&self, id: &str) -> Result<Vec<String>> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let count: i64 =
            tx.query_row("SELECT COUNT(*) FROM sessions WHERE workspace_id = (SELECT workspace_id FROM sessions WHERE id = ?1)", [id], |r| r.get(0))?;
        anyhow::ensure!(count > 1, "a workspace must keep at least one chat");
        let children = {
            let mut stmt = tx.prepare("SELECT id FROM sessions WHERE parent_session_id = ?1")?;
            {
                let rows = stmt.query_map([id], |r| r.get::<_, String>(0))?;
                rows.collect::<Result<Vec<_>, _>>()?
            }
        };
        tx.execute("UPDATE sessions SET parent_session_id = NULL WHERE parent_session_id = ?1", [id])?;
        tx.execute("DELETE FROM events WHERE session_id = ?1", [id])?;
        tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(children)
    }

    pub fn subtree_session_ids(&self, root: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("WITH RECURSIVE tree(id) AS (SELECT id FROM sessions WHERE id = ?1 UNION SELECT s.id FROM sessions s JOIN tree t ON s.parent_session_id = t.id) SELECT id FROM tree")?;
        {
            let rows = stmt.query_map([root], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        }
    }

    /// Queue an event for persistence. Writes are batched on a background thread.
    pub fn append_event(&self, session_id: &str, event: Value) {
        let _ = self.writer.send((session_id.to_string(), event));
    }

    pub fn events(&self, session_id: &str) -> Result<Vec<Value>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT data FROM events WHERE session_id = ?1 ORDER BY seq")?;
        let rows = stmt.query_map([session_id], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            if let Ok(v) = serde_json::from_str(&row?) {
                out.push(v);
            }
        }
        Ok(out)
    }
}

/// Additive migrations for databases created by older versions.
fn migrate(conn: &Connection) -> Result<()> {
    let add_column = |table: &str, column: &str, ddl: &str| -> Result<()> {
        let exists = conn.prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?.exists([column])?;
        if !exists {
            conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {ddl}"), [])?;
        }
        Ok(())
    };
    add_column("workspaces", "title", "TEXT NOT NULL DEFAULT ''")?;
    add_column("workspaces", "archived_at", "INTEGER")?;
    add_column("workspaces", "unread", "INTEGER NOT NULL DEFAULT 0")?;
    add_column("sessions", "model", "TEXT")?;
    add_column("sessions", "effort", "TEXT")?;
    add_column("sessions", "parent_session_id", "TEXT REFERENCES sessions(id) ON DELETE SET NULL")?;
    conn.execute_batch("CREATE INDEX IF NOT EXISTS sessions_parent ON sessions(parent_session_id);")?;
    // 0 for repos added before ordering existed, which keeps them sorted by name.
    add_column("repos", "position", "INTEGER NOT NULL DEFAULT 0")?;
    Ok(())
}

/// Background writer: drains the queue, merges consecutive text chunks of the
/// same message, and commits each batch in a single transaction.
fn spawn_event_writer(mut conn: Connection) -> mpsc::Sender<(String, Value)> {
    let (tx, rx) = mpsc::channel::<(String, Value)>();
    std::thread::Builder::new()
        .name("suneiro-event-writer".into())
        .spawn(move || {
            while let Ok(first) = rx.recv() {
                std::thread::sleep(std::time::Duration::from_millis(50));
                let mut batch = vec![first];
                batch.extend(rx.try_iter());
                let batch = coalesce_chunks(batch);
                let Ok(tx) = conn.transaction() else { continue };
                {
                    let Ok(mut stmt) = tx.prepare_cached("INSERT INTO events (session_id, data) VALUES (?1, ?2)") else {
                        continue;
                    };
                    for (sid, v) in &batch {
                        let _ = stmt.execute(params![sid, v.to_string()]);
                    }
                }
                let _ = tx.commit();
            }
        })
        .expect("spawn event writer");
    tx
}

fn chunk_key(v: &Value) -> Option<(&str, Option<&str>)> {
    let update = v.get("update")?;
    let kind = update.get("sessionUpdate")?.as_str()?;
    if !matches!(kind, "agent_message_chunk" | "agent_thought_chunk") {
        return None;
    }
    if update.pointer("/content/type")?.as_str()? != "text" {
        return None;
    }
    Some((kind, update.get("messageId").and_then(|m| m.as_str())))
}

fn coalesce_chunks(batch: Vec<(String, Value)>) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::with_capacity(batch.len());
    for (sid, v) in batch {
        if let Some((prev_sid, prev)) = out.last_mut() {
            if *prev_sid == sid && chunk_key(prev).is_some() && chunk_key(prev) == chunk_key(&v) {
                let add = v.pointer("/update/content/text").and_then(|t| t.as_str()).unwrap_or("");
                if let Some(Value::String(text)) = prev.pointer_mut("/update/content/text") {
                    text.push_str(add);
                    continue;
                }
            }
        }
        out.push((sid, v));
    }
    out
}

fn row_to_repo(r: &rusqlite::Row) -> rusqlite::Result<Repo> {
    Ok(Repo { id: r.get(0)?, name: r.get(1)?, path: r.get(2)?, default_branch: r.get(3)? })
}

fn row_to_workspace(r: &rusqlite::Row) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: r.get(0)?,
        repo_id: r.get(1)?,
        name: r.get(2)?,
        branch: r.get(3)?,
        base_branch: r.get(4)?,
        path: r.get(5)?,
        status: r.get(6)?,
        created_at: r.get(7)?,
        title: r.get(8)?,
        archived_at: r.get(9)?,
        unread: r.get(10)?,
    })
}

fn validate_parent(conn: &Connection, id: &str, workspace_id: &str, parent: Option<&str>, max_depth: u8) -> Result<()> {
    anyhow::ensure!((1..=8).contains(&max_depth), "maximum tab depth must be between 1 and 8");
    let (repo, status): (String, String) =
        conn.query_row("SELECT repo_id, status FROM workspaces WHERE id = ?1", [workspace_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    anyhow::ensure!(status != "archived", "cannot create a chat in an archived workspace");
    let mut next = parent.map(str::to_owned);
    let mut seen = std::collections::HashSet::from([id.to_string()]);
    let mut depth = 0;
    while let Some(parent_id) = next {
        anyhow::ensure!(seen.insert(parent_id.clone()), "tab hierarchy cannot contain a cycle");
        depth += 1;
        anyhow::ensure!(depth <= max_depth, "maximum tab depth ({max_depth}) exceeded");
        let (parent_repo, parent_status, ancestor): (String, String, Option<String>) = conn
            .query_row(
                "SELECT w.repo_id, w.status, s.parent_session_id FROM sessions s JOIN workspaces w ON w.id = s.workspace_id WHERE s.id = ?1",
                [&parent_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| anyhow::anyhow!("parent chat does not exist"))?;
        anyhow::ensure!(repo == parent_repo, "parent and child chats must belong to the same repository");
        anyhow::ensure!(parent_status != "archived", "parent chat belongs to an archived workspace");
        next = ancestor;
    }
    Ok(())
}

fn row_to_session(r: &rusqlite::Row) -> rusqlite::Result<Session> {
    Ok(Session {
        id: r.get(0)?,
        workspace_id: r.get(1)?,
        agent_id: r.get(2)?,
        acp_session_id: r.get(3)?,
        title: r.get(4)?,
        created_at: r.get(5)?,
        model: r.get(6)?,
        effort: r.get(7)?,
        parent_session_id: r.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chunk(text: &str, id: &str) -> Value {
        json!({"type": "sessionUpdate", "update": {"sessionUpdate": "agent_message_chunk", "messageId": id, "content": {"type": "text", "text": text}}})
    }

    #[test]
    fn coalesces_consecutive_chunks_of_same_message() {
        let out = coalesce_chunks(vec![("s".into(), chunk("hel", "m1")), ("s".into(), chunk("lo", "m1")), ("s".into(), chunk("x", "m2"))]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].1.pointer("/update/content/text").unwrap(), "hello");
    }

    #[test]
    fn persists_and_reads_events() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db.sqlite")).unwrap();
        store.append_event("s1", chunk("a", "m"));
        store.append_event("s1", chunk("b", "m"));
        std::thread::sleep(std::time::Duration::from_millis(300));
        let events = store.events("s1").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].pointer("/update/content/text").unwrap(), "ab");
    }

    #[test]
    fn repos_keep_their_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db.sqlite")).unwrap();
        let names = |s: &Store| s.repos().unwrap().into_iter().map(|r| r.name).collect::<Vec<_>>();
        for name in ["b", "a", "c"] {
            store.add_repo(&Repo { id: name.into(), name: name.into(), path: format!("/{name}"), default_branch: "main".into() }).unwrap();
        }
        assert_eq!(names(&store), ["b", "a", "c"]);
        store.reorder_repos(&["c".into(), "b".into(), "a".into()]).unwrap();
        assert_eq!(names(&store), ["c", "b", "a"]);
        store.add_repo(&Repo { id: "d".into(), name: "d".into(), path: "/d".into(), default_branch: "main".into() }).unwrap();
        assert_eq!(names(&store), ["c", "b", "a", "d"]);
    }
}
