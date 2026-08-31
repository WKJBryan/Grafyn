use super::{ActiveMarkdownRootLeaseV1, CoordinatorProcessLock, MutationError};
use crate::models::twin_event::{ContentDigest, DeviceId};
use crate::services::sync::identity::VaultIdentity;
use crate::services::twin_events::{AnchoredEntryKind, AnchoredRoot, LocalMutationJournal};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use uuid::Uuid;

const MARKER_SCHEMA_VERSION: u16 = 1;
const MIGRATION_HISTORY_ROOT: &str = "twin/stable-vault-migrations/v1";
const LEGACY_GLOBAL_MARKER: &str = "twin/stable-vault-migration-v1.json";
const LEGACY_TWIN_ASSIGNMENT: &str = "twin/legacy-assignment-v1.json";
const LEGACY_DERIVED_ASSIGNMENT: &str = "vault_derived/legacy-assignment-v1.json";
const MARKER_LIMIT: usize = 8 * 1024 * 1024;
const MAX_COMPONENTS: usize = 8 * 1024;
const MAX_TREE_ENTRIES: usize = 200_000;
const MAX_TREE_DEPTH: usize = 32;
const MAX_EVENT_DIRECTORY_ENTRIES: usize = MAX_TREE_ENTRIES;
const MAX_CANVAS_ENTRIES: usize = MAX_COMPONENTS;
const MAX_MIGRATION_RUNS: usize = 4096;
const MAX_MARKDOWN_MANIFEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_UNLEASED_OWNER_ENTRIES: usize = 256;
const MAX_MIGRATION_SCOPES: usize = 4096;
const MAX_MARKER_SCAN_BYTES: usize = 64 * 1024 * 1024;

