use super::*;

pub(super) fn ensure_overlay_target_root(
    data_root: &crate::services::twin_events::AnchoredRoot,
    root_scope: &crate::models::twin_event::ContentDigest,
) -> Result<(), MutationError> {
    data_root.open_directory(
        &format!(
            "vault_derived/v1/{}/vault_migration/overlay/notes",
            root_scope.as_str()
        ),
        true,
    )?;
    Ok(())
}

pub(super) fn markdown_root_scope_for(
    root: &Path,
) -> Result<crate::models::twin_event::ContentDigest, MutationError> {
    root_identity_for_path(root)
}

pub(super) fn root_scope_for_lease(
    root: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
) -> Result<crate::models::twin_event::ContentDigest, MutationError> {
    if lease.is_stable() {
        Ok(crate::services::sync::identity::load_vault_identity(root)?.root_scope)
    } else {
        markdown_root_scope_for(root)
    }
}

pub(crate) fn root_identity_for_path(
    root: &Path,
) -> Result<crate::models::twin_event::ContentDigest, MutationError> {
    crate::services::twin_events::validate_real_directory(root, "trusted Markdown vault root")?;
    let encoded = platform_canonical_path_bytes(root)?;
    let platform: &[u8] = if cfg!(windows) { b"windows" } else { b"unix" };
    let mut scoped = Vec::with_capacity(encoded.len() + 64);
    scoped.extend_from_slice(b"grafyn.root_identity.v1");
    scoped.extend_from_slice(&(platform.len() as u64).to_be_bytes());
    scoped.extend_from_slice(platform);
    scoped.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
    scoped.extend_from_slice(&encoded);
    Ok(crate::services::twin_events::digest_bytes(&scoped))
}

pub(super) fn validate_disjoint_roots(
    markdown: &Path,
    canvas: &Path,
    twin: &Path,
) -> Result<(), MutationError> {
    let roots = [
        ("Markdown", platform_canonical_path_bytes(markdown)?),
        ("Canvas", platform_canonical_path_bytes(canvas)?),
        ("Twin", platform_canonical_path_bytes(twin)?),
    ];
    for left in 0..roots.len() {
        for right in left + 1..roots.len() {
            if physical_paths_overlap(&roots[left].1, &roots[right].1) {
                return Err(MutationError::Invalid(format!(
                    "{} and {} roots overlap",
                    roots[left].0, roots[right].0
                )));
            }
        }
    }
    Ok(())
}

fn physical_paths_overlap(left: &[u8], right: &[u8]) -> bool {
    fn is_ancestor(ancestor: &[u8], descendant: &[u8]) -> bool {
        descendant.starts_with(ancestor)
            && (descendant.len() == ancestor.len()
                || descendant
                    .get(ancestor.len())
                    .is_some_and(|separator| *separator == b'/' || *separator == b'\\'))
    }
    is_ancestor(left, right) || is_ancestor(right, left)
}

#[cfg(unix)]
pub(super) fn platform_canonical_path_bytes(path: &Path) -> Result<Vec<u8>, MutationError> {
    use std::os::unix::ffi::OsStrExt;
    Ok(fs::canonicalize(path)?.as_os_str().as_bytes().to_vec())
}

