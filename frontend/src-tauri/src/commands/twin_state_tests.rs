use super::*;
use crate::models::twin::TwinExportRequest;
use crate::models::twin_event::{
    AllowedUses, CausalStream, ClaimObject, ClaimPolarity, ClaimPredicate, ObservationRecorded,
    Sensitivity, Visibility,
};
use crate::models::twin_state::{ExclusionReasonCode, ProjectedItemKind, TimelineState};
use chrono::{Duration, TimeZone};

fn companion_test_state() -> (crate::AppState, tempfile::TempDir, tempfile::TempDir) {
    let (mut state, vault, data) = crate::commands::commit_note_write_tests::build_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
    state.knowledge_store = std::sync::Arc::new(tokio::sync::RwLock::new(
        crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
            vault.path().to_path_buf(),
            coordinator.current_namespace_path().unwrap(),
            coordinator,
        ),
    ));
    (state, vault, data)
}

fn companion_capture_request(
    content: &str,
    grafyn_sync: CompanionSyncPolicy,
) -> CreateCompanionCaptureRequest {
    CreateCompanionCaptureRequest {
        content: content.to_string(),
        capture_kind: CompanionCaptureKind::Text,
        context: CompanionCaptureContextInput::default(),
        attachment_digests: Vec::new(),
        grafyn_sync,
    }
}

