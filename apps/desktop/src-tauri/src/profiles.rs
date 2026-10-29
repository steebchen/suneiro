use suneiro_core::profiles::{AgentProfile, ContextMessage, ContextPackage, ContextSelection};
use suneiro_core::Session;
use tauri::State;
use crate::{err, App, Res};

#[tauri::command]
pub fn list_profiles(app: State<'_, App>) -> Res<Vec<AgentProfile>> {
    app.core.profiles().map_err(err)
}

#[tauri::command]
pub fn save_profiles(app: State<'_, App>, profiles: Vec<AgentProfile>) -> Res<()> {
    app.core.save_profiles(profiles).map_err(err)
}

#[tauri::command]
pub fn context_messages(app: State<'_, App>, workspace_id: String, session_id: String) -> Res<Vec<ContextMessage>> {
    app.core.context_messages(&workspace_id, &session_id).map_err(err)
}

#[tauri::command]
pub async fn prepare_context(app: State<'_, App>, workspace_id: String, profile_id: String, agent_id: String, selection: ContextSelection) -> Res<ContextPackage> {
    app.core.prepare_context(&workspace_id, &profile_id, &agent_id, selection).await.map_err(err)
}

#[tauri::command]
pub fn launch_profile(app: State<'_, App>, workspace_id: String, package_id: String) -> Res<Session> {
    app.core.launch_profile(&workspace_id, &package_id).map_err(err)
}
