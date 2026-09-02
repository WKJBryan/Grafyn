use super::TwinStore;
use crate::models::canvas::{CanvasSession, ModelResponse, PromptTile, ResponseStatus};
use crate::models::twin::{
    CanvasFeedbackRequest, CanvasFeedbackResult, CanvasFeedbackType, CanvasResponseRef,
    CanvasResponseVersionWitness, EvidenceRef as LegacyEvidenceRef, PromotionState, RecordOrigin,
    TraceEvent, TraceEventType, UserRecordCreate, UserRecordKind,
};
use crate::models::twin_event::{
    ClaimAssertion, ClaimObject, ClaimPolarity, ClaimPredicate, EntityId, EvidenceRef,
    EvidenceType, Identifier, SourceChannel, TwinEvent, TwinEventPayload,
};
use anyhow::{bail, Result};
use serde_json::json;
use std::collections::HashSet;

type CanvasFeedbackMutationPlan = (
    CanvasFeedbackResult,
    crate::models::twin::SessionTrace,
    crate::models::twin::UserRecord,
    Vec<(std::path::PathBuf, String)>,
    Vec<crate::services::twin_events::TwinEventDraft>,
);

impl TwinStore {
    pub fn record_canvas_feedback(
        &mut self,
        session: &CanvasSession,
        request: CanvasFeedbackRequest,
    ) -> Result<CanvasFeedbackResult> {
        let (result, _commit) = self.record_canvas_feedback_with_commit(session, request)?;
        Ok(result)
    }

    pub(crate) fn record_canvas_feedback_with_commit(
        &mut self,
        session: &CanvasSession,
        request: CanvasFeedbackRequest,
    ) -> Result<(
        CanvasFeedbackResult,
        crate::services::twin_events::MutationCommit,
    )> {
        Self::validate_file_id(&session.id)?;
        if !self.event_recorder.is_noop() {
            let mut committed = None;
            let commit = self.commit_planned_twin_mutation(|store| {
                let durable_session = store
                    .read_canvas_session_bounded(&session.id)?
                    .ok_or_else(|| anyhow::anyhow!("Persisted Canvas session was not found"))?;
                let (result, _trace, _record, values, drafts) =
                    store.plan_canvas_feedback_mutation(&durable_session, &request)?;
                let targets = store.governed_json_targets(values)?;
                committed = Some(result);
                Ok(Some(crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::SyncEligible,
                    SourceChannel::parse("canvas").map_err(anyhow::Error::msg)?,
                    targets,
                    drafts,
                )))
            })?;
            let result =
                committed.ok_or_else(|| anyhow::anyhow!("Canvas feedback was not planned"))?;
            self.invalidate_mutation_caches();
            return Ok((result, commit));
        }
        self.ensure_record_cache()?;
        let (result, _trace, _record, values, drafts) =
            self.plan_canvas_feedback_mutation(session, &request)?;
        let commit = self.commit_governed_json_targets_with_source(
            SourceChannel::parse("canvas").map_err(anyhow::Error::msg)?,
            values,
            drafts,
        )?;
        self.invalidate_mutation_caches();
        Ok((result, commit))
    }

    fn plan_canvas_feedback_mutation(
        &self,
        session: &CanvasSession,
        request: &CanvasFeedbackRequest,
    ) -> Result<CanvasFeedbackMutationPlan> {
        validate_feedback_request(session, request)?;
        let payload = build_feedback_payload(session, request)?;
        let (trace_event, trace) = self.plan_trace_event(
            &session.id,
            trace_event_type(&request.feedback_type),
            payload,
        )?;
        let record = Self::materialize_user_record(build_record_from_feedback(
            session,
            &trace_event,
            request,
        )?);
        let session_digest = crate::services::twin_events::digest_bytes(
            serde_json::to_string_pretty(session)?.as_bytes(),
        );
        let canvas_evidence = EvidenceRef {
            evidence_type: EvidenceType::CanvasSession,
            source_id: Identifier::parse(&session.id).map_err(anyhow::Error::msg)?,
            digest: Some(session_digest),
        };
        let mut drafts = feedback_drafts(session, &trace_event, request)?;
        for draft in &mut drafts {
            draft.evidence.push(canvas_evidence.clone());
        }
        let preference_claim = explicit_preference_insight_claim(request)?;
        let preference_source = if let Some(claim) = preference_claim.as_ref() {
            let response_ref = required_response(request)?;
            let (_, response) = completed_response(session, response_ref)?;
            let response_witness = required_response_witness(request)?;
            let events = self.event_recorder.recorded_events()?;
            let source = source_canvas_response_event(
                &events,
                session,
                response_ref,
                response,
                response_witness,
            )?;
            if preference_already_captured(&events, claim, source) {
                bail!("Twin preference evidence for this response was already captured");
            }
            Some(source.clone())
        } else {
            None
        };
        let mut observation = self.record_observation_draft(&record, false, None)?;
        if let Some(claim) = preference_claim {
            let TwinEventPayload::ObservationRecorded(payload) = &mut observation.payload else {
                unreachable!("legacy record capture is always an observation")
            };
            payload.claims.push(claim);
            let source = preference_source.expect("preference source was resolved with its claim");
            observation.causal_parents.push(source.event_id.clone());
            observation.context = source.context.clone();
            observation.governance = source.governance.clone();
            observation.evidence.push(EvidenceRef {
                evidence_type: EvidenceType::Event,
                source_id: Identifier::parse(source.event_id.as_str())
                    .map_err(anyhow::Error::msg)?,
                digest: None,
            });
            observation.evidence.sort();
            observation.evidence.dedup();
        }
        drafts.push(observation);
        let values = vec![
            self.serialized_trace_target(&trace)?,
            (
                self.record_file_path(&record.id),
                serde_json::to_string_pretty(&record)?,
            ),
        ];
        let result = CanvasFeedbackResult {
            trace_event_id: trace_event.id,
            created_record_ids: vec![record.id.clone()],
        };
        Ok((result, trace, record, values, drafts))
    }
}

