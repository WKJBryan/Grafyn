// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod models;
mod services;

use services::{
    canvas_store::CanvasStore,
    feedback::FeedbackService,
    graph_index::GraphIndex,
    knowledge_store::KnowledgeStore,
    memory::MemoryService,
    openrouter::OpenRouterService,
    search::SearchService,
    settings::SettingsService,
};
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;

/// Application state holding all services
pub struct AppState {
    pub knowledge_store: Arc<RwLock<KnowledgeStore>>,
    pub graph_index: Arc<RwLock<GraphIndex>>,
    pub search_service: Arc<RwLock<SearchService>>,
    pub canvas_store: Arc<RwLock<CanvasStore>>,
    pub openrouter: Arc<RwLock<OpenRouterService>>,
    pub feedback_service: Arc<RwLock<FeedbackService>>,
    pub settings_service: Arc<RwLock<SettingsService>>,
    /// MemoryService is stateless — no lock needed, just Arc for shared ownership
    pub memory_service: Arc<MemoryService>,
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
            let canvas_store = CanvasStore::new(data_path.join("canvas"));

            // Get OpenRouter API key from settings, fall back to environment
            let api_key = settings_service
                .openrouter_api_key()
                .map(|s| s.to_string())
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
                .unwrap_or_default();
            let openrouter = OpenRouterService::new(api_key);

            // Initialize feedback service with compile-time credentials
            // These are embedded during build so users don't need to configure anything
            let feedback_service = FeedbackService::new_with_credentials(
                data_path.join("feedback"),
                get_feedback_repo(),
                get_feedback_token(),
            );

            // Build initial indices — parallelize note loading across threads
            let note_metas = knowledge_store.list_notes().unwrap_or_default();
            let note_ids: Vec<String> = note_metas.iter().map(|m| m.id.clone()).collect();

            let full_notes: Vec<_> = {
                let ks = &knowledge_store;
                let notes = std::sync::Mutex::new(Vec::with_capacity(note_ids.len()));
                let num_threads = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
                    .min(note_ids.len().max(1));
                let chunk_size = note_ids.len().div_ceil(num_threads).max(1);

                std::thread::scope(|s| {
                    for chunk in note_ids.chunks(chunk_size) {
                        let notes = &notes;
                        s.spawn(move || {
                            let mut batch = Vec::new();
                            for id in chunk {
                                if let Ok(note) = ks.get_note(id) {
                                    batch.push(note);
                                }
                            }
                            notes.lock().unwrap().extend(batch);
                        });
                    }
                });

                notes.into_inner().unwrap()
            };

            let mut graph_index = graph_index;
            graph_index.build_from_notes(&full_notes);

            // Create app state (MemoryService is stateless — no RwLock needed)
            let state = AppState {
                knowledge_store: Arc::new(RwLock::new(knowledge_store)),
                graph_index: Arc::new(RwLock::new(graph_index)),
                search_service: Arc::new(RwLock::new(search_service)),
                canvas_store: Arc::new(RwLock::new(canvas_store)),
                openrouter: Arc::new(RwLock::new(openrouter)),
                feedback_service: Arc::new(RwLock::new(feedback_service)),
                settings_service: Arc::new(RwLock::new(settings_service)),
                memory_service: Arc::new(MemoryService::new()),
            };

            app.manage(state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
            // Zettelkasten commands
            commands::zettelkasten::discover_links,
            commands::zettelkasten::apply_links,
            commands::zettelkasten::create_link,
            commands::zettelkasten::get_link_types,
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            log::error!("Error while running tauri application: {}", e);
            std::process::exit(1);
        });
}

/// Get feedback repository from compile-time env or runtime env
/// Priority: compile-time > runtime env
fn get_feedback_repo() -> String {
    // First try compile-time env (embedded in binary during build)
    option_env!("GITHUB_FEEDBACK_REPO")
        .map(|s| s.to_string())
        // Fall back to runtime env (for development)
        .or_else(|| std::env::var("GITHUB_FEEDBACK_REPO").ok())
        .unwrap_or_default()
}

/// Get feedback token from compile-time env or runtime env
/// Priority: compile-time > runtime env
fn get_feedback_token() -> String {
    // First try compile-time env (embedded in binary during build)
    option_env!("GITHUB_FEEDBACK_TOKEN")
        .map(|s| s.to_string())
        // Fall back to runtime env (for development)
        .or_else(|| std::env::var("GITHUB_FEEDBACK_TOKEN").ok())
        .unwrap_or_default()
}
