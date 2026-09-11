use crate::models::attachment::ImageAttachmentCatalogRecordV1;
use crate::models::twin_event::{
    CausalStream, ContentDigest, EventId, EvidenceType, Sensitivity, TwinEvent, TwinEventPayload,
    Visibility,
};
use crate::services::attachment_store::{validate_generated_image_bytes, AttachmentStore};
use crate::services::sync::device::DeviceSigningIdentity;
use crate::services::sync::graph::CausalGraph;
use crate::services::sync::identity::VaultIdentity;
use crate::services::sync::operation_store::{
    OperationArea, OperationStore, OperationStoreError, StoreInsertOutcome,
    MAX_BATCH_ENVELOPE_BYTES, MAX_OPERATIONS_PER_BATCH,
};
use crate::services::sync::secrets::SecretStore;
use crate::services::sync::vault_keys::load_vault_root_key;
use crate::services::twin_events::{
    AnchoredRoot, DesiredImage, MutationCoordinator, MutationError, MutationIntentV1,
    MutationLifecycle, MutationOrigin, TargetKind, TargetMutation, TwinEventStore,
};
use crate::services::vault_namespace::VaultAuthorityTokenV1;
use grafyn_sync_protocol::{
    open_operation, seal_operation, AttachmentChunkV1, AttachmentManifestV1, DeviceId,
    DevicePublicKey, DeviceSigningKey, Digest32, EnvelopeV1, NoteRevisionKind, NoteRevisionV1,
    OperationId, OperationPayloadV1, OperationV1, TrustedDevice, TwinEventV1, VaultId,
    VaultRootKey, VerifiedOperation, ATTACHMENT_CHUNK_BYTES, MAX_ATTACHMENT_BYTES,
    MAX_ENVELOPE_JSON_BYTES,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

#[path = "bootstrap.rs"]
mod bootstrap;
mod generated_images;
mod incoming;
mod ledger;
mod local_operations;
mod state_recovery;

use generated_images::*;
use incoming::*;
use ledger::*;
use local_operations::*;
use state_recovery::*;

const ENGINE_LEDGER_SCHEMA_VERSION: u16 = 1;
const ENGINE_LEDGER_LIMIT: usize = 1024 * 1024;
const MAX_TRUSTED_DEVICES: usize = 64;
const MAX_LOCAL_ONLY_NOTES: usize = 4096;
const MAX_NOTE_PROJECTIONS: usize = 4096;
const MAX_REJECTED_OPERATIONS: usize = 4096;
const MAX_PENDING_OPERATIONS_PER_DEVICE: usize = 512;
const MAX_PENDING_BYTES_PER_DEVICE: usize = 64 * 1024 * 1024;
const MAX_STANDALONE_BATCHES: usize = 256;
const MAX_NOTE_RECONCILE_ATTEMPTS: usize = 8;
const MAX_PENDING_GENERATED_IMAGES: usize = 8;
const MAX_PENDING_GENERATED_IMAGE_BYTES: usize =
    MAX_PENDING_GENERATED_IMAGES * crate::models::image_generation::MAX_GENERATED_IMAGE_BYTES;
const RESERVED_PROGRAM_NOTE: &str = "_grafyn/program.md";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct TrustedDeviceRecordV1 {
    device_id: String,
    public_key_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct EngineLedgerV1 {
    schema_version: u16,
    trusted_devices: Vec<TrustedDeviceRecordV1>,
    local_only_notes: BTreeMap<String, String>,
    #[serde(default)]
    note_paths: BTreeMap<String, String>,
    #[serde(default)]
    rejected_operations: BTreeSet<String>,
    standalone_batches: BTreeSet<ContentDigest>,
}

impl Default for EngineLedgerV1 {
    fn default() -> Self {
        Self {
            schema_version: ENGINE_LEDGER_SCHEMA_VERSION,
            trusted_devices: Vec::new(),
            local_only_notes: BTreeMap::new(),
            note_paths: BTreeMap::new(),
            rejected_operations: BTreeSet::new(),
            standalone_batches: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncConflict {
    pub(crate) note_key: String,
    pub(crate) head_ids: Vec<OperationId>,
    pub(crate) selected_id: OperationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncDrainReport {
    pub(crate) received: usize,
    pub(crate) duplicates: usize,
    pub(crate) applied: usize,
    pub(crate) deferred: usize,
    pub(crate) authority_token: Option<VaultAuthorityTokenV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyncStatus {
    pub(crate) provisioned: bool,
    pub(crate) outbox_operations: usize,
    pub(crate) pending_operations: usize,
    pub(crate) conflicts: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeneratedImageOutboxDisposition {
    pub(crate) manifest_count: u32,
    pub(crate) chunk_count: u32,
    pub(crate) operation_count: u32,
}

struct DeviceCrypto {
    device_id: DeviceId,
    signing_key: DeviceSigningKey,
    public_key: DevicePublicKey,
}

impl Clone for DeviceCrypto {
    fn clone(&self) -> Self {
        let seed = self.signing_key.export_seed();
        Self {
            device_id: self.device_id,
            signing_key: DeviceSigningKey::from_seed(*seed),
            public_key: self.public_key,
        }
    }
}

struct EngineState {
    vault_path: PathBuf,
    vault_id: VaultId,
    vault_scope: ContentDigest,
    root_key: Option<VaultRootKey>,
    device: Option<DeviceCrypto>,
    operation_store: OperationStore,
    attachment_store: AttachmentStore,
    ledger: EngineLedgerV1,
    trusted_devices: BTreeMap<DeviceId, DevicePublicKey>,
    operations: BTreeMap<OperationId, VerifiedOperation>,
    note_heads: BTreeMap<String, BTreeSet<OperationId>>,
    note_revisions: BTreeMap<OperationId, NoteRevisionV1>,
    event_operations: BTreeMap<String, OperationId>,
    applied: BTreeSet<OperationId>,
    pending_graph: CausalGraph,
}

pub(crate) struct PreparedSyncRetarget {
    state: EngineState,
}

struct PendingGeneratedImage {
    vault_scope: ContentDigest,
    bytes: Arc<[u8]>,
    media_type: String,
    leases: usize,
}

pub(crate) struct PendingGeneratedImageLease {
    engine: Weak<SyncEngine>,
    digest: Digest32,
}

impl Drop for PendingGeneratedImageLease {
    fn drop(&mut self) {
        if let Some(engine) = self.engine.upgrade() {
            engine.release_pending_generated_image(&self.digest);
        }
    }
}

pub(crate) struct SyncEngine {
    data_path: PathBuf,
    data_root: AnchoredRoot,
    secret_store: Arc<dyn SecretStore>,
    event_store: Arc<TwinEventStore>,
    state: Mutex<EngineState>,
    pending_generated_images: Mutex<BTreeMap<Digest32, PendingGeneratedImage>>,
    #[cfg(test)]
    pause_after_remote_snapshot_once:
        Mutex<Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>>,
}

impl fmt::Debug for SyncEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock().ok();
        formatter
            .debug_struct("SyncEngine")
            .field("data_path", &self.data_path)
            .field("vault_id", &state.as_ref().map(|state| state.vault_id))
            .field(
                "provisioned",
                &state.as_ref().map(|state| state.root_key.is_some()),
            )
            .finish_non_exhaustive()
    }
}

impl SyncEngine {
    fn lock_fresh_state(
        &self,
    ) -> Result<
        (
            MutexGuard<'_, EngineState>,
            crate::services::twin_events::AnchoredExclusiveLock,
        ),
        MutationError,
    > {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MutationError::Invalid("sync engine lock poisoned".into()))?;
        let lock = self
            .data_root
            .lock_exclusive(&engine_ledger_lock_key(&state.vault_scope))?;
        refresh_engine_state(&self.data_root, &mut state)?;
        Ok((state, lock))
    }

    pub(crate) fn open_core(
        data_path: impl AsRef<Path>,
        vault_path: impl AsRef<Path>,
        identity: VaultIdentity,
        root_key: Option<VaultRootKey>,
        secret_store: Arc<dyn SecretStore>,
        event_store: Arc<TwinEventStore>,
    ) -> Result<Self, MutationError> {
        let data_path = data_path.as_ref().to_path_buf();
        let vault_path = std::fs::canonicalize(vault_path.as_ref())?;
        let data_root = AnchoredRoot::open(&data_path)?;
        let vault_id = *identity.descriptor.vault_id();
        let operation_store =
            OperationStore::open(&data_path, identity.root_scope.clone(), vault_id)
                .map_err(operation_store_error)?;
        let attachment_store = open_attachment_store(&data_path, &identity.root_scope)?;
        let ledger_key = engine_ledger_key(&identity.root_scope);
        let ledger = load_ledger(&data_root, &ledger_key)?;
        let trusted_devices = trusted_devices_from_ledger(&ledger)?;
        let engine = Self {
            data_path,
            data_root,
            secret_store,
            event_store,
            state: Mutex::new(EngineState {
                vault_path,
                vault_id,
                vault_scope: identity.root_scope,
                root_key,
                device: None,
                operation_store,
                attachment_store,
                ledger,
                trusted_devices,
                operations: BTreeMap::new(),
                note_heads: BTreeMap::new(),
                note_revisions: BTreeMap::new(),
                event_operations: BTreeMap::new(),
                applied: BTreeSet::new(),
                pending_graph: CausalGraph::new(),
            }),
            pending_generated_images: Mutex::new(BTreeMap::new()),
            #[cfg(test)]
            pause_after_remote_snapshot_once: Mutex::new(None),
        };
        engine.recover_standalone_batches()?;
        engine.rebuild_state()?;
        engine.reconcile_generated_image_catalogs()?;
        Ok(engine)
    }

    pub(crate) fn attach_device_identity(
        &self,
        device: DeviceSigningIdentity,
    ) -> Result<(), MutationError> {
        let (mut state, _engine_lock) = self.lock_fresh_state()?;
        let device_id = *device.device_id();
        let public_key = *device.public_key();
        if let Some(current) = state.device.as_ref() {
            if current.device_id == device_id && current.public_key == public_key {
                return Ok(());
            }
            return Err(MutationError::RecoveryConflict(
                "sync engine device identity is already attached".into(),
            ));
        }
        register_trusted_record(&mut state.ledger, device_id, public_key)?;
        persist_ledger(
            &self.data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )?;
        state.trusted_devices = trusted_devices_from_ledger(&state.ledger)?;
        let signing_seed = device.signing_key().export_seed();
        state.device = Some(DeviceCrypto {
            device_id,
            signing_key: DeviceSigningKey::from_seed(*signing_seed),
            public_key,
        });
        rebuild_engine_state(&self.data_root, &mut state)
    }

    pub(crate) fn status(&self) -> Result<SyncStatus, MutationError> {
        let (state, _engine_lock) = self.lock_fresh_state()?;
        let outbox_operations = state
            .operation_store
            .list(OperationArea::Outbox)
            .map_err(operation_store_error)?
            .len();
        let pending_operations = state
            .operation_store
            .list(OperationArea::Inbox)
            .map_err(operation_store_error)?
            .iter()
            .filter(|record| !state.applied.contains(record.envelope().operation_id()))
            .count();
        let conflicts = collect_conflicts(&state).len();
        Ok(SyncStatus {
            provisioned: state.root_key.is_some(),
            outbox_operations,
            pending_operations,
            conflicts,
        })
    }

    pub(crate) fn generated_image_outbox_disposition(
        &self,
        expected_scope: &ContentDigest,
        mutation_id: &ContentDigest,
        attachment_digest: &Digest32,
    ) -> Result<Option<GeneratedImageOutboxDisposition>, MutationError> {
        let (state, _engine_lock) = self.lock_fresh_state()?;
        if &state.vault_scope != expected_scope {
            return Err(MutationError::RecoveryConflict(
                "generated image outbox query crossed vault scope".into(),
            ));
        }
        let records = state
            .operation_store
            .promoted_outbox_records(mutation_id)
            .map_err(operation_store_error)?;
        let Some(root_key) = state.root_key.as_ref() else {
            if records.is_some() {
                return Err(MutationError::RecoveryConflict(
                    "unprovisioned generated image unexpectedly has a promoted outbox batch".into(),
                ));
            }
            return Ok(None);
        };
        let records = records.ok_or_else(|| {
            MutationError::RecoveryConflict(
                "provisioned generated image mutation has no promoted outbox batch".into(),
            )
        })?;
        let verified = records
            .iter()
            .map(|record| verify_envelope(&state, root_key, record.envelope()))
            .collect::<Result<Vec<_>, _>>()?;
        let manifests = verified
            .iter()
            .filter_map(|operation| match operation.operation().payload() {
                OperationPayloadV1::AttachmentManifest(manifest)
                    if manifest.attachment_digest() == attachment_digest =>
                {
                    Some((*operation.operation_id(), manifest))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let [(manifest_operation_id, manifest)] = manifests.as_slice() else {
            return Err(MutationError::RecoveryConflict(
                "generated image outbox batch does not contain exactly one matching manifest"
                    .into(),
            ));
        };
        let mut chunks = verified
            .iter()
            .filter_map(|operation| match operation.operation().payload() {
                OperationPayloadV1::AttachmentChunk(chunk)
                    if chunk.manifest_operation_id() == manifest_operation_id =>
                {
                    Some((operation, chunk))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        chunks.sort_by_key(|(_, chunk)| chunk.chunk_index());
        if chunks.len() != manifest.chunk_count() as usize {
            return Err(MutationError::RecoveryConflict(
                "generated image outbox batch has an incomplete chunk set".into(),
            ));
        }
        for (expected_index, (operation, chunk)) in chunks.iter().enumerate() {
            if chunk.chunk_index() as usize != expected_index
                || operation.operation().causal_parents() != [*manifest_operation_id]
                || chunk.attachment_digest() != attachment_digest
                || chunk.chunk_count() != manifest.chunk_count()
            {
                return Err(MutationError::RecoveryConflict(
                    "generated image outbox batch has inconsistent chunk metadata".into(),
                ));
            }
        }
        let chunk_count = u32::try_from(chunks.len())
            .map_err(|_| MutationError::Invalid("generated image chunk count overflow".into()))?;
        Ok(Some(GeneratedImageOutboxDisposition {
            manifest_count: 1,
            chunk_count,
            operation_count: chunk_count + 1,
        }))
    }

    pub(crate) fn conflicts(&self) -> Result<Vec<SyncConflict>, MutationError> {
        let (state, _engine_lock) = self.lock_fresh_state()?;
        Ok(collect_conflicts(&state))
    }

    pub(crate) fn export_outbox(&self) -> Result<Vec<Vec<u8>>, MutationError> {
        let (state, _engine_lock) = self.lock_fresh_state()?;
        state
            .operation_store
            .list(OperationArea::Outbox)
            .map_err(operation_store_error)
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| record.bytes().to_vec())
                    .collect()
            })
    }

    pub(crate) fn trust_device(
        &self,
        device_id: DeviceId,
        public_key: DevicePublicKey,
    ) -> Result<(), MutationError> {
        let (mut state, _engine_lock) = self.lock_fresh_state()?;
        register_trusted_record(&mut state.ledger, device_id, public_key)?;
        persist_ledger(
            &self.data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )?;
        state.trusted_devices = trusted_devices_from_ledger(&state.ledger)?;
        rebuild_engine_state(&self.data_root, &mut state)
    }

    pub(crate) fn local_device(&self) -> Result<(DeviceId, DevicePublicKey), MutationError> {
        let state = self
            .state
            .lock()
            .map_err(|_| MutationError::Invalid("sync engine lock poisoned".into()))?;
        state
            .device
            .as_ref()
            .map(|device| (device.device_id, device.public_key))
            .ok_or_else(|| MutationError::Invalid("sync device identity is not attached".into()))
    }

    pub(crate) fn active_vault_path(&self) -> Result<PathBuf, MutationError> {
        self.state
            .lock()
            .map_err(|_| MutationError::Invalid("sync engine lock poisoned".into()))
            .map(|state| state.vault_path.clone())
    }

    pub(crate) fn preflight_retarget(&self) -> Result<(), MutationError> {
        let (state, _engine_lock) = self.lock_fresh_state()?;
        if !state
            .operation_store
            .list_staged_batches()
            .map_err(operation_store_error)?
            .is_empty()
        {
            return Err(MutationError::RecoveryConflict(
                "sync staging must be recovered before changing vault roots".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn prepare_retarget(
        &self,
        vault_path: impl AsRef<Path>,
    ) -> Result<PreparedSyncRetarget, MutationError> {
        self.preflight_retarget()?;
        let identity = crate::services::sync::identity::load_vault_identity(vault_path.as_ref())?;
        let vault_path = std::fs::canonicalize(vault_path.as_ref())?;
        let vault_id = *identity.descriptor.vault_id();
        let root_key = load_vault_root_key(self.secret_store.as_ref(), &vault_id.to_string())
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let device = self
            .state
            .lock()
            .map_err(|_| MutationError::Invalid("sync engine lock poisoned".into()))?
            .device
            .clone();
        let operation_store =
            OperationStore::open(&self.data_path, identity.root_scope.clone(), vault_id)
                .map_err(operation_store_error)?;
        let attachment_store = open_attachment_store(&self.data_path, &identity.root_scope)?;
        let ledger_key = engine_ledger_key(&identity.root_scope);
        let _engine_lock = self
            .data_root
            .lock_exclusive(&engine_ledger_lock_key(&identity.root_scope))?;
        let mut ledger = load_ledger(&self.data_root, &ledger_key)?;
        if let Some(device) = device.as_ref() {
            register_trusted_record(&mut ledger, device.device_id, device.public_key)?;
        }
        persist_ledger(&self.data_root, &ledger_key, &ledger)?;
        let mut state = EngineState {
            vault_path,
            vault_id,
            vault_scope: identity.root_scope,
            root_key,
            device,
            operation_store,
            attachment_store,
            trusted_devices: trusted_devices_from_ledger(&ledger)?,
            ledger,
            operations: BTreeMap::new(),
            note_heads: BTreeMap::new(),
            note_revisions: BTreeMap::new(),
            event_operations: BTreeMap::new(),
            applied: BTreeSet::new(),
            pending_graph: CausalGraph::new(),
        };
        recover_standalone_batches_in_state(&self.data_root, &mut state)?;
        rebuild_engine_state(&self.data_root, &mut state)?;
        Ok(PreparedSyncRetarget { state })
    }

    pub(crate) fn activate_retarget(
        &self,
        mut prepared: PreparedSyncRetarget,
    ) -> Result<(), MutationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MutationError::Invalid("sync engine lock poisoned".into()))?;
        let _engine_lock = self
            .data_root
            .lock_exclusive(&engine_ledger_lock_key(&prepared.state.vault_scope))?;
        refresh_engine_state(&self.data_root, &mut prepared.state)?;
        *state = prepared.state;
        Ok(())
    }

    pub(crate) fn materialized_attachment(
        &self,
        digest: &Digest32,
    ) -> Result<Option<Vec<u8>>, MutationError> {
        self.state
            .lock()
            .map_err(|_| MutationError::Invalid("sync engine lock poisoned".into()))?
            .attachment_store
            .materialized_bytes(digest)
    }

    pub(crate) fn register_pending_generated_image(
        self: &Arc<Self>,
        expected_scope: &ContentDigest,
        bytes: &[u8],
        declared_media_type: &str,
    ) -> Result<(PendingGeneratedImageLease, Digest32), MutationError> {
        let validated = validate_generated_image_bytes(bytes, Some(declared_media_type))?;
        let digest = crate::models::attachment::sha256_attachment_digest(bytes);
        {
            let (state, _engine_lock) = self.lock_fresh_state()?;
            if &state.vault_scope != expected_scope {
                return Err(MutationError::RecoveryConflict(
                    "pending generated image is bound to a different vault scope".into(),
                ));
            }
        }
        let mut pending = self
            .pending_generated_images
            .lock()
            .map_err(|_| MutationError::Invalid("pending generated image lock poisoned".into()))?;
        if let Some(existing) = pending.get_mut(&digest) {
            if &existing.vault_scope != expected_scope
                || existing.bytes.as_ref() != bytes
                || existing.media_type != validated.media_type()
            {
                return Err(MutationError::RecoveryConflict(
                    "pending generated image digest collides with different bytes or scope".into(),
                ));
            }
            existing.leases = existing.leases.checked_add(1).ok_or_else(|| {
                MutationError::Invalid("pending generated image lease count overflowed".into())
            })?;
        } else {
            let current_bytes = pending.values().try_fold(0usize, |total, image| {
                total.checked_add(image.bytes.len()).ok_or_else(|| {
                    MutationError::Invalid("pending generated image bytes overflowed".into())
                })
            })?;
            if pending.len() >= MAX_PENDING_GENERATED_IMAGES
                || current_bytes
                    .checked_add(bytes.len())
                    .is_none_or(|total| total > MAX_PENDING_GENERATED_IMAGE_BYTES)
            {
                return Err(MutationError::Invalid(
                    "pending generated image capacity is exhausted".into(),
                ));
            }
            pending.insert(
                digest,
                PendingGeneratedImage {
                    vault_scope: expected_scope.clone(),
                    bytes: Arc::from(bytes),
                    media_type: validated.media_type().to_string(),
                    leases: 1,
                },
            );
        }
        Ok((
            PendingGeneratedImageLease {
                engine: Arc::downgrade(self),
                digest,
            },
            digest,
        ))
    }

    fn release_pending_generated_image(&self, digest: &Digest32) {
        let Ok(mut pending) = self.pending_generated_images.lock() else {
            return;
        };
        let remove = pending.get_mut(digest).is_some_and(|image| {
            image.leases = image.leases.saturating_sub(1);
            image.leases == 0
        });
        if remove {
            pending.remove(digest);
        }
    }

    #[cfg(test)]
    pub(crate) fn pending_generated_image_count(&self) -> usize {
        self.pending_generated_images
            .lock()
            .map_or(usize::MAX, |pending| pending.len())
    }

    #[cfg(test)]
    pub(crate) fn generated_image_stage_file_count(&self) -> Result<usize, MutationError> {
        self.lock_fresh_state()?
            .0
            .attachment_store
            .generated_image_stage_file_count()
    }

    pub(crate) fn store_cataloged_image_expecting_scope(
        &self,
        expected_scope: &ContentDigest,
        bytes: &[u8],
        declared_media_type: Option<&str>,
    ) -> Result<ImageAttachmentCatalogRecordV1, MutationError> {
        let (state, _engine_lock) = self.lock_fresh_state()?;
        if &state.vault_scope != expected_scope {
            return Err(MutationError::RecoveryConflict(
                "image attachment CAS is bound to a different vault scope".into(),
            ));
        }
        state
            .attachment_store
            .store_cataloged_image(bytes, declared_media_type)
    }

    pub(crate) fn cataloged_image_expecting_scope(
        &self,
        expected_scope: &ContentDigest,
        digest: &Digest32,
    ) -> Result<Option<ImageAttachmentCatalogRecordV1>, MutationError> {
        let (state, _engine_lock) = self.lock_fresh_state()?;
        if &state.vault_scope != expected_scope {
            return Err(MutationError::RecoveryConflict(
                "image attachment catalog is bound to a different vault scope".into(),
            ));
        }
        state.attachment_store.cataloged_image(digest)
    }

    pub(crate) fn generated_image_thumbnail_expecting_scope(
        &self,
        expected_scope: &ContentDigest,
        digest: &Digest32,
    ) -> Result<Vec<u8>, MutationError> {
        let (state, _engine_lock) = self.lock_fresh_state()?;
        if &state.vault_scope != expected_scope {
            return Err(MutationError::RecoveryConflict(
                "generated image thumbnail is bound to a different vault scope".into(),
            ));
        }
        state.attachment_store.generated_image_thumbnail(digest)
    }

    pub(crate) fn queue_attachment(
        &self,
        media_type: &str,
        bytes: &[u8],
    ) -> Result<Digest32, MutationError> {
        if bytes.is_empty() || bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(MutationError::Invalid(
                "attachment must be between 1 byte and 24 MiB".into(),
            ));
        }
        let (mut state, _engine_lock) = self.lock_fresh_state()?;
        let root_key = state.root_key.as_ref().ok_or_else(|| {
            MutationError::Invalid("sync key is not provisioned for this vault".into())
        })?;
        let device = state
            .device
            .as_ref()
            .ok_or_else(|| MutationError::Invalid("sync device identity is not attached".into()))?;
        let attachment_digest = crate::models::attachment::sha256_attachment_digest(bytes);
        let manifest = AttachmentManifestV1::new(attachment_digest, media_type, bytes.len())
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let recorded_at_unix_ms =
            u64::try_from(chrono::Utc::now().timestamp_millis()).map_err(|_| {
                MutationError::Invalid("attachment timestamp predates Unix time".into())
            })?;
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
        let mut envelopes = vec![manifest_envelope];
        for (index, chunk) in bytes.chunks(ATTACHMENT_CHUNK_BYTES).enumerate() {
            let chunk = AttachmentChunkV1::new(
                manifest_operation_id,
                attachment_digest,
                u32::try_from(index).map_err(|_| {
                    MutationError::Invalid("attachment chunk index overflow".into())
                })?,
                manifest.chunk_count(),
                chunk.to_vec(),
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
        let mutation_id = standalone_attachment_mutation_id(&manifest_operation_id);
        if state.ledger.standalone_batches.len() >= MAX_STANDALONE_BATCHES {
            return Err(MutationError::Invalid(
                "standalone sync batch limit exceeded".into(),
            ));
        }
        state.ledger.standalone_batches.insert(mutation_id.clone());
        persist_ledger(
            &self.data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )?;
        if let Err(error) = state.operation_store.stage_batch(&mutation_id, &envelopes) {
            state.ledger.standalone_batches.remove(&mutation_id);
            persist_ledger(
                &self.data_root,
                &engine_ledger_key(&state.vault_scope),
                &state.ledger,
            )?;
            return Err(operation_store_error(error));
        }
        state
            .operation_store
            .promote_batch(&mutation_id)
            .map_err(operation_store_error)?;
        materialize_standalone_attachment_batch(&state, &mutation_id, &envelopes)?;
        for envelope in &envelopes {
            state
                .operation_store
                .mark_applied(envelope.operation_id())
                .map_err(operation_store_error)?;
        }
        state.ledger.standalone_batches.remove(&mutation_id);
        persist_ledger(
            &self.data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )?;
        rebuild_engine_state(&self.data_root, &mut state)?;
        Ok(attachment_digest)
    }

    pub(crate) fn receive_envelopes(
        &self,
        coordinator: &MutationCoordinator,
        envelope_json: &[Vec<u8>],
    ) -> Result<SyncDrainReport, MutationError> {
        let mut received = 0usize;
        let mut duplicates = 0usize;
        {
            let (state, _engine_lock) = self.lock_fresh_state()?;
            let root_key = state.root_key.as_ref().ok_or_else(|| {
                MutationError::Invalid("sync key is not provisioned for this vault".into())
            })?;
            if envelope_json.len() > MAX_OPERATIONS_PER_BATCH {
                return Err(MutationError::Invalid(
                    "sync receive batch operation limit exceeded".into(),
                ));
            }
            let mut total_bytes = 0usize;
            let mut candidates = Vec::with_capacity(envelope_json.len());
            let mut envelopes = Vec::with_capacity(envelope_json.len());
            let mut seen = BTreeMap::<OperationId, Vec<u8>>::new();
            for bytes in envelope_json {
                if bytes.len() > MAX_ENVELOPE_JSON_BYTES {
                    return Err(MutationError::Invalid(
                        "sync envelope exceeds its protocol limit".into(),
                    ));
                }
                total_bytes = total_bytes.checked_add(bytes.len()).ok_or_else(|| {
                    MutationError::Invalid("sync receive batch byte count overflowed".into())
                })?;
                if total_bytes > MAX_BATCH_ENVELOPE_BYTES {
                    return Err(MutationError::Invalid(
                        "sync receive batch byte limit exceeded".into(),
                    ));
                }
                let envelope = EnvelopeV1::from_json_bytes(bytes)
                    .map_err(|error| MutationError::Invalid(error.to_string()))?;
                let canonical = envelope
                    .to_json()
                    .map_err(|error| MutationError::Invalid(error.to_string()))?
                    .into_bytes();
                if let Some(existing) = seen.get(envelope.operation_id()) {
                    if existing != &canonical {
                        return Err(MutationError::Invalid(
                            "sync receive batch contains an operation identity collision".into(),
                        ));
                    }
                    envelopes.push(envelope);
                    continue;
                }
                seen.insert(*envelope.operation_id(), canonical.clone());
                if let Some(existing) = durable_envelope_bytes(&state, envelope.operation_id())? {
                    if existing != canonical {
                        return Err(MutationError::Invalid(
                            "sync receive operation identity collides with durable bytes".into(),
                        ));
                    }
                    envelopes.push(envelope);
                    continue;
                }
                let verified = verify_envelope(&state, root_key, &envelope)?;
                validate_incoming_operation(&state, &verified)?;
                candidates.push((envelope.clone(), verified));
                envelopes.push(envelope);
            }
            validate_incoming_batch(&state, &candidates)?;
            validate_pending_device_bounds(&state, &candidates)?;
            for outcome in state
                .operation_store
                .receive_batch(&envelopes)
                .map_err(operation_store_error)?
            {
                match outcome {
                    StoreInsertOutcome::Stored => received += 1,
                    StoreInsertOutcome::Duplicate => duplicates += 1,
                }
            }
        }
        self.rebuild_state()?;
        let (applied, authority_token) = match self.drain_pending(coordinator) {
            Ok(result) => result,
            Err(error) => {
                let _ = self.rebuild_state();
                return Err(error);
            }
        };
        let deferred = self
            .lock_fresh_state()?
            .0
            .pending_graph
            .deferred_ids()
            .len();
        Ok(SyncDrainReport {
            received,
            duplicates,
            applied,
            deferred,
            authority_token,
        })
    }

    pub(crate) fn recover_pending_inbox(
        &self,
        coordinator: &MutationCoordinator,
    ) -> Result<SyncDrainReport, MutationError> {
        self.rebuild_state()?;
        let (applied, authority_token) = match self.drain_pending(coordinator) {
            Ok(result) => result,
            Err(error) => {
                let _ = self.rebuild_state();
                return Err(error);
            }
        };
        let deferred = self
            .lock_fresh_state()?
            .0
            .pending_graph
            .deferred_ids()
            .len();
        Ok(SyncDrainReport {
            received: 0,
            duplicates: 0,
            applied,
            deferred,
            authority_token,
        })
    }

    fn drain_pending(
        &self,
        coordinator: &MutationCoordinator,
    ) -> Result<(usize, Option<VaultAuthorityTokenV1>), MutationError> {
        let vault_scope = self
            .state
            .lock()
            .map_err(|_| MutationError::Invalid("sync engine lock poisoned".into()))?
            .vault_scope
            .clone();
        let _materialization_lock = self
            .data_root
            .lock_exclusive(&remote_materialization_lock_key(&vault_scope))?;
        let ready = {
            let (mut state, _engine_lock) = self.lock_fresh_state()?;
            if state.vault_scope != vault_scope {
                return Err(MutationError::RecoveryConflict(
                    "sync vault changed while acquiring the materialization lock".into(),
                ));
            }
            let applied = state.applied.clone();
            state
                .pending_graph
                .drain_ready(&applied)
                .map_err(graph_error)?
        };
        let mut applied_count = 0usize;
        let mut authority_token = None;
        for operation_id in ready {
            let mut attempts = 0usize;
            loop {
                match self.materialize_remote_operation(coordinator, operation_id) {
                    Ok(Some(token)) => authority_token = Some(token),
                    Ok(None) => {}
                    Err(error)
                        if conditional_note_conflict(&error)
                            && attempts + 1 < MAX_NOTE_RECONCILE_ATTEMPTS =>
                    {
                        attempts += 1;
                        continue;
                    }
                    Err(error) if terminal_remote_error(&error) => {
                        self.quarantine_operation(operation_id)?;
                        break;
                    }
                    Err(error) => return Err(error),
                }
                break;
            }
            let (mut state, _engine_lock) = self.lock_fresh_state()?;
            if state.applied.contains(&operation_id) {
                continue;
            }
            state
                .operation_store
                .mark_applied(&operation_id)
                .map_err(operation_store_error)?;
            state.applied.insert(operation_id);
            applied_count += 1;
        }
        self.reconcile_generated_image_catalogs()?;
        if let Some(token) = self.reconcile_note_projections(coordinator)? {
            authority_token = Some(token);
        }
        Ok((applied_count, authority_token))
    }

    fn reconcile_note_projections(
        &self,
        coordinator: &MutationCoordinator,
    ) -> Result<Option<VaultAuthorityTokenV1>, MutationError> {
        let note_ids = self
            .lock_fresh_state()?
            .0
            .note_heads
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let mut authority_token = None;
        for note_id in note_ids {
            let mut attempts = 0usize;
            loop {
                let Some(target) = self.desired_note_projection(&note_id)? else {
                    break;
                };
                let Some(expected) = self.guarded_remote_note_before_image(
                    coordinator,
                    &note_id,
                    &target.relative_key,
                )?
                else {
                    break;
                };
                match coordinator
                    .apply_nonlocal(MutationOrigin::Remote, vec![target.expecting(expected)])
                {
                    Ok(commit) => {
                        if let Some(token) = commit.authority_token {
                            authority_token = Some(token);
                        }
                        break;
                    }
                    Err(error)
                        if conditional_note_conflict(&error)
                            && attempts + 1 < MAX_NOTE_RECONCILE_ATTEMPTS =>
                    {
                        attempts += 1;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(authority_token)
    }

    fn desired_note_projection(
        &self,
        note_id: &str,
    ) -> Result<Option<TargetMutation>, MutationError> {
        let (mut state, _engine_lock) = self.lock_fresh_state()?;
        let protected = local_only_references_with_paths(
            &state.ledger.local_only_notes,
            &state.ledger.note_paths,
        );
        if is_reserved_program(note_id) || protected.contains(note_id) {
            return Ok(None);
        }
        let Some(heads) = state.note_heads.get(note_id) else {
            return Ok(None);
        };
        let Some((_, winner)) = choose_note_winner(heads, &state.note_revisions) else {
            return Ok(None);
        };
        let winner = winner.clone();
        let Some(relative_key) = try_ensure_note_projection(&self.data_root, &mut state, note_id)?
        else {
            return Ok(None);
        };
        if protected.contains(&relative_key) {
            return Ok(None);
        }
        Ok(Some(match winner.kind() {
            NoteRevisionKind::Put { markdown }
                if crate::services::knowledge_store::note_allows_sync(markdown) =>
            {
                TargetMutation::put(TargetKind::Markdown, relative_key, markdown.clone())
            }
            NoteRevisionKind::Put { .. } => return Ok(None),
            NoteRevisionKind::Tombstone => {
                TargetMutation::tombstone(TargetKind::Markdown, relative_key)
            }
        }))
    }

    fn quarantine_operation(&self, operation_id: OperationId) -> Result<(), MutationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| MutationError::Invalid("sync engine lock poisoned".into()))?;
        let _engine_lock = self
            .data_root
            .lock_exclusive(&engine_ledger_lock_key(&state.vault_scope))?;
        state.ledger = load_ledger(&self.data_root, &engine_ledger_key(&state.vault_scope))?;
        state.trusted_devices = trusted_devices_from_ledger(&state.ledger)?;
        let rejected = related_attachment_operations(&state, operation_id);
        let new_rejections = rejected
            .iter()
            .filter(|candidate| {
                !state
                    .ledger
                    .rejected_operations
                    .contains(&candidate.to_string())
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
        for candidate in &rejected {
            state
                .ledger
                .rejected_operations
                .insert(candidate.to_string());
        }
        if new_rejections > 0 {
            persist_ledger(
                &self.data_root,
                &engine_ledger_key(&state.vault_scope),
                &state.ledger,
            )?;
        }
        for candidate in rejected {
            if !state.applied.contains(&candidate) {
                state
                    .operation_store
                    .mark_applied(&candidate)
                    .map_err(operation_store_error)?;
                state.applied.insert(candidate);
            }
        }
        Ok(())
    }

    fn materialize_remote_operation(
        &self,
        coordinator: &MutationCoordinator,
        operation_id: OperationId,
    ) -> Result<Option<VaultAuthorityTokenV1>, MutationError> {
        enum Materialization {
            None,
            Note {
                note_id: String,
                target: TargetMutation,
            },
            Event(TwinEvent),
        }

        let materialization = {
            let (mut state, _engine_lock) = self.lock_fresh_state()?;
            if state.applied.contains(&operation_id) {
                return Ok(None);
            }
            let verified = state
                .operations
                .get(&operation_id)
                .cloned()
                .ok_or_else(|| {
                    MutationError::RecoveryConflict(format!(
                        "ready sync operation is unavailable: {operation_id}"
                    ))
                })?;
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
            validate_materialization_dependencies(&state, &verified)?;
            match verified.operation().payload() {
                OperationPayloadV1::NoteRevision(revision) => {
                    apply_note_operation(&mut state, &verified)?;
                    if is_reserved_program(revision.note_id()) {
                        Materialization::None
                    } else {
                        let heads = state.note_heads.get(revision.note_id()).ok_or_else(|| {
                            MutationError::RecoveryConflict(
                                "materialized note has no causal head".into(),
                            )
                        })?;
                        let winner = choose_note_winner(heads, &state.note_revisions)
                            .map(|(_, revision)| revision.clone())
                            .ok_or_else(|| {
                                MutationError::RecoveryConflict(
                                    "materialized note winner is unavailable".into(),
                                )
                            })?;
                        let relative_key =
                            ensure_note_projection(&self.data_root, &mut state, winner.note_id())?;
                        let target = match winner.kind() {
                            NoteRevisionKind::Put { markdown }
                                if crate::services::knowledge_store::note_allows_sync(markdown) =>
                            {
                                TargetMutation::put(
                                    TargetKind::Markdown,
                                    relative_key,
                                    markdown.clone(),
                                )
                            }
                            NoteRevisionKind::Put { .. } => return Ok(None),
                            NoteRevisionKind::Tombstone => {
                                TargetMutation::tombstone(TargetKind::Markdown, relative_key)
                            }
                        };
                        Materialization::Note {
                            note_id: winner.note_id().to_owned(),
                            target,
                        }
                    }
                }
                OperationPayloadV1::TwinEvent(payload) => {
                    let event = parse_synced_event(payload, verified.device_id())?;
                    validate_generated_image_event_shape(&event, true)?;
                    let mut event_parent_operations = event_dependency_ids(&event)?
                        .into_iter()
                        .map(|dependency| {
                            state
                                .event_operations
                                .get(dependency.as_str())
                                .copied()
                                .ok_or_else(|| {
                                    MutationError::Invalid(format!(
                                        "synced Twin event dependency is unavailable: {}",
                                        dependency.as_str()
                                    ))
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    event_parent_operations.sort();
                    event_parent_operations.dedup();
                    if event_parent_operations != verified.operation().causal_parents() {
                        return Err(MutationError::Invalid(
                            "synced Twin event causal parents do not match its operation".into(),
                        ));
                    }
                    state
                        .event_operations
                        .insert(event.event_id.as_str().to_owned(), operation_id);
                    Materialization::Event(event)
                }
                OperationPayloadV1::AttachmentManifest(manifest) => {
                    state
                        .attachment_store
                        .ingest_manifest(operation_id, manifest)?;
                    Materialization::None
                }
                OperationPayloadV1::AttachmentChunk(chunk) => {
                    state.attachment_store.ingest_chunk(chunk)?;
                    Materialization::None
                }
            }
        };

        match materialization {
            Materialization::None => Ok(None),
            Materialization::Note { note_id, target } => {
                let Some(expected) = self.guarded_remote_note_before_image(
                    coordinator,
                    &note_id,
                    &target.relative_key,
                )?
                else {
                    return Err(MutationError::Invalid(
                        "synced note targets freshly detected local-only content".into(),
                    ));
                };
                coordinator
                    .apply_nonlocal(MutationOrigin::Remote, vec![target.expecting(expected)])
                    .map(|commit| commit.authority_token)
            }
            Materialization::Event(event) => coordinator
                .apply_nonlocal_finalized_events(MutationOrigin::Remote, vec![event])
                .map(|commit| commit.authority_token),
        }
    }

    fn guarded_remote_note_before_image(
        &self,
        coordinator: &MutationCoordinator,
        _note_id: &str,
        relative_key: &str,
    ) -> Result<Option<crate::services::twin_events::BeforeImage>, MutationError> {
        let root_guard = coordinator.begin_root_transition()?;
        let (before, bytes) = root_guard.current_markdown_snapshot(relative_key)?;
        let allows_sync = bytes.as_ref().is_none_or(|bytes| {
            std::str::from_utf8(bytes)
                .ok()
                .is_some_and(crate::services::knowledge_store::note_allows_sync)
        });
        #[cfg(test)]
        if let Some((entered, resume)) = self
            .pause_after_remote_snapshot_once
            .lock()
            .expect("remote snapshot pause lock")
            .take()
        {
            entered.wait();
            resume.wait();
        }
        if allows_sync {
            return Ok(Some(before));
        }
        let (mut state, _engine_lock) = self.lock_fresh_state()?;
        if state.ledger.local_only_notes.len() >= MAX_LOCAL_ONLY_NOTES
            && !state.ledger.local_only_notes.contains_key(relative_key)
        {
            return Err(MutationError::Invalid(
                "local-only note policy limit exceeded".into(),
            ));
        }
        let markdown = bytes
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok());
        let local_note_id =
            local_note_id(&state.vault_scope, &state.ledger, relative_key, markdown)?;
        state
            .ledger
            .local_only_notes
            .insert(relative_key.to_owned(), local_note_id);
        persist_ledger(
            &self.data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )?;
        Ok(None)
    }

    #[cfg(test)]
    fn pause_after_remote_snapshot_once(
        &self,
        entered: Arc<std::sync::Barrier>,
        resume: Arc<std::sync::Barrier>,
    ) {
        *self
            .pause_after_remote_snapshot_once
            .lock()
            .expect("remote snapshot pause lock") = Some((entered, resume));
    }

    fn rebuild_state(&self) -> Result<(), MutationError> {
        let (_state, _engine_lock) = self.lock_fresh_state()?;
        Ok(())
    }

    fn reconcile_generated_image_catalogs(&self) -> Result<(), MutationError> {
        let events = match self.event_store.ordered_events() {
            Ok(events) => events,
            Err(crate::services::twin_events::StoreError::NotInitialized) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let digests = generated_image_attachment_digests(&events, false).map_err(|error| {
            MutationError::RecoveryConflict(format!(
                "canonical generated-image attachment binding is invalid: {error}"
            ))
        })?;
        if digests.is_empty() {
            return Ok(());
        }
        let (mut state, _engine_lock) = self.lock_fresh_state()?;
        for digest in digests {
            match state.attachment_store.cataloged_image(&digest) {
                Ok(Some(_)) => continue,
                Ok(None) => {}
                Err(error) => {
                    return Err(MutationError::RecoveryConflict(format!(
                        "canonical generated-image attachment is corrupt: {error}"
                    )))
                }
            }
            let mut matching_manifests = Vec::<(OperationId, AttachmentManifestV1, bool)>::new();
            for operation_id in &state.applied {
                let rejected = state
                    .ledger
                    .rejected_operations
                    .contains(&operation_id.to_string());
                let Some(verified) = state.operations.get(operation_id) else {
                    return Err(MutationError::RecoveryConflict(
                        "applied attachment operation disappeared".into(),
                    ));
                };
                let OperationPayloadV1::AttachmentManifest(manifest) =
                    verified.operation().payload()
                else {
                    continue;
                };
                if manifest.attachment_digest() != &digest {
                    continue;
                }
                if let Some((_, existing, _)) = matching_manifests.first() {
                    if existing.media_type() != manifest.media_type()
                        || existing.decoded_size() != manifest.decoded_size()
                        || existing.chunk_count() != manifest.chunk_count()
                    {
                        return Err(MutationError::RecoveryConflict(
                            "generated image attachment has conflicting manifests".into(),
                        ));
                    }
                }
                matching_manifests.push((*operation_id, manifest.clone(), rejected));
            }
            let Some((manifest_operation_id, manifest, _)) = matching_manifests.into_iter().find(
                |(manifest_operation_id, manifest, rejected)| {
                    applied_manifest_has_complete_chunks(
                        &state,
                        *manifest_operation_id,
                        manifest,
                        *rejected,
                    )
                },
            ) else {
                return Err(MutationError::RecoveryConflict(format!(
                    "canonical generated-image attachment {} has no complete applied manifest",
                    digest
                )));
            };
            let Some(bytes) =
                state
                    .attachment_store
                    .materialized_bytes(&digest)
                    .map_err(|error| {
                        MutationError::RecoveryConflict(format!(
                            "canonical generated-image attachment blob is invalid: {error}"
                        ))
                    })?
            else {
                return Err(MutationError::RecoveryConflict(format!(
                    "canonical generated-image attachment {} is missing its materialized blob",
                    digest
                )));
            };
            match state
                .attachment_store
                .catalog_generated_image_from_manifest(&bytes, &manifest)
            {
                Ok(_) => {}
                Err(MutationError::Invalid(_)) => {
                    quarantine_invalid_generated_image_attachment(
                        &self.data_root,
                        &mut state,
                        &digest,
                    )?;
                    continue;
                }
                Err(error) => {
                    return Err(MutationError::RecoveryConflict(format!(
                        "canonical generated-image attachment {} could not be cataloged from manifest {}: {error}",
                        digest, manifest_operation_id
                    )))
                }
            }
            if state
                .attachment_store
                .cataloged_image(&digest)
                .map_err(|error| {
                    MutationError::RecoveryConflict(format!(
                        "repaired generated-image catalog is invalid: {error}"
                    ))
                })?
                .is_none()
            {
                return Err(MutationError::RecoveryConflict(
                    "repaired generated-image catalog did not become durable".into(),
                ));
            }
        }
        Ok(())
    }

    fn recover_standalone_batches(&self) -> Result<(), MutationError> {
        let (mut state, _engine_lock) = self.lock_fresh_state()?;
        recover_standalone_batches_in_state(&self.data_root, &mut state)?;
        rebuild_engine_state(&self.data_root, &mut state)
    }

    fn stage_local_intent(&self, intent: &MutationIntentV1) -> Result<(), MutationError> {
        if intent.origin != MutationOrigin::Local {
            return Err(MutationError::Invalid(
                "only local mutations may stage sync operations".into(),
            ));
        }
        let (state, _engine_lock) = self.lock_fresh_state()?;
        validate_intent_scope(&state, intent)?;
        let generated_digests = generated_image_attachment_digests(&intent.events, false)?;
        for digest in &generated_digests {
            if state.attachment_store.cataloged_image(digest)?.is_some() {
                continue;
            }
            let (bytes, media_type) = {
                let pending = self.pending_generated_images.lock().map_err(|_| {
                    MutationError::Invalid("pending generated image lock poisoned".into())
                })?;
                let image = pending.get(digest).ok_or_else(|| {
                    MutationError::Invalid(format!(
                        "generated image mutation {} has no bounded in-memory source",
                        intent.mutation_id.as_str()
                    ))
                })?;
                if image.vault_scope != state.vault_scope {
                    return Err(MutationError::RecoveryConflict(
                        "pending generated image belongs to another vault scope".into(),
                    ));
                }
                (image.bytes.clone(), image.media_type.clone())
            };
            state.attachment_store.stage_generated_image(
                &state.vault_scope,
                &intent.mutation_id,
                &bytes,
                Some(&media_type),
            )?;
        }
        let Some(root_key) = state.root_key.as_ref() else {
            return Ok(());
        };
        let envelopes = build_local_envelopes(&state, root_key, intent)?;
        state
            .operation_store
            .stage_batch(&intent.mutation_id, &envelopes)
            .map_err(operation_store_error)?;
        Ok(())
    }

    fn commit_local_intent(&self, intent: &MutationIntentV1) -> Result<(), MutationError> {
        let (mut state, _engine_lock) = self.lock_fresh_state()?;
        validate_intent_scope(&state, intent)?;
        for digest in generated_image_attachment_digests(&intent.events, false)? {
            state.attachment_store.promote_generated_image_stage(
                &state.vault_scope,
                &intent.mutation_id,
                &digest,
            )?;
        }
        if state.root_key.is_none() {
            return Ok(());
        }
        match state.operation_store.promote_batch(&intent.mutation_id) {
            Ok(_) => {}
            Err(error) => return Err(operation_store_error(error)),
        }
        state.ledger = ledger_after_intent(&state, intent)?;
        persist_ledger(
            &self.data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )?;
        let promoted_records = state
            .operation_store
            .promoted_outbox_records(&intent.mutation_id)
            .map_err(operation_store_error)?
            .ok_or_else(|| {
                MutationError::RecoveryConflict(
                    "promoted local sync batch disappeared before it was applied".into(),
                )
            })?;
        for record in promoted_records {
            state
                .operation_store
                .mark_applied(record.envelope().operation_id())
                .map_err(operation_store_error)?;
        }
        rebuild_engine_state(&self.data_root, &mut state)
    }

    fn cancel_local_intent(&self, mutation_id: Option<&str>) -> Result<(), MutationError> {
        let Some(mutation_id) = mutation_id else {
            return Ok(());
        };
        let mutation_id =
            ContentDigest::parse(mutation_id.to_owned()).map_err(MutationError::Invalid)?;
        let (state, _engine_lock) = self.lock_fresh_state()?;
        state
            .attachment_store
            .cancel_generated_image_stage(&mutation_id)?;
        if state.root_key.is_none() {
            return Ok(());
        }
        state
            .operation_store
            .cancel_batch(&mutation_id)
            .map_err(operation_store_error)?;
        Ok(())
    }
}

fn applied_manifest_has_complete_chunks(
    state: &EngineState,
    manifest_operation_id: OperationId,
    manifest: &AttachmentManifestV1,
    rejected: bool,
) -> bool {
    if state
        .ledger
        .rejected_operations
        .contains(&manifest_operation_id.to_string())
        != rejected
    {
        return false;
    }
    let mut indices = BTreeSet::new();
    for operation_id in &state.applied {
        let Some(verified) = state.operations.get(operation_id) else {
            return false;
        };
        let OperationPayloadV1::AttachmentChunk(chunk) = verified.operation().payload() else {
            continue;
        };
        if chunk.manifest_operation_id() != &manifest_operation_id {
            continue;
        }
        if state
            .ledger
            .rejected_operations
            .contains(&operation_id.to_string())
            != rejected
            || verified.operation().causal_parents() != [manifest_operation_id]
            || chunk.attachment_digest() != manifest.attachment_digest()
            || chunk.chunk_count() != manifest.chunk_count()
            || !indices.insert(chunk.chunk_index())
        {
            return false;
        }
    }
    indices.len() == manifest.chunk_count() as usize
        && indices.iter().copied().eq(0..manifest.chunk_count())
}

impl MutationLifecycle for SyncEngine {
    fn applied_twin_event_ids(&self) -> Result<Option<BTreeSet<EventId>>, MutationError> {
        let (state, _engine_lock) = self.lock_fresh_state()?;
        if state.root_key.is_none() {
            return Ok(None);
        }
        state
            .event_operations
            .keys()
            .map(|event_id| EventId::parse(event_id.clone()).map_err(MutationError::Invalid))
            .collect::<Result<BTreeSet<_>, _>>()
            .map(Some)
    }

    fn stage_before_local(&self, intent: &MutationIntentV1) -> Result<(), MutationError> {
        self.stage_local_intent(intent)
    }

    fn committed(&self, intent: &MutationIntentV1) -> Result<(), MutationError> {
        self.commit_local_intent(intent)
    }

    fn known_failure(&self, mutation_id: Option<&str>, _reason: &str) -> Result<(), MutationError> {
        self.cancel_local_intent(mutation_id)
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
