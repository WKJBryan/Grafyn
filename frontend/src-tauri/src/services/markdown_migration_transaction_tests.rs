use super::*;
use std::sync::Arc;
use tempfile::tempdir;

struct TransactionHarness {
    _root: tempfile::TempDir,
    vault: PathBuf,
    data: PathBuf,
    service: MarkdownMigrationService,
    store: KnowledgeStore,
    coordinator: Arc<crate::services::twin_events::MutationCoordinator>,
    events: Arc<crate::services::twin_events::TwinEventStore>,
}

impl TransactionHarness {
    fn new() -> Self {
        let root = tempdir().unwrap();
        let vault = root.path().join("vault");
        let data = root.path().join("data");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&data).unwrap();
        let events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
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
        let mut store =
            KnowledgeStore::with_event_recorder(vault.clone(), data.clone(), coordinator.clone());
        let authority = coordinator.current_authority_token().unwrap();
        let derived =
            crate::services::vault_namespace::scoped_data_path(&data, &authority.root_scope);
        store
            .adopt_coordinated_vault_path(vault.clone(), &derived)
            .unwrap();
        Self {
            _root: root,
            vault,
            data: data.clone(),
            service: MarkdownMigrationService::try_new(data).unwrap(),
            store,
            coordinator,
            events,
        }
    }

    fn preview(&mut self, request: MarkdownMigrationRequest) -> MarkdownMigrationPreview {
        let authority = self.coordinator.current_authority_token().unwrap();
        self.service
            .preview_scoped(&mut self.store, authority, request)
            .unwrap()
    }

    fn overlay_path(&self, note_id: &str) -> PathBuf {
        let authority = self.coordinator.current_authority_token().unwrap();
        crate::services::vault_namespace::scoped_data_path(&self.data, &authority.root_scope)
            .join("vault_migration/overlay/notes")
            .join(format!("{note_id}.json"))
    }

    fn run_record_path(&self, run_id: &str, name: &str) -> PathBuf {
        self.service.runs_dir.join(run_id).join(name)
    }
}

fn apply_and_get_run(
    harness: &mut TransactionHarness,
    preview: &MarkdownMigrationPreview,
    request: MarkdownMigrationRequest,
) -> String {
    let authority = harness.coordinator.current_authority_token().unwrap();
    let outcome = harness
        .service
        .apply_transaction(&preview.preview_id, request, &mut harness.store, authority)
        .unwrap();
    let (result, _, _, _, partial) = outcome.into_parts();
    assert!(!partial, "unexpected partial apply: {result:?}");
    assert_eq!(result.status, "applied", "resume result: {result:?}");
    result.run_id
}

fn current_migration_effects(
    harness: &TransactionHarness,
) -> (
    crate::services::vault_namespace::VaultAuthorityTokenV1,
    Vec<crate::models::twin_event::TwinEvent>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
) {
    (
        harness.coordinator.current_authority_token().unwrap(),
        harness.events.ordered_events().unwrap(),
        std::fs::read(harness.vault.join("alpha.md")).unwrap(),
        std::fs::read(harness.vault.join("_grafyn/program.md")).unwrap(),
        std::fs::read(harness.overlay_path("alpha")).unwrap(),
    )
}

fn migration_operation_path(
    harness: &TransactionHarness,
    operation: &MigrationOperationV1,
) -> PathBuf {
    match operation.target_kind {
        crate::services::twin_events::TargetKind::Markdown => {
            harness.vault.join(&operation.target_key)
        }
        crate::services::twin_events::TargetKind::OverlayJson => harness.overlay_path(
            operation
                .target_key
                .strip_suffix(".json")
                .expect("canonical overlay target"),
        ),
        _ => panic!("migration fixture selected an unsupported target"),
    }
}

fn interrupt_after_authority_with_target_drift<T>(
    harness: &mut TransactionHarness,
    operation: &MigrationOperationV1,
    run: impl FnOnce(&mut TransactionHarness) -> T,
) -> T {
    let path = migration_operation_path(harness, operation);
    let original = std::fs::read(&path).ok();
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    harness
        .coordinator
        .pause_after_authority_advance_once(entered.clone(), resume.clone());
    let drift_path = path.clone();
    let drift = std::thread::spawn(move || {
        entered.wait();
        if let Some(parent) = drift_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&drift_path, b"concurrent post-authority drift").unwrap();
        resume.wait();
    });
    let result = run(harness);
    drift.join().unwrap();
    match original {
        Some(bytes) => std::fs::write(path, bytes).unwrap(),
        None => match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("failed to restore absent migration target: {error}"),
        },
    }
    result
}

fn assert_exact_partial<T>(
    outcome: MigrationMutationOutcome<T>,
) -> (
    crate::models::twin_event::ContentDigest,
    crate::services::vault_namespace::VaultAuthorityTokenV1,
) {
    let (_, commit, authority, warning, partial) = outcome.into_parts();
    assert!(partial);
    assert_eq!(
        warning.as_ref().map(|warning| warning.code.as_str()),
        Some("derived_state_unavailable")
    );
    let commit = commit.expect("authority-only partial must return its exact commit");
    let mutation_id = commit
        .mutation_id
        .expect("authority-only partial must return its mutation ID");
    let authority = authority.expect("authority-only partial must return its authority");
    assert_eq!(commit.authority_token.as_ref(), Some(&authority));
    (mutation_id, authority)
}

fn retained_proof_count(
    harness: &TransactionHarness,
    mutation_id: &crate::models::twin_event::ContentDigest,
) -> usize {
    ["preauthority/v1", "receipts/v1"]
        .into_iter()
        .filter(|folder| {
            harness
                .data
                .join("twin/mutations")
                .join(folder)
                .join(format!("{}.json", mutation_id.as_str()))
                .exists()
        })
        .count()
}

fn retained_proof_inventory_count(harness: &TransactionHarness) -> usize {
    ["preauthority/v1", "receipts/v1"]
        .into_iter()
        .map(|folder| harness.data.join("twin/mutations").join(folder))
        .map(|folder| {
            std::fs::read_dir(folder)
                .map(|entries| entries.filter_map(Result::ok).count())
                .unwrap_or(0)
        })
        .sum()
}

fn tamper_manifest_last_commit_id(
    harness: &TransactionHarness,
    run_id: &str,
) -> crate::models::twin_event::ContentDigest {
    let manifest_path = harness.run_record_path(run_id, "manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let tampered = crate::services::twin_events::digest_bytes(b"tampered migration commit");
    manifest["last_commit"]["mutation_id"] = serde_json::to_value(&tampered).unwrap();
    std::fs::write(manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    tampered
}

fn mark_active_receipt_consumed(harness: &TransactionHarness, run_id: &str) {
    let manifest_path = harness.run_record_path(run_id, "manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["active_step"]["receipt_consumed"], false);
    manifest["active_step"]["receipt_consumed"] = Value::Bool(true);
    std::fs::write(manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}

fn stage_authority_only_count(
    harness: &mut TransactionHarness,
    run_id: &str,
    count: u64,
) -> (
    crate::models::twin_event::ContentDigest,
    crate::services::vault_namespace::VaultAuthorityTokenV1,
) {
    let mut manifest = harness.service.load_strict_manifest(run_id).unwrap();
    let starting = manifest.starting_authority.clone().unwrap();
    let authority = crate::services::vault_namespace::VaultAuthorityTokenV1 {
        root_scope: starting.root_scope.clone(),
        lease_epoch_uuid: starting.lease_epoch_uuid.clone(),
        authority_generation: starting.authority_generation
            + manifest.apply_next as u64
            + manifest.rollback_next as u64
            + count,
    };
    let mutation_id = crate::services::twin_events::digest_bytes(
        format!("authority-only fixture {run_id} {count}").as_bytes(),
    );
    manifest.authority_only_advances = count;
    manifest.final_authority = Some(authority.clone());
    manifest.last_commit = Some(MigrationCommitProofV1 {
        mutation_id: mutation_id.clone(),
        authority: authority.clone(),
    });
    let (direction, operation_index) = if manifest.applied_at.is_none() {
        manifest.status = "apply_partial".into();
        ("apply", manifest.apply_next)
    } else {
        manifest.rollback_total = manifest.apply_next;
        manifest.status = "rollback_partial".into();
        (
            "rollback",
            manifest.rollback_total - 1 - manifest.rollback_next,
        )
    };
    let commit_directory = harness.service.runs_dir.join(run_id).join("commits");
    std::fs::create_dir_all(&commit_directory).unwrap();
    std::fs::write(
        commit_directory.join(format!("{}.json", authority.authority_generation)),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "run_id": run_id,
            "mutation_id": mutation_id,
            "authority": authority,
            "direction": direction,
            "operation_index": operation_index,
            "outcome": "authority_only",
        }))
        .unwrap(),
    )
    .unwrap();
    harness.service.write_strict_manifest(&manifest).unwrap();

    std::fs::write(
        harness.data.join("twin/events/content-authority-v1.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 1,
            "authority_generation": authority.authority_generation,
        }))
        .unwrap(),
    )
    .unwrap();
    let ready_path = harness
        .data
        .join("vault_derived/v1")
        .join(authority.root_scope.as_str())
        .join("ready-v1.json");
    std::fs::create_dir_all(ready_path.parent().unwrap()).unwrap();
    std::fs::write(
        ready_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 2,
            "root_scope": authority.root_scope,
            "lease_epoch_uuid": authority.lease_epoch_uuid,
            "authority_generation": authority.authority_generation,
            "ready": true,
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        harness.coordinator.current_authority_token().unwrap(),
        authority
    );
    (mutation_id, authority)
}

fn restart_migration_service(harness: &mut TransactionHarness) {
    harness.service = MarkdownMigrationService::try_new(harness.data.clone()).unwrap();
}

