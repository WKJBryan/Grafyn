use super::*;
use crate::models::twin_event::{
    CausalStream, Governance, NoteChangeKind, NoteChanged, TwinEventPayload, Visibility,
};
use chrono::{TimeZone, Utc};
use std::sync::Arc;
use tempfile::tempdir;

fn draft(label: &str) -> TwinEventDraft {
    TwinEventDraft {
        actor_id: None,
        causal_parents: Vec::new(),
        recorded_at: Utc.with_ymd_and_hms(2026, 8, 30, 1, 0, 0).unwrap(),
        observed_at: Utc.with_ymd_and_hms(2026, 8, 30, 1, 0, 0).unwrap(),
        occurred_at: None,
        valid_from: None,
        valid_to: None,
        supersedes: Vec::new(),
        reinforces: Vec::new(),
        context: Default::default(),
        evidence: Vec::new(),
        governance: Governance::direct_observation(),
        payload: TwinEventPayload::NoteChanged(NoteChanged {
            note_id: crate::models::twin_event::Identifier::parse(label).unwrap(),
            change: NoteChangeKind::Created,
            content_digest: None,
        }),
    }
}

#[test]
fn writer_identity_is_stable_nonsecret_and_finalizer_chains_one_group() {
    let temp = tempdir().unwrap();
    let store = Arc::new(crate::services::twin_events::TwinEventStore::new(
        temp.path(),
    ));
    store.initialize().unwrap();
    let first_identity = PersistedMutationIdentityProvider::load_or_create(temp.path()).unwrap();
    let reopened = PersistedMutationIdentityProvider::load_or_create(temp.path()).unwrap();
    assert_eq!(first_identity.actor_id(), reopened.actor_id());
    assert_eq!(first_identity.device_id(), reopened.device_id());
    uuid::Uuid::parse_str(first_identity.device_id().as_str()).unwrap();
    let identity_json = std::fs::read_to_string(
        temp.path()
            .join("twin")
            .join("events")
            .join("writer-v1.json"),
    )
    .unwrap();
    assert!(!identity_json.to_ascii_lowercase().contains("secret"));
    assert!(!identity_json.to_ascii_lowercase().contains("key"));

    let finalizer = StoreEventGroupFinalizer::new(store.clone(), Arc::new(first_identity));
    let events = finalizer
        .finalize(
            CausalStream::SyncEligible,
            &[draft("note-a"), draft("note-b")],
        )
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].device_sequence, 1);
    assert_eq!(events[1].device_sequence, 2);
    assert!(events[1].causal_parents.contains(&events[0].event_id));
    assert_eq!(events[0].causal_stream, CausalStream::SyncEligible);

    for event in events {
        store.append(event).unwrap();
    }
    let next = finalizer
        .finalize(CausalStream::SyncEligible, &[draft("note-c")])
        .unwrap();
    assert_eq!(next[0].device_sequence, 3);
}

#[test]
fn concurrent_writer_identity_installers_converge_without_staging_litter() {
    let temp = tempdir().unwrap();
    let store = crate::services::twin_events::TwinEventStore::new(temp.path());
    store.initialize().unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let barrier = barrier.clone();
        let data_path = temp.path().to_path_buf();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            PersistedMutationIdentityProvider::load_or_create(data_path).unwrap()
        }));
    }
    barrier.wait();
    let first = threads.remove(0).join().unwrap();
    let second = threads.remove(0).join().unwrap();
    assert_eq!(first.actor_id(), second.actor_id());
    assert_eq!(first.device_id(), second.device_id());
    assert_eq!(
        std::fs::read_dir(temp.path().join("twin/events/staging/v1"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn restrictive_member_downgrades_the_entire_group_to_local_only() {
    let temp = tempdir().unwrap();
    let store = Arc::new(crate::services::twin_events::TwinEventStore::new(
        temp.path(),
    ));
    store.initialize().unwrap();
    let identity =
        Arc::new(PersistedMutationIdentityProvider::load_or_create(temp.path()).unwrap());
    let finalizer = StoreEventGroupFinalizer::new(store, identity);
    let mut private = draft("private");
    private.governance.visibility = Visibility::LocalOnly;
    private.governance.allowed_uses.sync = false;
    let events = finalizer
        .finalize(CausalStream::SyncEligible, &[draft("shared"), private])
        .unwrap();
    assert!(events
        .iter()
        .all(|event| event.causal_stream == CausalStream::LocalOnly));
}

#[test]
fn coordinator_rejects_overlapping_markdown_canvas_or_twin_roots() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let store = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    store.initialize().unwrap();

    let twin_overlap = MutationCoordinator::new(
        &data,
        data.join("twin"),
        store.clone(),
        Arc::new(NoopMutationLifecycle),
    );
    let twin_error = twin_overlap.err().expect("Twin overlap must fail");
    assert!(
        twin_error.to_string().contains("overlap"),
        "unexpected construction failure: {twin_error}"
    );
    let canvas_overlap = MutationCoordinator::new(
        &data,
        data.join("canvas"),
        store,
        Arc::new(NoopMutationLifecycle),
    );
    let canvas_error = canvas_overlap.err().expect("Canvas overlap must fail");
    assert!(
        canvas_error.to_string().contains("overlap"),
        "unexpected construction failure: {canvas_error}"
    );
}

#[test]
fn rejected_overlapping_retarget_keeps_old_root_and_lease_authoritative() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();

    assert!(coordinator
        .retarget_markdown_root(&data.join("canvas"))
        .is_err());
    let commit = coordinator
        .commit_local(
            CausalStream::SyncEligible,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "still-old.md",
                "old root",
            )],
            vec![draft("still-old")],
        )
        .unwrap();
    assert_eq!(commit.events.len(), 1);
    assert_eq!(
        std::fs::read_to_string(vault.join("still-old.md")).unwrap(),
        "old root"
    );
    assert!(!data.join("canvas").join("still-old.md").exists());
}

