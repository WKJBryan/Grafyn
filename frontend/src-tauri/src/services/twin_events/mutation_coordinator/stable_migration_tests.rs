use super::super::MutationIdentityProvider;
use super::*;
use crate::services::sync::identity::load_or_create_vault_identity;
use crate::services::twin_events::acquire_shared_coordinator_process_lock;
use tempfile::TempDir;

fn tree_snapshot(
    root: &std::path::Path,
) -> std::collections::BTreeMap<std::path::PathBuf, Option<Vec<u8>>> {
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

struct Fixture {
    _parent: TempDir,
    data: std::path::PathBuf,
    vault: std::path::PathBuf,
    root: AnchoredRoot,
    identity: VaultIdentity,
    legacy_scope: ContentDigest,
    legacy_lease: ActiveMarkdownRootLeaseV1,
    writer: DeviceId,
    journal: LocalMutationJournal,
    lock: CoordinatorProcessLock,
}

impl Fixture {
    fn new() -> Self {
        let parent = tempfile::tempdir().unwrap();
        let data = parent.path().join("data");
        let vault = parent.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(data.join("canvas")).unwrap();
        let identity = load_or_create_vault_identity(&vault).unwrap();
        let legacy_scope =
            crate::services::sync::identity::legacy_path_scope_for_migration(&vault).unwrap();
        let legacy_lease = ActiveMarkdownRootLeaseV1::new(legacy_scope.clone());
        let journal = LocalMutationJournal::initialize(&data).unwrap();
        std::fs::create_dir_all(data.join(LEGACY_EVENT_RECORDS)).unwrap();
        std::fs::create_dir_all(data.join(LEGACY_EVENT_QUARANTINE)).unwrap();
        std::fs::create_dir_all(data.join(LEGACY_EVENT_STAGING)).unwrap();
        let root = AnchoredRoot::open(&data).unwrap();
        super::super::write_active_root_lease(&root, &legacy_lease).unwrap();
        std::fs::create_dir_all(data.join("twin").join(legacy_scope.as_str())).unwrap();
        std::fs::write(
            data.join("twin")
                .join(legacy_scope.as_str())
                .join("state.json"),
            b"legacy twin",
        )
        .unwrap();
        let derived = data.join("vault_derived/v1").join(legacy_scope.as_str());
        std::fs::create_dir_all(derived.join("vault_migration/optimizer/pending-publications-v1"))
            .unwrap();
        std::fs::create_dir_all(derived.join("vault_migration/optimizer/pending-rollbacks-v1"))
            .unwrap();
        std::fs::create_dir_all(derived.join("vault_migration/runs")).unwrap();
        std::fs::write(data.join(LEGACY_EVENT_RECORDS).join("event.json"), b"event").unwrap();
        std::fs::write(data.join("canvas/session.json"), b"canvas").unwrap();
        let writer = DeviceId::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
        Self {
            _parent: parent,
            data,
            vault,
            root,
            identity,
            legacy_scope,
            legacy_lease,
            writer,
            journal,
            lock,
        }
    }

    fn migrate(
        &self,
        lease: &ActiveMarkdownRootLeaseV1,
        fault: Option<StableMigrationFault>,
    ) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
        migrate_legacy_to_stable_locked_inner(
            &self.data,
            &self.root,
            &self.vault,
            &self.identity,
            &self.legacy_scope,
            lease,
            &self.writer,
            &self.journal,
            &self.lock,
            None,
            fault,
        )
    }
}

