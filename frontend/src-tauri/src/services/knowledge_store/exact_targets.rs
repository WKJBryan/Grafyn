use super::*;

impl KnowledgeStore {
    pub(crate) fn materialize_note_update_at(
        &self,
        current: &Note,
        update: NoteUpdate,
        updated_at: chrono::DateTime<Utc>,
    ) -> Result<Note> {
        let mut note = current.clone();
        self.apply_note_update(&mut note, update, false)?;
        note.updated_at = updated_at;
        Ok(note)
    }

    pub(crate) fn serialized_note_bytes(&self, note: &Note) -> Result<(String, Vec<u8>)> {
        Self::canonical_serialized_note_bytes(note)
    }

    pub(crate) fn migration_note_event(
        &self,
        note: &Note,
        change: crate::models::twin_event::NoteChangeKind,
        payload_digest: crate::models::twin_event::ContentDigest,
        evidence_digest: crate::models::twin_event::ContentDigest,
        observed_at: chrono::DateTime<Utc>,
    ) -> ExactMigrationNoteEvent {
        ExactMigrationNoteEvent {
            note_id: note.id.clone(),
            change,
            observed_at,
            governance: note_capture_governance(note, "migration"),
            payload_digest,
            evidence_digest,
        }
    }

    pub(crate) fn migration_note_governance(
        &self,
        note: &Note,
    ) -> crate::models::twin_event::Governance {
        note_capture_governance(note, "migration")
    }

    pub(crate) fn optimizer_note_governance(
        &self,
        note: &Note,
    ) -> crate::models::twin_event::Governance {
        note_capture_governance(note, "vault_optimizer")
    }

    pub(crate) fn optimizer_overlay_governance(
        &self,
        relative_path: &str,
        markdown_raw_bytes: &[u8],
        overlay_raw_bytes: Option<&[u8]>,
    ) -> Result<crate::models::twin_event::Governance> {
        let relative_path = normalize_note_relative_path(relative_path)?;
        let content =
            std::str::from_utf8(markdown_raw_bytes).context("optimizer Markdown is not UTF-8")?;
        let path = self.resolve_vault_relative_path(&relative_path)?;
        let mut note = self.parse_note_content_without_overlay(&path, content, None)?;
        if let Some(overlay_raw_bytes) = overlay_raw_bytes {
            let overlay: OverlayNoteData = serde_json::from_slice(overlay_raw_bytes)
                .context("invalid optimizer overlay source")?;
            let markdown_digest = crate::services::twin_events::digest_bytes(markdown_raw_bytes);
            Self::merge_overlay_data(&mut note, overlay, &relative_path, &markdown_digest);
        }
        Ok(note_capture_governance(&note, "vault_optimizer"))
    }

    pub(crate) fn migration_target_bytes(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
    ) -> Result<Option<Vec<u8>>> {
        let root = match kind {
            crate::services::twin_events::TargetKind::Markdown => self.vault_root.as_ref(),
            crate::services::twin_events::TargetKind::OverlayJson => self.overlay_root.as_ref(),
            _ => None,
        }
        .ok_or_else(|| anyhow::anyhow!("retained migration target capability is unavailable"))?;
        root.read_bounded(
            relative_key,
            crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES,
        )
        .map_err(anyhow::Error::new)
    }

    pub(crate) fn migration_target_before_image(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
    ) -> Result<crate::services::twin_events::BeforeImage> {
        Ok(self.migration_target_bytes(kind, relative_key)?.map_or(
            crate::services::twin_events::BeforeImage::Absent,
            |bytes| {
                crate::services::twin_events::BeforeImage::Sha256(
                    crate::services::twin_events::digest_bytes(&bytes),
                )
            },
        ))
    }

    pub(crate) fn current_migration_authority(
        &self,
    ) -> Result<crate::services::vault_namespace::VaultAuthorityTokenV1> {
        self.event_recorder
            .current_authority_token()
            .map_err(anyhow::Error::new)
    }

