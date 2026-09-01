use super::*;

pub(super) fn rebuild_engine_state(
    data_root: &AnchoredRoot,
    state: &mut EngineState,
) -> Result<(), MutationError> {
    let durable_applied = state
        .operation_store
        .list_applied_ids()
        .map_err(operation_store_error)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let outbox = state
        .operation_store
        .list(OperationArea::Outbox)
        .map_err(operation_store_error)?;
    let inbox = state
        .operation_store
        .list(OperationArea::Inbox)
        .map_err(operation_store_error)?;
    reset_logical_state(state);
    state.applied = durable_applied;
    let Some(root_key) = state.root_key.as_ref() else {
        return Ok(());
    };
    let root_key_bytes = root_key.export_bytes();
    let root_key = VaultRootKey::from_bytes(*root_key_bytes);

    for record in outbox.iter().chain(inbox.iter()) {
        let verified = verify_envelope(state, &root_key, record.envelope())?;
        let operation_id = *verified.operation_id();
        if state.operations.insert(operation_id, verified).is_some() {
            return Err(MutationError::RecoveryConflict(format!(
                "sync operation appears in more than one durable area: {operation_id}"
            )));
        }
    }
    let mut rejection_changed = false;
    loop {
        let mut newly_rejected = BTreeSet::new();
        for record in &inbox {
            let operation_id = *record.envelope().operation_id();
            if state.applied.contains(&operation_id)
                || state
                    .ledger
                    .rejected_operations
                    .contains(&operation_id.to_string())
            {
                continue;
            }
            let verified = state.operations.get(&operation_id).ok_or_else(|| {
                MutationError::RecoveryConflict("stored sync operation disappeared".into())
            })?;
            match validate_incoming_operation(state, verified) {
                Ok(()) => {}
                Err(error) if terminal_remote_error(&error) => {
                    newly_rejected.extend(related_attachment_operations(state, operation_id));
                }
                Err(error) => return Err(error),
            }
        }
        newly_rejected.retain(|operation_id| {
            !state
                .ledger
                .rejected_operations
                .contains(&operation_id.to_string())
        });
        if newly_rejected.is_empty() {
            break;
        }
        if state
            .ledger
            .rejected_operations
            .len()
            .checked_add(newly_rejected.len())
            .is_none_or(|count| count > MAX_REJECTED_OPERATIONS)
        {
            return Err(MutationError::Invalid(
                "sync rejected-operation quarantine limit exceeded".into(),
            ));
        }
        state.ledger.rejected_operations.extend(
            newly_rejected
                .into_iter()
                .map(|operation_id| operation_id.to_string()),
        );
        rejection_changed = true;
    }
    if rejection_changed {
        persist_ledger(
            data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )?;
    }
    let rejected = state
        .operations
        .keys()
        .filter(|operation_id| {
            state
                .ledger
                .rejected_operations
                .contains(&operation_id.to_string())
        })
        .copied()
        .collect::<Vec<_>>();
    for operation_id in rejected {
        state
            .operation_store
            .mark_applied(&operation_id)
            .map_err(operation_store_error)?;
        state.applied.insert(operation_id);
    }
    for record in &outbox {
        state.applied.insert(*record.envelope().operation_id());
    }
    recompute_logical_state(state)?;
    for record in &inbox {
        let operation_id = *record.envelope().operation_id();
        if !state.applied.contains(&operation_id) {
            let verified = state
                .operations
                .get(&operation_id)
                .cloned()
                .ok_or_else(|| {
                    MutationError::RecoveryConflict("stored sync operation disappeared".into())
                })?;
            validate_incoming_operation(state, &verified)?;
            state
                .pending_graph
                .insert_verified(&verified, record.bytes().len())
                .map_err(graph_error)?;
        }
    }
    Ok(())
}

pub(super) fn refresh_engine_state(
    data_root: &AnchoredRoot,
    state: &mut EngineState,
) -> Result<(), MutationError> {
    state.ledger = load_ledger(data_root, &engine_ledger_key(&state.vault_scope))?;
    state.trusted_devices = trusted_devices_from_ledger(&state.ledger)?;
    rebuild_engine_state(data_root, state)
}