fn assert_migration_blocker_preserves_legacy_state(
    fixture: &Fixture,
    reason: &str,
    blocker: &std::path::Path,
) {
    let lease_before = super::super::load_active_root_lease(&fixture.root).unwrap();
    let legacy_twin = fixture
        .data
        .join("twin")
        .join(fixture.legacy_scope.as_str())
        .join("state.json");
    let legacy_event = fixture.data.join(LEGACY_EVENT_RECORDS).join("event.json");
    let legacy_canvas = fixture.data.join("canvas/session.json");
    let legacy_derived = fixture
        .data
        .join("vault_derived/v1")
        .join(fixture.legacy_scope.as_str());
    let blocker_before = std::fs::read(blocker).unwrap();
    let source_bytes_before = [
        std::fs::read(&legacy_twin).unwrap(),
        std::fs::read(&legacy_event).unwrap(),
        std::fs::read(&legacy_canvas).unwrap(),
    ];

    let error = fixture.migrate(&fixture.legacy_lease, None).unwrap_err();

    assert!(error.to_string().contains(reason), "{error}");
    assert!(read_marker(&fixture.root, &fixture.identity.root_scope)
        .unwrap()
        .is_none());
    assert_eq!(
        super::super::load_active_root_lease(&fixture.root).unwrap(),
        lease_before
    );
    assert_eq!(std::fs::read(blocker).unwrap(), blocker_before);
    assert_eq!(
        [
            std::fs::read(&legacy_twin).unwrap(),
            std::fs::read(&legacy_event).unwrap(),
            std::fs::read(&legacy_canvas).unwrap(),
        ],
        source_bytes_before
    );
    assert!(legacy_derived.is_dir());
}

#[test]
fn moves_every_legacy_component_and_publishes_the_stable_lease_last() {
    let fixture = Fixture::new();

    let stable = fixture.migrate(&fixture.legacy_lease, None).unwrap();

    assert!(stable.is_stable());
    assert_eq!(stable.root_scope, fixture.identity.root_scope);
    assert_eq!(
        super::super::load_active_root_lease(&fixture.root).unwrap(),
        stable
    );
    assert!(fixture
        .data
        .join("twin")
        .join(fixture.identity.root_scope.as_str())
        .join("state.json")
        .exists());
    assert!(fixture
        .data
        .join("vault_derived/v1")
        .join(fixture.identity.root_scope.as_str())
        .exists());
    assert!(fixture
        .data
        .join("twin/events/vaults/v1")
        .join(fixture.identity.root_scope.as_str())
        .join("records/v1/event.json")
        .exists());
    assert!(fixture
        .data
        .join("canvas/v1")
        .join(fixture.identity.root_scope.as_str())
        .join("session.json")
        .exists());
    let marker = read_marker(&fixture.root, &fixture.identity.root_scope)
        .unwrap()
        .unwrap();
    assert_eq!(marker.state, StableMigrationStateV1::Committed);
    assert!(marker.lease_published);
}

#[test]
fn migrates_unscoped_assignment_files_into_the_vault_scoped_history() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.data.join("twin/legacy-assignment-v1.json"),
        b"twin assignment",
    )
    .unwrap();
    std::fs::write(
        fixture.data.join("vault_derived/legacy-assignment-v1.json"),
        b"derived assignment",
    )
    .unwrap();

    fixture.migrate(&fixture.legacy_lease, None).unwrap();

    let history = fixture
        .data
        .join("twin/stable-vault-migrations/v1")
        .join(fixture.identity.root_scope.as_str())
        .join("history");
    assert!(!fixture.data.join("twin/legacy-assignment-v1.json").exists());
    assert!(!fixture
        .data
        .join("vault_derived/legacy-assignment-v1.json")
        .exists());
    assert_eq!(
        std::fs::read(history.join("twin-legacy-assignment-v1.json")).unwrap(),
        b"twin assignment"
    );
    assert_eq!(
        std::fs::read(history.join("vault-derived-legacy-assignment-v1.json")).unwrap(),
        b"derived assignment"
    );
}

