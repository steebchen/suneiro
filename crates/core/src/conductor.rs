//! Read-only import of Conductor (conductor.build) workspaces.
//!
//! Conductor keeps git worktrees in `~/conductor/workspaces/<repo>/<name>`
//! (renamed workspaces are symlinks to the city-named directory) and its
//! metadata in `~/Library/Application Support/com.conductor.app/conductor.db`.
//!
//! Nothing here ever writes to Conductor's data: the database is opened
//! read-only with `immutable=1` (no locks, no `-wal`/`-shm` writes), and
//! worktrees are inspected by reading their `.git`, `HEAD` and `commondir`
//! files instead of running git (even `git status` may rewrite the index).
//! An imported Suneiro workspace points at the existing worktree; no checkout,
//! no branch change.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value};

use crate::events::Event;
use crate::store::Session;
use crate::workspace::Scripts;

/// `workspace_imports.source` for Conductor imports.
pub const SOURCE: &str = "conductor";

/// Most recent chats imported per workspace.
const MAX_CHATS: usize = 20;
/// Most recent transcript events kept per imported chat.
const MAX_EVENTS: usize = 4000;
/// Longest string kept in an imported tool call's input.
const MAX_INPUT_CHARS: usize = 2000;

/// Where Conductor keeps its worktrees and database.
#[derive(Debug, Clone)]
pub struct Paths {
    /// `~/conductor` (contains `workspaces/`).
    pub root: PathBuf,
    /// Conductor's SQLite database.
    pub db: PathBuf,
}

impl Paths {
    /// The default locations for the current user.
    pub fn for_user() -> Option<Self> {
        let home = dirs::home_dir()?;
        let app_data = dirs::data_dir().unwrap_or_else(|| home.join("Library/Application Support"));
        Some(Self {
            root: home.join("conductor"),
            db: app_data.join("com.conductor.app").join("conductor.db"),
        })
    }

    pub fn detected(&self) -> bool {
        self.root.join("workspaces").is_dir() || self.db.is_file()
    }
}

/// A Conductor worktree that can be imported.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConductorWorkspace {
    /// Stable import key: the worktree's canonical path.
    pub key: String,
    pub repo_name: String,
    /// The repository's main checkout (what Suneiro adds as the repo).
    pub repo_path: String,
    /// The worktree.
    pub path: String,
    /// Directory name, e.g. `santiago`.
    pub name: String,
    pub branch: String,
    pub base_branch: Option<String>,
    pub title: String,
    /// Last activity in ms, when known.
    pub last_activity: Option<i64>,
    /// Conductor itself lists the workspace as archived (its worktree is still on disk).
    pub archived_in_conductor: bool,
    /// Chats with at least one message.
    pub chats: usize,
    /// The Suneiro workspace it was already imported as.
    pub imported: Option<String>,
    #[serde(skip)]
    pub conductor_id: Option<String>,
    #[serde(skip)]
    pub created_at: Option<i64>,
}

/// What `scan` found.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub detected: bool,
    pub root: String,
    /// The database couldn't be read; workspaces come from the folders only.
    pub db_error: Option<String>,
    pub workspaces: Vec<ConductorWorkspace>,
    /// Worktree folders that can't be imported as they are, with the reason.
    pub unavailable: Vec<Unavailable>,
    /// Scripts Conductor stores in its database per main checkout path.
    #[serde(skip)]
    pub repo_scripts: HashMap<String, Scripts>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unavailable {
    pub path: String,
    pub reason: String,
}

/// Why a folder with a `.git` file isn't an importable worktree.
pub fn diagnose(dir: &Path) -> String {
    let Ok(pointer) = std::fs::read_to_string(dir.join(".git")) else { return "not a git worktree".into() };
    let Some(git_dir) = pointer.lines().find_map(|l| l.strip_prefix("gitdir:")).map(|g| dir.join(g.trim())) else {
        return "unreadable .git file".into();
    };
    if !git_dir.is_dir() {
        return format!(
            "its repository moved or the worktree was pruned ({} is missing); run `git worktree repair {}` in the repository first",
            git_dir.display(),
            dir.display()
        );
    }
    match std::fs::read_to_string(git_dir.join("HEAD")) {
        Ok(h) if !h.trim().starts_with("ref: refs/heads/") => "detached HEAD (no branch checked out)".into(),
        _ => "worktree of a bare repository".into(),
    }
}

