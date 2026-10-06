//! Tauri bindings over `suneiro-core`. Keep logic in core; this file only maps
//! commands and streams events to the webview.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use suneiro_core::{AgentDef, Core, Event, Repo, Session, Workspace};
use serde_json::Value;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{Emitter, Manager, RunEvent, State};

mod updater;

type Res<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

/// Events are buffered and flushed to the UI as one batch per frame, so a
/// dozen streaming agents cost one IPC message every ~16ms instead of thousands.
#[derive(Default)]
struct EventBus {
    queue: Mutex<Vec<Event>>,
    channel: Mutex<Option<Channel<Vec<Event>>>>,
}

impl EventBus {
    fn start(self: &Arc<Self>) {
        let bus = self.clone();
        std::thread::Builder::new()
            .name("suneiro-event-flush".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(16));
                let batch = std::mem::take(&mut *bus.queue.lock());
                if batch.is_empty() {
                    continue;
                }
                if let Some(ch) = bus.channel.lock().as_ref() {
                    let _ = ch.send(batch);
                }
            })
            .expect("spawn flush thread");
    }
}

struct App {
    core: Arc<Core>,
    bus: Arc<EventBus>,
}

#[tauri::command]
fn subscribe(app: State<'_, App>, channel: Channel<Vec<Event>>) {
    *app.bus.channel.lock() = Some(channel);
}

#[tauri::command]
fn list_agents(app: State<'_, App>) -> Vec<AgentDef> {
    app.core.agents.defs()
}

#[tauri::command]
fn list_repos(app: State<'_, App>) -> Res<Vec<Repo>> {
    app.core.store.repos().map_err(err)
}

#[tauri::command]
async fn add_repo(app: State<'_, App>, path: String) -> Res<Repo> {
    app.core.add_repo(&path).await.map_err(err)
}

#[tauri::command]
async fn recent_projects(app: State<'_, App>) -> Res<Value> {
    let list = app.core.recent_projects(8).await.map_err(err)?;
    serde_json::to_value(list).map_err(|e| e.to_string())
}

#[tauri::command]
async fn clone_repo(app: State<'_, App>, spec: String) -> Res<Repo> {
    app.core.clone_repo(&spec).await.map_err(err)
}

#[tauri::command]
fn remove_repo(app: State<'_, App>, repo_id: String) -> Res<()> {
    app.core.store.remove_repo(&repo_id).map_err(err)
}

#[tauri::command]
fn reorder_repos(app: State<'_, App>, repo_ids: Vec<String>) -> Res<()> {
    app.core.store.reorder_repos(&repo_ids).map_err(err)
}

#[tauri::command]
fn list_workspaces(app: State<'_, App>) -> Res<Vec<Workspace>> {
    app.core.store.workspaces().map_err(err)
}

#[tauri::command]
async fn create_workspace(app: State<'_, App>, repo_id: String) -> Res<Workspace> {
    app.core.create_workspace(&repo_id).await.map_err(err)
}

#[tauri::command]
async fn create_workspace_from(app: State<'_, App>, repo_id: String, branch: Option<String>, pr: Option<u64>) -> Res<Workspace> {
    app.core.create_workspace_from(&repo_id, branch.as_deref(), pr).await.map_err(err)
}

#[tauri::command]
async fn list_branches(app: State<'_, App>, repo_id: String) -> Res<Value> {
    let list = app.core.branches(&repo_id).await.map_err(err)?;
    serde_json::to_value(list).map_err(|e| e.to_string())
}

#[tauri::command]
async fn open_prs(app: State<'_, App>, repo_id: String) -> Res<Vec<Value>> {
    app.core.open_prs(&repo_id).await.map_err(err)
}

#[tauri::command]
fn rename_workspace(app: State<'_, App>, workspace_id: String, title: String) -> Res<()> {
    app.core.rename_workspace(&workspace_id, &title).map_err(err)
}

#[tauri::command]
fn set_workspace_unread(app: State<'_, App>, workspace_id: String, unread: bool) -> Res<()> {
    app.core.store.set_workspace_unread(&workspace_id, unread).map_err(err)
}

