use super::*;

const MAX_MIGRATION_INPUTS: usize = 50_000;
// Full immutable-plan replay happens on every durable progress publication.
// Aligning with the coordinator's bounded inventory limits either direction to
// 1,536 normal progress publications. Even apply + rollback + the separately
// bounded authority-only recoveries stay below 4,700 full-record publications.
pub(super) const MAX_MIGRATION_OPERATIONS: usize = 256;
pub(super) const MAX_MIGRATION_AUTHORITY_ONLY_ADVANCES: u64 = 256;

pub(super) fn validate_operation_sequence(operations: &[MigrationOperationV1]) -> Result<()> {
    if operations.len() > MAX_MIGRATION_OPERATIONS {
        anyhow::bail!("migration operation inventory is too large");
    }
    let mut targets = std::collections::BTreeSet::new();
    for (index, operation) in operations.iter().enumerate() {
        if operation.index != index {
            anyhow::bail!("migration operation indices are not contiguous");
        }
        crate::services::twin_events::validate_target_key(
            operation.target_kind,
            &operation.target_key,
        )
        .map_err(anyhow::Error::new)?;
        if !targets.insert((
            operation.target_kind,
            migration_path_key(&operation.target_key),
        )) {
            anyhow::bail!("duplicate migration operation target");
        }
        match (&operation.after, operation.after_blob_key.as_ref()) {
            (crate::services::twin_events::BeforeImage::Sha256(digest), Some(_))
                if digest == &operation.after_digest => {}
            _ => anyhow::bail!("migration operation after-image is invalid"),
        }
    }
    Ok(())
}

pub(super) fn require_unique_bounded(values: &[String], label: &str) -> Result<()> {
    if values.len() > MAX_MIGRATION_OPERATIONS
        || values.iter().any(|value| value.is_empty())
        || values.iter().collect::<HashSet<_>>().len() != values.len()
    {
        anyhow::bail!("migration {label} are invalid");
    }
    Ok(())
}

pub(super) fn validate_current_preview(
    preview: &MarkdownMigrationPreview,
    preview_id: &str,
) -> Result<()> {
    if preview.preview_id != preview_id
        || preview.schema_version != MIGRATION_PREVIEW_SCHEMA_VERSION
        || preview.root_scope.is_none()
        || preview.authority.is_none()
        || preview.request.is_none()
        || preview.created_at.is_none()
        || preview.expected_program_target.is_none()
        || preview.program_after_digest.is_none()
    {
        anyhow::bail!("migration preview exact fields are invalid");
    }
    let authority = preview_authority_token(preview)?;
    if preview.root_scope.as_ref() != Some(&authority.root_scope) {
        anyhow::bail!("migration preview root authority is inconsistent");
    }
    let program = canonical_program_path(&preview.program_path)?;
    let hub = canonical_hub_folder(&preview.hub_folder)?;
    if program != preview.program_path || hub != preview.hub_folder {
        anyhow::bail!("migration preview target paths are not canonical");
    }
    crate::services::twin_events::validate_target_key(
        crate::services::twin_events::TargetKind::Markdown,
        &program,
    )
    .map_err(anyhow::Error::new)?;
    crate::services::twin_events::validate_target_key(
        crate::services::twin_events::TargetKind::Markdown,
        &format!("{hub}/probe.md"),
    )
    .map_err(anyhow::Error::new)?;
    let request = preview.request.as_ref().expect("validated request");
    if &canonical_migration_request(request, &hub, &program) != request
        || request.mode != preview.mode
        || request.hub_folder.as_deref() != Some(preview.hub_folder.as_str())
        || request.program_path.as_deref() != Some(preview.program_path.as_str())
    {
        anyhow::bail!("migration preview request is inconsistent");
    }
    validate_sorted_inventory(preview)?;
    let observed_program = preview
        .source_inventory
        .iter()
        .find(|source| migration_path_key(&source.relative_path) == migration_path_key(&program));
    match (observed_program, preview.expected_program_target.as_ref()) {
        (None, Some(ExpectedProgramTarget::Absent)) => {}
        (Some(source), Some(ExpectedProgramTarget::Present { digest }))
            if source.note_id.is_none() && &source.markdown_digest == digest => {}
        _ => anyhow::bail!("migration preview program state is inconsistent"),
    }
    if preview.program_after_digest.as_ref()
        != Some(&crate::services::twin_events::digest_bytes(
            default_program_file_contents(&hub, &program).as_bytes(),
        ))
    {
        anyhow::bail!("migration preview program after-image is inconsistent");
    }
    Ok(())
}