/// A linked git worktree, read from its files.
#[derive(Debug, Clone, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    pub main_checkout: PathBuf,
    pub branch: String,
    git_dir: PathBuf,
}

/// Read a linked worktree without running git. `None` for anything else
/// (plain clones, stale worktrees whose git dir is gone, detached HEADs).
pub fn inspect_worktree(dir: &Path) -> Option<Worktree> {
    let path = dir.canonicalize().ok()?;
    let dot_git = path.join(".git");
    if !dot_git.is_file() {
        return None;
    }
    let pointer = std::fs::read_to_string(&dot_git).ok()?;
    let git_dir = pointer.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let git_dir = path.join(git_dir).canonicalize().ok()?;
    let common = std::fs::read_to_string(git_dir.join("commondir")).ok()?;
    let common = git_dir.join(common.trim()).canonicalize().ok()?;
    // Only worktrees of a normal checkout (`<repo>/.git`), not of bare repos.
    if common.file_name()? != ".git" {
        return None;
    }
    let main_checkout = common.parent()?.to_path_buf();
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let branch = head.trim().strip_prefix("ref: refs/heads/")?.to_string();
    Some(Worktree { path, main_checkout, branch, git_dir })
}

fn mtime_ms(path: &Path) -> Option<i64> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64)
}

/// Conductor timestamps: `2026-02-27 06:24:59` (SQLite, UTC) or
/// `2026-08-10T15:17:11.298Z`. Milliseconds since the epoch.
pub fn parse_ts(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13).unwrap_or(0), num(14, 16).unwrap_or(0), num(17, 19).unwrap_or(0));
    let ms = s
        .get(19..)
        .and_then(|r| r.strip_prefix('.'))
        .map(|r| {
            let digits: String = r.chars().take_while(char::is_ascii_digit).take(3).collect();
            format!("{digits:0<3}").parse::<i64>().unwrap_or(0)
        })
        .unwrap_or(0);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + mi) * 60_000 + sec * 1000 + ms)
}

/// `file:` URI for SQLite, percent-encoding everything but unreserved chars and `/`.
fn sqlite_uri(path: &Path) -> String {
    let mut out = String::from("file:");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out.push_str("?immutable=1");
    out
}

/// Open Conductor's database strictly read-only. `immutable=1` skips locking
/// and the WAL index, so SQLite never creates or writes `-wal`/`-shm` files.
/// (Changes still only in Conductor's WAL aren't visible; it's a few KB.)
pub fn open_db(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        sqlite_uri(path),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.execute_batch("PRAGMA query_only = ON;")?;
    Ok(conn)
}

fn columns(conn: &Connection, table: &str) -> HashSet<String> {
    let Ok(mut stmt) = conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')")) else {
        return HashSet::new();
    };
    stmt.query_map([], |r| r.get::<_, String>(0))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

