use crate::models::note::{Note, NoteCreate, NoteMeta, NoteStatus, NoteUpdate};
use crate::models::twin::TraceEventType;
use crate::AppState;
use serde_json::json;
use tauri::State;

/// List all notes (metadata only)
#[tauri::command]
pub async fn list_notes(state: State<'_, AppState>) -> Result<Vec<NoteMeta>, String> {
    let root_ticket = crate::commands::acquire_root_epoch(state.inner()).await?;
    let result = {
        let mut store = state.knowledge_store.write().await;
        store.reload_authoritative_state();
        store.list_notes().map_err(|e| e.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Get a single note by ID
#[tauri::command]
pub async fn get_note(id: String, state: State<'_, AppState>) -> Result<Note, String> {
    let root_ticket = crate::commands::acquire_root_epoch(state.inner()).await?;
    let result = {
        let mut store = state.knowledge_store.write().await;
        store.reload_authoritative_state();
        store.get_note(&id).map_err(|e| e.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Create a new note
#[tauri::command]
pub async fn create_note(note: NoteCreate, state: State<'_, AppState>) -> Result<Note, String> {
    let root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.knowledge_store.write().await;
    let (created_note, mut latest_commit) = store
        .create_note_expecting_authority(
            note,
            "note_editor",
            root_epoch.authority().clone(),
        )
        .map_err(|e| e.to_string())?;
    let created_id = created_note.id.clone();
    let created_note_snapshot = created_note.clone();
    drop(store);

    if let Some(commit) = append_note_trace(
        state.inner(),
        TraceEventType::NoteCreated,
        &created_note_snapshot,
        latest_commit.authority_token.as_ref(),
    )
    .await
    {
        latest_commit = commit;
    }
    if created_note_snapshot.status == NoteStatus::Canonical {
        if let Some(commit) = append_note_trace(
            state.inner(),
            TraceEventType::NoteCanonicalPromoted,
            &created_note_snapshot,
            latest_commit.authority_token.as_ref(),
        )
        .await
        {
            latest_commit = commit;
        }
    }
    crate::commands::repair_after_authority_mutation(
        state.inner(),
        &latest_commit,
        "note create",
    )
    .await;
    if let Err(error) =
        crate::commands::enqueue_vault_optimizer_note(state.inner(), &created_id, "note_created")
            .await
    {
        log::warn!("Note was committed but optimizer enqueue failed: {error}");
    }

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
    let root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.knowledge_store.write().await;
    let previous_status = store.get_note(&id).ok().map(|note| note.status);
    let (updated_note, mut latest_commit) = store
        .update_note_expecting_authority(
            &id,
            update,
            "note_editor",
            root_epoch.authority().clone(),
        )
        .map_err(|e| e.to_string())?;
    let updated_id = updated_note.id.clone();
    let updated_note_snapshot = updated_note.clone();
    drop(store);

    if let Some(commit) = append_note_trace(
        state.inner(),
        TraceEventType::NoteUpdated,
        &updated_note_snapshot,
        latest_commit.authority_token.as_ref(),
    )
    .await
    {
        latest_commit = commit;
    }
    if previous_status != Some(NoteStatus::Canonical)
        && updated_note_snapshot.status == NoteStatus::Canonical
    {
        if let Some(commit) = append_note_trace(
            state.inner(),
            TraceEventType::NoteCanonicalPromoted,
            &updated_note_snapshot,
            latest_commit.authority_token.as_ref(),
        )
        .await
        {
            latest_commit = commit;
        }
    }
    crate::commands::repair_after_authority_mutation(
        state.inner(),
        &latest_commit,
        "note update",
    )
    .await;
    if let Err(error) =
        crate::commands::enqueue_vault_optimizer_note(state.inner(), &updated_id, "note_updated")
            .await
    {
        log::warn!("Note was committed but optimizer enqueue failed: {error}");
    }

    let store = state.knowledge_store.read().await;
    store.get_note(&updated_id).map_err(|e| e.to_string())
}

/// Delete a note
#[tauri::command]
pub async fn delete_note(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.knowledge_store.write().await;
    let commit = store
        .delete_note_expecting_authority(
            &id,
            "note_editor",
            root_epoch.authority().clone(),
        )
        .map_err(|e| e.to_string())?;
    drop(store);
    crate::commands::repair_after_authority_mutation(state.inner(), &commit, "note delete").await;
    if let Err(error) =
        crate::commands::enqueue_vault_optimizer_note(state.inner(), &id, "note_deleted").await
    {
        log::warn!("Note was deleted but optimizer enqueue failed: {error}");
    }
    Ok(())
}

async fn append_note_trace(
    state: &AppState,
    event_type: TraceEventType,
    note: &Note,
    expected: Option<&crate::services::vault_namespace::VaultAuthorityTokenV1>,
) -> Option<crate::services::twin_events::MutationCommit> {
    let payload = json!({
        "note_id": note.id.clone(),
        "title": note.title.clone(),
        "status": note.status.clone(),
        "tags": note.tags.clone(),
        "properties": note.properties.clone(),
    });
    let mut twin_store = state.twin_store.write().await;
    let result = match expected {
        Some(expected) => twin_store.append_trace_event_expecting_authority(
            &format!("note-{}", note.id),
            event_type,
            payload,
            expected.clone(),
        ),
        None => return None,
    };
    match result {
        Ok((_trace, commit)) => Some(commit),
        Err(error) => {
            log::error!(
                "Failed to append note twin trace for '{}': {}",
                note.id,
                error
            );
            None
        }
    }
}
