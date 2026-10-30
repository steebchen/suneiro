use super::*;
#[cfg(unix)]
#[tokio::test]
async fn socket_is_private_tokens_rotate_and_caller_cannot_be_overridden() {
    use std::{os::unix::fs::PermissionsExt, sync::Arc};
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::UnixStream,
    };
    let tmp = tempfile::tempdir().unwrap();
    let core = crate::Core::new(tmp.path(), Arc::new(|_| {})).unwrap();
    // Store sessions without launching an agent; only their IDs authenticate tools.
    let repo =
        crate::Repo { id: "repo".into(), name: "repo".into(), path: tmp.path().to_string_lossy().into(), default_branch: "main".into() };
    core.store.add_repo(&repo).unwrap();
    let ws = crate::Workspace {
        id: "ws".into(),
        repo_id: repo.id,
        name: "ws".into(),
        branch: "test".into(),
        base_branch: "main".into(),
        path: tmp.path().to_string_lossy().into(),
        status: "ready".into(),
        created_at: 0,
        title: String::new(),
        archived_at: None,
        unread: false,
    };
    core.store.add_workspace(&ws).unwrap();
    for id in ["first", "other"] {
        core.store
            .add_session(&crate::Session {
                id: id.into(),
                workspace_id: ws.id.clone(),
                agent_id: "fixture".into(),
                parent_session_id: None,
                acp_session_id: None,
                title: String::new(),
                created_at: 0,
                model: None,
                effort: None,
            })
            .unwrap();
    }
    let db = rusqlite::Connection::open(crate::storage::database_path(tmp.path())).unwrap();
    let spec =
        json!({"requestKey":"other-key","agentId":"fixture","brief":"private task","acceptanceCriteria":"private criteria"}).to_string();
    db.execute("INSERT INTO delegated_tasks(id,caller,repo,request_key,request,state,report,created_at) VALUES('foreign','other','repo','other-key',?1,'completed','PRIVATE REPORT',0)",[spec]).unwrap();
    core.set_delegation_executable(&std::env::current_exe().unwrap()).unwrap();
    let config = core.delegation_servers("first")[0].clone();
    let socket = config["env"][0]["value"].as_str().unwrap();
    let old = config["env"][1]["value"].as_str().unwrap();
    assert_eq!(std::fs::metadata(socket).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(Path::new(socket).parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    async fn request(socket: &str, token: &str) -> Value {
        let stream = UnixStream::connect(socket).await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut bytes = serde_json::to_vec(
            &json!({"token":token,"sessionId":"other","params":{"name":"suneiro_reports","arguments":{"sessionId":"other"}}}),
        )
        .unwrap();
        bytes.push(b'\n');
        write.write_all(&bytes).await.unwrap();
        let mut line = String::new();
        BufReader::new(read).read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap()
    }
    assert_eq!(request(socket, "invalid").await["error"], "unauthorized delegation capability");
    let scoped = request(socket, old).await;
    assert_eq!(scoped["result"]["items"], json!([]));
    assert!(!scoped.to_string().contains("PRIVATE REPORT"));
    let next = core.delegation_servers("first");
    let new = next[0]["env"][1]["value"].as_str().unwrap();
    assert_ne!(old, new);
    assert_eq!(request(socket, old).await["error"], "unauthorized delegation capability");
    assert!(request(socket, new).await.get("result").is_some());
    core.agents.close("first");
    assert_eq!(request(socket, new).await["error"], "unauthorized delegation capability");
    core.shutdown();
}
#[test]
fn tool_schemas_expose_real_operations_and_no_arbitrary_paths_or_session_selectors() {
    let tools = tools();
    assert_eq!(tools.len(), 7);
    for tool in tools {
        let properties = &tool["inputSchema"]["properties"];
        assert!(properties.get("sessionId").is_none());
        assert!(properties.get("workspacePath").is_none());
    }
}
