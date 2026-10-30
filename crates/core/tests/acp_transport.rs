use std::{sync::Arc, time::Duration};

use serde_json::json;
use suneiro_core::acp::{AcpConnection, Incoming};
use tokio::sync::mpsc;

fn spawn(script: &str) -> (Arc<AcpConnection>, mpsc::UnboundedReceiver<Incoming>) {
    AcpConnection::spawn(
        "node",
        &["-e".into(), script.into()],
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
    )
    .unwrap()
}

#[tokio::test]
async fn bounded_requests_fail_and_disconnect_without_replay() {
    let (conn, mut incoming) = spawn("let n=0; process.stdin.on('data', () => { n++; if(n>1) process.exit(99); }); setInterval(() => {}, 1000)");
    let error = conn
        .request_with_timeout("initialize", json!({}), Some(Duration::from_millis(200)))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("initialize"));
    assert!(format!("{error:#}").contains("timed out"));
    assert!(!conn.is_alive());
    assert!(matches!(
        incoming.recv().await,
        Some(Incoming::Closed { .. })
    ));
    assert!(conn.request("session/new", json!({})).await.is_err());
}

#[tokio::test]
async fn kill_wakes_all_pending_requests() {
    let (conn, _) = spawn("process.stdin.resume(); setInterval(() => {}, 1000)");
    let a = conn.request("session/prompt", json!({}));
    let b = conn.request("session/prompt", json!({}));
    let kill = async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        conn.kill();
    };
    let (a, b, ()) =
        tokio::time::timeout(Duration::from_secs(3), async { tokio::join!(a, b, kill) })
            .await
            .unwrap();
    assert!(a.is_err() && b.is_err());
}

#[tokio::test]
async fn split_frames_and_ordered_notifications_are_supported() {
    let (conn, mut incoming) = spawn("process.stdin.once('data', b => { const m=JSON.parse(b); const n=JSON.stringify({jsonrpc:'2.0',method:'session/update',params:{n:1}})+'\\n'; process.stdout.write(n.slice(0,9)); setTimeout(() => { process.stdout.write(n.slice(9)); console.log(JSON.stringify({jsonrpc:'2.0',id:m.id,result:{ok:true}})); },100); }); setInterval(() => {},1000)");
    let r = conn.request("session/prompt", json!({})).await.unwrap();
    assert_eq!(r["ok"], true);
    let consumer = async {
        assert!(matches!(
            incoming.recv().await,
            Some(Incoming::Notification { .. })
        ));
        match incoming.recv().await {
            Some(Incoming::Barrier(done)) => {
                let _ = done.send(());
            }
            other => panic!("expected barrier, got {other:?}"),
        }
    };
    let (drained, ()) = tokio::join!(conn.drain(), consumer);
    drained.unwrap();
    conn.kill();
}

#[tokio::test]
async fn partial_frame_at_exit_is_an_error() {
    let (conn, _) = spawn("process.stdin.once('data', () => { process.stdout.write('{\\\"jsonrpc\\\":'); process.exit(7); });");
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        conn.request("session/prompt", json!({})),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    assert!(!conn.is_alive());
}

#[tokio::test]
async fn process_exit_is_detected_even_if_descendant_holds_stdout_open() {
    let (conn, _) = spawn("require('child_process').spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:['ignore',1,2]}); process.stdin.once('data',()=>{ console.error('login expired'); process.exit(23); });");
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        conn.request("initialize", json!({})),
    )
    .await
    .unwrap();
    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains("exited"), "{error}");
    assert!(error.contains("login expired"), "{error}");
    assert!(!conn.is_alive());
}

#[tokio::test]
async fn oversized_partial_frame_is_bounded() {
    let (conn, _) = spawn("process.stdin.once('data',()=>process.stdout.write('x'.repeat(17*1024*1024))); setInterval(()=>{},1000)");
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        conn.request("initialize", json!({})),
    )
    .await
    .unwrap();
    assert!(format!("{:#}", result.unwrap_err()).contains("exceeds"));
}

#[tokio::test]
async fn rpc_errors_preserve_code_and_safe_diagnostics_only() {
    let (conn, _) = spawn("process.stdin.once('data', b => { const m=JSON.parse(b); console.log(JSON.stringify({jsonrpc:'2.0',id:m.id,error:{code:-32603,message:'OpenCode service failure',data:{error:{name:'ProviderError',statusCode:401,token:'super-secret'},authorization:'Bearer super-secret',body:'sk-private',message:'password=private'}}})); }); setInterval(()=>{},1000)");
    let error = format!(
        "{:#}",
        conn.request("initialize", json!({})).await.unwrap_err()
    );
    assert!(
        error.contains("-32603") && error.contains("ProviderError") && error.contains("401"),
        "{error}"
    );
    assert!(
        !error.contains("super-secret")
            && !error.contains("sk-private")
            && !error.contains("password=private"),
        "{error}"
    );
    conn.kill();
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_connection_kills_wrapper_descendants() {
    let (conn, mut incoming) = spawn("const child=require('child_process').spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'}); console.log(JSON.stringify({jsonrpc:'2.0',method:'pid',params:{parent:process.pid,child:child.pid}})); setInterval(()=>{},1000)");
    let Some(Incoming::Notification { params, .. }) =
        tokio::time::timeout(Duration::from_secs(3), incoming.recv())
            .await
            .unwrap()
    else {
        panic!("expected process ids")
    };
    drop(conn);
    for key in ["parent", "child"] {
        let pid = params[key].as_u64().unwrap().to_string();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let exists = suneiro_core::env::std_command("kill")
                    .args(["-0", &pid])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success();
                if !exists {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("agent process or descendant survived connection drop");
    }
}

#[tokio::test]
async fn kill_immediately_marks_connection_dead() {
    let (conn, _) = spawn("setInterval(() => {}, 1000)");
    conn.kill();
    assert!(!conn.is_alive(), "kill must close transport synchronously");
}

#[tokio::test]
async fn malformed_response_is_not_success() {
    let (conn, _) = spawn("process.stdin.once('data', b => { const m = JSON.parse(b); console.log(JSON.stringify({jsonrpc:'2.0',id:m.id})); })");
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        conn.request("initialize", json!({})),
    )
    .await
    .unwrap();
    assert!(
        result.is_err(),
        "a response without result or error cannot succeed: {result:?}"
    );
    conn.kill();
}

#[tokio::test]
async fn malformed_output_fails_waiters_instead_of_hanging() {
    let (conn, _) = spawn(
        "process.stdin.once('data', () => console.log('not json')); setInterval(() => {}, 1000)",
    );
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        conn.request("initialize", json!({})),
    )
    .await;
    conn.kill();
    assert!(
        result.is_ok(),
        "malformed protocol output left request hanging"
    );
    assert!(result.unwrap().is_err());
}
