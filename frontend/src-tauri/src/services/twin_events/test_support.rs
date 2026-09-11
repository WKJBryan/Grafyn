use crate::models::twin_event::*;
use chrono::{TimeZone, Utc};

pub fn valid_event(sequence: u64, parents: Vec<EventId>) -> TwinEvent {
    valid_event_for_device("device-a", sequence, parents)
}

pub fn valid_event_for_device(device: &str, sequence: u64, parents: Vec<EventId>) -> TwinEvent {
    valid_event_for_device_and_stream(device, CausalStream::LocalOnly, sequence, parents)
}

pub fn valid_event_for_device_and_stream(
    device: &str,
    causal_stream: CausalStream,
    sequence: u64,
    parents: Vec<EventId>,
) -> TwinEvent {
    let mut event = TwinEvent {
        schema_version: 1,
        event_id: EventId::parse(
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap(),
        event_type: TwinEventType::ObservationRecorded,
        actor_id: ActorId::parse("owner").unwrap(),
        device_id: DeviceId::parse(device).unwrap(),
        causal_stream,
        device_sequence: sequence,
        causal_parents: parents,
        recorded_at: Utc
            .with_ymd_and_hms(2026, 8, 29, 1, sequence as u32, 0)
            .unwrap(),
        observed_at: Utc.with_ymd_and_hms(2026, 8, 29, 1, 0, 0).unwrap(),
        occurred_at: None,
        valid_from: None,
        valid_to: None,
        supersedes: Vec::new(),
        reinforces: Vec::new(),
        context: EventContext::default(),
        evidence: Vec::new(),
        governance: Governance::direct_observation(),
        payload: TwinEventPayload::ObservationRecorded(ObservationRecorded {
            observation_id: Identifier::parse(format!("observation-{device}-{sequence}")).unwrap(),
            claims: vec![ClaimAssertion {
                subject_id: EntityId::parse("owner").unwrap(),
                predicate: ClaimPredicate::parse("prefers").unwrap(),
                object: ClaimObject::parse("quiet work").unwrap(),
                polarity: ClaimPolarity::Affirmed,
            }],
            summary: None,
            content_digest: None,
        }),
    };
    event.event_id = crate::services::twin_events::derive_event_id(&event);
    event
}

pub fn event_for_payload(payload: TwinEventPayload) -> TwinEvent {
    let mut event = valid_event(1, Vec::new());
    event.event_type = payload.event_type();
    event.payload = payload;
    event.event_id = crate::services::twin_events::derive_event_id(&event);
    event
}

pub fn all_payloads() -> Vec<TwinEventPayload> {
    let id = || Identifier::parse("id-1").unwrap();
    let digest = || {
        ContentDigest::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .unwrap()
    };
    vec![
        TwinEventPayload::ObservationRecorded(ObservationRecorded {
            observation_id: id(),
            claims: vec![ClaimAssertion {
                subject_id: EntityId::parse("owner").unwrap(),
                predicate: ClaimPredicate::parse("prefers").unwrap(),
                object: ClaimObject::parse("quiet work").unwrap(),
                polarity: ClaimPolarity::Affirmed,
            }],
            summary: None,
            content_digest: None,
        }),
        TwinEventPayload::NoteChanged(NoteChanged {
            note_id: id(),
            change: NoteChangeKind::Created,
            content_digest: None,
        }),
        TwinEventPayload::ConversationTurnRecorded(ConversationTurnRecorded {
            conversation_id: id(),
            turn_id: id(),
            role: BoundedRole::parse("user").unwrap(),
            content: BoundedContent::parse("conversation content").unwrap(),
            model_id: None,
            provenance: None,
            content_digest: Some(digest()),
        }),
        TwinEventPayload::CanvasResponseRecorded(CanvasResponseRecorded {
            session_id: id(),
            tile_id: id(),
            response_id: id(),
            prompt: BoundedContent::parse("prompt text").unwrap(),
            response: BoundedContent::parse("response text").unwrap(),
            model_id: ModelId::parse("anthropic/claude-3.5-haiku").unwrap(),
            provider: None,
            provenance: None,
            tokens_used: None,
            cost_usd_decimal: None,
            prompt_digest: None,
            response_digest: Some(digest()),
        }),
        TwinEventPayload::MemoryProposed(MemoryProposed {
            memory_id: id(),
            claim: ClaimAssertion {
                subject_id: EntityId::parse("owner").unwrap(),
                predicate: ClaimPredicate::parse("prefers").unwrap(),
                object: ClaimObject::parse("quiet work").unwrap(),
                polarity: ClaimPolarity::Affirmed,
            },
            summary: None,
            proposal_source: ProvenanceLabel::parse("rule").unwrap(),
        }),
        TwinEventPayload::MemoryReviewed(MemoryReviewed {
            memory_id: id(),
            decision: MemoryReviewDecision::Accept,
            reviewed_claim: None,
            rationale: None,
        }),
        TwinEventPayload::DecisionRecorded(DecisionRecorded {
            decision_id: id(),
            decision: BoundedContent::parse("choose").unwrap(),
            options: vec![BoundedContent::parse("a").unwrap()],
            stakes: Some(BoundedContent::parse("direction").unwrap()),
            initial_leaning: None,
            review_date: None,
            primitive_assessment: PrimitiveDecisionAssessmentPayload::default(),
        }),
        TwinEventPayload::DecisionOutcomeRecorded(DecisionOutcomeRecorded {
            decision_id: id(),
            outcome: Some(BoundedContent::parse("done").unwrap()),
            chosen_option: None,
            selected_response_id: None,
            confidence_basis_points: None,
            review_date: None,
            correction_note: None,
            regret_score: None,
            lesson: Some(BoundedContent::parse("iterate").unwrap()),
            missed_something: None,
            primitive_assessment: None,
        }),
        TwinEventPayload::FeedbackRecorded(FeedbackRecorded {
            feedback_id: id(),
            target_id: id(),
            kind: BoundedRole::parse("accept").unwrap(),
            content: Some(BoundedContent::parse("feedback content").unwrap()),
            rationale: None,
            rank: None,
        }),
        TwinEventPayload::RelationshipContextObserved(RelationshipContextObserved {}),
    ]
}
