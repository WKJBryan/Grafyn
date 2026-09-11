use super::*;
use crate::models::settings::UserSettings;
use crate::models::sync::VaultDescriptorV1;
use crate::services::sync::identity::{load_or_create_vault_identity, VAULT_DESCRIPTOR_KEY};
use crate::services::twin_events::{root_identity_for_path, ActiveMarkdownRootLeaseV1};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn fixture() -> (tempfile::TempDir, RootTransitionStore, RootTransitionV1) {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let config = temp.path().join("config");
    let old = temp.path().join("vault-a");
    let new = temp.path().join("vault-b");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&old).unwrap();
    std::fs::create_dir(&new).unwrap();
    let secrets = Arc::new(MemoryVersionedSecretStore::default());
    let store = RootTransitionStore::new(&data, config.join("settings.json"), secrets).unwrap();
    let before_settings = UserSettings {
        vault_path: Some(old.to_string_lossy().into_owned()),
        ..UserSettings::default()
    };
    let after_settings = UserSettings {
        vault_path: Some(new.to_string_lossy().into_owned()),
        theme: "dark".into(),
        ..UserSettings::default()
    };
    let old_key = Uuid::new_v4().to_string();
    let new_key = Uuid::new_v4().to_string();
    let before = RootAuthorityV1::new(
        &old,
        before_settings,
        ActiveMarkdownRootLeaseV1::new(root_identity_for_path(&old).unwrap()),
        Some(old_key),
    )
    .unwrap();
    let after = RootAuthorityV1::new(
        &new,
        after_settings,
        ActiveMarkdownRootLeaseV1::new(root_identity_for_path(&new).unwrap()),
        Some(new_key),
    )
    .unwrap();
    let transition = RootTransitionV1::prepared(before, after, store.authority_binding()).unwrap();
    (temp, store, transition)
}

