use super::*;
use crate::models::twin_event::{
    CausalStream, Governance, NoteChangeKind, NoteChanged, TwinEventPayload, Visibility,
};
use crate::services::twin_events::{TargetKind, TargetMutation};
use chrono::{TimeZone, Utc};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tempfile::tempdir;

fn tree_snapshot(root: &Path) -> std::collections::BTreeMap<PathBuf, Option<Vec<u8>>> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .map(Result::unwrap)
        .map(|entry| {
            let relative = entry.path().strip_prefix(root).unwrap().to_path_buf();
            let contents = if entry.file_type().is_dir() {
                None
            } else if entry.file_type().is_file() {
                Some(std::fs::read(entry.path()).unwrap())
            } else {
                panic!("unexpected test tree entry: {}", entry.path().display());
            };
            (relative, contents)
        })
        .collect()
}

fn assert_transition_wal_error(error: MutationError) {
    assert!(
        error.to_string().contains("root-transition"),
        "unexpected WAL preflight error: {error}"
    );
}

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
fn active_root_lease_read_and_write_reject_nil_and_noncanonical_epochs() {
    let temp = tempdir().unwrap();
    let root = crate::services::twin_events::AnchoredRoot::open(temp.path()).unwrap();
    let scope = crate::models::twin_event::ContentDigest::parse("a".repeat(64)).unwrap();

    for schema_version in [
        ACTIVE_ROOT_LEASE_SCHEMA_VERSION,
        STABLE_ROOT_LEASE_SCHEMA_VERSION,
    ] {
        for epoch_uuid in [
            uuid::Uuid::nil().to_string(),
            "123E4567-E89B-42D3-A456-426614174000".to_string(),
            "123e4567e89b42d3a456426614174000".to_string(),
        ] {
            let lease = ActiveMarkdownRootLeaseV1 {
                schema_version,
                root_scope: scope.clone(),
                epoch_uuid,
            };
            let encoded = serde_json::to_vec(&lease).unwrap();
            assert!(parse_active_root_lease(&encoded).is_err());
            assert!(write_active_root_lease(&root, &lease).is_err());
        }
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
fn coordinator_exposes_the_exact_persisted_writer_uuid_for_sync_binding() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir_all(data.join("twin/events")).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();

    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let exposed = coordinator.writer_device_id();
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(data.join("twin/events/writer-v1.json")).unwrap())
            .unwrap();

    assert_eq!(persisted["device_id"].as_str(), Some(exposed.as_str()));
    uuid::Uuid::parse_str(exposed.as_str()).unwrap();
}

#[test]
fn coordinator_binds_sync_signing_identity_after_recovery_under_its_process_lock() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();

    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let writer = coordinator.writer_device_id();
    let identity = coordinator
        .load_or_create_device_signing_identity(Arc::new(
            crate::services::sync::secrets::MemorySecretStore::default(),
        ))
        .unwrap();

    assert_eq!(identity.device_id().to_string(), writer.as_str());
    assert!(data.join("twin/events/device-signing-v1.json").is_file());
}

#[test]
fn stable_bootstrap_refuses_missing_writer_for_bound_history_without_writes() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir_all(data.join("twin/events")).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    let coordinator =
        MutationCoordinator::new_stable(&data, &vault, store, Arc::new(NoopMutationLifecycle))
            .unwrap();
    let original_writer = coordinator.writer_device_id();
    coordinator
        .load_or_create_device_signing_identity(Arc::new(
            crate::services::sync::secrets::MemorySecretStore::default(),
        ))
        .unwrap();
    let _ = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            Vec::new(),
            vec![draft("bound-history")],
        )
        .unwrap();
    drop(coordinator);

    std::fs::remove_file(data.join("twin/events/writer-v1.json")).unwrap();
    let data_before = tree_snapshot(&data);
    let vault_before = tree_snapshot(&vault);

    let error = match MutationCoordinator::new_stable(
        &data,
        &vault,
        Arc::new(TwinEventStore::new(&data)),
        Arc::new(NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("established bound history must not mint a replacement writer"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        MutationError::RecoveryConflict(message)
            if message == "writer-identity-missing-for-established-data-root"
    ));
    assert!(!data.join("twin/events/writer-v1.json").exists());
    assert_eq!(tree_snapshot(&data), data_before);
    assert_eq!(tree_snapshot(&vault), vault_before);
    assert_ne!(original_writer.as_str(), "");
}

#[test]
fn stable_bootstrap_refuses_missing_writer_for_unleased_legacy_binding_and_events_before_descriptor(
) {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    coordinator
        .load_or_create_device_signing_identity(Arc::new(
            crate::services::sync::secrets::MemorySecretStore::default(),
        ))
        .unwrap();
    let _ = coordinator
        .commit_local(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            Vec::new(),
            vec![draft("legacy-bound-history")],
        )
        .unwrap();
    drop(coordinator);

    std::fs::remove_file(data.join("twin/events/active-markdown-root-v1.json")).unwrap();
    std::fs::remove_file(data.join("twin/events/writer-v1.json")).unwrap();
    let data_before = tree_snapshot(&data);
    let vault_before = tree_snapshot(&vault);

    let error = match MutationCoordinator::new_stable(
        &data,
        &vault,
        Arc::new(TwinEventStore::new(&data)),
        Arc::new(NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("unleased legacy binding and event history must not mint a writer"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        MutationError::RecoveryConflict(message)
            if message == "writer-identity-missing-for-established-data-root"
    ));
    assert!(!vault.join("_grafyn/vault.json").exists());
    assert!(!data
        .join("twin/events/active-markdown-root-v1.json")
        .exists());
    assert!(!data.join("twin/events/writer-v1.json").exists());
    assert_eq!(tree_snapshot(&data), data_before);
    assert_eq!(tree_snapshot(&vault), vault_before);
}

#[test]
fn stable_first_install_mints_one_writer_and_reopens_it() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir_all(data.join("twin/events")).unwrap();

    let first = MutationCoordinator::new_stable(
        &data,
        &vault,
        Arc::new(TwinEventStore::new(&data)),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let writer = first.writer_device_id();
    drop(first);

    let reopened = MutationCoordinator::new_stable(
        &data,
        &vault,
        Arc::new(TwinEventStore::new(&data)),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();

    assert_eq!(reopened.writer_device_id(), writer);
}

#[test]
fn stable_first_install_tolerates_a_retained_orphaned_writer_install_temp() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    let staging = data.join(WRITER_STAGING_KEY);
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let orphan_name = format!(".{}.tmp", uuid::Uuid::new_v4());
    let orphan_path = staging.join(&orphan_name);
    std::fs::write(&orphan_path, b"crash-left writer installer bytes").unwrap();

    let first = MutationCoordinator::new_stable(
        &data,
        &vault,
        Arc::new(TwinEventStore::new(&data)),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let writer = first.writer_device_id();

    assert!(orphan_path.is_file());
    assert!(data.join(WRITER_KEY).is_file());
    drop(first);

    let reopened = MutationCoordinator::new_stable(
        &data,
        &vault,
        Arc::new(TwinEventStore::new(&data)),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    assert_eq!(reopened.writer_device_id(), writer);
}

#[test]
fn missing_writer_rejects_foreign_staging_file_without_mutating_it() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let staging = data.join(WRITER_STAGING_KEY);
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join(".crashed.tmp"), b"foreign staging bytes").unwrap();
    let root = crate::services::twin_events::AnchoredRoot::open(&data).unwrap();
    let process_lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    let before = tree_snapshot(&staging);

    let error =
        reject_missing_writer_for_established_data_root_locked(&root, &process_lock).unwrap_err();

    assert!(matches!(
        error,
        MutationError::RecoveryConflict(message)
            if message == "writer-identity-missing-for-established-data-root"
    ));
    assert_eq!(tree_snapshot(&staging), before);
    assert!(!data.join(WRITER_KEY).exists());
}

#[test]
fn missing_writer_never_reconciles_a_uuid_temp_from_legacy_event_staging() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let event_staging = data.join("twin/events/staging/v1");
    std::fs::create_dir_all(&event_staging).unwrap();
    let event_temp = event_staging.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&event_temp, b"possibly durable event bytes").unwrap();
    let root = crate::services::twin_events::AnchoredRoot::open(&data).unwrap();
    let process_lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    let before = tree_snapshot(&event_staging);

    let error =
        reject_missing_writer_for_established_data_root_locked(&root, &process_lock).unwrap_err();

    assert!(matches!(
        error,
        MutationError::RecoveryConflict(message)
            if message == "writer-identity-missing-for-established-data-root"
    ));
    assert_eq!(tree_snapshot(&event_staging), before);
    assert!(event_temp.is_file());
    assert!(!data.join(WRITER_KEY).exists());
}

