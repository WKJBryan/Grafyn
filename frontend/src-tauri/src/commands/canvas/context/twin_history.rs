use super::*;
use crate::models::canvas::TwinEvidenceSnapshot;
use crate::models::twin::{PromotionState, UserRecordKind};
use crate::models::twin_event::{
    AuthorityClass, ClaimPolarity, ContentDigest, EventId, ReviewState,
};
use crate::models::twin_state::{
    AttentionCandidate, AttentionProfile, AttentionRequest, ProjectedItemKind, ProjectedStateItem,
    RelationshipVariant, SelectionDestination,
};
use crate::services::twin_events::{project, rank};
use chrono::Utc;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const TWIN_HISTORY_CONTEXT_VERSION: &str = "ctx-v3-reviewed-projection-history";
const MAX_TWIN_HISTORY_MEMORIES: u16 = 24;
const COMPACT_HISTORY_RECENT_TURNS: usize = 2;
const COMPACT_HISTORY_EXCERPT_CHARS: usize = 240;
const MAX_PROMPT_CONTEXT_CANONICAL_BYTES: usize = 1_048_576;

#[derive(Serialize)]
struct CanonicalPromptContext<'a> {
    schema: &'static str,
    messages: Vec<CanonicalPromptMessage<'a>>,
    system_prompt: &'a str,
    context_version: &'a str,
}

#[derive(Serialize)]
struct CanonicalPromptMessage<'a> {
    role: &'a str,
    content: &'a str,
}

pub(super) fn build_compact_history_messages(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ChatMessage>, String> {
    let turns = build_selected_parent_chain(session, request)?;
    let mut messages = Vec::new();
    if turns.len() > COMPACT_HISTORY_RECENT_TURNS {
        let split_at = turns.len() - COMPACT_HISTORY_RECENT_TURNS;
        messages.push(ChatMessage {
            role: "user".into(),
            content: build_compact_history_summary(&turns[..split_at]),
        });
        for turn in &turns[split_at..] {
            messages.push(ChatMessage {
                role: "user".into(),
                content: turn.prompt.clone(),
            });
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: turn.response.clone(),
            });
        }
    } else {
        for turn in turns {
            messages.push(ChatMessage {
                role: "user".into(),
                content: turn.prompt,
            });
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: turn.response,
            });
        }
    }
    messages.push(ChatMessage {
        role: "user".into(),
        content: request.prompt.clone(),
    });
    Ok(messages)
}

fn build_compact_history_summary(turns: &[ConversationTurn]) -> String {
    let mut summary = String::from("Conversation summary before the most recent turns:\n");
    for (index, turn) in turns.iter().enumerate() {
        summary.push_str(&format!(
            "\nTurn {}:\nUser: {}\nAssistant ({}): {}\n",
            index + 1,
            truncate_for_compact_history(&turn.prompt),
            turn.model_id,
            truncate_for_compact_history(&turn.response),
        ));
    }
    summary
}

