use crate::models::canvas::CanvasSession;
use crate::models::twin::{
    ActionGap, CanvasFeedbackRequest, CanvasFeedbackResult, CanvasResponseRef,
    ConstitutionInferenceSummary, ConstitutionItem, ConstitutionItemCreate, ConstitutionItemUpdate,
    ConstitutionReviewRequest, ConstitutionSetup, DecisionEpisode, DecisionEpisodeWithReflections,
    DecisionMirrorConfig, DecisionMirrorConfigUpdate, DecisionOutcomeUpdate, MemoryDigestItem,
    MemoryDigestReviewRequest, PromotionState, ResolvedEvidenceRef, SessionTrace,
    TwinExportRequest, TwinInferenceRunSummary, TwinReviewRecord, UserRecord, UserRecordCreate,
    UserRecordUpdate,
};
use crate::AppState;
use tauri::State;

async fn run_twin_mutation<T>(
    state: &AppState,
    operation_name: &str,
    operation: impl FnOnce(
        &mut crate::services::twin::TwinStore,
    ) -> anyhow::Result<(T, crate::services::twin_events::MutationCommit)>,
) -> Result<T, String> {
    let root_ticket = crate::commands::acquire_root_epoch(state).await?;
    let result = {
        let mut store = state.twin_store.write().await;
        operation(&mut store)
    };
    finish_twin_mutation(state, root_ticket, operation_name, result).await
}

async fn finish_twin_mutation<T>(
    state: &AppState,
    root_ticket: crate::commands::RootReadTicket,
    operation_name: &str,
    result: anyhow::Result<(T, crate::services::twin_events::MutationCommit)>,
) -> Result<T, String> {
    match result {
        Ok((value, commit)) if commit.authority_token.is_some() => {
            drop(root_ticket);
            match crate::commands::repair_after_authority_mutation(state, &commit, operation_name)
                .await
            {
                crate::commands::PostAuthorityRepair::NotRequired
                | crate::commands::PostAuthorityRepair::Ready(_)
                | crate::commands::PostAuthorityRepair::Unavailable(_) => {}
            }
            Ok(value)
        }
        Ok((value, _commit)) => {
            root_ticket.finish(state).await?;
            Ok(value)
        }
        Err(error) => {
            if let Some(commit) = error
                .downcast_ref::<crate::services::twin_events::MutationError>()
                .and_then(|error| error.authority_advanced_commit())
            {
                drop(root_ticket);
                match crate::commands::repair_after_authority_mutation(
                    state,
                    &commit,
                    operation_name,
                )
                .await
                {
                    crate::commands::PostAuthorityRepair::NotRequired
                    | crate::commands::PostAuthorityRepair::Ready(_)
                    | crate::commands::PostAuthorityRepair::Unavailable(_) => {}
                }
                return Err(format!(
                    "{operation_name} committed and is being recovered; do not retry"
                ));
            }
            root_ticket.finish(state).await?;
            Err(error.to_string())
        }
    }
}

