use super::*;
use crate::models::note::{NoteCreate, NoteStatus};
use crate::services::atomic_io::assert_no_tmp_siblings;
use tempfile::tempdir;

fn try_symlink_file(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).unwrap();
        true
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_file(target, link) {
            Ok(()) => true,
            Err(error) if matches!(error.raw_os_error(), Some(5) | Some(1314)) => {
                eprintln!("skipping symlink regression without Windows symlink privilege");
                false
            }
            Err(error) => panic!("failed to create file symlink: {error}"),
        }
    }
}

fn make_note_create(title: &str) -> NoteCreate {
    NoteCreate {
        title: title.to_string(),
        content: format!("Content for {}", title),
        relative_path: None,
        aliases: Vec::new(),
        status: NoteStatus::Draft,
        tags: Vec::new(),
        schema_version: CURRENT_NOTE_SCHEMA_VERSION,
        migration_source: None,
        optimizer_managed: false,
        properties: HashMap::new(),
    }
}

fn run_one_optimizer_write(
    service: &mut VaultOptimizerService,
    store: &mut KnowledgeStore,
    settings: &UserSettings,
) {
    let tick = service
        .prepare_next(store, settings)
        .expect("optimizer prepare should not error");
    match tick {
        OptimizerTick::Pending(pending) => assert!(matches!(
            service
                .apply_pending(store, *pending)
                .expect("optimizer apply should not error"),
            OptimizerMutationResult::Committed { .. }
        )),
        OptimizerTick::RetryFenced(pending) => assert!(matches!(
            service
                .apply_retry_fenced(store, *pending)
                .expect("optimizer retry fence should resume"),
            OptimizerMutationResult::Committed { .. }
        )),
        OptimizerTick::Committed { .. } => {}
        OptimizerTick::NoWrite => panic!("expected an optimizer authority write"),
    }
}

fn make_note(id: &str, title: &str) -> Note {
    let now = Utc::now();
    Note {
        id: id.to_string(),
        title: title.to_string(),
        content: format!("Content of {}", title),
        relative_path: format!("{}.md", id),
        aliases: Vec::new(),
        status: NoteStatus::Draft,
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
        migration_source: None,
        optimizer_managed: false,
        wikilinks: Vec::new(),
        parsed_links: Vec::new(),
        properties: HashMap::new(),
        ..Default::default()
    }
}

fn authority_optimizer_fixture(
    title: &str,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    std::sync::Arc<crate::services::twin_events::MutationCoordinator>,
    KnowledgeStore,
    VaultOptimizerService,
    Note,
) {
    let vault_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
        data_dir.path(),
    ));
    events.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            data_dir.path(),
            vault_dir.path(),
            events,
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let namespace = coordinator.current_namespace_path().unwrap();
    let mut store = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        namespace.clone(),
        coordinator.clone(),
    );
    let note = store.create_note(make_note_create(title)).unwrap();
    let mut service = VaultOptimizerService::try_new(namespace).unwrap();
    service
        .bootstrap_checked(std::slice::from_ref(&note))
        .unwrap();
    (vault_dir, data_dir, coordinator, store, service, note)
}

fn prepare_authority_write(
    service: &mut VaultOptimizerService,
    store: &KnowledgeStore,
    coordinator: &crate::services::twin_events::MutationCoordinator,
) -> PendingOptimizerWrite {
    match service
        .prepare_next_expecting_authority(
            store,
            &UserSettings::default(),
            coordinator.current_authority_token().unwrap(),
        )
        .unwrap()
    {
        OptimizerTick::Pending(pending) => *pending,
        other => panic!("expected a pending authority write, got {other:?}"),
    }
}

#[test]
fn locked_restart_cleans_only_canonical_optimizer_orphan_temps() {
    let data_dir = tempdir().unwrap();
    let service = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
    let change_temp = service.changes_dir.join(format!(".{}.tmp", Uuid::new_v4()));
    let pending_temp = service
        .optimizer_dir
        .join(PENDING_PUBLICATIONS_DIRECTORY)
        .join(format!(".{}.tmp", Uuid::new_v4()));
    std::fs::write(&change_temp, b"fsynced orphan").unwrap();
    std::fs::write(&pending_temp, b"fsynced orphan").unwrap();
    drop(service);

    let restarted = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();

    assert!(!change_temp.exists());
    assert!(!pending_temp.exists());
    assert!(restarted.load_pending_publications().unwrap().is_empty());
}

#[test]
fn noncanonical_optimizer_temp_is_not_deleted_as_an_orphan() {
    let data_dir = tempdir().unwrap();
    let service = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
    let unknown = service.changes_dir.join(".not-a-uuid.tmp");
    std::fs::write(&unknown, b"untrusted file").unwrap();
    drop(service);

    assert!(VaultOptimizerService::try_new(data_dir.path().to_path_buf()).is_err());
    assert!(unknown.exists());
}

#[test]
fn corrupt_optimizer_audits_fail_closed_on_restart() {
    let decisions_dir = tempdir().unwrap();
    let decisions = VaultOptimizerService::try_new(decisions_dir.path().to_path_buf()).unwrap();
    std::fs::write(&decisions.decisions_path, b"{not-json").unwrap();
    drop(decisions);
    assert!(VaultOptimizerService::try_new(decisions_dir.path().to_path_buf()).is_err());

    let events_dir = tempdir().unwrap();
    let events = VaultOptimizerService::try_new(events_dir.path().to_path_buf()).unwrap();
    std::fs::write(&events.events_path, b"{not-json\n").unwrap();
    drop(events);
    assert!(VaultOptimizerService::try_new(events_dir.path().to_path_buf()).is_err());
}

#[test]
fn state_lock_and_protected_io_share_one_retained_root_capability() {
    let original = tempdir().unwrap();
    let replacement = tempdir().unwrap();
    let mut service = VaultOptimizerService::try_new(original.path().to_path_buf()).unwrap();
    service
        .bootstrap_checked(&[make_note("original-note", "Original")])
        .unwrap();
    let mut replacement_service =
        VaultOptimizerService::try_new(replacement.path().to_path_buf()).unwrap();
    replacement_service
        .bootstrap_checked(&[make_note("replacement-note", "Replacement")])
        .unwrap();

    let lock = service.acquire_state_lock().unwrap();
    service.optimizer_dir = replacement_service.optimizer_dir.clone();
    service.reload_from_disk_checked().unwrap();
    lock.unlock().unwrap();

    assert_eq!(service.state.queue.len(), 1);
    assert_eq!(service.state.queue[0].note_id, "original-note");
}