const LEGACY_EVENT_RECORDS: &str = "twin/events/v1";
const LEGACY_EVENT_QUARANTINE: &str = "twin/events/quarantine/v1";
const LEGACY_EVENT_STAGING: &str = "twin/events/staging/v1";
const LEGACY_DERIVED_COMPONENTS: [&str; 4] = [
    "search_index",
    "chunk_index",
    "link_discovery",
    "vault_migration",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum StableMigrationStateV1 {
    Prepared,
    Committed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum StableMigrationComponentKindV1 {
    Directory,
    RegularFile,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StableMigrationComponentV1 {
    source: String,
    destination: String,
    kind: StableMigrationComponentKindV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StableMigrationMarkerV1 {
    schema_version: u16,
    vault_id: String,
    stable_scope: ContentDigest,
    legacy_scope: ContentDigest,
    legacy_lease_epoch_uuid: String,
    writer_device_id: DeviceId,
    stable_lease: ActiveMarkdownRootLeaseV1,
    components: Vec<StableMigrationComponentV1>,
    moved_sources: Vec<String>,
    lease_published: bool,
    state: StableMigrationStateV1,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StableMigrationFault {
    DestinationCollisionAtFirstRename,
    AfterFirstRenameBeforeProgress,
    AfterTwinAssignmentRenameBeforeProgress,
    AfterLeaseBeforeProgress,
}

pub(super) fn marker_exists(
    root: &AnchoredRoot,
    stable_scope: &ContentDigest,
) -> Result<bool, MutationError> {
    reject_legacy_global_marker(root)?;
    let marker = read_marker(root, stable_scope)?;
    require_no_markerless_scope_history(root, stable_scope, marker.is_some())?;
    Ok(marker.is_some())
}

pub(super) fn inspect_prepared_migration_locked(
    root: &AnchoredRoot,
    process_lock: &CoordinatorProcessLock,
) -> Result<Option<ContentDigest>, MutationError> {
    if !process_lock.covers_data_path(root.canonical_path())? {
        return Err(MutationError::Invalid(
            "stable migration scan lock belongs to another data root".into(),
        ));
    }
    reject_legacy_global_marker(root)?;
    if !root.directory_exists(MIGRATION_HISTORY_ROOT)? {
        return Ok(None);
    }
    let mut remaining_bytes = MAX_MARKER_SCAN_BYTES;
    let mut prepared = None;
    for (name, kind) in bounded_entries(root, MIGRATION_HISTORY_ROOT, MAX_MIGRATION_SCOPES)? {
        if kind != AnchoredEntryKind::Directory {
            return Err(MutationError::Invalid(
                "stable migration scope entry is not a directory".into(),
            ));
        }
        let scope = ContentDigest::parse(name).map_err(MutationError::Invalid)?;
        let key = marker_key(&scope);
        let Some(bytes) = root.read_bounded(&key, MARKER_LIMIT.min(remaining_bytes))? else {
            require_no_markerless_scope_history(root, &scope, false)?;
            continue;
        };
        remaining_bytes = remaining_bytes.checked_sub(bytes.len()).ok_or_else(|| {
            MutationError::Invalid(format!(
                "stable migration marker scan exceeds {MAX_MARKER_SCAN_BYTES} bytes"
            ))
        })?;
        let marker: StableMigrationMarkerV1 = serde_json::from_slice(&bytes).map_err(|error| {
            MutationError::Invalid(format!("invalid stable migration marker: {error}"))
        })?;
        validate_marker_shape(&marker)?;
        if marker.stable_scope != scope {
            return Err(MutationError::Invalid(
                "stable migration marker scope does not match its directory".into(),
            ));
        }
        require_no_markerless_scope_history(root, &scope, true)?;
        if marker.state == StableMigrationStateV1::Prepared && prepared.replace(scope).is_some() {
            return Err(MutationError::RecoveryConflict(
                "multiple prepared stable migrations require recovery".into(),
            ));
        }
    }
    Ok(prepared)
}

pub(super) fn reject_prepared_migration_locked(
    root: &AnchoredRoot,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    if inspect_prepared_migration_locked(root, process_lock)?.is_some() {
        return Err(MutationError::RecoveryConflict(
            "prepared stable migration requires recovery".into(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn migrate_legacy_to_stable_locked(
    data_path: &Path,
    data_root: &AnchoredRoot,
    vault_path: &Path,
    stable_identity: &VaultIdentity,
    legacy_scope: &ContentDigest,
    current_lease: &ActiveMarkdownRootLeaseV1,
    writer: &DeviceId,
    journal: &LocalMutationJournal,
    process_lock: &CoordinatorProcessLock,
) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
    migrate_legacy_to_stable_locked_inner(
        data_path,
        data_root,
        vault_path,
        stable_identity,
        legacy_scope,
        current_lease,
        writer,
        journal,
        process_lock,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
#[cfg(feature = "mcp")]
pub(super) fn migrate_legacy_to_stable_locked_with_initial_publication(
    data_path: &Path,
    data_root: &AnchoredRoot,
    vault_path: &Path,
    stable_identity: &VaultIdentity,
    legacy_scope: &ContentDigest,
    current_lease: &ActiveMarkdownRootLeaseV1,
    writer: &DeviceId,
    journal: &LocalMutationJournal,
    process_lock: &CoordinatorProcessLock,
    before_initial_marker: &mut dyn FnMut() -> Result<(), MutationError>,
) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
    migrate_legacy_to_stable_locked_inner(
        data_path,
        data_root,
        vault_path,
        stable_identity,
        legacy_scope,
        current_lease,
        writer,
        journal,
        process_lock,
        Some(before_initial_marker),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn migrate_legacy_to_stable_locked_inner(
    data_path: &Path,
    data_root: &AnchoredRoot,
    vault_path: &Path,
    stable_identity: &VaultIdentity,
    legacy_scope: &ContentDigest,
    current_lease: &ActiveMarkdownRootLeaseV1,
    writer: &DeviceId,
    journal: &LocalMutationJournal,
    process_lock: &CoordinatorProcessLock,
    mut before_initial_marker: Option<&mut dyn FnMut() -> Result<(), MutationError>>,
    #[cfg_attr(not(test), allow(unused_variables))] fault: Option<StableMigrationFault>,
) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
    require_call_binding(
        data_path,
        data_root,
        vault_path,
        stable_identity,
        legacy_scope,
        current_lease,
        writer,
        process_lock,
    )?;
    reject_legacy_global_marker(data_root)?;

    data_root.open_directory("twin/events", false)?;
    let append_lock = data_root.lock_exclusive("twin/events/append-v1.lock")?;
    let result = (|| {
        let durable_lease = super::load_active_root_lease(data_root)?;
        if durable_lease != *current_lease {
            return Err(MutationError::RecoveryConflict(
                "stable-migration-caller-lease-is-stale".into(),
            ));
        }

        let existing_marker = read_marker(data_root, &stable_identity.root_scope)?;
        require_no_markerless_scope_history(
            data_root,
            &stable_identity.root_scope,
            existing_marker.is_some(),
        )?;
        if existing_marker.is_none() && current_lease.is_stable() {
            require_fresh_stable_install(
                data_root,
                vault_path,
                stable_identity,
                legacy_scope,
                current_lease,
            )?;
            return Ok(current_lease.clone());
        }

        let mut marker = match existing_marker {
            Some(marker) => {
                validate_marker_binding(
                    &marker,
                    stable_identity,
                    legacy_scope,
                    writer,
                    &durable_lease,
                )?;
                marker
            }
            None => {
                if current_lease.schema_version != 1 || current_lease.root_scope != *legacy_scope {
                    return Err(MutationError::RecoveryConflict(
                        "stable-lease-or-unsupported-lease-without-migration-marker".into(),
                    ));
                }
                let components =
                    build_initial_inventory(data_root, vault_path, stable_identity, legacy_scope)?;
                let marker = StableMigrationMarkerV1 {
                    schema_version: MARKER_SCHEMA_VERSION,
                    vault_id: stable_identity.descriptor.vault_id().to_string(),
                    stable_scope: stable_identity.root_scope.clone(),
                    legacy_scope: legacy_scope.clone(),
                    legacy_lease_epoch_uuid: current_lease.epoch_uuid.clone(),
                    writer_device_id: writer.clone(),
                    stable_lease: ActiveMarkdownRootLeaseV1::new_stable(
                        stable_identity.root_scope.clone(),
                    ),
                    components,
                    moved_sources: Vec::new(),
                    lease_published: false,
                    state: StableMigrationStateV1::Prepared,
                };
                validate_marker_shape(&marker)?;
                require_component_states(data_root, &marker, false)?;
                require_no_active_legacy_work(data_root, &marker)?;
                require_no_retained_mutation_owners(journal, process_lock)?;
                if let Some(publish) = before_initial_marker.take() {
                    publish()?;
                }
                install_initial_marker(data_root, &marker)?;
                marker
            }
        };

        reject_uninventoried_state(data_root, &marker, vault_path)?;
        reconcile_progress_from_disk(data_root, &mut marker)?;

        if marker.state == StableMigrationStateV1::Committed {
            require_component_states(data_root, &marker, true)?;
            if !durable_lease.is_stable() || durable_lease.root_scope != marker.stable_scope {
                return Err(MutationError::RecoveryConflict(
                    "committed-stable-migration-lease-changed".into(),
                ));
            }
            return Ok(durable_lease);
        }

        require_no_retained_mutation_owners(journal, process_lock)?;
        require_no_active_legacy_work(data_root, &marker)?;

        for component in marker.components.clone() {
            if marker
                .moved_sources
                .binary_search(&component.source)
                .is_ok()
            {
                continue;
            }
            match component_state(data_root, &component)? {
                ComponentState::SourceOnly => {
                    #[cfg(test)]
                    let is_first = marker.components.first() == Some(&component);
                    #[cfg(test)]
                    if is_first
                        && fault == Some(StableMigrationFault::DestinationCollisionAtFirstRename)
                    {
                        let destination = data_path.join(&component.destination);
                        data_root.rename_no_replace_with_hook(
                            &component.source,
                            &component.destination,
                            true,
                            || match component.kind {
                                StableMigrationComponentKindV1::Directory => {
                                    std::fs::create_dir(&destination)
                                        .expect("inject foreign destination directory");
                                    std::fs::write(destination.join("foreign.json"), b"foreign")
                                        .expect("inject foreign destination contents");
                                }
                                StableMigrationComponentKindV1::RegularFile => {
                                    std::fs::write(&destination, b"foreign")
                                        .expect("inject foreign destination file");
                                }
                            },
                        )?;
                    }
                    #[cfg(not(test))]
                    data_root.rename_no_replace(&component.source, &component.destination, true)?;
                    #[cfg(test)]
                    if !(is_first
                        && fault == Some(StableMigrationFault::DestinationCollisionAtFirstRename))
                    {
                        data_root.rename_no_replace(
                            &component.source,
                            &component.destination,
                            true,
                        )?;
                    }
                    #[cfg(test)]
                    if component.source == LEGACY_TWIN_ASSIGNMENT
                        && fault
                            == Some(StableMigrationFault::AfterTwinAssignmentRenameBeforeProgress)
                    {
                        return Err(MutationError::RecoveryConflict(
                            "injected-stable-migration-assignment-rename-boundary".into(),
                        ));
                    }
                    #[cfg(test)]
                    if is_first
                        && fault == Some(StableMigrationFault::AfterFirstRenameBeforeProgress)
                    {
                        return Err(MutationError::RecoveryConflict(
                            "injected-stable-migration-rename-boundary".into(),
                        ));
                    }
                }
                ComponentState::DestinationOnly => {}
                ComponentState::Both => {
                    return Err(MutationError::RecoveryConflict(format!(
                        "stable migration refuses to merge {}",
                        component.source
                    )))
                }
                ComponentState::Neither => {
                    return Err(MutationError::RecoveryConflict(format!(
                        "stable migration component disappeared: {}",
                        component.source
                    )))
                }
            }
            require_destination_only(data_root, &component)?;
            marker.moved_sources.push(component.source);
            marker.moved_sources.sort();
            write_marker(data_root, &marker)?;
        }

        if marker.moved_sources.len() != marker.components.len() {
            return Err(MutationError::RecoveryConflict(
                "stable-migration-progress-is-incomplete".into(),
            ));
        }

        match durable_lease {
            lease if lease == marker.stable_lease => {
                if !marker.lease_published {
                    marker.lease_published = true;
                    write_marker(data_root, &marker)?;
                }
            }
            lease
                if lease.schema_version == 1
                    && lease.root_scope == marker.legacy_scope
                    && lease.epoch_uuid == marker.legacy_lease_epoch_uuid =>
            {
                super::write_active_root_lease(data_root, &marker.stable_lease)?;
                #[cfg(test)]
                if fault == Some(StableMigrationFault::AfterLeaseBeforeProgress) {
                    return Err(MutationError::RecoveryConflict(
                        "injected-stable-migration-lease-boundary".into(),
                    ));
                }
                marker.lease_published = true;
                write_marker(data_root, &marker)?;
            }
            _ => {
                return Err(MutationError::RecoveryConflict(
                    "active-lease-changed-during-stable-migration".into(),
                ))
            }
        }

        marker.state = StableMigrationStateV1::Committed;
        write_marker(data_root, &marker)?;
        reject_uninventoried_state(data_root, &marker, vault_path)?;
        require_component_states(data_root, &marker, true)?;
        Ok(marker.stable_lease)
    })();
    let unlock = append_lock.unlock().map_err(MutationError::from);
    match (result, unlock) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(lease), Ok(())) => Ok(lease),
    }
}

#[allow(clippy::too_many_arguments)]
fn require_call_binding(
    data_path: &Path,
    data_root: &AnchoredRoot,
    vault_path: &Path,
    stable_identity: &VaultIdentity,
    legacy_scope: &ContentDigest,
    current_lease: &ActiveMarkdownRootLeaseV1,
    writer: &DeviceId,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    if !process_lock.covers_data_path(data_path)?
        || std::fs::canonicalize(data_path)? != data_root.canonical_path()
    {
        return Err(MutationError::Invalid(
            "stable migration lock or data capability belongs to another root".into(),
        ));
    }
    let durable_identity = crate::services::sync::identity::load_vault_identity(vault_path)?;
    if &durable_identity != stable_identity {
        return Err(MutationError::RecoveryConflict(
            "vault-descriptor-changed-before-stable-migration".into(),
        ));
    }
    let durable_legacy_scope =
        crate::services::sync::identity::legacy_path_scope_for_migration(vault_path)?;
    if &durable_legacy_scope != legacy_scope || stable_identity.root_scope == *legacy_scope {
        return Err(MutationError::RecoveryConflict(
            "stable-and-legacy-vault-scopes-are-not-distinct".into(),
        ));
    }
    require_canonical_uuid(current_lease.epoch_uuid.as_str(), "active lease epoch")?;
    require_canonical_uuid(writer.as_str(), "writer device ID")?;
    Ok(())
}

fn validate_marker_binding(
    marker: &StableMigrationMarkerV1,
    stable_identity: &VaultIdentity,
    legacy_scope: &ContentDigest,
    writer: &DeviceId,
    durable_lease: &ActiveMarkdownRootLeaseV1,
) -> Result<(), MutationError> {
    validate_marker_shape(marker)?;
    if marker.vault_id != stable_identity.descriptor.vault_id().to_string()
        || marker.stable_scope != stable_identity.root_scope
        || marker.writer_device_id != *writer
    {
        return Err(MutationError::RecoveryConflict(
            "stable-migration-marker-belongs-to-another-vault-or-writer".into(),
        ));
    }
    if marker.state == StableMigrationStateV1::Prepared && marker.legacy_scope != *legacy_scope {
        return Err(MutationError::RecoveryConflict(
            "vault-path-changed-during-stable-migration".into(),
        ));
    }
    let lease_matches_legacy = durable_lease.schema_version == 1
        && durable_lease.root_scope == marker.legacy_scope
        && durable_lease.epoch_uuid == marker.legacy_lease_epoch_uuid;
    let lease_matches_stable = durable_lease.is_stable()
        && durable_lease.root_scope == marker.stable_scope
        && (marker.state == StableMigrationStateV1::Committed
            || durable_lease == &marker.stable_lease);
    if !lease_matches_legacy && !lease_matches_stable {
        return Err(MutationError::RecoveryConflict(
            "stable-migration-marker-lease-binding-changed".into(),
        ));
    }
    if marker.lease_published && !lease_matches_stable {
        return Err(MutationError::RecoveryConflict(
            "stable-migration-marker-published-lease-is-not-durable".into(),
        ));
    }
    Ok(())
}

fn require_fresh_stable_install(
    root: &AnchoredRoot,
    vault_path: &Path,
    stable_identity: &VaultIdentity,
    legacy_scope: &ContentDigest,
    lease: &ActiveMarkdownRootLeaseV1,
) -> Result<(), MutationError> {
    require_no_markerless_scope_history(root, &stable_identity.root_scope, false)?;
    if lease.schema_version != 2 || lease.root_scope != stable_identity.root_scope {
        return Err(MutationError::RecoveryConflict(
            "fresh stable lease does not match the vault descriptor".into(),
        ));
    }
    let fnv = twin_key(&crate::models::settings::legacy_twin_data_path_for_vault(
        Path::new("."),
        vault_path,
    ))?;
    let path_sha = format!("twin/{}", legacy_scope.as_str());
    let legacy_derived = format!("vault_derived/v1/{}", legacy_scope.as_str());
    if entry_kind(root, &fnv)?.is_some()
        || entry_kind(root, &path_sha)?.is_some()
        || entry_kind(root, &legacy_derived)?.is_some()
        || root.read_bounded(LEGACY_TWIN_ASSIGNMENT, 4096)?.is_some()
        || root
            .read_bounded(LEGACY_DERIVED_ASSIGNMENT, 4096)?
            .is_some()
        || !collect_direct_canvas_files(root)?.is_empty()
    {
        return Err(MutationError::RecoveryConflict(
            "stable lease without a marker has unexplained legacy state".into(),
        ));
    }
    for directory in [
        "search_index",
        "chunk_index",
        "link_discovery",
        "vault_migration",
    ] {
        if root.directory_exists(directory)? {
            return Err(MutationError::RecoveryConflict(
                "stable lease without a marker has unexplained legacy state".into(),
            ));
        }
    }
    for directory in [
        LEGACY_EVENT_RECORDS,
        LEGACY_EVENT_QUARANTINE,
        LEGACY_EVENT_STAGING,
    ] {
        if event_directory_has_files(root, directory)? {
            return Err(MutationError::RecoveryConflict(
                "stable lease without a marker has unexplained legacy state".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn requires_schema_one_bootstrap_without_lease(
    root: &AnchoredRoot,
    vault_path: &Path,
    legacy_scope: &ContentDigest,
) -> Result<bool, MutationError> {
    reject_legacy_global_marker(root)?;
    let fnv = twin_key(&crate::models::settings::legacy_twin_data_path_for_vault(
        Path::new("."),
        vault_path,
    ))?;
    let path_sha = format!("twin/{}", legacy_scope.as_str());
    let fnv_kind = entry_kind(root, &fnv)?;
    let path_sha_kind = entry_kind(root, &path_sha)?;
    for (key, kind) in [(&fnv, fnv_kind), (&path_sha, path_sha_kind)] {
        if kind.is_some_and(|kind| kind != AnchoredEntryKind::Directory) {
            return Err(MutationError::Invalid(format!(
                "recognized legacy Twin namespace is not a real directory: {key}"
            )));
        }
    }
    if fnv_kind.is_some() && path_sha_kind.is_some() {
        return Err(MutationError::RecoveryConflict(
            "both legacy Twin namespaces exist; refusing to merge".into(),
        ));
    }

    let mut other_twin_namespace = false;
    if root.directory_exists("twin")? {
        for (name, _) in bounded_entries(root, "twin", MAX_COMPONENTS)? {
            let key = format!("twin/{name}");
            if key != fnv
                && key != path_sha
                && (is_legacy_fnv_twin_key(&key) || is_digest_name(&name))
            {
                other_twin_namespace = true;
            }
        }
    }

    let legacy_derived = format!("vault_derived/v1/{}", legacy_scope.as_str());
    let legacy_derived_kind = entry_kind(root, &legacy_derived)?;
    if legacy_derived_kind.is_some_and(|kind| kind != AnchoredEntryKind::Directory) {
        return Err(MutationError::Invalid(
            "legacy vault-derived namespace is not a real directory".into(),
        ));
    }
    let mut other_derived_scope = false;
    if root.directory_exists("vault_derived/v1")? {
        for (name, _) in bounded_entries(root, "vault_derived/v1", MAX_COMPONENTS)? {
            if name != legacy_scope.as_str() && is_digest_name(&name) {
                other_derived_scope = true;
            }
        }
    }
    let mut unscoped_derived = Vec::new();
    for component in LEGACY_DERIVED_COMPONENTS {
        if root.directory_exists(component)? {
            unscoped_derived.push(component);
        }
    }
    let has_assignment = entry_kind(root, LEGACY_TWIN_ASSIGNMENT)?.is_some()
        || entry_kind(root, LEGACY_DERIVED_ASSIGNMENT)?.is_some();
    let has_canvas =
        root.directory_exists("canvas")? && !collect_direct_canvas_files(root)?.is_empty();
    let has_legacy_events = [
        LEGACY_EVENT_RECORDS,
        LEGACY_EVENT_QUARANTINE,
        LEGACY_EVENT_STAGING,
    ]
    .into_iter()
    .map(|directory| event_directory_has_files(root, directory))
    .collect::<Result<Vec<_>, _>>()?
    .into_iter()
    .any(|has_files| has_files);
    let has_scoped_canvas = root.directory_exists("canvas/v1")?
        && !bounded_entries(root, "canvas/v1", MAX_COMPONENTS)?.is_empty();
    let has_scoped_events = root.directory_exists("twin/events/vaults/v1")?
        && !bounded_entries(root, "twin/events/vaults/v1", MAX_COMPONENTS)?.is_empty();

    let twin_source = match (fnv_kind.is_some(), path_sha_kind.is_some()) {
        (true, false) => Some(fnv.as_str()),
        (false, true) => Some(path_sha.as_str()),
        (false, false) => None,
        (true, true) => unreachable!("duplicate legacy Twin sources rejected above"),
    };
    let has_other_legacy_state = legacy_derived_kind.is_some()
        || !unscoped_derived.is_empty()
        || has_assignment
        || has_canvas
        || has_legacy_events;
    let has_ambiguous_scoped_state =
        other_twin_namespace || other_derived_scope || has_scoped_canvas || has_scoped_events;
    let Some(twin_source) = twin_source else {
        if has_other_legacy_state || has_ambiguous_scoped_state {
            return Err(MutationError::RecoveryConflict(
                "legacy state exists without one recognized legacy Twin namespace".into(),
            ));
        }
        return Ok(false);
    };
    if has_ambiguous_scoped_state {
        return Err(MutationError::RecoveryConflict(
            "multiple legacy or stable namespaces exist without an active lease".into(),
        ));
    }
    if has_assignment {
        return Err(MutationError::RecoveryConflict(
            "legacy assignment state without an active lease is ambiguous".into(),
        ));
    }
    if legacy_derived_kind.is_some() != unscoped_derived.is_empty() {
        return Err(MutationError::RecoveryConflict(
            "legacy vault-derived state is missing or has multiple sources".into(),
        ));
    }
    for directory in [
        "twin/mutations/pending/v1",
        "twin/mutations/preauthority/v1",
        "twin/mutations/quarantine/v1",
        "twin/mutations/staging/v1",
        "twin/mutations/receipts/v1",
    ] {
        if root.directory_exists(directory)?
            && !bounded_entries(root, directory, MAX_UNLEASED_OWNER_ENTRIES)?.is_empty()
        {
            return Err(MutationError::RecoveryConflict(
                "authority owner exists without its active lease".into(),
            ));
        }
    }
    let vault_migration_root = if legacy_derived_kind.is_some() {
        Some(format!("{legacy_derived}/vault_migration"))
    } else if unscoped_derived.contains(&"vault_migration") {
        Some("vault_migration".to_string())
    } else {
        None
    };
    if let Some(vault_migration_root) = vault_migration_root {
        require_no_active_vault_migration(root, &vault_migration_root)?;
    }
    #[cfg(not(windows))]
    if twin_source == fnv {
        return Err(MutationError::RecoveryConflict(
            "legacy Twin namespace uses a case-folded root hash and is ambiguous on this platform"
                .into(),
        ));
    }
    audit_tree(root, twin_source)?;
    if legacy_derived_kind.is_some() {
        audit_tree(root, &legacy_derived)?;
    }
    for component in unscoped_derived {
        audit_tree(root, component)?;
    }
    Ok(true)
}

fn validate_marker_shape(marker: &StableMigrationMarkerV1) -> Result<(), MutationError> {
    if marker.schema_version != MARKER_SCHEMA_VERSION
        || marker.components.len() < 2
        || marker.components.len() > MAX_COMPONENTS
        || marker.stable_lease.schema_version != 2
        || marker.stable_lease.root_scope != marker.stable_scope
    {
        return Err(MutationError::Invalid(
            "invalid stable migration marker shape".into(),
        ));
    }
    require_canonical_uuid(&marker.vault_id, "marker vault ID")?;
    require_canonical_uuid(&marker.legacy_lease_epoch_uuid, "marker legacy lease epoch")?;
    require_canonical_uuid(&marker.stable_lease.epoch_uuid, "marker stable lease epoch")?;
    require_canonical_uuid(marker.writer_device_id.as_str(), "marker writer device ID")?;
    if marker.state == StableMigrationStateV1::Committed
        && (!marker.lease_published || marker.moved_sources.len() != marker.components.len())
    {
        return Err(MutationError::Invalid(
            "committed stable migration marker is incomplete".into(),
        ));
    }

    let mut sorted_components = marker.components.clone();
    sorted_components.sort();
    if sorted_components != marker.components
        || sorted_components.windows(2).any(|pair| pair[0] == pair[1])
    {
        return Err(MutationError::Invalid(
            "stable migration component inventory is not canonical".into(),
        ));
    }
    let sources = marker
        .components
        .iter()
        .map(|component| component.source.as_str())
        .collect::<BTreeSet<_>>();
    let destinations = marker
        .components
        .iter()
        .map(|component| component.destination.as_str())
        .collect::<BTreeSet<_>>();
    if sources.len() != marker.components.len()
        || destinations.len() != marker.components.len()
        || sources.iter().any(|source| destinations.contains(source))
    {
        return Err(MutationError::Invalid(
            "stable migration component paths alias".into(),
        ));
    }
    for component in &marker.components {
        crate::services::twin_events::validate_relative_key(&component.source)?;
        crate::services::twin_events::validate_relative_key(&component.destination)?;
    }
    if !marker
        .moved_sources
        .windows(2)
        .all(|pair| pair[0] < pair[1])
        || marker
            .moved_sources
            .iter()
            .any(|source| !sources.contains(source.as_str()))
    {
        return Err(MutationError::Invalid(
            "stable migration progress is not canonical".into(),
        ));
    }
    validate_exact_inventory_contract(marker)
}

fn validate_exact_inventory_contract(
    marker: &StableMigrationMarkerV1,
) -> Result<(), MutationError> {
    let path_sha = format!("twin/{}", marker.legacy_scope.as_str());
    let stable_twin = format!("twin/{}", marker.stable_scope.as_str());
    let legacy_derived = format!("vault_derived/v1/{}", marker.legacy_scope.as_str());
    let stable_derived = format!("vault_derived/v1/{}", marker.stable_scope.as_str());

    let directory_components = marker
        .components
        .iter()
        .filter(|component| component.kind == StableMigrationComponentKindV1::Directory)
        .collect::<Vec<_>>();
    if !(2..=5).contains(&directory_components.len())
        || !directory_components.iter().any(|component| {
            (is_legacy_fnv_twin_key(&component.source) || component.source == path_sha)
                && component.destination == stable_twin
        })
        || !directory_components.iter().any(|component| {
            component.source == legacy_derived && component.destination == stable_derived
        })
    {
        return Err(MutationError::Invalid(
            "stable migration directory inventory is invalid".into(),
        ));
    }
    for component in directory_components {
        let core_component = ((is_legacy_fnv_twin_key(&component.source)
            || component.source == path_sha)
            && component.destination == stable_twin)
            || (component.source == legacy_derived && component.destination == stable_derived);
        if !core_component
            && expected_event_directory_destination(&component.source, &marker.stable_scope)
                .as_deref()
                != Some(component.destination.as_str())
        {
            return Err(MutationError::Invalid(
                "stable migration event-directory inventory is invalid".into(),
            ));
        }
    }

    for component in marker
        .components
        .iter()
        .filter(|component| component.kind == StableMigrationComponentKindV1::RegularFile)
    {
        if expected_file_destination(&component.source, &marker.stable_scope).as_deref()
            != Some(component.destination.as_str())
        {
            return Err(MutationError::Invalid(
                "stable migration regular-file inventory is invalid".into(),
            ));
        }
    }
    Ok(())
}

fn build_initial_inventory(
    root: &AnchoredRoot,
    vault_path: &Path,
    stable_identity: &VaultIdentity,
    legacy_scope: &ContentDigest,
) -> Result<Vec<StableMigrationComponentV1>, MutationError> {
    let fnv = twin_key(&crate::models::settings::legacy_twin_data_path_for_vault(
        Path::new("."),
        vault_path,
    ))?;
    let path_sha = format!("twin/{}", legacy_scope.as_str());
    let stable_twin = format!("twin/{}", stable_identity.root_scope.as_str());
    let fnv_kind = entry_kind(root, &fnv)?;
    let path_sha_kind = entry_kind(root, &path_sha)?;
    let twin_source = match (fnv_kind, path_sha_kind) {
        (Some(AnchoredEntryKind::Directory), None) => fnv,
        (None, Some(AnchoredEntryKind::Directory)) => path_sha,
        (Some(_), None) | (None, Some(_)) => {
            return Err(MutationError::Invalid(
                "legacy Twin namespace is not a real directory".into(),
            ))
        }
        (Some(_), Some(_)) => {
            return Err(MutationError::RecoveryConflict(
                "both legacy Twin namespaces exist; refusing to merge".into(),
            ))
        }
        (None, None) => {
            return Err(MutationError::RecoveryConflict(
                "legacy Twin namespace is missing".into(),
            ))
        }
    };
    if entry_kind(root, &stable_twin)?.is_some() {
        return Err(MutationError::RecoveryConflict(
            "stable Twin destination already contains state under schema one".into(),
        ));
    }
    audit_tree(root, &twin_source)?;

    let legacy_derived = format!("vault_derived/v1/{}", legacy_scope.as_str());
    let stable_derived = format!("vault_derived/v1/{}", stable_identity.root_scope.as_str());
    if entry_kind(root, &legacy_derived)? != Some(AnchoredEntryKind::Directory) {
        return Err(MutationError::RecoveryConflict(
            "legacy vault-derived namespace is missing or invalid".into(),
        ));
    }
    if entry_kind(root, &stable_derived)?.is_some() {
        return Err(MutationError::RecoveryConflict(
            "stable vault-derived destination already contains state under schema one".into(),
        ));
    }
    audit_tree(root, &legacy_derived)?;

    let mut components = vec![
        StableMigrationComponentV1 {
            source: twin_source,
            destination: stable_twin,
            kind: StableMigrationComponentKindV1::Directory,
        },
        StableMigrationComponentV1 {
            source: legacy_derived,
            destination: stable_derived,
            kind: StableMigrationComponentKindV1::Directory,
        },
    ];

    let stable_scope = &stable_identity.root_scope;
    for source in [LEGACY_TWIN_ASSIGNMENT, LEGACY_DERIVED_ASSIGNMENT] {
        match entry_kind(root, source)? {
            None => {}
            Some(AnchoredEntryKind::File) => push_component(
                &mut components,
                StableMigrationComponentV1 {
                    source: source.to_string(),
                    destination: expected_file_destination(source, stable_scope)
                        .expect("legacy assignment has a scoped history destination"),
                    kind: StableMigrationComponentKindV1::RegularFile,
                },
            )?,
            Some(AnchoredEntryKind::Directory) => {
                return Err(MutationError::Invalid(
                    "legacy assignment state is not a regular file".into(),
                ))
            }
        }
    }
    for source in [
        LEGACY_EVENT_RECORDS,
        LEGACY_EVENT_QUARANTINE,
        LEGACY_EVENT_STAGING,
    ] {
        let destination = expected_event_directory_destination(source, stable_scope)
            .expect("legacy event directory has a stable destination");
        if event_directory_has_files(root, source)? {
            if entry_kind(root, &destination)?.is_some() {
                return Err(MutationError::RecoveryConflict(
                    "stable event destination already exists under schema one".into(),
                ));
            }
            push_component(
                &mut components,
                StableMigrationComponentV1 {
                    source: source.to_string(),
                    destination,
                    kind: StableMigrationComponentKindV1::Directory,
                },
            )?;
        }
    }
    for source in collect_direct_canvas_files(root)? {
        push_component(
            &mut components,
            StableMigrationComponentV1 {
                destination: expected_file_destination(&source, stable_scope)
                    .expect("collected Canvas path has a stable destination"),
                source,
                kind: StableMigrationComponentKindV1::RegularFile,
            },
        )?;
    }
    if !collect_all_stable_event_files(root, stable_scope)?.is_empty()
        || !collect_stable_canvas_files(root, stable_scope)?.is_empty()
    {
        return Err(MutationError::RecoveryConflict(
            "stable file destination already contains state under schema one".into(),
        ));
    }
    components.sort();
    if components.len() > MAX_COMPONENTS {
        return Err(MutationError::Invalid(format!(
            "stable migration inventory exceeds {MAX_COMPONENTS} components"
        )));
    }
    Ok(components)
}

fn push_component(
    components: &mut Vec<StableMigrationComponentV1>,
    component: StableMigrationComponentV1,
) -> Result<(), MutationError> {
    if components.len() >= MAX_COMPONENTS {
        return Err(MutationError::Invalid(format!(
            "stable migration inventory exceeds {MAX_COMPONENTS} components"
        )));
    }
    components.push(component);
    Ok(())
}

fn reconcile_progress_from_disk(
    root: &AnchoredRoot,
    marker: &mut StableMigrationMarkerV1,
) -> Result<(), MutationError> {
    let mut changed = false;
    for component in &marker.components {
        let recorded = marker
            .moved_sources
            .binary_search(&component.source)
            .is_ok();
        match (recorded, component_state(root, component)?) {
            (false, ComponentState::DestinationOnly) => {
                marker.moved_sources.push(component.source.clone());
                marker.moved_sources.sort();
                changed = true;
            }
            (false, ComponentState::SourceOnly) | (true, ComponentState::DestinationOnly) => {}
            (false, ComponentState::Both) | (true, ComponentState::Both) => {
                return Err(MutationError::RecoveryConflict(format!(
                    "stable migration refuses to merge {}",
                    component.source
                )))
            }
            (false, ComponentState::Neither) | (true, ComponentState::Neither) => {
                return Err(MutationError::RecoveryConflict(format!(
                    "stable migration component disappeared: {}",
                    component.source
                )))
            }
            (true, ComponentState::SourceOnly) => {
                return Err(MutationError::RecoveryConflict(format!(
                    "stable migration marker advanced before rename: {}",
                    component.source
                )))
            }
        }
    }
    if changed {
        write_marker(root, marker)?;
    }
    Ok(())
}

fn require_component_states(
    root: &AnchoredRoot,
    marker: &StableMigrationMarkerV1,
    all_moved: bool,
) -> Result<(), MutationError> {
    for component in &marker.components {
        let recorded = marker
            .moved_sources
            .binary_search(&component.source)
            .is_ok();
        match component_state(root, component)? {
            ComponentState::SourceOnly if !recorded && !all_moved => {}
            ComponentState::DestinationOnly if recorded => {}
            ComponentState::Both => {
                return Err(MutationError::RecoveryConflict(format!(
                    "stable migration refuses to merge {}",
                    component.source
                )))
            }
            _ => {
                return Err(MutationError::RecoveryConflict(format!(
                    "stable migration component state is inconsistent: {}",
                    component.source
                )))
            }
        }
    }
    Ok(())
}

fn reject_uninventoried_state(
    root: &AnchoredRoot,
    marker: &StableMigrationMarkerV1,
    vault_path: &Path,
) -> Result<(), MutationError> {
    let sources = marker
        .components
        .iter()
        .map(|component| component.source.as_str())
        .collect::<BTreeSet<_>>();
    let destinations = marker
        .components
        .iter()
        .map(|component| component.destination.as_str())
        .collect::<BTreeSet<_>>();
    for source in [LEGACY_TWIN_ASSIGNMENT, LEGACY_DERIVED_ASSIGNMENT] {
        if entry_kind(root, source)?.is_some() && !sources.contains(source) {
            return Err(MutationError::RecoveryConflict(
                "legacy global assignment data appeared after stable assignment was prepared"
                    .into(),
            ));
        }
    }
    for source in [
        LEGACY_EVENT_RECORDS,
        LEGACY_EVENT_QUARANTINE,
        LEGACY_EVENT_STAGING,
    ] {
        if event_directory_has_files(root, source)? && !sources.contains(source) {
            return Err(MutationError::RecoveryConflict(
                "legacy global event data appeared after stable assignment was prepared".into(),
            ));
        }
    }
    for path in collect_direct_canvas_files(root)? {
        if !sources.contains(path.as_str()) {
            return Err(MutationError::RecoveryConflict(
                "legacy global data appeared after stable assignment was prepared".into(),
            ));
        }
    }
    if marker.state == StableMigrationStateV1::Prepared {
        for destination in [
            format!(
                "twin/events/vaults/v1/{}/records/v1",
                marker.stable_scope.as_str()
            ),
            format!(
                "twin/events/vaults/v1/{}/quarantine/v1",
                marker.stable_scope.as_str()
            ),
            format!(
                "twin/events/vaults/v1/{}/staging/v1",
                marker.stable_scope.as_str()
            ),
        ] {
            if event_directory_has_files(root, &destination)?
                && !destinations.contains(destination.as_str())
            {
                return Err(MutationError::RecoveryConflict(
                    "unowned event data appeared in the stable destination".into(),
                ));
            }
        }
        for path in collect_stable_canvas_files(root, &marker.stable_scope)? {
            if !destinations.contains(path.as_str()) {
                return Err(MutationError::RecoveryConflict(
                    "unowned data appeared in the stable destination".into(),
                ));
            }
        }
    }

    let stable_twin = format!("twin/{}", marker.stable_scope.as_str());
    let selected = marker
        .components
        .iter()
        .find(|component| {
            component.kind == StableMigrationComponentKindV1::Directory
                && component.destination == stable_twin
        })
        .map(|component| component.source.as_str())
        .ok_or_else(|| MutationError::Invalid("stable Twin component is missing".into()))?;
    let current_fnv = twin_key(&crate::models::settings::legacy_twin_data_path_for_vault(
        Path::new("."),
        vault_path,
    ))?;
    let current_legacy_scope =
        crate::services::sync::identity::legacy_path_scope_for_migration(vault_path)?;
    let historical_path_sha = format!("twin/{}", marker.legacy_scope.as_str());
    let current_path_sha = format!("twin/{}", current_legacy_scope.as_str());
    let mut legacy_candidates =
        BTreeSet::from([historical_path_sha, current_path_sha, current_fnv]);
    if root.directory_exists("twin")? {
        for (name, kind) in bounded_entries(root, "twin", MAX_COMPONENTS)? {
            let key = format!("twin/{name}");
            if kind == AnchoredEntryKind::Directory && is_legacy_fnv_twin_key(&key) {
                legacy_candidates.insert(key);
            }
        }
    }
    for candidate in legacy_candidates {
        if candidate != selected && entry_kind(root, &candidate)?.is_some() {
            return Err(MutationError::RecoveryConflict(
                "another legacy Twin namespace appeared after assignment".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn require_no_retained_mutation_owners(
    journal: &LocalMutationJournal,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    journal.cleanup_orphan_temps_locked(process_lock)?;
    if journal.retained_owner_count(process_lock)? != 0 {
        return Err(MutationError::RecoveryConflict(
            "stable migration requires an empty mutation-owner journal".into(),
        ));
    }
    Ok(())
}

fn require_no_active_legacy_work(
    root: &AnchoredRoot,
    marker: &StableMigrationMarkerV1,
) -> Result<(), MutationError> {
    let derived = marker
        .components
        .iter()
        .find(|component| component.source.starts_with("vault_derived/v1/"))
        .ok_or_else(|| MutationError::Invalid("stable derived component is missing".into()))?;
    let active_root = match component_state(root, derived)? {
        ComponentState::SourceOnly => &derived.source,
        ComponentState::DestinationOnly => &derived.destination,
        ComponentState::Both => {
            return Err(MutationError::RecoveryConflict(
                "stable migration refuses to merge vault-derived state".into(),
            ))
        }
        ComponentState::Neither => {
            return Err(MutationError::RecoveryConflict(
                "vault-derived state disappeared during stable migration".into(),
            ))
        }
    };
    audit_tree(root, active_root)?;
    require_no_active_vault_migration(root, &format!("{active_root}/vault_migration"))
}

fn require_no_active_vault_migration(
    root: &AnchoredRoot,
    vault_migration_root: &str,
) -> Result<(), MutationError> {
    let optimizer = format!("{vault_migration_root}/optimizer");
    for pending in ["pending-publications-v1", "pending-rollbacks-v1"] {
        let directory = format!("{optimizer}/{pending}");
        if root.directory_exists(&directory)?
            && !root.regular_file_names_bounded(&directory, 64)?.is_empty()
        {
            return Err(MutationError::RecoveryConflict(
                "stable migration is blocked by pending optimizer work".into(),
            ));
        }
    }

    let runs = format!("{vault_migration_root}/runs");
    if !root.directory_exists(&runs)? {
        return Ok(());
    }
    let entries = bounded_entries(root, &runs, MAX_MIGRATION_RUNS)?;
    for (run, kind) in entries {
        if kind != AnchoredEntryKind::Directory {
            return Err(MutationError::Invalid(
                "Markdown migration runs contain a non-directory entry".into(),
            ));
        }
        let manifest_key = format!("{runs}/{run}/manifest.json");
        let Some(bytes) = root.read_bounded(&manifest_key, MAX_MARKDOWN_MANIFEST_BYTES)? else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            MutationError::Invalid(format!("invalid Markdown migration manifest: {error}"))
        })?;
        let object = value.as_object().ok_or_else(|| {
            MutationError::Invalid("Markdown migration manifest must be an object".into())
        })?;
        let status = object
            .get("status")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                MutationError::Invalid("Markdown migration manifest status is missing".into())
            })?;
        let has_active_step = object
            .get("active_step")
            .is_some_and(|value| !value.is_null());
        if !matches!(status, "applied" | "rolled_back") || has_active_step {
            return Err(MutationError::RecoveryConflict(
                "stable migration is blocked by an active Markdown migration".into(),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComponentState {
    SourceOnly,
    DestinationOnly,
    Both,
    Neither,
}

fn component_state(
    root: &AnchoredRoot,
    component: &StableMigrationComponentV1,
) -> Result<ComponentState, MutationError> {
    let expected = match component.kind {
        StableMigrationComponentKindV1::Directory => AnchoredEntryKind::Directory,
        StableMigrationComponentKindV1::RegularFile => AnchoredEntryKind::File,
    };
    let event_directory = component.kind == StableMigrationComponentKindV1::Directory
        && is_legacy_event_directory(&component.source);
    let source = if event_directory {
        event_directory_has_files(root, &component.source)?.then_some(AnchoredEntryKind::Directory)
    } else {
        entry_kind(root, &component.source)?
    };
    let destination = if event_directory {
        event_directory_has_files(root, &component.destination)?
            .then_some(AnchoredEntryKind::Directory)
    } else {
        entry_kind(root, &component.destination)?
    };
    if source.is_some_and(|kind| kind != expected)
        || destination.is_some_and(|kind| kind != expected)
    {
        return Err(MutationError::Invalid(format!(
            "stable migration component kind changed: {}",
            component.source
        )));
    }
    Ok(match (source.is_some(), destination.is_some()) {
        (true, false) => ComponentState::SourceOnly,
        (false, true) => ComponentState::DestinationOnly,
        (true, true) => ComponentState::Both,
        (false, false) => ComponentState::Neither,
    })
}

fn require_destination_only(
    root: &AnchoredRoot,
    component: &StableMigrationComponentV1,
) -> Result<(), MutationError> {
    if component_state(root, component)? != ComponentState::DestinationOnly {
        return Err(MutationError::RecoveryConflict(format!(
            "stable migration rename did not become durable: {}",
            component.source
        )));
    }
    if component.kind == StableMigrationComponentKindV1::Directory {
        audit_tree(root, &component.destination)?;
    }
    Ok(())
}

fn entry_kind(
    root: &AnchoredRoot,
    relative_key: &str,
) -> Result<Option<AnchoredEntryKind>, MutationError> {
    crate::services::twin_events::validate_relative_key(relative_key)?;
    let (parent, leaf) = relative_key.rsplit_once('/').ok_or_else(|| {
        MutationError::Invalid("stable migration component must have a parent".into())
    })?;
    if !root.directory_exists(parent)? {
        return Ok(None);
    }
    Ok(root
        .directory_entries(parent)?
        .into_iter()
        .find_map(|(name, kind)| (name == leaf).then_some(kind)))
}

fn audit_tree(root: &AnchoredRoot, relative_root: &str) -> Result<(), MutationError> {
    let mut stack = vec![(relative_root.to_string(), 0_usize)];
    let mut total = 0_usize;
    while let Some((directory, depth)) = stack.pop() {
        if depth > MAX_TREE_DEPTH {
            return Err(MutationError::Invalid(format!(
                "stable migration tree exceeds {MAX_TREE_DEPTH} levels"
            )));
        }
        for (name, kind) in root.directory_entries(&directory)? {
            total = total.checked_add(1).ok_or_else(|| {
                MutationError::Invalid("stable migration tree count overflow".into())
            })?;
            if total > MAX_TREE_ENTRIES {
                return Err(MutationError::Invalid(format!(
                    "stable migration tree exceeds {MAX_TREE_ENTRIES} entries"
                )));
            }
            if kind == AnchoredEntryKind::Directory {
                stack.push((format!("{directory}/{name}"), depth + 1));
            }
        }
    }
    Ok(())
}

fn bounded_entries(
    root: &AnchoredRoot,
    directory: &str,
    limit: usize,
) -> Result<Vec<(String, AnchoredEntryKind)>, MutationError> {
    root.directory_entries_bounded(directory, limit)
}

fn collect_all_stable_event_files(
    root: &AnchoredRoot,
    scope: &ContentDigest,
) -> Result<Vec<String>, MutationError> {
    let base = format!("twin/events/vaults/v1/{}", scope.as_str());
    let mut files = collect_record_files_optional(root, &format!("{base}/records/v1"))?;
    files.extend(collect_flat_files_optional(
        root,
        &format!("{base}/quarantine/v1"),
    )?);
    files.extend(collect_flat_files_optional(
        root,
        &format!("{base}/staging/v1"),
    )?);
    files.sort();
    if files.len() > MAX_TREE_ENTRIES {
        return Err(MutationError::Invalid(format!(
            "stable event inventory exceeds {MAX_TREE_ENTRIES} files"
        )));
    }
    Ok(files)
}

fn collect_record_files_optional(
    root: &AnchoredRoot,
    directory: &str,
) -> Result<Vec<String>, MutationError> {
    if !root.directory_exists(directory)? {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for (name, kind) in bounded_entries(root, directory, MAX_EVENT_DIRECTORY_ENTRIES)? {
        match kind {
            AnchoredEntryKind::File => files.push(format!("{directory}/{name}")),
            AnchoredEntryKind::Directory => {
                let prefix = format!("{directory}/{name}");
                for (leaf, leaf_kind) in
                    bounded_entries(root, &prefix, MAX_EVENT_DIRECTORY_ENTRIES)?
                {
                    if leaf_kind != AnchoredEntryKind::File {
                        return Err(MutationError::Invalid(
                            "event record tree is deeper than one prefix".into(),
                        ));
                    }
                    files.push(format!("{prefix}/{leaf}"));
                    if files.len() > MAX_TREE_ENTRIES {
                        return Err(MutationError::Invalid(format!(
                            "event record inventory exceeds {MAX_TREE_ENTRIES} files"
                        )));
                    }
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

fn collect_flat_files_optional(
    root: &AnchoredRoot,
    directory: &str,
) -> Result<Vec<String>, MutationError> {
    if !root.directory_exists(directory)? {
        return Ok(Vec::new());
    }
    Ok(root
        .regular_file_names_bounded(directory, MAX_EVENT_DIRECTORY_ENTRIES)?
        .into_iter()
        .map(|name| format!("{directory}/{name}"))
        .collect())
}

fn is_legacy_event_directory(directory: &str) -> bool {
    matches!(
        directory,
        LEGACY_EVENT_RECORDS | LEGACY_EVENT_QUARANTINE | LEGACY_EVENT_STAGING
    )
}

fn event_directory_has_files(root: &AnchoredRoot, directory: &str) -> Result<bool, MutationError> {
    if directory.ends_with("/records/v1") || directory == LEGACY_EVENT_RECORDS {
        Ok(!collect_record_files_optional(root, directory)?.is_empty())
    } else if directory.ends_with("/quarantine/v1") || directory.ends_with("/staging/v1") {
        Ok(!collect_flat_files_optional(root, directory)?.is_empty())
    } else {
        Err(MutationError::Invalid(
            "unsupported event directory in stable migration".into(),
        ))
    }
}

fn collect_direct_canvas_files(root: &AnchoredRoot) -> Result<Vec<String>, MutationError> {
    if !root.directory_exists("canvas")? {
        return Err(MutationError::RecoveryConflict(
            "legacy Canvas directory is missing".into(),
        ));
    }
    let mut files = Vec::new();
    for (name, kind) in bounded_entries(root, "canvas", MAX_CANVAS_ENTRIES)? {
        match kind {
            AnchoredEntryKind::File if name.ends_with(".json") => {
                files.push(format!("canvas/{name}"));
            }
            AnchoredEntryKind::Directory if name == "v1" => {}
            _ => {
                return Err(MutationError::Invalid(
                    "Canvas root contains an unsupported legacy entry".into(),
                ))
            }
        }
    }
    files.sort();
    Ok(files)
}

fn collect_stable_canvas_files(
    root: &AnchoredRoot,
    scope: &ContentDigest,
) -> Result<Vec<String>, MutationError> {
    let directory = format!("canvas/v1/{}", scope.as_str());
    if !root.directory_exists(&directory)? {
        return Ok(Vec::new());
    }
    let names = root.regular_file_names_bounded(&directory, MAX_CANVAS_ENTRIES)?;
    if names.iter().any(|name| !name.ends_with(".json")) {
        return Err(MutationError::Invalid(
            "stable Canvas namespace contains a non-JSON file".into(),
        ));
    }
    Ok(names
        .into_iter()
        .map(|name| format!("{directory}/{name}"))
        .collect())
}

fn expected_file_destination(source: &str, scope: &ContentDigest) -> Option<String> {
    let history = migration_history_root(scope);
    match source {
        LEGACY_TWIN_ASSIGNMENT => return Some(format!("{history}/twin-legacy-assignment-v1.json")),
        LEGACY_DERIVED_ASSIGNMENT => {
            return Some(format!("{history}/vault-derived-legacy-assignment-v1.json"))
        }
        _ => {}
    }
    let stable_events = format!("twin/events/vaults/v1/{}", scope.as_str());
    for (legacy, stable) in [
        (LEGACY_EVENT_RECORDS, format!("{stable_events}/records/v1")),
        (
            LEGACY_EVENT_QUARANTINE,
            format!("{stable_events}/quarantine/v1"),
        ),
        (LEGACY_EVENT_STAGING, format!("{stable_events}/staging/v1")),
    ] {
        if let Some(suffix) = source.strip_prefix(&format!("{legacy}/")) {
            return Some(format!("{stable}/{suffix}"));
        }
    }
    let leaf = source.strip_prefix("canvas/")?;
    (!leaf.contains('/') && leaf.ends_with(".json"))
        .then(|| format!("canvas/v1/{}/{leaf}", scope.as_str()))
}

fn expected_event_directory_destination(source: &str, scope: &ContentDigest) -> Option<String> {
    let stable_events = format!("twin/events/vaults/v1/{}", scope.as_str());
    match source {
        LEGACY_EVENT_RECORDS => Some(format!("{stable_events}/records/v1")),
        LEGACY_EVENT_QUARANTINE => Some(format!("{stable_events}/quarantine/v1")),
        LEGACY_EVENT_STAGING => Some(format!("{stable_events}/staging/v1")),
        _ => None,
    }
}

fn twin_key(path: &Path) -> Result<String, MutationError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| MutationError::Invalid("Twin namespace name is not UTF-8".into()))?;
    Ok(format!("twin/{name}"))
}

fn is_legacy_fnv_twin_key(key: &str) -> bool {
    key.strip_prefix("twin/").is_some_and(|name| {
        name.len() == 16
            && name
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

fn is_digest_name(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn migration_scope_root(scope: &ContentDigest) -> String {
    format!("{MIGRATION_HISTORY_ROOT}/{}", scope.as_str())
}

fn marker_key(scope: &ContentDigest) -> String {
    format!("{}/marker.json", migration_scope_root(scope))
}

fn migration_history_root(scope: &ContentDigest) -> String {
    format!("{}/history", migration_scope_root(scope))
}

fn require_no_markerless_scope_history(
    root: &AnchoredRoot,
    scope: &ContentDigest,
    marker_exists: bool,
) -> Result<(), MutationError> {
    let scope_root = migration_scope_root(scope);
    if !root.directory_exists(&scope_root)? {
        return Ok(());
    }
    let entries = bounded_entries(root, &scope_root, MAX_COMPONENTS)?;
    let mut unexpected = false;
    for (name, kind) in entries {
        if is_marker_atomic_temp(&name) {
            if kind != AnchoredEntryKind::File {
                return Err(MutationError::Invalid(
                    "stable migration marker temporary is not a regular file".into(),
                ));
            }
            root.delete(&format!("{scope_root}/{name}"))?;
            continue;
        }
        let allowed = marker_exists
            && matches!(
                (name.as_str(), kind),
                ("marker.json", AnchoredEntryKind::File)
                    | ("history", AnchoredEntryKind::Directory)
            );
        unexpected |= !allowed;
    }
    if unexpected {
        let message = if marker_exists {
            "stable migration scope contains unexpected state"
        } else {
            "stable migration has markerless history for this vault scope"
        };
        return Err(MutationError::RecoveryConflict(message.into()));
    }
    Ok(())
}

fn is_marker_atomic_temp(name: &str) -> bool {
    name.strip_prefix('.')
        .and_then(|name| name.strip_suffix(".tmp"))
        .and_then(|uuid| Uuid::parse_str(uuid).ok().map(|parsed| (uuid, parsed)))
        .is_some_and(|(encoded, parsed)| {
            !parsed.is_nil() && parsed.hyphenated().to_string() == encoded
        })
}

fn reject_legacy_global_marker(root: &AnchoredRoot) -> Result<(), MutationError> {
    if entry_kind(root, LEGACY_GLOBAL_MARKER)?.is_some() {
        return Err(MutationError::RecoveryConflict(
            "unscoped stable migration marker is not authoritative".into(),
        ));
    }
    Ok(())
}

fn read_marker(
    root: &AnchoredRoot,
    scope: &ContentDigest,
) -> Result<Option<StableMigrationMarkerV1>, MutationError> {
    let Some(bytes) = root.read_bounded(&marker_key(scope), MARKER_LIMIT)? else {
        return Ok(None);
    };
    let marker = serde_json::from_slice(&bytes).map_err(|error| {
        MutationError::Invalid(format!("invalid stable migration marker: {error}"))
    })?;
    Ok(Some(marker))
}

fn encoded_marker(marker: &StableMigrationMarkerV1) -> Result<Vec<u8>, MutationError> {
    let mut bytes = serde_json::to_vec_pretty(marker)
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() > MARKER_LIMIT {
        return Err(MutationError::Invalid(format!(
            "stable migration marker exceeds its {MARKER_LIMIT}-byte limit"
        )));
    }
    Ok(bytes)
}

fn install_initial_marker(
    root: &AnchoredRoot,
    marker: &StableMigrationMarkerV1,
) -> Result<(), MutationError> {
    root.install_no_clobber(
        &marker_key(&marker.stable_scope),
        &migration_scope_root(&marker.stable_scope),
        &encoded_marker(marker)?,
    )?;
    let durable = read_marker(root, &marker.stable_scope)?.ok_or_else(|| {
        MutationError::RecoveryConflict("initial stable migration marker was not durable".into())
    })?;
    if durable != *marker {
        return Err(MutationError::RecoveryConflict(
            "initial stable migration marker was installed by another owner".into(),
        ));
    }
    Ok(())
}

fn write_marker(
    root: &AnchoredRoot,
    marker: &StableMigrationMarkerV1,
) -> Result<(), MutationError> {
    root.put_atomic(&marker_key(&marker.stable_scope), &encoded_marker(marker)?)
}

fn require_canonical_uuid(value: &str, label: &str) -> Result<(), MutationError> {
    if Uuid::parse_str(value)
        .ok()
        .is_none_or(|uuid| uuid.is_nil() || uuid.hyphenated().to_string() != value)
    {
        return Err(MutationError::Invalid(format!(
            "{label} must be a canonical non-nil UUID"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "stable_migration_tests.rs"]
mod tests;
