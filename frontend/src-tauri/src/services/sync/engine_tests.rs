use super::*;
use crate::models::twin_event::{
    EntityId, EvidenceRef, EvidenceType, Governance, NoteChangeKind, NoteChanged,
    ObservationRecorded, RelationshipAssertion, RelationshipDirection, RelationshipPredicate,
    SourceChannel, TwinEventPayload,
};
use crate::services::sync::identity::{
    load_or_create_vault_identity, load_vault_identity, VAULT_DESCRIPTOR_KEY,
};
use crate::services::sync::secrets::MemorySecretStore;
use crate::services::sync::vault_keys::provision_vault_root_key;
use crate::services::twin_events::TwinEventDraft;
use chrono::{TimeZone, Utc};
use grafyn_sync_protocol::{
    DeviceSigningKey, NoteRevisionKind, NoteRevisionV1, OperationId, OperationPayloadV1,
    OperationV1,
};
use std::fs;
use std::io::Cursor;
use tempfile::TempDir;

const REMOTE_A: &str = "123e4567-e89b-42d3-a456-4266141740a1";
const REMOTE_B: &str = "123e4567-e89b-42d3-a456-4266141740b2";
const ROOT_KEY_BYTES: [u8; 32] = [0x31; 32];

struct Harness {
    _data: TempDir,
    vault: TempDir,
    identity: VaultIdentity,
    secret_store: Arc<MemorySecretStore>,
    engine: Arc<SyncEngine>,
    coordinator: MutationCoordinator,
}

impl Harness {
    fn new(descriptor: Option<&[u8]>) -> Self {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        fs::create_dir_all(data.path().join("twin/events")).unwrap();
        if let Some(descriptor) = descriptor {
            let path = vault.path().join(VAULT_DESCRIPTOR_KEY);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, descriptor).unwrap();
        }
        let identity = if descriptor.is_some() {
            load_vault_identity(vault.path()).unwrap()
        } else {
            load_or_create_vault_identity(vault.path()).unwrap()
        };
        let event_store = Arc::new(TwinEventStore::new(data.path()));
        let secret_store = Arc::new(MemorySecretStore::default());
        provision_vault_root_key(
            secret_store.as_ref(),
            &identity.descriptor.vault_id().to_string(),
            &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        )
        .unwrap();
        let engine = Arc::new(
            SyncEngine::open_core(
                data.path(),
                vault.path(),
                identity.clone(),
                Some(VaultRootKey::from_bytes(ROOT_KEY_BYTES)),
                secret_store.clone(),
                event_store.clone(),
            )
            .unwrap(),
        );
        let coordinator =
            MutationCoordinator::new_stable(data.path(), vault.path(), event_store, engine.clone())
                .unwrap();
        let device = coordinator
            .load_or_create_device_signing_identity(secret_store.clone())
            .unwrap();
        engine.attach_device_identity(device).unwrap();
        Self {
            _data: data,
            vault,
            identity,
            secret_store,
            engine,
            coordinator,
        }
    }

    fn open_peer(&self) -> (Arc<SyncEngine>, MutationCoordinator) {
        let event_store = Arc::new(TwinEventStore::new(self._data.path()));
        let engine = Arc::new(
            SyncEngine::open_core(
                self._data.path(),
                self.vault.path(),
                load_vault_identity(self.vault.path()).unwrap(),
                Some(VaultRootKey::from_bytes(ROOT_KEY_BYTES)),
                self.secret_store.clone(),
                event_store.clone(),
            )
            .unwrap(),
        );
        let coordinator = MutationCoordinator::new_stable(
            self._data.path(),
            self.vault.path(),
            event_store,
            engine.clone(),
        )
        .unwrap();
        let device = coordinator
            .load_or_create_device_signing_identity(self.secret_store.clone())
            .unwrap();
        engine.attach_device_identity(device).unwrap();
        (engine, coordinator)
    }

    fn descriptor_bytes(&self) -> Vec<u8> {
        fs::read(self.vault.path().join(VAULT_DESCRIPTOR_KEY)).unwrap()
    }

    fn trust(&self, device: &str, seed: [u8; 32]) {
        let signing_key = DeviceSigningKey::from_seed(seed);
        self.engine
            .trust_device(
                DeviceId::parse_str(device).unwrap(),
                signing_key.public_key(),
            )
            .unwrap();
    }

    fn receive(&self, envelopes: &[EnvelopeV1]) -> SyncDrainReport {
        let bytes = envelopes
            .iter()
            .map(|envelope| envelope.to_json().unwrap().into_bytes())
            .collect::<Vec<_>>();
        self.engine
            .receive_envelopes(&self.coordinator, &bytes)
            .unwrap()
    }
}

fn note_envelope(
    harness: &Harness,
    device: &str,
    seed: [u8; 32],
    note_id: &str,
    revision: NoteRevisionKind,
    parents: Vec<OperationId>,
) -> EnvelopeV1 {
    let signing_key = DeviceSigningKey::from_seed(seed);
    let revision = match revision {
        NoteRevisionKind::Put { markdown } => NoteRevisionV1::put(note_id, markdown).unwrap(),
        NoteRevisionKind::Tombstone => NoteRevisionV1::tombstone(note_id).unwrap(),
    };
    let operation = OperationV1::new(
        1_800_000_000_000,
        parents,
        OperationPayloadV1::NoteRevision(revision),
    )
    .unwrap();
    seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &DeviceId::parse_str(device).unwrap(),
        &signing_key,
        &operation,
    )
    .unwrap()
}

fn event_envelope(
    harness: &Harness,
    device: &str,
    seed: [u8; 32],
    event: &TwinEvent,
    parents: Vec<OperationId>,
) -> EnvelopeV1 {
    let payload = TwinEventV1::new(
        Digest32::parse_hex(event.event_id.as_str()).unwrap(),
        serde_json::to_string(event).unwrap(),
    )
    .unwrap();
    let operation = OperationV1::new(
        1_800_000_000_007,
        parents,
        OperationPayloadV1::TwinEvent(payload),
    )
    .unwrap();
    seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &DeviceId::parse_str(device).unwrap(),
        &DeviceSigningKey::from_seed(seed),
        &operation,
    )
    .unwrap()
}

fn trust_each_other(first: &Harness, second: &Harness) {
    let (first_id, first_key) = first.engine.local_device().unwrap();
    let (second_id, second_key) = second.engine.local_device().unwrap();
    first.engine.trust_device(second_id, second_key).unwrap();
    second.engine.trust_device(first_id, first_key).unwrap();
}

fn relay(source: &Harness, destination: &Harness) -> SyncDrainReport {
    destination
        .engine
        .receive_envelopes(
            &destination.coordinator,
            &source.engine.export_outbox().unwrap(),
        )
        .unwrap()
}

fn commit_note(harness: &Harness, note_key: &str, markdown: &str) {
    let _ = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("note_editor").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                note_key,
                markdown,
            )],
            vec![],
        )
        .unwrap();
}

fn delete_note(harness: &Harness, note_key: &str) {
    let _ = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("note_editor").unwrap(),
            vec![TargetMutation::tombstone(TargetKind::Markdown, note_key)],
            vec![],
        )
        .unwrap();
}

fn projected_note_key(harness: &Harness, note_id: &str) -> String {
    harness
        .engine
        .state
        .lock()
        .unwrap()
        .ledger
        .note_paths
        .get(note_id)
        .cloned()
        .unwrap()
}

fn projected_note_path(harness: &Harness, note_id: &str) -> PathBuf {
    harness
        .vault
        .path()
        .join(projected_note_key(harness, note_id))
}

fn draft(label: &str) -> TwinEventDraft {
    TwinEventDraft {
        actor_id: None,
        causal_parents: Vec::new(),
        recorded_at: Utc.with_ymd_and_hms(2026, 8, 31, 1, 0, 0).unwrap(),
        observed_at: Utc.with_ymd_and_hms(2026, 8, 31, 1, 0, 0).unwrap(),
        occurred_at: None,
        valid_from: None,
        valid_to: None,
        supersedes: Vec::new(),
        reinforces: Vec::new(),
        context: Default::default(),
        evidence: Vec::new(),
        governance: Governance::direct_observation(),
        payload: TwinEventPayload::NoteChanged(NoteChanged {
            note_id: crate::models::twin_event::Identifier::parse(label).unwrap(),
            change: NoteChangeKind::Created,
            content_digest: None,
        }),
    }
}

fn id(byte: u8) -> OperationId {
    OperationId::from_bytes([byte; 32])
}

fn generated_png() -> Vec<u8> {
    let image = image::DynamicImage::new_rgba8(2, 2);
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn generic_non_square_png() -> Vec<u8> {
    let image = image::DynamicImage::new_rgba8(2, 1);
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

#[derive(Clone)]
struct FixedRemoteEventIdentity {
    device_id: crate::models::twin_event::DeviceId,
}

impl crate::services::twin_events::MutationIdentityProvider for FixedRemoteEventIdentity {
    fn actor_id(&self) -> crate::models::twin_event::ActorId {
        crate::models::twin_event::ActorId::parse("owner").unwrap()
    }

    fn device_id(&self) -> crate::models::twin_event::DeviceId {
        self.device_id.clone()
    }
}

fn split_generated_image_batch(harness: &Harness) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let mut canonical = Vec::new();
    let mut attachments = Vec::new();
    let outbox = harness.engine.export_outbox().unwrap();
    let state = harness.engine.state.lock().unwrap();
    for bytes in outbox {
        let verified = verify_envelope(
            &state,
            state.root_key.as_ref().unwrap(),
            &EnvelopeV1::from_json_bytes(&bytes).unwrap(),
        )
        .unwrap();
        if matches!(
            verified.operation().payload(),
            OperationPayloadV1::AttachmentManifest(_) | OperationPayloadV1::AttachmentChunk(_)
        ) {
            attachments.push(bytes);
        } else {
            canonical.push(bytes);
        }
    }
    (canonical, attachments)
}

fn image_observation_drafts(
    note_id: &str,
    attachment_digest: &ContentDigest,
    local_only: bool,
) -> Vec<TwinEventDraft> {
    let note_digest = ContentDigest::parse(
        crate::models::attachment::sha256_attachment_digest(note_id.as_bytes()).to_string(),
    )
    .unwrap();
    let governance = if local_only {
        crate::services::twin_events::local_capture_governance(Sensitivity::Standard)
    } else {
        crate::services::twin_events::standard_capture_governance()
    };
    let mut note_changed = draft(note_id);
    note_changed.governance = governance.clone();
    let TwinEventPayload::NoteChanged(note_change) = &mut note_changed.payload else {
        unreachable!("test fixture creates a NoteChanged draft")
    };
    note_change.content_digest = Some(note_digest.clone());
    note_changed.evidence.push(EvidenceRef {
        evidence_type: EvidenceType::Note,
        source_id: crate::models::twin_event::Identifier::parse(note_id).unwrap(),
        digest: Some(note_digest.clone()),
    });
    let mut observation = TwinEventDraft::observed(
        TwinEventPayload::ObservationRecorded(ObservationRecorded {
            observation_id: crate::models::twin_event::Identifier::parse(format!(
                "companion-capture-{note_id}"
            ))
            .unwrap(),
            claims: Vec::new(),
            summary: None,
            content_digest: Some(note_digest.clone()),
        }),
        Utc.with_ymd_and_hms(2026, 9, 1, 5, 0, 0).unwrap(),
        SourceChannel::parse("image_generation").unwrap(),
        governance,
    );
    observation.evidence.push(EvidenceRef {
        evidence_type: EvidenceType::Note,
        source_id: crate::models::twin_event::Identifier::parse(note_id).unwrap(),
        digest: Some(note_digest),
    });
    observation.evidence.push(EvidenceRef {
        evidence_type: EvidenceType::Attachment,
        source_id: crate::models::twin_event::Identifier::parse(attachment_digest.as_str())
            .unwrap(),
        digest: Some(attachment_digest.clone()),
    });
    vec![note_changed, observation]
}

fn attachment_artifact_path(harness: &Harness, directory: &str, digest: &Digest32) -> PathBuf {
    harness
        ._data
        .path()
        .join("sync/vaults/v1")
        .join(harness.identity.root_scope.as_str())
        .join("attachments/v1")
        .join(directory)
        .join(format!(
            "{digest}.{}",
            if directory == "blobs" { "blob" } else { "json" }
        ))
}

fn commit_generated_image(
    harness: &Harness,
    note_id: &str,
    local_only: bool,
) -> (Vec<u8>, Digest32, ImageAttachmentCatalogRecordV1) {
    let bytes = generated_png();
    let catalog = harness
        .engine
        .store_cataloged_image_expecting_scope(
            &harness.identity.root_scope,
            &bytes,
            Some("image/png"),
        )
        .unwrap();
    let digest = catalog.attachment_digest().unwrap();
    let content_digest = ContentDigest::parse(digest.to_string()).unwrap();
    let _ = harness
        .coordinator
        .commit_local(
            if local_only {
                CausalStream::LocalOnly
            } else {
                CausalStream::SyncEligible
            },
            SourceChannel::parse("image_generation").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                format!("{note_id}.md"),
                format!(
                    "---\nnote_id: {note_id}\ngrafyn_sync: {}\n---\nimage",
                    if local_only { "local_only" } else { "inherit" }
                ),
            )],
            image_observation_drafts(note_id, &content_digest, local_only),
        )
        .unwrap();
    (bytes, digest, catalog)
}