#[test]
fn rollback_resume_prevalidates_every_remaining_after_image_before_any_write() {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let run_id = apply_and_get_run(&mut harness, &preview, request);
    let overlay_path = harness.overlay_path("alpha");
    let overlay_before = std::fs::read(&overlay_path).unwrap();
    harness
        .service
        .mark_rollback_resume_for_test(&run_id)
        .unwrap();
    let edited_program = b"external program edit";
    std::fs::write(harness.vault.join("_grafyn/program.md"), edited_program).unwrap();
    let authority = harness.coordinator.current_authority_token().unwrap();

    assert!(harness
        .service
        .rollback_transaction(&run_id, &mut harness.store, authority.clone())
        .is_err());
    assert_eq!(std::fs::read(&overlay_path).unwrap(), overlay_before);
    assert_eq!(
        std::fs::read(harness.vault.join("_grafyn/program.md")).unwrap(),
        edited_program
    );
    assert_eq!(
        harness.coordinator.current_authority_token().unwrap(),
        authority
    );
}

#[test]
fn sidecar_apply_and_rollback_emit_one_manifest_bound_governed_update_each() {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\nprivate: true\nsensitivity: restricted\n---\n\nOriginal",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let run_id = apply_and_get_run(&mut harness, &preview, request);
    let markdown_digest = crate::services::twin_events::digest_bytes(
        &std::fs::read(harness.vault.join("alpha.md")).unwrap(),
    );
    let overlay_digest = crate::services::twin_events::digest_bytes(
        &std::fs::read(harness.overlay_path("alpha")).unwrap(),
    );
    let expected_before = overlay_state_digest_for_test(&markdown_digest, "absent");
    let expected_after = overlay_state_digest_for_test(&markdown_digest, overlay_digest.as_str());
    let durable_apply_events = harness.events.ordered_events().unwrap();
    let apply_events = alpha_updated_events(&durable_apply_events);
    assert_eq!(apply_events.len(), 1);
    assert_eq!(
        note_event_digests(apply_events[0]),
        (expected_after.clone(), expected_before.clone())
    );

    let authority = harness.coordinator.current_authority_token().unwrap();
    let outcome = harness
        .service
        .rollback_transaction(&run_id, &mut harness.store, authority)
        .unwrap();
    let (result, _, _, _, partial) = outcome.into_parts();
    assert!(!partial);
    assert!(result.rolled_back);

    let durable_events = harness.events.ordered_events().unwrap();
    let events = alpha_updated_events(&durable_events);
    assert_eq!(events.len(), 2);
    let (apply_payload, apply_evidence) = note_event_digests(&events[0]);
    let (rollback_payload, rollback_evidence) = note_event_digests(&events[1]);
    assert_eq!(apply_payload, rollback_evidence);
    assert_eq!(apply_evidence, rollback_payload);
    assert_ne!(apply_payload, rollback_payload);
    assert_eq!(events[0].governance, events[1].governance);
    assert_eq!(
        events[0].governance.sensitivity,
        crate::models::twin_event::Sensitivity::Restricted
    );
    assert!(events
        .iter()
        .all(|event| event.context.source_channel.as_str() == "migration"));
}

fn alpha_updated_events(
    events: &[crate::models::twin_event::TwinEvent],
) -> Vec<&crate::models::twin_event::TwinEvent> {
    events
        .iter()
        .filter(|event| {
            matches!(
                &event.payload,
                crate::models::twin_event::TwinEventPayload::NoteChanged(payload)
                    if payload.note_id.as_str() == "alpha"
                        && payload.change == crate::models::twin_event::NoteChangeKind::Updated
            )
        })
        .collect()
}

fn overlay_state_digest_for_test(
    markdown: &crate::models::twin_event::ContentDigest,
    overlay: &str,
) -> crate::models::twin_event::ContentDigest {
    crate::services::twin_events::digest_bytes(
        format!(
            "grafyn.migration.overlay-state.v1\n{}\n{overlay}",
            markdown.as_str()
        )
        .as_bytes(),
    )
}

fn note_event_digests(
    event: &crate::models::twin_event::TwinEvent,
) -> (
    crate::models::twin_event::ContentDigest,
    crate::models::twin_event::ContentDigest,
) {
    let crate::models::twin_event::TwinEventPayload::NoteChanged(payload) = &event.payload else {
        panic!("expected NoteChanged event");
    };
    (
        payload.content_digest.clone().unwrap(),
        event.evidence[0].digest.clone().unwrap(),
    )
}

#[test]
fn created_hub_rollback_emits_deleted_digest_and_repairs_all_readers() {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("health.md"),
        "---\nnote_id: health\ntitle: Health\n---\n\n#wellness",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    assert_eq!(preview.summary.proposed_hubs, 1);
    let run_id = apply_and_get_run(&mut harness, &preview, request);
    let events_after_apply = harness.events.ordered_events().unwrap();
    let created = events_after_apply
        .iter()
        .find(|event| {
            matches!(
                event.payload,
                crate::models::twin_event::TwinEventPayload::NoteChanged(
                    crate::models::twin_event::NoteChanged {
                        change: crate::models::twin_event::NoteChangeKind::Created,
                        ..
                    }
                )
            )
        })
        .unwrap();
    let created_digest = note_event_digests(created).0;
    let hub_path = preview
        .topic_candidates
        .iter()
        .find(|topic| topic.reuse_existing_hub_id.is_none())
        .map(|topic| {
            format!(
                "{}/{}.md",
                preview.hub_folder,
                super::slugify(&topic.display_name)
            )
        })
        .unwrap();
    assert!(harness.vault.join(&hub_path).is_file());

    let authority = harness.coordinator.current_authority_token().unwrap();
    let (rollback, _, authority, warning, partial) = harness
        .service
        .rollback_transaction(&run_id, &mut harness.store, authority)
        .unwrap()
        .into_parts();
    assert!(rollback.rolled_back);
    assert!(authority.is_some());
    assert!(warning.is_none());
    assert!(!partial);
    let events = harness.events.ordered_events().unwrap();
    let deleted = events
        .iter()
        .find(|event| {
            matches!(
                event.payload,
                crate::models::twin_event::TwinEventPayload::NoteChanged(
                    crate::models::twin_event::NoteChanged {
                        change: crate::models::twin_event::NoteChangeKind::Deleted,
                        ..
                    }
                )
            )
        })
        .unwrap();
    let (deleted_payload, deleted_evidence) = note_event_digests(deleted);
    assert_eq!(deleted_payload, created_digest);
    assert_eq!(deleted_evidence, created_digest);
    assert!(!harness.vault.join(&hub_path).exists());
    harness.store.reload_authoritative_state();
    assert!(harness
        .store
        .list_full_notes()
        .unwrap()
        .iter()
        .all(|note| note.relative_path != hub_path));
}

#[test]
fn rollback_restores_noncanonical_markdown_and_overlay_bytes_exactly() {
    let mut markdown = TransactionHarness::new();
    let original_markdown = b"---\r\nnote_id: alpha\r\ntitle:  Alpha\r\naliases: [A, B]\r\ncustom: {z: 1, a: 2}\r\n---\r\n\r\nOriginal   ";
    std::fs::write(markdown.vault.join("alpha.md"), original_markdown).unwrap();
    let full_rewrite = MarkdownMigrationRequest {
        mode: MarkdownMigrationMode::FullRewrite,
        ..MarkdownMigrationRequest::default()
    };
    let preview = markdown.preview(full_rewrite.clone());
    let run_id = apply_and_get_run(&mut markdown, &preview, full_rewrite);
    let authority = markdown.coordinator.current_authority_token().unwrap();
    let (rollback, _, authority, warning, partial) = markdown
        .service
        .rollback_transaction(&run_id, &mut markdown.store, authority)
        .unwrap()
        .into_parts();
    assert!(rollback.rolled_back);
    assert!(authority.is_some());
    assert!(warning.is_none());
    assert!(!partial);
    assert_eq!(
        std::fs::read(markdown.vault.join("alpha.md")).unwrap(),
        original_markdown
    );

    let mut sidecar = TransactionHarness::new();
    std::fs::write(
        sidecar.vault.join("beta.md"),
        "---\nnote_id: beta\ntitle: Beta\n---\n\nOriginal",
    )
    .unwrap();
    let overlay_path = sidecar.overlay_path("beta");
    std::fs::create_dir_all(overlay_path.parent().unwrap()).unwrap();
    let original_overlay =
        b"{\"properties\": {\"z\":1,\"private\":true}, \"aliases\":[\"Old\"],\"tags\":[]}";
    std::fs::write(&overlay_path, original_overlay).unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = sidecar.preview(request.clone());
    let run_id = apply_and_get_run(&mut sidecar, &preview, request);
    let authority = sidecar.coordinator.current_authority_token().unwrap();
    let (rollback, _, authority, warning, partial) = sidecar
        .service
        .rollback_transaction(&run_id, &mut sidecar.store, authority)
        .unwrap()
        .into_parts();
    assert!(rollback.rolled_back);
    assert!(authority.is_some());
    assert!(warning.is_none());
    assert!(!partial);
    assert_eq!(std::fs::read(overlay_path).unwrap(), original_overlay);

    let mut absent = TransactionHarness::new();
    std::fs::write(
        absent.vault.join("gamma.md"),
        "---\nnote_id: gamma\ntitle: Gamma\n---\n\nOriginal",
    )
    .unwrap();
    let overlay_path = absent.overlay_path("gamma");
    assert!(!overlay_path.exists());
    let request = MarkdownMigrationRequest::default();
    let preview = absent.preview(request.clone());
    let run_id = apply_and_get_run(&mut absent, &preview, request);
    assert!(overlay_path.exists());
    let authority = absent.coordinator.current_authority_token().unwrap();
    let (rollback, _, authority, warning, partial) = absent
        .service
        .rollback_transaction(&run_id, &mut absent.store, authority)
        .unwrap()
        .into_parts();
    assert!(rollback.rolled_back);
    assert!(authority.is_some());
    assert!(warning.is_none());
    assert!(!partial);
    assert!(!overlay_path.exists());
}

