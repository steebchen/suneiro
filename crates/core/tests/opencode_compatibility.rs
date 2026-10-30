//! Scripted native-OpenCode boundary regressions; these do not call a provider.
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};
use suneiro_core::{AgentDef, Core, Event, Repo, Workspace};

struct Harness {
    core: Arc<Core>,
    log: Arc<Mutex<Vec<Event>>>,
    sid: String,
    dir: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.core.shutdown();
    }
}

#[tokio::test]
async fn provider_errors_preserve_cause_and_explain_readiness_or_image_support() {
    let h = Harness::new("provider-error", None, None);
    h.ready().await;
    let image = h.dir.path().join("image.png");
    std::fs::write(&image, b"fixture").unwrap();
    for images in [vec![], vec![image.display().to_string()]] {
        h.log.lock().clear();
        let expected = if images.is_empty() {
            "provider quota or connectivity"
        } else {
            "does not guarantee model vision support"
        };
        h.core
            .agents
            .prompt_with(&h.sid, "provider failure".into(), images)
            .unwrap();
        h.wait(|e| matches!(e, Event::SessionState { error: Some(error), .. } if error.contains(expected) && error.contains("Authentication failed for provider"))).await;
    }
}

impl Harness {
    fn new(scenario: &str, model: Option<&str>, effort: Option<&str>) -> Self {
        std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
        let dir = tempfile::tempdir().unwrap();
        let log: Arc<Mutex<Vec<Event>>> = Arc::default();
        let events = log.clone();
        let core = Core::new(dir.path(), Arc::new(move |event| events.lock().push(event))).unwrap();
        core.agents.register(AgentDef {
            id: "opencode".into(),
            name: "OpenCode".into(),
            command: "node".into(),
            args: vec![
                concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/opencode/agent.mjs").into(),
                scenario.into(),
            ],
        });
        core.store
            .add_repo(&Repo {
                id: "r".into(),
                name: "r".into(),
                path: dir.path().display().to_string(),
                default_branch: "main".into(),
            })
            .unwrap();
        core.store
            .add_workspace(&Workspace {
                id: "w".into(),
                repo_id: "r".into(),
                name: "w".into(),
                branch: "test".into(),
                base_branch: "main".into(),
                path: dir.path().display().to_string(),
                status: "ready".into(),
                created_at: 0,
                title: "fixture".into(),
                archived_at: None,
                unread: false,
            })
            .unwrap();
        let sid = core
            .create_session("w", "opencode", model.map(str::to_string), effort.map(str::to_string))
            .unwrap()
            .id;
        Self { core, log, sid, dir }
    }

    async fn wait(&self, pred: impl Fn(&Event) -> bool) -> Event {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(event) = self.log.lock().iter().find(|event| pred(event)).cloned() {
                    return event;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("events: {:?}", self.log.lock()))
    }