fn validate_feedback_request(
    session: &CanvasSession,
    request: &CanvasFeedbackRequest,
) -> Result<()> {
    match request.feedback_type {
        CanvasFeedbackType::Accept | CanvasFeedbackType::Reject => {
            completed_response(session, required_response(request)?)?;
        }
        CanvasFeedbackType::Correction => {
            completed_response(session, required_response(request)?)?;
            if request.content.as_deref().is_none_or(str::is_empty) {
                bail!("Correction feedback requires content");
            }
        }
        CanvasFeedbackType::Insight => {
            if request
                .content
                .as_deref()
                .is_none_or(|content| content.trim().is_empty())
            {
                bail!("Insight capture requires content");
            }
            if request.kind.is_none() {
                bail!("Insight capture requires a record kind");
            }
            if is_explicit_preference_insight(request) {
                let response_ref = required_response(request)?;
                let (tile, response) = completed_response(session, response_ref)?;
                validate_response_witness(response, required_response_witness(request)?)?;
                tile.validate_twin_relationship_context(None)
                    .map_err(anyhow::Error::msg)?;
                if !tile.twin_relationship_variant.relationships.is_empty() {
                    bail!("Twin preference capture currently requires a global Canvas response");
                }
            }
            if let Some(response) = request.response.as_ref() {
                completed_response(session, response)?;
            }
        }
        CanvasFeedbackType::Ranking => {
            if !(2..=63).contains(&request.ranked_responses.len()) {
                bail!("A ranking must contain 2..=63 persisted responses");
            }
            let mut response_ids = HashSet::new();
            for response_ref in &request.ranked_responses {
                let (_, response) = completed_response(session, response_ref)?;
                if !response_ids.insert(response.id.as_str()) {
                    bail!("A ranking cannot contain the same persisted response twice");
                }
            }
        }
    }
    Ok(())
}

fn explicit_preference_insight_claim(
    request: &CanvasFeedbackRequest,
) -> Result<Option<ClaimAssertion>> {
    if !is_explicit_preference_insight(request) {
        return Ok(None);
    }
    let content = request
        .content
        .as_deref()
        .expect("validated preference insight content")
        .trim();
    Ok(Some(ClaimAssertion {
        subject_id: EntityId::parse("owner").map_err(anyhow::Error::msg)?,
        predicate: ClaimPredicate::parse("prefers").map_err(anyhow::Error::msg)?,
        object: ClaimObject::parse(content).map_err(anyhow::Error::msg)?,
        polarity: ClaimPolarity::Affirmed,
    }))
}

fn is_explicit_preference_insight(request: &CanvasFeedbackRequest) -> bool {
    request.feedback_type == CanvasFeedbackType::Insight
        && request.kind.as_ref() == Some(&UserRecordKind::Preference)
}

fn required_response_witness(
    request: &CanvasFeedbackRequest,
) -> Result<&CanvasResponseVersionWitness> {
    request.response_witness.as_ref().ok_or_else(|| {
        anyhow::anyhow!("Twin preference capture requires an exact response version witness")
    })
}

fn validate_response_witness(
    response: &ModelResponse,
    witness: &CanvasResponseVersionWitness,
) -> Result<()> {
    if witness.response_id != response.id || witness.response_content != response.content {
        bail!("Canvas response changed before capture; reopen the preference capture and review the current response");
    }
    Ok(())
}

fn source_canvas_response_event<'a>(
    events: &'a [TwinEvent],
    session: &CanvasSession,
    response_ref: &CanvasResponseRef,
    response: &ModelResponse,
    witness: &CanvasResponseVersionWitness,
) -> Result<&'a TwinEvent> {
    let response_digest =
        crate::services::twin_events::digest_bytes(witness.response_content.as_bytes());
    let superseded = events
        .iter()
        .flat_map(|event| event.supersedes.iter().map(|id| id.as_str()))
        .collect::<HashSet<_>>();
    let matches = events
        .iter()
        .filter(|event| {
            !superseded.contains(event.event_id.as_str())
                && match &event.payload {
                    TwinEventPayload::CanvasResponseRecorded(recorded) => {
                        recorded.session_id.as_str() == session.id
                            && recorded.tile_id.as_str() == response_ref.tile_id
                            && recorded.response_id.as_str() == witness.response_id
                            && recorded.model_id.as_str() == response.model_id
                            && recorded.response.as_str() == witness.response_content
                            && recorded.response_digest.as_ref() == Some(&response_digest)
                    }
                    _ => false,
                }
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [source] => Ok(*source),
        [] => Err(anyhow::anyhow!(
            "Persisted Canvas response evidence is unavailable for Twin preference capture"
        )),
        _ => Err(anyhow::anyhow!(
            "Persisted Canvas response evidence is ambiguous for Twin preference capture"
        )),
    }
}

fn same_canvas_response_identity(
    left: &crate::models::twin_event::CanvasResponseRecorded,
    right: &crate::models::twin_event::CanvasResponseRecorded,
) -> bool {
    left.session_id == right.session_id
        && left.tile_id == right.tile_id
        && left.response_id == right.response_id
}

