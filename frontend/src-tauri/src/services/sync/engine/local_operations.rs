use super::*;

pub(super) fn build_local_envelopes(
    state: &EngineState,
    root_key: &VaultRootKey,
    intent: &MutationIntentV1,
) -> Result<Vec<EnvelopeV1>, MutationError> {
    let device = state
        .device
        .as_ref()
        .ok_or_else(|| MutationError::Invalid("sync device identity is not attached".into()))?;
    let recorded_at_unix_ms = u64::try_from(intent.created_at.timestamp_millis())
        .map_err(|_| MutationError::Invalid("mutation timestamp predates Unix time".into()))?;
    let prospective_policy = policy_after_intent(&state.ledger, intent)?;
    let local_only_references =
        local_only_references_with_paths(&prospective_policy, &state.ledger.note_paths);
    let mut envelopes = Vec::new();
    let mut event_operations = state.event_operations.clone();
    let attachment_digests = generated_image_attachment_digests(&intent.events, true)?;

    for change in collect_local_note_changes(state, intent, &prospective_policy)? {
        let revision = match change.kind {
            NoteRevisionKind::Put { markdown } => {
                NoteRevisionV1::put(change.note_id.clone(), markdown)
            }
            NoteRevisionKind::Tombstone => NoteRevisionV1::tombstone(change.note_id.clone()),
        }
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let parents = state
            .note_heads
            .get(&change.note_id)
            .map(|heads| heads.iter().copied().collect())
            .unwrap_or_default();
        let operation = OperationV1::new(
            recorded_at_unix_ms,
            parents,
            OperationPayloadV1::NoteRevision(revision),
        )
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
        envelopes.push(
            seal_operation(
                root_key,
                &state.vault_id,
                &device.device_id,
                &device.signing_key,
                &operation,
            )
            .map_err(|error| MutationError::Invalid(error.to_string()))?,
        );
    }

    for event in &intent.events {
        if event.causal_stream != CausalStream::SyncEligible {
            continue;
        }
        if event_references_local_only_note(event, &local_only_references) {
            continue;
        }
        let event_json = serde_json::to_string(event)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let event_id = Digest32::parse_hex(event.event_id.as_str())
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let payload = TwinEventV1::new(event_id, event_json)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let mut parents = event_dependency_ids(event)?
            .into_iter()
            .map(|dependency| {
                event_operations
                    .get(dependency.as_str())
                    .copied()
                    .ok_or_else(|| {
                        MutationError::RecoveryConflict(format!(
                            "sync event dependency has not been bootstrapped: {}",
                            dependency.as_str()
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        parents.sort();
        parents.dedup();
        let operation = OperationV1::new(
            recorded_at_unix_ms,
            parents,
            OperationPayloadV1::TwinEvent(payload),
        )
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let envelope = seal_operation(
            root_key,
            &state.vault_id,
            &device.device_id,
            &device.signing_key,
            &operation,
        )
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
        event_operations.insert(event.event_id.as_str().to_owned(), *envelope.operation_id());
        envelopes.push(envelope);
    }

    for attachment_digest in attachment_digests {
        let (catalog, bytes) =
            if let Some(catalog) = state.attachment_store.cataloged_image(&attachment_digest)? {
                let bytes = state
                    .attachment_store
                    .materialized_bytes(&attachment_digest)?
                    .ok_or_else(|| {
                        MutationError::RecoveryConflict(format!(
                            "sync attachment {} lost its cataloged blob",
                            attachment_digest
                        ))
                    })?;
                (catalog, bytes)
            } else {
                state
                    .attachment_store
                    .staged_generated_image(
                        &state.vault_scope,
                        &intent.mutation_id,
                        &attachment_digest,
                    )?
                    .ok_or_else(|| {
                        MutationError::Invalid(format!(
                            "sync attachment {} has neither a catalog nor its owned mutation stage",
                            attachment_digest
                        ))
                    })?
            };
        let manifest =
            AttachmentManifestV1::new(attachment_digest, catalog.media_type(), bytes.len())
                .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let manifest_operation = OperationV1::new(
            recorded_at_unix_ms,
            Vec::new(),
            OperationPayloadV1::AttachmentManifest(manifest.clone()),
        )
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let manifest_envelope = seal_operation(
            root_key,
            &state.vault_id,
            &device.device_id,
            &device.signing_key,
            &manifest_operation,
        )
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let manifest_operation_id = *manifest_envelope.operation_id();
        envelopes.push(manifest_envelope);
        for (chunk_index, data) in bytes.chunks(ATTACHMENT_CHUNK_BYTES).enumerate() {
            let chunk = AttachmentChunkV1::new(
                manifest_operation_id,
                attachment_digest,
                chunk_index as u32,
                manifest.chunk_count(),
                data.to_vec(),
            )
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
            let operation = OperationV1::new(
                recorded_at_unix_ms,
                vec![manifest_operation_id],
                OperationPayloadV1::AttachmentChunk(chunk),
            )
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
            envelopes.push(
                seal_operation(
                    root_key,
                    &state.vault_id,
                    &device.device_id,
                    &device.signing_key,
                    &operation,
                )
                .map_err(|error| MutationError::Invalid(error.to_string()))?,
            );
        }
    }
    Ok(envelopes)
}

pub(super) fn policy_after_intent(
    ledger: &EngineLedgerV1,
    intent: &MutationIntentV1,
) -> Result<BTreeMap<String, String>, MutationError> {
    let mut policy = ledger.local_only_notes.clone();
    for target in &intent.targets {
        if target.kind != TargetKind::Markdown || is_reserved_program(&target.relative_key) {
            continue;
        }
        if let DesiredImage::Utf8Bytes(markdown) = &target.after {
            if crate::services::knowledge_store::note_allows_sync(markdown) {
                policy.remove(&target.relative_key);
            } else {
                if policy.len() >= MAX_LOCAL_ONLY_NOTES
                    && !policy.contains_key(&target.relative_key)
                {
                    return Err(MutationError::Invalid(
                        "local-only note policy limit exceeded".into(),
                    ));
                }
                let note_id =
                    crate::services::knowledge_store::note_identity_from_markdown(markdown)
                        .unwrap_or_else(|| target.relative_key.clone());
                policy.insert(target.relative_key.clone(), note_id);
            }
        }
    }
    Ok(policy)
}

#[derive(Debug, Clone)]
struct LocalNoteChange {
    note_id: String,
    relative_key: String,
    kind: NoteRevisionKind,
}

fn collect_local_note_changes(
    state: &EngineState,
    intent: &MutationIntentV1,
    prospective_policy: &BTreeMap<String, String>,
) -> Result<Vec<LocalNoteChange>, MutationError> {
    let tombstone_paths = intent
        .targets
        .iter()
        .filter(|target| {
            target.kind == TargetKind::Markdown && matches!(target.after, DesiredImage::Tombstone)
        })
        .map(|target| target.relative_key.clone())
        .collect::<BTreeSet<_>>();
    let mut changes = BTreeMap::<String, LocalNoteChange>::new();
    for target in &intent.targets {
        if target.kind != TargetKind::Markdown || is_reserved_program(&target.relative_key) {
            continue;
        }
        match &target.after {
            DesiredImage::Utf8Bytes(markdown)
                if crate::services::knowledge_store::note_allows_sync(markdown) =>
            {
                let note_id = local_note_id(
                    &state.vault_scope,
                    &state.ledger,
                    &target.relative_key,
                    Some(markdown),
                )?;
                if let Some(old_path) = state.ledger.note_paths.get(&note_id) {
                    if old_path != &target.relative_key && !tombstone_paths.contains(old_path) {
                        return Err(MutationError::RecoveryConflict(
                            "sync note rename must tombstone its previous local path".into(),
                        ));
                    }
                }
                if let Some(old_id) = note_id_for_path(&state.ledger, &target.relative_key) {
                    if old_id != note_id {
                        changes.entry(old_id.clone()).or_insert(LocalNoteChange {
                            note_id: old_id,
                            relative_key: target.relative_key.clone(),
                            kind: NoteRevisionKind::Tombstone,
                        });
                    }
                }
                match changes.get(&note_id) {
                    Some(existing)
                        if matches!(existing.kind, NoteRevisionKind::Put { .. })
                            && existing.relative_key != target.relative_key =>
                    {
                        return Err(MutationError::Invalid(
                            "one sync note identity cannot target two local paths".into(),
                        ));
                    }
                    _ => {
                        changes.insert(
                            note_id.clone(),
                            LocalNoteChange {
                                note_id,
                                relative_key: target.relative_key.clone(),
                                kind: NoteRevisionKind::Put {
                                    markdown: markdown.clone(),
                                },
                            },
                        );
                    }
                }
            }
            DesiredImage::Utf8Bytes(_) => {}
            DesiredImage::Tombstone if !prospective_policy.contains_key(&target.relative_key) => {
                let note_id = local_note_id(
                    &state.vault_scope,
                    &state.ledger,
                    &target.relative_key,
                    None,
                )?;
                changes.entry(note_id.clone()).or_insert(LocalNoteChange {
                    note_id,
                    relative_key: target.relative_key.clone(),
                    kind: NoteRevisionKind::Tombstone,
                });
            }
            DesiredImage::Tombstone => {}
        }
    }
    Ok(changes.into_values().collect())
}

pub(super) fn ledger_after_intent(
    state: &EngineState,
    intent: &MutationIntentV1,
) -> Result<EngineLedgerV1, MutationError> {
    let prospective_policy = policy_after_intent(&state.ledger, intent)?;
    let changes = collect_local_note_changes(state, intent, &prospective_policy)?;
    let mut ledger = state.ledger.clone();
    ledger.local_only_notes = prospective_policy;
    for change in changes
        .iter()
        .filter(|change| matches!(change.kind, NoteRevisionKind::Tombstone))
    {
        ledger.note_paths.remove(&change.note_id);
    }
    for change in changes {
        if matches!(change.kind, NoteRevisionKind::Put { .. }) {
            ledger
                .note_paths
                .retain(|note_id, path| note_id == &change.note_id || path != &change.relative_key);
            ledger
                .note_paths
                .insert(change.note_id, change.relative_key);
        }
    }
    if ledger.note_paths.len() > MAX_NOTE_PROJECTIONS {
        return Err(MutationError::Invalid(
            "sync note projection limit exceeded".into(),
        ));
    }
    Ok(ledger)
}

pub(super) fn local_note_id(
    scope: &ContentDigest,
    ledger: &EngineLedgerV1,
    relative_key: &str,
    markdown: Option<&str>,
) -> Result<String, MutationError> {
    let embedded = markdown.and_then(crate::services::knowledge_store::note_identity_from_markdown);
    let mapped = note_id_for_path(ledger, relative_key);
    let note_id = match (embedded, mapped) {
        (Some(embedded), Some(mapped))
            if embedded
                == crate::services::knowledge_store::default_note_identity_for_relative_path(
                    relative_key,
                ) =>
        {
            mapped
        }
        (Some(embedded), _) => embedded,
        (None, Some(mapped)) => mapped,
        (None, None) => legacy_note_id(scope, relative_key),
    };
    NoteRevisionV1::tombstone(note_id.clone())
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
    Ok(note_id)
}

pub(super) fn note_id_for_path(ledger: &EngineLedgerV1, relative_key: &str) -> Option<String> {
    ledger
        .note_paths
        .iter()
        .find_map(|(note_id, path)| (path == relative_key).then_some(note_id.clone()))
}

pub(super) fn legacy_note_id(scope: &ContentDigest, relative_key: &str) -> String {
    let identity = crate::services::twin_events::digest_bytes(
        format!("grafyn.sync.note-id.v1:{}:{relative_key}", scope.as_str()).as_bytes(),
    );
    format!("legacy-{}", identity.as_str())
}

pub(super) fn ensure_note_projection(
    data_root: &AnchoredRoot,
    state: &mut EngineState,
    note_id: &str,
) -> Result<String, MutationError> {
    try_ensure_note_projection(data_root, state, note_id)?.ok_or_else(|| {
        MutationError::RecoveryConflict(
            "sync note projection collided with an existing local path".into(),
        )
    })
}

pub(super) fn try_ensure_note_projection(
    data_root: &AnchoredRoot,
    state: &mut EngineState,
    note_id: &str,
) -> Result<Option<String>, MutationError> {
    if let Some(relative_key) = state.ledger.note_paths.get(note_id) {
        return Ok(Some(relative_key.clone()));
    }
    if state.ledger.note_paths.len() >= MAX_NOTE_PROJECTIONS {
        return Err(MutationError::Invalid(
            "sync note projection limit exceeded".into(),
        ));
    }
    let digest = crate::services::twin_events::digest_bytes(
        format!("grafyn.sync.remote-path.v1:{note_id}").as_bytes(),
    );
    let relative_key = format!("synced/{}.md", digest.as_str());
    crate::services::twin_events::validate_target_key(TargetKind::Markdown, &relative_key)?;
    if state
        .ledger
        .note_paths
        .iter()
        .any(|(existing_id, path)| existing_id != note_id && path == &relative_key)
    {
        return Ok(None);
    }
    state
        .ledger
        .note_paths
        .insert(note_id.to_owned(), relative_key.clone());
    persist_ledger(
        data_root,
        &engine_ledger_key(&state.vault_scope),
        &state.ledger,
    )?;
    Ok(Some(relative_key))
}

pub(super) fn local_only_references(policy: &BTreeMap<String, String>) -> BTreeSet<String> {
    policy
        .iter()
        .flat_map(|(path, note_id)| [path.clone(), note_id.clone()])
        .collect()
}

pub(super) fn local_only_references_with_paths(
    policy: &BTreeMap<String, String>,
    note_paths: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    let mut references = local_only_references(policy);
    references.extend(
        note_paths
            .iter()
            .filter(|(_, path)| policy.contains_key(path.as_str()))
            .map(|(note_id, _)| note_id.clone()),
    );
    references
}

pub(super) fn is_reserved_program(note_key: &str) -> bool {
    note_key
        .replace('\\', "/")
        .eq_ignore_ascii_case(RESERVED_PROGRAM_NOTE)
}