#[test]
fn coordinated_event_only_group_appends_distinct_primitive_assessments() {
    use crate::models::twin_event::{
        BoundedContent, DecisionRecorded, Identifier, PrimitiveDecisionAssessmentPayload,
    };
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(crate::services::twin_events::TwinEventStore::new(
        temp.path(),
    ));
    store.initialize().unwrap();
    let coordinator = MutationCoordinator::new(
        temp.path(),
        &vault,
        store.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let payload = |field: &str| {
        let mut assessment = PrimitiveDecisionAssessmentPayload::default();
        let value = Some(BoundedContent::parse(format!("{field} value")).unwrap());
        match field {
            "stakes" => assessment.stakes = value,
            "reversibility" => assessment.reversibility = value,
            _ => unreachable!(),
        }
        crate::models::twin_event::TwinEventPayload::DecisionRecorded(DecisionRecorded {
            decision_id: Identifier::parse("primitive-decision").unwrap(),
            decision: BoundedContent::parse("choose").unwrap(),
            options: vec![BoundedContent::parse("a").unwrap()],
            stakes: None,
            initial_leaning: None,
            review_date: None,
            primitive_assessment: assessment,
        })
    };
    let mut first = draft("placeholder-a");
    first.payload = payload("stakes");
    let mut second = draft("placeholder-b");
    second.payload = payload("reversibility");

    let committed = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("legacy_twin").unwrap(),
            Vec::new(),
            vec![first, second],
        )
        .unwrap();
    assert_eq!(committed.events.len(), 2);
    assert_ne!(committed.events[0].event_id, committed.events[1].event_id);
    assert_eq!(store.ordered_events().unwrap().len(), 2);
}

#[test]
fn authority_mutations_invalidate_ready_before_journal_stage_but_canvas_layout_does_not() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    coordinator.require_namespace_ready().unwrap();
    let before = coordinator.current_authority_token().unwrap();

    let canvas_commit = coordinator
        .apply_nonlocal(
            MutationOrigin::Recovery,
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::CanvasJson,
                "layout.json",
                "{}",
            )],
        )
        .unwrap();
    assert!(canvas_commit.events.is_empty());
    coordinator.require_namespace_ready().unwrap();

    coordinator.fail_once_at(MutationFaultPoint::AfterStage);
    let commit = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "authority.md",
                "changed",
            )],
            vec![draft("authority")],
        )
        .expect("the exact staged authority mutation must converge in-call");
    assert!(commit.postcommit_warning);
    assert!(coordinator.require_namespace_ready().is_err());
    assert_eq!(
        std::fs::read_to_string(vault.join("authority.md")).unwrap(),
        "changed"
    );
    let staged = coordinator.current_authority_token().unwrap();
    assert_eq!(staged.authority_generation, before.authority_generation + 1);

    assert_eq!(coordinator.recover_pending().unwrap(), 0);
    assert!(coordinator.require_namespace_ready().is_err());
    assert_eq!(coordinator.current_authority_token().unwrap(), staged);
}

#[test]
fn commit_returns_the_exact_post_mutation_authority_token() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();

    let canvas = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("canvas").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::CanvasJson,
                "layout.json",
                "{}",
            )],
            Vec::new(),
        )
        .unwrap();
    assert!(canvas.authority_token.is_none());

    let authority = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "token.md",
                "changed",
            )],
            Vec::new(),
        )
        .unwrap();
    assert_eq!(
        authority.authority_token.as_ref(),
        Some(&coordinator.current_authority_token().unwrap())
    );

    let recovery = crate::services::twin_events::EventRecorder::commit_mutation(
        &coordinator,
        MutationOrigin::Recovery,
        CausalStream::LocalOnly,
        crate::models::twin_event::SourceChannel::parse("recovery").unwrap(),
        vec![crate::services::twin_events::TargetMutation::put(
            crate::services::twin_events::TargetKind::Markdown,
            "recovery-token.md",
            "recovered",
        )],
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        recovery.authority_token.as_ref(),
        Some(&coordinator.current_authority_token().unwrap())
    );
}

#[test]
fn replay_failure_after_authority_preserves_the_exact_non_retryable_commit() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();

    coordinator.fail_next_replays_before_targets(2);
    let error = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "pending.md",
                "pending",
            )],
            vec![draft("pending")],
        )
        .expect_err("authority-advanced recovery must not look retryable");

    let MutationError::AuthorityAdvanced {
        authority_token,
        target_aborted,
        ..
    } = &error
    else {
        panic!("expected exact authority-advanced outcome, got {error:?}");
    };
    assert!(!target_aborted);
    assert_eq!(
        authority_token,
        &coordinator.current_authority_token().unwrap()
    );
    assert_eq!(
        error.authority_advanced_commit().unwrap().authority_token,
        Some(authority_token.clone())
    );
    assert!(!vault.join("pending.md").exists());
    assert_eq!(coordinator.pending_count().unwrap(), 1);
}

