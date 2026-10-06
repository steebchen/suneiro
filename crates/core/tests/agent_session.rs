use std::sync::Arc;

use std::time::Duration;

use parking_lot::Mutex;
use suneiro_core::{AgentDef, Core, Event, Repo, Workspace};
use serde_json::json;

fn event_log() -> (Arc<Mutex<Vec<Event>>>, suneiro_core::Sink) {
    let log: Arc<Mutex<Vec<Event>>> = Arc::default();
    let sink_log = log.clone();
    (log, Arc::new(move |e| sink_log.lock().push(e)))
}

async fn wait_for(log: &Mutex<Vec<Event>>, what: &str, pred: impl Fn(&Event) -> bool) -> Event {
    for _ in 0..200 {
        if let Some(e) = log.lock().iter().find(|e| pred(e)) {
            return e.clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}; got {:#?}", log.lock());
}

#[tokio::test]
async fn prompt_permission_config_and_resume() {
    std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
    let tmp = tempfile::tempdir().unwrap();
    let (log, sink) = event_log();
    let core = Core::new(tmp.path(), sink).unwrap();
    core.agents.register(AgentDef {
        id: "fake".into(),
        name: "Fake".into(),
        command: "node".into(),
        args: vec![concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_agent.mjs").into()],
    });

    let repo = Repo { id: "r".into(), name: "r".into(), path: tmp.path().display().to_string(), default_branch: "main".into() };
    core.store.add_repo(&repo).unwrap();
    core.store
        .add_workspace(&Workspace {
            id: "w".into(),
            repo_id: "r".into(),
            name: "w".into(),
            branch: "b".into(),
            base_branch: "main".into(),
            path: tmp.path().display().to_string(),
            status: "ready".into(),
            created_at: 0,
            title: String::new(),
            archived_at: None,
            unread: false,
        })
        .unwrap();

    let session = core.create_session("w", "fake", Some("smart".into()), None).unwrap();
    let sid = session.id.clone();

    // Warm-up connects, applies the preset model and reports config options.
    wait_for(&log, "preset model", |e| {
        matches!(e, Event::SessionConfig { config_options, .. } if config_options[0]["currentValue"] == "smart")
    })
    .await;
    core.agents.set_config(&sid, "model", json!("fast")).await.unwrap();
    wait_for(&log, "updated config", |e| {
        matches!(e, Event::SessionConfig { config_options, .. } if config_options[0]["currentValue"] == "fast")
    })
    .await;
    // Live config also feeds the model catalog.
    assert_eq!(core.catalogs()["fake"].models.len(), 2);

    // Default mode auto-accepts: the agent's permission request never reaches the UI.
    core.agents.prompt(&sid, "do the thing".into()).unwrap();
    wait_for(&log, "turn end", |e| matches!(e, Event::TurnEnd { .. })).await;
    wait_for(&log, "allowed", |e| {
        matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"] == "outcome:allow")
    })
    .await;
    assert!(!core.agents.is_running(&sid));
    assert_eq!(core.store.session(&sid).unwrap().acp_session_id.as_deref(), Some("fake-1"));
    assert_eq!(core.store.session(&sid).unwrap().title, "Do the thing");
    assert_eq!(core.store.workspace("w").unwrap().title, "Do the thing");

    // Transcript is persisted with streamed chunks merged.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let stored = core.store.events(&sid).unwrap();
    assert_eq!(stored[0]["type"], "userMessage");
    assert!(stored.iter().any(|e| e["update"]["content"]["text"] == "Hello world"));
    assert_eq!(stored.last().unwrap()["type"], "turnEnd");

    assert!(!log.lock().iter().any(|e| matches!(e, Event::PermissionRequest { .. })));

    // After the process dies, the next prompt resumes the same ACP session.
    // In plan mode, permission requests are shown to the user.
    core.agents.close(&sid);
    log.lock().clear();
    core.agents.set_plan_mode(&sid, true).await.unwrap();
    core.agents.prompt(&sid, "again".into()).unwrap();
    let Event::PermissionRequest { request_id, .. } =
        wait_for(&log, "permission 2", |e| matches!(e, Event::PermissionRequest { .. })).await
    else {
        unreachable!()
    };
    core.agents.respond_permission(&sid, &request_id, None).await.unwrap();
    wait_for(&log, "cancelled", |e| {
        matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"] == "outcome:cancelled")
    })
    .await;
    assert_eq!(core.store.session(&sid).unwrap().acp_session_id.as_deref(), Some("fake-1"));
    core.shutdown();
}

#[tokio::test]
async fn questions_are_forwarded_and_answered() {
    std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
    let tmp = tempfile::tempdir().unwrap();
    let (log, sink) = event_log();
    let core = Core::new(tmp.path(), sink).unwrap();
    core.agents.register(AgentDef {
        id: "fake".into(),
        name: "Fake".into(),
        command: "node".into(),
        args: vec![concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_agent.mjs").into()],
    });
    core.store
        .add_repo(&Repo { id: "r".into(), name: "r".into(), path: tmp.path().display().to_string(), default_branch: "main".into() })
        .unwrap();
    core.store
        .add_workspace(&Workspace {
            id: "w".into(),
            repo_id: "r".into(),
            name: "w".into(),
            branch: "b".into(),
            base_branch: "main".into(),
            path: tmp.path().display().to_string(),
            status: "ready".into(),
            created_at: 0,
            title: String::new(),
            archived_at: None,
            unread: false,
        })
        .unwrap();
    let sid = core.create_session("w", "fake", None, None).unwrap().id;

    // Auto mode still forwards questions: they need a human.
    core.agents.prompt(&sid, "ask me".into()).unwrap();
    let Event::Question { request_id, message, schema, .. } =
        wait_for(&log, "question", |e| matches!(e, Event::Question { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(message, "Which color?");
    assert!(schema["properties"]["question_0"]["oneOf"].is_array());
    core.agents
        .answer_question(&sid, &request_id, json!({"action": "accept", "content": {"question_0": "Blue"}}))
        .await
        .unwrap();
    wait_for(&log, "resolved", |e| matches!(e, Event::QuestionResolved { .. })).await;
    wait_for(&log, "agent got answer", |e| {
        matches!(e, Event::SessionUpdate { update, .. }
            if update["content"]["text"].as_str().is_some_and(|t| t.contains("\"question_0\":\"Blue\"")))
    })
    .await;
    core.shutdown();
}

async fn fake_core(no_steer: bool) -> (Arc<Core>, Arc<Mutex<Vec<Event>>>, String, tempfile::TempDir) {
    std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
    let tmp = tempfile::tempdir().unwrap();
    let (log, sink) = event_log();
    let core = Core::new(tmp.path(), sink).unwrap();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_agent.mjs");
    let (command, args) = if no_steer {
        ("env".to_string(), vec!["FAKE_NO_STEER=1".to_string(), "node".into(), script.into()])
    } else {
        ("node".to_string(), vec![script.into()])
    };
    core.agents.register(AgentDef { id: "fake".into(), name: "Fake".into(), command, args });
    core.store
        .add_repo(&Repo { id: "r".into(), name: "r".into(), path: tmp.path().display().to_string(), default_branch: "main".into() })
        .unwrap();
    core.store
        .add_workspace(&Workspace {
            id: "w".into(),
            repo_id: "r".into(),
            name: "w".into(),
            branch: "b".into(),
            base_branch: "main".into(),
            path: tmp.path().display().to_string(),
            status: "ready".into(),
            created_at: 0,
            title: String::new(),
            archived_at: None,
            unread: false,
        })
        .unwrap();
    let sid = core.create_session("w", "fake", None, None).unwrap().id;
    wait_for(&log, "connected", |e| matches!(e, Event::SessionConfig { .. })).await;
    (core, log, sid, tmp)
}

#[tokio::test]
async fn steering_injects_into_the_running_turn() {
    let (core, log, sid, _tmp) = fake_core(false).await;
    assert!(core.activity().is_idle());
    // Idle: steering is just a prompt.
    core.agents.prompt(&sid, "slow one".into()).unwrap();
    assert_eq!(core.activity(), suneiro_core::Activity { agents: 1, workspaces: 1 });
    wait_for(&log, "turn running", |e| {
        matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"] == "working on slow one")
    })
    .await;
    assert_eq!(core.agents.steer(&sid, "also this".into()).await.unwrap(), "injected");
    wait_for(&log, "steered", |e| {
        matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"] == "steered:also this")
    })
    .await;
    assert!(core.agents.is_running(&sid), "steering doesn't stop the turn");
    wait_for(&log, "turn end", |e| matches!(e, Event::TurnEnd { stop_reason, .. } if stop_reason == "end_turn")).await;
    assert!(core.activity().is_idle(), "a finished turn is no longer active");
    assert_eq!(core.agents.steer(&sid, "slow two".into()).await.unwrap(), "sent");
    core.shutdown();
}

#[tokio::test]
async fn without_steering_the_turn_is_interrupted() {
    let (core, log, sid, _tmp) = fake_core(true).await;
    core.agents.prompt(&sid, "slow one".into()).unwrap();
    wait_for(&log, "turn running", |e| {
        matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"] == "working on slow one")
    })
    .await;
    assert_eq!(core.agents.steer(&sid, "slow now".into()).await.unwrap(), "interrupted");
    wait_for(&log, "cancelled", |e| matches!(e, Event::TurnEnd { stop_reason, .. } if stop_reason == "cancelled")).await;
    wait_for(&log, "new prompt", |e| {
        matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"] == "working on slow now")
    })
    .await;
    core.shutdown();
}

#[tokio::test]
async fn checkpoints_restore_the_worktree_and_tell_the_agent() {
    std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
    let tmp = tempfile::tempdir().unwrap();
    let wt = tmp.path().join("wt");
    std::fs::create_dir(&wt).unwrap();
    for args in [&["init", "-q", "-b", "main"][..], &["config", "user.email", "t@t"], &["config", "user.name", "t"]] {
        suneiro_core::git::git(&wt, args).await.unwrap();
    }
    std::fs::write(wt.join("a.txt"), "a\n").unwrap();
    suneiro_core::git::git(&wt, &["add", "."]).await.unwrap();
    suneiro_core::git::git(&wt, &["commit", "-qm", "init"]).await.unwrap();

    let (log, sink) = event_log();
    let core = Core::new(&tmp.path().join("data"), sink).unwrap();
    core.agents.register(AgentDef {
        id: "fake".into(),
        name: "Fake".into(),
        command: "node".into(),
        args: vec![concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_agent.mjs").into()],
    });
    let path = wt.display().to_string();
    core.store.add_repo(&Repo { id: "r".into(), name: "r".into(), path: path.clone(), default_branch: "main".into() }).unwrap();
    core.store
        .add_workspace(&Workspace {
            id: "w".into(),
            repo_id: "r".into(),
            name: "w".into(),
            branch: "main".into(),
            base_branch: "main".into(),
            path,
            status: "ready".into(),
            created_at: 0,
            title: String::new(),
            archived_at: None,
            unread: false,
        })
        .unwrap();
    let sid = core.create_session("w", "fake", None, None).unwrap().id;

    core.agents.prompt(&sid, "write something".into()).unwrap();
    let Event::Checkpoint { commit, .. } = wait_for(&log, "checkpoint", |e| matches!(e, Event::Checkpoint { .. })).await else {
        unreachable!()
    };
    wait_for(&log, "turn end", |e| matches!(e, Event::TurnEnd { .. })).await;
    assert!(wt.join("agent.txt").exists());

    let undo = core.restore_checkpoint(&sid, &commit).await.unwrap();
    assert!(!wt.join("agent.txt").exists());
    wait_for(&log, "restored", |e| matches!(e, Event::CheckpointRestored { .. })).await;

    // The agent hears about it with the next prompt.
    log.lock().clear();
    core.agents.prompt(&sid, "write again".into()).unwrap();
    wait_for(&log, "note", |e| {
        matches!(e, Event::SessionUpdate { update, .. } if update["content"]["text"].as_str().is_some_and(|t| t.contains("restored")))
    })
    .await;
    wait_for(&log, "turn end", |e| matches!(e, Event::TurnEnd { .. })).await;

    // Undoing brings back the state from before the restore.
    std::fs::remove_file(wt.join("agent.txt")).unwrap();
    core.restore_checkpoint(&sid, &undo).await.unwrap();
    assert!(wt.join("agent.txt").exists());
    core.shutdown();
}

#[tokio::test]
async fn unpushed_branch_is_named_after_the_task() {
    std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
    let tmp = tempfile::tempdir().unwrap();
    let wt = tmp.path().join("wt");
    std::fs::create_dir(&wt).unwrap();
    for args in [&["init", "-q", "-b", "runner/tokyo"][..], &["config", "user.email", "t@t"], &["config", "user.name", "t"], &["commit", "-q", "--allow-empty", "-m", "init"]] {
        suneiro_core::git::git(&wt, args).await.unwrap();
    }
    let (log, sink) = event_log();
    let core = Core::new(&tmp.path().join("data"), sink).unwrap();
    core.agents.register(AgentDef {
        id: "fake".into(),
        name: "Fake".into(),
        command: "node".into(),
        args: vec![concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_agent.mjs").into()],
    });
    let path = wt.display().to_string();
    core.store.add_repo(&Repo { id: "r".into(), name: "r".into(), path: path.clone(), default_branch: "main".into() }).unwrap();
    core.store
        .add_workspace(&Workspace {
            id: "w".into(),
            repo_id: "r".into(),
            name: "tokyo".into(),
            branch: "runner/tokyo".into(),
            base_branch: "main".into(),
            path,
            status: "ready".into(),
            created_at: 0,
            title: String::new(),
            archived_at: None,
            unread: false,
        })
        .unwrap();
    let sid = core.create_session("w", "fake", None, None).unwrap().id;
    core.agents.prompt(&sid, "Fix the login bug".into()).unwrap();
    let Event::WorkspaceBranch { branch, .. } =
        wait_for(&log, "branch rename", |e| matches!(e, Event::WorkspaceBranch { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(branch, "runner/fix-the-login-bug");
    assert_eq!(core.store.workspace("w").unwrap().branch, branch);
    assert_eq!(suneiro_core::git::git(&wt, &["branch", "--show-current"]).await.unwrap().trim(), branch);
    core.shutdown();
}