fn stable_fixture() -> (tempfile::TempDir, RootTransitionStore, RootTransitionV1) {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let config = temp.path().join("config");
    let old = temp.path().join("stable-vault-a");
    let new = temp.path().join("stable-vault-b");
    let old_settings_alias = temp.path().join("old-settings-alias");
    let new_settings_alias = temp.path().join("new-settings-alias");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&old).unwrap();
    std::fs::create_dir(&new).unwrap();
    std::fs::create_dir(&old_settings_alias).unwrap();
    std::fs::create_dir(&new_settings_alias).unwrap();
    let old_identity = load_or_create_vault_identity(&old).unwrap();
    let new_identity = load_or_create_vault_identity(&new).unwrap();
    let store = RootTransitionStore::new(
        &data,
        config.join("settings.json"),
        Arc::new(MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    let before = RootAuthorityV1::new(
        &old,
        UserSettings {
            vault_path: Some(
                old_settings_alias
                    .join("..")
                    .join("stable-vault-a")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..UserSettings::default()
        },
        ActiveMarkdownRootLeaseV1::new_stable(old_identity.root_scope),
        None,
    )
    .unwrap();
    let after = RootAuthorityV1::new(
        &new,
        UserSettings {
            vault_path: Some(
                new_settings_alias
                    .join("..")
                    .join("stable-vault-b")
                    .to_string_lossy()
                    .into_owned(),
            ),
            theme: "dark".into(),
            ..UserSettings::default()
        },
        ActiveMarkdownRootLeaseV1::new_stable(new_identity.root_scope),
        None,
    )
    .unwrap();
    let transition = RootTransitionV1::prepared(before, after, store.authority_binding()).unwrap();
    (temp, store, transition)
}

fn stable_owned_descriptor_fixture() -> (
    tempfile::TempDir,
    RootTransitionStore,
    RootTransitionV1,
    PathBuf,
    Vec<u8>,
) {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let config = temp.path().join("config");
    let old = temp.path().join("stable-vault-a");
    let new = temp.path().join("stable-vault-b");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&old).unwrap();
    std::fs::create_dir(&new).unwrap();
    let old_identity = load_or_create_vault_identity(&old).unwrap();
    let descriptor = VaultDescriptorV1::generate();
    let mut descriptor_bytes = serde_json::to_vec_pretty(&descriptor).unwrap();
    descriptor_bytes.push(b'\n');
    let new_scope = crate::services::sync::identity::stable_vault_scope(descriptor.vault_id());
    let store = RootTransitionStore::new(
        &data,
        config.join("settings.json"),
        Arc::new(MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    let before = RootAuthorityV1::new(
        &old,
        UserSettings {
            vault_path: Some(old.to_string_lossy().into_owned()),
            ..UserSettings::default()
        },
        ActiveMarkdownRootLeaseV1::new_stable(old_identity.root_scope),
        None,
    )
    .unwrap();
    let after = RootAuthorityV1::new_with_pending_stable_descriptor(
        &new,
        UserSettings {
            vault_path: Some(new.to_string_lossy().into_owned()),
            theme: "dark".into(),
            ..UserSettings::default()
        },
        ActiveMarkdownRootLeaseV1::new_stable(new_scope),
        OpenRouterKeySource::Unset,
        None,
    )
    .unwrap();
    let transition = RootTransitionV1::prepared_with_owned_candidate_descriptor(
        before,
        after,
        store.authority_binding(),
        Some(descriptor_bytes.clone()),
    )
    .unwrap();
    (temp, store, transition, new, descriptor_bytes)
}

fn detached_stable_fixture() -> (
    tempfile::TempDir,
    RootTransitionStore,
    PathBuf,
    PathBuf,
    crate::services::sync::identity::VaultIdentity,
    ActiveMarkdownRootLeaseV1,
) {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let config = temp.path().join("config");
    let old = temp.path().join("old-vault");
    let moved = temp.path().join("moved-vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&old).unwrap();
    let identity = load_or_create_vault_identity(&old).unwrap();
    let store = RootTransitionStore::new(
        &data,
        config.join("settings.json"),
        Arc::new(MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    let settings = UserSettings {
        vault_path: Some(old.to_string_lossy().into_owned()),
        ..UserSettings::default()
    };
    let lease = ActiveMarkdownRootLeaseV1::new_stable(identity.root_scope.clone());
    store
        .write_settings(&NonsecretSettingsV1::from_settings(settings))
        .unwrap();
    store.write_lease(&lease).unwrap();
    std::fs::rename(&old, &moved).unwrap();
    (temp, store, old, moved, identity, lease)
}

#[test]
fn stable_same_identity_path_move_rotates_epoch_without_changing_namespace() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("vault-a");
    let moved = temp.path().join("vault-b");
    std::fs::create_dir(&old).unwrap();
    std::fs::create_dir(&moved).unwrap();
    let identity = load_or_create_vault_identity(&old).unwrap();
    std::fs::create_dir(moved.join("_grafyn")).unwrap();
    std::fs::copy(
        old.join(VAULT_DESCRIPTOR_KEY),
        moved.join(VAULT_DESCRIPTOR_KEY),
    )
    .unwrap();

    let before = RootAuthorityV1::new(
        &old,
        UserSettings {
            vault_path: Some(old.to_string_lossy().into_owned()),
            ..UserSettings::default()
        },
        ActiveMarkdownRootLeaseV1::new_stable(identity.root_scope.clone()),
        None,
    )
    .unwrap();
    let after = RootAuthorityV1::new(
        &moved,
        UserSettings {
            vault_path: Some(moved.to_string_lossy().into_owned()),
            ..UserSettings::default()
        },
        ActiveMarkdownRootLeaseV1::new_stable(identity.root_scope.clone()),
        None,
    )
    .unwrap();
    let transition = RootTransitionV1::prepared(
        before.clone(),
        after.clone(),
        ContentDigest::parse(&"a".repeat(64)).unwrap(),
    )
    .unwrap();

    assert_eq!(
        transition.schema_version,
        STABLE_ROOT_TRANSITION_SCHEMA_VERSION
    );
    assert_eq!(transition.path_changed, Some(true));
    assert_eq!(transition.vault_identity_changed, Some(false));
    assert!(transition.root_changed);
    assert_eq!(before.root_scope, after.root_scope);
    assert_ne!(before.lease.epoch_uuid, after.lease.epoch_uuid);
}

#[test]
fn stable_authority_rejects_descriptor_replacement_at_same_path() {
    let vault = tempfile::tempdir().unwrap();
    let identity = load_or_create_vault_identity(vault.path()).unwrap();
    let authority = RootAuthorityV1::new(
        vault.path(),
        UserSettings {
            vault_path: Some(vault.path().to_string_lossy().into_owned()),
            ..UserSettings::default()
        },
        ActiveMarkdownRootLeaseV1::new_stable(identity.root_scope),
        None,
    )
    .unwrap();
    let replacement = serde_json::to_vec_pretty(&VaultDescriptorV1::generate()).unwrap();
    std::fs::write(vault.path().join(VAULT_DESCRIPTOR_KEY), replacement).unwrap();

    assert!(authority.validate().is_err());
}

#[test]
fn stable_prepare_revalidates_both_live_authorities_before_persisting_wal() {
    let (_temp, missing_store, missing_transition) = stable_fixture();
    std::fs::remove_dir_all(&missing_transition.after.canonical_vault_path).unwrap();

    assert!(missing_store
        .prepare_transition(&missing_transition)
        .is_err());
    assert!(!missing_store.transition_exists_for_test().unwrap());

    let (_temp, replaced_store, replaced_transition) = stable_fixture();
    std::fs::write(
        Path::new(&replaced_transition.after.canonical_vault_path).join(VAULT_DESCRIPTOR_KEY),
        serde_json::to_vec_pretty(&VaultDescriptorV1::generate()).unwrap(),
    )
    .unwrap();

    assert!(replaced_store
        .prepare_transition(&replaced_transition)
        .is_err());
    assert!(!replaced_store.transition_exists_for_test().unwrap());
}

#[test]
fn missing_stable_vault_reattaches_forward_to_the_same_descriptor() {
    let (temp, store, old, moved, identity, original_lease) = detached_stable_fixture();
    let data = temp.path().join("data");

    let detached = store.detached_stable_vault().unwrap().unwrap();
    assert_eq!(detached.configured_path, old);
    assert_eq!(detached.root_scope, identity.root_scope);
    let reattached = store.reattach_missing_stable_vault(&moved).unwrap();

    assert_eq!(
        std::fs::canonicalize(reattached.settings.effective_vault_path()).unwrap(),
        std::fs::canonicalize(&moved).unwrap()
    );
    assert!(
        !old.exists(),
        "reattach must not recreate the missing old root"
    );
    let process_lock =
        crate::services::twin_events::acquire_shared_coordinator_process_lock(&data).unwrap();
    let durable = store.read_authority_locked(&process_lock).unwrap();
    process_lock.unlock().unwrap();
    assert_eq!(durable.authority.root_scope, identity.root_scope);
    assert_ne!(
        durable.authority.lease.epoch_uuid,
        original_lease.epoch_uuid
    );
    assert_eq!(store.recover().unwrap(), RecoveryWork::None);
}

#[test]
fn forward_reattach_rejects_a_prepared_stable_migration_without_writing() {
    let (temp, store, old, moved, identity, lease) = detached_stable_fixture();
    let data = temp.path().join("data");
    let legacy_scope = ContentDigest::parse("a".repeat(64)).unwrap();
    let marker = serde_json::json!({
        "schema_version": 1,
        "vault_id": identity.descriptor.vault_id().to_string(),
        "stable_scope": identity.root_scope.clone(),
        "legacy_scope": legacy_scope.clone(),
        "legacy_lease_epoch_uuid": "123e4567-e89b-42d3-a456-426614174001",
        "writer_device_id": "123e4567-e89b-42d3-a456-426614174002",
        "stable_lease": lease.clone(),
        "components": [
            {
                "source": format!("twin/{}", legacy_scope.as_str()),
                "destination": format!("twin/{}", identity.root_scope.as_str()),
                "kind": "directory"
            },
            {
                "source": format!("vault_derived/v1/{}", legacy_scope.as_str()),
                "destination": format!("vault_derived/v1/{}", identity.root_scope.as_str()),
                "kind": "directory"
            }
        ],
        "moved_sources": [],
        "lease_published": false,
        "state": "prepared"
    });
    let marker_path = data
        .join("twin/stable-vault-migrations/v1")
        .join(identity.root_scope.as_str())
        .join("marker.json");
    std::fs::create_dir_all(marker_path.parent().unwrap()).unwrap();
    std::fs::write(&marker_path, serde_json::to_vec_pretty(&marker).unwrap()).unwrap();
    let settings_before = store.read_settings_bytes_for_test().unwrap();
    let lease_before = store.read_lease().unwrap();
    let marker_before = std::fs::read(&marker_path).unwrap();

    let error = store.reattach_missing_stable_vault(&moved).unwrap_err();

    assert!(error.to_string().contains("prepared stable migration"));
    assert_eq!(
        store.read_settings_bytes_for_test().unwrap(),
        settings_before
    );
    assert_eq!(store.read_lease().unwrap(), lease_before);
    assert_eq!(std::fs::read(marker_path).unwrap(), marker_before);
    assert!(!store.transition_exists_for_test().unwrap());
    assert!(!old.exists());
}

#[test]
fn forward_reattach_rejects_ambiguous_candidates_without_changing_durable_state() {
    let (temp, store, old, _moved, _identity, original_lease) = detached_stable_fixture();
    let wrong_identity = temp.path().join("wrong-identity");
    let missing_descriptor = temp.path().join("missing-descriptor");
    let corrupt_descriptor = temp.path().join("corrupt-descriptor");
    std::fs::create_dir(&wrong_identity).unwrap();
    std::fs::create_dir(&missing_descriptor).unwrap();
    std::fs::create_dir(&corrupt_descriptor).unwrap();
    load_or_create_vault_identity(&wrong_identity).unwrap();
    std::fs::create_dir(corrupt_descriptor.join("_grafyn")).unwrap();
    std::fs::write(corrupt_descriptor.join(VAULT_DESCRIPTOR_KEY), b"not-json").unwrap();
    let settings_before = store.read_settings_bytes_for_test().unwrap();

    for candidate in [&wrong_identity, &missing_descriptor, &corrupt_descriptor] {
        assert!(store.reattach_missing_stable_vault(candidate).is_err());
        assert_eq!(
            store.read_settings_bytes_for_test().unwrap(),
            settings_before
        );
        assert_eq!(store.read_lease().unwrap(), original_lease);
        assert!(!store.transition_exists_for_test().unwrap());
        assert!(!old.exists());
    }
    assert!(!missing_descriptor.join(VAULT_DESCRIPTOR_KEY).exists());
}

#[test]
fn permission_denied_configured_vault_cannot_enable_forward_reattach() {
    let (_temp, store, old, moved, _identity, original_lease) = detached_stable_fixture();
    let settings_before = store.read_settings_bytes_for_test().unwrap();

    let error = store
        .reattach_missing_stable_vault_with_metadata(&moved, |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected configured-vault denial",
            ))
        })
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("injected configured-vault denial"));
    assert_eq!(
        store.read_settings_bytes_for_test().unwrap(),
        settings_before
    );
    assert_eq!(store.read_lease().unwrap(), original_lease);
    assert!(!store.transition_exists_for_test().unwrap());
    assert!(!old.exists());
}

#[test]
fn healthy_or_replaced_configured_stable_vault_never_enters_reattach_mode() {
    let (temp, store, old, moved, identity, _lease) = detached_stable_fixture();
    std::fs::rename(&moved, &old).unwrap();
    assert!(store.detached_stable_vault().unwrap().is_none());

    std::fs::create_dir(&moved).unwrap();
    std::fs::create_dir(moved.join("_grafyn")).unwrap();
    std::fs::copy(
        old.join(VAULT_DESCRIPTOR_KEY),
        moved.join(VAULT_DESCRIPTOR_KEY),
    )
    .unwrap();
    assert!(store.reattach_missing_stable_vault(&moved).is_err());

    let replacement = VaultDescriptorV1::generate();
    assert_ne!(
        crate::services::sync::identity::stable_vault_scope(replacement.vault_id()),
        identity.root_scope
    );
    std::fs::write(
        old.join(VAULT_DESCRIPTOR_KEY),
        serde_json::to_vec_pretty(&replacement).unwrap(),
    )
    .unwrap();
    assert!(store.detached_stable_vault().is_err());
    assert!(old.is_dir());
    assert!(temp.path().is_dir());
}

#[test]
fn first_run_without_a_durable_lease_is_not_misclassified_as_detached() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let config = temp.path().join("config");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&config).unwrap();
    let store = RootTransitionStore::new(
        &data,
        config.join("settings.json"),
        Arc::new(MemoryVersionedSecretStore::default()),
    )
    .unwrap();

    assert!(store.detached_stable_vault().unwrap().is_none());
}

