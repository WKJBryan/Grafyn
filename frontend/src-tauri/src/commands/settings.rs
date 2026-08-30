//! Tauri commands for user settings management

use crate::models::canvas::AvailableModel;
use crate::models::note::Note;
use crate::models::settings::{SettingsStatus, SettingsUpdate, UserSettings};
use crate::services::ollama::OllamaStatus;
use crate::AppState;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

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
    let updated = apply_settings_update(state.inner(), update).await?;
    Ok(redact_sensitive_settings(&updated))
}

pub(crate) async fn apply_settings_update(
    state: &AppState,
    update: SettingsUpdate,
) -> Result<UserSettings, String> {
    let _transition = state.vault_transition.lock().await;
    let new_api_key = update.openrouter_api_key.clone();
    let before = {
        let settings = state.settings_service.read().await;
        settings.get().clone()
    };
    let vault_path_changed = vault_path_update_changed(&before, update.vault_path.as_deref());

    let result = if vault_path_changed {
        let candidate = std::path::PathBuf::from(
            update
                .vault_path
                .as_deref()
                .expect("changed vault update has a candidate"),
        );
        crate::services::twin_events::validate_real_directory(&candidate, "vault directory")
            .map_err(|error| error.to_string())?;
        let candidate = std::fs::canonicalize(candidate).map_err(|error| error.to_string())?;

        let mut knowledge = state.knowledge_store.write().await;
        let mut twin = state.twin_store.write().await;
        let old_vault = knowledge.vault_path().to_path_buf();
        let old_twin = twin.root_path().to_path_buf();
        let old_notes = knowledge
            .list_full_notes()
            .map_err(|error| error.to_string())?;
        if comparable_path(before.effective_vault_path()) != comparable_path(old_vault.clone()) {
            return Err(
                "vault runtime and persisted settings disagree; restart before switching"
                    .to_string(),
            );
        }
        let data_path = twin
            .target_root_path()
            .parent()
            .ok_or_else(|| "Twin target root has no data parent".to_string())?;
        let candidate_twin =
            crate::models::settings::twin_data_path_for_vault(data_path, &candidate);

        knowledge
            .set_vault_path(candidate.clone())
            .map_err(|error| error.to_string())?;
        if let Err(error) = twin.replace_root_path(candidate_twin) {
            let rollback = knowledge.set_vault_path(old_vault.clone());
            return Err(match rollback {
                Ok(()) => error.to_string(),
                Err(rollback) => format!("{error}; vault rollback failed: {rollback}"),
            });
        }

        let new_notes = match knowledge.list_full_notes() {
            Ok(notes) => notes,
            Err(error) => {
                rollback_vault_roots(
                    state,
                    &mut knowledge,
                    &mut twin,
                    &old_vault,
                    &old_twin,
                    &old_notes,
                )
                .await?;
                return Err(error.to_string());
            }
        };
        if let Err(error) = rebuild_indexes_from_notes(state, &new_notes).await {
            rollback_vault_roots(
                state,
                &mut knowledge,
                &mut twin,
                &old_vault,
                &old_twin,
                &old_notes,
            )
            .await?;
            return Err(error);
        }

        let persisted = {
            let mut settings = state.settings_service.write().await;
            settings.update(update).map_err(|error| error.to_string())
        };
        match persisted {
            Ok(result) => result,
            Err(error) => {
                rollback_vault_roots(
                    state,
                    &mut knowledge,
                    &mut twin,
                    &old_vault,
                    &old_twin,
                    &old_notes,
                )
                .await?;
                return Err(error);
            }
        }
    } else {
        let mut settings = state.settings_service.write().await;
        settings.update(update).map_err(|error| error.to_string())?
    };

    // Sync OpenRouter service if API key was updated
    if let Some(api_key) = new_api_key {
        let mut openrouter = state.openrouter.write().await;
        openrouter.set_api_key(api_key);
        log::info!("OpenRouter API key updated from settings");
    }

    let mut ollama = state.ollama.write().await;
    ollama.set_base_url(result.ollama_base_url.clone());
    Ok(result)
}

