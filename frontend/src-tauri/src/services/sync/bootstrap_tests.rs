use super::engine::SyncEngine;
#[cfg(feature = "tauri-app")]
use super::identity::load_vault_identity;
use super::identity::{load_or_create_vault_identity, VaultIdentity, VAULT_DESCRIPTOR_KEY};
use super::secrets::MemorySecretStore;
use super::vault_keys::provision_vault_root_key;
#[cfg(feature = "tauri-app")]
use crate::models::image_generation::{GeneratedImageSyncPolicy, ImageMetadataRetentionPolicy};
#[cfg(feature = "tauri-app")]
use crate::models::twin_event::ContentDigest;
use crate::models::twin_event::{
    CausalStream, EvidenceRef, EvidenceType, Governance, NoteChangeKind, NoteChanged,
    SourceChannel, TwinEventPayload,
};
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::twin_events::{
    test_support, MutationCoordinator, TargetKind, TargetMutation, TwinEventDraft, TwinEventStore,
};
use chrono::{TimeZone, Utc};
#[cfg(feature = "tauri-app")]
use grafyn_sync_protocol::Digest32;
use grafyn_sync_protocol::{
    open_operation, seal_operation, DeviceId, DeviceSigningKey, NoteRevisionV1, OperationPayloadV1,
    OperationV1, TrustedDevice, VaultRootKey,
};
#[cfg(feature = "tauri-app")]
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

const ROOT_KEY_BYTES: [u8; 32] = [0x71; 32];

fn draft(note_id: &str) -> TwinEventDraft {
    TwinEventDraft {
        actor_id: None,
        causal_parents: Vec::new(),
        recorded_at: Utc.with_ymd_and_hms(2026, 8, 31, 4, 0, 0).unwrap(),
        observed_at: Utc.with_ymd_and_hms(2026, 8, 31, 4, 0, 0).unwrap(),
        occurred_at: None,
        valid_from: None,
        valid_to: None,
        supersedes: Vec::new(),
        reinforces: Vec::new(),
        context: Default::default(),
        evidence: Vec::new(),
        governance: Governance::direct_observation(),
        payload: TwinEventPayload::NoteChanged(NoteChanged {
            note_id: crate::models::twin_event::Identifier::parse(note_id).unwrap(),
            change: NoteChangeKind::Created,
            content_digest: None,
        }),
    }
}