/// `col` if the table has it, else `NULL`, so older/newer schemas still work.
fn col(cols: &HashSet<String>, name: &str) -> String {
    if cols.contains(name) {
        format!("\"{name}\"")
    } else {
        "NULL".into()
    }
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[derive(Debug, Default)]
struct DbWorkspace {
    id: String,
    repo_id: Option<String>,
    path: Option<String>,
    archived: bool,
    name: Option<String>,
    pr_title: Option<String>,
    base: Option<String>,
    created_at: Option<i64>,
    updated_at: Option<i64>,
}

#[derive(Debug, Default)]
struct DbSession {
    title: Option<String>,
    last: Option<i64>,
    messages: bool,
}

struct Db {
    workspaces: Vec<DbWorkspace>,
    /// Conductor repo id -> (name, root path).
    repos: HashMap<String, (String, Option<String>)>,
    scripts: HashMap<String, Scripts>,
    sessions: HashMap<String, Vec<DbSession>>,
}

fn read_db(conn: &Connection) -> rusqlite::Result<Db> {
    let rc = columns(conn, "repos");
    let mut repos = HashMap::new();
    let mut scripts = HashMap::new();
    {
        let sql = format!(
            "SELECT id, {}, {}, {}, {}, {} FROM repos",
            col(&rc, "name"),
            col(&rc, "root_path"),
            col(&rc, "setup_script"),
            col(&rc, "run_script"),
            col(&rc, "archive_script")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                Scripts { setup: non_empty(r.get(3)?), run: non_empty(r.get(4)?), archive: non_empty(r.get(5)?) },
            ))
        })?;
        for row in rows.flatten() {
            let (id, name, root, s) = row;
            if let Some(root) = &root {
                if s.setup.is_some() || s.run.is_some() || s.archive.is_some() {
                    let key = Path::new(root).canonicalize().map(|p| p.to_string_lossy().to_string()).unwrap_or(root.clone());
                    scripts.insert(key, s);
                }
            }
            repos.insert(id, (name.unwrap_or_default(), root));
        }
    }

    let wc = columns(conn, "workspaces");
    let id = if wc.contains("id") && wc.contains("local_id") {
        "COALESCE(\"id\", \"local_id\")".to_string()
    } else if wc.contains("local_id") {
        "\"local_id\"".into()
    } else {
        "\"id\"".into()
    };
    let sql = format!(
        "SELECT {id}, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM workspaces",
        col(&wc, "repository_id"),
        col(&wc, "workspace_path"),
        col(&wc, "state"),
        col(&wc, "workspace_name"),
        col(&wc, "pr_title"),
        col(&wc, "intended_target_branch"),
        col(&wc, "initialization_parent_branch"),
        col(&wc, "created_at"),
        col(&wc, "updated_at"),
    );
    let mut workspaces = Vec::new();
    {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| {
            let base = non_empty(r.get(6)?).or(non_empty(r.get(7)?));
            Ok(DbWorkspace {
                id: r.get(0)?,
                repo_id: r.get(1)?,
                path: r.get(2)?,
                archived: r.get::<_, Option<String>>(3)?.as_deref() == Some("archived"),
                name: non_empty(r.get(4)?),
                pr_title: non_empty(r.get(5)?),
                base: base.map(|b| b.strip_prefix("origin/").map(str::to_string).unwrap_or(b)),
                created_at: r.get::<_, Option<String>>(8)?.as_deref().and_then(parse_ts),
                updated_at: r.get::<_, Option<String>>(9)?.as_deref().and_then(parse_ts),
            })
        })?;
        workspaces.extend(rows.flatten());
    }

    let mut sessions: HashMap<String, Vec<DbSession>> = HashMap::new();
    let sc = columns(conn, "sessions");
    if sc.contains("workspace_id") {
        let with_messages: HashSet<String> = conn
            .prepare("SELECT DISTINCT session_id FROM session_messages WHERE role = 'user'")
            .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0)).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        let sql = format!(
            "SELECT id, workspace_id, {}, {}, {}, {} FROM sessions",
            col(&sc, "title"),
            col(&sc, "last_user_message_at"),
            col(&sc, "updated_at"),
            col(&sc, "is_hidden")
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<i64>>(5)?,
            ))
        })?;
        for (id, ws, title, last_user, updated, hidden) in rows.flatten() {
            let Some(ws) = ws else { continue };
            if hidden.unwrap_or(0) != 0 {
                continue;
            }
            sessions.entry(ws).or_default().push(DbSession {
                title: non_empty(title).filter(|t| t != "Untitled"),
                last: last_user.as_deref().and_then(parse_ts).or(updated.as_deref().and_then(parse_ts)),
                messages: with_messages.contains(&id),
            });
        }
    }
    Ok(Db { workspaces, repos, scripts, sessions })
}

