use super::*;

pub(super) enum RecoveredMigrationStep {
    None,
    Committed(crate::services::twin_events::MutationCommit),
    AuthorityOnly(crate::services::twin_events::MutationCommit),
}

impl MarkdownMigrationService {
    pub(super) fn recover_active_step(
        &self,
        manifest: &mut StoredManifest,
        store: &KnowledgeStore,
    ) -> Result<RecoveredMigrationStep> {
        let Some(witness) = manifest.active_step.clone() else {
            return Ok(RecoveredMigrationStep::None);
        };
        let Some(intent) = witness.intent.as_ref() else {
            // The placeholder is persisted before the coordinator is called.
            // Its missing finalized intent is therefore durable negative proof
            // that this migration never reached the Prepared owner hook.
            manifest.active_step = None;
            self.write_strict_manifest(manifest)?;
            return Ok(RecoveredMigrationStep::None);
        };
        let mutation_id = intent.mutation_id.clone();
        let operation = manifest
            .operations
            .get(witness.operation_index)
            .ok_or_else(|| anyhow::anyhow!("migration witness operation is out of range"))?;
        self.validate_witness_intent(manifest, &witness, operation, intent)?;
        if witness.abort_recorded {
            let authority_only = witness.committed_authority.is_some();
            store.consume_migration_witness(&mutation_id)?;
            if authority_only && !witness.receipt_consumed {
                manifest
                    .active_step
                    .as_mut()
                    .expect("active authority-only witness")
                    .receipt_consumed = true;
                self.write_strict_manifest(manifest)?;
            }
            let commit = authority_only
                .then(|| super::transaction::commit_from_witness(&witness))
                .transpose()?;
            manifest.active_step = None;
            self.write_strict_manifest(manifest)?;
            return Ok(match commit {
                Some(commit) => RecoveredMigrationStep::AuthorityOnly(commit),
                None => RecoveredMigrationStep::None,
            });
        }
        if witness.progress_recorded {
            store.consume_migration_witness(&mutation_id)?;
            if !witness.receipt_consumed {
                manifest
                    .active_step
                    .as_mut()
                    .expect("active witness")
                    .receipt_consumed = true;
                self.write_strict_manifest(manifest)?;
            }
            let commit = super::transaction::commit_from_witness(&witness)?;
            manifest.active_step = None;
            self.write_strict_manifest(manifest)?;
            return Ok(RecoveredMigrationStep::Committed(commit));
        }
        let (before, after) = match witness.direction {
            MigrationDirectionV1::Apply => (&operation.before, &operation.after),
            MigrationDirectionV1::Rollback => (&operation.after, &operation.before),
        };
        store.recover_coordinated_mutations()?;
        match store.classify_migration_witness(
            &mutation_id,
            &witness.expected_authority,
            operation.target_kind,
            &operation.target_key,
            before,
            after,
        )? {
            crate::services::twin_events::WitnessedMutationRecovery::NotCommitted => {
                let current = store.current_migration_authority()?;
                if current != witness.expected_authority {
                    anyhow::bail!(
                        "uncommitted migration witness cannot own a changed authority generation"
                    );
                }
                manifest.active_step = None;
                manifest.final_authority = Some(current);
                self.write_strict_manifest(manifest)?;
                Ok(RecoveredMigrationStep::None)
            }
            crate::services::twin_events::WitnessedMutationRecovery::Aborted => {
                manifest
                    .active_step
                    .as_mut()
                    .expect("active migration witness")
                    .abort_recorded = true;
                self.write_strict_manifest(manifest)?;
                store.consume_migration_witness(&mutation_id)?;
                manifest.active_step = None;
                self.write_strict_manifest(manifest)?;
                Ok(RecoveredMigrationStep::None)
            }
            crate::services::twin_events::WitnessedMutationRecovery::AbortedAfterAuthority(
                commit,
            ) => self.record_authority_only_abort(manifest, store, &witness, commit),
            crate::services::twin_events::WitnessedMutationRecovery::Committed(commit) => {
                if witness.committed_authority.is_some()
                    && witness.committed_authority != commit.authority_token
                {
                    anyhow::bail!("migration committed witness authority mismatch");
                }
                self.finish_committed_step(manifest, store, commit.clone())?;
                Ok(RecoveredMigrationStep::Committed(commit))
            }
        }
    }

