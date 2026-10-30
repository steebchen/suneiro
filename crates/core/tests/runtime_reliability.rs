use std::{sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde_json::json;
use suneiro_core::{AgentDef, Core, Event, Repo, Workspace};

const AGENT: &str = r#"
const mode=process.argv[1];
const fs=require('fs');
const opts=[{id:'model',category:'model',currentValue:'fast',options:[{value:'fast',name:'Fast'},{value:'smart',name:'Smart'}]},{id:'effort',category:'thought_level',currentValue:'low',options:[{value:'low',name:'Low'},{value:'high',name:'High'}]}];
const send=m=>console.log(JSON.stringify({jsonrpc:'2.0',...m}));
require('readline').createInterface({input:process.stdin}).on('line',line=>{
 const m=JSON.parse(line), p=m.params||{};
 const result=r=>send({id:m.id,result:r});
 switch(m.method){
 case 'initialize':
  fs.writeFileSync('started','yes');
  if(mode==='hang-init') return;
  return result({protocolVersion:1,agentCapabilities:{loadSession:mode!=='unsupported'}});
 case 'session/new':
  fs.appendFileSync('new-sessions','new\n');
  return result({sessionId:'saved-1',configOptions:opts});
 case 'session/load':
  if(mode==='resume-error') return send({id:m.id,error:{code:-32001,message:'saved conversation unavailable'}});
  return result({configOptions:opts});
 case 'session/set_config_option':
  opts.find(o=>o.id===p.configId).currentValue=p.value;
  return result({configOptions:opts});
 case 'session/prompt':
  fs.appendFileSync('prompts','prompt\n');
  if(mode==='ignore-cancel') return;
  for(let i=0;i<100;i++) send({method:'session/update',params:{sessionId:'saved-1',update:{sessionUpdate:'usage_update',cost:{amount:(i+1)/100,currency:'USD'}}}});
  return result({stopReason:'end_turn',usage:{inputTokens:3,outputTokens:2}});
 case 'session/cancel': return;
 default: if(m.id) return result({});
 }
});
setInterval(()=>{},1000);
"#;

type Log = Arc<Mutex<Vec<Event>>>;

fn core(mode: &str) -> (tempfile::TempDir, Arc<Core>, Log, String) {
    std::env::set_var("SUNEIRO_NO_AI_TITLES", "1");
    let tmp = tempfile::tempdir().unwrap();
    let log = Arc::new(Mutex::new(Vec::new()));
    let events = log.clone();
    let core = Core::new(tmp.path(), Arc::new(move |e| events.lock().push(e))).unwrap();
    core.agents.register(AgentDef {
        id: "fault".into(),
        name: "Fault".into(),
        command: "node".into(),
        args: vec!["-e".into(), AGENT.into(), mode.into()],
    });
    core.store
        .add_repo(&Repo {
            id: "r".into(),
            name: "r".into(),
            path: tmp.path().display().to_string(),
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
            path: tmp.path().display().to_string(),
            status: "ready".into(),
            created_at: 0,
            title: String::new(),
            archived_at: None,
            unread: false,
        })
        .unwrap();
    // Add directly so tests can install a saved ACP id before warm-up.
    let session = suneiro_core::Session {
        id: "s".into(),
        workspace_id: "w".into(),
        agent_id: "fault".into(),
        acp_session_id: None,
        title: "Runtime test".into(),
        created_at: 0,
        model: None,
        effort: None,
        parent_session_id: None,
    };
    core.store.add_session(&session).unwrap();
    (tmp, core, log, session.id)
}

async fn until(pred: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !pred() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("timed out waiting for runtime state");
}

#[tokio::test]
async fn failed_or_unsupported_resume_never_starts_a_new_conversation() {
    for mode in ["resume-error", "unsupported"] {
        let (tmp, core, log, sid) = core(mode);
        core.store
            .set_acp_session_id(&sid, "previous-conversation")
            .unwrap();
        core.agents
            .prompt(&sid, "do not silently restart".into())
            .unwrap();
        until(|| log.lock().iter().any(|e| matches!(e, Event::SessionState { state, error: Some(message), .. } if state == "error" && message.contains("conversation")))).await;
        assert_eq!(
            core.store.session(&sid).unwrap().acp_session_id.as_deref(),
            Some("previous-conversation")
        );
        assert!(!tmp.path().join("new-sessions").exists());
        assert!(!tmp.path().join("prompts").exists());
        assert!(!core.agents.is_running(&sid));
        core.shutdown();
    }
}

#[tokio::test]
async fn config_response_without_notification_updates_persistent_selection() {
    let (_tmp, core, _log, sid) = core("config");
    core.agents
        .set_config(&sid, "model", json!("smart"))
        .await
        .unwrap();
    core.agents
        .set_config(&sid, "effort", json!("high"))
        .await
        .unwrap();
    let session = core.store.session(&sid).unwrap();
    assert_eq!(session.model.as_deref(), Some("smart"));
    assert_eq!(session.effort.as_deref(), Some("high"));
    core.shutdown();
}

#[tokio::test]
async fn usage_notifications_are_drained_before_recording_turn() {
    let (_tmp, core, log, sid) = core("config");
    core.agents.prompt(&sid, "measure".into()).unwrap();
    until(|| {
        log.lock()
            .iter()
            .any(|e| matches!(e, Event::TurnEnd { .. }))
    })
    .await;
    let usage = core.store.usage_for_session(&sid).unwrap();
    assert_eq!(usage.len(), 1);
    assert!((usage[0].cost_usd.unwrap() - 1.0).abs() < 1e-9);
    core.shutdown();
}

#[tokio::test]
async fn close_during_hung_handshake_stops_the_original_prompt() {
    let (tmp, core, _log, sid) = core("hang-init");
    core.agents.prompt(&sid, "must not execute".into()).unwrap();
    until(|| tmp.path().join("started").exists()).await;
    core.agents.close(&sid);
    until(|| !core.agents.is_running(&sid)).await;
    assert!(!tmp.path().join("prompts").exists());
    core.shutdown();
}

#[tokio::test]
async fn cancel_during_hung_handshake_does_not_wait_for_startup_timeout() {
    let (tmp, core, log, sid) = core("hang-init");
    core.agents.prompt(&sid, "must not execute".into()).unwrap();
    until(|| tmp.path().join("started").exists()).await;
    tokio::time::timeout(Duration::from_secs(2), core.agents.cancel(&sid))
        .await
        .unwrap()
        .unwrap();
    until(|| !core.agents.is_running(&sid)).await;
    assert!(log
        .lock()
        .iter()
        .any(|e| matches!(e, Event::SessionState { state, .. } if state == "error")));
    assert!(!tmp.path().join("prompts").exists());
    core.shutdown();
}

#[tokio::test]
async fn cancellation_kills_an_uncooperative_agent_without_replaying_prompt() {
    let (tmp, core, _log, sid) = core("ignore-cancel");
    core.agents.prompt(&sid, "only once".into()).unwrap();
    until(|| tmp.path().join("prompts").exists()).await;
    tokio::time::timeout(Duration::from_secs(8), core.agents.cancel(&sid))
        .await
        .unwrap()
        .unwrap();
    until(|| !core.agents.is_running(&sid)).await;
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("prompts")).unwrap(),
        "prompt\n"
    );
    core.shutdown();
}