#[test]
fn two_service_instances_share_one_transaction_lock_without_interleaving() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let first_events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    first_events.initialize().unwrap();
    let first_coordinator = Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            first_events,
            Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let second_events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    second_events.initialize().unwrap();
    let second_coordinator = Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            second_events,
            Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let start = first_coordinator.current_authority_token().unwrap();
    let derived = crate::services::vault_namespace::scoped_data_path(&data, &start.root_scope);
    let mut first_store =
        KnowledgeStore::with_event_recorder(vault.clone(), data.clone(), first_coordinator.clone());
    first_store
        .adopt_coordinated_vault_path(vault.clone(), &derived)
        .unwrap();
    let mut second_store =
        KnowledgeStore::with_event_recorder(vault.clone(), data.clone(), second_coordinator);
    second_store
        .adopt_coordinated_vault_path(vault.clone(), &derived)
        .unwrap();
    let first_service = MarkdownMigrationService::try_new(data.clone()).unwrap();
    let second_service = MarkdownMigrationService::try_new(data.clone()).unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = first_service
        .preview_scoped(&mut first_store, start.clone(), request.clone())
        .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let spawn_apply =
        |service: MarkdownMigrationService,
         mut store: KnowledgeStore,
         barrier: Arc<std::sync::Barrier>,
         preview_id: String,
         request: MarkdownMigrationRequest,
         authority: crate::services::vault_namespace::VaultAuthorityTokenV1| {
            std::thread::spawn(move || {
                barrier.wait();
                service.apply_transaction(&preview_id, request, &mut store, authority)
            })
        };
    let first = spawn_apply(
        first_service,
        first_store,
        barrier.clone(),
        preview.preview_id.clone(),
        request.clone(),
        start.clone(),
    );
    let second = spawn_apply(
        second_service,
        second_store,
        barrier.clone(),
        preview.preview_id.clone(),
        request,
        start.clone(),
    );
    barrier.wait();
    assert!(first.join().unwrap().is_ok());
    assert!(second.join().unwrap().is_ok());
    let final_authority = first_coordinator.current_authority_token().unwrap();
    assert_eq!(
        final_authority.authority_generation,
        start.authority_generation + 1
    );
    assert!(vault.join("_grafyn/program.md").is_file());
}

#[test]
fn apply_rejects_full_snapshot_drift_after_prepared_before_first_authority_write() {
    let mut harness = TransactionHarness::new();
    std::fs::write(harness.vault.join("alpha.md"), "# Alpha\n\nOriginal").unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    harness
        .service
        .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
        .unwrap();
    let authority = harness.coordinator.current_authority_token().unwrap();
    std::fs::write(harness.vault.join("unrelated.md"), "# Unrelated").unwrap();

    let error = harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request,
            &mut harness.store,
            authority.clone(),
        )
        .unwrap_err();

    assert!(error.to_string().contains("physical inventory changed"));
    assert_eq!(
        harness.coordinator.current_authority_token().unwrap(),
        authority
    );
    assert!(!harness.vault.join("_grafyn/program.md").exists());
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(
            harness
                .service
                .runs_dir
                .join(&preview.preview_id)
                .join("manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["status"], "prepared");
    assert_eq!(manifest["apply_next"], 0);
}

#[test]
fn apply_rejects_deleted_markdown_and_overlay_inventory_drift_matrix() {
    for drift in [
        "markdown_deleted",
        "overlay_added",
        "overlay_deleted",
        "orphan_overlay_added",
        "orphan_overlay_deleted",
    ] {
        let mut harness = TransactionHarness::new();
        std::fs::write(
            harness.vault.join("alpha.md"),
            "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
        )
        .unwrap();
        let overlay_bytes = b"{\"properties\":{},\"aliases\":[],\"tags\":[]}";
        if matches!(drift, "overlay_deleted" | "orphan_overlay_deleted") {
            let note_id = if drift == "overlay_deleted" {
                "alpha"
            } else {
                "orphan"
            };
            let path = harness.overlay_path(note_id);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, overlay_bytes).unwrap();
        }

        let request = MarkdownMigrationRequest::default();
        let preview = harness.preview(request.clone());
        harness
            .service
            .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
            .unwrap();
        match drift {
            "markdown_deleted" => std::fs::remove_file(harness.vault.join("alpha.md")).unwrap(),
            "overlay_added" => {
                std::fs::write(harness.overlay_path("alpha"), overlay_bytes).unwrap()
            }
            "overlay_deleted" => std::fs::remove_file(harness.overlay_path("alpha")).unwrap(),
            "orphan_overlay_added" => {
                std::fs::write(harness.overlay_path("orphan"), overlay_bytes).unwrap()
            }
            "orphan_overlay_deleted" => {
                std::fs::remove_file(harness.overlay_path("orphan")).unwrap()
            }
            _ => unreachable!(),
        }
        let authority = harness.coordinator.current_authority_token().unwrap();
        let events = harness.events.ordered_events().unwrap();

        let error = harness
            .service
            .apply_transaction(
                &preview.preview_id,
                request,
                &mut harness.store,
                authority.clone(),
            )
            .unwrap_err();

        assert!(
            error.to_string().contains("physical inventory changed"),
            "unexpected {drift} error: {error}"
        );
        assert_eq!(
            harness.coordinator.current_authority_token().unwrap(),
            authority
        );
        assert_eq!(harness.events.ordered_events().unwrap(), events);
        assert!(!harness.vault.join("_grafyn/program.md").exists());
    }
}

#[test]
fn status_is_canonical_bounded_and_fail_closed_for_current_records() {
    let mut harness = TransactionHarness::new();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let authority = harness.coordinator.current_authority_token().unwrap();

    assert!(harness
        .service
        .status_scoped(Some("../outside"), &authority)
        .is_err());
    assert!(harness
        .service
        .status_scoped(Some(&preview.preview_id.to_uppercase()), &authority)
        .is_err());

    harness
        .service
        .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
        .unwrap();
    let manifest_path = harness.run_record_path(&preview.preview_id, "manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["status"] = Value::String("applied".into());
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    assert!(harness
        .service
        .status_scoped(Some(&preview.preview_id), &authority)
        .is_err());
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(manifest_path).unwrap()).unwrap()["status"],
        "applied"
    );
}