fn snapshot_tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = walkdir::WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            (
                entry
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

struct ExistingVault {
    _data_root: tempfile::TempDir,
    _vault_root: tempfile::TempDir,
    data_path: std::path::PathBuf,
    vault_path: std::path::PathBuf,
    identity: VaultIdentity,
    secrets: Arc<MemorySecretStore>,
    eligible_markdown: String,
    legacy_markdown: String,
    legacy_note_id: String,
    mapped_markdown: String,
    parent_event_id: String,
    child_event_id: String,
}

impl ExistingVault {
    fn seeded() -> Self {
        let data_root = tempfile::tempdir().unwrap();
        let vault_root = tempfile::tempdir().unwrap();
        let data_path = data_root.path().to_path_buf();
        let vault_path = vault_root.path().to_path_buf();
        std::fs::create_dir_all(data_path.join("twin/events")).unwrap();
        let identity = load_or_create_vault_identity(&vault_path).unwrap();
        let secrets = Arc::new(MemorySecretStore::default());
        let event_store = Arc::new(TwinEventStore::new(&data_path));
        let engine = Arc::new(
            SyncEngine::open_core(
                &data_path,
                &vault_path,
                identity.clone(),
                None,
                secrets.clone(),
                event_store.clone(),
            )
            .unwrap(),
        );
        let coordinator =
            MutationCoordinator::new_stable(&data_path, &vault_path, event_store, engine.clone())
                .unwrap();
        let device = coordinator
            .load_or_create_device_signing_identity(secrets.clone())
            .unwrap();
        engine.attach_device_identity(device).unwrap();

        let eligible_markdown = "---\r\nnote_id: exact-note\r\ntitle: Exact bytes\r\ngrafyn_sync: inherit\r\n---\r\nline one  \r\nline two\r\n".to_owned();
        std::fs::write(vault_path.join("eligible.md"), eligible_markdown.as_bytes()).unwrap();
        let legacy_markdown = "Legacy identity keeps these exact bytes  \r\n".to_owned();
        std::fs::write(vault_path.join("legacy.md"), legacy_markdown.as_bytes()).unwrap();
        let legacy_digest = crate::services::twin_events::digest_bytes(
            format!(
                "grafyn.sync.note-id.v1:{}:legacy.md",
                identity.root_scope.as_str()
            )
            .as_bytes(),
        );
        let legacy_note_id = format!("legacy-{}", legacy_digest.as_str());
        let mapped_markdown = "Existing ledger identity wins over the path\n".to_owned();
        std::fs::write(vault_path.join("mapped.md"), mapped_markdown.as_bytes()).unwrap();
        std::fs::write(
            vault_path.join("private.md"),
            b"---\nnote_id: private-note\ngrafyn_sync: local_only\n---\nprivate\n",
        )
        .unwrap();
        std::fs::write(
            vault_path.join("malformed.md"),
            b"---\nnote_id: malformed-note\ngrafyn_sync: [broken\n---\nprivate\n",
        )
        .unwrap();
        let program = vault_path.join("_grafyn/program.md");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::write(program, b"never synchronize this program\n").unwrap();

        let parent = coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("companion_capture").unwrap(),
                Vec::new(),
                vec![draft("event-parent")],
            )
            .unwrap()
            .events
            .into_iter()
            .next()
            .unwrap();
        let mut child_draft = draft("event-child");
        child_draft.causal_parents.push(parent.event_id.clone());
        let child = coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("companion_capture").unwrap(),
                Vec::new(),
                vec![child_draft],
            )
            .unwrap()
            .events
            .into_iter()
            .next()
            .unwrap();
        let mut private_reference = draft("private-note");
        private_reference
            .causal_parents
            .push(child.event_id.clone());
        private_reference.evidence.push(EvidenceRef {
            evidence_type: EvidenceType::Note,
            source_id: crate::models::twin_event::Identifier::parse("private-note").unwrap(),
            digest: None,
        });
        let _ = coordinator
            .commit_local(
                CausalStream::SyncEligible,
                SourceChannel::parse("companion_capture").unwrap(),
                Vec::new(),
                vec![private_reference],
            )
            .unwrap();
        let _ = coordinator
            .commit_local(
                CausalStream::LocalOnly,
                SourceChannel::parse("note_editor").unwrap(),
                Vec::new(),
                vec![draft("local-event")],
            )
            .unwrap();
        assert_eq!(engine.status().unwrap().outbox_operations, 0);
        drop(coordinator);
        drop(engine);

        let ledger_path = data_path
            .join("sync/vaults/v1")
            .join(identity.root_scope.as_str())
            .join("engine-ledger-v1.json");
        let mut ledger: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&ledger_path).unwrap()).unwrap();
        ledger["note_paths"]["mapped-note"] = serde_json::Value::String("mapped.md".into());
        let mut ledger_bytes = serde_json::to_vec_pretty(&ledger).unwrap();
        ledger_bytes.push(b'\n');
        std::fs::write(ledger_path, ledger_bytes).unwrap();

        provision_vault_root_key(
            secrets.as_ref(),
            &identity.descriptor.vault_id().to_string(),
            &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        )
        .unwrap();

        Self {
            _data_root: data_root,
            _vault_root: vault_root,
            data_path,
            vault_path,
            identity,
            secrets,
            eligible_markdown,
            legacy_markdown,
            legacy_note_id,
            mapped_markdown,
            parent_event_id: parent.event_id.as_str().to_owned(),
            child_event_id: child.event_id.as_str().to_owned(),
        }
    }

    fn open_provisioned(
        &self,
    ) -> (
        Arc<SyncEngine>,
        Arc<MutationCoordinator>,
        KnowledgeStore,
        Arc<TwinEventStore>,
    ) {
        let event_store = Arc::new(TwinEventStore::new(&self.data_path));
        let engine = Arc::new(
            SyncEngine::open_core(
                &self.data_path,
                &self.vault_path,
                self.identity.clone(),
                Some(VaultRootKey::from_bytes(ROOT_KEY_BYTES)),
                self.secrets.clone(),
                event_store.clone(),
            )
            .unwrap(),
        );
        let coordinator = Arc::new(
            MutationCoordinator::new_stable(
                &self.data_path,
                &self.vault_path,
                event_store.clone(),
                engine.clone(),
            )
            .unwrap(),
        );
        let device = coordinator
            .load_or_create_device_signing_identity(self.secrets.clone())
            .unwrap();
        engine.attach_device_identity(device).unwrap();
        let knowledge = KnowledgeStore::with_event_recorder(
            self.vault_path.clone(),
            self.data_path.join("bootstrap-derived"),
            coordinator.clone(),
        );
        (engine, coordinator, knowledge, event_store)
    }
}

#[cfg(feature = "tauri-app")]
struct UnprovisionedImageVault {
    _data_root: tempfile::TempDir,
    _vault_root: tempfile::TempDir,
    data_path: std::path::PathBuf,
    vault_path: std::path::PathBuf,
    identity: VaultIdentity,
    secrets: Arc<MemorySecretStore>,
    engine: Arc<SyncEngine>,
    coordinator: Arc<MutationCoordinator>,
    knowledge: KnowledgeStore,
}

#[cfg(feature = "tauri-app")]
impl UnprovisionedImageVault {
    fn new() -> Self {
        let data_root = tempfile::tempdir().unwrap();
        let vault_root = tempfile::tempdir().unwrap();
        let data_path = data_root.path().to_path_buf();
        let vault_path = vault_root.path().to_path_buf();
        std::fs::create_dir_all(data_path.join("twin/events")).unwrap();
        let identity = load_or_create_vault_identity(&vault_path).unwrap();
        let secrets = Arc::new(MemorySecretStore::default());
        let event_store = Arc::new(TwinEventStore::new(&data_path));
        let engine = Arc::new(
            SyncEngine::open_core(
                &data_path,
                &vault_path,
                identity.clone(),
                None,
                secrets.clone(),
                event_store.clone(),
            )
            .unwrap(),
        );
        let coordinator = Arc::new(
            MutationCoordinator::new_stable(&data_path, &vault_path, event_store, engine.clone())
                .unwrap(),
        );
        let device = coordinator
            .load_or_create_device_signing_identity(secrets.clone())
            .unwrap();
        engine.attach_device_identity(device).unwrap();
        let knowledge = KnowledgeStore::with_event_recorder(
            vault_path.clone(),
            data_path.join("image-bootstrap-derived"),
            coordinator.clone(),
        );
        Self {
            _data_root: data_root,
            _vault_root: vault_root,
            data_path,
            vault_path,
            identity,
            secrets,
            engine,
            coordinator,
            knowledge,
        }
    }

