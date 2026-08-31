use super::*;
use crate::models::twin::{PromotionState, RecordOrigin, UserRecordCreate, UserRecordKind};
use crate::models::twin_event::{
    AuthorityClass, BoundedContent, CausalStream, ClaimObject, DecisionOutcomeRecorded,
    DecisionRecorded, EntityId, EvidenceRef as EventEvidenceRef, EvidenceType, Governance,
    Identifier, MemoryProposed, MemoryReviewDecision, MemoryReviewed,
    PrimitiveDecisionAssessmentPayload, RelationshipAssertion, RelationshipDirection,
    RelationshipPredicate, ReviewState, Sensitivity, TwinEvent, TwinEventPayload, Visibility,
};
use crate::services::twin_events::{derive_event_id, AppendOutcome, TwinEventStore};
use chrono::{DateTime, TimeZone, Utc};
use std::path::Path;
use std::sync::Arc;
use tempfile::tempdir;

fn reference_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap()
}

fn export_request(bundle_name: &str) -> TwinExportRequest {
    TwinExportRequest {
        reference_time: Some(reference_time()),
        bundle_name: Some(bundle_name.to_string()),
        ..TwinExportRequest::default()
    }
}

fn coordinated_store(
    root: &Path,
) -> (
    TwinStore,
    Arc<TwinEventStore>,
    Arc<crate::services::twin_events::MutationCoordinator>,
) {
    let data = root.join("data");
    let vault = root.join("vault");
    let twin_root = data.join("twin").join("scope-one");
    std::fs::create_dir_all(&twin_root).unwrap();
    std::fs::create_dir_all(&vault).unwrap();
    let events = Arc::new(TwinEventStore::new(&data));
    events.initialize().unwrap();
    let coordinator = Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            events.clone(),
            Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    (
        TwinStore::with_event_recorder(twin_root, data.join("twin"), coordinator.clone()),
        events,
        coordinator,
    )
}

fn rehash(mut event: TwinEvent) -> TwinEvent {
    event.event_id = derive_event_id(&event);
    event
}

fn observation(
    device: &str,
    sequence: u64,
    parents: Vec<crate::models::twin_event::EventId>,
    marker: &str,
) -> TwinEvent {
    let mut event = crate::services::twin_events::test_support::valid_event_for_device_and_stream(
        device,
        CausalStream::SyncEligible,
        sequence,
        parents,
    );
    let TwinEventPayload::ObservationRecorded(payload) = &mut event.payload else {
        unreachable!()
    };
    payload.observation_id = Identifier::parse(format!("observation-{device}-{sequence}")).unwrap();
    payload.claims[0].object = ClaimObject::parse(marker).unwrap();
    rehash(event)
}

fn memory_proposal(
    device: &str,
    sequence: u64,
    parents: Vec<crate::models::twin_event::EventId>,
    memory_id: &str,
) -> TwinEvent {
    let mut event = observation(device, sequence, parents, "proposal evidence");
    let claim = match &event.payload {
        TwinEventPayload::ObservationRecorded(payload) => payload.claims[0].clone(),
        _ => unreachable!(),
    };
    event.payload = TwinEventPayload::MemoryProposed(MemoryProposed {
        memory_id: Identifier::parse(memory_id).unwrap(),
        claim,
        summary: None,
        proposal_source: crate::models::twin_event::ProvenanceLabel::parse("reviewed_capture")
            .unwrap(),
    });
    event.event_type = event.payload.event_type();
    event.governance.review = ReviewState::Pending;
    event.governance.authority = AuthorityClass::EvidenceObservation;
    rehash(event)
}

fn accepted_review(
    device: &str,
    sequence: u64,
    parents: Vec<crate::models::twin_event::EventId>,
    memory_id: &str,
) -> TwinEvent {
    let mut event = observation(device, sequence, parents, "accepted review");
    event.payload = TwinEventPayload::MemoryReviewed(MemoryReviewed {
        memory_id: Identifier::parse(memory_id).unwrap(),
        decision: MemoryReviewDecision::Accept,
        reviewed_claim: None,
        rationale: None,
    });
    event.event_type = event.payload.event_type();
    event.governance.review = ReviewState::Accepted;
    event.governance.authority = AuthorityClass::ReviewedMemory;
    rehash(event)
}

fn rejected_review(
    device: &str,
    sequence: u64,
    parents: Vec<crate::models::twin_event::EventId>,
    memory_id: &str,
) -> TwinEvent {
    let mut event = accepted_review(device, sequence, parents, memory_id);
    let TwinEventPayload::MemoryReviewed(payload) = &mut event.payload else {
        unreachable!()
    };
    payload.decision = MemoryReviewDecision::Reject;
    event.governance.review = ReviewState::Rejected;
    rehash(event)
}

