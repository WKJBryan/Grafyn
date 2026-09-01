use super::*;

pub(super) fn generated_image_attachment_digests(
    events: &[TwinEvent],
    sync_only: bool,
) -> Result<BTreeSet<Digest32>, MutationError> {
    generated_image_attachment_digests_with_policy(events, sync_only, false)
}

pub(super) fn quarantine_invalid_generated_image_attachment(
    data_root: &AnchoredRoot,
    state: &mut EngineState,
    digest: &Digest32,
) -> Result<(), MutationError> {
    let rejected = state
        .operations
        .iter()
        .filter_map(
            |(operation_id, verified)| match verified.operation().payload() {
                OperationPayloadV1::AttachmentManifest(manifest)
                    if manifest.attachment_digest() == digest =>
                {
                    Some(*operation_id)
                }
                OperationPayloadV1::AttachmentChunk(chunk)
                    if chunk.attachment_digest() == digest =>
                {
                    Some(*operation_id)
                }
                _ => None,
            },
        )
        .collect::<BTreeSet<_>>();
    let new_rejections = rejected
        .iter()
        .filter(|operation_id| {
            !state
                .ledger
                .rejected_operations
                .contains(&operation_id.to_string())
        })
        .count();
    if state
        .ledger
        .rejected_operations
        .len()
        .checked_add(new_rejections)
        .is_none_or(|count| count > MAX_REJECTED_OPERATIONS)
    {
        return Err(MutationError::Invalid(
            "sync rejected-operation quarantine limit exceeded".into(),
        ));
    }
    for operation_id in rejected {
        state
            .ledger
            .rejected_operations
            .insert(operation_id.to_string());
    }
    if new_rejections > 0 {
        persist_ledger(
            data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )?;
    }
    Ok(())
}

struct GeneratedImageObservationBinding {
    note_id: String,
    note_digest: ContentDigest,
    attachment_digest: Digest32,
    device_id: String,
    causal_stream: CausalStream,
    causal_parents: Vec<EventId>,
}

struct GeneratedImageNoteBinding {
    note_id: String,
    note_digest: ContentDigest,
    event_id: EventId,
    device_id: String,
    causal_stream: CausalStream,
}

pub(super) fn generated_image_attachment_digests_with_policy(
    events: &[TwinEvent],
    sync_only: bool,
    defer_incomplete_pair: bool,
) -> Result<BTreeSet<Digest32>, MutationError> {
    let mut observations = Vec::new();
    let mut note_changes = Vec::new();
    for event in events {
        match generated_image_event_binding(event, sync_only)? {
            Some(GeneratedImageEventBinding::Observation(binding)) => observations.push(binding),
            Some(GeneratedImageEventBinding::NoteChanged(binding)) => note_changes.push(binding),
            None => {}
        }
    }
    let mut digests = BTreeSet::new();
    for observation in observations {
        let matching = note_changes
            .iter()
            .filter(|note_change| {
                note_change.note_id == observation.note_id
                    && note_change.note_digest == observation.note_digest
                    && note_change.device_id == observation.device_id
                    && note_change.causal_stream == observation.causal_stream
            })
            .collect::<Vec<_>>();
        if matching.is_empty() && defer_incomplete_pair {
            continue;
        }
        if matching.len() != 1 {
            return Err(MutationError::Invalid(
                "image generation observation requires exactly one matching NoteChanged event"
                    .into(),
            ));
        }
        if !observation.causal_parents.contains(&matching[0].event_id) {
            return Err(MutationError::Invalid(
                "image generation observation must causally follow its matching NoteChanged event"
                    .into(),
            ));
        }
        digests.insert(observation.attachment_digest);
    }
    Ok(digests)
}

enum GeneratedImageEventBinding {
    Observation(GeneratedImageObservationBinding),
    NoteChanged(GeneratedImageNoteBinding),
}

pub(super) fn validate_generated_image_event_shape(
    event: &TwinEvent,
    sync_only: bool,
) -> Result<(), MutationError> {
    let _ = generated_image_event_binding(event, sync_only)?;
    Ok(())
}