#[cfg(windows)]
pub(super) fn platform_canonical_path_bytes(path: &Path) -> Result<Vec<u8>, MutationError> {
    let canonical = fs::canonicalize(path)?;
    let encoded = canonical
        .to_str()
        .ok_or_else(|| MutationError::Invalid("canonical root path must be Unicode".into()))?;
    let normalized = encoded
        .strip_prefix(r"\\?\")
        .unwrap_or(encoded)
        .replace('/', "\\")
        .to_lowercase();
    Ok(normalized.into_bytes())
}

pub(super) const ACTIVE_ROOT_LEASE_KEY: &str = "twin/events/active-markdown-root-v1.json";

pub(super) fn validate_or_prepare_active_root_lease(
    existing: Option<ActiveMarkdownRootLeaseV1>,
    expected_scope: crate::models::twin_event::ContentDigest,
    legacy_scope: crate::models::twin_event::ContentDigest,
    identity_mode: RootIdentityMode,
    bootstrap_schema_one: bool,
) -> Result<(ActiveMarkdownRootLeaseV1, bool), MutationError> {
    if let Some(lease) = existing {
        let expected = match (identity_mode, lease.schema_version) {
            (RootIdentityMode::LegacyPath, ACTIVE_ROOT_LEASE_SCHEMA_VERSION) => &expected_scope,
            (RootIdentityMode::StableVault, ACTIVE_ROOT_LEASE_SCHEMA_VERSION) => &legacy_scope,
            (RootIdentityMode::StableVault, STABLE_ROOT_LEASE_SCHEMA_VERSION) => &expected_scope,
            (RootIdentityMode::LegacyPath, STABLE_ROOT_LEASE_SCHEMA_VERSION) => {
                return Err(MutationError::RecoveryConflict(
                    "stable-vault-authority-requires-stable-bootstrap".into(),
                ))
            }
            _ => unreachable!("active lease parser rejects unknown schemas"),
        };
        if &lease.root_scope != expected {
            return Err(MutationError::RecoveryConflict(
                "configured-markdown-root-is-not-active".into(),
            ));
        }
        return Ok((lease, false));
    }
    let lease = match identity_mode {
        RootIdentityMode::LegacyPath => ActiveMarkdownRootLeaseV1::new(expected_scope),
        RootIdentityMode::StableVault if bootstrap_schema_one => {
            ActiveMarkdownRootLeaseV1::new(legacy_scope)
        }
        RootIdentityMode::StableVault => ActiveMarkdownRootLeaseV1::new_stable(expected_scope),
    };
    Ok((lease, true))
}

pub(super) fn load_optional_active_root_lease(
    data_root: &crate::services::twin_events::AnchoredRoot,
) -> Result<Option<ActiveMarkdownRootLeaseV1>, MutationError> {
    data_root
        .read_bounded(ACTIVE_ROOT_LEASE_KEY, ACTIVE_ROOT_LEASE_LIMIT as usize)?
        .map(|bytes| parse_active_root_lease(&bytes))
        .transpose()
}

pub(super) fn load_active_root_lease(
    data_root: &crate::services::twin_events::AnchoredRoot,
) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
    load_optional_active_root_lease(data_root)?
        .ok_or_else(|| MutationError::Invalid("active Markdown root lease is missing".into()))
}

pub(super) fn parse_active_root_lease(
    bytes: &[u8],
) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
    let lease: ActiveMarkdownRootLeaseV1 = serde_json::from_slice(bytes)
        .map_err(|error| MutationError::Invalid(format!("invalid root lease: {error}")))?;
    if !matches!(
        lease.schema_version,
        ACTIVE_ROOT_LEASE_SCHEMA_VERSION | STABLE_ROOT_LEASE_SCHEMA_VERSION
    ) || !is_canonical_non_nil_uuid(&lease.epoch_uuid)
    {
        return Err(MutationError::Invalid(
            "invalid active Markdown root lease".into(),
        ));
    }
    Ok(lease)
}

pub(super) fn write_active_root_lease(
    data_root: &crate::services::twin_events::AnchoredRoot,
    lease: &ActiveMarkdownRootLeaseV1,
) -> Result<(), MutationError> {
    if !matches!(
        lease.schema_version,
        ACTIVE_ROOT_LEASE_SCHEMA_VERSION | STABLE_ROOT_LEASE_SCHEMA_VERSION
    ) || !is_canonical_non_nil_uuid(&lease.epoch_uuid)
    {
        return Err(MutationError::Invalid(
            "invalid active Markdown root lease".into(),
        ));
    }
    let mut bytes = serde_json::to_vec_pretty(lease)
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() > ACTIVE_ROOT_LEASE_LIMIT as usize {
        return Err(MutationError::Invalid(
            "active Markdown root lease exceeds its size limit".into(),
        ));
    }
    data_root.put_atomic(ACTIVE_ROOT_LEASE_KEY, &bytes)
}

fn is_canonical_non_nil_uuid(value: &str) -> bool {
    Uuid::parse_str(value)
        .is_ok_and(|uuid| !uuid.is_nil() && uuid.hyphenated().to_string() == value)
}