#[test]
fn missing_writer_keeps_orphan_temp_when_other_established_evidence_exists() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let staging = data.join(WRITER_STAGING_KEY);
    std::fs::create_dir_all(&staging).unwrap();
    let orphan_path = staging.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&orphan_path, b"crash-left writer installer bytes").unwrap();
    std::fs::write(
        data.join("twin/events/content-authority-v1.json"),
        b"established authority",
    )
    .unwrap();
    let root = crate::services::twin_events::AnchoredRoot::open(&data).unwrap();
    let process_lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    let before = tree_snapshot(&staging);

    let error =
        reject_missing_writer_for_established_data_root_locked(&root, &process_lock).unwrap_err();

    assert!(matches!(
        error,
        MutationError::RecoveryConflict(message)
            if message == "writer-identity-missing-for-established-data-root"
    ));
    assert_eq!(tree_snapshot(&staging), before);
    assert!(orphan_path.is_file());
    assert_eq!(
        std::fs::read(data.join("twin/events/content-authority-v1.json")).unwrap(),
        b"established authority"
    );
    assert!(!data.join(WRITER_KEY).exists());
}

#[test]
fn writer_install_temp_name_must_match_the_exact_uuid_v4_shape() {
    let random = uuid::Uuid::new_v4().to_string();
    assert!(is_canonical_writer_install_temp(&format!(".{random}.tmp")));
    assert!(!is_canonical_writer_install_temp(&format!(
        ".{}.tmp",
        random.to_uppercase()
    )));
    assert!(!is_canonical_writer_install_temp(
        ".00000000-0000-0000-0000-000000000000.tmp"
    ));
    assert!(!is_canonical_writer_install_temp(
        ".67e55044-10b1-11ed-861d-0242ac120002.tmp"
    ));
    assert!(!is_canonical_writer_install_temp(
        ".67e55044-10b1-426f-9247-bb680e5fe0c8.partial"
    ));
    assert!(!is_canonical_writer_install_temp(".crashed.tmp"));
}

#[test]
fn existing_writer_tolerates_the_retained_post_link_install_temp() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(data.join("twin/events")).unwrap();
    let expected = PersistedMutationIdentityProvider::load_or_create(&data).unwrap();
    let staging = data.join(WRITER_STAGING_KEY);
    let orphan = staging.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&orphan, std::fs::read(data.join(WRITER_KEY)).unwrap()).unwrap();
    let root = crate::services::twin_events::AnchoredRoot::open(&data).unwrap();
    let process_lock = acquire_shared_coordinator_process_lock(&data).unwrap();

    let recovered = reject_missing_writer_for_established_data_root_locked(&root, &process_lock)
        .unwrap()
        .unwrap();

    assert_eq!(recovered.device_id(), expected.device_id());
    assert!(orphan.is_file());
}

#[test]
fn existing_writer_rejects_foreign_writer_staging_without_mutation() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(data.join("twin/events")).unwrap();
    PersistedMutationIdentityProvider::load_or_create(&data).unwrap();
    let staging = data.join(WRITER_STAGING_KEY);
    std::fs::write(staging.join("foreign.bin"), b"foreign staging bytes").unwrap();
    let root = crate::services::twin_events::AnchoredRoot::open(&data).unwrap();
    let process_lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    let before = tree_snapshot(&staging);

    let error =
        reject_missing_writer_for_established_data_root_locked(&root, &process_lock).unwrap_err();

    assert!(matches!(
        error,
        MutationError::RecoveryConflict(message)
            if message == "writer-install-staging-contains-unrecognized-entry"
    ));
    assert_eq!(tree_snapshot(&staging), before);
}

#[test]
fn writer_evidence_recursion_spends_one_total_entry_budget() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(data.join("first/empty-a")).unwrap();
    std::fs::create_dir_all(data.join("second/empty-b")).unwrap();
    let root = crate::services::twin_events::AnchoredRoot::open(&data).unwrap();
    let mut remaining_entries = 1;

    assert!(
        !anchored_directory_contains_regular_file(&root, "first", 0, &mut remaining_entries,)
            .unwrap()
    );
    assert_eq!(remaining_entries, 0);
    let error =
        anchored_directory_contains_regular_file(&root, "second", 0, &mut remaining_entries)
            .unwrap_err();

    assert!(error.to_string().contains("exceeds its 0-entry limit"));
}

#[test]
fn stable_bootstrap_mints_writer_for_coherent_prewriter_legacy_state_and_binds_marker() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let legacy_scope = markdown_root_scope_for(&vault).unwrap();
    let legacy_twin = crate::models::settings::twin_data_path_for_scope(&data, &legacy_scope);
    std::fs::create_dir_all(&legacy_twin).unwrap();
    std::fs::write(legacy_twin.join("legacy.json"), b"legacy-twin").unwrap();
    std::fs::create_dir(data.join("search_index")).unwrap();
    std::fs::write(data.join("search_index/legacy.json"), b"legacy-derived").unwrap();
    std::fs::create_dir(data.join("canvas")).unwrap();
    std::fs::write(data.join("canvas/session.json"), b"legacy-canvas").unwrap();
    std::fs::create_dir_all(data.join("twin/events")).unwrap();

    let coordinator = MutationCoordinator::new_stable(
        &data,
        &vault,
        Arc::new(TwinEventStore::new(&data)),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let lease = coordinator.current_root_epoch().unwrap();
    let writer = coordinator.writer_device_id();
    let marker: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            data.join("twin/stable-vault-migrations/v1")
                .join(lease.root_scope.as_str())
                .join("marker.json"),
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(marker["writer_device_id"].as_str(), Some(writer.as_str()));
}