#[test]
fn governed_image_attachment_joins_the_note_event_mutation_batch() {
    let harness = Harness::new(None);
    let bytes = generated_png();
    let catalog = harness
        .engine
        .store_cataloged_image_expecting_scope(
            &harness.identity.root_scope,
            &bytes,
            Some("image/png"),
        )
        .unwrap();
    let digest = ContentDigest::parse(catalog.attachment_digest().unwrap().to_string()).unwrap();
    let _commit = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("image_generation").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "generated-image-note.md",
                "---\nnote_id: generated-image-note\ngrafyn_sync: inherit\n---\nimage",
            )],
            image_observation_drafts("generated-image-note", &digest, false),
        )
        .unwrap();

    let outbox = harness.engine.export_outbox().unwrap();
    let state = harness.engine.state.lock().unwrap();
    let verified = outbox
        .iter()
        .map(|bytes| {
            verify_envelope(
                &state,
                state.root_key.as_ref().unwrap(),
                &EnvelopeV1::from_json_bytes(bytes).unwrap(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        verified
            .iter()
            .filter(|value| matches!(
                value.operation().payload(),
                OperationPayloadV1::NoteRevision(_)
            ))
            .count(),
        1
    );
    assert_eq!(
        verified
            .iter()
            .filter(|value| matches!(
                value.operation().payload(),
                OperationPayloadV1::TwinEvent(_)
            ))
            .count(),
        2
    );
    let manifests = verified
        .iter()
        .filter(|value| {
            matches!(
                value.operation().payload(),
                OperationPayloadV1::AttachmentManifest(_)
            )
        })
        .collect::<Vec<_>>();
    let chunks = verified
        .iter()
        .filter(|value| {
            matches!(
                value.operation().payload(),
                OperationPayloadV1::AttachmentChunk(_)
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(manifests.len(), 1);
    assert_eq!(chunks.len(), 1);
    assert_eq!(
        chunks[0].operation().causal_parents(),
        [*manifests[0].operation_id()]
    );
    assert_eq!(outbox.len(), 5);
}

#[test]
fn generic_attachment_evidence_keeps_its_existing_standalone_sync_path() {
    let harness = Harness::new(None);
    let bytes = b"existing generic attachment";
    let digest = harness
        .engine
        .queue_attachment("application/octet-stream", bytes)
        .unwrap();
    let content_digest = ContentDigest::parse(digest.to_string()).unwrap();
    let mut observation = TwinEventDraft::observed(
        TwinEventPayload::ObservationRecorded(ObservationRecorded {
            observation_id: crate::models::twin_event::Identifier::parse(
                "generic-attachment-observation",
            )
            .unwrap(),
            claims: Vec::new(),
            summary: None,
            content_digest: None,
        }),
        Utc.with_ymd_and_hms(2026, 9, 1, 5, 5, 0).unwrap(),
        SourceChannel::parse("companion_capture").unwrap(),
        crate::services::twin_events::standard_capture_governance(),
    );
    observation.evidence.push(EvidenceRef {
        evidence_type: EvidenceType::Attachment,
        source_id: crate::models::twin_event::Identifier::parse(content_digest.as_str()).unwrap(),
        digest: Some(content_digest),
    });

    let _ = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("companion_capture").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "generic-attachment-note.md",
                "---\nnote_id: generic-attachment-note\ngrafyn_sync: inherit\n---\ngeneric",
            )],
            vec![draft("generic-attachment-note"), observation],
        )
        .unwrap();

    let outbox = harness.engine.export_outbox().unwrap();
    let state = harness.engine.state.lock().unwrap();
    let payloads = outbox
        .iter()
        .map(|bytes| {
            verify_envelope(
                &state,
                state.root_key.as_ref().unwrap(),
                &EnvelopeV1::from_json_bytes(bytes).unwrap(),
            )
            .unwrap()
            .operation()
            .payload()
            .clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(outbox.len(), 5);
    assert_eq!(
        payloads
            .iter()
            .filter(|payload| matches!(payload, OperationPayloadV1::AttachmentManifest(_)))
            .count(),
        1,
        "the canonical event mutation must not duplicate the standalone manifest"
    );
    assert_eq!(
        payloads
            .iter()
            .filter(|payload| matches!(payload, OperationPayloadV1::AttachmentChunk(_)))
            .count(),
        1,
        "the canonical event mutation must not duplicate standalone chunks"
    );
}

#[test]
fn non_image_source_cannot_trigger_image_catalog_lookup_with_spoofed_evidence() {
    let harness = Harness::new(None);
    let digest = ContentDigest::parse("ab".repeat(32)).unwrap();
    let mut observation = TwinEventDraft::observed(
        TwinEventPayload::ObservationRecorded(ObservationRecorded {
            observation_id: crate::models::twin_event::Identifier::parse(
                "spoofed-image-observation",
            )
            .unwrap(),
            claims: Vec::new(),
            summary: None,
            content_digest: None,
        }),
        Utc.with_ymd_and_hms(2026, 9, 1, 5, 6, 0).unwrap(),
        SourceChannel::parse("companion_capture").unwrap(),
        crate::services::twin_events::standard_capture_governance(),
    );
    observation.evidence.push(EvidenceRef {
        evidence_type: EvidenceType::Attachment,
        source_id: crate::models::twin_event::Identifier::parse(digest.as_str()).unwrap(),
        digest: Some(digest),
    });

    let _ = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("companion_capture").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "spoofed-image-note.md",
                "---\nnote_id: spoofed-image-note\ngrafyn_sync: inherit\n---\nspoofed",
            )],
            vec![draft("spoofed-image-note"), observation],
        )
        .unwrap();

    let outbox = harness.engine.export_outbox().unwrap();
    let state = harness.engine.state.lock().unwrap();
    assert_eq!(outbox.len(), 3);
    assert!(outbox.iter().all(|bytes| {
        let verified = verify_envelope(
            &state,
            state.root_key.as_ref().unwrap(),
            &EnvelopeV1::from_json_bytes(bytes).unwrap(),
        )
        .unwrap();
        !matches!(
            verified.operation().payload(),
            OperationPayloadV1::AttachmentManifest(_) | OperationPayloadV1::AttachmentChunk(_)
        )
    }));
}

#[test]
fn generated_image_note_evidence_digest_must_match_observation_content() {
    let harness = Harness::new(None);
    let bytes = generated_png();
    let catalog = harness
        .engine
        .store_cataloged_image_expecting_scope(
            &harness.identity.root_scope,
            &bytes,
            Some("image/png"),
        )
        .unwrap();
    let digest = ContentDigest::parse(catalog.attachment_digest().unwrap().to_string()).unwrap();
    let mut drafts = image_observation_drafts("mismatched-note-digest", &digest, false);
    let observation = drafts
        .iter_mut()
        .find(|draft| matches!(draft.payload, TwinEventPayload::ObservationRecorded(_)))
        .unwrap();
    observation
        .evidence
        .iter_mut()
        .find(|evidence| evidence.evidence_type == EvidenceType::Note)
        .unwrap()
        .digest = Some(ContentDigest::parse("ed".repeat(32)).unwrap());

    let error = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("image_generation").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "mismatched-note-digest.md",
                "---\nnote_id: mismatched-note-digest\ngrafyn_sync: inherit\n---\nimage",
            )],
            drafts,
        )
        .unwrap_err();

    assert!(error.to_string().contains("note evidence digest"));
    assert!(harness.engine.export_outbox().unwrap().is_empty());
}

#[test]
fn generated_image_provenance_cannot_borrow_relationship_evidence() {
    let harness = Harness::new(None);
    let bytes = generated_png();
    let catalog = harness
        .engine
        .store_cataloged_image_expecting_scope(
            &harness.identity.root_scope,
            &bytes,
            Some("image/png"),
        )
        .unwrap();
    let digest = ContentDigest::parse(catalog.attachment_digest().unwrap().to_string()).unwrap();
    let mut drafts = image_observation_drafts("relationship-borrowed-image", &digest, false);
    let observation = drafts
        .iter_mut()
        .find(|draft| matches!(draft.payload, TwinEventPayload::ObservationRecorded(_)))
        .unwrap();
    let borrowed = std::mem::take(&mut observation.evidence);
    observation
        .context
        .relationships
        .push(RelationshipAssertion {
            subject_id: EntityId::parse("owner").unwrap(),
            predicate: RelationshipPredicate::parse("works_with").unwrap(),
            object_id: EntityId::parse("person-1").unwrap(),
            direction: RelationshipDirection::Directed,
            valid_from: None,
            valid_to: None,
            evidence: borrowed,
            governance: crate::services::twin_events::standard_capture_governance(),
        });

    let error = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("image_generation").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "relationship-borrowed-image.md",
                "---\nnote_id: relationship-borrowed-image\ngrafyn_sync: inherit\n---\nimage",
            )],
            drafts,
        )
        .unwrap_err();

    assert!(error.to_string().contains("direct evidence"));
    assert!(harness.engine.export_outbox().unwrap().is_empty());
}

#[test]
fn generated_image_observation_requires_its_matching_note_changed_event() {
    let harness = Harness::new(None);
    let bytes = generated_png();
    let catalog = harness
        .engine
        .store_cataloged_image_expecting_scope(
            &harness.identity.root_scope,
            &bytes,
            Some("image/png"),
        )
        .unwrap();
    let digest = ContentDigest::parse(catalog.attachment_digest().unwrap().to_string()).unwrap();
    let drafts = image_observation_drafts("unpaired-generated-image", &digest, false)
        .into_iter()
        .filter(|draft| matches!(draft.payload, TwinEventPayload::ObservationRecorded(_)))
        .collect();

    let error = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("image_generation").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "unpaired-generated-image.md",
                "---\nnote_id: unpaired-generated-image\ngrafyn_sync: inherit\n---\nimage",
            )],
            drafts,
        )
        .unwrap_err();

    assert!(error.to_string().contains("matching NoteChanged"));
    assert!(harness.engine.export_outbox().unwrap().is_empty());
}

#[test]
fn generated_image_event_before_chunks_fails_closed_then_reconstructs_after_reopen() {
    let first = Harness::new(None);
    let descriptor = first.descriptor_bytes();
    let second = Harness::new(Some(&descriptor));
    trust_each_other(&first, &second);
    let bytes = generated_png();
    let catalog = first
        .engine
        .store_cataloged_image_expecting_scope(
            &first.identity.root_scope,
            &bytes,
            Some("image/png"),
        )
        .unwrap();
    let digest = catalog.attachment_digest().unwrap();
    let content_digest = ContentDigest::parse(digest.to_string()).unwrap();
    let _ = first
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("image_generation").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "remote-generated-image.md",
                "---\nnote_id: remote-generated-image\ngrafyn_sync: inherit\n---\nimage",
            )],
            image_observation_drafts("remote-generated-image", &content_digest, false),
        )
        .unwrap();
    let (canonical, attachments) = split_generated_image_batch(&first);
    assert_eq!(canonical.len(), 3);
    assert_eq!(attachments.len(), 2);

    let error = second
        .engine
        .receive_envelopes(&second.coordinator, &canonical)
        .unwrap_err();
    assert!(matches!(error, MutationError::RecoveryConflict(_)));
    assert_eq!(
        second.engine.materialized_attachment(&digest).unwrap(),
        None
    );
    assert_eq!(
        second
            .engine
            .cataloged_image_expecting_scope(&second.identity.root_scope, &digest)
            .unwrap(),
        None
    );
    let (reopened, reopened_coordinator) = second.open_peer();
    reopened
        .receive_envelopes(&reopened_coordinator, &attachments)
        .unwrap();

    assert_eq!(
        reopened
            .cataloged_image_expecting_scope(&second.identity.root_scope, &digest)
            .unwrap(),
        Some(catalog)
    );
    assert_eq!(
        reopened.materialized_attachment(&digest).unwrap(),
        Some(bytes)
    );
    assert_eq!(reopened.status().unwrap().outbox_operations, 0);
}