#[test]
fn interrupted_forward_reattach_always_recovers_forward_and_never_recreates_old_root() {
    for point in [
        RootTransitionFaultPoint::AfterPrepared,
        RootTransitionFaultPoint::AfterLease,
        RootTransitionFaultPoint::AfterSettings,
        RootTransitionFaultPoint::AfterKeyRef,
        RootTransitionFaultPoint::AfterRecoveryWalDelete,
    ] {
        let (_temp, store, old, moved, identity, original_lease) = detached_stable_fixture();
        store.fail_once_at(point);
        assert!(
            store.reattach_missing_stable_vault(&moved).is_err(),
            "fault {point:?} must interrupt reattach"
        );

        let recovered = store.recover().unwrap();
        if point == RootTransitionFaultPoint::AfterRecoveryWalDelete {
            assert_eq!(recovered, RecoveryWork::None);
        } else {
            assert_eq!(recovered, RecoveryWork::RolledForward);
        }
        let settings = store.read_settings_snapshot().unwrap().settings;
        assert_eq!(
            std::fs::canonicalize(settings.effective_vault_path()).unwrap(),
            std::fs::canonicalize(&moved).unwrap()
        );
        let lease = store.read_lease().unwrap();
        assert!(lease.is_stable());
        assert_eq!(lease.root_scope, identity.root_scope);
        assert_ne!(lease.epoch_uuid, original_lease.epoch_uuid);
        assert!(!old.exists());
        assert_eq!(store.recover().unwrap(), RecoveryWork::None);
    }
}