    async fn ready(&self) {
        self.wait(|e| matches!(e, Event::SessionState { state, .. } if state == "idle"))
            .await;
    }
    async fn end(&self) {
        self.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
    }
    async fn text(&self, expected: &str) {
        self.wait(|e| matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"].as_str().is_some_and(|text| text.contains(expected)))).await;
    }
}

#[tokio::test]
async fn unavailable_model_fails_instead_of_running_default_even_after_retry() {
    let h = Harness::new("", Some("missing/model"), None);
    h.wait(|e| matches!(e, Event::SessionState { error: Some(error), .. } if error.contains("unavailable")))
        .await;
    h.log.lock().clear();
    h.core.agents.prompt(&h.sid, "retry".into()).unwrap();
    h.wait(|e| matches!(e, Event::SessionState { error: Some(error), .. } if error.contains("unavailable")))
        .await;
    assert!(!h.log.lock().iter().any(|e| matches!(e, Event::TurnEnd { .. })));
    h.core
        .agents
        .set_config(&h.sid, "model", json!("opencode/big-pickle"))
        .await
        .unwrap();
    h.log.lock().clear();
    h.core.agents.prompt(&h.sid, "recovered".into()).unwrap();
    h.end().await;
}

#[tokio::test]
async fn unsupported_initial_effort_is_explained() {
    let h = Harness::new("", Some("opencode/big-pickle"), Some("high"));
    h.wait(|e| matches!(e, Event::SessionState { error: Some(error), .. } if error.contains("does not advertise reasoning effort"))).await;
}

#[tokio::test]
async fn empty_provider_catalog_is_actionable() {
    let h = Harness::new("no-models", None, None);
    h.wait(|e| matches!(e, Event::SessionState { error: Some(error), .. } if error.contains("opencode auth login")))
        .await;
    let error = h.core.refresh_catalog("opencode").await.unwrap_err().to_string();
    assert!(error.contains("opencode models"), "{error}");
}

#[tokio::test]
async fn provider_qualified_models_switch_and_unsupported_controls_fail_locally() {
    let h = Harness::new("", Some("local/model-with/slash"), None);
    h.ready().await;
    assert_eq!(
        h.core.store.session(&h.sid).unwrap().model.as_deref(),
        Some("local/model-with/slash")
    );
    let catalog = &h.core.catalogs()["opencode"];
    assert_eq!(catalog.models.len(), 2);
    assert!(catalog.efforts.is_empty());
    assert!(!catalog.has_fast);
    h.core
        .agents
        .set_config(&h.sid, "model", json!("opencode/big-pickle"))
        .await
        .unwrap();
    h.wait(|e| matches!(e, Event::SessionConfig { config_options, .. } if config_options[0]["currentValue"] == "opencode/big-pickle")).await;
    for (id, value) in [("model", "missing/model"), ("effort", "high"), ("fast", "on")] {
        let error = h
            .core
            .agents
            .set_config(&h.sid, id, json!(value))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("unavailable") || error.contains("does not advertise"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn tools_questions_and_images_keep_native_acp_shapes() {
    let h = Harness::new("", None, None);
    h.ready().await;
    let image = h.dir.path().join("image.png");
    std::fs::write(&image, b"fixture image bytes").unwrap();
    h.core
        .agents
        .prompt_with(&h.sid, "exercise".into(), vec![image.display().to_string()])
        .unwrap();
    let Event::Question { request_id, schema, .. } = h.wait(|e| matches!(e, Event::Question { .. })).await else {
        unreachable!()
    };
    assert_eq!(schema["properties"]["color"]["enum"], json!(["blue", "red"]));
    assert!(
        h.core.agents.is_running(&h.sid),
        "questions must await the user, including auto mode"
    );
    h.core
        .agents
        .answer_question(
            &h.sid,
            &request_id,
            json!({"action":"accept","content":{"color":"blue"}}),
        )
        .await
        .unwrap();
    h.end().await;
    h.text("permission:allow").await;
    h.text("\"color\":\"blue\"").await;
    h.text("blocks:[\"text\",\"image\"]").await;
    assert!(h.log.lock().iter().any(|e| matches!(e, Event::SessionUpdate { update, .. } if update["sessionUpdate"] == "tool_call_update" && update["status"] == "completed")));
    assert!(!h
        .log
        .lock()
        .iter()
        .any(|e| matches!(e, Event::PermissionRequest { .. })));
}

#[tokio::test]
async fn unsupported_images_fall_back_to_links() {
    let h = Harness::new("no-images", None, None);
    h.ready().await;
    let image = h.dir.path().join("image.png");
    std::fs::write(&image, b"fixture").unwrap();
    h.core
        .agents
        .prompt_with(&h.sid, "image".into(), vec![image.display().to_string()])
        .unwrap();
    h.end().await;
    h.text("blocks:[\"text\",\"resource_link\"]").await;
}

#[tokio::test]
async fn negotiated_no_steering_cancels_then_continues_and_reconnects() {
    for scenario in ["", "load-only"] {
        let h = Harness::new(scenario, None, None);
        h.ready().await;
        h.core.agents.prompt(&h.sid, "wait".into()).unwrap();
        h.text("waiting").await;
        assert_eq!(
            h.core.agents.steer(&h.sid, "continue".into()).await.unwrap(),
            "interrupted"
        );
        h.wait(|e| matches!(e, Event::TurnEnd { stop_reason, .. } if stop_reason == "cancelled"))
            .await;
        h.wait(|e| matches!(e, Event::TurnEnd { stop_reason, .. } if stop_reason == "end_turn"))
            .await;
        h.core.agents.close(&h.sid);
        h.log.lock().clear();
        h.core.agents.prompt(&h.sid, "after reconnect".into()).unwrap();
        h.end().await;
        assert_eq!(
            h.core.store.session(&h.sid).unwrap().acp_session_id.as_deref(),
            Some("opencode-fixture")
        );
        assert!(!h.log.lock().iter().any(|e| matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"] == "replayed history must not be duplicated")));
    }
}

#[test]
fn recorded_capabilities_do_not_invent_steering_effort_or_fast() {
    let init: Value = serde_json::from_str(include_str!("fixtures/opencode/initialize.json")).unwrap();
    assert_eq!(init["agentCapabilities"]["promptCapabilities"]["image"], true);
    assert!(init.pointer("/agentCapabilities/sessionCapabilities/resume").is_some());
    assert!(init.pointer("/_meta/steering/supported").is_none());
}
