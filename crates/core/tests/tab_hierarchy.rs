use suneiro_core::{Repo, Session, Store, Workspace};

fn workspace(store: &Store, id: &str, repo: &str) {
    store
        .add_workspace(&Workspace {
            id: id.into(),
            repo_id: repo.into(),
            name: id.into(),
            branch: id.into(),
            base_branch: "main".into(),
            path: format!("/tmp/{id}"),
            status: "ready".into(),
            created_at: 1,
            title: String::new(),
            archived_at: None,
            unread: false,
        })
        .unwrap();
}

fn session(id: &str, ws: &str, parent: Option<&str>) -> Session {
    Session {
        id: id.into(),
        workspace_id: ws.into(),
        agent_id: "claude".into(),
        acp_session_id: None,
        title: id.into(),
        created_at: 1,
        model: None,
        effort: None,
        parent_session_id: parent.map(str::to_owned),
    }
}

fn fixture(path: &std::path::Path) -> Store {
    let store = Store::open(path).unwrap();
    for id in ["r", "other"] {
        store.add_repo(&Repo { id: id.into(), name: id.into(), path: format!("/tmp/{id}"), default_branch: "main".into() }).unwrap();
    }
    workspace(&store, "w", "r");
    workspace(&store, "isolated", "r");
    workspace(&store, "foreign", "other");
    store
}

#[test]
fn legacy_migration_and_restart_preserve_chats() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, agent_id TEXT NOT NULL, acp_session_id TEXT, title TEXT NOT NULL, created_at INTEGER NOT NULL); INSERT INTO sessions VALUES ('legacy', 'w', 'claude', 'acp', 'Old conversation', 1);").unwrap();
    drop(conn);
    let store = fixture(&path);
    let legacy = store.session("legacy").unwrap();
    assert!(legacy.parent_session_id.is_none());
    assert_eq!(legacy.acp_session_id.as_deref(), Some("acp"));
    store.add_session_with_depth(&session("child", "isolated", Some("legacy")), 4).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.session("legacy").unwrap().title, "Old conversation");
    assert_eq!(store.session("child").unwrap().parent_session_id.as_deref(), Some("legacy"));
    assert_eq!(store.subtree_session_ids("legacy").unwrap().len(), 2);
}

#[test]
fn depth_repo_missing_parent_and_cycles_reject_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("db"));
    store.add_session(&session("root", "w", None)).unwrap();
    store.add_session_with_depth(&session("child", "isolated", Some("root")), 2).unwrap();
    store.add_session_with_depth(&session("grandchild", "w", Some("child")), 2).unwrap();
    for (id, ws, parent, limit) in [
        ("overflow", "w", "grandchild", 2),
        ("cross", "foreign", "root", 4),
        ("missing", "w", "absent", 4),
        ("cycle", "w", "cycle", 4),
        ("invalid", "w", "root", 9),
    ] {
        assert!(store.add_session_with_depth(&session(id, ws, Some(parent)), limit).is_err(), "{id}");
        assert!(store.session(id).is_err(), "failed insert must leave no row");
    }
    store.detach_session("child").unwrap();
    assert_eq!(store.subtree_session_ids("root").unwrap(), ["root"]);
    store.add_session_with_depth(&session("now-fits", "w", Some("grandchild")), 2).unwrap();
    assert!(store.detach_session("absent").is_err());
}

#[test]
fn closing_parent_detaches_children_not_grandchildren_and_keeps_last_chat() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let store = fixture(&path);
    for s in
        [session("root", "w", None), session("spare", "w", None), session("child", "isolated", Some("root")), session("grandchild", "isolated", Some("child"))]
    {
        store.add_session(&s).unwrap();
    }
    let detached = store.delete_session("root").unwrap();
    assert_eq!(detached, ["child"]);
    assert!(store.session("root").is_err());
    assert!(store.session("child").unwrap().parent_session_id.is_none());
    assert_eq!(store.session("grandchild").unwrap().parent_session_id.as_deref(), Some("child"));
    assert!(store.delete_session("spare").is_err());
    assert!(store.session("spare").is_ok());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(store.session("child").unwrap().parent_session_id.is_none());
    assert_eq!(store.subtree_session_ids("child").unwrap().len(), 2);
}

#[test]
fn settings_default_is_compatible_and_bounded() {
    let settings: suneiro_core::setup::Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(settings.max_tab_depth, 4);
    let dir = tempfile::tempdir().unwrap();
    let core = suneiro_core::Core::new(dir.path(), std::sync::Arc::new(|_| {})).unwrap();
    let mut settings = core.settings();
    for depth in [0, 9] {
        settings.max_tab_depth = depth;
        assert!(core.save_settings(&settings).is_err());
    }
    settings.max_tab_depth = 8;
    core.save_settings(&settings).unwrap();
    assert_eq!(core.settings().max_tab_depth, 8);
}
