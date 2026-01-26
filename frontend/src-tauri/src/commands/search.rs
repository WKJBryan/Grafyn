use crate::models::note::SearchResult;
use crate::AppState;
use tauri::State;

/// Search notes by query string
#[tauri::command]
pub async fn search_notes(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let search = state.search_service.read().await;
    let limit = limit.unwrap_or(20);
    search.search(&query, limit).map_err(|e| e.to_string())
}

/// Find notes similar to a given note
#[tauri::command]
pub async fn find_similar(
    note_id: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let limit = limit.unwrap_or(10);

    // Get the note content
    let content = {
        let store = state.knowledge_store.read().await;
        store
            .get_note(&note_id)
            .map(|n| n.content)
            .map_err(|e| e.to_string())?
    };

    // Find similar notes
    let search = state.search_service.read().await;
    search
        .find_similar(&note_id, &content, limit)
        .map_err(|e| e.to_string())
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

    Ok(())
}