fn superseded_review(
    device: &str,
    sequence: u64,
    parents: Vec<crate::models::twin_event::EventId>,
    memory_id: &str,
) -> TwinEvent {
    let mut event = accepted_review(device, sequence, parents, memory_id);
    let TwinEventPayload::MemoryReviewed(payload) = &mut event.payload else {
        unreachable!()
    };
    payload.decision = MemoryReviewDecision::Supersede;
    event.governance.review = ReviewState::Superseded;
    rehash(event)
}

fn decision(device: &str, decision_id: &str) -> TwinEvent {
    let mut event = observation(device, 1, Vec::new(), "decision marker");
    event.payload = TwinEventPayload::DecisionRecorded(DecisionRecorded {
        decision_id: Identifier::parse(decision_id).unwrap(),
        decision: BoundedContent::parse("Choose a path").unwrap(),
        options: vec![BoundedContent::parse("Option A").unwrap()],
        stakes: None,
        initial_leaning: None,
        review_date: None,
        primitive_assessment: PrimitiveDecisionAssessmentPayload::default(),
    });
    event.event_type = event.payload.event_type();
    rehash(event)
}

fn decision_outcome(device: &str, decision_id: &str) -> TwinEvent {
    let mut event = observation(device, 1, Vec::new(), "outcome marker");
    event.payload = TwinEventPayload::DecisionOutcomeRecorded(DecisionOutcomeRecorded {
        decision_id: Identifier::parse(decision_id).unwrap(),
        outcome: Some(BoundedContent::parse("Outcome recorded").unwrap()),
        chosen_option: None,
        selected_response_id: None,
        confidence_basis_points: None,
        review_date: None,
        correction_note: None,
        regret_score: None,
        lesson: None,
        missed_something: None,
        primitive_assessment: None,
    });
    event.event_type = event.payload.event_type();
    rehash(event)
}

fn event_evidence(event: &TwinEvent) -> EventEvidenceRef {
    EventEvidenceRef {
        evidence_type: EvidenceType::Event,
        source_id: Identifier::parse(event.event_id.as_str()).unwrap(),
        digest: None,
    }
}

fn append_all(store: &TwinEventStore, events: &[TwinEvent]) {
    for event in events {
        assert_eq!(
            store.append(event.clone()).unwrap(),
            AppendOutcome::Appended
        );
    }
}