/// Every importable Conductor worktree, most recent first. Never fails:
/// a missing or unreadable database just means less metadata.
pub fn scan(paths: &Paths) -> Scan {
    let mut scan = Scan {
        detected: paths.detected(),
        root: paths.root.to_string_lossy().to_string(),
        ..Default::default()
    };
    if !scan.detected {
        return scan;
    }
    let db = if paths.db.is_file() {
        match open_db(&paths.db).and_then(|c| read_db(&c)) {
            Ok(db) => Some(db),
            Err(e) => {
                scan.db_error = Some(e.to_string());
                None
            }
        }
    } else {
        None
    };

    // Database rows by canonical worktree path; prefer the active row when
    // several (old, archived ones) used the same directory.
    let mut by_path: HashMap<PathBuf, &DbWorkspace> = HashMap::new();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(db) = &db {
        for w in &db.workspaces {
            let Some(p) = w.path.as_deref().and_then(|p| Path::new(p).canonicalize().ok()) else { continue };
            let replace = match by_path.get(&p) {
                None => true,
                Some(cur) => (cur.archived && !w.archived) || (cur.archived == w.archived && w.updated_at > cur.updated_at),
            };
            if replace {
                by_path.insert(p.clone(), w);
            }
            candidates.push(p);
        }
    }
    if let Ok(repos) = std::fs::read_dir(paths.root.join("workspaces")) {
        for repo in repos.flatten() {
            if let Ok(entries) = std::fs::read_dir(repo.path()) {
                candidates.extend(entries.flatten().map(|e| e.path()));
            }
        }
    }

    let mut seen = HashSet::new();
    let mut broken: Vec<PathBuf> = Vec::new();
    for dir in candidates {
        let Some(wt) = inspect_worktree(&dir) else {
            if let Ok(p) = dir.canonicalize() {
                if p.join(".git").is_file() && !broken.contains(&p) {
                    broken.push(p);
                }
            }
            continue;
        };
        if !seen.insert(wt.path.clone()) {
            continue;
        }
        let row = by_path.get(&wt.path).copied();
        let sessions = row.and_then(|r| db.as_ref()?.sessions.get(&r.id));
        let latest_titled = sessions.and_then(|s| s.iter().filter(|s| s.title.is_some()).max_by_key(|s| s.last).and_then(|s| s.title.clone()));
        let title = row
            .and_then(|r| r.name.clone().or(r.pr_title.clone()))
            .or(latest_titled)
            .unwrap_or_else(|| wt.branch.clone());
        let session_last = sessions.and_then(|s| s.iter().filter_map(|s| s.last).max());
        let last_activity = [session_last, row.and_then(|r| r.updated_at), mtime_ms(&wt.git_dir.join("index")), mtime_ms(&wt.git_dir.join("HEAD"))]
            .into_iter()
            .flatten()
            .max();
        let repo_name = row
            .and_then(|r| db.as_ref()?.repos.get(r.repo_id.as_ref()?))
            .map(|(n, _)| n.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| wt.main_checkout.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_default();
        scan.workspaces.push(ConductorWorkspace {
            key: wt.path.to_string_lossy().to_string(),
            repo_name,
            repo_path: wt.main_checkout.to_string_lossy().to_string(),
            path: wt.path.to_string_lossy().to_string(),
            name: wt.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            branch: wt.branch.clone(),
            base_branch: row.and_then(|r| r.base.clone()),
            title,
            last_activity,
            archived_in_conductor: row.is_some_and(|r| r.archived),
            chats: sessions.map(|s| s.iter().filter(|s| s.messages).count()).unwrap_or(0),
            imported: None,
            conductor_id: row.map(|r| r.id.clone()),
            created_at: row.and_then(|r| r.created_at),
        });
    }
    scan.workspaces.sort_by(|a, b| b.last_activity.cmp(&a.last_activity).then(a.key.cmp(&b.key)));
    broken.sort();
    scan.unavailable = broken
        .into_iter()
        .filter(|p| !seen.contains(p))
        .map(|p| Unavailable { reason: diagnose(&p), path: p.to_string_lossy().to_string() })
        .collect();
    if let Some(db) = db {
        scan.repo_scripts = db.scripts;
    }
    scan
}

/// Result of importing a selection.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: Vec<crate::Workspace>,
    pub skipped: Vec<Skipped>,
    /// Repositories whose Conductor scripts were copied into Suneiro's repo settings.
    pub scripts_for: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub key: String,
    pub reason: String,
}

/// A Conductor chat converted to a Suneiro session plus its transcript events.
pub struct ImportedChat {
    pub session: Session,
    pub events: Vec<Value>,
}

fn agent_id(agent_type: Option<&str>, conductor_session: &str, claude_session: Option<&str>) -> &'static str {
    match agent_type {
        Some("codex") => "codex",
        Some("acp") if conductor_session.contains("opencode") || claude_session.is_some_and(|c| c.contains("opencode")) => "opencode",
        _ => "claude",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max).collect();
    t.push('…');
    t
}

/// Tool input with long strings cut, for display only.
fn trim_input(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(truncate(s, MAX_INPUT_CHARS)),
        Value::Array(a) => Value::Array(a.iter().take(50).map(trim_input).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), trim_input(v))).collect()),
        other => other.clone(),
    }
}