fn validate_sorted_inventory(preview: &MarkdownMigrationPreview) -> Result<()> {
    if preview.source_inventory.len() > MAX_MIGRATION_INPUTS
        || preview.overlay_inventory.len() > MAX_MIGRATION_INPUTS
    {
        anyhow::bail!("migration preview inventory is too large");
    }
    let mut note_ids = HashSet::new();
    let mut previous = None;
    for source in &preview.source_inventory {
        let key = migration_path_key(&source.relative_path);
        if previous.as_ref().is_some_and(|old| old >= &key) {
            anyhow::bail!("migration Markdown inventory is not strictly sorted");
        }
        previous = Some(key);
        if let Some(note_id) = source.note_id.as_ref() {
            if !note_ids.insert(note_id.clone()) {
                anyhow::bail!("migration Markdown inventory has duplicate note IDs");
            }
        }
    }
    let mut overlay_by_path = HashMap::new();
    let mut previous = None;
    for overlay in &preview.overlay_inventory {
        let key = migration_path_key(&overlay.relative_path);
        if previous.as_ref().is_some_and(|old| old >= &key)
            || overlay_by_path.insert(key.clone(), overlay).is_some()
        {
            anyhow::bail!("migration overlay inventory is not strictly sorted");
        }
        previous = Some(key);
    }
    for source in &preview.source_inventory {
        if let Some(note_id) = source.note_id.as_ref() {
            let overlay_key = migration_path_key(&format!("{note_id}.json"));
            match (&source.overlay, overlay_by_path.get(&overlay_key)) {
                (crate::models::migration::MigrationSourceStateV1::Absent, None) => {}
                (
                    crate::models::migration::MigrationSourceStateV1::Present { digest, byte_len },
                    Some(overlay),
                ) if digest == &overlay.digest && byte_len == &overlay.byte_len => {}
                _ => anyhow::bail!("migration note overlay state is inconsistent"),
            }
        }
    }
    Ok(())
}

pub(super) fn preview_authority_token(
    preview: &MarkdownMigrationPreview,
) -> Result<crate::services::vault_namespace::VaultAuthorityTokenV1> {
    let authority = preview
        .authority
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("legacy migration preview is audit-only"))?;
    let lease = Uuid::parse_str(&authority.lease_epoch_uuid)
        .context("migration preview lease UUID is invalid")?;
    if lease.is_nil() || lease.to_string() != authority.lease_epoch_uuid {
        anyhow::bail!("migration preview lease UUID is not canonical");
    }
    Ok(crate::services::vault_namespace::VaultAuthorityTokenV1 {
        root_scope: authority.root_scope.clone(),
        lease_epoch_uuid: authority.lease_epoch_uuid.clone(),
        authority_generation: authority.authority_generation,
    })
}

pub(super) fn validate_manifest_authority(
    authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    root_scope: &crate::models::twin_event::ContentDigest,
) -> Result<()> {
    let lease = Uuid::parse_str(&authority.lease_epoch_uuid)
        .context("migration manifest lease UUID is invalid")?;
    if authority.root_scope != *root_scope
        || lease.is_nil()
        || lease.to_string() != authority.lease_epoch_uuid
    {
        anyhow::bail!("migration manifest authority identity is invalid");
    }
    Ok(())
}

pub(super) fn validate_canonical_uuid(value: &str, label: &str) -> Result<()> {
    let parsed = Uuid::parse_str(value).with_context(|| format!("{label} is invalid"))?;
    if parsed.is_nil() || parsed.to_string() != value {
        anyhow::bail!("{label} is not canonical");
    }
    Ok(())
}

pub(super) fn require_exact_fields(value: &Value, fields: &[&str]) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("migration record is not an object"))?;
    if object.len() != fields.len()
        || fields.iter().any(|field| !object.contains_key(*field))
        || object.keys().any(|key| !fields.contains(&key.as_str()))
    {
        anyhow::bail!("migration record has missing or unknown current-schema fields");
    }
    Ok(())
}
