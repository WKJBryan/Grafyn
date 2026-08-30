use crate::commands::run_retrieval;
use crate::models::memory::{
    Contradiction, ExtractRequest, ExtractedClaim, RecallRequest, RecallResult,
};
use crate::AppState;
use tauri::State;

/// Recall relevant notes using the temporal + graph-aware retrieval pipeline
#[tauri::command]
pub async fn recall_relevant(
    request: RecallRequest,
    state: State<'_, AppState>,
) -> Result<Vec<RecallResult>, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let results =
        run_retrieval(state.inner(), &request.query, request.limit, &request.context_note_ids)
            .await?;

    let result = results
        .into_iter()
        .map(|r| RecallResult {
            note_id: r.note.id,
            title: r.note.title,
            snippet: r.snippet,
            score: r.score,
            tags: r.note.tags,
            graph_boost: 0.0,
            total_score: r.score,
        })
        .collect();
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Find contradictions for a note
#[tauri::command]
pub async fn find_contradictions(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<Contradiction>, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let note = {
        let store = state.knowledge_store.read().await;
        store.get_note(&note_id).map_err(|error| error.to_string())?
    };
    let result = {
        let search = state.search_service.read().await;
        state.memory_service.find_contradictions_for_note(&search, &note)?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Extract claims from conversation
#[tauri::command]
pub async fn extract_claims(
    request: ExtractRequest,
    state: State<'_, AppState>,
) -> Result<Vec<ExtractedClaim>, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    Ok(state
        .memory_service
        .extract_from_conversation(&request.messages))
}