#[test]
fn fresh_stable_install_rejects_markerless_history_for_its_own_scope() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("data");
    let vault = parent.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(data.join("canvas")).unwrap();
    let identity = load_or_create_vault_identity(&vault).unwrap();
    let legacy_scope =
        crate::services::sync::identity::legacy_path_scope_for_migration(&vault).unwrap();
    let lease = ActiveMarkdownRootLeaseV1::new_stable(identity.root_scope.clone());
    let journal = LocalMutationJournal::initialize(&data).unwrap();
    std::fs::create_dir_all(data.join(LEGACY_EVENT_RECORDS)).unwrap();
    std::fs::create_dir_all(data.join(LEGACY_EVENT_QUARANTINE)).unwrap();
    std::fs::create_dir_all(data.join(LEGACY_EVENT_STAGING)).unwrap();
    std::fs::create_dir_all(data.join("twin").join(identity.root_scope.as_str())).unwrap();
    let history = data
        .join("twin/stable-vault-migrations/v1")
        .join(identity.root_scope.as_str())
        .join("history");
    std::fs::create_dir_all(&history).unwrap();
    std::fs::write(history.join("foreign.json"), b"unowned history").unwrap();
    let root = AnchoredRoot::open(&data).unwrap();
    super::super::write_active_root_lease(&root, &lease).unwrap();
    let writer = DeviceId::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let lock = acquire_shared_coordinator_process_lock(&data).unwrap();

    let error = migrate_legacy_to_stable_locked(
        &data,
        &root,
        &vault,
        &identity,
        &legacy_scope,
        &lease,
        &writer,
        &journal,
        &lock,
    )
    .unwrap_err();

    assert!(error.to_string().contains("markerless"));
    assert_eq!(
        std::fs::read(history.join("foreign.json")).unwrap(),
        b"unowned history"
    );
}

#[test]
fn marker_publication_temp_scaffold_is_reconciled_before_migration_resumes() {
    let fixture = Fixture::new();
    let scope_root = fixture
        .data
        .join(migration_scope_root(&fixture.identity.root_scope));
    std::fs::create_dir_all(&scope_root).unwrap();
    let temporary = scope_root.join(".123e4567-e89b-42d3-a456-426614174999.tmp");
    std::fs::write(&temporary, b"interrupted marker publication").unwrap();

    let stable = fixture.migrate(&fixture.legacy_lease, None).unwrap();

    assert!(stable.is_stable());
    assert!(!temporary.exists());
    assert_eq!(
        read_marker(&fixture.root, &fixture.identity.root_scope)
            .unwrap()
            .unwrap()
            .state,
        StableMigrationStateV1::Committed
    );
}

#[test]
fn fresh_schema_two_lease_needs_no_migration_marker() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("data");
    let vault = parent.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(data.join("canvas")).unwrap();
    let identity = load_or_create_vault_identity(&vault).unwrap();
    let legacy_scope =
        crate::services::sync::identity::legacy_path_scope_for_migration(&vault).unwrap();
    let lease = ActiveMarkdownRootLeaseV1::new_stable(identity.root_scope.clone());
    let journal = LocalMutationJournal::initialize(&data).unwrap();
    std::fs::create_dir_all(data.join(LEGACY_EVENT_RECORDS)).unwrap();
    std::fs::create_dir_all(data.join(LEGACY_EVENT_QUARANTINE)).unwrap();
    std::fs::create_dir_all(data.join(LEGACY_EVENT_STAGING)).unwrap();
    std::fs::create_dir_all(data.join("twin").join(identity.root_scope.as_str())).unwrap();
    let root = AnchoredRoot::open(&data).unwrap();
    super::super::write_active_root_lease(&root, &lease).unwrap();
    let writer = DeviceId::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let lock = acquire_shared_coordinator_process_lock(&data).unwrap();

    let returned = migrate_legacy_to_stable_locked(
        &data,
        &root,
        &vault,
        &identity,
        &legacy_scope,
        &lease,
        &writer,
        &journal,
        &lock,
    )
    .unwrap();

    assert_eq!(returned, lease);
    assert!(read_marker(&root, &identity.root_scope).unwrap().is_none());
}