#[test]
fn companion_capture_request_is_strict_at_every_boundary() {
    let request = serde_json::json!({
        "content": "A thought",
        "captureKind": "text",
        "context": {
            "person": null,
            "role": null,
            "relationship": null,
            "environment": null,
            "activity": null,
            "goal": null
        },
        "attachmentDigests": [],
        "grafynSync": "inherit"
    });
    assert!(serde_json::from_value::<CreateCompanionCaptureRequest>(request.clone()).is_ok());

    let mut unknown_request = request.clone();
    unknown_request["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<CreateCompanionCaptureRequest>(unknown_request).is_err());

    let mut unknown_context = request.clone();
    unknown_context["context"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<CreateCompanionCaptureRequest>(unknown_context).is_err());

    let mut unsupported_kind = request.clone();
    unsupported_kind["captureKind"] = serde_json::json!("video");
    assert!(serde_json::from_value::<CreateCompanionCaptureRequest>(unsupported_kind).is_err());

    let mut unsupported_policy = request.clone();
    unsupported_policy["grafynSync"] = serde_json::json!("public");
    assert!(serde_json::from_value::<CreateCompanionCaptureRequest>(unsupported_policy).is_err());

    let mut malformed_digest = request;
    malformed_digest["attachmentDigests"] = serde_json::json!(["../attachment"]);
    assert!(serde_json::from_value::<CreateCompanionCaptureRequest>(malformed_digest).is_err());
}

#[test]
fn companion_title_is_markdown_stripped_bounded_and_clock_deterministic() {
    let at = Utc.with_ymd_and_hms(2026, 9, 1, 4, 5, 0).unwrap();
    assert_eq!(
        derive_companion_capture_title("\n## **Build Grafyn** _carefully_\nDetails", at),
        "Build Grafyn carefully"
    );
    assert_eq!(
        derive_companion_capture_title(&format!("# {}", "a".repeat(90)), at)
            .chars()
            .count(),
        80
    );
    assert_eq!(
        derive_companion_capture_title("---\n***", at),
        "Capture 2026-09-01 04:05 UTC"
    );
}

#[tokio::test]
async fn blank_companion_capture_fails_before_any_durable_write() {
    let (state, _vault, _data) = companion_test_state();
    let at = Utc.with_ymd_and_hms(2026, 9, 1, 4, 5, 0).unwrap();

    let error = create_companion_capture_inner(
        &state,
        companion_capture_request(" \n\t ", CompanionSyncPolicy::Inherit),
        at,
    )
    .await
    .unwrap_err();

    assert!(error.contains("blank"), "{error}");
    assert!(state
        .knowledge_store
        .read()
        .await
        .list_notes()
        .unwrap()
        .is_empty());
    assert!(state.twin_event_store.ordered_events().unwrap().is_empty());
}

#[tokio::test]
async fn companion_capture_commits_note_and_contextual_observation_as_one_group() {
    let (state, _vault, _data) = companion_test_state();
    let at = Utc.with_ymd_and_hms(2026, 9, 1, 4, 5, 0).unwrap();
    let attachment = ContentDigest::parse("a".repeat(64)).unwrap();
    let before = state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .current_authority_token()
        .unwrap();
    let request = CreateCompanionCaptureRequest {
        content: "## **Prefers quiet work**\nEspecially in the morning.".into(),
        capture_kind: CompanionCaptureKind::Text,
        context: CompanionCaptureContextInput {
            person: Some("Alex Chen".into()),
            role: Some("manager".into()),
            relationship: Some("works with".into()),
            environment: Some("home office".into()),
            activity: Some("planning".into()),
            goal: Some("Ship Grafyn".into()),
        },
        attachment_digests: vec![attachment.clone(), attachment.clone()],
        grafyn_sync: CompanionSyncPolicy::Inherit,
    };

    let response = create_companion_capture_inner(&state, request, at)
        .await
        .unwrap();

    assert_eq!(response.note.title, "Prefers quiet work");
    assert_eq!(response.note.status, crate::models::note::NoteStatus::Draft);
    assert_eq!(response.note.tags, vec!["inbox"]);
    assert_eq!(
        response.note.properties.get("capture_kind"),
        Some(&serde_json::json!("text"))
    );
    assert_eq!(
        response.note.properties.get("attachment_digests"),
        Some(&serde_json::json!([attachment.as_str()]))
    );
    assert_eq!(
        response.note.properties.get("grafyn_sync"),
        Some(&serde_json::json!("inherit"))
    );

    let after = state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .current_authority_token()
        .unwrap();
    assert_eq!(after.authority_generation, before.authority_generation + 1);
    let events = state.twin_event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 2);
    let TwinEventPayload::NoteChanged(note_changed) = &events[0].payload else {
        panic!("the first event must be NoteChanged");
    };
    assert_eq!(note_changed.note_id.as_str(), response.note.id);
    let TwinEventPayload::ObservationRecorded(observation) = &events[1].payload else {
        panic!("the second event must be ObservationRecorded");
    };
    assert_eq!(events[1].event_id, response.observation_event_id);
    assert_eq!(
        observation.observation_id.as_str(),
        format!("companion-capture-{}", response.note.id)
    );
    assert!(observation.claims.is_empty());
    assert_eq!(observation.content_digest, note_changed.content_digest);
    assert_eq!(events[1].observed_at, at);
    assert_eq!(events[1].context.environments, vec!["home office"]);
    assert_eq!(events[1].context.activities, vec!["planning"]);
    assert_eq!(events[1].context.goals, vec!["Ship Grafyn"]);
    assert_eq!(events[1].context.entities.len(), 1);
    assert_eq!(
        events[1].context.entities[0]
            .display_label
            .as_ref()
            .unwrap()
            .as_str(),
        "Alex Chen"
    );
    assert_eq!(events[1].context.relationships.len(), 1);
    assert_eq!(
        events[1].context.relationships[0].predicate.as_str(),
        "works_with"
    );
    assert_eq!(
        events[1]
            .evidence
            .iter()
            .filter(|value| value.evidence_type == EvidenceType::Attachment)
            .count(),
        1
    );
    assert_eq!(events[1].causal_parents, vec![events[0].event_id.clone()]);
}