#[test]
fn planned_mutation_rejects_a_stale_expected_authority_before_staging() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let stale = coordinator.current_authority_token().unwrap();
    let peer_commit = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "peer.md",
                "peer",
            )],
            Vec::new(),
        )
        .unwrap();
    assert!(peer_commit.authority_token.is_some());

    let mut plan = Some(
        MutationPlan::new(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("canvas").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::CanvasJson,
                "stale.json",
                "{}",
            )],
            Vec::new(),
        )
        .expecting_authority(stale),
    );
    let error = coordinator
        .commit_planned(MutationOrigin::Local, &mut || Ok(plan.take()))
        .unwrap_err();
    assert!(error.to_string().contains("authority changed"));
    assert!(!data.join("canvas/stale.json").exists());
    assert_eq!(
        std::fs::read_dir(data.join("twin/mutations/pending/v1"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn retained_read_guard_drift_before_authority_aborts_without_wal_or_effect() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("guard.md"), b"guard-before").unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let before = coordinator.current_authority_token().unwrap();
    let guard_digest = crate::services::twin_events::digest_bytes(b"guard-before");
    let mut plan = Some(
        MutationPlan::new(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
            vec![
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "guard.md",
                    "guard-before",
                )
                .expecting(crate::services::twin_events::BeforeImage::Sha256(
                    guard_digest,
                ))
                .retaining_exact_precondition(),
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::OverlayJson,
                    "guard.json",
                    "{\"bound\":true}",
                ),
            ],
            Vec::new(),
        )
        .expecting_authority(before.clone())
        .retaining_commit_receipt(),
    );
    let mut mutation_id = None;
    let mut prepared = |intent: &crate::services::twin_events::MutationIntentV1| {
        mutation_id = Some(intent.mutation_id.clone());
        std::fs::write(vault.join("guard.md"), b"guard-edited").unwrap();
        Ok(())
    };
    let result = coordinator.commit_planned_with_hooks(
        MutationOrigin::Local,
        &mut || Ok(plan.take()),
        &mut prepared,
        &mut |_| Ok(()),
    );

    assert!(matches!(
        result,
        Err(MutationError::AbortedPrecondition {
            authority_advanced: false,
            ..
        })
    ));
    assert_eq!(coordinator.current_authority_token().unwrap(), before);
    assert_eq!(coordinator.pending_count().unwrap(), 1);
    assert!(!coordinator
        .current_namespace_path()
        .unwrap()
        .join("vault_migration/overlay/notes/guard.json")
        .exists());
    assert_eq!(
        std::fs::read_dir(data.join("twin/mutations/receipts/v1"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        std::fs::read(vault.join("guard.md")).unwrap(),
        b"guard-edited"
    );
    let guard = coordinator.begin_root_transition().unwrap();
    let mutation_id = mutation_id.expect("prepared owner mutation ID");
    assert!(matches!(
        guard
            .classify_witnessed_mutation(
                &mutation_id,
                &before,
                crate::services::twin_events::TargetKind::OverlayJson,
                "guard.json",
                &crate::services::twin_events::BeforeImage::Absent,
                &crate::services::twin_events::BeforeImage::Sha256(
                    crate::services::twin_events::digest_bytes(b"{\"bound\":true}"),
                ),
            )
            .unwrap(),
        WitnessedMutationRecovery::Aborted
    ));
    guard
        .consume_witnessed_mutation_receipt(&mutation_id)
        .unwrap();
    drop(guard);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
}

#[test]
fn restart_recovery_resolves_drifted_guard_with_all_writes_before_in_one_pass() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("guard.md"), b"guard-before").unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator = MutationCoordinator::new(
        &data,
        &vault,
        store.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    let process_lock = coordinator.finalizer.acquire_coordinator_lock().unwrap();
    let intent = coordinator
        .prepare_intent(
            &process_lock,
            MutationOrigin::Local,
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
            vec![
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "guard.md",
                    "guard-before",
                )
                .expecting(crate::services::twin_events::BeforeImage::Sha256(
                    crate::services::twin_events::digest_bytes(b"guard-before"),
                ))
                .retaining_exact_precondition(),
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::OverlayJson,
                    "guard.json",
                    "{\"bound\":true}",
                ),
            ],
            Vec::new(),
            true,
        )
        .unwrap()
        .unwrap();
    let lease = coordinator.root_lease.lock().unwrap().clone();
    let advanced =
        crate::services::vault_namespace::advance_authority_locked(&data, &lease, &process_lock)
            .unwrap();
    assert_eq!(
        advanced.authority_generation,
        expected.authority_generation + 1
    );
    coordinator.journal.stage(&process_lock, &intent).unwrap();
    let mutation_id = intent.mutation_id.clone();
    process_lock.unlock().unwrap();
    std::fs::write(vault.join("guard.md"), b"guard-edited").unwrap();
    drop(coordinator);

    let restarted =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    assert_eq!(restarted.recover_pending().unwrap(), 1);
    assert_eq!(restarted.pending_count().unwrap(), 1);
    assert!(!restarted
        .current_namespace_path()
        .unwrap()
        .join("vault_migration/overlay/notes/guard.json")
        .exists());
    assert_eq!(
        std::fs::read_dir(data.join("twin/mutations/receipts/v1"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        std::fs::read(vault.join("guard.md")).unwrap(),
        b"guard-edited"
    );
    let guard = restarted.begin_root_transition().unwrap();
    let recovered = guard
        .classify_witnessed_mutation(
            &mutation_id,
            &expected,
            crate::services::twin_events::TargetKind::OverlayJson,
            "guard.json",
            &crate::services::twin_events::BeforeImage::Absent,
            &crate::services::twin_events::BeforeImage::Sha256(
                crate::services::twin_events::digest_bytes(b"{\"bound\":true}"),
            ),
        )
        .unwrap();
    let WitnessedMutationRecovery::AbortedAfterAuthority(commit) = recovered else {
        panic!("post-authority guard abort lost its exact durable proof")
    };
    assert_eq!(
        commit.authority_token.unwrap().authority_generation,
        expected.authority_generation + 1
    );
    guard
        .consume_witnessed_mutation_receipt(&mutation_id)
        .unwrap();
    drop(guard);
    assert_eq!(restarted.pending_count().unwrap(), 0);
    assert_eq!(restarted.recover_pending().unwrap(), 0);
}

#[test]
fn postauthority_guard_abort_survives_fault_before_wal_promotion() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("guard.md"), b"guard-before").unwrap();
    let events = Arc::new(TwinEventStore::new(&data));
    events.initialize().unwrap();
    let coordinator = Arc::new(
        MutationCoordinator::new(
            &data,
            &vault,
            events.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let expected = coordinator.current_authority_token().unwrap();
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    coordinator.pause_after_authority_advance_once(entered.clone(), resume.clone());
    coordinator.fail_once_at(MutationFaultPoint::AfterPostAuthorityAbortProof);
    let mutation_id = Arc::new(std::sync::Mutex::new(None));
    let worker_id = mutation_id.clone();
    let worker = coordinator.clone();
    let worker_expected = expected.clone();
    let owner = std::thread::spawn(move || {
        let mut plan = Some(
            MutationPlan::new(
                CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
                vec![
                    crate::services::twin_events::TargetMutation::put(
                        crate::services::twin_events::TargetKind::Markdown,
                        "guard.md",
                        "guard-before",
                    )
                    .expecting(crate::services::twin_events::BeforeImage::Sha256(
                        crate::services::twin_events::digest_bytes(b"guard-before"),
                    ))
                    .retaining_exact_precondition(),
                    crate::services::twin_events::TargetMutation::put(
                        crate::services::twin_events::TargetKind::OverlayJson,
                        "guard.json",
                        "{\"bound\":true}",
                    ),
                ],
                Vec::new(),
            )
            .expecting_authority(worker_expected)
            .retaining_commit_receipt(),
        );
        worker.commit_planned_with_hooks(
            MutationOrigin::Local,
            &mut || Ok(plan.take()),
            &mut |intent| {
                *worker_id.lock().unwrap() = Some(intent.mutation_id.clone());
                Ok(())
            },
            &mut |_| Ok(()),
        )
    });

    entered.wait();
    std::fs::write(vault.join("guard.md"), b"guard-edited").unwrap();
    resume.wait();
    assert!(owner.join().unwrap().is_err());
    let mutation_id = mutation_id.lock().unwrap().clone().unwrap();
    assert_eq!(coordinator.pending_count().unwrap(), 1);
    assert!(!coordinator
        .current_namespace_path()
        .unwrap()
        .join("vault_migration/overlay/notes/guard.json")
        .exists());
    assert!(events.ordered_events().unwrap().is_empty());
    drop(coordinator);

    let restarted = MutationCoordinator::new(
        &data,
        &vault,
        events.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    assert_eq!(restarted.recover_pending().unwrap(), 0);
    assert_eq!(restarted.pending_count().unwrap(), 1);
    assert!(events.ordered_events().unwrap().is_empty());
    assert!(!restarted
        .current_namespace_path()
        .unwrap()
        .join("vault_migration/overlay/notes/guard.json")
        .exists());
    let guard = restarted.begin_root_transition().unwrap();
    assert!(matches!(
        guard
            .classify_witnessed_mutation(
                &mutation_id,
                &expected,
                crate::services::twin_events::TargetKind::OverlayJson,
                "guard.json",
                &crate::services::twin_events::BeforeImage::Absent,
                &crate::services::twin_events::BeforeImage::Sha256(
                    crate::services::twin_events::digest_bytes(b"{\"bound\":true}"),
                ),
            )
            .unwrap(),
        WitnessedMutationRecovery::AbortedAfterAuthority(_)
    ));
    guard
        .consume_witnessed_mutation_receipt(&mutation_id)
        .unwrap();
    drop(guard);
    assert_eq!(restarted.pending_count().unwrap(), 0);
}

#[test]
fn restart_transitions_a_prepared_postauthority_guard_drift_to_aborted() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("guard.md"), b"guard-before").unwrap();
    let events = Arc::new(TwinEventStore::new(&data));
    events.initialize().unwrap();
    let coordinator = MutationCoordinator::new(
        &data,
        &vault,
        events.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    let process_lock = coordinator.finalizer.acquire_coordinator_lock().unwrap();
    let intent = coordinator
        .prepare_intent(
            &process_lock,
            MutationOrigin::Local,
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
            vec![
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "guard.md",
                    "guard-before",
                )
                .expecting(crate::services::twin_events::BeforeImage::Sha256(
                    crate::services::twin_events::digest_bytes(b"guard-before"),
                ))
                .retaining_exact_precondition(),
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::OverlayJson,
                    "guard.json",
                    "{\"bound\":true}",
                ),
            ],
            Vec::new(),
            true,
        )
        .unwrap()
        .unwrap();
    coordinator
        .journal
        .stage_preauthority(&process_lock, &expected, &intent)
        .unwrap();
    let lease = coordinator.root_lease.lock().unwrap().clone();
    crate::services::vault_namespace::advance_authority_locked(&data, &lease, &process_lock)
        .unwrap();
    std::fs::write(vault.join("guard.md"), b"guard-edited").unwrap();
    process_lock.unlock().unwrap();
    let mutation_id = intent.mutation_id.clone();
    drop(coordinator);

    let restarted = MutationCoordinator::new(
        &data,
        &vault,
        events.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    assert_eq!(restarted.pending_count().unwrap(), 1);
    assert!(events.ordered_events().unwrap().is_empty());
    assert!(!restarted
        .current_namespace_path()
        .unwrap()
        .join("vault_migration/overlay/notes/guard.json")
        .exists());
    let guard = restarted.begin_root_transition().unwrap();
    assert!(matches!(
        guard
            .classify_witnessed_mutation(
                &mutation_id,
                &expected,
                crate::services::twin_events::TargetKind::OverlayJson,
                "guard.json",
                &crate::services::twin_events::BeforeImage::Absent,
                &crate::services::twin_events::BeforeImage::Sha256(
                    crate::services::twin_events::digest_bytes(b"{\"bound\":true}"),
                ),
            )
            .unwrap(),
        WitnessedMutationRecovery::AbortedAfterAuthority(_)
    ));
    guard
        .consume_witnessed_mutation_receipt(&mutation_id)
        .unwrap();
    drop(guard);
    assert_eq!(restarted.pending_count().unwrap(), 0);
}

