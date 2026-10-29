//! Thin verification/review command mappings.
use super::{err, App, Res};
use suneiro_core::{
    review::{IntegrationPreview, Review},
    verification::{CheckPreset, CheckRun},
    Session,
};
use tauri::State;

#[tauri::command]
pub fn verification_presets(app: State<'_, App>, repo_id: String) -> Res<Vec<CheckPreset>> {
    app.core.verification_presets(&repo_id).map_err(err)
}
#[tauri::command]
pub fn save_verification_presets(
    app: State<'_, App>,
    repo_id: String,
    presets: Vec<CheckPreset>,
) -> Res<()> {
    app.core
        .save_verification_presets(&repo_id, presets)
        .map_err(err)
}
#[tauri::command]
pub async fn verification_status(app: State<'_, App>, workspace_id: String) -> Res<Vec<CheckRun>> {
    app.core
        .verification_status(&workspace_id)
        .await
        .map_err(err)
}
#[tauri::command]
pub async fn run_check(
    app: State<'_, App>,
    workspace_id: String,
    preset_id: String,
) -> Res<CheckRun> {
    app.core
        .run_check(&workspace_id, &preset_id)
        .await
        .map_err(err)
}
#[tauri::command]
pub fn cancel_check(app: State<'_, App>, workspace_id: String, run_id: String) -> Res<()> {
    app.core.cancel_check(&workspace_id, &run_id).map_err(err)
}
#[tauri::command]
pub async fn start_review(
    app: State<'_, App>,
    workspace_id: String,
    agent_id: String,
    files: Vec<String>,
    parent_session_id: Option<String>,
) -> Res<Review> {
    app.core
        .start_review(
            &workspace_id,
            &agent_id,
            files,
            parent_session_id.as_deref(),
        )
        .await
        .map_err(err)
}
#[tauri::command]
pub fn verification_task(app: State<'_, App>, workspace_id: String, text: String) -> Res<Session> {
    app.core
        .verification_task(&workspace_id, &text)
        .map_err(err)
}
#[tauri::command]
pub async fn integration_preview(
    app: State<'_, App>,
    workspace_id: String,
    source_workspace_id: String,
) -> Res<IntegrationPreview> {
    app.core
        .integration_preview(&workspace_id, &source_workspace_id)
        .await
        .map_err(err)
}
#[tauri::command]
pub async fn integrate_workspace(
    app: State<'_, App>,
    workspace_id: String,
    preview: IntegrationPreview,
) -> Res<()> {
    app.core
        .integrate_workspace(&workspace_id, preview)
        .await
        .map_err(err)
}