#[tokio::test]
async fn companion_capture_recovers_one_plan_without_duplicate_note_or_event() {
    let (state, _vault, _data) = companion_test_state();
    let at = Utc.with_ymd_and_hms(2026, 9, 1, 4, 5, 0).unwrap();
    let _ = state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .fail_next_replays_before_targets(2);

    let response = create_companion_capture_inner(
        &state,
        companion_capture_request("Recovered capture", CompanionSyncPolicy::Inherit),
        at,
    )
    .await
    .unwrap();

    assert_eq!(
        state
            .knowledge_store
            .read()
            .await
            .list_notes()
            .unwrap()
            .len(),
        1
    );
    let events = state.twin_event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].event_id, response.observation_event_id);
    assert_eq!(
        state
            .mutation_coordinator
            .as_ref()
            .unwrap()
            .pending_count()
            .unwrap(),
        0
    );
}

async fn companion_optimizer_queue_size(state: &crate::AppState) -> usize {
    let settings = crate::models::settings::UserSettings::default();
    let mut optimizer = state.vault_optimizer.as_ref().unwrap().write().await;
    optimizer
        .with_locked_fresh_state(|optimizer| Ok(optimizer.status(&settings).queue_size))
        .unwrap()
}

async fn companion_optimizer_queue_ids(state: &crate::AppState) -> Vec<String> {
    let mut optimizer = state.vault_optimizer.as_ref().unwrap().write().await;
    optimizer
        .with_locked_fresh_state(|optimizer| Ok(optimizer.queued_note_ids()))
        .unwrap()
}

#[tokio::test]
async fn companion_capture_enqueues_at_authority_returned_by_same_vault_repair() {
    let (state, _vault, _data) = companion_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap();
    let setup = state
        .knowledge_store
        .write()
        .await
        .create_note_expecting_authority(
            NoteCreate {
                title: "Pending topic normalization".into(),
                content: "A tagged setup note".into(),
                relative_path: Some("pending-topic-normalization.md".into()),
                aliases: Vec::new(),
                status: crate::models::note::NoteStatus::Draft,
                tags: vec!["rust".into()],
                schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: None,
                optimizer_managed: false,
                properties: Default::default(),
            },
            "test",
            coordinator.current_authority_token().unwrap(),
        )
        .unwrap()
        .0;
    crate::commands::enqueue_vault_optimizer_note(&state, &setup.id, "test_setup")
        .await
        .unwrap();
    assert_eq!(
        companion_optimizer_queue_ids(&state).await,
        vec![setup.id.clone()]
    );
    let before_capture = coordinator.current_authority_token().unwrap();

    let capture = create_companion_capture_inner(
        &state,
        companion_capture_request(
            "Capture while topic normalization is pending",
            CompanionSyncPolicy::Inherit,
        ),
        Utc.with_ymd_and_hms(2026, 9, 1, 4, 5, 0).unwrap(),
    )
    .await
    .unwrap();

    let repaired = coordinator.current_authority_token().unwrap();
    assert!(
        repaired.authority_generation > before_capture.authority_generation + 1,
        "same-vault repair must advance beyond the capture commit"
    );
    let queued = companion_optimizer_queue_ids(&state).await;
    assert!(queued.contains(&setup.id));
    assert!(
        queued.contains(&capture.note.id),
        "the captured note must be enqueued under repair's continuation authority; queued={queued:?}"
    );
}

