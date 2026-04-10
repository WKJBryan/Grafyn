use crate::commands::{
    enqueue_vault_optimizer_note, remove_link_discovery_note, remove_note_chunks_from_index,
    sync_topic_hubs,
};
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
    let created_id = created_note.id.clone();
    drop(store);

    sync_topic_hubs(state.inner()).await?;
    enqueue_vault_optimizer_note(state.inner(), &created_id, "note_created").await;

    let store = state.knowledge_store.read().await;
    store.get_note(&created_id).map_err(|e| e.to_string())
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
    let updated_id = updated_note.id.clone();
    drop(store);

    sync_topic_hubs(state.inner()).await?;
    enqueue_vault_optimizer_note(state.inner(), &updated_id, "note_updated").await;

    let store = state.knowledge_store.read().await;
    store.get_note(&updated_id).map_err(|e| e.to_string())
}

/// Delete a note
#[tauri::command]
pub async fn delete_note(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.knowledge_store.write().await;
    store.delete_note(&id).map_err(|e| e.to_string())?;
    drop(store);

    {
        let mut search = state.search_service.write().await;
        if let Err(e) = search.remove_note(&id) {
            log::error!("Failed to remove note '{}' from search index: {}", id, e);
        }
        if let Err(e) = search.commit() {
            log::error!(
                "Failed to commit search index after deleting note '{}': {}",
                id,
                e
            );
        }
    }

    remove_note_chunks_from_index(state.inner(), &id).await;
    remove_link_discovery_note(state.inner(), &id).await;
    sync_topic_hubs(state.inner()).await?;
    enqueue_vault_optimizer_note(state.inner(), &id, "note_deleted").await;

    Ok(())
}
