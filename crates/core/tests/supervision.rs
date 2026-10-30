use parking_lot::Mutex;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use suneiro_core::{supervision::RepoControls, AgentDef, Core, Event, Repo, Workspace};

async fn until(log: &Mutex<Vec<Event>>, pred: impl Fn(&Event) -> bool) -> Event {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(e) = log.lock().iter().find(|e| pred(e)).cloned() {
                return e;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("agent event")
}

#[tokio::test]
async fn all_prompt_paths_use_admission_and_preserve_request_actions() {
    std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
    let dir = tempfile::tempdir().unwrap();
    let log: Arc<Mutex<Vec<Event>>> = Arc::default();
    let out = log.clone();
    let core = Core::new(dir.path(), Arc::new(move |e| out.lock().push(e))).unwrap();
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
            branch: "main".into(),
            base_branch: "main".into(),
            path: dir.path().display().to_string(),
            status: "ready".into(),
            created_at: 0,
            title: String::new(),
            archived_at: None,
            unread: false,
        })
        .unwrap();
    core.agents.register(AgentDef {
        id: "fake".into(),
        name: "Fake".into(),
        command: "node".into(),
        args: vec![concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_agent.mjs").into()],
    });
    let root = core.create_session("w", "fake", None, None).unwrap();
    let child = core.create_child_session(&root.id, "w", "fake", None, None).unwrap();
    core.save_repo_controls(
        "r",
        &RepoControls {
            max_concurrent: 1,
            ..Default::default()
        },
    )
    .unwrap();
    core.agents.prompt(&root.id, "ask first".into()).unwrap();
    let Event::Question { request_id, .. } = until(
        &log,
        |e| matches!(e,Event::Question{session_id,..} if session_id==&root.id),
    )
    .await
    else {
        unreachable!()
    };
    let inbox = core.inbox().unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, "question");
    assert!(core.dismiss_inbox(&inbox[0].id).is_err());
    assert_eq!(core.subtree_resources(&root.id).unwrap().running, 1);
    assert!(core
        .agents
        .prompt(&child.id, "must not start".into())
        .unwrap_err()
        .to_string()
        .contains("concurrency"));
    assert!(core.agents.prompt(&root.id, "duplicate".into()).is_err());
    assert!(!log
        .lock()
        .iter()
        .any(|e| matches!(e,Event::UserMessage{session_id,..} if session_id==&child.id)));
    core.agents
        .answer_question(
            &root.id,
            &request_id,
            json!({"action":"accept","content":{"question_0":"Blue"}}),
        )
        .await
        .unwrap();
    until(
        &log,
        |e| matches!(e,Event::SessionState{session_id,state,..} if session_id==&root.id && state=="idle"),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        while core.subtree_resources(&root.id).unwrap().running != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(core.inbox().unwrap().is_empty());
    core.agents.prompt(&child.id, "ask second".into()).unwrap();
    until(
        &log,
        |e| matches!(e,Event::Question{session_id,..} if session_id==&child.id),
    )
    .await;
    assert_eq!(core.inbox().unwrap()[0].session_id, child.id);
    core.agents.cancel(&child.id).await.unwrap();
    until(
        &log,
        |e| matches!(e,Event::SessionState{session_id,state,..} if session_id==&child.id && state=="idle"),
    )
    .await;
    assert!(core.inbox().unwrap().is_empty());
    core.record_delegation_completion("task-one", &child.id, "Implemented with tests")
        .unwrap();
    core.record_delegation_completion("task-one", &child.id, "Implemented with tests")
        .unwrap();
    assert_eq!(core.inbox().unwrap().len(), 1);
    let id = core.inbox().unwrap()[0].id.clone();
    core.dismiss_inbox(&id).unwrap();
    core.record_delegation_completion("task-one", &child.id, "Replayed after restart")
        .unwrap();
    assert!(core.inbox().unwrap().is_empty());
    core.shutdown();
}
