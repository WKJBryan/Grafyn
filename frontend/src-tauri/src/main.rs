// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod models;
mod services;

use models::boot::BootStatus;
use services::{
    canvas_store::CanvasStore,
    chunk_index::ChunkIndex,
    feedback::FeedbackService,
    graph_index::GraphIndex,
    knowledge_store::KnowledgeStore,
    memory::MemoryService,
    openrouter::OpenRouterService,
    priority::PriorityScoringService,
    retrieval::RetrievalService,
    search::SearchService,
    settings::SettingsService,
    twin_store::TwinStore,
};
use std::sync::Arc;
use std::time::Instant;
use tauri::Manager;
use tokio::sync::RwLock;

/// Application state holding all services
#[derive(Clone)]
pub struct AppState {
    pub knowledge_store: Arc<RwLock<KnowledgeStore>>,
    pub graph_index: Arc<RwLock<GraphIndex>>,
    pub search_service: Arc<RwLock<SearchService>>,
    pub canvas_store: Arc<RwLock<CanvasStore>>,
    pub openrouter: Arc<RwLock<OpenRouterService>>,
    pub feedback_service: Arc<RwLock<FeedbackService>>,
    pub settings_service: Arc<RwLock<SettingsService>>,
    pub priority_service: Arc<RwLock<PriorityScoringService>>,
    pub retrieval_service: Arc<RwLock<RetrievalService>>,
    pub chunk_index: Arc<RwLock<ChunkIndex>>,
    pub twin_store: Arc<RwLock<TwinStore>>,
    /// MemoryService is stateless — no lock needed, just Arc for shared ownership
    pub memory_service: Arc<MemoryService>,
    pub boot_state: Arc<RwLock<BootStatus>>,
}

