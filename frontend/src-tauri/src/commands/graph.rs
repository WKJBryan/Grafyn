use crate::models::note::{GraphNeighbor, NoteMeta};
use crate::services::graph_index::GraphStats;
use crate::AppState;
use tauri::State;

/// Get all notes that link to the given note (backlinks)
#[tauri::command]
pub async fn get_backlinks(note_id: String, state: State<'_, AppState>) -> Result<Vec<NoteMeta>, String> {
    let graph = state.graph_index.read().await;
    Ok(graph.get_backlinks(&note_id))
}

/// Get all notes that the given note links to (outgoing links)
#[tauri::command]
pub async fn get_outgoing(note_id: String, state: State<'_, AppState>) -> Result<Vec<NoteMeta>, String> {
    let graph = state.graph_index.read().await;
    Ok(graph.get_outgoing(&note_id))
}

/// Get all neighbors (both backlinks and outgoing) for graph visualization
#[tauri::command]
pub async fn get_neighbors(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<GraphNeighbor>, String> {
    let graph = state.graph_index.read().await;
    Ok(graph.get_neighbors(&note_id))
}

/// Get notes with no incoming or outgoing links
#[tauri::command]
pub async fn get_unlinked(state: State<'_, AppState>) -> Result<Vec<NoteMeta>, String> {
    let graph = state.graph_index.read().await;
    Ok(graph.get_unlinked())
}

/// Rebuild the graph index from all notes
#[tauri::command]
pub async fn rebuild_graph(state: State<'_, AppState>) -> Result<GraphStats, String> {
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

    // Rebuild graph
    let mut graph = state.graph_index.write().await;
    graph.build_from_notes(&notes);

    Ok(graph.stats())
}
