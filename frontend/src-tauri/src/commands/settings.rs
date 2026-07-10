//! Tauri commands for user settings management

use crate::commands::rebuild_all_indexes;
use crate::models::canvas::AvailableModel;
use crate::models::settings::{SettingsStatus, SettingsUpdate, UserSettings};
use crate::services::ollama::OllamaStatus;
use crate::AppState;
use tauri::State;

/// Get current settings
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<UserSettings, String> {
    let settings = state.settings_service.read().await;
    Ok(redact_sensitive_settings(settings.get()))
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
    update: SettingsUpdate,
) -> Result<UserSettings, String> {
    // Capture values before moving update
    let new_api_key = update.openrouter_api_key.clone();

    // Update settings
    let mut settings = state.settings_service.write().await;
    let vault_path_changed = vault_path_update_changed(settings.get(), update.vault_path.as_deref());
    let result = settings.update(update).map_err(|e| e.to_string())?;

    // Get new vault path while we still hold the lock
    let new_vault_path = if vault_path_changed {
        Some(settings.get().effective_vault_path())
    } else {
        None
    };
    let new_twin_path = if vault_path_changed {
        Some(settings.get().effective_twin_data_path())
    } else {
        None
    };
    drop(settings); // Release settings lock before acquiring others

    // Sync OpenRouter service if API key was updated
    if let Some(api_key) = new_api_key {
        let mut openrouter = state.openrouter.write().await;
        openrouter.set_api_key(api_key);
        log::info!("OpenRouter API key updated from settings");
    }

    {
        let settings = state.settings_service.read().await;
        let mut ollama = state.ollama.write().await;
        ollama.set_base_url(settings.get().ollama_base_url.clone());
    }

    // Sync KnowledgeStore, SearchService, and GraphIndex if vault path changed
    if let Some(new_vault_path) = new_vault_path {
        // Update KnowledgeStore to new vault
        {
            let mut ks = state.knowledge_store.write().await;
            ks.set_vault_path(new_vault_path);
        }

        rebuild_all_indexes(state.inner()).await?;
        if let Some(new_twin_path) = new_twin_path {
            let mut twin_store = state.twin_store.write().await;
            *twin_store = crate::services::twin::TwinStore::new(new_twin_path);
        }
        log::info!("Services rebuilt for new vault path");
    }

    Ok(redact_sensitive_settings(&result))
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
        .set_directory(dirs::document_dir().unwrap_or_else(|| std::path::PathBuf::from(".")))
        .pick_folder(move |folder_path| {
            let path = folder_path.map(|p| p.to_string_lossy().to_string());
            let _ = tx.send(path);
        });

    // Wait for the dialog result
    rx.recv().map_err(|e| format!("Dialog error: {}", e))
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

#[tauri::command]
pub async fn get_ollama_status(state: State<'_, AppState>) -> Result<OllamaStatus, String> {
    let selected_model = {
        let settings = state.settings_service.read().await;
        settings.get().ollama_model.clone()
    };
    let ollama = state.ollama.read().await;
    ollama
        .status(Some(&selected_model))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_ollama_models(state: State<'_, AppState>) -> Result<Vec<AvailableModel>, String> {
    let ollama = state.ollama.read().await;
    ollama
        .list_models()
        .await
        .map_err(|error| error.to_string())
}

#[derive(serde::Serialize)]
pub struct OpenRouterStatus {
    pub has_key: bool,
    pub is_configured: bool,
}

fn redact_sensitive_settings(settings: &UserSettings) -> UserSettings {
    let mut redacted = settings.clone();
    redacted.openrouter_api_key = None;
    redacted
}

fn vault_path_update_changed(settings: &UserSettings, candidate: Option<&str>) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };

    comparable_path(settings.effective_vault_path())
        != comparable_path(std::path::PathBuf::from(candidate))
}

fn comparable_path(path: std::path::PathBuf) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_vault_path_update_is_not_changed() {
        let mut settings = UserSettings::default();
        settings.vault_path = Some("C:\\Vault".to_string());

        assert!(!vault_path_update_changed(&settings, Some("C:/Vault")));
    }

    #[test]
    fn different_vault_path_update_is_changed() {
        let mut settings = UserSettings::default();
        settings.vault_path = Some("C:\\Vault".to_string());

        assert!(vault_path_update_changed(&settings, Some("C:/OtherVault")));
    }
}