    fn save_generated_image(
        &mut self,
        sync_policy: GeneratedImageSyncPolicy,
        marker: u8,
    ) -> (
        Vec<u8>,
        Digest32,
        crate::models::attachment::ImageAttachmentCatalogRecordV1,
    ) {
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([marker, 0, 0, 255]));
        let mut output = Cursor::new(Vec::new());
        image
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        let bytes = output.into_inner();
        let (lease, digest) = self
            .engine
            .register_pending_generated_image(&self.identity.root_scope, &bytes, "image/png")
            .unwrap();
        let attachment_digest = ContentDigest::parse(digest.to_string()).unwrap();
        let captured_at = Utc
            .with_ymd_and_hms(2026, 9, 1, 6, 0, u32::from(marker))
            .unwrap();
        let (create, observation) = crate::commands::twin_state::plan_generated_image_capture(
            crate::commands::twin_state::GeneratedImageCapturePlanInput {
                prompt: format!("bootstrap generated image {marker}"),
                annotation: None,
                model_id: "test/image-model".into(),
                gateway: "openrouter".into(),
                provider_tag: "test-provider".into(),
                resolution: "2x2".into(),
                aspect_ratio: "1:1".into(),
                attachment_digest,
                media_type: "image/png".into(),
                byte_size: bytes.len() as u64,
                width: 2,
                height: 2,
                retention_policy: ImageMetadataRetentionPolicy::RetainOriginal,
                grafyn_sync: sync_policy,
            },
            captured_at,
        )
        .unwrap();
        let expected = self.coordinator.current_authority_token().unwrap();
        let _ = self
            .knowledge
            .create_generated_image_capture_expecting_authority(create, observation, expected)
            .into_result()
            .unwrap();
        drop(lease);
        assert_eq!(self.engine.status().unwrap().outbox_operations, 0);
        let catalog = self
            .engine
            .cataloged_image_expecting_scope(&self.identity.root_scope, &digest)
            .unwrap()
            .unwrap();
        assert_eq!(
            self.engine.materialized_attachment(&digest).unwrap(),
            Some(bytes.clone())
        );
        (bytes, digest, catalog)
    }

    fn provision(&self) {
        provision_vault_root_key(
            self.secrets.as_ref(),
            &self.identity.descriptor.vault_id().to_string(),
            &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        )
        .unwrap();
        let prepared = self.engine.prepare_retarget(&self.vault_path).unwrap();
        self.engine.activate_retarget(prepared).unwrap();
        assert!(self.engine.status().unwrap().provisioned);
    }

    fn prepared_path(&self) -> std::path::PathBuf {
        self.data_path
            .join("sync/vaults/v1")
            .join(self.identity.root_scope.as_str())
            .join("bootstrap/v1/prepared.json")
    }

    fn catalog_path(&self, digest: &Digest32) -> std::path::PathBuf {
        self.data_path
            .join("sync/vaults/v1")
            .join(self.identity.root_scope.as_str())
            .join("attachments/v1/catalog")
            .join(format!("{digest}.json"))
    }
}

#[cfg(feature = "tauri-app")]
#[test]
fn unprovisioned_inherited_generated_image_bootstrap_reconstructs_catalog_on_peer() {
    let mut source = UnprovisionedImageVault::new();
    let (bytes, digest, catalog) =
        source.save_generated_image(GeneratedImageSyncPolicy::Inherit, 1);
    source.provision();

    source
        .engine
        .bootstrap_existing_vault(source.coordinator.as_ref(), &source.knowledge)
        .unwrap();

    let outbox = source.engine.export_outbox().unwrap();
    assert_eq!(outbox.len(), 5, "note + two events + manifest + chunk");
    let peer_data = tempfile::tempdir().unwrap();
    let peer_vault = tempfile::tempdir().unwrap();
    let descriptor_path = peer_vault.path().join(VAULT_DESCRIPTOR_KEY);
    std::fs::create_dir_all(descriptor_path.parent().unwrap()).unwrap();
    std::fs::copy(
        source.vault_path.join(VAULT_DESCRIPTOR_KEY),
        &descriptor_path,
    )
    .unwrap();
    std::fs::create_dir_all(peer_data.path().join("twin/events")).unwrap();
    let peer_identity = load_vault_identity(peer_vault.path()).unwrap();
    let peer_secrets = Arc::new(MemorySecretStore::default());
    provision_vault_root_key(
        peer_secrets.as_ref(),
        &peer_identity.descriptor.vault_id().to_string(),
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
    )
    .unwrap();
    let peer_events = Arc::new(TwinEventStore::new(peer_data.path()));
    let peer_engine = Arc::new(
        SyncEngine::open_core(
            peer_data.path(),
            peer_vault.path(),
            peer_identity.clone(),
            Some(VaultRootKey::from_bytes(ROOT_KEY_BYTES)),
            peer_secrets.clone(),
            peer_events.clone(),
        )
        .unwrap(),
    );
    let peer_coordinator = Arc::new(
        MutationCoordinator::new_stable(
            peer_data.path(),
            peer_vault.path(),
            peer_events,
            peer_engine.clone(),
        )
        .unwrap(),
    );
    let peer_device = peer_coordinator
        .load_or_create_device_signing_identity(peer_secrets)
        .unwrap();
    peer_engine.attach_device_identity(peer_device).unwrap();
    let (source_device, source_public_key) = source.engine.local_device().unwrap();
    peer_engine
        .trust_device(source_device, source_public_key)
        .unwrap();

    peer_engine
        .receive_envelopes(peer_coordinator.as_ref(), &outbox)
        .unwrap();

    assert_eq!(
        peer_engine
            .cataloged_image_expecting_scope(&peer_identity.root_scope, &digest)
            .unwrap(),
        Some(catalog)
    );
    assert_eq!(
        peer_engine.materialized_attachment(&digest).unwrap(),
        Some(bytes)
    );
    assert_eq!(peer_engine.status().unwrap().outbox_operations, 0);
}