#[test]
fn generated_image_chunks_before_event_stay_generic_across_reopen_then_catalog() {
    let first = Harness::new(None);
    let descriptor = first.descriptor_bytes();
    let second = Harness::new(Some(&descriptor));
    trust_each_other(&first, &second);
    let bytes = generated_png();
    let catalog = first
        .engine
        .store_cataloged_image_expecting_scope(
            &first.identity.root_scope,
            &bytes,
            Some("image/png"),
        )
        .unwrap();
    let digest = catalog.attachment_digest().unwrap();
    let content_digest = ContentDigest::parse(digest.to_string()).unwrap();
    let _ = first
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("image_generation").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "attachment-first-generated-image.md",
                "---\nnote_id: attachment-first-generated-image\ngrafyn_sync: inherit\n---\nimage",
            )],
            image_observation_drafts("attachment-first-generated-image", &content_digest, false),
        )
        .unwrap();
    let (canonical, attachments) = split_generated_image_batch(&first);

    second
        .engine
        .receive_envelopes(&second.coordinator, &attachments)
        .unwrap();
    assert_eq!(
        second.engine.materialized_attachment(&digest).unwrap(),
        Some(bytes.clone())
    );
    assert_eq!(
        second
            .engine
            .cataloged_image_expecting_scope(&second.identity.root_scope, &digest)
            .unwrap(),
        None
    );
    let (reopened, reopened_coordinator) = second.open_peer();
    assert_eq!(
        reopened.materialized_attachment(&digest).unwrap(),
        Some(bytes)
    );
    assert_eq!(
        reopened
            .cataloged_image_expecting_scope(&second.identity.root_scope, &digest)
            .unwrap(),
        None
    );

    reopened
        .receive_envelopes(&reopened_coordinator, &canonical)
        .unwrap();
    assert_eq!(
        reopened
            .cataloged_image_expecting_scope(&second.identity.root_scope, &digest)
            .unwrap(),
        Some(catalog)
    );
}

#[test]
fn generated_image_catalog_repairs_only_from_complete_applied_manifest_and_blob() {
    let harness = Harness::new(None);
    let (bytes, digest, catalog) =
        commit_generated_image(&harness, "repair-generated-image", false);
    let catalog_path = attachment_artifact_path(&harness, "catalog", &digest);
    std::fs::remove_file(&catalog_path).unwrap();

    harness
        .engine
        .recover_pending_inbox(&harness.coordinator)
        .unwrap();

    assert!(catalog_path.is_file());
    assert_eq!(
        harness
            .engine
            .cataloged_image_expecting_scope(&harness.identity.root_scope, &digest)
            .unwrap(),
        Some(catalog)
    );
    assert_eq!(
        harness.engine.materialized_attachment(&digest).unwrap(),
        Some(bytes)
    );
}

#[test]
fn local_only_generated_image_missing_catalog_fails_recovery_closed() {
    let harness = Harness::new(None);
    let (_, digest, _) = commit_generated_image(&harness, "local-missing-catalog", true);
    std::fs::remove_file(attachment_artifact_path(&harness, "catalog", &digest)).unwrap();

    let error = harness
        .engine
        .recover_pending_inbox(&harness.coordinator)
        .unwrap_err();

    assert!(matches!(error, MutationError::RecoveryConflict(_)));
}

#[test]
fn generated_image_missing_or_corrupt_blob_fails_recovery_closed() {
    for corrupt in [false, true] {
        let harness = Harness::new(None);
        let (_, digest, _) = commit_generated_image(
            &harness,
            if corrupt {
                "corrupt-generated-image-blob"
            } else {
                "missing-generated-image-blob"
            },
            false,
        );
        let blob_path = attachment_artifact_path(&harness, "blobs", &digest);
        if corrupt {
            std::fs::write(blob_path, b"corrupt blob bytes").unwrap();
        } else {
            std::fs::remove_file(blob_path).unwrap();
        }

        let error = harness
            .engine
            .recover_pending_inbox(&harness.coordinator)
            .unwrap_err();
        assert!(matches!(error, MutationError::RecoveryConflict(_)));
    }
}

#[test]
fn unreferenced_incomplete_remote_attachment_remains_nonfatal() {
    let first = Harness::new(None);
    let descriptor = first.descriptor_bytes();
    let second = Harness::new(Some(&descriptor));
    trust_each_other(&first, &second);
    let (_, digest, _) = commit_generated_image(&first, "unreferenced-incomplete", false);
    let (_, attachments) = split_generated_image_batch(&first);
    let manifest = {
        let state = first.engine.state.lock().unwrap();
        attachments
            .into_iter()
            .find(|bytes| {
                let envelope = EnvelopeV1::from_json_bytes(bytes).unwrap();
                matches!(
                    verify_envelope(&state, state.root_key.as_ref().unwrap(), &envelope)
                        .unwrap()
                        .operation()
                        .payload(),
                    OperationPayloadV1::AttachmentManifest(_)
                )
            })
            .unwrap()
    };

    second
        .engine
        .receive_envelopes(&second.coordinator, &[manifest])
        .unwrap();
    assert_eq!(
        second.engine.materialized_attachment(&digest).unwrap(),
        None
    );
    assert_eq!(
        second
            .engine
            .cataloged_image_expecting_scope(&second.identity.root_scope, &digest)
            .unwrap(),
        None
    );
    let (reopened, reopened_coordinator) = second.open_peer();
    reopened
        .recover_pending_inbox(&reopened_coordinator)
        .unwrap();
    assert_eq!(reopened.materialized_attachment(&digest).unwrap(), None);
}

#[test]
fn generic_non_square_png_remains_uncataloged_after_remote_reorder_and_reopen() {
    let first = Harness::new(None);
    let descriptor = first.descriptor_bytes();
    let second = Harness::new(Some(&descriptor));
    trust_each_other(&first, &second);
    let bytes = generic_non_square_png();
    let digest = first.engine.queue_attachment("image/png", &bytes).unwrap();
    let mut envelopes = first.engine.export_outbox().unwrap();
    envelopes.reverse();

    second
        .engine
        .receive_envelopes(&second.coordinator, &envelopes)
        .unwrap();
    assert_eq!(
        second.engine.materialized_attachment(&digest).unwrap(),
        Some(bytes.clone())
    );
    assert_eq!(
        second
            .engine
            .cataloged_image_expecting_scope(&second.identity.root_scope, &digest)
            .unwrap(),
        None
    );
    let (reopened, _coordinator) = second.open_peer();
    assert_eq!(
        reopened.materialized_attachment(&digest).unwrap(),
        Some(bytes)
    );
    assert_eq!(
        reopened
            .cataloged_image_expecting_scope(&second.identity.root_scope, &digest)
            .unwrap(),
        None
    );
}

#[test]
fn maximum_generated_image_stays_within_one_hundred_operation_sync_batch() {
    let manifest = AttachmentManifestV1::new(
        Digest32::from_bytes([0x5a; 32]),
        "image/png",
        crate::models::image_generation::MAX_GENERATED_IMAGE_BYTES,
    )
    .unwrap();
    let chunks = manifest.chunk_count() as usize;
    let grouped_operations = chunks + 1 + 1 + 2;

    assert_eq!(chunks, 96);
    assert_eq!(
        grouped_operations, 100,
        "96 chunks + manifest + note + 2 events"
    );
    assert!(grouped_operations < 128);
    assert!(
        crate::models::image_generation::MAX_GENERATED_IMAGE_BYTES * 2 < 64 * 1024 * 1024,
        "even a conservative 2x envelope bound stays under 64 MiB"
    );
}

#[test]
fn local_only_generated_image_keeps_catalog_but_emits_zero_outbox_operations() {
    let harness = Harness::new(None);
    let bytes = generated_png();
    let catalog = harness
        .engine
        .store_cataloged_image_expecting_scope(
            &harness.identity.root_scope,
            &bytes,
            Some("image/png"),
        )
        .unwrap();
    let digest = ContentDigest::parse(catalog.attachment_digest().unwrap().to_string()).unwrap();
    let _commit = harness
        .coordinator
        .commit_local(
            CausalStream::LocalOnly,
            SourceChannel::parse("image_generation").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "local-image-note.md",
                "---\nnote_id: local-image-note\ngrafyn_sync: local_only\n---\nimage",
            )],
            image_observation_drafts("local-image-note", &digest, true),
        )
        .unwrap();

    assert!(harness.engine.export_outbox().unwrap().is_empty());
    assert_eq!(
        harness
            .engine
            .materialized_attachment(&catalog.attachment_digest().unwrap())
            .unwrap()
            .unwrap(),
        bytes
    );
}

#[test]
fn cataloged_image_put_rejects_a_stale_or_cross_root_scope() {
    let harness = Harness::new(None);
    let wrong_scope = ContentDigest::parse("f".repeat(64)).unwrap();

    let error = harness
        .engine
        .store_cataloged_image_expecting_scope(&wrong_scope, &generated_png(), Some("image/png"))
        .unwrap_err();

    assert!(error.to_string().contains("scope"));
}

#[test]
fn two_open_engines_refresh_policy_heads_events_and_trusted_devices_before_writing() {
    let first = Harness::new(None);
    let (peer_engine, peer_coordinator) = first.open_peer();

    commit_note(
        &first,
        "private-a.md",
        "---\nnote_id: private-a\ngrafyn_sync: local_only\n---\nA",
    );
    let _ = peer_coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("note_editor").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "private-b.md",
                "---\nnote_id: private-b\ngrafyn_sync: local_only\n---\nB",
            )],
            vec![],
        )
        .unwrap();

    commit_note(&first, "shared.md", "---\nnote_id: shared-note\n---\nfirst");
    let _ = peer_coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("note_editor").unwrap(),
            vec![TargetMutation::put(
                TargetKind::Markdown,
                "shared.md",
                "---\nnote_id: shared-note\n---\nsecond",
            )],
            vec![],
        )
        .unwrap();

    let parent = first
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("companion_capture").unwrap(),
            vec![],
            vec![draft("cross-process-parent")],
        )
        .unwrap()
        .events
        .into_iter()
        .next()
        .unwrap();
    let mut child = draft("cross-process-child");
    child.causal_parents.push(parent.event_id.clone());
    let child_result = peer_coordinator.commit_local(
        CausalStream::SyncEligible,
        SourceChannel::parse("companion_capture").unwrap(),
        vec![],
        vec![child],
    );
    assert!(child_result.is_ok());

    let first_remote = DeviceSigningKey::from_seed([0xc1; 32]);
    let second_remote = DeviceSigningKey::from_seed([0xc2; 32]);
    first
        .engine
        .trust_device(
            DeviceId::parse_str(REMOTE_A).unwrap(),
            first_remote.public_key(),
        )
        .unwrap();
    peer_engine
        .trust_device(
            DeviceId::parse_str(REMOTE_B).unwrap(),
            second_remote.public_key(),
        )
        .unwrap();

    let (reopened, _) = first.open_peer();
    let state = reopened.state.lock().unwrap();
    assert_eq!(state.ledger.local_only_notes.len(), 2);
    assert!(state.ledger.local_only_notes.contains_key("private-a.md"));
    assert!(state.ledger.local_only_notes.contains_key("private-b.md"));
    assert!(state
        .trusted_devices
        .contains_key(&DeviceId::parse_str(REMOTE_A).unwrap()));
    assert!(state
        .trusted_devices
        .contains_key(&DeviceId::parse_str(REMOTE_B).unwrap()));
    assert!(state
        .event_operations
        .contains_key(parent.event_id.as_str()));
    assert_eq!(state.note_heads.get("shared-note").unwrap().len(), 1);
    drop(state);
    assert!(reopened.conflicts().unwrap().is_empty());
}

#[test]
fn local_rename_preserves_one_stable_note_history() {
    let harness = Harness::new(None);
    let first = "---\nnote_id: durable-note\n---\nfirst";
    let renamed = "---\nnote_id: durable-note\n---\nrenamed";
    commit_note(&harness, "before.md", first);

    let _ = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("note_editor").unwrap(),
            vec![
                TargetMutation::tombstone(TargetKind::Markdown, "before.md"),
                TargetMutation::put(TargetKind::Markdown, "after.md", renamed),
            ],
            vec![],
        )
        .unwrap();

    let state = harness.engine.state.lock().unwrap();
    assert_eq!(
        state.ledger.note_paths.get("durable-note").unwrap(),
        "after.md"
    );
    assert_eq!(state.note_heads.get("durable-note").unwrap().len(), 1);
    assert!(!state.note_heads.contains_key("before.md"));
    assert!(!state.note_heads.contains_key("after.md"));
    drop(state);
    assert_eq!(harness.engine.export_outbox().unwrap().len(), 2);
    assert!(!harness.vault.path().join("before.md").exists());
    assert_eq!(
        fs::read_to_string(harness.vault.path().join("after.md")).unwrap(),
        renamed
    );
}