#[test]
fn active_lease_rejects_nil_and_noncanonical_epochs_for_both_schemas() {
    let (_temp, store, transition) = fixture();
    let scope = transition.before.root_scope;
    for schema_version in [LEASE_SCHEMA_VERSION, STABLE_LEASE_SCHEMA_VERSION] {
        for epoch_uuid in [
            Uuid::nil().to_string(),
            "123E4567-E89B-42D3-A456-426614174000".to_string(),
            "123e4567e89b42d3a456426614174000".to_string(),
        ] {
            let lease = ActiveMarkdownRootLeaseV1 {
                schema_version,
                root_scope: scope.clone(),
                epoch_uuid,
            };
            assert!(
                store.write_lease(&lease).is_err(),
                "schema {schema_version} accepted a nil or noncanonical epoch"
            );
            let encoded = serde_json::to_vec(&lease).unwrap();
            store
                .data_root
                .put_atomic(ACTIVE_ROOT_LEASE_KEY, &encoded)
                .unwrap();
            assert!(
                store.read_lease().is_err(),
                "schema {schema_version} read a nil or noncanonical epoch"
            );
            let valid = if schema_version == STABLE_LEASE_SCHEMA_VERSION {
                ActiveMarkdownRootLeaseV1::new_stable(scope.clone())
            } else {
                ActiveMarkdownRootLeaseV1::new(scope.clone())
            };
            store.write_lease(&valid).unwrap();
        }
    }
}

#[test]
fn guarded_settings_patch_uses_fresh_durable_state_instead_of_stale_process_state() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let config = temp.path().join("config");
    let vault = temp.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&config).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let secrets = Arc::new(MemoryVersionedSecretStore::default());
    let first =
        RootTransitionStore::new(&data, config.join("settings.json"), secrets.clone()).unwrap();
    let stale = RootTransitionStore::new(&data, config.join("settings.json"), secrets).unwrap();
    let initial = UserSettings {
        vault_path: Some(vault.to_string_lossy().into_owned()),
        theme: "light".into(),
        mcp_enabled: false,
        ..UserSettings::default()
    };
    first
        .write_settings_guarded(&NonsecretSettingsV1::from_settings(initial))
        .unwrap();

    first
        .patch_settings_guarded(|fresh| {
            fresh.mcp_enabled = true;
            Ok(())
        })
        .unwrap();
    let patched = stale
        .patch_settings_guarded(|fresh| {
            fresh.theme = "dark".into();
            Ok(())
        })
        .unwrap();

    assert_eq!(patched.settings.theme, "dark");
    assert!(patched.settings.mcp_enabled);
    assert_eq!(patched.settings.effective_vault_path(), vault);
}

#[test]
fn durable_settings_snapshot_debug_never_exposes_resolved_secrets() {
    let resolved_secret = "resolved-debug-secret";
    let settings_secret = "settings-debug-secret";
    let snapshot = DurableSettingsSnapshot {
        settings: UserSettings {
            openrouter_api_key: Some(settings_secret.into()),
            ..UserSettings::default()
        },
        settings_generation: ContentDigest::parse(&"a".repeat(64)).unwrap(),
        active_key_version: Some(Uuid::new_v4().to_string()),
        key_source: OpenRouterKeySource::Versioned,
        resolved_secret: Some(resolved_secret.into()),
    };

    let debug = format!("{snapshot:?}");

    assert!(!debug.contains(resolved_secret));
    assert!(!debug.contains(settings_secret));
    assert!(debug.contains("resolved_secret_present: true"));
    assert!(debug.contains("settings_generation"));
}

#[test]
fn copied_transition_wal_cannot_act_for_another_data_config_or_key_authority() {
    let (_fixture, source, transition) = fixture();
    let other = tempfile::tempdir().unwrap();
    let data = other.path().join("data");
    let config = other.path().join("config");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&config).unwrap();
    let destination = RootTransitionStore::new(
        &data,
        config.join("settings.json"),
        Arc::new(MemoryVersionedSecretStore::default()),
    )
    .unwrap();

    assert_ne!(source.authority_binding(), destination.authority_binding());
    let error = destination.prepare_transition(&transition).unwrap_err();
    assert!(error.to_string().contains("authority-binding"));
    assert!(!destination.transition_exists_for_test().unwrap());
}

#[test]
fn prepared_recovery_rolls_back_mixed_authorities_and_second_restart_is_zero_work() {
    let (_temp, store, transition) = fixture();
    let old_key = transition.before.openrouter_key_version.as_deref().unwrap();
    let new_key = transition.after.openrouter_key_version.as_deref().unwrap();
    store.seed_secret_for_test(old_key, "old-secret").unwrap();
    store.seed_secret_for_test(new_key, "new-secret").unwrap();
    store.write_authority_for_test(&transition.before).unwrap();
    store.write_transition(&transition).unwrap();
    store.write_lease(&transition.after.lease).unwrap();
    store
        .write_key_ref(transition.after.openrouter_key_version.as_deref())
        .unwrap();

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);
    let recovered = store.read_authority_for_test().unwrap();
    assert_eq!(
        recovered.nonsecret_settings,
        transition.before.nonsecret_settings
    );
    assert_eq!(recovered.root_scope, transition.before.root_scope);
    assert_eq!(
        recovered.openrouter_key_version,
        transition.before.openrouter_key_version
    );
    assert_eq!(recovered.lease, transition.rollback_lease);
    assert!(store.read_secret_for_test(new_key).unwrap().is_none());
    assert_eq!(store.recover().unwrap(), RecoveryWork::None);
}