#[test]
fn retained_commit_receipt_blocks_stable_migration_before_marker_or_lease_moves() {
    let fixture = Fixture::new();
    let receipt = crate::services::twin_events::MutationCommitReceiptV1 {
        schema_version: 1,
        mutation_id: ContentDigest::parse("a".repeat(64)).unwrap(),
        root_scope: fixture.legacy_scope.clone(),
        lease_epoch_uuid: fixture.legacy_lease.epoch_uuid.clone(),
        authority_generation: 1,
    };
    let receipt_path = fixture
        .data
        .join("twin/mutations/receipts/v1")
        .join(format!("{}.json", receipt.mutation_id.as_str()));
    std::fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();

    assert_migration_blocker_preserves_legacy_state(
        &fixture,
        "empty mutation-owner journal",
        &receipt_path,
    );
}

#[test]
fn pending_optimizer_publication_blocks_stable_migration_before_marker_or_lease_moves() {
    let fixture = Fixture::new();
    let publication = fixture
        .data
        .join("vault_derived/v1")
        .join(fixture.legacy_scope.as_str())
        .join("vault_migration/optimizer/pending-publications-v1/publication.json");
    std::fs::write(&publication, b"pending publication").unwrap();

    assert_migration_blocker_preserves_legacy_state(
        &fixture,
        "pending optimizer work",
        &publication,
    );
}

#[test]
fn pending_optimizer_rollback_blocks_stable_migration_before_marker_or_lease_moves() {
    let fixture = Fixture::new();
    let rollback = fixture
        .data
        .join("vault_derived/v1")
        .join(fixture.legacy_scope.as_str())
        .join("vault_migration/optimizer/pending-rollbacks-v1/rollback.json");
    std::fs::write(&rollback, b"pending rollback").unwrap();

    assert_migration_blocker_preserves_legacy_state(&fixture, "pending optimizer work", &rollback);
}

#[test]
fn active_markdown_migration_blocks_stable_migration_before_marker_or_lease_moves() {
    let fixture = Fixture::new();
    let manifest = fixture
        .data
        .join("vault_derived/v1")
        .join(fixture.legacy_scope.as_str())
        .join("vault_migration/runs/123e4567-e89b-42d3-a456-426614174001/manifest.json");
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    std::fs::write(
        &manifest,
        br#"{"status":"applying","active_step":{"operation_index":0}}"#,
    )
    .unwrap();

    assert_migration_blocker_preserves_legacy_state(
        &fixture,
        "active Markdown migration",
        &manifest,
    );
}

#[test]
fn committed_assignment_allows_new_stable_data_but_rejects_new_legacy_data() {
    let fixture = Fixture::new();
    let stable = fixture.migrate(&fixture.legacy_lease, None).unwrap();
    let stable_events = fixture
        .data
        .join("twin/events/vaults/v1")
        .join(fixture.identity.root_scope.as_str())
        .join("records/v1/new.json");
    std::fs::write(&stable_events, b"new stable event").unwrap();
    let stable_canvas = fixture
        .data
        .join("canvas/v1")
        .join(fixture.identity.root_scope.as_str())
        .join("new.json");
    std::fs::write(&stable_canvas, b"new stable canvas").unwrap();

    assert_eq!(fixture.migrate(&stable, None).unwrap(), stable);

    std::fs::create_dir_all(fixture.data.join(LEGACY_EVENT_RECORDS)).unwrap();
    std::fs::write(
        fixture.data.join(LEGACY_EVENT_RECORDS).join("late.json"),
        b"late legacy event",
    )
    .unwrap();
    let error = fixture.migrate(&stable, None).unwrap_err();
    assert!(error.to_string().contains("refuses to merge"));
    assert_eq!(std::fs::read(stable_events).unwrap(), b"new stable event");
    assert_eq!(std::fs::read(stable_canvas).unwrap(), b"new stable canvas");
}

