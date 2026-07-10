use crate::commands::{commit_note_delete, commit_note_write};
use crate::models::note::{Note, NoteCreate, NoteMeta, NoteStatus, NoteUpdate};
use crate::models::twin::TraceEventType;
use crate::AppState;
use serde_json::json;
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
    let created_note_snapshot = created_note.clone();
    drop(store);

    append_note_trace(
        state.inner(),
        TraceEventType::NoteCreated,
        &created_note_snapshot,
    )
    .await;
    if created_note_snapshot.status == NoteStatus::Canonical {
        append_note_trace(
            state.inner(),
            TraceEventType::NoteCanonicalPromoted,
            &created_note_snapshot,
        )
        .await;
    }
    commit_note_write(state.inner(), &created_id, "note_created").await?;

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
    let previous_status = store.get_note(&id).ok().map(|note| note.status);
    let updated_note = store.update_note(&id, update).map_err(|e| e.to_string())?;
    let updated_id = updated_note.id.clone();
    let updated_note_snapshot = updated_note.clone();
    drop(store);

    append_note_trace(
        state.inner(),
        TraceEventType::NoteUpdated,
        &updated_note_snapshot,
    )
    .await;
    if previous_status != Some(NoteStatus::Canonical)
        && updated_note_snapshot.status == NoteStatus::Canonical
    {
        append_note_trace(
            state.inner(),
            TraceEventType::NoteCanonicalPromoted,
            &updated_note_snapshot,
        )
        .await;
    }
    commit_note_write(state.inner(), &updated_id, "note_updated").await?;

    let store = state.knowledge_store.read().await;
    store.get_note(&updated_id).map_err(|e| e.to_string())
}

/// Delete a note
#[tauri::command]
pub async fn delete_note(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.knowledge_store.write().await;
    store.delete_note(&id).map_err(|e| e.to_string())?;
    drop(store);

    commit_note_delete(state.inner(), &id, "note_deleted").await
}

async fn append_note_trace(state: &AppState, event_type: TraceEventType, note: &Note) {
    let payload = json!({
        "note_id": note.id.clone(),
        "title": note.title.clone(),
        "status": note.status.clone(),
        "tags": note.tags.clone(),
        "properties": note.properties.clone(),
    });
    let mut twin_store = state.twin_store.write().await;
    if let Err(error) =
        twin_store.append_trace_event(&format!("note-{}", note.id), event_type, payload)
    {
        log::error!(
            "Failed to append note twin trace for '{}': {}",
            note.id,
            error
        );
    }
}
