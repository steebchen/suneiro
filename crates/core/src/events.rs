use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

/// Everything the core reports to the UI. Session events that make up a
/// transcript are also persisted, so the same reducer rebuilds history.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Event {
    SessionUpdate {
        session_id: String,
        update: Value,
    },
    UserMessage {
        session_id: String,
        text: String,
        ts: i64,
        /// Attached images (files in the app's attachments folder).
        #[serde(skip_serializing_if = "Vec::is_empty")]
        images: Vec<String>,
    },
    /// Snapshot of the worktree taken right before the last user message was
    /// sent, so the files can be put back to how they were.
    Checkpoint {
        session_id: String,
        commit: String,
    },
    /// The files were restored to the checkpoint taken before a message.

    CheckpointRestored {
        session_id: String,
        commit: String,
        undo: Option<String>,
        ts: i64,
    },
    TurnEnd {
        session_id: String,
        stop_reason: String,
        ts: i64,
    },
    SessionState {
        session_id: String,
        state: String,
        error: Option<String>,
    },
    SessionConfig {
        session_id: String,
        config_options: Value,
    },
    SessionCapabilities {
        session_id: String,
        capabilities: crate::capabilities::SessionCapabilities,
    },
    SessionTitle {
        session_id: String,
        title: String,
    },

    /// Suneiro-level permission mode: plan (agent asks) or auto-accept everything.
    SessionMode {
        session_id: String,
        plan: bool,
    },
    /// A finished turn's usage (cost filled in or estimated when possible).
    Usage {
        session_id: String,
        usage: crate::usage::PricedUsage,
    },
    PermissionRequest {
        session_id: String,
        request_id: String,
        tool_call: Value,
        options: Value,
    },
    PermissionResolved {
        session_id: String,
        request_id: String,
    },
    /// The agent asks the user something (ACP form elicitation): `schema` is a
    /// JSON Schema object whose properties are the questions.
    Question {
        session_id: String,
        request_id: String,
        message: String,
        schema: Value,
        tool_call_id: Option<String>,
        /// Codex answers on its own after this long.
        auto_resolve_ms: Option<u64>,
    },
    QuestionResolved {
        session_id: String,
        request_id: String,
    },
    WorkspaceStatus {
        workspace_id: String,
        status: String,
    },
    WorkspaceTitle {
        workspace_id: String,
        title: String,
    },
    /// The workspace's branch was renamed (after its task).
    WorkspaceBranch {
        workspace_id: String,
        branch: String,
    },
    /// Latest PR for the workspace's branch (`null` if none).

    WorkspacePr {
        workspace_id: String,
        pr: Value,
    },
    ScriptOutput {
        workspace_id: String,
        data: String,
    },
    SessionAdded {
        session: crate::store::Session,
    },
    SessionHierarchyChanged {
        session_id: String,
        parent_session_id: Option<String>,
    },
    SessionRemoved {
        session_id: String,
        workspace_id: String,
    },

    ProfilesChanged { profiles: Vec<crate::profiles::AgentProfile> },

}

impl Event {
    /// The session whose transcript this event belongs to, if it should be stored.
    pub fn persist_key(&self) -> Option<&str> {
        match self {
            Event::SessionUpdate { session_id, update } => {
                let kind = update.get("sessionUpdate").and_then(|k| k.as_str()).unwrap_or("");
                let transient = matches!(kind, "available_commands_update" | "session_info_update");
                (!transient).then_some(session_id)
            }
            Event::UserMessage { session_id, .. }
            | Event::TurnEnd { session_id, .. }
            | Event::Checkpoint { session_id, .. }
            | Event::CheckpointRestored { session_id, .. } => Some(session_id),
            Event::SessionState { session_id, state, .. } if state == "error" => Some(session_id),
            _ => None,
        }
    }
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;
