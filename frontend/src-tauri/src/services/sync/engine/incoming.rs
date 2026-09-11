use super::*;

pub(super) fn verify_envelope(
    state: &EngineState,
    root_key: &VaultRootKey,
    envelope: &EnvelopeV1,
) -> Result<VerifiedOperation, MutationError> {
    let public_key = state
        .trusted_devices
        .get(envelope.device_id())
        .copied()
        .ok_or_else(|| MutationError::Invalid("sync envelope uses an untrusted device".into()))?;
    let trusted = TrustedDevice::new(*envelope.device_id(), public_key)
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
    open_operation(root_key, &state.vault_id, &trusted, envelope)
        .map_err(|error| MutationError::Invalid(error.to_string()))
}

pub(super) fn validate_incoming_operation(
    state: &EngineState,
    verified: &VerifiedOperation,
) -> Result<(), MutationError> {
    if verified.operation().causal_parents().iter().any(|parent| {
        state
            .ledger
            .rejected_operations
            .contains(&parent.to_string())
    }) {
        return Err(MutationError::Invalid(
            "sync operation depends on a rejected operation".into(),
        ));
    }
    let protected =
        local_only_references_with_paths(&state.ledger.local_only_notes, &state.ledger.note_paths);
    match verified.operation().payload() {
        OperationPayloadV1::NoteRevision(revision) => {
            if is_reserved_program(revision.note_id()) || protected.contains(revision.note_id()) {
                return Err(MutationError::Invalid(
                    "synced note targets locally protected content".into(),
                ));
            }
            if let NoteRevisionKind::Put { markdown } = revision.kind() {
                if !crate::services::knowledge_store::note_allows_sync(markdown) {
                    return Err(MutationError::Invalid(
                        "synced note is marked local-only or has invalid governance".into(),
                    ));
                }
                if crate::services::knowledge_store::note_identity_from_markdown(markdown)
                    .is_some_and(|embedded| embedded != revision.note_id())
                {
                    return Err(MutationError::Invalid(
                        "synced note identity does not match its signed revision".into(),
                    ));
                }
            }
        }
        OperationPayloadV1::TwinEvent(payload) => {
            let event = parse_synced_event(payload, verified.device_id())?;
            if event_references_local_only_note(&event, &protected) {
                return Err(MutationError::Invalid(
                    "synced Twin event references a locally protected note".into(),
                ));
            }
        }
        OperationPayloadV1::AttachmentManifest(_) | OperationPayloadV1::AttachmentChunk(_) => {}
    }
    Ok(())
}

pub(super) fn validate_incoming_batch(
    state: &EngineState,
    candidates: &[(EnvelopeV1, VerifiedOperation)],
) -> Result<(), MutationError> {
    let mut known = state.operations.clone();
    let mut canonical = BTreeMap::<OperationId, Vec<u8>>::new();
    for (envelope, verified) in candidates {
        let bytes = envelope
            .to_json()
            .map_err(|error| MutationError::Invalid(error.to_string()))?
            .into_bytes();
        match canonical.get(envelope.operation_id()) {
            Some(existing) if existing != &bytes => {
                return Err(MutationError::Invalid(
                    "sync receive batch contains an operation identity collision".into(),
                ));
            }
            Some(_) => {}
            None => {
                canonical.insert(*envelope.operation_id(), bytes);
                known.insert(*verified.operation_id(), verified.clone());
            }
        }
    }

    let mut event_operations = state.event_operations.clone();
    for (operation_id, verified) in &known {
        if let OperationPayloadV1::TwinEvent(payload) = verified.operation().payload() {
            let event = parse_synced_event(payload, verified.device_id())?;
            match event_operations.insert(event.event_id.as_str().to_owned(), *operation_id) {
                Some(existing) if existing != *operation_id => {
                    return Err(MutationError::Invalid(
                        "one Twin event identity maps to multiple sync operations".into(),
                    ));
                }
                _ => {}
            }
        }
    }

    for (_, verified) in candidates {
        match verified.operation().payload() {
            OperationPayloadV1::NoteRevision(revision) => {
                for parent in verified.operation().causal_parents() {
                    if let Some(parent) = known.get(parent) {
                        match parent.operation().payload() {
                            OperationPayloadV1::NoteRevision(parent_revision)
                                if parent_revision.note_id() == revision.note_id() => {}
                            _ => {
                                return Err(MutationError::Invalid(
                                    "sync note revision has a parent from another object".into(),
                                ));
                            }
                        }
                    }
                }
            }
            OperationPayloadV1::TwinEvent(payload) => {
                let event = parse_synced_event(payload, verified.device_id())?;
                let dependencies = event_dependency_ids(&event)?;
                if dependencies.len() != verified.operation().causal_parents().len() {
                    return Err(MutationError::Invalid(
                        "synced Twin event dependencies do not match its operation".into(),
                    ));
                }
                for dependency in &dependencies {
                    if let Some(operation_id) = event_operations.get(dependency.as_str()) {
                        if !verified.operation().causal_parents().contains(operation_id) {
                            return Err(MutationError::Invalid(
                                "synced Twin event dependencies do not match its operation".into(),
                            ));
                        }
                    }
                }
                for parent in verified.operation().causal_parents() {
                    if let Some(parent) = known.get(parent) {
                        let OperationPayloadV1::TwinEvent(parent_payload) =
                            parent.operation().payload()
                        else {
                            return Err(MutationError::Invalid(
                                "synced Twin event has a non-event parent".into(),
                            ));
                        };
                        if !dependencies.iter().any(|dependency| {
                            dependency.as_str() == parent_payload.event_id().to_string()
                        }) {
                            return Err(MutationError::Invalid(
                                "synced Twin event dependencies do not match its operation".into(),
                            ));
                        }
                    }
                }
            }
            OperationPayloadV1::AttachmentManifest(_) => {
                if !verified.operation().causal_parents().is_empty() {
                    return Err(MutationError::Invalid(
                        "attachment manifest cannot have causal parents".into(),
                    ));
                }
            }
            OperationPayloadV1::AttachmentChunk(chunk) => {
                validate_chunk_operation_dependencies(&known, verified, chunk)?;
            }
        }
    }
    Ok(())
}

