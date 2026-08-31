use super::{CoordinatorProcessLock, MutationError};
use crate::models::twin_event::ContentDigest;
use crate::services::twin_events::AnchoredRoot;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub(crate) const CUSTOM_MCP_ROOT_BINDING_KEY: &str = "twin/events/custom-mcp-root-binding-v1.json";
const CUSTOM_MCP_ROOT_BINDING_LIMIT: usize = 4096;
const CUSTOM_MCP_ROOT_BINDING_SCHEMA_VERSION: u16 = 1;
const CUSTOM_MCP_ROOT_BINDING_STAGING: &str = "twin/events/staging-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CustomMcpRootBindingV1 {
    pub(crate) schema_version: u16,
    pub(crate) canonical_vault_path: String,
    pub(crate) root_scope: ContentDigest,
}

pub(super) fn load_for_vault_locked(
    data_root: &AnchoredRoot,
    vault_path: &Path,
    process_lock: &CoordinatorProcessLock,
) -> Result<Option<CustomMcpRootBindingV1>, MutationError> {
    require_lock(data_root, process_lock)?;
    let Some(bytes) =
        data_root.read_bounded(CUSTOM_MCP_ROOT_BINDING_KEY, CUSTOM_MCP_ROOT_BINDING_LIMIT)?
    else {
        return Ok(None);
    };
    let binding = parse_binding(&bytes)?;
    if binding.canonical_vault_path != canonical_vault_path(vault_path)? {
        return Err(MutationError::RecoveryConflict(
            "custom MCP root binding does not match the configured vault path and identity".into(),
        ));
    }
    Ok(Some(binding))
}

pub(super) fn verify_or_install_locked(
    data_root: &AnchoredRoot,
    vault_path: &Path,
    root_scope: ContentDigest,
    process_lock: &CoordinatorProcessLock,
    allow_create: bool,
) -> Result<CustomMcpRootBindingV1, MutationError> {
    require_lock(data_root, process_lock)?;
    let expected = CustomMcpRootBindingV1 {
        schema_version: CUSTOM_MCP_ROOT_BINDING_SCHEMA_VERSION,
        canonical_vault_path: canonical_vault_path(vault_path)?,
        root_scope,
    };
    if let Some(bytes) =
        data_root.read_bounded(CUSTOM_MCP_ROOT_BINDING_KEY, CUSTOM_MCP_ROOT_BINDING_LIMIT)?
    {
        let binding = parse_binding(&bytes)?;
        if binding != expected {
            return Err(MutationError::RecoveryConflict(
                "custom MCP root binding does not match the configured vault path and identity"
                    .into(),
            ));
        }
        return Ok(binding);
    }
    if !allow_create {
        return Err(MutationError::RecoveryConflict(
            "stable custom MCP data has no durable vault-path binding".into(),
        ));
    }

    let mut bytes = serde_json::to_vec_pretty(&expected)
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() > CUSTOM_MCP_ROOT_BINDING_LIMIT {
        return Err(MutationError::Invalid(
            "custom MCP root binding exceeds its size limit".into(),
        ));
    }
    data_root.open_directory(CUSTOM_MCP_ROOT_BINDING_STAGING, true)?;
    data_root.install_no_clobber(
        CUSTOM_MCP_ROOT_BINDING_KEY,
        CUSTOM_MCP_ROOT_BINDING_STAGING,
        &bytes,
    )?;
    let durable = data_root
        .read_bounded(CUSTOM_MCP_ROOT_BINDING_KEY, CUSTOM_MCP_ROOT_BINDING_LIMIT)?
        .ok_or_else(|| {
            MutationError::RecoveryConflict(
                "custom MCP root binding is missing after install".into(),
            )
        })?;
    let durable = parse_binding(&durable)?;
    if durable != expected {
        return Err(MutationError::RecoveryConflict(
            "custom MCP root binding install collision".into(),
        ));
    }
    Ok(durable)
}

fn require_lock(
    data_root: &AnchoredRoot,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    if !process_lock.covers_data_path(data_root.canonical_path())? {
        return Err(MutationError::Invalid(
            "custom MCP root binding lock belongs to another data root".into(),
        ));
    }
    Ok(())
}

fn parse_binding(bytes: &[u8]) -> Result<CustomMcpRootBindingV1, MutationError> {
    let binding: CustomMcpRootBindingV1 = serde_json::from_slice(bytes).map_err(|error| {
        MutationError::Invalid(format!("invalid custom MCP root binding: {error}"))
    })?;
    if binding.schema_version != CUSTOM_MCP_ROOT_BINDING_SCHEMA_VERSION {
        return Err(MutationError::Invalid(
            "invalid custom MCP root binding schema version".into(),
        ));
    }
    Ok(binding)
}

fn canonical_vault_path(vault_path: &Path) -> Result<String, MutationError> {
    let canonical = std::fs::canonicalize(vault_path)?;
    canonical
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| MutationError::Invalid("custom MCP vault path must be Unicode".into()))
}