#[test]
fn concurrent_rename_and_edit_conflict_on_the_same_stable_identity() {
    let first = Harness::new(None);
    let descriptor = first.descriptor_bytes();
    let second = Harness::new(Some(&descriptor));
    trust_each_other(&first, &second);
    commit_note(&first, "before.md", "---\nnote_id: durable-note\n---\nseed");
    assert_eq!(relay(&first, &second).applied, 1);

    let _ = first
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("note_editor").unwrap(),
            vec![
                TargetMutation::tombstone(TargetKind::Markdown, "before.md"),
                TargetMutation::put(
                    TargetKind::Markdown,
                    "after.md",
                    "---\nnote_id: durable-note\n---\nrenamed",
                ),
            ],
            vec![],
        )
        .unwrap();
    let second_relative = second
        .engine
        .state
        .lock()
        .unwrap()
        .ledger
        .note_paths
        .get("durable-note")
        .cloned()
        .unwrap();
    commit_note(
        &second,
        &second_relative,
        "---\nnote_id: durable-note\n---\nedited",
    );

    relay(&first, &second);
    relay(&second, &first);
    let first_conflicts = first.engine.conflicts().unwrap();
    let second_conflicts = second.engine.conflicts().unwrap();
    assert_eq!(first_conflicts.len(), 1);
    assert_eq!(first_conflicts, second_conflicts);
    assert_eq!(first_conflicts[0].note_key, "durable-note");
    assert_eq!(first_conflicts[0].head_ids.len(), 2);
}

#[test]
fn hostile_peer_note_identity_is_projected_to_a_safe_local_path() {
    let harness = Harness::new(None);
    let seed = [0xa8; 32];
    harness.trust(REMOTE_A, seed);
    let envelope = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "../outside",
        NoteRevisionKind::Put {
            markdown: "safe bytes".into(),
        },
        vec![],
    );

    let report = harness.receive(&[envelope]);

    assert_eq!(report.applied, 1);
    let relative = harness
        .engine
        .state
        .lock()
        .unwrap()
        .ledger
        .note_paths
        .get("../outside")
        .cloned()
        .unwrap();
    assert!(relative.starts_with("synced/"));
    assert!(!relative.contains(".."));
    assert_eq!(
        fs::read_to_string(harness.vault.path().join(relative)).unwrap(),
        "safe bytes"
    );
    assert!(!harness
        .vault
        .path()
        .parent()
        .unwrap()
        .join("outside")
        .exists());
}

#[test]
fn editing_a_plain_remote_note_keeps_its_signed_identity() {
    let harness = Harness::new(None);
    let seed = [0xaf; 32];
    harness.trust(REMOTE_A, seed);
    let envelope = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "plain-remote-note",
        NoteRevisionKind::Put {
            markdown: "plain remote bytes".into(),
        },
        vec![],
    );
    harness.receive(&[envelope]);
    let relative = projected_note_key(&harness, "plain-remote-note");
    let parser_default =
        crate::services::knowledge_store::default_note_identity_for_relative_path(&relative);

    commit_note(
        &harness,
        &relative,
        &format!("---\nnote_id: {parser_default}\n---\nlocally edited"),
    );

    let outbox = harness.engine.export_outbox().unwrap();
    assert_eq!(outbox.len(), 1);
    let state = harness.engine.state.lock().unwrap();
    let verified = verify_envelope(
        &state,
        state.root_key.as_ref().unwrap(),
        &EnvelopeV1::from_json_bytes(&outbox[0]).unwrap(),
    )
    .unwrap();
    let OperationPayloadV1::NoteRevision(revision) = verified.operation().payload() else {
        panic!("local edit must seal a note revision");
    };
    assert_eq!(revision.note_id(), "plain-remote-note");
    assert_eq!(
        state.ledger.note_paths.get("plain-remote-note").unwrap(),
        &relative
    );
}

#[test]
fn invalid_receive_batch_is_rejected_before_any_valid_operation_is_stored() {
    let harness = Harness::new(None);
    let seed = [0xaa; 32];
    harness.trust(REMOTE_A, seed);
    let valid = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "valid-note",
        NoteRevisionKind::Put {
            markdown: "valid".into(),
        },
        vec![],
    );
    let invalid = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "private-note",
        NoteRevisionKind::Put {
            markdown: "---\ngrafyn_sync: local_only\n---\nprivate".into(),
        },
        vec![],
    );
    let bytes = [valid, invalid]
        .into_iter()
        .map(|envelope| envelope.to_json().unwrap().into_bytes())
        .collect::<Vec<_>>();

    assert!(harness
        .engine
        .receive_envelopes(&harness.coordinator, &bytes)
        .is_err());
    assert!(harness
        .engine
        .state
        .lock()
        .unwrap()
        .operation_store
        .list(OperationArea::Inbox)
        .unwrap()
        .is_empty());
    assert_eq!(harness.engine.status().unwrap().pending_operations, 0);
}

#[test]
fn materialized_note_prefers_tombstone_then_larger_operation_id() {
    let revisions = [
        (
            id(9),
            NoteRevisionV1::put("note.md", "later put".into()).unwrap(),
        ),
        (id(1), NoteRevisionV1::tombstone("note.md").unwrap()),
        (id(8), NoteRevisionV1::tombstone("note.md").unwrap()),
    ]
    .into_iter()
    .collect();
    let heads = [id(9), id(1), id(8)].into_iter().collect();

    let (winner, revision) = choose_note_winner(&heads, &revisions).unwrap();

    assert_eq!(winner, id(8));
    assert!(matches!(revision.kind(), NoteRevisionKind::Tombstone));
}

#[test]
fn materialized_note_uses_operation_id_not_timestamp_for_equal_kinds() {
    let revisions = [
        (
            id(2),
            NoteRevisionV1::put("note.md", "first".into()).unwrap(),
        ),
        (
            id(7),
            NoteRevisionV1::put("note.md", "second".into()).unwrap(),
        ),
    ]
    .into_iter()
    .collect();
    let heads = [id(2), id(7)].into_iter().collect();

    let (winner, revision) = choose_note_winner(&heads, &revisions).unwrap();

    assert_eq!(winner, id(7));
    assert_eq!(
        revision.kind(),
        &NoteRevisionKind::Put {
            markdown: "second".into()
        }
    );
}

#[test]
fn event_dependencies_include_causal_semantic_and_evidence_links() {
    let harness = Harness::new(None);
    let mut event = harness
        .coordinator
        .commit_local(
            CausalStream::LocalOnly,
            SourceChannel::parse("companion_capture").unwrap(),
            vec![],
            vec![draft("dependency-collector")],
        )
        .unwrap()
        .events
        .into_iter()
        .next()
        .unwrap();
    let causal = EventId::parse("11".repeat(32)).unwrap();
    let superseded = EventId::parse("22".repeat(32)).unwrap();
    let reinforced = EventId::parse("33".repeat(32)).unwrap();
    let evidence = EventId::parse("44".repeat(32)).unwrap();
    let relationship = EventId::parse("55".repeat(32)).unwrap();
    event.causal_parents = vec![causal.clone()];
    event.supersedes = vec![superseded.clone()];
    event.reinforces = vec![reinforced.clone()];
    event.evidence = vec![EvidenceRef {
        evidence_type: EvidenceType::Event,
        source_id: crate::models::twin_event::Identifier::parse(evidence.as_str()).unwrap(),
        digest: None,
    }];
    event.context.relationships = vec![RelationshipAssertion {
        subject_id: EntityId::parse("owner").unwrap(),
        predicate: RelationshipPredicate::parse("works_with").unwrap(),
        object_id: EntityId::parse("person-1").unwrap(),
        direction: RelationshipDirection::Directed,
        valid_from: None,
        valid_to: None,
        evidence: vec![EvidenceRef {
            evidence_type: EvidenceType::Event,
            source_id: crate::models::twin_event::Identifier::parse(relationship.as_str()).unwrap(),
            digest: None,
        }],
        governance: Governance::direct_observation(),
    }];

    assert_eq!(
        event_dependency_ids(&event).unwrap(),
        vec![causal, superseded, reinforced, evidence, relationship]
    );
}

#[test]
fn semantic_event_child_before_parent_is_deferred_then_materialized() {
    let harness = Harness::new(None);
    let parent_seed = [0xa7; 32];
    let child_seed = [0xb7; 32];
    harness.trust(REMOTE_A, parent_seed);
    harness.trust(REMOTE_B, child_seed);
    let mut parent = harness
        .coordinator
        .commit_local(
            CausalStream::LocalOnly,
            SourceChannel::parse("companion_capture").unwrap(),
            vec![],
            vec![draft("semantic-parent")],
        )
        .unwrap()
        .events
        .into_iter()
        .next()
        .unwrap();
    parent.device_id = crate::models::twin_event::DeviceId::parse(REMOTE_A).unwrap();
    parent.causal_stream = CausalStream::SyncEligible;
    parent.causal_parents.clear();
    parent.device_sequence = 1;
    parent.event_id = crate::services::twin_events::derive_event_id(&parent);
    let parent_envelope = event_envelope(&harness, REMOTE_A, parent_seed, &parent, vec![]);

    let mut child = harness
        .coordinator
        .commit_local(
            CausalStream::LocalOnly,
            SourceChannel::parse("companion_capture").unwrap(),
            vec![],
            vec![draft("semantic-child")],
        )
        .unwrap()
        .events
        .into_iter()
        .next()
        .unwrap();
    child.device_id = crate::models::twin_event::DeviceId::parse(REMOTE_B).unwrap();
    child.causal_stream = CausalStream::SyncEligible;
    child.causal_parents.clear();
    child.device_sequence = 1;
    child.supersedes = vec![parent.event_id.clone()];
    child.event_id = crate::services::twin_events::derive_event_id(&child);
    let child_envelope = event_envelope(
        &harness,
        REMOTE_B,
        child_seed,
        &child,
        vec![*parent_envelope.operation_id()],
    );

    let first = harness.receive(&[child_envelope]);
    assert_eq!(first.applied, 0);
    assert_eq!(first.deferred, 1);

    let second = harness.receive(&[parent_envelope]);
    assert_eq!(second.applied, 2);
    assert_eq!(second.deferred, 0);
    let events = harness.engine.event_store.ordered_events().unwrap();
    assert!(events
        .iter()
        .any(|candidate| candidate.event_id == parent.event_id));
    assert!(events
        .iter()
        .any(|candidate| candidate.event_id == child.event_id));
}

#[test]
fn terminally_invalid_event_is_quarantined_and_does_not_poison_reopen() {
    let harness = Harness::new(None);
    let parent_seed = [0xa9; 32];
    let child_seed = [0xb9; 32];
    harness.trust(REMOTE_A, parent_seed);
    harness.trust(REMOTE_B, child_seed);
    let mut parent = harness
        .coordinator
        .commit_local(
            CausalStream::LocalOnly,
            SourceChannel::parse("companion_capture").unwrap(),
            vec![],
            vec![draft("quarantine-parent")],
        )
        .unwrap()
        .events
        .into_iter()
        .next()
        .unwrap();
    parent.device_id = crate::models::twin_event::DeviceId::parse(REMOTE_A).unwrap();
    parent.causal_stream = CausalStream::SyncEligible;
    parent.causal_parents.clear();
    parent.device_sequence = 1;
    parent.event_id = crate::services::twin_events::derive_event_id(&parent);
    let parent_envelope = event_envelope(&harness, REMOTE_A, parent_seed, &parent, vec![]);

    let mut invalid = harness
        .coordinator
        .commit_local(
            CausalStream::LocalOnly,
            SourceChannel::parse("companion_capture").unwrap(),
            vec![],
            vec![draft("quarantine-child")],
        )
        .unwrap()
        .events
        .into_iter()
        .next()
        .unwrap();
    invalid.device_id = crate::models::twin_event::DeviceId::parse(REMOTE_B).unwrap();
    invalid.causal_stream = CausalStream::SyncEligible;
    invalid.causal_parents.clear();
    invalid.device_sequence = 1;
    invalid.supersedes = vec![EventId::parse("66".repeat(32)).unwrap()];
    invalid.event_id = crate::services::twin_events::derive_event_id(&invalid);
    let invalid_envelope = event_envelope(
        &harness,
        REMOTE_B,
        child_seed,
        &invalid,
        vec![*parent_envelope.operation_id()],
    );
    let invalid_operation_id = *invalid_envelope.operation_id();

    let first = harness.receive(&[invalid_envelope]);
    assert_eq!(first.received, 1);
    assert_eq!(first.applied, 0);
    assert_eq!(first.deferred, 1);

    let report = harness.receive(&[parent_envelope]);

    assert_eq!(report.received, 1);
    assert_eq!(report.applied, 1);
    assert_eq!(report.deferred, 0);
    assert!(harness
        .engine
        .state
        .lock()
        .unwrap()
        .ledger
        .rejected_operations
        .contains(&invalid_operation_id.to_string()));
    assert!(!harness
        .engine
        .event_store
        .ordered_events()
        .unwrap()
        .iter()
        .any(|candidate| candidate.event_id == invalid.event_id));

    let (reopened, _) = harness.open_peer();
    assert_eq!(reopened.status().unwrap().pending_operations, 0);
    assert!(reopened
        .state
        .lock()
        .unwrap()
        .ledger
        .rejected_operations
        .contains(&invalid_operation_id.to_string()));
}