#[test]
fn postauthority_guard_abort_survives_fault_before_wal_cleanup() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("guard.md"), b"guard-before").unwrap();
    let events = Arc::new(TwinEventStore::new(&data));
    events.initialize().unwrap();
    let coordinator = MutationCoordinator::new(
        &data,
        &vault,
        events.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    let process_lock = coordinator.finalizer.acquire_coordinator_lock().unwrap();
    let intent = coordinator
        .prepare_intent(
            &process_lock,
            MutationOrigin::Local,
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
            vec![
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "guard.md",
                    "guard-before",
                )
                .expecting(crate::services::twin_events::BeforeImage::Sha256(
                    crate::services::twin_events::digest_bytes(b"guard-before"),
                ))
                .retaining_exact_precondition(),
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::OverlayJson,
                    "guard.json",
                    "{\"bound\":true}",
                ),
            ],
            Vec::new(),
            true,
        )
        .unwrap()
        .unwrap();
    coordinator
        .journal
        .stage_preauthority(&process_lock, &expected, &intent)
        .unwrap();
    let lease = coordinator.root_lease.lock().unwrap().clone();
    let advanced =
        crate::services::vault_namespace::advance_authority_locked(&data, &lease, &process_lock)
            .unwrap();
    assert_eq!(
        advanced.authority_generation,
        expected.authority_generation + 1
    );
    let marker = coordinator
        .journal
        .preauthority_for(&process_lock, &intent.mutation_id)
        .unwrap()
        .unwrap();
    coordinator
        .journal
        .promote_preauthority(&process_lock, &marker)
        .unwrap();
    std::fs::write(vault.join("guard.md"), b"guard-edited").unwrap();
    coordinator.fail_once_at(MutationFaultPoint::AfterPostAuthorityAbortProof);
    assert!(matches!(
        coordinator.replay_intent_locked(&process_lock, &intent, true, true),
        Err(MutationError::AuthorityAdvanced { .. })
    ));
    process_lock.unlock().unwrap();
    assert_eq!(coordinator.pending_count().unwrap(), 2);
    let mutation_id = intent.mutation_id.clone();
    drop(coordinator);

    let restarted = MutationCoordinator::new(
        &data,
        &vault,
        events.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    assert_eq!(restarted.pending_count().unwrap(), 1);
    assert_eq!(restarted.recover_pending().unwrap(), 0);
    assert!(events.ordered_events().unwrap().is_empty());
    assert!(!restarted
        .current_namespace_path()
        .unwrap()
        .join("vault_migration/overlay/notes/guard.json")
        .exists());
    let guard = restarted.begin_root_transition().unwrap();
    assert!(matches!(
        guard
            .classify_witnessed_mutation(
                &mutation_id,
                &expected,
                crate::services::twin_events::TargetKind::OverlayJson,
                "guard.json",
                &crate::services::twin_events::BeforeImage::Absent,
                &crate::services::twin_events::BeforeImage::Sha256(
                    crate::services::twin_events::digest_bytes(b"{\"bound\":true}"),
                ),
            )
            .unwrap(),
        WitnessedMutationRecovery::AbortedAfterAuthority(_)
    ));
    guard
        .consume_witnessed_mutation_receipt(&mutation_id)
        .unwrap();
    drop(guard);
    assert_eq!(restarted.pending_count().unwrap(), 0);
}