#[cfg(feature = "tauri-app")]
#[test]
fn mixed_generated_image_bootstrap_includes_only_inherited_attachment() {
    let mut source = UnprovisionedImageVault::new();
    let (_, inherited_digest, _) =
        source.save_generated_image(GeneratedImageSyncPolicy::Inherit, 2);
    let (_, local_digest, _) = source.save_generated_image(GeneratedImageSyncPolicy::LocalOnly, 3);
    source.provision();

    source
        .engine
        .bootstrap_existing_vault(source.coordinator.as_ref(), &source.knowledge)
        .unwrap();

    let (device_id, public_key) = source.engine.local_device().unwrap();
    let trusted = TrustedDevice::new(device_id, public_key).unwrap();
    let mut manifests = std::collections::BTreeSet::new();
    for bytes in source.engine.export_outbox().unwrap() {
        let envelope = grafyn_sync_protocol::EnvelopeV1::from_json_bytes(&bytes).unwrap();
        let verified = open_operation(
            &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
            source.identity.descriptor.vault_id(),
            &trusted,
            &envelope,
        )
        .unwrap();
        if let OperationPayloadV1::AttachmentManifest(manifest) = verified.operation().payload() {
            manifests.insert(*manifest.attachment_digest());
        }
    }
    assert_eq!(
        manifests,
        std::collections::BTreeSet::from([inherited_digest])
    );
    assert!(!manifests.contains(&local_digest));
    assert!(source
        .engine
        .cataloged_image_expecting_scope(&source.identity.root_scope, &local_digest)
        .unwrap()
        .is_some());
}

#[cfg(feature = "tauri-app")]
#[test]
fn bootstrap_rejects_missing_generated_image_catalog_before_witness_install() {
    let mut source = UnprovisionedImageVault::new();
    let (_, digest, _) = source.save_generated_image(GeneratedImageSyncPolicy::Inherit, 4);
    source.provision();
    std::fs::remove_file(source.catalog_path(&digest)).unwrap();

    let error = source
        .engine
        .prepare_existing_vault_bootstrap(source.coordinator.as_ref(), &source.knowledge)
        .unwrap_err();

    assert!(error.to_string().contains("catalog"));
    assert_eq!(source.engine.status().unwrap().outbox_operations, 0);
    assert!(!source.prepared_path().exists());
}

#[cfg(feature = "tauri-app")]
#[test]
fn bootstrap_rejects_corrupt_generated_image_catalog_before_witness_install() {
    let mut source = UnprovisionedImageVault::new();
    let (_, digest, _) = source.save_generated_image(GeneratedImageSyncPolicy::Inherit, 5);
    source.provision();
    std::fs::write(source.catalog_path(&digest), b"not a catalog").unwrap();

    let error = source
        .engine
        .prepare_existing_vault_bootstrap(source.coordinator.as_ref(), &source.knowledge)
        .unwrap_err();

    assert!(error.to_string().contains("catalog"));
    assert_eq!(source.engine.status().unwrap().outbox_operations, 0);
    assert!(!source.prepared_path().exists());
}

#[cfg(feature = "tauri-app")]
#[test]
fn finish_rejects_incomplete_generated_image_attachment_witness() {
    let mut source = UnprovisionedImageVault::new();
    let _ = source.save_generated_image(GeneratedImageSyncPolicy::Inherit, 6);
    source.provision();
    assert!(source
        .engine
        .prepare_existing_vault_bootstrap(source.coordinator.as_ref(), &source.knowledge)
        .unwrap());
    let witness_path = source.prepared_path();
    let mut witness: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&witness_path).unwrap()).unwrap();
    let (device_id, public_key) = source.engine.local_device().unwrap();
    let trusted = TrustedDevice::new(device_id, public_key).unwrap();
    let root_key = VaultRootKey::from_bytes(ROOT_KEY_BYTES);
    let envelopes = witness["envelopes"].as_array_mut().unwrap();
    let chunk_index = envelopes
        .iter()
        .position(|value| {
            let bytes = serde_json::to_vec(value).unwrap();
            let envelope = grafyn_sync_protocol::EnvelopeV1::from_json_bytes(&bytes).unwrap();
            let verified = open_operation(
                &root_key,
                source.identity.descriptor.vault_id(),
                &trusted,
                &envelope,
            )
            .unwrap();
            matches!(
                verified.operation().payload(),
                OperationPayloadV1::AttachmentChunk(_)
            )
        })
        .expect("prepared generated image bootstrap must contain a chunk");
    envelopes.remove(chunk_index);
    std::fs::write(&witness_path, serde_json::to_vec_pretty(&witness).unwrap()).unwrap();

    let error = source
        .engine
        .finish_existing_vault_bootstrap(source.coordinator.as_ref(), &source.knowledge)
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("incomplete generated image attachment"));
    assert_eq!(source.engine.status().unwrap().outbox_operations, 0);
}

