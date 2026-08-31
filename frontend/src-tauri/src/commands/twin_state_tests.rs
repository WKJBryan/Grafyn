use super::*;
use crate::models::twin::TwinExportRequest;
use crate::models::twin_event::{
    AllowedUses, CausalStream, ClaimObject, ClaimPolarity, ClaimPredicate, ObservationRecorded,
    Sensitivity, Visibility,
};
use crate::models::twin_state::{ExclusionReasonCode, ProjectedItemKind, TimelineState};
use chrono::{Duration, TimeZone};

#[test]
fn command_requests_reject_unknown_fields_and_initial_supersede() {
    let page = serde_json::json!({
        "referenceTime": "2026-08-31T00:00:00Z",
        "filter": { "relationships": [], "goals": [], "tags": [] },
        "cursor": null,
        "limit": 25
    });
    assert!(serde_json::from_value::<TwinStatePageRequest>(page.clone()).is_ok());

    let mut unknown_page = page;
    unknown_page["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<TwinStatePageRequest>(unknown_page).is_err());

    let review = serde_json::json!({
        "memoryId": "memory-one",
        "decision": "supersede",
        "reviewedClaim": null,
        "rationale": null,
        "expectedSnapshotId": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "snapshotReferenceTime": "2026-08-31T00:00:00Z"
    });
    assert!(serde_json::from_value::<ReviewTwinProposalRequest>(review).is_err());
}

fn page_request(
    filter: TwinStateFilter,
    cursor: Option<String>,
    limit: u16,
) -> TwinStatePageRequest {
    TwinStatePageRequest {
        reference_time: Utc.with_ymd_and_hms(2026, 8, 31, 0, 0, 0).unwrap(),
        filter,
        cursor,
        limit,
    }
}

#[test]
fn normalized_filters_are_order_independent_and_cursor_bound() {
    let first = TwinStateFilter {
        relationships: Vec::new(),
        goals: vec![" Ship Grafyn ".into(), "Learn Rust".into()],
        tags: vec!["Focus".into(), "work".into()],
    };
    let reordered = TwinStateFilter {
        relationships: Vec::new(),
        goals: vec!["learn   rust".into(), "ship grafyn".into()],
        tags: vec!["WORK".into(), "focus".into()],
    };
    let first = normalize_filter(&first).unwrap();
    let reordered = normalize_filter(&reordered).unwrap();
    assert_eq!(
        filter_digest(&first).unwrap(),
        filter_digest(&reordered).unwrap()
    );

    let snapshot = SnapshotId::parse("a".repeat(64)).unwrap();
    let digest = filter_digest(&first).unwrap();
    let (items, cursor) = page_by_key(
        "proposals",
        &snapshot,
        &digest,
        vec!["alpha".to_string(), "beta".to_string()],
        None,
        1,
        |item| item.clone(),
    )
    .unwrap();
    assert_eq!(items, vec!["alpha"]);
    let cursor = cursor.unwrap();
    let (items, next) = page_by_key(
        "proposals",
        &snapshot,
        &digest,
        vec!["alpha".to_string(), "beta".to_string()],
        Some(&cursor),
        1,
        |item| item.clone(),
    )
    .unwrap();
    assert_eq!(items, vec!["beta"]);
    assert!(next.is_none());
    assert!(page_by_key(
        "proposals",
        &snapshot,
        &"b".repeat(64),
        vec!["alpha".to_string(), "beta".to_string()],
        Some(&cursor),
        1,
        |item| item.clone(),
    )
    .is_err());
}

#[test]
fn page_requests_are_strictly_bounded() {
    let filter = TwinStateFilter::default();
    assert!(validate_page_request(&page_request(filter.clone(), None, 0)).is_err());
    assert!(validate_page_request(&page_request(filter.clone(), None, 101)).is_err());
    let oversized = TwinStateFilter {
        relationships: Vec::new(),
        goals: (0..65).map(|index| format!("goal-{index}")).collect(),
        tags: Vec::new(),
    };
    assert!(validate_page_request(&page_request(oversized, None, 25)).is_err());
    assert!(validate_page_request(&page_request(filter, Some("x".repeat(8193)), 25)).is_err());
}

fn claim(object: &str) -> ClaimAssertion {
    ClaimAssertion {
        subject_id: EntityId::parse("owner").unwrap(),
        predicate: ClaimPredicate::parse("prefers").unwrap(),
        object: ClaimObject::parse(object).unwrap(),
        polarity: ClaimPolarity::Affirmed,
    }
}

fn local_governance() -> Governance {
    Governance {
        review: ReviewState::NotApplicable,
        authority: AuthorityClass::EvidenceObservation,
        sensitivity: Sensitivity::Standard,
        visibility: Visibility::LocalOnly,
        allowed_uses: AllowedUses {
            recall: true,
            twin_advisor: true,
            twin_simulation: false,
            export: false,
            training: false,
            sync: false,
        },
    }
}

fn observation_draft(
    id: &str,
    object: &str,
    relationship_object: Option<&str>,
    goals: &[&str],
    tags: &[&str],
    local_only: bool,
    observed_at: DateTime<Utc>,
) -> TwinEventDraft {
    let governance = if local_only {
        local_governance()
    } else {
        Governance::direct_observation()
    };
    let mut draft = TwinEventDraft::observed(
        TwinEventPayload::ObservationRecorded(ObservationRecorded {
            observation_id: Identifier::parse(id).unwrap(),
            claims: vec![claim(object)],
            summary: None,
            content_digest: None,
        }),
        observed_at,
        SourceChannel::parse("twin_state_test").unwrap(),
        governance.clone(),
    );
    draft.context.goals = goals.iter().map(|value| (*value).to_string()).collect();
    draft.context.tags = tags.iter().map(|value| (*value).to_string()).collect();
    draft.context.relationships = relationship_object
        .map(|object_id| {
            vec![RelationshipAssertion {
                subject_id: EntityId::parse("owner").unwrap(),
                predicate: RelationshipPredicate::parse("works_with").unwrap(),
                object_id: EntityId::parse(object_id).unwrap(),
                direction: RelationshipDirection::Directed,
                valid_from: None,
                valid_to: None,
                evidence: Vec::new(),
                governance,
            }]
        })
        .unwrap_or_default();
    draft
}

fn seeded_state() -> (
    AppState,
    tempfile::TempDir,
    tempfile::TempDir,
    DateTime<Utc>,
) {
    let (state, vault, data) = crate::commands::commit_note_write_tests::build_test_state();
    let observed_at = Utc::now() - Duration::minutes(5);
    let mut sync_drafts = Vec::new();
    let mut local_drafts = Vec::new();
    for index in 0..3 {
        sync_drafts.push(observation_draft(
            &format!("manager-{index}"),
            "quiet focused work",
            Some("manager"),
            &["Ship Grafyn"],
            &["Focus"],
            false,
            observed_at + Duration::seconds(index),
        ));
        sync_drafts.push(observation_draft(
            &format!("friend-{index}"),
            "quiet focused work",
            Some("friend"),
            &["Relationships"],
            &["social"],
            false,
            observed_at + Duration::seconds(index + 10),
        ));
        local_drafts.push(observation_draft(
            &format!("private-{index}"),
            "private reflection",
            None,
            &["Reflect"],
            &["private"],
            true,
            observed_at + Duration::seconds(index + 20),
        ));
    }
    let mut restricted_governance = local_governance();
    restricted_governance.review = ReviewState::Pending;
    restricted_governance.sensitivity = Sensitivity::Restricted;
    let mut restricted = TwinEventDraft::observed(
        TwinEventPayload::MemoryProposed(MemoryProposed {
            memory_id: Identifier::parse("restricted-memory").unwrap(),
            claim: claim("restricted memory"),
            summary: None,
            proposal_source: ProvenanceLabel::parse("test fixture").unwrap(),
        }),
        observed_at + Duration::seconds(30),
        SourceChannel::parse("twin_state_test").unwrap(),
        restricted_governance,
    );
    restricted.context.tags = vec!["restricted".into()];
    local_drafts.push(restricted);
    let sync_commit = state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("twin_state_test").unwrap(),
            Vec::new(),
            sync_drafts,
        )
        .unwrap();
    let local_commit = state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .commit_local(
            CausalStream::LocalOnly,
            SourceChannel::parse("twin_state_test").unwrap(),
            Vec::new(),
            local_drafts,
        )
        .unwrap();
    assert!(sync_commit.authority_token.is_some());
    assert!(local_commit.authority_token.is_some());
    (state, vault, data, Utc::now())
}

