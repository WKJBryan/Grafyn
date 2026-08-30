use crate::models::twin_event::ContentDigest;
use crate::services::twin_events::{
    ActiveMarkdownRootLeaseV1, AnchoredRoot, CoordinatorProcessLock, MutationError,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const NAMESPACE_SCHEMA_VERSION: u16 = 1;
const MARKER_LIMIT: usize = 4096;
const DERIVED_ROOT: &str = "vault_derived";
const LEGACY_ASSIGNMENT_KEY: &str = "vault_derived/legacy-assignment-v1.json";
const LEGACY_COMPONENTS: [&str; 4] = [
    "search_index",
    "chunk_index",
    "link_discovery",
    "vault_migration",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyAssignmentState {
    Prepared,
    Committed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyAssignmentV1 {
    schema_version: u16,
    root_scope: ContentDigest,
    state: LegacyAssignmentState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NamespaceReadyV1 {
    schema_version: u16,
    root_scope: ContentDigest,
    lease_epoch_uuid: String,
    ready: bool,
}

pub(crate) fn scoped_data_path(data_path: &Path, root_scope: &ContentDigest) -> PathBuf {
    data_path
        .join(DERIVED_ROOT)
        .join("v1")
        .join(root_scope.as_str())
}

pub(crate) fn initialize_locked(
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
    process_lock: &CoordinatorProcessLock,
) -> Result<PathBuf, MutationError> {
    require_lock(data_path, process_lock)?;
    let root = AnchoredRoot::open(data_path)?;
    root.open_directory(DERIVED_ROOT, true)?;
    root.open_directory("vault_derived/v1", true)?;
    let scope_key = scope_key(&lease.root_scope);
    root.open_directory(&scope_key, true)?;
    assign_legacy_once(&root, &lease.root_scope)?;
    Ok(scoped_data_path(data_path, &lease.root_scope))
}

pub(crate) fn invalidate_locked(
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    require_lock(data_path, process_lock)?;
    let root = AnchoredRoot::open(data_path)?;
    root.delete(&ready_key(&lease.root_scope))
}

pub(crate) fn publish_ready_locked(
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    require_lock(data_path, process_lock)?;
    write_ready_marker(data_path, lease, true)
}

pub(crate) fn require_ready(
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
) -> Result<(), MutationError> {
    let root = AnchoredRoot::open(data_path)?;
    let key = ready_key(&lease.root_scope);
    let bytes = root.read_bounded(&key, MARKER_LIMIT)?.ok_or_else(|| {
        MutationError::RecoveryConflict("vault-derived-namespace-not-ready".into())
    })?;
    let marker: NamespaceReadyV1 = serde_json::from_slice(&bytes)
        .map_err(|error| MutationError::Invalid(format!("invalid namespace readiness: {error}")))?;
    if marker.schema_version != NAMESPACE_SCHEMA_VERSION
        || marker.root_scope != lease.root_scope
        || marker.lease_epoch_uuid != lease.epoch_uuid
        || !marker.ready
    {
        return Err(MutationError::RecoveryConflict(
            "vault-derived-namespace-not-ready".into(),
        ));
    }
    Ok(())
}

#[cfg(feature = "mcp")]
pub(crate) fn require_ready_locked(
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    require_lock(data_path, process_lock)?;
    require_ready(data_path, lease)
}

fn require_lock(
    data_path: &Path,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    if !process_lock.covers_data_path(data_path)? {
        return Err(MutationError::Invalid(
            "vault namespace lock token belongs to another data root".into(),
        ));
    }
    Ok(())
}

fn assign_legacy_once(
    root: &AnchoredRoot,
    root_scope: &ContentDigest,
) -> Result<(), MutationError> {
    let assignment = read_assignment(root)?;
    let legacy_exists = LEGACY_COMPONENTS
        .iter()
        .map(|component| root.directory_exists(component))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(existing) = &assignment {
        if existing.root_scope != *root_scope {
            if legacy_exists.iter().any(|exists| *exists) {
                return Err(MutationError::RecoveryConflict(
                    "legacy-derived-state-belongs-to-another-root".into(),
                ));
            }
            return Ok(());
        }
        if existing.state == LegacyAssignmentState::Committed {
            if legacy_exists.iter().any(|exists| *exists) {
                return Err(MutationError::RecoveryConflict(
                    "legacy-derived-state-appeared-after-assignment".into(),
                ));
            }
            return Ok(());
        }
    }

    let scope_key = scope_key(root_scope);
    for (component, source_exists) in LEGACY_COMPONENTS.iter().zip(&legacy_exists) {
        let destination = format!("{scope_key}/{component}");
        if *source_exists && root.directory_exists(&destination)? {
            return Err(MutationError::RecoveryConflict(format!(
                "legacy-derived-state-collision-{component}"
            )));
        }
    }

    if assignment.is_none() {
        write_assignment(
            root,
            &LegacyAssignmentV1 {
                schema_version: NAMESPACE_SCHEMA_VERSION,
                root_scope: root_scope.clone(),
                state: LegacyAssignmentState::Prepared,
            },
        )?;
    }
    for (component, source_exists) in LEGACY_COMPONENTS.iter().zip(legacy_exists) {
        if source_exists {
            root.rename(component, &format!("{scope_key}/{component}"), false)?;
        }
    }
    write_assignment(
        root,
        &LegacyAssignmentV1 {
            schema_version: NAMESPACE_SCHEMA_VERSION,
            root_scope: root_scope.clone(),
            state: LegacyAssignmentState::Committed,
        },
    )
}

fn read_assignment(root: &AnchoredRoot) -> Result<Option<LegacyAssignmentV1>, MutationError> {
    let Some(bytes) = root.read_bounded(LEGACY_ASSIGNMENT_KEY, MARKER_LIMIT)? else {
        return Ok(None);
    };
    let assignment: LegacyAssignmentV1 = serde_json::from_slice(&bytes)
        .map_err(|error| MutationError::Invalid(format!("invalid legacy assignment: {error}")))?;
    if assignment.schema_version != NAMESPACE_SCHEMA_VERSION {
        return Err(MutationError::Invalid(
            "unsupported legacy assignment schema".into(),
        ));
    }
    Ok(Some(assignment))
}

fn write_assignment(
    root: &AnchoredRoot,
    assignment: &LegacyAssignmentV1,
) -> Result<(), MutationError> {
    let bytes = encoded_json(assignment)?;
    root.put_atomic(LEGACY_ASSIGNMENT_KEY, &bytes)
}

fn write_ready_marker(
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
    ready: bool,
) -> Result<(), MutationError> {
    let root = AnchoredRoot::open(data_path)?;
    root.open_directory(&scope_key(&lease.root_scope), true)?;
    let marker = NamespaceReadyV1 {
        schema_version: NAMESPACE_SCHEMA_VERSION,
        root_scope: lease.root_scope.clone(),
        lease_epoch_uuid: lease.epoch_uuid.clone(),
        ready,
    };
    root.put_atomic(&ready_key(&lease.root_scope), &encoded_json(&marker)?)
}

fn encoded_json<T: Serialize>(value: &T) -> Result<Vec<u8>, MutationError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() > MARKER_LIMIT {
        return Err(MutationError::Invalid(
            "vault namespace marker exceeds its size limit".into(),
        ));
    }
    Ok(bytes)
}

fn scope_key(root_scope: &ContentDigest) -> String {
    format!("vault_derived/v1/{}", root_scope.as_str())
}

fn ready_key(root_scope: &ContentDigest) -> String {
    format!("{}/ready-v1.json", scope_key(root_scope))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::twin_events::{
        acquire_shared_coordinator_process_lock, root_identity_for_path,
    };

    fn fixture() -> (tempfile::TempDir, PathBuf, ActiveMarkdownRootLeaseV1) {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir_all(data.join("twin/events")).unwrap();
        let lease = ActiveMarkdownRootLeaseV1::new(root_identity_for_path(&vault).unwrap());
        (temp, data, lease)
    }

    #[test]
    fn scopes_use_disjoint_full_digest_paths_and_readiness_is_epoch_bound() {
        let (temp, data, lease_a) = fixture();
        let vault_b = temp.path().join("vault-b");
        std::fs::create_dir(&vault_b).unwrap();
        let lease_b = ActiveMarkdownRootLeaseV1::new(root_identity_for_path(&vault_b).unwrap());
        let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
        let path_a = initialize_locked(&data, &lease_a, &lock).unwrap();
        let path_b = initialize_locked(&data, &lease_b, &lock).unwrap();
        assert_ne!(path_a, path_b);
        assert_eq!(path_a.file_name().unwrap(), lease_a.root_scope.as_str());
        assert_eq!(path_b.file_name().unwrap(), lease_b.root_scope.as_str());
        invalidate_locked(&data, &lease_a, &lock).unwrap();
        assert!(require_ready(&data, &lease_a).is_err());
        publish_ready_locked(&data, &lease_a, &lock).unwrap();
        require_ready(&data, &lease_a).unwrap();
        let next_epoch = ActiveMarkdownRootLeaseV1::new(lease_a.root_scope.clone());
        assert!(require_ready(&data, &next_epoch).is_err());
        lock.unlock().unwrap();
    }

    #[test]
    fn legacy_state_is_assigned_once_without_cross_root_merge() {
        let (temp, data, lease_a) = fixture();
        let vault_b = temp.path().join("vault-b");
        std::fs::create_dir(&vault_b).unwrap();
        let lease_b = ActiveMarkdownRootLeaseV1::new(root_identity_for_path(&vault_b).unwrap());
        std::fs::create_dir(data.join("search_index")).unwrap();
        std::fs::write(data.join("search_index/sentinel"), b"root-a").unwrap();
        let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
        let path_a = initialize_locked(&data, &lease_a, &lock).unwrap();
        assert_eq!(
            std::fs::read(path_a.join("search_index/sentinel")).unwrap(),
            b"root-a"
        );
        initialize_locked(&data, &lease_b, &lock).unwrap();
        std::fs::create_dir(data.join("chunk_index")).unwrap();
        std::fs::write(data.join("chunk_index/sentinel"), b"late-legacy").unwrap();
        assert!(initialize_locked(&data, &lease_b, &lock).is_err());
        assert_eq!(
            std::fs::read(data.join("chunk_index/sentinel")).unwrap(),
            b"late-legacy"
        );
        lock.unlock().unwrap();
    }
}
