use crate::models::canvas::{CanvasSession, ModelResponse, PromptTile};
use crate::models::twin::{
    ActionGap, CanvasFeedbackRequest, CanvasFeedbackResult, CanvasFeedbackType, CanvasResponseRef,
    ConstitutionInferenceSummary, ConstitutionItem, ConstitutionItemCreate, ConstitutionItemUpdate,
    ConstitutionReviewRequest, ConstitutionSetup, DecisionEpisode, DecisionEpisodeWithReflections,
    DecisionMirrorConfig, DecisionMirrorConfigUpdate, DecisionOutcomeUpdate, EvidenceRef,
    MemoryDigestItem, MemoryDigestReviewRequest, PromotionState, RecordOrigin, ResolvedEvidenceRef,
    SessionTrace, TraceEvent, TraceEventType, TwinExportRequest, TwinInferenceRunSummary,
    TwinReviewRecord, UserRecord, UserRecordCreate, UserRecordKind, UserRecordUpdate,
};
use crate::AppState;
use serde_json::json;
use tauri::State;

#[tauri::command]
pub async fn list_user_records(state: State<'_, AppState>) -> Result<Vec<UserRecord>, String> {
    let mut store = state.twin_store.write().await;
    store.list_user_records().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_user_record(id: String, state: State<'_, AppState>) -> Result<UserRecord, String> {
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
    let mut store = state.twin_store.write().await;
    store
        .get_session_trace(&session_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn run_twin_inference(
    state: State<'_, AppState>,
) -> Result<TwinInferenceRunSummary, String> {
    let mut store = state.twin_store.write().await;
    store
        .run_twin_inference()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_twin_review(state: State<'_, AppState>) -> Result<Vec<TwinReviewRecord>, String> {
    let mut store = state.twin_store.write().await;
    store.get_twin_review().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resolve_user_record_evidence(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ResolvedEvidenceRef>, String> {
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
    let mut store = state.twin_store.write().await;
    store
        .export_bundle(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_decision_episodes(
    state: State<'_, AppState>,
) -> Result<Vec<DecisionEpisodeWithReflections>, String> {
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
    let mut store = state.twin_store.write().await;
    store
        .update_decision_outcome(&id, update)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_decision_mirror_config(
    state: State<'_, AppState>,
) -> Result<DecisionMirrorConfig, String> {
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
    let store = state.twin_store.read().await;
    store
        .update_decision_mirror_config(update)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn reset_decision_mirror_config(
    state: State<'_, AppState>,
) -> Result<DecisionMirrorConfig, String> {
    let store = state.twin_store.read().await;
    store
        .reset_decision_mirror_config()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_memory_digest(
    state: State<'_, AppState>,
) -> Result<Vec<MemoryDigestItem>, String> {
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
    let mut store = state.twin_store.write().await;
    store
        .review_memory_digest_item(&id, request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_constitution_items(
    state: State<'_, AppState>,
) -> Result<Vec<ConstitutionItem>, String> {
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
    let store = state.twin_store.read().await;
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
    let store = state.twin_store.read().await;
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
    let mut store = state.twin_store.write().await;
    store
        .review_constitution_item(&id, request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_action_gaps(state: State<'_, AppState>) -> Result<Vec<ActionGap>, String> {
    let store = state.twin_store.read().await;
    store.list_action_gaps().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn review_action_gap(
    id: String,
    request: ConstitutionReviewRequest,
    state: State<'_, AppState>,
) -> Result<ActionGap, String> {
    let mut store = state.twin_store.write().await;
    store
        .review_action_gap(&id, request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_constitution_setup(
    state: State<'_, AppState>,
) -> Result<ConstitutionSetup, String> {
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
    let mut store = state.twin_store.write().await;
    store
        .save_constitution_setup(setup)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn run_constitution_inference(
    state: State<'_, AppState>,
) -> Result<ConstitutionInferenceSummary, String> {
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
    let session = {
        let mut canvas_store = state.canvas_store.write().await;
        canvas_store
            .get_session(&session_id)
            .map_err(|error| error.to_string())?
    };

    let payload = build_feedback_payload(&session, &request)?;
    let trace_event = {
        let mut twin_store = state.twin_store.write().await;
        twin_store
            .append_trace_event(
                &session_id,
                trace_event_type_for_feedback(&request.feedback_type),
                payload,
            )
            .map_err(|error| error.to_string())?
    };

    let mut created_record_ids = Vec::new();
    if let Some(record_create) =
        build_record_from_feedback(&session, &session_id, &trace_event, &request)?
    {
        let record = {
            let mut twin_store = state.twin_store.write().await;
            twin_store
                .create_user_record(record_create)
                .map_err(|error| error.to_string())?
        };
        created_record_ids.push(record.id);
    }

    Ok(CanvasFeedbackResult {
        trace_event_id: trace_event.id,
        created_record_ids,
    })
}

fn trace_event_type_for_feedback(feedback_type: &CanvasFeedbackType) -> TraceEventType {
    match feedback_type {
        CanvasFeedbackType::Ranking => TraceEventType::RankingRecorded,
        CanvasFeedbackType::Insight => TraceEventType::InsightCaptured,
        CanvasFeedbackType::Accept
        | CanvasFeedbackType::Reject
        | CanvasFeedbackType::Correction => TraceEventType::FeedbackRecorded,
    }
}

fn build_feedback_payload(
    session: &CanvasSession,
    request: &CanvasFeedbackRequest,
) -> Result<serde_json::Value, String> {
    let payload = match &request.feedback_type {
        CanvasFeedbackType::Accept
        | CanvasFeedbackType::Reject
        | CanvasFeedbackType::Correction => {
            let response_ref = request
                .response
                .as_ref()
                .ok_or_else(|| "A response reference is required".to_string())?;
            let (tile, response) = find_response(session, response_ref)?;
            json!({
                "feedback_type": request.feedback_type,
                "response": response_snapshot(response_ref, tile, response),
                "content": request.content,
                "rationale": request.rationale,
                "kind": request.kind,
            })
        }
        CanvasFeedbackType::Ranking => {
            if request.ranked_responses.len() < 2 {
                return Err("At least two ranked responses are required".to_string());
            }

            let ranked = request
                .ranked_responses
                .iter()
                .enumerate()
                .map(|(index, response_ref)| {
                    let (tile, response) = find_response(session, response_ref)?;
                    Ok(json!({
                        "rank": index + 1,
                        "response": response_snapshot(response_ref, tile, response),
                    }))
                })
                .collect::<Result<Vec<_>, String>>()?;

            json!({
                "feedback_type": request.feedback_type,
                "ranked_responses": ranked,
                "content": request.content,
                "rationale": request.rationale,
            })
        }
        CanvasFeedbackType::Insight => {
            if request.kind.is_none() {
                return Err("Insight capture requires a record kind".to_string());
            }

            let evidence = if let Some(response_ref) = request.response.as_ref() {
                let (tile, response) = find_response(session, response_ref)?;
                Some(response_snapshot(response_ref, tile, response))
            } else {
                None
            };

            json!({
                "feedback_type": request.feedback_type,
                "kind": request.kind,
                "content": request.content,
                "rationale": request.rationale,
                "evidence": evidence,
            })
        }
    };

    Ok(payload)
}

fn build_record_from_feedback(
    session: &CanvasSession,
    session_id: &str,
    trace_event: &TraceEvent,
    request: &CanvasFeedbackRequest,
) -> Result<Option<UserRecordCreate>, String> {
    let mut evidence_refs = Vec::new();

    if let Some(response_ref) = request.response.as_ref() {
        evidence_refs.push(EvidenceRef {
            trace_id: session_id.to_string(),
            event_id: trace_event.id.clone(),
            session_id: session_id.to_string(),
            tile_id: Some(response_ref.tile_id.clone()),
            model_id: Some(response_ref.model_id.clone()),
            note: request.rationale.clone(),
            source_type: Some("behavior".to_string()),
            source_id: Some(trace_event.id.clone()),
            source_label: Some("Canvas feedback".to_string()),
            excerpt: request.rationale.clone(),
            speaker_role: Some("user".to_string()),
        });
    } else if let Some(first_ranked) = request.ranked_responses.first() {
        evidence_refs.push(EvidenceRef {
            trace_id: session_id.to_string(),
            event_id: trace_event.id.clone(),
            session_id: session_id.to_string(),
            tile_id: Some(first_ranked.tile_id.clone()),
            model_id: Some(first_ranked.model_id.clone()),
            note: request.rationale.clone(),
            source_type: Some("behavior".to_string()),
            source_id: Some(trace_event.id.clone()),
            source_label: Some("Canvas ranking".to_string()),
            excerpt: request.rationale.clone(),
            speaker_role: Some("user".to_string()),
        });
    } else {
        evidence_refs.push(EvidenceRef {
            trace_id: session_id.to_string(),
            event_id: trace_event.id.clone(),
            session_id: session_id.to_string(),
            tile_id: None,
            model_id: None,
            note: request.rationale.clone(),
            source_type: Some("behavior".to_string()),
            source_id: Some(trace_event.id.clone()),
            source_label: Some("Canvas feedback".to_string()),
            excerpt: request.rationale.clone(),
            speaker_role: Some("user".to_string()),
        });
    }

    let record = match &request.feedback_type {
        CanvasFeedbackType::Accept | CanvasFeedbackType::Reject => {
            let response_ref = request
                .response
                .as_ref()
                .ok_or_else(|| "A response reference is required".to_string())?;
            let (tile, response) = find_response(session, response_ref)?;
            let label = match &request.feedback_type {
                CanvasFeedbackType::Accept => "Accepted",
                CanvasFeedbackType::Reject => "Rejected",
                _ => unreachable!(),
            };
            let content = request.content.clone().unwrap_or_else(|| {
                format!(
                    "{} response from {} for prompt: {}",
                    label, response.model_name, tile.prompt
                )
            });

            Some(UserRecordCreate {
                kind: request.kind.clone().unwrap_or(UserRecordKind::Preference),
                content,
                evidence_refs,
                confidence: request.confidence,
                origin: RecordOrigin::User,
                promotion_state: Some(PromotionState::AutoPromoted),
                valid_from: None,
                valid_until: None,
                links: request.links.clone(),
                metadata: json!({
                    "feedback_type": request.feedback_type,
                    "prompt": tile.prompt,
                    "model_id": response.model_id,
                    "model_name": response.model_name,
                    "response_excerpt": excerpt(&response.content),
                    "rationale": request.rationale,
                })
                .as_object()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            })
        }
        CanvasFeedbackType::Ranking => {
            if request.ranked_responses.len() < 2 {
                return Err("At least two ranked responses are required".to_string());
            }

            let ranked_snapshots = request
                .ranked_responses
                .iter()
                .enumerate()
                .map(|(index, response_ref)| {
                    let (tile, response) = find_response(session, response_ref)?;
                    Ok(json!({
                        "rank": index + 1,
                        "tile_id": response_ref.tile_id,
                        "model_id": response.model_id,
                        "model_name": response.model_name,
                        "prompt": tile.prompt,
                        "response_excerpt": excerpt(&response.content),
                    }))
                })
                .collect::<Result<Vec<_>, String>>()?;

            let content = request.content.clone().unwrap_or_else(|| {
                let summary = ranked_snapshots
                    .iter()
                    .map(|snapshot| {
                        format!(
                            "{}. {}",
                            snapshot["rank"].as_u64().unwrap_or_default(),
                            snapshot["model_name"].as_str().unwrap_or("model")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" > ");
                format!("Preference ranking recorded: {}", summary)
            });

            Some(UserRecordCreate {
                kind: request.kind.clone().unwrap_or(UserRecordKind::Preference),
                content,
                evidence_refs,
                confidence: request.confidence,
                origin: RecordOrigin::User,
                promotion_state: Some(PromotionState::AutoPromoted),
                valid_from: None,
                valid_until: None,
                links: request.links.clone(),
                metadata: json!({
                    "feedback_type": request.feedback_type,
                    "rationale": request.rationale,
                    "ranked_responses": ranked_snapshots,
                })
                .as_object()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            })
        }
        CanvasFeedbackType::Correction => {
            let content = request
                .content
                .clone()
                .ok_or_else(|| "Correction feedback requires content".to_string())?;

            Some(UserRecordCreate {
                kind: request.kind.clone().unwrap_or(UserRecordKind::Fact),
                content,
                evidence_refs,
                confidence: request.confidence,
                origin: RecordOrigin::User,
                promotion_state: Some(PromotionState::AutoPromoted),
                valid_from: None,
                valid_until: None,
                links: request.links.clone(),
                metadata: json!({
                    "feedback_type": request.feedback_type,
                    "rationale": request.rationale,
                })
                .as_object()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            })
        }
        CanvasFeedbackType::Insight => {
            let content = request
                .content
                .clone()
                .ok_or_else(|| "Insight capture requires content".to_string())?;
            let kind = request
                .kind
                .clone()
                .ok_or_else(|| "Insight capture requires a record kind".to_string())?;

            Some(UserRecordCreate {
                kind,
                content,
                evidence_refs,
                confidence: request.confidence,
                origin: RecordOrigin::User,
                promotion_state: Some(PromotionState::AutoPromoted),
                valid_from: None,
                valid_until: None,
                links: request.links.clone(),
                metadata: json!({
                    "feedback_type": request.feedback_type,
                    "rationale": request.rationale,
                })
                .as_object()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            })
        }
    };

    Ok(record)
}

fn response_snapshot(
    response_ref: &CanvasResponseRef,
    tile: &PromptTile,
    response: &ModelResponse,
) -> serde_json::Value {
    json!({
        "tile_id": response_ref.tile_id,
        "model_id": response.model_id,
        "model_name": response.model_name,
        "prompt": tile.prompt,
        "response_content": response.content,
        "status": response.status,
    })
}

fn find_response<'a>(
    session: &'a CanvasSession,
    response_ref: &CanvasResponseRef,
) -> Result<(&'a PromptTile, &'a ModelResponse), String> {
    let tile = session
        .prompt_tiles
        .iter()
        .find(|tile| tile.id == response_ref.tile_id)
        .ok_or_else(|| format!("Tile not found: {}", response_ref.tile_id))?;
    let response = tile
        .responses
        .get(&response_ref.model_id)
        .ok_or_else(|| format!("Response not found: {}", response_ref.model_id))?;

    Ok((tile, response))
}

fn excerpt(content: &str) -> String {
    const MAX_LEN: usize = 220;
    if content.chars().count() <= MAX_LEN {
        return content.to_string();
    }

    let mut excerpt: String = content.chars().take(MAX_LEN).collect();
    excerpt.push_str("...");
    excerpt
}