fn relationship_filter(object_id: &str) -> TwinRelationshipFilter {
    TwinRelationshipFilter {
        subject_id: EntityId::parse("owner").unwrap(),
        predicate: RelationshipPredicate::parse("works_with").unwrap(),
        object_id: EntityId::parse(object_id).unwrap(),
        direction: RelationshipDirection::Directed,
    }
}

fn state_filter(relationship: Option<&str>, goals: &[&str], tags: &[&str]) -> TwinStateFilter {
    TwinStateFilter {
        relationships: relationship.map(relationship_filter).into_iter().collect(),
        goals: goals.iter().map(|value| (*value).to_string()).collect(),
        tags: tags.iter().map(|value| (*value).to_string()).collect(),
    }
}

#[tokio::test]
async fn read_commands_page_filter_rank_and_hold_snapshot_stable() {
    let (state, _vault, _data, reference_time) = seeded_state();
    let filter = state_filter(Some("manager"), &["ship   grafyn"], &["FOCUS"]);
    let first = list_twin_observations_inner(
        &state,
        TwinStatePageRequest {
            reference_time,
            filter: filter.clone(),
            cursor: None,
            limit: 2,
        },
    )
    .await
    .unwrap();
    assert_eq!(first.items.len(), 2);
    let second = list_twin_observations_inner(
        &state,
        TwinStatePageRequest {
            reference_time,
            filter: filter.clone(),
            cursor: first.next_cursor.clone(),
            limit: 2,
        },
    )
    .await
    .unwrap();
    assert_eq!(second.items.len(), 1);
    assert!(second.next_cursor.is_none());
    let first_ids = first
        .items
        .iter()
        .chain(&second.items)
        .map(|item| item.item_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(first_ids.len(), 3);

    let proposals = list_twin_proposals_inner(
        &state,
        TwinStatePageRequest {
            reference_time,
            filter: filter.clone(),
            cursor: None,
            limit: 10,
        },
    )
    .await
    .unwrap();
    assert_eq!(proposals.items.len(), 1);
    let pending_id = proposals.items[0].item_id.clone();
    assert_eq!(proposals.items[0].kind, ProjectedItemKind::PendingProposal);

    let changed_filter = state_filter(Some("manager"), &[], &["different"]);
    assert!(list_twin_observations_inner(
        &state,
        TwinStatePageRequest {
            reference_time,
            filter: changed_filter,
            cursor: first.next_cursor,
            limit: 2,
        },
    )
    .await
    .unwrap_err()
    .contains("stale"));

    let projection =
        get_twin_state_projection_inner(&state, TwinProjectionRequest { reference_time })
            .await
            .unwrap();
    let repeated =
        get_twin_state_projection_inner(&state, TwinProjectionRequest { reference_time })
            .await
            .unwrap();
    assert_eq!(projection.snapshot_id, repeated.snapshot_id);

    let timeline = get_twin_event_timeline_inner(
        &state,
        TwinStatePageRequest {
            reference_time,
            filter: filter.clone(),
            cursor: None,
            limit: 100,
        },
    )
    .await
    .unwrap();
    assert!(timeline.items.len() >= 4);

    let rank_request = |profile, destination, filter| TwinAttentionRankRequest {
        reference_time,
        profile,
        query: "quiet focus".into(),
        relationship_variant: TwinRelationshipVariantInput {
            relationships: vec![relationship_filter("manager")],
        },
        goals: vec!["ship grafyn".into()],
        destination,
        filter,
        limit: 100,
    };
    let capture = rank_twin_attention_inner(
        &state,
        rank_request(
            AttentionProfile::CaptureReview,
            SelectionDestination::Local,
            filter.clone(),
        ),
    )
    .await
    .unwrap();
    assert!(capture
        .trace
        .selected
        .iter()
        .any(|selected| selected.item_id == pending_id));
    let recall = rank_twin_attention_inner(
        &state,
        rank_request(
            AttentionProfile::Recall,
            SelectionDestination::Local,
            filter,
        ),
    )
    .await
    .unwrap();
    assert!(recall.trace.excluded.iter().any(|excluded| {
        excluded.item_id == pending_id && excluded.reason == ExclusionReasonCode::Review
    }));

    let private = rank_twin_attention_inner(
        &state,
        TwinAttentionRankRequest {
            reference_time,
            profile: AttentionProfile::Recall,
            query: "reflection".into(),
            relationship_variant: TwinRelationshipVariantInput {
                relationships: Vec::new(),
            },
            goals: Vec::new(),
            destination: SelectionDestination::Network,
            filter: state_filter(None, &[], &["private"]),
            limit: 100,
        },
    )
    .await
    .unwrap();
    assert!(!private.trace.excluded.is_empty());
    assert!(
        private
            .trace
            .excluded
            .iter()
            .all(|excluded| excluded.reason == ExclusionReasonCode::Visibility),
        "unexpected gate reasons: {:?}",
        private.trace.excluded
    );

    let private_export = rank_twin_attention_inner(
        &state,
        TwinAttentionRankRequest {
            reference_time,
            profile: AttentionProfile::Recall,
            query: "reflection".into(),
            relationship_variant: TwinRelationshipVariantInput {
                relationships: Vec::new(),
            },
            goals: Vec::new(),
            destination: SelectionDestination::Export,
            filter: state_filter(None, &[], &["private"]),
            limit: 100,
        },
    )
    .await
    .unwrap();
    assert!(private_export
        .trace
        .excluded
        .iter()
        .all(|excluded| excluded.reason == ExclusionReasonCode::AllowedUse));

    let restricted = rank_twin_attention_inner(
        &state,
        TwinAttentionRankRequest {
            reference_time,
            profile: AttentionProfile::Recall,
            query: "memory".into(),
            relationship_variant: TwinRelationshipVariantInput {
                relationships: Vec::new(),
            },
            goals: Vec::new(),
            destination: SelectionDestination::Network,
            filter: state_filter(None, &[], &["restricted"]),
            limit: 100,
        },
    )
    .await
    .unwrap();
    assert_eq!(restricted.trace.excluded.len(), 1);
    assert_eq!(
        restricted.trace.excluded[0].reason,
        ExclusionReasonCode::Sensitivity
    );
}

#[tokio::test]
async fn derived_review_is_atomic_contextual_private_and_snapshot_cas_guarded() {
    let (state, _vault, _data, reference_time) = seeded_state();
    let before = get_twin_state_projection_inner(&state, TwinProjectionRequest { reference_time })
        .await
        .unwrap();
    let manager = before
        .pending_proposals
        .iter()
        .find(|item| {
            item.relationship_variant
                .relationships
                .iter()
                .any(|relationship| relationship.object_id.as_str() == "manager")
        })
        .unwrap()
        .clone();
    let friend_id = before
        .pending_proposals
        .iter()
        .find(|item| {
            item.relationship_variant
                .relationships
                .iter()
                .any(|relationship| relationship.object_id.as_str() == "friend")
        })
        .unwrap()
        .item_id
        .clone();
    let request = ReviewTwinProposalRequest {
        memory_id: manager.item_id.clone(),
        decision: TwinProposalReviewDecision::Accept,
        reviewed_claim: Some(manager.claim.clone()),
        rationale: Some("Confirmed in the manager context".into()),
        expected_snapshot_id: before.snapshot_id.clone(),
        snapshot_reference_time: reference_time,
    };
    let event_count = state.twin_event_store.ordered_events().unwrap().len();
    let invalid_reject = ReviewTwinProposalRequest {
        memory_id: manager.item_id.clone(),
        decision: TwinProposalReviewDecision::Reject,
        reviewed_claim: Some(manager.claim.clone()),
        rationale: None,
        expected_snapshot_id: before.snapshot_id.clone(),
        snapshot_reference_time: reference_time,
    };
    assert!(review_twin_proposal_inner(&state, invalid_reject)
        .await
        .unwrap_err()
        .contains("only valid when accepting"));
    assert_eq!(
        state.twin_event_store.ordered_events().unwrap().len(),
        event_count
    );

    let future = ReviewTwinProposalRequest {
        snapshot_reference_time: Utc::now() + Duration::hours(1),
        ..request.clone()
    };
    assert!(review_twin_proposal_inner(&state, future)
        .await
        .unwrap_err()
        .contains("future"));
    assert_eq!(
        state.twin_event_store.ordered_events().unwrap().len(),
        event_count
    );

    let response = review_twin_proposal_inner(&state, request.clone())
        .await
        .unwrap();
    let events = state.twin_event_store.ordered_events().unwrap();
    assert_eq!(events.len(), event_count + 2);
    let proposal = events
        .iter()
        .find(|event| event.event_id == response.proposal_event_id)
        .unwrap();
    let review = events
        .iter()
        .find(|event| event.event_id == response.review_event_id)
        .unwrap();
    assert!(review.causal_parents.contains(&proposal.event_id));
    assert_eq!(proposal.context.goals, manager.goals);
    assert_eq!(proposal.context.tags, manager.tags);
    assert_eq!(proposal.evidence.len(), manager.evidence_event_ids.len());
    assert_eq!(proposal.context.relationships.len(), 1);
    assert_eq!(
        proposal.context.relationships[0].object_id.as_str(),
        "manager"
    );
    assert!(response
        .snapshot
        .reviewed_memories
        .iter()
        .any(|item| item.item_id == manager.item_id));
    assert!(response
        .snapshot
        .pending_proposals
        .iter()
        .any(|item| item.item_id == friend_id));

    let stale = review_twin_proposal_inner(&state, request)
        .await
        .unwrap_err();
    assert!(stale.contains("stale"));
    assert_eq!(
        state.twin_event_store.ordered_events().unwrap().len(),
        event_count + 2
    );

    let reject_snapshot = get_twin_state_projection_inner(
        &state,
        TwinProjectionRequest {
            reference_time: Utc::now(),
        },
    )
    .await
    .unwrap();
    let friend = reject_snapshot
        .pending_proposals
        .iter()
        .find(|item| item.item_id == friend_id)
        .unwrap()
        .clone();
    let rejected = review_twin_proposal_inner(
        &state,
        ReviewTwinProposalRequest {
            memory_id: friend.item_id.clone(),
            decision: TwinProposalReviewDecision::Reject,
            reviewed_claim: None,
            rationale: Some("Not a stable memory".into()),
            expected_snapshot_id: reject_snapshot.snapshot_id,
            snapshot_reference_time: reject_snapshot.reference_time,
        },
    )
    .await
    .unwrap();
    rejected.snapshot.validate().unwrap();
    assert!(!rejected
        .snapshot
        .pending_proposals
        .iter()
        .any(|item| item.item_id == friend.item_id));
    assert!(!rejected
        .snapshot
        .reviewed_memories
        .iter()
        .any(|item| item.item_id == friend.item_id));
    assert!(rejected.snapshot.timeline.iter().any(|entry| {
        entry.item_id == friend.item_id && entry.state == TimelineState::Rejected
    }));

    let private_snapshot = get_twin_state_projection_inner(
        &state,
        TwinProjectionRequest {
            reference_time: Utc::now(),
        },
    )
    .await
    .unwrap();
    let private = private_snapshot
        .pending_proposals
        .iter()
        .find(|item| item.tags.iter().any(|tag| tag == "private"))
        .unwrap()
        .clone();
    let private_response = review_twin_proposal_inner(
        &state,
        ReviewTwinProposalRequest {
            memory_id: private.item_id.clone(),
            decision: TwinProposalReviewDecision::Accept,
            reviewed_claim: None,
            rationale: None,
            expected_snapshot_id: private_snapshot.snapshot_id,
            snapshot_reference_time: private_snapshot.reference_time,
        },
    )
    .await
    .unwrap();
    let private_item = private_response
        .snapshot
        .reviewed_memories
        .iter()
        .find(|item| item.item_id == private.item_id)
        .unwrap();
    assert_eq!(private_item.causal_stream, CausalStream::LocalOnly);
    assert_eq!(private_item.governance.visibility, Visibility::LocalOnly);
    assert!(!private_item.governance.allowed_uses.export);
    assert!(!private_item.governance.allowed_uses.sync);
}

#[tokio::test]
async fn review_rejects_a_viewed_derived_proposal_that_expired_before_commit() {
    let (state, _vault, _data) = crate::commands::commit_note_write_tests::build_test_state();
    let viewed_at = Utc::now() - Duration::minutes(10);
    let expired_at = viewed_at + Duration::minutes(1);
    let drafts = (0..3)
        .map(|index| {
            let mut draft = observation_draft(
                &format!("expiring-{index}"),
                "temporary preference",
                None,
                &["temporary"],
                &["expiring"],
                false,
                viewed_at - Duration::seconds(index + 1),
            );
            draft.valid_to = Some(expired_at);
            draft
        })
        .collect::<Vec<_>>();
    let _ = state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("twin_state_test").unwrap(),
            Vec::new(),
            drafts,
        )
        .unwrap();
    let viewed = get_twin_state_projection_inner(
        &state,
        TwinProjectionRequest {
            reference_time: viewed_at,
        },
    )
    .await
    .unwrap();
    let proposal = viewed
        .pending_proposals
        .iter()
        .find(|item| item.claim.object.as_str() == "temporary preference")
        .unwrap();
    let event_count = state.twin_event_store.ordered_events().unwrap().len();

    let error = review_twin_proposal_inner(
        &state,
        ReviewTwinProposalRequest {
            memory_id: proposal.item_id.clone(),
            decision: TwinProposalReviewDecision::Accept,
            reviewed_claim: None,
            rationale: None,
            expected_snapshot_id: viewed.snapshot_id,
            snapshot_reference_time: viewed.reference_time,
        },
    )
    .await
    .unwrap_err();

    assert!(
        error.contains("no longer pending") || error.contains("changed since it was viewed"),
        "{error}"
    );
    assert_eq!(
        state.twin_event_store.ordered_events().unwrap().len(),
        event_count
    );
}

