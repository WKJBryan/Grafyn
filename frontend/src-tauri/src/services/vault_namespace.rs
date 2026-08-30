use crate::models::twin_event::ContentDigest;
use crate::services::twin_events::{
    ActiveMarkdownRootLeaseV1, AnchoredRoot, CoordinatorProcessLock, MutationError,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const READY_SCHEMA_VERSION: u16 = 2;
const LEGACY_ASSIGNMENT_SCHEMA_VERSION: u16 = 2;
const AUTHORITY_SCHEMA_VERSION: u16 = 1;
const MARKER_LIMIT: usize = 4096;
const DERIVED_ROOT: &str = "vault_derived";
const LEGACY_ASSIGNMENT_KEY: &str = "vault_derived/legacy-assignment-v1.json";
const AUTHORITY_GENERATION_KEY: &str = "twin/events/content-authority-v1.json";
const AUTHORITY_STAGING_KEY: &str = "twin/events/staging/v1";
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
    initial_components: Vec<String>,
    moved_components: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorityGenerationV1 {
    schema_version: u16,
    authority_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VaultAuthorityTokenV1 {
    pub(crate) root_scope: ContentDigest,
    pub(crate) lease_epoch_uuid: String,
    pub(crate) authority_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NamespaceReadyV1 {
    schema_version: u16,
    root_scope: ContentDigest,
    lease_epoch_uuid: String,
    authority_generation: u64,
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

pub(crate) fn initialize_authority_locked(
    data_path: &Path,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    require_lock(data_path, process_lock)?;
    initialize_authority_generation(&AnchoredRoot::open(data_path)?)
}

pub(crate) fn invalidate_locked(
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    require_lock(data_path, process_lock)?;
    let root = AnchoredRoot::open(data_path)?;
    let generation = read_authority_generation(&root)?;
    write_ready_marker(
        data_path,
        &VaultAuthorityTokenV1 {
            root_scope: lease.root_scope.clone(),
            lease_epoch_uuid: lease.epoch_uuid.clone(),
            authority_generation: generation.authority_generation,
        },
        false,
    )
}

#[cfg(test)]
pub(crate) fn publish_ready_locked(
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    require_lock(data_path, process_lock)?;
    let token = capture_authority_token_locked(data_path, lease, process_lock)?;
    publish_ready_token_locked(data_path, &token, process_lock)
}

#[cfg(test)]
pub(crate) fn require_ready(
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
) -> Result<(), MutationError> {
    let root = AnchoredRoot::open(data_path)?;
    let generation = read_authority_generation(&root)?;
    require_ready_token(
        &root,
        &VaultAuthorityTokenV1 {
            root_scope: lease.root_scope.clone(),
            lease_epoch_uuid: lease.epoch_uuid.clone(),
            authority_generation: generation.authority_generation,
        },
    )
}

pub(crate) fn capture_authority_token_locked(
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
    process_lock: &CoordinatorProcessLock,
) -> Result<VaultAuthorityTokenV1, MutationError> {
    require_lock(data_path, process_lock)?;
    let root = AnchoredRoot::open(data_path)?;
    let generation = read_authority_generation(&root)?;
    Ok(VaultAuthorityTokenV1 {
        root_scope: lease.root_scope.clone(),
        lease_epoch_uuid: lease.epoch_uuid.clone(),
        authority_generation: generation.authority_generation,
    })
}

pub(crate) fn advance_authority_locked(
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
    process_lock: &CoordinatorProcessLock,
) -> Result<VaultAuthorityTokenV1, MutationError> {
    require_lock(data_path, process_lock)?;
    let root = AnchoredRoot::open(data_path)?;
    let current = read_authority_generation(&root)?;
    let authority_generation = current
        .authority_generation
        .checked_add(1)
        .ok_or_else(|| MutationError::RecoveryConflict("authority-generation-exhausted".into()))?;
    write_ready_marker(
        data_path,
        &VaultAuthorityTokenV1 {
            root_scope: lease.root_scope.clone(),
            lease_epoch_uuid: lease.epoch_uuid.clone(),
            authority_generation: current.authority_generation,
        },
        false,
    )?;
    root.put_atomic(
        AUTHORITY_GENERATION_KEY,
        &encoded_json(&AuthorityGenerationV1 {
            schema_version: AUTHORITY_SCHEMA_VERSION,
            authority_generation,
        })?,
    )?;
    Ok(VaultAuthorityTokenV1 {
        root_scope: lease.root_scope.clone(),
        lease_epoch_uuid: lease.epoch_uuid.clone(),
        authority_generation,
    })
}

pub(crate) fn publish_ready_token_locked(
    data_path: &Path,
    token: &VaultAuthorityTokenV1,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    require_lock(data_path, process_lock)?;
    let root = AnchoredRoot::open(data_path)?;
    if read_authority_generation(&root)?.authority_generation != token.authority_generation {
        return Err(MutationError::RecoveryConflict(
            "vault-derived-generation-changed-before-publish".into(),
        ));
    }
    write_ready_marker(data_path, token, true)
}

pub(crate) fn require_ready_token_locked(
    data_path: &Path,
    token: &VaultAuthorityTokenV1,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    require_lock(data_path, process_lock)?;
    let root = AnchoredRoot::open(data_path)?;
    if read_authority_generation(&root)?.authority_generation != token.authority_generation {
        return Err(MutationError::RecoveryConflict(
            "vault-derived-namespace-not-ready".into(),
        ));
    }
    require_ready_token(&root, token)
}

fn require_ready_token(
    root: &AnchoredRoot,
    token: &VaultAuthorityTokenV1,
) -> Result<(), MutationError> {
    let key = ready_key(&token.root_scope);
    let bytes = root.read_bounded(&key, MARKER_LIMIT)?.ok_or_else(|| {
        MutationError::RecoveryConflict("vault-derived-namespace-not-ready".into())
    })?;
    let marker: NamespaceReadyV1 = serde_json::from_slice(&bytes)
        .map_err(|error| MutationError::Invalid(format!("invalid namespace readiness: {error}")))?;
    if marker.schema_version != READY_SCHEMA_VERSION
        || marker.root_scope != token.root_scope
        || marker.lease_epoch_uuid != token.lease_epoch_uuid
        || marker.authority_generation != token.authority_generation
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
    let token = capture_authority_token_locked(data_path, lease, process_lock)?;
    require_ready_token_locked(data_path, &token, process_lock)
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
        validate_assignment(existing)?;
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
        return resume_legacy_assignment(root, existing, &legacy_exists);
    }

    let initial_components = LEGACY_COMPONENTS
        .iter()
        .zip(&legacy_exists)
        .filter_map(|(component, exists)| exists.then_some((*component).to_string()))
        .collect::<Vec<_>>();
    if initial_components.is_empty() {
        return Ok(());
    }
    let scope_key = scope_key(root_scope);
    if !root.directory_entries(&scope_key)?.is_empty() {
        return Err(MutationError::RecoveryConflict(
            "legacy-derived-state-collision-scoped-namespace".into(),
        ));
    }
    let prepared = LegacyAssignmentV1 {
        schema_version: LEGACY_ASSIGNMENT_SCHEMA_VERSION,
        root_scope: root_scope.clone(),
        state: LegacyAssignmentState::Prepared,
        initial_components,
        moved_components: Vec::new(),
    };
    write_assignment(root, &prepared)?;
    resume_legacy_assignment(root, &prepared, &legacy_exists)
}

fn resume_legacy_assignment(
    root: &AnchoredRoot,
    existing: &LegacyAssignmentV1,
    legacy_exists: &[bool],
) -> Result<(), MutationError> {
    let scope_key = scope_key(&existing.root_scope);
    let mut progress = existing.clone();
    for (component, source_exists) in LEGACY_COMPONENTS.iter().zip(legacy_exists) {
        let was_initial = progress
            .initial_components
            .iter()
            .any(|initial| initial == component);
        let was_moved = progress
            .moved_components
            .iter()
            .any(|moved| moved == component);
        let destination = format!("{scope_key}/{component}");
        let destination_exists = root.directory_exists(&destination)?;
        match (was_initial, was_moved, *source_exists, destination_exists) {
            (false, _, false, _) | (true, true, false, true) => {}
            (false, _, true, _) => {
                return Err(MutationError::RecoveryConflict(
                    "legacy-derived-state-appeared-after-assignment".into(),
                ));
            }
            (true, false, true, false) => {
                root.rename(component, &destination, false)?;
                progress.moved_components.push((*component).to_string());
                write_assignment(root, &progress)?;
            }
            // Rename is durable before its progress write, so infer this exact partial step.
            (true, false, false, true) => {
                progress.moved_components.push((*component).to_string());
                write_assignment(root, &progress)?;
            }
            _ => {
                return Err(MutationError::RecoveryConflict(format!(
                    "legacy-derived-state-collision-{component}"
                )));
            }
        }
    }
    progress.state = LegacyAssignmentState::Committed;
    write_assignment(root, &progress)
}

fn validate_assignment(assignment: &LegacyAssignmentV1) -> Result<(), MutationError> {
    if assignment.schema_version != LEGACY_ASSIGNMENT_SCHEMA_VERSION {
        return Err(MutationError::Invalid(
            "unsupported legacy assignment schema".into(),
        ));
    }
    let known = LEGACY_COMPONENTS
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let initial = assignment
        .initial_components
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let moved = assignment
        .moved_components
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    if initial.len() != assignment.initial_components.len()
        || moved.len() != assignment.moved_components.len()
        || !initial.is_subset(&known)
        || !moved.is_subset(&initial)
        || (assignment.state == LegacyAssignmentState::Committed && moved != initial)
    {
        return Err(MutationError::Invalid(
            "invalid legacy assignment progress".into(),
        ));
    }
    Ok(())
}

fn read_assignment(root: &AnchoredRoot) -> Result<Option<LegacyAssignmentV1>, MutationError> {
    let Some(bytes) = root.read_bounded(LEGACY_ASSIGNMENT_KEY, MARKER_LIMIT)? else {
        return Ok(None);
    };
    let assignment: LegacyAssignmentV1 = serde_json::from_slice(&bytes)
        .map_err(|error| MutationError::Invalid(format!("invalid legacy assignment: {error}")))?;
    validate_assignment(&assignment)?;
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
    token: &VaultAuthorityTokenV1,
    ready: bool,
) -> Result<(), MutationError> {
    let root = AnchoredRoot::open(data_path)?;
    root.open_directory(&scope_key(&token.root_scope), true)?;
    let marker = NamespaceReadyV1 {
        schema_version: READY_SCHEMA_VERSION,
        root_scope: token.root_scope.clone(),
        lease_epoch_uuid: token.lease_epoch_uuid.clone(),
        authority_generation: token.authority_generation,
        ready,
    };
    root.put_atomic(&ready_key(&token.root_scope), &encoded_json(&marker)?)
}

fn initialize_authority_generation(root: &AnchoredRoot) -> Result<(), MutationError> {
    if root
        .read_bounded(AUTHORITY_GENERATION_KEY, MARKER_LIMIT)?
        .is_none()
    {
        root.install_no_clobber(
            AUTHORITY_GENERATION_KEY,
            AUTHORITY_STAGING_KEY,
            &encoded_json(&AuthorityGenerationV1 {
                schema_version: AUTHORITY_SCHEMA_VERSION,
                authority_generation: 0,
            })?,
        )?;
    }
    read_authority_generation(root).map(|_| ())
}

fn read_authority_generation(
    root: &AnchoredRoot,
) -> Result<AuthorityGenerationV1, MutationError> {
    let bytes = root
        .read_bounded(AUTHORITY_GENERATION_KEY, MARKER_LIMIT)?
        .ok_or_else(|| MutationError::Invalid("authority generation is missing".into()))?;
    let generation: AuthorityGenerationV1 = serde_json::from_slice(&bytes)
        .map_err(|error| MutationError::Invalid(format!("invalid authority generation: {error}")))?;
    if generation.schema_version != AUTHORITY_SCHEMA_VERSION {
        return Err(MutationError::Invalid(
            "unsupported authority generation schema".into(),
        ));
    }
    Ok(generation)
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
        initialize_authority_locked(&data, &lock).unwrap();
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

    #[test]
    fn legacy_assignment_rejects_any_scoped_occupancy_before_its_first_move() {
        let (_temp, data, lease) = fixture();
        std::fs::create_dir(data.join("search_index")).unwrap();
        std::fs::write(data.join("search_index/sentinel"), b"legacy").unwrap();
        let scoped = scoped_data_path(&data, &lease.root_scope);
        std::fs::create_dir_all(scoped.join("chunk_index")).unwrap();
        std::fs::write(scoped.join("chunk_index/sentinel"), b"scoped").unwrap();

        let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
        let error = initialize_locked(&data, &lease, &lock).unwrap_err();

        assert!(error.to_string().contains("collision"));
        assert_eq!(
            std::fs::read(data.join("search_index/sentinel")).unwrap(),
            b"legacy"
        );
        assert_eq!(
            std::fs::read(scoped.join("chunk_index/sentinel")).unwrap(),
            b"scoped"
        );
        lock.unlock().unwrap();
    }

    #[test]
    fn authority_generation_invalidates_ready_and_publish_is_exact_cas() {
        let (_temp, data, lease) = fixture();
        let lock = acquire_shared_coordinator_process_lock(&data).unwrap();
        initialize_locked(&data, &lease, &lock).unwrap();
        initialize_authority_locked(&data, &lock).unwrap();
        let generation_zero = capture_authority_token_locked(&data, &lease, &lock).unwrap();
        publish_ready_token_locked(&data, &generation_zero, &lock).unwrap();
        require_ready_token_locked(&data, &generation_zero, &lock).unwrap();

        let generation_one = advance_authority_locked(&data, &lease, &lock).unwrap();
        assert_eq!(generation_one.authority_generation, 1);
        assert!(require_ready_token_locked(&data, &generation_zero, &lock).is_err());
        assert!(publish_ready_token_locked(&data, &generation_zero, &lock).is_err());
        publish_ready_token_locked(&data, &generation_one, &lock).unwrap();
        require_ready_token_locked(&data, &generation_one, &lock).unwrap();
        lock.unlock().unwrap();
    }
}