#[test]
fn recovery_ignores_late_guard_drift_after_one_write_and_finishes_once() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("guard.md"), b"guard-before").unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let process_lock = coordinator.finalizer.acquire_coordinator_lock().unwrap();
    let intent = coordinator
        .prepare_intent(
            &process_lock,
            MutationOrigin::Local,
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
            vec![
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "guard.md",
                    "guard-before",
                )
                .expecting(crate::services::twin_events::BeforeImage::Sha256(
                    crate::services::twin_events::digest_bytes(b"guard-before"),
                ))
                .retaining_exact_precondition(),
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::OverlayJson,
                    "first.json",
                    "{\"first\":true}",
                ),
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::OverlayJson,
                    "second.json",
                    "{\"second\":true}",
                ),
            ],
            Vec::new(),
            true,
        )
        .unwrap()
        .unwrap();
    let lease = coordinator.root_lease.lock().unwrap().clone();
    crate::services::vault_namespace::advance_authority_locked(&data, &lease, &process_lock)
        .unwrap();
    coordinator.journal.stage(&process_lock, &intent).unwrap();
    process_lock.unlock().unwrap();
    let first = intent
        .targets
        .iter()
        .find(|target| target.relative_key == "first.json")
        .unwrap();
    coordinator.apply_intent_target(first).unwrap();
    std::fs::write(vault.join("guard.md"), b"guard-edited-after-write").unwrap();

    assert_eq!(coordinator.recover_pending().unwrap(), 1);
    let namespace = coordinator.current_namespace_path().unwrap();
    assert_eq!(
        std::fs::read_to_string(namespace.join("vault_migration/overlay/notes/first.json"))
            .unwrap(),
        "{\"first\":true}"
    );
    assert_eq!(
        std::fs::read_to_string(namespace.join("vault_migration/overlay/notes/second.json"))
            .unwrap(),
        "{\"second\":true}"
    );
    assert_eq!(
        std::fs::read(vault.join("guard.md")).unwrap(),
        b"guard-edited-after-write"
    );
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(
        std::fs::read_dir(data.join("twin/mutations/receipts/v1"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(coordinator.recover_pending().unwrap(), 0);
}

#[test]
fn retained_exact_guards_commit_governed_events_in_the_same_intent() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("guard.md"), b"guard-before").unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let before = coordinator.current_authority_token().unwrap();
    let mut plan = Some(
        MutationPlan::new(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
            vec![
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "guard.md",
                    "guard-before",
                )
                .expecting(crate::services::twin_events::BeforeImage::Sha256(
                    crate::services::twin_events::digest_bytes(b"guard-before"),
                ))
                .retaining_exact_precondition(),
                crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::OverlayJson,
                    "guard.json",
                    "{}",
                ),
            ],
            vec![draft("guard-event")],
        )
        .expecting_authority(before.clone())
        .retaining_commit_receipt(),
    );
    let commit = coordinator
        .commit_planned(MutationOrigin::Local, &mut || Ok(plan.take()))
        .unwrap();
    assert_eq!(commit.events.len(), 1);
    assert_eq!(
        commit
            .authority_token
            .as_ref()
            .unwrap()
            .authority_generation,
        before.authority_generation + 1
    );
    assert_eq!(
        std::fs::read(vault.join("guard.md")).unwrap(),
        b"guard-before"
    );
    let namespace = coordinator.current_namespace_path().unwrap();
    assert_eq!(
        std::fs::read_to_string(namespace.join("vault_migration/overlay/notes/guard.json"))
            .unwrap(),
        "{}"
    );
    assert_eq!(coordinator.pending_count().unwrap(), 0);
}