#[tauri::command]
pub async fn list_user_records(state: State<'_, AppState>) -> Result<Vec<UserRecord>, String> {
    let root_ticket = crate::commands::acquire_root_epoch(state.inner()).await?;
    let result = {
        let mut store = state.twin_store.write().await;
        store
            .rebuild_mutation_caches()
            .map_err(|error| error.to_string())?;
        store.list_user_records().map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn get_user_record(id: String, state: State<'_, AppState>) -> Result<UserRecord, String> {
    let root_ticket = crate::commands::acquire_root_epoch(state.inner()).await?;
    let result = {
        let mut store = state.twin_store.write().await;
        store
            .rebuild_mutation_caches()
            .map_err(|error| error.to_string())?;
        store
            .get_user_record(&id)
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn create_user_record(
    record: UserRecordCreate,
    state: State<'_, AppState>,
) -> Result<UserRecord, String> {
    run_twin_mutation(state.inner(), "user record create", move |store| {
        store.create_user_record_with_commit(record)
    })
    .await
}

#[tauri::command]
pub async fn update_user_record(
    id: String,
    update: UserRecordUpdate,
    state: State<'_, AppState>,
) -> Result<UserRecord, String> {
    run_twin_mutation(state.inner(), "user record update", move |store| {
        store.update_user_record_with_commit(&id, update)
    })
    .await
}

#[tauri::command]
pub async fn get_session_trace(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<SessionTrace, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let mut store = state.twin_store.write().await;
        store
            .get_session_trace(&session_id)
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn run_twin_inference(
    state: State<'_, AppState>,
) -> Result<TwinInferenceRunSummary, String> {
    run_twin_mutation(state.inner(), "Twin inference", |store| {
        store.run_twin_inference_with_commit()
    })
    .await
}

#[tauri::command]
pub async fn get_twin_review(state: State<'_, AppState>) -> Result<Vec<TwinReviewRecord>, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let mut store = state.twin_store.write().await;
        store.get_twin_review().map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn resolve_user_record_evidence(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ResolvedEvidenceRef>, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let mut store = state.twin_store.write().await;
        store
            .resolve_user_record_evidence(&id)
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn set_user_record_promotion(
    id: String,
    promotion_state: PromotionState,
    rationale: Option<String>,
    state: State<'_, AppState>,
) -> Result<UserRecord, String> {
    run_twin_mutation(state.inner(), "user record review", move |store| {
        store.set_user_record_promotion_with_commit(&id, promotion_state, rationale)
    })
    .await
}

#[tauri::command]
pub async fn export_twin_data(
    request: TwinExportRequest,
    state: State<'_, AppState>,
) -> Result<crate::models::twin::ExportBundle, String> {
    run_twin_mutation(state.inner(), "Twin export", move |store| {
        store.export_bundle_with_commit(request)
    })
    .await
}

#[tauri::command]
pub async fn list_decision_episodes(
    state: State<'_, AppState>,
) -> Result<Vec<DecisionEpisodeWithReflections>, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let store = state.twin_store.read().await;
        store
            .list_decision_episodes_with_reflections()
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn update_decision_outcome(
    id: String,
    update: DecisionOutcomeUpdate,
    state: State<'_, AppState>,
) -> Result<DecisionEpisode, String> {
    let root_ticket = crate::commands::acquire_root_epoch(state.inner()).await?;
    let selected_response_id = if let Some(selected) = update.selected_response.as_ref() {
        let session_id = {
            let mut store = state.twin_store.write().await;
            store
                .rebuild_mutation_caches()
                .map_err(|error| error.to_string())?;
            store
                .get_decision_episode(&id)
                .map_err(|error| error.to_string())?
                .session_id
        };
        let session = {
            let mut canvas = state.canvas_store.write().await;
            canvas.reload_authoritative_state();
            canvas
                .get_session(&session_id)
                .map_err(|error| error.to_string())?
        };
        Some(
            resolve_persisted_response_id(&session, selected)
                .ok_or_else(|| "Selected Canvas response no longer exists".to_string())?,
        )
    } else {
        None
    };
    root_ticket.validate(state.inner()).await?;
    let result = {
        let mut store = state.twin_store.write().await;
        store.update_decision_outcome_with_response_id_and_commit(&id, update, selected_response_id)
    };
    finish_twin_mutation(
        state.inner(),
        root_ticket,
        "decision outcome update",
        result,
    )
    .await
}

fn resolve_persisted_response_id(
    session: &CanvasSession,
    selected: &CanvasResponseRef,
) -> Option<String> {
    session
        .prompt_tiles
        .iter()
        .find(|tile| tile.id == selected.tile_id)?
        .responses
        .values()
        .find(|response| response.model_id == selected.model_id)
        .map(|response| response.id.clone())
}

#[tauri::command]
pub async fn get_decision_mirror_config(
    state: State<'_, AppState>,
) -> Result<DecisionMirrorConfig, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let store = state.twin_store.read().await;
        store
            .get_decision_mirror_config()
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn update_decision_mirror_config(
    update: DecisionMirrorConfigUpdate,
    state: State<'_, AppState>,
) -> Result<DecisionMirrorConfig, String> {
    run_twin_mutation(
        state.inner(),
        "decision mirror config update",
        move |store| store.update_decision_mirror_config_with_commit(update),
    )
    .await
}

#[tauri::command]
pub async fn reset_decision_mirror_config(
    state: State<'_, AppState>,
) -> Result<DecisionMirrorConfig, String> {
    run_twin_mutation(state.inner(), "decision mirror config reset", |store| {
        store.reset_decision_mirror_config_with_commit()
    })
    .await
}

#[tauri::command]
pub async fn list_memory_digest(
    state: State<'_, AppState>,
) -> Result<Vec<MemoryDigestItem>, String> {
    run_twin_mutation(state.inner(), "memory digest refresh", |store| {
        store.list_memory_digest_with_commit()
    })
    .await
}

#[tauri::command]
pub async fn review_memory_digest_item(
    id: String,
    request: MemoryDigestReviewRequest,
    state: State<'_, AppState>,
) -> Result<MemoryDigestItem, String> {
    run_twin_mutation(state.inner(), "memory digest review", move |store| {
        store.review_memory_digest_item_with_commit(&id, request)
    })
    .await
}

#[tauri::command]
pub async fn list_constitution_items(
    state: State<'_, AppState>,
) -> Result<Vec<ConstitutionItem>, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let store = state.twin_store.read().await;
        store
            .list_constitution_items()
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn create_constitution_item(
    item: ConstitutionItemCreate,
    state: State<'_, AppState>,
) -> Result<ConstitutionItem, String> {
    run_twin_mutation(state.inner(), "constitution item create", move |store| {
        store.create_constitution_item_with_commit(item)
    })
    .await
}

#[tauri::command]
pub async fn update_constitution_item(
    id: String,
    update: ConstitutionItemUpdate,
    state: State<'_, AppState>,
) -> Result<ConstitutionItem, String> {
    run_twin_mutation(state.inner(), "constitution item update", move |store| {
        store.update_constitution_item_with_commit(&id, update)
    })
    .await
}

#[tauri::command]
pub async fn review_constitution_item(
    id: String,
    request: ConstitutionReviewRequest,
    state: State<'_, AppState>,
) -> Result<ConstitutionItem, String> {
    run_twin_mutation(state.inner(), "constitution item review", move |store| {
        store.review_constitution_item_with_commit(&id, request)
    })
    .await
}

#[tauri::command]
pub async fn list_action_gaps(state: State<'_, AppState>) -> Result<Vec<ActionGap>, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let store = state.twin_store.read().await;
        store.list_action_gaps().map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn review_action_gap(
    id: String,
    request: ConstitutionReviewRequest,
    state: State<'_, AppState>,
) -> Result<ActionGap, String> {
    run_twin_mutation(state.inner(), "action gap review", move |store| {
        store.review_action_gap_with_commit(&id, request)
    })
    .await
}

#[tauri::command]
pub async fn get_constitution_setup(
    state: State<'_, AppState>,
) -> Result<ConstitutionSetup, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let store = state.twin_store.read().await;
        store
            .get_constitution_setup()
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

#[tauri::command]
pub async fn save_constitution_setup(
    setup: ConstitutionSetup,
    state: State<'_, AppState>,
) -> Result<ConstitutionSetup, String> {
    run_twin_mutation(state.inner(), "constitution setup save", move |store| {
        store.save_constitution_setup_with_commit(setup)
    })
    .await
}

#[tauri::command]
pub async fn run_constitution_inference(
    state: State<'_, AppState>,
) -> Result<ConstitutionInferenceSummary, String> {
    let root_ticket = crate::commands::acquire_root_epoch(state.inner()).await?;
    let notes = {
        let store = state.knowledge_store.read().await;
        store.list_full_notes().map_err(|error| error.to_string())?
    };
    let result = {
        let mut store = state.twin_store.write().await;
        store.run_constitution_inference_with_notes_and_commit(&notes)
    };
    finish_twin_mutation(state.inner(), root_ticket, "constitution inference", result).await
}

#[tauri::command]
pub async fn record_canvas_feedback(
    session_id: String,
    request: CanvasFeedbackRequest,
    state: State<'_, AppState>,
) -> Result<CanvasFeedbackResult, String> {
    let root_ticket = crate::commands::acquire_root_epoch(state.inner()).await?;
    let session = {
        let mut canvas_store = state.canvas_store.write().await;
        canvas_store.reload_authoritative_state();
        canvas_store
            .get_session(&session_id)
            .map_err(|error| error.to_string())?
    };
    root_ticket.validate(state.inner()).await?;

    let result = {
        let mut twin_store = state.twin_store.write().await;
        twin_store.record_canvas_feedback_with_commit(&session, request)
    };
    finish_twin_mutation(state.inner(), root_ticket, "Canvas feedback", result).await
}

#[cfg(test)]
mod tests {
    use super::{resolve_persisted_response_id, run_twin_mutation};
    use crate::models::canvas::{CanvasSession, ModelResponse, PromptTile};
    use crate::models::twin::{ConstitutionItemCreate, ConstitutionItemUpdate, TwinExportRequest};
    use crate::services::twin::TwinStore;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::RwLock;

    #[tokio::test]
    async fn export_command_repairs_its_exact_commit_and_publishes_ready() {
        let (mut state, _vault_dir, data_dir) =
            crate::commands::commit_note_write_tests::build_test_state();
        let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
        let initial = coordinator.current_authority_token().unwrap();
        *state.loaded_authority.write().await = Some(initial.clone());
        let twin_target_root = data_dir.path().join("twin");
        let twin_root = twin_target_root.join("scope-one");
        state.twin_store = Arc::new(RwLock::new(TwinStore::with_event_recorder(
            twin_root,
            twin_target_root,
            coordinator.clone(),
        )));

        let bundle = run_twin_mutation(&state, "Twin export", |store| {
            store.export_bundle_with_commit(TwinExportRequest::default())
        })
        .await
        .expect("export and exact-token repair should succeed");

        let ready = coordinator.current_authority_token().unwrap();
        assert_eq!(ready.authority_generation, initial.authority_generation + 1);
        assert_eq!(state.loaded_authority.read().await.as_ref(), Some(&ready));
        coordinator.require_namespace_ready().unwrap();
        assert!(std::path::Path::new(&bundle.manifest_path).is_file());
    }

    #[test]
    fn selected_outcome_response_resolves_the_persisted_canvas_response_id() {
        let mut session = CanvasSession {
            id: "session-one".to_string(),
            ..CanvasSession::default()
        };
        let mut tile = PromptTile {
            id: "tile-one".to_string(),
            ..PromptTile::default()
        };
        tile.responses.insert(
            "model-a".to_string(),
            ModelResponse {
                id: "response-persisted".to_string(),
                model_id: "model-a".to_string(),
                ..ModelResponse::default()
            },
        );
        session.prompt_tiles.push(tile);
        assert_eq!(
            resolve_persisted_response_id(
                &session,
                &crate::models::twin::CanvasResponseRef {
                    tile_id: "tile-one".to_string(),
                    model_id: "model-a".to_string(),
                }
            )
            .as_deref(),
            Some("response-persisted")
        );
    }

    /// Regression test for the read-modify-write lock fix: `update_constitution_item`
    /// reads the current item file, applies the update's `Some` fields, and writes the
    /// whole item back. Two concurrent updates to the *same* item under a shared read
    /// lock (the pre-fix behavior) could both read the same pre-update file and the
    /// second writer would silently clobber the first's change — a classic lost
    /// update. With the write lock in place, `Arc<RwLock<TwinStore>>::write().await`
    /// fully serializes the two calls (tokio's RwLock allows only one writer at a
    /// time, with no `.await` point inside `update_constitution_item` for the second
    /// task to interleave into), so both edits — one to `claim`, the other to
    /// `priority` — must survive.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_constitution_updates_do_not_lose_a_write() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let store = Arc::new(RwLock::new(TwinStore::new(temp_dir.path().to_path_buf())));

        let item = {
            let mut guard = store.write().await;
            guard
                .create_constitution_item(ConstitutionItemCreate {
                    claim: "Original claim".to_string(),
                    dimension: "values".to_string(),
                    scope: vec!["general".to_string()],
                    priority: 0.5,
                    confidence: 0.5,
                    status: Default::default(),
                    evidence_refs: Vec::new(),
                    tensions: Vec::new(),
                    linked_record_ids: Vec::new(),
                    source: None,
                })
                .expect("constitution item should be created")
        };

        let store_a = store.clone();
        let item_id_a = item.id.clone();
        let claim_update = tokio::spawn(async move {
            // Mirrors the fixed `update_constitution_item` command: acquire the
            // write lock for the whole read-modify-write sequence.
            let mut guard = store_a.write().await;
            guard
                .update_constitution_item(
                    &item_id_a,
                    ConstitutionItemUpdate {
                        claim: Some("Updated claim".to_string()),
                        ..Default::default()
                    },
                )
                .expect("claim update should persist")
        });

        let store_b = store.clone();
        let item_id_b = item.id.clone();
        let priority_update = tokio::spawn(async move {
            let mut guard = store_b.write().await;
            guard
                .update_constitution_item(
                    &item_id_b,
                    ConstitutionItemUpdate {
                        priority: Some(0.9),
                        ..Default::default()
                    },
                )
                .expect("priority update should persist")
        });

        let (a, b) = tokio::join!(claim_update, priority_update);
        a.expect("claim-update task panicked");
        b.expect("priority-update task panicked");

        let final_item = {
            let guard = store.read().await;
            guard
                .list_constitution_items()
                .expect("constitution items should list")
                .into_iter()
                .find(|candidate| candidate.id == item.id)
                .expect("updated item should still exist")
        };

        assert_eq!(
            final_item.claim, "Updated claim",
            "claim update must not be lost to the concurrent priority update"
        );
        assert_eq!(
            final_item.priority, 0.9,
            "priority update must not be lost to the concurrent claim update"
        );
    }
}
