use crate::services::priority::{PrioritySettings, PrioritySettingsUpdate};
use crate::AppState;
use tauri::State;

/// Get current priority scoring settings
#[tauri::command]
pub async fn get_priority_settings(state: State<'_, AppState>) -> Result<PrioritySettings, String> {
    let svc = state.priority_service.read().await;
    Ok(svc.get_settings().clone())
}

/// Update priority scoring settings (partial update)
#[tauri::command]
pub async fn update_priority_settings(
    update: PrioritySettingsUpdate,
    state: State<'_, AppState>,
) -> Result<PrioritySettings, String> {
    let mut svc = state.priority_service.write().await;
    svc.update_settings(update).map_err(|e| e.to_string())
}

/// Reset priority scoring settings to defaults
#[tauri::command]
pub async fn reset_priority_settings(
    state: State<'_, AppState>,
) -> Result<PrioritySettings, String> {
    let mut svc = state.priority_service.write().await;
    svc.reset_settings().map_err(|e| e.to_string())
}