#[test]
fn committed_assignment_rejects_a_late_unscoped_assignment_file() {
    let fixture = Fixture::new();
    let stable = fixture.migrate(&fixture.legacy_lease, None).unwrap();
    let late = fixture.data.join(LEGACY_TWIN_ASSIGNMENT);
    std::fs::write(&late, b"late assignment").unwrap();

    let error = fixture.migrate(&stable, None).unwrap_err();

    assert!(error.to_string().contains("assignment"));
    assert_eq!(std::fs::read(late).unwrap(), b"late assignment");
}

#[test]
fn committed_assignment_follows_the_same_vault_uuid_after_a_path_move() {
    let fixture = Fixture::new();
    let stable = fixture.migrate(&fixture.legacy_lease, None).unwrap();
    let moved_vault = fixture._parent.path().join("moved-vault");
    std::fs::rename(&fixture.vault, &moved_vault).unwrap();
    let moved_legacy_scope =
        crate::services::sync::identity::legacy_path_scope_for_migration(&moved_vault).unwrap();

    let returned = migrate_legacy_to_stable_locked(
        &fixture.data,
        &fixture.root,
        &moved_vault,
        &fixture.identity,
        &moved_legacy_scope,
        &stable,
        &fixture.writer,
        &fixture.journal,
        &fixture.lock,
    )
    .unwrap();

    assert_eq!(returned, stable);
    assert_eq!(
        read_marker(&fixture.root, &fixture.identity.root_scope)
            .unwrap()
            .unwrap()
            .legacy_scope,
        fixture.legacy_scope
    );
}

#[test]
fn committed_marker_accepts_a_fresh_same_scope_lease_epoch() {
    let fixture = Fixture::new();
    let original = fixture.migrate(&fixture.legacy_lease, None).unwrap();
    let replacement = ActiveMarkdownRootLeaseV1::new_stable(original.root_scope.clone());
    assert_ne!(replacement.epoch_uuid, original.epoch_uuid);
    super::super::write_active_root_lease(&fixture.root, &replacement).unwrap();

    let reopened = fixture.migrate(&replacement, None).unwrap();

    assert_eq!(reopened, replacement);
    assert_eq!(
        super::super::load_active_root_lease(&fixture.root).unwrap(),
        replacement
    );
}

#[test]
fn refuses_source_destination_collisions_without_merging() {
    let fixture = Fixture::new();
    let destination = fixture
        .data
        .join("twin")
        .join(fixture.identity.root_scope.as_str());
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("foreign.json"), b"foreign").unwrap();

    let error = fixture.migrate(&fixture.legacy_lease, None).unwrap_err();

    assert!(error.to_string().contains("destination"));
    assert!(fixture
        .data
        .join("twin")
        .join(fixture.legacy_scope.as_str())
        .exists());
    assert_eq!(
        std::fs::read(destination.join("foreign.json")).unwrap(),
        b"foreign"
    );
    assert!(read_marker(&fixture.root, &fixture.identity.root_scope)
        .unwrap()
        .is_none());
    assert_eq!(
        super::super::load_active_root_lease(&fixture.root).unwrap(),
        fixture.legacy_lease
    );
}

#[test]
fn destination_created_at_the_rename_boundary_is_never_clobbered() {
    let fixture = Fixture::new();
    let error = fixture
        .migrate(
            &fixture.legacy_lease,
            Some(StableMigrationFault::DestinationCollisionAtFirstRename),
        )
        .unwrap_err();

    assert!(error.to_string().contains("destination"));
    let marker = read_marker(&fixture.root, &fixture.identity.root_scope)
        .unwrap()
        .unwrap();
    let first = marker.components.first().unwrap();
    assert!(fixture.data.join(&first.source).exists());
    let foreign = match first.kind {
        StableMigrationComponentKindV1::Directory => {
            fixture.data.join(&first.destination).join("foreign.json")
        }
        StableMigrationComponentKindV1::RegularFile => fixture.data.join(&first.destination),
    };
    assert_eq!(std::fs::read(foreign).unwrap(), b"foreign");
    assert!(marker.moved_sources.is_empty());
}