#[tauri::command]
fn list_all_workspaces(app: State<'_, App>) -> Res<Vec<Workspace>> {
    app.core.store.all_workspaces().map_err(err)
}

#[tauri::command]
async fn restore_workspace(app: State<'_, App>, workspace_id: String) -> Res<Workspace> {
    app.core.restore_workspace(&workspace_id).await.map_err(err)
}

#[tauri::command]
async fn archive_workspace(app: State<'_, App>, workspace_id: String) -> Res<()> {
    app.core.archive_workspace(&workspace_id).await.map_err(err)
}

#[tauri::command]
fn repo_config(app: State<'_, App>, workspace_id: String) -> Res<Value> {
    let config = app.core.repo_config(&workspace_id).map_err(err)?;
    serde_json::to_value(config).map_err(|e| e.to_string())
}

#[tauri::command]
fn repo_settings(app: State<'_, App>, repo_id: String) -> Res<Value> {
    let file = app.core.repo_file_config(&repo_id).map_err(err)?;
    Ok(serde_json::json!({
        "config": app.core.repo_settings(&repo_id),
        "file": file.map(|(name, config)| serde_json::json!({"name": name, "config": config})),
    }))
}

#[tauri::command]
fn save_repo_settings(app: State<'_, App>, repo_id: String, config: suneiro_core::workspace::RepoConfig) -> Res<()> {
    app.core.save_repo_settings(&repo_id, &config).map_err(err)
}

#[tauri::command]
fn list_sessions(app: State<'_, App>, workspace_id: String) -> Res<Vec<Session>> {
    app.core.store.sessions(&workspace_id).map_err(err)
}

#[tauri::command]
async fn create_session(
    app: State<'_, App>,
    workspace_id: String,
    agent_id: String,
    model: Option<String>,
    effort: Option<String>,
) -> Res<Session> {
    app.core.create_session(&workspace_id, &agent_id, model, effort).map_err(err)
}

/// Start a chat's agent without prompting (e.g. to show its model options).
#[tauri::command]
async fn connect_session(app: State<'_, App>, session_id: String) -> Res<()> {
    app.core.agents.warm_up(&session_id);
    Ok(())
}

#[tauri::command]
fn delete_session(app: State<'_, App>, session_id: String) -> Res<()> {
    app.core.delete_session(&session_id).map_err(err)
}

#[tauri::command]
fn session_events(app: State<'_, App>, session_id: String) -> Res<Vec<Value>> {
    app.core.store.events(&session_id).map_err(err)
}

#[tauri::command]
async fn send_prompt(app: State<'_, App>, session_id: String, text: String, images: Option<Vec<String>>) -> Res<()> {
    app.core.agents.prompt_with(&session_id, text, images.unwrap_or_default()).map_err(err)
}

#[tauri::command]
async fn steer(app: State<'_, App>, session_id: String, text: String, images: Option<Vec<String>>) -> Res<String> {
    app.core.agents.steer_with(&session_id, text, images.unwrap_or_default()).await.map(str::to_string).map_err(err)
}

#[tauri::command]
async fn save_attachment(app: State<'_, App>, mime_type: String, data: String) -> Res<String> {
    app.core.save_attachment(&mime_type, &data).map_err(err)
}

#[tauri::command]
async fn import_attachment(app: State<'_, App>, path: String) -> Res<String> {
    app.core.import_attachment(&path).map_err(err)
}

#[tauri::command]
async fn attachment_data_url(app: State<'_, App>, path: String) -> Res<String> {
    app.core.attachment_data_url(&path).map_err(err)
}

#[tauri::command]
async fn cancel_prompt(app: State<'_, App>, session_id: String) -> Res<()> {
    app.core.agents.cancel(&session_id).await.map_err(err)
}

#[tauri::command]
async fn respond_permission(
    app: State<'_, App>,
    session_id: String,
    request_id: String,
    option_id: Option<String>,
) -> Res<()> {
    app.core.agents.respond_permission(&session_id, &request_id, option_id).await.map_err(err)
}