#[tokio::test]
async fn derived_relationship_review_preserves_source_qualifier_and_exports_the_accepted_chain() {
    let (state, vault, data) = crate::commands::commit_note_write_tests::build_test_state();
    let reference_time = Utc::now();
    let anchor = state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("twin_state_test").unwrap(),
            Vec::new(),
            vec![observation_draft(
                "relationship-anchor",
                "anchor",
                None,
                &[],
                &[],
                false,
                reference_time - Duration::minutes(2),
            )],
        )
        .unwrap()
        .events
        .into_iter()
        .next()
        .unwrap();
    let relationship_evidence = EvidenceRef {
        evidence_type: EvidenceType::Event,
        source_id: Identifier::parse(anchor.event_id.as_str()).unwrap(),
        digest: None,
    };
    let mut drafts = (0..3)
        .map(|index| {
            observation_draft(
                &format!("relationship-support-{index}"),
                "contextual focus",
                Some("manager"),
                &["ship"],
                &["focus"],
                false,
                reference_time - Duration::seconds(index + 1),
            )
        })
        .collect::<Vec<_>>();
    for (index, draft) in drafts.iter_mut().enumerate() {
        let relationship = &mut draft.context.relationships[0];
        relationship.valid_from = Some(reference_time - Duration::days(3 - index as i64));
        relationship.valid_to = Some(reference_time + Duration::days(index as i64 + 1));
        relationship.evidence = vec![relationship_evidence.clone()];
    }
    let mut expected_relationships = drafts
        .iter()
        .map(|draft| draft.context.relationships[0].clone())
        .collect::<Vec<_>>();
    expected_relationships.sort();
    let _ = state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("twin_state_test").unwrap(),
            Vec::new(),
            drafts,
        )
        .unwrap();
    let viewed = get_twin_state_projection_inner(&state, TwinProjectionRequest { reference_time })
        .await
        .unwrap();
    let proposal = viewed
        .pending_proposals
        .iter()
        .find(|item| item.claim.object.as_str() == "contextual focus")
        .unwrap();

    let response = review_twin_proposal_inner(
        &state,
        ReviewTwinProposalRequest {
            memory_id: proposal.item_id.clone(),
            decision: TwinProposalReviewDecision::Accept,
            reviewed_claim: None,
            rationale: None,
            expected_snapshot_id: viewed.snapshot_id,
            snapshot_reference_time: viewed.reference_time,
        },
    )
    .await
    .unwrap();
    let events = state.twin_event_store.ordered_events().unwrap();
    let materialized_proposal = events
        .iter()
        .find(|event| event.event_id == response.proposal_event_id)
        .unwrap();
    let materialized_review = events
        .iter()
        .find(|event| event.event_id == response.review_event_id)
        .unwrap();
    assert_eq!(
        materialized_proposal.context.relationships,
        expected_relationships
    );
    assert_eq!(
        materialized_review.context.relationships,
        expected_relationships
    );
    assert!(
        response
            .snapshot
            .reviewed_memories
            .iter()
            .any(|item| item.item_id == response.memory_id),
        "the accepted materialized relationship memory must be current before export"
    );

    let mut export_store = crate::services::twin::TwinStore::with_event_recorder(
        crate::models::settings::twin_data_path_for_vault(data.path(), vault.path()).unwrap(),
        data.path().join("twin"),
        state.mutation_coordinator.as_ref().unwrap().clone(),
    );
    let bundle = export_store
        .export_bundle(TwinExportRequest {
            bundle_name: Some("relationship-review".into()),
            reference_time: Some(response.reference_time),
            ..TwinExportRequest::default()
        })
        .unwrap();
    let exported_ids = std::fs::read_to_string(&bundle.twin_events.path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<TwinEvent>(line).unwrap().event_id)
        .collect::<BTreeSet<_>>();
    assert!(
        exported_ids.contains(&response.proposal_event_id),
        "exported IDs: {exported_ids:?}"
    );
    assert!(
        exported_ids.contains(&response.review_event_id),
        "exported IDs: {exported_ids:?}"
    );
}

