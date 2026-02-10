use crate::models::memory::{ExtractRequest, ExtractedClaim, Contradiction, RecallRequest, RecallResult};
use crate::AppState;
use tauri::State;

/// Recall relevant notes with graph-aware boosting
#[tauri::command]
pub async fn recall_relevant(
    request: RecallRequest,
    state: State<'_, AppState>,
) -> Result<Vec<RecallResult>, String> {
    let search = state.search_service.read().await;
    let graph = state.graph_index.read().await;
    let memory = state.memory_service.read().await;

    memory.recall_relevant(
        &search,
        &graph,
        &request.query,
        &request.context_note_ids,
        request.limit,
    )
}

/// Find contradictions for a note
#[tauri::command]
pub async fn find_contradictions(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Contradiction>, String> {
    let search = state.search_service.read().await;
    let store = state.knowledge_store.read().await;
    let memory = state.memory_service.read().await;

    memory.find_contradictions(&search, &store, &note_id)
}

/// Extract claims from conversation
#[tauri::command]
pub async fn extract_claims(
    request: ExtractRequest,
    state: State<'_, AppState>,
) -> Result<Vec<ExtractedClaim>, String> {
    let memory = state.memory_service.read().await;
    Ok(memory.extract_from_conversation(&request.messages))
}