/// `trim_input`, and at most a few KB in total (big multi-file edits).
fn capped_input(v: &Value) -> Value {
    let trimmed = trim_input(v);
    let text = trimmed.to_string();
    if text.len() <= 4 * MAX_INPUT_CHARS {
        return trimmed;
    }
    json!({ "input": truncate(&text, 4 * MAX_INPUT_CHARS) })
}

/// ACP tool kind and a one-line title for a Claude-style `tool_use`.
fn tool_title(name: &str, input: &Value) -> (&'static str, String) {
    let s = |k: &str| input.get(k).and_then(Value::as_str).map(|v| truncate(v.lines().next().unwrap_or(""), 160));
    let lower = name.to_ascii_lowercase();
    let kind = match lower.as_str() {
        "bash" | "shell" | "exec_command" | "local_shell" => "execute",
        "read" | "notebookread" | "ls" => "read",
        "edit" | "multiedit" | "write" | "notebookedit" | "apply_patch" => "edit",
        "grep" | "glob" | "toolsearch" => "search",
        "webfetch" | "websearch" => "fetch",
        "think" => "think",
        _ => "other",
    };
    let detail = s("command")
        .or_else(|| s("file_path"))
        .or_else(|| s("path"))
        .or_else(|| s("pattern"))
        .or_else(|| s("url"))
        .or_else(|| s("query"))
        .or_else(|| s("description"));
    (kind, match detail {
        Some(d) if !d.is_empty() => format!("{name}: {d}"),
        _ => name.to_string(),
    })
}