async fn rebuild_indexes_from_notes(state: &AppState, notes: &[Note]) -> Result<(), String> {
    {
        let mut search = state.search_service.write().await;
        search
            .reindex_all(notes)
            .map_err(|error| error.to_string())?;
    }
    {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(notes);
    }
    {
        let mut chunks = state.chunk_index.write().await;
        chunks
            .reindex_all(notes)
            .map_err(|error| error.to_string())?;
    }
    crate::commands::rebuild_link_discovery(state, notes).await;
    crate::commands::bootstrap_vault_optimizer(state, notes).await;
    Ok(())
}

async fn rollback_vault_roots(
    state: &AppState,
    knowledge: &mut crate::services::knowledge_store::KnowledgeStore,
    twin: &mut crate::services::twin::TwinStore,
    old_vault: &std::path::Path,
    old_twin: &std::path::Path,
    old_notes: &[Note],
) -> Result<(), String> {
    knowledge
        .set_vault_path(old_vault.to_path_buf())
        .map_err(|error| format!("vault rollback failed: {error}"))?;
    twin.replace_root_path(old_twin.to_path_buf())
        .map_err(|error| format!("Twin rollback failed: {error}"))?;
    rebuild_indexes_from_notes(state, old_notes)
        .await
        .map_err(|error| format!("index rollback failed: {error}"))
}

/// Complete initial setup
#[tauri::command]
pub async fn complete_setup(state: State<'_, AppState>) -> Result<(), String> {
    let mut settings = state.settings_service.write().await;
    settings.complete_setup().map_err(|e| e.to_string())
}