    pub(crate) fn classify_migration_witness(
        &self,
        mutation_id: &crate::models::twin_event::ContentDigest,
        expected_authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
        target_kind: crate::services::twin_events::TargetKind,
        target_key: &str,
        expected_before: &crate::services::twin_events::BeforeImage,
        expected_after: &crate::services::twin_events::BeforeImage,
    ) -> Result<crate::services::twin_events::WitnessedMutationRecovery> {
        self.event_recorder
            .classify_witnessed_mutation(
                mutation_id,
                expected_authority,
                target_kind,
                target_key,
                expected_before,
                expected_after,
            )
            .map_err(anyhow::Error::new)
    }

    pub(crate) fn consume_migration_witness(
        &self,
        mutation_id: &crate::models::twin_event::ContentDigest,
    ) -> Result<()> {
        self.event_recorder
            .consume_witnessed_mutation_receipt(mutation_id)
            .map_err(anyhow::Error::new)
    }

    pub(crate) fn recover_coordinated_mutations(&self) -> Result<usize> {
        self.event_recorder
            .recover_pending_mutations()
            .map_err(anyhow::Error::new)
    }

    pub(crate) fn optimizer_markdown_snapshot(
        &self,
        relative_path: &str,
    ) -> Result<Option<(Note, OptimizerMarkdownPrecondition, Vec<u8>)>> {
        let normalized = normalize_note_relative_path(relative_path)?;
        let root = crate::services::twin_events::AnchoredRoot::open(&self.vault_path)
            .map_err(anyhow::Error::new)?;
        let Some(bytes) = root
            .read_bounded(
                &normalized,
                crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES,
            )
            .map_err(anyhow::Error::new)?
        else {
            return Ok(None);
        };
        let content = std::str::from_utf8(&bytes).context("optimizer Markdown is not UTF-8")?;
        let path = self.resolve_vault_relative_path(&normalized)?;
        let note = self.parse_note_content_without_overlay(&path, content, None)?;
        Ok(Some((
            note,
            OptimizerMarkdownPrecondition {
                relative_path: normalized,
                expected_digest: crate::services::twin_events::digest_bytes(&bytes),
                root: Arc::new(root),
            },
            bytes,
        )))
    }