#[test]
fn retained_receipt_failure_after_exact_effect_returns_committed_warning() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    let receipts = data.join("twin/mutations/receipts/v1");
    let mut plan = Some(
        MutationPlan::new(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "receipt-warning.md",
                "committed",
            )],
            Vec::new(),
        )
        .expecting_authority(expected)
        .retaining_commit_receipt(),
    );
    let mut prepared = |_: &crate::services::twin_events::MutationIntentV1| {
        std::fs::remove_dir(&receipts).unwrap();
        std::fs::write(&receipts, b"block receipt retention").unwrap();
        Ok(())
    };
    let mut committed_called = false;
    let commit = coordinator
        .commit_planned_with_hooks(
            MutationOrigin::Local,
            &mut || Ok(plan.take()),
            &mut prepared,
            &mut |_| {
                committed_called = true;
                Ok(())
            },
        )
        .expect("an exact durable effect must not escape as retryable failure");

    assert!(commit.postcommit_warning);
    assert!(commit.authority_token.is_some());
    assert!(!committed_called);
    assert_eq!(
        std::fs::read_to_string(vault.join("receipt-warning.md")).unwrap(),
        "committed"
    );
    assert_eq!(
        std::fs::read_dir(data.join("twin/mutations/pending/v1"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn durability_classifier_error_does_not_claim_a_committed_result() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    let receipts = data.join("twin/mutations/receipts/v1");
    coordinator.fail_once_at(MutationFaultPoint::BeforeDurabilityClassification);
    let mut plan = Some(
        MutationPlan::new(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "classifier-error.md",
                "committed-but-unclassified",
            )],
            Vec::new(),
        )
        .expecting_authority(expected)
        .retaining_commit_receipt(),
    );
    let mut prepared = |_: &crate::services::twin_events::MutationIntentV1| {
        std::fs::remove_dir(&receipts).unwrap();
        std::fs::write(&receipts, b"block receipt retention").unwrap();
        Ok(())
    };
    let result = coordinator.commit_planned_with_hooks(
        MutationOrigin::Local,
        &mut || Ok(plan.take()),
        &mut prepared,
        &mut |_| Ok(()),
    );

    assert!(result.is_err());
    assert_eq!(
        std::fs::read_to_string(vault.join("classifier-error.md")).unwrap(),
        "committed-but-unclassified"
    );
    assert_eq!(
        std::fs::read_dir(data.join("twin/mutations/pending/v1"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn overlay_target_recovery_uses_the_staged_generation_once() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator = MutationCoordinator::new(
        &data,
        &vault,
        store.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let before = coordinator.current_authority_token().unwrap();
    coordinator.fail_once_at(MutationFaultPoint::AfterStage);
    let commit = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::OverlayJson,
                "captured.json",
                "{}",
            )],
            Vec::new(),
        )
        .expect("the exact staged overlay mutation must converge in-call");
    assert!(commit.postcommit_warning);
    let staged = coordinator.current_authority_token().unwrap();
    assert_eq!(staged.authority_generation, before.authority_generation + 1);
    let overlay = crate::services::vault_namespace::scoped_data_path(&data, &staged.root_scope)
        .join("vault_migration/overlay/notes/captured.json");
    assert_eq!(std::fs::read_to_string(&overlay).unwrap(), "{}");
    assert_eq!(coordinator.current_authority_token().unwrap(), staged);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(coordinator.recover_pending().unwrap(), 0);
    assert_eq!(coordinator.current_authority_token().unwrap(), staged);

    drop(coordinator);
    let restarted =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    assert_eq!(restarted.recover_pending().unwrap(), 0);
    assert_eq!(restarted.current_authority_token().unwrap(), staged);
    assert_eq!(std::fs::read_to_string(overlay).unwrap(), "{}");
}

#[test]
fn ordinary_authority_change_recovers_after_advance_before_wal() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator = MutationCoordinator::new(
        &data,
        &vault,
        store.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let before = coordinator.current_authority_token().unwrap();

    coordinator.fail_once_at(MutationFaultPoint::AfterAuthorityAdvance);
    assert!(coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "false-dirty.md",
                "not staged",
            )],
            vec![draft("false-dirty")],
        )
        .is_err());
    let advanced = coordinator.current_authority_token().unwrap();
    assert_eq!(
        advanced.authority_generation,
        before.authority_generation + 1
    );
    assert!(coordinator.require_namespace_ready().is_err());
    assert_eq!(coordinator.pending_count().unwrap(), 1);
    assert!(!vault.join("false-dirty.md").exists());
    assert_eq!(coordinator.recover_pending().unwrap(), 1);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(
        std::fs::read_to_string(vault.join("false-dirty.md")).unwrap(),
        "not staged"
    );

    drop(coordinator);
    let restarted =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    assert_eq!(restarted.current_authority_token().unwrap(), advanced);
    assert_eq!(restarted.recover_pending().unwrap(), 0);
    assert_eq!(
        std::fs::read_to_string(vault.join("false-dirty.md")).unwrap(),
        "not staged"
    );
}

#[test]
fn ordinary_preauthority_marker_aborts_before_a_peer_advances() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let before = coordinator.current_authority_token().unwrap();

    coordinator.fail_once_at(MutationFaultPoint::AfterPreAuthorityMarker);
    assert!(coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "aborted-owner.md",
                "must not be written",
            )],
            vec![draft("aborted-owner")],
        )
        .is_err());
    assert_eq!(coordinator.current_authority_token().unwrap(), before);
    assert_eq!(coordinator.pending_count().unwrap(), 1);

    let peer = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "peer-after-abort.md",
                "peer",
            )],
            Vec::new(),
        )
        .unwrap();
    assert_eq!(
        peer.authority_token.unwrap().authority_generation,
        before.authority_generation + 1
    );
    assert!(!vault.join("aborted-owner.md").exists());
    assert_eq!(
        std::fs::read_to_string(vault.join("peer-after-abort.md")).unwrap(),
        "peer"
    );
    assert_eq!(coordinator.pending_count().unwrap(), 0);
}