#[test]
fn stable_coordinator_initializes_twin_events_only_after_its_wal_preflight() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let config = temp.path().join("config");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let identity = crate::services::sync::identity::load_or_create_vault_identity(&vault).unwrap();
    let transition_store = crate::services::root_transition::RootTransitionStore::new(
        &data,
        config.join("settings.json"),
        Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    transition_store
        .write_settings(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                crate::models::settings::UserSettings {
                    vault_path: Some(vault.to_string_lossy().into_owned()),
                    ..crate::models::settings::UserSettings::default()
                },
            ),
        )
        .unwrap();
    transition_store
        .write_lease(&ActiveMarkdownRootLeaseV1::new_stable(identity.root_scope))
        .unwrap();
    transition_store
        .write_key_authority(
            crate::services::root_transition::OpenRouterKeySource::Unset,
            None,
        )
        .unwrap();
    PersistedMutationIdentityProvider::load_or_create(&data).unwrap();
    let events = Arc::new(TwinEventStore::new(&data));
    assert!(matches!(
        events.ordered_events(),
        Err(crate::services::twin_events::StoreError::NotInitialized)
    ));

    let coordinator = MutationCoordinator::new_stable(
        &data,
        &vault,
        events.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();

    assert!(events.ordered_events().unwrap().is_empty());
    assert!(coordinator.current_root_epoch().unwrap().is_stable());
}

#[test]
fn coordinator_rejects_a_foreign_event_store_before_writing_either_data_root() {
    let temp = tempdir().unwrap();
    let data_a = temp.path().join("data-a");
    let data_b = temp.path().join("data-b");
    let config = temp.path().join("config");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data_a).unwrap();
    std::fs::create_dir(&data_b).unwrap();
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let identity = crate::services::sync::identity::load_or_create_vault_identity(&vault).unwrap();
    let transition_store = crate::services::root_transition::RootTransitionStore::new(
        &data_a,
        config.join("settings.json"),
        Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    transition_store
        .write_settings(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                crate::models::settings::UserSettings {
                    vault_path: Some(vault.to_string_lossy().into_owned()),
                    ..crate::models::settings::UserSettings::default()
                },
            ),
        )
        .unwrap();
    transition_store
        .write_lease(&ActiveMarkdownRootLeaseV1::new_stable(identity.root_scope))
        .unwrap();
    transition_store
        .write_key_authority(
            crate::services::root_transition::OpenRouterKeySource::Unset,
            None,
        )
        .unwrap();
    let foreign_events = Arc::new(TwinEventStore::new(&data_b));
    let data_a_before = tree_snapshot(&data_a);
    let data_b_before = tree_snapshot(&data_b);
    let vault_before = tree_snapshot(&vault);

    let error = match MutationCoordinator::new_stable(
        &data_a,
        &vault,
        foreign_events,
        Arc::new(NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("a coordinator must not accept an event store for another data root"),
        Err(error) => error,
    };

    assert_eq!(tree_snapshot(&data_a), data_a_before);
    assert_eq!(tree_snapshot(&data_b), data_b_before);
    assert_eq!(tree_snapshot(&vault), vault_before);
    assert!(error.to_string().contains("Twin event store data root"));
}

#[test]
fn coordinator_rejects_a_same_root_scoped_event_store_before_any_write() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let config = temp.path().join("config");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let identity = crate::services::sync::identity::load_or_create_vault_identity(&vault).unwrap();
    let transition_store = crate::services::root_transition::RootTransitionStore::new(
        &data,
        config.join("settings.json"),
        Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    transition_store
        .write_settings(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                crate::models::settings::UserSettings {
                    vault_path: Some(vault.to_string_lossy().into_owned()),
                    ..crate::models::settings::UserSettings::default()
                },
            ),
        )
        .unwrap();
    transition_store
        .write_lease(&ActiveMarkdownRootLeaseV1::new_stable(identity.root_scope))
        .unwrap();
    transition_store
        .write_key_authority(
            crate::services::root_transition::OpenRouterKeySource::Unset,
            None,
        )
        .unwrap();
    let scoped_events = Arc::new(TwinEventStore::new_scoped(
        &data,
        crate::models::twin_event::ContentDigest::parse("f".repeat(64)).unwrap(),
    ));
    let data_before = tree_snapshot(&data);
    let vault_before = tree_snapshot(&vault);

    let error = match MutationCoordinator::new_stable(
        &data,
        &vault,
        scoped_events,
        Arc::new(NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("a writable coordinator must begin with the legacy event namespace"),
        Err(error) => error,
    };

    assert_eq!(tree_snapshot(&data), data_before);
    assert_eq!(tree_snapshot(&vault), vault_before);
    assert!(error.to_string().contains("legacy namespace"));
}

