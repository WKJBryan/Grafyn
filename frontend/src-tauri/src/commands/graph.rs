use crate::models::note::{GraphNeighbor, NoteMeta};
use crate::services::graph_index::{FullGraph, GraphStats};
use crate::services::perspective_history::{NewPerspectiveState, PerspectiveState};
use crate::AppState;
use tauri::State;

/// Get all notes that link to the given note (backlinks)
#[tauri::command]
pub async fn get_backlinks(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<NoteMeta>, String> {
    let graph = state.graph_index.read().await;
    Ok(graph.get_backlinks(&note_id))
}

/// Get all notes that the given note links to (outgoing links)
#[tauri::command]
pub async fn get_outgoing(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<NoteMeta>, String> {
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

/// Get the full graph structure (nodes + links) for visualization
#[tauri::command]
pub async fn get_full_graph(state: State<'_, AppState>) -> Result<FullGraph, String> {
    let graph = state.graph_index.read().await;
    Ok(graph.get_full_graph())
}

/// Perspectives are explicit, sourced statements; ordinary note edits do not create states.
#[tauri::command]
pub async fn list_perspective_states(state: State<'_, AppState>) -> Result<Vec<PerspectiveState>, String> {
    state.perspective_history.read().await.list().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn record_perspective_state(
    input: NewPerspectiveState,
    state: State<'_, AppState>,
) -> Result<PerspectiveState, String> {
    // Validate evidence before writing the immutable state. Keep this lock order:
    // knowledge_store, then perspective_history.
    let store = state.knowledge_store.read().await;
    for id in &input.source_note_ids {
        store.get_note(id).map_err(|_| format!("Source note does not exist: {id}"))?;
    }
    let history = state.perspective_history.write().await;
    history.append(input).map_err(|error| error.to_string())
}

/// Rebuild the graph index from all notes
#[tauri::command]
pub async fn rebuild_graph(state: State<'_, AppState>) -> Result<GraphStats, String> {
    let notes = crate::commands::sync_topic_hubs(state.inner()).await?;

    // Rebuild graph
    let mut graph = state.graph_index.write().await;
    graph.build_from_notes(&notes);

    Ok(graph.stats())
}