    fn record_authority_only_abort(
        &self,
        manifest: &mut StoredManifest,
        store: &KnowledgeStore,
        witness: &MigrationStepWitnessV1,
        commit: crate::services::twin_events::MutationCommit,
    ) -> Result<RecoveredMigrationStep> {
        let mutation_id = commit
            .mutation_id
            .clone()
            .filter(|id| witness.intent.as_ref().map(|intent| &intent.mutation_id) == Some(id))
            .ok_or_else(|| anyhow::anyhow!("authority-only abort mutation ID is inconsistent"))?;
        let authority = commit
            .authority_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("authority-only abort lost its authority token"))?;
        if witness
            .committed_authority
            .as_ref()
            .is_some_and(|known| known != &authority)
            || authority.root_scope != witness.expected_authority.root_scope
            || authority.lease_epoch_uuid != witness.expected_authority.lease_epoch_uuid
            || authority.authority_generation
                != witness
                    .expected_authority
                    .authority_generation
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("migration authority overflowed"))?
        {
            anyhow::bail!("authority-only abort did not return the exact next authority");
        }
        self.write_immutable_commit_record(
            manifest,
            witness,
            &commit,
            MigrationCommitOutcomeV1::AuthorityOnly,
        )?;
        manifest.authority_only_advances = manifest
            .authority_only_advances
            .checked_add(1)
            .filter(|count| *count <= super::validation::MAX_MIGRATION_AUTHORITY_ONLY_ADVANCES)
            .ok_or_else(|| {
                anyhow::anyhow!("migration authority-only advances exceed their limit")
            })?;
        manifest.final_authority = Some(authority.clone());
        manifest.last_commit = Some(MigrationCommitProofV1 {
            mutation_id: mutation_id.clone(),
            authority: authority.clone(),
        });
        let active = manifest
            .active_step
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("authority-only abort lost its active witness"))?;
        active.committed_authority = Some(authority);
        active.abort_recorded = true;
        manifest.status = match witness.direction {
            MigrationDirectionV1::Apply => "apply_partial",
            MigrationDirectionV1::Rollback => "rollback_partial",
        }
        .into();
        self.write_strict_manifest(manifest)?;
        store.consume_migration_witness(&mutation_id)?;
        manifest
            .active_step
            .as_mut()
            .expect("active authority-only witness")
            .receipt_consumed = true;
        self.write_strict_manifest(manifest)?;
        manifest.active_step = None;
        self.write_strict_manifest(manifest)?;
        Ok(RecoveredMigrationStep::AuthorityOnly(commit))
    }

    pub(super) fn validate_witness_intent(
        &self,
        manifest: &StoredManifest,
        witness: &MigrationStepWitnessV1,
        operation: &MigrationOperationV1,
        intent: &crate::services::twin_events::MutationIntentV1,
    ) -> Result<()> {
        intent.validate().map_err(anyhow::Error::new)?;
        if intent.schema_version != 3
            || !intent.retain_commit_receipt
            || intent.mutation_id != crate::services::twin_events::derive_mutation_id(intent)
            || intent.markdown_root_scope.as_ref() != Some(&witness.expected_authority.root_scope)
            || intent.content_authority_generation
                != Some(
                    witness
                        .expected_authority
                        .authority_generation
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("migration authority overflowed"))?,
                )
            || intent.source_channel.as_str() != "migration"
            || intent.targets.len() != 1
        {
            anyhow::bail!("migration witness intent identity is invalid");
        }
        let (expected_before, after_state, stored_event) = match witness.direction {
            MigrationDirectionV1::Apply => (
                &operation.before,
                operation.after.clone(),
                operation.note_event.as_ref(),
            ),
            MigrationDirectionV1::Rollback => (
                &operation.after,
                operation.before.clone(),
                operation.rollback_note_event.as_ref(),
            ),
        };
        let expected_after = super::transaction::desired_for_state(
            after_state,
            match witness.direction {
                MigrationDirectionV1::Apply => operation.after_blob_key.as_deref(),
                MigrationDirectionV1::Rollback => operation.before_blob_key.as_deref(),
            },
            manifest,
            self,
        )?;
        let target = &intent.targets[0];
        if target.kind != operation.target_kind
            || target.relative_key != operation.target_key
            || &target.before != expected_before
            || target.after != expected_after
            || target.after_digest != crate::services::twin_events::desired_digest(&target.after)
        {
            anyhow::bail!("migration witness intent target differs from its operation");
        }
        match (stored_event, intent.events.as_slice()) {
            (None, []) => {}
            (Some(expected), [event]) => {
                let crate::models::twin_event::TwinEventPayload::NoteChanged(payload) =
                    &event.payload
                else {
                    anyhow::bail!("migration witness event has the wrong payload");
                };
                let evidence = event
                    .evidence
                    .first()
                    .filter(|_| event.evidence.len() == 1)
                    .ok_or_else(|| anyhow::anyhow!("migration witness evidence is invalid"))?;
                if payload.note_id.as_str() != expected.note_id
                    || payload.change != expected.change
                    || payload.content_digest.as_ref() != Some(&expected.payload_digest)
                    || event.observed_at != expected.observed_at
                    || event.governance != expected.governance
                    || event.context.source_channel.as_str() != "migration"
                    || evidence.source_id.as_str() != expected.note_id
                    || evidence.digest.as_ref() != Some(&expected.evidence_digest)
                {
                    anyhow::bail!("migration witness event differs from its operation");
                }
            }
            _ => anyhow::bail!("migration witness event count differs from its operation"),
        }
        Ok(())
    }

    pub(super) fn finish_committed_step(
        &self,
        manifest: &mut StoredManifest,
        store: &KnowledgeStore,
        commit: crate::services::twin_events::MutationCommit,
    ) -> Result<()> {
        let mutation_id = commit
            .mutation_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("migration commit is missing its mutation ID"))?;
        let authority = commit
            .authority_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("migration commit is missing its authority token"))?;
        let record_witness = manifest
            .active_step
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("migration commit has no active witness"))?
            .clone();
        let (direction, operation_index, expected_authority) = {
            (
                record_witness.direction,
                record_witness.operation_index,
                record_witness.expected_authority.clone(),
            )
        };
        if authority.root_scope != expected_authority.root_scope
            || authority.lease_epoch_uuid != expected_authority.lease_epoch_uuid
            || authority.authority_generation
                != expected_authority
                    .authority_generation
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("migration authority overflowed"))?
        {
            anyhow::bail!("migration commit did not return the exact next authority token");
        }
        if record_witness
            .intent
            .as_ref()
            .map(|intent| &intent.mutation_id)
            != Some(&mutation_id)
        {
            anyhow::bail!("migration commit mutation ID differs from its witness");
        }
        self.write_immutable_commit_record(
            manifest,
            &record_witness,
            &commit,
            MigrationCommitOutcomeV1::Committed,
        )?;
        self.project_manifest_inventory(manifest, operation_index, direction)?;
        let witness = manifest
            .active_step
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("migration commit has no active witness"))?;
        witness.committed_authority = Some(authority.clone());
        match witness.direction {
            MigrationDirectionV1::Apply => {
                manifest.apply_next += 1;
                manifest.status = "apply_partial".into();
            }
            MigrationDirectionV1::Rollback => {
                manifest.rollback_next += 1;
                manifest.status = "rollback_partial".into();
            }
        }
        manifest.final_authority = Some(authority.clone());
        manifest.last_commit = Some(MigrationCommitProofV1 {
            mutation_id: mutation_id.clone(),
            authority,
        });
        witness.progress_recorded = true;
        self.write_strict_manifest(manifest)?;
        store.consume_migration_witness(&mutation_id)?;
        manifest
            .active_step
            .as_mut()
            .expect("active witness")
            .receipt_consumed = true;
        self.write_strict_manifest(manifest)?;
        manifest.active_step = None;
        self.write_strict_manifest(manifest)
    }
}