#[tauri::command]
async fn answer_question(app: State<'_, App>, session_id: String, request_id: String, response: Value) -> Res<()> {
    app.core.agents.answer_question(&session_id, &request_id, response).await.map_err(err)
}

#[tauri::command]
async fn set_config(app: State<'_, App>, session_id: String, config_id: String, value: Value) -> Res<()> {
    app.core.agents.set_config(&session_id, &config_id, value).await.map_err(err)
}

#[tauri::command]
async fn sync_status(app: State<'_, App>, workspace_id: String) -> Res<Value> {
    let st = app.core.sync_status(&workspace_id).await.map_err(err)?;
    serde_json::to_value(st).map_err(|e| e.to_string())
}

#[tauri::command]
async fn merge_base_branch(app: State<'_, App>, workspace_id: String) -> Res<Vec<String>> {
    app.core.merge_base_branch(&workspace_id).await.map_err(err)
}

#[tauri::command]
async fn abort_merge(app: State<'_, App>, workspace_id: String) -> Res<()> {
    app.core.abort_merge(&workspace_id).await.map_err(err)
}

#[tauri::command]
async fn restore_checkpoint(app: State<'_, App>, session_id: String, commit: String) -> Res<String> {
    app.core.restore_checkpoint(&session_id, &commit).await.map_err(err)
}

#[tauri::command]
async fn set_plan_mode(app: State<'_, App>, session_id: String, plan: bool) -> Res<()> {
    app.core.agents.set_plan_mode(&session_id, plan).await.map_err(err)
}