#[tokio::test]
async fn companion_capture_root_retarget_keeps_commit_identity_and_skips_stale_optimizer() {
    let at = Utc.with_ymd_and_hms(2026, 9, 1, 4, 5, 0).unwrap();

    let (state, _vault, _data) = companion_test_state();
    let replacement_vault = tempfile::tempdir().unwrap();
    let response = create_companion_capture_inner_with_post_commit_checkpoint(
        &state,
        companion_capture_request("Capture before a root switch", CompanionSyncPolicy::Inherit),
        at,
        async {
            let coordinator = state.mutation_coordinator.as_ref().unwrap();
            coordinator
                .retarget_markdown_root(replacement_vault.path())
                .unwrap();
            let replacement = coordinator.current_authority_token().unwrap();
            state
                .twin_event_store
                .activate_vault_scope(replacement.root_scope)
                .unwrap();
        },
    )
    .await
    .expect("the response identity must come from the committed event group");
    assert!(!response.observation_event_id.as_str().is_empty());

    let (state, _vault, _data) = companion_test_state();
    let replacement_vault = tempfile::tempdir().unwrap();
    let empty_optimizer_root = tempfile::tempdir().unwrap();
    create_companion_capture_inner_with_post_commit_checkpoint(
        &state,
        companion_capture_request("Do not enqueue across roots", CompanionSyncPolicy::Inherit),
        at,
        async {
            *state.vault_optimizer.as_ref().unwrap().write().await =
                crate::services::vault_optimizer::VaultOptimizerService::new(
                    empty_optimizer_root.path().to_path_buf(),
                );
            state
                .mutation_coordinator
                .as_ref()
                .unwrap()
                .retarget_markdown_root(replacement_vault.path())
                .unwrap();
        },
    )
    .await
    .unwrap();
    assert_eq!(companion_optimizer_queue_size(&state).await, 0);
}

fn companion_recall_rank_request(
    reference_time: DateTime<Utc>,
    note_ids: &[String],
    destination: SelectionDestination,
) -> TwinAttentionRankRequest {
    serde_json::from_value(serde_json::json!({
        "referenceTime": reference_time,
        "profile": "recall",
        "query": "quiet work",
        "relationshipVariant": { "relationships": [] },
        "goals": [],
        "destination": destination,
        "filter": { "relationships": [], "goals": [], "tags": [] },
        "limit": 10,
        "candidateNoteIds": note_ids,
    }))
    .unwrap()
}

#[tokio::test]
async fn companion_recall_ranking_is_candidate_bound_and_deterministic() {
    let (state, _vault, _data) = companion_test_state();
    let at = Utc.with_ymd_and_hms(2026, 9, 1, 4, 5, 0).unwrap();
    let unrelated = create_companion_capture_inner(
        &state,
        companion_capture_request("An unrelated capture", CompanionSyncPolicy::Inherit),
        at - Duration::minutes(1),
    )
    .await
    .unwrap();
    let capture = create_companion_capture_inner(
        &state,
        companion_capture_request("Quiet work helps me focus", CompanionSyncPolicy::Inherit),
        at,
    )
    .await
    .unwrap();
    let events = state.twin_event_store.ordered_events().unwrap();
    assert!(events
        .iter()
        .filter_map(|event| match &event.payload {
            TwinEventPayload::ObservationRecorded(value) => Some(value),
            _ => None,
        })
        .all(|observation| observation.claims.is_empty()));

    let request = companion_recall_rank_request(
        at,
        std::slice::from_ref(&capture.note.id),
        SelectionDestination::Local,
    );
    let first = rank_twin_attention_inner(&state, request.clone())
        .await
        .unwrap();
    let second = rank_twin_attention_inner(&state, request).await.unwrap();

    assert_eq!(first, second);
    assert_eq!(first.trace.selected.len(), 1);
    assert!(first.trace.excluded.is_empty());
    let selected = &first.trace.selected[0];
    assert_eq!(
        selected.item_id.as_str(),
        format!("observation:{}:capture", capture.observation_event_id)
    );
    assert_ne!(
        selected.item_id.as_str(),
        format!("observation:{}:capture", unrelated.observation_event_id)
    );
    assert_eq!(selected.attention.profile, AttentionProfile::Recall);
    assert_eq!(selected.attention.profile_version, 1);
    assert_eq!(selected.attention.vector.confidence.get(), 2_000);
    assert_eq!(selected.attention.vector.recency.get(), 10_000);

    let encoded = serde_json::to_value(&first).unwrap();
    assert_eq!(
        encoded["noteBindings"],
        serde_json::json!([{
            "itemId": selected.item_id.as_str(),
            "noteId": capture.note.id,
        }])
    );

    let mut limited_request = companion_recall_rank_request(
        at,
        &[capture.note.id, unrelated.note.id],
        SelectionDestination::Local,
    );
    limited_request.limit = 1;
    let limited = rank_twin_attention_inner(&state, limited_request)
        .await
        .unwrap();
    assert_eq!(limited.trace.selected.len(), 1);
    assert_eq!(limited.note_bindings.len(), 1);
    assert_eq!(
        limited.note_bindings[0].item_id,
        limited.trace.selected[0].item_id
    );
}