#[test]
fn stable_coordinator_rechecks_transition_wal_under_its_retained_process_lock() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let config = temp.path().join("config");
    let vault_a = temp.path().join("vault-a");
    let vault_b = temp.path().join("vault-b");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&vault_a).unwrap();
    std::fs::create_dir(&vault_b).unwrap();
    let identity_a =
        crate::services::sync::identity::load_or_create_vault_identity(&vault_a).unwrap();
    let identity_b =
        crate::services::sync::identity::load_or_create_vault_identity(&vault_b).unwrap();
    let transition_store = crate::services::root_transition::RootTransitionStore::new(
        &data,
        config.join("settings.json"),
        Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    let before = crate::services::root_transition::RootAuthorityV1::new(
        &vault_a,
        crate::models::settings::UserSettings {
            vault_path: Some(vault_a.to_string_lossy().into_owned()),
            ..crate::models::settings::UserSettings::default()
        },
        ActiveMarkdownRootLeaseV1::new_stable(identity_a.root_scope),
        None,
    )
    .unwrap();
    let after = crate::services::root_transition::RootAuthorityV1::new(
        &vault_b,
        crate::models::settings::UserSettings {
            vault_path: Some(vault_b.to_string_lossy().into_owned()),
            ..crate::models::settings::UserSettings::default()
        },
        ActiveMarkdownRootLeaseV1::new_stable(identity_b.root_scope),
        None,
    )
    .unwrap();
    transition_store
        .write_settings(&before.nonsecret_settings)
        .unwrap();
    transition_store.write_lease(&before.lease).unwrap();
    transition_store
        .write_key_authority(
            crate::services::root_transition::OpenRouterKeySource::Unset,
            None,
        )
        .unwrap();
    PersistedMutationIdentityProvider::load_or_create(&data).unwrap();
    let events = Arc::new(TwinEventStore::new(&data));

    transition_store
        .load_startup_settings(
            crate::services::root_transition::StartupSecretPolicy::AllowLegacy,
            || Ok(None),
            || Ok(()),
        )
        .unwrap();
    let transition = crate::services::root_transition::RootTransitionV1::prepared(
        before,
        after,
        transition_store.authority_binding(),
    )
    .unwrap();
    transition_store.prepare_transition(&transition).unwrap();
    assert!(!data.join("canvas").exists());
    assert!(data.join("twin/events/writer-v1.json").exists());
    assert!(!data.join("twin/mutations").exists());
    let tree_before = tree_snapshot(&data);
    let settings_before = std::fs::read(config.join("settings.json")).unwrap();

    let error = match MutationCoordinator::new_stable(
        &data,
        &vault_a,
        events,
        Arc::new(NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("stable coordinator must reject a prepared transition WAL"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("root-transition"));
    assert_eq!(tree_snapshot(&data), tree_before);
    assert!(!data.join("canvas").exists());
    assert!(data.join("twin/events/writer-v1.json").exists());
    assert!(!data.join("twin/mutations").exists());
    assert_eq!(
        std::fs::read(config.join("settings.json")).unwrap(),
        settings_before
    );
}

#[test]
fn live_stable_coordinator_rejects_every_fresh_entry_after_peer_prepares_transition() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let config = temp.path().join("config");
    let vault_a = temp.path().join("vault-a");
    let vault_b = temp.path().join("vault-b");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&vault_a).unwrap();
    std::fs::create_dir(&vault_b).unwrap();
    let identity_a =
        crate::services::sync::identity::load_or_create_vault_identity(&vault_a).unwrap();
    let identity_b =
        crate::services::sync::identity::load_or_create_vault_identity(&vault_b).unwrap();
    let settings_a = crate::models::settings::UserSettings {
        vault_path: Some(vault_a.to_string_lossy().into_owned()),
        ..crate::models::settings::UserSettings::default()
    };
    let settings_b = crate::models::settings::UserSettings {
        vault_path: Some(vault_b.to_string_lossy().into_owned()),
        ..crate::models::settings::UserSettings::default()
    };
    let lease_a = ActiveMarkdownRootLeaseV1::new_stable(identity_a.root_scope);
    let transition_store = crate::services::root_transition::RootTransitionStore::new(
        &data,
        config.join("settings.json"),
        Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    transition_store
        .write_settings(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                settings_a.clone(),
            ),
        )
        .unwrap();
    transition_store.write_lease(&lease_a).unwrap();
    transition_store
        .write_key_authority(
            crate::services::root_transition::OpenRouterKeySource::Unset,
            None,
        )
        .unwrap();
    PersistedMutationIdentityProvider::load_or_create(&data).unwrap();
    let events = Arc::new(TwinEventStore::new(&data));
    events.initialize().unwrap();
    let coordinator = MutationCoordinator::new_stable(
        &data,
        &vault_a,
        events.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let actual_lease = coordinator.current_root_epoch().unwrap();
    assert_eq!(actual_lease, lease_a);
    let authority_before = coordinator.current_authority_token().unwrap();
    let before = crate::services::root_transition::RootAuthorityV1::new(
        &vault_a,
        settings_a,
        actual_lease,
        None,
    )
    .unwrap();
    let after = crate::services::root_transition::RootAuthorityV1::new(
        &vault_b,
        settings_b,
        ActiveMarkdownRootLeaseV1::new_stable(identity_b.root_scope),
        None,
    )
    .unwrap();
    let transition = crate::services::root_transition::RootTransitionV1::prepared(
        before,
        after,
        transition_store.authority_binding(),
    )
    .unwrap();
    transition_store.prepare_transition(&transition).unwrap();

    let data_before = tree_snapshot(&data);
    let vault_before = tree_snapshot(&vault_a);
    let events_before = events.ordered_events().unwrap();
    assert!(!vault_a.join("blocked.md").exists());

    let error = coordinator
        .commit_local(
            CausalStream::SyncEligible,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "blocked.md",
                "must not be written",
            )],
            vec![draft("blocked")],
        )
        .expect_err("a live stable coordinator must stop before mutation planning");
    assert_transition_wal_error(error);

    assert_transition_wal_error(
        coordinator
            .recover_pending()
            .expect_err("recovery must stop at the WAL preflight"),
    );
    assert_transition_wal_error(
        coordinator
            .pending_count()
            .expect_err("pending reads must not clean state while a root transition is prepared"),
    );
    assert_transition_wal_error(
        coordinator
            .quarantine_count()
            .expect_err("quarantine reads must stop at the WAL preflight"),
    );
    assert_transition_wal_error(
        coordinator
            .current_root_epoch()
            .expect_err("root reads must stop at the WAL preflight"),
    );
    assert_transition_wal_error(
        coordinator
            .current_authority_token()
            .expect_err("authority reads must stop at the WAL preflight"),
    );
    let begin_error = coordinator
        .begin_root_transition()
        .err()
        .expect("a second root-transition guard must not enter through a peer WAL");
    assert_transition_wal_error(begin_error);
    assert_transition_wal_error(
        coordinator
            .invalidate_namespace_before_recovery(&authority_before)
            .expect_err("repair entry must stop at the WAL preflight"),
    );
    let derived_ran = std::cell::Cell::new(false);
    assert_transition_wal_error(
        coordinator
            .with_locked_derived_state(&authority_before, true, || {
                derived_ran.set(true);
                Ok(())
            })
            .expect_err("derived-state access must stop at the WAL preflight"),
    );
    assert!(!derived_ran.get());
    assert_transition_wal_error(
        coordinator
            .finalizer
            .finalize(CausalStream::LocalOnly, &[draft("finalizer-blocked")])
            .expect_err("the public finalizer lock entry must share the WAL preflight"),
    );

    assert_eq!(tree_snapshot(&data), data_before);
    assert_eq!(tree_snapshot(&vault_a), vault_before);
    assert_eq!(events.ordered_events().unwrap(), events_before);
    assert!(!vault_a.join("blocked.md").exists());
    assert!(data.join("twin/events/root-transition-v1.json").is_file());
}