#[test]
fn clean_head_legacy_preview_remains_bounded_audit_only() {
    let mut harness = TransactionHarness::new();
    let preview = harness.preview(MarkdownMigrationRequest::default());
    let preview_path = harness.run_record_path(&preview.preview_id, "preview.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&preview_path).unwrap()).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("schema_version");
    object.remove("authority");
    object.remove("request");
    object.remove("source_inventory");
    object.remove("overlay_inventory");
    std::fs::write(&preview_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

    let authority = harness.coordinator.current_authority_token().unwrap();
    let status = harness
        .service
        .status_scoped(Some(&preview.preview_id), &authority)
        .unwrap();
    assert_eq!(status.status, "legacy_audit_only");
    assert!(!status.rollback_available);

    let latest = harness.service.status_scoped(None, &authority).unwrap();
    assert_eq!(latest.run_id.as_deref(), Some(preview.preview_id.as_str()));
    assert_eq!(latest.status, "legacy_audit_only");
    assert!(!latest.rollback_available);
}

#[test]
fn explicit_legacy_status_rejects_a_different_scoped_root_but_keeps_unscoped_audit() {
    let mut harness = TransactionHarness::new();
    let preview = harness.preview(MarkdownMigrationRequest::default());
    let preview_path = harness.run_record_path(&preview.preview_id, "preview.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&preview_path).unwrap()).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("schema_version");
    object.remove("authority");
    object.remove("request");
    object.remove("source_inventory");
    object.remove("overlay_inventory");
    value["root_scope"] = serde_json::to_value(crate::services::twin_events::digest_bytes(
        b"another Markdown root",
    ))
    .unwrap();
    std::fs::write(&preview_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let authority = harness.coordinator.current_authority_token().unwrap();

    assert!(harness
        .service
        .status_scoped(Some(&preview.preview_id), &authority)
        .is_err());

    value["root_scope"] = Value::Null;
    std::fs::write(&preview_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let status = harness
        .service
        .status_scoped(Some(&preview.preview_id), &authority)
        .unwrap();
    assert_eq!(status.status, "legacy_audit_only");
    assert!(!status.rollback_available);
}

#[test]
fn migration_operation_inventory_is_bounded_at_256() {
    let operation = |index: usize| {
        let digest = crate::services::twin_events::digest_bytes(index.to_string().as_bytes());
        MigrationOperationV1 {
            index,
            target_kind: crate::services::twin_events::TargetKind::Markdown,
            target_key: format!("bounded/{index}.md"),
            before: crate::services::twin_events::BeforeImage::Absent,
            after: crate::services::twin_events::BeforeImage::Sha256(digest.clone()),
            after_digest: digest,
            after_blob_key: Some(format!("runs/bounded/blobs/{index}.after")),
            before_blob_key: None,
            note_event: None,
            rollback_note_event: None,
        }
    };
    let mut operations = (0..256).map(operation).collect::<Vec<_>>();
    super::validation::validate_operation_sequence(&operations).unwrap();

    operations.push(operation(256));
    let error = super::validation::validate_operation_sequence(&operations).unwrap_err();
    assert!(error.to_string().contains("inventory is too large"));
}

#[test]
fn latest_status_scan_enforces_an_aggregate_preview_byte_budget() {
    let mut harness = TransactionHarness::new();
    let preview = harness.preview(MarkdownMigrationRequest::default());
    let preview_bytes =
        std::fs::metadata(harness.run_record_path(&preview.preview_id, "preview.json"))
            .unwrap()
            .len() as usize;
    let authority = harness.coordinator.current_authority_token().unwrap();
    harness
        .service
        .set_status_scan_byte_limit_for_test(preview_bytes.saturating_sub(1));

    let error = harness.service.status_scoped(None, &authority).unwrap_err();
    assert!(error.to_string().contains("aggregate byte limit"));
    assert_eq!(
        harness
            .service
            .status_scoped(Some(&preview.preview_id), &authority)
            .unwrap()
            .status,
        "previewed"
    );
}

#[test]
fn applied_manifest_operation_tamper_is_rejected_by_status_and_rollback_without_effect() {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let run_id = apply_and_get_run(&mut harness, &preview, request);
    let manifest_path = harness.run_record_path(&run_id, "manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let governed = manifest["operations"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|operation| !operation["note_event"].is_null())
        .unwrap();
    governed["note_event"]["observed_at"] = Value::String("2000-01-01T00:00:00Z".into());
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let effects = current_migration_effects(&harness);

    let status_error = harness
        .service
        .status_scoped(Some(&run_id), &effects.0)
        .unwrap_err();
    assert!(status_error.to_string().contains("immutable prepared plan"));
    let rollback_error = harness
        .service
        .rollback_transaction(&run_id, &mut harness.store, effects.0.clone())
        .unwrap_err();
    assert!(rollback_error
        .to_string()
        .contains("immutable prepared plan"));
    assert_eq!(current_migration_effects(&harness), effects);
}

#[test]
fn missing_malformed_and_oversized_plan_reject_every_current_entrypoint_without_effect() {
    for corruption in ["missing", "malformed", "oversized"] {
        let mut harness = TransactionHarness::new();
        std::fs::write(
            harness.vault.join("alpha.md"),
            "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
        )
        .unwrap();
        let request = MarkdownMigrationRequest::default();
        let preview = harness.preview(request.clone());
        let run_id = apply_and_get_run(&mut harness, &preview, request.clone());
        let plan_path = harness.run_record_path(&run_id, "plan.json");
        match corruption {
            "missing" => std::fs::remove_file(&plan_path).unwrap(),
            "malformed" => std::fs::write(&plan_path, b"{not-json").unwrap(),
            "oversized" => {
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&plan_path)
                    .unwrap();
                file.set_len((MAX_MIGRATION_RECORD_BYTES + 1) as u64)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let effects = current_migration_effects(&harness);

        assert!(harness
            .service
            .apply_transaction(&run_id, request, &mut harness.store, effects.0.clone(),)
            .is_err());
        assert!(harness
            .service
            .status_scoped(Some(&run_id), &effects.0)
            .is_err());
        assert!(harness
            .service
            .rollback_transaction(&run_id, &mut harness.store, effects.0.clone())
            .is_err());
        assert_eq!(current_migration_effects(&harness), effects);
    }
}

#[test]
fn aggregate_blob_budget_is_checked_before_apply_status_or_rollback_effects() {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let run_id = apply_and_get_run(&mut harness, &preview, request.clone());
    let effects = current_migration_effects(&harness);
    harness.service.set_blob_byte_limit_for_test(1);

    for error in [
        harness
            .service
            .apply_transaction(&run_id, request, &mut harness.store, effects.0.clone())
            .unwrap_err(),
        harness
            .service
            .status_scoped(Some(&run_id), &effects.0)
            .unwrap_err(),
        harness
            .service
            .rollback_transaction(&run_id, &mut harness.store, effects.0.clone())
            .unwrap_err(),
    ] {
        assert!(
            error.to_string().contains("aggregate limit"),
            "unexpected aggregate-budget error: {error}"
        );
    }
    assert_eq!(current_migration_effects(&harness), effects);
}

#[test]
fn tampered_active_witness_cannot_consume_an_unrelated_receipt_or_abort_marker() {
    for proof_kind in ["receipt", "abort"] {
        let mut harness = TransactionHarness::new();
        std::fs::write(
            harness.vault.join("alpha.md"),
            "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
        )
        .unwrap();
        let request = MarkdownMigrationRequest::default();
        let preview = harness.preview(request.clone());
        harness
            .service
            .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
            .unwrap();
        let expected = harness.coordinator.current_authority_token().unwrap();
        let mut plan = Some(
            crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("migration").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    format!("unrelated-{proof_kind}.md"),
                    "unrelated",
                )],
                Vec::new(),
            )
            .expecting_authority(expected)
            .retaining_commit_receipt(),
        );
        let mut unrelated_intent = None;
        if proof_kind == "abort" {
            harness
                .coordinator
                .fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterPreparedHook);
        }
        let result = harness.coordinator.commit_planned_with_hooks(
            crate::services::twin_events::MutationOrigin::Local,
            &mut || Ok(plan.take()),
            &mut |intent| {
                unrelated_intent = Some(intent.clone());
                Ok(())
            },
            &mut |_| Ok(()),
        );
        assert_eq!(result.is_err(), proof_kind == "abort");
        let unrelated_intent = unrelated_intent.unwrap();
        let proof_path = harness
            .data
            .join("twin/mutations")
            .join(if proof_kind == "receipt" {
                "receipts/v1"
            } else {
                "preauthority/v1"
            })
            .join(format!("{}.json", unrelated_intent.mutation_id.as_str()));
        let proof_bytes = std::fs::read(&proof_path).unwrap();

        harness
            .service
            .install_prehook_witness_for_test(&preview.preview_id)
            .unwrap();
        let manifest_path = harness.run_record_path(&preview.preview_id, "manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest["active_step"]["intent"] = serde_json::to_value(&unrelated_intent).unwrap();
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let authority = harness.coordinator.current_authority_token().unwrap();

        assert!(harness
            .service
            .apply_transaction(&preview.preview_id, request, &mut harness.store, authority,)
            .is_err());
        assert_eq!(std::fs::read(&proof_path).unwrap(), proof_bytes);
    }
}

#[test]
fn semantic_preview_and_manifest_tampering_are_rejected_before_authority_write() {
    let mut preview_harness = TransactionHarness::new();
    std::fs::write(preview_harness.vault.join("alpha.md"), "# Alpha\n\nBody").unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = preview_harness.preview(request.clone());
    let preview_path = preview_harness.run_record_path(&preview.preview_id, "preview.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&preview_path).unwrap()).unwrap();
    value["note_proposals"][0]["inferred_tags"] = serde_json::json!(["injected"]);
    std::fs::write(&preview_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let authority = preview_harness
        .coordinator
        .current_authority_token()
        .unwrap();
    assert!(preview_harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request,
            &mut preview_harness.store,
            authority.clone(),
        )
        .is_err());
    assert_eq!(
        preview_harness
            .coordinator
            .current_authority_token()
            .unwrap(),
        authority
    );
    assert!(!preview_harness.vault.join("_grafyn/program.md").exists());

    let mut manifest_harness = TransactionHarness::new();
    std::fs::write(
        manifest_harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nBody",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = manifest_harness.preview(request.clone());
    manifest_harness
        .service
        .prepare_apply_only_for_test(&preview, &request, &mut manifest_harness.store)
        .unwrap();
    let manifest_path = manifest_harness.run_record_path(&preview.preview_id, "manifest.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let operation = value["operations"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|operation| !operation["note_event"].is_null())
        .unwrap();
    operation["note_event"]["payload_digest"] =
        serde_json::to_value(crate::services::twin_events::digest_bytes(b"tampered")).unwrap();
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let authority = manifest_harness
        .coordinator
        .current_authority_token()
        .unwrap();
    assert!(manifest_harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request,
            &mut manifest_harness.store,
            authority.clone(),
        )
        .is_err());
    assert_eq!(
        manifest_harness
            .coordinator
            .current_authority_token()
            .unwrap(),
        authority
    );
    assert!(!manifest_harness.vault.join("_grafyn/program.md").exists());
}

#[test]
fn partial_resume_rejects_tampered_remaining_operation_before_any_new_effect() {
    let mut harness = TransactionHarness::new();
    for note_id in ["alpha", "beta"] {
        std::fs::write(
            harness.vault.join(format!("{note_id}.md")),
            format!("---\nnote_id: {note_id}\ntitle: {note_id}\n---\n\nOriginal {note_id}"),
        )
        .unwrap();
    }
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let start = harness.coordinator.current_authority_token().unwrap();
    harness
        .service
        .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
        .unwrap();
    harness.service.fail_manifest_write_at_for_test(10);
    let first = harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request.clone(),
            &mut harness.store,
            start,
        )
        .unwrap();
    assert!(first.into_parts().4);

    let manifest_path = harness.run_record_path(&preview.preview_id, "manifest.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let apply_next = manifest["apply_next"].as_u64().unwrap();
    let operation = manifest["operations"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|operation| {
            operation["index"].as_u64().unwrap() > apply_next && !operation["note_event"].is_null()
        })
        .expect("fixture must retain a later governed operation");
    let tampered_apply =
        crate::services::twin_events::digest_bytes(b"tampered remaining apply state");
    let tampered_before =
        crate::services::twin_events::digest_bytes(b"tampered remaining before state");
    operation["note_event"]["payload_digest"] = serde_json::to_value(&tampered_apply).unwrap();
    operation["note_event"]["evidence_digest"] = serde_json::to_value(&tampered_before).unwrap();
    operation["rollback_note_event"]["payload_digest"] =
        serde_json::to_value(&tampered_before).unwrap();
    operation["rollback_note_event"]["evidence_digest"] =
        serde_json::to_value(&tampered_apply).unwrap();
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let authority = harness.coordinator.current_authority_token().unwrap();
    let events_before = harness.events.ordered_events().unwrap();
    let error = harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request,
            &mut harness.store,
            authority.clone(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("immutable prepared plan"));
    assert_eq!(
        harness.coordinator.current_authority_token().unwrap(),
        authority
    );
    assert_eq!(harness.events.ordered_events().unwrap(), events_before);
}

#[test]
fn multi_operation_apply_chains_and_publishes_the_exact_final_authority() {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nBody",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let starting = harness.coordinator.current_authority_token().unwrap();
    let outcome = harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request,
            &mut harness.store,
            starting.clone(),
        )
        .unwrap();
    let (result, commit, authority, _, partial) = outcome.into_parts();
    assert!(!partial);
    let commit = commit.expect("multi-operation apply must return its final commit");
    let authority = authority.expect("multi-operation apply must return final authority");
    assert_eq!(commit.authority_token.as_ref(), Some(&authority));

    let manifest: Value = serde_json::from_slice(
        &std::fs::read(harness.run_record_path(&result.run_id, "manifest.json")).unwrap(),
    )
    .unwrap();
    let operation_count = manifest["operations"].as_array().unwrap().len() as u64;
    assert!(operation_count >= 2);
    assert_eq!(manifest["apply_next"].as_u64(), Some(operation_count));
    assert_eq!(
        manifest["final_authority"]["authority_generation"].as_u64(),
        Some(starting.authority_generation + operation_count)
    );
    assert_eq!(
        manifest["last_commit"]["authority"],
        manifest["final_authority"]
    );
    assert_eq!(
        manifest["final_authority"],
        serde_json::to_value(&authority).unwrap()
    );
    assert_eq!(
        manifest["last_commit"]["mutation_id"],
        serde_json::to_value(commit.mutation_id.unwrap()).unwrap()
    );
}

#[test]
fn completed_manifest_rejects_a_tampered_last_commit_id() {
    let mut harness = TransactionHarness::new();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let run_id = apply_and_get_run(&mut harness, &preview, request);
    let authority = harness.coordinator.current_authority_token().unwrap();

    tamper_manifest_last_commit_id(&harness, &run_id);

    assert!(harness
        .service
        .status_scoped(Some(&run_id), &authority)
        .is_err());
}

#[test]
fn status_marks_an_applied_run_stale_after_a_peer_authority_write() {
    let mut harness = TransactionHarness::new();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let run_id = apply_and_get_run(&mut harness, &preview, request);
    let applied_authority = harness.coordinator.current_authority_token().unwrap();
    assert!(
        harness
            .service
            .status_scoped(Some(&run_id), &applied_authority)
            .unwrap()
            .rollback_available
    );

    let (_, peer_commit) = harness
        .store
        .create_note_from_source_with_commit(
            NoteCreate {
                title: "Peer".into(),
                content: "peer write".into(),
                relative_path: Some("peer.md".into()),
                aliases: Vec::new(),
                status: Default::default(),
                tags: Vec::new(),
                schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: None,
                optimizer_managed: false,
                properties: HashMap::new(),
            },
            "note_editor",
        )
        .unwrap();
    let current = peer_commit.authority_token.unwrap();
    let status = harness
        .service
        .status_scoped(Some(&run_id), &current)
        .unwrap();
    assert_eq!(status.status, "applied");
    assert!(!status.rollback_available);
}

#[test]
fn corrupt_present_current_records_are_never_treated_as_absent() {
    let mut harness = TransactionHarness::new();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    harness
        .service
        .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
        .unwrap();
    let manifest_path = harness
        .service
        .runs_dir
        .join(&preview.preview_id)
        .join("manifest.json");
    let corrupt = br#"{"schema_version":1,"run_id":"truncated"}"#;
    std::fs::write(&manifest_path, corrupt).unwrap();
    let authority = harness.coordinator.current_authority_token().unwrap();

    assert!(harness
        .service
        .apply_transaction(&preview.preview_id, request, &mut harness.store, authority)
        .is_err());
    assert_eq!(std::fs::read(manifest_path).unwrap(), corrupt);
    assert!(!harness.vault.join("_grafyn/program.md").exists());
}

#[test]
fn prehook_crash_without_finalized_mutation_resumes_without_wedging() {
    let mut harness = TransactionHarness::new();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    harness
        .service
        .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
        .unwrap();
    harness
        .service
        .install_prehook_witness_for_test(&preview.preview_id)
        .unwrap();
    let authority = harness.coordinator.current_authority_token().unwrap();

    let outcome = harness
        .service
        .apply_transaction(&preview.preview_id, request, &mut harness.store, authority)
        .unwrap();
    let (result, commit, returned_authority, _, partial) = outcome.into_parts();

    assert_eq!(result.status, "applied");
    assert!(!partial);
    assert!(commit.and_then(|commit| commit.mutation_id).is_some());
    assert_eq!(
        returned_authority,
        Some(harness.coordinator.current_authority_token().unwrap())
    );
    assert!(harness.vault.join("_grafyn/program.md").is_file());
}

fn assert_preauthority_marker_restart_recovers(direction: MigrationDirectionV1) {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    if direction == MigrationDirectionV1::Rollback {
        apply_and_get_run(&mut harness, &preview, request.clone());
    }
    assert_eq!(retained_proof_inventory_count(&harness), 0);
    let authority = harness.coordinator.current_authority_token().unwrap();
    harness
        .coordinator
        .fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterPreAuthorityMarker);

    match direction {
        MigrationDirectionV1::Apply => assert!(harness
            .service
            .apply_transaction(
                &preview.preview_id,
                request.clone(),
                &mut harness.store,
                authority.clone(),
            )
            .is_err()),
        MigrationDirectionV1::Rollback => assert!(harness
            .service
            .rollback_transaction(&preview.preview_id, &mut harness.store, authority.clone())
            .is_err()),
    }
    assert_eq!(
        harness.coordinator.current_authority_token().unwrap(),
        authority
    );
    assert_eq!(retained_proof_inventory_count(&harness), 0);
    restart_migration_service(&mut harness);

    match direction {
        MigrationDirectionV1::Apply => {
            let (result, _, _, _, partial) = harness
                .service
                .apply_transaction(&preview.preview_id, request, &mut harness.store, authority)
                .unwrap()
                .into_parts();
            assert!(!partial);
            assert_eq!(result.status, "applied");
        }
        MigrationDirectionV1::Rollback => {
            let (result, _, _, _, partial) = harness
                .service
                .rollback_transaction(&preview.preview_id, &mut harness.store, authority)
                .unwrap()
                .into_parts();
            assert!(!partial);
            assert!(result.rolled_back);
        }
    }
    assert_eq!(retained_proof_inventory_count(&harness), 0);
}

#[test]
fn apply_recovers_after_preauthority_marker_before_authority() {
    assert_preauthority_marker_restart_recovers(MigrationDirectionV1::Apply);
}

#[test]
fn rollback_recovers_after_preauthority_marker_before_authority() {
    assert_preauthority_marker_restart_recovers(MigrationDirectionV1::Rollback);
}

fn assert_exact_intent_without_marker_restart_recovers(direction: MigrationDirectionV1) {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    match direction {
        MigrationDirectionV1::Apply => harness
            .service
            .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
            .unwrap(),
        MigrationDirectionV1::Rollback => {
            apply_and_get_run(&mut harness, &preview, request.clone());
            harness
                .service
                .mark_rollback_resume_for_test(&preview.preview_id)
                .unwrap();
        }
    }

    let mut manifest = harness
        .service
        .load_strict_manifest(&preview.preview_id)
        .unwrap();
    let operation_index = match direction {
        MigrationDirectionV1::Apply => manifest.apply_next,
        MigrationDirectionV1::Rollback => manifest.rollback_total - 1 - manifest.rollback_next,
    };
    let operation = manifest.operations[operation_index].clone();
    let authority = manifest.final_authority.clone().unwrap();
    let target = harness
        .service
        .runtime_target(&manifest, &operation, direction, &harness.store)
        .unwrap();
    manifest.active_step = Some(MigrationStepWitnessV1 {
        direction,
        operation_index: operation.index,
        expected_authority: authority.clone(),
        intent: None,
        committed_authority: None,
        progress_recorded: false,
        receipt_consumed: false,
        abort_recorded: false,
    });
    harness.service.write_strict_manifest(&manifest).unwrap();
    assert_eq!(retained_proof_inventory_count(&harness), 0);

    harness
        .coordinator
        .fail_once_at(crate::services::twin_events::MutationFaultPoint::BeforePreAuthorityMarker);
    let manifest_cell = std::cell::RefCell::new(manifest);
    let mut prepared_hook = |intent: &crate::services::twin_events::MutationIntentV1| {
        let mut durable = manifest_cell.borrow_mut();
        durable.active_step.as_mut().unwrap().intent = Some(intent.clone());
        harness
            .service
            .write_strict_manifest(&durable)
            .map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })
    };
    let mut committed_hook = |_: &crate::services::twin_events::MutationCommit| Ok(());
    let result = harness.store.commit_exact_migration_target_with_hooks(
        target,
        authority.clone(),
        &mut prepared_hook,
        &mut committed_hook,
    );
    assert!(result.is_err());

    let durable = harness
        .service
        .load_strict_manifest(&preview.preview_id)
        .unwrap();
    assert!(durable
        .active_step
        .as_ref()
        .and_then(|witness| witness.intent.as_ref())
        .is_some());
    assert_eq!(
        harness.coordinator.current_authority_token().unwrap(),
        authority
    );
    assert_eq!(harness.coordinator.pending_count().unwrap(), 0);
    assert_eq!(retained_proof_inventory_count(&harness), 0);

    restart_migration_service(&mut harness);
    match direction {
        MigrationDirectionV1::Apply => {
            let (result, _, _, _, partial) = harness
                .service
                .apply_transaction(&preview.preview_id, request, &mut harness.store, authority)
                .unwrap()
                .into_parts();
            assert!(!partial);
            assert_eq!(result.status, "applied");
        }
        MigrationDirectionV1::Rollback => {
            let (result, _, _, _, partial) = harness
                .service
                .rollback_transaction(&preview.preview_id, &mut harness.store, authority)
                .unwrap()
                .into_parts();
            assert!(!partial);
            assert!(result.rolled_back);
        }
    }
    assert_eq!(retained_proof_inventory_count(&harness), 0);
}

#[test]
fn apply_recovers_exact_intent_crash_before_preauthority_marker() {
    assert_exact_intent_without_marker_restart_recovers(MigrationDirectionV1::Apply);
}

#[test]
fn rollback_recovers_exact_intent_crash_before_preauthority_marker() {
    assert_exact_intent_without_marker_restart_recovers(MigrationDirectionV1::Rollback);
}

#[test]
fn post_authority_publication_failure_returns_partial_and_recovers_exactly_once() {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let start = harness.coordinator.current_authority_token().unwrap();
    harness
        .service
        .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
        .unwrap();
    // Program consumes six manifest publications. Fail the overlay's durable
    // progress publication after its target/event/receipt have committed.
    harness.service.fail_manifest_write_at_for_test(10);

    let outcome = harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request.clone(),
            &mut harness.store,
            start,
        )
        .unwrap();
    let (result, commit, authority, warning, partial) = outcome.into_parts();
    assert!(partial);
    assert_eq!(result.status, "partial");
    assert!(commit.and_then(|commit| commit.mutation_id).is_some());
    assert!(warning.is_some());
    assert_eq!(
        authority,
        Some(harness.coordinator.current_authority_token().unwrap())
    );
    assert!(harness.vault.join("_grafyn/program.md").exists());
    let committed_events = harness.events.ordered_events().unwrap();
    let committed_alpha_events = alpha_updated_events(&committed_events);
    assert_eq!(committed_alpha_events.len(), 1);
    let committed_event_id = committed_alpha_events[0].event_id.clone();

    let resumed_authority = harness.coordinator.current_authority_token().unwrap();
    let resumed = harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request,
            &mut harness.store,
            resumed_authority,
        )
        .unwrap();
    let (result, commit, authority, _, partial) = resumed.into_parts();
    assert_eq!(result.status, "applied");
    assert!(!partial);
    assert!(commit.and_then(|commit| commit.mutation_id).is_some());
    assert_eq!(
        authority,
        Some(harness.coordinator.current_authority_token().unwrap())
    );
    assert!(harness.vault.join("_grafyn/program.md").is_file());
    let resumed_events = harness.events.ordered_events().unwrap();
    assert_eq!(alpha_updated_events(&resumed_events).len(), 1);
    assert_eq!(
        alpha_updated_events(&resumed_events)[0].event_id,
        committed_event_id
    );
    let completed_authority = harness.coordinator.current_authority_token().unwrap();
    let retried = harness
        .service
        .apply_transaction(
            &preview.preview_id,
            MarkdownMigrationRequest::default(),
            &mut harness.store,
            completed_authority.clone(),
        )
        .unwrap();
    assert_eq!(retried.into_parts().0.status, "applied");
    assert_eq!(
        harness.coordinator.current_authority_token().unwrap(),
        completed_authority
    );
    let retried_events = harness.events.ordered_events().unwrap();
    assert_eq!(alpha_updated_events(&retried_events).len(), 1);
}