#[test]
fn child_before_parent_is_deferred_then_materialized_without_outbox_echo() {
    let harness = Harness::new(None);
    let seed = [0xa1; 32];
    harness.trust(REMOTE_A, seed);
    let parent = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "ideas.md",
        NoteRevisionKind::Put {
            markdown: "parent".into(),
        },
        vec![],
    );
    let child = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "ideas.md",
        NoteRevisionKind::Put {
            markdown: "child".into(),
        },
        vec![*parent.operation_id()],
    );

    let first = harness.receive(&[child]);
    assert_eq!(first.applied, 0);
    assert_eq!(first.deferred, 1);
    assert!(!harness.vault.path().join("ideas.md").exists());

    let second = harness.receive(&[parent]);
    assert_eq!(second.applied, 2);
    assert_eq!(second.deferred, 0);
    assert_eq!(
        second.authority_token,
        Some(harness.coordinator.current_authority_token().unwrap())
    );
    assert_eq!(
        fs::read_to_string(projected_note_path(&harness, "ideas.md")).unwrap(),
        "child"
    );
    let status = harness.engine.status().unwrap();
    assert_eq!(status.outbox_operations, 0);
    assert_eq!(status.pending_operations, 0);
}

#[test]
fn applied_note_projection_is_repaired_when_no_inbox_operation_is_ready() {
    let harness = Harness::new(None);
    let seed = [0xb2; 32];
    harness.trust(REMOTE_A, seed);
    let envelope = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "repair-note",
        NoteRevisionKind::Put {
            markdown: "---\nnote_id: repair-note\n---\nrepaired".into(),
        },
        vec![],
    );
    let operation_id = *envelope.operation_id();
    {
        let state = harness.engine.state.lock().unwrap();
        state
            .operation_store
            .receive_batch(std::slice::from_ref(&envelope))
            .unwrap();
        state.operation_store.mark_applied(&operation_id).unwrap();
    }
    assert!(!harness
        .engine
        .state
        .lock()
        .unwrap()
        .ledger
        .note_paths
        .contains_key("repair-note"));

    let report = harness
        .engine
        .recover_pending_inbox(&harness.coordinator)
        .unwrap();

    assert_eq!(report.applied, 0);
    assert!(
        fs::read_to_string(projected_note_path(&harness, "repair-note"))
            .unwrap()
            .ends_with("repaired")
    );
}

#[test]
fn concurrent_remote_drains_leave_disk_at_the_deterministic_note_winner() {
    let harness = Harness::new(None);
    let seed_a = [0xb3; 32];
    let seed_b = [0xb4; 32];
    harness.trust(REMOTE_A, seed_a);
    harness.trust(REMOTE_B, seed_b);
    let (peer, peer_coordinator) = harness.open_peer();
    let put = note_envelope(
        &harness,
        REMOTE_A,
        seed_a,
        "racing-note",
        NoteRevisionKind::Put {
            markdown: "---\nnote_id: racing-note\n---\nstale put".into(),
        },
        vec![],
    );
    let tombstone = note_envelope(
        &harness,
        REMOTE_B,
        seed_b,
        "racing-note",
        NoteRevisionKind::Tombstone,
        vec![],
    );
    {
        let state = harness.engine.state.lock().unwrap();
        state
            .operation_store
            .receive_batch(&[put, tombstone])
            .unwrap();
    }

    std::thread::scope(|scope| {
        let first = scope.spawn(|| harness.engine.recover_pending_inbox(&harness.coordinator));
        let second = scope.spawn(|| peer.recover_pending_inbox(&peer_coordinator));
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
    });

    assert!(!projected_note_path(&harness, "racing-note").exists());
    assert_eq!(harness.engine.conflicts().unwrap().len(), 1);
    assert_eq!(harness.engine.status().unwrap().pending_operations, 0);
    assert_eq!(peer.status().unwrap().pending_operations, 0);
}

#[test]
fn concurrent_two_device_notes_converge_independent_of_delivery_order() {
    let first = Harness::new(None);
    let descriptor = first.descriptor_bytes();
    let second = Harness::new(Some(&descriptor));
    let seed_a = [0xa2; 32];
    let seed_b = [0xb2; 32];
    for harness in [&first, &second] {
        harness.trust(REMOTE_A, seed_a);
        harness.trust(REMOTE_B, seed_b);
    }
    let left = note_envelope(
        &first,
        REMOTE_A,
        seed_a,
        "shared.md",
        NoteRevisionKind::Put {
            markdown: "left".into(),
        },
        vec![],
    );
    let right = note_envelope(
        &first,
        REMOTE_B,
        seed_b,
        "shared.md",
        NoteRevisionKind::Put {
            markdown: "right".into(),
        },
        vec![],
    );
    let expected = if left.operation_id() > right.operation_id() {
        "left"
    } else {
        "right"
    };

    first.receive(&[left.clone(), right.clone()]);
    second.receive(&[right, left]);

    assert_eq!(
        fs::read_to_string(projected_note_path(&first, "shared.md")).unwrap(),
        expected
    );
    assert_eq!(
        fs::read_to_string(projected_note_path(&second, "shared.md")).unwrap(),
        expected
    );
    assert_eq!(first.engine.conflicts().unwrap().len(), 1);
    assert_eq!(second.engine.conflicts().unwrap().len(), 1);
    assert_eq!(first.engine.status().unwrap().outbox_operations, 0);
    assert_eq!(second.engine.status().unwrap().outbox_operations, 0);
}

#[test]
fn two_real_devices_create_update_resolve_delete_and_never_echo_remote_operations() {
    let first = Harness::new(None);
    let descriptor = first.descriptor_bytes();
    let second = Harness::new(Some(&descriptor));
    trust_each_other(&first, &second);

    commit_note(&first, "shared.md", "---\nnote_id: shared-note\n---\nseed");
    assert_eq!(relay(&first, &second).applied, 1);
    assert!(
        fs::read_to_string(projected_note_path(&second, "shared-note"))
            .unwrap()
            .ends_with("seed")
    );

    commit_note(&first, "shared.md", "---\nnote_id: shared-note\n---\nleft");
    let second_key = projected_note_key(&second, "shared-note");
    commit_note(
        &second,
        &second_key,
        "---\nnote_id: shared-note\n---\nright",
    );
    relay(&first, &second);
    relay(&second, &first);
    let first_bytes = fs::read(first.vault.path().join("shared.md")).unwrap();
    let second_bytes = fs::read(projected_note_path(&second, "shared-note")).unwrap();
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first.engine.conflicts().unwrap().len(), 1);
    assert_eq!(second.engine.conflicts().unwrap().len(), 1);

    commit_note(
        &first,
        "shared.md",
        "---\nnote_id: shared-note\n---\nresolved",
    );
    let resolution = relay(&first, &second);
    assert!(resolution.duplicates >= 2);
    assert_eq!(resolution.applied, 1);
    assert!(
        fs::read_to_string(projected_note_path(&second, "shared-note"))
            .unwrap()
            .ends_with("resolved")
    );
    assert!(first.engine.conflicts().unwrap().is_empty());
    assert!(second.engine.conflicts().unwrap().is_empty());

    delete_note(&second, &second_key);
    assert_eq!(relay(&second, &first).applied, 1);
    assert!(!first.vault.path().join("shared.md").exists());
    assert!(!second.vault.path().join(&second_key).exists());
    assert_eq!(first.engine.status().unwrap().outbox_operations, 3);
    assert_eq!(second.engine.status().unwrap().outbox_operations, 2);
    assert_eq!(relay(&first, &second).applied, 0);
    assert_eq!(relay(&second, &first).applied, 0);
    assert_eq!(first.engine.status().unwrap().outbox_operations, 3);
    assert_eq!(second.engine.status().unwrap().outbox_operations, 2);
}

#[test]
fn two_real_devices_converge_on_exact_immutable_events_without_echo() {
    let first = Harness::new(None);
    let descriptor = first.descriptor_bytes();
    let second = Harness::new(Some(&descriptor));
    trust_each_other(&first, &second);
    let committed = first
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("companion_capture").unwrap(),
            vec![],
            vec![draft("shared-event-note")],
        )
        .unwrap();
    assert_eq!(committed.events.len(), 1);

    let report = relay(&first, &second);

    assert_eq!(report.applied, 1);
    assert!(report.authority_token.is_some());
    assert_eq!(
        first.engine.event_store.ordered_events().unwrap(),
        second.engine.event_store.ordered_events().unwrap()
    );
    assert_eq!(first.engine.status().unwrap().outbox_operations, 1);
    assert_eq!(second.engine.status().unwrap().outbox_operations, 0);
    assert_eq!(relay(&first, &second).applied, 0);
    assert_eq!(second.engine.status().unwrap().outbox_operations, 0);
}

#[test]
fn encrypted_attachment_converges_after_reordered_duplicates_without_partial_output() {
    let first = Harness::new(None);
    let descriptor = first.descriptor_bytes();
    let second = Harness::new(Some(&descriptor));
    trust_each_other(&first, &second);
    let mut bytes = vec![0x4a; ATTACHMENT_CHUNK_BYTES * 2 + 9];
    bytes[ATTACHMENT_CHUNK_BYTES] = 0x7c;
    bytes[ATTACHMENT_CHUNK_BYTES * 2] = 0x2d;
    let digest = first
        .engine
        .queue_attachment("application/octet-stream", &bytes)
        .unwrap();
    let mut envelopes = first.engine.export_outbox().unwrap();
    assert_eq!(envelopes.len(), 4);
    envelopes.reverse();

    for (index, envelope) in envelopes.iter().enumerate() {
        let report = second
            .engine
            .receive_envelopes(&second.coordinator, std::slice::from_ref(envelope))
            .unwrap();
        assert_eq!(report.received, 1);
        if index + 1 < envelopes.len() {
            assert_eq!(
                second.engine.materialized_attachment(&digest).unwrap(),
                None
            );
        }
        let duplicate = second
            .engine
            .receive_envelopes(&second.coordinator, std::slice::from_ref(envelope))
            .unwrap();
        assert_eq!(duplicate.duplicates, 1);
    }

    assert_eq!(
        second.engine.materialized_attachment(&digest).unwrap(),
        Some(bytes)
    );
    assert_eq!(second.engine.status().unwrap().outbox_operations, 0);
}

#[test]
fn remote_image_mime_without_generated_provenance_remains_a_generic_attachment() {
    let harness = Harness::new(None);
    let seed = [0xac; 32];
    harness.trust(REMOTE_A, seed);
    let bytes = b"not a decoded raster";
    let digest = crate::models::attachment::sha256_attachment_digest(bytes);
    let manifest = AttachmentManifestV1::new(digest, "image/png", bytes.len()).unwrap();
    let manifest_operation = OperationV1::new(
        1_800_000_000_012,
        vec![],
        OperationPayloadV1::AttachmentManifest(manifest.clone()),
    )
    .unwrap();
    let signing_key = DeviceSigningKey::from_seed(seed);
    let device_id = DeviceId::parse_str(REMOTE_A).unwrap();
    let manifest_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &manifest_operation,
    )
    .unwrap();
    let chunk = AttachmentChunkV1::new(
        *manifest_envelope.operation_id(),
        digest,
        0,
        1,
        bytes.to_vec(),
    )
    .unwrap();
    let chunk_operation = OperationV1::new(
        1_800_000_000_013,
        vec![*manifest_envelope.operation_id()],
        OperationPayloadV1::AttachmentChunk(chunk),
    )
    .unwrap();
    let chunk_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &chunk_operation,
    )
    .unwrap();
    let report = harness.receive(&[manifest_envelope, chunk_envelope]);

    assert_eq!(report.applied, 2);
    assert_eq!(
        harness.engine.materialized_attachment(&digest).unwrap(),
        Some(bytes.to_vec())
    );
    assert_eq!(
        harness
            .engine
            .cataloged_image_expecting_scope(&harness.identity.root_scope, &digest)
            .unwrap(),
        None
    );
    assert!(harness
        .engine
        .state
        .lock()
        .unwrap()
        .ledger
        .rejected_operations
        .is_empty());
}