#[test]
fn stable_bootstrap_migrates_legacy_vault_data_once_without_merging() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let legacy_store = Arc::new(TwinEventStore::new(&data));
    legacy_store.initialize().unwrap();
    let legacy = MutationCoordinator::new(
        &data,
        &vault,
        legacy_store.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let guard = legacy.begin_root_transition().unwrap();
    let legacy_lease = guard.current_lease().unwrap();
    let legacy_twin = guard.prepare_twin_data_path(&vault, &legacy_lease).unwrap();
    std::fs::create_dir_all(&legacy_twin).unwrap();
    std::fs::write(legacy_twin.join("sentinel.json"), b"legacy-twin").unwrap();
    let legacy_derived = guard.initialize_namespace(&legacy_lease).unwrap();
    std::fs::write(legacy_derived.join("sentinel.json"), b"legacy-derived").unwrap();
    std::fs::write(data.join("canvas/session.json"), b"legacy-canvas").unwrap();
    drop(guard);
    let _commit = legacy
        .commit_local(
            CausalStream::SyncEligible,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            Vec::new(),
            vec![draft("legacy-event")],
        )
        .unwrap();
    drop(legacy);
    drop(legacy_store);

    let stable_store = Arc::new(TwinEventStore::new(&data));
    stable_store.initialize().unwrap();
    let stable = MutationCoordinator::new_stable(
        &data,
        &vault,
        stable_store.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let stable_lease = stable.current_root_epoch().unwrap();
    assert!(stable_lease.is_stable());
    assert_ne!(stable_lease.root_scope, legacy_lease.root_scope);
    assert_eq!(
        std::fs::read(
            crate::models::settings::twin_data_path_for_scope(&data, &stable_lease.root_scope)
                .join("sentinel.json")
        )
        .unwrap(),
        b"legacy-twin"
    );
    assert_eq!(
        std::fs::read(
            crate::services::vault_namespace::scoped_data_path(&data, &stable_lease.root_scope)
                .join("sentinel.json")
        )
        .unwrap(),
        b"legacy-derived"
    );
    assert_eq!(
        std::fs::read(
            crate::services::canvas_store::scoped_canvas_path(&data, &stable_lease.root_scope)
                .join("session.json")
        )
        .unwrap(),
        b"legacy-canvas"
    );
    assert_eq!(stable_store.ordered_events().unwrap().len(), 1);

    drop(stable);
    drop(stable_store);
    let reopened_events = Arc::new(TwinEventStore::new(&data));
    reopened_events.initialize().unwrap();
    let reopened = MutationCoordinator::new_stable(
        &data,
        &vault,
        reopened_events.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    assert_eq!(reopened.current_root_epoch().unwrap(), stable_lease);
    assert_eq!(reopened_events.ordered_events().unwrap().len(), 1);
}

#[test]
fn stable_bootstrap_migrates_a_coherent_legacy_install_without_an_active_lease() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let legacy_scope = markdown_root_scope_for(&vault).unwrap();
    let legacy_twin = crate::models::settings::twin_data_path_for_scope(&data, &legacy_scope);
    std::fs::create_dir_all(&legacy_twin).unwrap();
    std::fs::write(legacy_twin.join("sentinel.json"), b"legacy-twin").unwrap();
    std::fs::create_dir(data.join("search_index")).unwrap();
    std::fs::write(data.join("search_index/sentinel.json"), b"legacy-derived").unwrap();
    std::fs::create_dir(data.join("canvas")).unwrap();
    std::fs::write(data.join("canvas/session.json"), b"legacy-canvas").unwrap();

    let legacy_events = Arc::new(TwinEventStore::new(&data));
    legacy_events.initialize().unwrap();
    let identity = Arc::new(PersistedMutationIdentityProvider::load_or_create(&data).unwrap());
    let finalized = StoreEventGroupFinalizer::new(legacy_events.clone(), identity)
        .finalize(CausalStream::SyncEligible, &[draft("legacy-event")])
        .unwrap();
    for event in finalized {
        legacy_events.append(event).unwrap();
    }
    assert_eq!(legacy_events.ordered_events().unwrap().len(), 1);
    assert!(!data
        .join("twin/events/active-markdown-root-v1.json")
        .exists());

    let stable_events = Arc::new(TwinEventStore::new(&data));
    let coordinator = MutationCoordinator::new_stable(
        &data,
        &vault,
        stable_events.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let stable_lease = coordinator.current_root_epoch().unwrap();

    assert!(stable_lease.is_stable());
    assert_ne!(stable_lease.root_scope, legacy_scope);
    assert_eq!(
        std::fs::read(
            crate::models::settings::twin_data_path_for_scope(&data, &stable_lease.root_scope)
                .join("sentinel.json")
        )
        .unwrap(),
        b"legacy-twin"
    );
    assert_eq!(
        std::fs::read(
            crate::services::vault_namespace::scoped_data_path(&data, &stable_lease.root_scope)
                .join("search_index/sentinel.json")
        )
        .unwrap(),
        b"legacy-derived"
    );
    assert_eq!(
        std::fs::read(
            crate::services::canvas_store::scoped_canvas_path(&data, &stable_lease.root_scope)
                .join("session.json")
        )
        .unwrap(),
        b"legacy-canvas"
    );
    assert_eq!(stable_events.ordered_events().unwrap().len(), 1);
    assert!(data
        .join("twin/stable-vault-migrations/v1")
        .join(stable_lease.root_scope.as_str())
        .join("marker.json")
        .is_file());
}

#[test]
fn stable_bootstrap_recovers_schema_one_pending_before_a_failed_unscoped_assignment() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(&data));
    store.initialize().unwrap();
    let legacy =
        MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle)).unwrap();
    let legacy_lease = legacy.current_root_epoch().unwrap();
    std::fs::create_dir_all(crate::models::settings::twin_data_path_for_scope(
        &data,
        &legacy_lease.root_scope,
    ))
    .unwrap();
    let process_lock = legacy.finalizer.acquire_coordinator_lock().unwrap();
    let intent = legacy
        .prepare_intent(
            &process_lock,
            MutationOrigin::Local,
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                "recovered.md",
                "recovered-before-assignment",
            )],
            Vec::new(),
            false,
        )
        .unwrap()
        .unwrap();
    crate::services::vault_namespace::advance_authority_locked(&data, &legacy_lease, &process_lock)
        .unwrap();
    legacy.journal.stage(&process_lock, &intent).unwrap();
    process_lock.unlock().unwrap();
    drop(legacy);

    let legacy_derived =
        crate::services::vault_namespace::scoped_data_path(&data, &legacy_lease.root_scope);
    std::fs::remove_dir_all(&legacy_derived).unwrap();
    std::fs::create_dir_all(&legacy_derived).unwrap();
    std::fs::write(legacy_derived.join("foreign.json"), b"foreign").unwrap();
    std::fs::create_dir(data.join("search_index")).unwrap();
    std::fs::write(data.join("search_index/sentinel.json"), b"legacy-derived").unwrap();

    let error = match MutationCoordinator::new_stable(
        &data,
        &vault,
        Arc::new(TwinEventStore::new(&data)),
        Arc::new(NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("the derived assignment collision must remain fail-closed"),
        Err(error) => error,
    };

    assert!(error
        .to_string()
        .contains("legacy-derived-state-collision-scoped-namespace"));
    assert_eq!(
        std::fs::read(vault.join("recovered.md")).unwrap(),
        b"recovered-before-assignment"
    );
    assert_eq!(
        std::fs::read_dir(data.join("twin/mutations/pending/v1"))
            .unwrap()
            .count(),
        0
    );
    assert!(data.join("search_index/sentinel.json").is_file());
}