#[test]
fn progress_witness_receipt_flag_cannot_skip_owned_proof_cleanup() {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    harness
        .service
        .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
        .unwrap();
    harness.service.fail_manifest_write_at_for_test(10);
    let authority = harness.coordinator.current_authority_token().unwrap();
    let outcome = harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request.clone(),
            &mut harness.store,
            authority,
        )
        .unwrap();
    let (_, commit, returned_authority, _, partial) = outcome.into_parts();
    assert!(partial);
    let mutation_id = commit.unwrap().mutation_id.unwrap();
    assert_eq!(retained_proof_count(&harness, &mutation_id), 1);
    mark_active_receipt_consumed(&harness, &preview.preview_id);
    restart_migration_service(&mut harness);

    let resumed = harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request,
            &mut harness.store,
            returned_authority.unwrap(),
        )
        .unwrap();

    assert!(!resumed.into_parts().4);
    assert_eq!(retained_proof_count(&harness, &mutation_id), 0);
}

#[test]
fn preprogress_receipt_flag_cannot_skip_owned_abort_or_commit_proof_cleanup() {
    for proof_kind in ["abort", "commit"] {
        let mut harness = TransactionHarness::new();
        std::fs::write(
            harness.vault.join("alpha.md"),
            "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
        )
        .unwrap();
        let request = MarkdownMigrationRequest::default();
        let preview = harness.preview(request.clone());
        harness
            .service
            .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
            .unwrap();

        let mut manifest = harness
            .service
            .load_strict_manifest(&preview.preview_id)
            .unwrap();
        let operation = manifest.operations[manifest.apply_next].clone();
        let operation_count = manifest.operations.len() as u64;
        let authority = manifest.final_authority.clone().unwrap();
        let target = harness
            .service
            .runtime_target(
                &manifest,
                &operation,
                MigrationDirectionV1::Apply,
                &harness.store,
            )
            .unwrap();
        manifest.active_step = Some(MigrationStepWitnessV1 {
            direction: MigrationDirectionV1::Apply,
            operation_index: operation.index,
            expected_authority: authority.clone(),
            intent: None,
            committed_authority: None,
            progress_recorded: false,
            receipt_consumed: false,
            abort_recorded: false,
        });
        harness.service.write_strict_manifest(&manifest).unwrap();

        let manifest_cell = std::cell::RefCell::new(manifest);
        let mut prepared_hook = |intent: &crate::services::twin_events::MutationIntentV1| {
            let mut durable = manifest_cell.borrow_mut();
            durable.active_step.as_mut().unwrap().intent = Some(intent.clone());
            harness
                .service
                .write_strict_manifest(&durable)
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })
        };
        let mut committed_hook = |_: &crate::services::twin_events::MutationCommit| {
            Err(crate::services::twin_events::MutationError::Invalid(
                "simulated crash before migration commit publication".into(),
            ))
        };
        if proof_kind == "abort" {
            harness
                .coordinator
                .fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterPreparedHook);
        }
        let result = harness.store.commit_exact_migration_target_with_hooks(
            target,
            authority.clone(),
            &mut prepared_hook,
            &mut committed_hook,
        );
        let mut manifest = manifest_cell.into_inner();
        if proof_kind == "abort" {
            assert!(result.is_err());
            manifest.active_step.as_mut().unwrap().abort_recorded = true;
            harness.service.write_strict_manifest(&manifest).unwrap();
        } else {
            let commit = result.unwrap();
            assert!(commit.postcommit_warning);
        }
        let mutation_id = manifest
            .active_step
            .as_ref()
            .and_then(|witness| witness.intent.as_ref())
            .map(|intent| intent.mutation_id.clone())
            .unwrap();
        assert_eq!(retained_proof_count(&harness, &mutation_id), 1);

        mark_active_receipt_consumed(&harness, &preview.preview_id);
        restart_migration_service(&mut harness);
        let current = harness.coordinator.current_authority_token().unwrap();
        let resumed = harness
            .service
            .apply_transaction(&preview.preview_id, request, &mut harness.store, current)
            .unwrap();
        let (result, _, _, _, partial) = resumed.into_parts();

        assert!(
            !partial,
            "{proof_kind} recovery unexpectedly stopped partial"
        );
        assert_eq!(result.status, "applied");
        assert_eq!(retained_proof_count(&harness, &mutation_id), 0);
        assert_eq!(
            harness
                .coordinator
                .current_authority_token()
                .unwrap()
                .authority_generation,
            authority.authority_generation + operation_count,
            "{proof_kind} recovery duplicated or skipped a migration operation"
        );
    }
}

