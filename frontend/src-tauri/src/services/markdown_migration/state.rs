use super::*;

const CURRENT_MANIFEST_FIELDS: &[&str] = &[
    "schema_version",
    "run_id",
    "preview_id",
    "root_scope",
    "vault_path",
    "mode",
    "created_at",
    "applied_at",
    "status",
    "created_files",
    "backup_files",
    "overlay_note_ids",
    "touched_note_ids",
    "created_hub_note_ids",
    "skipped_fallback_note_ids",
    "expected_program_target",
    "program_after_digest",
    "program_path",
    "request",
    "source_inventory",
    "overlay_inventory",
    "starting_authority",
    "final_authority",
    "operations",
    "apply_next",
    "rollback_next",
    "rollback_total",
    "authority_only_advances",
    "active_step",
    "last_commit",
];
const CURRENT_COMMIT_RECORD_FIELDS: &[&str] = &[
    "schema_version",
    "run_id",
    "mutation_id",
    "authority",
    "direction",
    "operation_index",
    "outcome",
];
const MAX_MIGRATION_COMMIT_RECORD_BYTES: usize = 64 * 1024;

impl MarkdownMigrationService {
    fn blob_byte_limit(&self) -> usize {
        #[cfg(test)]
        {
            return self
                .blob_byte_limit
                .load(std::sync::atomic::Ordering::SeqCst);
        }
        #[cfg(not(test))]
        {
            super::transaction::MAX_MIGRATION_BLOB_BYTES
        }
    }

    pub(super) fn verify_manifest_blobs(&self, manifest: &StoredManifest) -> Result<()> {
        let mut total_blob_bytes = 0usize;
        for operation in &manifest.operations {
            match (&operation.after, operation.after_blob_key.as_deref()) {
                (crate::services::twin_events::BeforeImage::Sha256(expected), Some(key)) => {
                    let bytes = self.read_blob(manifest, key)?;
                    total_blob_bytes = total_blob_bytes
                        .checked_add(bytes.len())
                        .filter(|total| *total <= self.blob_byte_limit())
                        .ok_or_else(|| {
                            anyhow::anyhow!("migration blobs exceed their aggregate limit")
                        })?;
                    if crate::services::twin_events::digest_bytes(&bytes) != *expected {
                        anyhow::bail!("migration operation after blob is invalid");
                    }
                }
                _ => anyhow::bail!("migration operation after blob is invalid"),
            }
            match (&operation.before, operation.before_blob_key.as_deref()) {
                (crate::services::twin_events::BeforeImage::Absent, None) => {}
                (crate::services::twin_events::BeforeImage::Sha256(expected), Some(key)) => {
                    let bytes = self.read_blob(manifest, key)?;
                    total_blob_bytes = total_blob_bytes
                        .checked_add(bytes.len())
                        .filter(|total| *total <= self.blob_byte_limit())
                        .ok_or_else(|| {
                            anyhow::anyhow!("migration blobs exceed their aggregate limit")
                        })?;
                    if crate::services::twin_events::digest_bytes(&bytes) != *expected {
                        anyhow::bail!("migration operation before blob is invalid");
                    }
                }
                _ => anyhow::bail!("migration operation before blob is invalid"),
            }
        }
        Ok(())
    }