#[test]
fn provenance_qualified_non_raster_is_rejected_without_catalog_or_recovery_wedge() {
    use crate::services::twin_events::{EventGroupFinalizer, StoreEventGroupFinalizer};

    let harness = Harness::new(None);
    let seed = [0xad; 32];
    harness.trust(REMOTE_A, seed);
    let bytes = b"not a decoded generated raster";
    let digest = crate::models::attachment::sha256_attachment_digest(bytes);
    let content_digest = ContentDigest::parse(digest.to_string()).unwrap();
    let mut drafts = image_observation_drafts("invalid-remote-generated", &content_digest, false);
    for draft in &mut drafts {
        draft.context.source_channel = SourceChannel::parse("image_generation").unwrap();
    }
    let finalizer_data = tempfile::tempdir().unwrap();
    let finalizer_store = Arc::new(TwinEventStore::new(finalizer_data.path()));
    finalizer_store.initialize().unwrap();
    let finalizer = StoreEventGroupFinalizer::new(
        finalizer_store,
        Arc::new(FixedRemoteEventIdentity {
            device_id: crate::models::twin_event::DeviceId::parse(REMOTE_A).unwrap(),
        }),
    );
    let events = finalizer
        .finalize(CausalStream::SyncEligible, &drafts)
        .unwrap();
    assert_eq!(events.len(), 2);
    let note_event = event_envelope(&harness, REMOTE_A, seed, &events[0], vec![]);
    let observation_event = event_envelope(
        &harness,
        REMOTE_A,
        seed,
        &events[1],
        vec![*note_event.operation_id()],
    );
    let error = harness
        .engine
        .receive_envelopes(
            &harness.coordinator,
            &[
                note_event.to_json().unwrap().into_bytes(),
                observation_event.to_json().unwrap().into_bytes(),
            ],
        )
        .unwrap_err();
    assert!(matches!(error, MutationError::RecoveryConflict(_)));

    let manifest = AttachmentManifestV1::new(digest, "image/png", bytes.len()).unwrap();
    let manifest_operation = OperationV1::new(
        1_800_000_000_014,
        vec![],
        OperationPayloadV1::AttachmentManifest(manifest.clone()),
    )
    .unwrap();
    let signing_key = DeviceSigningKey::from_seed(seed);
    let device_id = DeviceId::parse_str(REMOTE_A).unwrap();
    let manifest_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &manifest_operation,
    )
    .unwrap();
    let chunk = AttachmentChunkV1::new(
        *manifest_envelope.operation_id(),
        digest,
        0,
        1,
        bytes.to_vec(),
    )
    .unwrap();
    let chunk_operation = OperationV1::new(
        1_800_000_000_015,
        vec![*manifest_envelope.operation_id()],
        OperationPayloadV1::AttachmentChunk(chunk),
    )
    .unwrap();
    let chunk_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &chunk_operation,
    )
    .unwrap();
    let manifest_id = *manifest_envelope.operation_id();
    let chunk_id = *chunk_envelope.operation_id();

    let report = harness.receive(&[manifest_envelope, chunk_envelope]);
    assert_eq!(report.applied, 2);
    assert_eq!(
        harness
            .engine
            .cataloged_image_expecting_scope(&harness.identity.root_scope, &digest)
            .unwrap(),
        None
    );
    let rejected = harness
        .engine
        .state
        .lock()
        .unwrap()
        .ledger
        .rejected_operations
        .clone();
    assert!(rejected.contains(&manifest_id.to_string()));
    assert!(rejected.contains(&chunk_id.to_string()));
    {
        let mut state = harness.engine.state.lock().unwrap();
        assert!(applied_manifest_has_complete_chunks(
            &state,
            manifest_id,
            &manifest,
            true,
        ));
        state.applied.remove(&chunk_id);
        assert!(!applied_manifest_has_complete_chunks(
            &state,
            manifest_id,
            &manifest,
            true,
        ));
        state.applied.insert(chunk_id);
    }

    let (reopened, reopened_coordinator) = harness.open_peer();
    reopened
        .recover_pending_inbox(&reopened_coordinator)
        .unwrap();
    assert_eq!(
        reopened
            .cataloged_image_expecting_scope(&harness.identity.root_scope, &digest)
            .unwrap(),
        None
    );
    let blob_path = attachment_artifact_path(&harness, "blobs", &digest);
    let valid_blob = fs::read(&blob_path).unwrap();
    fs::remove_file(&blob_path).unwrap();
    assert!(matches!(
        reopened.recover_pending_inbox(&reopened_coordinator),
        Err(MutationError::RecoveryConflict(_))
    ));
    fs::write(&blob_path, b"corrupt quarantined blob").unwrap();
    assert!(matches!(
        reopened.recover_pending_inbox(&reopened_coordinator),
        Err(MutationError::RecoveryConflict(_))
    ));
    fs::write(blob_path, valid_blob).unwrap();
    reopened
        .recover_pending_inbox(&reopened_coordinator)
        .unwrap();
}

#[test]
fn full_digest_mismatch_quarantines_the_complete_attachment_operation_set() {
    let harness = Harness::new(None);
    let seed = [0xab; 32];
    harness.trust(REMOTE_A, seed);
    let expected = b"expected contents";
    let wrong = b"tampered contents";
    assert_eq!(expected.len(), wrong.len());
    let digest = crate::models::attachment::sha256_attachment_digest(expected);
    let manifest = AttachmentManifestV1::new(digest, "image/png", expected.len()).unwrap();
    let manifest_operation = OperationV1::new(
        1_800_000_000_010,
        vec![],
        OperationPayloadV1::AttachmentManifest(manifest.clone()),
    )
    .unwrap();
    let signing_key = DeviceSigningKey::from_seed(seed);
    let device_id = DeviceId::parse_str(REMOTE_A).unwrap();
    let manifest_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &manifest_operation,
    )
    .unwrap();
    let chunk = AttachmentChunkV1::new(
        *manifest_envelope.operation_id(),
        digest,
        0,
        1,
        wrong.to_vec(),
    )
    .unwrap();
    let chunk_operation = OperationV1::new(
        1_800_000_000_011,
        vec![*manifest_envelope.operation_id()],
        OperationPayloadV1::AttachmentChunk(chunk),
    )
    .unwrap();
    let chunk_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &chunk_operation,
    )
    .unwrap();
    let manifest_id = *manifest_envelope.operation_id();
    let chunk_id = *chunk_envelope.operation_id();

    let report = harness.receive(&[manifest_envelope, chunk_envelope]);

    assert_eq!(report.applied, 1);
    assert_eq!(report.deferred, 0);
    assert_eq!(
        harness.engine.materialized_attachment(&digest).unwrap(),
        None
    );
    let rejected = harness
        .engine
        .state
        .lock()
        .unwrap()
        .ledger
        .rejected_operations
        .clone();
    assert!(rejected.contains(&manifest_id.to_string()));
    assert!(rejected.contains(&chunk_id.to_string()));
    let (reopened, _) = harness.open_peer();
    assert_eq!(reopened.status().unwrap().pending_operations, 0);
    assert_eq!(reopened.materialized_attachment(&digest).unwrap(), None);
}

#[test]
fn quarantined_duplicate_cannot_block_a_fresh_valid_operation() {
    let harness = Harness::new(None);
    let seed = [0xae; 32];
    harness.trust(REMOTE_A, seed);
    let expected = b"expected contents";
    let wrong = b"tampered contents";
    let digest = crate::models::attachment::sha256_attachment_digest(expected);
    let manifest = AttachmentManifestV1::new(digest, "image/png", expected.len()).unwrap();
    let manifest_operation = OperationV1::new(
        1_800_000_000_017,
        vec![],
        OperationPayloadV1::AttachmentManifest(manifest),
    )
    .unwrap();
    let signing_key = DeviceSigningKey::from_seed(seed);
    let device_id = DeviceId::parse_str(REMOTE_A).unwrap();
    let manifest_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &manifest_operation,
    )
    .unwrap();
    let chunk = AttachmentChunkV1::new(
        *manifest_envelope.operation_id(),
        digest,
        0,
        1,
        wrong.to_vec(),
    )
    .unwrap();
    let chunk_operation = OperationV1::new(
        1_800_000_000_018,
        vec![*manifest_envelope.operation_id()],
        OperationPayloadV1::AttachmentChunk(chunk),
    )
    .unwrap();
    let chunk_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &chunk_operation,
    )
    .unwrap();
    harness.receive(&[manifest_envelope, chunk_envelope.clone()]);
    let fresh = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "fresh-note",
        NoteRevisionKind::Put {
            markdown: "---\nnote_id: fresh-note\n---\nfresh".into(),
        },
        vec![],
    );
    let batch = [chunk_envelope, fresh]
        .iter()
        .map(|envelope| envelope.to_json().unwrap().into_bytes())
        .collect::<Vec<_>>();

    let report = harness
        .engine
        .receive_envelopes(&harness.coordinator, &batch)
        .unwrap();

    assert_eq!(report.duplicates, 1);
    assert_eq!(report.received, 1);
    assert_eq!(report.applied, 1);
    assert!(
        fs::read_to_string(projected_note_path(&harness, "fresh-note"))
            .unwrap()
            .ends_with("fresh")
    );
}

#[test]
fn persisted_bad_chunk_before_quarantine_does_not_poison_reopen() {
    let harness = Harness::new(None);
    let seed = [0xac; 32];
    harness.trust(REMOTE_A, seed);
    let expected = b"expected contents";
    let wrong = b"tampered contents";
    assert_eq!(expected.len(), wrong.len());
    let digest = crate::models::attachment::sha256_attachment_digest(expected);
    let manifest = AttachmentManifestV1::new(digest, "image/png", expected.len()).unwrap();
    let manifest_operation = OperationV1::new(
        1_800_000_000_012,
        vec![],
        OperationPayloadV1::AttachmentManifest(manifest.clone()),
    )
    .unwrap();
    let signing_key = DeviceSigningKey::from_seed(seed);
    let device_id = DeviceId::parse_str(REMOTE_A).unwrap();
    let manifest_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &manifest_operation,
    )
    .unwrap();
    let chunk = AttachmentChunkV1::new(
        *manifest_envelope.operation_id(),
        digest,
        0,
        1,
        wrong.to_vec(),
    )
    .unwrap();
    let chunk_operation = OperationV1::new(
        1_800_000_000_013,
        vec![*manifest_envelope.operation_id()],
        OperationPayloadV1::AttachmentChunk(chunk.clone()),
    )
    .unwrap();
    let chunk_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &chunk_operation,
    )
    .unwrap();
    let manifest_id = *manifest_envelope.operation_id();
    let chunk_id = *chunk_envelope.operation_id();

    {
        let state = harness.engine.state.lock().unwrap();
        state
            .operation_store
            .receive_batch(&[manifest_envelope, chunk_envelope])
            .unwrap();
        state.operation_store.mark_applied(&manifest_id).unwrap();
        state
            .attachment_store
            .ingest_manifest(manifest_id, &manifest)
            .unwrap();
        let error = state.attachment_store.ingest_chunk(&chunk).unwrap_err();
        assert!(error
            .to_string()
            .contains("full digest does not match its manifest"));
    }

    let (reopened, coordinator) = harness.open_peer();
    let report = reopened.recover_pending_inbox(&coordinator).unwrap();

    assert_eq!(report.applied, 0);
    assert_eq!(report.deferred, 0);
    assert_eq!(reopened.materialized_attachment(&digest).unwrap(), None);
    let rejected = reopened
        .state
        .lock()
        .unwrap()
        .ledger
        .rejected_operations
        .clone();
    assert!(rejected.contains(&manifest_id.to_string()));
    assert!(rejected.contains(&chunk_id.to_string()));
    assert_eq!(reopened.status().unwrap().pending_operations, 0);
}

#[test]
fn durable_quarantine_repairs_missing_applied_markers_on_reopen() {
    let harness = Harness::new(None);
    let seed = [0xad; 32];
    harness.trust(REMOTE_A, seed);
    let digest = crate::models::attachment::sha256_attachment_digest(b"expected");
    let manifest = AttachmentManifestV1::new(digest, "image/png", 8).unwrap();
    let manifest_operation = OperationV1::new(
        1_800_000_000_014,
        vec![],
        OperationPayloadV1::AttachmentManifest(manifest),
    )
    .unwrap();
    let signing_key = DeviceSigningKey::from_seed(seed);
    let device_id = DeviceId::parse_str(REMOTE_A).unwrap();
    let manifest_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &manifest_operation,
    )
    .unwrap();
    let chunk = AttachmentChunkV1::new(
        *manifest_envelope.operation_id(),
        digest,
        0,
        1,
        b"tampered".to_vec(),
    )
    .unwrap();
    let chunk_operation = OperationV1::new(
        1_800_000_000_015,
        vec![*manifest_envelope.operation_id()],
        OperationPayloadV1::AttachmentChunk(chunk),
    )
    .unwrap();
    let chunk_envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &device_id,
        &signing_key,
        &chunk_operation,
    )
    .unwrap();
    let manifest_id = *manifest_envelope.operation_id();
    let chunk_id = *chunk_envelope.operation_id();
    {
        let mut state = harness.engine.state.lock().unwrap();
        state
            .operation_store
            .receive_batch(&[manifest_envelope, chunk_envelope])
            .unwrap();
        state
            .ledger
            .rejected_operations
            .extend([manifest_id.to_string(), chunk_id.to_string()]);
        persist_ledger(
            &harness.engine.data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )
        .unwrap();
    }

    let (reopened, _) = harness.open_peer();

    assert_eq!(reopened.status().unwrap().pending_operations, 0);
    let applied = reopened
        .state
        .lock()
        .unwrap()
        .operation_store
        .list_applied_ids()
        .unwrap();
    assert!(applied.contains(&manifest_id));
    assert!(applied.contains(&chunk_id));
}

