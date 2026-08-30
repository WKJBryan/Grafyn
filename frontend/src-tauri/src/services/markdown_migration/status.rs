use super::*;

const MAX_MIGRATION_RUNS: usize = 50_000;
pub(super) const MAX_MIGRATION_STATUS_SCAN_BYTES: usize = 256 * 1024 * 1024;

enum StatusPreviewRecord {
    Current(MarkdownMigrationPreview),
    Legacy(MarkdownMigrationPreview),
}

enum StatusManifestRecord {
    Missing,
    Current(StoredManifest),
    Legacy,
}

impl MarkdownMigrationService {
    pub(crate) fn status_transaction(
        &self,
        run_id: Option<&str>,
        expected_authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<MarkdownMigrationStatus> {
        let migration_root = self.strict_root()?;
        let _transaction_lock = migration_root
            .lock_exclusive("runs/transaction.lock")
            .map_err(anyhow::Error::new)?;
        let target_id = match run_id {
            Some(run_id) => {
                super::validation::validate_canonical_uuid(run_id, "migration run ID")?;
                Some(run_id.to_string())
            }
            None => self.latest_strict_run_for_scope(&expected_authority.root_scope)?,
        };
        let Some(target_id) = target_id else {
            return Ok(MarkdownMigrationStatus {
                status: "idle".into(),
                ..Default::default()
            });
        };
        match self.load_status_preview(&target_id)? {
            StatusPreviewRecord::Legacy(preview) => {
                if preview
                    .root_scope
                    .as_ref()
                    .is_some_and(|scope| scope != &expected_authority.root_scope)
                {
                    anyhow::bail!("legacy migration preview belongs to another vault scope");
                }
                Ok(MarkdownMigrationStatus {
                    run_id: Some(target_id.clone()),
                    preview_id: Some(target_id),
                    status: "legacy_audit_only".into(),
                    mode: Some(preview.mode),
                    created_at: preview.created_at,
                    rollback_available: false,
                    summary: Some(preview.summary),
                    ..Default::default()
                })
            }
            StatusPreviewRecord::Current(preview) => {
                if preview.root_scope.as_ref() != Some(&expected_authority.root_scope) {
                    anyhow::bail!("migration run belongs to another vault scope");
                }
                match self.load_status_manifest(&target_id)? {
                    StatusManifestRecord::Missing => Ok(MarkdownMigrationStatus {
                        run_id: Some(target_id.clone()),
                        preview_id: Some(target_id),
                        status: "previewed".into(),
                        mode: Some(preview.mode),
                        created_at: preview.created_at,
                        rollback_available: false,
                        summary: Some(preview.summary),
                        ..Default::default()
                    }),
                    StatusManifestRecord::Legacy => Ok(MarkdownMigrationStatus {
                        run_id: Some(target_id.clone()),
                        preview_id: Some(target_id),
                        status: "legacy_manifest_audit_only".into(),
                        mode: Some(preview.mode),
                        created_at: preview.created_at,
                        rollback_available: false,
                        summary: Some(preview.summary),
                        ..Default::default()
                    }),
                    StatusManifestRecord::Current(manifest) => {
                        if manifest.root_scope.as_ref() != Some(&expected_authority.root_scope) {
                            anyhow::bail!("migration manifest belongs to another vault scope");
                        }
                        let rollback_available = manifest.apply_next > manifest.rollback_next
                            && manifest.final_authority.as_ref() == Some(expected_authority)
                            && matches!(
                                manifest.status.as_str(),
                                "applied"
                                    | "apply_partial"
                                    | "rollback_prepared"
                                    | "rollback_partial"
                            );
                        Ok(MarkdownMigrationStatus {
                            run_id: Some(target_id.clone()),
                            preview_id: Some(target_id),
                            status: manifest.status,
                            mode: Some(preview.mode),
                            created_at: preview.created_at,
                            applied_at: manifest.applied_at,
                            rollback_available,
                            summary: Some(preview.summary),
                        })
                    }
                }
            }
        }
    }

    fn latest_strict_run_for_scope(
        &self,
        expected_root_scope: &crate::models::twin_event::ContentDigest,
    ) -> Result<Option<String>> {
        let entries = self
            .strict_root()?
            .directory_entries("runs")
            .map_err(anyhow::Error::new)?;
        if entries.len() > MAX_MIGRATION_RUNS + 1 {
            anyhow::bail!("migration run directory exceeds its bounded inventory");
        }
        let mut newest: Option<(DateTime<Utc>, String)> = None;
        let mut scanned_bytes = 0usize;
        for (name, kind) in entries {
            if kind == crate::services::twin_events::AnchoredEntryKind::File
                && name == "transaction.lock"
            {
                continue;
            }
            if kind != crate::services::twin_events::AnchoredEntryKind::Directory {
                anyhow::bail!("migration run inventory contains an unsupported entry");
            }
            if super::validation::validate_canonical_uuid(&name, "migration run ID").is_err() {
                continue;
            }
            let (record, record_bytes) = self.load_status_preview_with_size(&name)?;
            scanned_bytes = scanned_bytes
                .checked_add(record_bytes)
                .filter(|total| *total <= self.status_scan_byte_limit())
                .ok_or_else(|| {
                    anyhow::anyhow!("migration status scan exceeds its aggregate byte limit")
                })?;
            let preview = match record {
                StatusPreviewRecord::Legacy(preview) | StatusPreviewRecord::Current(preview) => {
                    preview
                }
            };
            if preview.root_scope.as_ref() != Some(expected_root_scope) {
                continue;
            }
            let created_at = preview
                .created_at
                .ok_or_else(|| anyhow::anyhow!("migration preview timestamp is missing"))?;
            if newest
                .as_ref()
                .is_none_or(|(current, id)| (created_at, &name) > (*current, id))
            {
                newest = Some((created_at, name));
            }
        }
        Ok(newest.map(|(_, id)| id))
    }

    fn load_status_preview(&self, run_id: &str) -> Result<StatusPreviewRecord> {
        self.load_status_preview_with_size(run_id)
            .map(|(record, _)| record)
    }

    fn load_status_preview_with_size(&self, run_id: &str) -> Result<(StatusPreviewRecord, usize)> {
        super::validation::validate_canonical_uuid(run_id, "migration run ID")?;
        let bytes = self
            .strict_root()?
            .read_bounded(
                &format!("runs/{run_id}/preview.json"),
                MAX_MIGRATION_RECORD_BYTES,
            )
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow::anyhow!("migration preview does not exist"))?;
        let value: Value = serde_json::from_slice(&bytes)?;
        let schema = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if schema == u64::from(MIGRATION_PREVIEW_SCHEMA_VERSION) {
            return self
                .load_strict_preview(run_id)
                .map(|preview| (StatusPreviewRecord::Current(preview), bytes.len()));
        }
        if [
            "authority",
            "request",
            "source_inventory",
            "overlay_inventory",
        ]
        .iter()
        .any(|field| value.get(*field).is_some())
        {
            anyhow::bail!("migration preview has current fields with a legacy schema");
        }
        let preview: MarkdownMigrationPreview = serde_json::from_value(value)?;
        if preview.preview_id != run_id {
            anyhow::bail!("legacy migration preview identity mismatch");
        }
        Ok((StatusPreviewRecord::Legacy(preview), bytes.len()))
    }

