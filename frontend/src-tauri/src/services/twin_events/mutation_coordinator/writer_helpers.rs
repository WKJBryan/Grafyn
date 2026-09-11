use super::*;

pub(super) const WRITER_SCHEMA_VERSION: u16 = 1;
pub(super) const WRITER_FILE_LIMIT: u64 = 4096;
pub(super) const WRITER_KEY: &str = "twin/events/writer-v1.json";
pub(super) const WRITER_STAGING_KEY: &str = "twin/events/writer-staging/v1";
const WRITER_EVIDENCE_MAX_DEPTH: usize = 32;
const WRITER_EVIDENCE_MAX_ENTRIES: usize = 8 * 1024;

pub(super) fn reject_missing_writer_for_established_data_root_locked(
    data_root: &crate::services::twin_events::AnchoredRoot,
    process_lock: &CoordinatorProcessLock,
) -> Result<Option<PersistedMutationIdentityProvider>, MutationError> {
    if !process_lock.covers_data_path(data_root.canonical_path())? {
        return Err(MutationError::Invalid(
            "writer identity scan lock belongs to another data root".into(),
        ));
    }
    let identity = PersistedMutationIdentityProvider::load_optional(data_root.canonical_path())?;
    let mut remaining_entries = WRITER_EVIDENCE_MAX_ENTRIES;
    if identity.is_none()
        && writer_aware_established_evidence_exists(data_root, &mut remaining_entries)?
    {
        return Err(MutationError::RecoveryConflict(
            "writer-identity-missing-for-established-data-root".into(),
        ));
    }
    if writer_staging_contains_unrecognized_entry(data_root, &mut remaining_entries)? {
        let reason = if identity.is_none() {
            "writer-identity-missing-for-established-data-root"
        } else {
            "writer-install-staging-contains-unrecognized-entry"
        };
        return Err(MutationError::RecoveryConflict(reason.into()));
    }
    Ok(identity)
}

fn writer_staging_contains_unrecognized_entry(
    data_root: &crate::services::twin_events::AnchoredRoot,
    remaining_entries: &mut usize,
) -> Result<bool, MutationError> {
    if !data_root.directory_exists(WRITER_STAGING_KEY)? {
        return Ok(false);
    }
    let entries = data_root.directory_entries_bounded(WRITER_STAGING_KEY, *remaining_entries)?;
    *remaining_entries -= entries.len();
    for (name, kind) in entries {
        let is_recognized_writer_temp = kind
            == crate::services::twin_events::AnchoredEntryKind::File
            && is_canonical_writer_install_temp(&name);
        if !is_recognized_writer_temp {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn is_canonical_writer_install_temp(name: &str) -> bool {
    let Some(uuid_text) = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    Uuid::parse_str(uuid_text).is_ok_and(|uuid| {
        !uuid.is_nil() && uuid.get_version_num() == 4 && uuid.to_string() == uuid_text
    })
}

fn writer_aware_established_evidence_exists(
    data_root: &crate::services::twin_events::AnchoredRoot,
    remaining_entries: &mut usize,
) -> Result<bool, MutationError> {
    for key in [
        ACTIVE_ROOT_LEASE_KEY,
        crate::services::sync::device::DEVICE_SIGNING_BINDING_KEY,
        "twin/events/content-authority-v1.json",
    ] {
        if data_root
            .read_bounded(key, WRITER_FILE_LIMIT as usize)?
            .is_some()
        {
            return Ok(true);
        }
    }
    for directory in [
        "twin/stable-vault-migrations/v1",
        "twin/events/v1",
        "twin/events/quarantine/v1",
        "twin/events/staging/v1",
        "twin/events/vaults/v1",
        "twin/mutations/pending/v1",
        "twin/mutations/preauthority/v1",
        "twin/mutations/quarantine/v1",
        "twin/mutations/receipts/v1",
    ] {
        if anchored_directory_contains_regular_file(data_root, directory, 0, remaining_entries)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn anchored_directory_contains_regular_file(
    data_root: &crate::services::twin_events::AnchoredRoot,
    directory: &str,
    depth: usize,
    remaining_entries: &mut usize,
) -> Result<bool, MutationError> {
    if !data_root.directory_exists(directory)? {
        return Ok(false);
    }
    if depth >= WRITER_EVIDENCE_MAX_DEPTH {
        return Err(MutationError::Invalid(
            "writer identity evidence tree exceeds its depth limit".into(),
        ));
    }
    let entries = data_root.directory_entries_bounded(directory, *remaining_entries)?;
    *remaining_entries -= entries.len();
    for (name, kind) in entries {
        let child = format!("{directory}/{name}");
        match kind {
            crate::services::twin_events::AnchoredEntryKind::File => return Ok(true),
            crate::services::twin_events::AnchoredEntryKind::Directory => {
                if anchored_directory_contains_regular_file(
                    data_root,
                    &child,
                    depth + 1,
                    remaining_entries,
                )? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