#[tokio::test]
async fn companion_note_binding_is_emitted_only_after_governance_selection() {
    let (state, _vault, _data) = companion_test_state();
    let at = Utc.with_ymd_and_hms(2026, 9, 1, 4, 5, 0).unwrap();
    let capture = create_companion_capture_inner(
        &state,
        companion_capture_request("Local quiet work", CompanionSyncPolicy::LocalOnly),
        at,
    )
    .await
    .unwrap();

    let network = rank_twin_attention_inner(
        &state,
        companion_recall_rank_request(
            at,
            std::slice::from_ref(&capture.note.id),
            SelectionDestination::Network,
        ),
    )
    .await
    .unwrap();
    assert!(network.trace.selected.is_empty());
    assert_eq!(network.trace.excluded.len(), 1);
    assert_eq!(
        network.trace.excluded[0].reason,
        ExclusionReasonCode::Visibility
    );
    assert_eq!(
        serde_json::to_value(&network).unwrap()["noteBindings"],
        serde_json::json!([])
    );

    let local = rank_twin_attention_inner(
        &state,
        companion_recall_rank_request(
            at,
            std::slice::from_ref(&capture.note.id),
            SelectionDestination::Local,
        ),
    )
    .await
    .unwrap();
    assert_eq!(local.trace.selected.len(), 1);
    assert_eq!(
        serde_json::to_value(&local).unwrap()["noteBindings"][0]["noteId"],
        capture.note.id
    );
}

#[tokio::test]
async fn attention_candidate_note_ids_default_and_reject_oversized_requests() {
    let reference_time = Utc.with_ymd_and_hms(2026, 9, 1, 4, 5, 0).unwrap();
    let defaulted = serde_json::json!({
        "referenceTime": reference_time,
        "profile": "recall",
        "query": "quiet work",
        "relationshipVariant": { "relationships": [] },
        "goals": [],
        "destination": "local",
        "filter": { "relationships": [], "goals": [], "tags": [] },
        "limit": 10,
    });
    let defaulted = serde_json::from_value::<TwinAttentionRankRequest>(defaulted).unwrap();
    assert!(defaulted.candidate_note_ids.is_empty());

    let oversized = (0..=MAX_PAGE_LIMIT)
        .map(|index| format!("note-{index}"))
        .collect::<Vec<_>>();
    let request =
        companion_recall_rank_request(reference_time, &oversized, SelectionDestination::Local);
    let (state, _vault, _data) = companion_test_state();
    assert!(rank_twin_attention_inner(&state, request)
        .await
        .unwrap_err()
        .contains("candidate note"));
}

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
    state_filter_relationships(&relationship.into_iter().collect::<Vec<_>>(), goals, tags)
}

fn state_filter_relationships(
    relationships: &[&str],
    goals: &[&str],
    tags: &[&str],
) -> TwinStateFilter {
    TwinStateFilter {
        relationships: relationships
            .iter()
            .map(|relationship| relationship_filter(relationship))
            .collect(),
        goals: goals.iter().map(|value| (*value).to_string()).collect(),
        tags: tags.iter().map(|value| (*value).to_string()).collect(),
    }
}