#[test]
fn existing_vault_bootstrap_seals_exact_markdown_and_causally_linked_events_once() {
    let existing = ExistingVault::seeded();
    let vault_before = snapshot_tree(&existing.vault_path);
    let events_before = snapshot_tree(&existing.data_path.join("twin/events"));
    let (engine, coordinator, knowledge, _event_store) = existing.open_provisioned();

    engine
        .bootstrap_existing_vault(coordinator.as_ref(), &knowledge)
        .unwrap();

    let outbox = engine.export_outbox().unwrap();
    assert_eq!(outbox.len(), 5);
    let (device_id, public_key) = engine.local_device().unwrap();
    let trusted = TrustedDevice::new(device_id, public_key).unwrap();
    let root_key = VaultRootKey::from_bytes(ROOT_KEY_BYTES);
    let mut note_markdown = std::collections::BTreeMap::new();
    let mut event_operations = std::collections::BTreeMap::new();
    for bytes in &outbox {
        let envelope = grafyn_sync_protocol::EnvelopeV1::from_json_bytes(bytes).unwrap();
        let verified = open_operation(
            &root_key,
            existing.identity.descriptor.vault_id(),
            &trusted,
            &envelope,
        )
        .unwrap();
        match verified.operation().payload() {
            OperationPayloadV1::NoteRevision(revision) => {
                if let grafyn_sync_protocol::NoteRevisionKind::Put { markdown } = revision.kind() {
                    note_markdown.insert(revision.note_id().to_owned(), markdown.clone());
                }
            }
            OperationPayloadV1::TwinEvent(event) => {
                event_operations.insert(
                    event.event_id().to_string(),
                    (
                        *verified.operation_id(),
                        verified.operation().causal_parents().to_vec(),
                    ),
                );
            }
            other => panic!("unexpected bootstrap payload: {other:?}"),
        }
    }
    assert_eq!(
        note_markdown.get("exact-note"),
        Some(&existing.eligible_markdown)
    );
    assert_eq!(
        note_markdown.get(&existing.legacy_note_id),
        Some(&existing.legacy_markdown)
    );
    assert_eq!(
        note_markdown.get("mapped-note"),
        Some(&existing.mapped_markdown)
    );
    assert_eq!(event_operations.len(), 2);
    let parent_operation = event_operations[&existing.parent_event_id].0;
    assert_eq!(
        event_operations[&existing.child_event_id].1,
        vec![parent_operation]
    );
    assert_eq!(snapshot_tree(&existing.vault_path), vault_before);
    assert_eq!(
        snapshot_tree(&existing.data_path.join("twin/events")),
        events_before
    );
    let ledger: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            existing
                .data_path
                .join("sync/vaults/v1")
                .join(existing.identity.root_scope.as_str())
                .join("engine-ledger-v1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(ledger["note_paths"]["exact-note"], "eligible.md");
    assert_eq!(ledger["note_paths"][&existing.legacy_note_id], "legacy.md");
    assert_eq!(ledger["note_paths"]["mapped-note"], "mapped.md");

    let first_outbox = outbox;
    drop(knowledge);
    drop(coordinator);
    drop(engine);
    let (reopened, coordinator, knowledge, _event_store) = existing.open_provisioned();
    reopened
        .bootstrap_existing_vault(coordinator.as_ref(), &knowledge)
        .unwrap();
    assert_eq!(reopened.export_outbox().unwrap(), first_outbox);
}

#[test]
fn unprovisioned_existing_vault_bootstrap_leaves_the_outbox_empty() {
    let data = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("twin/events")).unwrap();
    std::fs::write(vault.path().join("existing.md"), b"existing\n").unwrap();
    let identity = load_or_create_vault_identity(vault.path()).unwrap();
    assert!(vault.path().join(VAULT_DESCRIPTOR_KEY).exists());
    let events = Arc::new(TwinEventStore::new(data.path()));
    let secrets = Arc::new(MemorySecretStore::default());
    let engine = Arc::new(
        SyncEngine::open_core(
            data.path(),
            vault.path(),
            identity,
            None,
            secrets.clone(),
            events.clone(),
        )
        .unwrap(),
    );
    let coordinator = Arc::new(
        MutationCoordinator::new_stable(data.path(), vault.path(), events, engine.clone()).unwrap(),
    );
    let device = coordinator
        .load_or_create_device_signing_identity(secrets)
        .unwrap();
    engine.attach_device_identity(device).unwrap();
    let knowledge = KnowledgeStore::with_event_recorder(
        vault.path().to_path_buf(),
        data.path().join("derived"),
        coordinator.clone(),
    );

    engine
        .bootstrap_existing_vault(coordinator.as_ref(), &knowledge)
        .unwrap();

    assert!(!engine.status().unwrap().provisioned);
    assert_eq!(engine.status().unwrap().outbox_operations, 0);
}

