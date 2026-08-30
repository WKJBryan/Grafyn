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

#[tauri::command]
pub async fn list_user_records(state: State<'_, AppState>) -> Result<Vec<UserRecord>, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store.list_user_records().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_user_record(id: String, state: State<'_, AppState>) -> Result<UserRecord, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .get_user_record(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn create_user_record(
    record: UserRecordCreate,
    state: State<'_, AppState>,
) -> Result<UserRecord, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .create_user_record(record)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn update_user_record(
    id: String,
    update: UserRecordUpdate,
    state: State<'_, AppState>,
) -> Result<UserRecord, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .update_user_record(&id, update)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_session_trace(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<SessionTrace, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .get_session_trace(&session_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn run_twin_inference(
    state: State<'_, AppState>,
) -> Result<TwinInferenceRunSummary, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .run_twin_inference()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_twin_review(state: State<'_, AppState>) -> Result<Vec<TwinReviewRecord>, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store.get_twin_review().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resolve_user_record_evidence(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ResolvedEvidenceRef>, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .resolve_user_record_evidence(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_user_record_promotion(
    id: String,
    promotion_state: PromotionState,
    rationale: Option<String>,
    state: State<'_, AppState>,
) -> Result<UserRecord, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .set_user_record_promotion(&id, promotion_state, rationale)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn export_twin_data(
    request: TwinExportRequest,
    state: State<'_, AppState>,
) -> Result<crate::models::twin::ExportBundle, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .export_bundle(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_decision_episodes(
    state: State<'_, AppState>,
) -> Result<Vec<DecisionEpisodeWithReflections>, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let store = state.twin_store.read().await;
    store
        .list_decision_episodes_with_reflections()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn update_decision_outcome(
    id: String,
    update: DecisionOutcomeUpdate,
    state: State<'_, AppState>,
) -> Result<DecisionEpisode, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let selected_response_id = if let Some(selected) = update.selected_response.as_ref() {
        let session_id = {
            let store = state.twin_store.read().await;
            store
                .get_decision_episode(&id)
                .map_err(|error| error.to_string())?
                .session_id
        };
        let session = {
            let mut canvas = state.canvas_store.write().await;
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
    let mut store = state.twin_store.write().await;
    store
        .update_decision_outcome_with_response_id(&id, update, selected_response_id)
        .map_err(|error| error.to_string())
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
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let store = state.twin_store.read().await;
    store
        .get_decision_mirror_config()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn update_decision_mirror_config(
    update: DecisionMirrorConfigUpdate,
    state: State<'_, AppState>,
) -> Result<DecisionMirrorConfig, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    // Read-modify-write over the config file: must hold the write lock so two
    // concurrent updates can't both read the same on-disk state and have the
    // second writer silently clobber the first's change.
    let mut store = state.twin_store.write().await;
    store
        .update_decision_mirror_config(update)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn reset_decision_mirror_config(
    state: State<'_, AppState>,
) -> Result<DecisionMirrorConfig, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .reset_decision_mirror_config()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_memory_digest(
    state: State<'_, AppState>,
) -> Result<Vec<MemoryDigestItem>, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .list_memory_digest()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn review_memory_digest_item(
    id: String,
    request: MemoryDigestReviewRequest,
    state: State<'_, AppState>,
) -> Result<MemoryDigestItem, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .review_memory_digest_item(&id, request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_constitution_items(
    state: State<'_, AppState>,
) -> Result<Vec<ConstitutionItem>, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let store = state.twin_store.read().await;
    store
        .list_constitution_items()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn create_constitution_item(
    item: ConstitutionItemCreate,
    state: State<'_, AppState>,
) -> Result<ConstitutionItem, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    // Write lock: create_constitution_item does a read-modify-write file
    // sequence (see update_decision_mirror_config above for why read() is unsafe here).
    let mut store = state.twin_store.write().await;
    store
        .create_constitution_item(item)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn update_constitution_item(
    id: String,
    update: ConstitutionItemUpdate,
    state: State<'_, AppState>,
) -> Result<ConstitutionItem, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    // Write lock: update_constitution_item reads the current item file, applies the
    // update, then writes it back. Two concurrent updates to the same item under a
    // shared read lock could both read the pre-update file and the second writer
    // would silently clobber the first's change.
    let mut store = state.twin_store.write().await;
    store
        .update_constitution_item(&id, update)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn review_constitution_item(
    id: String,
    request: ConstitutionReviewRequest,
    state: State<'_, AppState>,
) -> Result<ConstitutionItem, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .review_constitution_item(&id, request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_action_gaps(state: State<'_, AppState>) -> Result<Vec<ActionGap>, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let store = state.twin_store.read().await;
    store.list_action_gaps().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn review_action_gap(
    id: String,
    request: ConstitutionReviewRequest,
    state: State<'_, AppState>,
) -> Result<ActionGap, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .review_action_gap(&id, request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_constitution_setup(
    state: State<'_, AppState>,
) -> Result<ConstitutionSetup, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let store = state.twin_store.read().await;
    store
        .get_constitution_setup()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn save_constitution_setup(
    setup: ConstitutionSetup,
    state: State<'_, AppState>,
) -> Result<ConstitutionSetup, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut store = state.twin_store.write().await;
    store
        .save_constitution_setup(setup)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn run_constitution_inference(
    state: State<'_, AppState>,
) -> Result<ConstitutionInferenceSummary, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let notes = {
        let store = state.knowledge_store.read().await;
        store.list_full_notes().map_err(|error| error.to_string())?
    };
    let mut store = state.twin_store.write().await;
    store
        .run_constitution_inference_with_notes(&notes)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn record_canvas_feedback(
    session_id: String,
    request: CanvasFeedbackRequest,
    state: State<'_, AppState>,
) -> Result<CanvasFeedbackResult, String> {
    let _root_epoch = crate::commands::acquire_root_epoch(state.inner()).await?;
    let session = {
        let mut canvas_store = state.canvas_store.write().await;
        canvas_store
            .get_session(&session_id)
            .map_err(|error| error.to_string())?
    };

    let mut twin_store = state.twin_store.write().await;
    twin_store
        .record_canvas_feedback(&session, request)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::resolve_persisted_response_id;
    use crate::models::canvas::{CanvasSession, ModelResponse, PromptTile};
    use crate::models::twin::{ConstitutionItemCreate, ConstitutionItemUpdate};
    use crate::services::twin::TwinStore;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::RwLock;

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
