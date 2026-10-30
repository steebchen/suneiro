//! Opt-in installed-CLI checks. Discovery sends no model prompt; prompting may
//! incur provider charges and requires SUNEIRO_OPENCODE_MODEL explicitly.
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, ensure, Context, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use suneiro_core::acp::{AcpConnection, Incoming, PROTOCOL_VERSION};

struct Smoke {
    conn: Arc<AcpConnection>,
    init: Value,
    session: Value,
    updates: Arc<Mutex<Vec<Value>>>,
    questions: Arc<Mutex<usize>>,
    _dir: tempfile::TempDir,
}

impl Drop for Smoke {
    fn drop(&mut self) {
        self.conn.kill();
    }
}

#[tokio::test]
#[ignore = "requires SUNEIRO_OPENCODE_MODEL; native structured questions are not assumed supported"]
async fn native_opencode_questions() -> Result<()> {
    let model = std::env::var("SUNEIRO_OPENCODE_MODEL").context("set SUNEIRO_OPENCODE_MODEL")?;
    let smoke = Smoke::start().await?;
    suneiro_core::opencode::validate_control(&smoke.session["configOptions"], "model", &json!(model))?;
    request(
        &smoke.conn,
        "session/set_config_option",
        json!({"sessionId":smoke.session["sessionId"],"configId":"model","value":model}),
    )
    .await?;
    smoke.prompt(json!([{"type":"text","text":"Use the question tool to ask me exactly one multiple-choice question about whether I prefer blue or red before doing anything else. Do not use any other tools. If the question tool is unavailable, say so; do not simulate a tool."}])).await?;
    let count = *smoke.questions.lock();
    ensure!(count > 0, "UNAVAILABLE/UNVERIFIED: OpenCode/model emitted no ACP elicitation despite an explicit question-tool prompt; plain text is not a structured question");
    eprintln!(
        "REAL structured ACP questions observed={count}; smoke client cancelled them rather than guessing answers"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires SUNEIRO_OPENCODE_MODEL with vision; may incur charges"]
async fn native_opencode_images() -> Result<()> {
    let model = std::env::var("SUNEIRO_OPENCODE_MODEL")
        .context("set SUNEIRO_OPENCODE_MODEL to a vision-capable provider/model")?;
    let smoke = Smoke::start().await?;
    ensure!(
        smoke.init["agentCapabilities"]["promptCapabilities"]["image"] == true,
        "UNAVAILABLE: agent did not negotiate image prompts"
    );
    suneiro_core::opencode::validate_control(&smoke.session["configOptions"], "model", &json!(model))?;
    request(
        &smoke.conn,
        "session/set_config_option",
        json!({"sessionId":smoke.session["sessionId"],"configId":"model","value":model}),
    )
    .await?;
    let result = smoke.prompt(json!([
        {"type":"text","text":"Describe the color of this image briefly; do not use tools."},
        {"type":"image","mimeType":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAIAAAD8GO2jAAAANUlEQVR4nO3QsQ0AMAzDsLT//9yeoCkbeYAN6LzZdZf3x0GSKEmUJEoSJYmSREmiJFGSaMoHo8QBPwYSAhsAAAAASUVORK5CYII="}
    ])).await.context("OpenCode could not complete the image prompt; check input validity, model vision support, provider credentials/quota and native diagnostics")?;
    ensure!(result["stopReason"] == "end_turn", "image turn did not complete");
    eprintln!("REAL negotiated image prompt completed (semantic vision correctness not asserted)");
    Ok(())
}

async fn request(conn: &AcpConnection, method: &str, params: Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(90), conn.request(method, params))
        .await
        .with_context(|| format!("OpenCode timed out during {method}; check provider readiness"))?
}

impl Smoke {
    async fn start() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let output = suneiro_core::env::tokio_command("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .output()
            .await?;
        ensure!(output.status.success(), "cannot initialize disposable smoke repository");
        std::fs::write(dir.path().join("smoke.txt"), "suneiro-opencode-smoke\n")?;
        let (conn, mut rx) = AcpConnection::spawn("opencode", &["acp".into()], dir.path())?;
        let updates: Arc<Mutex<Vec<Value>>> = Arc::default();
        let questions: Arc<Mutex<usize>> = Arc::default();
        let event_updates = updates.clone();
        let event_questions = questions.clone();
        let weak = Arc::downgrade(&conn);
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let Some(conn) = weak.upgrade() else { break };
                match event {
                    Incoming::Notification { method, params } if method == "session/update" => {
                        event_updates.lock().push(params["update"].clone())
                    }
                    Incoming::Request { id, method, params } if method == "session/request_permission" => {
                        // Smoke asks only for reading smoke.txt. Do not approve shell/edit/network tools.
                        let safe = params["toolCall"]["kind"] == "read";
                        let allow = params["options"]
                            .as_array()
                            .and_then(|options| options.iter().find(|o| o["kind"] == "allow_once"))
                            .and_then(|o| o["optionId"].as_str());
                        let outcome = match (safe, allow) {
                            (true, Some(id)) => json!({"outcome":"selected","optionId":id}),
                            _ => json!({"outcome":"cancelled"}),
                        };
                        let _ = conn.respond(id, json!({"outcome":outcome})).await;
                    }
                    Incoming::Request { id, method, .. } if method == "elicitation/create" => {
                        *event_questions.lock() += 1;
                        let _ = conn.respond(id, json!({"action":"cancel"})).await;
                    }
                    Incoming::Request { id, .. } => {
                        let _ = conn.respond_error(id, -32601, "Unsupported smoke client request").await;
                    }
                    _ => {}
                }
            }
        });
        // Construct a guard before requests so failures/timeouts also kill the CLI.
        let mut smoke = Self {
            conn,
            init: Value::Null,
            session: Value::Null,
            updates,
            questions,
            _dir: dir,
        };
        smoke.init = request(&smoke.conn, "initialize", json!({
            "protocolVersion": PROTOCOL_VERSION,
            "clientCapabilities": {"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false,"elicitation":{"form":{}}},
            "clientInfo":{"name":"suneiro-smoke","version":env!("CARGO_PKG_VERSION")}
        })).await?;
        smoke.session = request(
            &smoke.conn,
            "session/new",
            json!({"cwd":smoke._dir.path(),"mcpServers":[]}),
        )
        .await?;
        suneiro_core::opencode::validate_models(&smoke.session["configOptions"])?;
        eprintln!(
            "REAL OpenCode {}: image={}, resume={}, load={}, steering={}, controls={}",
            smoke.init["agentInfo"]["version"].as_str().unwrap_or("unknown"),
            smoke.init["agentCapabilities"]["promptCapabilities"]["image"] == true,
            smoke
                .init
                .pointer("/agentCapabilities/sessionCapabilities/resume")
                .is_some(),
            smoke.init["agentCapabilities"]["loadSession"] == true,
            smoke
                .init
                .pointer("/_meta/steering/supported")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            smoke.session["configOptions"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|o| o["id"].as_str())
                .collect::<Vec<_>>()
                .join(",")
        );
        Ok(smoke)
    }

    async fn prompt(&self, blocks: Value) -> Result<Value> {
        request(
            &self.conn,
            "session/prompt",
            json!({"sessionId":self.session["sessionId"],"prompt":blocks}),
        )
        .await
    }
}