#[tokio::test]
async fn timeline_filters_keep_global_alex_and_bob_context_identity() {
    let (state, _vault, _data) = crate::commands::commit_note_write_tests::build_test_state();
    let observed_at = Utc::now() - Duration::minutes(2);
    let mut alex_bob = observation_draft(
        "timeline-alex-bob",
        "alex and bob context",
        Some("alex"),
        &[],
        &[],
        false,
        observed_at + Duration::seconds(3),
    );
    alex_bob.context.relationships.push(RelationshipAssertion {
        subject_id: EntityId::parse("owner").unwrap(),
        predicate: RelationshipPredicate::parse("works_with").unwrap(),
        object_id: EntityId::parse("bob").unwrap(),
        direction: RelationshipDirection::Directed,
        valid_from: None,
        valid_to: None,
        evidence: Vec::new(),
        governance: Governance::direct_observation(),
    });
    let drafts = vec![
        observation_draft(
            "timeline-global",
            "global context",
            None,
            &[],
            &[],
            false,
            observed_at,
        ),
        observation_draft(
            "timeline-alex",
            "alex context",
            Some("alex"),
            &[],
            &[],
            false,
            observed_at + Duration::seconds(1),
        ),
        observation_draft(
            "timeline-bob",
            "bob context",
            Some("bob"),
            &[],
            &[],
            false,
            observed_at + Duration::seconds(2),
        ),
        alex_bob,
    ];
    let _ = state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("timeline_context_test").unwrap(),
            Vec::new(),
            drafts,
        )
        .unwrap();
    let reference_time = Utc::now();

    let all = get_twin_event_timeline_inner(
        &state,
        TwinStatePageRequest {
            reference_time,
            filter: state_filter(None, &[], &[]),
            cursor: None,
            limit: 100,
        },
    )
    .await
    .unwrap();
    let all_contexts = all
        .items
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
        all_contexts,
        BTreeSet::from([
            Vec::new(),
            vec!["alex".into()],
            vec!["alex".into(), "bob".into()],
            vec!["bob".into()],
        ])
    );

    for (relationships, expected) in [
        (vec!["alex"], vec!["alex"]),
        (vec!["bob"], vec!["bob"]),
        (vec!["alex", "bob"], vec!["alex", "bob"]),
    ] {
        let page = get_twin_event_timeline_inner(
            &state,
            TwinStatePageRequest {
                reference_time,
                filter: state_filter_relationships(&relationships, &[], &[]),
                cursor: None,
                limit: 100,
            },
        )
        .await
        .unwrap();
        assert!(!page.items.is_empty());
        assert!(page.items.iter().all(|entry| {
            let serialized = serde_json::to_value(entry).unwrap();
            let actual = serialized["relationship_variant"]["relationships"]
                .as_array()
                .unwrap()
                .iter()
                .map(|relationship| relationship["object_id"].as_str().unwrap())
                .collect::<Vec<_>>();
            actual == expected
        }));

        let observations = list_twin_observations_inner(
            &state,
            TwinStatePageRequest {
                reference_time,
                filter: state_filter_relationships(&relationships, &[], &[]),
                cursor: None,
                limit: 100,
            },
        )
        .await
        .unwrap();
        assert!(!observations.items.is_empty());
        assert!(observations.items.iter().all(|item| {
            item.relationship_variant
                .relationships
                .iter()
                .map(|relationship| relationship.object_id.as_str())
                .collect::<Vec<_>>()
                == expected
        }));
    }
}