#[test]
fn stable_prepared_recovery_rolls_back_when_the_unchosen_after_root_is_missing() {
    let (_temp, store, transition) = stable_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.write_transition(&transition).unwrap();
    std::fs::remove_dir_all(&transition.after.canonical_vault_path).unwrap();

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);
    let recovered = store.read_authority_for_test().unwrap();
    assert_eq!(
        recovered.nonsecret_settings,
        transition.before.nonsecret_settings
    );
    assert_eq!(recovered.root_scope, transition.before.root_scope);
    assert_eq!(recovered.lease, transition.rollback_lease);
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn prepared_recovery_removes_the_exact_candidate_descriptor_owned_by_the_wal() {
    let (_temp, store, mut transition, candidate, descriptor_bytes) =
        stable_owned_descriptor_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    store
        .install_owned_candidate_vault_descriptor(&mut transition)
        .unwrap();
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);
    assert_eq!(std::fs::read(&descriptor_path).unwrap(), descriptor_bytes);

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);

    assert!(!descriptor_path.exists());
    assert!(!store.transition_exists_for_test().unwrap());
    assert_eq!(store.recover().unwrap(), RecoveryWork::None);
}

#[test]
fn prepared_recovery_before_candidate_descriptor_install_has_nothing_to_remove() {
    let (_temp, store, transition, candidate, _descriptor_bytes) =
        stable_owned_descriptor_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);
    assert!(!descriptor_path.exists());

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);

    assert!(!descriptor_path.exists());
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn candidate_descriptor_install_requires_its_exact_prepared_wal() {
    let (_temp, store, mut transition, candidate, _descriptor_bytes) =
        stable_owned_descriptor_fixture();
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);

    assert!(store
        .install_owned_candidate_vault_descriptor(&mut transition)
        .is_err());

    assert!(!descriptor_path.exists());
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn candidate_descriptor_collision_before_wal_is_preserved_without_publication() {
    let (_temp, store, transition, candidate, _descriptor_bytes) =
        stable_owned_descriptor_fixture();
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);
    std::fs::create_dir(candidate.join("_grafyn")).unwrap();
    let mut existing = serde_json::to_vec_pretty(&VaultDescriptorV1::generate()).unwrap();
    existing.push(b'\n');
    std::fs::write(&descriptor_path, &existing).unwrap();

    assert!(store.prepare_transition(&transition).is_err());

    assert_eq!(std::fs::read(descriptor_path).unwrap(), existing);
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn candidate_descriptor_collision_after_wal_is_never_adopted_even_when_bytes_match() {
    let (_temp, store, mut transition, candidate, descriptor_bytes) =
        stable_owned_descriptor_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);
    std::fs::create_dir(candidate.join("_grafyn")).unwrap();
    std::fs::write(&descriptor_path, &descriptor_bytes).unwrap();

    assert!(store
        .install_owned_candidate_vault_descriptor(&mut transition)
        .is_err());
    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);

    assert_eq!(std::fs::read(descriptor_path).unwrap(), descriptor_bytes);
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn candidate_descriptor_witness_collision_after_wal_never_bootstraps_ownership() {
    let (_temp, store, mut transition, candidate, descriptor_bytes) =
        stable_owned_descriptor_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    let witness_path = candidate.join(transition.candidate_descriptor_rollback_key());
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);
    std::fs::create_dir(candidate.join("_grafyn")).unwrap();
    std::fs::write(&witness_path, &descriptor_bytes).unwrap();
    std::fs::hard_link(&witness_path, &descriptor_path).unwrap();

    assert!(store
        .install_owned_candidate_vault_descriptor(&mut transition)
        .is_err());
    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);

    assert_eq!(std::fs::read(descriptor_path).unwrap(), descriptor_bytes);
    assert_eq!(std::fs::read(witness_path).unwrap(), descriptor_bytes);
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn raced_live_link_after_witness_claim_is_preserved_without_live_install_proof() {
    let (_temp, store, mut transition, candidate, descriptor_bytes) =
        stable_owned_descriptor_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    let root = AnchoredRoot::open(&candidate).unwrap();
    let witness_key = transition.candidate_descriptor_rollback_key();
    assert_eq!(
        root.install_no_clobber_with_outcome(&witness_key, "_grafyn", &descriptor_bytes)
            .unwrap(),
        crate::services::twin_events::NoClobberInstallOutcome::Installed
    );
    let witness_identity = root.regular_file_identity(&witness_key).unwrap().unwrap();
    store
        .record_owned_candidate_descriptor_witness(&mut transition, witness_identity)
        .unwrap();
    let witness = transition.owned_candidate_vault_descriptor_witness.unwrap();
    assert!(!witness.live_installed);
    let witness_path = candidate.join(&witness_key);
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);
    std::fs::hard_link(&witness_path, &descriptor_path).unwrap();

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);

    assert_eq!(std::fs::read(descriptor_path).unwrap(), descriptor_bytes);
    assert!(!witness_path.exists());
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn prepared_recovery_removes_descriptor_published_before_live_install_wal_update() {
    let (_temp, store, mut transition, candidate, descriptor_bytes) =
        stable_owned_descriptor_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    store.fail_once_at(RootTransitionFaultPoint::AfterCandidateDescriptorLiveInstall);

    assert!(store
        .install_owned_candidate_vault_descriptor(&mut transition)
        .is_err());
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);
    assert_eq!(std::fs::read(&descriptor_path).unwrap(), descriptor_bytes);
    assert!(
        !transition
            .owned_candidate_vault_descriptor_witness
            .unwrap()
            .live_installed
    );

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);

    assert!(!descriptor_path.exists());
    assert!(!candidate
        .join(transition.candidate_descriptor_rollback_key())
        .exists());
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn prepared_recovery_preserves_a_preexisting_candidate_descriptor() {
    let (_temp, store, transition) = stable_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    let descriptor_path =
        Path::new(&transition.after.canonical_vault_path).join(VAULT_DESCRIPTOR_KEY);
    let descriptor_before = std::fs::read(&descriptor_path).unwrap();
    store.write_transition(&transition).unwrap();

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);

    assert_eq!(std::fs::read(descriptor_path).unwrap(), descriptor_before);
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn prepared_recovery_preserves_a_replaced_owned_descriptor_without_blocking_rollback() {
    let (_temp, store, mut transition, candidate, _descriptor_bytes) =
        stable_owned_descriptor_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    store
        .install_owned_candidate_vault_descriptor(&mut transition)
        .unwrap();
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);
    let mut replacement = serde_json::to_vec_pretty(&VaultDescriptorV1::generate()).unwrap();
    replacement.push(b'\n');
    std::fs::remove_file(&descriptor_path).unwrap();
    std::fs::write(&descriptor_path, &replacement).unwrap();

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);

    assert_eq!(std::fs::read(descriptor_path).unwrap(), replacement);
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn prepared_recovery_preserves_same_byte_descriptor_replacement_with_a_new_identity() {
    let (_temp, store, mut transition, candidate, descriptor_bytes) =
        stable_owned_descriptor_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    store
        .install_owned_candidate_vault_descriptor(&mut transition)
        .unwrap();
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);
    let witness_path = candidate.join(transition.candidate_descriptor_rollback_key());
    std::fs::remove_file(&descriptor_path).unwrap();
    std::fs::write(&descriptor_path, &descriptor_bytes).unwrap();

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);

    assert_eq!(std::fs::read(descriptor_path).unwrap(), descriptor_bytes);
    assert!(!witness_path.exists());
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn prepared_recovery_resumes_after_owned_descriptor_cleanup() {
    let (_temp, store, mut transition, candidate, _descriptor_bytes) =
        stable_owned_descriptor_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    store
        .install_owned_candidate_vault_descriptor(&mut transition)
        .unwrap();
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);
    store.fail_once_at(RootTransitionFaultPoint::AfterRollbackCandidateDescriptor);

    assert!(store.recover().is_err());
    assert!(!descriptor_path.exists());
    assert!(store.transition_exists_for_test().unwrap());

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledBack);
    assert!(!descriptor_path.exists());
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn committed_recovery_preserves_the_owned_candidate_descriptor_and_rolls_forward() {
    let (_temp, store, mut transition, candidate, descriptor_bytes) =
        stable_owned_descriptor_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    store
        .install_owned_candidate_vault_descriptor(&mut transition)
        .unwrap();
    let committed = match store.mark_committed(&transition) {
        MarkCommittedResult::Committed(committed) => committed,
        other => panic!("expected a durable committed transition, got {other:?}"),
    };
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledForward);

    assert_eq!(std::fs::read(&descriptor_path).unwrap(), descriptor_bytes);
    assert_eq!(store.read_authority_for_test().unwrap(), committed.after);
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn committed_recovery_resumes_after_candidate_descriptor_witness_cleanup() {
    let (_temp, store, mut transition, candidate, descriptor_bytes) =
        stable_owned_descriptor_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    store
        .install_owned_candidate_vault_descriptor(&mut transition)
        .unwrap();
    let committed = match store.mark_committed(&transition) {
        MarkCommittedResult::Committed(committed) => committed,
        other => panic!("expected a durable committed transition, got {other:?}"),
    };
    let descriptor_path = candidate.join(VAULT_DESCRIPTOR_KEY);
    let witness_path = candidate.join(committed.candidate_descriptor_rollback_key());
    store.fail_once_at(RootTransitionFaultPoint::AfterCommittedCandidateDescriptor);

    assert!(store.recover().is_err());
    assert_eq!(std::fs::read(&descriptor_path).unwrap(), descriptor_bytes);
    assert!(!witness_path.exists());
    assert!(store.transition_exists_for_test().unwrap());

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledForward);
    assert_eq!(std::fs::read(descriptor_path).unwrap(), descriptor_bytes);
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn stable_committed_recovery_rolls_forward_when_the_unchosen_before_root_is_missing() {
    let (_temp, store, mut transition) = stable_fixture();
    store.write_authority_for_test(&transition.before).unwrap();
    transition.decision = RootTransitionDecision::Committed;
    store.write_transition(&transition).unwrap();
    std::fs::remove_dir_all(&transition.before.canonical_vault_path).unwrap();

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledForward);
    assert_eq!(store.read_authority_for_test().unwrap(), transition.after);
    assert!(!store.transition_exists_for_test().unwrap());
}