fn preference_already_captured(
    events: &[TwinEvent],
    claim: &ClaimAssertion,
    source_event: &TwinEvent,
) -> bool {
    let TwinEventPayload::CanvasResponseRecorded(source_response) = &source_event.payload else {
        return false;
    };
    let source_event_ids = events
        .iter()
        .filter_map(|event| match &event.payload {
            TwinEventPayload::CanvasResponseRecorded(response)
                if same_canvas_response_identity(response, source_response) =>
            {
                Some(event.event_id.as_str())
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    events.iter().any(|event| {
        let TwinEventPayload::ObservationRecorded(observation) = &event.payload else {
            return false;
        };
        observation.claims.contains(claim)
            && (event.evidence.iter().any(|evidence| {
                evidence.evidence_type == EvidenceType::Event
                    && source_event_ids.contains(evidence.source_id.as_str())
            }) || event
                .causal_parents
                .iter()
                .any(|parent| source_event_ids.contains(parent.as_str())))
    })
}

fn feedback_drafts(
    session: &CanvasSession,
    trace_event: &TraceEvent,
    request: &CanvasFeedbackRequest,
) -> Result<Vec<crate::services::twin_events::TwinEventDraft>> {
    let kind = feedback_kind(&request.feedback_type);
    let feedback_id = format!(
        "feedback-{}",
        crate::services::twin_events::digest_bytes(
            format!("{}\0{}\0{}", session.id, trace_event.id, kind).as_bytes(),
        )
        .as_str()
    );
    let source = SourceChannel::parse("canvas").map_err(anyhow::Error::msg)?;
    let governance = crate::services::twin_events::standard_capture_governance();

    let members = match request.feedback_type {
        CanvasFeedbackType::Ranking => request
            .ranked_responses
            .iter()
            .enumerate()
            .map(|(index, response_ref)| {
                let (_, response) = completed_response(session, response_ref)?;
                crate::services::twin_events::feedback_draft(
                    crate::services::twin_events::FeedbackDraft {
                        feedback_id: &feedback_id,
                        target_id: &response.id,
                        kind,
                        content: None,
                        rationale: request.rationale.as_deref(),
                        rank: Some(u16::try_from(index + 1)?),
                        observed_at: trace_event.created_at,
                        source_channel: source.clone(),
                        governance: governance.clone(),
                    },
                )
                .map_err(anyhow::Error::msg)
            })
            .collect::<Result<Vec<_>>>()?,
        CanvasFeedbackType::Insight => {
            let target_id = if let Some(response_ref) = request.response.as_ref() {
                completed_response(session, response_ref)?.1.id.as_str()
            } else {
                session.id.as_str()
            };
            vec![crate::services::twin_events::feedback_draft(
                crate::services::twin_events::FeedbackDraft {
                    feedback_id: &feedback_id,
                    target_id,
                    kind,
                    content: request.content.as_deref(),
                    rationale: request.rationale.as_deref(),
                    rank: None,
                    observed_at: trace_event.created_at,
                    source_channel: source,
                    governance,
                },
            )
            .map_err(anyhow::Error::msg)?]
        }
        CanvasFeedbackType::Accept
        | CanvasFeedbackType::Reject
        | CanvasFeedbackType::Correction => {
            let (_, response) = completed_response(session, required_response(request)?)?;
            let content = (request.feedback_type == CanvasFeedbackType::Correction)
                .then_some(request.content.as_deref())
                .flatten();
            vec![crate::services::twin_events::feedback_draft(
                crate::services::twin_events::FeedbackDraft {
                    feedback_id: &feedback_id,
                    target_id: &response.id,
                    kind,
                    content,
                    rationale: request.rationale.as_deref(),
                    rank: None,
                    observed_at: trace_event.created_at,
                    source_channel: source,
                    governance,
                },
            )
            .map_err(anyhow::Error::msg)?]
        }
    };
    Ok(members)
}

fn feedback_kind(feedback_type: &CanvasFeedbackType) -> &'static str {
    match feedback_type {
        CanvasFeedbackType::Accept => "accept",
        CanvasFeedbackType::Reject => "reject",
        CanvasFeedbackType::Ranking => "ranking",
        CanvasFeedbackType::Correction => "correction",
        CanvasFeedbackType::Insight => "insight",
    }
}

fn trace_event_type(feedback_type: &CanvasFeedbackType) -> TraceEventType {
    match feedback_type {
        CanvasFeedbackType::Ranking => TraceEventType::RankingRecorded,
        CanvasFeedbackType::Insight => TraceEventType::InsightCaptured,
        CanvasFeedbackType::Accept
        | CanvasFeedbackType::Reject
        | CanvasFeedbackType::Correction => TraceEventType::FeedbackRecorded,
    }
}

fn required_response(request: &CanvasFeedbackRequest) -> Result<&CanvasResponseRef> {
    request
        .response
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("A response reference is required"))
}

fn completed_response<'a>(
    session: &'a CanvasSession,
    response_ref: &CanvasResponseRef,
) -> Result<(&'a PromptTile, &'a ModelResponse)> {
    let tile = session
        .prompt_tiles
        .iter()
        .find(|tile| tile.id == response_ref.tile_id)
        .ok_or_else(|| anyhow::anyhow!("Tile not found: {}", response_ref.tile_id))?;
    let response = tile
        .responses
        .get(&response_ref.model_id)
        .ok_or_else(|| anyhow::anyhow!("Response not found: {}", response_ref.model_id))?;
    if response.status != ResponseStatus::Completed || response.content.trim().is_empty() {
        bail!("Feedback requires a completed persisted response");
    }
    Ok((tile, response))
}