#[tokio::test]
async fn relationship_filtered_timeline_keeps_supersession_with_another_context_source() {
    let (state, _vault, _data) = crate::commands::commit_note_write_tests::build_test_state();
    let relationship = |object_id: &str| RelationshipAssertion {
        subject_id: EntityId::parse("owner").unwrap(),
        predicate: RelationshipPredicate::parse("works_with").unwrap(),
        object_id: EntityId::parse(object_id).unwrap(),
        direction: RelationshipDirection::Directed,
        valid_from: None,
        valid_to: None,
        evidence: Vec::new(),
        governance: Governance::direct_observation(),
    };
    let memory_id = Identifier::parse("timeline-superseded-alex-memory").unwrap();
    let mut proposed = crate::services::twin_events::test_support::valid_event_for_device(
        "timeline-proposal-alex",
        1,
        Vec::new(),
    );
    let proposed_claim = match &proposed.payload {
        TwinEventPayload::ObservationRecorded(observation) => observation.claims[0].clone(),
        _ => unreachable!(),
    };
    proposed.event_type = crate::models::twin_event::TwinEventType::MemoryProposed;
    proposed.payload = TwinEventPayload::MemoryProposed(MemoryProposed {
        memory_id: memory_id.clone(),
        claim: proposed_claim,
        summary: None,
        proposal_source: ProvenanceLabel::parse("timeline-filter-test").unwrap(),
    });
    proposed.context.relationships = vec![relationship("alex")];
    proposed.context.tags = vec!["proposal-tag".into()];
    proposed.governance.review = ReviewState::Pending;
    proposed.event_id = crate::services::twin_events::derive_event_id(&proposed);

    let mut accepted = crate::services::twin_events::test_support::valid_event_for_device(
        "timeline-review-alex",
        1,
        vec![proposed.event_id.clone()],
    );
    accepted.event_type = crate::models::twin_event::TwinEventType::MemoryReviewed;
    accepted.payload = TwinEventPayload::MemoryReviewed(MemoryReviewed {
        memory_id: memory_id.clone(),
        decision: MemoryReviewDecision::Accept,
        reviewed_claim: None,
        rationale: None,
    });
    accepted.context.relationships = vec![relationship("alex")];
    accepted.governance.review = ReviewState::Accepted;
    accepted.governance.authority = AuthorityClass::ReviewedMemory;
    accepted.event_id = crate::services::twin_events::derive_event_id(&accepted);

    let mut superseder = crate::services::twin_events::test_support::valid_event_for_device(
        "timeline-superseder-bob",
        1,
        Vec::new(),
    );
    superseder.supersedes = vec![proposed.event_id.clone()];
    superseder.context.relationships = vec![relationship("bob")];
    superseder.context.tags = vec!["transition-tag".into()];
    superseder.event_id = crate::services::twin_events::derive_event_id(&superseder);

    for event in [proposed, accepted, superseder] {
        state.twin_event_store.append(event).unwrap();
    }
    let reference_time = Utc::now();

    let alex = get_twin_event_timeline_inner(
        &state,
        TwinStatePageRequest {
            reference_time,
            filter: state_filter(Some("alex"), &[], &[]),
            cursor: None,
            limit: 100,
        },
    )
    .await
    .unwrap();
    assert!(alex
        .items
        .iter()
        .any(|entry| { entry.item_id == memory_id && entry.state == TimelineState::Superseded }));

    let tagged = get_twin_event_timeline_inner(
        &state,
        TwinStatePageRequest {
            reference_time,
            filter: state_filter(Some("alex"), &[], &["transition-tag"]),
            cursor: None,
            limit: 100,
        },
    )
    .await
    .unwrap();
    assert!(tagged
        .items
        .iter()
        .any(|entry| { entry.item_id == memory_id && entry.state == TimelineState::Superseded }));

    let missing_tag = get_twin_event_timeline_inner(
        &state,
        TwinStatePageRequest {
            reference_time,
            filter: state_filter(Some("alex"), &[], &["missing-tag"]),
            cursor: None,
            limit: 100,
        },
    )
    .await
    .unwrap();
    assert!(!missing_tag
        .items
        .iter()
        .any(|entry| { entry.item_id == memory_id && entry.state == TimelineState::Superseded }));
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
        candidate_note_ids: Vec::new(),
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
            candidate_note_ids: Vec::new(),
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
            candidate_note_ids: Vec::new(),
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
            candidate_note_ids: Vec::new(),
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