fn generated_image_event_binding(
    event: &TwinEvent,
    sync_only: bool,
) -> Result<Option<GeneratedImageEventBinding>, MutationError> {
    if event.context.source_channel.as_str() != "image_generation"
        || (sync_only && event.causal_stream != CausalStream::SyncEligible)
    {
        return Ok(None);
    }
    if !event.context.relationships.is_empty() {
        return Err(MutationError::Invalid(
            "image generation provenance requires direct evidence, not relationship evidence"
                .into(),
        ));
    }
    match &event.payload {
        TwinEventPayload::ObservationRecorded(observation) => {
            let Some(note_id) = observation
                .observation_id
                .as_str()
                .strip_prefix("companion-capture-")
            else {
                return Err(MutationError::Invalid(
                    "image generation observation has no canonical companion capture identity"
                        .into(),
                ));
            };
            let Some(note_digest) = observation.content_digest.as_ref() else {
                return Err(MutationError::Invalid(
                    "image generation observation must remain claim-free captured evidence".into(),
                ));
            };
            if !observation.claims.is_empty() {
                return Err(MutationError::Invalid(
                    "image generation observation must remain claim-free captured evidence".into(),
                ));
            }
            if event.evidence.len() != 2 {
                return Err(MutationError::Invalid(
                    "image generation observation requires exactly two direct evidence entries"
                        .into(),
                ));
            }
            let mut note_matches = 0usize;
            let mut attachment = None;
            for evidence in &event.evidence {
                match evidence.evidence_type {
                    EvidenceType::Note if evidence.source_id.as_str() == note_id => {
                        if evidence.digest.as_ref() != Some(note_digest) {
                            return Err(MutationError::Invalid(
                                "image generation note evidence digest must equal observation content digest"
                                    .into(),
                            ));
                        }
                        note_matches += 1;
                    }
                    EvidenceType::Attachment => {
                        let digest = evidence.digest.as_ref().ok_or_else(|| {
                            MutationError::Invalid(
                                "image generation attachment evidence requires a digest".into(),
                            )
                        })?;
                        if evidence.source_id.as_str() != digest.as_str() {
                            return Err(MutationError::Invalid(
                                "image generation attachment identity must equal its digest".into(),
                            ));
                        }
                        if attachment
                            .replace(
                                Digest32::parse_hex(digest.as_str())
                                    .map_err(|error| MutationError::Invalid(error.to_string()))?,
                            )
                            .is_some()
                        {
                            return Err(MutationError::Invalid(
                                "image generation observation must bind one attachment".into(),
                            ));
                        }
                    }
                    _ => {
                        return Err(MutationError::Invalid(
                            "image generation observation has non-canonical direct evidence".into(),
                        ))
                    }
                }
            }
            if note_matches != 1 || attachment.is_none() {
                return Err(MutationError::Invalid(
                    "image generation observation must bind one note and one attachment".into(),
                ));
            }
            Ok(Some(GeneratedImageEventBinding::Observation(
                GeneratedImageObservationBinding {
                    note_id: note_id.to_owned(),
                    note_digest: note_digest.clone(),
                    attachment_digest: attachment.expect("attachment checked"),
                    device_id: event.device_id.as_str().to_owned(),
                    causal_stream: event.causal_stream,
                    causal_parents: event.causal_parents.clone(),
                },
            )))
        }
        TwinEventPayload::NoteChanged(note_change) => {
            if note_change.change != crate::models::twin_event::NoteChangeKind::Created {
                return Err(MutationError::Invalid(
                    "image generation NoteChanged event must describe creation".into(),
                ));
            }
            let Some(note_digest) = note_change.content_digest.as_ref() else {
                return Err(MutationError::Invalid(
                    "image generation NoteChanged event requires a content digest".into(),
                ));
            };
            if event.evidence.len() != 1 {
                return Err(MutationError::Invalid(
                    "image generation NoteChanged event requires one direct note evidence entry"
                        .into(),
                ));
            }
            let evidence = &event.evidence[0];
            if evidence.evidence_type != EvidenceType::Note
                || evidence.source_id.as_str() != note_change.note_id.as_str()
                || evidence.digest.as_ref() != Some(note_digest)
            {
                return Err(MutationError::Invalid(
                    "image generation NoteChanged evidence must bind its exact note content".into(),
                ));
            }
            Ok(Some(GeneratedImageEventBinding::NoteChanged(
                GeneratedImageNoteBinding {
                    note_id: note_change.note_id.as_str().to_owned(),
                    note_digest: note_digest.clone(),
                    event_id: event.event_id.clone(),
                    device_id: event.device_id.as_str().to_owned(),
                    causal_stream: event.causal_stream,
                },
            )))
        }
        _ => Err(MutationError::Invalid(
            "image generation source is reserved for canonical capture events".into(),
        )),
    }
}
