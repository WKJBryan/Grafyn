use crate::models::memory::{ExtractRequest, ExtractedClaim, Contradiction, RecallRequest, RecallResult};
use crate::AppState;
use tauri::State;

/// Recall relevant notes using the temporal + graph-aware retrieval pipeline
#[tauri::command]
pub async fn recall_relevant(
    request: RecallRequest,
    state: State<'_, AppState>,
) -> Result<Vec<RecallResult>, String> {
    let search = state.search_service.read().await;
    let graph = state.graph_index.read().await;
    let priority = state.priority_service.read().await;
    let retrieval = state.retrieval_service.read().await;

    let results = retrieval.retrieve(
        &search,
        &graph,
        &priority,
        &request.query,
        request.limit,
        &request.context_note_ids,
    )?;

    // Convert RetrievalResult → RecallResult
    Ok(results
        .into_iter()
        .map(|r| RecallResult {
            note_id: r.note.id,
            title: r.note.title,
            snippet: r.snippet,
            score: r.score,
            tags: r.note.tags,
            graph_boost: 0.0, // now integrated into the composite score
            total_score: r.score,
        })
        .collect())
}

/// Find contradictions for a note
#[tauri::command]
pub async fn find_contradictions(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Contradiction>, String> {
    let search = state.search_service.read().await;
    let store = state.knowledge_store.read().await;

    state.memory_service.find_contradictions(&search, &store, &note_id)
}

/// Extract claims from conversation
#[tauri::command]
pub async fn extract_claims(
    request: ExtractRequest,
    state: State<'_, AppState>,
) -> Result<Vec<ExtractedClaim>, String> {
    Ok(state.memory_service.extract_from_conversation(&request.messages))
}
