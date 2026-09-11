use super::*;
use crate::models::twin_event::*;
use crate::services::twin_events::{derive_event_id, test_support::valid_event_for_device};
use chrono::{Duration, TimeZone};

fn reference() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap()
}

#[test]
fn legacy_v2_snapshot_identity_omits_typed_timeline_context() {
    let snapshot = project(&[], reference()).unwrap();
    let body = String::from_utf8(legacy_v2_snapshot_body_json(&snapshot).unwrap()).unwrap();

    assert_eq!(
            body,
            "{\"schema_version\":1,\"projection_version\":2,\"attention_profile_version\":1,\"reference_time\":\"2026-09-01T00:00:00Z\",\"applied_event_ids\":[],\"reviewed_memories\":[],\"pending_proposals\":[],\"relationship_variants\":[],\"timeline\":[],\"contradiction_clusters\":[],\"recent_observations\":[]}"
        );
    let legacy = legacy_v2_snapshot_id(&snapshot).unwrap();
    assert_ne!(legacy, snapshot.snapshot_id);
    assert_eq!(legacy, legacy_v2_snapshot_id(&snapshot).unwrap());
}

fn contextual_observation(device: &str) -> TwinEvent {
    let mut event = valid_event_for_device(device, 1, Vec::new());
    event.context.relationships = vec![RelationshipAssertion {
        subject_id: EntityId::parse("owner").unwrap(),
        predicate: RelationshipPredicate::parse("works_with").unwrap(),
        object_id: EntityId::parse("manager").unwrap(),
        direction: RelationshipDirection::Directed,
        valid_from: None,
        valid_to: None,
        evidence: Vec::new(),
        governance: Governance::direct_observation(),
    }];
    event.event_id = derive_event_id(&event);
    event
}

fn companion_capture_observation(device: &str, note_id: &str) -> TwinEvent {
    let mut event = contextual_observation(device);
    let digest = ContentDigest::parse("d".repeat(64)).unwrap();
    event.context.source_channel = SourceChannel::parse("companion_capture").unwrap();
    event.context.goals = vec!["Ship Grafyn".into()];
    event.evidence = vec![EvidenceRef {
        evidence_type: EvidenceType::Note,
        source_id: Identifier::parse(note_id).unwrap(),
        digest: Some(digest.clone()),
    }];
    event.payload = TwinEventPayload::ObservationRecorded(ObservationRecorded {
        observation_id: Identifier::parse(format!("companion-capture-{note_id}")).unwrap(),
        claims: Vec::new(),
        summary: Some(BoundedSummary::parse("Captured preference").unwrap()),
        content_digest: Some(digest),
    });
    event.event_id = derive_event_id(&event);
    event
}

#[derive(Debug, Clone, Copy)]
enum InapplicableRelationshipCase {
    ExpiredLocalOnly,
    RejectedRestricted,
    SupersededSyncDisabled,
    FutureSensitiveSyncDisabled,
}

fn apply_inapplicable_relationship_case(event: &mut TwinEvent, case: InapplicableRelationshipCase) {
    let relationship = &mut event.context.relationships[0];
    relationship.governance.allowed_uses.recall = false;
    relationship.governance.allowed_uses.export = false;
    relationship.governance.allowed_uses.sync = false;
    match case {
        InapplicableRelationshipCase::ExpiredLocalOnly => {
            relationship.valid_to = Some(reference() - Duration::seconds(1));
            relationship.governance.visibility = Visibility::LocalOnly;
        }
        InapplicableRelationshipCase::RejectedRestricted => {
            relationship.governance.review = ReviewState::Rejected;
            relationship.governance.sensitivity = Sensitivity::Restricted;
        }
        InapplicableRelationshipCase::SupersededSyncDisabled => {
            relationship.governance.review = ReviewState::Superseded;
        }
        InapplicableRelationshipCase::FutureSensitiveSyncDisabled => {
            relationship.valid_from = Some(reference() + Duration::seconds(1));
            relationship.governance.sensitivity = Sensitivity::Sensitive;
        }
    }
    event.event_id = derive_event_id(event);
}

fn proposal(device: &str, memory: &str) -> TwinEvent {
    let mut event = valid_event_for_device(device, 1, Vec::new());
    let claim = match &event.payload {
        TwinEventPayload::ObservationRecorded(value) => value.claims[0].clone(),
        _ => unreachable!(),
    };
    event.payload = TwinEventPayload::MemoryProposed(MemoryProposed {
        memory_id: Identifier::parse(memory).unwrap(),
        claim,
        summary: Some(BoundedSummary::parse("quiet-work preference").unwrap()),
        proposal_source: ProvenanceLabel::parse("exact-rule-v1").unwrap(),
    });
    event.event_type = TwinEventType::MemoryProposed;
    event.governance.review = ReviewState::Pending;
    event.event_id = derive_event_id(&event);
    event
}

fn review(
    device: &str,
    memory: &str,
    decision: MemoryReviewDecision,
    parents: Vec<EventId>,
) -> TwinEvent {
    let mut event = valid_event_for_device(device, 1, parents);
    event.payload = TwinEventPayload::MemoryReviewed(MemoryReviewed {
        memory_id: Identifier::parse(memory).unwrap(),
        decision: decision.clone(),
        reviewed_claim: None,
        rationale: None,
    });
    event.event_type = TwinEventType::MemoryReviewed;
    event.governance.review = match decision {
        MemoryReviewDecision::Accept => ReviewState::Accepted,
        MemoryReviewDecision::Reject => ReviewState::Rejected,
        MemoryReviewDecision::Supersede => ReviewState::Superseded,
    };
    event.governance.authority = AuthorityClass::ReviewedMemory;
    event.event_id = derive_event_id(&event);
    event
}

fn set_relationship_context(event: &mut TwinEvent, object_id: &str) {
    event.context.relationships = vec![RelationshipAssertion {
        subject_id: EntityId::parse("owner").unwrap(),
        predicate: RelationshipPredicate::parse("works_with").unwrap(),
        object_id: EntityId::parse(object_id).unwrap(),
        direction: RelationshipDirection::Directed,
        valid_from: None,
        valid_to: None,
        evidence: Vec::new(),
        governance: Governance::direct_observation(),
    }];
    event.event_id = derive_event_id(event);
}