#[test]
fn stable_bootstrap_rejects_ambiguous_no_lease_twin_sources_before_publishing_a_lease() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let legacy_scope = markdown_root_scope_for(&vault).unwrap();
    std::fs::create_dir_all(crate::models::settings::twin_data_path_for_scope(
        &data,
        &legacy_scope,
    ))
    .unwrap();
    std::fs::create_dir_all(crate::models::settings::legacy_twin_data_path_for_vault(
        &data, &vault,
    ))
    .unwrap();
    std::fs::create_dir(data.join("search_index")).unwrap();
    std::fs::create_dir(data.join("canvas")).unwrap();
    std::fs::create_dir_all(data.join("twin/events")).unwrap();
    let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    lock.unlock().unwrap();
    let data_before = tree_snapshot(&data);
    let vault_before = tree_snapshot(&vault);

    let error = match MutationCoordinator::new_stable(
        &data,
        &vault,
        Arc::new(TwinEventStore::new(&data)),
        Arc::new(NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("two no-lease legacy Twin sources must not be merged"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("legacy Twin"));
    assert!(!data
        .join("twin/events/active-markdown-root-v1.json")
        .exists());
    assert_eq!(tree_snapshot(&data), data_before);
    assert_eq!(tree_snapshot(&vault), vault_before);
}

#[test]
fn stable_bootstrap_rejects_unleased_authority_owners_before_publishing_a_lease() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let legacy_scope = markdown_root_scope_for(&vault).unwrap();
    std::fs::create_dir_all(crate::models::settings::twin_data_path_for_scope(
        &data,
        &legacy_scope,
    ))
    .unwrap();
    std::fs::create_dir(data.join("search_index")).unwrap();
    std::fs::create_dir(data.join("canvas")).unwrap();
    std::fs::create_dir_all(data.join("twin/events")).unwrap();
    std::fs::create_dir_all(data.join("twin/mutations/preauthority/v1")).unwrap();
    std::fs::write(
        data.join("twin/mutations/preauthority/v1/orphan.json"),
        b"unprovable owner",
    )
    .unwrap();
    let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    lock.unlock().unwrap();
    let data_before = tree_snapshot(&data);
    let vault_before = tree_snapshot(&vault);

    let error = match MutationCoordinator::new_stable(
        &data,
        &vault,
        Arc::new(TwinEventStore::new(&data)),
        Arc::new(NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("an authority owner without its lease must not be rebound"),
        Err(error) => error,
    };

    assert!(error
        .to_string()
        .contains("writer-identity-missing-for-established-data-root"));
    assert!(!data
        .join("twin/events/active-markdown-root-v1.json")
        .exists());
    assert!(!vault.join("_grafyn/vault.json").exists());
    assert_eq!(tree_snapshot(&data), data_before);
    assert_eq!(tree_snapshot(&vault), vault_before);
}

#[test]
fn stable_bootstrap_keeps_a_truly_empty_no_lease_root_markerless() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir_all(data.join("twin/events")).unwrap();
    let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    lock.unlock().unwrap();

    let coordinator = MutationCoordinator::new_stable(
        &data,
        &vault,
        Arc::new(TwinEventStore::new(&data)),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let lease = coordinator.current_root_epoch().unwrap();

    assert!(lease.is_stable());
    assert!(!data
        .join("twin/stable-vault-migrations/v1")
        .join(lease.root_scope.as_str())
        .exists());
}

#[test]
fn stable_bootstrap_restarts_a_then_fresh_b_then_a_without_cross_vault_marker_binding() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let vault_a = temp.path().join("vault-a");
    let vault_b = temp.path().join("vault-b");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault_a).unwrap();
    std::fs::create_dir(&vault_b).unwrap();

    let legacy_store = Arc::new(TwinEventStore::new(&data));
    legacy_store.initialize().unwrap();
    let legacy = MutationCoordinator::new(
        &data,
        &vault_a,
        legacy_store.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let guard = legacy.begin_root_transition().unwrap();
    let legacy_lease = guard.current_lease().unwrap();
    let legacy_twin = guard
        .prepare_twin_data_path(&vault_a, &legacy_lease)
        .unwrap();
    std::fs::create_dir_all(&legacy_twin).unwrap();
    std::fs::write(legacy_twin.join("a.json"), b"vault-a").unwrap();
    guard.initialize_namespace(&legacy_lease).unwrap();
    drop(guard);
    let _ = legacy
        .commit_local(
            CausalStream::SyncEligible,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            Vec::new(),
            vec![draft("vault-a-event")],
        )
        .unwrap();
    drop(legacy);
    drop(legacy_store);

    let events_a = Arc::new(TwinEventStore::new(&data));
    events_a.initialize().unwrap();
    let coordinator_a = MutationCoordinator::new_stable(
        &data,
        &vault_a,
        events_a.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let lease_a = coordinator_a.current_root_epoch().unwrap();
    assert_eq!(events_a.ordered_events().unwrap().len(), 1);
    assert!(data
        .join("twin/stable-vault-migrations/v1")
        .join(lease_a.root_scope.as_str())
        .join("marker.json")
        .is_file());
    assert!(!data.join("twin/stable-vault-migration-v1.json").exists());
    assert!(!data.join("twin/legacy-assignment-v1.json").exists());
    assert!(!data
        .join("vault_derived/legacy-assignment-v1.json")
        .exists());
    drop(coordinator_a);
    drop(events_a);

    let identity_b =
        crate::services::sync::identity::load_or_create_vault_identity(&vault_b).unwrap();
    let lease_b = ActiveMarkdownRootLeaseV1::new_stable(identity_b.root_scope.clone());
    let data_root = crate::services::twin_events::AnchoredRoot::open(&data).unwrap();
    write_active_root_lease(&data_root, &lease_b).unwrap();
    let events_b = Arc::new(TwinEventStore::new(&data));
    events_b.initialize().unwrap();
    let coordinator_b = MutationCoordinator::new_stable(
        &data,
        &vault_b,
        events_b.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    assert_eq!(coordinator_b.current_root_epoch().unwrap(), lease_b);
    assert!(events_b.ordered_events().unwrap().is_empty());
    drop(coordinator_b);
    drop(events_b);

    write_active_root_lease(&data_root, &lease_a).unwrap();
    let reopened_events_a = Arc::new(TwinEventStore::new(&data));
    reopened_events_a.initialize().unwrap();
    let reopened_a = MutationCoordinator::new_stable(
        &data,
        &vault_a,
        reopened_events_a.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();

    assert_eq!(reopened_a.current_root_epoch().unwrap(), lease_a);
    assert_eq!(reopened_events_a.ordered_events().unwrap().len(), 1);
    assert_eq!(
        std::fs::read(
            crate::models::settings::twin_data_path_for_scope(&data, &lease_a.root_scope)
                .join("a.json")
        )
        .unwrap(),
        b"vault-a"
    );
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
        std::fs::read_dir(temp.path().join(WRITER_STAGING_KEY))
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
    assert!(
        matches!(&result, Err(MutationError::AuthorityAdvanced { .. })),
        "unexpected result: {result:?}"
    );
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
    let desktop_coordinator = desktop.find("MutationCoordinator::new_stable(").unwrap();
    assert!(!desktop[..desktop_coordinator]
        .contains("twin_event_store\n                    .initialize()"));
    let mcp_coordinator = mcp.find("MutationCoordinator::new_custom_mcp(").unwrap();
    assert!(!mcp[..mcp_coordinator].contains("twin_event_store.initialize()?"));
    let recover = mcp.find("recover_pending()").unwrap();
    let serve = mcp.find(".serve(rmcp::transport::stdio())").unwrap();
    assert!(recover < serve);
    assert!(!mcp.contains("KnowledgeStore::new(vault_path"));
}

fn finalized_event_group(labels: &[&str]) -> Vec<crate::models::twin_event::TwinEvent> {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(temp.path()));
    store.initialize().unwrap();
    let coordinator =
        MutationCoordinator::new(temp.path(), &vault, store, Arc::new(NoopMutationLifecycle))
            .unwrap();
    coordinator
        .commit_local(
            CausalStream::SyncEligible,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            Vec::new(),
            labels.iter().map(|label| draft(label)).collect(),
        )
        .unwrap()
        .events
}

#[test]
fn retained_wal_replay_heals_append_before_integrity_head_update() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(temp.path()));
    store.initialize().unwrap();
    let coordinator = MutationCoordinator::new_stable(
        temp.path(),
        &vault,
        store.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let marker = store
        .events_dir()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("integrity/v1/adoption.json");
    let marker_before = std::fs::read(&marker).unwrap();
    let process_lock = coordinator.finalizer.acquire_coordinator_lock().unwrap();
    let intent = coordinator
        .prepare_intent(
            &process_lock,
            MutationOrigin::Local,
            CausalStream::SyncEligible,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            Vec::new(),
            vec![draft("append-before-integrity-head")],
            false,
        )
        .unwrap()
        .unwrap();
    let lease = coordinator.root_lease.lock().unwrap().clone();
    let advanced = crate::services::vault_namespace::advance_authority_locked(
        temp.path(),
        &lease,
        &process_lock,
    )
    .unwrap();
    assert_eq!(
        Some(advanced.authority_generation),
        intent.content_authority_generation
    );
    coordinator.journal.stage(&process_lock, &intent).unwrap();
    coordinator.fail_once_at(MutationFaultPoint::AfterEvent(0));

    let error = coordinator
        .replay_intent_locked(&process_lock, &intent, true, true)
        .unwrap_err();
    assert!(matches!(
        error,
        MutationError::Io(message)
            if message == "injected mutation crash at AfterEvent(0)"
    ));
    process_lock.unlock().unwrap();
    assert_eq!(store.ordered_events().unwrap(), intent.events);
    assert_eq!(std::fs::read(&marker).unwrap(), marker_before);
    drop(coordinator);
    drop(store);

    let reopened_store = Arc::new(TwinEventStore::new(temp.path()));
    let reopened = MutationCoordinator::new_stable(
        temp.path(),
        &vault,
        reopened_store.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();

    assert_eq!(reopened.pending_count().unwrap(), 0);
    assert_eq!(reopened_store.ordered_events().unwrap(), intent.events);
    reopened_store.validate_integrity(None).unwrap();
    assert_ne!(std::fs::read(marker).unwrap(), marker_before);
}

#[derive(Default)]
struct RecordingLifecycle {
    calls: Mutex<Vec<&'static str>>,
}

impl MutationLifecycle for RecordingLifecycle {
    fn stage_before_local(
        &self,
        _: &crate::services::twin_events::MutationIntentV1,
    ) -> Result<(), MutationError> {
        self.calls.lock().unwrap().push("stage");
        Ok(())
    }

    fn committed(
        &self,
        _: &crate::services::twin_events::MutationIntentV1,
    ) -> Result<(), MutationError> {
        self.calls.lock().unwrap().push("committed");
        Ok(())
    }

    fn known_failure(&self, _: Option<&str>, _: &str) -> Result<(), MutationError> {
        self.calls.lock().unwrap().push("known_failure");
        Ok(())
    }
}

#[derive(Default)]
struct FailFirstCommitLifecycle {
    attempts: AtomicUsize,
    calls: Mutex<Vec<&'static str>>,
}

impl MutationLifecycle for FailFirstCommitLifecycle {
    fn stage_before_local(
        &self,
        _: &crate::services::twin_events::MutationIntentV1,
    ) -> Result<(), MutationError> {
        self.calls.lock().unwrap().push("stage");
        Ok(())
    }

    fn committed(
        &self,
        _: &crate::services::twin_events::MutationIntentV1,
    ) -> Result<(), MutationError> {
        self.calls.lock().unwrap().push("committed");
        if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(MutationError::Io("injected sync promotion failure".into()));
        }
        Ok(())
    }
}

#[derive(Default)]
struct FailFirstCancelLifecycle {
    attempts: AtomicUsize,
    calls: Mutex<Vec<&'static str>>,
}

impl MutationLifecycle for FailFirstCancelLifecycle {
    fn stage_before_local(
        &self,
        _: &crate::services::twin_events::MutationIntentV1,
    ) -> Result<(), MutationError> {
        self.calls.lock().unwrap().push("stage");
        Err(MutationError::Io(
            "injected failure after durable sync staging".into(),
        ))
    }

    fn known_failure(&self, _: Option<&str>, _: &str) -> Result<(), MutationError> {
        self.calls.lock().unwrap().push("known_failure");
        if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(MutationError::Io(
                "injected sync cancellation failure".into(),
            ));
        }
        Ok(())
    }
}

