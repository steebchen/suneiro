use super::*;
fn request(key: &str) -> TaskRequest {
    TaskRequest {
        request_key: key.into(),
        relation: Relation::Child,
        isolation: Isolation::Isolated,
        agent_id: "fake".into(),
        model: None,
        effort: None,
        brief: "Implement the bounded task".into(),
        acceptance_criteria: "Tests pass; report evidence".into(),
        selected_context: "Only selected context".into(),
        base: TaskBase::CallerHead,
    }
}
#[test]
fn defaults_and_boundary_validation() {
    let value = serde_json::json!({"requestKey":"one","agentId":"fake","brief":"brief","acceptanceCriteria":"tests"});
    let task: TaskRequest = serde_json::from_value(value).unwrap();
    assert_eq!(task.isolation, Isolation::Isolated);
    assert_eq!(task.relation, Relation::Child);
    task.validate().unwrap();
    let mut bad = task.clone();
    bad.brief.clear();
    assert!(bad.validate().is_err());
    bad = task.clone();
    bad.selected_context = "x".repeat(MAX_TEXT + 1);
    assert!(bad.validate().is_err());
    let mut value = serde_json::to_value(task).unwrap();
    value["workspacePath"] = "/etc".into();
    assert!(serde_json::from_value::<TaskRequest>(value).is_err());
}
#[test]
fn duplicate_key_never_reserves_or_relaunches_and_payload_is_immutable() {
    let tmp = tempfile::tempdir().unwrap();
    let ledger = Delegation::open(&tmp.path().join("db")).unwrap();
    let (task, fresh) = ledger.reserve("parent", "repo", &request("stable")).unwrap();
    assert!(fresh);
    let (same, fresh) = ledger.reserve("parent", "repo", &request("stable")).unwrap();
    assert!(!fresh);
    assert_eq!(task.id, same.id);
    let mut other = request("stable");
    other.brief = "Different task".into();
    assert!(ledger.reserve("parent", "repo", &other).is_err());
    assert_eq!(ledger.reports("parent").unwrap().len(), 1);
}
#[test]
fn scope_messages_and_exactly_once_report_survive_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db");
    let id;
    {
        let ledger = Delegation::open(&path).unwrap();
        let task = ledger.reserve("parent", "repo", &request("one")).unwrap().0;
        id = task.id;
        ledger.bind(&id, "child", "worktree").unwrap();
        assert!(ledger.status("stranger", &id).is_err());
        assert!(ledger.send("stranger", &id, "steal").is_err());
        assert!(ledger.finish("parent", &id, "spoof report").is_err());
        assert!(ledger.finish("stranger", &id, "spoof report").is_err());
        ledger.send("parent", &id, "Selected follow-up").unwrap();
        ledger.send("child", &id, "Need clarification").unwrap();
        assert_eq!(ledger.messages("child").unwrap().len(), 1);
        assert_eq!(ledger.messages("parent").unwrap().len(), 1);
        assert!(ledger.messages("stranger").unwrap().is_empty());
        ledger.finish("child", &id, "Done; tests passed").unwrap();
        ledger.finish("child", &id, "Done; tests passed").unwrap();
        assert!(ledger.finish("child", &id, "Changed report").is_err());
        assert!(ledger.send("parent", &id, "late").is_err());
    }
    let ledger = Delegation::open(&path).unwrap();
    let task = ledger.status("parent", &id).unwrap();
    assert_eq!(task.state, "completed");
    assert_eq!(task.report.as_deref(), Some("Done; tests passed"));
    assert_eq!(ledger.reports("parent").unwrap().len(), 1);
    assert_eq!(ledger.reports("child").unwrap().len(), 1);
    assert_eq!(ledger.messages("child").unwrap().len(), 1);
    assert!(!ledger.reserve("parent", "repo", &request("one")).unwrap().1);
}
#[test]
fn restart_interrupts_uncertain_launch_without_retry_and_retains_bound_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("db");
    let (launch, running);
    {
        let ledger = Delegation::open(&path).unwrap();
        launch = ledger.reserve("parent", "repo", &request("launch")).unwrap().0.id;
        running = ledger.reserve("parent", "repo", &request("running")).unwrap().0.id;
        ledger.bind(&running, "child", "workspace").unwrap();
    }
    let ledger = Delegation::open(&path).unwrap();
    assert_eq!(ledger.status("parent", &launch).unwrap().state, "interrupted");
    let task = ledger.status("parent", &running).unwrap();
    assert_eq!(task.state, "interrupted");
    assert_eq!(task.session_id.as_deref(), Some("child"));
    assert!(!ledger.reserve("parent", "repo", &request("launch")).unwrap().1);
    assert!(!ledger.reserve("parent", "repo", &request("running")).unwrap().1);
    ledger.finish("child", &running, "Recovered by explicit user action").unwrap();
}
#[test]
fn repo_concurrency_and_delegation_chain_depth_are_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let ledger = Delegation::open(&tmp.path().join("db")).unwrap();
    for i in 0..MAX_ACTIVE {
        ledger.reserve(&format!("caller{i}"), "repo", &request(&i.to_string())).unwrap();
    }
    assert!(ledger.reserve("other", "repo", &request("blocked")).is_err());
    assert!(ledger.reserve("other", "different-repo", &request("allowed")).is_ok());
    let mut caller = "root".to_string();
    for i in 0..MAX_DEPTH {
        let task = ledger.reserve(&caller, &format!("depth-repo{i}"), &request(&i.to_string())).unwrap().0;
        let child = format!("level{i}");
        ledger.bind(&task.id, &child, "workspace").unwrap();

        caller = child;
    }
    assert!(ledger.reserve(&caller, "depth-repo", &request("too deep")).is_err());
}
#[test]
fn racing_completion_and_cancel_cannot_overwrite_first_result() { let tmp = tempfile::tempdir().unwrap();
let ledger = Arc::new(Delegation::open(&tmp.path().join("db")).unwrap());
let task = ledger.reserve("parent", "repo", &request("race")).unwrap().0;
ledger.bind(&task.id, "child", "workspace").unwrap();
let first = ledger.clone();
let id = task.id.clone();
let thread = std::thread::spawn(move || first.finish("child", &id, "Completed evidence"));
let cancel = ledger.terminal(&task.id, "cancelled", "Cancelled");
let finish = thread.join().unwrap();
assert_ne!(cancel.is_ok(), finish.is_ok());
let stored = ledger.status("parent", &task.id).unwrap();
assert!(stored.report.is_some());
assert_eq!(ledger.reports("parent").unwrap().len(), 1); }
#[test]
fn cancel_cascades_only_through_owned_tasks_and_cannot_restart_finished_sessions() {
 let tmp=tempfile::tempdir().unwrap(); let ledger=Delegation::open(&tmp.path().join("db")).unwrap();
 let root=ledger.reserve("parent","repo",&request("root")).unwrap().0; ledger.bind(&root.id,"child","ws").unwrap();
 let descendant=ledger.reserve("child","repo",&request("descendant")).unwrap().0; ledger.bind(&descendant.id,"grandchild","other-ws").unwrap();
 let unrelated=ledger.reserve("stranger","repo",&request("unrelated")).unwrap().0;
 let cancelled=ledger.cancel_tree(&root.id).unwrap(); assert_eq!(cancelled.len(),2);
 assert_eq!(ledger.get(&descendant.id).unwrap().state,"cancelled"); assert_eq!(ledger.get(&unrelated.id).unwrap().state,"launching");
 assert!(ledger.reserve("child","repo",&request("post-cancel")).is_err());
 assert!(ledger.finish("grandchild",&descendant.id,"Too late").is_err());
 assert!(ledger.cancel_tree(&root.id).unwrap().is_empty());
}