#[test]
fn stable_missing_unchosen_authority_still_validates_every_serialized_boundary() {
    for committed in [false, true] {
        for malformed in ["shape", "settings", "path", "key", "lease"] {
            let (_temp, store, mut transition) = stable_fixture();
            store.write_authority_for_test(&transition.before).unwrap();
            if committed {
                transition.decision = RootTransitionDecision::Committed;
            }
            let missing_path = if committed {
                transition.before.canonical_vault_path.clone()
            } else {
                transition.after.canonical_vault_path.clone()
            };
            let side_name = if committed { "before" } else { "after" };
            let mut value = serde_json::to_value(&transition).unwrap();
            let side = value
                .get_mut(side_name)
                .and_then(serde_json::Value::as_object_mut)
                .unwrap();
            match malformed {
                "shape" => {
                    side.insert("unknown".into(), serde_json::Value::Bool(true));
                }
                "settings" => {
                    side.get_mut("nonsecret_settings").unwrap()["theme"] =
                        serde_json::Value::String("x".repeat(32 * 1024 + 1));
                }
                "path" => {
                    side.insert(
                        "canonical_vault_path".into(),
                        serde_json::Value::String("relative-vault".into()),
                    );
                }
                "key" => {
                    side.insert(
                        "openrouter_key_source".into(),
                        serde_json::Value::String("versioned".into()),
                    );
                }
                "lease" => {
                    side.get_mut("lease").unwrap()["epoch_uuid"] =
                        serde_json::Value::String(Uuid::nil().to_string());
                }
                _ => unreachable!(),
            }
            store
                .write_raw_transition_for_test(&serde_json::to_vec(&value).unwrap())
                .unwrap();
            std::fs::remove_dir_all(missing_path).unwrap();
            let settings_before = store.read_settings_bytes_for_test().unwrap();
            let lease_before = store.read_lease().unwrap();

            assert!(
                store.recover().is_err(),
                "{side_name} {malformed} must fail closed"
            );
            assert_eq!(
                store.read_settings_bytes_for_test().unwrap(),
                settings_before
            );
            assert_eq!(store.read_lease().unwrap(), lease_before);
            assert!(store.transition_exists_for_test().unwrap());
        }
    }
}