#[test]
fn partial_manifest_rejects_a_tampered_last_commit_id() {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    harness
        .service
        .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
        .unwrap();
    let manifest = harness
        .service
        .load_strict_manifest(&preview.preview_id)
        .unwrap();
    let operation = manifest.operations[manifest.apply_next].clone();
    let authority = harness.coordinator.current_authority_token().unwrap();
    let partial =
        interrupt_after_authority_with_target_drift(&mut harness, &operation, |harness| {
            harness
                .service
                .apply_transaction(&preview.preview_id, request, &mut harness.store, authority)
                .unwrap()
        });
    assert_exact_partial(partial);
    tamper_manifest_last_commit_id(&harness, &preview.preview_id);
    let current = harness.coordinator.current_authority_token().unwrap();

    assert!(harness
        .service
        .status_scoped(Some(&preview.preview_id), &current)
        .is_err());
}

#[test]
fn retained_migration_wal_owns_after_authority_advance_before_effect() {
    let mut harness = TransactionHarness::new();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let start = harness.coordinator.current_authority_token().unwrap();
    harness
        .coordinator
        .fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterAuthorityAdvance);

    let outcome = harness.service.apply_transaction(
        &preview.preview_id,
        request,
        &mut harness.store,
        start.clone(),
    );

    assert!(
        outcome.is_ok(),
        "owned post-authority failure must not be returned as retryable Err: {outcome:?}"
    );
    assert!(harness.vault.join("_grafyn/program.md").is_file());
    assert_eq!(harness.coordinator.pending_count().unwrap(), 0);
    assert_eq!(
        harness
            .coordinator
            .current_authority_token()
            .unwrap()
            .authority_generation,
        start.authority_generation + 1
    );
}

#[test]
fn authority_advance_with_persistent_replay_failure_returns_exact_partial() {
    let mut harness = TransactionHarness::new();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    let start = harness.coordinator.current_authority_token().unwrap();
    harness
        .coordinator
        .fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterAuthorityAdvance);
    harness.coordinator.fail_next_replays_before_targets(1);

    let first = harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request.clone(),
            &mut harness.store,
            start.clone(),
        )
        .unwrap();
    let (result, commit, authority, warning, partial) = first.into_parts();
    assert!(partial);
    assert_eq!(result.status, "partial");
    let commit = commit.expect("authority-owning partial commit");
    assert!(commit.mutation_id.is_some());
    let authority = authority.expect("authority-owning partial token");
    assert_eq!(
        authority.authority_generation,
        start.authority_generation + 1
    );
    assert_eq!(commit.authority_token, Some(authority.clone()));
    assert_eq!(
        warning.as_ref().map(|warning| warning.code.as_str()),
        Some("derived_state_unavailable")
    );

    let resumed = harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request,
            &mut harness.store,
            authority.clone(),
        )
        .unwrap();
    let (result, _, resumed_authority, _, partial) = resumed.into_parts();
    assert_eq!(result.status, "applied");
    assert!(!partial);
    assert_eq!(resumed_authority, Some(authority));
    assert!(harness.vault.join("_grafyn/program.md").is_file());
    assert_eq!(harness.coordinator.pending_count().unwrap(), 0);
}

