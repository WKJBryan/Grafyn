use crate::models::twin_event::{
    CausalStream, ContentDigest, EventId, EvidenceType, Sensitivity, TwinEvent, Visibility,
};
use crate::services::attachment_store::AttachmentStore;
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
use std::sync::{Arc, Mutex, MutexGuard};

#[path = "bootstrap.rs"]
mod bootstrap;

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

pub(crate) struct SyncEngine {
    data_path: PathBuf,
    data_root: AnchoredRoot,
    secret_store: Arc<dyn SecretStore>,
    event_store: Arc<TwinEventStore>,
    state: Mutex<EngineState>,
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
            #[cfg(test)]
            pause_after_remote_snapshot_once: Mutex::new(None),
        };
        engine.recover_standalone_batches()?;
        engine.rebuild_state()?;
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
        let relative_key = ensure_note_projection(&self.data_root, &mut state, note_id)?;
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
        let Some(root_key) = state.root_key.as_ref() else {
            return Ok(());
        };
        validate_intent_scope(&state, intent)?;
        let envelopes = build_local_envelopes(&state, root_key, intent)?;
        state
            .operation_store
            .stage_batch(&intent.mutation_id, &envelopes)
            .map_err(operation_store_error)?;
        Ok(())
    }

    fn commit_local_intent(&self, intent: &MutationIntentV1) -> Result<(), MutationError> {
        let (mut state, _engine_lock) = self.lock_fresh_state()?;
        if state.root_key.is_none() {
            return Ok(());
        }
        validate_intent_scope(&state, intent)?;
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
        let outbox_ids = state
            .operation_store
            .list(OperationArea::Outbox)
            .map_err(operation_store_error)?
            .into_iter()
            .map(|record| *record.envelope().operation_id())
            .collect::<Vec<_>>();
        for operation_id in outbox_ids {
            state
                .operation_store
                .mark_applied(&operation_id)
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

impl MutationLifecycle for SyncEngine {
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

fn rebuild_engine_state(
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

fn refresh_engine_state(
    data_root: &AnchoredRoot,
    state: &mut EngineState,
) -> Result<(), MutationError> {
    state.ledger = load_ledger(data_root, &engine_ledger_key(&state.vault_scope))?;
    state.trusted_devices = trusted_devices_from_ledger(&state.ledger)?;
    rebuild_engine_state(data_root, state)
}

fn recover_standalone_batches_in_state(
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

fn standalone_attachment_mutation_id(manifest_operation_id: &OperationId) -> ContentDigest {
    crate::services::twin_events::digest_bytes(
        format!("grafyn.sync.attachment.v1:{manifest_operation_id}").as_bytes(),
    )
}

fn materialize_standalone_attachment_batch(
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

fn open_attachment_store(
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

fn validate_intent_scope(
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

fn build_local_envelopes(
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
    Ok(envelopes)
}

fn policy_after_intent(
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

fn ledger_after_intent(
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

fn local_note_id(
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

fn note_id_for_path(ledger: &EngineLedgerV1, relative_key: &str) -> Option<String> {
    ledger
        .note_paths
        .iter()
        .find_map(|(note_id, path)| (path == relative_key).then_some(note_id.clone()))
}

fn legacy_note_id(scope: &ContentDigest, relative_key: &str) -> String {
    let identity = crate::services::twin_events::digest_bytes(
        format!("grafyn.sync.note-id.v1:{}:{relative_key}", scope.as_str()).as_bytes(),
    );
    format!("legacy-{}", identity.as_str())
}

fn ensure_note_projection(
    data_root: &AnchoredRoot,
    state: &mut EngineState,
    note_id: &str,
) -> Result<String, MutationError> {
    if let Some(relative_key) = state.ledger.note_paths.get(note_id) {
        return Ok(relative_key.clone());
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
        return Err(MutationError::RecoveryConflict(
            "sync note projection collided with an existing local path".into(),
        ));
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
    Ok(relative_key)
}

fn local_only_references(policy: &BTreeMap<String, String>) -> BTreeSet<String> {
    policy
        .iter()
        .flat_map(|(path, note_id)| [path.clone(), note_id.clone()])
        .collect()
}

fn local_only_references_with_paths(
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

fn is_reserved_program(note_key: &str) -> bool {
    note_key
        .replace('\\', "/")
        .eq_ignore_ascii_case(RESERVED_PROGRAM_NOTE)
}

fn collect_conflicts(state: &EngineState) -> Vec<SyncConflict> {
    state
        .note_heads
        .iter()
        .filter(|(_, heads)| heads.len() > 1)
        .filter_map(|(note_key, heads)| {
            choose_note_winner(heads, &state.note_revisions).map(|(selected_id, _)| SyncConflict {
                note_key: note_key.clone(),
                head_ids: heads.iter().copied().collect(),
                selected_id,
            })
        })
        .collect()
}

fn reset_logical_state(state: &mut EngineState) {
    state.operations.clear();
    state.note_heads.clear();
    state.note_revisions.clear();
    state.event_operations.clear();
    state.applied.clear();
    state.pending_graph = CausalGraph::new();
}

fn verify_envelope(
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

fn validate_incoming_operation(
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

fn validate_incoming_batch(
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

fn durable_envelope_bytes(
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

fn validate_pending_device_bounds(
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

fn validate_materialization_dependencies(
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

fn validate_chunk_operation_dependencies(
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

fn related_attachment_operations(
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

fn recompute_logical_state(state: &mut EngineState) -> Result<(), MutationError> {
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

fn apply_note_operation(
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

fn parse_synced_event(
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

fn governance_allows_sync(governance: &crate::models::twin_event::Governance) -> bool {
    governance.visibility == Visibility::SyncedVault
        && governance.sensitivity != Sensitivity::Restricted
        && governance.allowed_uses.sync
}

fn engine_ledger_key(scope: &ContentDigest) -> String {
    format!("sync/vaults/v1/{}/engine-ledger-v1.json", scope.as_str())
}

fn engine_ledger_lock_key(scope: &ContentDigest) -> String {
    format!("sync/vaults/v1/{}/engine-ledger-v1.lock", scope.as_str())
}

fn remote_materialization_lock_key(scope: &ContentDigest) -> String {
    format!(
        "sync/vaults/v1/{}/remote-materialization-v1.lock",
        scope.as_str()
    )
}

fn load_ledger(root: &AnchoredRoot, key: &str) -> Result<EngineLedgerV1, MutationError> {
    let Some(bytes) = root.read_bounded(key, ENGINE_LEDGER_LIMIT)? else {
        return Ok(EngineLedgerV1::default());
    };
    let ledger: EngineLedgerV1 = serde_json::from_slice(&bytes)
        .map_err(|error| MutationError::Invalid(format!("invalid sync engine ledger: {error}")))?;
    validate_ledger(&ledger)?;
    Ok(ledger)
}

fn persist_ledger(
    root: &AnchoredRoot,
    key: &str,
    ledger: &EngineLedgerV1,
) -> Result<(), MutationError> {
    validate_ledger(ledger)?;
    let mut bytes = serde_json::to_vec_pretty(ledger)
        .map_err(|error| MutationError::Invalid(format!("invalid sync engine ledger: {error}")))?;
    bytes.push(b'\n');
    if bytes.len() > ENGINE_LEDGER_LIMIT {
        return Err(MutationError::Invalid(
            "sync engine ledger exceeds its storage limit".into(),
        ));
    }
    root.put_atomic(key, &bytes)
}

fn validate_ledger(ledger: &EngineLedgerV1) -> Result<(), MutationError> {
    if ledger.schema_version != ENGINE_LEDGER_SCHEMA_VERSION
        || ledger.trusted_devices.len() > MAX_TRUSTED_DEVICES
        || ledger.local_only_notes.len() > MAX_LOCAL_ONLY_NOTES
        || ledger.note_paths.len() > MAX_NOTE_PROJECTIONS
        || ledger.rejected_operations.len() > MAX_REJECTED_OPERATIONS
        || ledger.standalone_batches.len() > MAX_STANDALONE_BATCHES
    {
        return Err(MutationError::Invalid(
            "sync engine ledger violates its schema or bounds".into(),
        ));
    }
    let mut devices = BTreeSet::new();
    for record in &ledger.trusted_devices {
        let device = DeviceId::parse_str(&record.device_id)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        decode_public_key(&record.public_key_hex)?;
        if !devices.insert(device) {
            return Err(MutationError::Invalid(
                "sync engine ledger contains a duplicate trusted device".into(),
            ));
        }
    }
    for note_key in ledger.local_only_notes.keys() {
        crate::services::twin_events::validate_target_key(TargetKind::Markdown, note_key)?;
    }
    let mut projected_paths = BTreeSet::new();
    for (note_id, relative_key) in &ledger.note_paths {
        NoteRevisionV1::tombstone(note_id.clone())
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        crate::services::twin_events::validate_target_key(TargetKind::Markdown, relative_key)?;
        if is_reserved_program(relative_key) || !projected_paths.insert(relative_key) {
            return Err(MutationError::Invalid(
                "sync note projections contain a reserved or duplicate path".into(),
            ));
        }
    }
    for operation_id in &ledger.rejected_operations {
        OperationId::parse_hex(operation_id)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
    }
    Ok(())
}

fn register_trusted_record(
    ledger: &mut EngineLedgerV1,
    device_id: DeviceId,
    public_key: DevicePublicKey,
) -> Result<(), MutationError> {
    let device_id = device_id.to_string();
    let public_key_hex = lower_hex(public_key.as_bytes());
    if let Some(existing) = ledger
        .trusted_devices
        .iter()
        .find(|record| record.device_id == device_id)
    {
        if existing.public_key_hex == public_key_hex {
            return Ok(());
        }
        return Err(MutationError::RecoveryConflict(
            "trusted sync device public key changed".into(),
        ));
    }
    if ledger.trusted_devices.len() >= MAX_TRUSTED_DEVICES {
        return Err(MutationError::Invalid(
            "trusted sync device limit exceeded".into(),
        ));
    }
    ledger.trusted_devices.push(TrustedDeviceRecordV1 {
        device_id,
        public_key_hex,
    });
    ledger
        .trusted_devices
        .sort_by(|left, right| left.device_id.cmp(&right.device_id));
    Ok(())
}

fn trusted_devices_from_ledger(
    ledger: &EngineLedgerV1,
) -> Result<BTreeMap<DeviceId, DevicePublicKey>, MutationError> {
    ledger
        .trusted_devices
        .iter()
        .map(|record| {
            Ok((
                DeviceId::parse_str(&record.device_id)
                    .map_err(|error| MutationError::Invalid(error.to_string()))?,
                decode_public_key(&record.public_key_hex)?,
            ))
        })
        .collect()
}

fn decode_public_key(value: &str) -> Result<DevicePublicKey, MutationError> {
    if value.len() != 64 {
        return Err(MutationError::Invalid(
            "trusted device public key is invalid".into(),
        ));
    }
    let mut bytes = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        bytes[index] = (high << 4) | low;
    }
    Ok(DevicePublicKey::from_bytes(bytes))
}

fn hex_nibble(byte: u8) -> Result<u8, MutationError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(MutationError::Invalid(
            "trusted device public key is invalid".into(),
        )),
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn operation_store_error(error: impl fmt::Display) -> MutationError {
    MutationError::Io(format!("sync operation store: {error}"))
}

fn graph_error(error: impl fmt::Display) -> MutationError {
    MutationError::Invalid(format!("sync causal graph: {error}"))
}

fn terminal_remote_error(error: &MutationError) -> bool {
    matches!(
        error,
        MutationError::Invalid(_)
            | MutationError::Store(crate::services::twin_events::StoreError::Invalid(_))
    )
}

fn conditional_note_conflict(error: &MutationError) -> bool {
    matches!(
        error,
        MutationError::RecoveryConflict(message)
            if message.starts_with("conditional mutation target changed:")
    )
}

fn choose_note_winner<'a>(
    heads: &BTreeSet<OperationId>,
    revisions: &'a BTreeMap<OperationId, NoteRevisionV1>,
) -> Option<(OperationId, &'a NoteRevisionV1)> {
    heads
        .iter()
        .filter_map(|operation_id| {
            revisions
                .get(operation_id)
                .map(|revision| (*operation_id, revision))
        })
        .max_by_key(|(operation_id, revision)| {
            (
                matches!(revision.kind(), NoteRevisionKind::Tombstone),
                *operation_id,
            )
        })
}

fn event_references_local_only_note(
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

fn event_dependency_ids(event: &TwinEvent) -> Result<Vec<EventId>, MutationError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin_event::{
        EntityId, EvidenceRef, EvidenceType, Governance, NoteChangeKind, NoteChanged,
        RelationshipAssertion, RelationshipDirection, RelationshipPredicate, SourceChannel,
        TwinEventPayload,
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
            let coordinator = MutationCoordinator::new_stable(
                data.path(),
                vault.path(),
                event_store,
                engine.clone(),
            )
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
                source_id: crate::models::twin_event::Identifier::parse(relationship.as_str())
                    .unwrap(),
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
        let digest = first.engine.queue_attachment("image/png", &bytes).unwrap();
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
            assert!(remote.join().unwrap().is_ok());
            assert!(local.join().unwrap().is_ok());
        });

        assert_eq!(
            fs::read_to_string(path).unwrap(),
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
            .queue_attachment("image/png", &bytes)
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
        let manifest = AttachmentManifestV1::new(digest, "image/png", bytes.len()).unwrap();
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
}
