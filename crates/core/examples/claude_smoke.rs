//! Read-only diagnostics by default; --run explicitly opts into billed turns.
//! All git/tool work stays in a disposable repository, never a user's checkout.
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Result};
use parking_lot::Mutex;
use serde_json::json;
use suneiro_core::{Core, Event, Repo, Session, Workspace};

struct Smoke {
    core: Arc<Core>,
    events: Arc<Mutex<Vec<Event>>>,
    sid: String,
    _dir: tempfile::TempDir,
}

impl Drop for Smoke {
    fn drop(&mut self) { self.core.shutdown(); }
}

impl Smoke {
    async fn new() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo)?;
        suneiro_core::git::git(&repo, &["init", "-b", "main"]).await?;
        suneiro_core::git::git(&repo, &["-c", "user.name=Suneiro smoke", "-c", "user.email=smoke@example.invalid", "commit", "--allow-empty", "-m", "fixture"]).await?;
        let events: Arc<Mutex<Vec<Event>>> = Arc::default();
        let log = events.clone();
        let core = Core::new(&dir.path().join("data"), Arc::new(move |e| log.lock().push(e)))?;
        let path = repo.display().to_string();
        core.store.add_repo(&Repo { id: "r".into(), name: "smoke".into(), path: path.clone(), default_branch: "main".into() })?;
        core.store.add_workspace(&Workspace {
            id: "w".into(), repo_id: "r".into(), name: "smoke".into(), branch: "main".into(), base_branch: "main".into(),
            path, status: "ready".into(), created_at: 0, title: "Claude smoke".into(), archived_at: None, unread: false,
        })?;
        let sid = uuid::Uuid::new_v4().to_string();
        let model = std::env::var("SUNEIRO_CLAUDE_MODEL").ok();
        let effort = std::env::var("SUNEIRO_CLAUDE_EFFORT").ok();
        core.store.add_session(&Session {
            id: sid.clone(), workspace_id: "w".into(), agent_id: "claude".into(), acp_session_id: None,
            title: "Claude smoke".into(), created_at: 0, model: model.clone(), effort: effort.clone(), parent_session_id: None,
        })?;
        core.agents.preset(&sid, true, model.clone(), effort.clone())?;
        core.agents.warm_up(&sid);
        let smoke = Self { core, events, sid, _dir: dir };
        smoke.wait(|e| matches!(e, Event::SessionState { state, .. } if state == "idle")).await?;
        let session = smoke.core.store.session(&smoke.sid)?;
        if let Some(model) = model { ensure!(session.model.as_deref() == Some(&model), "requested model was not applied"); }
        if let Some(effort) = effort { ensure!(session.effort.as_deref() == Some(&effort), "requested effort was not applied"); }
        ensure!(smoke.events.lock().iter().any(|e| matches!(e, Event::SessionConfig { config_options, .. } if config_options.as_array().is_some_and(|options| options.iter().any(|o| o["id"] == "mode" && o["currentValue"] == "plan")))), "adapter did not enter plan mode");
                println!("PASS initialize, requested model/effort, plan config; adapter pinned in builtin_agents");
        Ok(smoke)
    }

    async fn wait(&self, pred: impl Fn(&Event) -> bool) -> Result<Event> {
        tokio::time::timeout(Duration::from_secs(180), async {
            loop {
                {
                    let events = self.events.lock();
                    for e in events.iter() {
                        if let Event::SessionState { state, error: Some(error), .. } = e {
                            if state == "error" { bail!("Claude error: {error}"); }
                        }
                    }
                    if let Some(e) = events.iter().find(|e| pred(e)) { return Ok(e.clone()); }
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }).await.map_err(|_| anyhow::anyhow!("Claude smoke timed out after 180s"))?
    }

    fn text(&self) -> String {
        self.events.lock().iter().filter_map(|e| match e {
            Event::SessionUpdate { update, .. } if update["sessionUpdate"] == "agent_message_chunk" => update["content"]["text"].as_str(),
            _ => None,
        }).collect()
    }

    fn prompt(&self, prompt: &str) -> Result<()> {
        self.events.lock().clear();
        self.core.agents.prompt(&self.sid, prompt.into())
    }

    async fn end(&self) -> Result<()> {
        self.wait(|e| matches!(e, Event::TurnEnd { .. })).await?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        println!("claude_smoke [--check | --catalog | --run]\n--check: CLI auth/Node/npx preflight only (default).\n--catalog: additionally start the actual pinned adapter, no model prompts.\n--run: billed real-agent suite in a disposable git repository. Auto-answers only its own fixture questions; never changes logins or creates PRs.\nOptional SUNEIRO_CLAUDE_MODEL / SUNEIRO_CLAUDE_EFFORT select advertised values.");
        return Ok(());
    }
    ensure!(args.len() <= 1 && args.first().is_none_or(|a| ["--check", "--catalog", "--run"].contains(&a.as_str())), "unknown arguments; use --help");
    let diagnostics = suneiro_core::setup::claude_diagnostics().await;
    println!("CLI={:?} authenticated={} Node={:?} npx={:?}", diagnostics.cli.version, diagnostics.cli.logged_in, diagnostics.node_version, diagnostics.npx_version);
    for issue in &diagnostics.issues { eprintln!("CHECK: {issue}"); }
    ensure!(diagnostics.issues.is_empty(), "Claude preflight failed; no model request was sent");
    if args.first().is_none_or(|a| a == "--check") { return Ok(()); }
    let smoke = Smoke::new().await?;
    if args[0] == "--catalog" {
        let catalogs = smoke.core.catalogs();
        let catalog = catalogs.get("claude").ok_or_else(|| anyhow::anyhow!("adapter reported no catalog"))?;
        ensure!(!catalog.models.is_empty(), "no models");
        println!("PASS real catalog: {} models, {} effort choices; no model request sent", catalog.models.len(), catalog.efforts.len());
        return Ok(());
    }
    // Unique marker allows replay and resumed-memory checks without real repo data.
    let marker = format!("SMOKE_{}", uuid::Uuid::new_v4().simple());
    smoke.prompt(&format!("Remember the marker {marker}. Reply with exactly that marker, without tools."))?;
    smoke.end().await?;
    ensure!(smoke.text().contains(&marker), "basic reply omitted marker");
    println!("PASS prompt in plan mode");
    smoke.core.agents.set_plan_mode(&smoke.sid, false).await?;
    smoke.prompt("Use AskUserQuestion to ask which color to choose, red or blue. Do not choose for me. After the answer, reply with QUESTION_OK.")?;
    let Event::Question { request_id, schema, .. } = smoke.wait(|e| matches!(e, Event::Question { .. })).await? else { unreachable!() };
    let mut answer = serde_json::Map::new();
    for (key, value) in schema["properties"].as_object().ok_or_else(|| anyhow::anyhow!("question has no properties"))? {
        let choice = value["enum"][0].as_str().or_else(|| value["oneOf"][0]["const"].as_str()).unwrap_or("blue");
        answer.insert(key.clone(), json!(choice));
    }
    smoke.core.agents.answer_question(&smoke.sid, &request_id, json!({"action": "accept", "content": answer})).await?;
    smoke.end().await?;
    ensure!(smoke.text().contains("QUESTION_OK"), "question answer not acknowledged");
    println!("PASS structured question in auto mode");
    smoke.prompt("Use the Write tool to create smoke.txt containing only TOOL_OK in the current directory. Then reply TOOL_OK. Do not run any shell commands or access any other files.")?;
    smoke.end().await?;
    ensure!(std::fs::read_to_string(smoke._dir.path().join("repo/smoke.txt"))?.trim() == "TOOL_OK", "tool write failed");
    ensure!(smoke.events.lock().iter().any(|e| matches!(e, Event::SessionUpdate { update, .. } if update["sessionUpdate"] == "tool_call")), "no tool updates");
    println!("PASS auto-accepted tool and tool updates");
    smoke.core.agents.close(&smoke.sid);
    smoke.prompt("Without tools, repeat the unique SMOKE_ marker I asked you to remember at the start of this conversation, and nothing else.")?;
    smoke.end().await?;
    ensure!(smoke.text().trim() == marker, "resumed history missing or replayed into output");
    let usage = smoke.core.store.usage_for_session(&smoke.sid)?;
    ensure!(usage.len() >= 4, "missing usage rows");
    ensure!(usage.iter().all(|u| u.cost_usd.is_some_and(|v| v.is_finite() && v >= 0.0)), "missing/invalid reported USD cost");
    println!("PASS resume without transcript duplication, {} usage rows", usage.len());
    smoke.prompt("Use Bash to run sleep 20 in this disposable fixture directory, then reply FINISHED. Do not run other commands.")?;
    smoke.wait(|e| matches!(e, Event::SessionUpdate { update, .. } if update["sessionUpdate"] == "tool_call")).await?;
    smoke.core.agents.cancel(&smoke.sid).await?;
    let Event::TurnEnd { stop_reason, .. } = smoke.wait(|e| matches!(e, Event::TurnEnd { .. })).await? else { unreachable!() };
    ensure!(stop_reason == "cancelled", "cancel did not cancel turn: {stop_reason}");
    println!("PASS cancellation; image capability and steering contracts also have deterministic fixture coverage, not asserted by this real run");
    Ok(())
}
