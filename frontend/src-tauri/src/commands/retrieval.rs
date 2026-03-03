use crate::services::retrieval::{RetrievalConfig, RetrievalConfigUpdate, RetrievalResult};
use crate::AppState;
use tauri::State;

/// Retrieve relevant notes using the temporal + graph-aware retrieval pipeline
#[tauri::command]
pub async fn retrieve_relevant(
    query: String,
    limit: Option<usize>,
    context_note_ids: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<Vec<RetrievalResult>, String> {
    let limit = limit.unwrap_or(10);
    let context_ids = context_note_ids.unwrap_or_default();

    let search = state.search_service.read().await;
    let graph = state.graph_index.read().await;
    let priority = state.priority_service.read().await;
    let retrieval = state.retrieval_service.read().await;

    retrieval.retrieve(&search, &graph, &priority, &query, limit, &context_ids)
}

/// Get current retrieval configuration
#[tauri::command]
pub async fn get_retrieval_config(
    state: State<'_, AppState>,
) -> Result<RetrievalConfig, String> {
    let retrieval = state.retrieval_service.read().await;
    Ok(retrieval.get_config().clone())
}

/// Update retrieval configuration
#[tauri::command]
pub async fn update_retrieval_config(
    update: RetrievalConfigUpdate,
    state: State<'_, AppState>,
) -> Result<RetrievalConfig, String> {
    let mut retrieval = state.retrieval_service.write().await;
    retrieval.update_config(update).map_err(|e| e.to_string())
}
