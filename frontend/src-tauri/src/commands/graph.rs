use crate::models::note::{GraphNeighbor, NoteMeta};
use crate::services::graph_index::{FullGraph, GraphStats};
use crate::AppState;
use tauri::State;

/// Get all notes that link to the given note (backlinks)
#[tauri::command]
pub async fn get_backlinks(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<NoteMeta>, String> {
    let ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = state.graph_index.read().await.get_backlinks(&note_id);
    ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Get all notes that the given note links to (outgoing links)
#[tauri::command]
pub async fn get_outgoing(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<NoteMeta>, String> {
    let ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = state.graph_index.read().await.get_outgoing(&note_id);
    ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Get all neighbors (both backlinks and outgoing) for graph visualization
#[tauri::command]
pub async fn get_neighbors(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<GraphNeighbor>, String> {
    let ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = state.graph_index.read().await.get_neighbors(&note_id);
    ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Get notes with no incoming or outgoing links
#[tauri::command]
pub async fn get_unlinked(state: State<'_, AppState>) -> Result<Vec<NoteMeta>, String> {
    let ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = state.graph_index.read().await.get_unlinked();
    ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Get the full graph structure (nodes + links) for visualization
#[tauri::command]
pub async fn get_full_graph(state: State<'_, AppState>) -> Result<FullGraph, String> {
    let ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = state.graph_index.read().await.get_full_graph();
    ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Rebuild the graph index from all notes
#[tauri::command]
pub async fn rebuild_graph(state: State<'_, AppState>) -> Result<GraphStats, String> {
    let ticket = crate::commands::acquire_root_epoch(state.inner()).await?;
    let notes = crate::commands::sync_topic_hubs(state.inner()).await?;

    // Rebuild graph
    let stats = {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(&notes);
        graph.stats()
    };
    ticket.finish(state.inner()).await?;
    Ok(stats)
}