fn build_feedback_payload(
    session: &CanvasSession,
    request: &CanvasFeedbackRequest,
) -> Result<serde_json::Value> {
    Ok(match request.feedback_type {
        CanvasFeedbackType::Accept
        | CanvasFeedbackType::Reject
        | CanvasFeedbackType::Correction => {
            let response_ref = required_response(request)?;
            let (tile, response) = completed_response(session, response_ref)?;
            json!({
                "feedback_type": request.feedback_type,
                "response": response_snapshot(response_ref, tile, response),
                "content": request.content,
                "rationale": request.rationale,
                "kind": request.kind,
            })
        }
        CanvasFeedbackType::Ranking => json!({
            "feedback_type": request.feedback_type,
            "ranked_responses": request.ranked_responses.iter().enumerate().map(|(index, response_ref)| {
                let (tile, response) = completed_response(session, response_ref)?;
                Ok(json!({
                    "rank": index + 1,
                    "response": response_snapshot(response_ref, tile, response),
                }))
            }).collect::<Result<Vec<_>>>()?,
            "content": request.content,
            "rationale": request.rationale,
        }),
        CanvasFeedbackType::Insight => {
            let evidence = request
                .response
                .as_ref()
                .map(|response_ref| {
                    let (tile, response) = completed_response(session, response_ref)?;
                    Ok::<_, anyhow::Error>(response_snapshot(response_ref, tile, response))
                })
                .transpose()?;
            json!({
                "feedback_type": request.feedback_type,
                "kind": request.kind,
                "content": request.content,
                "rationale": request.rationale,
                "evidence": evidence,
            })
        }
    })
}

fn build_record_from_feedback(
    session: &CanvasSession,
    trace_event: &TraceEvent,
    request: &CanvasFeedbackRequest,
) -> Result<UserRecordCreate> {
    let primary = request
        .response
        .as_ref()
        .or_else(|| request.ranked_responses.first());
    let evidence_refs = vec![LegacyEvidenceRef {
        trace_id: session.id.clone(),
        event_id: trace_event.id.clone(),
        session_id: session.id.clone(),
        tile_id: primary.map(|value| value.tile_id.clone()),
        model_id: primary.map(|value| value.model_id.clone()),
        note: request.rationale.clone(),
        source_type: Some("behavior".to_string()),
        source_id: Some(trace_event.id.clone()),
        source_label: Some("Canvas feedback".to_string()),
        excerpt: request.rationale.clone(),
        speaker_role: Some("user".to_string()),
    }];

    let (kind, content, metadata) = match request.feedback_type {
        CanvasFeedbackType::Accept | CanvasFeedbackType::Reject => {
            let response_ref = required_response(request)?;
            let (tile, response) = completed_response(session, response_ref)?;
            let label = if request.feedback_type == CanvasFeedbackType::Accept {
                "Accepted"
            } else {
                "Rejected"
            };
            (
                request.kind.clone().unwrap_or(UserRecordKind::Preference),
                request.content.clone().unwrap_or_else(|| {
                    format!(
                        "{} response from {} for prompt: {}",
                        label, response.model_name, tile.prompt
                    )
                }),
                json!({
                    "feedback_type": request.feedback_type,
                    "prompt": tile.prompt,
                    "model_id": response.model_id,
                    "model_name": response.model_name,
                    "response_excerpt": excerpt(&response.content),
                    "rationale": request.rationale,
                }),
            )
        }
        CanvasFeedbackType::Ranking => {
            let ranked_snapshots = request
                .ranked_responses
                .iter()
                .enumerate()
                .map(|(index, response_ref)| {
                    let (tile, response) = completed_response(session, response_ref)?;
                    Ok(json!({
                        "rank": index + 1,
                        "tile_id": response_ref.tile_id,
                        "model_id": response.model_id,
                        "model_name": response.model_name,
                        "prompt": tile.prompt,
                        "response_excerpt": excerpt(&response.content),
                    }))
                })
                .collect::<Result<Vec<_>>>()?;
            let labels = ranked_snapshots
                .iter()
                .map(|snapshot| {
                    format!(
                        "{}. {}",
                        snapshot["rank"].as_u64().unwrap_or_default(),
                        snapshot["model_name"].as_str().unwrap_or("model")
                    )
                })
                .collect::<Vec<_>>();
            (
                request.kind.clone().unwrap_or(UserRecordKind::Preference),
                request.content.clone().unwrap_or_else(|| {
                    format!("Preference ranking recorded: {}", labels.join(" > "))
                }),
                json!({
                    "feedback_type": request.feedback_type,
                    "rationale": request.rationale,
                    "ranked_responses": ranked_snapshots,
                }),
            )
        }
        CanvasFeedbackType::Correction => (
            request.kind.clone().unwrap_or(UserRecordKind::Fact),
            request
                .content
                .clone()
                .expect("validated correction content"),
            json!({
                "feedback_type": request.feedback_type,
                "rationale": request.rationale,
            }),
        ),
        CanvasFeedbackType::Insight => (
            request.kind.clone().expect("validated insight kind"),
            request.content.clone().expect("validated insight content"),
            json!({
                "feedback_type": request.feedback_type,
                "rationale": request.rationale,
            }),
        ),
    };

    Ok(UserRecordCreate {
        kind,
        content,
        origin: RecordOrigin::User,
        evidence_refs,
        confidence: request.confidence,
        promotion_state: Some(PromotionState::Candidate),
        valid_from: None,
        valid_until: None,
        links: request.links.clone(),
        metadata: metadata
            .as_object()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect(),
    })
}

fn response_snapshot(
    response_ref: &CanvasResponseRef,
    tile: &PromptTile,
    response: &ModelResponse,
) -> serde_json::Value {
    json!({
        "response_id": response.id,
        "tile_id": response_ref.tile_id,
        "model_id": response.model_id,
        "model_name": response.model_name,
        "prompt": tile.prompt,
        "response_content": response.content,
        "status": response.status,
    })
}