    pub(super) fn read_blob(&self, manifest: &StoredManifest, key: &str) -> Result<Vec<u8>> {
        let prefix = format!("runs/{}/blobs/", manifest.run_id);
        if !key.starts_with(&prefix) {
            anyhow::bail!("migration blob key escapes its run");
        }
        self.strict_root()?
            .read_bounded(key, crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES)
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow::anyhow!("migration blob is missing"))
    }

    pub(super) fn write_immutable_commit_record(
        &self,
        manifest: &StoredManifest,
        witness: &MigrationStepWitnessV1,
        commit: &crate::services::twin_events::MutationCommit,
        outcome: MigrationCommitOutcomeV1,
    ) -> Result<()> {
        let mutation_id = commit
            .mutation_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("migration commit record lost its mutation ID"))?;
        let authority = commit
            .authority_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("migration commit record lost its authority"))?;
        if witness.intent.as_ref().map(|intent| &intent.mutation_id) != Some(&mutation_id)
            || authority.root_scope != witness.expected_authority.root_scope
            || authority.lease_epoch_uuid != witness.expected_authority.lease_epoch_uuid
            || authority.authority_generation
                != witness
                    .expected_authority
                    .authority_generation
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("migration authority overflowed"))?
        {
            anyhow::bail!("migration commit record differs from its active witness");
        }
        let record = MigrationCommitRecordV1 {
            schema_version: MIGRATION_COMMIT_RECORD_SCHEMA_VERSION,
            run_id: manifest.run_id.clone(),
            mutation_id,
            authority,
            direction: witness.direction,
            operation_index: witness.operation_index,
            outcome,
        };
        self.validate_commit_record_shape(manifest, &record)?;
        let bytes = serde_json::to_vec_pretty(&record)?;
        if bytes.len() > MAX_MIGRATION_COMMIT_RECORD_BYTES {
            anyhow::bail!("migration commit record exceeds its bounded record limit");
        }
        let key = commit_record_key(&record.run_id, record.authority.authority_generation);
        self.strict_root()?
            .install_no_clobber(&key, "staging", &bytes)
            .map_err(anyhow::Error::new)?;
        let durable = self
            .strict_root()?
            .read_bounded(&key, MAX_MIGRATION_COMMIT_RECORD_BYTES)
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow::anyhow!("immutable migration commit record disappeared"))?;
        if durable != bytes {
            anyhow::bail!("immutable migration commit record identity collision");
        }
        Ok(())
    }

    pub(super) fn require_manifest_commit_binding(&self, manifest: &StoredManifest) -> Result<()> {
        let Some(proof) = manifest.last_commit.as_ref() else {
            return Ok(());
        };
        let record = self
            .load_immutable_commit_record(&manifest.run_id, proof.authority.authority_generation)?;
        self.validate_commit_record_shape(manifest, &record)?;
        if record.mutation_id != proof.mutation_id || record.authority != proof.authority {
            anyhow::bail!("migration last commit differs from its immutable commit record");
        }
        let progress_matches = match (record.outcome, record.direction) {
            (MigrationCommitOutcomeV1::Committed, MigrationDirectionV1::Apply) => manifest
                .apply_next
                .checked_sub(1)
                .is_some_and(|index| index == record.operation_index),
            (MigrationCommitOutcomeV1::Committed, MigrationDirectionV1::Rollback) => manifest
                .rollback_total
                .checked_sub(manifest.rollback_next)
                .is_some_and(|index| manifest.rollback_next > 0 && index == record.operation_index),
            (MigrationCommitOutcomeV1::AuthorityOnly, MigrationDirectionV1::Apply) => {
                manifest.authority_only_advances > 0
                    && manifest.apply_next == record.operation_index
            }
            (MigrationCommitOutcomeV1::AuthorityOnly, MigrationDirectionV1::Rollback) => manifest
                .rollback_total
                .checked_sub(manifest.rollback_next + 1)
                .is_some_and(|index| {
                    manifest.authority_only_advances > 0 && index == record.operation_index
                }),
        };
        if !progress_matches {
            anyhow::bail!("migration commit record differs from manifest progress");
        }
        Ok(())
    }

    fn load_immutable_commit_record(
        &self,
        run_id: &str,
        authority_generation: u64,
    ) -> Result<MigrationCommitRecordV1> {
        super::validation::validate_canonical_uuid(run_id, "migration run ID")?;
        let bytes = self
            .strict_root()?
            .read_bounded(
                &commit_record_key(run_id, authority_generation),
                MAX_MIGRATION_COMMIT_RECORD_BYTES,
            )
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow::anyhow!("immutable migration commit record does not exist"))?;
        let value: Value = serde_json::from_slice(&bytes)?;
        super::validation::require_exact_fields(&value, CURRENT_COMMIT_RECORD_FIELDS)?;
        let record: MigrationCommitRecordV1 = serde_json::from_value(value)?;
        if record.schema_version != MIGRATION_COMMIT_RECORD_SCHEMA_VERSION
            || record.run_id != run_id
            || record.authority.authority_generation != authority_generation
        {
            anyhow::bail!("immutable migration commit record identity mismatch");
        }
        Ok(record)
    }

    fn validate_commit_record_shape(
        &self,
        manifest: &StoredManifest,
        record: &MigrationCommitRecordV1,
    ) -> Result<()> {
        if record.schema_version != MIGRATION_COMMIT_RECORD_SCHEMA_VERSION
            || record.run_id != manifest.run_id
            || manifest
                .operations
                .get(record.operation_index)
                .is_none_or(|operation| operation.index != record.operation_index)
        {
            anyhow::bail!("migration commit record fields are invalid");
        }
        let root_scope = manifest
            .root_scope
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("migration manifest root scope is missing"))?;
        super::validation::validate_manifest_authority(&record.authority, root_scope)
    }

    pub(super) fn write_immutable_plan(&self, plan: &StoredManifest) -> Result<()> {
        super::validation::validate_canonical_uuid(&plan.run_id, "migration run ID")?;
        self.validate_current_manifest(plan)?;
        if plan.status != "prepared"
            || plan.apply_next != 0
            || plan.rollback_next != 0
            || plan.rollback_total != 0
            || plan.authority_only_advances != 0
            || plan.active_step.is_some()
            || plan.last_commit.is_some()
        {
            anyhow::bail!("immutable migration plan is not in its initial prepared state");
        }
        let bytes = serde_json::to_vec_pretty(plan)?;
        if bytes.len() > MAX_MIGRATION_RECORD_BYTES {
            anyhow::bail!("immutable migration plan exceeds its bounded record limit");
        }
        let key = format!("runs/{}/plan.json", plan.run_id);
        self.strict_root()?
            .install_no_clobber(&key, "staging", &bytes)
            .map_err(anyhow::Error::new)?;
        let durable = self
            .strict_root()?
            .read_bounded(&key, MAX_MIGRATION_RECORD_BYTES)
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow::anyhow!("immutable migration plan disappeared"))?;
        if durable != bytes {
            anyhow::bail!("immutable migration plan identity collision");
        }
        Ok(())
    }

    pub(super) fn load_immutable_plan(&self, run_id: &str) -> Result<StoredManifest> {
        super::validation::validate_canonical_uuid(run_id, "migration run ID")?;
        let bytes = self
            .strict_root()?
            .read_bounded(
                &format!("runs/{run_id}/plan.json"),
                MAX_MIGRATION_RECORD_BYTES,
            )
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow::anyhow!("immutable migration plan does not exist"))?;
        let value: Value = serde_json::from_slice(&bytes)?;
        super::validation::require_exact_fields(&value, CURRENT_MANIFEST_FIELDS)?;
        let plan: StoredManifest = serde_json::from_value(value)?;
        if plan.run_id != run_id || plan.preview_id != run_id {
            anyhow::bail!("immutable migration plan identity mismatch");
        }
        self.validate_current_manifest(&plan)?;
        if plan.status != "prepared"
            || plan.apply_next != 0
            || plan.rollback_next != 0
            || plan.rollback_total != 0
            || plan.authority_only_advances != 0
            || plan.active_step.is_some()
            || plan.last_commit.is_some()
        {
            anyhow::bail!("immutable migration plan has mutable execution progress");
        }
        Ok(plan)
    }

    pub(super) fn require_manifest_plan_binding(
        &self,
        manifest: &StoredManifest,
        plan: &StoredManifest,
    ) -> Result<()> {
        if immutable_plan_value(manifest) != immutable_plan_value(plan) {
            anyhow::bail!("migration manifest differs from its immutable prepared plan");
        }
        let mut steps = Vec::with_capacity(manifest.apply_next + manifest.rollback_next);
        for operation_index in 0..manifest.apply_next {
            steps.push((operation_index, MigrationDirectionV1::Apply));
        }
        for rollback_step in 0..manifest.rollback_next {
            let operation_index = manifest
                .rollback_total
                .checked_sub(rollback_step + 1)
                .ok_or_else(|| anyhow::anyhow!("migration rollback projection underflow"))?;
            steps.push((operation_index, MigrationDirectionV1::Rollback));
        }
        let (source_inventory, overlay_inventory) =
            self.projected_manifest_inventory(plan, steps)?;
        if manifest.source_inventory != source_inventory
            || manifest.overlay_inventory != overlay_inventory
        {
            anyhow::bail!("migration manifest inventory differs from immutable plan progress");
        }
        Ok(())
    }

    pub(super) fn projected_manifest_inventory<I>(
        &self,
        manifest: &StoredManifest,
        steps: I,
    ) -> Result<(
        Vec<crate::models::migration::MarkdownMigrationSourceV1>,
        Vec<crate::models::migration::MarkdownMigrationOverlaySourceV1>,
    )>
    where
        I: IntoIterator<Item = (usize, MigrationDirectionV1)>,
    {
        let mut sources = BTreeMap::new();
        let mut source_path_by_note = HashMap::new();
        for source in &manifest.source_inventory {
            let path = migration_path_key(&source.relative_path);
            if sources.insert(path.clone(), source.clone()).is_some() {
                anyhow::bail!("migration Markdown inventory has duplicate paths");
            }
            if let Some(note_id) = source.note_id.as_ref() {
                if source_path_by_note.insert(note_id.clone(), path).is_some() {
                    anyhow::bail!("migration Markdown inventory has duplicate note IDs");
                }
            }
        }
        let mut overlays = BTreeMap::new();
        for overlay in &manifest.overlay_inventory {
            if overlays
                .insert(migration_path_key(&overlay.relative_path), overlay.clone())
                .is_some()
            {
                anyhow::bail!("migration overlay inventory has duplicate paths");
            }
        }

        for (operation_index, direction) in steps {
            let operation = manifest
                .operations
                .get(operation_index)
                .ok_or_else(|| anyhow::anyhow!("migration operation is out of range"))?;
            let (state, blob_key, event) = match direction {
                MigrationDirectionV1::Apply => (
                    operation.after.clone(),
                    operation.after_blob_key.as_deref(),
                    operation.note_event.as_ref(),
                ),
                MigrationDirectionV1::Rollback => (
                    operation.before.clone(),
                    operation.before_blob_key.as_deref(),
                    operation.rollback_note_event.as_ref(),
                ),
            };
            match operation.target_kind {
                crate::services::twin_events::TargetKind::Markdown => {
                    let physical = migration_path_key(&operation.target_key);
                    match state {
                        crate::services::twin_events::BeforeImage::Absent => {
                            if let Some(removed) = sources.remove(&physical) {
                                if let Some(note_id) = removed.note_id {
                                    source_path_by_note.remove(&note_id);
                                }
                            }
                        }
                        crate::services::twin_events::BeforeImage::Sha256(digest) => {
                            let key = blob_key.ok_or_else(|| {
                                anyhow::anyhow!("migration Markdown projection blob is missing")
                            })?;
                            let byte_len = u64::try_from(self.read_blob(manifest, key)?.len())?;
                            let prior_overlay = sources
                                .get(&physical)
                                .map(|source| source.overlay.clone())
                                .unwrap_or(
                                    crate::models::migration::MigrationSourceStateV1::Absent,
                                );
                            if let Some(previous) = sources.get(&physical) {
                                if let Some(note_id) = previous.note_id.as_ref() {
                                    source_path_by_note.remove(note_id);
                                }
                            }
                            let note_id = event.map(|event| event.note_id.clone());
                            if let Some(note_id) = note_id.as_ref() {
                                if source_path_by_note
                                    .insert(note_id.clone(), physical.clone())
                                    .is_some()
                                {
                                    anyhow::bail!(
                                        "migration Markdown projection has duplicate note IDs"
                                    );
                                }
                            }
                            sources.insert(
                                physical,
                                crate::models::migration::MarkdownMigrationSourceV1 {
                                    relative_path: operation.target_key.clone(),
                                    markdown_digest: digest,
                                    byte_len,
                                    note_id,
                                    overlay: prior_overlay,
                                },
                            );
                        }
                    }
                }
                crate::services::twin_events::TargetKind::OverlayJson => {
                    let physical = migration_path_key(&operation.target_key);
                    let note_id = operation
                        .target_key
                        .strip_suffix(".json")
                        .ok_or_else(|| anyhow::anyhow!("migration overlay key is invalid"))?;
                    let projected_state = match state {
                        crate::services::twin_events::BeforeImage::Absent => {
                            overlays.remove(&physical);
                            crate::models::migration::MigrationSourceStateV1::Absent
                        }
                        crate::services::twin_events::BeforeImage::Sha256(digest) => {
                            let key = blob_key.ok_or_else(|| {
                                anyhow::anyhow!("migration overlay projection blob is missing")
                            })?;
                            let byte_len = u64::try_from(self.read_blob(manifest, key)?.len())?;
                            overlays.insert(
                                physical,
                                crate::models::migration::MarkdownMigrationOverlaySourceV1 {
                                    relative_path: operation.target_key.clone(),
                                    digest: digest.clone(),
                                    byte_len,
                                },
                            );
                            crate::models::migration::MigrationSourceStateV1::Present {
                                digest,
                                byte_len,
                            }
                        }
                    };
                    let source_path = source_path_by_note.get(note_id).ok_or_else(|| {
                        anyhow::anyhow!("migration overlay note is absent from source inventory")
                    })?;
                    sources
                        .get_mut(source_path)
                        .expect("indexed migration source")
                        .overlay = projected_state;
                }
                _ => anyhow::bail!("migration operation target kind is invalid"),
            }
        }

        Ok((
            sources.into_values().collect(),
            overlays.into_values().collect(),
        ))
    }
}

fn commit_record_key(run_id: &str, authority_generation: u64) -> String {
    format!("runs/{run_id}/commits/{authority_generation}.json")
}

fn immutable_plan_value(manifest: &StoredManifest) -> Value {
    serde_json::json!({
        "schema_version": manifest.schema_version,
        "run_id": manifest.run_id,
        "preview_id": manifest.preview_id,
        "root_scope": manifest.root_scope,
        "vault_path": manifest.vault_path,
        "mode": manifest.mode,
        "created_at": manifest.created_at,
        "created_files": manifest.created_files,
        "backup_files": manifest.backup_files,
        "overlay_note_ids": manifest.overlay_note_ids,
        "touched_note_ids": manifest.touched_note_ids,
        "created_hub_note_ids": manifest.created_hub_note_ids,
        "skipped_fallback_note_ids": manifest.skipped_fallback_note_ids,
        "expected_program_target": manifest.expected_program_target,
        "program_after_digest": manifest.program_after_digest,
        "program_path": manifest.program_path,
        "request": manifest.request,
        "starting_authority": manifest.starting_authority,
        "operations": manifest.operations,
    })
}
