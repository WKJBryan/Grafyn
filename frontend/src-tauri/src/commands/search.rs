use crate::models::note::SearchResult;
use crate::AppState;
use tauri::State;

/// Search notes by query string, with priority scoring applied
#[tauri::command]
pub async fn search_notes(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let search = state.search_service.read().await;
    let limit = limit.unwrap_or(20);
    let mut results = search.search(&query, limit).map_err(|e| e.to_string())?;

    // Apply priority scoring (recency, status, tag boosts)
    let priority = state.priority_service.read().await;
    priority.score_results(&mut results);

    Ok(results)
}

/// Find notes similar to a given note (enhanced with graph-aware retrieval)
#[tauri::command]
pub async fn find_similar(
    note_id: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let limit = limit.unwrap_or(10);

    // Get the note content for keyword extraction
    let content = {
        let store = state.knowledge_store.read().await;
        store
            .get_note(&note_id)
            .map(|n| n.content)
            .map_err(|e| e.to_string())?
    };

    // Extract keywords from content
    let query_words: Vec<&str> = content
        .split_whitespace()
        .filter(|w| w.len() > 4)
        .take(20)
        .collect();

    if query_words.is_empty() {
        return Ok(Vec::new());
    }

    let query_str = query_words.join(" ");

    // Use retrieval pipeline with the source note as context
    let search = state.search_service.read().await;
    let graph = state.graph_index.read().await;
    let priority = state.priority_service.read().await;
    let retrieval = state.retrieval_service.read().await;

    let results = retrieval.retrieve(
        &search,
        &graph,
        &priority,
        &query_str,
        limit + 1,
        &[note_id.clone()],
    )?;

    // Convert RetrievalResult → SearchResult, filtering out the source note
    let search_results: Vec<SearchResult> = results
        .into_iter()
        .filter(|r| r.note.id != note_id)
        .take(limit)
        .map(|r| SearchResult {
            note: r.note,
            score: r.score,
            snippet: if r.snippet.is_empty() {
                None
            } else {
                Some(r.snippet)
            },
        })
        .collect();

    Ok(search_results)
}

/// Reindex all notes
#[tauri::command]
pub async fn reindex(state: State<'_, AppState>) -> Result<(), String> {
    // Get all full notes
    let notes = {
        let store = state.knowledge_store.read().await;
        let metas = store.list_notes().map_err(|e| e.to_string())?;
        let mut notes = Vec::new();
        for meta in metas {
            if let Ok(note) = store.get_note(&meta.id) {
                notes.push(note);
            }
        }
        notes
    };

    // Reindex search
    {
        let mut search = state.search_service.write().await;
        search.reindex_all(&notes).map_err(|e| e.to_string())?;
    }

    // Rebuild graph
    {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(&notes);
    }

    // Rebuild chunk index
    {
        let mut chunks = state.chunk_index.write().await;
        if let Err(e) = chunks.reindex_all(&notes) {
            log::error!("Failed to rebuild chunk index: {}", e);
        }
    }

    Ok(())
}