/// Convert one chat's stored messages into Suneiro transcript events.
/// `rows` are (role, content, created_at) in insertion order. Conductor
/// stores user prompts as text and agent output as Claude Agent SDK stream
/// messages (also for Codex and OpenCode).
pub fn convert_messages(session_id: &str, rows: &[(String, String, Option<i64>)]) -> Vec<Value> {
    let mut events: Vec<Event> = Vec::new();
    let mut open_turn = false;
    let mut last_ts = 0i64;
    let update = |u: Value| Event::SessionUpdate { session_id: session_id.into(), update: u };
    for (role, content, ts) in rows {
        let ts = ts.unwrap_or(last_ts);
        last_ts = ts;
        match role.as_str() {
            "user" => {
                let text = content.trim();
                if text.is_empty() {
                    continue;
                }
                if open_turn {
                    events.push(Event::TurnEnd { session_id: session_id.into(), stop_reason: "end_turn".into(), ts });
                }
                events.push(Event::UserMessage { session_id: session_id.into(), text: text.into(), ts, images: Vec::new() });
                open_turn = true;
            }
            "assistant" => {
                let Ok(msg) = serde_json::from_str::<Value>(content) else { continue };
                match msg["type"].as_str() {
                    Some("assistant") => {
                        let message_id = msg["message"]["id"].as_str().or(msg["uuid"].as_str()).unwrap_or("").to_string();
                        for block in msg["message"]["content"].as_array().into_iter().flatten() {
                            match block["type"].as_str() {
                                Some("text") => {
                                    let text = block["text"].as_str().unwrap_or("");
                                    if text.trim().is_empty() {
                                        continue;
                                    }
                                    let mut u = json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}});
                                    if !message_id.is_empty() {
                                        u["messageId"] = json!(message_id);
                                    }
                                    events.push(update(u));
                                }
                                Some("tool_use") => {
                                    let name = block["name"].as_str().unwrap_or("Tool");
                                    let (kind, title) = tool_title(name, &block["input"]);
                                    events.push(update(json!({
                                        "sessionUpdate": "tool_call",
                                        "toolCallId": block["id"].as_str().unwrap_or(""),
                                        "title": title,
                                        "kind": kind,
                                        "status": "completed",
                                        "rawInput": capped_input(&block["input"]),
                                    })));
                                }
                                _ => {}
                            }
                        }
                    }
                    Some("user") => {
                        for block in msg["message"]["content"].as_array().into_iter().flatten() {
                            if block["type"] == "tool_result" && block["is_error"].as_bool() == Some(true) {
                                events.push(update(json!({
                                    "sessionUpdate": "tool_call_update",
                                    "toolCallId": block["tool_use_id"].as_str().unwrap_or(""),
                                    "status": "failed",
                                })));
                            }
                        }
                    }
                    Some("result") if open_turn => {
                        events.push(Event::TurnEnd { session_id: session_id.into(), stop_reason: "end_turn".into(), ts });
                        open_turn = false;
                    }
                    Some("error") => {
                        let text = msg["error"]["message"].as_str().or(msg["message"].as_str()).or(msg["error"].as_str()).unwrap_or("The agent reported an error");
                        events.push(Event::SessionState { session_id: session_id.into(), state: "error".into(), error: Some(truncate(text, 2000)) });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if open_turn {
        events.push(Event::TurnEnd { session_id: session_id.into(), stop_reason: "end_turn".into(), ts: last_ts });
    }
    let mut values: Vec<Value> = events.into_iter().filter_map(|e| serde_json::to_value(e).ok()).collect();
    if values.len() > MAX_EVENTS {
        // Keep the latest part, starting at a prompt so turns stay whole.
        let mut cut = values.len() - MAX_EVENTS;
        while cut < values.len() && values[cut]["type"] != "userMessage" {
            cut += 1;
        }
        values.drain(..cut);
    }
    let note = "_Imported from Conductor. The agent doesn't see this history, so continuing here starts a new conversation; mention what it needs from above._\n\n";
    values.insert(
        0,
        serde_json::to_value(Event::SessionUpdate {
            session_id: session_id.into(),
            update: json!({"sessionUpdate": "agent_message_chunk", "messageId": "conductor-import-note", "content": {"type": "text", "text": note}}),
        })
        .unwrap_or_default(),
    );
    values
}

/// The workspace's most recent chats with their transcripts (read-only).
pub fn load_chats(db: &Path, conductor_workspace_id: &str, suneiro_workspace_id: &str) -> rusqlite::Result<Vec<ImportedChat>> {
    let conn = open_db(db)?;
    let sc = columns(&conn, "sessions");
    let sql = format!(
        "SELECT id, {}, {}, {}, {}, {}, {} FROM sessions WHERE workspace_id = ?1",
        col(&sc, "title"),
        col(&sc, "agent_type"),
        col(&sc, "claude_session_id"),
        col(&sc, "created_at"),
        col(&sc, "last_user_message_at"),
        col(&sc, "is_hidden"),
    );
    type Row = (String, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<i64>);
    let mut sessions: Vec<Row> = {
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([conductor_workspace_id], |r| -> rusqlite::Result<Row> {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
        })?;
        rows.flatten().filter(|r| r.6.unwrap_or(0) == 0).collect()
    };
    let last = |r: &Row| r.5.as_deref().and_then(parse_ts).or(r.4.as_deref().and_then(parse_ts)).unwrap_or(0);
    sessions.sort_by_key(|r| std::cmp::Reverse(last(r)));

    let mut chats = Vec::new();
    let mut msgs = conn.prepare("SELECT role, content, created_at FROM session_messages WHERE session_id = ?1 ORDER BY rowid")?;
    for row in sessions {
        if chats.len() >= MAX_CHATS {
            break;
        }
        let rows: Vec<(String, String, Option<i64>)> = msgs
            .query_map([&row.0], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(2)?.as_deref().and_then(parse_ts),
                ))
            })?
            .flatten()
            .collect();
        let first_prompt = rows.iter().find(|r| r.0 == "user" && !r.1.trim().is_empty()).map(|r| r.1.clone());
        let Some(first_prompt) = first_prompt else { continue };
        let id = uuid::Uuid::new_v4().to_string();
        let title = non_empty(row.1.clone())
            .filter(|t| t != "Untitled")
            .unwrap_or_else(|| crate::title::quick_title(&first_prompt));
        chats.push(ImportedChat {
            events: convert_messages(&id, &rows),
            session: Session {
                id,
                workspace_id: suneiro_workspace_id.into(),
                agent_id: agent_id(row.2.as_deref(), &row.0, row.3.as_deref()).into(),
                acp_session_id: None,
                title,
                created_at: row.4.as_deref().and_then(parse_ts).unwrap_or_else(|| last(&row)),
                model: None,
                effort: None,
                parent_session_id: None,
            },
        });
    }
    // Oldest first, like tabs created over time; the latest ends up selected.
    chats.sort_by_key(|c| c.session.created_at);
    Ok(chats)
}

#[cfg(test)]
#[path = "conductor_tests.rs"]
mod tests;