#[test]
fn rejected_parent_transitively_quarantines_a_partially_deferred_child() {
    let harness = Harness::new(None);
    let seed = [0xb1; 32];
    harness.trust(REMOTE_A, seed);
    let parent = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "quarantined-history",
        NoteRevisionKind::Put {
            markdown: "---\nnote_id: quarantined-history\n---\nparent".into(),
        },
        vec![],
    );
    let parent_id = *parent.operation_id();
    let missing = OperationId::parse_hex(&"88".repeat(32)).unwrap();
    let mut child_parents = vec![parent_id, missing];
    child_parents.sort();
    let child = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "quarantined-history",
        NoteRevisionKind::Put {
            markdown: "---\nnote_id: quarantined-history\n---\nchild".into(),
        },
        child_parents,
    );
    let child_id = *child.operation_id();
    {
        let mut state = harness.engine.state.lock().unwrap();
        state
            .operation_store
            .receive_batch(&[parent, child])
            .unwrap();
        state.operation_store.mark_applied(&parent_id).unwrap();
        state
            .ledger
            .rejected_operations
            .insert(parent_id.to_string());
        persist_ledger(
            &harness.engine.data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )
        .unwrap();
    }

    let (reopened, _) = harness.open_peer();

    assert_eq!(reopened.status().unwrap().pending_operations, 0);
    let state = reopened.state.lock().unwrap();
    assert!(state
        .ledger
        .rejected_operations
        .contains(&child_id.to_string()));
    assert!(state
        .operation_store
        .list_applied_ids()
        .unwrap()
        .contains(&child_id));
}

#[test]
fn remote_tombstone_wins_over_a_concurrent_put() {
    let harness = Harness::new(None);
    let seed_a = [0xa3; 32];
    let seed_b = [0xb3; 32];
    harness.trust(REMOTE_A, seed_a);
    harness.trust(REMOTE_B, seed_b);
    let put = note_envelope(
        &harness,
        REMOTE_A,
        seed_a,
        "removed.md",
        NoteRevisionKind::Put {
            markdown: "content".into(),
        },
        vec![],
    );
    let tombstone = note_envelope(
        &harness,
        REMOTE_B,
        seed_b,
        "removed.md",
        NoteRevisionKind::Tombstone,
        vec![],
    );

    harness.receive(&[put, tombstone]);

    assert!(!projected_note_path(&harness, "removed.md").exists());
    assert_eq!(harness.engine.conflicts().unwrap().len(), 1);
}

#[test]
fn local_only_policy_blocks_remote_overwrite_and_incoming_policy_relaxation() {
    let harness = Harness::new(None);
    let seed = [0xa4; 32];
    harness.trust(REMOTE_A, seed);
    commit_note(
        &harness,
        "private.md",
        "---\ngrafyn_sync: local_only\nnote_id: private-note\n---\nprivate",
    );
    let overwrite = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "private-note",
        NoteRevisionKind::Put {
            markdown: "remote shared value".into(),
        },
        vec![],
    );
    let improper_local_only = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "leaked-note",
        NoteRevisionKind::Put {
            markdown: "---\ngrafyn_sync: local_only\n---\nsecret".into(),
        },
        vec![],
    );

    for envelope in [overwrite, improper_local_only] {
        let error = harness
            .engine
            .receive_envelopes(
                &harness.coordinator,
                &[envelope.to_json().unwrap().into_bytes()],
            )
            .unwrap_err();
        assert!(matches!(error, MutationError::Invalid(_)));
    }

    assert!(fs::read_to_string(harness.vault.path().join("private.md"))
        .unwrap()
        .ends_with("private"));
    assert!(!harness.vault.path().join("leaked.md").exists());
    assert_eq!(harness.engine.status().unwrap().outbox_operations, 0);
    assert_eq!(harness.engine.status().unwrap().pending_operations, 0);
}

#[test]
fn external_local_only_identity_change_protects_new_and_mapped_ids() {
    let harness = Harness::new(None);
    let seed = [0xa5; 32];
    harness.trust(REMOTE_A, seed);
    let initial = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "external-policy-note",
        NoteRevisionKind::Put {
            markdown: "remote initial".into(),
        },
        vec![],
    );
    let initial_id = *initial.operation_id();
    harness.receive(&[initial]);
    let relative_key = projected_note_key(&harness, "external-policy-note");
    let path = harness.vault.path().join(&relative_key);
    let protected = b"---\ngrafyn_sync: local_only\nnote_id: local-private\n---\nprivate edit";
    fs::write(&path, protected).unwrap();

    let overwrite = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "external-policy-note",
        NoteRevisionKind::Put {
            markdown: "remote overwrite".into(),
        },
        vec![initial_id],
    );
    let overwrite_id = *overwrite.operation_id();
    harness.receive(&[overwrite]);

    assert_eq!(fs::read(&path).unwrap(), protected);
    {
        let state = harness.engine.state.lock().unwrap();
        assert_eq!(
            state
                .ledger
                .local_only_notes
                .get(&relative_key)
                .map(String::as_str),
            Some("local-private")
        );
        assert_eq!(
            state
                .ledger
                .note_paths
                .get("external-policy-note")
                .map(String::as_str),
            Some(relative_key.as_str())
        );
        assert!(state
            .ledger
            .rejected_operations
            .contains(&overwrite_id.to_string()));
    }

    for (index, protected_id) in ["external-policy-note", "local-private"]
        .into_iter()
        .enumerate()
    {
        let tombstone = note_envelope(
            &harness,
            REMOTE_A,
            seed,
            protected_id,
            NoteRevisionKind::Tombstone,
            vec![],
        );
        let error = harness
            .engine
            .receive_envelopes(
                &harness.coordinator,
                &[tombstone.to_json().unwrap().into_bytes()],
            )
            .unwrap_err();
        assert!(matches!(
            error,
            MutationError::Invalid(message)
                if message == "synced note targets locally protected content"
        ));

        let mut event = harness
            .coordinator
            .commit_local(
                CausalStream::LocalOnly,
                SourceChannel::parse("companion_capture").unwrap(),
                vec![],
                vec![draft(&format!("external-local-only-reference-{index}"))],
            )
            .unwrap()
            .events
            .into_iter()
            .next()
            .unwrap();
        event.device_id = crate::models::twin_event::DeviceId::parse(REMOTE_A).unwrap();
        event.causal_stream = CausalStream::SyncEligible;
        event.causal_parents.clear();
        event.device_sequence = u64::try_from(index + 2).unwrap();
        event.evidence = vec![EvidenceRef {
            evidence_type: EvidenceType::Note,
            source_id: crate::models::twin_event::Identifier::parse(protected_id).unwrap(),
            digest: None,
        }];
        event.event_id = crate::services::twin_events::derive_event_id(&event);
        let event = event_envelope(&harness, REMOTE_A, seed, &event, vec![]);
        let error = harness
            .engine
            .receive_envelopes(
                &harness.coordinator,
                &[event.to_json().unwrap().into_bytes()],
            )
            .unwrap_err();
        assert!(matches!(
            error,
            MutationError::Invalid(message)
                if message == "synced Twin event references a locally protected note"
        ));
    }

    let (reopened, coordinator) = harness.open_peer();
    reopened.recover_pending_inbox(&coordinator).unwrap();
    assert_eq!(reopened.status().unwrap().pending_operations, 0);
    assert_eq!(fs::read(path).unwrap(), protected);
}

#[test]
fn external_privacy_snapshot_is_linearized_with_local_edits() {
    let harness = Harness::new(None);
    let seed = [0xa4; 32];
    harness.trust(REMOTE_A, seed);
    let initial = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "linearized-policy-note",
        NoteRevisionKind::Put {
            markdown: "remote initial".into(),
        },
        vec![],
    );
    let initial_id = *initial.operation_id();
    harness.receive(&[initial]);
    let relative_key = projected_note_key(&harness, "linearized-policy-note");
    let path = harness.vault.path().join(&relative_key);
    fs::write(
        &path,
        "---\ngrafyn_sync: local_only\nnote_id: local-private\n---\nprivate edit",
    )
    .unwrap();
    let overwrite = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "linearized-policy-note",
        NoteRevisionKind::Put {
            markdown: "remote overwrite".into(),
        },
        vec![initial_id],
    );
    let overwrite_id = *overwrite.operation_id();
    let remote_bytes = vec![overwrite.to_json().unwrap().into_bytes()];
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    harness
        .engine
        .pause_after_remote_snapshot_once(entered.clone(), resume.clone());

    std::thread::scope(|scope| {
        let remote = scope.spawn(|| {
            harness
                .engine
                .receive_envelopes(&harness.coordinator, &remote_bytes)
        });
        entered.wait();
        let local_started = Arc::new(std::sync::Barrier::new(2));
        let local_thread_started = local_started.clone();
        let local_coordinator = &harness.coordinator;
        let local_key = &relative_key;
        let local = scope.spawn(move || {
            local_thread_started.wait();
            local_coordinator.commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("note_editor").unwrap(),
                vec![TargetMutation::put(
                    TargetKind::Markdown,
                    local_key,
                    "---\nnote_id: local-private\n---\npublic edit",
                )],
                vec![],
            )
        });
        local_started.wait();
        resume.wait();
        let remote = remote.join().unwrap();
        assert!(remote.is_ok(), "remote drain failed: {remote:?}");
        assert!(local.join().unwrap().is_ok());
    });

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "---\nnote_id: local-private\n---\npublic edit"
    );
    assert!(!harness
        .engine
        .state
        .lock()
        .unwrap()
        .ledger
        .local_only_notes
        .contains_key(&relative_key));

    let (reopened, coordinator) = harness.open_peer();
    let report = reopened.recover_pending_inbox(&coordinator).unwrap();
    assert_eq!(report.deferred, 0);
    assert_eq!(reopened.status().unwrap().pending_operations, 0);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "---\nnote_id: local-private\n---\npublic edit"
    );
    let state = reopened.state.lock().unwrap();
    assert_eq!(
        state.ledger.note_paths.get("local-private"),
        Some(&relative_key)
    );
    assert!(!state
        .ledger
        .note_paths
        .contains_key("linearized-policy-note"));
    assert!(state
        .ledger
        .rejected_operations
        .contains(&overwrite_id.to_string()));
}

#[test]
fn fresh_remote_put_cannot_displace_a_different_public_note_owner() {
    let harness = Harness::new(None);
    let seed = [0xa5; 32];
    harness.trust(REMOTE_A, seed);
    let remote_note_id = "colliding-remote-note";
    let digest = crate::services::twin_events::digest_bytes(
        format!("grafyn.sync.remote-path.v1:{remote_note_id}").as_bytes(),
    );
    let relative_key = format!("synced/{}.md", digest.as_str());
    let public = "---\nnote_id: local-public-owner\n---\npublic bytes";
    commit_note(&harness, &relative_key, public);
    let envelope = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        remote_note_id,
        NoteRevisionKind::Put {
            markdown: "remote bytes".into(),
        },
        vec![],
    );
    let operation_id = *envelope.operation_id();

    let error = harness
        .engine
        .receive_envelopes(
            &harness.coordinator,
            &[envelope.to_json().unwrap().into_bytes()],
        )
        .unwrap_err();

    assert!(matches!(
        error,
        MutationError::RecoveryConflict(message)
            if message == "sync note projection collided with an existing local path"
    ));
    assert_eq!(
        fs::read_to_string(harness.vault.path().join(&relative_key)).unwrap(),
        public
    );
    let state = harness.engine.state.lock().unwrap();
    assert_eq!(
        state.ledger.note_paths.get("local-public-owner"),
        Some(&relative_key)
    );
    assert!(!state.ledger.note_paths.contains_key(remote_note_id));
    assert!(!state.applied.contains(&operation_id));
}

#[test]
fn newly_local_only_note_quarantines_its_deferred_remote_update() {
    let harness = Harness::new(None);
    let seed = [0xb0; 32];
    harness.trust(REMOTE_A, seed);
    let missing_parent = OperationId::parse_hex(&"77".repeat(32)).unwrap();
    let deferred = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "protected-note",
        NoteRevisionKind::Put {
            markdown: "---\nnote_id: protected-note\n---\nremote".into(),
        },
        vec![missing_parent],
    );
    let deferred_id = *deferred.operation_id();
    let first = harness.receive(&[deferred]);
    assert_eq!(first.deferred, 1);

    commit_note(
        &harness,
        "private.md",
        "---\ngrafyn_sync: local_only\nnote_id: protected-note\n---\nprivate",
    );

    assert_eq!(harness.engine.status().unwrap().pending_operations, 0);
    assert!(harness
        .engine
        .state
        .lock()
        .unwrap()
        .ledger
        .rejected_operations
        .contains(&deferred_id.to_string()));
    let fresh = note_envelope(
        &harness,
        REMOTE_A,
        seed,
        "unrelated-note",
        NoteRevisionKind::Put {
            markdown: "---\nnote_id: unrelated-note\n---\nunrelated".into(),
        },
        vec![],
    );
    let report = harness.receive(&[fresh]);
    assert_eq!(report.applied, 1);
    assert!(
        fs::read_to_string(projected_note_path(&harness, "unrelated-note"))
            .unwrap()
            .ends_with("unrelated")
    );
}

