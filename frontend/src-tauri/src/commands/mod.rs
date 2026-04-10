pub mod boot;
pub mod canvas;
pub mod distill;
pub mod feedback;
pub mod graph;
pub mod import;
pub mod mcp;
pub mod memory;
pub mod migration;
pub mod notes;
pub mod priority;
pub mod retrieval;
pub mod search;
pub mod settings;
pub mod twin;
pub mod zettelkasten;

use crate::models::note::Note;
use crate::AppState;
use std::collections::HashSet;

pub(crate) async fn sync_chunk_index_for_note(state: &AppState, note: &Note) {
    sync_chunk_index_for_notes(state, std::slice::from_ref(note)).await;
}

pub(crate) async fn sync_chunk_index_for_notes(state: &AppState, notes: &[Note]) {
    if notes.is_empty() {
        return;
    }

    let mut chunks = state.chunk_index.write().await;
    for note in notes {
        if let Err(error) = chunks.index_note_chunks(note) {
            log::error!("Failed to index chunks for note '{}': {}", note.id, error);
        }
    }
    if let Err(error) = chunks.commit() {
        log::error!("Failed to commit chunk index: {}", error);
    }
}

pub(crate) async fn remove_note_chunks_from_index(state: &AppState, note_id: &str) {
    let mut chunks = state.chunk_index.write().await;
    if let Err(error) = chunks.remove_note_chunks(note_id) {
        log::error!("Failed to remove chunks for note '{}': {}", note_id, error);
    }
    if let Err(error) = chunks.commit() {
        log::error!("Failed to commit chunk index: {}", error);
    }
}

pub(crate) async fn rebuild_link_discovery(state: &AppState, notes: &[Note]) {
    let mut discovery = state.link_discovery.write().await;
    discovery.bootstrap(notes);
}

pub(crate) async fn bootstrap_vault_optimizer(state: &AppState, notes: &[Note]) {
    let mut optimizer = state.vault_optimizer.write().await;
    optimizer.bootstrap(notes);
}

pub(crate) async fn enqueue_vault_optimizer_note(state: &AppState, note_id: &str, reason: &str) {
    let mut optimizer = state.vault_optimizer.write().await;
    optimizer.enqueue_note(note_id, reason);
}

pub(crate) async fn remove_link_discovery_note(state: &AppState, note_id: &str) {
    let mut discovery = state.link_discovery.write().await;
    discovery.remove_note(note_id);
}

pub(crate) async fn sync_topic_hubs(state: &AppState) -> Result<Vec<Note>, String> {
    let sync_result = {
        let mut store = state.knowledge_store.write().await;
        crate::services::topic_hub::sync_topic_hubs(&mut store)
            .map_err(|error| error.to_string())?
    };

    let changed_ids = sync_result
        .changed_note_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let changed_notes = sync_result
        .all_notes
        .iter()
        .filter(|note| changed_ids.contains(&note.id))
        .cloned()
        .collect::<Vec<_>>();

    for removed_note_id in &sync_result.removed_note_ids {
        remove_note_chunks_from_index(state, removed_note_id).await;
        remove_link_discovery_note(state, removed_note_id).await;

        let mut search = state.search_service.write().await;
        if let Err(error) = search.remove_note(removed_note_id) {
            log::error!(
                "Failed to remove topic-hub note '{}' from search index: {}",
                removed_note_id,
                error
            );
        }
        if let Err(error) = search.commit() {
            log::error!(
                "Failed to commit search index after topic-hub removal: {}",
                error
            );
        }
    }

    if !changed_notes.is_empty() {
        {
            let mut search = state.search_service.write().await;
            for note in &changed_notes {
                if let Err(error) = search.index_note(note) {
                    log::error!("Failed to index topic-hub note '{}': {}", note.id, error);
                }
            }
            if let Err(error) = search.commit() {
                log::error!("Failed to commit search index after topic sync: {}", error);
            }
        }

        sync_chunk_index_for_notes(state, &changed_notes).await;
    }

    {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(&sync_result.all_notes);
    }

    rebuild_link_discovery(state, &sync_result.all_notes).await;
    bootstrap_vault_optimizer(state, &sync_result.all_notes).await;

    Ok(sync_result.all_notes)
}

pub(crate) async fn rebuild_all_indexes(state: &AppState) -> Result<Vec<Note>, String> {
    let notes = sync_topic_hubs(state).await?;

    {
        let mut search = state.search_service.write().await;
        search.reindex_all(&notes).map_err(|error| error.to_string())?;
    }

    {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(&notes);
    }

    {
        let mut chunks = state.chunk_index.write().await;
        if let Err(error) = chunks.reindex_all(&notes) {
            log::error!("Failed to rebuild chunk index: {}", error);
        }
    }

    bootstrap_vault_optimizer(state, &notes).await;
    Ok(notes)
}