fn main() {
    env_logger::init();

    tauri::Builder::default()
        .setup(|app| {
            // Load user settings first (fall back to defaults on error)
            let settings_service = match SettingsService::load() {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Failed to load settings: {}. Using defaults.", e);
                    SettingsService::load_defaults()
                }
            };

            let vault_path = settings_service.vault_path();
            let data_path = settings_service.data_path();

            log::info!("Vault path: {:?}", vault_path);
            log::info!("Data path: {:?}", data_path);

            // Create directories if they don't exist
            if let Err(e) = std::fs::create_dir_all(&vault_path) {
                log::error!("Failed to create vault directory {}: {}", vault_path.display(), e);
            }
            if let Err(e) = std::fs::create_dir_all(&data_path) {
                log::error!("Failed to create data directory {}: {}", data_path.display(), e);
            }

            // Initialize services
            let knowledge_store = KnowledgeStore::new(vault_path.clone());
            let graph_index = GraphIndex::new();
            let search_service = match SearchService::new(data_path.clone()) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("Failed to initialize search service: {}. Attempting index rebuild.", e);
                    // Try deleting corrupted index and retrying
                    let index_path = data_path.join("search_index");
                    if index_path.exists() {
                        if let Err(rm_err) = std::fs::remove_dir_all(&index_path) {
                            log::error!("Failed to remove corrupted index: {}", rm_err);
                        }
                    }
                    SearchService::new(data_path.clone()).unwrap_or_else(|e2| {
                        log::error!("Search service initialization failed after rebuild: {}", e2);
                        std::process::exit(1);
                    })
                }
            };
            // Initialize chunk index (parallel to search index)
            let chunk_index = match ChunkIndex::new(data_path.clone()) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("Failed to initialize chunk index: {}. Attempting rebuild.", e);
                    let chunk_path = data_path.join("chunk_index");
                    if chunk_path.exists() {
                        let _ = std::fs::remove_dir_all(&chunk_path);
                    }
                    ChunkIndex::new(data_path.clone()).unwrap_or_else(|e2| {
                        log::error!("Chunk index initialization failed: {}", e2);
                        std::process::exit(1);
                    })
                }
            };

            let canvas_store = CanvasStore::new(data_path.join("canvas"));
            let twin_store = TwinStore::new(data_path.join("twin"));

            // Get OpenRouter API key from settings, fall back to environment
            let api_key = settings_service
                .openrouter_api_key()
                .map(|s| s.to_string())
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
                .unwrap_or_default();
            let openrouter = OpenRouterService::new(api_key);

            // Initialize priority scoring service
            let priority_service = PriorityScoringService::new(data_path.clone());

            // Initialize retrieval service
            let retrieval_service = RetrievalService::new(data_path.clone());

            // Initialize feedback service using runtime environment only.
            // Release builds must not embed repository credentials.
            let feedback_service = FeedbackService::new(data_path.join("feedback"));
            let boot_state = Arc::new(RwLock::new(BootStatus::default()));

            // Create app state (MemoryService is stateless — no RwLock needed)
            let state = AppState {
                knowledge_store: Arc::new(RwLock::new(knowledge_store)),
                graph_index: Arc::new(RwLock::new(graph_index)),
                search_service: Arc::new(RwLock::new(search_service)),
                canvas_store: Arc::new(RwLock::new(canvas_store)),
                openrouter: Arc::new(RwLock::new(openrouter)),
                feedback_service: Arc::new(RwLock::new(feedback_service)),
                settings_service: Arc::new(RwLock::new(settings_service)),
                priority_service: Arc::new(RwLock::new(priority_service)),
                retrieval_service: Arc::new(RwLock::new(retrieval_service)),
                chunk_index: Arc::new(RwLock::new(chunk_index)),
                twin_store: Arc::new(RwLock::new(twin_store)),
                memory_service: Arc::new(MemoryService::new()),
                boot_state,
            };

            app.manage(state);

            let app_handle = app.handle();
            let state = app.state::<AppState>().inner().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = warm_start_services(app_handle.clone(), state.clone()).await {
                    publish_boot_status(
                        &app_handle,
                        &state,
                        BootStatus::failed("failed", "Startup failed", error),
                    )
                    .await;
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::boot::get_boot_status,
            // Note commands
            commands::notes::list_notes,
            commands::notes::get_note,
            commands::notes::create_note,
            commands::notes::update_note,
            commands::notes::delete_note,
            // Search commands
            commands::search::search_notes,
            commands::search::find_similar,
            commands::search::reindex,
            // Graph commands
            commands::graph::get_backlinks,
            commands::graph::get_outgoing,
            commands::graph::get_neighbors,
            commands::graph::get_unlinked,
            commands::graph::get_full_graph,
            commands::graph::rebuild_graph,
            // Canvas commands
            commands::canvas::list_sessions,
            commands::canvas::get_session,
            commands::canvas::create_session,
            commands::canvas::update_session,
            commands::canvas::delete_session,
            commands::canvas::get_available_models,
            commands::canvas::send_prompt,
            commands::canvas::update_tile_position,
            commands::canvas::delete_tile,
            commands::canvas::delete_response,
            commands::canvas::update_viewport,
            commands::canvas::update_llm_node_position,
            commands::canvas::auto_arrange,
            commands::canvas::export_to_note,
            commands::canvas::start_debate,
            commands::canvas::continue_debate,
            commands::canvas::add_models_to_tile,
            commands::canvas::regenerate_response,
            // Twin collector commands
            commands::twin::list_user_records,
            commands::twin::get_user_record,
            commands::twin::create_user_record,
            commands::twin::update_user_record,
            commands::twin::get_session_trace,
            commands::twin::record_canvas_feedback,
            commands::twin::export_twin_data,
            // Feedback commands
            commands::feedback::submit_feedback,
            commands::feedback::get_system_info,
            commands::feedback::feedback_status,
            commands::feedback::get_pending_feedback,
            commands::feedback::retry_pending_feedback,
            commands::feedback::clear_pending_feedback,
            // Settings commands
            commands::settings::get_settings,
            commands::settings::get_settings_status,
            commands::settings::update_settings,
            commands::settings::complete_setup,
            commands::settings::pick_vault_folder,
            commands::settings::validate_openrouter_key,
            commands::settings::get_openrouter_status,
            // Distill commands
            commands::distill::distill_note,
            commands::distill::normalize_tags,
            // MCP commands
            commands::mcp::get_mcp_status,
            commands::mcp::get_mcp_config_snippet,
            // Memory commands
            commands::memory::recall_relevant,
            commands::memory::find_contradictions,
            commands::memory::extract_claims,
            // Priority commands
            commands::priority::get_priority_settings,
            commands::priority::update_priority_settings,
            commands::priority::reset_priority_settings,
            // Zettelkasten commands
            commands::zettelkasten::discover_links,
            commands::zettelkasten::apply_links,
            commands::zettelkasten::create_link,
            commands::zettelkasten::get_link_types,
            // Import commands
            commands::import::preview_import,
            commands::import::apply_import,
            commands::import::get_supported_formats,
            // Retrieval commands
            commands::retrieval::retrieve_relevant,
            commands::retrieval::get_retrieval_config,
            commands::retrieval::update_retrieval_config,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            log::error!("Error while running tauri application: {}", e);
            std::process::exit(1);
        });
}