fn expected_relationship_variant(object_id: &str) -> RelationshipVariant {
    RelationshipVariant::new(vec![RelationshipKey {
        subject_id: EntityId::parse("owner").unwrap(),
        predicate: RelationshipPredicate::parse("works_with").unwrap(),
        object_id: EntityId::parse(object_id).unwrap(),
        direction: RelationshipDirection::Directed,
    }])
}

#[test]
fn review_timeline_transition_keeps_the_proposal_relationship_variant() {
    let mut proposed = proposal("proposal-alex", "memory-alex");
    set_relationship_context(&mut proposed, "alex");
    let mut accepted = review(
        "review-bob",
        "memory-alex",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    set_relationship_context(&mut accepted, "bob");

    let snapshot = project(&[proposed, accepted.clone()], reference()).unwrap();
    let transition = snapshot
        .timeline
        .iter()
        .find(|entry| entry.source_event_id == accepted.event_id)
        .unwrap();

    assert_eq!(
        transition.relationship_variant,
        expected_relationship_variant("alex")
    );
}

#[test]
fn conflict_timeline_transition_keeps_the_proposal_relationship_variant() {
    let mut proposed = proposal("proposal-alex-conflict", "memory-alex-conflict");
    set_relationship_context(&mut proposed, "alex");
    let accepted = review(
        "review-global-accept",
        "memory-alex-conflict",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let mut rejected = review(
        "review-bob-reject",
        "memory-alex-conflict",
        MemoryReviewDecision::Reject,
        vec![proposed.event_id.clone()],
    );
    set_relationship_context(&mut rejected, "bob");

    let snapshot = project(&[proposed, accepted, rejected], reference()).unwrap();
    let transition = snapshot
        .timeline
        .iter()
        .find(|entry| entry.state == TimelineState::PendingConflict)
        .unwrap();

    assert_eq!(
        transition.relationship_variant,
        expected_relationship_variant("alex")
    );
}

#[test]
fn superseded_timeline_transition_keeps_the_proposal_relationship_variant() {
    let mut proposed = proposal("proposal-alex-superseded", "memory-alex-superseded");
    set_relationship_context(&mut proposed, "alex");
    let accepted = review(
        "review-alex-accepted",
        "memory-alex-superseded",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let mut superseder =
        valid_event_for_device("superseder-bob", 1, vec![proposed.event_id.clone()]);
    superseder.supersedes = vec![proposed.event_id.clone()];
    set_relationship_context(&mut superseder, "bob");

    let snapshot = project(&[proposed, accepted, superseder], reference()).unwrap();
    let transition = snapshot
        .timeline
        .iter()
        .find(|entry| entry.state == TimelineState::Superseded)
        .unwrap();

    assert_eq!(
        transition.relationship_variant,
        expected_relationship_variant("alex")
    );
}

#[test]
fn projection_entry_point_is_pure_and_explicitly_timed() {
    let snapshot = project(&[], reference()).unwrap();
    assert_eq!(snapshot.reference_time, reference());
    assert!(snapshot.reviewed_memories.is_empty());
    assert_eq!(
        canonical_snapshot_json(&snapshot).unwrap(),
        serde_json::to_vec(&snapshot).unwrap()
    );
}

#[test]
fn valid_companion_capture_projects_one_evidence_only_observation() {
    let event = companion_capture_observation("companion-device", "captured-note");
    let TwinEventPayload::ObservationRecorded(raw) = &event.payload else {
        unreachable!()
    };
    assert!(raw.claims.is_empty());

    let first = project(&[event.clone()], reference()).unwrap();
    let second = project(&[event.clone()], reference()).unwrap();

    assert_eq!(first.projection_version, 3);
    assert_eq!(first.recent_observations.len(), 1);
    assert!(first.pending_proposals.is_empty());
    assert!(first.reviewed_memories.is_empty());
    let item = &first.recent_observations[0];
    assert_eq!(
        item.item_id.as_str(),
        format!("observation:{}:capture", event.event_id)
    );
    assert_eq!(item.kind, ProjectedItemKind::Observation);
    assert_eq!(item.claim.subject_id.as_str(), "owner");
    assert_eq!(item.claim.predicate.as_str(), "recorded_note");
    assert_eq!(item.claim.object.as_str(), "captured-note");
    assert_eq!(item.claim.polarity, ClaimPolarity::Affirmed);
    assert_eq!(item.support_count, 1);
    assert_eq!(item.opposition_count, 0);
    assert_eq!(item.prior_exact_support_count, 0);
    assert_eq!(item.evidence_event_ids, vec![event.event_id.clone()]);
    assert_eq!(
        item.summary.as_ref().unwrap().as_str(),
        "Captured preference"
    );
    assert_eq!(item.goals, vec!["Ship Grafyn"]);
    assert_eq!(item.relationship_variant.relationships.len(), 1);
    assert_eq!(item.last_confirmed_at, event.observed_at);
    assert_eq!(item.causal_stream, CausalStream::LocalOnly);
    assert_eq!(item.governance.visibility, Visibility::LocalOnly);
    assert_eq!(first.timeline.len(), 1);
    assert_eq!(first.timeline[0].state, TimelineState::Observed);
    assert_eq!(first.timeline[0].item_id, item.item_id);
    assert_eq!(
        canonical_snapshot_json(&first).unwrap(),
        canonical_snapshot_json(&second).unwrap()
    );
}

#[test]
fn malformed_or_noncompanion_empty_observations_do_not_gain_capture_state() {
    let base = companion_capture_observation("companion-device", "captured-note");

    let mut wrong_source = base.clone();
    wrong_source.context.source_channel = SourceChannel::parse("import").unwrap();
    wrong_source.event_id = derive_event_id(&wrong_source);

    let mut wrong_identity = base.clone();
    let TwinEventPayload::ObservationRecorded(payload) = &mut wrong_identity.payload else {
        unreachable!()
    };
    payload.observation_id = Identifier::parse("not-a-companion-capture").unwrap();
    wrong_identity.event_id = derive_event_id(&wrong_identity);

    let mut claim_bearing = base.clone();
    let TwinEventPayload::ObservationRecorded(payload) = &mut claim_bearing.payload else {
        unreachable!()
    };
    payload.claims.push(ClaimAssertion {
        subject_id: EntityId::parse("owner").unwrap(),
        predicate: ClaimPredicate::parse("prefers").unwrap(),
        object: ClaimObject::parse("quiet work").unwrap(),
        polarity: ClaimPolarity::Affirmed,
    });
    claim_bearing.event_id = derive_event_id(&claim_bearing);

    let mut missing_note = base.clone();
    missing_note.evidence.clear();
    missing_note.event_id = derive_event_id(&missing_note);

    let mut multiple_notes = base.clone();
    multiple_notes.evidence.push(EvidenceRef {
        evidence_type: EvidenceType::Note,
        source_id: Identifier::parse("another-note").unwrap(),
        digest: Some(ContentDigest::parse("e".repeat(64)).unwrap()),
    });
    multiple_notes.evidence.sort();
    multiple_notes.event_id = derive_event_id(&multiple_notes);

    let mut mismatched_digest = base;
    mismatched_digest.evidence[0].digest = Some(ContentDigest::parse("e".repeat(64)).unwrap());
    mismatched_digest.event_id = derive_event_id(&mismatched_digest);

    for event in [
        wrong_source,
        wrong_identity,
        claim_bearing,
        missing_note,
        multiple_notes,
        mismatched_digest,
    ] {
        let snapshot = project(&[event], reference()).unwrap();
        assert!(snapshot
            .recent_observations
            .iter()
            .all(|item| !item.item_id.as_str().ends_with(":capture")));
        assert!(snapshot
            .timeline
            .iter()
            .all(|item| !item.item_id.as_str().ends_with(":capture")));
    }
}

#[test]
fn canonical_snapshot_rejects_rehashed_versions_and_noncanonical_vectors() {
    let mut wrong_version = project(&[], reference()).unwrap();
    wrong_version.projection_version += 1;
    wrong_version.snapshot_id = derive_snapshot_id(&wrong_version).unwrap();
    assert!(matches!(
        canonical_snapshot_json(&wrong_version),
        Err(ProjectionError::InvalidSnapshot(_))
    ));

    let observations = ["canonical-a", "canonical-b", "canonical-c"]
        .into_iter()
        .map(|device| valid_event_for_device(device, 1, Vec::new()))
        .collect::<Vec<_>>();
    let canonical = project(&observations, reference()).unwrap();

    let mut unsorted = canonical.clone();
    unsorted.applied_event_ids.reverse();
    unsorted.snapshot_id = derive_snapshot_id(&unsorted).unwrap();
    assert!(matches!(
        canonical_snapshot_json(&unsorted),
        Err(ProjectionError::InvalidSnapshot(_))
    ));

    let mut forged_exposure = canonical.clone();
    forged_exposure.recent_observations[0].governance.visibility = Visibility::SyncedVault;
    forged_exposure.recent_observations[0]
        .governance
        .allowed_uses
        .sync = true;
    forged_exposure.snapshot_id = derive_snapshot_id(&forged_exposure).unwrap();
    assert!(matches!(
        canonical_snapshot_json(&forged_exposure),
        Err(ProjectionError::InvalidSnapshot(_))
    ));

    let mut nested_duplicate = canonical;
    let duplicate = nested_duplicate.pending_proposals[0].evidence_event_ids[0].clone();
    nested_duplicate.pending_proposals[0]
        .evidence_event_ids
        .push(duplicate);
    nested_duplicate.snapshot_id = derive_snapshot_id(&nested_duplicate).unwrap();
    assert!(matches!(
        canonical_snapshot_json(&nested_duplicate),
        Err(ProjectionError::InvalidSnapshot(_))
    ));
}

#[test]
fn explicit_accept_promotes_while_reject_remains_audit_only() {
    let proposed = proposal("proposal-device", "memory-one");
    let accepted = review(
        "review-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let snapshot = project(&[accepted.clone(), proposed.clone()], reference()).unwrap();
    assert_eq!(snapshot.reviewed_memories.len(), 1);
    assert!(snapshot.pending_proposals.is_empty());
    assert_eq!(
        snapshot.reviewed_memories[0].last_confirmed_at,
        accepted.observed_at
    );

    let rejected = review(
        "reject-device",
        "memory-one",
        MemoryReviewDecision::Reject,
        vec![proposed.event_id.clone()],
    );
    let snapshot = project(&[proposed, rejected], reference()).unwrap();
    assert!(snapshot.reviewed_memories.is_empty());
    assert!(snapshot.pending_proposals.is_empty());
    assert!(snapshot
        .timeline
        .iter()
        .any(|entry| entry.state == TimelineState::Rejected));
}

#[test]
fn review_cannot_upgrade_proposal_privacy_or_allowed_uses() {
    let mut proposed = proposal("proposal-device", "memory-one");
    proposed.governance.sensitivity = Sensitivity::Restricted;
    proposed.governance.visibility = Visibility::LocalOnly;
    proposed.governance.allowed_uses.twin_simulation = false;
    proposed.governance.allowed_uses.export = false;
    proposed.event_id = derive_event_id(&proposed);
    let accepted = review(
        "review-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let snapshot = project(&[proposed, accepted], reference()).unwrap();
    let governance = &snapshot.reviewed_memories[0].governance;
    assert_eq!(governance.sensitivity, Sensitivity::Restricted);
    assert_eq!(governance.visibility, Visibility::LocalOnly);
    assert!(!governance.allowed_uses.twin_simulation);
    assert!(!governance.allowed_uses.export);
}

#[test]
fn reviewed_memory_folds_referenced_event_exposure() {
    let mut source = valid_event_for_device("source-device", 1, Vec::new());
    source.payload = TwinEventPayload::DecisionRecorded(DecisionRecorded {
        decision_id: Identifier::parse("source-decision").unwrap(),
        decision: BoundedContent::parse("private source").unwrap(),
        options: Vec::new(),
        stakes: None,
        initial_leaning: None,
        review_date: None,
        primitive_assessment: PrimitiveDecisionAssessmentPayload::default(),
    });
    source.event_type = TwinEventType::DecisionRecorded;
    source.causal_stream = CausalStream::LocalOnly;
    source.governance.sensitivity = Sensitivity::Restricted;
    source.governance.visibility = Visibility::LocalOnly;
    source.governance.allowed_uses.recall = false;
    source.governance.allowed_uses.export = false;
    source.governance.allowed_uses.sync = false;
    source.event_id = derive_event_id(&source);

    let mut relationship_source = source.clone();
    relationship_source.device_id = DeviceId::parse("relationship-source-device").unwrap();
    relationship_source.payload = TwinEventPayload::DecisionRecorded(DecisionRecorded {
        decision_id: Identifier::parse("relationship-source-decision").unwrap(),
        decision: BoundedContent::parse("relationship evidence source").unwrap(),
        options: Vec::new(),
        stakes: None,
        initial_leaning: None,
        review_date: None,
        primitive_assessment: PrimitiveDecisionAssessmentPayload::default(),
    });
    relationship_source.governance.sensitivity = Sensitivity::Standard;
    relationship_source.governance.allowed_uses.recall = true;
    relationship_source.governance.allowed_uses.twin_advisor = false;
    relationship_source.event_id = derive_event_id(&relationship_source);

    let mut proposed = proposal("proposal-device", "memory-one");
    proposed.evidence = vec![EvidenceRef {
        evidence_type: EvidenceType::Event,
        source_id: Identifier::parse(source.event_id.as_str()).unwrap(),
        digest: None,
    }];
    proposed.context.relationships = vec![RelationshipAssertion {
        subject_id: EntityId::parse("owner").unwrap(),
        predicate: RelationshipPredicate::parse("works_with").unwrap(),
        object_id: EntityId::parse("manager").unwrap(),
        direction: RelationshipDirection::Directed,
        valid_from: None,
        valid_to: None,
        evidence: vec![EvidenceRef {
            evidence_type: EvidenceType::Event,
            source_id: Identifier::parse(relationship_source.event_id.as_str()).unwrap(),
            digest: None,
        }],
        governance: Governance::direct_observation(),
    }];
    proposed.event_id = derive_event_id(&proposed);
    let accepted = review(
        "review-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );

    let snapshot = project(
        &[accepted, proposed, source, relationship_source],
        reference(),
    )
    .unwrap();
    let item = &snapshot.reviewed_memories[0];
    assert_eq!(item.causal_stream, CausalStream::LocalOnly);
    assert_eq!(item.governance.sensitivity, Sensitivity::Restricted);
    assert_eq!(item.governance.visibility, Visibility::LocalOnly);
    assert!(!item.governance.allowed_uses.recall);
    assert!(!item.governance.allowed_uses.twin_advisor);
    assert!(!item.governance.allowed_uses.export);
    assert!(!item.governance.allowed_uses.sync);
}

#[test]
fn review_decision_and_governance_state_must_agree() {
    let proposed = proposal("proposal-device", "memory-one");
    let mut accepted = review(
        "review-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    accepted.governance.review = ReviewState::Pending;
    accepted.event_id = derive_event_id(&accepted);
    assert!(matches!(
        project(&[proposed, accepted], reference()),
        Err(ProjectionError::InvalidReview(_))
    ));

    let proposed = proposal("proposal-device", "memory-two");
    let mut rejected = review(
        "review-device",
        "memory-two",
        MemoryReviewDecision::Reject,
        vec![proposed.event_id.clone()],
    );
    rejected.governance.authority = AuthorityClass::EvidenceObservation;
    rejected.event_id = derive_event_id(&rejected);
    assert!(matches!(
        project(&[proposed, rejected], reference()),
        Err(ProjectionError::InvalidReview(_))
    ));
}

#[test]
fn observations_and_proposals_cannot_forge_reviewed_governance() {
    let mut observation = valid_event_for_device("observation-device", 1, Vec::new());
    observation.governance.review = ReviewState::Accepted;
    observation.governance.authority = AuthorityClass::CanonicalUserRule;
    observation.governance.allowed_uses.twin_simulation = true;
    observation.event_id = derive_event_id(&observation);
    assert!(matches!(
        project(&[observation], reference()),
        Err(ProjectionError::InvalidReview(_))
    ));

    let mut proposed = proposal("proposal-device", "forged-memory");
    proposed.governance.review = ReviewState::Accepted;
    proposed.governance.authority = AuthorityClass::DeterministicallyVerified {
        method: VerificationMethod::HumanReview,
    };
    proposed.governance.allowed_uses.twin_simulation = true;
    proposed.event_id = derive_event_id(&proposed);
    assert!(matches!(
        project(&[proposed], reference()),
        Err(ProjectionError::InvalidReview(_))
    ));
}

#[test]
fn event_projection_and_ranking_never_turn_pending_simulation_consent_into_authority() {
    let mut proposed = proposal("proposal-device", "pending-simulation");
    proposed.governance.allowed_uses.twin_simulation = true;
    proposed.event_id = derive_event_id(&proposed);
    let snapshot = project(&[proposed], reference()).unwrap();
    let candidates = snapshot
        .pending_proposals
        .iter()
        .cloned()
        .map(|item| crate::models::twin_state::AttentionCandidate { item })
        .collect::<Vec<_>>();
    let trace = crate::services::twin_events::rank(
        snapshot.snapshot_id,
        &candidates,
        crate::models::twin_state::AttentionProfile::Simulation,
        &crate::models::twin_state::AttentionRequest {
            query: "quiet work".to_string(),
            relationship_variant: RelationshipVariant::global(),
            goals: Vec::new(),
            reference_time: reference(),
            destination: crate::models::twin_state::SelectionDestination::Local,
            limit: 10,
        },
    )
    .unwrap();
    assert!(trace.selected.is_empty());
    assert_eq!(trace.excluded.len(), 1);
    assert_eq!(
        trace.excluded[0].reason,
        crate::models::twin_state::ExclusionReasonCode::Review
    );
}

#[test]
fn closed_validity_negative_age_reinforcement_and_supersession_are_deterministic() {
    let mut proposed = proposal("proposal-device", "memory-one");
    proposed.valid_from = Some(reference());
    proposed.valid_to = Some(reference());
    proposed.event_id = derive_event_id(&proposed);
    let accepted = review(
        "review-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let snapshot = project(&[proposed.clone(), accepted.clone()], reference()).unwrap();
    assert_eq!(snapshot.reviewed_memories.len(), 1);

    let mut reinforcement =
        valid_event_for_device("reinforce-device", 1, vec![accepted.event_id.clone()]);
    reinforcement.reinforces = vec![accepted.event_id.clone()];
    reinforcement.observed_at = reference() + Duration::days(1);
    reinforcement.event_id = derive_event_id(&reinforcement);
    let snapshot = project(
        &[proposed.clone(), accepted.clone(), reinforcement.clone()],
        reference(),
    )
    .unwrap();
    assert_eq!(
        snapshot.reviewed_memories[0].last_confirmed_at,
        reinforcement.observed_at
    );

    let mut superseder =
        valid_event_for_device("supersede-device", 1, vec![proposed.event_id.clone()]);
    superseder.supersedes = vec![proposed.event_id.clone()];
    superseder.event_id = derive_event_id(&superseder);
    let snapshot = project(
        &[proposed, accepted, superseder, reinforcement],
        reference(),
    )
    .unwrap();
    assert!(snapshot.reviewed_memories.is_empty());
    assert!(snapshot
        .timeline
        .iter()
        .any(|entry| entry.state == TimelineState::Superseded));
}

#[test]
fn concurrent_contradictory_reviews_abstain_until_causally_resolved() {
    let proposed = proposal("proposal-device", "memory-one");
    let accept = review(
        "accept-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let reject = review(
        "reject-device",
        "memory-one",
        MemoryReviewDecision::Reject,
        vec![proposed.event_id.clone()],
    );
    let snapshot = project(
        &[accept.clone(), proposed.clone(), reject.clone()],
        reference(),
    )
    .unwrap();
    assert!(snapshot.reviewed_memories.is_empty());
    assert_eq!(snapshot.pending_proposals.len(), 1);
    assert!(snapshot
        .timeline
        .iter()
        .any(|entry| entry.state == TimelineState::PendingConflict));

    let resolved = review(
        "resolution-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![accept.event_id.clone(), reject.event_id.clone()],
    );
    let snapshot = project(&[reject, resolved, proposed, accept], reference()).unwrap();
    assert_eq!(snapshot.reviewed_memories.len(), 1);
}

#[test]
fn accepted_review_that_supersedes_an_older_review_becomes_the_frontier() {
    let proposed = proposal("proposal-device", "memory-one");
    let first = review(
        "review-one-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let mut correction = review(
        "review-two-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let TwinEventPayload::MemoryReviewed(payload) = &mut correction.payload else {
        unreachable!()
    };
    let mut corrected_claim = match &proposed.payload {
        TwinEventPayload::MemoryProposed(payload) => payload.claim.clone(),
        _ => unreachable!(),
    };
    corrected_claim.object = ClaimObject::parse("quiet collaborative work").unwrap();
    payload.reviewed_claim = Some(corrected_claim.clone());
    correction.supersedes = vec![first.event_id.clone()];
    correction.observed_at = reference() - Duration::hours(1);
    correction.event_id = derive_event_id(&correction);

    let snapshot = project(&[first, correction.clone(), proposed], reference()).unwrap();
    assert_eq!(snapshot.reviewed_memories.len(), 1);
    assert_eq!(snapshot.reviewed_memories[0].claim, corrected_claim);
    assert_eq!(
        snapshot.reviewed_memories[0].last_confirmed_at,
        correction.observed_at
    );
}

#[test]
fn concurrent_accepts_compare_complete_effective_outcomes() {
    let proposed = proposal("proposal-device", "memory-one");
    let mut first = review(
        "review-one-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let mut second = review(
        "review-two-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let TwinEventPayload::MemoryReviewed(first_payload) = &mut first.payload else {
        unreachable!()
    };
    let mut edited = match &proposed.payload {
        TwinEventPayload::MemoryProposed(payload) => payload.claim.clone(),
        _ => unreachable!(),
    };
    edited.object = ClaimObject::parse("quiet solo work").unwrap();
    first_payload.reviewed_claim = Some(edited);
    first.event_id = derive_event_id(&first);
    second.governance.allowed_uses.export = false;
    second.event_id = derive_event_id(&second);

    let divergent = project(&[first, second, proposed.clone()], reference()).unwrap();
    assert!(divergent.reviewed_memories.is_empty());
    assert_eq!(divergent.pending_proposals.len(), 1);
    assert!(divergent
        .timeline
        .iter()
        .any(|entry| entry.state == TimelineState::PendingConflict));

    let mut shared_proposal = proposal("shared-proposal-device", "shared-memory");
    shared_proposal.causal_stream = CausalStream::SyncEligible;
    shared_proposal.event_id = derive_event_id(&shared_proposal);
    let mut standard = review(
        "standard-review-device",
        "shared-memory",
        MemoryReviewDecision::Accept,
        vec![shared_proposal.event_id.clone()],
    );
    standard.causal_stream = CausalStream::SyncEligible;
    standard.event_id = derive_event_id(&standard);
    let mut sensitive = review(
        "sensitive-review-device",
        "shared-memory",
        MemoryReviewDecision::Accept,
        vec![shared_proposal.event_id.clone()],
    );
    sensitive.causal_stream = CausalStream::SyncEligible;
    sensitive.governance.sensitivity = Sensitivity::Sensitive;
    sensitive.event_id = derive_event_id(&sensitive);
    let governance_divergent =
        project(&[sensitive, shared_proposal, standard], reference()).unwrap();
    assert!(governance_divergent.reviewed_memories.is_empty());
    assert!(governance_divergent
        .timeline
        .iter()
        .any(|entry| entry.state == TimelineState::PendingConflict));

    let identical_one = review(
        "identical-one-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let identical_two = review(
        "identical-two-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let identical = project(&[identical_two, proposed, identical_one], reference()).unwrap();
    assert_eq!(identical.reviewed_memories.len(), 1);
    assert!(!identical
        .timeline
        .iter()
        .any(|entry| entry.state == TimelineState::PendingConflict));
}

#[test]
fn reinforcement_of_historical_review_does_not_refresh_current_memory() {
    let proposed = proposal("proposal-device", "memory-one");
    let first = review(
        "review-one-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    let mut current = review(
        "review-two-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    current.supersedes = vec![first.event_id.clone()];
    current.observed_at = reference() - Duration::hours(2);
    current.event_id = derive_event_id(&current);
    let mut historical_reinforcement =
        valid_event_for_device("reinforcement-device", 1, vec![first.event_id.clone()]);
    historical_reinforcement.reinforces = vec![first.event_id.clone()];
    historical_reinforcement.observed_at = reference() - Duration::hours(1);
    historical_reinforcement.event_id = derive_event_id(&historical_reinforcement);

    let snapshot = project(
        &[proposed, first, current.clone(), historical_reinforcement],
        reference(),
    )
    .unwrap();
    assert_eq!(snapshot.reviewed_memories.len(), 1);
    assert_eq!(
        snapshot.reviewed_memories[0].last_confirmed_at,
        current.observed_at
    );
}

#[test]
fn dangling_and_wrong_type_references_fail_closed() {
    let proposed = proposal("proposal-device", "memory-one");
    let dangling = review(
        "review-device",
        "missing-memory",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    assert!(matches!(
        project(&[proposed.clone(), dangling], reference()),
        Err(ProjectionError::DanglingMemory(_))
    ));

    let mut decision = valid_event_for_device("decision-device", 1, Vec::new());
    decision.payload = TwinEventPayload::DecisionRecorded(DecisionRecorded {
        decision_id: Identifier::parse("decision-one").unwrap(),
        decision: BoundedContent::parse("choose").unwrap(),
        options: Vec::new(),
        stakes: None,
        initial_leaning: None,
        review_date: None,
        primitive_assessment: PrimitiveDecisionAssessmentPayload::default(),
    });
    decision.event_type = TwinEventType::DecisionRecorded;
    decision.event_id = derive_event_id(&decision);
    let mut bad = proposed;
    bad.supersedes = vec![decision.event_id.clone()];
    bad.event_id = derive_event_id(&bad);
    assert!(matches!(
        project(&[decision, bad], reference()),
        Err(ProjectionError::WrongReferenceType(_))
    ));
}

#[test]
fn canvas_regeneration_may_supersede_only_the_same_persisted_response_identity() {
    let canvas_payload = |response_id: &str, response: &str| {
        TwinEventPayload::CanvasResponseRecorded(CanvasResponseRecorded {
            session_id: Identifier::parse("session-one").unwrap(),
            tile_id: Identifier::parse("tile-one").unwrap(),
            response_id: Identifier::parse(response_id).unwrap(),
            prompt: BoundedContent::parse("prompt").unwrap(),
            response: BoundedContent::parse(response).unwrap(),
            model_id: ModelId::parse("openai/gpt-5").unwrap(),
            provider: None,
            provenance: None,
            tokens_used: None,
            cost_usd_decimal: None,
            prompt_digest: None,
            response_digest: None,
        })
    };
    let mut first = valid_event_for_device("canvas-device", 1, Vec::new());
    first.payload = canvas_payload("response-one", "first");
    first.event_type = first.payload.event_type();
    first.event_id = derive_event_id(&first);

    let mut regeneration = valid_event_for_device("canvas-device", 2, vec![first.event_id.clone()]);
    regeneration.payload = canvas_payload("response-one", "regenerated");
    regeneration.event_type = regeneration.payload.event_type();
    regeneration.supersedes = vec![first.event_id.clone()];
    regeneration.event_id = derive_event_id(&regeneration);
    assert!(project(&[first.clone(), regeneration], reference()).is_ok());

    let mut wrong_identity =
        valid_event_for_device("canvas-device", 2, vec![first.event_id.clone()]);
    wrong_identity.payload = canvas_payload("response-two", "other");
    wrong_identity.event_type = wrong_identity.payload.event_type();
    wrong_identity.supersedes = vec![first.event_id.clone()];
    wrong_identity.event_id = derive_event_id(&wrong_identity);
    assert!(matches!(
        project(&[first, wrong_identity], reference()),
        Err(ProjectionError::WrongReferenceType(_))
    ));
}

#[test]
fn relationship_variants_remain_distinct_and_reordered_input_is_byte_identical() {
    let mut global = proposal("global-device", "global-memory");
    let mut contextual = proposal("context-device", "context-memory");
    contextual.context.relationships = vec![RelationshipAssertion {
        subject_id: EntityId::parse("owner").unwrap(),
        predicate: RelationshipPredicate::parse("works_with").unwrap(),
        object_id: EntityId::parse("manager").unwrap(),
        direction: RelationshipDirection::Directed,
        valid_from: None,
        valid_to: None,
        evidence: Vec::new(),
        governance: Governance::direct_observation(),
    }];
    contextual.event_id = derive_event_id(&contextual);
    global.event_id = derive_event_id(&global);
    let first = project(&[contextual.clone(), global.clone()], reference()).unwrap();
    let second = project(&[global, contextual], reference()).unwrap();
    assert_eq!(
        canonical_snapshot_json(&first).unwrap(),
        canonical_snapshot_json(&second).unwrap()
    );
    assert_eq!(first.relationship_variants.len(), 2);
}

#[test]
fn timeline_preserves_global_alex_and_bob_context_deterministically() {
    let global = valid_event_for_device("global-device", 1, Vec::new());
    let mut alex = contextual_observation("alex-device");
    alex.context.relationships[0].object_id = EntityId::parse("alex").unwrap();
    alex.event_id = derive_event_id(&alex);
    let mut bob = contextual_observation("bob-device");
    bob.context.relationships[0].object_id = EntityId::parse("bob").unwrap();
    bob.event_id = derive_event_id(&bob);

    let first = project(&[bob.clone(), global.clone(), alex.clone()], reference()).unwrap();
    let second = project(&[alex, bob, global], reference()).unwrap();

    assert_eq!(
        canonical_snapshot_json(&first).unwrap(),
        canonical_snapshot_json(&second).unwrap()
    );
    let variants = first
        .timeline
        .iter()
        .map(|entry| {
            serde_json::to_value(entry).unwrap()["relationship_variant"]["relationships"]
                .as_array()
                .unwrap()
                .iter()
                .map(|relationship| relationship["object_id"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        variants,
        BTreeSet::from([Vec::new(), vec!["alex".into()], vec!["bob".into()]])
    );
}

#[test]
fn future_recorded_events_are_not_applied_and_direct_observations_never_become_memory() {
    let mut observation = valid_event_for_device("observation-device", 1, Vec::new());
    observation.recorded_at = reference() + Duration::seconds(1);
    observation.event_id = derive_event_id(&observation);
    let snapshot = project(&[observation], reference()).unwrap();
    assert!(snapshot.applied_event_ids.is_empty());
    assert!(snapshot.reviewed_memories.is_empty());
}

#[test]
fn repeated_observations_are_projected_as_pending_rule_drafts() {
    let observations = ["observation-a", "observation-b", "observation-c"]
        .into_iter()
        .map(|device| valid_event_for_device(device, 1, Vec::new()))
        .collect::<Vec<_>>();

    let snapshot = project(&observations, reference()).unwrap();

    assert_eq!(snapshot.pending_proposals.len(), 1);
    let draft = &snapshot.pending_proposals[0];
    assert_eq!(draft.kind, ProjectedItemKind::PendingProposal);
    assert_eq!(draft.proposal_event_id, None);
    assert_eq!(draft.governance.review, ReviewState::Pending);
    assert_eq!(
        draft.governance.authority,
        AuthorityClass::EvidenceObservation
    );
    assert_eq!(draft.evidence_event_ids.len(), 3);
}

#[test]
fn projected_observation_folds_local_lane_before_selection() {
    let mut observation = valid_event_for_device("observation-device", 1, Vec::new());
    observation.causal_stream = CausalStream::LocalOnly;
    observation.governance.visibility = Visibility::SyncedVault;
    observation.governance.allowed_uses.export = true;
    observation.governance.allowed_uses.sync = true;
    observation.event_id = derive_event_id(&observation);

    let snapshot = project(&[observation], reference()).unwrap();
    let item = &snapshot.recent_observations[0];
    assert_eq!(item.causal_stream, CausalStream::LocalOnly);
    assert_eq!(item.governance.visibility, Visibility::LocalOnly);
    assert!(!item.governance.allowed_uses.export);
    assert!(!item.governance.allowed_uses.sync);
}

#[test]
fn direct_projection_retains_inapplicable_relationship_exposure() {
    for case in [
        InapplicableRelationshipCase::ExpiredLocalOnly,
        InapplicableRelationshipCase::RejectedRestricted,
        InapplicableRelationshipCase::SupersededSyncDisabled,
        InapplicableRelationshipCase::FutureSensitiveSyncDisabled,
    ] {
        let mut observation = contextual_observation("observation-device");
        apply_inapplicable_relationship_case(&mut observation, case);

        let snapshot = project(&[observation], reference()).unwrap();
        let item = &snapshot.recent_observations[0];
        assert_eq!(
            item.relationship_variant.relationships,
            vec![RelationshipKey {
                subject_id: EntityId::parse("owner").unwrap(),
                predicate: RelationshipPredicate::parse("works_with").unwrap(),
                object_id: EntityId::parse("manager").unwrap(),
                direction: RelationshipDirection::Directed,
            }],
            "case: {case:?}"
        );
        assert!(!item.governance.allowed_uses.recall, "case: {case:?}");
        assert!(!item.governance.allowed_uses.export, "case: {case:?}");
        assert!(!item.governance.allowed_uses.sync, "case: {case:?}");
        let expected_sensitivity = match case {
            InapplicableRelationshipCase::RejectedRestricted => Sensitivity::Restricted,
            InapplicableRelationshipCase::FutureSensitiveSyncDisabled => Sensitivity::Sensitive,
            _ => Sensitivity::Standard,
        };
        assert_eq!(
            item.governance.sensitivity, expected_sensitivity,
            "case: {case:?}"
        );
    }
}

#[test]
fn relationship_variant_state_lists_every_recent_observation_event() {
    let observations = ["observation-a", "observation-b", "observation-c"]
        .into_iter()
        .map(|device| valid_event_for_device(device, 1, Vec::new()))
        .collect::<Vec<_>>();
    let expected = observations
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    let snapshot = project(&observations, reference()).unwrap();

    assert_eq!(snapshot.relationship_variants.len(), 1);
    assert_eq!(
        snapshot.relationship_variants[0].observation_event_ids,
        expected
    );
}

#[test]
fn projection_retains_lowest_64_evidence_ids_but_complete_counts_and_timeline() {
    let mut observations = (0..65)
        .map(|index| valid_event_for_device(&format!("observation-{index:03}"), 1, Vec::new()))
        .collect::<Vec<_>>();
    let expected = observations
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(crate::models::twin_state::MAX_STATE_LINKS)
        .collect::<Vec<_>>();

    let first = project(&observations, reference()).unwrap();
    let draft = first
        .pending_proposals
        .iter()
        .find(|item| item.proposal_event_id.is_none())
        .unwrap();
    assert_eq!(draft.support_count, 65);
    assert_eq!(draft.evidence_event_ids, expected);
    assert_eq!(first.recent_observations.len(), 65);
    assert!(first
        .recent_observations
        .iter()
        .all(|item| item.support_count == 65 && item.evidence_event_ids.len() == 64));
    assert_eq!(first.timeline.len(), 66);

    observations.reverse();
    let second = project(&observations, reference()).unwrap();
    assert_eq!(
        canonical_snapshot_json(&first).unwrap(),
        canonical_snapshot_json(&second).unwrap()
    );
}

#[test]
fn derived_proposal_aggregates_context_and_time_from_unretained_support() {
    let mut observations = (0..64)
        .map(|index| contextual_observation(&format!("observation-{index:03}")))
        .collect::<Vec<_>>();
    let retained_max = observations
        .iter()
        .map(|event| event.event_id.clone())
        .max()
        .unwrap();
    let newest = (0..10_000)
        .find_map(|nonce| {
            let mut event = contextual_observation(&format!("newest-observation-{nonce:04}"));
            event.observed_at = reference() - Duration::minutes(1);
            event.recorded_at = reference() - Duration::seconds(30);
            event.context.goals = vec!["newest-goal".to_string()];
            event.context.tags = vec!["newest-tag".to_string()];
            event.context.relationships[0].governance.sensitivity = Sensitivity::Sensitive;
            event.context.relationships[0]
                .governance
                .allowed_uses
                .recall = false;
            event.context.relationships[0].governance.visibility = Visibility::LocalOnly;
            event.context.relationships[0]
                .governance
                .allowed_uses
                .export = false;
            event.context.relationships[0].governance.allowed_uses.sync = false;
            event.event_id = derive_event_id(&event);
            (event.event_id > retained_max).then_some(event)
        })
        .expect("bounded deterministic search finds an ID outside the retained lowest 64");
    let newest_id = newest.event_id.clone();
    let newest_time = newest.observed_at;
    observations.push(newest);

    let first = project(&observations, reference()).unwrap();
    let draft = first
        .pending_proposals
        .iter()
        .find(|item| item.proposal_event_id.is_none())
        .unwrap();
    assert_eq!(draft.support_count, 65);
    assert_eq!(draft.evidence_event_ids.len(), 64);
    assert!(!draft.evidence_event_ids.contains(&newest_id));
    assert_eq!(draft.last_confirmed_at, newest_time);
    assert_eq!(draft.goals, vec!["newest-goal"]);
    assert_eq!(draft.tags, vec!["newest-tag"]);
    assert_eq!(draft.governance.sensitivity, Sensitivity::Sensitive);
    assert_eq!(draft.governance.visibility, Visibility::LocalOnly);
    assert!(!draft.governance.allowed_uses.recall);
    assert!(!draft.governance.allowed_uses.export);
    assert!(!draft.governance.allowed_uses.sync);
    let pending_timeline = first
        .timeline
        .iter()
        .find(|entry| entry.item_id == draft.item_id && entry.state == TimelineState::Pending)
        .unwrap();
    assert_eq!(pending_timeline.source_event_id, newest_id);
    assert_eq!(pending_timeline.effective_at, newest_time);

    observations.reverse();
    let second = project(&observations, reference()).unwrap();
    assert_eq!(
        canonical_snapshot_json(&first).unwrap(),
        canonical_snapshot_json(&second).unwrap()
    );
}

#[test]
fn a_review_only_applies_inside_its_closed_validity_interval() {
    let proposed = proposal("proposal-device", "memory-one");
    let mut accepted = review(
        "review-device",
        "memory-one",
        MemoryReviewDecision::Accept,
        vec![proposed.event_id.clone()],
    );
    accepted.valid_from = Some(reference() + Duration::seconds(1));
    accepted.event_id = derive_event_id(&accepted);

    let before = project(&[proposed.clone(), accepted.clone()], reference()).unwrap();
    assert!(before.reviewed_memories.is_empty());
    assert_eq!(before.pending_proposals.len(), 1);

    let at_boundary = project(&[proposed, accepted], reference() + Duration::seconds(1)).unwrap();
    assert_eq!(at_boundary.reviewed_memories.len(), 1);
    assert!(at_boundary.pending_proposals.is_empty());
}

#[test]
fn superseded_observations_do_not_contribute_current_support() {
    let observations = ["observation-a", "observation-b", "observation-c"]
        .into_iter()
        .map(|device| valid_event_for_device(device, 1, Vec::new()))
        .collect::<Vec<_>>();
    let mut proposed = proposal("proposal-device", "memory-one");
    let claim = match &observations[0].payload {
        TwinEventPayload::ObservationRecorded(value) => value.claims[0].clone(),
        _ => unreachable!(),
    };
    let TwinEventPayload::MemoryProposed(payload) = &mut proposed.payload else {
        unreachable!()
    };
    payload.claim = claim;
    proposed.event_id = derive_event_id(&proposed);

    let mut superseder = valid_event_for_device(
        "superseder-device",
        1,
        vec![observations[0].event_id.clone()],
    );
    superseder.payload = TwinEventPayload::NoteChanged(NoteChanged {
        note_id: Identifier::parse("note-one").unwrap(),
        change: NoteChangeKind::Updated,
        content_digest: None,
    });
    superseder.event_type = TwinEventType::NoteChanged;
    superseder.supersedes = vec![observations[0].event_id.clone()];
    superseder.event_id = derive_event_id(&superseder);

    let mut events = observations;
    events.extend([proposed, superseder]);
    let snapshot = project(&events, reference()).unwrap();
    let projected = snapshot
        .pending_proposals
        .iter()
        .find(|item| item.item_id.as_str() == "memory-one")
        .unwrap();
    assert_eq!(projected.support_count, 2);
    assert!(!projected.evidence_event_ids.contains(&events[0].event_id));
}
