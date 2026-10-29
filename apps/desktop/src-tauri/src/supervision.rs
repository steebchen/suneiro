use crate::{err, App, Res};
use suneiro_core::supervision::{InboxItem, RepoControls, ResourceUsage};
use tauri::State;

#[tauri::command]
pub fn supervision_inbox(app: State<'_, App>) -> Res<Vec<InboxItem>> {
    app.core.inbox().map_err(err)
}
#[tauri::command]
pub fn dismiss_inbox(app: State<'_, App>, id: String) -> Res<()> {
    app.core.dismiss_inbox(&id).map_err(err)
}
#[tauri::command]
pub fn claim_notification(app: State<'_, App>, id: String) -> Res<bool> {
    app.core.store.claim_notification(&id).map_err(err)
}
#[tauri::command]
pub fn repo_controls(app: State<'_, App>, repo_id: String) -> Res<RepoControls> {
    app.core.repo_controls(&repo_id).map_err(err)
}
#[tauri::command]
pub fn save_repo_controls(app: State<'_, App>, repo_id: String, controls: RepoControls) -> Res<()> {
    app.core.save_repo_controls(&repo_id, &controls).map_err(err)
}
#[tauri::command]
pub fn repo_resources(app: State<'_, App>, repo_id: String) -> Res<ResourceUsage> {
    app.core.repo_resources(&repo_id).map_err(err)
}
#[tauri::command]
pub fn subtree_resources(app: State<'_, App>, session_id: String) -> Res<ResourceUsage> {
    app.core.subtree_resources(&session_id).map_err(err)
}