#[test]
fn resumes_when_rename_won_but_progress_write_was_lost() {
    let fixture = Fixture::new();
    let error = fixture
        .migrate(
            &fixture.legacy_lease,
            Some(StableMigrationFault::AfterFirstRenameBeforeProgress),
        )
        .unwrap_err();
    assert!(error.to_string().contains("rename-boundary"));

    let stable = fixture.migrate(&fixture.legacy_lease, None).unwrap();

    assert!(stable.is_stable());
    assert_eq!(
        read_marker(&fixture.root, &fixture.identity.root_scope)
            .unwrap()
            .unwrap()
            .state,
        StableMigrationStateV1::Committed
    );
}

#[test]
fn prepared_marker_blocks_fresh_entries_until_recovery_commits_it() {
    let fixture = Fixture::new();
    let error = fixture
        .migrate(
            &fixture.legacy_lease,
            Some(StableMigrationFault::AfterFirstRenameBeforeProgress),
        )
        .unwrap_err();
    assert!(error.to_string().contains("rename-boundary"));

    assert_eq!(
        inspect_prepared_migration_locked(&fixture.root, &fixture.lock).unwrap(),
        Some(fixture.identity.root_scope.clone())
    );
    assert!(reject_prepared_migration_locked(&fixture.root, &fixture.lock).is_err());

    let stable = fixture.migrate(&fixture.legacy_lease, None).unwrap();

    assert!(stable.is_stable());
    assert_eq!(
        inspect_prepared_migration_locked(&fixture.root, &fixture.lock).unwrap(),
        None
    );
    reject_prepared_migration_locked(&fixture.root, &fixture.lock).unwrap();
}

#[test]
fn live_legacy_entry_rejects_prepared_marker_and_stable_constructor_resumes_it() {
    let mut fixture = Fixture::new();
    std::fs::remove_file(fixture.data.join(LEGACY_EVENT_RECORDS).join("event.json")).unwrap();
    let persisted = std::sync::Arc::new(
        super::super::PersistedMutationIdentityProvider::load_or_create(&fixture.data).unwrap(),
    );
    fixture.writer = persisted.device_id();
    let legacy_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
        &fixture.data,
    ));
    let error = fixture
        .migrate(
            &fixture.legacy_lease,
            Some(StableMigrationFault::AfterFirstRenameBeforeProgress),
        )
        .unwrap_err();
    assert!(error.to_string().contains("rename-boundary"));
    let Fixture {
        _parent,
        data,
        vault,
        root: _,
        identity,
        legacy_scope: _,
        legacy_lease: _,
        writer: _,
        journal: _,
        lock,
    } = fixture;
    lock.unlock().unwrap();
    let data_before = tree_snapshot(&data);
    let vault_before = tree_snapshot(&vault);
    let legacy_finalizer = super::super::StoreEventGroupFinalizer::new(legacy_store, persisted);

    let error = match legacy_finalizer.acquire_coordinator_lock() {
        Ok(_) => panic!("a live legacy entry must stop at the prepared migration marker"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("prepared stable migration"));
    assert_eq!(tree_snapshot(&data), data_before);
    assert_eq!(tree_snapshot(&vault), vault_before);

    let coordinator = super::super::MutationCoordinator::new_stable(
        &data,
        &vault,
        std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data)),
        std::sync::Arc::new(super::super::NoopMutationLifecycle),
    )
    .unwrap();

    assert_eq!(
        coordinator.current_root_epoch().unwrap().root_scope,
        identity.root_scope
    );
    assert!(coordinator.current_root_epoch().unwrap().is_stable());
}

