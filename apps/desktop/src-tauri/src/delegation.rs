use super::{App, Res};
use suneiro_core::delegation::{Message, Task, TaskRequest};
use tauri::State;

#[tauri::command]
pub async fn delegate_task(app: State<'_, App>, session_id: String, request: TaskRequest) -> Res<Task> {
    app.core.delegate(&session_id, request).await.map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub fn delegation_tasks(app: State<'_, App>, session_id: String) -> Res<Vec<Task>> {
    app.core.store.session(&session_id).map_err(|e| e.to_string())?;
    app.core.delegation.reports(&session_id).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn delegation_messages(app: State<'_, App>, session_id: String) -> Res<Vec<Message>> {
    app.core.store.session(&session_id).map_err(|e| e.to_string())?;
    app.core.delegation.messages(&session_id).map_err(|e| e.to_string())
}
#[tauri::command]
pub fn delegation_send(app: State<'_, App>, session_id: String, task_id: String, message: String) -> Res<()> {
    app.core.send_delegation_message(&session_id, &task_id, &message).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn delegation_cancel(app: State<'_, App>, session_id: String, task_id: String) -> Res<Task> {
    app.core.cancel_delegation(&session_id, &task_id).await.map_err(|e| e.to_string())
}
#[tauri::command]
pub fn delegation_finish(app: State<'_, App>, session_id: String, task_id: String, report: String) -> Res<Task> {
    app.core.finish_delegation(&session_id, &task_id, &report).map_err(|e| e.to_string())
}
