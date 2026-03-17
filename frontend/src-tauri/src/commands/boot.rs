use crate::models::boot::BootStatus;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub async fn get_boot_status(state: State<'_, AppState>) -> Result<BootStatus, String> {
    Ok(state.boot_state.read().await.clone())
}