fn excerpt(content: &str) -> String {
    const MAX_LEN: usize = 220;
    if content.chars().count() <= MAX_LEN {
        return content.to_string();
    }
    let mut excerpt = content.chars().take(MAX_LEN).collect::<String>();
    excerpt.push_str("...");
    excerpt
}

#[cfg(test)]
mod tests {
    use super::super::TwinStore;
    use crate::models::canvas::{CanvasSession, ModelResponse, PromptTile, ResponseStatus};
    use crate::models::twin::{
        CanvasFeedbackRequest, CanvasFeedbackType, CanvasResponseRef, UserRecordKind,
    };
    use crate::models::twin_event::{
        CausalStream, ClaimPolarity, EntityId, EvidenceType, RelationshipDirection,
        RelationshipPredicate, SourceChannel, TwinEventPayload,
    };
    use crate::models::twin_state::{RelationshipKey, RelationshipVariant};
    use crate::services::twin_events::{canvas_transition_drafts, digest_bytes, project};
    use chrono::Utc;
    use tempfile::tempdir;

    fn session() -> CanvasSession {
        let mut session = CanvasSession {
            id: "session-feedback".to_string(),
            ..CanvasSession::default()
        };
        let mut tile = PromptTile {
            id: "tile-feedback".to_string(),
            prompt: "Which answer is stronger?".to_string(),
            ..PromptTile::default()
        };
        for (model, response_id, content) in [
            ("model-a", "response-a", "Answer A"),
            ("model-b", "response-b", "Answer B"),
            ("model-c", "response-c", "Answer C"),
        ] {
            tile.responses.insert(
                model.to_string(),
                ModelResponse {
                    id: response_id.to_string(),
                    model_id: model.to_string(),
                    model_name: model.to_string(),
                    content: content.to_string(),
                    status: ResponseStatus::Completed,
                    ..ModelResponse::default()
                },
            );
        }
        session.prompt_tiles.push(tile);
        session
    }

    fn coordinated_store(
        root: &std::path::Path,
    ) -> (
        TwinStore,
        std::sync::Arc<crate::services::twin_events::TwinEventStore>,
        std::sync::Arc<crate::services::twin_events::MutationCoordinator>,
    ) {
        let data = root.join("data");
        let vault = root.join("vault");
        let twin_root = data.join("twin").join("scope-one");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                events.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        (
            TwinStore::with_event_recorder(twin_root, data.join("twin"), coordinator.clone()),
            events,
            coordinator,
        )
    }

    fn persist_session(root: &std::path::Path, session: &CanvasSession) {
        let canvas = root.join("data").join("canvas");
        std::fs::create_dir_all(&canvas).unwrap();
        std::fs::write(
            canvas.join(format!("{}.json", session.id)),
            serde_json::to_vec_pretty(session).unwrap(),
        )
        .unwrap();
    }

