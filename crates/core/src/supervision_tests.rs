use super::*;
use crate::{
    store::UsageRecord,
    usage::{price, ModelPrice},
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Barrier,
};

fn fixture() -> (tempfile::TempDir, Arc<Store>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("db.sqlite")).unwrap());
    store.conn.lock().execute_batch("INSERT INTO repos (id,name,path,default_branch,created_at) VALUES ('r','repo','/r','main',0),('other','other','/other','main',0);
        INSERT INTO workspaces (id,repo_id,name,branch,base_branch,path,status,created_at) VALUES ('w','r','workspace','b','main','/w','ready',0);
        INSERT INTO sessions (id,workspace_id,agent_id,title,created_at) VALUES ('s','w','claude','Root',0),('c','w','claude','Child',0),('g','w','claude','Grandchild',0),('unrelated','w','claude','Unrelated',0);").unwrap();
    (dir, store)
}
fn row(session: &str, model: &str, cost: Option<f64>) -> UsageRecord {
    UsageRecord {
        session_id: session.into(),
        workspace_id: "w".into(),
        repo_id: "r".into(),
        agent: "claude".into(),
        model: model.into(),
        ts: 1,
        input_tokens: 1_000_000,
        cached_tokens: 0,
        output_tokens: 0,
        cost_usd: cost,
    }
}

#[test]
fn admission_is_atomic_and_duplicate_starts_do_not_queue() {
    let (_dir, store) = fixture();
    store
        .save_repo_controls(
            "r",
            &RepoControls {
                max_concurrent: 2,
                ..Default::default()
            },
        )
        .unwrap();
    let admissions = Arc::new(Admissions::default());
    let start = Arc::new(Barrier::new(16));
    let hold = Arc::new(Barrier::new(16));
    let admitted = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|scope| {
        for n in 0..16 {
            let (store, admissions, start, hold, admitted) = (
                store.clone(),
                admissions.clone(),
                start.clone(),
                hold.clone(),
                admitted.clone(),
            );
            scope.spawn(move || {
                start.wait();
                let permit = admissions.admit(&store, &format!("s{n}"), "r");
                if permit.is_ok() {
                    admitted.fetch_add(1, Ordering::SeqCst);
                }
                hold.wait();
                drop(permit);
            });
        }
    });
    assert_eq!(admitted.load(Ordering::SeqCst), 2);
    let first = admissions.admit(&store, "s", "r").unwrap();
    assert!(admissions.admit(&store, "s", "r").is_err());
    let other = admissions.admit(&store, "elsewhere", "other").unwrap();
    drop((first, other));
    assert_eq!(admissions.running(&["s".into()].into()), 0);
    assert!(admissions.admit(&store, "s", "r").is_ok());
}

#[test]
fn durable_controls_validate_pause_and_observed_spend() {
    let (dir, store) = fixture();
    let controls = RepoControls {
        max_concurrent: 1,
        stop_usd: Some(2.0),
        warning_usd: Some(1.0),
        ..Default::default()
    };
    store.save_repo_controls("r", &controls).unwrap();
    store.add_usage(&row("s", "reported", Some(2.0))).unwrap();
    let admissions = Admissions::default();
    assert!(admissions
        .admit(&store, "s", "r")
        .unwrap_err()
        .to_string()
        .contains("spend"));
    let reopened = Store::open(&dir.path().join("db.sqlite")).unwrap();
    assert_eq!(reopened.repo_controls("r").unwrap(), controls);
    let paused = RepoControls {
        paused: true,
        ..Default::default()
    };
    reopened.save_repo_controls("r", &paused).unwrap();
    assert!(admissions.admit(&reopened, "s", "r").is_err());
    for bad in [
        RepoControls {
            max_concurrent: 0,
            ..Default::default()
        },
        RepoControls {
            stop_usd: Some(f64::NAN),
            ..Default::default()
        },
        RepoControls {
            stop_usd: Some(-1.0),
            ..Default::default()
        },
    ] {
        assert!(reopened.save_repo_controls("r", &bad).is_err());
    }
}

#[test]
fn inbox_requests_resolve_and_failures_are_deduplicated_per_turn() {
    let (_dir, store) = fixture();
    let question = Event::Question {
        session_id: "s".into(),
        request_id: "q1".into(),
        message: "Choose".into(),
        schema: serde_json::json!({}),
        tool_call_id: None,
        auto_resolve_ms: None,
    };
    assert_eq!(store.observe_supervision(&question).unwrap().len(), 1);
    assert!(store.observe_supervision(&question).unwrap().is_empty());
    assert_eq!(store.inbox().unwrap().len(), 1);
    store
        .observe_supervision(&Event::QuestionResolved {
            session_id: "s".into(),
            request_id: "q1".into(),
        })
        .unwrap();
    assert!(store.inbox().unwrap().is_empty());
    let fail = Event::SessionState {
        session_id: "s".into(),
        state: "error".into(),
        error: Some("boom".into()),
    };
    assert_eq!(store.observe_supervision(&fail).unwrap().len(), 1);
    assert!(store.observe_supervision(&fail).unwrap().is_empty());
    drop(Admissions::default().admit(&store, "s", "r").unwrap());
    store
        .observe_supervision(&Event::UserMessage {
            session_id: "s".into(),
            text: "retry".into(),
            ts: 1,
            images: vec![],
        })
        .unwrap();
    assert_eq!(store.observe_supervision(&fail).unwrap().len(), 1);
    assert_eq!(store.inbox().unwrap().len(), 1);
    store.observe_supervision(&question).unwrap();
    store.recover_inbox().unwrap();
    assert!(store.inbox().unwrap().iter().all(|i| i.kind == "failure"));
}

