//! Durable attention inbox and repository admission controls. Costs are observations,
//! not reservations: a stop threshold prevents new turns, never promises a dollar cap.
#[cfg(test)]
#[path = "supervision_tests.rs"]
mod tests;
use crate::{
    events::Event,
    store::{now, Store},
    usage::PricedUsage,
};
use anyhow::{bail, Result};
use parking_lot::Mutex;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS supervision_inbox (
 id TEXT PRIMARY KEY, session_id TEXT NOT NULL, workspace_id TEXT NOT NULL,
 repo_id TEXT NOT NULL, kind TEXT NOT NULL, message TEXT NOT NULL,
 created_at INTEGER NOT NULL, resolved INTEGER NOT NULL DEFAULT 0,
 notified INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS supervision_inbox_open ON supervision_inbox(resolved, created_at);
CREATE TABLE IF NOT EXISTS supervision_turns (session_id TEXT PRIMARY KEY, turn INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS supervision_notifications (id TEXT PRIMARY KEY);
"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct RepoControls {
    pub max_concurrent: usize,
    pub paused: bool,
    pub warning_usd: Option<f64>,
    pub stop_usd: Option<f64>,
    pub spend_since: i64,
}
impl Default for RepoControls {
    fn default() -> Self {
        Self {
            max_concurrent: 4,
            paused: false,
            warning_usd: None,
            stop_usd: None,
            spend_since: 0,
        }
    }
}
impl RepoControls {
    pub fn validate(&self) -> Result<()> {
        if !(1..=64).contains(&self.max_concurrent) {
            bail!("concurrency must be between 1 and 64");
        }
        for value in [self.warning_usd, self.stop_usd].into_iter().flatten() {
            if !value.is_finite() || value <= 0.0 {
                bail!("spend thresholds must be positive finite amounts");
            }
        }
        if self.spend_since < 0 {
            bail!("invalid spend start");
        }
        if let (Some(warn), Some(stop)) = (self.warning_usd, self.stop_usd) {
            if warn > stop {
                bail!("warning must not exceed stop threshold");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    pub id: String,
    pub session_id: String,
    pub workspace_id: String,
    pub repo_id: String,
    pub kind: String,
    pub message: String,
    pub created_at: i64,
    pub resolved: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUsage {
    pub sessions: usize,
    pub running: usize,
    pub turns: usize,
    pub input_tokens: i64,
    pub cached_tokens: i64,
    pub output_tokens: i64,
    pub reported_usd: f64,
    pub estimated_usd: f64,
    pub unknown_turns: usize,
}
impl ResourceUsage {
    pub fn sum(ids: &HashSet<String>, rows: Vec<PricedUsage>) -> Self {
        let mut total = Self {
            sessions: ids.len(),
            ..Self::default()
        };
        for u in rows.into_iter().filter(|u| ids.contains(&u.record.session_id)) {
            total.turns += 1;
            total.input_tokens += u.record.input_tokens;
            total.cached_tokens += u.record.cached_tokens;
            total.output_tokens += u.record.output_tokens;
            match u.cost {
                Some(cost) if u.estimated => total.estimated_usd += cost,
                Some(cost) => total.reported_usd += cost,
                None => total.unknown_turns += 1,
            }
        }
        total
    }
}

type Active = Arc<Mutex<HashMap<String, String>>>;
#[derive(Default)]
pub struct Admissions {
    active: Active,
}
#[derive(Debug)]
pub struct Permit {
    active: Active,
    session_id: String,
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.active.lock().remove(&self.session_id);
    }
}
impl Admissions {
    /// The same lock covers duplicate detection, policy/spend reads and insertion.
    /// No hidden queue: rejected turns are never persisted or started later.
    pub fn admit(&self, store: &Store, session_id: &str, repo_id: &str) -> Result<Permit> {
        let mut active = self.active.lock();
        if active.contains_key(session_id) {
            bail!("agent is still working");
        }
        let controls = store.repo_controls(repo_id)?;
        if controls.paused {
            bail!("repository scheduling is paused");
        }
        if active.values().filter(|r| r.as_str() == repo_id).count() >= controls.max_concurrent {
            bail!("repository concurrency limit reached; retry when a turn finishes");
        }
        if let Some(stop) = controls.stop_usd {
            let pricing = crate::usage::load_pricing(store);
            let spent: f64 = store
                .usage_since(controls.spend_since)?
                .into_iter()
                .filter(|u| u.repo_id == repo_id)
                .filter_map(|u| crate::usage::price(u, &pricing).cost)
                .sum();
            if spent >= stop {
                bail!("observed spend stop threshold reached; change repository controls to continue (unknown costs may be missing)");
            }
        }
        store.conn.lock().execute(
            "INSERT INTO supervision_turns VALUES (?1,1) ON CONFLICT(session_id) DO UPDATE SET turn=turn+1",
            [session_id],
        )?;
        active.insert(session_id.into(), repo_id.into());
        Ok(Permit {
            active: self.active.clone(),
            session_id: session_id.into(),
        })
    }
    pub fn running(&self, ids: &HashSet<String>) -> usize {
        self.active.lock().keys().filter(|id| ids.contains(*id)).count()
    }
}

impl Store {
    pub fn repo_controls(&self, repo_id: &str) -> Result<RepoControls> {
        self.repo(repo_id)?;
        Ok(self
            .setting(&format!("supervision:{repo_id}"))?
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default())
    }
    pub fn save_repo_controls(&self, repo_id: &str, controls: &RepoControls) -> Result<()> {
        self.repo(repo_id)?;
        controls.validate()?;
        self.set_setting(&format!("supervision:{repo_id}"), &serde_json::to_string(controls)?)
    }
    pub fn inbox(&self) -> Result<Vec<InboxItem>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT id,session_id,workspace_id,repo_id,kind,message,created_at,resolved FROM supervision_inbox WHERE resolved=0 ORDER BY created_at DESC,id")?;
        let rows = stmt.query_map([], |r| {
            Ok(InboxItem {
                id: r.get(0)?,
                session_id: r.get(1)?,
                workspace_id: r.get(2)?,
                repo_id: r.get(3)?,
                kind: r.get(4)?,
                message: r.get(5)?,
                created_at: r.get(6)?,
                resolved: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn resolve_inbox(&self, id: &str) -> Result<bool> {
        Ok(self.conn.lock().execute(
            "UPDATE supervision_inbox SET resolved=1 WHERE id=?1 AND resolved=0",
            [id],
        )? > 0)
    }
    /// Transactional claim before delivery provides at-most-once desktop notifications,
    /// including after reload/restart. OS delivery itself is necessarily best effort.
    pub fn claim_notification(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn
            .lock()
            .execute("INSERT OR IGNORE INTO supervision_notifications (id) VALUES (?1)", [id])?
            > 0)
    }
    pub fn push_inbox(&self, session_id: &str, id: &str, kind: &str, message: &str) -> Result<Option<InboxItem>> {
        let session = self.session(session_id)?;
        let ws = self.workspace(&session.workspace_id)?;
        let item = InboxItem {
            id: id.into(),
            session_id: session_id.into(),
            workspace_id: ws.id,
            repo_id: ws.repo_id,
            kind: kind.into(),
            message: message.chars().take(1000).collect(),
            created_at: now(),
            resolved: false,
        };
        let n = self.conn.lock().execute("INSERT OR IGNORE INTO supervision_inbox (id,session_id,workspace_id,repo_id,kind,message,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![item.id,item.session_id,item.workspace_id,item.repo_id,item.kind,item.message,item.created_at])?;
        Ok((n > 0).then_some(item))
    }
    pub fn observe_supervision(&self, event: &Event) -> Result<Vec<Event>> {
        let mut changes = Vec::new();
        let terminal = match event {
            Event::TurnEnd { session_id, .. } | Event::SessionRemoved { session_id, .. } => Some(session_id),
            Event::SessionState { session_id, state, .. } if state == "error" || state == "disconnected" => {
                Some(session_id)
            }
            _ => None,
        };
        if let Some(session_id) = terminal {
            for item in self.inbox()?.into_iter().filter(|i| {
                i.session_id == *session_id
                    && (matches!(event, Event::SessionRemoved { .. })
                        || matches!(i.kind.as_str(), "question" | "permission"))
            }) {
                if self.resolve_inbox(&item.id)? {
                    changes.push(Event::InboxResolved { id: item.id });
                }
            }
        }
        let pending = match event {
            Event::UserMessage { session_id, .. } => {
                for item in self
                    .inbox()?
                    .into_iter()
                    .filter(|i| i.session_id == *session_id && i.kind == "failure")
                {
                    if self.resolve_inbox(&item.id)? {
                        changes.push(Event::InboxResolved { id: item.id });
                    }
                }
                None
            }
            Event::Question {
                session_id,
                request_id,
                message,
                ..
            } => Some((
                session_id,
                self.request_inbox_id("question", session_id, request_id)?,
                "question",
                message.as_str(),
            )),
            Event::PermissionRequest {
                session_id, request_id, ..
            } => Some((
                session_id,
                self.request_inbox_id("permission", session_id, request_id)?,
                "permission",
                "Permission required",
            )),
            Event::SessionState {
                session_id,
                state,
                error: Some(error),
            } if state == "error" => Some((
                session_id,
                self.request_inbox_id("failure", session_id, "")?,
                "failure",
                error.as_str(),
            )),
            Event::QuestionResolved { session_id, request_id } => {
                let id = self.request_inbox_id("question", session_id, request_id)?;
                if self.resolve_inbox(&id)? {
                    changes.push(Event::InboxResolved { id });
                }
                None
            }
            Event::PermissionResolved { session_id, request_id } => {
                let id = self.request_inbox_id("permission", session_id, request_id)?;
                if self.resolve_inbox(&id)? {
                    changes.push(Event::InboxResolved { id });
                }
                None
            }
            Event::DelegationChanged { task, .. }
                if matches!(task.state.as_str(), "completed" | "failed" | "cancelled") =>
            {
                if let Some(report) = task.report.as_deref() {
                    let session_id = task.session_id.as_deref()
                        .filter(|id| self.session(id).is_ok())
                        .unwrap_or(&task.caller_session_id);
                    if self.session(session_id).is_ok() {
                        if let Some(item) = self.push_inbox(session_id, &format!("delegation:{}", task.id), "delegation", report)? {
                            changes.push(Event::InboxUpdated { item });
                        }
                    }
                }
                None
            }
            Event::Usage { session_id, usage } => {
                let controls = self.repo_controls(&usage.record.repo_id)?;
                if let Some(warn) = controls.warning_usd {
                    let pricing = crate::usage::load_pricing(self);
                    let spent: f64 = self
                        .usage_since(controls.spend_since)?
                        .into_iter()
                        .filter(|u| u.repo_id == usage.record.repo_id)
                        .filter_map(|u| crate::usage::price(u, &pricing).cost)
                        .sum();
                    if spent >= warn {
                        let id = format!("spend:{}:{}:{warn}", usage.record.repo_id, controls.spend_since);
                        if let Some(item) = self.push_inbox(session_id, &id, "spend", "Observed spend warning reached. Costs can be estimated or unknown; running turns may continue spending.")? {
                        changes.push(Event::InboxUpdated { item });
                    }
                    }
                }
                None
            }
            _ => None,
        };
        if let Some((session_id, id, kind, message)) = pending {
            if let Some(item) = self.push_inbox(session_id, &id, kind, message)? {
                changes.push(Event::InboxUpdated { item });
            }
        }
        Ok(changes)
    }
    fn request_inbox_id(&self, kind: &str, session_id: &str, request_id: &str) -> Result<String> {
        let turn: i64 = self
            .conn
            .lock()
            .query_row(
                "SELECT turn FROM supervision_turns WHERE session_id=?1",
                [session_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok(format!("{kind}:{session_id}:{turn}:{request_id}"))
    }
    pub fn recover_inbox(&self) -> Result<()> {
        // Requests belong to the old process and cannot be answered after restart.
        self.conn.lock().execute(
            "UPDATE supervision_inbox SET resolved=1 WHERE kind IN ('question','permission')",
            [],
        )?;
        Ok(())
    }
}
impl crate::Core {
    pub fn inbox(&self) -> Result<Vec<InboxItem>> {
        self.store.inbox()
    }
    pub fn dismiss_inbox(&self, id: &str) -> Result<()> {
        if let Some(item) = self.store.inbox()?.into_iter().find(|i| i.id == id) {
            if matches!(item.kind.as_str(), "question" | "permission") {
                bail!("answer this request in its conversation");
            }
            if self.store.resolve_inbox(id)? {
                self.emitter.emit(Event::InboxResolved { id: id.into() });
            }
        }
        Ok(())
    }
    pub fn repo_controls(&self, repo_id: &str) -> Result<RepoControls> {
        self.store.repo_controls(repo_id)
    }
    pub fn save_repo_controls(&self, repo_id: &str, controls: &RepoControls) -> Result<()> {
        self.store.save_repo_controls(repo_id, controls)?;
        self.emitter.emit(Event::RepoControlsChanged {
            repo_id: repo_id.into(),
            controls: controls.clone(),
        });
        Ok(())
    }
    pub fn repo_resources(&self, repo_id: &str) -> Result<ResourceUsage> {
        let controls = self.store.repo_controls(repo_id)?;
        let rows = self
            .usage(controls.spend_since)?
            .into_iter()
            .filter(|u| u.record.repo_id == repo_id)
            .collect::<Vec<_>>();
        let mut ids: HashSet<String> = rows.iter().map(|u| u.record.session_id.clone()).collect();
        for ws in self.store.workspaces()?.into_iter().filter(|w| w.repo_id == repo_id) {
            ids.extend(self.store.sessions(&ws.id)?.into_iter().map(|s| s.id));
        }
        let mut total = ResourceUsage::sum(&ids, rows);
        total.running = self.agents.admissions.running(&ids);
        Ok(total)
    }
    pub fn subtree_resources(&self, session_id: &str) -> Result<ResourceUsage> {
        let ids: HashSet<String> = self.store.subtree_session_ids(session_id)?.into_iter().collect();
        let mut total = ResourceUsage::sum(&ids, self.usage(0)?);
        total.running = self.agents.admissions.running(&ids);
        Ok(total)
    }
    pub fn record_delegation_completion(&self, task_id: &str, session_id: &str, report: &str) -> Result<()> {
        if let Some(item) = self
            .store
            .push_inbox(session_id, &format!("delegation:{task_id}"), "delegation", report)?
        {
            self.emitter.emit(Event::InboxUpdated { item });
        }
        Ok(())
    }
}