fn truncate_for_compact_history(content: &str) -> String {
    if content.chars().count() <= COMPACT_HISTORY_EXCERPT_CHARS {
        return content.to_string();
    }
    let mut truncated = content
        .chars()
        .take(COMPACT_HISTORY_EXCERPT_CHARS)
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

pub(super) async fn resolve_twin_history_prompt_context(
    state: &AppState,
    messages: Vec<ChatMessage>,
    session: &CanvasSession,
    request: &PromptRequest,
    replay_tile: Option<&PromptTile>,
    destination: SelectionDestination,
) -> Result<ResolvedPromptContext, String> {
    resolve_twin_history_prompt_context_inner(
        state,
        messages,
        session,
        request,
        replay_tile,
        destination,
    )
    .await
}

async fn resolve_twin_history_prompt_context_inner(
    state: &AppState,
    messages: Vec<ChatMessage>,
    session: &CanvasSession,
    request: &PromptRequest,
    replay_tile: Option<&PromptTile>,
    destination: SelectionDestination,
) -> Result<ResolvedPromptContext, String> {
    let persisted_evidence = replay_tile.and_then(|tile| tile.twin_evidence_snapshot.as_ref());
    if replay_tile.is_some() && persisted_evidence.is_none() {
        return Err("Twin History replay requires its persisted evidence snapshot".into());
    }
    if let Some(metadata) = persisted_evidence {
        metadata.validate()?;
    }

    let reference_time = persisted_evidence
        .map(|metadata| metadata.reference_time)
        .unwrap_or_else(Utc::now);
    let events = state
        .twin_event_store
        .ordered_events()
        .map_err(|error| error.to_string())?;
    let snapshot = project(&events, reference_time).map_err(|error| error.to_string())?;
    if persisted_evidence
        .is_some_and(|metadata| metadata.projection_snapshot_id != snapshot.snapshot_id)
    {
        return Err("Twin History evidence snapshot can no longer be reproduced".into());
    }

    let selected_items = select_reviewed_projection_memory(
        &snapshot,
        &request.prompt,
        &request.twin_answer_mode,
        destination,
    )?;
    let records = selected_items
        .iter()
        .map(projected_record)
        .collect::<Vec<_>>();

    let (available_notes, note_contexts) = if let Some(tile) = replay_tile {
        let notes = tile.context_notes.clone();
        let contexts = notes
            .iter()
            .map(|note| (note.id.clone(), note.title.clone(), note.snippet.clone()))
            .collect();
        (notes, contexts)
    } else {
        collect_reproducible_notes(state, session, request).await
    };

    let selection = apply_twin_context_budget(
        Vec::new(),
        Vec::new(),
        records,
        Vec::new(),
        Vec::new(),
        note_contexts,
        TWIN_CONTEXT_TOKEN_BUDGET,
    );
    let selected_by_id = selected_items
        .into_iter()
        .map(|item| (item.item_id.to_string(), item))
        .collect::<BTreeMap<_, _>>();
    let used_record_ids = selection
        .approved
        .iter()
        .map(|record| record.id.clone())
        .collect::<Vec<_>>();
    let used_items = used_record_ids
        .iter()
        .map(|id| {
            selected_by_id
                .get(id)
                .cloned()
                .ok_or_else(|| "Twin History selected an unknown projected memory".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;

    let used_note_ids = selection
        .notes
        .iter()
        .map(|(id, _, _)| id.clone())
        .collect::<BTreeSet<_>>();
    let context_notes = available_notes
        .into_iter()
        .filter(|note| used_note_ids.contains(&note.id))
        .collect::<Vec<_>>();
    if context_notes.len() != used_note_ids.len() {
        return Err("Twin History note evidence must be unique and complete".into());
    }
    let setup = {
        let twin_store = state.twin_store.write().await;
        twin_store
            .get_constitution_setup()
            .map_err(|error| error.to_string())?
    };
    validate_twin_identity_for_answer_mode(&setup, &request.twin_answer_mode)?;
    let twin_prompt = build_twin_context_prompt(
        &setup,
        &[],
        &selection.notes,
        &selection.approved,
        &[],
        &[],
        &[],
        &request.twin_answer_mode,
        &request.prompt_type,
        request.decision_metadata.as_ref(),
    );
    let system_prompt = match &request.system_prompt {
        Some(user_prompt) if !user_prompt.is_empty() => {
            format!("{twin_prompt}\n\n{user_prompt}")
        }
        _ => twin_prompt.clone(),
    };
    let context_digest =
        prompt_context_digest(&messages, &system_prompt, TWIN_HISTORY_CONTEXT_VERSION)?;
    validate_replay_prompt_context_digest(replay_tile, &context_digest)?;
    let evidence = evidence_snapshot(&snapshot, &used_items, &context_notes, context_digest)?;

    if let Some(tile) = replay_tile {
        let persisted_record_ids = tile
            .approved_twin_records
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        if persisted_record_ids != used_record_ids
            || !tile.candidate_twin_records.is_empty()
            || tile.twin_evidence_snapshot.as_ref() != Some(&evidence)
        {
            return Err(
                "Twin History persisted context does not match its projection evidence".into(),
            );
        }
    }

    Ok(ResolvedPromptContext {
        messages,
        context_notes,
        approved_twin_records: selection.approved,
        candidate_twin_records: Vec::new(),
        constitution_items: Vec::new(),
        action_gaps: Vec::new(),
        system_prompt: Some(system_prompt),
        twin_context_prompt: Some(twin_prompt),
        context_version: Some(TWIN_HISTORY_CONTEXT_VERSION.to_string()),
        decision_case_ids: Vec::new(),
        twin_evidence_snapshot: Some(evidence),
    })
}

async fn collect_reproducible_notes(
    state: &AppState,
    session: &CanvasSession,
    request: &PromptRequest,
) -> (Vec<TileContextNote>, Vec<(String, String, String)>) {
    let pinned_ids = session.pinned_note_ids.clone();
    let results = run_retrieval(state, &request.prompt, 5, &pinned_ids)
        .await
        .unwrap_or_default();
    if should_use_retrieved_notes(&request.prompt, &results)
        != RetrievalDecisionReason::UseRetrievedNotes
    {
        return (Vec::new(), Vec::new());
    }

    let fetched = fetch_note_contexts(state, &results).await;
    let score_by_id = results
        .iter()
        .map(|result| (result.note.id.as_str(), result.score))
        .collect::<BTreeMap<_, _>>();
    let notes = fetched
        .into_iter()
        .map(|(id, title, content)| TileContextNote {
            score: score_by_id.get(id.as_str()).copied().unwrap_or_default(),
            pinned: pinned_ids.contains(&id),
            id,
            title,
            snippet: content,
        })
        .collect::<Vec<_>>();
    let contexts = notes
        .iter()
        .map(|note| (note.id.clone(), note.title.clone(), note.snippet.clone()))
        .collect();
    (notes, contexts)
}

fn select_reviewed_projection_memory(
    snapshot: &crate::models::twin_state::ProjectionSnapshot,
    query: &str,
    answer_mode: &TwinAnswerMode,
    destination: SelectionDestination,
) -> Result<Vec<ProjectedStateItem>, String> {
    let candidates = snapshot
        .reviewed_memories
        .iter()
        .filter(|item| {
            item.kind == ProjectedItemKind::ReviewedMemory
                && item.governance.review == ReviewState::Accepted
                && matches!(
                    item.governance.authority,
                    AuthorityClass::ReviewedMemory
                        | AuthorityClass::CanonicalUserRule
                        | AuthorityClass::DeterministicallyVerified { .. }
                )
        })
        .cloned()
        .map(|item| AttentionCandidate { item })
        .collect::<Vec<_>>();
    let profile = match answer_mode {
        TwinAnswerMode::Advisor => AttentionProfile::Decision,
        TwinAnswerMode::Simulation => AttentionProfile::Simulation,
    };
    let trace = rank(
        snapshot.snapshot_id.clone(),
        &candidates,
        profile,
        &AttentionRequest {
            query: query.to_string(),
            relationship_variant: RelationshipVariant::global(),
            goals: Vec::new(),
            reference_time: snapshot.reference_time,
            destination,
            limit: MAX_TWIN_HISTORY_MEMORIES,
        },
    )
    .map_err(|error| error.to_string())?;
    let by_id = candidates
        .into_iter()
        .map(|candidate| (candidate.item.item_id.clone(), candidate.item))
        .collect::<BTreeMap<_, _>>();
    trace
        .selected
        .into_iter()
        .map(|selected| {
            by_id
                .get(&selected.item_id)
                .cloned()
                .ok_or_else(|| "Twin attention selected an unknown memory".to_string())
        })
        .collect()
}

fn projected_record(item: &ProjectedStateItem) -> TwinContextRecord {
    let predicate = item.claim.predicate.as_str();
    let kind = if predicate.contains("prefer") {
        UserRecordKind::Preference
    } else if predicate.contains("reason") {
        UserRecordKind::ReasoningPattern
    } else {
        UserRecordKind::Fact
    };
    let relation = match item.claim.polarity {
        ClaimPolarity::Affirmed => predicate.to_string(),
        ClaimPolarity::Denied => format!("does not {predicate}"),
    };
    let content = format!(
        "{} {} {}",
        item.claim.subject_id.as_str(),
        relation,
        item.claim.object.as_str()
    );
    let total = u32::from(item.support_count) + u32::from(item.opposition_count);
    let confidence = if total == 0 {
        0.0
    } else {
        f32::from(item.support_count) / total as f32
    };
    TwinContextRecord {
        id: item.item_id.to_string(),
        kind,
        content,
        confidence,
        promotion_state: PromotionState::Endorsed,
        evidence_count: item.evidence_event_ids.len(),
        source_label: Some("governed_projection".into()),
    }
}

fn evidence_snapshot(
    snapshot: &crate::models::twin_state::ProjectionSnapshot,
    items: &[ProjectedStateItem],
    notes: &[TileContextNote],
    prompt_context_digest: ContentDigest,
) -> Result<TwinEvidenceSnapshot, String> {
    let mut event_ids = BTreeSet::<EventId>::new();
    for item in items {
        event_ids.extend(item.evidence_event_ids.iter().cloned());
        event_ids.extend(item.review_event_ids.iter().cloned());
        if let Some(proposal_id) = &item.proposal_event_id {
            event_ids.insert(proposal_id.clone());
        }
    }
    let metadata = TwinEvidenceSnapshot {
        projection_snapshot_id: snapshot.snapshot_id.clone(),
        reference_time: snapshot.reference_time,
        prompt_context_digest,
        evidence_event_ids: event_ids.into_iter().collect(),
        note_ids: notes
            .iter()
            .map(|note| note.id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    };
    metadata.validate()?;
    Ok(metadata)
}

fn prompt_context_digest(
    messages: &[ChatMessage],
    system_prompt: &str,
    context_version: &str,
) -> Result<ContentDigest, String> {
    let canonical = CanonicalPromptContext {
        schema: "grafyn.twin_history.prompt_context.v1",
        messages: messages
            .iter()
            .map(|message| CanonicalPromptMessage {
                role: &message.role,
                content: &message.content,
            })
            .collect(),
        system_prompt,
        context_version,
    };
    let bytes = serde_json::to_vec(&canonical).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_PROMPT_CONTEXT_CANONICAL_BYTES {
        return Err("Twin History prompt context exceeds its bounded digest input".into());
    }
    Ok(crate::services::twin_events::digest_bytes(&bytes))
}

fn validate_replay_prompt_context_digest(
    replay_tile: Option<&PromptTile>,
    actual: &ContentDigest,
) -> Result<(), String> {
    let Some(tile) = replay_tile else {
        return Ok(());
    };
    let Some(expected) = tile.twin_evidence_snapshot.as_ref() else {
        return Err("Twin History replay requires its persisted evidence snapshot".into());
    };
    if &expected.prompt_context_digest != actual {
        return Err("Twin History persisted prompt context can no longer be reproduced".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::canvas::test_support::build_tile;
    use crate::models::canvas::CanvasViewport;
    use crate::models::twin_event::{
        Identifier, MemoryProposed, MemoryReviewDecision, MemoryReviewed, ProvenanceLabel,
        TwinEvent, TwinEventPayload, TwinEventType,
    };
    use crate::services::twin_events::{derive_event_id, test_support::valid_event_for_device};
    use chrono::{Duration, TimeZone};

    fn reference() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 29, 3, 0, 0).unwrap()
    }

    fn history_request(parent: Option<&str>) -> PromptRequest {
        PromptRequest {
            prompt: "Newest prompt".into(),
            prompt_type: PromptType::Standard,
            system_prompt: None,
            models: vec!["openai/gpt-4".into()],
            position: None,
            context_mode: ContextMode::TwinHistory,
            twin_answer_mode: TwinAnswerMode::Advisor,
            twin_context_policy: None,
            twin_llm_provider: None,
            decision_metadata: None,
            parent_tile_id: parent.map(str::to_string),
            parent_model_id: parent.map(|_| "openai/gpt-4".to_string()),
            temperature: 0.7,
            max_tokens: None,
            web_search: false,
            web_search_max_results: 5,
            reasoning_effort: "none".into(),
        }
    }

    fn history_session() -> CanvasSession {
        CanvasSession {
            id: "session-1".into(),
            title: "Twin history".into(),
            description: None,
            prompt_tiles: vec![
                build_tile(
                    "root",
                    "Root prompt",
                    "openai/gpt-4",
                    "Root response",
                    None,
                    None,
                ),
                build_tile(
                    "leaf",
                    "Leaf prompt",
                    "openai/gpt-4",
                    "Leaf response",
                    Some("root"),
                    Some("openai/gpt-4"),
                ),
            ],
            debates: Vec::new(),
            viewport: CanvasViewport::default(),
            created_at: reference(),
            updated_at: reference(),
            tags: Vec::new(),
            status: "draft".into(),
            pinned_note_ids: Vec::new(),
        }
    }

    #[test]
    fn twin_history_messages_are_compact_and_root_to_leaf() {
        let messages =
            build_canvas_messages(&history_session(), &history_request(Some("leaf"))).unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Root prompt",
                "Root response",
                "Leaf prompt",
                "Leaf response",
                "Newest prompt"
            ]
        );
    }

    #[test]
    fn root_twin_history_prompt_does_not_leak_unrelated_tiles() {
        let messages = build_canvas_messages(&history_session(), &history_request(None)).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Newest prompt");
    }

    fn proposal(memory_id: &str) -> TwinEvent {
        let mut event = valid_event_for_device("proposal-device", 1, Vec::new());
        let claim = match &event.payload {
            TwinEventPayload::ObservationRecorded(payload) => payload.claims[0].clone(),
            _ => unreachable!(),
        };
        event.recorded_at = reference() - Duration::minutes(2);
        event.event_type = TwinEventType::MemoryProposed;
        event.payload = TwinEventPayload::MemoryProposed(MemoryProposed {
            memory_id: Identifier::parse(memory_id).unwrap(),
            claim,
            summary: None,
            proposal_source: ProvenanceLabel::parse("canvas-test").unwrap(),
        });
        event.governance.review = ReviewState::Pending;
        event.event_id = derive_event_id(&event);
        event
    }

    fn review(proposal: &TwinEvent, decision: MemoryReviewDecision) -> TwinEvent {
        let mut event = valid_event_for_device("review-device", 1, vec![proposal.event_id.clone()]);
        event.recorded_at = reference() - Duration::minutes(1);
        event.event_type = TwinEventType::MemoryReviewed;
        event.payload = TwinEventPayload::MemoryReviewed(MemoryReviewed {
            memory_id: Identifier::parse("memory-one").unwrap(),
            decision: decision.clone(),
            reviewed_claim: None,
            rationale: None,
        });
        event.governance.review = match decision {
            MemoryReviewDecision::Accept => ReviewState::Accepted,
            MemoryReviewDecision::Reject => ReviewState::Rejected,
            MemoryReviewDecision::Supersede => ReviewState::Superseded,
        };
        event.governance.authority = AuthorityClass::ReviewedMemory;
        event.event_id = derive_event_id(&event);
        event
    }

    #[test]
    fn projection_selection_uses_only_accepted_reviewed_memory() {
        let proposal = proposal("memory-one");
        let accepted = review(&proposal, MemoryReviewDecision::Accept);
        let accepted_snapshot = project(&[proposal.clone(), accepted], reference()).unwrap();
        let selected = select_reviewed_projection_memory(
            &accepted_snapshot,
            "quiet work",
            &TwinAnswerMode::Advisor,
            SelectionDestination::Local,
        )
        .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].item_id.as_str(), "memory-one");

        let rejected = review(&proposal, MemoryReviewDecision::Reject);
        let rejected_snapshot = project(&[proposal.clone(), rejected], reference()).unwrap();
        assert!(select_reviewed_projection_memory(
            &rejected_snapshot,
            "quiet work",
            &TwinAnswerMode::Advisor,
            SelectionDestination::Local,
        )
        .unwrap()
        .is_empty());

        let pending_snapshot = project(&[proposal], reference()).unwrap();
        assert!(select_reviewed_projection_memory(
            &pending_snapshot,
            "quiet work",
            &TwinAnswerMode::Advisor,
            SelectionDestination::Local,
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn evidence_snapshot_is_sorted_unique_and_includes_review_chain() {
        let proposal = proposal("memory-one");
        let accepted = review(&proposal, MemoryReviewDecision::Accept);
        let snapshot = project(&[accepted, proposal], reference()).unwrap();
        let metadata = evidence_snapshot(
            &snapshot,
            &snapshot.reviewed_memories,
            &[],
            crate::services::twin_events::digest_bytes(b"prompt-context"),
        )
        .unwrap();

        assert!(metadata.validate().is_ok());
        assert_eq!(metadata.evidence_event_ids.len(), 2);
        assert!(metadata
            .evidence_event_ids
            .windows(2)
            .all(|pair| pair[0] < pair[1]));
    }

    fn replay_tile_with_digest(digest: crate::models::twin_event::ContentDigest) -> PromptTile {
        PromptTile {
            twin_evidence_snapshot: Some(TwinEvidenceSnapshot {
                projection_snapshot_id: crate::models::twin_state::SnapshotId::parse(
                    "a".repeat(64),
                )
                .unwrap(),
                reference_time: reference(),
                prompt_context_digest: digest,
                evidence_event_ids: Vec::new(),
                note_ids: Vec::new(),
            }),
            ..PromptTile::default()
        }
    }

    fn digest_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage {
                role: "user".into(),
                content: "Earlier".into(),
            },
            ChatMessage {
                role: "assistant".into(),
                content: "Answer".into(),
            },
            ChatMessage {
                role: "user".into(),
                content: "Now".into(),
            },
        ]
    }

    #[test]
    fn prompt_context_digest_is_canonical_and_preserves_message_order() {
        let digest =
            prompt_context_digest(&digest_messages(), "System", TWIN_HISTORY_CONTEXT_VERSION)
                .unwrap();

        assert_eq!(
            digest.as_str(),
            "cc07737bf0b44e5c7dd61c5cbad7dd67504006f1c2e5d6c85bae3b1204a21580"
        );

        let mut reordered = digest_messages();
        reordered.swap(0, 1);
        assert_ne!(
            prompt_context_digest(&reordered, "System", TWIN_HISTORY_CONTEXT_VERSION).unwrap(),
            digest
        );
    }

    #[test]
    fn replay_rejects_an_ancestor_history_edit_with_unchanged_evidence_ids() {
        let original =
            prompt_context_digest(&digest_messages(), "System", TWIN_HISTORY_CONTEXT_VERSION)
                .unwrap();
        let tile = replay_tile_with_digest(original);
        let mut changed_messages = digest_messages();
        changed_messages[1].content = "Regenerated answer".into();
        let changed =
            prompt_context_digest(&changed_messages, "System", TWIN_HISTORY_CONTEXT_VERSION)
                .unwrap();

        let error = validate_replay_prompt_context_digest(Some(&tile), &changed).unwrap_err();

        assert!(error.contains("prompt context"));
    }

    #[test]
    fn replay_rejects_a_constitution_setup_change_with_unchanged_evidence_ids() {
        let original = prompt_context_digest(
            &digest_messages(),
            "Original Twin operating contract",
            TWIN_HISTORY_CONTEXT_VERSION,
        )
        .unwrap();
        let tile = replay_tile_with_digest(original);
        let changed = prompt_context_digest(
            &digest_messages(),
            "Changed Twin operating contract",
            TWIN_HISTORY_CONTEXT_VERSION,
        )
        .unwrap();

        let error = validate_replay_prompt_context_digest(Some(&tile), &changed).unwrap_err();

        assert!(error.contains("prompt context"));
    }
}