#[test]
fn existing_vault_bootstrap_does_not_seal_unterminated_frontmatter() {
    let data = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("twin/events")).unwrap();
    let shared_markdown = b"# Shared note\n";
    let private_markdown = b"---\nnote_id: private-note\ngrafyn_sync: local_only\nprivate body\n";
    std::fs::write(vault.path().join("shared.md"), shared_markdown).unwrap();
    std::fs::write(vault.path().join("private.md"), private_markdown).unwrap();
    let identity = load_or_create_vault_identity(vault.path()).unwrap();
    let secrets = Arc::new(MemorySecretStore::default());
    provision_vault_root_key(
        secrets.as_ref(),
        &identity.descriptor.vault_id().to_string(),
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
    )
    .unwrap();
    let events = Arc::new(TwinEventStore::new(data.path()));
    let engine = Arc::new(
        SyncEngine::open_core(
            data.path(),
            vault.path(),
            identity.clone(),
            Some(VaultRootKey::from_bytes(ROOT_KEY_BYTES)),
            secrets.clone(),
            events.clone(),
        )
        .unwrap(),
    );
    let coordinator = Arc::new(
        MutationCoordinator::new_stable(data.path(), vault.path(), events, engine.clone()).unwrap(),
    );
    let device = coordinator
        .load_or_create_device_signing_identity(secrets)
        .unwrap();
    engine.attach_device_identity(device).unwrap();
    let knowledge = KnowledgeStore::with_event_recorder(
        vault.path().to_path_buf(),
        data.path().join("derived"),
        coordinator.clone(),
    );

    engine
        .bootstrap_existing_vault(coordinator.as_ref(), &knowledge)
        .unwrap();

    let outbox = engine.export_outbox().unwrap();
    assert_eq!(outbox.len(), 1);
    let (device_id, public_key) = engine.local_device().unwrap();
    let trusted = TrustedDevice::new(device_id, public_key).unwrap();
    let envelope = grafyn_sync_protocol::EnvelopeV1::from_json_bytes(&outbox[0]).unwrap();
    let verified = open_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        identity.descriptor.vault_id(),
        &trusted,
        &envelope,
    )
    .unwrap();
    let OperationPayloadV1::NoteRevision(revision) = verified.operation().payload() else {
        panic!("unexpected bootstrap payload");
    };
    let grafyn_sync_protocol::NoteRevisionKind::Put { markdown } = revision.kind() else {
        panic!("unexpected bootstrap tombstone");
    };
    assert_eq!(markdown.as_bytes(), shared_markdown);
    assert_eq!(
        std::fs::read(vault.path().join("private.md")).unwrap(),
        private_markdown
    );
}

#[test]
fn restart_resumes_the_exact_sealed_witness_after_a_partial_outbox_install() {
    let existing = ExistingVault::seeded();
    let (engine, coordinator, knowledge, _event_store) = existing.open_provisioned();

    assert!(engine
        .prepare_existing_vault_bootstrap(coordinator.as_ref(), &knowledge)
        .unwrap());
    assert_eq!(engine.status().unwrap().outbox_operations, 0);
    let witness_path = existing
        .data_path
        .join("sync/vaults/v1")
        .join(existing.identity.root_scope.as_str())
        .join("bootstrap/v1/prepared.json");
    let exact_witness = std::fs::read(&witness_path).unwrap();
    let witness: serde_json::Value = serde_json::from_slice(&exact_witness).unwrap();
    let mut exact_envelopes = witness["envelopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| {
            let bytes = serde_json::to_vec(value).unwrap();
            let envelope = grafyn_sync_protocol::EnvelopeV1::from_json_bytes(&bytes).unwrap();
            envelope.to_json().unwrap().into_bytes()
        })
        .collect::<Vec<_>>();
    assert_eq!(exact_envelopes.len(), 5);
    assert_eq!(witness["note_paths"]["exact-note"], "eligible.md");
    assert_eq!(witness["note_paths"][&existing.legacy_note_id], "legacy.md");
    assert_eq!(witness["note_paths"]["mapped-note"], "mapped.md");
    let first = grafyn_sync_protocol::EnvelopeV1::from_json_bytes(&exact_envelopes[0]).unwrap();
    super::operation_store::OperationStore::open(
        &existing.data_path,
        existing.identity.root_scope.clone(),
        *existing.identity.descriptor.vault_id(),
    )
    .unwrap()
    .bootstrap_outbox(&first)
    .unwrap();
    assert_eq!(engine.status().unwrap().outbox_operations, 1);
    drop(knowledge);
    drop(coordinator);
    drop(engine);

    let (reopened, coordinator, knowledge, _event_store) = existing.open_provisioned();
    assert!(reopened
        .prepare_existing_vault_bootstrap(coordinator.as_ref(), &knowledge)
        .unwrap());
    assert_eq!(std::fs::read(&witness_path).unwrap(), exact_witness);
    reopened
        .finish_existing_vault_bootstrap(coordinator.as_ref(), &knowledge)
        .unwrap();

    let mut actual = reopened.export_outbox().unwrap();
    exact_envelopes.sort();
    actual.sort();
    assert_eq!(actual, exact_envelopes);
    assert!(!witness_path.exists());
}

#[test]
fn split_finish_rejects_note_content_drift_before_ledger_or_outbox_writes() {
    let existing = ExistingVault::seeded();
    let (engine, coordinator, knowledge, _event_store) = existing.open_provisioned();

    assert!(engine
        .prepare_existing_vault_bootstrap(coordinator.as_ref(), &knowledge)
        .unwrap());
    let ledger_path = existing
        .data_path
        .join("sync/vaults/v1")
        .join(existing.identity.root_scope.as_str())
        .join("engine-ledger-v1.json");
    let ledger_before = std::fs::read(&ledger_path).unwrap();
    std::fs::write(
        existing.vault_path.join("eligible.md"),
        b"---\nnote_id: exact-note\ngrafyn_sync: inherit\n---\nchanged after prepare\n",
    )
    .unwrap();

    let error = engine
        .finish_existing_vault_bootstrap(coordinator.as_ref(), &knowledge)
        .unwrap_err();

    assert!(matches!(
        error,
        crate::services::twin_events::MutationError::RecoveryConflict(_)
    ));
    assert_eq!(engine.status().unwrap().outbox_operations, 0);
    assert_eq!(std::fs::read(ledger_path).unwrap(), ledger_before);
}