#[tauri::command]
async fn changed_files(app: State<'_, App>, workspace_id: String) -> Res<Value> {
    let files = app.core.changed_files(&workspace_id).await.map_err(err)?;
    serde_json::to_value(files).map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_files(app: State<'_, App>, workspace_id: String) -> Res<Vec<String>> {
    app.core.list_files(&workspace_id).await.map_err(err)
}

#[tauri::command]
async fn file_diff(app: State<'_, App>, workspace_id: String, path: String) -> Res<String> {
    app.core.file_diff(&workspace_id, &path).await.map_err(err)
}

#[tauri::command]
async fn revert_file(app: State<'_, App>, workspace_id: String, path: String) -> Res<()> {
    app.core.revert_file(&workspace_id, &path).await.map_err(err)
}

#[tauri::command]
async fn commit_all(app: State<'_, App>, workspace_id: String, message: String) -> Res<()> {
    app.core.commit_all(&workspace_id, &message).await.map_err(err)
}

#[tauri::command]
async fn commit_and_push(app: State<'_, App>, workspace_id: String) -> Res<()> {
    app.core.commit_and_push(&workspace_id).await.map_err(err)
}

#[tauri::command]
async fn draft_pr(app: State<'_, App>, workspace_id: String) -> Res<(String, String)> {
    app.core.draft_pr(&workspace_id).await.map_err(err)
}

#[tauri::command]
async fn push(app: State<'_, App>, workspace_id: String) -> Res<()> {
    app.core.push(&workspace_id).await.map_err(err)
}

#[tauri::command]
async fn pr_status(app: State<'_, App>, workspace_id: String) -> Res<Option<Value>> {
    app.core.pr_status(&workspace_id).await.map_err(err)
}

#[tauri::command]
async fn create_pr(app: State<'_, App>, workspace_id: String, title: String, body: String) -> Res<String> {
    app.core.create_pr(&workspace_id, &title, &body).await.map_err(err)
}

#[tauri::command]
fn usage(app: State<'_, App>, since: i64) -> Res<Value> {
    let rows = app.core.usage(since).map_err(err)?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_pricing(app: State<'_, App>) -> Value {
    serde_json::to_value(app.core.pricing()).unwrap_or_default()
}

#[tauri::command]
fn save_pricing(app: State<'_, App>, pricing: suneiro_core::usage::Pricing) -> Res<()> {
    app.core.save_pricing(&pricing).map_err(err)
}

#[tauri::command]
fn model_catalogs(app: State<'_, App>) -> Value {
    serde_json::to_value(app.core.catalogs()).unwrap_or_default()
}

#[tauri::command]
async fn refresh_catalog(app: State<'_, App>, agent_id: String) -> Res<Value> {
    let c = app.core.refresh_catalog(&agent_id).await.map_err(err)?;
    serde_json::to_value(c).map_err(|e| e.to_string())
}

#[tauri::command]
async fn pr_details(app: State<'_, App>, workspace_id: String) -> Res<Value> {
    app.core.pr_details(&workspace_id).await.map_err(err)
}

#[tauri::command]
async fn fetch_image(url: String) -> Res<String> {
    suneiro_core::forge::fetch_image(&url).await.map_err(err)
}

#[tauri::command]
fn cached_prs(app: State<'_, App>) -> std::collections::HashMap<String, Value> {
    app.core.cached_prs()
}

#[tauri::command]
fn refresh_prs(app: State<'_, App>) {
    app.core.refresh_prs();
}

#[tauri::command]
async fn merge_pr(app: State<'_, App>, workspace_id: String) -> Res<()> {
    app.core.merge_pr(&workspace_id).await.map_err(err)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn terminal_open(
    app: State<'_, App>,
    workspace_id: String,
    terminal_id: String,
    cols: u16,
    rows: u16,
    command: Option<String>,
    on_data: Channel<InvokeResponseBody>,
    on_exit: Channel<bool>,
) -> Res<()> {
    app.core
        .open_terminal(
            &workspace_id,
            &terminal_id,
            cols,
            rows,
            command.as_deref(),
            move |bytes| {
                let _ = on_data.send(InvokeResponseBody::Raw(bytes));
            },
            move || {
                let _ = on_exit.send(true);
            },
        )
        .map_err(err)
}

#[tauri::command]
fn terminal_write(app: State<'_, App>, terminal_id: String, data: String) -> Res<()> {
    app.core.terminals.write(&terminal_id, data.as_bytes()).map_err(err)
}

#[tauri::command]
fn terminal_resize(app: State<'_, App>, terminal_id: String, cols: u16, rows: u16) -> Res<()> {
    app.core.terminals.resize(&terminal_id, cols, rows).map_err(err)
}

#[tauri::command]
fn terminal_kill(app: State<'_, App>, terminal_id: String) {
    app.core.terminals.kill(&terminal_id);
}

#[tauri::command]
fn get_settings(app: State<'_, App>) -> suneiro_core::setup::Settings {
    app.core.settings()
}

#[tauri::command]
fn save_settings(app: State<'_, App>, settings: suneiro_core::setup::Settings) -> Res<()> {
    app.core.save_settings(&settings).map_err(err)
}

#[tauri::command]
async fn detect_agents() -> Vec<suneiro_core::setup::AgentStatus> {
    let (a, b, c) = tokio::join!(
        suneiro_core::setup::detect("claude"),
        suneiro_core::setup::detect("codex"),
        suneiro_core::setup::detect("opencode"),
    );
    [a, b, c].into_iter().flatten().collect()
}

/// Run an install/login command in a pty so interactive flows work.
#[tauri::command]
fn setup_terminal_open(
    app: State<'_, App>,
    terminal_id: String,
    cols: u16,
    rows: u16,
    command: String,
    on_data: Channel<InvokeResponseBody>,
    on_exit: Channel<bool>,
) -> Res<()> {
    app.core
        .open_home_terminal(
            &terminal_id,
            cols,
            rows,
            &command,
            move |bytes| {
                let _ = on_data.send(InvokeResponseBody::Raw(bytes));
            },
            move || {
                let _ = on_exit.send(true);
            },
        )
        .map_err(err)
}

/// Open a path in Finder or in a named app (e.g. "Visual Studio Code", "Cursor").
#[tauri::command]
fn open_path(path: String, app_name: Option<String>) -> Res<()> {
    let mut cmd = std::process::Command::new("open");
    if let Some(name) = app_name {
        cmd.args(["-a", &name]);
    }
    let status = cmd.arg(&path).status().map_err(|e| e.to_string())?;
    status.success().then_some(()).ok_or_else(|| format!("could not open {path}"))
}

/// Default macOS menus, plus Suneiro's own items. Custom items emit a "menu"
/// event with their id for the UI to handle.
fn install_menu(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
    let handle = app.handle();
    let menu = Menu::default(handle)?;
    let item = |id: &str, label: &str, accel: &str| MenuItem::with_id(handle, id, label, true, Some(accel));
    for (i, entry) in menu.items()?.into_iter().enumerate() {
        let MenuItemKind::Submenu(sub) = entry else { continue };
        if i == 0 {
            sub.insert(&MenuItem::with_id(handle, "check-updates", "Check for Updates…", true, None::<&str>)?, 1)?;
            sub.insert(&PredefinedMenuItem::separator(handle)?, 2)?;
            sub.insert(&item("settings", "Settings…", "CmdOrCtrl+,")?, 3)?;
        } else if sub.text()? == "File" {
            // Replace "Close Window" (⌘W) with chat-level actions.
            for old in sub.items()? {
                sub.remove(&old)?;
            }
            sub.append(&item("new-workspace", "New Workspace", "CmdOrCtrl+N")?)?;
            sub.append(&item("new-workspace-from", "New Workspace from Branch or PR…", "CmdOrCtrl+Shift+N")?)?;
            sub.append(&item("new-chat", "New Chat", "CmdOrCtrl+T")?)?;
            sub.append(&PredefinedMenuItem::separator(handle)?)?;
            sub.append(&item("palette", "Command Palette…", "CmdOrCtrl+K")?)?;
            sub.append(&item("shortcuts", "Keyboard Shortcuts", "CmdOrCtrl+/")?)?;
            sub.append(&PredefinedMenuItem::separator(handle)?)?;
            sub.append(&item("close-chat", "Close Chat", "CmdOrCtrl+W")?)?;
        }
    }
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        let _ = app.emit("menu", event.id().as_ref());
    });
    Ok(())
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updater::Updates::default())
        .setup(|app| {
            let data_dir = suneiro_core::storage::data_dir(&app.path().app_data_dir()?);
            let bus = Arc::new(EventBus::default());
            bus.start();
            let sink_bus = bus.clone();
            let core = Core::new(&data_dir, Arc::new(move |e| sink_bus.queue.lock().push(e)))?;
            let poller = core.clone();
            tauri::async_runtime::spawn(async move { poller.start_pr_poller() });
            app.manage(App { core, bus });
            install_menu(app)?;
            updater::start(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            subscribe,
            list_agents,
            list_repos,
            add_repo,
            remove_repo,
            reorder_repos,
            recent_projects,
            clone_repo,
            list_workspaces,
            create_workspace,
            create_workspace_from,
            list_branches,
            open_prs,
            archive_workspace,
            list_all_workspaces,
            rename_workspace,
            set_workspace_unread,
            restore_workspace,
            repo_config,
            repo_settings,
            save_repo_settings,
            list_sessions,
            create_session,
            delete_session,
            connect_session,
            session_events,
            send_prompt,
            cancel_prompt,
            steer,
            respond_permission,
            set_config,
            answer_question,
            set_plan_mode,
            restore_checkpoint,
            sync_status,
            merge_base_branch,
            abort_merge,
            save_attachment,
            import_attachment,
            attachment_data_url,
            changed_files,
            file_diff,
            list_files,
            revert_file,
            commit_all,
            push,
            commit_and_push,
            draft_pr,
            pr_status,
            create_pr,
            merge_pr,
            cached_prs,
            pr_details,
            fetch_image,
            model_catalogs,
            usage,
            get_pricing,
            save_pricing,
            refresh_catalog,
            refresh_prs,
            terminal_open,
            terminal_write,
            terminal_resize,
            terminal_kill,
            open_path,
            get_settings,
            save_settings,
            detect_agents,
            setup_terminal_open,
            updater::check_for_updates,
            updater::update_status,
            updater::install_update,
            updater::cancel_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            handle.state::<App>().core.shutdown();
        }
    });
}
