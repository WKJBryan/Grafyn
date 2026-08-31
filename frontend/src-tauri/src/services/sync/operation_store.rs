use crate::models::twin_event::ContentDigest;
use crate::services::twin_events::{AnchoredEntryKind, AnchoredRoot, MutationError};
use grafyn_sync_protocol::{
    EnvelopeV1, OperationId, ProtocolError, VaultId, MAX_ENVELOPE_JSON_BYTES,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

pub(crate) const MAX_OPERATIONS_PER_AREA: usize = 4096;
pub(crate) const MAX_OPERATION_BYTES_PER_AREA: usize = 256 * 1024 * 1024;
pub(crate) const MAX_STAGED_BATCHES: usize = 256;
pub(crate) const MAX_OPERATIONS_PER_BATCH: usize = 128;
pub(crate) const MAX_BATCH_ENVELOPE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_APPLIED_OPERATIONS: usize = 16 * 1024;
const MAX_BATCH_RECORD_BYTES: usize = 64 * 1024;
const MAX_APPLIED_MARKER_BYTES: usize = 256;
const MAX_SCRATCH_FILES: usize = 256;
const MAX_SCRATCH_BYTES: usize = MAX_OPERATION_BYTES_PER_AREA;
const BATCH_SCHEMA_VERSION: u16 = 1;
const APPLIED_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum OperationArea {
    Staged,
    Inbox,
    Outbox,
}

impl OperationArea {
    const ALL: [Self; 3] = [Self::Staged, Self::Inbox, Self::Outbox];

    const fn directory_name(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Inbox => "inbox",
            Self::Outbox => "outbox",
        }
    }
}

impl fmt::Display for OperationArea {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.directory_name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OperationStoreError {
    Protocol(ProtocolError),
    Filesystem(String),
    Invalid(String),
    VaultMismatch { expected: VaultId, actual: VaultId },
    Collision(OperationId),
    InvalidBatch,
    BatchCollision(ContentDigest),
    MissingBatch(ContentDigest),
    BatchCancelled(ContentDigest),
    IncompleteBatch(OperationId),
    BatchLimitExceeded,
    BatchOperationLimitExceeded,
    BatchByteLimitExceeded,
    AppliedLimitExceeded,
    UnknownOperation(OperationId),
    OperationLimitExceeded(OperationArea),
    ByteLimitExceeded(OperationArea),
}

impl fmt::Display for OperationStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => error.fmt(formatter),
            Self::Filesystem(message) | Self::Invalid(message) => formatter.write_str(message),
            Self::VaultMismatch { expected, actual } => {
                write!(
                    formatter,
                    "envelope vault {actual} does not match store vault {expected}"
                )
            }
            Self::Collision(id) => write!(formatter, "operation ID collision: {id}"),
            Self::InvalidBatch => formatter.write_str("invalid staged operation batch"),
            Self::BatchCollision(id) => {
                write!(formatter, "staged batch collision: {}", id.as_str())
            }
            Self::MissingBatch(id) => {
                write!(formatter, "staged batch is missing: {}", id.as_str())
            }
            Self::BatchCancelled(id) => {
                write!(formatter, "staged batch was cancelled: {}", id.as_str())
            }
            Self::IncompleteBatch(id) => {
                write!(formatter, "staged batch is missing operation: {id}")
            }
            Self::BatchLimitExceeded => formatter.write_str("staged batch limit exceeded"),
            Self::BatchOperationLimitExceeded => {
                formatter.write_str("staged batch operation limit exceeded")
            }
            Self::BatchByteLimitExceeded => formatter.write_str("staged batch byte limit exceeded"),
            Self::AppliedLimitExceeded => formatter.write_str("applied operation limit exceeded"),
            Self::UnknownOperation(id) => {
                write!(formatter, "cannot mark unknown operation as applied: {id}")
            }
            Self::OperationLimitExceeded(area) => {
                write!(formatter, "{area} operation limit exceeded")
            }
            Self::ByteLimitExceeded(area) => write!(formatter, "{area} byte limit exceeded"),
        }
    }
}

impl std::error::Error for OperationStoreError {}