#[test]
fn split_finish_rejects_eligible_event_addition_before_outbox_writes() {
    let existing = ExistingVault::seeded();
    let (engine, coordinator, knowledge, event_store) = existing.open_provisioned();

    assert!(engine
        .prepare_existing_vault_bootstrap(coordinator.as_ref(), &knowledge)
        .unwrap());
    let latest = event_store
        .ordered_events()
        .unwrap()
        .into_iter()
        .filter(|event| event.causal_stream == CausalStream::SyncEligible)
        .max_by_key(|event| event.device_sequence)
        .unwrap();
    let added = test_support::valid_event_for_device_and_stream(
        latest.device_id.as_str(),
        CausalStream::SyncEligible,
        latest.device_sequence + 1,
        vec![latest.event_id],
    );
    event_store.append(added).unwrap();

    let error = engine
        .finish_existing_vault_bootstrap(coordinator.as_ref(), &knowledge)
        .unwrap_err();

    assert!(matches!(
        error,
        crate::services::twin_events::MutationError::RecoveryConflict(_)
    ));
    assert_eq!(engine.status().unwrap().outbox_operations, 0);
}

#[test]
fn production_bootstrap_regenerates_a_witness_after_note_content_drift() {
    let existing = ExistingVault::seeded();
    let (engine, coordinator, knowledge, _event_store) = existing.open_provisioned();

    assert!(engine
        .prepare_existing_vault_bootstrap(coordinator.as_ref(), &knowledge)
        .unwrap());
    let current_markdown =
        "---\nnote_id: exact-note\ngrafyn_sync: inherit\n---\ncurrent after prepare\n";
    std::fs::write(
        existing.vault_path.join("eligible.md"),
        current_markdown.as_bytes(),
    )
    .unwrap();

    engine
        .bootstrap_existing_vault(coordinator.as_ref(), &knowledge)
        .unwrap();

    let (device_id, public_key) = engine.local_device().unwrap();
    let trusted = TrustedDevice::new(device_id, public_key).unwrap();
    let root_key = VaultRootKey::from_bytes(ROOT_KEY_BYTES);
    let exact_note_markdown = engine
        .export_outbox()
        .unwrap()
        .into_iter()
        .find_map(|bytes| {
            let envelope = grafyn_sync_protocol::EnvelopeV1::from_json_bytes(&bytes).unwrap();
            let verified = open_operation(
                &root_key,
                existing.identity.descriptor.vault_id(),
                &trusted,
                &envelope,
            )
            .unwrap();
            match verified.operation().payload() {
                OperationPayloadV1::NoteRevision(revision)
                    if revision.note_id() == "exact-note" =>
                {
                    match revision.kind() {
                        grafyn_sync_protocol::NoteRevisionKind::Put { markdown } => {
                            Some(markdown.clone())
                        }
                        grafyn_sync_protocol::NoteRevisionKind::Tombstone => None,
                    }
                }
                _ => None,
            }
        })
        .unwrap();
    assert_eq!(exact_note_markdown, current_markdown);
}