#[test]
fn authority_only_abort_stops_and_restarts_first_and_later_apply_without_projection() {
    for stage in ["first", "later"] {
        let mut harness = TransactionHarness::new();
        std::fs::write(
            harness.vault.join("alpha.md"),
            "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
        )
        .unwrap();
        let request = MarkdownMigrationRequest::default();
        let preview = harness.preview(request.clone());
        harness
            .service
            .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
            .unwrap();
        if stage == "later" {
            harness.service.fail_manifest_write_at_for_test(6);
            let authority = harness.coordinator.current_authority_token().unwrap();
            let partial = harness
                .service
                .apply_transaction(
                    &preview.preview_id,
                    request.clone(),
                    &mut harness.store,
                    authority,
                )
                .unwrap();
            assert_exact_partial(partial);
        }
        let before = harness
            .service
            .load_strict_manifest(&preview.preview_id)
            .unwrap();
        assert!(before.apply_next < before.operations.len());
        assert_eq!(before.apply_next == 0, stage == "first");
        let operation = before.operations[before.apply_next].clone();
        let current = harness.coordinator.current_authority_token().unwrap();

        let outcome =
            interrupt_after_authority_with_target_drift(&mut harness, &operation, |harness| {
                harness
                    .service
                    .apply_transaction(
                        &preview.preview_id,
                        request.clone(),
                        &mut harness.store,
                        current,
                    )
                    .unwrap()
            });
        let (mutation_id, authority) = assert_exact_partial(outcome);
        let after = harness
            .service
            .load_strict_manifest(&preview.preview_id)
            .unwrap();
        assert_eq!(after.apply_next, before.apply_next);
        assert_eq!(after.rollback_next, before.rollback_next);
        assert_eq!(after.source_inventory, before.source_inventory);
        assert_eq!(after.overlay_inventory, before.overlay_inventory);
        assert_eq!(
            after.authority_only_advances,
            before.authority_only_advances + 1
        );
        assert!(after.active_step.is_none());
        assert_eq!(
            after.last_commit.as_ref().map(|proof| &proof.mutation_id),
            Some(&mutation_id)
        );
        assert_eq!(after.final_authority.as_ref(), Some(&authority));
        assert_eq!(retained_proof_count(&harness, &mutation_id), 0);

        restart_migration_service(&mut harness);
        let resumed = harness
            .service
            .apply_transaction(&preview.preview_id, request, &mut harness.store, authority)
            .unwrap();
        let (result, _, _, _, partial) = resumed.into_parts();
        assert!(!partial, "{stage} apply restart remained partial");
        assert_eq!(result.status, "applied");
        let completed = harness
            .service
            .load_strict_manifest(&preview.preview_id)
            .unwrap();
        assert_eq!(completed.authority_only_advances, 1);
        assert_eq!(
            completed
                .final_authority
                .as_ref()
                .unwrap()
                .authority_generation,
            completed
                .starting_authority
                .as_ref()
                .unwrap()
                .authority_generation
                + completed.apply_next as u64
                + completed.rollback_next as u64
                + completed.authority_only_advances
        );
    }
}

#[test]
fn authority_only_abort_stops_and_restarts_first_and_later_rollback_without_projection() {
    for stage in ["first", "later"] {
        let mut harness = TransactionHarness::new();
        std::fs::write(
            harness.vault.join("alpha.md"),
            "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
        )
        .unwrap();
        let request = MarkdownMigrationRequest::default();
        let preview = harness.preview(request.clone());
        let run_id = apply_and_get_run(&mut harness, &preview, request);
        if stage == "later" {
            harness.service.fail_manifest_write_at_for_test(7);
            let authority = harness.coordinator.current_authority_token().unwrap();
            let partial = harness
                .service
                .rollback_transaction(&run_id, &mut harness.store, authority)
                .unwrap();
            assert_exact_partial(partial);
        }
        let before = harness.service.load_strict_manifest(&run_id).unwrap();
        let rollback_total = if before.rollback_total == 0 {
            before.apply_next
        } else {
            before.rollback_total
        };
        assert!(before.rollback_next < rollback_total);
        assert_eq!(before.rollback_next == 0, stage == "first");
        let operation = before.operations[rollback_total - 1 - before.rollback_next].clone();
        let current = harness.coordinator.current_authority_token().unwrap();

        let outcome =
            interrupt_after_authority_with_target_drift(&mut harness, &operation, |harness| {
                harness
                    .service
                    .rollback_transaction(&run_id, &mut harness.store, current)
                    .unwrap()
            });
        let (mutation_id, authority) = assert_exact_partial(outcome);
        let after = harness.service.load_strict_manifest(&run_id).unwrap();
        assert_eq!(after.apply_next, before.apply_next);
        assert_eq!(after.rollback_next, before.rollback_next);
        assert_eq!(after.rollback_total, rollback_total);
        assert_eq!(after.source_inventory, before.source_inventory);
        assert_eq!(after.overlay_inventory, before.overlay_inventory);
        assert_eq!(
            after.authority_only_advances,
            before.authority_only_advances + 1
        );
        assert!(after.active_step.is_none());
        assert_eq!(
            after.last_commit.as_ref().map(|proof| &proof.mutation_id),
            Some(&mutation_id)
        );
        assert_eq!(after.final_authority.as_ref(), Some(&authority));
        assert_eq!(retained_proof_count(&harness, &mutation_id), 0);

        restart_migration_service(&mut harness);
        let resumed = harness
            .service
            .rollback_transaction(&run_id, &mut harness.store, authority)
            .unwrap();
        let (result, _, _, _, partial) = resumed.into_parts();
        assert!(!partial, "{stage} rollback restart remained partial");
        assert!(result.rolled_back);
        let completed = harness.service.load_strict_manifest(&run_id).unwrap();
        assert_eq!(completed.authority_only_advances, 1);
        assert_eq!(
            completed
                .final_authority
                .as_ref()
                .unwrap()
                .authority_generation,
            completed
                .starting_authority
                .as_ref()
                .unwrap()
                .authority_generation
                + completed.apply_next as u64
                + completed.rollback_next as u64
                + completed.authority_only_advances
        );
    }
}

#[test]
fn authority_only_abort_survives_manifest_faults_before_and_after_proof_consumption() {
    for (direction, fail_at, proof_count_after_fault) in [
        ("apply", 3, 1),
        ("apply", 4, 0),
        ("rollback", 4, 1),
        ("rollback", 5, 0),
    ] {
        let mut harness = TransactionHarness::new();
        std::fs::write(
            harness.vault.join("alpha.md"),
            "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
        )
        .unwrap();
        let request = MarkdownMigrationRequest::default();
        let preview = harness.preview(request.clone());
        if direction == "apply" {
            harness
                .service
                .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
                .unwrap();
        } else {
            apply_and_get_run(&mut harness, &preview, request.clone());
        }
        let before = harness
            .service
            .load_strict_manifest(&preview.preview_id)
            .unwrap();
        let operation = if direction == "apply" {
            before.operations[before.apply_next].clone()
        } else {
            before.operations[before.apply_next - 1].clone()
        };
        let apply_next = before.apply_next;
        let rollback_next = before.rollback_next;
        let source_inventory = before.source_inventory.clone();
        let overlay_inventory = before.overlay_inventory.clone();
        let current = harness.coordinator.current_authority_token().unwrap();
        harness.service.fail_manifest_write_at_for_test(fail_at);

        let outcome =
            interrupt_after_authority_with_target_drift(&mut harness, &operation, |harness| {
                if direction == "apply" {
                    let outcome = harness
                        .service
                        .apply_transaction(
                            &preview.preview_id,
                            request.clone(),
                            &mut harness.store,
                            current,
                        )
                        .unwrap();
                    let (_, commit, authority, warning, partial) = outcome.into_parts();
                    (commit, authority, warning, partial)
                } else {
                    let outcome = harness
                        .service
                        .rollback_transaction(&preview.preview_id, &mut harness.store, current)
                        .unwrap();
                    let (_, commit, authority, warning, partial) = outcome.into_parts();
                    (commit, authority, warning, partial)
                }
            });
        let (commit, authority, warning, partial) = outcome;
        assert!(partial);
        assert_eq!(
            warning.as_ref().map(|warning| warning.code.as_str()),
            Some("derived_state_unavailable")
        );
        let commit = commit.expect("faulted authority-only abort lost its exact commit");
        let mutation_id = commit.mutation_id.clone().unwrap();
        let authority = authority.unwrap();
        assert_eq!(commit.authority_token.as_ref(), Some(&authority));
        assert_eq!(
            retained_proof_count(&harness, &mutation_id),
            proof_count_after_fault,
            "{direction} manifest fault ordinal {fail_at}"
        );
        let faulted = harness
            .service
            .load_strict_manifest(&preview.preview_id)
            .unwrap();
        assert_eq!(faulted.apply_next, apply_next);
        assert_eq!(faulted.rollback_next, rollback_next);
        assert_eq!(faulted.source_inventory, source_inventory);
        assert_eq!(faulted.overlay_inventory, overlay_inventory);
        assert_eq!(faulted.authority_only_advances, 1);

        restart_migration_service(&mut harness);
        let generation = harness
            .coordinator
            .current_authority_token()
            .unwrap()
            .authority_generation;
        let cleanup = if direction == "apply" {
            let outcome = harness
                .service
                .apply_transaction(
                    &preview.preview_id,
                    request.clone(),
                    &mut harness.store,
                    authority.clone(),
                )
                .unwrap();
            let (_, commit, returned, warning, partial) = outcome.into_parts();
            (commit, returned, warning, partial)
        } else {
            let outcome = harness
                .service
                .rollback_transaction(&preview.preview_id, &mut harness.store, authority.clone())
                .unwrap();
            let (_, commit, returned, warning, partial) = outcome.into_parts();
            (commit, returned, warning, partial)
        };
        assert!(
            cleanup.3,
            "authority-only cleanup must stop its restart call"
        );
        assert_eq!(cleanup.1, Some(authority.clone()));
        assert!(cleanup.0.and_then(|commit| commit.mutation_id).is_some());
        assert!(cleanup.2.is_some());
        assert_eq!(
            harness
                .coordinator
                .current_authority_token()
                .unwrap()
                .authority_generation,
            generation
        );
        assert_eq!(retained_proof_count(&harness, &mutation_id), 0);
        let cleaned = harness
            .service
            .load_strict_manifest(&preview.preview_id)
            .unwrap();
        assert!(cleaned.active_step.is_none());
        assert_eq!(cleaned.apply_next, apply_next);
        assert_eq!(cleaned.rollback_next, rollback_next);

        let completed = if direction == "apply" {
            harness
                .service
                .apply_transaction(
                    &preview.preview_id,
                    request.clone(),
                    &mut harness.store,
                    authority,
                )
                .unwrap()
                .into_parts()
                .4
        } else {
            harness
                .service
                .rollback_transaction(&preview.preview_id, &mut harness.store, authority)
                .unwrap()
                .into_parts()
                .4
        };
        assert!(!completed, "{direction} did not finish after proof cleanup");
    }
}