async fn warm_start_services(app_handle: tauri::AppHandle, state: AppState) -> Result<(), String> {
    let boot_started = Instant::now();

    publish_boot_phase(
        &app_handle,
        &state,
        &boot_started,
        BootStatus::new("opening_store", "Loading notes from your vault"),
    )
    .await;

    let full_notes = {
        let store = state.knowledge_store.read().await;
        let note_metas = store.list_notes().map_err(|e| e.to_string())?;
        let note_ids: Vec<String> = note_metas.iter().map(|m| m.id.clone()).collect();
        let mut notes = Vec::with_capacity(note_ids.len());

        for id in note_ids {
            if let Ok(note) = store.get_note(&id) {
                notes.push(note);
            }
        }

        notes
    };

    publish_boot_phase(
        &app_handle,
        &state,
        &boot_started,
        BootStatus::new("building_graph", "Building graph from your notes"),
    )
    .await;

    {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(&full_notes);
    }

    publish_boot_phase(
        &app_handle,
        &state,
        &boot_started,
        BootStatus::new("building_search_index", "Building search index"),
    )
    .await;

    {
        let mut search = state.search_service.write().await;
        search.reindex_all(&full_notes).map_err(|e| e.to_string())?;
    }

    publish_boot_phase(
        &app_handle,
        &state,
        &boot_started,
        BootStatus::new("building_chunk_index", "Building chunk index"),
    )
    .await;

    {
        let mut chunks = state.chunk_index.write().await;
        if let Err(e) = chunks.reindex_all(&full_notes) {
            log::error!("Failed to build chunk index: {}", e);
        }
    }

    publish_boot_phase(
        &app_handle,
        &state,
        &boot_started,
        BootStatus::ready("Grafyn is ready"),
    )
    .await;

    Ok(())
}

async fn publish_boot_phase(
    app_handle: &tauri::AppHandle,
    state: &AppState,
    boot_started: &Instant,
    status: BootStatus,
) {
    log::info!(
        "Boot phase '{}' at {:.2?}: {}",
        status.phase,
        boot_started.elapsed(),
        status.message
    );
    publish_boot_status(app_handle, state, status).await;
}

async fn update_boot_state(boot_state: &Arc<RwLock<BootStatus>>, status: &BootStatus) {
    let mut current = boot_state.write().await;
    *current = status.clone();
}

async fn publish_boot_status(app_handle: &tauri::AppHandle, state: &AppState, status: BootStatus) {
    update_boot_state(&state.boot_state, &status).await;

    let _ = app_handle.emit_all("boot-status", status);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn update_boot_state_replaces_existing_status() {
        let boot_state = Arc::new(RwLock::new(BootStatus::default()));
        let next = BootStatus::new("building_search_index", "Building search index");

        update_boot_state(&boot_state, &next).await;

        assert_eq!(*boot_state.read().await, next);
    }
}

