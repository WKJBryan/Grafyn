use super::*;
use crate::models::note::{NoteCreate, NoteStatus};
use crate::models::twin_event::{NoteChangeKind, TwinEventPayload};
use crate::services::twin_events::{BeforeImage, TargetKind};
use tempfile::tempdir;

fn note_create(title: &str) -> NoteCreate {
    NoteCreate {
        title: title.to_string(),
        content: format!("Content for {title}"),
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

fn fixture(
    title: &str,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    std::sync::Arc<crate::services::twin_events::TwinEventStore>,
    std::sync::Arc<crate::services::twin_events::MutationCoordinator>,
    KnowledgeStore,
    VaultOptimizerService,
    Note,
) {
    let vault = tempdir().unwrap();
    let data = tempdir().unwrap();
    let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
        data.path(),
    ));
    events.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            data.path(),
            vault.path(),
            events.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let namespace = coordinator.current_namespace_path().unwrap();
    let mut store = KnowledgeStore::with_event_recorder(
        vault.path().to_path_buf(),
        namespace.clone(),
        coordinator.clone(),
    );
    let note = store.create_note(note_create(title)).unwrap();
    let service = VaultOptimizerService::try_new(namespace).unwrap();
    (vault, data, events, coordinator, store, service, note)
}

fn install_sidecar_change(
    service: &VaultOptimizerService,
    store: &KnowledgeStore,
    note: &Note,
    restore_raw: Option<&[u8]>,
) -> (String, rollback::ExactOptimizerRollbackMaterialV1, Vec<u8>) {
    let markdown_raw = store
        .migration_target_bytes(TargetKind::Markdown, &note.relative_path)
        .unwrap()
        .unwrap();
    let source_digest = crate::services::twin_events::digest_bytes(&markdown_raw);
    let after_value = serde_json::json!({"tags": ["optimizer-after"]});
    let after_raw = serde_json::to_vec_pretty(&after_value).unwrap();
    let apply_after = BeforeImage::Sha256(crate::services::twin_events::digest_bytes(&after_raw));
    let restore_before = restore_raw.map_or(BeforeImage::Absent, |raw| {
        BeforeImage::Sha256(crate::services::twin_events::digest_bytes(raw))
    });
    let apply_payload = rollback::effective_sidecar_digest(&source_digest, &apply_after);
    let rollback_payload = rollback::effective_sidecar_digest(&source_digest, &restore_before);
    let material = rollback::ExactOptimizerRollbackMaterialV1 {
        schema_version: rollback::EXACT_OPTIMIZER_ROLLBACK_SCHEMA_VERSION,
        target_kind: TargetKind::OverlayJson,
        target_key: format!("{}.json", note.id),
        restore_before: restore_before.clone(),
        restore_utf8: restore_raw.map(|raw| String::from_utf8(raw.to_vec()).unwrap()),
        apply_after: apply_after.clone(),
        source_relative_path: note.relative_path.clone(),
        source_digest: source_digest.clone(),
        apply_payload_digest: apply_payload.clone(),
        apply_evidence_digest: rollback_payload.clone(),
        rollback_payload_digest: rollback_payload,
        rollback_evidence_digest: apply_payload,
        apply_governance: store
            .optimizer_overlay_governance(&note.relative_path, &markdown_raw, Some(&after_raw))
            .unwrap(),
        rollback_governance: store
            .optimizer_overlay_governance(&note.relative_path, &markdown_raw, restore_raw)
            .unwrap(),
    };
    std::fs::write(store.overlay_path(&note.id), &after_raw).unwrap();
    let change_id = Uuid::new_v4().to_string();
    service
        .write_change(&OptimizerChange {
            change_id: change_id.clone(),
            note_id: note.id.clone(),
            mode: "sidecar_first".to_string(),
            overlay_before: restore_raw.map(|raw| serde_json::from_slice(raw).unwrap()),
            overlay_after: Some(after_value),
            note_before: None,
            note_after: None,
            markdown_before_digest: Some(source_digest),
            markdown_relative_path: Some(note.relative_path.clone()),
            created_at: Some(Utc::now()),
            exact_rollback: Some(material.clone()),
        })
        .unwrap();
    (change_id, material, after_raw)
}