pub(super) fn recover_standalone_batches_in_state(
    data_root: &AnchoredRoot,
    state: &mut EngineState,
) -> Result<(), MutationError> {
    let batches = state
        .ledger
        .standalone_batches
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut promoted = Vec::new();
    for mutation_id in batches {
        match state.operation_store.promote_batch(&mutation_id) {
            Ok(_) => {
                promoted.push(mutation_id);
                continue;
            }
            Err(OperationStoreError::MissingBatch(_))
            | Err(OperationStoreError::BatchCancelled(_)) => {}
            Err(error) => return Err(operation_store_error(error)),
        }
        state.ledger.standalone_batches.remove(&mutation_id);
        persist_ledger(
            data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )?;
    }
    if promoted.is_empty() || state.root_key.is_none() {
        return Ok(());
    }
    let outbox = state
        .operation_store
        .list(OperationArea::Outbox)
        .map_err(operation_store_error)?
        .into_iter()
        .map(|record| record.envelope().clone())
        .collect::<Vec<_>>();
    for mutation_id in &promoted {
        materialize_standalone_attachment_batch(state, mutation_id, &outbox)?;
    }
    for envelope in &outbox {
        state
            .operation_store
            .mark_applied(envelope.operation_id())
            .map_err(operation_store_error)?;
    }
    for mutation_id in promoted {
        state.ledger.standalone_batches.remove(&mutation_id);
        persist_ledger(
            data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )?;
    }
    Ok(())
}

pub(super) fn standalone_attachment_mutation_id(
    manifest_operation_id: &OperationId,
) -> ContentDigest {
    crate::services::twin_events::digest_bytes(
        format!("grafyn.sync.attachment.v1:{manifest_operation_id}").as_bytes(),
    )
}

pub(super) fn materialize_standalone_attachment_batch(
    state: &EngineState,
    mutation_id: &ContentDigest,
    envelopes: &[EnvelopeV1],
) -> Result<(), MutationError> {
    let root_key = state.root_key.as_ref().ok_or_else(|| {
        MutationError::RecoveryConflict("promoted attachment batch lost its vault key".into())
    })?;
    let verified = envelopes
        .iter()
        .map(|envelope| verify_envelope(state, root_key, envelope))
        .collect::<Result<Vec<_>, _>>()?;
    let manifests = verified
        .iter()
        .filter_map(|operation| match operation.operation().payload() {
            OperationPayloadV1::AttachmentManifest(manifest)
                if standalone_attachment_mutation_id(operation.operation_id()) == *mutation_id =>
            {
                Some((*operation.operation_id(), manifest.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [(manifest_operation_id, manifest)] = manifests.as_slice() else {
        return Err(MutationError::RecoveryConflict(
            "promoted attachment batch does not contain exactly one matching manifest".into(),
        ));
    };
    let mut chunks = verified
        .iter()
        .filter_map(|operation| match operation.operation().payload() {
            OperationPayloadV1::AttachmentChunk(chunk)
                if chunk.manifest_operation_id() == manifest_operation_id =>
            {
                Some((operation, chunk.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    chunks.sort_by_key(|(_, chunk)| chunk.chunk_index());
    if chunks.len() != manifest.chunk_count() as usize {
        return Err(MutationError::RecoveryConflict(
            "promoted attachment batch has an incomplete chunk set".into(),
        ));
    }
    for (expected_index, (operation, chunk)) in chunks.iter().enumerate() {
        if chunk.chunk_index() as usize != expected_index
            || operation.operation().causal_parents() != [*manifest_operation_id]
            || chunk.attachment_digest() != manifest.attachment_digest()
            || chunk.chunk_count() != manifest.chunk_count()
        {
            return Err(MutationError::RecoveryConflict(
                "promoted attachment batch has inconsistent chunk metadata".into(),
            ));
        }
    }
    state
        .attachment_store
        .ingest_manifest(*manifest_operation_id, manifest)?;
    for (_, chunk) in chunks {
        state.attachment_store.ingest_chunk(&chunk)?;
    }
    if state
        .attachment_store
        .materialized_bytes(manifest.attachment_digest())?
        .is_none()
    {
        return Err(MutationError::RecoveryConflict(
            "promoted attachment batch did not materialize its complete blob".into(),
        ));
    }
    Ok(())
}

pub(super) fn open_attachment_store(
    data_path: &Path,
    vault_scope: &ContentDigest,
) -> Result<AttachmentStore, MutationError> {
    let path = data_path
        .join("sync")
        .join("vaults")
        .join("v1")
        .join(vault_scope.as_str());
    std::fs::create_dir_all(&path)?;
    crate::services::twin_events::validate_real_directory(&path, "vault-scoped attachment root")?;
    AttachmentStore::open(path)
}

pub(super) fn validate_intent_scope(
    state: &EngineState,
    intent: &MutationIntentV1,
) -> Result<(), MutationError> {
    if intent
        .markdown_root_scope
        .as_ref()
        .is_some_and(|scope| scope != &state.vault_scope)
    {
        return Err(MutationError::RecoveryConflict(
            "sync engine is bound to a different vault scope".into(),
        ));
    }
    Ok(())
}
