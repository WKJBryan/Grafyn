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
    openrouter::OpenRouterService,
    search::SearchService,
};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;

/// Application state holding all services
pub struct AppState {
    pub knowledge_store: Arc<RwLock<KnowledgeStore>>,
    pub graph_index: Arc<RwLock<GraphIndex>>,
    pub search_service: Arc<RwLock<SearchService>>,
    pub canvas_store: Arc<RwLock<CanvasStore>>,
    pub openrouter: Arc<OpenRouterService>,
    pub feedback_service: Arc<RwLock<FeedbackService>>,
}

fn get_data_paths() -> (PathBuf, PathBuf) {
    // Get user's documents directory
    let documents = dirs::document_dir()
        .unwrap_or_else(|| PathBuf::from("."));

    let base_path = documents.join("Seedream");
    let vault_path = base_path.join("vault");
    let data_path = base_path.join("data");

    // Create directories if they don't exist
    std::fs::create_dir_all(&vault_path).ok();
    std::fs::create_dir_all(&data_path).ok();

    (vault_path, data_path)
}

fn main() {
    env_logger::init();

    tauri::Builder::default()
        .setup(|app| {
            let (vault_path, data_path) = get_data_paths();

            log::info!("Vault path: {:?}", vault_path);
            log::info!("Data path: {:?}", data_path);

            // Initialize services
            let knowledge_store = KnowledgeStore::new(vault_path.clone());
            let graph_index = GraphIndex::new();
            let search_service = SearchService::new(data_path.clone())
                .expect("Failed to initialize search service");
            let canvas_store = CanvasStore::new(data_path.join("canvas"));

            // Get OpenRouter API key from environment
            let api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
            let openrouter = OpenRouterService::new(api_key);

            // Initialize feedback service for bug reports and feature requests
            let feedback_service = FeedbackService::new(data_path.join("feedback"));

            // Build initial indices
            let mut ks = knowledge_store.clone();
            let notes = ks.list_notes().unwrap_or_default();

            let mut gi = graph_index.clone();
            gi.build_index(&notes);

            // Create app state
            let state = AppState {
                knowledge_store: Arc::new(RwLock::new(knowledge_store)),
                graph_index: Arc::new(RwLock::new(graph_index)),
                search_service: Arc::new(RwLock::new(search_service)),
                canvas_store: Arc::new(RwLock::new(canvas_store)),
                openrouter: Arc::new(openrouter),
                feedback_service: Arc::new(RwLock::new(feedback_service)),
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
            // Feedback commands
            commands::feedback::submit_feedback,
            commands::feedback::get_system_info,
            commands::feedback::feedback_status,
            commands::feedback::get_pending_feedback,
            commands::feedback::retry_pending_feedback,
            commands::feedback::clear_pending_feedback,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
