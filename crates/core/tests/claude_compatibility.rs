//! Deterministic Claude-shaped ACP contracts. Real CLI coverage is opt-in;
//! see `cargo run -p suneiro-core --example claude_smoke -- --help`.
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::json;
use suneiro_core::{AgentDef, Core, Event, Repo, Session, Workspace};

struct Fixture {
    core: Arc<Core>,
    log: Arc<Mutex<Vec<Event>>>,
    sid: String,
    _dir: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.core.shutdown();
    }
}

impl Fixture {
    async fn new(args: &[&str], model: Option<&str>, effort: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let log: Arc<Mutex<Vec<Event>>> = Arc::default();
        let events = log.clone();
        let core = Core::new(dir.path(), Arc::new(move |e| events.lock().push(e))).unwrap();
        let mut argv = vec![concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/claude_acp.mjs").into()];
        argv.extend(args.iter().map(|s| s.to_string()));
        core.agents.register(AgentDef { id: "claude".into(), name: "Claude fixture".into(), command: "node".into(), args: argv });
        core.store.add_repo(&Repo { id: "r".into(), name: "fixture".into(), path: dir.path().display().to_string(), default_branch: "main".into() }).unwrap();
        core.store.add_workspace(&Workspace {
            id: "w".into(), repo_id: "r".into(), name: "fixture".into(), branch: "fixture".into(),
            base_branch: "main".into(), path: dir.path().display().to_string(), status: "ready".into(),
            created_at: 0, title: "Fixture".into(), archived_at: None, unread: false,
        }).unwrap();
        // Avoid background AI titles without changing process-wide environment.
        let sid = "fixture-session".to_string();
        core.store.add_session(&Session { id: sid.clone(), workspace_id: "w".into(), agent_id: "claude".into(), acp_session_id: None, title: "Fixture".into(), created_at: 0, model: model.map(str::to_string), effort: effort.map(str::to_string), parent_session_id: None }).unwrap();
        core.agents.preset(&sid, false, model.map(str::to_string), effort.map(str::to_string)).unwrap();
        core.agents.warm_up(&sid);
        let f = Self { core, log, sid, _dir: dir };
        f.wait(|e| matches!(e, Event::SessionState { state, .. } if state == "idle")).await;
        f.log.lock().clear();
        f
    }

    async fn wait(&self, pred: impl Fn(&Event) -> bool) -> Event {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(e) = self.log.lock().iter().find(|e| pred(e)).cloned() { return e; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap_or_else(|_| panic!("events: {:#?}", self.log.lock()))
    }

    async fn turn(&self, text: &str) {
        self.log.lock().clear();
        self.core.agents.prompt(&self.sid, text.into()).unwrap();
        self.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
    }

    fn has_text(&self, needle: &str) -> bool {
        self.log.lock().iter().any(|e| matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"].as_str().is_some_and(|s| s.contains(needle))))
    }
}

#[tokio::test]
async fn reply_only_config_changes_update_model_effort_and_usage() {
    let f = Fixture::new(&[], Some("opus"), Some("max")).await;
    let session = f.core.store.session(&f.sid).unwrap();
    assert_eq!(session.model.as_deref(), Some("opus"));
    assert_eq!(session.effort.as_deref(), Some("max"));
    assert!(f.core.catalogs()["claude"].efforts.iter().any(|e| e.value == "max"));
    f.core.agents.set_config(&f.sid, "model", json!("sonnet")).await.unwrap();
    f.core.agents.set_config(&f.sid, "effort", json!("high")).await.unwrap();
    let session = f.core.store.session(&f.sid).unwrap();
    assert_eq!(session.model.as_deref(), Some("sonnet"), "reply-only model change must update core state");
    assert_eq!(session.effort.as_deref(), Some("high"));
    f.turn("hello").await;
    let rows = f.core.store.usage_for_session(&f.sid).unwrap();
    assert_eq!(rows[0].model, "sonnet");
    assert_eq!((rows[0].input_tokens, rows[0].cached_tokens, rows[0].output_tokens), (10, 5, 5));
}

#[tokio::test]
async fn load_suppresses_history_and_resumed_cumulative_usage_is_not_double_counted() {
    let f = Fixture::new(&[], None, None).await;
    f.turn("first").await;
    f.core.agents.close(&f.sid);
    f.turn("second").await;
    assert!(!f.has_text("REPLAY_MUST_NOT_APPEAR"));
    assert_eq!(f.core.store.session(&f.sid).unwrap().acp_session_id.as_deref(), Some("claude-fixture"));
    let rows = f.core.store.usage_for_session(&f.sid).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter_map(|r| r.cost_usd).sum::<f64>(), 0.5);
}

#[tokio::test]
async fn failed_load_does_not_silently_replace_conversation() {
    let f = Fixture::new(&["--load-fails"], None, None).await;
    f.turn("first").await;
    f.core.agents.close(&f.sid);
    f.log.lock().clear();
    f.core.agents.prompt(&f.sid, "second".into()).unwrap();
    f.wait(|e| matches!(e, Event::SessionState { state, .. } if state == "error" || state == "idle")).await;
    assert!(f.log.lock().iter().any(|e| matches!(e, Event::SessionState { state, error: Some(error), .. } if state == "error" && error.contains("Claude session unavailable"))), "resume failure must be actionable, not a new conversation");
    assert!(!f.log.lock().iter().any(|e| matches!(e, Event::TurnEnd { .. })));
}

#[tokio::test]
async fn plan_permissions_tools_and_questions_remain_interactive() {
    let f = Fixture::new(&[], None, None).await;
    f.turn("permission").await;
    assert!(f.has_text("\"optionId\":\"allow\""));
    assert!(!f.log.lock().iter().any(|e| matches!(e, Event::PermissionRequest { .. })));
    assert!(f.log.lock().iter().any(|e| matches!(e, Event::SessionUpdate { update, .. } if update["sessionUpdate"] == "tool_call_update" && update["status"] == "completed")));
    f.core.agents.set_plan_mode(&f.sid, true).await.unwrap();
    f.log.lock().clear();
    f.core.agents.prompt(&f.sid, "permission".into()).unwrap();
    let Event::PermissionRequest { request_id, .. } = f.wait(|e| matches!(e, Event::PermissionRequest { .. })).await else { unreachable!() };
    f.core.agents.respond_permission(&f.sid, &request_id, None).await.unwrap();
    f.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
    assert!(f.has_text("cancelled"));
    f.core.agents.set_plan_mode(&f.sid, false).await.unwrap();
    f.log.lock().clear();
    f.core.agents.prompt(&f.sid, "question".into()).unwrap();
    let Event::Question { request_id, schema, tool_call_id, .. } = f.wait(|e| matches!(e, Event::Question { .. })).await else { unreachable!() };
    assert_eq!(tool_call_id.as_deref(), Some("ask-1"));
    assert_eq!(schema["properties"]["color"]["enum"], json!(["red", "blue"]));
    f.core.agents.answer_question(&f.sid, &request_id, json!({"action": "accept", "content": {"color": "blue"}})).await.unwrap();
    f.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
    assert!(f.has_text("blue"));
}

#[tokio::test]
async fn cancellation_resolves_question_and_unsupported_steering_interrupts() {
    let f = Fixture::new(&[], None, None).await;
    f.core.agents.prompt(&f.sid, "question".into()).unwrap();
    f.wait(|e| matches!(e, Event::Question { .. })).await;
    f.core.agents.cancel(&f.sid).await.unwrap();
    f.wait(|e| matches!(e, Event::QuestionResolved { .. })).await;
    f.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
    f.log.lock().clear();
    f.core.agents.prompt(&f.sid, "slow".into()).unwrap();
    f.wait(|e| matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"] == "working")).await;
    assert_eq!(f.core.agents.steer(&f.sid, "replacement".into()).await.unwrap(), "interrupted");
    f.wait(|e| matches!(e, Event::TurnEnd { stop_reason, .. } if stop_reason == "end_turn")).await;
    assert!(f.has_text("model:"));
}

#[tokio::test]
async fn image_capability_selects_inline_or_link_content() {
    for (args, expected) in [(vec![], "image"), (vec!["--no-images"], "resource_link")] {
        let f = Fixture::new(&args, None, None).await;
        let image = f._dir.path().join("attachment.png");
        std::fs::write(&image, [1, 2, 3]).unwrap();
        f.core.agents.prompt_with(&f.sid, "image".into(), vec![image.display().to_string()]).unwrap();
        f.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
        assert!(f.has_text(&format!("\"type\":\"{expected}\"")));
        if expected == "image" { assert!(f.has_text("AQID")); }
    }
}