#[test]
fn notification_claim_is_atomic_and_survives_restart() {
    let (dir, store) = fixture();
    let claimed = Arc::new(AtomicUsize::new(0));
    std::thread::scope(|scope| {
        for _ in 0..20 {
            let store = store.clone();
            let claimed = claimed.clone();
            scope.spawn(move || {
                if store.claim_notification("same-event").unwrap() {
                    claimed.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
    });
    assert_eq!(claimed.load(Ordering::SeqCst), 1);
    let reopened = Store::open(&dir.path().join("db.sqlite")).unwrap();
    assert!(!reopened.claim_notification("same-event").unwrap());
}
#[test]
fn request_ids_can_repeat_after_reconnect_but_steering_keeps_the_generation() {
    let (_dir, store) = fixture();
    let admissions = Admissions::default();
    let question = Event::Question {
        session_id: "s".into(),
        request_id: "reused".into(),
        message: "Choose".into(),
        schema: serde_json::json!({}),
        tool_call_id: None,
        auto_resolve_ms: None,
    };
    let resolve = Event::QuestionResolved {
        session_id: "s".into(),
        request_id: "reused".into(),
    };
    let first = admissions.admit(&store, "s", "r").unwrap();
    store.observe_supervision(&question).unwrap();
    let first_id = store.inbox().unwrap()[0].id.clone();
    store
        .observe_supervision(&Event::UserMessage {
            session_id: "s".into(),
            text: "steer the active turn".into(),
            ts: 1,
            images: vec![],
        })
        .unwrap();
    store.observe_supervision(&resolve).unwrap();
    assert!(store.inbox().unwrap().is_empty());
    drop(first);
    let _second = admissions.admit(&store, "s", "r").unwrap();
    assert_eq!(store.observe_supervision(&question).unwrap().len(), 1);
    assert_ne!(store.inbox().unwrap()[0].id, first_id);
}
#[test]
fn spend_warning_and_delegation_report_are_durable_deduplicated_entries() {
    let (dir, store) = fixture();
    store
        .save_repo_controls(
            "r",
            &RepoControls {
                warning_usd: Some(1.0),
                ..Default::default()
            },
        )
        .unwrap();
    let u = row("s", "reported", Some(2.0));
    store.add_usage(&u).unwrap();
    let usage = Event::Usage {
        session_id: "s".into(),
        usage: price(u, &Default::default()),
    };
    assert_eq!(store.observe_supervision(&usage).unwrap().len(), 1);
    assert!(store.observe_supervision(&usage).unwrap().is_empty());
    assert!(store
        .push_inbox("s", "delegation:task", "delegation", "Done")
        .unwrap()
        .is_some());
    let reopened = Store::open(&dir.path().join("db.sqlite")).unwrap();
    assert!(reopened
        .push_inbox("s", "delegation:task", "delegation", "Done")
        .unwrap()
        .is_none());
    assert_eq!(reopened.inbox().unwrap().len(), 2);
    reopened.resolve_inbox("delegation:task").unwrap();
    assert!(reopened
        .push_inbox("s", "delegation:task", "delegation", "Replayed")
        .unwrap()
        .is_none());
    assert_eq!(reopened.inbox().unwrap().len(), 1);
}

#[test]
fn hierarchy_rollup_prices_each_original_row_once_after_reopen() {
    let (dir, store) = fixture();
    store.conn.lock().execute_batch("UPDATE sessions SET parent_session_id='s' WHERE id='c'; UPDATE sessions SET parent_session_id='c' WHERE id='g';").unwrap();
    for u in [
        row("s", "reported", Some(3.0)),
        row("c", "priced", None),
        row("g", "unknown", None),
        row("unrelated", "reported", Some(100.0)),
    ] {
        store.add_usage(&u).unwrap();
    }
    let pricing = [(
        "priced".into(),
        ModelPrice {
            input: 2.0,
            ..Default::default()
        },
    )]
    .into();
    for _ in 0..2 {
        let reopened = Store::open(&dir.path().join("db.sqlite")).unwrap();
        let ids = reopened.subtree_session_ids("s").unwrap().into_iter().collect();
        let total = ResourceUsage::sum(
            &ids,
            reopened
                .usage_since(0)
                .unwrap()
                .into_iter()
                .map(|u| price(u, &pricing))
                .collect(),
        );
        assert_eq!(total.sessions, 3);
        assert_eq!(total.turns, 3);
        assert_eq!(total.reported_usd, 3.0);
        assert_eq!(total.estimated_usd, 2.0);
        assert_eq!(total.unknown_turns, 1);
        assert_eq!(total.input_tokens, 3_000_000);
    }
}