#[test]
fn resumes_an_assignment_file_rename_whose_progress_write_was_lost() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.data.join(LEGACY_TWIN_ASSIGNMENT),
        b"twin assignment",
    )
    .unwrap();
    let error = fixture
        .migrate(
            &fixture.legacy_lease,
            Some(StableMigrationFault::AfterTwinAssignmentRenameBeforeProgress),
        )
        .unwrap_err();
    assert!(error.to_string().contains("assignment-rename-boundary"));

    let stable = fixture.migrate(&fixture.legacy_lease, None).unwrap();
    let destination = migration_history_root(&stable.root_scope);
    assert_eq!(
        fixture
            .root
            .read_bounded(
                &format!("{destination}/twin-legacy-assignment-v1.json"),
                4096
            )
            .unwrap()
            .unwrap(),
        b"twin assignment"
    );
    assert!(fixture
        .root
        .read_bounded(LEGACY_TWIN_ASSIGNMENT, 4096)
        .unwrap()
        .is_none());
}

#[test]
fn resumes_when_stable_lease_won_but_marker_progress_was_lost() {
    let fixture = Fixture::new();
    let error = fixture
        .migrate(
            &fixture.legacy_lease,
            Some(StableMigrationFault::AfterLeaseBeforeProgress),
        )
        .unwrap_err();
    assert!(error.to_string().contains("lease-boundary"));
    let durable = super::super::load_active_root_lease(&fixture.root).unwrap();
    assert!(durable.is_stable());
    std::fs::create_dir_all(fixture.data.join(LEGACY_EVENT_RECORDS)).unwrap();
    std::fs::create_dir_all(fixture.data.join(LEGACY_EVENT_QUARANTINE)).unwrap();
    std::fs::create_dir_all(fixture.data.join(LEGACY_EVENT_STAGING)).unwrap();

    let stable = fixture.migrate(&durable, None).unwrap();

    assert_eq!(stable, durable);
    assert_eq!(
        read_marker(&fixture.root, &fixture.identity.root_scope)
            .unwrap()
            .unwrap()
            .state,
        StableMigrationStateV1::Committed
    );
}

#[test]
fn stable_constructor_rejects_a_descriptor_removed_after_preflight_without_writing() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("data");
    let vault = parent.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir_all(data.join("twin/events")).unwrap();
    let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    lock.unlock().unwrap();
    let identity = load_or_create_vault_identity(&vault).unwrap();
    let root = AnchoredRoot::open(&data).unwrap();
    super::super::write_active_root_lease(
        &root,
        &ActiveMarkdownRootLeaseV1::new_stable(identity.root_scope),
    )
    .unwrap();
    super::super::PersistedMutationIdentityProvider::load_or_create(&data).unwrap();

    crate::services::sync::identity::load_vault_identity(&vault).unwrap();
    std::fs::remove_file(vault.join("_grafyn/vault.json")).unwrap();
    let data_before = tree_snapshot(&data);
    let vault_before = tree_snapshot(&vault);
    let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));

    let error = match super::super::MutationCoordinator::new_stable(
        &data,
        &vault,
        events,
        std::sync::Arc::new(super::super::NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("a stable lease must not recreate its missing vault descriptor"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("vault-descriptor-missing"));
    assert_eq!(tree_snapshot(&data), data_before);
    assert_eq!(tree_snapshot(&vault), vault_before);
    assert!(!vault.join("_grafyn/vault.json").exists());
}

#[test]
fn stable_constructor_rejects_noncanonical_descriptor_bytes_without_replacement() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("data");
    let vault = parent.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir_all(data.join("twin/events")).unwrap();
    let descriptor_path = vault.join("_grafyn/vault.json");
    let noncanonical = br#"{"schema_version":1,"vault_id":"123E4567-E89B-42D3-A456-426614174000"}"#;
    std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
    std::fs::write(&descriptor_path, noncanonical).unwrap();
    let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    lock.unlock().unwrap();
    let data_before = tree_snapshot(&data);
    let vault_before = tree_snapshot(&vault);

    let error = match super::super::MutationCoordinator::new_stable(
        &data,
        &vault,
        std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data)),
        std::sync::Arc::new(super::super::NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("a noncanonical descriptor must never be replaced during stable startup"),
        Err(error) => error,
    };

    assert!(error.to_string().contains("invalid vault descriptor"));
    assert_eq!(std::fs::read(&descriptor_path).unwrap(), noncanonical);
    assert_eq!(tree_snapshot(&data), data_before);
    assert_eq!(tree_snapshot(&vault), vault_before);
}