#[test]
fn schema_one_recovery_still_requires_both_vault_roots_to_be_live() {
    let (_temp, prepared_store, prepared) = fixture();
    let old_key = prepared.before.openrouter_key_version.as_deref().unwrap();
    let new_key = prepared.after.openrouter_key_version.as_deref().unwrap();
    prepared_store
        .seed_secret_for_test(old_key, "old-secret")
        .unwrap();
    prepared_store
        .seed_secret_for_test(new_key, "new-secret")
        .unwrap();
    prepared_store
        .write_authority_for_test(&prepared.before)
        .unwrap();
    prepared_store.write_transition(&prepared).unwrap();
    std::fs::remove_dir_all(&prepared.after.canonical_vault_path).unwrap();
    assert!(prepared_store.recover().is_err());
    assert!(prepared_store.transition_exists_for_test().unwrap());

    let (_temp, committed_store, mut committed) = fixture();
    let old_key = committed.before.openrouter_key_version.as_deref().unwrap();
    let new_key = committed.after.openrouter_key_version.as_deref().unwrap();
    committed_store
        .seed_secret_for_test(old_key, "old-secret")
        .unwrap();
    committed_store
        .seed_secret_for_test(new_key, "new-secret")
        .unwrap();
    committed_store
        .write_authority_for_test(&committed.before)
        .unwrap();
    committed.decision = RootTransitionDecision::Committed;
    committed_store.write_transition(&committed).unwrap();
    std::fs::remove_dir_all(&committed.before.canonical_vault_path).unwrap();
    assert!(committed_store.recover().is_err());
    assert!(committed_store.transition_exists_for_test().unwrap());
}

#[test]
fn committed_recovery_rolls_forward_and_second_restart_is_zero_work() {
    let (_temp, store, mut transition) = fixture();
    let old_key = transition.before.openrouter_key_version.clone().unwrap();
    let new_key = transition.after.openrouter_key_version.clone().unwrap();
    store.seed_secret_for_test(&old_key, "old-secret").unwrap();
    store.seed_secret_for_test(&new_key, "new-secret").unwrap();
    store.write_authority_for_test(&transition.before).unwrap();
    transition.decision = RootTransitionDecision::Committed;
    store.write_transition(&transition).unwrap();
    store
        .write_settings(&transition.after.nonsecret_settings)
        .unwrap();

    assert_eq!(store.recover().unwrap(), RecoveryWork::RolledForward);
    assert_eq!(store.read_authority_for_test().unwrap(), transition.after);
    assert!(store.read_secret_for_test(&old_key).unwrap().is_none());
    assert_eq!(store.recover().unwrap(), RecoveryWork::None);
}

#[test]
fn unexpected_third_settings_state_preserves_every_byte_and_wal() {
    let (_temp, store, transition) = fixture();
    let old_key = transition.before.openrouter_key_version.as_deref().unwrap();
    let new_key = transition.after.openrouter_key_version.as_deref().unwrap();
    store.seed_secret_for_test(old_key, "old-secret").unwrap();
    store.seed_secret_for_test(new_key, "new-secret").unwrap();
    store.write_authority_for_test(&transition.before).unwrap();
    store.write_transition(&transition).unwrap();
    let third = UserSettings {
        theme: "third".into(),
        ..UserSettings::default()
    };
    store
        .write_settings(&NonsecretSettingsV1::from_settings(third))
        .unwrap();
    let before = store.read_settings_bytes_for_test().unwrap();
    assert!(store.recover().is_err());
    assert_eq!(store.read_settings_bytes_for_test().unwrap(), before);
    assert!(store.transition_exists_for_test().unwrap());
}

#[test]
fn strict_and_bounded_transition_read_fails_closed() {
    let (_temp, store, transition) = fixture();
    let mut value = serde_json::to_value(&transition).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("unknown".into(), true.into());
    store
        .write_raw_transition_for_test(&serde_json::to_vec(&value).unwrap())
        .unwrap();
    assert!(store.recover().is_err());
    store
        .write_raw_transition_for_test(&vec![b'x'; ROOT_TRANSITION_LIMIT + 1])
        .unwrap();
    assert!(store.recover().is_err());
}

#[test]
fn prepared_recovery_restarts_after_every_rollback_durable_phase() {
    for point in [
        RootTransitionFaultPoint::AfterRollbackLease,
        RootTransitionFaultPoint::AfterRollbackSettings,
        RootTransitionFaultPoint::AfterRollbackKeyRef,
        RootTransitionFaultPoint::AfterRollbackSecretDelete,
        RootTransitionFaultPoint::AfterRecoveryWalDelete,
    ] {
        let (_temp, store, transition) = fixture();
        let old_key = transition.before.openrouter_key_version.as_deref().unwrap();
        let new_key = transition.after.openrouter_key_version.as_deref().unwrap();
        store.seed_secret_for_test(old_key, "old-secret").unwrap();
        store.seed_secret_for_test(new_key, "new-secret").unwrap();
        store.write_authority_for_test(&transition.before).unwrap();
        store.write_transition(&transition).unwrap();
        store.write_lease(&transition.after.lease).unwrap();
        store
            .write_settings(&transition.after.nonsecret_settings)
            .unwrap();
        store
            .write_key_ref(transition.after.openrouter_key_version.as_deref())
            .unwrap();
        store.fail_once_at(point);
        assert!(
            store.recover().is_err(),
            "fault {point:?} must interrupt recovery"
        );
        let resumed = store.recover().unwrap();
        if point == RootTransitionFaultPoint::AfterRecoveryWalDelete {
            assert_eq!(resumed, RecoveryWork::None);
        } else {
            assert_eq!(resumed, RecoveryWork::RolledBack);
        }
        assert_eq!(store.recover().unwrap(), RecoveryWork::None);
    }
}