pub(super) fn durable_envelope_bytes(
    state: &EngineState,
    operation_id: &OperationId,
) -> Result<Option<Vec<u8>>, MutationError> {
    let mut durable = None::<Vec<u8>>;
    for area in [
        OperationArea::Staged,
        OperationArea::Inbox,
        OperationArea::Outbox,
    ] {
        let Some(record) = state
            .operation_store
            .load(area, operation_id)
            .map_err(operation_store_error)?
        else {
            continue;
        };
        match durable.as_ref() {
            Some(existing) if existing.as_slice() != record.bytes() => {
                return Err(MutationError::RecoveryConflict(format!(
                    "sync operation has inconsistent durable copies: {operation_id}"
                )));
            }
            Some(_) => {}
            None => durable = Some(record.bytes().to_vec()),
        }
    }
    Ok(durable)
}

pub(super) fn validate_pending_device_bounds(
    state: &EngineState,
    candidates: &[(EnvelopeV1, VerifiedOperation)],
) -> Result<(), MutationError> {
    let mut usage = BTreeMap::<DeviceId, (usize, usize)>::new();
    for record in state
        .operation_store
        .list(OperationArea::Inbox)
        .map_err(operation_store_error)?
    {
        if state.applied.contains(record.envelope().operation_id()) {
            continue;
        }
        let entry = usage.entry(*record.envelope().device_id()).or_default();
        entry.0 = entry
            .0
            .checked_add(1)
            .ok_or_else(|| MutationError::Invalid("sync pending device count overflowed".into()))?;
        entry.1 = entry.1.checked_add(record.bytes().len()).ok_or_else(|| {
            MutationError::Invalid("sync pending device byte count overflowed".into())
        })?;
    }
    let mut seen = state.operations.keys().copied().collect::<BTreeSet<_>>();
    let mut touched = BTreeSet::new();
    for (envelope, _) in candidates {
        if !seen.insert(*envelope.operation_id()) {
            continue;
        }
        let device_id = *envelope.device_id();
        touched.insert(device_id);
        let entry = usage.entry(device_id).or_default();
        entry.0 = entry
            .0
            .checked_add(1)
            .ok_or_else(|| MutationError::Invalid("sync pending device count overflowed".into()))?;
        let bytes = envelope
            .to_json()
            .map_err(|error| MutationError::Invalid(error.to_string()))?
            .len();
        entry.1 = entry.1.checked_add(bytes).ok_or_else(|| {
            MutationError::Invalid("sync pending device byte count overflowed".into())
        })?;
    }
    if touched.iter().any(|device_id| {
        usage.get(device_id).is_some_and(|(count, bytes)| {
            *count > MAX_PENDING_OPERATIONS_PER_DEVICE || *bytes > MAX_PENDING_BYTES_PER_DEVICE
        })
    }) {
        return Err(MutationError::Invalid(
            "sync pending operations exceed the per-device bound".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_materialization_dependencies(
    state: &EngineState,
    verified: &VerifiedOperation,
) -> Result<(), MutationError> {
    match verified.operation().payload() {
        OperationPayloadV1::NoteRevision(revision) => {
            for parent in verified.operation().causal_parents() {
                let parent = state.operations.get(parent).ok_or_else(|| {
                    MutationError::RecoveryConflict(
                        "ready note revision parent is unavailable".into(),
                    )
                })?;
                match parent.operation().payload() {
                    OperationPayloadV1::NoteRevision(parent_revision)
                        if parent_revision.note_id() == revision.note_id() => {}
                    _ => {
                        return Err(MutationError::Invalid(
                            "sync note revision has a parent from another object".into(),
                        ));
                    }
                }
            }
        }
        OperationPayloadV1::TwinEvent(_) => {}
        OperationPayloadV1::AttachmentManifest(_) => {
            if !verified.operation().causal_parents().is_empty() {
                return Err(MutationError::Invalid(
                    "attachment manifest cannot have causal parents".into(),
                ));
            }
        }
        OperationPayloadV1::AttachmentChunk(chunk) => {
            validate_chunk_operation_dependencies(&state.operations, verified, chunk)?;
        }
    }
    Ok(())
}

pub(super) fn validate_chunk_operation_dependencies(
    known: &BTreeMap<OperationId, VerifiedOperation>,
    verified: &VerifiedOperation,
    chunk: &AttachmentChunkV1,
) -> Result<(), MutationError> {
    if verified.operation().causal_parents() != [*chunk.manifest_operation_id()] {
        return Err(MutationError::Invalid(
            "attachment chunk must depend exactly on its manifest operation".into(),
        ));
    }
    let Some(parent) = known.get(chunk.manifest_operation_id()) else {
        return Ok(());
    };
    let OperationPayloadV1::AttachmentManifest(manifest) = parent.operation().payload() else {
        return Err(MutationError::Invalid(
            "attachment chunk parent is not a manifest".into(),
        ));
    };
    if chunk.attachment_digest() != manifest.attachment_digest()
        || chunk.chunk_count() != manifest.chunk_count()
    {
        return Err(MutationError::Invalid(
            "attachment chunk metadata does not match its manifest".into(),
        ));
    }
    Ok(())
}

pub(super) fn related_attachment_operations(
    state: &EngineState,
    operation_id: OperationId,
) -> BTreeSet<OperationId> {
    let Some(verified) = state.operations.get(&operation_id) else {
        return [operation_id].into_iter().collect();
    };
    let manifest_operation_id = match verified.operation().payload() {
        OperationPayloadV1::AttachmentManifest(_) => operation_id,
        OperationPayloadV1::AttachmentChunk(chunk) => *chunk.manifest_operation_id(),
        OperationPayloadV1::NoteRevision(_) | OperationPayloadV1::TwinEvent(_) => {
            return [operation_id].into_iter().collect();
        }
    };
    state
        .operations
        .iter()
        .filter_map(
            |(candidate_id, candidate)| match candidate.operation().payload() {
                OperationPayloadV1::AttachmentManifest(_)
                    if *candidate_id == manifest_operation_id =>
                {
                    Some(*candidate_id)
                }
                OperationPayloadV1::AttachmentChunk(chunk)
                    if chunk.manifest_operation_id() == &manifest_operation_id =>
                {
                    Some(*candidate_id)
                }
                _ => None,
            },
        )
        .chain(std::iter::once(operation_id))
        .collect()
}

pub(super) fn recompute_logical_state(state: &mut EngineState) -> Result<(), MutationError> {
    state.note_heads.clear();
    state.note_revisions.clear();
    state.event_operations.clear();
    let applied = state
        .applied
        .iter()
        .filter(|operation_id| {
            !state
                .ledger
                .rejected_operations
                .contains(&operation_id.to_string())
        })
        .copied()
        .collect::<BTreeSet<_>>();
    for operation_id in &applied {
        let verified = state.operations.get(operation_id).ok_or_else(|| {
            MutationError::RecoveryConflict(format!(
                "applied sync operation is unavailable: {operation_id}"
            ))
        })?;
        match verified.operation().payload() {
            OperationPayloadV1::NoteRevision(revision) => {
                state.note_revisions.insert(*operation_id, revision.clone());
                state
                    .note_heads
                    .entry(revision.note_id().to_owned())
                    .or_default()
                    .insert(*operation_id);
            }
            OperationPayloadV1::TwinEvent(event) => {
                state
                    .event_operations
                    .insert(event.event_id().to_string(), *operation_id);
            }
            OperationPayloadV1::AttachmentManifest(_) | OperationPayloadV1::AttachmentChunk(_) => {}
        }
    }
    for operation_id in &applied {
        let verified = state.operations.get(operation_id).ok_or_else(|| {
            MutationError::RecoveryConflict("applied sync operation disappeared".into())
        })?;
        let OperationPayloadV1::NoteRevision(revision) = verified.operation().payload() else {
            continue;
        };
        let heads = state
            .note_heads
            .get_mut(revision.note_id())
            .expect("note head set was just initialized");
        for parent in verified.operation().causal_parents() {
            heads.remove(parent);
        }
    }
    Ok(())
}

pub(super) fn apply_note_operation(
    state: &mut EngineState,
    verified: &VerifiedOperation,
) -> Result<(), MutationError> {
    let OperationPayloadV1::NoteRevision(revision) = verified.operation().payload() else {
        return Err(MutationError::Invalid(
            "non-note operation entered note materialization".into(),
        ));
    };
    let operation_id = *verified.operation_id();
    state.note_revisions.insert(operation_id, revision.clone());
    let heads = state
        .note_heads
        .entry(revision.note_id().to_owned())
        .or_default();
    for parent in verified.operation().causal_parents() {
        heads.remove(parent);
    }
    heads.insert(operation_id);
    Ok(())
}

pub(super) fn parse_synced_event(
    payload: &TwinEventV1,
    envelope_device_id: &DeviceId,
) -> Result<TwinEvent, MutationError> {
    let event: TwinEvent = serde_json::from_str(payload.event_json())
        .map_err(|error| MutationError::Invalid(format!("invalid synced Twin event: {error}")))?;
    event.validate().map_err(MutationError::Invalid)?;
    let event_id = Digest32::parse_hex(event.event_id.as_str())
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
    if &event_id != payload.event_id() {
        return Err(MutationError::Invalid(
            "synced Twin event payload ID does not match its operation".into(),
        ));
    }
    if event.device_id.as_str() != envelope_device_id.to_string() {
        return Err(MutationError::Invalid(
            "synced Twin event device does not match its signed envelope".into(),
        ));
    }
    if event.causal_stream != CausalStream::SyncEligible
        || !governance_allows_sync(&event.governance)
        || event
            .context
            .relationships
            .iter()
            .any(|relationship| !governance_allows_sync(&relationship.governance))
    {
        return Err(MutationError::Invalid(
            "synced Twin event violates its governance contract".into(),
        ));
    }
    Ok(event)
}

pub(super) fn governance_allows_sync(governance: &crate::models::twin_event::Governance) -> bool {
    governance.visibility == Visibility::SyncedVault
        && governance.sensitivity != Sensitivity::Restricted
        && governance.allowed_uses.sync
}

pub(super) fn event_references_local_only_note(
    event: &TwinEvent,
    local_only_notes: &BTreeSet<String>,
) -> bool {
    let evidence_references_note = |evidence: &crate::models::twin_event::EvidenceRef| {
        evidence.evidence_type == crate::models::twin_event::EvidenceType::Note
            && local_only_notes.contains(evidence.source_id.as_str())
    };
    event.evidence.iter().any(evidence_references_note)
        || event
            .context
            .relationships
            .iter()
            .any(|relationship| relationship.evidence.iter().any(evidence_references_note))
        || matches!(
            &event.payload,
            crate::models::twin_event::TwinEventPayload::NoteChanged(note)
                if local_only_notes.contains(note.note_id.as_str())
        )
}

pub(super) fn event_dependency_ids(event: &TwinEvent) -> Result<Vec<EventId>, MutationError> {
    let mut dependencies = event
        .causal_parents
        .iter()
        .chain(&event.supersedes)
        .chain(&event.reinforces)
        .cloned()
        .collect::<Vec<_>>();
    for evidence in event.evidence.iter().chain(
        event
            .context
            .relationships
            .iter()
            .flat_map(|relationship| &relationship.evidence),
    ) {
        if evidence.evidence_type == EvidenceType::Event {
            dependencies.push(
                EventId::parse(evidence.source_id.as_str().to_owned())
                    .map_err(MutationError::Invalid)?,
            );
        }
    }
    dependencies.sort();
    dependencies.dedup();
    if dependencies
        .iter()
        .any(|dependency| dependency == &event.event_id)
    {
        return Err(MutationError::Invalid(
            "Twin event cannot depend on itself".into(),
        ));
    }
    Ok(dependencies)
}