#[test]
fn stable_constructor_rejects_a_mismatched_schema1_lease_before_creating_a_descriptor() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("data");
    let active_vault = parent.path().join("active-vault");
    let configured_vault = parent.path().join("configured-vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&active_vault).unwrap();
    std::fs::create_dir(&configured_vault).unwrap();
    std::fs::create_dir_all(data.join("twin/events")).unwrap();
    let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    lock.unlock().unwrap();
    let root = AnchoredRoot::open(&data).unwrap();
    let active_scope = super::super::markdown_root_scope_for(&active_vault).unwrap();
    super::super::write_active_root_lease(&root, &ActiveMarkdownRootLeaseV1::new(active_scope))
        .unwrap();
    super::super::PersistedMutationIdentityProvider::load_or_create(&data).unwrap();
    let data_before = tree_snapshot(&data);
    let configured_vault_before = tree_snapshot(&configured_vault);
    let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));

    let error = match super::super::MutationCoordinator::new_stable(
        &data,
        &configured_vault,
        events,
        std::sync::Arc::new(super::super::NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("a schema-1 lease for another path must fail before descriptor creation"),
        Err(error) => error,
    };

    assert!(error
        .to_string()
        .contains("configured-markdown-root-is-not-active"));
    assert_eq!(tree_snapshot(&data), data_before);
    assert_eq!(tree_snapshot(&configured_vault), configured_vault_before);
    assert!(!configured_vault.join("_grafyn/vault.json").exists());
}

#[test]
fn stable_constructor_rejects_a_stale_root_after_peer_commit_without_writing() {
    let parent = tempfile::tempdir().unwrap();
    let data = parent.path().join("data");
    let vault_a = parent.path().join("vault-a");
    let vault_b = parent.path().join("vault-b");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault_a).unwrap();
    std::fs::create_dir(&vault_b).unwrap();
    std::fs::create_dir_all(data.join("twin/events")).unwrap();
    let identity_a = load_or_create_vault_identity(&vault_a).unwrap();
    let identity_b = load_or_create_vault_identity(&vault_b).unwrap();
    let root = AnchoredRoot::open(&data).unwrap();
    super::super::write_active_root_lease(
        &root,
        &ActiveMarkdownRootLeaseV1::new_stable(identity_a.root_scope),
    )
    .unwrap();
    super::super::PersistedMutationIdentityProvider::load_or_create(&data).unwrap();
    let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    lock.unlock().unwrap();

    crate::services::sync::identity::load_vault_identity(&vault_a).unwrap();
    let peer_lock = acquire_shared_coordinator_process_lock(&data).unwrap();
    super::super::write_active_root_lease(
        &root,
        &ActiveMarkdownRootLeaseV1::new_stable(identity_b.root_scope),
    )
    .unwrap();
    peer_lock.unlock().unwrap();
    assert!(!data.join("twin/events/root-transition-v1.json").exists());
    let data_before = tree_snapshot(&data);
    let vault_a_before = tree_snapshot(&vault_a);
    let vault_b_before = tree_snapshot(&vault_b);
    let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));

    let error = match super::super::MutationCoordinator::new_stable(
        &data,
        &vault_a,
        events,
        std::sync::Arc::new(super::super::NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("a stale configured root must not start after a peer commits another root"),
        Err(error) => error,
    };

    assert!(error
        .to_string()
        .contains("configured-markdown-root-is-not-active"));
    assert_eq!(tree_snapshot(&data), data_before);
    assert_eq!(tree_snapshot(&vault_a), vault_a_before);
    assert_eq!(tree_snapshot(&vault_b), vault_b_before);
}
