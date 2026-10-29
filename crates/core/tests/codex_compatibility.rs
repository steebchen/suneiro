use std::{sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde_json::{json, Value};
use suneiro_core::{AgentDef, Core, Event, Repo, Workspace};

struct Harness {
    core: Arc<Core>,
    log: Arc<Mutex<Vec<Event>>>,
    sid: String,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn new(flags: &[&str], model: Option<&str>, effort: Option<&str>) -> Self {
        std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        assert!(suneiro_core::env::std_command("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        let log: Arc<Mutex<Vec<Event>>> = Arc::default();
        let sink = log.clone();
        let core = Core::new(&dir.path().join("data"), Arc::new(move |e| sink.lock().push(e))).unwrap();
        core.agents.register(AgentDef {
            id: "codex".into(),
            name: "Codex fixture (not real CLI)".into(),
            command: "node".into(),
            args: std::iter::once(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/codex/agent.mjs").to_string())
                .chain(flags.iter().map(|s| s.to_string()))
                .collect(),
        });
        core.store
            .add_repo(&Repo {
                id: "r".into(),
                name: "r".into(),
                path: repo.display().to_string(),
                default_branch: "main".into(),
            })
            .unwrap();
        core.store
            .add_workspace(&Workspace {
                id: "w".into(),
                repo_id: "r".into(),
                name: "w".into(),
                branch: "b".into(),
                base_branch: "main".into(),
                path: repo.display().to_string(),
                status: "ready".into(),
                created_at: 0,
                title: "Codex test".into(),
                archived_at: None,
                unread: false,
            })
            .unwrap();
        let sid = core
            .create_session("w", "codex", model.map(str::to_string), effort.map(str::to_string))
            .unwrap()
            .id;
        Self { core, log, sid, _dir: dir }
    }

    async fn wait(&self, pred: impl Fn(&Event) -> bool) -> Event {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(e) = self.log.lock().iter().find(|e| pred(e)).cloned() {
                    return e;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("events: {:#?}", self.log.lock()))
    }

    async fn ready(&self) {
        self.wait(|e| matches!(e, Event::SessionState { state, .. } if state == "idle"))
            .await;
    }

    async fn turn(&self, text: &str) {
        self.log.lock().clear();
        self.core.agents.prompt(&self.sid, text.into()).unwrap();
        self.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
    }

    fn config_trace(&self) -> Value {
        self.log
            .lock()
            .iter()
            .find_map(|e| match e {
                Event::SessionUpdate { update, .. } => update["content"]["text"]
                    .as_str()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .filter(|v| v.get("model").is_some()),
                _ => None,
            })
            .expect("config trace")
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.core.shutdown();
    }
}

#[tokio::test]
async fn codex_legacy_model_effort_is_applied_after_model_changes() {
    let h = Harness::new(&[], Some("gpt-small[low]"), None);
    h.ready().await;
    h.turn("tokens").await;
    let trace = h.config_trace();
    assert_eq!(trace["model"], "gpt-small");
    assert_eq!(trace["effort"], "low");
}

#[tokio::test]
async fn codex_plan_uses_read_only_sandbox_and_forwards_permissions() {
    let h = Harness::new(&[], None, None);
    h.ready().await;
    h.core.agents.set_plan_mode(&h.sid, true).await.unwrap();
    h.log.lock().clear();
    h.core.agents.prompt(&h.sid, "permission".into()).unwrap();
    let Event::PermissionRequest { request_id, .. } = h.wait(|e| matches!(e, Event::PermissionRequest { .. })).await else {
        unreachable!()
    };
    let trace = h.config_trace();
    assert_eq!(trace["mode"], "read-only");
    assert_eq!(trace["collaboration"], "plan");
    h.core
        .agents
        .respond_permission(&h.sid, &request_id, Some("no".into()))
        .await
        .unwrap();
    h.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
    h.core.agents.set_plan_mode(&h.sid, false).await.unwrap();
    h.turn("permission").await;
    assert!(!h.log.lock().iter().any(|e| matches!(e, Event::PermissionRequest { .. })));
    assert_eq!(h.config_trace()["mode"], "agent-full-access");
}

#[tokio::test]
async fn codex_usage_uses_reported_model_and_separate_cached_tokens() {
    let h = Harness::new(&[], None, None);
    h.ready().await;
    let mut pricing = suneiro_core::usage::Pricing::new();
    pricing.insert(
        "gpt-small".into(),
        suneiro_core::usage::ModelPrice {
            input: 2.0,
            cached_input: 0.5,
            output: 10.0,
        },
    );
    suneiro_core::usage::save_pricing(&h.core.store, &pricing).unwrap();
    h.turn("tokens").await;
    let rows = h.core.usage(0).unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.record.model, "gpt-small");
    assert_eq!(
        (row.record.input_tokens, row.record.cached_tokens, row.record.output_tokens),
        (193, 23936, 12)
    );
    assert!(row.record.cost_usd.is_none());
    assert!(row.estimated);
    assert!((row.cost.unwrap() - (193.0 * 2.0 + 23936.0 * 0.5 + 12.0 * 10.0) / 1_000_000.0).abs() < 1e-12);
}

#[tokio::test]
async fn codex_questions_are_never_auto_answered_and_url_mode_is_declined() {
    let h = Harness::new(&[], None, None);
    h.ready().await;
    h.log.lock().clear();
    h.core.agents.prompt(&h.sid, "question".into()).unwrap();
    let Event::Question {
        request_id,
        schema,
        auto_resolve_ms,
        ..
    } = h.wait(|e| matches!(e, Event::Question { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(schema["required"], json!(["approach"]));
    assert_eq!(auto_resolve_ms, Some(60000));
    assert!(h.core.agents.is_running(&h.sid));
    h.core
        .agents
        .answer_question(&h.sid, &request_id, json!({"action":"accept","content":{"approach":"small"}}))
        .await
        .unwrap();
    h.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
    assert!(h.log.lock().iter().any(|e| matches!(e, Event::QuestionResolved { .. })));
    h.turn("url-question").await;
    assert!(!h.log.lock().iter().any(|e| matches!(e, Event::Question { .. })));
    assert!(h.log.lock().iter().any(
        |e| matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"].as_str().is_some_and(|t| t.contains("decline")))
    ));
}

#[tokio::test]
async fn codex_cancel_clears_pending_questions() {
    let h = Harness::new(&[], None, None);
    h.ready().await;
    h.log.lock().clear();
    h.core.agents.prompt(&h.sid, "question".into()).unwrap();
    h.wait(|e| matches!(e, Event::Question { .. })).await;
    h.core.agents.cancel(&h.sid).await.unwrap();
    h.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
    assert!(h.log.lock().iter().any(|e| matches!(e, Event::QuestionResolved { .. })));
    assert!(!h.core.agents.is_running(&h.sid));
}

#[tokio::test]
async fn codex_steering_follows_advertised_capabilities() {
    for supported in [true, false] {
        let h = Harness::new(if supported { &[] } else { &["--no-steering"] }, None, None);
        h.ready().await;
        h.log.lock().clear();
        h.core.agents.prompt(&h.sid, "slow".into()).unwrap();
        h.wait(|e| matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"].as_str().is_some_and(|t| t.contains("blocks")))).await;
        let outcome = h.core.agents.steer(&h.sid, "tokens".into()).await.unwrap();
        if supported {
            assert_eq!(outcome, "injected");
            h.core.agents.cancel(&h.sid).await.unwrap();
        } else {
            assert_eq!(outcome, "interrupted");
        }
        h.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
        if !supported {
            h.wait(|e| matches!(e, Event::Usage { .. })).await;
        }
    }
}

#[tokio::test]
async fn codex_restarts_resume_without_duplicate_transcript_or_usage() {
    let h = Harness::new(&[], Some("gpt-small"), Some("low"));
    h.ready().await;
    h.turn("tokens").await;
    let before = h.core.store.session(&h.sid).unwrap().acp_session_id;
    h.core.agents.close(&h.sid);
    h.turn("tokens").await;
    assert_eq!(h.core.store.session(&h.sid).unwrap().acp_session_id, before);
    assert_eq!(h.config_trace()["model"], "gpt-small");
    assert_eq!(h.config_trace()["effort"], "low");
    assert_eq!(h.core.usage(0).unwrap().len(), 2);
    assert_eq!(h.log.lock().iter().filter(|e| matches!(e, Event::TurnEnd { .. })).count(), 1);
}

#[tokio::test]
async fn codex_unsupported_model_does_not_silently_prompt_another_model() {
    let h = Harness::new(&[], Some("unavailable-model"), None);
    h.wait(|e| matches!(e, Event::SessionState { state, error, .. } if state == "error" && error.as_deref().is_some_and(|e| e.contains("does not support requested model")))).await;
    h.log.lock().clear();
    h.core.agents.prompt(&h.sid, "tokens".into()).unwrap();
    h.wait(|e| matches!(e, Event::SessionState { state, error, .. } if state == "error" && error.as_deref().is_some_and(|e| e.contains("does not support requested model")))).await;
    assert!(h.core.usage(0).unwrap().is_empty());
}

#[tokio::test]
async fn codex_missing_collaboration_mode_reports_unsupported_capability() {
    let h = Harness::new(&["--no-plan"], None, None);
    h.ready().await;
    let error = h.core.agents.set_plan_mode(&h.sid, true).await.unwrap_err();
    assert!(error.to_string().contains("does not advertise collaboration_mode"));
    h.log.lock().clear();
    h.core.agents.prompt(&h.sid, "tokens".into()).unwrap();
    h.wait(|e| matches!(e, Event::SessionState { state, error, .. } if state == "error" && error.as_deref().is_some_and(|e| e.contains("does not advertise collaboration_mode")))).await;
    assert!(h.core.usage(0).unwrap().is_empty());
}

#[tokio::test]
async fn codex_effort_validation_uses_new_models_advertised_choices() {
    let h = Harness::new(&[], Some("gpt-small"), Some("high"));
    h.wait(|e| matches!(e, Event::SessionState { state, error, .. } if state == "error" && error.as_deref().is_some_and(|e| e.contains("reasoning effort `high`")))).await;
    assert!(h.core.usage(0).unwrap().is_empty());
}

#[tokio::test]
async fn codex_image_blocks_follow_prompt_capability() {
    for supported in [true, false] {
        let h = Harness::new(if supported { &[] } else { &["--no-images"] }, None, None);
        h.ready().await;
        let image = h._dir.path().join("pixel.png");
        std::fs::write(&image, b"fixture image bytes").unwrap();
        h.log.lock().clear();
        h.core
            .agents
            .prompt_with(&h.sid, "tokens".into(), vec![image.display().to_string()])
            .unwrap();
        h.wait(|e| matches!(e, Event::TurnEnd { .. })).await;
        let trace = h.config_trace();
        let blocks = trace["blocks"].as_array().unwrap();
        assert_eq!(blocks.iter().any(|b| b["type"] == "image"), supported);
        if !supported {
            assert!(blocks.iter().any(|b| b["type"] == "resource_link" && b["name"] == "pixel.png"));
        }
    }
}