impl From<ProtocolError> for OperationStoreError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoreInsertOutcome {
    Stored,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromotionOutcome {
    Promoted,
    AlreadyPromoted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StageBatchOutcome {
    Staged,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppliedMarkOutcome {
    Marked,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredEnvelope {
    envelope: EnvelopeV1,
    bytes: Vec<u8>,
}

impl StoredEnvelope {
    pub(crate) const fn envelope(&self) -> &EnvelopeV1 {
        &self.envelope
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedBatch {
    mutation_id: ContentDigest,
    operation_ids: Vec<OperationId>,
    envelope_digests: Vec<ContentDigest>,
}

impl StagedBatch {
    pub(crate) fn mutation_id(&self) -> &ContentDigest {
        &self.mutation_id
    }

    pub(crate) fn operation_ids(&self) -> &[OperationId] {
        &self.operation_ids
    }

    fn envelope_digest(&self, operation_id: &OperationId) -> Option<&ContentDigest> {
        self.operation_ids
            .binary_search(operation_id)
            .ok()
            .and_then(|index| self.envelope_digests.get(index))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchRecordV1 {
    schema_version: u16,
    mutation_id: ContentDigest,
    operations: Vec<BatchOperationRecordV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchOperationRecordV1 {
    operation_id: String,
    envelope_sha256: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppliedRecordV1 {
    schema_version: u16,
    operation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoreLimits {
    max_operations_per_area: usize,
    max_bytes_per_area: usize,
    max_staged_batches: usize,
    max_operations_per_batch: usize,
    max_batch_envelope_bytes: usize,
    max_applied_operations: usize,
}

impl Default for StoreLimits {
    fn default() -> Self {
        Self {
            max_operations_per_area: MAX_OPERATIONS_PER_AREA,
            max_bytes_per_area: MAX_OPERATION_BYTES_PER_AREA,
            max_staged_batches: MAX_STAGED_BATCHES,
            max_operations_per_batch: MAX_OPERATIONS_PER_BATCH,
            max_batch_envelope_bytes: MAX_BATCH_ENVELOPE_BYTES,
            max_applied_operations: MAX_APPLIED_OPERATIONS,
        }
    }
}

#[derive(Debug, Clone)]
struct StoreNamespace {
    vault_scope: ContentDigest,
}

#[derive(Debug, Clone, Copy)]
enum BatchState {
    Preparing,
    Staged,
    Promoted,
    Cancelled,
}

impl BatchState {
    const fn directory_name(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Staged => "staged",
            Self::Promoted => "promoted",
            Self::Cancelled => "cancelled",
        }
    }
}

impl StoreNamespace {
    fn base(&self) -> String {
        format!("sync/vaults/v1/{}/operations/v1", self.vault_scope.as_str())
    }

    fn area(&self, area: OperationArea) -> String {
        format!("{}/{}", self.base(), area.directory_name())
    }

    fn scratch(&self) -> String {
        format!("{}/scratch", self.base())
    }

    fn batches(&self) -> String {
        format!("{}/batches/v1", self.base())
    }

    fn batch_state(&self, state: BatchState) -> String {
        format!("{}/{}", self.batches(), state.directory_name())
    }

    fn batch_key(&self, state: BatchState, mutation_id: &ContentDigest) -> String {
        format!(
            "{}/{}/{}.json",
            self.batch_state(state),
            &mutation_id.as_str()[..2],
            mutation_id.as_str()
        )
    }

    fn applied(&self) -> String {
        format!("{}/applied/v1", self.base())
    }

    fn applied_key(&self, operation_id: &OperationId) -> String {
        let id = operation_id.to_string();
        format!("{}/{}/{}.json", self.applied(), &id[..2], id)
    }

    fn lock(&self) -> String {
        format!(
            "sync/vaults/v1/{}/operation-store-v1.lock",
            self.vault_scope.as_str()
        )
    }

    fn operation_key(&self, area: OperationArea, id: &OperationId) -> String {
        let id = id.to_string();
        format!("{}/{}/{}.json", self.area(area), &id[..2], id)
    }
}

struct AreaSnapshot {
    records: BTreeMap<OperationId, StoredEnvelope>,
    total_bytes: usize,
}

pub(crate) struct OperationStore {
    root: AnchoredRoot,
    namespace: StoreNamespace,
    vault_id: VaultId,
    limits: StoreLimits,
    process_lock: Mutex<()>,
}

impl fmt::Debug for OperationStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationStore")
            .field("root", &self.root)
            .field("namespace", &self.namespace)
            .field("vault_id", &self.vault_id)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl OperationStore {
    pub(crate) fn open(
        data_path: impl AsRef<Path>,
        vault_scope: ContentDigest,
        vault_id: VaultId,
    ) -> Result<Self, OperationStoreError> {
        Self::open_with_limits(data_path, vault_scope, vault_id, StoreLimits::default())
    }

    fn open_with_limits(
        data_path: impl AsRef<Path>,
        vault_scope: ContentDigest,
        vault_id: VaultId,
        limits: StoreLimits,
    ) -> Result<Self, OperationStoreError> {
        fs::create_dir_all(data_path.as_ref())
            .map_err(|error| OperationStoreError::Filesystem(error.to_string()))?;
        let root = AnchoredRoot::open(data_path).map_err(filesystem_error)?;
        let store = Self {
            root,
            namespace: StoreNamespace { vault_scope },
            vault_id,
            limits,
            process_lock: Mutex::new(()),
        };
        store.ensure_layout()?;
        store.with_lock(|| Ok(()))?;
        Ok(store)
    }

    pub(crate) fn stage_batch(
        &self,
        mutation_id: &ContentDigest,
        envelopes: &[EnvelopeV1],
    ) -> Result<StageBatchOutcome, OperationStoreError> {
        if envelopes.len() > self.limits.max_operations_per_batch {
            return Err(OperationStoreError::BatchOperationLimitExceeded);
        }
        let mut candidates: BTreeMap<OperationId, (EnvelopeV1, Vec<u8>)> = BTreeMap::new();
        let mut total_bytes = 0usize;
        for envelope in envelopes {
            let bytes = self.canonical_envelope_bytes(envelope)?;
            let id = *envelope.operation_id();
            match candidates.get(&id) {
                Some((_, existing_bytes)) if existing_bytes == &bytes => continue,
                Some(_) => return Err(OperationStoreError::Collision(id)),
                None => {
                    total_bytes = total_bytes
                        .checked_add(bytes.len())
                        .ok_or(OperationStoreError::BatchByteLimitExceeded)?;
                    candidates.insert(id, (envelope.clone(), bytes));
                }
            }
        }
        if candidates.len() > self.limits.max_operations_per_batch {
            return Err(OperationStoreError::BatchOperationLimitExceeded);
        }
        if total_bytes > self.limits.max_batch_envelope_bytes {
            return Err(OperationStoreError::BatchByteLimitExceeded);
        }
        let batch = StagedBatch {
            mutation_id: mutation_id.clone(),
            operation_ids: candidates.keys().copied().collect(),
            envelope_digests: candidates
                .values()
                .map(|(_, bytes)| crate::services::twin_events::digest_bytes(bytes))
                .collect(),
        };
        let batch_bytes = serialize_batch(&batch)?;

        self.with_lock(|| {
            self.validate_layout()?;
            for state in [
                BatchState::Staged,
                BatchState::Promoted,
                BatchState::Cancelled,
            ] {
                if let Some(existing) = self.read_batch(state, mutation_id)? {
                    if existing != batch {
                        return Err(batch_mismatch_error(&existing, &batch));
                    }
                    if matches!(state, BatchState::Cancelled) {
                        return Err(OperationStoreError::BatchCancelled(mutation_id.clone()));
                    }
                    for (operation_id, (_, candidate_bytes)) in &candidates {
                        let durable = self
                            .find_operation(operation_id)?
                            .ok_or(OperationStoreError::IncompleteBatch(*operation_id))?;
                        if durable.bytes() != candidate_bytes {
                            return Err(OperationStoreError::Collision(*operation_id));
                        }
                    }
                    return Ok(StageBatchOutcome::Duplicate);
                }
            }
            let active = self.scan_batches(BatchState::Staged)?;
            let promoted = self.scan_batches(BatchState::Promoted)?;
            let cancelled = self.scan_batches(BatchState::Cancelled)?;
            if active.len() >= self.limits.max_staged_batches {
                return Err(OperationStoreError::BatchLimitExceeded);
            }
            if active
                .values()
                .chain(promoted.values())
                .chain(cancelled.values())
                .any(|existing| {
                    existing
                        .operation_ids()
                        .iter()
                        .any(|id| candidates.contains_key(id))
                })
            {
                return Err(OperationStoreError::InvalidBatch);
            }
            self.install_batch_marker(BatchState::Preparing, &batch, &batch_bytes)?;
            for (envelope, bytes) in candidates.into_values() {
                self.put_locked(OperationArea::Staged, envelope, bytes)?;
            }
            self.install_batch_marker(BatchState::Staged, &batch, &batch_bytes)?;
            self.root
                .delete(
                    &self
                        .namespace
                        .batch_key(BatchState::Preparing, batch.mutation_id()),
                )
                .map_err(filesystem_error)?;
            Ok(StageBatchOutcome::Staged)
        })
    }

    pub(crate) fn list_staged_batches(&self) -> Result<Vec<StagedBatch>, OperationStoreError> {
        self.with_lock(|| {
            self.validate_layout()?;
            let staged = self.scan_batches(BatchState::Staged)?;
            let promoted = self.scan_batches(BatchState::Promoted)?;
            let cancelled = self.scan_batches(BatchState::Cancelled)?;
            for (id, batch) in &promoted {
                if cancelled.contains_key(id) {
                    return Err(OperationStoreError::BatchCollision(id.clone()));
                }
                if let Some(staged) = staged.get(id) {
                    if staged != batch {
                        return Err(batch_mismatch_error(staged, batch));
                    }
                }
                self.validate_promoted_batch(batch)?;
            }
            for (id, batch) in &cancelled {
                if let Some(staged) = staged.get(id) {
                    if staged != batch {
                        return Err(batch_mismatch_error(staged, batch));
                    }
                }
            }
            for batch in promoted.values().chain(cancelled.values()) {
                self.cleanup_staged_batch(batch)?;
            }
            Ok(staged
                .into_iter()
                .filter(|(id, _)| !promoted.contains_key(id) && !cancelled.contains_key(id))
                .map(|(_, batch)| batch)
                .collect())
        })
    }

    pub(crate) fn promote_batch(
        &self,
        mutation_id: &ContentDigest,
    ) -> Result<PromotionOutcome, OperationStoreError> {
        self.with_lock(|| {
            self.validate_layout()?;
            let staged = self.read_batch(BatchState::Staged, mutation_id)?;
            let promoted = self.read_batch(BatchState::Promoted, mutation_id)?;
            let cancelled = self.read_batch(BatchState::Cancelled, mutation_id)?;
            if let Some(cancelled) = cancelled {
                if let Some(staged) = &staged {
                    if staged != &cancelled {
                        return Err(batch_mismatch_error(staged, &cancelled));
                    }
                }
                self.cleanup_staged_batch(&cancelled)?;
                return Err(OperationStoreError::BatchCancelled(mutation_id.clone()));
            }
            if let Some(promoted) = promoted {
                if let Some(staged) = &staged {
                    if staged != &promoted {
                        return Err(batch_mismatch_error(staged, &promoted));
                    }
                }
                self.validate_promoted_batch(&promoted)?;
                self.cleanup_staged_batch(&promoted)?;
                return Ok(PromotionOutcome::AlreadyPromoted);
            }
            let batch =
                staged.ok_or_else(|| OperationStoreError::MissingBatch(mutation_id.clone()))?;

            let mut to_promote = Vec::new();
            for operation_id in batch.operation_ids() {
                let staged = self.read_record(OperationArea::Staged, operation_id)?;
                let inbox = self.read_record(OperationArea::Inbox, operation_id)?;
                let outbox = self.read_record(OperationArea::Outbox, operation_id)?;
                ensure_batch_operation_matches(&batch, operation_id, [&staged, &inbox, &outbox])?;
                match (staged, inbox, outbox) {
                    (_, _, Some(_)) | (_, Some(_), None) => {}
                    (Some(staged), None, None) => to_promote.push(staged),
                    (None, None, None) => {
                        return Err(OperationStoreError::IncompleteBatch(*operation_id))
                    }
                }
            }
            let added_bytes =
                to_promote.iter().try_fold(0usize, |total, record| {
                    total.checked_add(record.bytes.len()).ok_or(
                        OperationStoreError::ByteLimitExceeded(OperationArea::Outbox),
                    )
                })?;
            self.ensure_capacity_for(OperationArea::Outbox, to_promote.len(), added_bytes)?;
            for record in &to_promote {
                let operation_id = record.envelope.operation_id();
                let from = self
                    .namespace
                    .operation_key(OperationArea::Staged, operation_id);
                let to = self
                    .namespace
                    .operation_key(OperationArea::Outbox, operation_id);
                self.root
                    .hard_link_no_clobber(&from, &to, true)
                    .map_err(filesystem_error)?;
                let durable = self
                    .read_record(OperationArea::Outbox, operation_id)?
                    .ok_or(OperationStoreError::IncompleteBatch(*operation_id))?;
                if durable.bytes != record.bytes {
                    return Err(OperationStoreError::Collision(*operation_id));
                }
            }
            let bytes = serialize_batch(&batch)?;
            self.install_batch_marker(BatchState::Promoted, &batch, &bytes)?;
            self.cleanup_staged_batch(&batch)?;
            Ok(PromotionOutcome::Promoted)
        })
    }

    pub(crate) fn cancel_batch(
        &self,
        mutation_id: &ContentDigest,
    ) -> Result<bool, OperationStoreError> {
        self.with_lock(|| {
            self.validate_layout()?;
            if self
                .read_batch(BatchState::Promoted, mutation_id)?
                .is_some()
            {
                return Err(OperationStoreError::BatchCollision(mutation_id.clone()));
            }
            if let Some(cancelled) = self.read_batch(BatchState::Cancelled, mutation_id)? {
                self.cleanup_staged_batch(&cancelled)?;
                return Ok(false);
            }
            let Some(batch) = self.read_batch(BatchState::Staged, mutation_id)? else {
                return Ok(false);
            };
            let bytes = serialize_batch(&batch)?;
            self.install_batch_marker(BatchState::Cancelled, &batch, &bytes)?;
            self.cleanup_staged_batch(&batch)?;
            Ok(true)
        })
    }

    pub(crate) fn mark_applied(
        &self,
        operation_id: &OperationId,
    ) -> Result<AppliedMarkOutcome, OperationStoreError> {
        self.with_lock(|| {
            self.validate_layout()?;
            if self.find_operation(operation_id)?.is_none() {
                return Err(OperationStoreError::UnknownOperation(*operation_id));
            }
            if self.read_applied(operation_id)? {
                return Ok(AppliedMarkOutcome::Duplicate);
            }
            if self.scan_applied()?.len() >= self.limits.max_applied_operations {
                return Err(OperationStoreError::AppliedLimitExceeded);
            }
            let bytes = serialize_applied(operation_id)?;
            self.root
                .install_no_clobber(
                    &self.namespace.applied_key(operation_id),
                    &self.namespace.scratch(),
                    &bytes,
                )
                .map_err(filesystem_error)?;
            if !self.read_applied(operation_id)? {
                return Err(OperationStoreError::Invalid(
                    "applied marker disappeared after installation".into(),
                ));
            }
            Ok(AppliedMarkOutcome::Marked)
        })
    }

    pub(crate) fn list_applied_ids(&self) -> Result<Vec<OperationId>, OperationStoreError> {
        self.with_lock(|| {
            self.validate_layout()?;
            Ok(self.scan_applied()?.into_iter().collect())
        })
    }

    pub(crate) fn receive(
        &self,
        envelope: &EnvelopeV1,
    ) -> Result<StoreInsertOutcome, OperationStoreError> {
        let bytes = self.canonical_envelope_bytes(envelope)?;
        self.put(OperationArea::Inbox, envelope.clone(), bytes)
    }

    /// Receives one bounded caller batch after validating and preflighting the
    /// complete canonical set. Outcomes align with caller order: the first
    /// occurrence of a newly installed operation is `Stored`; byte-identical
    /// repeats and already durable operations are `Duplicate`.
    pub(crate) fn receive_batch(
        &self,
        envelopes: &[EnvelopeV1],
    ) -> Result<Vec<StoreInsertOutcome>, OperationStoreError> {
        if envelopes.len() > self.limits.max_operations_per_batch {
            return Err(OperationStoreError::BatchOperationLimitExceeded);
        }
        let mut candidates: BTreeMap<OperationId, (EnvelopeV1, Vec<u8>, usize)> = BTreeMap::new();
        let mut total_bytes = 0usize;
        for (index, envelope) in envelopes.iter().enumerate() {
            let bytes = self.canonical_envelope_bytes(envelope)?;
            total_bytes = total_bytes
                .checked_add(bytes.len())
                .ok_or(OperationStoreError::BatchByteLimitExceeded)?;
            if total_bytes > self.limits.max_batch_envelope_bytes {
                return Err(OperationStoreError::BatchByteLimitExceeded);
            }
            let operation_id = *envelope.operation_id();
            match candidates.get(&operation_id) {
                Some((_, existing_bytes, _)) if existing_bytes == &bytes => {}
                Some(_) => return Err(OperationStoreError::Collision(operation_id)),
                None => {
                    candidates.insert(operation_id, (envelope.clone(), bytes, index));
                }
            }
        }

        self.with_lock(|| {
            self.validate_layout()?;
            let mut additional_operations = 0usize;
            let mut additional_bytes = 0usize;
            for (operation_id, (_, bytes, _)) in &candidates {
                if let Some(existing) = self.find_operation(operation_id)? {
                    if existing.bytes() != bytes {
                        return Err(OperationStoreError::Collision(*operation_id));
                    }
                } else {
                    additional_operations = additional_operations.checked_add(1).ok_or(
                        OperationStoreError::OperationLimitExceeded(OperationArea::Inbox),
                    )?;
                    additional_bytes = additional_bytes
                        .checked_add(bytes.len())
                        .ok_or(OperationStoreError::ByteLimitExceeded(OperationArea::Inbox))?;
                }
            }
            self.ensure_capacity_for(
                OperationArea::Inbox,
                additional_operations,
                additional_bytes,
            )?;

            let mut outcomes = vec![StoreInsertOutcome::Duplicate; envelopes.len()];
            for (envelope, bytes, first_index) in candidates.into_values() {
                if self.put_locked(OperationArea::Inbox, envelope, bytes)?
                    == StoreInsertOutcome::Stored
                {
                    outcomes[first_index] = StoreInsertOutcome::Stored;
                }
            }
            Ok(outcomes)
        })
    }

    pub(crate) fn receive_json(
        &self,
        bytes: &[u8],
    ) -> Result<StoreInsertOutcome, OperationStoreError> {
        let envelope = EnvelopeV1::from_json_bytes(bytes)?;
        self.receive(&envelope)
    }

    /// Installs one already-witnessed bootstrap envelope directly into the
    /// immutable outbox. The caller owns the durable multi-operation witness;
    /// this primitive is intentionally exact and idempotent per operation ID.
    pub(crate) fn bootstrap_outbox(
        &self,
        envelope: &EnvelopeV1,
    ) -> Result<StoreInsertOutcome, OperationStoreError> {
        let bytes = self.canonical_envelope_bytes(envelope)?;
        self.put(OperationArea::Outbox, envelope.clone(), bytes)
    }

    pub(crate) fn load(
        &self,
        area: OperationArea,
        operation_id: &OperationId,
    ) -> Result<Option<StoredEnvelope>, OperationStoreError> {
        self.with_lock(|| {
            self.validate_layout()?;
            self.read_record(area, operation_id)
        })
    }

    pub(crate) fn list(
        &self,
        area: OperationArea,
    ) -> Result<Vec<StoredEnvelope>, OperationStoreError> {
        self.with_lock(|| {
            self.validate_layout()?;
            Ok(self.scan_area(area)?.records.into_values().collect())
        })
    }

    fn put(
        &self,
        target_area: OperationArea,
        envelope: EnvelopeV1,
        bytes: Vec<u8>,
    ) -> Result<StoreInsertOutcome, OperationStoreError> {
        self.with_lock(|| {
            self.validate_layout()?;
            self.put_locked(target_area, envelope, bytes)
        })
    }

    fn put_locked(
        &self,
        target_area: OperationArea,
        envelope: EnvelopeV1,
        bytes: Vec<u8>,
    ) -> Result<StoreInsertOutcome, OperationStoreError> {
        let operation_id = *envelope.operation_id();
        for area in OperationArea::ALL {
            if let Some(existing) = self.read_record(area, &operation_id)? {
                return if existing.bytes == bytes {
                    Ok(StoreInsertOutcome::Duplicate)
                } else {
                    Err(OperationStoreError::Collision(operation_id))
                };
            }
        }
        self.ensure_capacity(target_area, bytes.len())?;
        let destination = self.namespace.operation_key(target_area, &operation_id);
        let result = self
            .root
            .install_no_clobber_with_outcome(&destination, &self.namespace.scratch(), &bytes)
            .map_err(filesystem_error)?;
        let durable = self
            .read_record(target_area, &operation_id)?
            .ok_or_else(|| {
                OperationStoreError::Invalid(
                    "installed operation disappeared from immutable storage".into(),
                )
            })?;
        if durable.bytes != bytes {
            return Err(OperationStoreError::Collision(operation_id));
        }
        Ok(match result {
            crate::services::twin_events::NoClobberInstallOutcome::Installed => {
                StoreInsertOutcome::Stored
            }
            crate::services::twin_events::NoClobberInstallOutcome::AlreadyExists => {
                StoreInsertOutcome::Duplicate
            }
        })
    }

    fn ensure_capacity(
        &self,
        area: OperationArea,
        additional_bytes: usize,
    ) -> Result<(), OperationStoreError> {
        self.ensure_capacity_for(area, 1, additional_bytes)
    }

    fn ensure_capacity_for(
        &self,
        area: OperationArea,
        additional_operations: usize,
        additional_bytes: usize,
    ) -> Result<(), OperationStoreError> {
        let snapshot = self.scan_area(area)?;
        let count = snapshot
            .records
            .len()
            .checked_add(additional_operations)
            .ok_or(OperationStoreError::OperationLimitExceeded(area))?;
        if count > self.limits.max_operations_per_area {
            return Err(OperationStoreError::OperationLimitExceeded(area));
        }
        let total = snapshot
            .total_bytes
            .checked_add(additional_bytes)
            .ok_or(OperationStoreError::ByteLimitExceeded(area))?;
        if total > self.limits.max_bytes_per_area {
            return Err(OperationStoreError::ByteLimitExceeded(area));
        }
        Ok(())
    }

    fn scan_area(&self, area: OperationArea) -> Result<AreaSnapshot, OperationStoreError> {
        let area_directory = self.namespace.area(area);
        let prefixes = self
            .root
            .directory_entries_bounded(&area_directory, 257)
            .map_err(filesystem_error)?;
        let mut records = BTreeMap::new();
        let mut total_bytes = 0usize;
        for (prefix, kind) in prefixes {
            if kind != AnchoredEntryKind::Directory || !is_hex_prefix(&prefix) {
                return Err(OperationStoreError::Invalid(format!(
                    "invalid entry in {area} operation tree: {prefix}"
                )));
            }
            let directory = format!("{area_directory}/{prefix}");
            let leaves = self
                .root
                .directory_entries_bounded(
                    &directory,
                    self.limits.max_operations_per_area.saturating_add(1),
                )
                .map_err(filesystem_error)?;
            for (leaf, leaf_kind) in leaves {
                if leaf_kind != AnchoredEntryKind::File {
                    return Err(OperationStoreError::Invalid(format!(
                        "non-file entry in {area} operation prefix: {leaf}"
                    )));
                }
                let operation_id = operation_id_from_leaf(&prefix, &leaf)?;
                if records.len() >= self.limits.max_operations_per_area {
                    return Err(OperationStoreError::OperationLimitExceeded(area));
                }
                let record = self.read_record(area, &operation_id)?.ok_or_else(|| {
                    OperationStoreError::Invalid(format!(
                        "{area} operation disappeared during bounded enumeration"
                    ))
                })?;
                total_bytes = total_bytes
                    .checked_add(record.bytes.len())
                    .ok_or(OperationStoreError::ByteLimitExceeded(area))?;
                if total_bytes > self.limits.max_bytes_per_area {
                    return Err(OperationStoreError::ByteLimitExceeded(area));
                }
                if records.insert(operation_id, record).is_some() {
                    return Err(OperationStoreError::Collision(operation_id));
                }
            }
        }
        Ok(AreaSnapshot {
            records,
            total_bytes,
        })
    }

    fn read_record(
        &self,
        area: OperationArea,
        operation_id: &OperationId,
    ) -> Result<Option<StoredEnvelope>, OperationStoreError> {
        let key = self.namespace.operation_key(area, operation_id);
        let Some(bytes) = self
            .root
            .read_bounded(&key, MAX_ENVELOPE_JSON_BYTES)
            .map_err(filesystem_error)?
        else {
            return Ok(None);
        };
        let envelope = EnvelopeV1::from_json_bytes(&bytes)?;
        self.validate_vault(&envelope)?;
        if envelope.operation_id() != operation_id {
            return Err(OperationStoreError::Invalid(format!(
                "operation path {operation_id} contains envelope {}",
                envelope.operation_id()
            )));
        }
        let canonical = self.canonical_envelope_bytes(&envelope)?;
        if canonical != bytes {
            return Err(OperationStoreError::Invalid(format!(
                "stored operation {operation_id} is not canonical envelope JSON"
            )));
        }
        Ok(Some(StoredEnvelope { envelope, bytes }))
    }

    fn find_operation(
        &self,
        operation_id: &OperationId,
    ) -> Result<Option<StoredEnvelope>, OperationStoreError> {
        let records = OperationArea::ALL
            .into_iter()
            .map(|area| self.read_record(area, operation_id))
            .collect::<Result<Vec<_>, _>>()?;
        ensure_matching_records(operation_id, records.iter())?;
        Ok(records.into_iter().flatten().next())
    }

    fn install_batch_marker(
        &self,
        state: BatchState,
        batch: &StagedBatch,
        bytes: &[u8],
    ) -> Result<(), OperationStoreError> {
        let key = self.namespace.batch_key(state, batch.mutation_id());
        if self.read_batch(state, batch.mutation_id())?.is_none()
            && self.scan_batches(state)?.len() >= self.limits.max_staged_batches
        {
            return Err(OperationStoreError::BatchLimitExceeded);
        }
        self.root
            .install_no_clobber(&key, &self.namespace.scratch(), bytes)
            .map_err(filesystem_error)?;
        let durable = self
            .read_batch(state, batch.mutation_id())?
            .ok_or_else(|| {
                OperationStoreError::Invalid("installed batch marker disappeared".into())
            })?;
        if &durable != batch {
            return Err(batch_mismatch_error(&durable, batch));
        }
        Ok(())
    }

    fn read_batch(
        &self,
        state: BatchState,
        mutation_id: &ContentDigest,
    ) -> Result<Option<StagedBatch>, OperationStoreError> {
        let key = self.namespace.batch_key(state, mutation_id);
        let Some(bytes) = self
            .root
            .read_bounded(&key, MAX_BATCH_RECORD_BYTES)
            .map_err(filesystem_error)?
        else {
            return Ok(None);
        };
        let record: BatchRecordV1 =
            serde_json::from_slice(&bytes).map_err(|_| OperationStoreError::InvalidBatch)?;
        if record.schema_version != BATCH_SCHEMA_VERSION || &record.mutation_id != mutation_id {
            return Err(OperationStoreError::InvalidBatch);
        }
        let operation_ids = record
            .operations
            .iter()
            .map(|operation| {
                OperationId::parse_hex(&operation.operation_id).map_err(OperationStoreError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if operation_ids.len() > self.limits.max_operations_per_batch
            || operation_ids.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(OperationStoreError::InvalidBatch);
        }
        let batch = StagedBatch {
            mutation_id: record.mutation_id,
            operation_ids,
            envelope_digests: record
                .operations
                .into_iter()
                .map(|operation| operation.envelope_sha256)
                .collect(),
        };
        if serialize_batch(&batch)? != bytes {
            return Err(OperationStoreError::InvalidBatch);
        }
        Ok(Some(batch))
    }

    fn scan_batches(
        &self,
        state: BatchState,
    ) -> Result<BTreeMap<ContentDigest, StagedBatch>, OperationStoreError> {
        let base = self.namespace.batch_state(state);
        let prefixes = self
            .root
            .directory_entries_bounded(&base, 257)
            .map_err(filesystem_error)?;
        let mut batches = BTreeMap::new();
        for (prefix, kind) in prefixes {
            if kind != AnchoredEntryKind::Directory || !is_hex_prefix(&prefix) {
                return Err(OperationStoreError::InvalidBatch);
            }
            let directory = format!("{base}/{prefix}");
            let leaves = self
                .root
                .directory_entries_bounded(
                    &directory,
                    self.limits.max_staged_batches.saturating_add(1),
                )
                .map_err(filesystem_error)?;
            for (leaf, leaf_kind) in leaves {
                if leaf_kind != AnchoredEntryKind::File {
                    return Err(OperationStoreError::InvalidBatch);
                }
                let mutation_id = content_digest_from_leaf(&prefix, &leaf)?;
                if batches.len() >= self.limits.max_staged_batches {
                    return Err(OperationStoreError::BatchLimitExceeded);
                }
                let batch = self
                    .read_batch(state, &mutation_id)?
                    .ok_or(OperationStoreError::MissingBatch(mutation_id.clone()))?;
                if batches.insert(mutation_id.clone(), batch).is_some() {
                    return Err(OperationStoreError::BatchCollision(mutation_id));
                }
            }
        }
        Ok(batches)
    }

    fn cleanup_staged_batch(&self, batch: &StagedBatch) -> Result<(), OperationStoreError> {
        for operation_id in batch.operation_ids() {
            self.root
                .delete(
                    &self
                        .namespace
                        .operation_key(OperationArea::Staged, operation_id),
                )
                .map_err(filesystem_error)?;
        }
        self.root
            .delete(
                &self
                    .namespace
                    .batch_key(BatchState::Staged, batch.mutation_id()),
            )
            .map_err(filesystem_error)
    }

    fn validate_promoted_batch(&self, batch: &StagedBatch) -> Result<(), OperationStoreError> {
        for operation_id in batch.operation_ids() {
            let inbox = self.read_record(OperationArea::Inbox, operation_id)?;
            let outbox = self.read_record(OperationArea::Outbox, operation_id)?;
            ensure_batch_operation_matches(batch, operation_id, [&inbox, &outbox])?;
        }
        Ok(())
    }

    fn validate_staged_batch(&self, batch: &StagedBatch) -> Result<(), OperationStoreError> {
        for operation_id in batch.operation_ids() {
            let staged = self.read_record(OperationArea::Staged, operation_id)?;
            let inbox = self.read_record(OperationArea::Inbox, operation_id)?;
            let outbox = self.read_record(OperationArea::Outbox, operation_id)?;
            ensure_batch_operation_matches(batch, operation_id, [&staged, &inbox, &outbox])?;
        }
        Ok(())
    }

    fn read_applied(&self, operation_id: &OperationId) -> Result<bool, OperationStoreError> {
        let key = self.namespace.applied_key(operation_id);
        let Some(bytes) = self
            .root
            .read_bounded(&key, MAX_APPLIED_MARKER_BYTES)
            .map_err(filesystem_error)?
        else {
            return Ok(false);
        };
        let record: AppliedRecordV1 = serde_json::from_slice(&bytes)
            .map_err(|_| OperationStoreError::Invalid("invalid applied operation marker".into()))?;
        let parsed = OperationId::parse_hex(&record.operation_id)?;
        if record.schema_version != APPLIED_SCHEMA_VERSION
            || &parsed != operation_id
            || serialize_applied(operation_id)? != bytes
        {
            return Err(OperationStoreError::Invalid(
                "invalid applied operation marker".into(),
            ));
        }
        Ok(true)
    }

    fn scan_applied(&self) -> Result<std::collections::BTreeSet<OperationId>, OperationStoreError> {
        let base = self.namespace.applied();
        let prefixes = self
            .root
            .directory_entries_bounded(&base, 257)
            .map_err(filesystem_error)?;
        let mut applied = std::collections::BTreeSet::new();
        for (prefix, kind) in prefixes {
            if kind != AnchoredEntryKind::Directory || !is_hex_prefix(&prefix) {
                return Err(OperationStoreError::Invalid(
                    "invalid applied operation tree".into(),
                ));
            }
            let directory = format!("{base}/{prefix}");
            let leaves = self
                .root
                .directory_entries_bounded(
                    &directory,
                    self.limits.max_applied_operations.saturating_add(1),
                )
                .map_err(filesystem_error)?;
            for (leaf, leaf_kind) in leaves {
                if leaf_kind != AnchoredEntryKind::File {
                    return Err(OperationStoreError::Invalid(
                        "invalid applied operation entry".into(),
                    ));
                }
                let operation_id = operation_id_from_leaf(&prefix, &leaf)?;
                if applied.len() >= self.limits.max_applied_operations {
                    return Err(OperationStoreError::AppliedLimitExceeded);
                }
                if !self.read_applied(&operation_id)? || !applied.insert(operation_id) {
                    return Err(OperationStoreError::Invalid(
                        "duplicate applied operation marker".into(),
                    ));
                }
            }
        }
        Ok(applied)
    }

    fn canonical_envelope_bytes(
        &self,
        envelope: &EnvelopeV1,
    ) -> Result<Vec<u8>, OperationStoreError> {
        self.validate_vault(envelope)?;
        let bytes = envelope.to_json()?.into_bytes();
        if bytes.is_empty() || bytes.len() > MAX_ENVELOPE_JSON_BYTES {
            return Err(OperationStoreError::Protocol(ProtocolError::JsonTooLarge));
        }
        Ok(bytes)
    }

    fn validate_vault(&self, envelope: &EnvelopeV1) -> Result<(), OperationStoreError> {
        if envelope.vault_id() != &self.vault_id {
            return Err(OperationStoreError::VaultMismatch {
                expected: self.vault_id,
                actual: *envelope.vault_id(),
            });
        }
        Ok(())
    }

    fn ensure_layout(&self) -> Result<(), OperationStoreError> {
        for directory in [
            "sync".to_string(),
            "sync/vaults".to_string(),
            "sync/vaults/v1".to_string(),
            format!("sync/vaults/v1/{}", self.namespace.vault_scope.as_str()),
            self.namespace.base(),
            self.namespace.area(OperationArea::Staged),
            self.namespace.area(OperationArea::Inbox),
            self.namespace.area(OperationArea::Outbox),
            self.namespace.scratch(),
            self.namespace.batches(),
            self.namespace.batch_state(BatchState::Preparing),
            self.namespace.batch_state(BatchState::Staged),
            self.namespace.batch_state(BatchState::Promoted),
            self.namespace.batch_state(BatchState::Cancelled),
            self.namespace.applied(),
        ] {
            self.root
                .open_directory(&directory, true)
                .map_err(filesystem_error)?;
        }
        Ok(())
    }

    fn validate_layout(&self) -> Result<(), OperationStoreError> {
        for directory in [
            self.namespace.base(),
            self.namespace.area(OperationArea::Staged),
            self.namespace.area(OperationArea::Inbox),
            self.namespace.area(OperationArea::Outbox),
            self.namespace.scratch(),
            self.namespace.batches(),
            self.namespace.batch_state(BatchState::Preparing),
            self.namespace.batch_state(BatchState::Staged),
            self.namespace.batch_state(BatchState::Promoted),
            self.namespace.batch_state(BatchState::Cancelled),
            self.namespace.applied(),
        ] {
            self.root
                .open_directory(&directory, false)
                .map_err(filesystem_error)?;
        }
        self.validate_scratch()
    }

    fn validate_scratch(&self) -> Result<(), OperationStoreError> {
        let scratch = self.namespace.scratch();
        let entries = self
            .root
            .directory_entries_bounded(&scratch, MAX_SCRATCH_FILES.saturating_add(1))
            .map_err(filesystem_error)?;
        if entries.len() > MAX_SCRATCH_FILES {
            return Err(OperationStoreError::Invalid(
                "operation scratch file limit exceeded".into(),
            ));
        }
        let mut total_bytes = 0usize;
        for (name, kind) in entries {
            let uuid = name
                .strip_prefix('.')
                .and_then(|name| name.strip_suffix(".tmp"));
            if kind != AnchoredEntryKind::File
                || uuid.and_then(|value| Uuid::parse_str(value).ok()).is_none()
            {
                return Err(OperationStoreError::Invalid(
                    "invalid operation scratch entry".into(),
                ));
            }
            let bytes = self
                .root
                .read_bounded(&format!("{scratch}/{name}"), MAX_ENVELOPE_JSON_BYTES)
                .map_err(filesystem_error)?
                .ok_or_else(|| {
                    OperationStoreError::Invalid(
                        "operation scratch entry disappeared during validation".into(),
                    )
                })?;
            total_bytes = total_bytes.checked_add(bytes.len()).ok_or_else(|| {
                OperationStoreError::Invalid("operation scratch byte limit exceeded".into())
            })?;
            if total_bytes > MAX_SCRATCH_BYTES {
                return Err(OperationStoreError::Invalid(
                    "operation scratch byte limit exceeded".into(),
                ));
            }
        }
        Ok(())
    }

    fn reconcile_batches_locked(&self) -> Result<(), OperationStoreError> {
        let preparing = self.scan_batches(BatchState::Preparing)?;
        let staged = self.scan_batches(BatchState::Staged)?;
        let promoted = self.scan_batches(BatchState::Promoted)?;
        let cancelled = self.scan_batches(BatchState::Cancelled)?;

        for (id, batch) in &preparing {
            for other in [staged.get(id), promoted.get(id), cancelled.get(id)]
                .into_iter()
                .flatten()
            {
                if other != batch {
                    return Err(batch_mismatch_error(other, batch));
                }
            }
        }
        for (id, batch) in &staged {
            for other in [promoted.get(id), cancelled.get(id)].into_iter().flatten() {
                if other != batch {
                    return Err(batch_mismatch_error(other, batch));
                }
            }
        }
        for id in promoted.keys() {
            if cancelled.contains_key(id) {
                return Err(OperationStoreError::BatchCollision(id.clone()));
            }
        }
        for batch in promoted.values() {
            self.validate_promoted_batch(batch)?;
        }
        for (id, batch) in &staged {
            if !promoted.contains_key(id) && !cancelled.contains_key(id) {
                self.validate_staged_batch(batch)?;
            }
        }

        for (id, batch) in &preparing {
            if !staged.contains_key(id) && !promoted.contains_key(id) && !cancelled.contains_key(id)
            {
                let bytes = serialize_batch(batch)?;
                self.install_batch_marker(BatchState::Cancelled, batch, &bytes)?;
            }
        }
        for batch in promoted.values().chain(cancelled.values()) {
            self.cleanup_staged_batch(batch)?;
            self.root
                .delete(
                    &self
                        .namespace
                        .batch_key(BatchState::Preparing, batch.mutation_id()),
                )
                .map_err(filesystem_error)?;
        }
        for (id, batch) in &preparing {
            if staged.get(id) == Some(batch) {
                self.root
                    .delete(&self.namespace.batch_key(BatchState::Preparing, id))
                    .map_err(filesystem_error)?;
            } else if !promoted.contains_key(id) && !cancelled.contains_key(id) {
                self.cleanup_staged_batch(batch)?;
                self.root
                    .delete(&self.namespace.batch_key(BatchState::Preparing, id))
                    .map_err(filesystem_error)?;
            }
        }

        let active_ids: std::collections::BTreeSet<_> = staged
            .iter()
            .filter(|(id, _)| !promoted.contains_key(*id) && !cancelled.contains_key(*id))
            .flat_map(|(_, batch)| batch.operation_ids().iter().copied())
            .collect();
        for operation_id in self.scan_area(OperationArea::Staged)?.records.keys() {
            if !active_ids.contains(operation_id) {
                self.root
                    .delete(
                        &self
                            .namespace
                            .operation_key(OperationArea::Staged, operation_id),
                    )
                    .map_err(filesystem_error)?;
            }
        }
        Ok(())
    }

    fn with_lock<T>(
        &self,
        operation: impl FnOnce() -> Result<T, OperationStoreError>,
    ) -> Result<T, OperationStoreError> {
        let _process_guard = self
            .process_lock
            .lock()
            .map_err(|_| OperationStoreError::Invalid("operation store lock poisoned".into()))?;
        let lock = self
            .root
            .lock_exclusive(&self.namespace.lock())
            .map_err(filesystem_error)?;
        let result = self
            .validate_layout()
            .and_then(|_| self.reconcile_batches_locked())
            .and_then(|_| operation());
        let unlock = lock
            .unlock()
            .map_err(|error| OperationStoreError::Filesystem(error.to_string()));
        match result {
            Ok(value) => {
                unlock?;
                Ok(value)
            }
            Err(error) => {
                let _ = unlock;
                Err(error)
            }
        }
    }
}

fn serialize_batch(batch: &StagedBatch) -> Result<Vec<u8>, OperationStoreError> {
    if batch.operation_ids.len() != batch.envelope_digests.len() {
        return Err(OperationStoreError::InvalidBatch);
    }
    let record = BatchRecordV1 {
        schema_version: BATCH_SCHEMA_VERSION,
        mutation_id: batch.mutation_id.clone(),
        operations: batch
            .operation_ids
            .iter()
            .zip(&batch.envelope_digests)
            .map(|(operation_id, envelope_sha256)| BatchOperationRecordV1 {
                operation_id: operation_id.to_string(),
                envelope_sha256: envelope_sha256.clone(),
            })
            .collect(),
    };
    let bytes = serde_json::to_vec(&record).map_err(|_| OperationStoreError::InvalidBatch)?;
    if bytes.len() > MAX_BATCH_RECORD_BYTES {
        return Err(OperationStoreError::InvalidBatch);
    }
    Ok(bytes)
}

fn serialize_applied(operation_id: &OperationId) -> Result<Vec<u8>, OperationStoreError> {
    let bytes = serde_json::to_vec(&AppliedRecordV1 {
        schema_version: APPLIED_SCHEMA_VERSION,
        operation_id: operation_id.to_string(),
    })
    .map_err(|error| OperationStoreError::Invalid(error.to_string()))?;
    if bytes.len() > MAX_APPLIED_MARKER_BYTES {
        return Err(OperationStoreError::Invalid(
            "applied operation marker exceeds its byte limit".into(),
        ));
    }
    Ok(bytes)
}

fn ensure_matching_records<'a>(
    operation_id: &OperationId,
    records: impl IntoIterator<Item = &'a Option<StoredEnvelope>>,
) -> Result<(), OperationStoreError> {
    let mut expected: Option<&[u8]> = None;
    for record in records.into_iter().flatten() {
        match expected {
            Some(bytes) if bytes != record.bytes() => {
                return Err(OperationStoreError::Collision(*operation_id))
            }
            Some(_) => {}
            None => expected = Some(record.bytes()),
        }
    }
    Ok(())
}

fn ensure_batch_operation_matches<'a>(
    batch: &StagedBatch,
    operation_id: &OperationId,
    records: impl IntoIterator<Item = &'a Option<StoredEnvelope>>,
) -> Result<(), OperationStoreError> {
    let records = records.into_iter().collect::<Vec<_>>();
    ensure_matching_records(operation_id, records.iter().copied())?;
    let record = records
        .into_iter()
        .find_map(Option::as_ref)
        .ok_or(OperationStoreError::IncompleteBatch(*operation_id))?;
    let expected = batch
        .envelope_digest(operation_id)
        .ok_or(OperationStoreError::InvalidBatch)?;
    if &crate::services::twin_events::digest_bytes(record.bytes()) != expected {
        return Err(OperationStoreError::Collision(*operation_id));
    }
    Ok(())
}

fn batch_mismatch_error(expected: &StagedBatch, actual: &StagedBatch) -> OperationStoreError {
    if expected.mutation_id() == actual.mutation_id()
        && expected.operation_ids() == actual.operation_ids()
    {
        for (index, operation_id) in expected.operation_ids().iter().enumerate() {
            if expected.envelope_digests.get(index) != actual.envelope_digests.get(index) {
                return OperationStoreError::Collision(*operation_id);
            }
        }
    }
    OperationStoreError::BatchCollision(actual.mutation_id().clone())
}

fn is_hex_prefix(value: &str) -> bool {
    value.len() == 2
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn content_digest_from_leaf(
    expected_prefix: &str,
    leaf: &str,
) -> Result<ContentDigest, OperationStoreError> {
    let id = leaf
        .strip_suffix(".json")
        .ok_or(OperationStoreError::InvalidBatch)?;
    let digest =
        ContentDigest::parse(id.to_string()).map_err(|_| OperationStoreError::InvalidBatch)?;
    if &id[..2] != expected_prefix {
        return Err(OperationStoreError::InvalidBatch);
    }
    Ok(digest)
}

fn operation_id_from_leaf(
    expected_prefix: &str,
    leaf: &str,
) -> Result<OperationId, OperationStoreError> {
    let id = leaf.strip_suffix(".json").ok_or_else(|| {
        OperationStoreError::Invalid("operation filename must end in .json".into())
    })?;
    let operation_id = OperationId::parse_hex(id)?;
    if &id[..2] != expected_prefix {
        return Err(OperationStoreError::Invalid(
            "operation filename is stored below the wrong prefix".into(),
        ));
    }
    Ok(operation_id)
}

fn filesystem_error(error: MutationError) -> OperationStoreError {
    OperationStoreError::Filesystem(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin_event::ContentDigest;
    use grafyn_sync_protocol::{
        seal_operation, DeviceId, DeviceSigningKey, NoteRevisionV1, OperationPayloadV1,
        OperationV1, VaultRootKey,
    };

    const VAULT_ID: &str = "123e4567-e89b-42d3-a456-426614174000";

    fn vault_id() -> VaultId {
        VaultId::parse_str(VAULT_ID).unwrap()
    }

    fn scope() -> ContentDigest {
        ContentDigest::parse("ab".repeat(32)).unwrap()
    }

    fn mutation_id(byte: u8) -> ContentDigest {
        ContentDigest::parse(format!("{byte:02x}").repeat(32)).unwrap()
    }

    fn envelope(recorded_at: u64, markdown: &str) -> EnvelopeV1 {
        let operation = OperationV1::new(
            recorded_at,
            vec![],
            OperationPayloadV1::NoteRevision(
                NoteRevisionV1::put("note", markdown.to_string()).unwrap(),
            ),
        )
        .unwrap();
        seal_operation(
            &VaultRootKey::from_bytes([7; 32]),
            &vault_id(),
            &DeviceId::parse_str("123e4567-e89b-42d3-a456-426614174001").unwrap(),
            &DeviceSigningKey::from_seed([9; 32]),
            &operation,
        )
        .unwrap()
    }

    fn store(temp: &tempfile::TempDir) -> OperationStore {
        OperationStore::open(temp.path(), scope(), vault_id()).unwrap()
    }

    #[test]
    fn valid_json_is_canonicalized_and_an_exact_duplicate_is_a_noop() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let envelope = envelope(1, "first");
        let pretty = serde_json::to_vec_pretty(&envelope).unwrap();

        assert_eq!(
            store.receive_json(&pretty).unwrap(),
            StoreInsertOutcome::Stored
        );
        assert_eq!(
            store.receive(&envelope).unwrap(),
            StoreInsertOutcome::Duplicate
        );

        let stored = store
            .load(OperationArea::Inbox, envelope.operation_id())
            .unwrap()
            .unwrap();
        assert_eq!(stored.bytes(), envelope.to_json().unwrap().as_bytes());
        assert_eq!(stored.envelope(), &envelope);
    }

    #[test]
    fn receive_batch_deduplicates_before_writes_and_preserves_input_outcomes() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let existing = envelope(1, "existing");
        let new = envelope(2, "new");
        store.receive(&existing).unwrap();

        let batch = [new.clone(), existing.clone(), new.clone(), existing.clone()];
        assert_eq!(
            store.receive_batch(&batch).unwrap(),
            vec![
                StoreInsertOutcome::Stored,
                StoreInsertOutcome::Duplicate,
                StoreInsertOutcome::Duplicate,
                StoreInsertOutcome::Duplicate,
            ]
        );
        assert_eq!(
            store.receive_batch(&batch).unwrap(),
            vec![StoreInsertOutcome::Duplicate; batch.len()]
        );
        assert_eq!(store.list(OperationArea::Inbox).unwrap().len(), 2);
    }

    #[test]
    fn receive_batch_rejects_same_id_different_bytes_without_partial_write() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let unrelated = envelope(2, "must not be written");
        let first = envelope(1, "same operation");
        let resealed = envelope(1, "same operation");
        assert_eq!(first.operation_id(), resealed.operation_id());
        assert_ne!(first.to_json().unwrap(), resealed.to_json().unwrap());

        assert_eq!(
            store
                .receive_batch(&[unrelated, first.clone(), resealed])
                .unwrap_err(),
            OperationStoreError::Collision(*first.operation_id())
        );
        assert!(store.list(OperationArea::Inbox).unwrap().is_empty());
    }

    #[test]
    fn receive_batch_preflights_a_durable_collision_without_partial_write() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let durable = envelope(1, "same operation");
        let resealed = envelope(1, "same operation");
        let unrelated = (2..=512)
            .map(|recorded_at| envelope(recorded_at, "must not be written"))
            .find(|candidate| candidate.operation_id() < durable.operation_id())
            .unwrap();
        assert_eq!(durable.operation_id(), resealed.operation_id());
        assert_ne!(durable.to_json().unwrap(), resealed.to_json().unwrap());
        store.receive(&durable).unwrap();

        assert_eq!(
            store.receive_batch(&[unrelated, resealed]).unwrap_err(),
            OperationStoreError::Collision(*durable.operation_id())
        );
        let inbox = store.list(OperationArea::Inbox).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].bytes(), durable.to_json().unwrap().as_bytes());
    }

    #[test]
    fn receive_batch_bounds_the_raw_call_before_deduplication() {
        let envelope = envelope(1, "duplicate input");
        let count_temp = tempfile::tempdir().unwrap();
        let count_store = OperationStore::open_with_limits(
            count_temp.path(),
            scope(),
            vault_id(),
            StoreLimits {
                max_operations_per_batch: 1,
                max_batch_envelope_bytes: usize::MAX,
                ..StoreLimits::default()
            },
        )
        .unwrap();
        assert_eq!(
            count_store
                .receive_batch(&[envelope.clone(), envelope.clone()])
                .unwrap_err(),
            OperationStoreError::BatchOperationLimitExceeded
        );
        assert!(count_store.list(OperationArea::Inbox).unwrap().is_empty());

        let byte_temp = tempfile::tempdir().unwrap();
        let byte_store = OperationStore::open_with_limits(
            byte_temp.path(),
            scope(),
            vault_id(),
            StoreLimits {
                max_operations_per_batch: 2,
                max_batch_envelope_bytes: envelope.to_json().unwrap().len(),
                ..StoreLimits::default()
            },
        )
        .unwrap();
        assert_eq!(
            byte_store
                .receive_batch(&[envelope.clone(), envelope])
                .unwrap_err(),
            OperationStoreError::BatchByteLimitExceeded
        );
        assert!(byte_store.list(OperationArea::Inbox).unwrap().is_empty());
    }

    #[test]
    fn receive_batch_preflights_inbox_count_capacity_as_a_whole() {
        let temp = tempfile::tempdir().unwrap();
        let store = OperationStore::open_with_limits(
            temp.path(),
            scope(),
            vault_id(),
            StoreLimits {
                max_operations_per_area: 2,
                max_bytes_per_area: usize::MAX,
                ..StoreLimits::default()
            },
        )
        .unwrap();
        let existing = envelope(1, "existing");
        let first = envelope(2, "first new");
        let second = envelope(3, "second new");
        store.receive(&existing).unwrap();

        assert_eq!(
            store.receive_batch(&[first, second]).unwrap_err(),
            OperationStoreError::OperationLimitExceeded(OperationArea::Inbox)
        );
        let inbox = store.list(OperationArea::Inbox).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].envelope(), &existing);
    }

    #[test]
    fn receive_batch_preflights_inbox_byte_capacity_as_a_whole() {
        let temp = tempfile::tempdir().unwrap();
        let existing = envelope(1, "existing");
        let first = envelope(2, "first new");
        let second = envelope(3, "second new");
        let max_bytes = existing.to_json().unwrap().len()
            + first.to_json().unwrap().len()
            + second.to_json().unwrap().len()
            - 1;
        let store = OperationStore::open_with_limits(
            temp.path(),
            scope(),
            vault_id(),
            StoreLimits {
                max_operations_per_area: 3,
                max_bytes_per_area: max_bytes,
                ..StoreLimits::default()
            },
        )
        .unwrap();
        store.receive(&existing).unwrap();

        assert_eq!(
            store.receive_batch(&[first, second]).unwrap_err(),
            OperationStoreError::ByteLimitExceeded(OperationArea::Inbox)
        );
        let inbox = store.list(OperationArea::Inbox).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].envelope(), &existing);
    }

    #[test]
    fn same_operation_id_with_different_envelope_bytes_is_a_collision() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let first = envelope(1, "same operation");
        let second = envelope(1, "same operation");
        assert_eq!(first.operation_id(), second.operation_id());
        assert_ne!(first.to_json().unwrap(), second.to_json().unwrap());

        store.receive(&first).unwrap();
        assert_eq!(
            store.receive(&second).unwrap_err(),
            OperationStoreError::Collision(*first.operation_id())
        );
        assert_eq!(
            store
                .load(OperationArea::Inbox, first.operation_id())
                .unwrap()
                .unwrap()
                .bytes(),
            first.to_json().unwrap().as_bytes()
        );
    }

    #[test]
    fn an_exact_duplicate_in_another_area_is_not_stored_twice() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let envelope = envelope(1, "duplicate");

        store
            .stage_batch(&mutation_id(1), std::slice::from_ref(&envelope))
            .unwrap();
        assert_eq!(
            store.receive(&envelope).unwrap(),
            StoreInsertOutcome::Duplicate
        );
        assert!(store.list(OperationArea::Inbox).unwrap().is_empty());
    }

