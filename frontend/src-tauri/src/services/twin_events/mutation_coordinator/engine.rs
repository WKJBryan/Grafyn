use super::*;

impl MutationCoordinator {
    pub(super) fn prepare_intent(
        &self,
        process_lock: &CoordinatorProcessLock,
        origin: MutationOrigin,
        requested_stream: CausalStream,
        source_channel: crate::models::twin_event::SourceChannel,
        targets: Vec<crate::services::twin_events::TargetMutation>,
        drafts: Vec<TwinEventDraft>,
        retain_commit_receipt: bool,
    ) -> Result<Option<crate::services::twin_events::MutationIntentV1>, MutationError> {
        if targets.len() > crate::services::twin_events::MAX_INTENT_TARGETS {
            return Err(MutationError::Invalid(
                "local mutation must contain at most 64 targets".into(),
            ));
        }
        if targets.is_empty() && drafts.is_empty() {
            return Err(MutationError::Invalid(
                "mutation must contain a target or event draft".into(),
            ));
        }
        let event_only = targets.is_empty();
        let mut targets = targets;
        targets.sort_by(|left, right| {
            (left.kind, left.relative_key.as_str()).cmp(&(right.kind, right.relative_key.as_str()))
        });
        let mut physical_keys = std::collections::BTreeSet::new();
        for target in &targets {
            let key = self.physical_target_id(target.kind, &target.relative_key)?;
            if !physical_keys.insert(key) {
                return Err(MutationError::Invalid(
                    "duplicate or aliased mutation target".into(),
                ));
            }
        }
        let mut prepared_targets = Vec::new();
        let mut has_writable_target = false;
        for target in targets {
            crate::services::twin_events::validate_target_key(target.kind, &target.relative_key)?;
            let before = self.before_image(target.kind, &target.relative_key)?;
            let after_digest = crate::services::twin_events::desired_digest(&target.after);
            let matches_after = target_matches_after(&before, &target.after, &after_digest);
            if target.check_expected_before_before_after_elision
                && target
                    .expected_before
                    .as_ref()
                    .is_some_and(|expected| expected != &before)
            {
                return Err(MutationError::RecoveryConflict(format!(
                    "strict conditional mutation target changed: {}",
                    target.relative_key
                )));
            }
            if matches_after && !target.retain_exact_precondition {
                continue;
            }
            if target
                .expected_before
                .as_ref()
                .is_some_and(|expected| expected != &before)
            {
                return Err(MutationError::RecoveryConflict(format!(
                    "conditional mutation target changed: {}",
                    target.relative_key
                )));
            }
            if target.retain_exact_precondition {
                if !retain_commit_receipt || !matches_after {
                    return Err(MutationError::Invalid(
                        "exact mutation preconditions require a retained schema-3 intent and an unchanged source"
                            .into(),
                    ));
                }
            } else {
                has_writable_target = true;
            }
            prepared_targets.push(crate::services::twin_events::MutationTargetV1 {
                kind: target.kind,
                relative_key: target.relative_key,
                before,
                after: target.after,
                after_digest,
            });
        }
        if prepared_targets.is_empty() && !event_only {
            return Ok(None);
        }
        if !has_writable_target && drafts.is_empty() {
            return Ok(None);
        }
        if origin != MutationOrigin::Local && !drafts.is_empty() {
            return Err(MutationError::Invalid(
                "nonlocal mutations cannot generate local events".into(),
            ));
        }
        let drafts = drafts
            .into_iter()
            .map(|mut draft| {
                draft.context.source_channel = source_channel.clone();
                draft
            })
            .collect::<Vec<_>>();
        let events = if drafts.is_empty() {
            Vec::new()
        } else {
            self.finalizer.finalize_locked(requested_stream, &drafts)?
        };
        let stream = events
            .first()
            .map_or(requested_stream, |event| event.causal_stream);
        let changes_authority = !events.is_empty()
            || prepared_targets.iter().any(|target| {
                matches!(
                    target.kind,
                    crate::services::twin_events::TargetKind::Markdown
                        | crate::services::twin_events::TargetKind::OverlayJson
                        | crate::services::twin_events::TargetKind::TwinJson
                )
            });
        let markdown_root_scope = if prepared_targets.iter().any(|target| {
            matches!(
                target.kind,
                crate::services::twin_events::TargetKind::Markdown
                    | crate::services::twin_events::TargetKind::OverlayJson
            )
        }) || (retain_commit_receipt && changes_authority)
        {
            Some(self.current_markdown_root_scope()?)
        } else {
            None
        };
        let content_authority_generation = if changes_authority {
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let current = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                process_lock,
            )?;
            Some(current.authority_generation.checked_add(1).ok_or_else(|| {
                MutationError::RecoveryConflict("authority-generation-exhausted".into())
            })?)
        } else {
            None
        };
        let mut intent = crate::services::twin_events::MutationIntentV1 {
            schema_version: if retain_commit_receipt { 3 } else { 2 },
            mutation_id: crate::services::twin_events::digest_bytes(b"placeholder"),
            origin,
            actor_id: self.finalizer.actor_id(),
            device_id: self.finalizer.device_id(),
            causal_stream: stream,
            source_channel,
            markdown_root_scope,
            content_authority_generation,
            retain_commit_receipt,
            targets: prepared_targets,
            events,
            created_at: Utc::now(),
        };
        intent.mutation_id = crate::services::twin_events::derive_mutation_id(&intent);
        intent.validate()?;
        self.store.preflight_append_group(&intent.events)?;
        let serialized = serde_json::to_vec(&intent)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        if serialized.len() > crate::services::twin_events::MAX_SERIALIZED_INTENT_BYTES {
            return Err(MutationError::Invalid(
                "serialized mutation intent exceeds the 32 MiB limit".into(),
            ));
        }
        Ok(Some(intent))
    }

    pub(super) fn recover_preauthority_locked(
        &self,
        process_lock: &CoordinatorProcessLock,
    ) -> Result<usize, MutationError> {
        let markers = self.journal.load_preauthority(process_lock)?;
        let mut recovered = 0usize;
        for (_, marker) in markers {
            match marker.state {
                crate::services::twin_events::PreAuthorityMutationStateV1::AbortedBeforeAuthority => {
                    if !marker.intent.retain_commit_receipt {
                        self.journal.consume_aborted_preauthority(
                            process_lock,
                            &marker.intent.mutation_id,
                        )?;
                    }
                    continue;
                }
                crate::services::twin_events::PreAuthorityMutationStateV1::AbortedAfterAuthority => {
                    if self
                        .journal
                        .remove_matching_aborted_wal(process_lock, &marker)?
                    {
                        recovered += 1;
                    }
                    if marker.intent.origin == MutationOrigin::Local
                        && !marker.intent.events.is_empty()
                    {
                        self.lifecycle.known_failure(
                            Some(marker.intent.mutation_id.as_str()),
                            "mutation aborted after authority ownership",
                        );
                    }
                    if !marker.intent.retain_commit_receipt {
                        self.journal.consume_aborted_preauthority(
                            process_lock,
                            &marker.intent.mutation_id,
                        )?;
                    }
                    continue;
                }
                crate::services::twin_events::PreAuthorityMutationStateV1::Prepared => {}
            }
            self.verify_root_lease_locked()?;
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let current = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                process_lock,
            )?;
            if current.root_scope != marker.expected_authority.root_scope
                || current.lease_epoch_uuid != marker.expected_authority.lease_epoch_uuid
            {
                return Err(MutationError::RecoveryConflict(
                    "pre-authority mutation belongs to another root lease".into(),
                ));
            }
            let intended_generation = marker
                .expected_authority
                .authority_generation
                .checked_add(1)
                .ok_or_else(|| {
                    MutationError::RecoveryConflict("authority-generation-exhausted".into())
                })?;
            if current.authority_generation == marker.expected_authority.authority_generation {
                if self
                    .journal
                    .load_pending(process_lock)?
                    .iter()
                    .any(|(_, pending)| pending.mutation_id == marker.intent.mutation_id)
                    || self
                        .journal
                        .load_committed_receipt(process_lock, &marker.intent.mutation_id)?
                        .is_some()
                {
                    return Err(MutationError::RecoveryConflict(
                        "pre-authority mutation has impossible durable progress".into(),
                    ));
                }
                // The full intent marker proves planning ownership, but no
                // authority effect. Retiring it lets the owner revalidate its
                // wider source snapshot before retrying and never attributes a
                // later peer generation to this mutation.
                self.journal.abort_preauthority(process_lock, &marker)?;
                if marker.intent.origin == MutationOrigin::Local && !marker.intent.events.is_empty()
                {
                    self.lifecycle.known_failure(
                        Some(marker.intent.mutation_id.as_str()),
                        "mutation aborted before authority ownership",
                    );
                }
                continue;
            }
            if current.authority_generation != intended_generation {
                return Err(MutationError::RecoveryConflict(
                    "pre-authority mutation authority is neither before nor owned-after".into(),
                ));
            }
            if self
                .journal
                .load_committed_receipt(process_lock, &marker.intent.mutation_id)?
                .is_some()
            {
                return Err(MutationError::RecoveryConflict(
                    "pre-authority owner cannot coexist with a committed receipt".into(),
                ));
            }
            let mut exact_guard_drifted = false;
            for target in &marker.intent.targets {
                let durable = self.before_image(target.kind, &target.relative_key)?;
                if target_is_exact_precondition(&marker.intent, target) {
                    exact_guard_drifted |= durable != target.before;
                } else if durable != target.before {
                    return Err(MutationError::RecoveryConflict(
                        "pre-authority mutation target changed before WAL promotion".into(),
                    ));
                }
            }
            if exact_guard_drifted {
                self.journal.retain_aborted_after_authority(
                    process_lock,
                    &marker.intent,
                    &current,
                )?;
                if marker.intent.origin == MutationOrigin::Local && !marker.intent.events.is_empty()
                {
                    self.lifecycle.known_failure(
                        Some(marker.intent.mutation_id.as_str()),
                        "mutation exact guard changed after authority ownership",
                    );
                }
                continue;
            }
            self.journal
                .preflight_commit_receipt_slot(process_lock, &marker.intent, &current)?;
            if let Err(error) = self.journal.promote_preauthority(process_lock, &marker) {
                return Err(MutationError::AuthorityAdvanced {
                    mutation_id: marker.intent.mutation_id.clone(),
                    authority_token: current,
                    target_aborted: false,
                    reason: error.to_string(),
                });
            }
            if let Err(error) = self.replay_intent_locked(process_lock, &marker.intent, false, true)
            {
                return match error {
                    error @ MutationError::AuthorityAdvanced { .. } => Err(error),
                    error => Err(MutationError::AuthorityAdvanced {
                        mutation_id: marker.intent.mutation_id.clone(),
                        authority_token: current,
                        target_aborted: false,
                        reason: error.to_string(),
                    }),
                };
            }
            recovered += 1;
        }
        Ok(recovered)
    }

    pub(super) fn replay_intent_locked(
        &self,
        process_lock: &CoordinatorProcessLock,
        intent: &crate::services::twin_events::MutationIntentV1,
        inject_faults: bool,
        cleanup: bool,
    ) -> Result<(), MutationError> {
        intent.validate()?;
        self.store.preflight_append_group(&intent.events)?;
        if let Some(intent_scope) = &intent.markdown_root_scope {
            if &self.current_markdown_root_scope()? != intent_scope {
                self.journal.quarantine_intent(process_lock, intent)?;
                return Err(MutationError::RecoveryConflict(
                    intent.mutation_id.as_str().to_string(),
                ));
            }
        }
        if matches!(intent.schema_version, 2 | 3) && intent_changes_authority(intent) {
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let current = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                process_lock,
            )?;
            if Some(current.authority_generation) != intent.content_authority_generation {
                self.journal.quarantine_intent(process_lock, intent)?;
                return Err(MutationError::RecoveryConflict(
                    intent.mutation_id.as_str().to_string(),
                ));
            }
        }
        #[cfg(test)]
        {
            let mut remaining = self
                .replay_failures_before_targets
                .lock()
                .map_err(|_| MutationError::Invalid("replay fault lock poisoned".into()))?;
            if *remaining > 0 {
                *remaining -= 1;
                return Err(MutationError::Io(
                    "injected mutation replay failure before targets".into(),
                ));
            }
        }
        let mut classifications = Vec::with_capacity(intent.targets.len());
        for target in &intent.targets {
            let current = self.before_image(target.kind, &target.relative_key)?;
            let classification = if current == target.before {
                TargetClassification::Before
            } else if target_matches_after(&current, &target.after, &target.after_digest) {
                TargetClassification::After
            } else {
                TargetClassification::Third
            };
            classifications.push(classification);
        }
        let writable_targets = intent
            .targets
            .iter()
            .enumerate()
            .filter(|(_, target)| !target_is_exact_precondition(intent, target))
            .collect::<Vec<_>>();
        if writable_targets
            .iter()
            .any(|(index, _)| classifications[*index] == TargetClassification::Third)
        {
            self.journal.quarantine_intent(process_lock, intent)?;
            return Err(MutationError::RecoveryConflict(
                intent.mutation_id.as_str().to_string(),
            ));
        }
        let any_writable_after = writable_targets
            .iter()
            .any(|(index, _)| classifications[*index] == TargetClassification::After);
        if !any_writable_after
            && intent.targets.iter().enumerate().any(|(index, target)| {
                target_is_exact_precondition(intent, target)
                    && classifications[index] != TargetClassification::Before
            })
        {
            // Authority already advanced, so publish exact durable negative
            // ownership proof before removing the WAL. The external owner must
            // acknowledge this proof before it can be consumed.
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let authority = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                process_lock,
            )?;
            self.journal
                .retain_aborted_after_authority(process_lock, intent, &authority)?;
            if inject_faults {
                if let Err(error) = self.inject(MutationFaultPoint::AfterPostAuthorityAbortProof) {
                    return Err(MutationError::AuthorityAdvanced {
                        mutation_id: intent.mutation_id.clone(),
                        authority_token: authority,
                        target_aborted: true,
                        reason: error.to_string(),
                    });
                }
            }
            self.journal.remove(process_lock, intent)?;
            if cleanup {
                return Ok(());
            }
            return Err(MutationError::AuthorityAdvanced {
                mutation_id: intent.mutation_id.clone(),
                authority_token: authority,
                target_aborted: true,
                reason: "exact source guard changed after authority ownership".into(),
            });
        }

        let mut application_order = intent
            .targets
            .iter()
            .enumerate()
            .filter(|(index, target)| {
                !target_is_exact_precondition(intent, target)
                    && classifications[*index] == TargetClassification::Before
                    && !matches!(
                        target.after,
                        crate::services::twin_events::DesiredImage::Tombstone
                    )
            })
            .chain(intent.targets.iter().enumerate().filter(|(index, target)| {
                !target_is_exact_precondition(intent, target)
                    && classifications[*index] == TargetClassification::Before
                    && matches!(
                        target.after,
                        crate::services::twin_events::DesiredImage::Tombstone
                    )
            }))
            .collect::<Vec<_>>();
        for (applied_index, (_, target)) in application_order.drain(..).enumerate() {
            self.apply_intent_target(target)?;
            if inject_faults {
                self.inject(MutationFaultPoint::AfterTarget(applied_index))?;
            }
        }
        if inject_faults {
            self.inject(MutationFaultPoint::AfterTargets)?;
        }
        for (index, event) in intent.events.iter().enumerate() {
            self.store.append(event.clone())?;
            if inject_faults {
                self.inject(MutationFaultPoint::AfterEvent(index))?;
            }
        }
        if intent.retain_commit_receipt {
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let authority = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                process_lock,
            )?;
            self.journal
                .retain_committed_receipt(process_lock, intent, &authority)?;
        }
        if inject_faults {
            self.inject(MutationFaultPoint::BeforeCleanup)?;
        }
        if cleanup {
            self.journal.remove(process_lock, intent)?;
            if inject_faults {
                self.inject(MutationFaultPoint::AfterCleanupBeforeFanout)?;
            }
        }
        Ok(())
    }

    pub(super) fn validate_exact_preconditions_locked(
        &self,
        intent: &crate::services::twin_events::MutationIntentV1,
    ) -> Result<(), MutationError> {
        for target in intent
            .targets
            .iter()
            .filter(|target| target_is_exact_precondition(intent, target))
        {
            let current = self.before_image(target.kind, &target.relative_key)?;
            if current != target.before {
                return Err(MutationError::AbortedPrecondition {
                    mutation_id: intent.mutation_id.as_str().to_string(),
                    authority_advanced: false,
                });
            }
        }
        Ok(())
    }

    pub(super) fn validate_all_targets_before_locked(
        &self,
        intent: &crate::services::twin_events::MutationIntentV1,
    ) -> Result<(), MutationError> {
        for target in &intent.targets {
            if self.before_image(target.kind, &target.relative_key)? != target.before {
                return Err(MutationError::RecoveryConflict(
                    "retained mutation target changed before authority ownership".into(),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn intent_effects_are_durable(
        &self,
        intent: &crate::services::twin_events::MutationIntentV1,
    ) -> Result<bool, MutationError> {
        for target in intent
            .targets
            .iter()
            .filter(|target| !target_is_exact_precondition(intent, target))
        {
            let current = self.before_image(target.kind, &target.relative_key)?;
            if !target_matches_after(&current, &target.after, &target.after_digest) {
                return Ok(false);
            }
        }
        if intent.events.is_empty() {
            return Ok(true);
        }
        let durable = self.store.ordered_events()?;
        let durable = durable
            .into_iter()
            .map(|event| (event.event_id.clone(), event))
            .collect::<std::collections::BTreeMap<_, _>>();
        Ok(intent.events.iter().all(|expected| {
            durable.get(&expected.event_id).is_some_and(|event| {
                crate::services::twin_events::semantic_bytes(event)
                    == crate::services::twin_events::semantic_bytes(expected)
            })
        }))
    }

    pub(super) fn apply_intent_target(
        &self,
        target: &crate::services::twin_events::MutationTargetV1,
    ) -> Result<(), MutationError> {
        self.apply_target_mutation(&crate::services::twin_events::TargetMutation {
            kind: target.kind,
            relative_key: target.relative_key.clone(),
            after: target.after.clone(),
            expected_before: None,
            retain_exact_precondition: false,
            check_expected_before_before_after_elision: false,
        })
    }

    pub(super) fn classify_witnessed_mutation_locked(
        &self,
        process_lock: &CoordinatorProcessLock,
        mutation_id: &crate::models::twin_event::ContentDigest,
        expected_authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
        target_kind: crate::services::twin_events::TargetKind,
        target_key: &str,
        expected_before: &crate::services::twin_events::BeforeImage,
        expected_after: &crate::services::twin_events::BeforeImage,
    ) -> Result<WitnessedMutationRecovery, MutationError> {
        self.verify_root_lease_locked()?;
        let lease = self
            .root_lease
            .lock()
            .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
            .clone();
        let current_authority = crate::services::vault_namespace::capture_authority_token_locked(
            &self.data_path,
            &lease,
            process_lock,
        )?;
        if current_authority.root_scope != expected_authority.root_scope
            || current_authority.lease_epoch_uuid != expected_authority.lease_epoch_uuid
        {
            return Err(MutationError::RecoveryConflict(
                "witnessed mutation belongs to another vault authority".into(),
            ));
        }
        let current_target = self.before_image(target_kind, target_key)?;
        let after_matches = &current_target == expected_after;
        let committed_generation = expected_authority
            .authority_generation
            .checked_add(1)
            .ok_or_else(|| {
                MutationError::RecoveryConflict("authority-generation-exhausted".into())
            })?;
        let preauthority = self.journal.preauthority_for(process_lock, mutation_id)?;
        let receipt = self
            .journal
            .load_committed_receipt(process_lock, mutation_id)?;
        if receipt.is_some() && preauthority.is_some() {
            return Err(MutationError::RecoveryConflict(
                "witnessed mutation has conflicting owner records".into(),
            ));
        }
        if let Some(marker) = preauthority {
            if marker.state == crate::services::twin_events::PreAuthorityMutationStateV1::Prepared
                || marker.expected_authority != *expected_authority
            {
                return Err(MutationError::RecoveryConflict(
                    "witnessed mutation still has an active pre-authority owner".into(),
                ));
            }
            let matching_targets = marker
                .intent
                .targets
                .iter()
                .filter(|target| target.kind == target_kind && target.relative_key == target_key)
                .collect::<Vec<_>>();
            let Some(target) = matching_targets.first().copied() else {
                return Err(MutationError::RecoveryConflict(
                    "aborted mutation does not bind the witnessed target".into(),
                ));
            };
            let marker_after = match target.after {
                crate::services::twin_events::DesiredImage::Tombstone => {
                    crate::services::twin_events::BeforeImage::Absent
                }
                crate::services::twin_events::DesiredImage::Utf8Bytes(_) => {
                    crate::services::twin_events::BeforeImage::Sha256(target.after_digest.clone())
                }
            };
            if matching_targets.len() != 1
                || &target.before != expected_before
                || &marker_after != expected_after
            {
                return Err(MutationError::RecoveryConflict(
                    "aborted mutation proof does not match the exact witnessed state".into(),
                ));
            }
            return Ok(match marker.state {
                crate::services::twin_events::PreAuthorityMutationStateV1::AbortedBeforeAuthority => {
                    WitnessedMutationRecovery::Aborted
                }
                crate::services::twin_events::PreAuthorityMutationStateV1::AbortedAfterAuthority => {
                    WitnessedMutationRecovery::AbortedAfterAuthority(MutationCommit {
                        mutation_id: Some(mutation_id.clone()),
                        events: Vec::new(),
                        authority_token: Some(
                            crate::services::vault_namespace::VaultAuthorityTokenV1 {
                                root_scope: marker.expected_authority.root_scope.clone(),
                                lease_epoch_uuid: marker
                                    .expected_authority
                                    .lease_epoch_uuid
                                    .clone(),
                                authority_generation: committed_generation,
                            },
                        ),
                        postcommit_warning: true,
                    })
                }
                crate::services::twin_events::PreAuthorityMutationStateV1::Prepared => {
                    unreachable!("prepared owner rejected above")
                }
            });
        }
        if let Some(receipt) = receipt {
            if receipt.root_scope != expected_authority.root_scope
                || receipt.lease_epoch_uuid != expected_authority.lease_epoch_uuid
                || receipt.authority_generation != committed_generation
                || current_authority.authority_generation < receipt.authority_generation
                || (current_authority.authority_generation == receipt.authority_generation
                    && !after_matches)
            {
                return Err(MutationError::RecoveryConflict(
                    "retained mutation receipt does not match its exact authority effect".into(),
                ));
            }
            return Ok(WitnessedMutationRecovery::Committed(MutationCommit {
                mutation_id: Some(mutation_id.clone()),
                events: Vec::new(),
                authority_token: Some(crate::services::vault_namespace::VaultAuthorityTokenV1 {
                    root_scope: receipt.root_scope,
                    lease_epoch_uuid: receipt.lease_epoch_uuid,
                    authority_generation: receipt.authority_generation,
                }),
                postcommit_warning: true,
            }));
        }
        if &current_target == expected_before && current_authority == *expected_authority {
            return Ok(WitnessedMutationRecovery::NotCommitted);
        }
        Err(MutationError::RecoveryConflict(
            "prepared mutation lacks an exact committed receipt".into(),
        ))
    }

    pub(super) fn apply_target_mutation(
        &self,
        target: &crate::services::twin_events::TargetMutation,
    ) -> Result<(), MutationError> {
        crate::services::twin_events::validate_target_key(target.kind, &target.relative_key)?;
        match &target.after {
            crate::services::twin_events::DesiredImage::Utf8Bytes(content) => {
                self.put_target(target.kind, &target.relative_key, content.as_bytes())?;
            }
            crate::services::twin_events::DesiredImage::Tombstone => {
                self.delete_target(target.kind, &target.relative_key)?;
            }
        }
        Ok(())
    }

    pub(super) fn target_root(
        &self,
        kind: crate::services::twin_events::TargetKind,
    ) -> Result<PathBuf, MutationError> {
        Ok(match kind {
            crate::services::twin_events::TargetKind::Markdown => self
                .vault_root
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))?
                .canonical_path()
                .to_path_buf(),
            crate::services::twin_events::TargetKind::OverlayJson => {
                let lease = self
                    .root_lease
                    .lock()
                    .map_err(|_| {
                        MutationError::Invalid("Markdown root lease lock poisoned".into())
                    })?
                    .clone();
                crate::services::vault_namespace::scoped_data_path(
                    &self.data_path,
                    &lease.root_scope,
                )
                .join("vault_migration")
                .join("overlay")
                .join("notes")
            }
            crate::services::twin_events::TargetKind::TwinJson => self.data_path.join("twin"),
            crate::services::twin_events::TargetKind::CanvasJson => self.data_path.join("canvas"),
        })
    }

    pub(super) fn current_markdown_root_scope(
        &self,
    ) -> Result<crate::models::twin_event::ContentDigest, MutationError> {
        markdown_root_scope_for(
            &self.target_root(crate::services::twin_events::TargetKind::Markdown)?,
        )
    }

    pub(super) fn verify_root_lease_locked(&self) -> Result<(), MutationError> {
        let expected = self
            .root_lease
            .lock()
            .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
            .clone();
        let durable = load_active_root_lease(&self.data_root)?;
        if durable != expected || durable.root_scope != self.current_markdown_root_scope()? {
            return Err(MutationError::RecoveryConflict(
                "stale-markdown-root-lease".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn target_key(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
    ) -> Result<String, MutationError> {
        crate::services::twin_events::validate_target_key(kind, relative_key)?;
        Ok(match kind {
            crate::services::twin_events::TargetKind::Markdown => relative_key.to_string(),
            crate::services::twin_events::TargetKind::OverlayJson => {
                let root = self.target_root(kind)?;
                let relative_root = root.strip_prefix(&self.data_path).map_err(|_| {
                    MutationError::Invalid("overlay target root escaped app data".into())
                })?;
                format!(
                    "{}/{}",
                    relative_root.to_string_lossy().replace('\\', "/"),
                    relative_key
                )
            }
            crate::services::twin_events::TargetKind::TwinJson => {
                format!("twin/{relative_key}")
            }
            crate::services::twin_events::TargetKind::CanvasJson => {
                format!("canvas/{relative_key}")
            }
        })
    }

    pub(super) fn before_image(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
    ) -> Result<crate::services::twin_events::BeforeImage, MutationError> {
        let limit = match kind {
            crate::services::twin_events::TargetKind::Markdown
            | crate::services::twin_events::TargetKind::OverlayJson
            | crate::services::twin_events::TargetKind::TwinJson => {
                crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES
            }
            crate::services::twin_events::TargetKind::CanvasJson => {
                crate::services::twin_events::MAX_CANVAS_BYTES
            }
        };
        let bytes = self.read_target(kind, relative_key, limit)?;
        match bytes {
            Some(bytes) => Ok(crate::services::twin_events::BeforeImage::Sha256(
                crate::services::twin_events::digest_bytes(&bytes),
            )),
            None => Ok(crate::services::twin_events::BeforeImage::Absent),
        }
    }

    pub(super) fn read_target(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, MutationError> {
        match kind {
            crate::services::twin_events::TargetKind::Markdown => self
                .vault_root
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))?
                .read_bounded(relative_key, limit),
            crate::services::twin_events::TargetKind::OverlayJson
            | crate::services::twin_events::TargetKind::TwinJson
            | crate::services::twin_events::TargetKind::CanvasJson => self
                .data_root
                .read_bounded(&self.target_key(kind, relative_key)?, limit),
        }
    }

    pub(super) fn put_target(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
        bytes: &[u8],
    ) -> Result<(), MutationError> {
        match kind {
            crate::services::twin_events::TargetKind::Markdown => self
                .vault_root
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))?
                .put_atomic(relative_key, bytes),
            crate::services::twin_events::TargetKind::OverlayJson
            | crate::services::twin_events::TargetKind::TwinJson
            | crate::services::twin_events::TargetKind::CanvasJson => self
                .data_root
                .put_atomic(&self.target_key(kind, relative_key)?, bytes),
        }
    }

    pub(super) fn delete_target(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
    ) -> Result<(), MutationError> {
        match kind {
            crate::services::twin_events::TargetKind::Markdown => self
                .vault_root
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))?
                .delete(relative_key),
            crate::services::twin_events::TargetKind::OverlayJson
            | crate::services::twin_events::TargetKind::TwinJson
            | crate::services::twin_events::TargetKind::CanvasJson => {
                self.data_root.delete(&self.target_key(kind, relative_key)?)
            }
        }
    }

    pub(super) fn physical_target_id(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
    ) -> Result<Vec<u8>, MutationError> {
        crate::services::twin_events::validate_target_key(kind, relative_key)?;
        let mut target = platform_canonical_path_bytes(&self.target_root(kind)?)?;
        target.push(b'/');
        #[cfg(windows)]
        target.extend_from_slice(relative_key.to_lowercase().as_bytes());
        #[cfg(not(windows))]
        target.extend_from_slice(relative_key.as_bytes());
        Ok(target)
    }

    pub(super) fn inject(&self, point: MutationFaultPoint) -> Result<(), MutationError> {
        let mut configured = self
            .fault_once
            .lock()
            .map_err(|_| MutationError::Invalid("fault injector lock poisoned".into()))?;
        if configured.as_ref() == Some(&point) {
            *configured = None;
            return Err(MutationError::Io(format!(
                "injected mutation crash at {point:?}"
            )));
        }
        Ok(())
    }
}