#[test]
fn markdown_only_local_mutation_uses_sync_lifecycle_but_remote_never_echoes() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(temp.path()));
    store.initialize().unwrap();
    let lifecycle = Arc::new(RecordingLifecycle::default());
    let coordinator =
        MutationCoordinator::new(temp.path(), &vault, store, lifecycle.clone()).unwrap();

    let _ = coordinator
        .commit_local(
            CausalStream::SyncEligible,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![TargetMutation::put(TargetKind::Markdown, "note.md", "note")],
            Vec::new(),
        )
        .unwrap();
    let _ = coordinator
        .apply_nonlocal(
            MutationOrigin::Remote,
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "remote.md",
                "remote",
            )],
        )
        .unwrap();
    assert_eq!(*lifecycle.calls.lock().unwrap(), vec!["stage", "committed"]);
}

#[test]
fn lifecycle_promotion_failure_retains_local_wal_for_recovery() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(temp.path()));
    store.initialize().unwrap();
    let lifecycle = Arc::new(FailFirstCommitLifecycle::default());
    let coordinator =
        MutationCoordinator::new(temp.path(), &vault, store, lifecycle.clone()).unwrap();

    let commit = coordinator
        .commit_local(
            CausalStream::SyncEligible,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "recover.md",
                "durable",
            )],
            Vec::new(),
        )
        .unwrap();
    assert!(commit.postcommit_warning);
    assert_eq!(coordinator.pending_count().unwrap(), 1);
    assert_eq!(coordinator.recover_pending().unwrap(), 1);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(
        *lifecycle.calls.lock().unwrap(),
        vec!["stage", "committed", "committed"]
    );
}