    fn load_status_manifest(&self, run_id: &str) -> Result<StatusManifestRecord> {
        super::validation::validate_canonical_uuid(run_id, "migration run ID")?;
        let Some(bytes) = self
            .strict_root()?
            .read_bounded(
                &format!("runs/{run_id}/manifest.json"),
                MAX_MIGRATION_RECORD_BYTES,
            )
            .map_err(anyhow::Error::new)?
        else {
            return Ok(StatusManifestRecord::Missing);
        };
        let value: Value = serde_json::from_slice(&bytes)?;
        let schema = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if schema == u64::from(MIGRATION_MANIFEST_SCHEMA_VERSION) {
            return self
                .load_strict_manifest(run_id)
                .map(StatusManifestRecord::Current);
        }
        if [
            "request",
            "starting_authority",
            "final_authority",
            "operations",
            "apply_next",
            "rollback_next",
            "rollback_total",
            "active_step",
            "last_commit",
        ]
        .iter()
        .any(|field| value.get(*field).is_some())
        {
            anyhow::bail!("migration manifest has current fields with a legacy schema");
        }
        Ok(StatusManifestRecord::Legacy)
    }

    fn status_scan_byte_limit(&self) -> usize {
        #[cfg(test)]
        {
            return self
                .status_scan_byte_limit
                .load(std::sync::atomic::Ordering::SeqCst);
        }
        #[cfg(not(test))]
        {
            MAX_MIGRATION_STATUS_SCAN_BYTES
        }
    }
}