#[test]
fn post_authority_owner_replays_before_a_later_peer_advances() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    let target_key = "prepared-before-wal.md";
    let target_bytes = b"not staged";
    let mut plan = Some(
        MutationPlan::new(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                target_key,
                std::str::from_utf8(target_bytes).unwrap(),
            )],
            Vec::new(),
        )
        .expecting_authority(expected.clone())
        .retaining_commit_receipt(),
    );
    let mut mutation_id = None;
    coordinator.fail_once_at(MutationFaultPoint::AfterAuthorityAdvance);
    let result = coordinator.commit_planned_with_hooks(
        MutationOrigin::Local,
        &mut || Ok(plan.take()),
        &mut |intent| {
            mutation_id = Some(intent.mutation_id.clone());
            Ok(())
        },
        &mut |_| Ok(()),
    );
    assert!(matches!(
        result,
        Err(MutationError::AuthorityAdvanced { .. })
    ));
    assert!(!vault.join(target_key).exists());

    let _ = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "later-peer.md",
                "peer",
            )],
            Vec::new(),
        )
        .unwrap();
    assert_eq!(
        coordinator
            .current_authority_token()
            .unwrap()
            .authority_generation,
        expected.authority_generation + 2
    );
    assert_eq!(
        std::fs::read(&vault.join(target_key)).unwrap(),
        target_bytes
    );

    let guard = coordinator.begin_root_transition().unwrap();
    let classification = guard
        .classify_witnessed_mutation(
            &mutation_id.unwrap(),
            &expected,
            crate::services::twin_events::TargetKind::Markdown,
            target_key,
            &crate::services::twin_events::BeforeImage::Absent,
            &crate::services::twin_events::BeforeImage::Sha256(
                crate::services::twin_events::digest_bytes(target_bytes),
            ),
        )
        .unwrap();
    assert!(matches!(
        classification,
        WitnessedMutationRecovery::Committed(_)
    ));
}

#[test]
fn preauthority_owner_is_aborted_before_a_later_peer_advances() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    let target_key = "prepared-before-authority.md";
    let target_bytes = b"owned desired";
    let mut plan = Some(
        MutationPlan::new(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                target_key,
                std::str::from_utf8(target_bytes).unwrap(),
            )],
            Vec::new(),
        )
        .expecting_authority(expected.clone())
        .retaining_commit_receipt(),
    );
    let mut mutation_id = None;
    coordinator.fail_once_at(MutationFaultPoint::AfterPreparedHook);
    let result = coordinator.commit_planned_with_hooks(
        MutationOrigin::Local,
        &mut || Ok(plan.take()),
        &mut |intent| {
            mutation_id = Some(intent.mutation_id.clone());
            Ok(())
        },
        &mut |_| Ok(()),
    );
    assert!(result.is_err());
    assert_eq!(coordinator.current_authority_token().unwrap(), expected);
    assert!(!vault.join(target_key).exists());

    let peer = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                target_key,
                "peer bytes",
            )],
            Vec::new(),
        )
        .unwrap();
    assert_eq!(
        peer.authority_token.unwrap().authority_generation,
        expected.authority_generation + 1
    );
    assert_eq!(
        std::fs::read(&vault.join(target_key)).unwrap(),
        b"peer bytes"
    );

    let mutation_id = mutation_id.unwrap();
    let guard = coordinator.begin_root_transition().unwrap();
    assert!(matches!(
        guard
            .classify_witnessed_mutation(
                &mutation_id,
                &expected,
                crate::services::twin_events::TargetKind::Markdown,
                target_key,
                &crate::services::twin_events::BeforeImage::Absent,
                &crate::services::twin_events::BeforeImage::Sha256(
                    crate::services::twin_events::digest_bytes(target_bytes),
                ),
            )
            .unwrap(),
        WitnessedMutationRecovery::Aborted
    ));
    guard
        .consume_witnessed_mutation_receipt(&mutation_id)
        .unwrap();
    assert!(guard
        .classify_witnessed_mutation(
            &mutation_id,
            &expected,
            crate::services::twin_events::TargetKind::Markdown,
            target_key,
            &crate::services::twin_events::BeforeImage::Absent,
            &crate::services::twin_events::BeforeImage::Sha256(
                crate::services::twin_events::digest_bytes(target_bytes),
            ),
        )
        .is_err());
}

#[test]
fn post_authority_marker_never_adopts_an_unmanaged_after_or_third_image() {
    for unmanaged in [b"owned desired".as_slice(), b"third bytes".as_slice()] {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(&data));
        store.initialize().unwrap();
        let coordinator = Arc::new(
            MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap(),
        );
        let expected = coordinator.current_authority_token().unwrap();
        let entered = Arc::new(std::sync::Barrier::new(2));
        let resume = Arc::new(std::sync::Barrier::new(2));
        coordinator.pause_after_authority_advance_once(entered.clone(), resume.clone());
        let worker = coordinator.clone();
        let expected_worker = expected.clone();
        let mutation_id = Arc::new(std::sync::Mutex::new(None));
        let worker_mutation_id = mutation_id.clone();
        let thread = std::thread::spawn(move || {
            let mut plan = Some(
                MutationPlan::new(
                    CausalStream::LocalOnly,
                    crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
                    vec![crate::services::twin_events::TargetMutation::put(
                        crate::services::twin_events::TargetKind::Markdown,
                        "paused.md",
                        "owned desired",
                    )],
                    Vec::new(),
                )
                .expecting_authority(expected_worker)
                .retaining_commit_receipt(),
            );
            worker.commit_planned_with_hooks(
                MutationOrigin::Local,
                &mut || Ok(plan.take()),
                &mut |intent| {
                    *worker_mutation_id.lock().unwrap() = Some(intent.mutation_id.clone());
                    Ok(())
                },
                &mut |_| Ok(()),
            )
        });
        entered.wait();
        std::fs::write(vault.join("paused.md"), unmanaged).unwrap();
        resume.wait();
        assert!(matches!(
            thread.join().unwrap(),
            Err(MutationError::AuthorityAdvanced { .. })
        ));
        assert_eq!(std::fs::read(vault.join("paused.md")).unwrap(), unmanaged);
        assert_eq!(coordinator.recover_pending().unwrap(), 0);
        assert_eq!(std::fs::read(vault.join("paused.md")).unwrap(), unmanaged);
        assert_eq!(
            coordinator
                .current_authority_token()
                .unwrap()
                .authority_generation,
            expected.authority_generation + 1
        );
        assert_eq!(coordinator.pending_count().unwrap(), 1);
        let mutation_id = mutation_id.lock().unwrap().clone().unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        assert!(matches!(
            guard
                .classify_witnessed_mutation(
                    &mutation_id,
                    &expected,
                    crate::services::twin_events::TargetKind::Markdown,
                    "paused.md",
                    &crate::services::twin_events::BeforeImage::Absent,
                    &crate::services::twin_events::BeforeImage::Sha256(
                        crate::services::twin_events::digest_bytes(b"owned desired"),
                    ),
                )
                .unwrap(),
            WitnessedMutationRecovery::AbortedAfterAuthority(_)
        ));
        guard
            .consume_witnessed_mutation_receipt(&mutation_id)
            .unwrap();
        drop(guard);
        assert_eq!(coordinator.pending_count().unwrap(), 0);
    }
}