    fn persist_session_with_response_events(
        root: &std::path::Path,
        session: &CanvasSession,
        coordinator: &crate::services::twin_events::MutationCoordinator,
    ) {
        persist_session(root, session);
        let session_bytes = serde_json::to_vec_pretty(session).unwrap();
        let drafts =
            canvas_transition_drafts(None, session, &[], digest_bytes(&session_bytes)).unwrap();
        let _commit = coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("canvas").unwrap(),
                Vec::new(),
                drafts,
            )
            .unwrap();
    }

    #[test]
    fn feedback_targets_persisted_response_ids_and_preserves_ranking_order() {
        let root = tempdir().unwrap();
        let (mut store, events, _) = coordinated_store(root.path());
        let session = session();
        persist_session(root.path(), &session);

        let accepted_result = store
            .record_canvas_feedback(
                &session,
                CanvasFeedbackRequest {
                    feedback_type: CanvasFeedbackType::Accept,
                    response: Some(CanvasResponseRef {
                        tile_id: "tile-feedback".to_string(),
                        model_id: "model-a".to_string(),
                    }),
                    rationale: Some("More concrete".to_string()),
                    ..serde_json::from_value(serde_json::json!({
                        "feedback_type": "accept"
                    }))
                    .unwrap()
                },
            )
            .unwrap();
        let accepted_record = store
            .get_user_record(&accepted_result.created_record_ids[0])
            .unwrap();
        assert_eq!(
            accepted_record
                .metadata
                .get("prompt")
                .and_then(serde_json::Value::as_str),
            Some("Which answer is stronger?")
        );
        assert_eq!(
            accepted_record
                .metadata
                .get("model_id")
                .and_then(serde_json::Value::as_str),
            Some("model-a")
        );
        assert_eq!(
            accepted_record
                .metadata
                .get("model_name")
                .and_then(serde_json::Value::as_str),
            Some("model-a")
        );
        assert_eq!(
            accepted_record
                .metadata
                .get("response_excerpt")
                .and_then(serde_json::Value::as_str),
            Some("Answer A")
        );

        let ranking_result = store
            .record_canvas_feedback(
                &session,
                CanvasFeedbackRequest {
                    feedback_type: CanvasFeedbackType::Ranking,
                    ranked_responses: vec![
                        CanvasResponseRef {
                            tile_id: "tile-feedback".to_string(),
                            model_id: "model-b".to_string(),
                        },
                        CanvasResponseRef {
                            tile_id: "tile-feedback".to_string(),
                            model_id: "model-a".to_string(),
                        },
                    ],
                    kind: Some(UserRecordKind::Preference),
                    ..serde_json::from_value(serde_json::json!({
                        "feedback_type": "ranking"
                    }))
                    .unwrap()
                },
            )
            .unwrap();
        let ranking_record = store
            .get_user_record(&ranking_result.created_record_ids[0])
            .unwrap();
        let snapshots = ranking_record
            .metadata
            .get("ranked_responses")
            .and_then(serde_json::Value::as_array)
            .unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0]["rank"], 1);
        assert_eq!(snapshots[0]["tile_id"], "tile-feedback");
        assert_eq!(snapshots[0]["model_id"], "model-b");
        assert_eq!(snapshots[0]["model_name"], "model-b");
        assert_eq!(snapshots[0]["prompt"], "Which answer is stronger?");
        assert_eq!(snapshots[0]["response_excerpt"], "Answer B");
        assert_eq!(snapshots[1]["rank"], 2);
        assert_eq!(snapshots[1]["model_id"], "model-a");

        let captured = events.ordered_events().unwrap();
        assert_eq!(captured.len(), 5);
        let TwinEventPayload::FeedbackRecorded(accepted) = &captured[0].payload else {
            panic!("accept must be typed feedback");
        };
        assert_eq!(accepted.target_id.as_str(), "response-a");
        assert_eq!(accepted.kind.as_str(), "accept");
        assert!(accepted.content.is_none());
        let ranking = captured[2..4]
            .iter()
            .map(|event| {
                let TwinEventPayload::FeedbackRecorded(feedback) = &event.payload else {
                    panic!("ranking members must be typed feedback");
                };
                (
                    feedback.target_id.as_str().to_string(),
                    feedback.rank,
                    feedback.feedback_id.as_str().to_string(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(ranking[0].0, "response-b");
        assert_eq!(ranking[0].1, Some(1));
        assert_eq!(ranking[1].0, "response-a");
        assert_eq!(ranking[1].1, Some(2));
        assert_eq!(ranking[0].2, ranking[1].2);
        assert!(captured.iter().all(|event| !matches!(
            event.payload,
            TwinEventPayload::MemoryProposed(_) | TwinEventPayload::MemoryReviewed(_)
        )));
    }

    #[test]
    fn feedback_trace_record_and_events_recover_as_one_mutation() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_store(root.path());
        let session = session();
        persist_session(root.path(), &session);
        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));

        let (_, commit) = store
            .record_canvas_feedback_with_commit(
                &session,
                CanvasFeedbackRequest {
                    feedback_type: CanvasFeedbackType::Correction,
                    response: Some(CanvasResponseRef {
                        tile_id: "tile-feedback".to_string(),
                        model_id: "model-a".to_string(),
                    }),
                    content: Some("The corrected fact".to_string()),
                    ..serde_json::from_value(serde_json::json!({
                        "feedback_type": "correction"
                    }))
                    .unwrap()
                },
            )
            .unwrap();
        assert!(commit.postcommit_warning);
        assert!(!store.trace_cache.contains_key(&session.id));
        assert!(store.record_cache.is_empty());
        assert_eq!(coordinator.recover_pending().unwrap(), 0);

        assert_eq!(events.ordered_events().unwrap().len(), 2);
        let data = root.path().join("data");
        let mut restarted = TwinStore::with_event_recorder(
            data.join("twin/scope-one"),
            data.join("twin"),
            coordinator,
        );
        assert_eq!(
            restarted
                .get_session_trace(&session.id)
                .unwrap()
                .events
                .len(),
            1
        );
        assert_eq!(restarted.list_user_records().unwrap().len(), 1);
    }

    #[test]
    fn canvas_feedback_validates_the_fresh_persisted_session_under_the_shared_lock() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_store(root.path());
        let stale_session = session();
        let mut durable_session = stale_session.clone();
        durable_session.prompt_tiles[0]
            .responses
            .get_mut("model-a")
            .unwrap()
            .status = ResponseStatus::Error;
        persist_session(root.path(), &durable_session);

        let result = store.record_canvas_feedback(
            &stale_session,
            CanvasFeedbackRequest {
                feedback_type: CanvasFeedbackType::Accept,
                response: Some(CanvasResponseRef {
                    tile_id: "tile-feedback".to_string(),
                    model_id: "model-a".to_string(),
                }),
                ..serde_json::from_value(serde_json::json!({
                    "feedback_type": "accept"
                }))
                .unwrap()
            },
        );

        assert!(result.is_err());
        assert!(events.ordered_events().unwrap().is_empty());
        assert_eq!(coordinator.pending_count().unwrap(), 0);
    }

    fn insight_request(
        model_id: &str,
        kind: UserRecordKind,
        content: &str,
    ) -> CanvasFeedbackRequest {
        let (response_id, response_content) = match model_id {
            "model-a" => ("response-a", "Answer A"),
            "model-b" => ("response-b", "Answer B"),
            "model-c" => ("response-c", "Answer C"),
            _ => panic!("unknown fixture model: {model_id}"),
        };
        CanvasFeedbackRequest {
            feedback_type: CanvasFeedbackType::Insight,
            response: Some(CanvasResponseRef {
                tile_id: "tile-feedback".to_string(),
                model_id: model_id.to_string(),
            }),
            kind: Some(kind),
            content: Some(content.to_string()),
            ..serde_json::from_value(serde_json::json!({
                "feedback_type": "insight",
                "response_witness": {
                    "response_id": response_id,
                    "response_content": response_content
                }
            }))
            .unwrap()
        }
    }

    #[test]
    fn preference_insight_requires_an_exact_response_version_witness() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_store(root.path());
        let session = session();
        persist_session_with_response_events(root.path(), &session, &coordinator);
        let event_count = events.ordered_events().unwrap().len();

        let error = store
            .record_canvas_feedback(
                &session,
                CanvasFeedbackRequest {
                    feedback_type: CanvasFeedbackType::Insight,
                    response: Some(CanvasResponseRef {
                        tile_id: "tile-feedback".to_string(),
                        model_id: "model-a".to_string(),
                    }),
                    kind: Some(UserRecordKind::Preference),
                    content: Some("answers with concrete implementation details".to_string()),
                    ..serde_json::from_value(serde_json::json!({
                        "feedback_type": "insight"
                    }))
                    .unwrap()
                },
            )
            .unwrap_err();

        assert!(error.to_string().contains("response version witness"));
        assert_eq!(events.ordered_events().unwrap().len(), event_count);
    }

    #[test]
    fn preference_insight_rejects_a_stale_witness_after_regeneration() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_store(root.path());
        let stale_session = session();
        persist_session_with_response_events(root.path(), &stale_session, &coordinator);
        let request = insight_request(
            "model-a",
            UserRecordKind::Preference,
            "answers with concrete implementation details",
        );

        let mut regenerated_session = stale_session.clone();
        regenerated_session.prompt_tiles[0]
            .responses
            .get_mut("model-a")
            .unwrap()
            .content = "Regenerated answer A".to_string();
        persist_session(root.path(), &regenerated_session);
        let prior_events = events.ordered_events().unwrap();
        let session_bytes = serde_json::to_vec_pretty(&regenerated_session).unwrap();
        let drafts = canvas_transition_drafts(
            Some(&stale_session),
            &regenerated_session,
            &prior_events,
            digest_bytes(&session_bytes),
        )
        .unwrap();
        let _ = coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("canvas").unwrap(),
                Vec::new(),
                drafts,
            )
            .unwrap();
        let event_count = events.ordered_events().unwrap().len();

        let error = store
            .record_canvas_feedback(&stale_session, request)
            .unwrap_err();

        assert!(error.to_string().contains("changed before capture"));
        assert_eq!(events.ordered_events().unwrap().len(), event_count);
    }

    #[test]
    fn preference_insight_rejects_a_response_identity_mismatch() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_store(root.path());
        let session = session();
        persist_session_with_response_events(root.path(), &session, &coordinator);
        let event_count = events.ordered_events().unwrap().len();
        let request = serde_json::from_value(serde_json::json!({
            "feedback_type": "insight",
            "response": {
                "tile_id": "tile-feedback",
                "model_id": "model-a"
            },
            "response_witness": {
                "response_id": "response-from-another-version",
                "response_content": "Answer A"
            },
            "kind": "preference",
            "content": "answers with concrete implementation details"
        }))
        .unwrap();

        let error = store.record_canvas_feedback(&session, request).unwrap_err();

        assert!(error.to_string().contains("changed before capture"));
        assert_eq!(events.ordered_events().unwrap().len(), event_count);
    }

    #[test]
    fn only_explicit_preference_insight_adds_a_canonical_claim() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_store(root.path());
        let session = session();
        persist_session_with_response_events(root.path(), &session, &coordinator);

        store
            .record_canvas_feedback(
                &session,
                CanvasFeedbackRequest {
                    feedback_type: CanvasFeedbackType::Accept,
                    response: Some(CanvasResponseRef {
                        tile_id: "tile-feedback".to_string(),
                        model_id: "model-a".to_string(),
                    }),
                    ..serde_json::from_value(serde_json::json!({
                        "feedback_type": "accept"
                    }))
                    .unwrap()
                },
            )
            .unwrap();
        store
            .record_canvas_feedback(
                &session,
                insight_request("model-a", UserRecordKind::Fact, "A user-authored fact"),
            )
            .unwrap();
        store
            .record_canvas_feedback(
                &session,
                insight_request(
                    "model-b",
                    UserRecordKind::ReasoningPattern,
                    "A user-authored reasoning pattern",
                ),
            )
            .unwrap();
        store
            .record_canvas_feedback(
                &session,
                insight_request(
                    "model-c",
                    UserRecordKind::Preference,
                    "  answers with concrete implementation details  ",
                ),
            )
            .unwrap();

        let captured = events.ordered_events().unwrap();
        let observations = captured
            .iter()
            .filter_map(|event| match &event.payload {
                TwinEventPayload::ObservationRecorded(observation) => Some(observation),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(observations.len(), 4);
        assert!(observations[..3]
            .iter()
            .all(|observation| observation.claims.is_empty()));
        assert_eq!(observations[3].claims.len(), 1);
        let claim = &observations[3].claims[0];
        assert_eq!(claim.subject_id.as_str(), "owner");
        assert_eq!(claim.predicate.as_str(), "prefers");
        assert_eq!(
            claim.object.as_str(),
            "answers with concrete implementation details"
        );
        assert_eq!(claim.polarity, ClaimPolarity::Affirmed);
        let source_response = captured
            .iter()
            .find(|event| {
                matches!(
                    &event.payload,
                    TwinEventPayload::CanvasResponseRecorded(response)
                        if response.response_id.as_str() == "response-c"
                )
            })
            .unwrap();
        let preference_observation = captured
            .iter()
            .find(|event| {
                matches!(
                    &event.payload,
                    TwinEventPayload::ObservationRecorded(observation)
                        if !observation.claims.is_empty()
                )
            })
            .unwrap();
        assert_eq!(
            preference_observation.governance,
            source_response.governance
        );
        assert!(preference_observation
            .causal_parents
            .contains(&source_response.event_id));
        assert!(preference_observation.evidence.iter().any(|evidence| {
            evidence.evidence_type == EvidenceType::Event
                && evidence.source_id.as_str() == source_response.event_id.as_str()
        }));
    }

    #[test]
    fn three_distinct_preference_insights_cross_the_canonical_proposal_threshold() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_store(root.path());
        let session = session();
        persist_session_with_response_events(root.path(), &session, &coordinator);
        let content = "answers with concrete implementation details";

        for (index, model_id) in ["model-a", "model-b", "model-c"].into_iter().enumerate() {
            let result = store
                .record_canvas_feedback(
                    &session,
                    insight_request(model_id, UserRecordKind::Preference, content),
                )
                .unwrap();
            let record = store
                .get_user_record(&result.created_record_ids[0])
                .unwrap();
            assert_eq!(record.evidence_refs.len(), 1);
            assert_eq!(record.evidence_refs[0].model_id.as_deref(), Some(model_id));

            let snapshot = project(&events.ordered_events().unwrap(), Utc::now()).unwrap();
            if index < 2 {
                assert!(snapshot.pending_proposals.is_empty());
            } else {
                assert_eq!(snapshot.pending_proposals.len(), 1);
                assert!(snapshot.reviewed_memories.is_empty());
                let proposal = &snapshot.pending_proposals[0];
                assert_eq!(proposal.support_count, 3);
                assert!(proposal.proposal_event_id.is_none());
                assert_eq!(proposal.claim.subject_id.as_str(), "owner");
                assert_eq!(proposal.claim.predicate.as_str(), "prefers");
                assert_eq!(proposal.claim.object.as_str(), content);
                assert_eq!(proposal.claim.polarity, ClaimPolarity::Affirmed);
            }
        }
    }

    #[test]
    fn preference_insight_cannot_count_the_same_response_twice() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_store(root.path());
        let session = session();
        persist_session_with_response_events(root.path(), &session, &coordinator);
        let request = insight_request(
            "model-a",
            UserRecordKind::Preference,
            "answers with concrete implementation details",
        );

        store
            .record_canvas_feedback(&session, request.clone())
            .unwrap();
        let event_count = events.ordered_events().unwrap().len();
        let error = store.record_canvas_feedback(&session, request).unwrap_err();

        assert!(error.to_string().contains("already captured"));
        assert_eq!(events.ordered_events().unwrap().len(), event_count);
    }

    #[test]
    fn preference_insight_cannot_reuse_regenerations_of_the_same_response_identity() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_store(root.path());
        let mut current = session();
        persist_session_with_response_events(root.path(), &current, &coordinator);
        let request = insight_request(
            "model-a",
            UserRecordKind::Preference,
            "answers with concrete implementation details",
        );
        store
            .record_canvas_feedback(&current, request.clone())
            .unwrap();

        for regenerated_content in ["Regenerated answer A", "Regenerated answer B"] {
            let before = current.clone();
            current.prompt_tiles[0]
                .responses
                .get_mut("model-a")
                .unwrap()
                .content = regenerated_content.to_string();
            persist_session(root.path(), &current);
            let persisted_events = events.ordered_events().unwrap();
            let session_bytes = serde_json::to_vec_pretty(&current).unwrap();
            let drafts = canvas_transition_drafts(
                Some(&before),
                &current,
                &persisted_events,
                digest_bytes(&session_bytes),
            )
            .unwrap();
            let _ = coordinator
                .commit_local(
                    CausalStream::SyncEligible,
                    SourceChannel::parse("canvas").unwrap(),
                    Vec::new(),
                    drafts,
                )
                .unwrap();

            let mut regenerated_request = request.clone();
            regenerated_request
                .response_witness
                .as_mut()
                .unwrap()
                .response_content = regenerated_content.to_string();
            let error = store
                .record_canvas_feedback(&current, regenerated_request)
                .unwrap_err();
            assert!(error.to_string().contains("already captured"));
        }

        let snapshot = project(&events.ordered_events().unwrap(), Utc::now()).unwrap();
        assert!(snapshot.pending_proposals.is_empty());
        assert_eq!(
            snapshot
                .recent_observations
                .iter()
                .filter(|item| item.claim.object.as_str()
                    == "answers with concrete implementation details")
                .count(),
            1
        );
    }

    #[test]
    fn preference_insight_fails_closed_for_ambiguous_active_response_events() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_store(root.path());
        let session = session();
        persist_session_with_response_events(root.path(), &session, &coordinator);

        let session_bytes = serde_json::to_vec_pretty(&session).unwrap();
        let duplicate_drafts =
            canvas_transition_drafts(None, &session, &[], digest_bytes(&session_bytes)).unwrap();
        let _ = coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("canvas").unwrap(),
                Vec::new(),
                duplicate_drafts,
            )
            .unwrap();
        let event_count = events.ordered_events().unwrap().len();

        let error = store
            .record_canvas_feedback(
                &session,
                insight_request(
                    "model-a",
                    UserRecordKind::Preference,
                    "answers with concrete implementation details",
                ),
            )
            .unwrap_err();

        assert!(error.to_string().contains("ambiguous"));
        assert_eq!(events.ordered_events().unwrap().len(), event_count);
    }

    #[test]
    fn preference_insight_rejects_relationship_scoped_response_until_scope_is_proven() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_store(root.path());
        let mut session = session();
        session.prompt_tiles[0].twin_relationship_variant =
            RelationshipVariant::new(vec![RelationshipKey {
                subject_id: EntityId::parse("owner").unwrap(),
                predicate: RelationshipPredicate::parse("works_with").unwrap(),
                object_id: EntityId::parse("alex").unwrap(),
                direction: RelationshipDirection::Directed,
            }]);
        persist_session_with_response_events(root.path(), &session, &coordinator);
        let event_count = events.ordered_events().unwrap().len();

        let error = store
            .record_canvas_feedback(
                &session,
                insight_request(
                    "model-a",
                    UserRecordKind::Preference,
                    "answers with concrete implementation details",
                ),
            )
            .unwrap_err();

        assert!(error.to_string().contains("global Canvas response"));
        assert_eq!(events.ordered_events().unwrap().len(), event_count);
    }
}
