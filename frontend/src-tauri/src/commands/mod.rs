pub mod boot;
pub mod canvas;
pub mod distill;
pub mod feedback;
pub mod graph;
pub mod import;
pub mod mcp;
pub mod memory;
pub mod notes;
pub mod priority;
pub mod retrieval;
pub mod search;
pub mod settings;
pub mod twin;
pub mod zettelkasten;

use crate::models::note::Note;
use crate::AppState;

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

pub(crate) async fn sync_link_discovery_for_note(state: &AppState, note: &Note) {
    sync_link_discovery_for_notes(state, std::slice::from_ref(note)).await;
}

pub(crate) async fn sync_link_discovery_for_notes(state: &AppState, notes: &[Note]) {
    if notes.is_empty() {
        return;
    }

    let mut discovery = state.link_discovery.write().await;
    discovery.sync_notes(notes);
}

pub(crate) async fn rebuild_link_discovery(state: &AppState, notes: &[Note]) {
    let mut discovery = state.link_discovery.write().await;
    discovery.bootstrap(notes);
}

pub(crate) async fn remove_link_discovery_note(state: &AppState, note_id: &str) {
    let mut discovery = state.link_discovery.write().await;
    discovery.remove_note(note_id);
}
