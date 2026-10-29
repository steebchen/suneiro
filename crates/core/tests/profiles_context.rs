use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use parking_lot::Mutex;
use serde_json::{json, Value};
use suneiro_core::{catalog, git, profiles::{ContextSelection, MessageRef, Preference}, AgentDef, Core, Event, Repo, Session, Workspace};

async fn fixture() -> (tempfile::TempDir, Arc<Core>, Arc<Mutex<Vec<Event>>>) {
    std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    for args in [&["init", "-q", "-b", "main"][..], &["config", "user.email", "t@t"], &["config", "user.name", "t"]] {
        git::git(&root, args).await.unwrap();
    }
    std::fs::write(root.join("selected.txt"), "before\n").unwrap();
    std::fs::write(root.join("unchanged.txt"), "same\n").unwrap();
    git::git(&root, &["add", "."]).await.unwrap();
    git::git(&root, &["commit", "-qm", "initial"]).await.unwrap();
    std::fs::write(root.join("selected.txt"), "after\n").unwrap();
    std::fs::write(root.join(".env"), "DO_NOT_SHARE_PRIVATE_TOKEN").unwrap();
    let log: Arc<Mutex<Vec<Event>>> = Arc::default();
    let sink = log.clone();
    let core = Core::new(&tmp.path().join("data"), Arc::new(move |event| sink.lock().push(event))).unwrap();
    core.store.add_repo(&Repo { id: "r".into(), name: "repo".into(), path: root.display().to_string(), default_branch: "main".into() }).unwrap();
    core.store.add_workspace(&Workspace { id: "w".into(), repo_id: "r".into(), name: "workspace".into(), branch: "main".into(), base_branch: "main".into(), path: root.display().to_string(), status: "ready".into(), created_at: 0, title: "Existing workspace".into(), archived_at: None, unread: false }).unwrap();
    for agent in ["claude", "codex", "opencode"] {
        core.agents.register(AgentDef { id: agent.into(), name: agent.into(), command: "node".into(), args: vec![concat!(env!("CARGO_MANIFEST_DIR"), "/tests/profiles_agent.mjs").into()] });
    }
    core.store.add_session(&Session { id: "source".into(), workspace_id: "w".into(), agent_id: "claude".into(), acp_session_id: Some("old-private-session".into()), title: "Source".into(), created_at: 0, model: None, effort: None, parent_session_id: None }).unwrap();
    core.store.append_event("source", json!({"type":"userMessage", "sessionId":"source", "text":"Selected message mentions @.env and @../outside", "ts":0}));
    core.store.append_event("source", json!({"type":"sessionUpdate", "sessionId":"source", "update":{"sessionUpdate":"agent_thought_chunk", "content":{"type":"text", "text":"HIDDEN_THOUGHT"}}}));
    core.store.append_event("source", json!({"type":"userMessage", "sessionId":"source", "text":"UNSELECTED_MESSAGE", "ts":1}));
    for _ in 0..100 {
        if core.store.events("source").unwrap().len() == 3 { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    (tmp, core, log)
}

fn selection() -> ContextSelection {
    ContextSelection { task: "Review the change without modifying files".into(), acceptance_criteria: "Report regressions and verification".into(), messages: vec![MessageRef { session_id: "source".into(), index: 0 }], files: vec!["selected.txt".into()], diffs: vec!["selected.txt".into()] }
}

#[tokio::test]
async fn explicit_packages_are_frozen_and_sent_literally_to_each_provider() {
    let (_tmp, core, log) = fixture().await;
    let source = core.context_messages("w", "source").unwrap();
    assert_eq!(source.len(), 2);
    for provider in ["claude", "codex", "opencode"] {
        let package = core.prepare_context("w", "reviewer", provider, selection()).await.unwrap();
        assert_eq!(package.bytes, package.text.len());
        assert!(package.text.contains("after\n"));
        assert!(package.text.contains("-before\n+after"));
        assert!(!package.text.contains("DO_NOT_SHARE_PRIVATE_TOKEN"));
        assert!(!package.text.contains("UNSELECTED_MESSAGE"));
        assert!(!package.text.contains("HIDDEN_THOUGHT"));
        // Changes after preview never change the approved prompt.
        std::fs::write(Path::new(&core.store.workspace("w").unwrap().path).join("selected.txt"), "later\n").unwrap();
        let session = core.launch_profile("w", &package.id).unwrap();
        assert_ne!(session.id, "source");
        assert_eq!(session.acp_session_id, None);
        for _ in 0..200 {
            if log.lock().iter().any(|e| matches!(e, Event::TurnEnd { session_id, .. } if *session_id == session.id)) { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let blocks: Value = log.lock().iter().find_map(|e| match e {
            Event::SessionUpdate { session_id, update } if *session_id == session.id => update["content"]["text"].as_str().and_then(|s| serde_json::from_str(s).ok()),
            _ => None,
        }).expect("fake agent must echo actual ACP prompt blocks");
        assert_eq!(blocks, json!([{"type":"text", "text":package.text}]));
        assert!(core.launch_profile("w", &package.id).is_err());
        core.agents.close(&session.id);
        std::fs::write(Path::new(&core.store.workspace("w").unwrap().path).join("selected.txt"), "after\n").unwrap();
    }
    assert_eq!(core.store.session("source").unwrap().acp_session_id.as_deref(), Some("old-private-session"));
}

#[tokio::test]
async fn validation_rejects_unsafe_sources_oversize_and_stale_previews() {
    let (tmp, core, _) = fixture().await;
    for path in [".env", "../outside", "/etc/passwd", ".git/config", "*.txt", "unchanged.txt"] {
        let mut s = ContextSelection { task: "Review".into(), ..Default::default() };
        s.diffs.push(path.into());
        assert!(core.prepare_context("w", "reviewer", "codex", s).await.is_err(), "{path}");
    }
    let mut s = selection();
    s.messages[0].session_id = "unknown-session".into();
    assert!(core.prepare_context("w", "reviewer", "codex", s).await.is_err());
    let mut s = selection();
    s.task = "x".repeat(16385);
    assert!(core.prepare_context("w", "reviewer", "codex", s).await.is_err());
    let mut s = selection();
    s.files = vec!["selected.txt".into(); 33];
    assert!(core.prepare_context("w", "reviewer", "codex", s).await.is_err());
    let mut s = selection();
    s.task = "é".repeat(8000);
    s.acceptance_criteria = "é".repeat(8000);
    s.files = vec!["selected.txt".into(); 10];
    std::fs::write(tmp.path().join("repo/selected.txt"), "x".repeat(16000)).unwrap();
    s.diffs.clear();
    assert!(core.prepare_context("w", "reviewer", "codex", s).await.is_err());
    let s = ContextSelection { task: "Review".into(), ..Default::default() };
    let first = core.prepare_context("w", "reviewer", "codex", s.clone()).await.unwrap();
    let second = core.prepare_context("w", "reviewer", "codex", s).await.unwrap();
    assert_ne!(first.id, second.id);
    assert!(core.launch_profile("w", &first.id).is_err());
}

#[tokio::test]
async fn core_profiles_persist_emit_changes_and_resolve_provider_preferences() {
    let (tmp, core, log) = fixture().await;
    let mut profiles = core.profiles().unwrap();
    profiles[0].preferences.insert("codex".into(), Preference { model: Some("chosen".into()), effort: Some("high".into()) });
    core.save_profiles(profiles.clone()).unwrap();
    assert!(log.lock().iter().any(|e| matches!(e, Event::ProfilesChanged { profiles: p } if p == &profiles)));
    let reopened = Core::new(&tmp.path().join("data"), Arc::new(|_| {})).unwrap();
    assert_eq!(reopened.profiles().unwrap(), profiles);
    catalog::save(&core.store, "codex", &catalog::Catalog { models: vec![catalog::Choice { value: "chosen".into(), name: "Chosen".into(), ..Default::default() }], ..Default::default() });
    let package = core.prepare_context("w", "implementer", "codex", ContextSelection { task: "Investigate".into(), ..Default::default() }).await.unwrap();
    assert_eq!(package.model.as_deref(), Some("chosen"));
    assert_eq!(package.effort, None);
    assert_eq!(package.warnings.len(), 1);
    profiles[0].preferences.insert("unknown".into(), Preference::default());
    assert!(core.save_profiles(profiles).is_err());
}