#[test]
fn failed_lifecycle_cancel_retains_abort_proof_across_restart() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(temp.path()));
    store.initialize().unwrap();
    let lifecycle = Arc::new(FailFirstCancelLifecycle::default());
    let coordinator =
        MutationCoordinator::new(temp.path(), &vault, store, lifecycle.clone()).unwrap();
    let before = coordinator.current_authority_token().unwrap();

    assert!(coordinator
        .commit_local(
            CausalStream::SyncEligible,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "cancel-retry.md",
                "must not commit",
            )],
            Vec::new(),
        )
        .is_err());
    assert_eq!(coordinator.pending_count().unwrap(), 1);
    assert_eq!(
        *lifecycle.calls.lock().unwrap(),
        vec!["stage", "known_failure"]
    );

    drop(coordinator);
    let reopened_store = Arc::new(TwinEventStore::new(temp.path()));
    reopened_store.initialize().unwrap();
    let reopened =
        MutationCoordinator::new(temp.path(), &vault, reopened_store, lifecycle.clone()).unwrap();
    assert_eq!(reopened.pending_count().unwrap(), 0);
    assert_eq!(reopened.current_authority_token().unwrap(), before);
    assert!(!vault.join("cancel-retry.md").exists());
    assert_eq!(
        *lifecycle.calls.lock().unwrap(),
        vec!["stage", "known_failure", "known_failure"]
    );
}

#[test]
fn post_authority_fault_restarts_into_commit_without_canceling_staged_lifecycle() {
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(temp.path()));
    store.initialize().unwrap();
    let lifecycle = Arc::new(RecordingLifecycle::default());
    let coordinator =
        MutationCoordinator::new(temp.path(), &vault, store.clone(), lifecycle.clone()).unwrap();

    coordinator.fail_once_at(MutationFaultPoint::AfterAuthorityAdvance);
    assert!(matches!(
        coordinator.commit_local(
            CausalStream::SyncEligible,
            crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "recover-after-authority.md",
                "durable",
            )],
            vec![draft("recover-after-authority")],
        ),
        Err(MutationError::AuthorityAdvanced { .. })
    ));
    assert_eq!(*lifecycle.calls.lock().unwrap(), vec!["stage"]);

    drop(coordinator);
    drop(store);
    let reopened_store = Arc::new(TwinEventStore::new(temp.path()));
    reopened_store.initialize().unwrap();
    let reopened = MutationCoordinator::new(
        temp.path(),
        &vault,
        reopened_store.clone(),
        lifecycle.clone(),
    )
    .unwrap();
    assert_eq!(*lifecycle.calls.lock().unwrap(), vec!["stage", "committed"]);
    assert_eq!(reopened.pending_count().unwrap(), 0);
    assert_eq!(
        std::fs::read_to_string(vault.join("recover-after-authority.md")).unwrap(),
        "durable"
    );
    assert_eq!(reopened_store.ordered_events().unwrap().len(), 1);
}

#[test]
fn nonlocal_finalized_event_is_exact_idempotent_and_never_echoes() {
    let event = finalized_event_group(&["remote-exact"]).remove(0);
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(temp.path()));
    store.initialize().unwrap();
    let lifecycle = Arc::new(RecordingLifecycle::default());
    let coordinator =
        MutationCoordinator::new(temp.path(), &vault, store.clone(), lifecycle.clone()).unwrap();
    let before = coordinator.current_authority_token().unwrap();

    let first = coordinator
        .apply_nonlocal_finalized_events(MutationOrigin::Remote, vec![event.clone()])
        .unwrap();
    assert_eq!(first.events, vec![event.clone()]);
    assert_eq!(
        coordinator
            .current_authority_token()
            .unwrap()
            .authority_generation,
        before.authority_generation + 1
    );
    let after_first = coordinator.current_authority_token().unwrap();
    let duplicate = coordinator
        .apply_nonlocal_finalized_events(MutationOrigin::Remote, vec![event.clone()])
        .unwrap();
    assert!(duplicate.mutation_id.is_none());
    assert_eq!(coordinator.current_authority_token().unwrap(), after_first);
    assert_eq!(store.ordered_events().unwrap(), vec![event]);
    assert!(lifecycle.calls.lock().unwrap().is_empty());
}

#[test]
fn nonlocal_finalized_event_recovers_after_authority_advance_without_echo() {
    let event = finalized_event_group(&["remote-recovery"]).remove(0);
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(temp.path()));
    store.initialize().unwrap();
    let lifecycle = Arc::new(RecordingLifecycle::default());
    let coordinator =
        MutationCoordinator::new(temp.path(), &vault, store.clone(), lifecycle.clone()).unwrap();

    coordinator.fail_once_at(MutationFaultPoint::AfterAuthorityAdvance);
    assert!(matches!(
        coordinator.apply_nonlocal_finalized_events(MutationOrigin::Remote, vec![event.clone()]),
        Err(MutationError::AuthorityAdvanced { .. })
    ));
    assert_eq!(coordinator.pending_count().unwrap(), 1);
    assert!(store.ordered_events().unwrap().is_empty());

    drop(coordinator);
    drop(store);
    let reopened_store = Arc::new(TwinEventStore::new(temp.path()));
    reopened_store.initialize().unwrap();
    let reopened = MutationCoordinator::new(
        temp.path(),
        &vault,
        reopened_store.clone(),
        lifecycle.clone(),
    )
    .unwrap();
    assert_eq!(reopened.pending_count().unwrap(), 0);
    assert_eq!(reopened_store.ordered_events().unwrap(), vec![event]);
    assert!(lifecycle.calls.lock().unwrap().is_empty());
}

#[test]
fn nonlocal_finalized_events_reject_wrong_id_governance_and_mixed_identity() {
    let event = finalized_event_group(&["remote-invalid"]).remove(0);
    let mut mixed = finalized_event_group(&["remote-group-a", "remote-group-b"]);
    let temp = tempdir().unwrap();
    let vault = temp.path().join("vault");
    std::fs::create_dir(&vault).unwrap();
    let store = Arc::new(TwinEventStore::new(temp.path()));
    store.initialize().unwrap();
    let coordinator = MutationCoordinator::new(
        temp.path(),
        &vault,
        store.clone(),
        Arc::new(NoopMutationLifecycle),
    )
    .unwrap();
    let before = coordinator.current_authority_token().unwrap();

    let mut wrong_id = event.clone();
    wrong_id.event_id = crate::models::twin_event::EventId::parse("0".repeat(64)).unwrap();
    assert!(coordinator
        .apply_nonlocal_finalized_events(MutationOrigin::Remote, vec![wrong_id])
        .is_err());

    let mut disallowed = event;
    disallowed.governance.allowed_uses.sync = false;
    disallowed.event_id = derive_event_id(&disallowed);
    assert!(coordinator
        .apply_nonlocal_finalized_events(MutationOrigin::Remote, vec![disallowed])
        .is_err());

    mixed[1].actor_id = crate::models::twin_event::ActorId::parse("other-owner").unwrap();
    mixed[1].event_id = derive_event_id(&mixed[1]);
    assert!(coordinator
        .apply_nonlocal_finalized_events(MutationOrigin::Remote, mixed)
        .is_err());
    assert_eq!(coordinator.current_authority_token().unwrap(), before);
    assert!(store.ordered_events().unwrap().is_empty());
}