fn committed_result(
    outcome: OptimizerRollbackMutationOutcome,
) -> (
    crate::models::migration::VaultOptimizerRollbackResult,
    crate::services::twin_events::MutationCommit,
) {
    match outcome {
        OptimizerRollbackMutationOutcome::Committed { result, commit, .. } => (result, commit),
        other => panic!("expected committed optimizer rollback, got {other:?}"),
    }
}

#[test]
fn rollback_restores_noncanonical_overlay_bytes_and_is_idempotent() {
    let (_vault, _data, events, coordinator, mut store, mut service, note) =
        fixture("Exact Overlay Rollback");
    let restore = b"{\r\n  \"tags\" : [ \"before\" ]\r\n}\r\n";
    let (change_id, material, _) = install_sidecar_change(&service, &store, &note, Some(restore));
    let before = coordinator.current_authority_token().unwrap();

    let (result, commit) = committed_result(
        service
            .rollback_change_expecting_authority(&change_id, &mut store, before.clone())
            .unwrap(),
    );
    assert!(result.rolled_back);
    assert_eq!(
        std::fs::read(store.overlay_path(&note.id)).unwrap(),
        restore
    );
    assert_eq!(service.state.rollback_count, 1);
    assert_eq!(
        commit
            .authority_token
            .as_ref()
            .unwrap()
            .authority_generation,
        before.authority_generation + 1
    );

    let governed = events
        .ordered_events()
        .unwrap()
        .into_iter()
        .filter(|event| {
            event.context.source_channel.as_str() == "vault_optimizer"
                && matches!(
                    &event.payload,
                    TwinEventPayload::NoteChanged(changed)
                        if changed.note_id.as_str() == note.id
                            && changed.change == NoteChangeKind::Updated
                            && changed.content_digest.as_ref()
                                == Some(&material.rollback_payload_digest)
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(governed.len(), 1);
    assert_eq!(
        governed[0].evidence[0].digest.as_ref(),
        Some(&material.rollback_evidence_digest)
    );
    assert_eq!(governed[0].governance, material.rollback_governance);

    let authority = coordinator.current_authority_token().unwrap();
    assert!(matches!(
        service
            .rollback_change_expecting_authority(&change_id, &mut store, authority.clone())
            .unwrap(),
        OptimizerRollbackMutationOutcome::NoWrite(_)
    ));
    assert_eq!(coordinator.current_authority_token().unwrap(), authority);
    assert_eq!(service.state.rollback_count, 1);
    assert_eq!(events.ordered_events().unwrap().len(), governed.len() + 1);
}

#[test]
fn rollback_deletes_an_overlay_that_was_absent_before_apply() {
    let (_vault, _data, _events, coordinator, mut store, mut service, note) =
        fixture("Absent Overlay Rollback");
    let (change_id, _, _) = install_sidecar_change(&service, &store, &note, None);

    let (result, commit) = committed_result(
        service
            .rollback_change_expecting_authority(
                &change_id,
                &mut store,
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap(),
    );
    assert!(result.rolled_back);
    assert!(commit.mutation_id.is_some());
    assert!(commit.authority_token.is_some());
    assert!(!store.overlay_path(&note.id).exists());
}

#[test]
fn rollback_rejects_source_drift_without_authority_or_target_effect() {
    let (_vault, _data, _events, coordinator, mut store, mut service, note) =
        fixture("Rollback Source Drift");
    let restore = b"{\"tags\":[\"before\"]}";
    let (change_id, _, after_raw) = install_sidecar_change(&service, &store, &note, Some(restore));
    let authority = coordinator.current_authority_token().unwrap();
    let mut source = store
        .migration_target_bytes(TargetKind::Markdown, &note.relative_path)
        .unwrap()
        .unwrap();
    source.extend_from_slice(b"\nexternal edit\n");
    std::fs::write(store.vault_path().join(&note.relative_path), source).unwrap();

    assert!(service
        .rollback_change_expecting_authority(&change_id, &mut store, authority.clone())
        .is_err());
    assert_eq!(coordinator.current_authority_token().unwrap(), authority);
    assert_eq!(
        std::fs::read(store.overlay_path(&note.id)).unwrap(),
        after_raw
    );
    assert_eq!(service.state.rollback_count, 0);
    assert!(service
        .retained_optimizer_root()
        .unwrap()
        .regular_file_names_bounded(
            rollback::PENDING_ROLLBACKS_DIRECTORY,
            rollback::MAX_PENDING_ROLLBACKS,
        )
        .unwrap()
        .is_empty());
}

#[test]
fn post_authority_rollback_failure_returns_exact_partial_and_recovers_once() {
    let (_vault, _data, events, coordinator, mut store, mut service, note) =
        fixture("Rollback Point Of No Return");
    let restore = b"{\"tags\":[\"before\"]}";
    let (change_id, material, after_raw) =
        install_sidecar_change(&service, &store, &note, Some(restore));
    let expected = coordinator.current_authority_token().unwrap();
    coordinator
        .fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterAuthorityAdvance);
    coordinator.fail_next_replays_before_targets(1);

    let partial = service
        .rollback_change_expecting_authority(&change_id, &mut store, expected.clone())
        .unwrap();
    let OptimizerRollbackMutationOutcome::Partial {
        result,
        commit,
        warning,
        recovery_pending,
    } = partial
    else {
        panic!("expected exact partial optimizer rollback outcome");
    };
    assert!(!result.rolled_back);
    assert!(recovery_pending);
    assert_eq!(result.warning.as_ref(), Some(&warning));
    assert_eq!(warning.code, "optimizer_rollback_recovery_pending");
    assert_eq!(
        commit
            .authority_token
            .as_ref()
            .unwrap()
            .authority_generation,
        expected.authority_generation + 1
    );
    assert!(commit.mutation_id.is_some());
    assert_eq!(
        std::fs::read(store.overlay_path(&note.id)).unwrap(),
        after_raw
    );
    assert_eq!(service.state.rollback_count, 0);
    assert_eq!(
        service
            .retained_optimizer_root()
            .unwrap()
            .regular_file_names_bounded(
                rollback::PENDING_ROLLBACKS_DIRECTORY,
                rollback::MAX_PENDING_ROLLBACKS,
            )
            .unwrap()
            .len(),
        1
    );

    let resumed = committed_result(
        service
            .rollback_change_expecting_authority(
                &change_id,
                &mut store,
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap(),
    );
    assert!(resumed.0.rolled_back);
    assert_eq!(
        std::fs::read(store.overlay_path(&note.id)).unwrap(),
        restore
    );
    assert_eq!(service.state.rollback_count, 1);
    let matching = events
        .ordered_events()
        .unwrap()
        .into_iter()
        .filter(|event| {
            event.context.source_channel.as_str() == "vault_optimizer"
                && matches!(
                    &event.payload,
                    TwinEventPayload::NoteChanged(changed)
                        if changed.note_id.as_str() == note.id
                            && changed.content_digest.as_ref()
                                == Some(&material.rollback_payload_digest)
                )
        })
        .count();
    assert_eq!(matching, 1);

    let authority = coordinator.current_authority_token().unwrap();
    assert!(matches!(
        service
            .rollback_change_expecting_authority(&change_id, &mut store, authority.clone())
            .unwrap(),
        OptimizerRollbackMutationOutcome::NoWrite(_)
    ));
    assert_eq!(coordinator.current_authority_token().unwrap(), authority);
    assert_eq!(service.state.rollback_count, 1);
}

#[test]
fn rollback_restores_crlf_markdown_bytes_exactly() {
    let (vault, _data, _events, coordinator, mut store, mut service, note) =
        fixture("CRLF Markdown Rollback");
    let after_raw = std::fs::read(vault.path().join(&note.relative_path)).unwrap();
    let before_raw = String::from_utf8(after_raw.clone())
        .unwrap()
        .replace('\n', "\r\n")
        .into_bytes();
    assert_ne!(before_raw, after_raw);
    std::fs::write(vault.path().join(&note.relative_path), &before_raw).unwrap();
    let before_note = store
        .optimizer_markdown_snapshot(&note.relative_path)
        .unwrap()
        .unwrap()
        .0;
    std::fs::write(vault.path().join(&note.relative_path), &after_raw).unwrap();
    let after_note = store
        .optimizer_markdown_snapshot(&note.relative_path)
        .unwrap()
        .unwrap()
        .0;
    let before_digest = crate::services::twin_events::digest_bytes(&before_raw);
    let after_digest = crate::services::twin_events::digest_bytes(&after_raw);
    let material = rollback::ExactOptimizerRollbackMaterialV1 {
        schema_version: rollback::EXACT_OPTIMIZER_ROLLBACK_SCHEMA_VERSION,
        target_kind: TargetKind::Markdown,
        target_key: note.relative_path.clone(),
        restore_before: BeforeImage::Sha256(before_digest.clone()),
        restore_utf8: Some(String::from_utf8(before_raw.clone()).unwrap()),
        apply_after: BeforeImage::Sha256(after_digest.clone()),
        source_relative_path: note.relative_path.clone(),
        source_digest: before_digest.clone(),
        apply_payload_digest: after_digest.clone(),
        apply_evidence_digest: before_digest.clone(),
        rollback_payload_digest: before_digest.clone(),
        rollback_evidence_digest: after_digest,
        apply_governance: store.optimizer_note_governance(&after_note),
        rollback_governance: store.optimizer_note_governance(&before_note),
    };
    let change_id = Uuid::new_v4().to_string();
    service
        .write_change(&OptimizerChange {
            change_id: change_id.clone(),
            note_id: note.id.clone(),
            mode: "full_rewrite".to_string(),
            overlay_before: None,
            overlay_after: None,
            note_before: Some(before_note),
            note_after: Some(after_note),
            markdown_before_digest: Some(before_digest),
            markdown_relative_path: Some(note.relative_path.clone()),
            created_at: Some(Utc::now()),
            exact_rollback: Some(material),
        })
        .unwrap();

    let (result, commit) = committed_result(
        service
            .rollback_change_expecting_authority(
                &change_id,
                &mut store,
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap(),
    );
    assert!(result.rolled_back);
    assert!(commit.mutation_id.is_some());
    assert!(commit.authority_token.is_some());
    assert_eq!(
        std::fs::read(vault.path().join(&note.relative_path)).unwrap(),
        before_raw
    );
}

#[test]
fn sidecar_apply_and_rollback_emit_one_governed_update_each() {
    let (_vault, _data, events, coordinator, mut store, mut service, note) =
        fixture("Governed Sidecar Rollback");
    service
        .bootstrap_checked(std::slice::from_ref(&note))
        .unwrap();
    let pending = match service
        .prepare_next_expecting_authority(
            &store,
            &UserSettings::default(),
            coordinator.current_authority_token().unwrap(),
        )
        .unwrap()
    {
        OptimizerTick::Pending(pending) => *pending,
        other => panic!("expected optimizer proposal, got {other:?}"),
    };
    let change_id = match service.apply_pending(&mut store, pending).unwrap() {
        OptimizerMutationResult::Committed { result, .. } => result.change_id().to_string(),
        other => panic!("expected committed optimizer apply, got {other:?}"),
    };

    let state_lock = service.acquire_state_lock().unwrap();
    service.reload_from_disk_checked().unwrap();
    let guard = coordinator.begin_root_transition().unwrap();
    service
        .recover_pending_publications_locked(&store, &guard)
        .unwrap();
    drop(guard);
    state_lock.unlock().unwrap();
    let change = service.read_change(&change_id).unwrap();
    let material = change.exact_rollback.unwrap();

    let (result, commit) = committed_result(
        service
            .rollback_change_expecting_authority(
                &change_id,
                &mut store,
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap(),
    );
    assert!(result.rolled_back);
    assert!(commit.mutation_id.is_some());
    assert!(commit.authority_token.is_some());

    let updates = events
        .ordered_events()
        .unwrap()
        .into_iter()
        .filter(|event| {
            event.context.source_channel.as_str() == "vault_optimizer"
                && matches!(
                    &event.payload,
                    TwinEventPayload::NoteChanged(changed)
                        if changed.note_id.as_str() == note.id
                            && changed.change == NoteChangeKind::Updated
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(updates.len(), 2);
    let TwinEventPayload::NoteChanged(apply_update) = &updates[0].payload else {
        unreachable!("filtered optimizer update must be NoteChanged");
    };
    let TwinEventPayload::NoteChanged(rollback_update) = &updates[1].payload else {
        unreachable!("filtered optimizer update must be NoteChanged");
    };
    assert_eq!(
        apply_update.content_digest.as_ref(),
        Some(&material.apply_payload_digest)
    );
    assert_eq!(
        rollback_update.content_digest.as_ref(),
        Some(&material.rollback_payload_digest)
    );
    assert_eq!(updates[0].governance, material.apply_governance);
    assert_eq!(updates[1].governance, material.rollback_governance);
}
