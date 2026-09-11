use super::*;

impl VaultOptimizerService {
    pub(super) fn commit_staged_publication(
        &mut self,
        store: &mut KnowledgeStore,
        mut publication: PendingOptimizerPublication,
        compatibility_update: Option<NoteUpdate>,
    ) -> Result<OptimizerMutationResult<OptimizerAppliedResult>> {
        let expected_authority = publication.expected_authority.clone();
        let change_id = publication.change_id.clone();
        let job_id = publication.job.job_id.clone();
        let note_id = publication.note.id.clone();
        let source_precondition = match &publication.target {
            OptimizerPublicationTarget::Overlay {
                source_relative_path,
                source_digest,
                ..
            } => {
                let source_digest = source_digest.clone().ok_or_else(|| {
                    anyhow::anyhow!("optimizer sidecar witness lacks its source digest")
                });
                match source_digest.and_then(|digest| {
                    store.optimizer_markdown_precondition(source_relative_path, digest)
                }) {
                    Ok(precondition) => Some(precondition),
                    Err(error) => {
                        if expected_authority.is_some() {
                            self.abort_retry_fence_and_defer(&change_id, &job_id, &error, true)?;
                        } else {
                            self.defer_or_park_job_fresh(&job_id, &error)?;
                        }
                        return Ok(OptimizerMutationResult::NoWrite);
                    }
                }
            }
            OptimizerPublicationTarget::Markdown { .. } => None,
        };
        if let Some(precondition) = source_precondition.as_ref() {
            if let Err(error) = precondition.verify().map_err(anyhow::Error::new) {
                if expected_authority.is_some() {
                    self.abort_retry_fence_and_defer(&change_id, &job_id, &error, true)?;
                } else {
                    self.defer_or_park_job_fresh(&job_id, &error)?;
                }
                return Ok(OptimizerMutationResult::NoWrite);
            }
        }
        let source_guard = if expected_authority.is_some() {
            match source_precondition.as_ref() {
                Some(precondition) => match precondition.retained_target() {
                    Ok(target) => Some(target),
                    Err(error) => {
                        let error = anyhow::Error::new(error);
                        self.abort_retry_fence_and_defer(&change_id, &job_id, &error, true)?;
                        return Ok(OptimizerMutationResult::NoWrite);
                    }
                },
                None => None,
            }
        } else {
            None
        };

        // No optimizer state lock is held across this authority CAS. The
        // coordinator hooks durably install Prepared after it has finalized
        // the exact intent and mark Committed before releasing the shared
        // process lock, closing the post-effect publication race.
        let optimizer_root = self.retained_optimizer_root_handle()?;
        let prepared_template = publication.clone();
        let committed_template = publication.clone();
        let fail_prepared_publication = {
            #[cfg(test)]
            {
                std::mem::take(&mut self.fail_prepared_publication_once)
            }
            #[cfg(not(test))]
            {
                false
            }
        };
        let fail_committed_publication = {
            #[cfg(test)]
            {
                std::mem::take(&mut self.fail_committed_publication_once)
            }
            #[cfg(not(test))]
            {
                false
            }
        };
        let post_publication = std::cell::RefCell::new(None);
        let post_error = std::cell::RefCell::new(None);
        #[cfg(test)]
        let mut pause_after_prepared_publication =
            self.pause_after_prepared_publication_once.take();
        let mut prepared_hook = |intent: &crate::services::twin_events::MutationIntentV1| {
            if fail_prepared_publication {
                return Err(crate::services::twin_events::MutationError::Io(
                    "injected optimizer Prepared publication failure".into(),
                ));
            }
            if let Some(precondition) = source_precondition.as_ref() {
                precondition.verify()?;
            }
            let mut prepared = prepared_template.clone();
            let expected = prepared.expected_authority.as_ref().ok_or_else(|| {
                crate::services::twin_events::MutationError::Invalid(
                    "optimizer witness requires an exact source authority".into(),
                )
            })?;
            let targets_match = match &prepared.target {
                OptimizerPublicationTarget::Overlay {
                    note_id,
                    before_digest,
                    after_digest,
                    source_relative_path,
                    source_digest,
                } => {
                    let overlay_before = before_digest.as_ref().map_or(
                        crate::services::twin_events::BeforeImage::Absent,
                        |digest| crate::services::twin_events::BeforeImage::Sha256(digest.clone()),
                    );
                    source_digest.as_ref().is_some_and(|source_digest| {
                        intent.targets.len() == 2
                            && intent.targets.iter().any(|target| {
                                target.kind == crate::services::twin_events::TargetKind::OverlayJson
                                    && target.relative_key == format!("{note_id}.json")
                                    && target.before == overlay_before
                                    && &target.after_digest == after_digest
                            })
                            && intent.targets.iter().any(|target| {
                                target.kind == crate::services::twin_events::TargetKind::Markdown
                                    && target.relative_key == *source_relative_path
                                    && target.before
                                        == crate::services::twin_events::BeforeImage::Sha256(
                                            source_digest.clone(),
                                        )
                                    && target.after_digest == *source_digest
                            })
                    })
                }
                OptimizerPublicationTarget::Markdown {
                    relative_path,
                    before_digest,
                    after_digest,
                } => {
                    intent.targets.len() == 1
                        && intent.targets.first().is_some_and(|target| {
                            target.kind == crate::services::twin_events::TargetKind::Markdown
                                && target.relative_key == *relative_path
                                && target.before
                                    == crate::services::twin_events::BeforeImage::Sha256(
                                        before_digest.clone(),
                                    )
                                && &target.after_digest == after_digest
                        })
                }
            };
            if intent.schema_version != 3
                || !intent.retain_commit_receipt
                || intent.markdown_root_scope.as_ref() != Some(&expected.root_scope)
                || !targets_match
            {
                return Err(crate::services::twin_events::MutationError::Invalid(
                    "optimizer witness does not bind the finalized mutation intent".into(),
                ));
            }
            prepared.phase = OptimizerPublicationPhase::Prepared;
            prepared.mutation_id = Some(intent.mutation_id.clone());
            if prepared.expected_authority.is_some()
                && intent.content_authority_generation
                    != prepared
                        .expected_authority
                        .as_ref()
                        .and_then(|token| token.authority_generation.checked_add(1))
            {
                return Err(crate::services::twin_events::MutationError::Invalid(
                    "optimizer publication authority generation mismatch".into(),
                ));
            }
            write_pending_publication(&optimizer_root, &prepared).map_err(|error| {
                crate::services::twin_events::MutationError::Io(error.to_string())
            })?;
            #[cfg(test)]
            if let Some((entered, resume)) = pause_after_prepared_publication.take() {
                entered.wait();
                resume.wait();
            }
            Ok(())
        };
        let committed_root = self.retained_optimizer_root_handle()?;
        let mut committed_hook = |commit: &crate::services::twin_events::MutationCommit| {
            if fail_committed_publication {
                let error = "injected optimizer Committed publication failure".to_string();
                *post_error.borrow_mut() = Some(error);
                return Err(crate::services::twin_events::MutationError::Io(
                    "optimizer committed publication could not be persisted".into(),
                ));
            }
            let mut committed = committed_template.clone();
            committed.phase = OptimizerPublicationPhase::Committed;
            committed.retry_fenced = true;
            committed.mutation_id = commit.mutation_id.clone();
            committed.committed_authority = commit.authority_token.clone();
            match write_pending_publication(&committed_root, &committed) {
                Ok(()) => {
                    *post_publication.borrow_mut() = Some(committed);
                    Ok(())
                }
                Err(error) => {
                    *post_error.borrow_mut() = Some(error.to_string());
                    Err(crate::services::twin_events::MutationError::Io(
                        "optimizer committed publication could not be persisted".into(),
                    ))
                }
            }
        };

        #[cfg(test)]
        if let Some((entered, resume)) = self.pause_before_prepared_hook_once.take() {
            entered.wait();
            resume.wait();
        }

        let write_result = if let Some(overlay_after) = publication.change.overlay_after.as_ref() {
            if expected_authority.is_some() {
                let material = publication.change.exact_rollback.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("optimizer sidecar publication lacks exact rollback material")
                })?;
                material.validate(&publication.change)?;
                let desired = crate::services::twin_events::DesiredImage::Utf8Bytes(
                    String::from_utf8(serde_json::to_vec_pretty(overlay_after)?)
                        .expect("optimizer overlay serialization is UTF-8"),
                );
                store.commit_exact_optimizer_target_with_hooks(
                    crate::services::knowledge_store::ExactMigrationTarget {
                        kind: crate::services::twin_events::TargetKind::OverlayJson,
                        relative_key: format!("{note_id}.json"),
                        expected_before: material.restore_before.clone(),
                        desired,
                        note_event: Some(
                            crate::services::knowledge_store::ExactMigrationNoteEvent {
                                note_id: note_id.clone(),
                                change: crate::models::twin_event::NoteChangeKind::Updated,
                                observed_at: publication.decision.created_at.ok_or_else(|| {
                                    anyhow::anyhow!("optimizer publication lacks its stable time")
                                })?,
                                governance: material.apply_governance.clone(),
                                payload_digest: material.apply_payload_digest.clone(),
                                evidence_digest: material.apply_evidence_digest.clone(),
                            },
                        ),
                    },
                    source_guard,
                    expected_authority
                        .clone()
                        .expect("authority was checked above"),
                    &mut prepared_hook,
                    &mut committed_hook,
                )
            } else {
                store.write_overlay_from_source_with_authority(
                    &note_id,
                    overlay_after,
                    "vault_optimizer",
                    None,
                )
            }
        } else {
            let before = publication
                .change
                .note_before
                .clone()
                .ok_or_else(|| anyhow::anyhow!("optimizer publication has no source note"))?;
            let before_digest = publication
                .change
                .markdown_before_digest
                .clone()
                .ok_or_else(|| anyhow::anyhow!("optimizer publication has no source digest"))?;
            let exact = publication
                .change
                .note_after
                .clone()
                .ok_or_else(|| anyhow::anyhow!("optimizer publication has no exact note"))?;
            match expected_authority.clone() {
                Some(expected) => store
                    .replace_note_exact_expecting_authority_with_hooks(
                        &note_id,
                        before,
                        before_digest,
                        exact,
                        "vault_optimizer",
                        expected,
                        &mut prepared_hook,
                        &mut committed_hook,
                    )
                    .map(|(_, commit)| commit),
                None => store
                    .update_note_from_source_with_commit(
                        &note_id,
                        compatibility_update.ok_or_else(|| {
                            anyhow::anyhow!("optimizer compatibility update is missing")
                        })?,
                        "vault_optimizer",
                    )
                    .map(|(_, commit)| commit),
            }
        };
        let commit = match write_result {
            Ok(commit) => commit,
            Err(error) => {
                let aborted_precondition = error
                    .downcast_ref::<crate::services::twin_events::MutationError>()
                    .and_then(|error| match error {
                        crate::services::twin_events::MutationError::AbortedPrecondition {
                            mutation_id,
                            authority_advanced,
                        } => Some((mutation_id.clone(), *authority_advanced)),
                        _ => None,
                    });
                if let Some((mutation_id, authority_advanced)) = aborted_precondition {
                    if self.abort_precondition_owner_and_defer(
                        &change_id,
                        &job_id,
                        &mutation_id,
                        &error,
                        Some(store),
                    )? && !authority_advanced
                    {
                        return Ok(OptimizerMutationResult::NoWrite);
                    }
                    return Err(error);
                }
                // A RetryFenced witness proves the Prepared hook never
                // completed, so no authoritative effect could have started.
                // Prepared/Committed witnesses remain for exact rebuild
                // classification and are never cleared optimistically.
                if expected_authority.is_some() {
                    if self.abort_retry_fence_and_defer(&change_id, &job_id, &error, false)? {
                        return Ok(OptimizerMutationResult::NoWrite);
                    }
                }
                return Err(error);
            }
        };
        if expected_authority.is_some()
            && commit.mutation_id.is_none()
            && commit.authority_token.is_none()
        {
            self.complete_retry_fenced_noop(&change_id, &job_id)?;
            return Ok(OptimizerMutationResult::NoWrite);
        }
        publication.phase = OptimizerPublicationPhase::Committed;
        publication.retry_fenced = true;
        publication.mutation_id = commit.mutation_id.clone();
        publication.committed_authority = commit.authority_token.clone();
        let mark_result = if expected_authority.is_none() {
            Ok(())
        } else if let Some(error) = post_error.into_inner() {
            Err(anyhow::anyhow!(error))
        } else if let Some(committed_publication) = post_publication.into_inner() {
            publication = committed_publication;
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "optimizer committed hook did not publish its durable fence"
            ))
        };
        let warning = match mark_result {
            Ok(()) if expected_authority.is_some() => None,
            Ok(()) => match self.with_locked_fresh_state(|service| {
                service.finalize_compatibility_publication(&publication)
            }) {
                Ok(()) => None,
                Err(error) => {
                    log::error!("Optimizer compatibility publication {change_id} failed: {error}");
                    Some(
                        crate::models::mutation::CommittedMutationWarningV1::optimizer_publication_pending(),
                    )
                }
            },
            Err(error) => {
                log::error!(
                "Optimizer authority change {change_id} committed but postwrite publication failed: {error}"
            );
                Some(
                    crate::models::mutation::CommittedMutationWarningV1::optimizer_publication_pending(
                    ),
                )
            }
        };
        Ok(OptimizerMutationResult::Committed {
            result: OptimizerAppliedResult { note_id, change_id },
            commit,
            warning,
        })
    }

    pub(super) fn stage_pending_publication(
        &self,
        pending: &PendingOptimizerPublication,
    ) -> Result<()> {
        write_pending_publication(self.retained_optimizer_root()?, pending)
    }

    pub(super) fn load_pending_publications(
        &self,
    ) -> Result<Vec<(String, PendingOptimizerPublication)>> {
        load_pending_publications(self.retained_optimizer_root()?)
    }

    pub(super) fn complete_retry_fenced_noop(
        &mut self,
        change_id: &str,
        job_id: &str,
    ) -> Result<()> {
        self.with_locked_fresh_state(|service| {
            let pending = service
                .load_pending_publications()?
                .into_iter()
                .find(|(_, pending)| pending.change_id == change_id)
                .map(|(_, pending)| pending)
                .ok_or_else(|| anyhow::anyhow!("optimizer no-op retry fence disappeared"))?;
            if pending.phase != OptimizerPublicationPhase::RetryFenced
                || pending.job.job_id != job_id
            {
                anyhow::bail!("optimizer no-op retry fence changed before cleanup");
            }
            remove_pending_publication(service.retained_optimizer_root()?, change_id)?;
            let before = service.state.queue.len();
            service.remove_queued_job_id(job_id);
            if service.state.queue.len() != before {
                service.state.last_run_at = Some(Utc::now());
                service.persist_state()?;
            }
            Ok(())
        })
    }

    pub(super) fn abort_retry_fence_and_defer(
        &mut self,
        change_id: &str,
        job_id: &str,
        error: &anyhow::Error,
        missing_is_unprepared: bool,
    ) -> Result<bool> {
        let error_message = error.to_string();
        self.with_locked_fresh_state(|service| {
            let pending = service
                .load_pending_publications()?
                .into_iter()
                .find(|(_, pending)| pending.change_id == change_id);
            match pending {
                Some((_, pending))
                    if pending.phase == OptimizerPublicationPhase::RetryFenced
                        && pending.job.job_id == job_id =>
                {
                    remove_pending_publication(service.retained_optimizer_root()?, change_id)?;
                    service.defer_or_park_job_id(job_id, anyhow::anyhow!(error_message))?;
                    Ok(true)
                }
                None if missing_is_unprepared => {
                    service.defer_or_park_job_id(job_id, anyhow::anyhow!(error_message))?;
                    Ok(true)
                }
                _ => Ok(false),
            }
        })
    }

    pub(super) fn abort_precondition_owner_and_defer(
        &mut self,
        change_id: &str,
        job_id: &str,
        mutation_id: &str,
        error: &anyhow::Error,
        store: Option<&KnowledgeStore>,
    ) -> Result<bool> {
        let error_message = error.to_string();
        self.with_locked_fresh_state(|service| {
            let pending = service
                .load_pending_publications()?
                .into_iter()
                .find(|(_, pending)| pending.change_id == change_id)
                .map(|(_, pending)| pending);
            let Some(mut pending) = pending else {
                return Ok(false);
            };
            if pending.phase != OptimizerPublicationPhase::Prepared
                || pending.job.job_id != job_id
                || pending.mutation_id.as_ref().map(|id| id.as_str()) != Some(mutation_id)
            {
                return Ok(false);
            }
            pending.phase = OptimizerPublicationPhase::Aborted;
            pending.committed_authority = None;
            service.stage_pending_publication(&pending)?;
            service.reconcile_aborted_publication_queue(
                &mut pending,
                anyhow::anyhow!(error_message),
            )?;
            let mutation_id = pending
                .mutation_id
                .clone()
                .expect("validated optimizer abort mutation ID");
            store
                .ok_or_else(|| anyhow::anyhow!("optimizer abort proof consumer is unavailable"))?
                .consume_migration_witness(&mutation_id)?;
            remove_pending_publication(service.retained_optimizer_root()?, change_id)?;
            Ok(true)
        })
    }

    pub(super) fn reconcile_aborted_publication_queue(
        &mut self,
        pending: &mut PendingOptimizerPublication,
        error: anyhow::Error,
    ) -> Result<()> {
        if pending.abort_queue_reconciled {
            return Ok(());
        }
        let expected_attempts = pending.job.attempts;
        match self
            .state
            .queue
            .iter()
            .find(|job| job.job_id == pending.job.job_id)
            .map(|job| job.attempts)
        {
            Some(attempts) if attempts == expected_attempts => {
                self.defer_or_park_job_id(&pending.job.job_id, error)?;
            }
            Some(attempts)
                if attempts
                    == expected_attempts
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("optimizer attempt count exhausted"))? =>
            {
                // The state write completed before the owner acknowledgement.
            }
            None if expected_attempts
                .checked_add(1)
                .is_some_and(|attempts| attempts >= MAX_OPTIMIZER_ATTEMPTS)
                && self
                    .state
                    .queue
                    .iter()
                    .all(|job| job.job_id != pending.job.job_id) =>
            {
                // Parking removes the job. The pending owner is the exclusive
                // fence, so absence at the terminal attempt is its durable
                // post-state.
            }
            _ => anyhow::bail!("optimizer aborted publication queue state changed"),
        }
        pending.abort_queue_reconciled = true;
        self.stage_pending_publication(pending)
    }

    pub(super) fn preserve_retry_fence_or_defer(
        &mut self,
        change_id: &str,
        job_id: &str,
        error: &anyhow::Error,
    ) -> Result<()> {
        let error_message = error.to_string();
        self.with_locked_fresh_state(|service| {
            let owners = service
                .load_pending_publications()?
                .into_iter()
                .filter(|(_, pending)| pending.job.job_id == job_id)
                .collect::<Vec<_>>();
            if let Some((_, owner)) = owners.first() {
                if owner.change_id == change_id
                    && owner.phase == OptimizerPublicationPhase::RetryFenced
                {
                    return Ok(());
                }
                return Ok(());
            }
            service.defer_or_park_job_id(job_id, anyhow::anyhow!(error_message))
        })
    }

    pub(super) fn finalize_pending_publication(
        &mut self,
        pending: &mut PendingOptimizerPublication,
    ) -> Result<()> {
        validate_pending_publication(pending)?;
        if pending.phase != OptimizerPublicationPhase::Committed {
            anyhow::bail!("optimizer publication is not durably committed");
        }

        if !pending.audit_written {
            let inbox_entry = publication_inbox_entry(pending)?;
            let event = publication_audit_event(pending)?;
            self.write_change(&pending.change)?;
            self.push_decision_unique(pending.decision.clone())?;
            self.push_inbox_unique(inbox_entry)?;
            self.append_event_unique(&pending.change_id, event)?;
            pending.audit_written = true;
            self.stage_pending_publication(pending)?;
        }

        if !pending.counted {
            let decisions = self.load_decisions()?;
            self.state.accepted_count = decisions
                .iter()
                .filter(|decision| decision.kind == "optimizer_update")
                .count();
            self.state.last_run_at = decisions
                .iter()
                .filter_map(|decision| decision.created_at)
                .max();
            let today = Utc::now().date_naive();
            self.state.daily_write_date = Some(today.to_string());
            self.state.daily_write_count = decisions
                .iter()
                .filter(|decision| {
                    decision.kind == "optimizer_update"
                        && decision
                            .created_at
                            .is_some_and(|created| created.date_naive() == today)
                })
                .count()
                .try_into()
                .map_err(|_| anyhow::anyhow!("optimizer daily write count overflow"))?;
            self.persist_state()?;
            pending.counted = true;
            self.stage_pending_publication(pending)?;
        }

        if !pending.queue_removed {
            self.remove_queued_job_id(&pending.job.job_id);
            self.persist_state()?;
            pending.queue_removed = true;
            self.stage_pending_publication(pending)?;
        }
        Ok(())
    }

    pub(super) fn finalize_compatibility_publication(
        &mut self,
        pending: &PendingOptimizerPublication,
    ) -> Result<()> {
        let inbox_entry = publication_inbox_entry(pending)?;
        let event = publication_audit_event(pending)?;
        let audit_result = (|| -> Result<()> {
            self.write_change(&pending.change)?;
            self.push_decision_unique(pending.decision.clone())?;
            self.push_inbox_unique(inbox_entry)?;
            self.append_event_unique(&pending.change_id, event)
        })();

        if audit_result.is_ok() {
            let decisions = self.load_decisions()?;
            self.state.accepted_count = decisions
                .iter()
                .filter(|decision| decision.kind == "optimizer_update")
                .count();
            self.state.last_run_at = decisions
                .iter()
                .filter_map(|decision| decision.created_at)
                .max();
            let today = Utc::now().date_naive();
            self.state.daily_write_date = Some(today.to_string());
            self.state.daily_write_count = decisions
                .iter()
                .filter(|decision| {
                    decision.kind == "optimizer_update"
                        && decision
                            .created_at
                            .is_some_and(|created| created.date_naive() == today)
                })
                .count()
                .try_into()
                .map_err(|_| anyhow::anyhow!("optimizer daily write count overflow"))?;
        }
        // Even compatibility callers that do not supply an authority token
        // must never retry a write that already returned a commit.
        self.remove_queued_job_id(&pending.job.job_id);
        self.persist_state()?;
        audit_result
    }

    pub(crate) fn recover_pending_publications_locked(
        &mut self,
        _store: &KnowledgeStore,
        guard: &crate::services::twin_events::MutationRootTransitionGuard<'_>,
    ) -> Result<()> {
        cleanup_optimizer_orphan_temps(self.retained_optimizer_root()?)?;
        rollback::recover_pending_rollbacks_locked(self, guard)?;
        for (_, mut pending) in self.load_pending_publications()? {
            if pending.phase == OptimizerPublicationPhase::RetryFenced {
                // This pre-effect witness is both the stable retry identity
                // and the durable reservation for all four audit outputs.
                // It may belong to a live writer waiting for the coordinator,
                // or to a crashed writer; either way, deleting it here races
                // the former and strands the latter. Leave it adoptable by a
                // later optimizer tick.
                continue;
            }
            let expected = pending.expected_authority.as_ref().ok_or_else(|| {
                anyhow::anyhow!("optimizer publication is missing its source authority")
            })?;
            let mutation_id = pending.mutation_id.clone().ok_or_else(|| {
                anyhow::anyhow!("optimizer publication is missing its finalized mutation ID")
            })?;
            if pending.phase == OptimizerPublicationPhase::Aborted {
                self.reconcile_aborted_publication_queue(
                    &mut pending,
                    anyhow::anyhow!("optimizer publication was aborted"),
                )?;
                guard
                    .consume_witnessed_mutation_receipt(&mutation_id)
                    .map_err(anyhow::Error::new)?;
                remove_pending_publication(self.retained_optimizer_root()?, &pending.change_id)?;
                continue;
            }
            if pending.phase == OptimizerPublicationPhase::Committed
                && pending.audit_written
                && pending.counted
                && pending.queue_removed
            {
                // Publication completion is itself a durable proof. This
                // branch makes the receipt-delete / witness-delete crash gap
                // idempotent: a missing receipt can only be accepted after all
                // four monotonic phases were fsynced in the witness.
                guard
                    .consume_witnessed_mutation_receipt(&mutation_id)
                    .map_err(anyhow::Error::new)?;
                remove_pending_publication(self.retained_optimizer_root()?, &pending.change_id)?;
                continue;
            }
            let (kind, key, before, after) = match &pending.target {
                OptimizerPublicationTarget::Overlay {
                    note_id,
                    before_digest,
                    after_digest,
                    ..
                } => (
                    crate::services::twin_events::TargetKind::OverlayJson,
                    format!("{note_id}.json"),
                    before_digest.as_ref().map_or(
                        crate::services::twin_events::BeforeImage::Absent,
                        |digest| crate::services::twin_events::BeforeImage::Sha256(digest.clone()),
                    ),
                    crate::services::twin_events::BeforeImage::Sha256(after_digest.clone()),
                ),
                OptimizerPublicationTarget::Markdown {
                    relative_path,
                    before_digest,
                    after_digest,
                } => (
                    crate::services::twin_events::TargetKind::Markdown,
                    relative_path.clone(),
                    crate::services::twin_events::BeforeImage::Sha256(before_digest.clone()),
                    crate::services::twin_events::BeforeImage::Sha256(after_digest.clone()),
                ),
            };
            match guard
                .classify_witnessed_mutation(&mutation_id, expected, kind, &key, &before, &after)
                .map_err(anyhow::Error::new)?
            {
                crate::services::twin_events::WitnessedMutationRecovery::NotCommitted => {
                    remove_pending_publication(
                        self.retained_optimizer_root()?,
                        &pending.change_id,
                    )?;
                }
                crate::services::twin_events::WitnessedMutationRecovery::Aborted => {
                    pending.phase = OptimizerPublicationPhase::Aborted;
                    pending.committed_authority = None;
                    self.stage_pending_publication(&pending)?;
                    self.reconcile_aborted_publication_queue(
                        &mut pending,
                        anyhow::anyhow!("optimizer publication was aborted before authority"),
                    )?;
                    guard
                        .consume_witnessed_mutation_receipt(&mutation_id)
                        .map_err(anyhow::Error::new)?;
                    remove_pending_publication(
                        self.retained_optimizer_root()?,
                        &pending.change_id,
                    )?;
                }
                crate::services::twin_events::WitnessedMutationRecovery::AbortedAfterAuthority(
                    commit,
                ) => {
                    pending.phase = OptimizerPublicationPhase::Aborted;
                    pending.committed_authority = commit.authority_token;
                    self.stage_pending_publication(&pending)?;
                    self.reconcile_aborted_publication_queue(
                        &mut pending,
                        anyhow::anyhow!("optimizer publication was aborted after authority"),
                    )?;
                    guard
                        .consume_witnessed_mutation_receipt(&mutation_id)
                        .map_err(anyhow::Error::new)?;
                    remove_pending_publication(
                        self.retained_optimizer_root()?,
                        &pending.change_id,
                    )?;
                }
                crate::services::twin_events::WitnessedMutationRecovery::Committed(commit) => {
                    if pending.phase == OptimizerPublicationPhase::Committed
                        && pending.committed_authority != commit.authority_token
                    {
                        anyhow::bail!("optimizer committed witness authority mismatch");
                    }
                    pending.phase = OptimizerPublicationPhase::Committed;
                    pending.retry_fenced = true;
                    pending.committed_authority = commit.authority_token;
                    self.stage_pending_publication(&pending)?;
                    self.finalize_pending_publication(&mut pending)?;
                    guard
                        .consume_witnessed_mutation_receipt(&mutation_id)
                        .map_err(anyhow::Error::new)?;
                    remove_pending_publication(
                        self.retained_optimizer_root()?,
                        &pending.change_id,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Records a processing failure for `job`. Below `MAX_OPTIMIZER_ATTEMPTS`
    /// the job stays queued (in its original position) with `attempts`
    /// incremented, so it's retried on a later tick. At the limit it's parked:
    /// removed from the queue and recorded in the inbox with status `"failed"`
    /// so a human can see it, instead of spinning on a poisoned entry forever.
    pub(super) fn defer_or_park_job(
        &mut self,
        job: QueuedOptimizerNote,
        error: anyhow::Error,
    ) -> Result<()> {
        self.defer_or_park_job_id(&job.job_id, error)
    }

    pub(super) fn defer_or_park_job_fresh(
        &mut self,
        job_id: &str,
        error: &anyhow::Error,
    ) -> Result<()> {
        let error_message = error.to_string();
        self.with_locked_fresh_state(|service| {
            if service
                .load_pending_publications()?
                .into_iter()
                .any(|(_, pending)| pending.job.job_id == job_id)
            {
                return Ok(());
            }
            service.defer_or_park_job_id(job_id, anyhow::anyhow!(error_message))
        })
    }

    pub(super) fn defer_or_park_job_id(
        &mut self,
        job_id: &str,
        error: anyhow::Error,
    ) -> Result<()> {
        let Some(position) = self
            .state
            .queue
            .iter()
            .position(|entry| entry.job_id == job_id)
        else {
            return Ok(());
        };
        let mut job = self.state.queue[position].clone();
        if job.pending_parking.is_some() {
            return self.finish_pending_parking(&job);
        }
        job.attempts = job
            .attempts
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("optimizer attempt count exhausted"))?;
        log::warn!(
            "Vault optimizer job for note '{}' failed (attempt {}/{}): {}",
            job.note_id,
            job.attempts,
            MAX_OPTIMIZER_ATTEMPTS,
            error
        );

        if job.attempts >= MAX_OPTIMIZER_ATTEMPTS {
            job.pending_parking = Some(PendingOptimizerParkingV1 {
                error: error.to_string(),
                at: Utc::now(),
            });
            self.state.queue[position] = job.clone();
            // The still-queued job is the durable owner for all later
            // publications. A restart resumes this exact parking record
            // before it can process or retry the job again.
            self.persist_state()?;
            return self.finish_pending_parking(&job);
        } else {
            self.state.queue[position].attempts = job.attempts;
        }

        self.persist_state()?;
        Ok(())
    }

    pub(super) fn recover_pending_parkings(&mut self) -> Result<()> {
        let pending = self
            .state
            .queue
            .iter()
            .filter(|job| job.pending_parking.is_some())
            .cloned()
            .collect::<Vec<_>>();
        for job in pending {
            self.finish_pending_parking(&job)?;
        }
        Ok(())
    }

    pub(super) fn finish_pending_parking(&mut self, job: &QueuedOptimizerNote) -> Result<()> {
        let parking = job
            .pending_parking
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("optimizer job lacks its parking witness"))?;
        let inbox_entry = VaultOptimizerInboxEntry {
            id: job.job_id.clone(),
            note_id: Some(job.note_id.clone()),
            status: "failed".to_string(),
            title: job.note_id.clone(),
            reason: format!(
                "Vault optimizer parked after {} failed attempts: {}",
                job.attempts, parking.error
            ),
            diff_preview: String::new(),
            confidence: 0.0,
            created_at: Some(parking.at),
            change_id: None,
        };
        self.push_inbox_unique(inbox_entry)?;
        self.append_event_unique(
            &job.job_id,
            OptimizerAuditEventV1::OptimizerParked {
                job_id: job.job_id.clone(),
                note_id: job.note_id.clone(),
                attempts: job.attempts,
                error: parking.error.clone(),
                at: parking.at,
            },
        )?;
        #[cfg(test)]
        if std::mem::take(&mut self.fail_terminal_parking_after_publication_once) {
            anyhow::bail!("injected crash after terminal parking publication");
        }
        self.remove_queued_job_id(&job.job_id);
        self.persist_state()
    }

    pub(super) fn remove_queued_job(&mut self, note_id: &str) {
        if let Some(pos) = self
            .state
            .queue
            .iter()
            .position(|entry| entry.note_id == note_id)
        {
            self.state.queue.remove(pos);
        }
    }

    pub(super) fn remove_queued_job_id(&mut self, job_id: &str) {
        if let Some(position) = self
            .state
            .queue
            .iter()
            .position(|entry| entry.job_id == job_id)
        {
            self.state.queue.remove(position);
        }
    }

    pub(super) fn complete_noop_job(&mut self, note_id: &str) -> Result<()> {
        self.remove_queued_job(note_id);
        self.state.last_run_at = Some(Utc::now());
        self.persist_state()
    }

    pub(super) fn complete_noop_job_fresh(&mut self, job_id: &str) -> Result<()> {
        self.with_locked_fresh_state(|service| {
            if service
                .load_pending_publications()?
                .into_iter()
                .any(|(_, pending)| pending.job.job_id == job_id)
            {
                return Ok(());
            }
            let before = service.state.queue.len();
            service.remove_queued_job_id(job_id);
            if service.state.queue.len() == before {
                return Ok(());
            }
            service.state.last_run_at = Some(Utc::now());
            service.persist_state()
        })
    }

    /// Whether today's write count has already reached
    /// `background_vault_optimizer_max_daily_writes`. Only meaningful once at
    /// least one write has happened today; a fresh day always reports `false`
    /// regardless of yesterday's count.
    pub(super) fn daily_write_cap_reached(&self, settings: &UserSettings) -> bool {
        let today = Utc::now().date_naive().to_string();
        self.state.daily_write_date.as_deref() == Some(today.as_str())
            && self.state.daily_write_count >= settings.background_vault_optimizer_max_daily_writes
    }

    pub(super) fn persist_state(&mut self) -> Result<()> {
        let previous_revision = self.state.state_revision;
        self.state.schema_version = OPTIMIZER_STATE_SCHEMA_VERSION;
        self.state.state_revision = self
            .state
            .state_revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("vault optimizer state revision exhausted"))?;
        let bytes = serde_json::to_vec_pretty(&self.state)?;
        if bytes.len() > MAX_OPTIMIZER_STATE_BYTES {
            self.state.state_revision = previous_revision;
            anyhow::bail!("vault optimizer state exceeds its 4 MiB limit");
        }
        let result = self
            .retained_optimizer_root()?
            .put_atomic(QUEUE_KEY, &bytes)
            .map_err(anyhow::Error::new);
        if let Err(error) = result {
            self.state.state_revision = previous_revision;
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn pending_audit_reservations(
        &self,
        exclude_change_id: Option<&str>,
    ) -> Result<Vec<PendingOptimizerPublication>> {
        Ok(self
            .load_pending_publications()?
            .into_iter()
            .map(|(_, pending)| pending)
            .filter(|pending| {
                !pending.audit_written && exclude_change_id != Some(pending.change_id.as_str())
            })
            .collect())
    }

    pub(super) fn ensure_decision_capacity(
        &self,
        additional: &VaultOptimizerDecision,
        exclude_change_id: Option<&str>,
    ) -> Result<()> {
        let mut decisions = self.load_decisions()?;
        for pending in self.pending_audit_reservations(exclude_change_id)? {
            merge_optimizer_decision(&mut decisions, pending.decision)?;
        }
        merge_optimizer_decision(&mut decisions, additional.clone())?;
        validate_json_audit_capacity(&decisions, "optimizer decisions")
    }

    pub(super) fn ensure_inbox_capacity(
        &self,
        additional: &VaultOptimizerInboxEntry,
        exclude_change_id: Option<&str>,
    ) -> Result<()> {
        let mut inbox = self.load_inbox()?;
        for pending in self.pending_audit_reservations(exclude_change_id)? {
            merge_optimizer_inbox(&mut inbox, publication_inbox_entry(&pending)?)?;
        }
        merge_optimizer_inbox(&mut inbox, additional.clone())?;
        validate_json_audit_capacity(&inbox, "optimizer inbox")
    }

    pub(super) fn ensure_event_capacity(
        &self,
        additional: &OptimizerAuditEventV1,
        exclude_change_id: Option<&str>,
    ) -> Result<()> {
        let mut events = self.load_events()?;
        for pending in self.pending_audit_reservations(exclude_change_id)? {
            merge_optimizer_event(&mut events, publication_audit_event(&pending)?)?;
        }
        merge_optimizer_event(&mut events, additional.clone())?;
        serialize_optimizer_events(&events).map(|_| ())
    }

    pub(super) fn ensure_change_capacity(
        &self,
        additional: &OptimizerChange,
        exclude_change_id: Option<&str>,
    ) -> Result<()> {
        let root = self.retained_optimizer_root()?;
        let names = root
            .regular_file_names(CHANGES_DIRECTORY)
            .map_err(anyhow::Error::new)?;
        if names.len() > MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer change audit exceeds 4096 entries");
        }
        let mut changes = HashMap::new();
        for name in names {
            let change_id = name
                .strip_suffix(".json")
                .ok_or_else(|| anyhow::anyhow!("invalid optimizer change filename"))?;
            merge_optimizer_change(&mut changes, self.read_change(change_id)?)?;
        }
        for pending in self.pending_audit_reservations(exclude_change_id)? {
            merge_optimizer_change(&mut changes, pending.change)?;
        }
        merge_optimizer_change(&mut changes, additional.clone())?;
        if changes.len() > MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer change audit has reached 4096 entries");
        }
        for change in changes.values() {
            if serde_json::to_vec_pretty(change)?.len() > MAX_OPTIMIZER_CHANGE_BYTES {
                anyhow::bail!("optimizer change exceeds its 8 MiB limit");
            }
        }
        Ok(())
    }

    pub(super) fn preflight_publication_audit(
        &self,
        pending: &PendingOptimizerPublication,
    ) -> Result<()> {
        validate_pending_publication(pending)?;
        let inbox = publication_inbox_entry(pending)?;
        let event = publication_audit_event(pending)?;
        self.ensure_decision_capacity(&pending.decision, None)?;
        self.ensure_inbox_capacity(&inbox, None)?;
        self.ensure_event_capacity(&event, None)?;
        self.ensure_change_capacity(&pending.change, None)
    }

    pub(super) fn load_decisions(&self) -> Result<Vec<VaultOptimizerDecision>> {
        load_bounded_json_vec(
            self.retained_optimizer_root()?,
            DECISIONS_KEY,
            "optimizer decisions",
        )
    }

    pub(super) fn push_decision_unique(&self, decision: VaultOptimizerDecision) -> Result<()> {
        self.ensure_decision_capacity(&decision, decision.change_id.as_deref())?;
        let mut decisions = self.load_decisions()?;
        if let Some(existing) = decisions
            .iter()
            .find(|existing| existing.id == decision.id || existing.change_id == decision.change_id)
        {
            if existing == &decision {
                return Ok(());
            }
            anyhow::bail!("optimizer decision identity collision");
        }
        if decisions.len() >= MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer decisions have reached 4096 entries");
        }
        decisions.push(decision);
        write_bounded_json_vec(self.retained_optimizer_root()?, DECISIONS_KEY, &decisions)
    }

    pub(super) fn load_inbox(&self) -> Result<Vec<VaultOptimizerInboxEntry>> {
        load_bounded_json_vec(
            self.retained_optimizer_root()?,
            INBOX_KEY,
            "optimizer inbox",
        )
    }

    #[cfg(test)]
    pub(super) fn push_inbox(&self, entry: VaultOptimizerInboxEntry) -> Result<()> {
        self.ensure_inbox_capacity(&entry, None)?;
        let mut inbox = self.load_inbox()?;
        if inbox.len() >= MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer inbox has reached 4096 entries");
        }
        inbox.push(entry);
        write_bounded_json_vec(self.retained_optimizer_root()?, INBOX_KEY, &inbox)
    }

    pub(super) fn push_inbox_unique(&self, entry: VaultOptimizerInboxEntry) -> Result<()> {
        self.ensure_inbox_capacity(&entry, entry.change_id.as_deref())?;
        let mut inbox = self.load_inbox()?;
        if let Some(existing) = inbox.iter().find(|existing| {
            existing.id == entry.id
                || (entry.change_id.is_some() && existing.change_id == entry.change_id)
        }) {
            if existing == &entry {
                return Ok(());
            }
            anyhow::bail!("optimizer inbox identity collision");
        }
        if inbox.len() >= MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer inbox has reached 4096 entries");
        }
        inbox.push(entry);
        write_bounded_json_vec(self.retained_optimizer_root()?, INBOX_KEY, &inbox)
    }

    pub(super) fn write_change(&self, change: &OptimizerChange) -> Result<()> {
        parse_canonical_uuid(&change.change_id, "optimizer change ID")?;
        self.ensure_change_capacity(change, Some(change.change_id.as_str()))?;
        let root = self.retained_optimizer_root()?;
        let key = format!("{CHANGES_DIRECTORY}/{}.json", change.change_id);
        if let Some(bytes) = root
            .read_bounded(&key, MAX_OPTIMIZER_CHANGE_BYTES)
            .map_err(anyhow::Error::new)?
        {
            let existing: OptimizerChange =
                serde_json::from_slice(&bytes).context("invalid optimizer change audit")?;
            if serde_json::to_value(existing)? == serde_json::to_value(change)? {
                return Ok(());
            }
            anyhow::bail!("optimizer change identity collision");
        }
        let bytes = serde_json::to_vec_pretty(change)?;
        if bytes.len() > MAX_OPTIMIZER_CHANGE_BYTES {
            anyhow::bail!("optimizer change exceeds its 8 MiB limit");
        }
        root.put_atomic(&key, &bytes).map_err(anyhow::Error::new)
    }

    pub(super) fn read_change(&self, change_id: &str) -> Result<OptimizerChange> {
        parse_canonical_uuid(change_id, "optimizer change ID")?;
        let key = format!("{CHANGES_DIRECTORY}/{change_id}.json");
        let bytes = self
            .retained_optimizer_root()?
            .read_bounded(&key, MAX_OPTIMIZER_CHANGE_BYTES)
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow::anyhow!("optimizer change does not exist"))?;
        let change: OptimizerChange =
            serde_json::from_slice(&bytes).context("invalid optimizer change audit")?;
        if change.change_id != change_id {
            anyhow::bail!("optimizer change identity mismatch");
        }
        if let Some(material) = change.exact_rollback.as_ref() {
            material.validate(&change)?;
        }
        Ok(change)
    }

    pub(super) fn load_events(&self) -> Result<Vec<OptimizerAuditEventV1>> {
        let Some(bytes) = self
            .retained_optimizer_root()?
            .read_bounded(EVENTS_KEY, MAX_OPTIMIZER_AUDIT_BYTES)
            .map_err(anyhow::Error::new)?
        else {
            return Ok(Vec::new());
        };
        let contents = std::str::from_utf8(&bytes).context("optimizer event audit is not UTF-8")?;
        let mut events = Vec::new();
        for (index, line) in contents.lines().enumerate() {
            if index >= MAX_OPTIMIZER_AUDIT_ENTRIES {
                anyhow::bail!("optimizer event audit exceeds 4096 entries");
            }
            if line.is_empty() {
                anyhow::bail!("optimizer event audit contains an empty record");
            }
            events.push(
                serde_json::from_str::<OptimizerAuditEventV1>(line).with_context(|| {
                    format!("invalid optimizer event audit at line {}", index + 1)
                })?,
            );
        }
        Ok(events)
    }

    pub(super) fn write_events(&self, events: &[OptimizerAuditEventV1]) -> Result<()> {
        let bytes = serialize_optimizer_events(events)?;
        self.retained_optimizer_root()?
            .put_atomic(EVENTS_KEY, &bytes)
            .map_err(anyhow::Error::new)
    }

    #[cfg(test)]
    pub(super) fn append_event(&self, event: OptimizerAuditEventV1) -> Result<()> {
        self.ensure_event_capacity(&event, None)?;
        let mut events = self.load_events()?;
        if events.len() >= MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer event audit has reached 4096 entries");
        }
        events.push(event);
        self.write_events(&events)
    }

    pub(super) fn append_event_unique(
        &self,
        identity: &str,
        event: OptimizerAuditEventV1,
    ) -> Result<()> {
        self.ensure_event_capacity(&event, Some(identity))?;
        let mut events = self.load_events()?;
        if let Some(existing) = events.iter().find(|existing| {
            matches!(
                existing,
                OptimizerAuditEventV1::OptimizerApply {
                    change_id: existing_id,
                    ..
                } if matches!(&event, OptimizerAuditEventV1::OptimizerApply { .. })
                    && existing_id == identity
            ) || matches!(
                existing,
                OptimizerAuditEventV1::Rollback {
                    change_id: existing_id,
                    ..
                } if matches!(&event, OptimizerAuditEventV1::Rollback { .. })
                    && existing_id == identity
            ) || matches!(
                existing,
                OptimizerAuditEventV1::OptimizerParked { job_id, .. }
                    if matches!(&event, OptimizerAuditEventV1::OptimizerParked { .. })
                        && !job_id.is_empty()
                        && job_id == identity
            )
        }) {
            if existing == &event {
                return Ok(());
            }
            anyhow::bail!("optimizer event audit identity collision");
        }
        if events.len() >= MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer event audit has reached 4096 entries");
        }
        events.push(event);
        self.write_events(&events)
    }
}