#[tokio::test]
async fn concurrent_reviewers_with_one_snapshot_append_one_compound_review() {
    let (state, _vault, _data, reference_time) = seeded_state();
    let snapshot =
        get_twin_state_projection_inner(&state, TwinProjectionRequest { reference_time })
            .await
            .unwrap();
    let proposal = snapshot.pending_proposals[0].clone();
    let request = ReviewTwinProposalRequest {
        memory_id: proposal.item_id,
        decision: TwinProposalReviewDecision::Accept,
        reviewed_claim: None,
        rationale: None,
        expected_snapshot_id: snapshot.snapshot_id,
        snapshot_reference_time: snapshot.reference_time,
    };
    let before = state.twin_event_store.ordered_events().unwrap().len();
    let (left, right) = tokio::join!(
        review_twin_proposal_inner(&state, request.clone()),
        review_twin_proposal_inner(&state, request)
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let error = left.err().or_else(|| right.err()).unwrap();
    assert!(
        error.contains("stale") || error.contains("changed"),
        "{error}"
    );
    assert_eq!(
        state.twin_event_store.ordered_events().unwrap().len(),
        before + 2
    );
}

#[test]
fn materialized_review_emits_only_review_with_explicit_proposal_parent() {
    let mut item = ProjectedStateItem {
        item_id: Identifier::parse("memory-existing").unwrap(),
        kind: ProjectedItemKind::PendingProposal,
        claim: claim("quiet work"),
        summary: None,
        proposal_event_id: Some(EventId::parse("a".repeat(64)).unwrap()),
        review_event_ids: Vec::new(),
        causal_stream: CausalStream::SyncEligible,
        governance: Governance {
            review: ReviewState::Pending,
            ..Governance::direct_observation()
        },
        relationship_variant: RelationshipVariant::global(),
        evidence_event_ids: Vec::new(),
        support_count: 3,
        opposition_count: 0,
        prior_exact_support_count: 3,
        last_confirmed_at: Utc::now(),
        valid_from: None,
        valid_to: None,
        superseded_by: Vec::new(),
        goals: Vec::new(),
        tags: Vec::new(),
    };
    let mut proposal_event = crate::services::twin_events::test_support::valid_event_for_device(
        "proposal-device",
        1,
        Vec::new(),
    );
    proposal_event.event_id = item.proposal_event_id.clone().unwrap();
    proposal_event.context.relationships = Vec::new();
    let events = vec![proposal_event];
    let drafts = build_review_drafts(
        &item,
        &events,
        TwinProposalReviewDecision::Reject,
        None,
        None,
        Utc::now(),
    )
    .unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(
        drafts[0].causal_parents,
        item.proposal_event_id
            .clone()
            .into_iter()
            .collect::<Vec<_>>()
    );
    assert!(matches!(
        drafts[0].payload,
        TwinEventPayload::MemoryReviewed(_)
    ));

    item.review_event_ids = vec![EventId::parse("b".repeat(64)).unwrap()];
    assert!(build_review_drafts(
        &item,
        &events,
        TwinProposalReviewDecision::Accept,
        None,
        None,
        Utc::now(),
    )
    .is_err());
}