#[test]
fn crash_resume_finish_rejects_peer_policy_and_mapping_drift_before_stale_promotion() {
    let existing = ExistingVault::seeded();
    let (engine, coordinator, knowledge, _event_store) = existing.open_provisioned();

    assert!(engine
        .prepare_existing_vault_bootstrap(coordinator.as_ref(), &knowledge)
        .unwrap());
    let witness_path = existing
        .data_path
        .join("sync/vaults/v1")
        .join(existing.identity.root_scope.as_str())
        .join("bootstrap/v1/prepared.json");
    let witness: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&witness_path).unwrap()).unwrap();
    let (device_id, public_key) = engine.local_device().unwrap();
    let trusted = TrustedDevice::new(device_id, public_key).unwrap();
    let root_key = VaultRootKey::from_bytes(ROOT_KEY_BYTES);
    let stale_mapped_operation_id = witness["envelopes"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|value| {
            let bytes = serde_json::to_vec(value).unwrap();
            let envelope = grafyn_sync_protocol::EnvelopeV1::from_json_bytes(&bytes).unwrap();
            let verified = open_operation(
                &root_key,
                existing.identity.descriptor.vault_id(),
                &trusted,
                &envelope,
            )
            .unwrap();
            matches!(
                verified.operation().payload(),
                OperationPayloadV1::NoteRevision(revision) if revision.note_id() == "mapped-note"
            )
            .then(|| envelope.operation_id().to_string())
        })
        .unwrap();

    let peer_events = Arc::new(TwinEventStore::new(&existing.data_path));
    let peer_engine = Arc::new(
        SyncEngine::open_core(
            &existing.data_path,
            &existing.vault_path,
            existing.identity.clone(),
            Some(VaultRootKey::from_bytes(ROOT_KEY_BYTES)),
            existing.secrets.clone(),
            peer_events.clone(),
        )
        .unwrap(),
    );
    let peer_coordinator = Arc::new(
        MutationCoordinator::new_stable(
            &existing.data_path,
            &existing.vault_path,
            peer_events,
            peer_engine.clone(),
        )
        .unwrap(),
    );
    let peer_device = peer_coordinator
        .load_or_create_device_signing_identity(existing.secrets.clone())
        .unwrap();
    peer_engine.attach_device_identity(peer_device).unwrap();
    let _ = peer_coordinator
        .commit_local(
            CausalStream::SyncEligible,
            SourceChannel::parse("note_editor").unwrap(),
            vec![
                TargetMutation::put(
                    TargetKind::Markdown,
                    "mapped.md",
                    "---\nnote_id: mapped-note\ngrafyn_sync: local_only\n---\npeer private edit\n",
                ),
                TargetMutation::put(
                    TargetKind::Markdown,
                    "peer-created.md",
                    "---\nnote_id: peer-created\n---\npeer shared sibling\n",
                ),
            ],
            Vec::new(),
        )
        .unwrap();
    let peer_outbox = peer_engine.export_outbox().unwrap();
    assert_eq!(peer_outbox.len(), 1);

    let error = engine
        .finish_existing_vault_bootstrap(coordinator.as_ref(), &knowledge)
        .unwrap_err();
    assert!(matches!(
        error,
        crate::services::twin_events::MutationError::RecoveryConflict(_)
    ));
    assert_eq!(engine.export_outbox().unwrap(), peer_outbox);

    engine
        .bootstrap_existing_vault(coordinator.as_ref(), &knowledge)
        .unwrap();

    let final_outbox = engine.export_outbox().unwrap();
    assert_eq!(final_outbox.len(), 5);
    let final_operation_ids = final_outbox
        .iter()
        .map(|bytes| {
            grafyn_sync_protocol::EnvelopeV1::from_json_bytes(bytes)
                .unwrap()
                .operation_id()
                .to_string()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert!(!final_operation_ids.contains(&stale_mapped_operation_id));
    let ledger: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            existing
                .data_path
                .join("sync/vaults/v1")
                .join(existing.identity.root_scope.as_str())
                .join("engine-ledger-v1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(ledger["local_only_notes"]["mapped.md"], "mapped-note");
    assert_eq!(ledger["note_paths"]["peer-created"], "peer-created.md");
}

#[test]
fn restart_drains_a_durable_pending_inbox_without_creating_an_outbox_echo() {
    let data = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("twin/events")).unwrap();
    let identity = load_or_create_vault_identity(vault.path()).unwrap();
    let secrets = Arc::new(MemorySecretStore::default());
    provision_vault_root_key(
        secrets.as_ref(),
        &identity.descriptor.vault_id().to_string(),
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
    )
    .unwrap();
    let event_store = Arc::new(TwinEventStore::new(data.path()));
    let engine = Arc::new(
        SyncEngine::open_core(
            data.path(),
            vault.path(),
            identity.clone(),
            Some(VaultRootKey::from_bytes(ROOT_KEY_BYTES)),
            secrets.clone(),
            event_store.clone(),
        )
        .unwrap(),
    );
    let coordinator = Arc::new(
        MutationCoordinator::new_stable(data.path(), vault.path(), event_store, engine.clone())
            .unwrap(),
    );
    let local_device = coordinator
        .load_or_create_device_signing_identity(secrets.clone())
        .unwrap();
    engine.attach_device_identity(local_device).unwrap();

    let remote_id = DeviceId::parse_str("123e4567-e89b-42d3-a456-4266141740c3").unwrap();
    let remote_key = DeviceSigningKey::from_seed([0xc3; 32]);
    engine
        .trust_device(remote_id, remote_key.public_key())
        .unwrap();
    let operation = OperationV1::new(
        1_800_000_000_000,
        Vec::new(),
        OperationPayloadV1::NoteRevision(
            NoteRevisionV1::put("pending.md", "recovered exactly\r\n".into()).unwrap(),
        ),
    )
    .unwrap();
    let envelope = seal_operation(
        &VaultRootKey::from_bytes(ROOT_KEY_BYTES),
        identity.descriptor.vault_id(),
        &remote_id,
        &remote_key,
        &operation,
    )
    .unwrap();
    super::operation_store::OperationStore::open(
        data.path(),
        identity.root_scope.clone(),
        *identity.descriptor.vault_id(),
    )
    .unwrap()
    .receive(&envelope)
    .unwrap();
    drop(coordinator);
    drop(engine);

    let event_store = Arc::new(TwinEventStore::new(data.path()));
    let engine = Arc::new(
        SyncEngine::open_core(
            data.path(),
            vault.path(),
            identity,
            Some(VaultRootKey::from_bytes(ROOT_KEY_BYTES)),
            secrets.clone(),
            event_store.clone(),
        )
        .unwrap(),
    );
    let coordinator = Arc::new(
        MutationCoordinator::new_stable(data.path(), vault.path(), event_store, engine.clone())
            .unwrap(),
    );
    let local_device = coordinator
        .load_or_create_device_signing_identity(secrets)
        .unwrap();
    engine.attach_device_identity(local_device).unwrap();
    assert_eq!(engine.status().unwrap().pending_operations, 1);
    assert_eq!(engine.status().unwrap().outbox_operations, 0);
    assert!(!vault.path().join("pending.md").exists());

    let report = engine.recover_pending_inbox(&coordinator).unwrap();

    assert_eq!(report.received, 0);
    assert_eq!(report.duplicates, 0);
    assert_eq!(report.applied, 1);
    assert_eq!(report.deferred, 0);
    assert_eq!(
        report.authority_token,
        Some(coordinator.current_authority_token().unwrap())
    );
    assert_eq!(
        std::fs::read(vault.path().join(format!(
            "synced/{}.md",
            crate::services::twin_events::digest_bytes(
                b"grafyn.sync.remote-path.v1:pending.md"
            )
            .as_str()
        )))
        .unwrap(),
        b"recovered exactly\r\n"
    );
    assert_eq!(engine.status().unwrap().outbox_operations, 0);
    let second = engine.recover_pending_inbox(&coordinator).unwrap();
    assert_eq!(second.applied, 0);
    assert_eq!(second.deferred, 0);
    assert!(second.authority_token.is_none());
    assert_eq!(engine.status().unwrap().outbox_operations, 0);
}