#[test]
fn note_evidence_reference_to_local_only_material_never_enters_the_outbox() {
    let harness = Harness::new(None);
    commit_note(
        &harness,
        "private.md",
        "---\ngrafyn_sync: local_only\nnote_id: private-note\n---\nprivate",
    );
    let mut observation = draft("public-observation");
    observation.evidence.push(EvidenceRef {
        evidence_type: EvidenceType::Note,
        source_id: crate::models::twin_event::Identifier::parse("private-note").unwrap(),
        digest: None,
    });

    let committed = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("companion_capture").unwrap(),
            vec![],
            vec![observation],
        )
        .unwrap();

    assert_eq!(
        committed.events[0].causal_stream,
        CausalStream::SyncEligible
    );
    assert_eq!(harness.engine.status().unwrap().outbox_operations, 0);
}

#[test]
fn incoming_event_referencing_a_locally_protected_note_is_rejected_before_storage() {
    let harness = Harness::new(None);
    let seed = [0xa6; 32];
    commit_note(
        &harness,
        "private.md",
        "---\ngrafyn_sync: local_only\nnote_id: private-note\n---\nprivate",
    );
    let local = harness
        .coordinator
        .commit_local(
            CausalStream::LocalOnly,
            SourceChannel::parse("note_editor").unwrap(),
            vec![],
            vec![draft("remote-private-reference")],
        )
        .unwrap()
        .events
        .into_iter()
        .next()
        .unwrap();
    let mut event = local;
    event.device_id = crate::models::twin_event::DeviceId::parse(REMOTE_A).unwrap();
    event.causal_stream = CausalStream::SyncEligible;
    event.evidence.push(EvidenceRef {
        evidence_type: EvidenceType::Note,
        source_id: crate::models::twin_event::Identifier::parse("private-note").unwrap(),
        digest: None,
    });
    event.event_id = crate::services::twin_events::derive_event_id(&event);
    let rejected_id = event.event_id.clone();
    harness.trust(REMOTE_A, seed);
    let payload = TwinEventV1::new(
        Digest32::parse_hex(event.event_id.as_str()).unwrap(),
        serde_json::to_string(&event).unwrap(),
    )
    .unwrap();
    let operation = OperationV1::new(
        1_800_000_000_006,
        vec![],
        OperationPayloadV1::TwinEvent(payload),
    )
    .unwrap();
    let envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &DeviceId::parse_str(REMOTE_A).unwrap(),
        &DeviceSigningKey::from_seed(seed),
        &operation,
    )
    .unwrap();

    let error = harness
        .engine
        .receive_envelopes(
            &harness.coordinator,
            &[envelope.to_json().unwrap().into_bytes()],
        )
        .unwrap_err();

    assert!(error.to_string().contains("locally protected note"));
    assert_eq!(harness.engine.status().unwrap().pending_operations, 0);
    assert!(!harness
        .engine
        .event_store
        .ordered_events()
        .unwrap()
        .iter()
        .any(|candidate| candidate.event_id == rejected_id));
}

#[test]
fn local_attachment_is_chunked_promoted_and_materialized_by_digest() {
    let harness = Harness::new(None);
    let mut bytes = vec![0x5a; ATTACHMENT_CHUNK_BYTES + 17];
    bytes[ATTACHMENT_CHUNK_BYTES] = 0x73;

    let digest = harness
        .engine
        .queue_attachment("application/octet-stream", &bytes)
        .unwrap();

    assert_eq!(
        harness.engine.materialized_attachment(&digest).unwrap(),
        Some(bytes)
    );
    assert_eq!(harness.engine.status().unwrap().outbox_operations, 3);
    assert!(harness
        .engine
        .state
        .lock()
        .unwrap()
        .ledger
        .standalone_batches
        .is_empty());
}

#[test]
fn promoted_local_attachment_batch_recovers_before_clearing_its_witness() {
    let harness = Harness::new(None);
    let mut bytes = vec![0x61; ATTACHMENT_CHUNK_BYTES + 11];
    bytes[ATTACHMENT_CHUNK_BYTES] = 0x72;
    let digest = crate::models::attachment::sha256_attachment_digest(&bytes);
    let manifest =
        AttachmentManifestV1::new(digest, "application/octet-stream", bytes.len()).unwrap();
    let (vault_id, device_id, signing_seed, root_key) = {
        let state = harness.engine.state.lock().unwrap();
        let device = state.device.as_ref().unwrap();
        (
            state.vault_id,
            device.device_id,
            *device.signing_key.export_seed(),
            *state.root_key.as_ref().unwrap().export_bytes(),
        )
    };
    let manifest_operation = OperationV1::new(
        1_800_000_000_016,
        vec![],
        OperationPayloadV1::AttachmentManifest(manifest.clone()),
    )
    .unwrap();
    let manifest_envelope = seal_operation(
        &VaultRootKey::from_bytes(root_key),
        &vault_id,
        &device_id,
        &DeviceSigningKey::from_seed(signing_seed),
        &manifest_operation,
    )
    .unwrap();
    let manifest_id = *manifest_envelope.operation_id();
    let mut envelopes = vec![manifest_envelope];
    for (index, data) in bytes.chunks(ATTACHMENT_CHUNK_BYTES).enumerate() {
        let chunk = AttachmentChunkV1::new(
            manifest_id,
            digest,
            index as u32,
            manifest.chunk_count(),
            data.to_vec(),
        )
        .unwrap();
        let operation = OperationV1::new(
            1_800_000_000_016,
            vec![manifest_id],
            OperationPayloadV1::AttachmentChunk(chunk),
        )
        .unwrap();
        envelopes.push(
            seal_operation(
                &VaultRootKey::from_bytes(root_key),
                &vault_id,
                &device_id,
                &DeviceSigningKey::from_seed(signing_seed),
                &operation,
            )
            .unwrap(),
        );
    }
    let mutation_id = standalone_attachment_mutation_id(&manifest_id);
    {
        let mut state = harness.engine.state.lock().unwrap();
        state.ledger.standalone_batches.insert(mutation_id.clone());
        persist_ledger(
            &harness.engine.data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )
        .unwrap();
        state
            .operation_store
            .stage_batch(&mutation_id, &envelopes)
            .unwrap();
        state.operation_store.promote_batch(&mutation_id).unwrap();
    }

    let (reopened, _) = harness.open_peer();

    assert_eq!(
        reopened.materialized_attachment(&digest).unwrap(),
        Some(bytes)
    );
    let state = reopened.state.lock().unwrap();
    assert!(state.ledger.standalone_batches.is_empty());
    let applied = state.operation_store.list_applied_ids().unwrap();
    assert!(envelopes
        .iter()
        .all(|envelope| applied.contains(envelope.operation_id())));
}

#[test]
fn cancelled_standalone_batch_marker_is_cleared_during_recovery() {
    let harness = Harness::new(None);
    let mutation_id = crate::services::twin_events::digest_bytes(
        b"grafyn.sync.attachment.v1:cancelled-crash-window",
    );
    let envelope = note_envelope(
        &harness,
        REMOTE_A,
        [0xd1; 32],
        "cancelled-witness",
        NoteRevisionKind::Put {
            markdown: "never promoted".into(),
        },
        vec![],
    );
    {
        let mut state = harness.engine.state.lock().unwrap();
        state
            .operation_store
            .stage_batch(&mutation_id, &[envelope])
            .unwrap();
        assert!(state.operation_store.cancel_batch(&mutation_id).unwrap());
        state.ledger.standalone_batches.insert(mutation_id.clone());
        persist_ledger(
            &harness.engine.data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )
        .unwrap();
    }

    let (reopened, _) = harness.open_peer();

    assert!(reopened
        .state
        .lock()
        .unwrap()
        .ledger
        .standalone_batches
        .is_empty());
    assert_eq!(reopened.status().unwrap().outbox_operations, 0);
}

#[test]
fn retarget_to_same_vault_identity_preserves_the_existing_sync_scope() {
    let harness = Harness::new(None);
    let _ = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("note_editor").unwrap(),
            vec![],
            vec![draft("moved-vault-note")],
        )
        .unwrap();
    let before = harness.engine.export_outbox().unwrap();
    assert_eq!(before.len(), 1);
    let moved = tempfile::tempdir().unwrap();
    let descriptor_path = moved.path().join(VAULT_DESCRIPTOR_KEY);
    fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
    fs::write(&descriptor_path, harness.descriptor_bytes()).unwrap();

    let prepared = harness.engine.prepare_retarget(moved.path()).unwrap();
    harness.engine.activate_retarget(prepared).unwrap();

    assert_eq!(harness.engine.export_outbox().unwrap(), before);
    let state = harness.engine.state.lock().unwrap();
    assert_eq!(state.vault_scope, harness.identity.root_scope);
    assert_eq!(state.vault_path, fs::canonicalize(moved.path()).unwrap());
}

#[test]
fn retarget_to_different_vault_identity_isolates_then_restores_the_old_scope() {
    let harness = Harness::new(None);
    let _ = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("note_editor").unwrap(),
            vec![],
            vec![draft("isolated-vault-note")],
        )
        .unwrap();
    let old_outbox = harness.engine.export_outbox().unwrap();
    let other = tempfile::tempdir().unwrap();
    let other_identity = load_or_create_vault_identity(other.path()).unwrap();
    assert_ne!(other_identity.root_scope, harness.identity.root_scope);

    let prepared = harness.engine.prepare_retarget(other.path()).unwrap();
    harness.engine.activate_retarget(prepared).unwrap();
    let isolated = harness.engine.status().unwrap();
    assert!(!isolated.provisioned);
    assert_eq!(isolated.outbox_operations, 0);
    assert_eq!(
        harness.engine.state.lock().unwrap().vault_scope,
        other_identity.root_scope
    );

    let prepared = harness
        .engine
        .prepare_retarget(harness.vault.path())
        .unwrap();
    harness.engine.activate_retarget(prepared).unwrap();
    assert_eq!(harness.engine.export_outbox().unwrap(), old_outbox);
    assert!(harness.engine.status().unwrap().provisioned);
}

#[test]
fn failed_retarget_preparation_leaves_the_active_engine_unchanged() {
    let harness = Harness::new(None);
    let _ = harness
        .coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("note_editor").unwrap(),
            vec![],
            vec![draft("retained-vault-note")],
        )
        .unwrap();
    let before_outbox = harness.engine.export_outbox().unwrap();
    let before_path = harness.engine.state.lock().unwrap().vault_path.clone();
    let invalid = tempfile::tempdir().unwrap();
    let descriptor_path = invalid.path().join(VAULT_DESCRIPTOR_KEY);
    fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
    fs::write(&descriptor_path, b"not a vault descriptor").unwrap();

    assert!(harness.engine.prepare_retarget(invalid.path()).is_err());

    assert_eq!(harness.engine.export_outbox().unwrap(), before_outbox);
    assert_eq!(harness.engine.state.lock().unwrap().vault_path, before_path);
}

#[test]
fn remote_finalized_event_keeps_sender_fields_and_never_enters_the_outbox() {
    let harness = Harness::new(None);
    let seed = [0xa5; 32];
    let local = harness
        .coordinator
        .commit_local(
            CausalStream::LocalOnly,
            SourceChannel::parse("note_editor").unwrap(),
            vec![],
            vec![draft("remote-event-note")],
        )
        .unwrap()
        .events
        .into_iter()
        .next()
        .unwrap();
    let mut event = local;
    event.device_id = crate::models::twin_event::DeviceId::parse(REMOTE_A).unwrap();
    event.causal_stream = CausalStream::SyncEligible;
    event.event_id = crate::services::twin_events::derive_event_id(&event);
    harness.trust(REMOTE_A, seed);
    let event_json = serde_json::to_string(&event).unwrap();
    let payload = TwinEventV1::new(
        Digest32::parse_hex(event.event_id.as_str()).unwrap(),
        event_json,
    )
    .unwrap();
    let operation = OperationV1::new(
        1_800_000_000_001,
        vec![],
        OperationPayloadV1::TwinEvent(payload),
    )
    .unwrap();
    let envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        harness.identity.descriptor.vault_id(),
        &DeviceId::parse_str(REMOTE_A).unwrap(),
        &DeviceSigningKey::from_seed(seed),
        &operation,
    )
    .unwrap();

    let report = harness.receive(&[envelope]);

    assert_eq!(report.applied, 1);
    let events = harness.engine.event_store.ordered_events().unwrap();
    let received = events
        .iter()
        .find(|candidate| candidate.event_id == event.event_id)
        .unwrap();
    assert_eq!(received, &event);
    assert_eq!(harness.engine.status().unwrap().outbox_operations, 0);
}