    pub(crate) fn optimizer_note_snapshot(
        &self,
        note_id: &str,
    ) -> Result<Option<OptimizerNoteSnapshot>> {
        Self::validate_note_id(note_id)?;
        let path = self.note_path(note_id)?;
        let relative_path = path
            .strip_prefix(&self.vault_path)
            .map_err(|_| anyhow::anyhow!("optimizer note path escaped the vault"))?
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("optimizer note path is not UTF-8"))?;
        let Some((mut note, markdown_precondition, markdown_raw_bytes)) =
            self.optimizer_markdown_snapshot(relative_path)?
        else {
            return Ok(None);
        };
        if note.id != note_id {
            anyhow::bail!("optimizer note identity changed before snapshot");
        }
        let (overlay_value, overlay_digest, overlay_raw_bytes) =
            self.optimizer_overlay_snapshot(note_id)?;
        if let Some(value) = overlay_value.as_ref() {
            let overlay: OverlayNoteData = serde_json::from_value(value.clone())
                .context("invalid optimizer overlay source")?;
            Self::merge_overlay_data(
                &mut note,
                overlay,
                markdown_precondition.relative_path(),
                markdown_precondition.expected_digest(),
            );
        }
        Ok(Some(OptimizerNoteSnapshot {
            note,
            markdown_precondition,
            markdown_raw_bytes,
            overlay_value,
            overlay_digest,
            overlay_raw_bytes,
        }))
    }

    pub(crate) fn optimizer_markdown_precondition(
        &self,
        relative_path: &str,
        expected_digest: crate::models::twin_event::ContentDigest,
    ) -> Result<OptimizerMarkdownPrecondition> {
        let relative_path = normalize_note_relative_path(relative_path)?;
        let root = crate::services::twin_events::AnchoredRoot::open(&self.vault_path)
            .map_err(anyhow::Error::new)?;
        Ok(OptimizerMarkdownPrecondition {
            relative_path,
            expected_digest,
            root: Arc::new(root),
        })
    }

    pub(crate) fn optimizer_overlay_snapshot(
        &self,
        note_id: &str,
    ) -> Result<(
        Option<serde_json::Value>,
        Option<crate::models::twin_event::ContentDigest>,
        Option<Vec<u8>>,
    )> {
        Self::validate_note_id(note_id)?;
        let data_root = self
            .overlay_notes_dir
            .ancestors()
            .nth(3)
            .ok_or_else(|| anyhow::anyhow!("optimizer overlay root is invalid"))?;
        let root = crate::services::twin_events::AnchoredRoot::open(data_root)
            .map_err(anyhow::Error::new)?;
        let Some(bytes) = root
            .read_bounded(
                &format!("vault_migration/overlay/notes/{note_id}.json"),
                crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES,
            )
            .map_err(anyhow::Error::new)?
        else {
            return Ok((None, None, None));
        };
        let digest = crate::services::twin_events::digest_bytes(&bytes);
        let value = serde_json::from_slice(&bytes).context("invalid optimizer overlay source")?;
        Ok((Some(value), Some(digest), Some(bytes)))
    }

    pub(crate) fn replace_note_exact_expecting_authority_with_hooks(
        &mut self,
        id: &str,
        before: Note,
        before_digest: crate::models::twin_event::ContentDigest,
        exact: Note,
        source: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
        prepared: &mut dyn FnMut(
            &crate::services::twin_events::MutationIntentV1,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
        committed: &mut dyn FnMut(
            &crate::services::twin_events::MutationCommit,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        Self::validate_note_id(id)?;
        if exact.id != id {
            anyhow::bail!("exact replacement note identity does not match target");
        }
        if before.id != id || before.relative_path != exact.relative_path {
            anyhow::bail!("exact replacement source does not match target");
        }
        if self.event_recorder.is_noop() {
            self.write_note_file(&exact)?;
            self.cache_exact_note(&exact)?;
            return Ok((
                exact,
                crate::services::twin_events::MutationCommit {
                    mutation_id: None,
                    events: Vec::new(),
                    authority_token: None,
                    postcommit_warning: false,
                },
            ));
        }
        let recorder = self.event_recorder.clone();
        let source_channel =
            crate::models::twin_event::SourceChannel::parse(source).map_err(anyhow::Error::msg)?;
        let exact_for_plan = exact.clone();
        let mut planner = || {
            let (relative_path, exact_bytes) =
                Self::canonical_serialized_note_bytes(&exact_for_plan).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            let after_digest = crate::services::twin_events::digest_bytes(&exact_bytes);
            if before_digest == after_digest {
                return Ok(None);
            }
            let target = crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                normalize_relative_path_for_output(&relative_path),
                String::from_utf8(exact_bytes).expect("Markdown serialization is UTF-8"),
            )
            .expecting(crate::services::twin_events::BeforeImage::Sha256(
                before_digest.clone(),
            ));
            let draft = crate::services::twin_events::note_changed_draft(
                &exact_for_plan.id,
                crate::models::twin_event::NoteChangeKind::Updated,
                after_digest,
                before_digest.clone(),
                exact_for_plan.updated_at,
                source_channel.clone(),
                note_capture_governance(&exact_for_plan, source_channel.as_str()),
            )
            .map_err(crate::services::twin_events::MutationError::Invalid)?;
            let plan = crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::SyncEligible,
                source_channel.clone(),
                vec![target],
                vec![draft],
            )
            .expecting_authority(expected.clone())
            .retaining_commit_receipt();
            Ok(Some(plan))
        };
        let result = recorder.commit_planned_mutation_with_hooks(
            crate::services::twin_events::MutationOrigin::Local,
            &mut planner,
            prepared,
            committed,
        );
        let commit = result.map_err(anyhow::Error::new)?;
        self.cache_exact_note(&exact)?;
        Ok((exact, commit))
    }

    /// Administrative variant of `update_note` that preserves the note's existing
    /// `updated_at` timestamp. See `backfill_legacy_grafyn_notes` for the motivating
    /// case: schema/provenance bookkeeping writes should not bump recency ranking.
    pub(crate) fn update_note_preserving_timestamp(
        &mut self,
        id: &str,
        update: NoteUpdate,
    ) -> Result<Note> {
        self.update_note_with_options(
            id,
            update,
            false,
            NoteMutationContext::local("migration")?,
            None,
        )
        .map(|(note, _)| note)
    }

    #[allow(dead_code)] // Retained for bounded legacy migration recovery.
    pub(crate) fn restore_note_bytes_from_source(
        &mut self,
        relative_path: &str,
        bytes: &[u8],
        source: &str,
    ) -> Result<Note> {
        let relative_path = normalize_note_relative_path(relative_path)?;
        let after = std::str::from_utf8(bytes)
            .with_context(|| format!("restored Markdown is not UTF-8: {relative_path}"))?
            .to_string();
        if self.event_recorder.is_noop() {
            let path = self.resolve_vault_relative_path(&relative_path)?;
            let note = self
                .find_note_by_relative_path(&relative_path)?
                .ok_or_else(|| {
                    anyhow::anyhow!("note to restore is not indexed: {relative_path}")
                })?;
            let before = std::fs::read(&path)?;
            if before == bytes {
                return Ok(note);
            }
            write_atomic(&path, bytes)?;
            self.refresh_cache();
            self.get_note(&note.id)
        } else {
            let recorder = self.event_recorder.clone();
            let source = source.to_string();
            let mut restored_note_id = None;
            let mut planner = || {
                self.refresh_cache();
                let path = self
                    .resolve_vault_relative_path(&relative_path)
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                let note = self
                    .find_note_by_relative_path(&relative_path)
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?
                    .ok_or_else(|| {
                        crate::services::twin_events::MutationError::Invalid(format!(
                            "note to restore is not indexed: {relative_path}"
                        ))
                    })?;
                restored_note_id = Some(note.id.clone());
                let before = std::fs::read(&path).map_err(|error| {
                    crate::services::twin_events::MutationError::Io(error.to_string())
                })?;
                if before == bytes {
                    return Ok(None);
                }
                let source_channel = crate::models::twin_event::SourceChannel::parse(&source)
                    .map_err(crate::services::twin_events::MutationError::Invalid)?;
                let draft = crate::services::twin_events::note_changed_draft(
                    &note.id,
                    crate::models::twin_event::NoteChangeKind::Updated,
                    crate::services::twin_events::digest_bytes(bytes),
                    crate::services::twin_events::digest_bytes(&before),
                    Utc::now(),
                    source_channel.clone(),
                    note_capture_governance(&note, &source),
                )
                .map_err(crate::services::twin_events::MutationError::Invalid)?;
                Ok(Some(crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::SyncEligible,
                    source_channel,
                    vec![crate::services::twin_events::TargetMutation::put(
                        crate::services::twin_events::TargetKind::Markdown,
                        relative_path.clone(),
                        after.clone(),
                    )],
                    vec![draft],
                )))
            };
            let commit_result = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            );
            if let Err(error) = commit_result {
                self.refresh_cache();
                return Err(anyhow::Error::new(error));
            }
            self.refresh_cache();
            self.get_note(
                restored_note_id
                    .as_deref()
                    .expect("restore planner returned a note ID"),
            )
        }
    }

    #[allow(dead_code)] // Retained for bounded legacy migration recovery.
    pub(crate) fn put_vault_file_target_only_expected(
        &mut self,
        relative_path: &str,
        bytes: &[u8],
        source: &str,
        expected_before: Option<crate::services::twin_events::BeforeImage>,
    ) -> Result<()> {
        let relative_path = normalize_note_relative_path(relative_path)?;
        let after = std::str::from_utf8(bytes)
            .with_context(|| format!("Markdown target is not UTF-8: {relative_path}"))?
            .to_string();
        if self.event_recorder.is_noop() {
            let path = self.resolve_vault_relative_path(&relative_path)?;
            if let Some(expected) = expected_before.as_ref() {
                let current = before_image_for_path(&path)?;
                if current
                    == crate::services::twin_events::BeforeImage::Sha256(
                        crate::services::twin_events::digest_bytes(bytes),
                    )
                {
                    return Ok(());
                }
                if &current != expected {
                    anyhow::bail!("conditional Markdown target changed: {relative_path}");
                }
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            write_atomic(&path, bytes)?;
            self.refresh_cache();
            return Ok(());
        }
        let recorder = self.event_recorder.clone();
        let source_channel =
            crate::models::twin_event::SourceChannel::parse(source).map_err(anyhow::Error::msg)?;
        let target = crate::services::twin_events::TargetMutation::put(
            crate::services::twin_events::TargetKind::Markdown,
            relative_path,
            after,
        );
        let target = match expected_before {
            Some(expected) => target.expecting(expected),
            None => target,
        };
        let mut plan = Some(crate::services::twin_events::MutationPlan::new(
            crate::models::twin_event::CausalStream::LocalOnly,
            source_channel,
            vec![target],
            Vec::new(),
        ));
        let result = recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut || Ok(plan.take()),
        );
        self.refresh_cache();
        result.map(|_| ()).map_err(anyhow::Error::new)
    }

    #[allow(dead_code)] // Retained for bounded legacy migration recovery.
    pub(crate) fn delete_vault_file_target_only(
        &mut self,
        relative_path: &str,
        source: &str,
    ) -> Result<()> {
        self.delete_vault_file_target_only_expected(relative_path, source, None)
    }

    #[allow(dead_code)] // Retained for bounded legacy migration recovery.
    pub(crate) fn delete_vault_file_target_only_expected(
        &mut self,
        relative_path: &str,
        source: &str,
        expected_before: Option<crate::services::twin_events::BeforeImage>,
    ) -> Result<()> {
        let relative_path = normalize_note_relative_path(relative_path)?;
        if self.event_recorder.is_noop() {
            let path = self.resolve_vault_relative_path(&relative_path)?;
            if let Some(expected) = expected_before.as_ref() {
                let current = before_image_for_path(&path)?;
                if current == crate::services::twin_events::BeforeImage::Absent {
                    return Ok(());
                }
                if &current != expected {
                    anyhow::bail!("conditional Markdown target changed: {relative_path}");
                }
            }
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            self.refresh_cache();
            return Ok(());
        }
        let recorder = self.event_recorder.clone();
        let source_channel =
            crate::models::twin_event::SourceChannel::parse(source).map_err(anyhow::Error::msg)?;
        let target = crate::services::twin_events::TargetMutation::tombstone(
            crate::services::twin_events::TargetKind::Markdown,
            relative_path,
        );
        let target = match expected_before {
            Some(expected) => target.expecting(expected),
            None => target,
        };
        let mut plan = Some(crate::services::twin_events::MutationPlan::new(
            crate::models::twin_event::CausalStream::LocalOnly,
            source_channel,
            vec![target],
            Vec::new(),
        ));
        let result = recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut || Ok(plan.take()),
        );
        self.refresh_cache();
        result.map(|_| ()).map_err(anyhow::Error::new)
    }

    #[allow(dead_code)] // Retained for bounded legacy migration recovery.
    pub(crate) fn validate_vault_file_target(
        &mut self,
        relative_path: &str,
        expected: crate::services::twin_events::BeforeImage,
    ) -> Result<()> {
        let relative_path = normalize_note_relative_path(relative_path)?;
        let recorder = self.event_recorder.clone();
        let mut planner = || {
            let path = self
                .resolve_vault_relative_path(&relative_path)
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            let current = before_image_for_path(&path).map_err(|error| {
                crate::services::twin_events::MutationError::Io(error.to_string())
            })?;
            if current != expected {
                return Err(
                    crate::services::twin_events::MutationError::RecoveryConflict(format!(
                        "conditional Markdown target changed: {relative_path}"
                    )),
                );
            }
            Ok(None)
        };
        recorder
            .commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            )
            .map(|_| ())
            .map_err(anyhow::Error::new)
    }

    pub fn delete_note(&mut self, id: &str) -> Result<()> {
        self.delete_note_from_source(id, "note_editor")
    }

    pub fn delete_note_from_source(&mut self, id: &str, source: &str) -> Result<()> {
        self.delete_note_from_source_with_commit(id, source)
            .map(|_| ())
    }

    pub(crate) fn delete_note_from_source_with_commit(
        &mut self,
        id: &str,
        source: &str,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.delete_note_with_context(id, source, None)
    }

    pub(crate) fn delete_note_expecting_authority(
        &mut self,
        id: &str,
        source: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.delete_note_with_context(id, source, Some(expected))
    }

    fn delete_note_with_context(
        &mut self,
        id: &str,
        source: &str,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        Self::validate_note_id(id)?;
        let context = NoteMutationContext::local(source)?;
        let commit = if self.event_recorder.is_noop() {
            let path = self.note_path(id)?;
            let note = self.get_note(id)?;
            self.persist_note_delete(&note, &path, context)?;
            crate::services::twin_events::MutationCommit {
                mutation_id: None,
                events: Vec::new(),
                authority_token: None,
                postcommit_warning: false,
            }
        } else {
            let recorder = self.event_recorder.clone();
            let note_id = id.to_string();
            let mut planner = || {
                self.refresh_cache();
                let note = self.get_note(&note_id).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                let path = self.note_path(&note_id).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                let mut plan = self
                    .plan_note_delete(&note, &path, context.clone())
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                if let Some(expected) = expected.clone() {
                    plan = plan.expecting_authority(expected);
                }
                Ok(Some(plan))
            };
            let result = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            );
            match result {
                Ok(commit) => commit,
                Err(error) => {
                    self.refresh_cache();
                    return Err(super::preserve_knowledge_authority_error(
                        error,
                        vec![id.to_string()],
                    ));
                }
            }
        };
        if self.event_recorder.is_noop() {
            let overlay_path = self.overlay_path(id);
            if overlay_path.exists() {
                let _ = std::fs::remove_file(&overlay_path);
            }
        }
        self.refresh_cache();
        Ok(commit)
    }

    pub fn overlay_path(&self, note_id: &str) -> PathBuf {
        self.overlay_notes_dir.join(format!("{}.json", note_id))
    }

    pub fn write_overlay(&self, note_id: &str, overlay: &serde_json::Value) -> Result<()> {
        self.write_overlay_from_source(note_id, overlay, "vault_optimizer")
    }

    pub fn write_overlay_from_source(
        &self,
        note_id: &str,
        overlay: &serde_json::Value,
        source: &str,
    ) -> Result<()> {
        self.write_overlay_from_source_with_authority(note_id, overlay, source, None)
            .map(|_| ())
    }

    #[allow(dead_code)] // Retained for exact-target compatibility tests.
    pub(crate) fn write_overlay_from_source_expecting_authority(
        &self,
        note_id: &str,
        overlay: &serde_json::Value,
        source: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.write_overlay_from_source_with_authority(note_id, overlay, source, Some(expected))
    }

    pub(crate) fn write_overlay_from_source_with_authority(
        &self,
        note_id: &str,
        overlay: &serde_json::Value,
        source: &str,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.write_overlay_from_source_with_authority_and_hooks(
            note_id,
            overlay,
            source,
            expected,
            &mut |_| Ok(()),
            &mut |_| Ok(()),
            false,
            None,
        )
    }

    pub(crate) fn write_overlay_from_source_with_authority_and_hooks(
        &self,
        note_id: &str,
        overlay: &serde_json::Value,
        source: &str,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
        prepared: &mut dyn FnMut(
            &crate::services::twin_events::MutationIntentV1,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
        committed: &mut dyn FnMut(
            &crate::services::twin_events::MutationCommit,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
        retain_commit_receipt: bool,
        source_guard: Option<crate::services::twin_events::TargetMutation>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        Self::validate_note_id(note_id)?;
        let content = serde_json::to_string_pretty(overlay)?;
        if !self.event_recorder.is_noop() {
            let source_channel = crate::models::twin_event::SourceChannel::parse(source)
                .map_err(anyhow::Error::msg)?;
            let target_key = format!("{note_id}.json");
            if source_guard.is_some() && !retain_commit_receipt {
                anyhow::bail!("optimizer source guards require retained schema-3 receipts");
            }
            let mut targets = Vec::with_capacity(1 + usize::from(source_guard.is_some()));
            if let Some(source_guard) = source_guard {
                targets.push(source_guard);
            }
            targets.push(crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::OverlayJson,
                target_key,
                content,
            ));
            let mut mutation_plan = crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::LocalOnly,
                source_channel,
                targets,
                Vec::new(),
            );
            if let Some(expected) = expected {
                mutation_plan = mutation_plan.expecting_authority(expected);
            }
            if retain_commit_receipt {
                mutation_plan = mutation_plan.retaining_commit_receipt();
            }
            let mut plan = Some(mutation_plan);
            let commit = self
                .event_recorder
                .commit_planned_mutation_with_hooks(
                    crate::services::twin_events::MutationOrigin::Local,
                    &mut || Ok(plan.take()),
                    prepared,
                    committed,
                )
                .map_err(anyhow::Error::new)?;
            return Ok(commit);
        }
        if let Some(parent) = self.overlay_path(note_id).parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(&self.overlay_path(note_id), content.as_bytes())
            .with_context(|| format!("Failed to write overlay for '{}'", note_id))?;
        Ok(crate::services::twin_events::MutationCommit {
            mutation_id: None,
            events: Vec::new(),
            authority_token: None,
            postcommit_warning: false,
        })
    }

    pub fn delete_overlay(&self, note_id: &str) -> Result<()> {
        self.delete_overlay_from_source(note_id, "vault_optimizer")
    }

    pub fn delete_overlay_from_source(&self, note_id: &str, source: &str) -> Result<()> {
        self.delete_overlay_from_source_with_authority(note_id, source, None)
            .map(|_| ())
    }

    pub(crate) fn delete_overlay_from_source_with_authority(
        &self,
        note_id: &str,
        source: &str,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        Self::validate_note_id(note_id)?;
        if !self.event_recorder.is_noop() {
            let source_channel = crate::models::twin_event::SourceChannel::parse(source)
                .map_err(anyhow::Error::msg)?;
            let target_key = format!("{note_id}.json");
            let mut mutation_plan = crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::LocalOnly,
                source_channel,
                vec![crate::services::twin_events::TargetMutation::tombstone(
                    crate::services::twin_events::TargetKind::OverlayJson,
                    target_key,
                )],
                Vec::new(),
            );
            if let Some(expected) = expected {
                mutation_plan = mutation_plan.expecting_authority(expected);
            }
            let mut plan = Some(mutation_plan);
            let commit = self
                .event_recorder
                .commit_planned_mutation(
                    crate::services::twin_events::MutationOrigin::Local,
                    &mut || Ok(plan.take()),
                )
                .map_err(anyhow::Error::new)?;
            return Ok(commit);
        }
        let path = self.overlay_path(note_id);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(crate::services::twin_events::MutationCommit {
            mutation_id: None,
            events: Vec::new(),
            authority_token: None,
            postcommit_warning: false,
        })
    }

    pub fn extract_wikilinks(&self, content: &str) -> Vec<String> {
        WIKILINK_REGEX
            .captures_iter(content)
            .filter_map(|cap| cap.get(1).map(|m| m.as_str().trim().to_string()))
            .filter(|value| !value.is_empty())
            .collect()
    }

    pub fn extract_links(&self, content: &str, source_relative_path: &str) -> Vec<ParsedLink> {
        let mut links = self.extract_typed_wikilinks(content);
        links.extend(self.extract_markdown_links(content, source_relative_path));
        links
    }

    /// Extract typed wikilinks with relationship information.
    pub fn extract_typed_wikilinks(&self, content: &str) -> Vec<ParsedLink> {
        TYPED_WIKILINK_REGEX
            .captures_iter(content)
            .filter_map(|cap| {
                let target_title = cap.get(1)?.as_str().trim().to_string();
                if target_title.is_empty() {
                    return None;
                }
                let relation = cap
                    .get(2)
                    .map(|m| RelationType::from_str_lossy(m.as_str()))
                    .unwrap_or(RelationType::Untyped);
                Some(ParsedLink {
                    target_title,
                    target_path: None,
                    relation,
                })
            })
            .collect()
    }
}