fn exported_events(bundle: &ExportBundle) -> Vec<TwinEvent> {
    std::fs::read_to_string(&bundle.twin_events.path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn eligible_reviewed_chain_exports_unchanged_and_projects_only_accepted_state() {
    let root = tempdir().unwrap();
    let (mut store, events, _) = coordinated_store(root.path());
    let observation = observation("chain", 1, Vec::new(), "quiet mornings");
    let mut proposal = memory_proposal(
        "chain",
        2,
        vec![observation.event_id.clone()],
        "memory-chain",
    );
    proposal.evidence.push(event_evidence(&observation));
    proposal = rehash(proposal);
    let review = accepted_review("chain", 3, vec![proposal.event_id.clone()], "memory-chain");
    append_all(
        &events,
        &[observation.clone(), proposal.clone(), review.clone()],
    );

    let bundle = store
        .export_bundle(export_request("eligible-chain"))
        .unwrap();

    assert_eq!(bundle.reference_time, reference_time());
    assert_eq!(
        exported_events(&bundle),
        vec![observation, proposal, review]
    );
    let projection: Value =
        serde_json::from_slice(&std::fs::read(&bundle.projection_manifest.path).unwrap()).unwrap();
    assert_eq!(projection["schema_version"], 1);
    assert_eq!(
        projection["reference_time"],
        serde_json::to_value(reference_time()).unwrap()
    );
    assert_eq!(projection["reviewed_memories"].as_array().unwrap().len(), 1);
    assert_eq!(
        projection["reviewed_memories"][0]["item_id"],
        "memory-chain"
    );
    for forbidden in [
        "pending_proposals",
        "contradiction_clusters",
        "recent_observations",
    ] {
        assert!(projection.get(forbidden).is_none(), "leaked {forbidden}");
    }
}

#[test]
fn export_excludes_every_governance_and_review_denial_before_serialization() {
    let outer = tempdir().unwrap();
    let root = outer.path().join("ABSOLUTE_PATH_SENTINEL");
    let (mut store, events, _) = coordinated_store(&root);

    let mut sensitive = observation("sensitive", 1, Vec::new(), "explicit sensitive export");
    sensitive.governance.sensitivity = Sensitivity::Sensitive;
    sensitive = rehash(sensitive);

    let mut no_export = observation("no-export", 1, Vec::new(), "SECRET_SENTINEL_NO_EXPORT");
    no_export.governance.allowed_uses.export = false;
    no_export = rehash(no_export);

    let mut local = observation("local", 1, Vec::new(), "local visibility marker");
    local.causal_stream = CausalStream::LocalOnly;
    local.governance.visibility = Visibility::LocalOnly;
    local.governance.allowed_uses.export = false;
    local.governance.allowed_uses.sync = false;
    local = rehash(local);

    let mut restricted = observation("restricted", 1, Vec::new(), "restricted marker");
    restricted.causal_stream = CausalStream::LocalOnly;
    restricted.governance.sensitivity = Sensitivity::Restricted;
    restricted = rehash(restricted);

    let mut relationship = observation("relationship", 1, Vec::new(), "relationship marker");
    let mut relationship_governance = Governance::direct_observation();
    relationship_governance.allowed_uses.export = false;
    relationship
        .context
        .relationships
        .push(RelationshipAssertion {
            subject_id: EntityId::parse("owner").unwrap(),
            predicate: RelationshipPredicate::parse("works_with").unwrap(),
            object_id: EntityId::parse("person-a").unwrap(),
            direction: RelationshipDirection::Directed,
            valid_from: None,
            valid_to: None,
            evidence: Vec::new(),
            governance: relationship_governance,
        });
    relationship = rehash(relationship);

    let mut relationship_pending = observation(
        "relationship-pending",
        1,
        Vec::new(),
        "pending relationship marker",
    );
    let mut pending_relationship_governance = Governance::direct_observation();
    pending_relationship_governance.review = ReviewState::Pending;
    relationship_pending
        .context
        .relationships
        .push(RelationshipAssertion {
            subject_id: EntityId::parse("owner").unwrap(),
            predicate: RelationshipPredicate::parse("works_with").unwrap(),
            object_id: EntityId::parse("person-b").unwrap(),
            direction: RelationshipDirection::Directed,
            valid_from: None,
            valid_to: None,
            evidence: Vec::new(),
            governance: pending_relationship_governance,
        });
    relationship_pending = rehash(relationship_pending);

    let pending = memory_proposal("pending", 1, Vec::new(), "pending-memory");
    let rejected_proposal = memory_proposal("rejected", 1, Vec::new(), "rejected-memory");
    let rejected = rejected_review(
        "rejected",
        2,
        vec![rejected_proposal.event_id.clone()],
        "rejected-memory",
    );
    let superseded_proposal = memory_proposal("superseded", 1, Vec::new(), "superseded-memory");
    let superseded = superseded_review(
        "superseded",
        2,
        vec![superseded_proposal.event_id.clone()],
        "superseded-memory",
    );
    let mut future = observation("future", 1, Vec::new(), "future marker");
    future.recorded_at = reference_time() + chrono::Duration::seconds(1);
    future.observed_at = future.recorded_at;
    future = rehash(future);
    append_all(
        &events,
        &[
            sensitive.clone(),
            no_export,
            local,
            restricted,
            relationship,
            relationship_pending,
            pending,
            rejected_proposal,
            rejected,
            superseded_proposal,
            superseded,
            future,
        ],
    );

    let bundle = store.export_bundle(export_request("governance")).unwrap();

    assert_eq!(exported_events(&bundle), vec![sensitive]);
    for forbidden in ["SECRET_SENTINEL_NO_EXPORT", "ABSOLUTE_PATH_SENTINEL"] {
        for entry in std::fs::read_dir(&bundle.output_dir).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains(forbidden));
        }
    }
}