#[test]
fn authority_only_receipt_flag_cannot_skip_owned_proof_cleanup() {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    harness
        .service
        .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
        .unwrap();
    let before = harness
        .service
        .load_strict_manifest(&preview.preview_id)
        .unwrap();
    let operation = before.operations[before.apply_next].clone();
    let current = harness.coordinator.current_authority_token().unwrap();
    harness.service.fail_manifest_write_at_for_test(3);
    let outcome =
        interrupt_after_authority_with_target_drift(&mut harness, &operation, |harness| {
            harness
                .service
                .apply_transaction(
                    &preview.preview_id,
                    request.clone(),
                    &mut harness.store,
                    current,
                )
                .unwrap()
        });
    let (mutation_id, authority) = assert_exact_partial(outcome);
    assert_eq!(retained_proof_count(&harness, &mutation_id), 1);
    mark_active_receipt_consumed(&harness, &preview.preview_id);
    restart_migration_service(&mut harness);

    let cleanup = harness
        .service
        .apply_transaction(&preview.preview_id, request, &mut harness.store, authority)
        .unwrap();

    assert!(cleanup.into_parts().4);
    assert_eq!(retained_proof_count(&harness, &mutation_id), 0);
}

#[test]
fn authority_only_cap_boundary_is_fail_closed_for_apply_and_rollback() {
    for direction in ["apply", "rollback"] {
        let mut harness = TransactionHarness::new();
        std::fs::write(
            harness.vault.join("alpha.md"),
            "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
        )
        .unwrap();
        let request = MarkdownMigrationRequest::default();
        let preview = harness.preview(request.clone());
        if direction == "apply" {
            harness
                .service
                .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
                .unwrap();
        } else {
            apply_and_get_run(&mut harness, &preview, request.clone());
        }
        let (_, staged_authority) =
            stage_authority_only_count(&mut harness, &preview.preview_id, 255);
        let before = harness
            .service
            .load_strict_manifest(&preview.preview_id)
            .unwrap();
        let operation = if direction == "apply" {
            before.operations[before.apply_next].clone()
        } else {
            before.operations[before.apply_next - 1].clone()
        };

        let boundary =
            interrupt_after_authority_with_target_drift(&mut harness, &operation, |harness| {
                if direction == "apply" {
                    let outcome = harness
                        .service
                        .apply_transaction(
                            &preview.preview_id,
                            request.clone(),
                            &mut harness.store,
                            staged_authority,
                        )
                        .unwrap();
                    let (_, commit, authority, warning, partial) = outcome.into_parts();
                    (commit, authority, warning, partial)
                } else {
                    let outcome = harness
                        .service
                        .rollback_transaction(
                            &preview.preview_id,
                            &mut harness.store,
                            staged_authority,
                        )
                        .unwrap();
                    let (_, commit, authority, warning, partial) = outcome.into_parts();
                    (commit, authority, warning, partial)
                }
            });
        assert!(boundary.3, "{direction} must retain its 256th advance");
        let boundary_commit = boundary.0.unwrap();
        let boundary_mutation = boundary_commit.mutation_id.clone().unwrap();
        let boundary_authority = boundary.1.unwrap();
        assert_eq!(
            boundary_commit.authority_token,
            Some(boundary_authority.clone())
        );
        assert_eq!(retained_proof_count(&harness, &boundary_mutation), 0);

        let capped = harness
            .service
            .load_strict_manifest(&preview.preview_id)
            .unwrap();
        assert_eq!(capped.authority_only_advances, 256);
        assert!(capped.active_step.is_none());
        let target_before = std::fs::read(migration_operation_path(&harness, &operation)).ok();
        let events_before = harness.events.ordered_events().unwrap();
        let pending_before = harness.coordinator.pending_count().unwrap();
        let proofs_before = retained_proof_inventory_count(&harness);

        let stopped = if direction == "apply" {
            let outcome = harness
                .service
                .apply_transaction(
                    &preview.preview_id,
                    request.clone(),
                    &mut harness.store,
                    boundary_authority.clone(),
                )
                .unwrap();
            let (_, commit, authority, warning, partial) = outcome.into_parts();
            (commit, authority, warning, partial)
        } else {
            let outcome = harness
                .service
                .rollback_transaction(
                    &preview.preview_id,
                    &mut harness.store,
                    boundary_authority.clone(),
                )
                .unwrap();
            let (_, commit, authority, warning, partial) = outcome.into_parts();
            (commit, authority, warning, partial)
        };
        assert!(stopped.3, "{direction} cap must return an exact partial");
        assert_eq!(
            stopped.0.and_then(|commit| commit.mutation_id),
            Some(boundary_mutation)
        );
        assert_eq!(stopped.1, Some(boundary_authority.clone()));
        assert!(stopped.2.is_some());
        assert_eq!(
            harness.coordinator.current_authority_token().unwrap(),
            boundary_authority
        );
        assert_eq!(harness.events.ordered_events().unwrap(), events_before);
        assert_eq!(harness.coordinator.pending_count().unwrap(), pending_before);
        assert_eq!(retained_proof_inventory_count(&harness), proofs_before);
        assert_eq!(
            std::fs::read(migration_operation_path(&harness, &operation)).ok(),
            target_before
        );
        let unchanged = harness
            .service
            .load_strict_manifest(&preview.preview_id)
            .unwrap();
        assert_eq!(unchanged.authority_only_advances, 256);
        assert!(unchanged.active_step.is_none());
        assert_eq!(unchanged.apply_next, capped.apply_next);
        assert_eq!(unchanged.rollback_next, capped.rollback_next);
        assert_eq!(unchanged.source_inventory, capped.source_inventory);
        assert_eq!(unchanged.overlay_inventory, capped.overlay_inventory);
    }
}

#[test]
fn capped_manifest_rejects_a_tampered_last_commit_id() {
    let mut harness = TransactionHarness::new();
    std::fs::write(
        harness.vault.join("alpha.md"),
        "---\nnote_id: alpha\ntitle: Alpha\n---\n\nOriginal",
    )
    .unwrap();
    let request = MarkdownMigrationRequest::default();
    let preview = harness.preview(request.clone());
    harness
        .service
        .prepare_apply_only_for_test(&preview, &request, &mut harness.store)
        .unwrap();
    let (_, staged_authority) = stage_authority_only_count(&mut harness, &preview.preview_id, 255);
    let manifest = harness
        .service
        .load_strict_manifest(&preview.preview_id)
        .unwrap();
    let operation = manifest.operations[manifest.apply_next].clone();
    let boundary =
        interrupt_after_authority_with_target_drift(&mut harness, &operation, |harness| {
            harness
                .service
                .apply_transaction(
                    &preview.preview_id,
                    request.clone(),
                    &mut harness.store,
                    staged_authority,
                )
                .unwrap()
        });
    let (_, boundary_authority) = assert_exact_partial(boundary);
    tamper_manifest_last_commit_id(&harness, &preview.preview_id);

    assert!(harness
        .service
        .apply_transaction(
            &preview.preview_id,
            request,
            &mut harness.store,
            boundary_authority,
        )
        .is_err());
}

#[test]
fn strict_preview_canonicalizes_extensionless_program_path_once() {
    let mut harness = TransactionHarness::new();
    let request = MarkdownMigrationRequest {
        program_path: Some("custom/program".into()),
        ..MarkdownMigrationRequest::default()
    };

    let preview = harness.preview(request.clone());
    assert_eq!(preview.program_path, "custom/program.md");
    assert_eq!(
        preview
            .request
            .as_ref()
            .and_then(|value| value.program_path.as_deref()),
        Some("custom/program.md")
    );

    let authority = harness.coordinator.current_authority_token().unwrap();
    let result = harness
        .service
        .apply_transaction(&preview.preview_id, request, &mut harness.store, authority)
        .unwrap();
    let (_, commit, _, _, _) = result.into_parts();
    assert!(commit.and_then(|value| value.authority_token).is_some());
    assert!(harness.vault.join("custom/program.md").is_file());
    assert!(!harness.vault.join("custom/program").exists());
}
