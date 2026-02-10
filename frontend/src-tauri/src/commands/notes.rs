use crate::models::note::{Note, NoteCreate, NoteMeta, NoteUpdate};
use crate::AppState;
use tauri::State;

/// List all notes (metadata only)
#[tauri::command]
pub async fn list_notes(state: State<'_, AppState>) -> Result<Vec<NoteMeta>, String> {
    let store = state.knowledge_store.read().await;
    store.list_notes().map_err(|e| e.to_string())
}

/// Get a single note by ID
#[tauri::command]
pub async fn get_note(id: String, state: State<'_, AppState>) -> Result<Note, String> {
    let store = state.knowledge_store.read().await;
    store.get_note(&id).map_err(|e| e.to_string())
}

/// Create a new note
#[tauri::command]
pub async fn create_note(note: NoteCreate, state: State<'_, AppState>) -> Result<Note, String> {
    let mut store = state.knowledge_store.write().await;
    let created_note = store.create_note(note).map_err(|e| e.to_string())?;

    // Update search index
    {
        let mut search = state.search_service.write().await;
        if let Err(e) = search.index_note(&created_note) {
            log::error!("Failed to index note '{}': {}", created_note.id, e);
        }
        if let Err(e) = search.commit() {
            log::error!("Failed to commit search index after creating note '{}': {}", created_note.id, e);
        }
    }

    // Update graph index
    {
        let mut graph = state.graph_index.write().await;
        graph.update_note(&created_note);
    }

    Ok(created_note)
}

/// Update an existing note
#[tauri::command]
pub async fn update_note(
    id: String,
    update: NoteUpdate,
    state: State<'_, AppState>,
) -> Result<Note, String> {
    let mut store = state.knowledge_store.write().await;
    let updated_note = store.update_note(&id, update).map_err(|e| e.to_string())?;

    // Update search index
    {
        let mut search = state.search_service.write().await;
        if let Err(e) = search.index_note(&updated_note) {
            log::error!("Failed to index note '{}': {}", updated_note.id, e);
        }
        if let Err(e) = search.commit() {
            log::error!("Failed to commit search index after updating note '{}': {}", updated_note.id, e);
        }
    }

    // Update graph index
    {
        let mut graph = state.graph_index.write().await;
        graph.update_note(&updated_note);
    }

    Ok(updated_note)
}

/// Delete a note
#[tauri::command]
pub async fn delete_note(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.knowledge_store.write().await;
    store.delete_note(&id).map_err(|e| e.to_string())?;

    // Remove from search index
    {
        let mut search = state.search_service.write().await;
        if let Err(e) = search.remove_note(&id) {
            log::error!("Failed to remove note '{}' from search index: {}", id, e);
        }
        if let Err(e) = search.commit() {
            log::error!("Failed to commit search index after deleting note '{}': {}", id, e);
        }
    }

    // Remove from graph index
    {
        let mut graph = state.graph_index.write().await;
        graph.remove_note(&id);
    }

    Ok(())
}