#[test]
fn later_review_revocation_removes_every_previously_accepted_memory_event() {
    let root = tempdir().unwrap();
    let (mut store, events, _) = coordinated_store(root.path());

    let rejected_proposal = memory_proposal("accepted-rejected", 1, Vec::new(), "revoked-memory");
    let accepted_before_reject = accepted_review(
        "accepted-rejected",
        2,
        vec![rejected_proposal.event_id.clone()],
        "revoked-memory",
    );
    let rejected_after_accept = rejected_review(
        "accepted-rejected",
        3,
        vec![accepted_before_reject.event_id.clone()],
        "revoked-memory",
    );

    let superseded_proposal =
        memory_proposal("accepted-superseded", 1, Vec::new(), "superseded-memory");
    let accepted_before_supersede = accepted_review(
        "accepted-superseded",
        2,
        vec![superseded_proposal.event_id.clone()],
        "superseded-memory",
    );
    let superseded_after_accept = superseded_review(
        "accepted-superseded",
        3,
        vec![accepted_before_supersede.event_id.clone()],
        "superseded-memory",
    );
    let survivor = observation("revocation-survivor", 1, Vec::new(), "retained survivor");
    append_all(
        &events,
        &[
            rejected_proposal,
            accepted_before_reject,
            rejected_after_accept,
            superseded_proposal,
            accepted_before_supersede,
            superseded_after_accept,
            survivor.clone(),
        ],
    );

    let bundle = store
        .export_bundle(export_request("review-revocation"))
        .unwrap();

    assert_eq!(exported_events(&bundle), vec![survivor]);
    let projection: Value =
        serde_json::from_slice(&std::fs::read(&bundle.projection_manifest.path).unwrap()).unwrap();
    assert!(projection["reviewed_memories"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn private_superseder_revokes_an_older_exportable_event() {
    let root = tempdir().unwrap();
    let (mut store, events, _) = coordinated_store(root.path());
    let original = observation("exportable-original", 1, Vec::new(), "revoked observation");
    let mut private_revoker = observation("private-revoker", 1, Vec::new(), "private correction");
    private_revoker.causal_stream = CausalStream::LocalOnly;
    private_revoker.governance.visibility = Visibility::LocalOnly;
    private_revoker.governance.sensitivity = Sensitivity::Restricted;
    private_revoker.governance.allowed_uses.export = false;
    private_revoker.governance.allowed_uses.sync = false;
    private_revoker.supersedes = vec![original.event_id.clone()];
    private_revoker = rehash(private_revoker);
    append_all(&events, &[original, private_revoker]);

    let bundle = store
        .export_bundle(export_request("private-revocation"))
        .unwrap();

    assert!(exported_events(&bundle).is_empty());
}

#[test]
fn dependency_closure_prunes_every_explicit_and_typed_semantic_dangling_reference() {
    let root = tempdir().unwrap();
    let (mut store, events, _) = coordinated_store(root.path());
    let mut denied = observation("denied", 1, Vec::new(), "denied dependency");
    denied.governance.allowed_uses.export = false;
    denied = rehash(denied);

    let causal = observation(
        "causal-child",
        1,
        vec![denied.event_id.clone()],
        "causal child",
    );
    let mut evidence = observation("evidence-child", 1, Vec::new(), "evidence child");
    evidence.evidence.push(event_evidence(&denied));
    evidence = rehash(evidence);
    let mut supersedes = observation("supersedes-child", 1, Vec::new(), "supersedes child");
    supersedes.supersedes.push(denied.event_id.clone());
    supersedes = rehash(supersedes);
    let mut reinforces = observation("reinforces-child", 1, Vec::new(), "reinforces child");
    reinforces.reinforces.push(denied.event_id.clone());
    reinforces = rehash(reinforces);
    let mut relationship = observation("relationship-child", 1, Vec::new(), "relationship child");
    relationship
        .context
        .relationships
        .push(RelationshipAssertion {
            subject_id: EntityId::parse("owner").unwrap(),
            predicate: RelationshipPredicate::parse("works_with").unwrap(),
            object_id: EntityId::parse("person-b").unwrap(),
            direction: RelationshipDirection::Directed,
            valid_from: None,
            valid_to: None,
            evidence: vec![event_evidence(&denied)],
            governance: Governance::direct_observation(),
        });
    relationship = rehash(relationship);

    let mut denied_proposal = memory_proposal("proposal", 1, Vec::new(), "semantic-memory");
    denied_proposal.governance.allowed_uses.export = false;
    denied_proposal = rehash(denied_proposal);
    let review = accepted_review("review", 1, Vec::new(), "semantic-memory");

    let mut denied_decision = decision("decision", "semantic-decision");
    denied_decision.governance.allowed_uses.export = false;
    denied_decision = rehash(denied_decision);
    let outcome = decision_outcome("outcome", "semantic-decision");
    let missing_outcome = decision_outcome("missing-outcome", "missing-decision");
    let survivor = observation("survivor", 1, Vec::new(), "retained survivor");
    append_all(
        &events,
        &[
            denied,
            causal,
            evidence,
            supersedes,
            reinforces,
            relationship,
            denied_proposal,
            review,
            denied_decision,
            outcome,
            missing_outcome,
            survivor.clone(),
        ],
    );

    let bundle = store.export_bundle(export_request("closure")).unwrap();

    assert_eq!(exported_events(&bundle), vec![survivor]);
}

#[test]
fn artifact_manifest_uses_relative_names_and_atomic_export_adds_one_generation() {
    let root = tempdir().unwrap();
    let (mut store, events, coordinator) = coordinated_store(root.path());
    append_all(
        &events,
        &[observation("atomic", 1, Vec::new(), "atomic export")],
    );
    let before = coordinator.current_authority_token().unwrap();

    let (bundle, commit) = store
        .export_bundle_with_commit(export_request("atomic"))
        .unwrap();

    assert_eq!(
        commit.authority_token.unwrap().authority_generation,
        before.authority_generation + 1
    );
    for path in [
        &bundle.twin_events.path,
        &bundle.projection_manifest.path,
        &bundle.train.path,
        &bundle.eval.path,
        &bundle.holdout.path,
        &bundle.manifest_path,
    ] {
        assert!(Path::new(path).is_file(), "missing {path}");
    }
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(&bundle.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["bundle_schema_version"], 3);
    assert!(manifest.get("root_path").is_none());
    assert_eq!(
        manifest["reference_time"],
        serde_json::to_value(reference_time()).unwrap()
    );
    fn assert_relative_paths(value: &Value) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    if key == "path" {
                        let path = child.as_str().unwrap();
                        assert!(
                            Path::new(path).is_relative(),
                            "absolute artifact path: {path}"
                        );
                        assert_eq!(
                            Path::new(path).components().count(),
                            1,
                            "nested path: {path}"
                        );
                    }
                    assert_relative_paths(child);
                }
            }
            Value::Array(values) => values.iter().for_each(assert_relative_paths),
            _ => {}
        }
    }
    assert_relative_paths(&manifest);
}