#[test]
fn legacy_queue_job_id_is_stable_across_restarts() {
    let data_dir = tempdir().unwrap();
    let optimizer_dir = data_dir.path().join("vault_migration/optimizer");
    std::fs::create_dir_all(&optimizer_dir).unwrap();
    std::fs::write(
        optimizer_dir.join(QUEUE_KEY),
        serde_json::to_vec_pretty(&serde_json::json!({
            "queue": [{
                "note_id": "legacy-note",
                "reason": "legacy",
                "enqueued_at": "2026-08-30T00:00:00Z",
                "attempts": 0
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let first = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
    let first_id = first.state.queue[0].job_id.clone();
    parse_canonical_uuid(&first_id, "legacy optimizer job ID").unwrap();
    drop(first);
    let second = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();

    assert_eq!(second.state.queue[0].job_id, first_id);
}

#[test]
fn prepared_hook_failure_releases_retry_fence_for_same_process_retry() {
    let (_vault, _data, coordinator, mut store, mut service, _) =
        authority_optimizer_fixture("Prepared Hook Failure");
    let first = prepare_authority_write(&mut service, &store, &coordinator);
    let first_change_id = first.change_id.clone();
    let job_id = first.job.job_id.clone();
    service.fail_next_prepared_publication();

    assert!(matches!(
        service.apply_pending(&mut store, first).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    assert!(service.load_pending_publications().unwrap().is_empty());

    let retry = prepare_authority_write(&mut service, &store, &coordinator);
    assert_eq!(retry.job.job_id, job_id);
    assert_ne!(retry.change_id, first_change_id);
}

#[test]
fn receipt_capacity_failure_releases_retry_fence_for_same_process_retry() {
    let (_vault, data, coordinator, mut store, mut service, _) =
        authority_optimizer_fixture("Receipt Capacity Failure");
    let receipts = data.path().join("twin/mutations/receipts/v1");
    for index in 0..256_u16 {
        std::fs::write(receipts.join(format!("{index:064x}.json")), b"{}").unwrap();
    }
    let first = prepare_authority_write(&mut service, &store, &coordinator);
    let first_change_id = first.change_id.clone();
    let job_id = first.job.job_id.clone();

    assert!(matches!(
        service.apply_pending(&mut store, first).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    assert!(service.load_pending_publications().unwrap().is_empty());

    let retry = prepare_authority_write(&mut service, &store, &coordinator);
    assert_eq!(retry.job.job_id, job_id);
    assert_ne!(retry.change_id, first_change_id);
}

#[test]
fn ambiguous_retry_fence_stage_failure_preserves_adoptable_owner() {
    let (_vault, _data, coordinator, mut store, mut service, note) =
        authority_optimizer_fixture("Retry Fence Stage Failure");
    let first = prepare_authority_write(&mut service, &store, &coordinator);
    let first_change_id = first.change_id.clone();
    let job_id = first.job.job_id.clone();
    service.fail_next_retry_fence_stage_after_write();

    assert!(matches!(
        service.apply_pending(&mut store, first).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    let pending = service.load_pending_publications().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].1.change_id, first_change_id);
    assert_eq!(pending[0].1.phase, OptimizerPublicationPhase::RetryFenced);
    assert_eq!(service.state.queue[0].attempts, 0);
    assert!(!store.overlay_path(&note.id).exists());

    let retry = match service
        .prepare_next_expecting_authority(
            &store,
            &UserSettings::default(),
            coordinator.current_authority_token().unwrap(),
        )
        .unwrap()
    {
        OptimizerTick::RetryFenced(retry) => *retry,
        other => panic!("expected the stable retry fence, got {other:?}"),
    };
    assert_eq!(retry.publication.job.job_id, job_id);
    assert_eq!(retry.publication.change_id, first_change_id);
    assert!(matches!(
        service.apply_retry_fenced(&mut store, retry).unwrap(),
        OptimizerMutationResult::Committed { .. }
    ));
    assert!(store.overlay_path(&note.id).exists());
}

#[test]
fn optimizer_source_snapshot_rejects_oversize_before_witness() {
    let (_vault, _data, coordinator, mut store, mut service, note) =
        authority_optimizer_fixture("Oversize Overlay Source");
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    let authority_before = coordinator.current_authority_token().unwrap();
    let oversized = serde_json::to_vec(&serde_json::json!({
        "tags": ["x".repeat(crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES)]
    }))
    .unwrap();
    std::fs::write(store.overlay_path(&note.id), oversized).unwrap();

    assert!(store.optimizer_overlay_snapshot(&note.id).is_err());
    assert!(matches!(
        service.apply_pending(&mut store, pending).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    assert_eq!(service.state.queue[0].attempts, 1);
    assert_eq!(
        coordinator.current_authority_token().unwrap(),
        authority_before
    );
    assert!(service.load_pending_publications().unwrap().is_empty());
    assert!(service.list_decisions(10).unwrap().is_empty());

    assert!(matches!(
        service
            .prepare_next_expecting_authority(
                &store,
                &UserSettings::default(),
                authority_before.clone(),
            )
            .unwrap(),
        OptimizerTick::NoWrite
    ));
    assert_eq!(service.state.queue[0].attempts, 2);
    assert!(matches!(
        service
            .prepare_next_expecting_authority(
                &store,
                &UserSettings::default(),
                authority_before.clone(),
            )
            .unwrap(),
        OptimizerTick::NoWrite
    ));
    assert!(service.state.queue.is_empty());
    assert_eq!(service.inbox(Some("failed"), 10).unwrap().len(), 1);
    assert!(service.list_decisions(10).unwrap().is_empty());
}

#[test]
fn optimizer_prepare_rejects_oversize_markdown_without_authority_or_witness() {
    let (vault, _data, coordinator, store, mut service, note) =
        authority_optimizer_fixture("Oversize Markdown Source");
    let authority_before = coordinator.current_authority_token().unwrap();
    std::fs::write(
        vault.path().join(&note.relative_path),
        vec![b'x'; crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES + 1],
    )
    .unwrap();

    assert!(matches!(
        service
            .prepare_next_expecting_authority(
                &store,
                &UserSettings::default(),
                authority_before.clone(),
            )
            .unwrap(),
        OptimizerTick::NoWrite
    ));
    assert_eq!(service.state.queue[0].attempts, 1);
    assert_eq!(
        coordinator.current_authority_token().unwrap(),
        authority_before
    );
    assert!(service.load_pending_publications().unwrap().is_empty());
}

#[test]
fn optimizer_apply_rejects_symlinked_markdown_without_reading_outside() {
    let (vault, _data, coordinator, mut store, mut service, note) =
        authority_optimizer_fixture("Symlink Markdown Source");
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    let authority_before = coordinator.current_authority_token().unwrap();
    let outside = vault
        .path()
        .parent()
        .unwrap()
        .join(format!("optimizer-outside-{}.md", Uuid::new_v4()));
    let secret = "outside-markdown-must-not-be-read";
    std::fs::write(&outside, secret).unwrap();
    let markdown = vault.path().join(&note.relative_path);
    std::fs::remove_file(&markdown).unwrap();
    if !try_symlink_file(&outside, &markdown) {
        let _ = std::fs::remove_file(&outside);
        return;
    }

    assert!(matches!(
        service.apply_pending(&mut store, pending).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    assert_eq!(std::fs::read(&outside).unwrap(), secret.as_bytes());
    assert_eq!(service.state.queue[0].attempts, 1);
    assert_eq!(
        coordinator.current_authority_token().unwrap(),
        authority_before
    );
    assert!(service.load_pending_publications().unwrap().is_empty());
    std::fs::remove_file(&markdown).unwrap();
    std::fs::remove_file(&outside).unwrap();
}

#[test]
fn optimizer_apply_rejects_symlinked_overlay_without_reading_outside() {
    let (vault, _data, coordinator, mut store, mut service, note) =
        authority_optimizer_fixture("Symlink Overlay Source");
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    let authority_before = coordinator.current_authority_token().unwrap();
    let outside = vault
        .path()
        .parent()
        .unwrap()
        .join(format!("optimizer-outside-{}.json", Uuid::new_v4()));
    let secret = "outside-overlay-must-not-be-read";
    std::fs::write(&outside, format!(r#"{{"tags":["{secret}"]}}"#)).unwrap();
    let overlay = store.overlay_path(&note.id);
    if overlay.exists() {
        std::fs::remove_file(&overlay).unwrap();
    }
    if !try_symlink_file(&outside, &overlay) {
        let _ = std::fs::remove_file(&outside);
        return;
    }

    assert!(matches!(
        service.apply_pending(&mut store, pending).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        format!(r#"{{"tags":["{secret}"]}}"#)
    );
    assert_eq!(service.state.queue[0].attempts, 1);
    assert_eq!(
        coordinator.current_authority_token().unwrap(),
        authority_before
    );
    assert!(service.load_pending_publications().unwrap().is_empty());
    std::fs::remove_file(&overlay).unwrap();
    std::fs::remove_file(&outside).unwrap();
}

#[test]
fn pending_publication_rejects_cross_field_corruption() {
    let (_vault, _data, coordinator, mut store, mut service, _) =
        authority_optimizer_fixture("Witness Binding");
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    let _ = service.apply_pending(&mut store, pending).unwrap();
    let (_, valid) = service.load_pending_publications().unwrap().remove(0);

    let mut wrong_kind = valid.clone();
    wrong_kind.decision.kind = "other".into();
    assert!(validate_pending_publication(&wrong_kind).is_err());

    let mut wrong_note = valid.clone();
    wrong_note.decision.note_id = Some("other-note".into());
    assert!(validate_pending_publication(&wrong_note).is_err());

    let mut wrong_mode = valid.clone();
    wrong_mode.change.mode = "full_rewrite".into();
    assert!(validate_pending_publication(&wrong_mode).is_err());

    let mut wrong_after = valid;
    match &mut wrong_after.target {
        OptimizerPublicationTarget::Overlay { after_digest, .. }
        | OptimizerPublicationTarget::Markdown { after_digest, .. } => {
            *after_digest = crate::services::twin_events::digest_bytes(b"wrong-after");
        }
    }
    assert!(validate_pending_publication(&wrong_after).is_err());
}

#[test]
fn exact_overlay_target_completes_as_noop_before_retry_fence() {
    let (_vault, _data, coordinator, mut store, mut service, _) =
        authority_optimizer_fixture("Exact Overlay Noop");
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    let snapshot = store
        .optimizer_note_snapshot(&pending.job.note_id)
        .unwrap()
        .unwrap();
    let overlay = optimizer_sidecar_overlay(
        &pending.proposal,
        snapshot.markdown_precondition.relative_path(),
        snapshot.markdown_precondition.expected_digest(),
    );
    std::fs::write(
        store.overlay_path(&pending.job.note_id),
        serde_json::to_string_pretty(&overlay).unwrap(),
    )
    .unwrap();
    let authority_before = coordinator.current_authority_token().unwrap();

    assert!(matches!(
        service.apply_pending(&mut store, pending).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    assert_eq!(
        coordinator.current_authority_token().unwrap(),
        authority_before
    );
    assert!(service.state.queue.is_empty());
    assert!(service.load_pending_publications().unwrap().is_empty());
    assert!(service.list_decisions(10).unwrap().is_empty());
    assert!(service.inbox(None, 10).unwrap().is_empty());

    let restarted =
        VaultOptimizerService::try_new(coordinator.current_namespace_path().unwrap()).unwrap();
    assert!(restarted.state.queue.is_empty());
    assert!(restarted.load_pending_publications().unwrap().is_empty());
}

#[test]
fn externally_satisfied_markdown_target_cleans_retry_fence_as_noop() {
    let (vault, _data, coordinator, mut store, mut service, note) =
        authority_optimizer_fixture("Exact Markdown Noop");
    let settings = UserSettings {
        background_vault_optimizer_edit_mode: "full_rewrite".into(),
        ..UserSettings::default()
    };
    let pending = match service
        .prepare_next_expecting_authority(
            &store,
            &settings,
            coordinator.current_authority_token().unwrap(),
        )
        .unwrap()
    {
        OptimizerTick::Pending(pending) => *pending,
        other => panic!("expected a pending full rewrite, got {other:?}"),
    };
    let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
    let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
    service.pause_after_retry_fence_once(entered.clone(), resume.clone());
    let owner = std::thread::spawn(move || {
        let result = service.apply_pending(&mut store, pending);
        (service, result)
    });

    entered.wait();
    let witness =
        VaultOptimizerService::try_new(coordinator.current_namespace_path().unwrap()).unwrap();
    let publication = witness.load_pending_publications().unwrap().remove(0).1;
    let exact = publication.change.note_after.unwrap();
    let (_, bytes) = KnowledgeStore::canonical_serialized_note_bytes(&exact).unwrap();
    std::fs::write(vault.path().join(&note.relative_path), bytes).unwrap();
    let authority_before = coordinator.current_authority_token().unwrap();
    resume.wait();

    let (service, result) = owner.join().unwrap();
    assert!(matches!(result.unwrap(), OptimizerMutationResult::NoWrite));
    assert_eq!(
        coordinator.current_authority_token().unwrap(),
        authority_before
    );
    assert!(service.state.queue.is_empty());
    assert!(service.load_pending_publications().unwrap().is_empty());
    assert!(service.list_decisions(10).unwrap().is_empty());
    assert!(service.inbox(None, 10).unwrap().is_empty());

    let restarted =
        VaultOptimizerService::try_new(coordinator.current_namespace_path().unwrap()).unwrap();
    assert!(restarted.state.queue.is_empty());
    assert!(restarted.load_pending_publications().unwrap().is_empty());
}

#[test]
fn sidecar_source_edit_before_coordinator_has_no_authority_or_overlay_effect() {
    let (vault, _data, coordinator, mut store, mut service, note) =
        authority_optimizer_fixture("Sidecar Source Guard");
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    let overlay_path = store.overlay_path(&note.id);
    let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
    let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
    service.pause_before_prepared_hook_once(entered.clone(), resume.clone());
    let owner = std::thread::spawn(move || {
        let result = service.apply_pending(&mut store, pending);
        (service, result)
    });

    entered.wait();
    let markdown_path = vault.path().join(&note.relative_path);
    let mut external = std::fs::read_to_string(&markdown_path).unwrap();
    external.push_str("\n\nExternal edit before sidecar commit.\n");
    std::fs::write(&markdown_path, external).unwrap();
    let authority_before = coordinator.current_authority_token().unwrap();
    resume.wait();

    let (service, result) = owner.join().unwrap();
    assert!(matches!(result.unwrap(), OptimizerMutationResult::NoWrite));
    assert_eq!(
        coordinator.current_authority_token().unwrap(),
        authority_before
    );
    assert!(!overlay_path.exists());
    assert!(service.load_pending_publications().unwrap().is_empty());
    assert!(service.list_decisions(10).unwrap().is_empty());
    assert_eq!(service.state.queue[0].attempts, 1);
}

#[test]
fn prepared_sidecar_guard_abort_retires_exact_owner_and_defers_without_authority() {
    let (vault, data, coordinator, mut store, mut service, note) =
        authority_optimizer_fixture("Prepared Sidecar Guard Abort");
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    let job_id = pending.job.job_id.clone();
    let overlay_path = store.overlay_path(&note.id);
    let namespace = coordinator.current_namespace_path().unwrap();
    let authority_before = coordinator.current_authority_token().unwrap();
    coordinator
        .begin_root_transition()
        .unwrap()
        .publish_namespace_ready(&authority_before)
        .unwrap();
    let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
    let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
    service.pause_after_prepared_publication_once(entered.clone(), resume.clone());
    let owner = std::thread::spawn(move || {
        let result = service.apply_pending(&mut store, pending);
        (service, result)
    });

    entered.wait();
    let mut observer = VaultOptimizerService::try_new(namespace.clone()).unwrap();
    let owners = observer.load_pending_publications().unwrap();
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0].1.phase, OptimizerPublicationPhase::Prepared);
    let prepared_change_id = owners[0].1.change_id.clone();
    let prepared_mutation_id = owners[0].1.mutation_id.clone().unwrap();
    assert!(!observer
        .abort_precondition_owner_and_defer(
            &prepared_change_id,
            &job_id,
            "wrong-mutation-id",
            &anyhow::anyhow!("must not retire a different owner"),
            None,
        )
        .unwrap());
    let unchanged_owners = observer.load_pending_publications().unwrap();
    assert_eq!(unchanged_owners.len(), 1);
    assert_eq!(
        unchanged_owners[0].1.mutation_id.as_ref(),
        Some(&prepared_mutation_id)
    );
    assert_eq!(observer.state.queue[0].attempts, 0);
    assert_eq!(
        std::fs::read_dir(data.path().join("twin/mutations/pending/v1"))
            .unwrap()
            .count(),
        0
    );
    let markdown_path = vault.path().join(&note.relative_path);
    let mut external = std::fs::read(&markdown_path).unwrap();
    external.extend_from_slice(b"\nExternal edit after Prepared publication.\n");
    std::fs::write(&markdown_path, &external).unwrap();
    resume.wait();

    let (service, result) = owner.join().unwrap();
    assert!(matches!(result.unwrap(), OptimizerMutationResult::NoWrite));
    assert_eq!(
        coordinator.current_authority_token().unwrap(),
        authority_before
    );
    coordinator.require_namespace_ready().unwrap();
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert!(!overlay_path.exists());
    assert_eq!(std::fs::read(&markdown_path).unwrap(), external);
    assert!(service.load_pending_publications().unwrap().is_empty());
    assert!(service.list_decisions(10).unwrap().is_empty());
    assert!(service.inbox(None, 10).unwrap().is_empty());
    assert_eq!(service.state.queue.len(), 1);
    assert_eq!(service.state.queue[0].job_id, job_id);
    assert_eq!(service.state.queue[0].attempts, 1);
    assert_eq!(
        std::fs::read_dir(data.path().join("twin/mutations/receipts/v1"))
            .unwrap()
            .count(),
        0
    );

    let restarted = VaultOptimizerService::try_new(namespace).unwrap();
    assert!(restarted.load_pending_publications().unwrap().is_empty());
    assert_eq!(restarted.state.queue[0].attempts, 1);
    assert!(!restarted
        .load_pending_publications()
        .unwrap()
        .iter()
        .any(|(_, owner)| owner.mutation_id.as_ref() == Some(&prepared_mutation_id)));
}

#[test]
fn two_prepared_peers_keep_one_stable_job_owner_through_restart() {
    let (vault, _data, coordinator, mut owner_store, mut owner, _note) =
        authority_optimizer_fixture("Single Pending Owner");
    let namespace = coordinator.current_namespace_path().unwrap();
    let mut peer = VaultOptimizerService::try_new(namespace.clone()).unwrap();
    let mut peer_store = KnowledgeStore::with_event_recorder(
        vault.path().to_path_buf(),
        namespace.clone(),
        coordinator.clone(),
    );
    let expected = coordinator.current_authority_token().unwrap();
    let first = prepare_authority_write(&mut owner, &owner_store, &coordinator);
    let second = prepare_authority_write(&mut peer, &peer_store, &coordinator);
    assert_eq!(first.job.job_id, second.job.job_id);
    assert_ne!(first.change_id, second.change_id);
    let first_change_id = first.change_id.clone();
    let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
    let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
    owner.pause_after_retry_fence_once(entered.clone(), resume.clone());
    let owner_thread = std::thread::spawn(move || {
        let result = owner.apply_pending(&mut owner_store, first);
        (owner, result)
    });

    entered.wait();
    assert!(matches!(
        peer.apply_pending(&mut peer_store, second).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    let owners = peer.load_pending_publications().unwrap();
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0].1.change_id, first_change_id);
    assert_eq!(peer.state.queue[0].attempts, 0);
    assert!(peer.inbox(Some("failed"), 10).unwrap().is_empty());
    resume.wait();

    let (owner, result) = owner_thread.join().unwrap();
    assert!(matches!(
        result.unwrap(),
        OptimizerMutationResult::Committed { .. }
    ));
    drop(owner);
    let mut restarted = VaultOptimizerService::try_new(namespace).unwrap();
    let owners = restarted.load_pending_publications().unwrap();
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0].1.change_id, first_change_id);

    let state_lock = restarted.acquire_state_lock().unwrap();
    restarted.reload_from_disk_checked().unwrap();
    {
        let guard = coordinator.begin_root_transition().unwrap();
        restarted
            .recover_pending_publications_locked(&peer_store, &guard)
            .unwrap();
    }
    state_lock.unlock().unwrap();
    assert!(restarted.state.queue.is_empty());
    assert!(restarted.load_pending_publications().unwrap().is_empty());
    assert_eq!(restarted.list_decisions(10).unwrap().len(), 1);
    assert_eq!(
        coordinator.current_authority_token().unwrap(),
        crate::services::vault_namespace::VaultAuthorityTokenV1 {
            authority_generation: expected.authority_generation + 1,
            ..expected
        }
    );
}

#[test]
fn duplicate_persisted_job_owners_fail_closed_on_restart() {
    let (_vault, _data, coordinator, mut store, mut service, _) =
        authority_optimizer_fixture("Duplicate Pending Owner");
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    service.fail_next_retry_fence_stage_after_write();
    assert!(matches!(
        service.apply_pending(&mut store, pending).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    let mut duplicate = service.load_pending_publications().unwrap().remove(0).1;
    let duplicate_id = Uuid::new_v4().to_string();
    duplicate.change_id = duplicate_id.clone();
    duplicate.decision.id = duplicate_id.clone();
    duplicate.decision.change_id = Some(duplicate_id.clone());
    duplicate.change.change_id = duplicate_id;
    write_pending_publication(service.retained_optimizer_root().unwrap(), &duplicate).unwrap();
    let namespace = coordinator.current_namespace_path().unwrap();
    drop(service);

    let error = VaultOptimizerService::try_new(namespace).unwrap_err();
    assert!(error
        .to_string()
        .contains("multiple pending-publication owners"));
}

#[test]
fn full_change_audit_rejects_optimizer_before_authority_effect() {
    let (_vault, _data, coordinator, mut store, mut service, note) =
        authority_optimizer_fixture("Full Change Audit");
    let before = coordinator.current_authority_token().unwrap();
    for index in 0..MAX_OPTIMIZER_AUDIT_ENTRIES {
        let change_id = Uuid::from_u128(index as u128 + 1).to_string();
        std::fs::write(
            service.changes_dir.join(format!("{change_id}.json")),
            serde_json::to_vec(&OptimizerChange {
                change_id,
                note_id: format!("old-{index}"),
                mode: "sidecar_first".into(),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    }
    let pending = prepare_authority_write(&mut service, &store, &coordinator);

    assert!(matches!(
        service.apply_pending(&mut store, pending).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    assert_eq!(coordinator.current_authority_token().unwrap(), before);
    assert!(!store.overlay_path(&note.id).exists());
    assert!(service.load_pending_publications().unwrap().is_empty());
    assert_eq!(
        std::fs::read_dir(&service.changes_dir).unwrap().count(),
        4096
    );
}

#[test]
fn pending_reservation_blocks_other_audit_writers_at_boundary() {
    let (_vault, _data, coordinator, mut store, mut service, _) =
        authority_optimizer_fixture("Reserved Audit Slot");
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    let _ = service.apply_pending(&mut store, pending).unwrap();
    let now = Utc::now();
    let inbox = (0..(MAX_OPTIMIZER_AUDIT_ENTRIES - 1))
        .map(|index| VaultOptimizerInboxEntry {
            id: format!("legacy-inbox-{index}"),
            status: "failed".into(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    write_bounded_json_vec(
        service.retained_optimizer_root().unwrap(),
        INBOX_KEY,
        &inbox,
    )
    .unwrap();
    assert!(service
        .push_inbox(VaultOptimizerInboxEntry {
            id: "would-consume-reserved-inbox".into(),
            status: "failed".into(),
            ..Default::default()
        })
        .is_err());

    let events = (0..(MAX_OPTIMIZER_AUDIT_ENTRIES - 1))
        .map(|index| OptimizerAuditEventV1::Rollback {
            change_id: format!("legacy-rollback-{index}"),
            rollback_id: String::new(),
            at: now,
        })
        .collect::<Vec<_>>();
    service.write_events(&events).unwrap();
    assert!(service
        .append_event(OptimizerAuditEventV1::Rollback {
            change_id: "would-consume-reserved-event".into(),
            rollback_id: String::new(),
            at: now,
        })
        .is_err());
}

#[test]
fn peer_recovery_preserves_a_live_retry_fence_and_its_last_audit_slot() {
    let (vault, _data, coordinator, mut store, mut service, _) =
        authority_optimizer_fixture("Live Retry Fence");
    let inbox = (0..(MAX_OPTIMIZER_AUDIT_ENTRIES - 1))
        .map(|index| VaultOptimizerInboxEntry {
            id: format!("existing-inbox-{index}"),
            status: "failed".into(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    write_bounded_json_vec(
        service.retained_optimizer_root().unwrap(),
        INBOX_KEY,
        &inbox,
    )
    .unwrap();
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
    let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
    service.pause_after_retry_fence_once(entered.clone(), resume.clone());

    let owner = std::thread::spawn(move || {
        let outcome = service.apply_pending(&mut store, pending);
        (service, store, outcome)
    });
    entered.wait();

    let namespace = coordinator.current_namespace_path().unwrap();
    let peer_store = KnowledgeStore::with_event_recorder(
        vault.path().to_path_buf(),
        namespace.clone(),
        coordinator.clone(),
    );
    let mut peer = VaultOptimizerService::try_new(namespace).unwrap();
    let state_lock = peer.acquire_state_lock().unwrap();
    peer.reload_from_disk_checked().unwrap();
    let guard = coordinator.begin_root_transition().unwrap();
    peer.recover_pending_publications_locked(&peer_store, &guard)
        .unwrap();
    drop(guard);
    state_lock.unlock().unwrap();

    assert!(matches!(
        peer.prepare_next_expecting_authority(
            &peer_store,
            &UserSettings::default(),
            coordinator.current_authority_token().unwrap(),
        )
        .unwrap(),
        OptimizerTick::RetryFenced(_)
    ));
    assert!(peer
        .push_inbox(VaultOptimizerInboxEntry {
            id: "would-steal-live-reservation".into(),
            status: "failed".into(),
            ..Default::default()
        })
        .is_err());

    resume.wait();
    let (_service, _store, outcome) = owner.join().unwrap();
    assert!(matches!(
        outcome.unwrap(),
        OptimizerMutationResult::Committed { .. }
    ));
}

#[test]
fn committed_publication_replays_all_phases_once_after_restart() {
    let (_vault, _data, coordinator, mut store, mut service, note) =
        authority_optimizer_fixture("Restarted Publication");
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    let outcome = service.apply_pending(&mut store, pending).unwrap();
    assert!(matches!(outcome, OptimizerMutationResult::Committed { .. }));
    drop(service);

    let namespace = coordinator.current_namespace_path().unwrap();
    let mut restarted = VaultOptimizerService::try_new(namespace.clone()).unwrap();
    let state_lock = restarted.acquire_state_lock().unwrap();
    restarted.reload_from_disk_checked().unwrap();
    let guard = coordinator.begin_root_transition().unwrap();
    restarted
        .recover_pending_publications_locked(&store, &guard)
        .unwrap();
    state_lock.unlock().unwrap();
    drop(guard);

    assert!(restarted.load_pending_publications().unwrap().is_empty());
    assert!(restarted
        .state
        .queue
        .iter()
        .all(|job| job.note_id != note.id));
    assert_eq!(restarted.state.accepted_count, 1);
    assert_eq!(restarted.load_decisions().unwrap().len(), 1);
    assert_eq!(restarted.load_inbox().unwrap().len(), 1);
    assert_eq!(restarted.load_events().unwrap().len(), 1);

    drop(restarted);
    let replayed = VaultOptimizerService::try_new(namespace).unwrap();
    assert_eq!(replayed.state.accepted_count, 1);
    assert_eq!(replayed.load_decisions().unwrap().len(), 1);
    assert_eq!(replayed.load_inbox().unwrap().len(), 1);
    assert_eq!(replayed.load_events().unwrap().len(), 1);
}

#[test]
fn completed_witness_survives_receipt_delete_before_witness_delete() {
    let (_vault, _data, coordinator, mut store, mut service, _) =
        authority_optimizer_fixture("Receipt Witness Gap");
    let pending = prepare_authority_write(&mut service, &store, &coordinator);
    let _ = service.apply_pending(&mut store, pending).unwrap();
    let state_lock = service.acquire_state_lock().unwrap();
    service.reload_from_disk_checked().unwrap();
    let (_, mut publication) = service.load_pending_publications().unwrap().remove(0);
    service
        .finalize_pending_publication(&mut publication)
        .unwrap();
    let mutation_id = publication.mutation_id.clone().unwrap();
    state_lock.unlock().unwrap();

    let guard = coordinator.begin_root_transition().unwrap();
    guard
        .consume_witnessed_mutation_receipt(&mutation_id)
        .unwrap();
    drop(guard);
    drop(service);

    let namespace = coordinator.current_namespace_path().unwrap();
    let mut restarted = VaultOptimizerService::try_new(namespace).unwrap();
    let state_lock = restarted.acquire_state_lock().unwrap();
    restarted.reload_from_disk_checked().unwrap();
    let guard = coordinator.begin_root_transition().unwrap();
    restarted
        .recover_pending_publications_locked(&store, &guard)
        .unwrap();
    state_lock.unlock().unwrap();

    assert!(restarted.load_pending_publications().unwrap().is_empty());
    assert_eq!(restarted.state.accepted_count, 1);
    assert_eq!(restarted.load_decisions().unwrap().len(), 1);
    assert_eq!(restarted.load_events().unwrap().len(), 1);
}

#[test]
fn queue_state_writes_are_atomic_with_no_tmp_litter() {
    let data_dir = tempdir().expect("temp dir should be created");
    let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());

    service.bootstrap(&[make_note("note-1", "Optimizer Adoption")]);

    let persisted = std::fs::read_to_string(&service.queue_path).expect("queue.json should exist");
    assert!(persisted.contains("note-1"));
    assert_no_tmp_siblings(&service.optimizer_dir);
}

#[test]
fn peer_instances_reload_revision_before_enqueuing() {
    let data_dir = tempdir().expect("temp dir should be created");
    let mut first = VaultOptimizerService::new(data_dir.path().to_path_buf());
    let mut peer = VaultOptimizerService::new(data_dir.path().to_path_buf());

    first
        .with_locked_fresh_state(|service| {
            service.enqueue_note_checked("note-a", "first").map(|_| ())
        })
        .unwrap();
    let first_revision = first.state_revision();
    peer.with_locked_fresh_state(|service| {
        service.enqueue_note_checked("note-b", "peer").map(|_| ())
    })
    .unwrap();
    let peer_revision = peer.state_revision();
    let queued = first
        .with_locked_fresh_state(|service| {
            Ok(service
                .state
                .queue
                .iter()
                .map(|entry| entry.note_id.clone())
                .collect::<std::collections::BTreeSet<_>>())
        })
        .unwrap();

    assert_eq!(queued, ["note-a".to_string(), "note-b".to_string()].into());
    assert!(peer_revision > first_revision);
    assert_eq!(first.state_revision(), peer_revision);
}

#[test]
fn daily_write_cap_defers_third_write_in_same_day() {
    let vault_dir = tempdir().expect("vault tempdir should be created");
    let data_dir = tempdir().expect("data tempdir should be created");
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
        data_dir.path(),
    ));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            data_dir.path(),
            vault_dir.path(),
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut store = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
        coordinator,
    );

    let notes = vec![
        store
            .create_note(make_note_create("Alpha Topic"))
            .expect("note 1 should be created"),
        store
            .create_note(make_note_create("Beta Topic"))
            .expect("note 2 should be created"),
        store
            .create_note(make_note_create("Gamma Topic"))
            .expect("note 3 should be created"),
    ];

    let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
    service.bootstrap(&notes);
    assert_eq!(service.state.queue.len(), 3);

    let settings = UserSettings {
        background_vault_optimizer_max_daily_writes: 2,
        ..UserSettings::default()
    };

    run_one_optimizer_write(&mut service, &mut store, &settings);
    run_one_optimizer_write(&mut service, &mut store, &settings);
    assert_eq!(
        service.state.queue.len(),
        1,
        "two notes should have been dequeued after being written"
    );
    assert_eq!(service.state.accepted_count, 2);
    assert_eq!(service.state.daily_write_count, 2);

    let queue_before_cap = service.state.queue.clone();
    assert!(matches!(
        service
            .prepare_next(&store, &settings)
            .expect("tick 3 (capped) should not error"),
        OptimizerTick::NoWrite
    ));
    assert_eq!(
        service.state.queue.len(),
        1,
        "the third note must stay queued once the daily cap is hit"
    );
    assert_eq!(
        service.state.queue, queue_before_cap,
        "the deferred job must be untouched (no attempts bump, no removal)"
    );
    assert_eq!(
        service.state.accepted_count, 2,
        "no write should be recorded past the daily cap"
    );
    assert_eq!(
        event_store.ordered_events().unwrap().len(),
        3,
        "sidecar overlays and capped no-ops must not emit beyond note creation"
    );
}

#[test]
fn stale_optimizer_source_aborts_before_overlay_and_queue_publication() {
    let vault_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
        data_dir.path(),
    ));
    events.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            data_dir.path(),
            vault_dir.path(),
            events,
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let namespace = coordinator.current_namespace_path().unwrap();
    let mut store = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        namespace.clone(),
        coordinator.clone(),
    );
    let note = store.create_note(make_note_create("Stale Topic")).unwrap();
    let source = coordinator.current_authority_token().unwrap();
    let mut service = VaultOptimizerService::new(namespace);
    service.bootstrap(std::slice::from_ref(&note));
    let queue_before = service.state.queue.clone();

    store
        .update_note(
            &note.id,
            NoteUpdate {
                tags: Some(vec!["peer".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    assert_ne!(
        coordinator.current_authority_token().unwrap(),
        source,
        "peer note update must advance the exact optimizer source authority"
    );
    let pending = match service
        .prepare_next_expecting_authority(&store, &UserSettings::default(), source)
        .unwrap()
    {
        OptimizerTick::Pending(pending) => *pending,
        other => panic!("expected a pending stale-authority write, got {other:?}"),
    };
    assert!(matches!(
        service.apply_pending(&mut store, pending).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    assert_eq!(service.state.queue.len(), queue_before.len());
    assert_eq!(service.state.queue[0].job_id, queue_before[0].job_id);
    assert_eq!(service.state.queue[0].attempts, 1);
    assert!(!store.overlay_path(&note.id).exists());
    assert_eq!(service.state.accepted_count, 0);
    assert!(service.load_pending_publications().unwrap().is_empty());
}

#[test]
fn postwrite_publication_failure_returns_explicit_committed_result() {
    let vault_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
        data_dir.path(),
    ));
    events.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            data_dir.path(),
            vault_dir.path(),
            events,
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let namespace = coordinator.current_namespace_path().unwrap();
    let mut store = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        namespace.clone(),
        coordinator.clone(),
    );
    let note = store
        .create_note(make_note_create("Committed Optimizer Topic"))
        .unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    let mut service = VaultOptimizerService::new(namespace);
    service.bootstrap(std::slice::from_ref(&note));

    // The governed overlay write and retained receipt succeed, while the
    // committed-witness publication is faulted before the coordinator
    // releases its retained process guard.
    service.fail_next_committed_publication();

    let tick = service
        .prepare_next_expecting_authority(&store, &UserSettings::default(), expected)
        .expect("a durable governed write must not be returned as retryable failure");
    let outcome = match tick {
        OptimizerTick::Pending(pending) => service
            .apply_pending(&mut store, *pending)
            .expect("a durable governed write must not be returned as retryable failure"),
        other => panic!("expected pending optimizer write, got {other:?}"),
    };
    match outcome {
        OptimizerMutationResult::Committed {
            result,
            commit,
            warning,
        } => {
            assert_eq!(result.note_id(), note.id);
            assert!(commit.authority_token.is_some());
            assert_eq!(
                warning.as_ref().map(|value| value.code.as_str()),
                Some("optimizer_publication_pending")
            );
        }
        other => panic!("expected explicit committed optimizer result, got {other:?}"),
    }
    assert!(store.overlay_path(&note.id).exists());
    let pending = service.load_pending_publications().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].1.phase, OptimizerPublicationPhase::Prepared);
    assert!(pending[0].1.retry_fenced);
    assert!(
        matches!(
            service
                .prepare_next_expecting_authority(
                    &store,
                    &UserSettings::default(),
                    coordinator.current_authority_token().unwrap(),
                )
                .unwrap(),
            OptimizerTick::NoWrite
        ),
        "the durable publication witness must fence the queued job from retry"
    );
}

#[test]
fn stale_rollback_source_aborts_before_overlay_and_rollback_state_publication() {
    let vault_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
        data_dir.path(),
    ));
    events.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            data_dir.path(),
            vault_dir.path(),
            events,
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let namespace = coordinator.current_namespace_path().unwrap();
    let mut store = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        namespace.clone(),
        coordinator.clone(),
    );
    let note = store
        .create_note(make_note_create("Rollback Source"))
        .unwrap();
    let old_overlay = serde_json::json!({"tags": ["before"]});
    let overlay = serde_json::json!({"tags": ["optimizer"]});
    store.write_overlay(&note.id, &old_overlay).unwrap();
    store.write_overlay(&note.id, &overlay).unwrap();
    let source = coordinator.current_authority_token().unwrap();
    let mut service = VaultOptimizerService::new(namespace);
    let change_id = Uuid::new_v4().to_string();
    std::fs::write(
        service.changes_dir.join(format!("{change_id}.json")),
        serde_json::to_vec_pretty(&OptimizerChange {
            change_id: change_id.to_string(),
            note_id: note.id.clone(),
            mode: "sidecar_first".to_string(),
            overlay_before: Some(old_overlay),
            overlay_after: Some(overlay.clone()),
            note_before: None,
            note_after: None,
            markdown_before_digest: None,
            markdown_relative_path: None,
            created_at: Some(Utc::now()),
            exact_rollback: None,
        })
        .unwrap(),
    )
    .unwrap();

    store.create_note(make_note_create("Peer Change")).unwrap();
    assert_ne!(coordinator.current_authority_token().unwrap(), source);
    let error = service
        .rollback_change_expecting_authority(&change_id, &mut store, source)
        .unwrap_err();

    assert!(error.to_string().contains("authority"));
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(store.overlay_path(&note.id)).unwrap())
            .unwrap(),
        overlay
    );
    assert_eq!(service.state.rollback_count, 0);
    assert!(!service.events_path.exists());
}

#[test]
fn sidecar_rollback_restores_an_absent_overlay() {
    let vault_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
        data_dir.path(),
    ));
    events.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            data_dir.path(),
            vault_dir.path(),
            events,
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let namespace = coordinator.current_namespace_path().unwrap();
    let mut store = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        namespace.clone(),
        coordinator.clone(),
    );
    let note = store
        .create_note(make_note_create("Absent Overlay"))
        .unwrap();
    let overlay = serde_json::json!({"tags": ["optimizer"]});
    store.write_overlay(&note.id, &overlay).unwrap();
    let source = coordinator.current_authority_token().unwrap();
    let mut service = VaultOptimizerService::new(namespace);
    let change_id = Uuid::new_v4().to_string();
    std::fs::write(
        service.changes_dir.join(format!("{change_id}.json")),
        serde_json::to_vec_pretty(&OptimizerChange {
            change_id: change_id.to_string(),
            note_id: note.id.clone(),
            mode: "sidecar_first".to_string(),
            overlay_before: None,
            overlay_after: Some(overlay),
            note_before: None,
            note_after: None,
            markdown_before_digest: None,
            markdown_relative_path: None,
            created_at: Some(Utc::now()),
            exact_rollback: None,
        })
        .unwrap(),
    )
    .unwrap();

    let error = service
        .rollback_change_expecting_authority(&change_id, &mut store, source)
        .unwrap_err();

    assert!(
        format!("{error:#}").contains("audit-only"),
        "unexpected rollback error: {error:#}"
    );
    assert!(store.overlay_path(&note.id).exists());
    assert_eq!(service.state.rollback_count, 0);
}

#[test]
fn apply_pending_merges_against_current_note_not_stale_snapshot() {
    // Between `prepare_next` (read lock) and `apply_pending` (write lock)
    // there is a real await suspension in the background worker, so a
    // concurrent user `update_note` can land in the gap. The apply stage
    // must merge the proposal's ADDITIONS against the note's CURRENT
    // state, not the snapshot captured in `prepare_next` — otherwise it
    // silently drops the user's fresh tag and reverts their rename.
    let vault_dir = tempdir().expect("vault tempdir should be created");
    let data_dir = tempdir().expect("data tempdir should be created");
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
        data_dir.path(),
    ));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            data_dir.path(),
            vault_dir.path(),
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut store = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
        coordinator,
    );

    let note = store
        .create_note(make_note_create("Interleaved Edit Topic"))
        .expect("note should be created");

    let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
    service.bootstrap(std::slice::from_ref(&note));

    let settings = UserSettings {
        background_vault_optimizer_edit_mode: "full_rewrite".to_string(),
        ..UserSettings::default()
    };

    let pending = match service
        .prepare_next(&store, &settings)
        .expect("prepare should not error")
    {
        OptimizerTick::Pending(pending) => pending,
        other => panic!(
            "full_rewrite mode must return a pending write, got {:?}",
            other
        ),
    };

    // Simulate the interleaved user edit landing between the read-locked
    // prepare stage and the write-locked apply stage: add a tag and move
    // the note to a new path.
    store
        .update_note(
            &note.id,
            NoteUpdate {
                tags: Some(vec!["user-fresh-tag".to_string()]),
                relative_path: Some("renamed-by-user.md".to_string()),
                ..Default::default()
            },
        )
        .expect("interleaved user edit should succeed");

    let applied = service
        .apply_pending(&mut store, *pending)
        .expect("apply should not error");
    let OptimizerMutationResult::Committed { result, .. } = applied else {
        panic!("apply_pending must report its committed write")
    };
    assert_eq!(result.note_id(), note.id);

    let final_note = store.get_note(&note.id).expect("note should still exist");
    assert!(
        final_note.tags.iter().any(|tag| tag == "user-fresh-tag"),
        "the user's interleaved tag must survive the optimizer apply, got tags: {:?}",
        final_note.tags
    );
    assert!(
        final_note
            .tags
            .iter()
            .any(|tag| tag == "interleaved_edit_topic"),
        "the proposal's additive tag must still be applied, got tags: {:?}",
        final_note.tags
    );
    assert_eq!(
        final_note.relative_path, "renamed-by-user.md",
        "the user's interleaved rename must not be reverted to the snapshot path"
    );
    let events = event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].context.source_channel.as_str(), "note_editor");
    assert_eq!(events[1].context.source_channel.as_str(), "note_editor");
    assert_eq!(events[2].context.source_channel.as_str(), "vault_optimizer");
}

#[test]
fn external_markdown_edit_between_refetch_and_digest_is_not_overwritten() {
    let (vault, _data, coordinator, mut store, mut service, note) =
        authority_optimizer_fixture("Torn Snapshot Topic");
    let settings = UserSettings {
        background_vault_optimizer_edit_mode: "full_rewrite".into(),
        ..UserSettings::default()
    };
    let pending = match service
        .prepare_next_expecting_authority(
            &store,
            &settings,
            coordinator.current_authority_token().unwrap(),
        )
        .unwrap()
    {
        OptimizerTick::Pending(pending) => *pending,
        other => panic!("expected a pending full rewrite, got {other:?}"),
    };
    let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
    let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
    service.pause_before_markdown_digest_once(entered.clone(), resume.clone());
    let owner = std::thread::spawn(move || {
        let outcome = service.apply_pending(&mut store, pending);
        (store, outcome)
    });

    entered.wait();
    let markdown_path = vault.path().join(&note.relative_path);
    let mut external = std::fs::read_to_string(&markdown_path).unwrap();
    external.push_str("\n\nExternal B survives.\n");
    std::fs::write(&markdown_path, external).unwrap();
    resume.wait();

    let (store, outcome) = owner.join().unwrap();
    assert!(matches!(outcome.unwrap(), OptimizerMutationResult::NoWrite));
    assert!(store
        .get_note(&note.id)
        .unwrap()
        .content
        .contains("External B survives."));
    let restarted =
        VaultOptimizerService::try_new(coordinator.current_namespace_path().unwrap()).unwrap();
    assert_eq!(restarted.state.queue[0].attempts, 1);
    assert!(restarted.load_pending_publications().unwrap().is_empty());
}

#[test]
fn apply_pending_parks_job_when_note_deleted_in_the_gap() {
    // If the note is deleted between prepare and apply, the apply stage
    // must not resurrect it — the job is dropped like any missing note.
    let vault_dir = tempdir().expect("vault tempdir should be created");
    let data_dir = tempdir().expect("data tempdir should be created");
    let mut store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );

    let note = store
        .create_note(make_note_create("Deleted In Gap Topic"))
        .expect("note should be created");

    let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
    service.bootstrap(std::slice::from_ref(&note));

    let settings = UserSettings {
        background_vault_optimizer_edit_mode: "full_rewrite".to_string(),
        ..UserSettings::default()
    };

    let pending = match service
        .prepare_next(&store, &settings)
        .expect("prepare should not error")
    {
        OptimizerTick::Pending(pending) => pending,
        other => panic!(
            "full_rewrite mode must return a pending write, got {:?}",
            other
        ),
    };

    store
        .delete_note(&note.id)
        .expect("interleaved delete should succeed");

    let applied = service
        .apply_pending(&mut store, *pending)
        .expect("apply of a deleted note must not error");
    assert!(matches!(applied, OptimizerMutationResult::NoWrite));

    assert!(
        store.get_note(&note.id).is_err(),
        "the optimizer must not resurrect a note deleted in the gap"
    );
    assert!(
        service.state.queue.is_empty(),
        "the job for a deleted note must be dropped from the queue"
    );
    assert_eq!(
        service.state.accepted_count, 0,
        "no write should be recorded for a deleted note"
    );
}

#[test]
fn apply_noop_does_not_clobber_a_peer_enqueue() {
    let vault_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let mut store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );
    let removed = store.create_note(make_note_create("Removed job")).unwrap();
    let peer_note = store.create_note(make_note_create("Peer enqueue")).unwrap();
    let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
    service.bootstrap(std::slice::from_ref(&removed));
    let settings = UserSettings {
        background_vault_optimizer_edit_mode: "full_rewrite".into(),
        ..UserSettings::default()
    };
    let pending = match service.prepare_next(&store, &settings).unwrap() {
        OptimizerTick::Pending(pending) => *pending,
        other => panic!("expected pending optimizer write, got {other:?}"),
    };

    let mut peer = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
    peer.with_locked_fresh_state(|peer| {
        assert!(peer.enqueue_note_checked(&peer_note.id, "peer")?);
        Ok(())
    })
    .unwrap();
    store.delete_note(&removed.id).unwrap();

    assert!(matches!(
        service.apply_pending(&mut store, pending).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    let restarted = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
    assert_eq!(restarted.state.queue.len(), 1);
    assert_eq!(restarted.state.queue[0].note_id, peer_note.id);
}

#[test]
fn apply_error_does_not_clobber_a_peer_enqueue() {
    let vault_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let mut store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );
    let note = store.create_note(make_note_create("Poison Topic")).unwrap();
    let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
    service.bootstrap(std::slice::from_ref(&note));
    let pending = match service
        .prepare_next(&store, &UserSettings::default())
        .unwrap()
    {
        OptimizerTick::Pending(pending) => *pending,
        other => panic!("expected pending optimizer write, got {other:?}"),
    };
    poison_overlay_directory(&store, &note);

    let mut peer = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
    peer.with_locked_fresh_state(|peer| {
        assert!(peer.enqueue_note_checked("peer-survivor", "peer")?);
        Ok(())
    })
    .unwrap();

    assert!(matches!(
        service.apply_pending(&mut store, pending).unwrap(),
        OptimizerMutationResult::NoWrite
    ));
    let restarted = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
    assert_eq!(restarted.state.queue.len(), 2);
    assert_eq!(restarted.state.queue[0].note_id, note.id);
    assert_eq!(restarted.state.queue[0].attempts, 1);
    assert_eq!(restarted.state.queue[1].note_id, "peer-survivor");
}

#[test]
fn run_next_ignores_llm_enabled_because_no_llm_path_exists() {
    // vault_optimizer has no LLM/network call path today:
    // `build_optimizer_proposal` is purely rule-based, and neither
    // `prepare_next` nor `apply_pending` reference `OpenRouterService` or
    // any network client anywhere in this file (confirmed by inspection —
    // there is no seam to stub). This test characterizes that fact:
    // toggling `background_vault_optimizer_llm_enabled` produces
    // identical decisions, proving enabling it doesn't silently add
    // behavior and disabling it doesn't block the rules pipeline. If an
    // LLM-backed enrichment step is ever added, it must be gated on this
    // flag and this test should then be replaced with one that exercises
    // the real seam.
    fn run_with_llm_flag(llm_enabled: bool) -> VaultOptimizerDecision {
        let vault_dir = tempdir().expect("vault tempdir should be created");
        let data_dir = tempdir().expect("data tempdir should be created");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );
        let note = store
            .create_note(make_note_create("Shared Topic"))
            .expect("note should be created");

        let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
        service.bootstrap(&[note]);

        let settings = UserSettings {
            background_vault_optimizer_llm_enabled: llm_enabled,
            ..UserSettings::default()
        };
        run_one_optimizer_write(&mut service, &mut store, &settings);

        service
            .list_decisions(1)
            .expect("decisions should be readable")
            .into_iter()
            .next()
            .expect("a decision should have been recorded")
    }

    let disabled = run_with_llm_flag(false);
    let enabled = run_with_llm_flag(true);

    assert_eq!(disabled.reason, enabled.reason);
    assert_eq!(disabled.diff_preview, enabled.diff_preview);
    assert_eq!(disabled.confidence, enabled.confidence);
}

/// Writes an oversized overlay so the bounded optimizer snapshot fails
/// before any authority mutation. Returns the poisoned store and the note
/// that will always fail to process.
fn seed_poisoned_note(
    vault_dir: &std::path::Path,
    data_dir: &std::path::Path,
) -> (KnowledgeStore, Note) {
    let mut store = KnowledgeStore::new(vault_dir.to_path_buf(), data_dir.to_path_buf());
    let note = store
        .create_note(make_note_create("Poison Topic"))
        .expect("note should be created");

    poison_overlay_directory(&store, &note);

    (store, note)
}

fn poison_overlay_directory(store: &KnowledgeStore, note: &Note) {
    std::fs::write(
        store.overlay_path(&note.id),
        vec![b'x'; crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES + 1],
    )
    .expect("oversized overlay should be writable");
}

#[test]
fn processing_error_keeps_job_queued_and_increments_attempts() {
    let vault_dir = tempdir().expect("vault tempdir should be created");
    let data_dir = tempdir().expect("data tempdir should be created");
    let (store, note) = seed_poisoned_note(vault_dir.path(), data_dir.path());

    let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
    service.bootstrap(std::slice::from_ref(&note));

    let settings = UserSettings::default();

    assert!(matches!(
        service
            .prepare_next(&store, &settings)
            .expect("preparing a processing attempt must not error"),
        OptimizerTick::NoWrite
    ));

    assert_eq!(
        service.state.queue.len(),
        1,
        "a failed job must stay queued, not be dropped"
    );
    assert_eq!(service.state.queue[0].note_id, note.id);
    assert_eq!(
        service.state.queue[0].attempts, 1,
        "the first failure should record exactly one attempt"
    );
    assert_eq!(
        service.state.accepted_count, 0,
        "no write should have been recorded for a failed job"
    );
}

#[test]
fn poison_job_is_parked_after_max_attempts() {
    let vault_dir = tempdir().expect("vault tempdir should be created");
    let data_dir = tempdir().expect("data tempdir should be created");
    let (store, note) = seed_poisoned_note(vault_dir.path(), data_dir.path());

    let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
    service.bootstrap(std::slice::from_ref(&note));

    let settings = UserSettings::default();

    for attempt in 1..=MAX_OPTIMIZER_ATTEMPTS {
        assert!(matches!(
            service
                .prepare_next(&store, &settings)
                .expect("preparing a processing attempt must not error"),
            OptimizerTick::NoWrite
        ));
        if attempt < MAX_OPTIMIZER_ATTEMPTS {
            assert_eq!(
                service.state.queue.len(),
                1,
                "job should still be queued before the attempt limit"
            );
        }
    }

    assert!(
        service.state.queue.is_empty(),
        "a poisoned job must be dropped from the queue after {} attempts",
        MAX_OPTIMIZER_ATTEMPTS
    );
    let inbox = service
        .inbox(Some("failed"), 10)
        .expect("inbox should be readable");
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].note_id.as_deref(), Some(note.id.as_str()));
    assert_eq!(inbox[0].status, "failed");
}

#[test]
fn terminal_parking_recovers_once_after_crash_between_audit_and_queue_removal() {
    let vault_dir = tempdir().expect("vault tempdir should be created");
    let data_dir = tempdir().expect("data tempdir should be created");
    let (store, note) = seed_poisoned_note(vault_dir.path(), data_dir.path());
    let settings = UserSettings::default();
    let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
    service.bootstrap(std::slice::from_ref(&note));

    for _ in 1..MAX_OPTIMIZER_ATTEMPTS {
        assert!(matches!(
            service.prepare_next(&store, &settings).unwrap(),
            OptimizerTick::NoWrite
        ));
    }
    service.fail_next_terminal_parking_after_publication();
    assert!(service.prepare_next(&store, &settings).is_err());
    assert_eq!(service.inbox(Some("failed"), 10).unwrap().len(), 1);
    assert_eq!(service.load_events().unwrap().len(), 1);
    drop(service);

    let restarted = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
    assert!(restarted.state.queue.is_empty());
    assert_eq!(restarted.inbox(Some("failed"), 10).unwrap().len(), 1);
    assert_eq!(restarted.load_events().unwrap().len(), 1);
    drop(restarted);

    let restarted_again = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
    assert!(restarted_again.state.queue.is_empty());
    assert_eq!(restarted_again.inbox(Some("failed"), 10).unwrap().len(), 1);
    assert_eq!(restarted_again.load_events().unwrap().len(), 1);
}

#[test]
fn root_retarget_discards_old_queue_and_bootstraps_only_new_vault_ids() {
    let old_vault = tempdir().unwrap();
    let new_vault = tempdir().unwrap();
    let data = tempdir().unwrap();
    let mut old_store =
        KnowledgeStore::new(old_vault.path().to_path_buf(), data.path().to_path_buf());
    let mut new_store =
        KnowledgeStore::new(new_vault.path().to_path_buf(), data.path().to_path_buf());
    let old_note = old_store.create_note(make_note_create("Old root")).unwrap();
    let new_note = new_store.create_note(make_note_create("New root")).unwrap();
    let mut service = VaultOptimizerService::new(data.path().to_path_buf());
    service.bootstrap(std::slice::from_ref(&old_note));
    assert_eq!(service.state.queue[0].note_id, old_note.id);

    service.reset_for_vault(std::slice::from_ref(&new_note));
    assert_eq!(service.state.queue.len(), 1);
    assert_eq!(service.state.queue[0].note_id, new_note.id);
    assert!(service
        .state
        .queue
        .iter()
        .all(|entry| entry.note_id != old_note.id));
}