/// Open folder picker dialog for vault selection
#[tauri::command]
pub async fn pick_vault_folder(app: AppHandle) -> Result<Option<String>, String> {
    #[cfg(desktop)]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .set_title("Select Vault Folder")
            .set_directory(dirs::document_dir().unwrap_or_else(|| std::path::PathBuf::from(".")))
            .pick_folder(move |folder_path| {
                let _ = tx.send(folder_path.map(|path| path.to_string()));
            });

        return rx.await.map_err(|e| format!("Dialog error: {}", e));
    }

    #[cfg(mobile)]
    {
        let _ = app;
        Err("Vault folder selection is unavailable on mobile".to_string())
    }
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
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase();
    normalized
        .strip_prefix("//?/")
        .unwrap_or(&normalized)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    fn vault_update(path: &std::path::Path) -> SettingsUpdate {
        SettingsUpdate {
            vault_path: Some(path.to_string_lossy().into_owned()),
            openrouter_api_key: None,
            setup_completed: None,
            theme: None,
            mcp_enabled: None,
            llm_model: None,
            twin_llm_provider: None,
            ollama_base_url: None,
            ollama_model: None,
            smart_web_search: None,
            background_link_discovery_enabled: None,
            background_link_discovery_llm_enabled: None,
            background_vault_optimizer_enabled: None,
            background_vault_optimizer_llm_enabled: None,
            background_vault_optimizer_budget_monthly: None,
            background_vault_optimizer_max_daily_writes: None,
            background_vault_optimizer_edit_mode: None,
            background_vault_optimizer_program_enabled: None,
            vault_optimizer_program_path: None,
            canvas_model_presets: None,
        }
    }

    fn root_switch_state(
        settings_path_is_directory: bool,
        readonly_search: bool,
    ) -> (AppState, TempDir, std::path::PathBuf, std::path::PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let old_vault = root.path().join("vault-a");
        let new_vault = root.path().join("vault-b");
        let data = root.path().join("data");
        std::fs::create_dir(&old_vault).unwrap();
        std::fs::create_dir(&new_vault).unwrap();
        std::fs::create_dir(&data).unwrap();
        let events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &old_vault,
                events.clone(),
                Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let user_settings = UserSettings {
            vault_path: Some(old_vault.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let config_path = if settings_path_is_directory {
            let path = root.path().join("settings-as-directory");
            std::fs::create_dir(&path).unwrap();
            path
        } else {
            root.path().join("settings.json")
        };
        let settings =
            crate::services::settings::SettingsService::for_test(config_path, user_settings);
        let search = if readonly_search {
            drop(crate::services::search::SearchService::new(data.clone()).unwrap());
            crate::services::search::SearchService::new_readonly(data.clone()).unwrap()
        } else {
            crate::services::search::SearchService::new(data.clone()).unwrap()
        };
        let twin_root = crate::models::settings::twin_data_path_for_vault(&data, &old_vault);
        let state = AppState {
            knowledge_store: Arc::new(RwLock::new(
                crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
                    old_vault.clone(),
                    data.clone(),
                    coordinator.clone(),
                ),
            )),
            graph_index: Arc::new(RwLock::new(crate::services::graph_index::GraphIndex::new())),
            search_service: Arc::new(RwLock::new(search)),
            canvas_store: Arc::new(RwLock::new(
                crate::services::canvas_store::CanvasStore::with_event_recorder(
                    data.join("canvas"),
                    coordinator.clone(),
                ),
            )),
            openrouter: Arc::new(RwLock::new(
                crate::services::openrouter::OpenRouterService::new(String::new()),
            )),
            ollama: Arc::new(RwLock::new(crate::services::ollama::OllamaService::new(
                "http://localhost:11434".into(),
            ))),
            feedback_service: Arc::new(RwLock::new(
                crate::services::feedback::FeedbackService::new(data.join("feedback")),
            )),
            settings_service: Arc::new(RwLock::new(settings)),
            priority_service: Arc::new(RwLock::new(
                crate::services::priority::PriorityScoringService::new(data.clone()),
            )),
            retrieval_service: Arc::new(RwLock::new(
                crate::services::retrieval::RetrievalService::new(data.clone()),
            )),
            chunk_index: Arc::new(RwLock::new(
                crate::services::chunk_index::ChunkIndex::new(data.clone()).unwrap(),
            )),
            link_discovery: Arc::new(RwLock::new(
                crate::services::link_discovery::LinkDiscoveryService::new(data.clone()),
            )),
            markdown_migration: Arc::new(RwLock::new(
                crate::services::markdown_migration::MarkdownMigrationService::new(data.clone()),
            )),
            vault_optimizer: Arc::new(RwLock::new(
                crate::services::vault_optimizer::VaultOptimizerService::new(data.clone()),
            )),
            twin_store: Arc::new(RwLock::new(
                crate::services::twin::TwinStore::with_event_recorder(
                    twin_root,
                    data.join("twin"),
                    coordinator,
                ),
            )),
            twin_event_store: events,
            mutation_startup_error: None,
            vault_transition: Arc::new(tokio::sync::Mutex::new(())),
            memory_service: Arc::new(crate::services::memory::MemoryService::new()),
            boot_state: Arc::new(RwLock::new(crate::models::boot::BootStatus::default())),
        };
        (state, root, old_vault, new_vault)
    }

    #[test]
    fn same_vault_path_update_is_not_changed() {
        let settings = UserSettings {
            vault_path: Some("C:\\Vault".to_string()),
            ..Default::default()
        };

        assert!(!vault_path_update_changed(&settings, Some("C:/Vault")));
    }

    #[test]
    fn different_vault_path_update_is_changed() {
        let settings = UserSettings {
            vault_path: Some("C:\\Vault".to_string()),
            ..Default::default()
        };

        assert!(vault_path_update_changed(&settings, Some("C:/OtherVault")));
    }

    #[tokio::test]
    async fn vault_switch_publishes_settings_after_knowledge_twin_and_indexes() {
        let (state, _root, old_vault, new_vault) = root_switch_state(false, false);
        let updated = apply_settings_update(&state, vault_update(&new_vault))
            .await
            .unwrap();
        assert_eq!(
            comparable_path(updated.effective_vault_path()),
            comparable_path(new_vault.clone())
        );
        assert_eq!(
            comparable_path(
                state
                    .knowledge_store
                    .read()
                    .await
                    .vault_path()
                    .to_path_buf()
            ),
            comparable_path(new_vault.clone())
        );
        let twin = state.twin_store.read().await;
        assert_eq!(
            twin.root_path(),
            crate::models::settings::twin_data_path_for_vault(
                twin.target_root_path().parent().unwrap(),
                &new_vault,
            )
        );
        drop(twin);
        assert_ne!(comparable_path(old_vault), comparable_path(new_vault));
    }

    #[tokio::test]
    async fn settings_persist_failure_rolls_back_live_knowledge_twin_and_lease() {
        let (state, _root, old_vault, new_vault) = root_switch_state(true, false);
        let old_twin = state.twin_store.read().await.root_path().to_path_buf();
        assert!(apply_settings_update(&state, vault_update(&new_vault))
            .await
            .is_err());
        assert_eq!(
            comparable_path(state.settings_service.read().await.vault_path()),
            comparable_path(old_vault.clone())
        );
        assert_eq!(
            comparable_path(
                state
                    .knowledge_store
                    .read()
                    .await
                    .vault_path()
                    .to_path_buf()
            ),
            comparable_path(old_vault)
        );
        assert_eq!(state.twin_store.read().await.root_path(), old_twin);
    }

    #[tokio::test]
    async fn index_rebuild_failure_rolls_back_roots_before_settings_publish() {
        let (state, _root, old_vault, new_vault) = root_switch_state(false, true);
        let old_twin = state.twin_store.read().await.root_path().to_path_buf();
        assert!(apply_settings_update(&state, vault_update(&new_vault))
            .await
            .is_err());
        assert_eq!(
            comparable_path(state.settings_service.read().await.vault_path()),
            comparable_path(old_vault.clone())
        );
        assert_eq!(
            comparable_path(
                state
                    .knowledge_store
                    .read()
                    .await
                    .vault_path()
                    .to_path_buf()
            ),
            comparable_path(old_vault)
        );
        assert_eq!(state.twin_store.read().await.root_path(), old_twin);
    }

    #[tokio::test]
    async fn concurrent_note_command_observes_exactly_old_or_new_vault_boundary() {
        let (state, _root, old_vault, new_vault) = root_switch_state(false, false);
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let switch_state = state.clone();
        let switch_barrier = barrier.clone();
        let switch_path = new_vault.clone();
        let switch = tokio::spawn(async move {
            switch_barrier.wait().await;
            apply_settings_update(&switch_state, vault_update(&switch_path)).await
        });
        let note_state = state.clone();
        let note =
            tokio::spawn(async move {
                barrier.wait().await;
                note_state.knowledge_store.write().await.create_note(
                    crate::models::note::NoteCreate {
                        title: "Root boundary marker".into(),
                        content: "root-switch-boundary-marker".into(),
                        relative_path: None,
                        aliases: Vec::new(),
                        status: crate::models::note::NoteStatus::Draft,
                        tags: Vec::new(),
                        schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                        migration_source: None,
                        optimizer_managed: false,
                        properties: Default::default(),
                    },
                )
            });
        switch.await.unwrap().unwrap();
        note.await.unwrap().unwrap();

        let contains_marker = |root: &std::path::Path| {
            walkdir::WalkDir::new(root)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file())
                .any(|entry| {
                    std::fs::read_to_string(entry.path())
                        .is_ok_and(|value| value.contains("root-switch-boundary-marker"))
                })
        };
        assert_ne!(contains_marker(&old_vault), contains_marker(&new_vault));
    }
}
