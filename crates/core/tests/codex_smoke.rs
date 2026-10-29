//! Opt-in, billable real Codex check. Uses the existing CLI login, never reads credentials.
use std::{sync::Arc, time::Duration};

use parking_lot::Mutex;
use suneiro_core::{Core, Event, Repo, Workspace};

struct Shutdown(Arc<Core>);
impl Drop for Shutdown {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

async fn wait(log: &Mutex<Vec<Event>>, pred: impl Fn(&Event) -> bool) -> Event {
    tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let events = log.lock().clone();
            if let Some(Event::SessionState {
                error: Some(error), state, ..
            }) = events
                .iter()
                .find(|e| matches!(e, Event::SessionState { state, .. } if state == "error"))
            {
                panic!("Codex {state}: {error}");
            }
            if let Some(e) = events.iter().find(|e| pred(e)) {
                return e.clone();
            }
            drop(events);
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("real Codex timed out")
}

#[tokio::test]
#[ignore = "real Codex; existing login and billable model calls; set SUNEIRO_CODEX_SMOKE=1"]
async fn codex_real_login_plan_model_tokens_and_restart_continuation() {
    assert_eq!(
        std::env::var("SUNEIRO_CODEX_SMOKE").as_deref(),
        Ok("1"),
        "opt in explicitly; this uses your existing Codex login and model quota"
    );
    std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
    let status = suneiro_core::setup::detect("codex").await.expect("Codex definition");
    assert!(status.installed, "install the official Codex CLI first");
    assert!(
        status.logged_in,
        "run `codex login` first; credentials are never copied by this test"
    );
    println!(
        "real codex-cli {} / pinned codex-acp 2.0.0",
        status.version.as_deref().unwrap_or("unknown")
    );
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
    let _shutdown = Shutdown(core.clone());
    let mut settings = core.settings();
    settings.plan_by_default = true;
    settings.rename_branches = false;
    core.save_settings(&settings).unwrap();
    core.store
        .add_repo(&Repo {
            id: "r".into(),
            name: "disposable".into(),
            path: repo.display().to_string(),
            default_branch: "main".into(),
        })
        .unwrap();
    core.store
        .add_workspace(&Workspace {
            id: "w".into(),
            repo_id: "r".into(),
            name: "smoke".into(),
            branch: "smoke".into(),
            base_branch: "main".into(),
            path: repo.display().to_string(),
            status: "ready".into(),
            created_at: 0,
            title: "Codex smoke".into(),
            archived_at: None,
            unread: false,
        })
        .unwrap();
    let model = std::env::var("SUNEIRO_CODEX_MODEL").ok();
    let effort = std::env::var("SUNEIRO_CODEX_EFFORT").ok();
    let sid = core.create_session("w", "codex", model.clone(), effort.clone()).unwrap().id;
    wait(&log, |e| matches!(e, Event::SessionState { state, .. } if state == "idle")).await;
    let session = core.store.session(&sid).unwrap();
    if let Some(want) = model {
        assert_eq!(session.model.as_deref(), Some(want.as_str()));
    }
    if let Some(want) = effort {
        assert_eq!(session.effort.as_deref(), Some(want.as_str()));
    }
    assert!(log.lock().iter().any(|e| matches!(e, Event::SessionConfig { config_options, .. } if config_options.as_array().is_some_and(|opts| opts.iter().any(|o| o["id"] == "mode" && o["currentValue"] == "read-only")))));
    let nonce = format!("suneiro-{}", uuid::Uuid::new_v4().simple());
    for (i, prompt) in [
        format!("Do not use tools, modify files, or access the network. Remember this exact marker for our next turn: {nonce}. Reply only OK."),
        "Do not use tools, modify files, or access the network. What exact marker did I ask you to remember in our previous turn? Reply only with that marker.".to_string(),
    ].into_iter().enumerate() {
        if i == 1 { core.agents.close(&sid); }
        log.lock().clear();
        core.agents.prompt(&sid, prompt).unwrap();
        wait(&log, |e| matches!(e, Event::TurnEnd { .. })).await;
        assert_eq!(core.store.session(&sid).unwrap().acp_session_id, session.acp_session_id, "resume must retain the original conversation");
        if i == 1 {
            let text = log.lock().iter().filter_map(|e| match e {
                Event::SessionUpdate { update, .. } if update["sessionUpdate"] == "agent_message_chunk" => update["content"]["text"].as_str(),
                _ => None,
            }).collect::<String>();
            assert!(text.contains(&nonce), "resumed agent lost prior-turn context");
        }
    }
    let usage = core.usage(0).unwrap();
    assert_eq!(usage.len(), 2, "one usage record per real turn");
    assert!(usage
        .iter()
        .all(|u| u.record.input_tokens + u.record.cached_tokens + u.record.output_tokens > 0));
    assert!(
        usage.iter().all(|u| u.record.cost_usd.is_none()),
        "adapter changed cost semantics; review compatibility expectations"
    );
    assert!(
        !suneiro_core::git::has_uncommitted(&repo).await.unwrap(),
        "read-only smoke must not change repository files"
    );
    println!("real login, advertised model/effort, read-only plan, two token reports and process-restart conversation recall passed");
}
