use crate::models::note::{Note, NoteCreate, NoteMeta, NoteStatus, NoteUpdate};
use crate::models::twin::TraceEventType;
use crate::AppState;
use serde_json::json;
use tauri::State;

async fn finish_note_authority_error(
    state: &AppState,
    error: &anyhow::Error,
    operation: &str,
) -> Result<
    (
        crate::services::knowledge_store::KnowledgeAuthorityAdvancedOutcome,
        crate::services::vault_namespace::VaultAuthorityTokenV1,
    ),
    String,
> {
    let outcome = crate::services::knowledge_store::knowledge_authority_advanced_outcome(error)
        .ok_or_else(|| error.to_string())?;
    let repair =
        crate::commands::repair_after_authority_mutation(state, &outcome.commit, operation).await;
    if outcome.target_aborted {
        return Err(format!(
            "{operation} was not applied after vault authority changed; refresh state before deciding whether to retry"
        ));
    }
    match repair {
        crate::commands::PostAuthorityRepair::Ready(authority) => Ok((outcome, authority)),
        crate::commands::PostAuthorityRepair::NotRequired
        | crate::commands::PostAuthorityRepair::Unavailable(_) => Err(format!(
            "{operation} committed and recovery is pending; do not retry"
        )),
    }
}

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
    let mutation = {
        let mut store = state.knowledge_store.write().await;
        store.create_note_expecting_authority(note, "note_editor", root_epoch.authority().clone())
    };
    let (created_note, mut latest_commit, mut continuation_authority) = match mutation {
        Ok((note, commit)) => {
            let authority = commit.authority_token.clone();
            (note, commit, authority)
        }
        Err(error) => {
            let (outcome, authority) =
                finish_note_authority_error(state.inner(), &error, "note create").await?;
            let note_id = outcome
                .note_ids
                .first()
                .ok_or_else(|| "note create committed without its result identity".to_string())?;
            let note = state
                .knowledge_store
                .read()
                .await
                .get_note(note_id)
                .map_err(|error| {
                    log::error!("Recovered note create '{note_id}' could not be loaded: {error}");
                    "note create committed and recovered; do not retry".to_string()
                })?;
            (note, outcome.commit, Some(authority))
        }
    };
    let created_id = created_note.id.clone();
    let created_note_snapshot = created_note.clone();

    if let Some(commit) = append_note_trace(
        state.inner(),
        TraceEventType::NoteCreated,
        &created_note_snapshot,
        continuation_authority.as_ref(),
    )
    .await
    {
        latest_commit = commit;
        continuation_authority = latest_commit.authority_token.clone();
    }
    if created_note_snapshot.status == NoteStatus::Canonical {
        if let Some(commit) = append_note_trace(
            state.inner(),
            TraceEventType::NoteCanonicalPromoted,
            &created_note_snapshot,
            continuation_authority.as_ref(),
        )
        .await
        {
            latest_commit = commit;
        }
    }
    crate::commands::acknowledge_reported_repair(
        crate::commands::repair_after_authority_mutation(
            state.inner(),
            &latest_commit,
            "note create",
        )
        .await,
    );
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
    let (previous_status, mutation) = {
        let mut store = state.knowledge_store.write().await;
        let previous_status = store.get_note(&id).ok().map(|note| note.status);
        let mutation = store.update_note_expecting_authority(
            &id,
            update,
            "note_editor",
            root_epoch.authority().clone(),
        );
        (previous_status, mutation)
    };
    let (updated_note, mut latest_commit, mut continuation_authority) = match mutation {
        Ok((note, commit)) => {
            let authority = commit.authority_token.clone();
            (note, commit, authority)
        }
        Err(error) => {
            let (outcome, authority) =
                finish_note_authority_error(state.inner(), &error, "note update").await?;
            let note_id = outcome
                .note_ids
                .first()
                .ok_or_else(|| "note update committed without its result identity".to_string())?;
            let note = state
                .knowledge_store
                .read()
                .await
                .get_note(note_id)
                .map_err(|error| {
                    log::error!("Recovered note update '{note_id}' could not be loaded: {error}");
                    "note update committed and recovered; do not retry".to_string()
                })?;
            (note, outcome.commit, Some(authority))
        }
    };
    let updated_id = updated_note.id.clone();
    let updated_note_snapshot = updated_note.clone();

    if let Some(commit) = append_note_trace(
        state.inner(),
        TraceEventType::NoteUpdated,
        &updated_note_snapshot,
        continuation_authority.as_ref(),
    )
    .await
    {
        latest_commit = commit;
        continuation_authority = latest_commit.authority_token.clone();
    }
    if previous_status != Some(NoteStatus::Canonical)
        && updated_note_snapshot.status == NoteStatus::Canonical
    {
        if let Some(commit) = append_note_trace(
            state.inner(),
            TraceEventType::NoteCanonicalPromoted,
            &updated_note_snapshot,
            continuation_authority.as_ref(),
        )
        .await
        {
            latest_commit = commit;
        }
    }
    crate::commands::acknowledge_reported_repair(
        crate::commands::repair_after_authority_mutation(
            state.inner(),
            &latest_commit,
            "note update",
        )
        .await,
    );
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
    let mutation = {
        let mut store = state.knowledge_store.write().await;
        store.delete_note_expecting_authority(&id, "note_editor", root_epoch.authority().clone())
    };
    let commit = match mutation {
        Ok(commit) => commit,
        Err(error) => {
            finish_note_authority_error(state.inner(), &error, "note delete")
                .await?
                .0
                .commit
        }
    };
    crate::commands::acknowledge_reported_repair(
        crate::commands::repair_after_authority_mutation(state.inner(), &commit, "note delete")
            .await,
    );
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
    finish_note_trace_result(&note.id, result)
}