#[test]
fn ordinary_expected_before_keeps_idempotent_after_image_elision() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("note.md"), b"desired").unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let authority = coordinator.current_authority_token().unwrap();

    let commit = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "note.md",
                "desired",
            )
            .expecting(crate::services::twin_events::BeforeImage::Sha256(
                crate::services::twin_events::digest_bytes(b"stale-before"),
            ))],
            Vec::new(),
        )
        .unwrap();

    assert!(commit.mutation_id.is_none());
    assert_eq!(coordinator.current_authority_token().unwrap(), authority);
    assert_eq!(std::fs::read(vault.join("note.md")).unwrap(), b"desired");
}

#[test]
fn strict_expected_before_rejects_an_externally_installed_after_image() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("note.md"), b"desired").unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let authority = coordinator.current_authority_token().unwrap();

    let error = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("markdown_migration").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "note.md",
                "desired",
            )
            .expecting(crate::services::twin_events::BeforeImage::Sha256(
                crate::services::twin_events::digest_bytes(b"stale-before"),
            ))
            .checking_expected_before_before_after_elision()],
            Vec::new(),
        )
        .unwrap_err();

    assert!(matches!(error, MutationError::RecoveryConflict(_)));
    assert_eq!(coordinator.current_authority_token().unwrap(), authority);
    assert_eq!(std::fs::read(vault.join("note.md")).unwrap(), b"desired");
}

#[test]
fn witnessed_tombstone_classification_distinguishes_commit_before_and_conflict() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::write(vault.join("delete.md"), b"before").unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let authority = coordinator.current_authority_token().unwrap();
    let before = crate::services::twin_events::BeforeImage::Sha256(
        crate::services::twin_events::digest_bytes(b"before"),
    );
    let desired = crate::services::twin_events::BeforeImage::Absent;
    let mut plan = Some(
        MutationPlan::new(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("markdown_migration").unwrap(),
            vec![crate::services::twin_events::TargetMutation::tombstone(
                crate::services::twin_events::TargetKind::Markdown,
                "delete.md",
            )
            .expecting(before.clone())
            .checking_expected_before_before_after_elision()],
            Vec::new(),
        )
        .expecting_authority(authority.clone())
        .retaining_commit_receipt(),
    );
    let mut mutation_id = None;
    let commit = coordinator
        .commit_planned_with_hooks(
            MutationOrigin::Local,
            &mut || Ok(plan.take()),
            &mut |intent| {
                mutation_id = Some(intent.mutation_id.clone());
                Ok(())
            },
            &mut |_| Err(MutationError::Invalid("simulate witness crash".into())),
        )
        .unwrap();
    assert!(commit.postcommit_warning);
    assert!(!vault.join("delete.md").exists());

    let guard = coordinator.begin_root_transition().unwrap();
    assert!(matches!(
        guard
            .classify_witnessed_mutation(
                mutation_id.as_ref().unwrap(),
                &authority,
                crate::services::twin_events::TargetKind::Markdown,
                "delete.md",
                &before,
                &desired,
            )
            .unwrap(),
        WitnessedMutationRecovery::Committed(_)
    ));
    guard
        .consume_witnessed_mutation_receipt(mutation_id.as_ref().unwrap())
        .unwrap();
    drop(guard);

    std::fs::write(vault.join("unchanged.md"), b"before").unwrap();
    let current = coordinator.current_authority_token().unwrap();
    let absent_id = crate::services::twin_events::digest_bytes(b"not-committed");
    let guard = coordinator.begin_root_transition().unwrap();
    assert!(matches!(
        guard
            .classify_witnessed_mutation(
                &absent_id,
                &current,
                crate::services::twin_events::TargetKind::Markdown,
                "unchanged.md",
                &before,
                &desired,
            )
            .unwrap(),
        WitnessedMutationRecovery::NotCommitted
    ));
    drop(guard);

    std::fs::write(vault.join("unchanged.md"), b"third-state").unwrap();
    let guard = coordinator.begin_root_transition().unwrap();
    assert!(matches!(
        guard.classify_witnessed_mutation(
            &absent_id,
            &current,
            crate::services::twin_events::TargetKind::Markdown,
            "unchanged.md",
            &before,
            &desired,
        ),
        Err(MutationError::RecoveryConflict(_))
    ));
}

#[test]
fn production_desktop_and_mcp_use_coordinated_non_noop_construction() {
    let desktop = include_str!("../../lib.rs");
    let mcp = include_str!("../../mcp.rs");
    assert!(desktop.contains("KnowledgeStore::with_event_recorder"));
    assert!(desktop.contains("CanvasStore::with_event_recorder"));
    assert!(desktop.contains("recover_pending()"));
    assert!(desktop.contains("UnavailableEventRecorder"));
    assert!(mcp.contains("KnowledgeStore::with_event_recorder"));
    let recover = mcp.find("recover_pending()").unwrap();
    let serve = mcp.find(".serve(rmcp::transport::stdio())").unwrap();
    assert!(recover < serve);
    assert!(!mcp.contains("KnowledgeStore::new(vault_path"));
}
