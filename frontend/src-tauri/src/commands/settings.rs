//! Tauri commands for user settings management

use crate::models::settings::{SettingsStatus, SettingsUpdate, UserSettings};
use crate::AppState;
use tauri::State;

/// Get current settings
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<UserSettings, String> {
    let settings = state.settings_service.read().await;
    Ok(settings.get().clone())
}

/// Get settings status (for checking if setup is needed)
#[tauri::command]
pub async fn get_settings_status(state: State<'_, AppState>) -> Result<SettingsStatus, String> {
    let settings = state.settings_service.read().await;
    Ok(settings.status())
}

/// Update settings
#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    update: SettingsUpdate,
) -> Result<UserSettings, String> {
    // Capture values before moving update
    let new_api_key = update.openrouter_api_key.clone();
    let new_mcp_enabled = update.mcp_enabled;

    // Update settings
    let mut settings = state.settings_service.write().await;
    let result = settings.update(update).map_err(|e| e.to_string())?;

    // Sync OpenRouter service if API key was updated
    if let Some(api_key) = new_api_key {
        let mut openrouter = state.openrouter.write().await;
        openrouter.set_api_key(api_key);
        log::info!("OpenRouter API key updated from settings");
    }

    // Sync MCP sidecar if mcp_enabled was changed
    if let Some(mcp_enabled) = new_mcp_enabled {
        let sidecar = &state.mcp_sidecar;
        if mcp_enabled {
            sidecar.set_enabled(true);
            if let Err(e) = sidecar.start(&app_handle).await {
                log::error!("Failed to start MCP sidecar from settings: {}", e);
            }
        } else {
            if let Err(e) = sidecar.stop().await {
                log::error!("Failed to stop MCP sidecar from settings: {}", e);
            }
            sidecar.set_enabled(false);
        }
    }

    Ok(result)
}

/// Complete initial setup
#[tauri::command]
pub async fn complete_setup(state: State<'_, AppState>) -> Result<(), String> {
    let mut settings = state.settings_service.write().await;
    settings.complete_setup().map_err(|e| e.to_string())
}

/// Open folder picker dialog for vault selection
#[tauri::command]
pub async fn pick_vault_folder() -> Result<Option<String>, String> {
    use std::sync::mpsc;
    use tauri::api::dialog::FileDialogBuilder;

    let (tx, rx) = mpsc::channel();

    FileDialogBuilder::new()
        .set_title("Select Vault Folder")
        .set_directory(
            dirs::document_dir()
                .unwrap_or_else(|| std::path::PathBuf::from(".")),
        )
        .pick_folder(move |folder_path| {
            let path = folder_path.map(|p| p.to_string_lossy().to_string());
            let _ = tx.send(path);
        });

    // Wait for the dialog result
    rx.recv()
        .map_err(|e| format!("Dialog error: {}", e))
}

/// Check if OpenRouter API key is valid by making a test request
#[tauri::command]
pub async fn validate_openrouter_key(api_key: String) -> Result<bool, String> {
    if api_key.is_empty() {
        return Ok(false);
    }

    let client = reqwest::Client::new();
    let response = client
        .get("https://openrouter.ai/api/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    Ok(response.status().is_success())
}

/// Get OpenRouter API key status (configured or not, without exposing the key)
#[tauri::command]
pub async fn get_openrouter_status(state: State<'_, AppState>) -> Result<OpenRouterStatus, String> {
    let settings = state.settings_service.read().await;
    let has_key = settings.get().has_openrouter_key();

    // Check if the service is actually working
    let openrouter = &state.openrouter;
    let is_configured = openrouter.read().await.is_configured();

    Ok(OpenRouterStatus {
        has_key,
        is_configured,
    })
}

#[derive(serde::Serialize)]
pub struct OpenRouterStatus {
    pub has_key: bool,
    pub is_configured: bool,
}