fn finish_note_trace_result(
    note_id: &str,
    result: anyhow::Result<(
        crate::models::twin::TraceEvent,
        crate::services::twin_events::MutationCommit,
    )>,
) -> Option<crate::services::twin_events::MutationCommit> {
    match result {
        Ok((_trace, commit)) => Some(commit),
        Err(error) => {
            if let Some(commit) = error
                .downcast_ref::<crate::services::twin_events::MutationError>()
                .and_then(|error| error.authority_advanced_commit())
            {
                log::warn!(
                    "Note twin trace for '{note_id}' advanced authority and requires central repair"
                );
                return Some(commit);
            }
            log::error!(
                "Failed to append note twin trace for '{}': {}",
                note_id,
                error
            );
            None
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_advanced_note_trace_preserves_its_exact_repair_commit() {
        let (state, _vault, _data) = crate::commands::commit_note_write_tests::build_test_state();
        let authority = state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .current_authority_token()
            .unwrap();
        let mutation_id = crate::models::twin_event::ContentDigest::parse("a".repeat(64)).unwrap();
        let error = anyhow::Error::new(
            crate::services::twin_events::MutationError::AuthorityAdvanced {
                mutation_id: mutation_id.clone(),
                authority_token: authority.clone(),
                target_aborted: true,
                reason: "injected post-authority trace abort".into(),
            },
        );

        let commit = finish_note_trace_result("note-1", Err(error))
            .expect("the exact trace commit must remain available for central repair");
        assert_eq!(commit.mutation_id, Some(mutation_id));
        assert_eq!(commit.authority_token, Some(authority));
    }

    #[tokio::test]
    async fn authority_advanced_note_create_recovers_the_exact_result_without_retry() {
        let (mut state, vault, _data) =
            crate::commands::commit_note_write_tests::build_test_state();
        let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
        state.knowledge_store = std::sync::Arc::new(tokio::sync::RwLock::new(
            crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
                vault.path().to_path_buf(),
                coordinator.current_namespace_path().unwrap(),
                coordinator.clone(),
            ),
        ));
        coordinator.fail_next_replays_before_targets(2);
        let error = {
            let mut store = state.knowledge_store.write().await;
            store
                .create_note_expecting_authority(
                    NoteCreate {
                        title: "Recovered note command".into(),
                        content: "exactly once".into(),
                        relative_path: Some("recovered-note-command.md".into()),
                        aliases: Vec::new(),
                        status: NoteStatus::Draft,
                        tags: vec!["rust".into()],
                        schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                        migration_source: None,
                        optimizer_managed: false,
                        properties: Default::default(),
                    },
                    "note_editor",
                    coordinator.current_authority_token().unwrap(),
                )
                .expect_err("the command boundary must receive authority recovery")
        };

        let (outcome, continuation_authority) =
            finish_note_authority_error(&state, &error, "note create")
                .await
                .expect("the exact committed note should recover without retry");
        assert_eq!(outcome.note_ids, vec!["recovered-note-command"]);
        assert!(
            continuation_authority.authority_generation
                > outcome
                    .commit
                    .authority_token
                    .as_ref()
                    .unwrap()
                    .authority_generation,
            "hub normalization must return the newer authority used by the next trace"
        );
        assert!(!state
            .knowledge_store
            .read()
            .await
            .get_note("recovered-note-command")
            .unwrap()
            .topic_hub_ids()
            .is_empty());
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert!(vault.path().join("recovered-note-command.md").exists());
    }

    #[tokio::test]
    async fn authority_advanced_note_update_and_delete_recover_without_retry() {
        let (mut state, vault, _data) =
            crate::commands::commit_note_write_tests::build_test_state();
        let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
        state.knowledge_store = std::sync::Arc::new(tokio::sync::RwLock::new(
            crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
                vault.path().to_path_buf(),
                coordinator.current_namespace_path().unwrap(),
                coordinator.clone(),
            ),
        ));
        let note_id = {
            let mut store = state.knowledge_store.write().await;
            store
                .create_note(NoteCreate {
                    title: "Update then delete".into(),
                    content: "before".into(),
                    relative_path: Some("update-delete-command.md".into()),
                    aliases: Vec::new(),
                    status: NoteStatus::Draft,
                    tags: Vec::new(),
                    schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                    migration_source: None,
                    optimizer_managed: false,
                    properties: Default::default(),
                })
                .unwrap()
                .id
        };

        coordinator.fail_next_replays_before_targets(2);
        let update_error = {
            let mut store = state.knowledge_store.write().await;
            store
                .update_note_expecting_authority(
                    &note_id,
                    crate::models::note::NoteUpdate {
                        title: Some("Recovered update".into()),
                        content: None,
                        relative_path: None,
                        aliases: None,
                        status: None,
                        tags: None,
                        schema_version: None,
                        migration_source: None,
                        optimizer_managed: None,
                        properties: None,
                    },
                    "note_editor",
                    coordinator.current_authority_token().unwrap(),
                )
                .expect_err("the update boundary must receive authority recovery")
        };
        let (update, _) = finish_note_authority_error(&state, &update_error, "note update")
            .await
            .expect("the update must recover without retry");
        assert_eq!(update.note_ids, vec![note_id.clone()]);
        assert_eq!(
            state
                .knowledge_store
                .read()
                .await
                .get_note(&note_id)
                .unwrap()
                .title,
            "Recovered update"
        );

        let (mut state, vault, _data) =
            crate::commands::commit_note_write_tests::build_test_state();
        let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
        state.knowledge_store = std::sync::Arc::new(tokio::sync::RwLock::new(
            crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
                vault.path().to_path_buf(),
                coordinator.current_namespace_path().unwrap(),
                coordinator.clone(),
            ),
        ));
        let note_id = {
            let mut store = state.knowledge_store.write().await;
            store
                .create_note(NoteCreate {
                    title: "Delete after recovery".into(),
                    content: "remove me".into(),
                    relative_path: Some("delete-command.md".into()),
                    aliases: Vec::new(),
                    status: NoteStatus::Draft,
                    tags: Vec::new(),
                    schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                    migration_source: None,
                    optimizer_managed: false,
                    properties: Default::default(),
                })
                .unwrap()
                .id
        };
        coordinator.fail_next_replays_before_targets(2);
        let delete_error = {
            let mut store = state.knowledge_store.write().await;
            store
                .delete_note_expecting_authority(
                    &note_id,
                    "note_editor",
                    coordinator.current_authority_token().unwrap(),
                )
                .expect_err("the delete boundary must receive authority recovery")
        };
        let (delete, _) = finish_note_authority_error(&state, &delete_error, "note delete")
            .await
            .expect("the delete must recover without retry");
        assert_eq!(delete.note_ids, vec![note_id.clone()]);
        assert!(state
            .knowledge_store
            .read()
            .await
            .get_note(&note_id)
            .is_err());
        assert_eq!(coordinator.pending_count().unwrap(), 0);
    }
}