#[test]
fn fixed_reference_time_is_deterministic_and_legacy_split_bytes_and_counts_stay_unchanged() {
    let root = tempdir().unwrap();
    let (mut store, _, _) = coordinated_store(root.path());
    let record = store
        .create_user_record(UserRecordCreate {
            kind: UserRecordKind::Fact,
            content: "Legacy split contract".to_string(),
            origin: RecordOrigin::User,
            evidence_refs: Vec::new(),
            confidence: 0.9,
            promotion_state: Some(PromotionState::Candidate),
            valid_from: None,
            valid_until: None,
            links: Vec::new(),
            metadata: HashMap::new(),
        })
        .unwrap();
    store
        .set_user_record_promotion(&record.id, PromotionState::Endorsed, None)
        .unwrap();

    let first = store
        .export_bundle(export_request("deterministic-a"))
        .unwrap();
    let first_events = std::fs::read(&first.twin_events.path).unwrap();
    let first_projection = std::fs::read(&first.projection_manifest.path).unwrap();
    let first_manifest = std::fs::read(&first.manifest_path).unwrap();
    let first_splits = [
        (first.train.count, std::fs::read(&first.train.path).unwrap()),
        (first.eval.count, std::fs::read(&first.eval.path).unwrap()),
        (
            first.holdout.count,
            std::fs::read(&first.holdout.path).unwrap(),
        ),
    ];

    let second = store
        .export_bundle(export_request("deterministic-b"))
        .unwrap();
    let second_splits = [
        (
            second.train.count,
            std::fs::read(&second.train.path).unwrap(),
        ),
        (second.eval.count, std::fs::read(&second.eval.path).unwrap()),
        (
            second.holdout.count,
            std::fs::read(&second.holdout.path).unwrap(),
        ),
    ];

    assert_eq!(
        first_events,
        std::fs::read(&second.twin_events.path).unwrap()
    );
    assert_eq!(
        first_projection,
        std::fs::read(&second.projection_manifest.path).unwrap()
    );
    assert_eq!(
        first_manifest,
        std::fs::read(&second.manifest_path).unwrap()
    );
    assert_eq!(first_splits, second_splits);
    assert_eq!(
        first_splits.iter().map(|(count, _)| count).sum::<usize>(),
        1
    );
    let expected = format!(
        "{}\n",
        serde_json::to_string(&TwinStore::record_to_export_value(
            &store.get_user_record(&record.id).unwrap()
        ))
        .unwrap()
    )
    .into_bytes();
    assert_eq!(
        first_splits
            .iter()
            .find(|(count, _)| *count == 1)
            .unwrap()
            .1,
        expected
    );
}
