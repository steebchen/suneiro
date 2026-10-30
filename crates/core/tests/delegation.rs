#![cfg(unix)]
#[path = "support/delegation_evidence.rs"]
mod evidence;
use evidence::{reports_evidence, reports_permission, SmokeShutdown};
use std::{path::Path, sync::Arc, time::Duration};
use suneiro_core::{
    delegation::{Isolation, Relation, TaskBase, TaskRequest},
    env::tokio_command,
    git, AgentDef, Core,
};

async fn fixture(hold: bool) -> (tempfile::TempDir, Arc<Core>, String) {
    std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "commit.gpgsign", "false"],
        &["config", "user.email", "test@example.com"],
        &["config", "user.name", "Test"],
        &["commit", "--allow-empty", "-qm", "init"],
    ] {
        git::git(&repo, args).await.unwrap();
    }
    let core = Core::new(&tmp.path().join("data"), Arc::new(|_| {})).unwrap();
    core.set_delegation_executable(Path::new(env!("CARGO_BIN_EXE_suneiro-delegation-mcp"))).unwrap();
    let mut args = vec![concat!(env!("CARGO_MANIFEST_DIR"), "/tests/delegation_agent.mjs").into()];
    if hold {
        args.push("--hold".into());
    }
    core.agents.register(AgentDef { id: "fixture".into(), name: "MCP fixture".into(), command: "node".into(), args });
    let mut settings = core.settings();
    settings.workspaces_root = tmp.path().join("ws").to_string_lossy().into();
    settings.rename_branches = false;
    core.save_settings(&settings).unwrap();
    let repo = core.add_repo(repo.to_str().unwrap()).await.unwrap();
    let ws = core.create_workspace(&repo.id).await.unwrap();
    wait(|| core.store.workspace(&ws.id).unwrap().status == "ready").await;
    let session = core.create_session(&ws.id, "fixture", None, None).unwrap();
    (tmp, core, session.id)
}
async fn wait(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition timed out");
}
fn request(key: &str) -> TaskRequest {
    TaskRequest {
        request_key: key.into(),
        relation: Relation::Child,
        isolation: Isolation::Isolated,
        base: TaskBase::CallerHead,
        agent_id: "fixture".into(),
        model: None,
        effort: None,
        brief: "Inspect @secret.txt literally; do not open it".into(),
        acceptance_criteria: "Report through real MCP stdio bridge".into(),
        selected_context: "Explicit selected context only".into(),
    }
}
#[tokio::test]
async fn actual_acp_mcp_spawn_inherits_committed_head_not_dirty_files_and_reports_once() {
    let (_tmp, core, parent) = fixture(false).await;
    let parent_session = core.store.session(&parent).unwrap();
    let source = core.store.workspace(&parent_session.workspace_id).unwrap();
    let path = Path::new(&source.path);
    std::fs::write(path.join("parent-commit.txt"), "committed parent context").unwrap();
    git::commit_all(path, "parent commit").await.unwrap();
    std::fs::write(path.join("secret.txt"), "MUST NOT BE IMPLICITLY ATTACHED").unwrap();
    let head = git::git(path, &["rev-parse", "HEAD"]).await.unwrap();
    let task = core.delegate(&parent, request("stable")).await.unwrap();
    let child = task.session_id.clone().unwrap();
    let target = core.store.workspace(task.workspace_id.as_ref().unwrap()).unwrap();
    assert_ne!(target.id, source.id);
    assert_ne!(target.path, source.path);
    assert_eq!(core.store.session(&child).unwrap().parent_session_id.as_deref(), Some(parent.as_str()));
    assert_eq!(core.store.sessions(&target.id).unwrap().len(), 1, "no hidden warmed-up default chat");
    wait(|| core.delegation.status(&parent, &task.id).unwrap().report.is_some()).await;
    assert_eq!(git::git(Path::new(&target.path), &["rev-parse", "HEAD"]).await.unwrap(), head);
    assert!(Path::new(&target.path).join("parent-commit.txt").is_file());
    assert!(!Path::new(&target.path).join("secret.txt").exists());
    let blocks: serde_json::Value =
        serde_json::from_slice(&std::fs::read(Path::new(&target.path).join("prompt-observed.json")).unwrap()).unwrap();
    assert_eq!(blocks.as_array().unwrap().len(), 1);
    assert_eq!(blocks[0]["type"], "text");
    assert!(!blocks.to_string().contains("MUST NOT BE IMPLICITLY ATTACHED"));
    let duplicate = core.delegate(&parent, request("stable")).await.unwrap();
    assert_eq!(duplicate.id, task.id);
    assert_eq!(core.delegation.reports(&parent).unwrap().len(), 1);
    assert_eq!(core.store.sessions(&target.id).unwrap().len(), 1);
    assert_eq!(core.delegation.status(&parent, &task.id).unwrap().state, "completed");
    core.shutdown();
}
#[tokio::test]
async fn explicit_shared_workspace_sibling_messages_cancel_and_restart_do_not_reprompt() {
    let (tmp, core, parent) = fixture(true).await;
    let mut req = request("same");
    req.isolation = Isolation::SameWorkspace;
    req.relation = Relation::Sibling;
    let task = core.delegate(&parent, req.clone()).await.unwrap();
    let child = task.session_id.clone().unwrap();
    assert_eq!(task.workspace_id.as_deref(), Some(core.store.session(&parent).unwrap().workspace_id.as_str()));
    assert!(core.store.session(&child).unwrap().parent_session_id.is_none());
    let path = core.store.workspace(task.workspace_id.as_ref().unwrap()).unwrap().path;
    wait(|| Path::new(&path).join("prompt-observed.json").exists()).await;
    core.send_delegation_message(&parent, &task.id, "Review before acting").unwrap();
    assert_eq!(core.delegation.messages(&child).unwrap().len(), 1);
    assert!(core.cancel_delegation(&child, &task.id).await.is_err());
    core.cancel_delegation(&parent, &task.id).await.unwrap();
    assert!(!core.agents.is_running(&child));
    core.shutdown();
    drop(core);
    let core = Core::new(&tmp.path().join("data"), Arc::new(|_| {})).unwrap();
    let task_after = core.delegation.status(&parent, &task.id).unwrap();
    assert_eq!(task_after.state, "cancelled");
    assert_eq!(task_after.report.as_deref(), Some("Cancelled by task creator"));
    assert!(!core.agents.is_running(&child));
    assert_eq!(core.delegation.messages(&child).unwrap().len(), 1);
    core.shutdown();
}
#[tokio::test]
async fn stdio_mcp_discovery_and_missing_auth_fail_honestly() {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut child = tokio_command(env!("CARGO_BIN_EXE_suneiro-delegation-mcp"))
        .env_remove("SUNEIRO_DELEGATION_SOCKET")
        .env_remove("SUNEIRO_DELEGATION_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    input.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n").await.unwrap();
    let response: serde_json::Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 7);
    input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"suneiro_reports\",\"arguments\":{}}}\n")
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(response["result"]["isError"], true);
    drop(input);
    assert!(child.wait().await.unwrap().success());
}
#[tokio::test]
#[ignore = "opt-in: uses an authenticated real agent and its tokens; SUNEIRO_DELEGATION_AGENT=claude|codex|opencode"]
async fn real_adapter_can_use_local_mcp_tools() {
    use std::sync::Mutex;
    let agent = std::env::var("SUNEIRO_DELEGATION_AGENT").expect("explicit real agent selection required");
    assert!(matches!(agent.as_str(), "codex" | "opencode" | "claude"), "unknown real agent");
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("empty-fixture");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "commit.gpgsign", "false"],
        &["config", "user.email", "test@example.com"],
        &["config", "user.name", "Fixture"],
        &["commit", "--allow-empty", "-qm", "fixture"],
    ] {
        git::git(&repo, args).await.unwrap();
    }
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let core = Core::new(&tmp.path().join("data"), Arc::new(move |event| sink.lock().unwrap().push(event))).unwrap();
    let _shutdown = SmokeShutdown(core.clone());
    core.set_delegation_executable(Path::new(env!("CARGO_BIN_EXE_suneiro-delegation-mcp"))).unwrap();
    let mut settings = core.settings();
    settings.plan_by_default = true;
    settings.rename_branches = false;
    settings.workspaces_root = tmp.path().join("workspaces").to_string_lossy().into();
    core.save_settings(&settings).unwrap();
    let repository = core.add_repo(repo.to_str().unwrap()).await.unwrap();
    let ws = core.create_workspace(&repository.id).await.unwrap();
    wait(|| core.store.workspace(&ws.id).unwrap().status == "ready").await;
    let model = std::env::var("SUNEIRO_DELEGATION_MODEL").ok();
    let effort = std::env::var("SUNEIRO_DELEGATION_EFFORT").ok();
    let session = core.create_session(&ws.id, &agent, model, effort).unwrap();
    core.agents.prompt_brief(&session.id,"Read-only MCP qualification in a disposable empty repository. Call exactly the suneiro_reports tool from the suneiro-delegation MCP server once, with no arguments. Do not use shell, filesystem, network, planning, delegation creation, or any other tool. If that tool succeeds, reply exactly BRIDGE_REPORTS_OK. If unavailable or failed, reply BRIDGE_REPORTS_UNAVAILABLE and briefly explain; never emit the success marker without actually calling it.".into()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(150), async {
        let mut cursor = 0;
        let mut approved = false;
        loop {
            let pending = {
                let log = events.lock().unwrap();
                let batch = log[cursor..].to_vec();
                cursor = log.len();
                batch
            };
            for event in pending {
                match event {
                    suneiro_core::Event::PermissionRequest { session_id, request_id, tool_call, .. } => {
                        eprintln!(
                            "permission method=session/request_permission tool={}",
                            tool_call.get("title").unwrap_or(&serde_json::Value::Null)
                        );
                        let option = {
                            let log = events.lock().unwrap();
                            if !approved && session_id == session.id && reports_permission(&log, &session.id, &tool_call) {
                                log.iter().find_map(|event| match event {
                                    suneiro_core::Event::PermissionRequest { session_id: owner, request_id: id, options, .. }
                                        if owner == &session.id && id == &request_id =>
                                    {
                                        options
                                            .as_array()
                                            .and_then(|options| options.iter().find(|option| option["kind"] == "allow_once"))
                                            .and_then(|option| option["optionId"].as_str())
                                            .map(str::to_string)
                                    }
                                    _ => None,
                                })
                            } else {
                                None
                            }
                        };
                        approved |= option.is_some();
                        eprintln!("permission scoped_reports_allow_once={}", option.is_some());
                        core.agents.respond_permission(&session_id, &request_id, option).await.unwrap();
                    }
                    suneiro_core::Event::Question { session_id, request_id, .. } => {
                        core.agents.answer_question(&session_id, &request_id, serde_json::json!({"action":"cancel"})).await.unwrap();
                    }
                    _ => {}
                }
            }
            if !core.agents.is_running(&session.id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let log = events.lock().unwrap().clone();
    for event in &log {
        match event {
            suneiro_core::Event::SessionUpdate { session_id, update } if session_id == &session.id => {
                let kind = update["sessionUpdate"].as_str().unwrap_or("");
                if matches!(kind, "tool_call" | "tool_call_update" | "agent_message_chunk") {
                    eprintln!("ACP method=session/update update={update}");
                }
            }
            suneiro_core::Event::SessionState { session_id, state, error } if session_id == &session.id => {
                eprintln!("state={state} error={error:?}")
            }
            _ => {}
        }
    }
    assert!(result.is_ok(), "bounded real MCP qualification timed out; no retry");
    let (tool, marker) = reports_evidence(&log, &session.id);
    assert!(tool, "missing completed ACP tool_call/rawInput identity for suneiro_reports; plain text is not evidence");
    assert!(marker, "missing exact BRIDGE_REPORTS_OK assistant result");
}