    #[test]
    fn a_different_vault_envelope_is_rejected_before_storage() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let other_vault = VaultId::parse_str("123e4567-e89b-42d3-a456-426614174099").unwrap();
        let operation = OperationV1::new(
            1,
            vec![],
            OperationPayloadV1::NoteRevision(NoteRevisionV1::tombstone("note").unwrap()),
        )
        .unwrap();
        let envelope = seal_operation(
            &VaultRootKey::from_bytes([7; 32]),
            &other_vault,
            &DeviceId::parse_str("123e4567-e89b-42d3-a456-426614174001").unwrap(),
            &DeviceSigningKey::from_seed([9; 32]),
            &operation,
        )
        .unwrap();

        assert_eq!(
            store.receive(&envelope).unwrap_err(),
            OperationStoreError::VaultMismatch {
                expected: vault_id(),
                actual: other_vault,
            }
        );
        assert!(store.list(OperationArea::Inbox).unwrap().is_empty());
    }

    #[test]
    fn promotion_moves_the_exact_stage_to_the_immutable_outbox() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let envelope = envelope(1, "promote");
        let mutation_id = mutation_id(2);
        store
            .stage_batch(&mutation_id, std::slice::from_ref(&envelope))
            .unwrap();

        assert_eq!(
            store.promote_batch(&mutation_id).unwrap(),
            PromotionOutcome::Promoted
        );
        assert!(store
            .load(OperationArea::Staged, envelope.operation_id())
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .load(OperationArea::Outbox, envelope.operation_id())
                .unwrap()
                .unwrap()
                .bytes(),
            envelope.to_json().unwrap().as_bytes()
        );
        assert_eq!(
            store.promote_batch(&mutation_id).unwrap(),
            PromotionOutcome::AlreadyPromoted
        );
    }

    #[test]
    fn cancellation_removes_only_a_valid_staged_operation() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let envelope = envelope(1, "cancel");
        let mutation_id = mutation_id(3);
        store
            .stage_batch(&mutation_id, std::slice::from_ref(&envelope))
            .unwrap();

        assert!(store.cancel_batch(&mutation_id).unwrap());
        assert!(!store.cancel_batch(&mutation_id).unwrap());
        assert!(store.list(OperationArea::Staged).unwrap().is_empty());
    }

    #[test]
    fn per_area_count_and_byte_limits_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let first = envelope(1, "first");
        let second = envelope(2, "second");
        let first_len = first.to_json().unwrap().len();
        let count_store = OperationStore::open_with_limits(
            temp.path(),
            scope(),
            vault_id(),
            StoreLimits {
                max_operations_per_area: 1,
                max_bytes_per_area: usize::MAX,
                ..StoreLimits::default()
            },
        )
        .unwrap();
        count_store.receive(&first).unwrap();
        assert_eq!(
            count_store.receive(&second).unwrap_err(),
            OperationStoreError::OperationLimitExceeded(OperationArea::Inbox)
        );

        let other_temp = tempfile::tempdir().unwrap();
        let byte_store = OperationStore::open_with_limits(
            other_temp.path(),
            scope(),
            vault_id(),
            StoreLimits {
                max_operations_per_area: 10,
                max_bytes_per_area: first_len,
                ..StoreLimits::default()
            },
        )
        .unwrap();
        byte_store.receive(&first).unwrap();
        assert_eq!(
            byte_store.receive(&second).unwrap_err(),
            OperationStoreError::ByteLimitExceeded(OperationArea::Inbox)
        );
    }

    #[test]
    fn listing_is_sorted_by_canonical_operation_id() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let envelopes = [
            envelope(30, "third"),
            envelope(10, "first"),
            envelope(20, "second"),
        ];
        for envelope in &envelopes {
            store.receive(envelope).unwrap();
        }

        let ids = store
            .list(OperationArea::Inbox)
            .unwrap()
            .into_iter()
            .map(|stored| *stored.envelope().operation_id())
            .collect::<Vec<_>>();
        let mut expected = ids.clone();
        expected.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn staged_batch_keeps_mutation_identity_and_promotes_idempotently() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mutation_id = mutation_id(7);
        let mut envelopes = vec![envelope(2, "second"), envelope(1, "first")];

        assert_eq!(
            store.stage_batch(&mutation_id, &envelopes).unwrap(),
            StageBatchOutcome::Staged
        );
        envelopes.reverse();
        assert_eq!(
            store.stage_batch(&mutation_id, &envelopes).unwrap(),
            StageBatchOutcome::Duplicate
        );
        let batches = store.list_staged_batches().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].mutation_id(), &mutation_id);
        assert!(batches[0]
            .operation_ids()
            .windows(2)
            .all(|pair| pair[0] < pair[1]));

        assert_eq!(
            store.promote_batch(&mutation_id).unwrap(),
            PromotionOutcome::Promoted
        );
        assert!(store.list_staged_batches().unwrap().is_empty());
        assert_eq!(store.list(OperationArea::Outbox).unwrap().len(), 2);
        assert_eq!(
            store.promote_batch(&mutation_id).unwrap(),
            PromotionOutcome::AlreadyPromoted
        );
    }

    #[test]
    fn promotion_replays_after_a_crash_between_outbox_links() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mutation_id = mutation_id(9);
        let envelopes = [envelope(1, "first"), envelope(2, "second")];
        store.stage_batch(&mutation_id, &envelopes).unwrap();
        let first_id = store.list_staged_batches().unwrap()[0].operation_ids()[0];
        store
            .root
            .hard_link_no_clobber(
                &store
                    .namespace
                    .operation_key(OperationArea::Staged, &first_id),
                &store
                    .namespace
                    .operation_key(OperationArea::Outbox, &first_id),
                true,
            )
            .unwrap();

        assert_eq!(
            store.promote_batch(&mutation_id).unwrap(),
            PromotionOutcome::Promoted
        );
        assert_eq!(store.list(OperationArea::Outbox).unwrap().len(), 2);
        assert!(store.list(OperationArea::Staged).unwrap().is_empty());
    }

    #[test]
    fn promoted_marker_recovery_finishes_interrupted_stage_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mutation_id = mutation_id(10);
        let envelope = envelope(1, "cleanup");
        store
            .stage_batch(&mutation_id, std::slice::from_ref(&envelope))
            .unwrap();
        let batch = store
            .read_batch(BatchState::Staged, &mutation_id)
            .unwrap()
            .unwrap();
        store
            .root
            .hard_link_no_clobber(
                &store
                    .namespace
                    .operation_key(OperationArea::Staged, envelope.operation_id()),
                &store
                    .namespace
                    .operation_key(OperationArea::Outbox, envelope.operation_id()),
                true,
            )
            .unwrap();
        store
            .install_batch_marker(
                BatchState::Promoted,
                &batch,
                &serialize_batch(&batch).unwrap(),
            )
            .unwrap();

        assert!(store.list_staged_batches().unwrap().is_empty());
        assert!(store.list(OperationArea::Staged).unwrap().is_empty());
        assert_eq!(
            store.promote_batch(&mutation_id).unwrap(),
            PromotionOutcome::AlreadyPromoted
        );
    }

    #[test]
    fn repeated_batch_id_still_checks_exact_envelope_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mutation_id = mutation_id(11);
        let first = envelope(1, "same operation");
        let resealed = envelope(1, "same operation");
        assert_eq!(first.operation_id(), resealed.operation_id());
        assert_ne!(first.to_json().unwrap(), resealed.to_json().unwrap());
        store
            .stage_batch(&mutation_id, std::slice::from_ref(&first))
            .unwrap();

        assert_eq!(
            store
                .stage_batch(&mutation_id, std::slice::from_ref(&resealed))
                .unwrap_err(),
            OperationStoreError::Collision(*first.operation_id())
        );
    }

    #[test]
    fn batch_witness_rejects_a_same_id_new_envelope_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mutation_id = mutation_id(16);
        let first = envelope(1, "same operation");
        let resealed = envelope(1, "same operation");
        assert_eq!(first.operation_id(), resealed.operation_id());
        store
            .stage_batch(&mutation_id, std::slice::from_ref(&first))
            .unwrap();
        store
            .root
            .put_atomic(
                &store
                    .namespace
                    .operation_key(OperationArea::Staged, first.operation_id()),
                resealed.to_json().unwrap().as_bytes(),
            )
            .unwrap();

        assert_eq!(
            store.list_staged_batches().unwrap_err(),
            OperationStoreError::Collision(*first.operation_id())
        );
    }

    #[test]
    fn batch_counts_operations_and_envelope_bytes_are_bounded() {
        let first = envelope(1, "first");
        let second = envelope(2, "second");
        let within_byte_limit = envelope(3, "other");
        let temp = tempfile::tempdir().unwrap();
        let store = OperationStore::open_with_limits(
            temp.path(),
            scope(),
            vault_id(),
            StoreLimits {
                max_staged_batches: 1,
                max_operations_per_batch: 1,
                max_batch_envelope_bytes: first.to_json().unwrap().len(),
                ..StoreLimits::default()
            },
        )
        .unwrap();

        assert_eq!(
            store
                .stage_batch(&mutation_id(1), &[first.clone(), second.clone()])
                .unwrap_err(),
            OperationStoreError::BatchOperationLimitExceeded
        );
        store
            .stage_batch(&mutation_id(1), std::slice::from_ref(&first))
            .unwrap();
        assert_eq!(
            store
                .stage_batch(&mutation_id(2), std::slice::from_ref(&second))
                .unwrap_err(),
            OperationStoreError::BatchByteLimitExceeded
        );
        assert_eq!(
            store
                .stage_batch(&mutation_id(3), std::slice::from_ref(&within_byte_limit),)
                .unwrap_err(),
            OperationStoreError::BatchLimitExceeded
        );
    }

    #[test]
    fn empty_batch_is_a_durable_noop_not_a_missing_batch() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let noop = mutation_id(12);

        assert_eq!(
            store.stage_batch(&noop, &[]).unwrap(),
            StageBatchOutcome::Staged
        );
        let batches = store.list_staged_batches().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].mutation_id(), &noop);
        assert!(batches[0].operation_ids().is_empty());
        assert_eq!(
            store.promote_batch(&noop).unwrap(),
            PromotionOutcome::Promoted
        );
        assert_eq!(
            store.promote_batch(&noop).unwrap(),
            PromotionOutcome::AlreadyPromoted
        );
        assert_eq!(
            store.promote_batch(&mutation_id(13)).unwrap_err(),
            OperationStoreError::MissingBatch(mutation_id(13))
        );
    }

    #[test]
    fn reopening_reconciles_a_markerless_staged_orphan() {
        let temp = tempfile::tempdir().unwrap();
        let envelope = envelope(1, "orphan");
        {
            let store = store(&temp);
            let bytes = store.canonical_envelope_bytes(&envelope).unwrap();
            store
                .put(OperationArea::Staged, envelope.clone(), bytes)
                .unwrap();
            assert!(store
                .read_record(OperationArea::Staged, envelope.operation_id())
                .unwrap()
                .is_some());
        }

        let reopened = store(&temp);
        assert!(reopened.list(OperationArea::Staged).unwrap().is_empty());
    }

    #[test]
    fn preparing_witness_cancels_an_interrupted_stage_before_local_commit() {
        let temp = tempfile::tempdir().unwrap();
        let mutation_id = mutation_id(14);
        let envelope = envelope(1, "witnessed orphan");
        let batch = StagedBatch {
            mutation_id: mutation_id.clone(),
            operation_ids: vec![*envelope.operation_id()],
            envelope_digests: vec![crate::services::twin_events::digest_bytes(
                envelope.to_json().unwrap().as_bytes(),
            )],
        };
        {
            let store = store(&temp);
            let bytes = serialize_batch(&batch).unwrap();
            store
                .install_batch_marker(BatchState::Preparing, &batch, &bytes)
                .unwrap();
            store
                .put_locked(
                    OperationArea::Staged,
                    envelope.clone(),
                    store.canonical_envelope_bytes(&envelope).unwrap(),
                )
                .unwrap();
        }

        let reopened = store(&temp);
        assert!(reopened.list(OperationArea::Staged).unwrap().is_empty());
        assert!(reopened
            .read_batch(BatchState::Preparing, &mutation_id)
            .unwrap()
            .is_none());
        assert_eq!(
            reopened
                .read_batch(BatchState::Cancelled, &mutation_id)
                .unwrap(),
            Some(batch)
        );
        assert_eq!(
            reopened
                .stage_batch(&mutation_id, std::slice::from_ref(&envelope))
                .unwrap_err(),
            OperationStoreError::BatchCancelled(mutation_id)
        );
    }

    #[test]
    fn ready_marker_wins_a_crash_before_preparing_witness_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let mutation_id = mutation_id(15);
        let envelope = envelope(1, "ready");
        let batch = StagedBatch {
            mutation_id: mutation_id.clone(),
            operation_ids: vec![*envelope.operation_id()],
            envelope_digests: vec![crate::services::twin_events::digest_bytes(
                envelope.to_json().unwrap().as_bytes(),
            )],
        };
        {
            let store = store(&temp);
            let bytes = serialize_batch(&batch).unwrap();
            store
                .install_batch_marker(BatchState::Preparing, &batch, &bytes)
                .unwrap();
            store
                .put_locked(
                    OperationArea::Staged,
                    envelope.clone(),
                    store.canonical_envelope_bytes(&envelope).unwrap(),
                )
                .unwrap();
            store
                .install_batch_marker(BatchState::Staged, &batch, &bytes)
                .unwrap();
        }

        let reopened = store(&temp);
        assert!(reopened
            .read_batch(BatchState::Preparing, &mutation_id)
            .unwrap()
            .is_none());
        assert_eq!(reopened.list_staged_batches().unwrap(), vec![batch]);
        assert_eq!(
            reopened.promote_batch(&mutation_id).unwrap(),
            PromotionOutcome::Promoted
        );
    }

    #[test]
    fn batch_identity_collision_and_cancellation_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let mutation_id = mutation_id(8);
        let first = envelope(1, "first");
        store.stage_batch(&mutation_id, &[first]).unwrap();

        assert_eq!(
            store
                .stage_batch(&mutation_id, &[envelope(2, "different")])
                .unwrap_err(),
            OperationStoreError::BatchCollision(mutation_id.clone())
        );
        assert!(store.cancel_batch(&mutation_id).unwrap());
        assert!(!store.cancel_batch(&mutation_id).unwrap());
        assert!(store.list_staged_batches().unwrap().is_empty());
        assert_eq!(
            store.promote_batch(&mutation_id).unwrap_err(),
            OperationStoreError::BatchCancelled(mutation_id)
        );
    }

    #[test]
    fn applied_operation_ids_are_immutable_bounded_and_sorted() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        let first = envelope(2, "second");
        let second = envelope(1, "first");
        store.receive(&first).unwrap();
        store.receive(&second).unwrap();

        assert_eq!(
            store.mark_applied(first.operation_id()).unwrap(),
            AppliedMarkOutcome::Marked
        );
        assert_eq!(
            store.mark_applied(second.operation_id()).unwrap(),
            AppliedMarkOutcome::Marked
        );
        assert_eq!(
            store.mark_applied(first.operation_id()).unwrap(),
            AppliedMarkOutcome::Duplicate
        );
        let ids = store.list_applied_ids().unwrap();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(ids.len(), 2);

        let limited_temp = tempfile::tempdir().unwrap();
        let limited = OperationStore::open_with_limits(
            limited_temp.path(),
            scope(),
            vault_id(),
            StoreLimits {
                max_applied_operations: 1,
                ..StoreLimits::default()
            },
        )
        .unwrap();
        limited.receive(&first).unwrap();
        limited.receive(&second).unwrap();
        limited.mark_applied(first.operation_id()).unwrap();
        assert_eq!(
            limited.mark_applied(second.operation_id()).unwrap_err(),
            OperationStoreError::AppliedLimitExceeded
        );
    }

    #[test]
    fn malformed_or_oversized_raw_json_never_reaches_disk() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);

        assert!(matches!(
            store.receive_json(b"not-json").unwrap_err(),
            OperationStoreError::Protocol(_)
        ));
        assert!(matches!(
            store
                .receive_json(&vec![b'x'; MAX_ENVELOPE_JSON_BYTES + 1])
                .unwrap_err(),
            OperationStoreError::Protocol(_)
        ));
        assert!(store.list(OperationArea::Inbox).unwrap().is_empty());
    }
}