#[tokio::test]
#[ignore = "requires installed OpenCode; no inference or credentials copied"]
async fn native_opencode_discovery_and_controls() -> Result<()> {
    let smoke = Smoke::start().await?;
    let config = &smoke.session["configOptions"];
    let model = &suneiro_core::catalog::choices(suneiro_core::catalog::model_option(config).unwrap())[0].value;
    let switched = request(
        &smoke.conn,
        "session/set_config_option",
        json!({"sessionId":smoke.session["sessionId"],"configId":"model","value":model}),
    )
    .await?;
    ensure!(
        switched["configOptions"].as_array().is_some_and(|options| options
            .iter()
            .any(|o| o["id"] == "model" && o["currentValue"] == model.as_str())),
        "OpenCode did not confirm selected provider/model"
    );
    let error = request(
        &smoke.conn,
        "session/set_config_option",
        json!({"sessionId":smoke.session["sessionId"],"configId":"model","value":"suneiro-missing/no-model"}),
    )
    .await
    .unwrap_err();
    ensure!(
        error.to_string().contains("not found"),
        "unexpected missing-model diagnostic: {error}"
    );
    eprintln!("REAL discovery and valid/invalid model switching passed; no model prompts sent");
    Ok(())
}

#[tokio::test]
#[ignore = "requires SUNEIRO_OPENCODE_MODEL; uses user's provider credentials and may incur charges"]
async fn native_opencode_prompt_tools_and_continuation() -> Result<()> {
    let model = std::env::var("SUNEIRO_OPENCODE_MODEL")
        .context("set SUNEIRO_OPENCODE_MODEL to an ID from `opencode models`")?;
    let smoke = Smoke::start().await?;
    suneiro_core::opencode::validate_control(&smoke.session["configOptions"], "model", &json!(model))?;
    request(
        &smoke.conn,
        "session/set_config_option",
        json!({"sessionId":smoke.session["sessionId"],"configId":"model","value":model}),
    )
    .await?;
    let result = smoke.prompt(json!([{"type":"text","text":"Read smoke.txt using the read tool. Reply with its exact contents. Do not edit files, run shell commands or access the network."}])).await?;
    ensure!(
        result["stopReason"] == "end_turn",
        "provider did not complete read turn: {}",
        result["stopReason"]
    );
    ensure!(
        smoke.updates.lock().iter().any(|u| u["sessionUpdate"] == "tool_call"),
        "provider returned without exercising a tool"
    );
    eprintln!("REAL provider prompt + tool event passed");

    // Begin a tracked request before cancel; continuation must remain usable.
    let prompt = smoke.prompt(json!([{"type":"text","text":"Explain Rust ownership in detail without using tools."}]));
    tokio::pin!(prompt);
    tokio::select! {
        result = &mut prompt => { result?; eprintln!("UNVERIFIED cancellation: provider finished before cancellation window"); }
        _ = tokio::time::sleep(Duration::from_millis(200)) => {
            smoke.conn.notify("session/cancel", json!({"sessionId":smoke.session["sessionId"]})).await?;
            let result = prompt.await?;
            ensure!(result["stopReason"] == "cancelled", "agent did not acknowledge cancellation");
            eprintln!("REAL cancellation acknowledged");
        }
    }
    let result = smoke
        .prompt(json!([{"type":"text","text":"Reply with exactly OK. Do not use tools."}]))
        .await?;
    ensure!(result["stopReason"] == "end_turn", "continuation failed");
    eprintln!("REAL continuation passed; structured question requests observed={}; no claim of question support unless observed", smoke.questions.lock());
    if smoke.updates.lock().is_empty() {
        return Err(anyhow!("provider emitted no transcript updates"));
    }
    Ok(())
}