#[test]
fn committed_recovery_restarts_after_every_rollforward_durable_phase() {
    for point in [
        RootTransitionFaultPoint::AfterLease,
        RootTransitionFaultPoint::AfterSettings,
        RootTransitionFaultPoint::AfterKeyRef,
        RootTransitionFaultPoint::AfterOldKeyDelete,
        RootTransitionFaultPoint::AfterRecoveryWalDelete,
    ] {
        let (_temp, store, mut transition) = fixture();
        let old_key = transition.before.openrouter_key_version.as_deref().unwrap();
        let new_key = transition.after.openrouter_key_version.as_deref().unwrap();
        store.seed_secret_for_test(old_key, "old-secret").unwrap();
        store.seed_secret_for_test(new_key, "new-secret").unwrap();
        store.write_authority_for_test(&transition.before).unwrap();
        transition.decision = RootTransitionDecision::Committed;
        store.write_transition(&transition).unwrap();
        store.fail_once_at(point);
        assert!(
            store.recover().is_err(),
            "fault {point:?} must interrupt recovery"
        );
        let resumed = store.recover().unwrap();
        if point == RootTransitionFaultPoint::AfterRecoveryWalDelete {
            assert_eq!(resumed, RecoveryWork::None);
        } else {
            assert_eq!(resumed, RecoveryWork::RolledForward);
        }
        assert_eq!(store.recover().unwrap(), RecoveryWork::None);
    }
}

#[test]
fn prepared_wal_is_no_clobber_and_commit_is_same_transaction_cas() {
    let (_temp, store, first) = fixture();
    let (_second_temp, _second_store, second) = fixture();
    store.prepare_transition(&first).unwrap();
    let first_bytes = store.read_transition_bytes_for_test().unwrap();
    assert!(store.prepare_transition(&second).is_err());
    assert_eq!(store.read_transition_bytes_for_test().unwrap(), first_bytes);

    let wrong = second;
    assert!(matches!(
        store.mark_committed(&wrong),
        MarkCommittedResult::Uncertain(_)
    ));
    assert_eq!(store.read_transition_bytes_for_test().unwrap(), first_bytes);
    let exact = first;
    let committed = match store.mark_committed(&exact) {
        MarkCommittedResult::Committed(committed) => committed,
        other => panic!("expected committed WAL, got {other:?}"),
    };
    assert_eq!(committed.decision, RootTransitionDecision::Committed);
    let committed_bytes = store.read_transition_bytes_for_test().unwrap();
    assert!(matches!(
        store.mark_committed(&committed),
        MarkCommittedResult::Uncertain(_)
    ));
    assert_eq!(
        store.read_transition_bytes_for_test().unwrap(),
        committed_bytes
    );
}

#[test]
fn commit_result_distinguishes_prepared_committed_and_uncertain_durability() {
    let (_temp, store, transition) = fixture();
    store.prepare_transition(&transition).unwrap();
    store.fail_once_at(RootTransitionFaultPoint::BeforeCommitWalWrite);
    assert!(matches!(
        store.mark_committed(&transition),
        MarkCommittedResult::DefinitelyPrepared(_)
    ));
    assert_eq!(store.read_transition().unwrap(), Some(transition.clone()));

    store.fail_once_at(RootTransitionFaultPoint::AfterCommitWalWrite);
    let committed = match store.mark_committed(&transition) {
        MarkCommittedResult::Committed(committed) => committed,
        other => panic!("durable committed WAL must win after an uncertain write: {other:?}"),
    };
    assert_eq!(committed.decision, RootTransitionDecision::Committed);

    let (_other_temp, other_store, other) = fixture();
    other_store.prepare_transition(&other).unwrap();
    let mut wrong = other.clone();
    wrong.transaction_id = Uuid::new_v4().to_string();
    assert!(matches!(
        other_store.mark_committed(&wrong),
        MarkCommittedResult::Uncertain(_)
    ));
    assert_eq!(other_store.read_transition().unwrap(), Some(other));
}

#[test]
fn prepared_rollback_helpers_require_the_exact_durable_transaction() {
    let (_temp, store, transition) = fixture();
    let old_key = transition.before.openrouter_key_version.as_deref().unwrap();
    let new_key = transition.after.openrouter_key_version.as_deref().unwrap();
    store.seed_secret_for_test(old_key, "old-secret").unwrap();
    store.seed_secret_for_test(new_key, "new-secret").unwrap();
    store.write_authority_for_test(&transition.before).unwrap();
    store.prepare_transition(&transition).unwrap();
    let authority_before = store.read_authority_for_test().unwrap();

    let mut wrong = transition.clone();
    wrong.transaction_id = Uuid::new_v4().to_string();
    assert!(store.restore_prepared_authorities(&wrong).is_err());
    assert!(store.finalize_prepared_rollback(&wrong).is_err());
    assert_eq!(store.read_authority_for_test().unwrap(), authority_before);
    assert!(store.transition_exists_for_test().unwrap());
    assert!(store.read_secret_for_test(new_key).unwrap().is_some());
}

#[test]
fn legacy_migration_uses_one_discoverable_version_and_sanitized_settings() {
    let (_temp, store, _transition) = fixture();

    let (version, secret) = store
        .migrate_legacy_secret_authority("new-keychain")
        .unwrap();
    assert_eq!(version, LEGACY_MIGRATION_KEY_VERSION);
    assert_eq!(secret, "new-keychain");
    assert_eq!(
        store.active_key_version().unwrap().as_deref(),
        Some(version.as_str())
    );
    assert_eq!(
        store.resolve_secret(Some(&version)).unwrap().as_deref(),
        Some("new-keychain")
    );
    assert!(
        !String::from_utf8(store.read_settings_bytes_for_test().unwrap())
            .unwrap()
            .contains("stale-plaintext")
    );

    let repeated = store
        .migrate_legacy_secret_authority("new-keychain")
        .unwrap();
    assert_eq!(repeated, (version, "new-keychain".into()));
}
