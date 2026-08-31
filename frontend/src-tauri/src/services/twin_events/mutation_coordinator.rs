use crate::models::twin_event::{
    ActorId, CausalStream, DeviceId, EventContext, EventId, EvidenceRef, Governance, Sensitivity,
    TwinEvent, TwinEventPayload, Visibility,
};
use crate::services::twin_events::{derive_event_id, StoreError, TwinEventStore};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;
#[cfg(feature = "mcp")]
mod custom_mcp;
mod engine;
mod root_lease;
mod stable_migration;

pub(crate) use root_lease::root_identity_for_path;
use root_lease::*;

#[cfg(all(feature = "mcp", test))]
pub(crate) use custom_mcp::{CustomMcpRootBindingV1, CUSTOM_MCP_ROOT_BINDING_KEY};

pub(crate) struct CoordinatorProcessLock {
    lock: crate::services::twin_events::AnchoredExclusiveLock,
}

impl std::ops::Deref for CoordinatorProcessLock {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.lock
    }
}

impl CoordinatorProcessLock {
    pub(crate) fn unlock(self) -> io::Result<()> {
        self.lock.unlock()
    }

    pub(crate) fn covers_data_path(&self, data_path: &Path) -> Result<bool, MutationError> {
        let root = crate::services::twin_events::AnchoredRoot::open(data_path)?;
        Ok(root.canonical_path() == self.lock.root_path())
    }
}

const WRITER_SCHEMA_VERSION: u16 = 1;
const WRITER_FILE_LIMIT: u64 = 4096;
const WRITER_KEY: &str = "twin/events/writer-v1.json";
const WRITER_STAGING_KEY: &str = "twin/events/writer-staging/v1";
const WRITER_EVIDENCE_MAX_DEPTH: usize = 32;
const WRITER_EVIDENCE_MAX_ENTRIES: usize = 8 * 1024;

#[derive(Debug)]
#[allow(private_interfaces)] // Public recorder errors carry crate-internal repair authority.
pub enum MutationError {
    Store(StoreError),
    Io(String),
    Invalid(String),
    RecoveryConflict(String),
    AbortedPrecondition {
        mutation_id: String,
        authority_advanced: bool,
    },
    AuthorityAdvanced {
        mutation_id: crate::models::twin_event::ContentDigest,
        authority_token: crate::services::vault_namespace::VaultAuthorityTokenV1,
        target_aborted: bool,
        reason: String,
    },
}

impl MutationError {
    pub(crate) fn authority_advanced_commit(&self) -> Option<MutationCommit> {
        match self {
            Self::AuthorityAdvanced {
                mutation_id,
                authority_token,
                ..
            } => Some(MutationCommit {
                mutation_id: Some(mutation_id.clone()),
                events: Vec::new(),
                authority_token: Some(authority_token.clone()),
                postcommit_warning: true,
            }),
            _ => None,
        }
    }

    pub(crate) fn authority_advanced_target_aborted(&self) -> bool {
        matches!(
            self,
            Self::AuthorityAdvanced {
                target_aborted: true,
                ..
            }
        )
    }
}

impl std::fmt::Display for MutationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => error.fmt(formatter),
            Self::Io(message) | Self::Invalid(message) => formatter.write_str(message),
            Self::RecoveryConflict(id) => {
                write!(formatter, "recoverable local mutation conflict: {id}")
            }
            Self::AbortedPrecondition { mutation_id, .. } => {
                write!(
                    formatter,
                    "retained mutation precondition changed: {mutation_id}"
                )
            }
            Self::AuthorityAdvanced { mutation_id, .. } => write!(
                formatter,
                "mutation authority advanced and durable recovery remains pending: {}",
                mutation_id.as_str()
            ),
        }
    }
}

impl std::error::Error for MutationError {}
impl From<StoreError> for MutationError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}
impl From<io::Error> for MutationError {
    fn from(value: io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct TwinEventDraft {
    pub actor_id: Option<ActorId>,
    pub causal_parents: Vec<EventId>,
    pub recorded_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub occurred_at: Option<DateTime<Utc>>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
    pub supersedes: Vec<EventId>,
    pub reinforces: Vec<EventId>,
    pub context: EventContext,
    pub evidence: Vec<EvidenceRef>,
    pub governance: Governance,
    pub payload: TwinEventPayload,
}

impl TwinEventDraft {
    pub fn observed(
        payload: TwinEventPayload,
        observed_at: DateTime<Utc>,
        source_channel: crate::models::twin_event::SourceChannel,
        governance: Governance,
    ) -> Self {
        let context = EventContext {
            source_channel,
            ..EventContext::default()
        };
        Self {
            actor_id: None,
            causal_parents: Vec::new(),
            recorded_at: observed_at,
            observed_at,
            occurred_at: None,
            valid_from: None,
            valid_to: None,
            supersedes: Vec::new(),
            reinforces: Vec::new(),
            context,
            evidence: Vec::new(),
            governance,
            payload,
        }
    }
}

pub trait MutationIdentityProvider: Send + Sync {
    fn actor_id(&self) -> ActorId;
    fn device_id(&self) -> DeviceId;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WriterIdentityV1 {
    schema_version: u16,
    device_id: DeviceId,
    actor_id: ActorId,
}

#[derive(Debug, Clone)]
pub struct PersistedMutationIdentityProvider {
    identity: WriterIdentityV1,
}

impl PersistedMutationIdentityProvider {
    pub fn load_optional(data_path: impl AsRef<Path>) -> Result<Option<Self>, MutationError> {
        let root = crate::services::twin_events::AnchoredRoot::open(data_path)?;
        root.read_bounded(WRITER_KEY, WRITER_FILE_LIMIT as usize)?
            .map(|bytes| Self::load(&bytes))
            .transpose()
    }

    pub fn load_or_create(data_path: impl AsRef<Path>) -> Result<Self, MutationError> {
        if let Some(identity) = Self::load_optional(data_path.as_ref())? {
            return Ok(identity);
        }
        let root = crate::services::twin_events::AnchoredRoot::open(data_path)?;
        root.open_directory("twin/events", false)?;
        root.open_directory(WRITER_STAGING_KEY, true)?;

        let identity = WriterIdentityV1 {
            schema_version: WRITER_SCHEMA_VERSION,
            device_id: DeviceId::parse(Uuid::new_v4().to_string())
                .map_err(MutationError::Invalid)?,
            actor_id: ActorId::parse("owner").map_err(MutationError::Invalid)?,
        };
        let mut bytes = serde_json::to_vec_pretty(&identity)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        bytes.push(b'\n');
        root.install_no_clobber(WRITER_KEY, WRITER_STAGING_KEY, &bytes)?;
        let installed = root
            .read_bounded(WRITER_KEY, WRITER_FILE_LIMIT as usize)?
            .ok_or_else(|| {
                MutationError::RecoveryConflict("writer identity disappeared after install".into())
            })?;
        Self::load(&installed)
    }

    fn load(bytes: &[u8]) -> Result<Self, MutationError> {
        let identity: WriterIdentityV1 = serde_json::from_slice(bytes)
            .map_err(|error| MutationError::Invalid(format!("invalid writer identity: {error}")))?;
        if identity.schema_version != WRITER_SCHEMA_VERSION {
            return Err(MutationError::Invalid(
                "unsupported Twin writer identity schema".into(),
            ));
        }
        Uuid::parse_str(identity.device_id.as_str())
            .map_err(|_| MutationError::Invalid("writer device ID must be a UUID".into()))?;
        Ok(Self { identity })
    }
}

fn reject_missing_writer_for_established_data_root_locked(
    data_root: &crate::services::twin_events::AnchoredRoot,
    process_lock: &CoordinatorProcessLock,
) -> Result<Option<PersistedMutationIdentityProvider>, MutationError> {
    if !process_lock.covers_data_path(data_root.canonical_path())? {
        return Err(MutationError::Invalid(
            "writer identity scan lock belongs to another data root".into(),
        ));
    }
    let identity = PersistedMutationIdentityProvider::load_optional(data_root.canonical_path())?;
    let mut remaining_entries = WRITER_EVIDENCE_MAX_ENTRIES;
    if identity.is_none()
        && writer_aware_established_evidence_exists(data_root, &mut remaining_entries)?
    {
        return Err(MutationError::RecoveryConflict(
            "writer-identity-missing-for-established-data-root".into(),
        ));
    }
    if writer_staging_contains_unrecognized_entry(data_root, &mut remaining_entries)? {
        let reason = if identity.is_none() {
            "writer-identity-missing-for-established-data-root"
        } else {
            "writer-install-staging-contains-unrecognized-entry"
        };
        return Err(MutationError::RecoveryConflict(reason.into()));
    }
    Ok(identity)
}

fn writer_staging_contains_unrecognized_entry(
    data_root: &crate::services::twin_events::AnchoredRoot,
    remaining_entries: &mut usize,
) -> Result<bool, MutationError> {
    if !data_root.directory_exists(WRITER_STAGING_KEY)? {
        return Ok(false);
    }
    let entries = data_root.directory_entries_bounded(WRITER_STAGING_KEY, *remaining_entries)?;
    *remaining_entries -= entries.len();
    for (name, kind) in entries {
        let is_recognized_writer_temp = kind
            == crate::services::twin_events::AnchoredEntryKind::File
            && is_canonical_writer_install_temp(&name);
        if !is_recognized_writer_temp {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_canonical_writer_install_temp(name: &str) -> bool {
    let Some(uuid_text) = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    Uuid::parse_str(uuid_text).is_ok_and(|uuid| {
        !uuid.is_nil() && uuid.get_version_num() == 4 && uuid.to_string() == uuid_text
    })
}

fn writer_aware_established_evidence_exists(
    data_root: &crate::services::twin_events::AnchoredRoot,
    remaining_entries: &mut usize,
) -> Result<bool, MutationError> {
    for key in [
        ACTIVE_ROOT_LEASE_KEY,
        crate::services::sync::device::DEVICE_SIGNING_BINDING_KEY,
        "twin/events/content-authority-v1.json",
    ] {
        if data_root
            .read_bounded(key, WRITER_FILE_LIMIT as usize)?
            .is_some()
        {
            return Ok(true);
        }
    }
    for directory in [
        "twin/stable-vault-migrations/v1",
        "twin/events/v1",
        "twin/events/quarantine/v1",
        "twin/events/staging/v1",
        "twin/events/vaults/v1",
        "twin/mutations/pending/v1",
        "twin/mutations/preauthority/v1",
        "twin/mutations/quarantine/v1",
        "twin/mutations/receipts/v1",
    ] {
        if anchored_directory_contains_regular_file(data_root, directory, 0, remaining_entries)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn anchored_directory_contains_regular_file(
    data_root: &crate::services::twin_events::AnchoredRoot,
    directory: &str,
    depth: usize,
    remaining_entries: &mut usize,
) -> Result<bool, MutationError> {
    if !data_root.directory_exists(directory)? {
        return Ok(false);
    }
    if depth >= WRITER_EVIDENCE_MAX_DEPTH {
        return Err(MutationError::Invalid(
            "writer identity evidence tree exceeds its depth limit".into(),
        ));
    }
    let entries = data_root.directory_entries_bounded(directory, *remaining_entries)?;
    *remaining_entries -= entries.len();
    for (name, kind) in entries {
        let child = format!("{directory}/{name}");
        match kind {
            crate::services::twin_events::AnchoredEntryKind::File => return Ok(true),
            crate::services::twin_events::AnchoredEntryKind::Directory => {
                if anchored_directory_contains_regular_file(
                    data_root,
                    &child,
                    depth + 1,
                    remaining_entries,
                )? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

impl MutationIdentityProvider for PersistedMutationIdentityProvider {
    fn actor_id(&self) -> ActorId {
        self.identity.actor_id.clone()
    }

    fn device_id(&self) -> DeviceId {
        self.identity.device_id.clone()
    }
}

pub trait EventGroupFinalizer: Send + Sync {
    fn finalize(
        &self,
        requested_stream: CausalStream,
        drafts: &[TwinEventDraft],
    ) -> Result<Vec<TwinEvent>, MutationError>;
}

pub struct StoreEventGroupFinalizer {
    store: Arc<TwinEventStore>,
    identity: Arc<dyn MutationIdentityProvider>,
    reject_root_transition_wal: bool,
}

impl StoreEventGroupFinalizer {
    pub fn new(store: Arc<TwinEventStore>, identity: Arc<dyn MutationIdentityProvider>) -> Self {
        Self {
            store,
            identity,
            reject_root_transition_wal: false,
        }
    }

    fn new_stable(store: Arc<TwinEventStore>, identity: Arc<dyn MutationIdentityProvider>) -> Self {
        Self {
            store,
            identity,
            reject_root_transition_wal: true,
        }
    }

    pub(crate) fn acquire_coordinator_lock(&self) -> Result<CoordinatorProcessLock, MutationError> {
        let lock = acquire_shared_coordinator_process_lock(self.store.data_path())?;
        if self.reject_root_transition_wal {
            crate::services::root_transition::reject_transition_wal_locked(
                self.store.data_path(),
                &lock,
            )?;
        }
        reject_prepared_stable_migration_locked(self.store.data_path(), &lock)?;
        Ok(lock)
    }

    pub(crate) fn finalize_locked(
        &self,
        requested_stream: CausalStream,
        drafts: &[TwinEventDraft],
    ) -> Result<Vec<TwinEvent>, MutationError> {
        if drafts.is_empty() || drafts.len() > crate::models::twin_event::MAX_EVENT_LINKS {
            return Err(MutationError::Invalid(
                "event group must contain 1..=64 drafts".into(),
            ));
        }
        let stream = if requested_stream == CausalStream::SyncEligible
            && drafts.iter().all(draft_is_sync_eligible)
        {
            CausalStream::SyncEligible
        } else {
            CausalStream::LocalOnly
        };
        let device_id = self.identity.device_id();
        let ordered = self.store.ordered_events_for_stream(stream)?;
        let predecessor = ordered
            .iter()
            .filter(|event| event.device_id == device_id)
            .max_by_key(|event| event.device_sequence);
        let mut sequence = predecessor.map_or(1, |event| event.device_sequence + 1);
        let mut prior_id = predecessor.map(|event| event.event_id.clone());
        let mut events = Vec::with_capacity(drafts.len());
        for draft in drafts {
            let mut parents = draft.causal_parents.clone();
            if let Some(parent) = &prior_id {
                parents.push(parent.clone());
            }
            parents.sort();
            parents.dedup();
            let mut event = TwinEvent {
                schema_version: 1,
                event_id: EventId::parse("0".repeat(64)).expect("static placeholder ID"),
                event_type: draft.payload.event_type(),
                actor_id: draft
                    .actor_id
                    .clone()
                    .unwrap_or_else(|| self.identity.actor_id()),
                device_id: device_id.clone(),
                causal_stream: stream,
                device_sequence: sequence,
                causal_parents: parents,
                recorded_at: draft.recorded_at,
                observed_at: draft.observed_at,
                occurred_at: draft.occurred_at,
                valid_from: draft.valid_from,
                valid_to: draft.valid_to,
                supersedes: draft.supersedes.clone(),
                reinforces: draft.reinforces.clone(),
                context: draft.context.clone(),
                evidence: draft.evidence.clone(),
                governance: draft.governance.clone(),
                payload: draft.payload.clone(),
            };
            event.validate().map_err(MutationError::Invalid)?;
            event.normalize();
            event.event_id = derive_event_id(&event);
            prior_id = Some(event.event_id.clone());
            sequence += 1;
            events.push(event);
        }
        Ok(events)
    }

    fn actor_id(&self) -> ActorId {
        self.identity.actor_id()
    }

    fn device_id(&self) -> DeviceId {
        self.identity.device_id()
    }
}

pub(crate) fn acquire_shared_coordinator_process_lock(
    data_path: &Path,
) -> Result<CoordinatorProcessLock, MutationError> {
    let root = crate::services::twin_events::AnchoredRoot::open(data_path)?;
    root.open_directory("twin/events", false)?;
    let lock = root.lock_exclusive("twin/events/mutation-v1.lock")?;
    Ok(CoordinatorProcessLock { lock })
}

pub(crate) fn reject_prepared_stable_migration_locked(
    data_path: &Path,
    process_lock: &CoordinatorProcessLock,
) -> Result<(), MutationError> {
    let root = crate::services::twin_events::AnchoredRoot::open(data_path)?;
    stable_migration::reject_prepared_migration_locked(&root, process_lock)
}

#[cfg_attr(not(feature = "mcp"), allow(dead_code))]
pub(crate) fn prepared_stable_migration_scope_locked(
    data_path: &Path,
    process_lock: &CoordinatorProcessLock,
) -> Result<Option<crate::models::twin_event::ContentDigest>, MutationError> {
    let root = crate::services::twin_events::AnchoredRoot::open(data_path)?;
    stable_migration::inspect_prepared_migration_locked(&root, process_lock)
}

impl EventGroupFinalizer for StoreEventGroupFinalizer {
    fn finalize(
        &self,
        requested_stream: CausalStream,
        drafts: &[TwinEventDraft],
    ) -> Result<Vec<TwinEvent>, MutationError> {
        let lock = self.acquire_coordinator_lock()?;
        let result = self.finalize_locked(requested_stream, drafts);
        lock.unlock()?;
        result
    }
}

fn draft_is_sync_eligible(draft: &TwinEventDraft) -> bool {
    governance_is_sync_eligible(&draft.governance)
        && draft
            .context
            .relationships
            .iter()
            .all(|relationship| governance_is_sync_eligible(&relationship.governance))
}

fn governance_is_sync_eligible(governance: &Governance) -> bool {
    governance.visibility == Visibility::SyncedVault
        && governance.sensitivity != Sensitivity::Restricted
        && governance.allowed_uses.sync
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationOrigin {
    Local,
    Remote,
    Recovery,
}

#[derive(Debug)]
pub struct MutationPlan {
    pub requested_stream: CausalStream,
    pub source_channel: crate::models::twin_event::SourceChannel,
    pub targets: Vec<crate::services::twin_events::TargetMutation>,
    pub drafts: Vec<TwinEventDraft>,
    pub(crate) expected_authority: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    pub(crate) retain_commit_receipt: bool,
}

impl MutationPlan {
    pub fn new(
        requested_stream: CausalStream,
        source_channel: crate::models::twin_event::SourceChannel,
        targets: Vec<crate::services::twin_events::TargetMutation>,
        drafts: Vec<TwinEventDraft>,
    ) -> Self {
        Self {
            requested_stream,
            source_channel,
            targets,
            drafts,
            expected_authority: None,
            retain_commit_receipt: false,
        }
    }

    pub(crate) fn retaining_commit_receipt(mut self) -> Self {
        self.retain_commit_receipt = true;
        self
    }

    pub(crate) fn expecting_authority(
        mut self,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Self {
        self.expected_authority = Some(expected);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationFaultPoint {
    BeforePreAuthorityMarker,
    AfterPreAuthorityMarker,
    AfterPreparedHook,
    AfterAuthorityAdvance,
    AfterStage,
    AfterPostAuthorityAbortProof,
    AfterTarget(usize),
    AfterTargets,
    AfterEvent(usize),
    BeforeDurabilityClassification,
    BeforePendingRecovery,
    BeforeCleanup,
    AfterCleanupBeforeFanout,
}

pub trait MutationLifecycle: Send + Sync {
    fn stage_before_local(
        &self,
        _intent: &crate::services::twin_events::MutationIntentV1,
    ) -> Result<(), MutationError> {
        Ok(())
    }

    fn committed(
        &self,
        _intent: &crate::services::twin_events::MutationIntentV1,
    ) -> Result<(), MutationError> {
        Ok(())
    }

    fn known_failure(&self, _mutation_id: Option<&str>, _reason: &str) {}
}

#[derive(Debug, Default)]
pub struct NoopMutationLifecycle;
impl MutationLifecycle for NoopMutationLifecycle {}

#[must_use = "a committed mutation may carry the exact authority token required for repair"]
#[derive(Debug, Clone)]
pub struct MutationCommit {
    pub mutation_id: Option<crate::models::twin_event::ContentDigest>,
    pub events: Vec<TwinEvent>,
    pub(crate) authority_token: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    pub(crate) postcommit_warning: bool,
}

pub struct MutationCoordinator {
    data_path: PathBuf,
    data_root: crate::services::twin_events::AnchoredRoot,
    vault_root: std::sync::Mutex<crate::services::twin_events::AnchoredRoot>,
    store: Arc<TwinEventStore>,
    finalizer: StoreEventGroupFinalizer,
    journal: crate::services::twin_events::LocalMutationJournal,
    lifecycle: Arc<dyn MutationLifecycle>,
    in_process: std::sync::Mutex<()>,
    fault_once: std::sync::Mutex<Option<MutationFaultPoint>>,
    #[cfg(test)]
    replay_failures_before_targets: std::sync::Mutex<usize>,
    #[cfg(test)]
    pause_after_authority_advance_once: std::sync::Mutex<
        Option<(
            std::sync::Arc<std::sync::Barrier>,
            std::sync::Arc<std::sync::Barrier>,
        )>,
    >,
    root_lease: std::sync::Mutex<ActiveMarkdownRootLeaseV1>,
}

pub(crate) struct MutationRootTransitionGuard<'a> {
    coordinator: &'a MutationCoordinator,
    _process_lock: CoordinatorProcessLock,
}

#[derive(Debug)]
pub(crate) enum WitnessedMutationRecovery {
    NotCommitted,
    Aborted,
    AbortedAfterAuthority(MutationCommit),
    Committed(MutationCommit),
}

const ACTIVE_ROOT_LEASE_SCHEMA_VERSION: u16 = 1;
const STABLE_ROOT_LEASE_SCHEMA_VERSION: u16 = 2;
const ACTIVE_ROOT_LEASE_LIMIT: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootIdentityMode {
    LegacyPath,
    StableVault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActiveMarkdownRootLeaseV1 {
    pub(crate) schema_version: u16,
    pub(crate) root_scope: crate::models::twin_event::ContentDigest,
    pub(crate) epoch_uuid: String,
}

impl ActiveMarkdownRootLeaseV1 {
    pub(crate) fn new(root_scope: crate::models::twin_event::ContentDigest) -> Self {
        Self {
            schema_version: ACTIVE_ROOT_LEASE_SCHEMA_VERSION,
            root_scope,
            epoch_uuid: Uuid::new_v4().to_string(),
        }
    }

    pub(crate) fn new_stable(root_scope: crate::models::twin_event::ContentDigest) -> Self {
        Self {
            schema_version: STABLE_ROOT_LEASE_SCHEMA_VERSION,
            root_scope,
            epoch_uuid: Uuid::new_v4().to_string(),
        }
    }

    pub(crate) fn is_stable(&self) -> bool {
        self.schema_version == STABLE_ROOT_LEASE_SCHEMA_VERSION
    }
}

impl MutationCoordinator {
    pub fn new(
        data_path: impl AsRef<Path>,
        vault_path: impl AsRef<Path>,
        store: Arc<TwinEventStore>,
        lifecycle: Arc<dyn MutationLifecycle>,
    ) -> Result<Self, MutationError> {
        Self::new_internal(
            data_path,
            vault_path,
            store,
            lifecycle,
            false,
            None,
            RootIdentityMode::LegacyPath,
        )
    }

    pub(crate) fn new_stable(
        data_path: impl AsRef<Path>,
        vault_path: impl AsRef<Path>,
        store: Arc<TwinEventStore>,
        lifecycle: Arc<dyn MutationLifecycle>,
    ) -> Result<Self, MutationError> {
        Self::new_internal(
            data_path,
            vault_path,
            store,
            lifecycle,
            false,
            None,
            RootIdentityMode::StableVault,
        )
    }

    #[cfg(feature = "mcp")]
    pub(crate) fn new_custom_mcp(
        data_path: impl AsRef<Path>,
        vault_path: impl AsRef<Path>,
        store: Arc<TwinEventStore>,
        lifecycle: Arc<dyn MutationLifecycle>,
    ) -> Result<Self, MutationError> {
        Self::new_internal(
            data_path,
            vault_path,
            store,
            lifecycle,
            true,
            None,
            RootIdentityMode::StableVault,
        )
    }

    #[cfg(all(test, feature = "mcp"))]
    pub(crate) fn new_custom_mcp_with_hook(
        data_path: impl AsRef<Path>,
        vault_path: impl AsRef<Path>,
        store: Arc<TwinEventStore>,
        lifecycle: Arc<dyn MutationLifecycle>,
        after_wal_check: impl FnOnce() + Send + 'static,
    ) -> Result<Self, MutationError> {
        Self::new_internal(
            data_path,
            vault_path,
            store,
            lifecycle,
            true,
            Some(Box::new(after_wal_check)),
            RootIdentityMode::StableVault,
        )
    }

    fn new_internal(
        data_path: impl AsRef<Path>,
        vault_path: impl AsRef<Path>,
        store: Arc<TwinEventStore>,
        lifecycle: Arc<dyn MutationLifecycle>,
        custom_mcp: bool,
        after_custom_wal_check: Option<Box<dyn FnOnce() + Send>>,
        identity_mode: RootIdentityMode,
    ) -> Result<Self, MutationError> {
        let data_path = data_path.as_ref().to_path_buf();
        let vault_path = vault_path.as_ref().to_path_buf();
        crate::services::twin_events::validate_real_directory(&data_path, "trusted app-data root")?;
        crate::services::twin_events::validate_real_directory(
            &vault_path,
            "trusted Markdown vault root",
        )?;
        let data_path = fs::canonicalize(data_path)?;
        crate::services::twin_events::validate_real_directory(
            store.data_path(),
            "Twin event store data root",
        )?;
        if fs::canonicalize(store.data_path())? != data_path {
            return Err(MutationError::Invalid(
                "Twin event store data root does not match the coordinator data root".into(),
            ));
        }
        if !store.is_legacy_namespace().map_err(MutationError::Store)? {
            return Err(MutationError::Invalid(
                "Twin event store must begin in the legacy namespace".into(),
            ));
        }
        let vault_path = fs::canonicalize(vault_path)?;
        let data_root = crate::services::twin_events::AnchoredRoot::open(&data_path)?;
        let process_lock = acquire_shared_coordinator_process_lock(&data_path)?;
        let persisted_writer =
            reject_missing_writer_for_established_data_root_locked(&data_root, &process_lock)?;
        if identity_mode == RootIdentityMode::StableVault {
            crate::services::root_transition::reject_transition_wal_locked(
                &data_path,
                &process_lock,
            )?;
        }
        let prepared_migration_scope =
            stable_migration::inspect_prepared_migration_locked(&data_root, &process_lock)?;
        if identity_mode == RootIdentityMode::LegacyPath && prepared_migration_scope.is_some() {
            return Err(MutationError::RecoveryConflict(
                "prepared stable migration requires stable recovery".into(),
            ));
        }
        #[cfg(feature = "mcp")]
        if custom_mcp {
            if let Some(after_wal_check) = after_custom_wal_check {
                after_wal_check();
            }
        }
        #[cfg(not(feature = "mcp"))]
        {
            debug_assert!(!custom_mcp);
            debug_assert!(after_custom_wal_check.is_none());
        }
        let existing_root_lease = load_optional_active_root_lease(&data_root)?;
        #[cfg(feature = "mcp")]
        // Read an established custom binding before identity creation, but do not
        // publish a new one until the no-lease legacy ownership audit below passes.
        let custom_mcp_binding = if custom_mcp {
            custom_mcp::load_for_vault_locked(&data_root, &vault_path, &process_lock)?
        } else {
            None
        };
        if prepared_migration_scope.is_some() && existing_root_lease.is_none() {
            return Err(MutationError::RecoveryConflict(
                "prepared stable migration is missing its active lease".into(),
            ));
        }
        #[cfg(feature = "mcp")]
        if custom_mcp
            && custom_mcp_binding.is_none()
            && (prepared_migration_scope.is_some()
                || existing_root_lease
                    .as_ref()
                    .is_some_and(ActiveMarkdownRootLeaseV1::is_stable))
        {
            return Err(MutationError::RecoveryConflict(
                "stable custom MCP data has no durable vault-path binding".into(),
            ));
        }
        let legacy_scope = markdown_root_scope_for(&vault_path)?;
        let bootstrap_schema_one = identity_mode == RootIdentityMode::StableVault
            && existing_root_lease.is_none()
            && prepared_migration_scope.is_none()
            && stable_migration::requires_schema_one_bootstrap_without_lease(
                &data_root,
                &vault_path,
                &legacy_scope,
            )?;
        if let Some(lease) = existing_root_lease.as_ref() {
            match (identity_mode, lease.schema_version) {
                (_, ACTIVE_ROOT_LEASE_SCHEMA_VERSION) if lease.root_scope != legacy_scope => {
                    return Err(MutationError::RecoveryConflict(
                        "configured-markdown-root-is-not-active".into(),
                    ));
                }
                (RootIdentityMode::LegacyPath, STABLE_ROOT_LEASE_SCHEMA_VERSION) => {
                    return Err(MutationError::RecoveryConflict(
                        "stable-vault-authority-requires-stable-bootstrap".into(),
                    ));
                }
                _ => {}
            }
        }
        let stable_identity = if identity_mode == RootIdentityMode::StableVault {
            let require_existing_identity = existing_root_lease
                .as_ref()
                .is_some_and(ActiveMarkdownRootLeaseV1::is_stable)
                || prepared_migration_scope.is_some();
            #[cfg(feature = "mcp")]
            let require_existing_identity =
                require_existing_identity || (custom_mcp && custom_mcp_binding.is_some());
            let identity = if require_existing_identity {
                crate::services::sync::identity::load_vault_identity(&vault_path)?
            } else {
                crate::services::sync::identity::load_or_create_vault_identity(&vault_path)?
            };
            #[cfg(feature = "mcp")]
            if custom_mcp_binding
                .as_ref()
                .is_some_and(|binding| binding.root_scope != identity.root_scope)
            {
                return Err(MutationError::RecoveryConflict(
                    "custom MCP root binding does not match the configured vault path and identity"
                        .into(),
                ));
            }
            if prepared_migration_scope
                .as_ref()
                .is_some_and(|scope| scope != &identity.root_scope)
            {
                return Err(MutationError::RecoveryConflict(
                    "prepared stable migration belongs to another vault".into(),
                ));
            }
            if existing_root_lease
                .as_ref()
                .is_some_and(|lease| lease.is_stable() && lease.root_scope != identity.root_scope)
            {
                return Err(MutationError::RecoveryConflict(
                    "configured-markdown-root-is-not-active".into(),
                ));
            }
            Some(identity)
        } else {
            None
        };
        let expected_scope = stable_identity.as_ref().map_or_else(
            || legacy_scope.clone(),
            |identity| identity.root_scope.clone(),
        );
        let (mut root_lease, root_lease_needs_write) = validate_or_prepare_active_root_lease(
            existing_root_lease,
            expected_scope,
            legacy_scope.clone(),
            identity_mode,
            bootstrap_schema_one,
        )?;
        store.initialize().map_err(MutationError::Store)?;
        data_root.open_directory("canvas", true)?;
        let twin_path = fs::canonicalize(data_path.join("twin"))?;
        let canvas_path = fs::canonicalize(data_path.join("canvas"))?;
        validate_disjoint_roots(&vault_path, &canvas_path, &twin_path)?;
        #[cfg(feature = "mcp")]
        if custom_mcp && custom_mcp_binding.is_none() && root_lease_needs_write {
            custom_mcp::verify_or_install_locked(
                &data_root,
                &vault_path,
                stable_identity
                    .as_ref()
                    .expect("custom MCP uses stable identity")
                    .root_scope
                    .clone(),
                &process_lock,
                true,
            )?;
        }
        // A writer must be durable before the first writer-aware artifact
        // (the active-root lease) can make this root established.
        let identity = Arc::new(match persisted_writer {
            Some(identity) => identity,
            None => PersistedMutationIdentityProvider::load_or_create(&data_path)?,
        });
        if root_lease_needs_write {
            write_active_root_lease(&data_root, &root_lease)?;
        }
        let vault_root = crate::services::twin_events::AnchoredRoot::open(&vault_path)?;
        let finalizer = if identity_mode == RootIdentityMode::StableVault {
            StoreEventGroupFinalizer::new_stable(store.clone(), identity)
        } else {
            StoreEventGroupFinalizer::new(store.clone(), identity)
        };
        let journal = crate::services::twin_events::LocalMutationJournal::initialize(&data_path)?;
        journal.cleanup_orphan_temps_locked(&process_lock)?;
        crate::services::vault_namespace::initialize_authority_locked(&data_path, &process_lock)?;
        let resume_stable_migration = match stable_identity.as_ref() {
            Some(identity) => stable_migration::marker_exists(&data_root, &identity.root_scope)?,
            None => false,
        };
        if resume_stable_migration {
            let was_legacy = !root_lease.is_stable();
            root_lease = stable_migration::migrate_legacy_to_stable_locked(
                &data_path,
                &data_root,
                &vault_path,
                stable_identity.as_ref().expect("stable identity checked"),
                &legacy_scope,
                &root_lease,
                &finalizer.device_id(),
                &journal,
                &process_lock,
            )?;
            store
                .activate_vault_scope_locked(root_lease.root_scope.clone(), &process_lock)
                .map_err(MutationError::Store)?;
            crate::services::vault_namespace::initialize_locked(
                &data_path,
                &root_lease,
                &process_lock,
            )?;
            ensure_overlay_target_root(&data_root, &root_lease.root_scope)?;
            if was_legacy {
                crate::services::vault_namespace::invalidate_locked(
                    &data_path,
                    &root_lease,
                    &process_lock,
                )?;
            }
        } else {
            if root_lease.is_stable() {
                store
                    .activate_vault_scope_locked(root_lease.root_scope.clone(), &process_lock)
                    .map_err(MutationError::Store)?;
            }
            let delay_schema_one_assignment =
                identity_mode == RootIdentityMode::StableVault && !root_lease.is_stable();
            if !delay_schema_one_assignment {
                crate::services::vault_namespace::initialize_locked(
                    &data_path,
                    &root_lease,
                    &process_lock,
                )?;
                ensure_overlay_target_root(&data_root, &root_lease.root_scope)?;
            }
        }
        let coordinator = Self {
            data_path,
            data_root,
            vault_root: std::sync::Mutex::new(vault_root),
            store,
            finalizer,
            journal,
            lifecycle,
            in_process: std::sync::Mutex::new(()),
            fault_once: std::sync::Mutex::new(None),
            #[cfg(test)]
            replay_failures_before_targets: std::sync::Mutex::new(0),
            #[cfg(test)]
            pause_after_authority_advance_once: std::sync::Mutex::new(None),
            root_lease: std::sync::Mutex::new(root_lease),
        };
        // Version-1 intents predate content generations. Stable bootstrap drains
        // every intent before it changes the authority namespace; the legacy
        // constructor preserves its historical schema-1-only bootstrap behavior.
        coordinator.recover_preauthority_locked(&process_lock)?;
        for (_, pending) in coordinator.journal.load_pending(&process_lock)? {
            if identity_mode == RootIdentityMode::StableVault || pending.schema_version == 1 {
                coordinator.replay_intent_locked(&process_lock, &pending, false, true)?;
            }
        }
        if let Some(stable_identity) = stable_identity
            .as_ref()
            .filter(|_| !resume_stable_migration)
        {
            let current_lease = coordinator
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let was_legacy = !current_lease.is_stable();
            if was_legacy {
                stable_migration::require_no_retained_mutation_owners(
                    &coordinator.journal,
                    &process_lock,
                )?;
                crate::services::vault_namespace::initialize_locked(
                    &coordinator.data_path,
                    &current_lease,
                    &process_lock,
                )?;
                ensure_overlay_target_root(&coordinator.data_root, &current_lease.root_scope)?;
                crate::services::settings::prepare_twin_data_path_locked(
                    &coordinator.data_path,
                    &vault_path,
                    &current_lease,
                    &process_lock,
                )
                .map_err(|error| MutationError::Invalid(error.to_string()))?;
            }
            #[cfg(feature = "mcp")]
            let stable_lease = if custom_mcp && custom_mcp_binding.is_none() {
                let mut publish_custom_binding = || {
                    custom_mcp::verify_or_install_locked(
                        &coordinator.data_root,
                        &vault_path,
                        stable_identity.root_scope.clone(),
                        &process_lock,
                        true,
                    )
                    .map(|_| ())
                };
                stable_migration::migrate_legacy_to_stable_locked_with_initial_publication(
                    &coordinator.data_path,
                    &coordinator.data_root,
                    &vault_path,
                    stable_identity,
                    &legacy_scope,
                    &current_lease,
                    &coordinator.finalizer.device_id(),
                    &coordinator.journal,
                    &process_lock,
                    &mut publish_custom_binding,
                )?
            } else {
                stable_migration::migrate_legacy_to_stable_locked(
                    &coordinator.data_path,
                    &coordinator.data_root,
                    &vault_path,
                    stable_identity,
                    &legacy_scope,
                    &current_lease,
                    &coordinator.finalizer.device_id(),
                    &coordinator.journal,
                    &process_lock,
                )?
            };
            #[cfg(not(feature = "mcp"))]
            let stable_lease = stable_migration::migrate_legacy_to_stable_locked(
                &coordinator.data_path,
                &coordinator.data_root,
                &vault_path,
                stable_identity,
                &legacy_scope,
                &current_lease,
                &coordinator.finalizer.device_id(),
                &coordinator.journal,
                &process_lock,
            )?;
            if was_legacy {
                coordinator
                    .store
                    .activate_vault_scope_locked(stable_lease.root_scope.clone(), &process_lock)
                    .map_err(MutationError::Store)?;
            }
            crate::services::vault_namespace::initialize_locked(
                &coordinator.data_path,
                &stable_lease,
                &process_lock,
            )?;
            ensure_overlay_target_root(&coordinator.data_root, &stable_lease.root_scope)?;
            if was_legacy {
                crate::services::vault_namespace::invalidate_locked(
                    &coordinator.data_path,
                    &stable_lease,
                    &process_lock,
                )?;
            }
            *coordinator.root_lease.lock().map_err(|_| {
                MutationError::Invalid("Markdown root lease lock poisoned".into())
            })? = stable_lease;
        }
        #[cfg(test)]
        crate::services::vault_namespace::publish_ready_locked(
            &coordinator.data_path,
            &coordinator
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone(),
            &process_lock,
        )?;
        process_lock.unlock()?;
        Ok(coordinator)
    }

    pub(crate) fn writer_device_id(&self) -> DeviceId {
        self.finalizer.device_id()
    }

    pub(crate) fn load_or_create_device_signing_identity(
        &self,
        secret_store: Arc<dyn crate::services::sync::secrets::SecretStore>,
    ) -> Result<crate::services::sync::device::DeviceSigningIdentity, MutationError> {
        let guard = self.begin_root_transition()?;
        crate::services::sync::device::load_or_create_device_signing_identity(
            &self.data_path,
            guard.process_lock(),
            secret_store,
            &self.writer_device_id(),
        )
    }

    #[cfg(test)]
    pub fn fail_once_at(&self, point: MutationFaultPoint) {
        *self.fault_once.lock().expect("fault lock") = Some(point);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_replays_before_targets(&self, count: usize) {
        *self
            .replay_failures_before_targets
            .lock()
            .expect("replay fault lock") = count;
    }

    #[cfg(test)]
    pub(crate) fn pause_after_authority_advance_once(
        &self,
        entered: std::sync::Arc<std::sync::Barrier>,
        resume: std::sync::Arc<std::sync::Barrier>,
    ) {
        *self
            .pause_after_authority_advance_once
            .lock()
            .expect("authority pause lock") = Some((entered, resume));
    }

    pub fn commit_local(
        &self,
        requested_stream: CausalStream,
        source_channel: crate::models::twin_event::SourceChannel,
        targets: Vec<crate::services::twin_events::TargetMutation>,
        drafts: Vec<TwinEventDraft>,
    ) -> Result<MutationCommit, MutationError> {
        let mut plan = Some(MutationPlan::new(
            requested_stream,
            source_channel,
            targets,
            drafts,
        ));
        self.commit_planned(MutationOrigin::Local, &mut || Ok(plan.take()))
    }

    pub fn commit_planned(
        &self,
        origin: MutationOrigin,
        planner: &mut dyn FnMut() -> Result<Option<MutationPlan>, MutationError>,
    ) -> Result<MutationCommit, MutationError> {
        self.commit_planned_with_hooks(origin, planner, &mut |_| Ok(()), &mut |_| Ok(()))
    }

    pub(crate) fn commit_planned_with_hooks(
        &self,
        origin: MutationOrigin,
        planner: &mut dyn FnMut() -> Result<Option<MutationPlan>, MutationError>,
        prepared_hook: &mut dyn FnMut(
            &crate::services::twin_events::MutationIntentV1,
        ) -> Result<(), MutationError>,
        committed_hook: &mut dyn FnMut(&MutationCommit) -> Result<(), MutationError>,
    ) -> Result<MutationCommit, MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        if let Err(error) = self.verify_root_lease_locked() {
            process_lock.unlock()?;
            return Err(error);
        }
        self.journal.cleanup_orphan_temps_locked(&process_lock)?;
        self.recover_preauthority_locked(&process_lock)?;
        for (_, pending) in self.journal.load_pending(&process_lock)? {
            if let Err(error) = self.replay_intent_locked(&process_lock, &pending, false, true) {
                process_lock.unlock()?;
                return Err(error);
            }
        }
        let Some(plan) = (match planner() {
            Ok(plan) => plan,
            Err(error) => {
                process_lock.unlock()?;
                if origin == MutationOrigin::Local {
                    self.lifecycle.known_failure(None, &error.to_string());
                }
                return Err(error);
            }
        }) else {
            process_lock.unlock()?;
            return Ok(MutationCommit {
                mutation_id: None,
                events: Vec::new(),
                authority_token: None,
                postcommit_warning: false,
            });
        };
        if let Some(expected) = plan.expected_authority.as_ref() {
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let current = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                &process_lock,
            )?;
            if &current != expected {
                process_lock.unlock()?;
                let error = MutationError::RecoveryConflict(
                    "root authority changed while mutation was in flight".into(),
                );
                if origin == MutationOrigin::Local {
                    self.lifecycle.known_failure(None, &error.to_string());
                }
                return Err(error);
            }
        }
        let prepared = match self.prepare_intent(
            &process_lock,
            origin,
            plan.requested_stream,
            plan.source_channel,
            plan.targets,
            plan.drafts,
            plan.retain_commit_receipt,
        ) {
            Ok(Some(intent)) => intent,
            Ok(None) => {
                process_lock.unlock()?;
                return Ok(MutationCommit {
                    mutation_id: None,
                    events: Vec::new(),
                    authority_token: None,
                    postcommit_warning: false,
                });
            }
            Err(error) => {
                process_lock.unlock()?;
                if origin == MutationOrigin::Local {
                    self.lifecycle.known_failure(None, &error.to_string());
                }
                return Err(error);
            }
        };

        let invoke_lifecycle = origin == MutationOrigin::Local && !prepared.events.is_empty();
        let mut preauthority_expected = None;
        if origin == MutationOrigin::Local && intent_changes_authority(&prepared) {
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let current = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                &process_lock,
            )?;
            let intended_authority = crate::services::vault_namespace::VaultAuthorityTokenV1 {
                root_scope: current.root_scope.clone(),
                lease_epoch_uuid: current.lease_epoch_uuid.clone(),
                authority_generation: current.authority_generation.checked_add(1).ok_or_else(
                    || MutationError::RecoveryConflict("authority-generation-exhausted".into()),
                )?,
            };
            if prepared.content_authority_generation
                != Some(intended_authority.authority_generation)
            {
                process_lock.unlock()?;
                return Err(MutationError::RecoveryConflict(
                    "authority generation does not match finalized intent".into(),
                ));
            }
            if prepared.retain_commit_receipt {
                self.journal.preflight_commit_receipt_slot(
                    &process_lock,
                    &prepared,
                    &intended_authority,
                )?;
            }
            preauthority_expected = Some(current);
        }
        if preauthority_expected.is_some() {
            self.validate_exact_preconditions_locked(&prepared)?;
            self.validate_all_targets_before_locked(&prepared)?;
        }
        if let Err(error) = prepared_hook(&prepared) {
            process_lock.unlock()?;
            if origin == MutationOrigin::Local {
                self.lifecycle
                    .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
            }
            return Err(error);
        }
        let mut preauthority_marker = None;
        if let Some(expected) = preauthority_expected.as_ref() {
            if let Err(error) = self.inject(MutationFaultPoint::BeforePreAuthorityMarker) {
                process_lock.unlock()?;
                if origin == MutationOrigin::Local {
                    self.lifecycle
                        .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
                }
                return Err(error);
            }
            self.journal
                .stage_preauthority(&process_lock, expected, &prepared)?;
            let marker = self
                .journal
                .preauthority_for(&process_lock, &prepared.mutation_id)?
                .ok_or_else(|| {
                    MutationError::Invalid("pre-authority mutation disappeared".into())
                })?;
            preauthority_marker = Some(marker);
            if let Err(error) = self.inject(MutationFaultPoint::AfterPreAuthorityMarker) {
                process_lock.unlock()?;
                return Err(error);
            }
        }
        if let Err(error) = self.inject(MutationFaultPoint::AfterPreparedHook) {
            if let Some(marker) = preauthority_marker.as_ref() {
                self.journal.abort_preauthority(&process_lock, marker)?;
            }
            process_lock.unlock()?;
            if origin == MutationOrigin::Local {
                self.lifecycle
                    .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
            }
            return Err(error);
        }
        // Exact guards are validated after the owner Prepared hook and before
        // lifecycle staging. A later guard failure is paired with
        // `known_failure`, including restart recovery, so retained guarded
        // mutations may safely carry their governed event in the same intent.
        if let Err(error) = self.validate_exact_preconditions_locked(&prepared) {
            if let Some(marker) = preauthority_marker.as_ref() {
                self.journal.abort_preauthority(&process_lock, marker)?;
            }
            process_lock.unlock()?;
            if invoke_lifecycle {
                self.lifecycle
                    .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
            }
            return Err(error);
        }
        if invoke_lifecycle {
            if let Err(error) = self.lifecycle.stage_before_local(&prepared) {
                if let Some(marker) = preauthority_marker.as_ref() {
                    self.journal.abort_preauthority(&process_lock, marker)?;
                }
                process_lock.unlock()?;
                self.lifecycle
                    .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
                return Err(error);
            }
        }
        if let Some(preauthority_marker) = preauthority_marker.as_ref() {
            if let Err(error) = self.validate_all_targets_before_locked(&prepared) {
                self.journal
                    .abort_preauthority(&process_lock, preauthority_marker)?;
                process_lock.unlock()?;
                if invoke_lifecycle {
                    self.lifecycle
                        .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
                }
                return Err(error);
            }
        }
        let mut authority_token = None;
        if intent_changes_authority(&prepared) {
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let advanced = match crate::services::vault_namespace::advance_authority_locked(
                &self.data_path,
                &lease,
                &process_lock,
            ) {
                Ok(advanced) => advanced,
                Err(error) => {
                    if let (Some(expected), Some(marker)) =
                        (preauthority_expected.as_ref(), preauthority_marker.as_ref())
                    {
                        let current =
                            crate::services::vault_namespace::capture_authority_token_locked(
                                &self.data_path,
                                &lease,
                                &process_lock,
                            )?;
                        if &current == expected {
                            self.journal.abort_preauthority(&process_lock, marker)?;
                        } else if current.root_scope == expected.root_scope
                            && current.lease_epoch_uuid == expected.lease_epoch_uuid
                            && Some(current.authority_generation)
                                == prepared.content_authority_generation
                        {
                            process_lock.unlock()?;
                            if invoke_lifecycle {
                                self.lifecycle.known_failure(
                                    Some(prepared.mutation_id.as_str()),
                                    &error.to_string(),
                                );
                            }
                            return Err(MutationError::AuthorityAdvanced {
                                mutation_id: prepared.mutation_id.clone(),
                                authority_token: current,
                                target_aborted: false,
                                reason: error.to_string(),
                            });
                        } else {
                            process_lock.unlock()?;
                            return Err(MutationError::RecoveryConflict(
                                "authority advance has an unowned durable state".into(),
                            ));
                        }
                    }
                    process_lock.unlock()?;
                    if invoke_lifecycle {
                        self.lifecycle
                            .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
                    }
                    return Err(error);
                }
            };
            if advanced.authority_generation != prepared.content_authority_generation.unwrap() {
                process_lock.unlock()?;
                let error = MutationError::RecoveryConflict(
                    "content-authority-generation-cas-failed".into(),
                );
                if invoke_lifecycle {
                    self.lifecycle
                        .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
                }
                return Err(error);
            }
            authority_token = Some(advanced);
            #[cfg(test)]
            let pause_after_authority_advance = {
                self.pause_after_authority_advance_once
                    .lock()
                    .map_err(|_| MutationError::Invalid("authority pause lock poisoned".into()))?
                    .take()
            };
            #[cfg(test)]
            if let Some((entered, resume)) = pause_after_authority_advance {
                entered.wait();
                resume.wait();
            }
            if let Err(error) = self.inject(MutationFaultPoint::AfterAuthorityAdvance) {
                process_lock.unlock()?;
                if invoke_lifecycle {
                    self.lifecycle
                        .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
                }
                return Err(MutationError::AuthorityAdvanced {
                    mutation_id: prepared.mutation_id.clone(),
                    authority_token: authority_token
                        .clone()
                        .expect("authority token was just advanced"),
                    target_aborted: false,
                    reason: error.to_string(),
                });
            }
            if preauthority_marker.is_some() {
                if let Err(error) = self.validate_all_targets_before_locked(&prepared) {
                    let authority = authority_token
                        .clone()
                        .expect("authority token was just advanced");
                    let proof_result = self
                        .journal
                        .retain_aborted_after_authority(&process_lock, &prepared, &authority)
                        .and_then(|()| {
                            self.inject(MutationFaultPoint::AfterPostAuthorityAbortProof)
                        });
                    process_lock.unlock()?;
                    if invoke_lifecycle {
                        self.lifecycle
                            .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
                    }
                    return Err(MutationError::AuthorityAdvanced {
                        mutation_id: prepared.mutation_id.clone(),
                        authority_token: authority,
                        target_aborted: true,
                        reason: proof_result.err().unwrap_or(error).to_string(),
                    });
                }
            }
        }
        let stage_result = match preauthority_marker.as_ref() {
            Some(marker) => self.journal.promote_preauthority(&process_lock, marker),
            None => self.journal.stage(&process_lock, &prepared),
        };
        if let Err(error) = stage_result {
            process_lock.unlock()?;
            if invoke_lifecycle {
                self.lifecycle
                    .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
            }
            if let Some(authority_token) = authority_token {
                return Err(MutationError::AuthorityAdvanced {
                    mutation_id: prepared.mutation_id.clone(),
                    authority_token,
                    target_aborted: false,
                    reason: error.to_string(),
                });
            }
            return Err(error);
        }
        let first_replay = self
            .inject(MutationFaultPoint::AfterStage)
            .and_then(|()| self.replay_intent_locked(&process_lock, &prepared, true, false));
        if let Err(
            error @ (MutationError::AbortedPrecondition { .. }
            | MutationError::AuthorityAdvanced { .. }),
        ) = first_replay
        {
            process_lock.unlock()?;
            if invoke_lifecycle {
                self.lifecycle
                    .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
            }
            return Err(error);
        }
        let mut postcommit_warning = first_replay.is_err();
        let mut replay_complete = first_replay.is_ok();
        if !replay_complete {
            if let Err(error) = self.replay_intent_locked(&process_lock, &prepared, false, false) {
                if matches!(
                    error,
                    MutationError::AbortedPrecondition { .. }
                        | MutationError::AuthorityAdvanced { .. }
                ) {
                    process_lock.unlock()?;
                    if invoke_lifecycle {
                        self.lifecycle
                            .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
                    }
                    return Err(error);
                }
                match self
                    .inject(MutationFaultPoint::BeforeDurabilityClassification)
                    .and_then(|()| self.intent_effects_are_durable(&prepared))
                {
                    Ok(false) => {
                        process_lock.unlock()?;
                        if invoke_lifecycle {
                            self.lifecycle.known_failure(
                                Some(prepared.mutation_id.as_str()),
                                &error.to_string(),
                            );
                        }
                        if let Some(authority_token) = authority_token.clone() {
                            return Err(MutationError::AuthorityAdvanced {
                                mutation_id: prepared.mutation_id.clone(),
                                authority_token,
                                target_aborted: false,
                                reason: error.to_string(),
                            });
                        }
                        return Err(error);
                    }
                    Ok(true) => {}
                    Err(classification_error) => {
                        process_lock.unlock()?;
                        if invoke_lifecycle {
                            self.lifecycle.known_failure(
                                Some(prepared.mutation_id.as_str()),
                                &classification_error.to_string(),
                            );
                        }
                        if let Some(authority_token) = authority_token.clone() {
                            return Err(MutationError::AuthorityAdvanced {
                                mutation_id: prepared.mutation_id.clone(),
                                authority_token,
                                target_aborted: false,
                                reason: classification_error.to_string(),
                            });
                        }
                        return Err(classification_error);
                    }
                }
                // The authoritative targets/events are exact and durable, but
                // receipt retention or another post-effect step could not be
                // completed. Keep the journal plus Prepared witness for
                // guarded recovery and return a non-retryable committed
                // warning to the caller.
                log::error!(
                    "mutation {} committed its authoritative effect but post-effect recovery remains pending: {error}",
                    prepared.mutation_id.as_str()
                );
            } else {
                replay_complete = true;
            }
        }
        let mut commit = MutationCommit {
            mutation_id: Some(prepared.mutation_id.clone()),
            events: prepared.events.clone(),
            authority_token,
            postcommit_warning,
        };
        if !replay_complete {
            if let Err(error) = process_lock.unlock() {
                log::error!("committed mutation process-lock release failed: {error}");
            }
            commit.postcommit_warning = true;
            return Ok(commit);
        }
        match committed_hook(&commit) {
            Ok(()) => {}
            Err(error) => {
                log::error!("postcommit mutation publication failed: {error}");
                postcommit_warning = true;
            }
        };
        match self.journal.remove(&process_lock, &prepared) {
            Ok(()) => {}
            Err(error) => {
                log::error!("committed mutation journal cleanup failed: {error}");
                postcommit_warning = true;
            }
        };
        // Retained receipts are consumed only by the owner after its
        // idempotent audit/queue publication has completed under this same
        // process-lock domain. Coordinator cleanup alone cannot prove that
        // external publication finished.
        if let Err(error) = self.inject(MutationFaultPoint::AfterCleanupBeforeFanout) {
            log::error!("committed mutation post-cleanup fault: {error}");
            postcommit_warning = true;
        }
        if let Err(error) = process_lock.unlock() {
            log::error!("committed mutation process-lock release failed: {error}");
            postcommit_warning = true;
        }
        if invoke_lifecycle {
            if let Err(error) = self.lifecycle.committed(&prepared) {
                log::error!("committed mutation lifecycle publication failed: {error}");
                postcommit_warning = true;
            }
        }
        commit.postcommit_warning = postcommit_warning;
        Ok(commit)
    }

    pub fn apply_nonlocal(
        &self,
        origin: MutationOrigin,
        targets: Vec<crate::services::twin_events::TargetMutation>,
    ) -> Result<MutationCommit, MutationError> {
        if origin == MutationOrigin::Local {
            return Err(MutationError::Invalid(
                "local mutations must use commit_local".into(),
            ));
        }
        let source = match origin {
            MutationOrigin::Remote => "remote",
            MutationOrigin::Recovery => "recovery",
            MutationOrigin::Local => unreachable!(),
        };
        let mut plan = Some(MutationPlan::new(
            CausalStream::LocalOnly,
            crate::models::twin_event::SourceChannel::parse(source)
                .map_err(MutationError::Invalid)?,
            targets,
            Vec::new(),
        ));
        self.commit_planned(origin, &mut || Ok(plan.take()))
    }

    pub fn recover_pending(&self) -> Result<usize, MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        self.verify_root_lease_locked()?;
        self.journal.cleanup_orphan_temps_locked(&process_lock)?;
        let preauthority_recovered = self.recover_preauthority_locked(&process_lock)?;
        let pending = self.journal.load_pending(&process_lock)?;
        if let Err(error) = self.inject(MutationFaultPoint::BeforePendingRecovery) {
            process_lock.unlock()?;
            return Err(error);
        }
        let mut recovered = preauthority_recovered;
        for (_, intent) in pending {
            if let Err(error) = self.replay_intent_locked(&process_lock, &intent, false, true) {
                process_lock.unlock()?;
                return Err(error);
            }
            recovered += 1;
        }
        process_lock.unlock()?;
        Ok(recovered)
    }

    pub fn pending_count(&self) -> Result<usize, MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        self.journal.cleanup_orphan_temps_locked(&process_lock)?;
        self.journal.pending_count(&process_lock)
    }

    #[cfg_attr(not(feature = "mcp"), allow(dead_code))]
    pub(crate) fn current_root_epoch(&self) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        let result = (|| {
            self.verify_root_lease_locked()?;
            self.root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))
                .map(|lease| lease.clone())
        })();
        process_lock.unlock()?;
        result
    }

    #[cfg_attr(not(feature = "mcp"), allow(dead_code))]
    pub(crate) fn current_namespace_path(&self) -> Result<PathBuf, MutationError> {
        let lease = self.current_root_epoch()?;
        Ok(crate::services::vault_namespace::scoped_data_path(
            &self.data_path,
            &lease.root_scope,
        ))
    }

    pub(crate) fn data_path(&self) -> &Path {
        &self.data_path
    }

    pub(crate) fn current_authority_token(
        &self,
    ) -> Result<crate::services::vault_namespace::VaultAuthorityTokenV1, MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        let result = (|| {
            self.verify_root_lease_locked()?;
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                &process_lock,
            )
        })();
        process_lock.unlock()?;
        result
    }

    #[cfg(any(test, feature = "mcp"))]
    pub(crate) fn require_namespace_ready(&self) -> Result<(), MutationError> {
        let token = self.current_authority_token()?;
        self.validate_authority_token(&token, true)
    }

    #[cfg(feature = "mcp")]
    pub(crate) fn acquire_ready_namespace_guard(
        &self,
        expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<CoordinatorProcessLock, MutationError> {
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        let result = (|| {
            self.verify_root_lease_locked()?;
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let current = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                &process_lock,
            )?;
            if &current != expected {
                return Err(MutationError::RecoveryConflict(
                    "root authority changed while derived state was in flight".into(),
                ));
            }
            crate::services::vault_namespace::require_ready_token_locked(
                &self.data_path,
                &current,
                &process_lock,
            )
        })();
        if let Err(error) = result {
            process_lock.unlock()?;
            return Err(error);
        }
        Ok(process_lock)
    }

    pub(crate) fn validate_root_epoch(
        &self,
        expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(), MutationError> {
        self.validate_authority_token(expected, false)
    }

    pub(crate) fn validate_authority_token(
        &self,
        expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
        require_ready: bool,
    ) -> Result<(), MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        let result = (|| {
            self.verify_root_lease_locked()?;
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let current = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                &process_lock,
            )?;
            if &current != expected {
                return Err(MutationError::RecoveryConflict(
                    "root authority changed while work was in flight".into(),
                ));
            }
            if require_ready {
                crate::services::vault_namespace::require_ready_token_locked(
                    &self.data_path,
                    &current,
                    &process_lock,
                )?;
            }
            Ok(())
        })();
        process_lock.unlock()?;
        if let Err(error) = result {
            return Err(MutationError::RecoveryConflict(error.to_string()));
        }
        Ok(())
    }

    pub(crate) fn with_locked_derived_state<T>(
        &self,
        expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
        require_ready: bool,
        action: impl FnOnce() -> Result<T, MutationError>,
    ) -> Result<T, MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        let result = (|| {
            self.verify_root_lease_locked()?;
            self.journal.cleanup_orphan_temps_locked(&process_lock)?;
            self.recover_preauthority_locked(&process_lock)?;
            for (_, pending) in self.journal.load_pending(&process_lock)? {
                self.replay_intent_locked(&process_lock, &pending, false, true)?;
            }
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let current = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                &process_lock,
            )?;
            if &current != expected {
                return Err(MutationError::RecoveryConflict(
                    "root authority changed before derived-state access".into(),
                ));
            }
            if require_ready {
                crate::services::vault_namespace::require_ready_token_locked(
                    &self.data_path,
                    &current,
                    &process_lock,
                )?;
            }
            action()
        })();
        process_lock.unlock()?;
        result
    }

    #[cfg(test)]
    pub(crate) fn retarget_markdown_root(&self, vault_path: &Path) -> Result<(), MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        let result = (|| -> Result<(), MutationError> {
            self.verify_root_lease_locked()?;
            crate::services::twin_events::validate_real_directory(
                vault_path,
                "trusted Markdown vault root",
            )?;
            let new_root = fs::canonicalize(vault_path)?;
            validate_disjoint_roots(
                &new_root,
                self.data_root.canonical_path().join("canvas").as_path(),
                self.data_root.canonical_path().join("twin").as_path(),
            )?;
            let new_root_capability = crate::services::twin_events::AnchoredRoot::open(&new_root)?;
            self.journal.cleanup_orphan_temps_locked(&process_lock)?;
            self.recover_preauthority_locked(&process_lock)?;
            for (_, pending) in self.journal.load_pending(&process_lock)? {
                self.replay_intent_locked(&process_lock, &pending, false, true)?;
            }
            let next_lease = ActiveMarkdownRootLeaseV1 {
                schema_version: ACTIVE_ROOT_LEASE_SCHEMA_VERSION,
                root_scope: markdown_root_scope_for(&new_root)?,
                epoch_uuid: Uuid::new_v4().to_string(),
            };
            crate::services::vault_namespace::initialize_locked(
                &self.data_path,
                &next_lease,
                &process_lock,
            )?;
            ensure_overlay_target_root(&self.data_root, &next_lease.root_scope)?;
            write_active_root_lease(&self.data_root, &next_lease)?;
            *self
                .vault_root
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))? =
                new_root_capability;
            *self.root_lease.lock().map_err(|_| {
                MutationError::Invalid("Markdown root lease lock poisoned".into())
            })? = next_lease;
            Ok(())
        })();
        process_lock.unlock()?;
        result
    }

    pub(crate) fn begin_root_transition(
        &self,
    ) -> Result<MutationRootTransitionGuard<'_>, MutationError> {
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        let result = (|| -> Result<(), MutationError> {
            self.verify_root_lease_locked()?;
            self.journal.cleanup_orphan_temps_locked(&process_lock)?;
            self.recover_preauthority_locked(&process_lock)?;
            for (_, pending) in self.journal.load_pending(&process_lock)? {
                self.replay_intent_locked(&process_lock, &pending, false, true)?;
            }
            self.verify_root_lease_locked()
        })();
        if let Err(error) = result {
            process_lock.unlock()?;
            return Err(error);
        }
        Ok(MutationRootTransitionGuard {
            coordinator: self,
            _process_lock: process_lock,
        })
    }

    /// Clears durable readiness before a repair attempts WAL replay. Unlike
    /// `begin_root_transition`, this deliberately does not replay pending
    /// intents first: a replay failure must leave every peer fail-closed.
    pub(crate) fn invalidate_namespace_before_recovery(
        &self,
        expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(), MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        let result = (|| {
            self.verify_root_lease_locked()?;
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let current = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                &process_lock,
            )?;
            if current.root_scope != expected.root_scope
                || current.lease_epoch_uuid != expected.lease_epoch_uuid
            {
                return Err(MutationError::RecoveryConflict(
                    "vault root changed before authority recovery".into(),
                ));
            }
            crate::services::vault_namespace::invalidate_locked(
                &self.data_path,
                &lease,
                &process_lock,
            )
        })();
        process_lock.unlock()?;
        result
    }

    pub fn quarantine_count(&self) -> Result<usize, MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        self.journal.quarantine_count(&process_lock)
    }
}

fn intent_changes_authority(intent: &crate::services::twin_events::MutationIntentV1) -> bool {
    !intent.events.is_empty()
        || intent.targets.iter().any(|target| {
            matches!(
                target.kind,
                crate::services::twin_events::TargetKind::Markdown
                    | crate::services::twin_events::TargetKind::OverlayJson
                    | crate::services::twin_events::TargetKind::TwinJson
            )
        })
}

impl MutationRootTransitionGuard<'_> {
    pub(crate) fn process_lock(&self) -> &CoordinatorProcessLock {
        &self._process_lock
    }

    pub(crate) fn current_lease(&self) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
        self.coordinator
            .root_lease
            .lock()
            .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))
            .map(|lease| lease.clone())
    }

    pub(crate) fn initialize_namespace(
        &self,
        lease: &ActiveMarkdownRootLeaseV1,
    ) -> Result<PathBuf, MutationError> {
        crate::services::vault_namespace::initialize_locked(
            &self.coordinator.data_path,
            lease,
            &self._process_lock,
        )
    }

    pub(crate) fn prepare_twin_data_path(
        &self,
        vault_path: &Path,
        lease: &ActiveMarkdownRootLeaseV1,
    ) -> Result<PathBuf, MutationError> {
        crate::services::settings::prepare_twin_data_path_locked(
            &self.coordinator.data_path,
            vault_path,
            lease,
            &self._process_lock,
        )
        .map_err(|error| MutationError::Invalid(error.to_string()))
    }

    pub(crate) fn invalidate_namespace(
        &self,
        lease: &ActiveMarkdownRootLeaseV1,
    ) -> Result<(), MutationError> {
        crate::services::vault_namespace::invalidate_locked(
            &self.coordinator.data_path,
            lease,
            &self._process_lock,
        )
    }

    pub(crate) fn publish_namespace_ready(
        &self,
        token: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(), MutationError> {
        crate::services::vault_namespace::publish_ready_token_locked(
            &self.coordinator.data_path,
            token,
            &self._process_lock,
        )
    }

    pub(crate) fn capture_authority_token(
        &self,
        lease: &ActiveMarkdownRootLeaseV1,
    ) -> Result<crate::services::vault_namespace::VaultAuthorityTokenV1, MutationError> {
        crate::services::vault_namespace::capture_authority_token_locked(
            &self.coordinator.data_path,
            lease,
            &self._process_lock,
        )
    }

    /// Validates an authority token while retaining the already-acquired
    /// coordinator process lock. Rebuild code must use this helper instead of
    /// calling `MutationCoordinator::validate_authority_token`, which would
    /// recursively try to acquire the same cross-process lock.
    pub(crate) fn validate_authority_token(
        &self,
        expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
        require_ready: bool,
    ) -> Result<(), MutationError> {
        self.coordinator.verify_root_lease_locked()?;
        let lease = self.current_lease()?;
        let current = self.capture_authority_token(&lease)?;
        if &current != expected {
            return Err(MutationError::RecoveryConflict(
                "root authority changed while retained work was in flight".into(),
            ));
        }
        if require_ready {
            crate::services::vault_namespace::require_ready_token_locked(
                &self.coordinator.data_path,
                &current,
                &self._process_lock,
            )?;
        }
        Ok(())
    }

    pub(crate) fn classify_witnessed_mutation(
        &self,
        mutation_id: &crate::models::twin_event::ContentDigest,
        expected_authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
        target_kind: crate::services::twin_events::TargetKind,
        target_key: &str,
        expected_before: &crate::services::twin_events::BeforeImage,
        expected_after: &crate::services::twin_events::BeforeImage,
    ) -> Result<WitnessedMutationRecovery, MutationError> {
        self.coordinator.classify_witnessed_mutation_locked(
            &self._process_lock,
            mutation_id,
            expected_authority,
            target_kind,
            target_key,
            expected_before,
            expected_after,
        )
    }

    pub(crate) fn consume_witnessed_mutation_receipt(
        &self,
        mutation_id: &crate::models::twin_event::ContentDigest,
    ) -> Result<(), MutationError> {
        let receipt = self
            .coordinator
            .journal
            .load_committed_receipt(&self._process_lock, mutation_id)?;
        let marker = self
            .coordinator
            .journal
            .preauthority_for(&self._process_lock, mutation_id)?;
        match (receipt, marker) {
            (Some(_), None) => self
                .coordinator
                .journal
                .consume_committed_receipt(&self._process_lock, mutation_id),
            (None, Some(marker))
                if matches!(
                    marker.state,
                    crate::services::twin_events::PreAuthorityMutationStateV1::AbortedBeforeAuthority
                        | crate::services::twin_events::PreAuthorityMutationStateV1::AbortedAfterAuthority
                ) =>
            {
                self.coordinator
                    .journal
                    .consume_aborted_preauthority(&self._process_lock, mutation_id)
            }
            (None, None) => Ok(()),
            _ => Err(MutationError::RecoveryConflict(
                "witnessed mutation has conflicting or uncommitted owner proof".into(),
            )),
        }
    }

    pub(crate) fn current_vault_path(&self) -> Result<PathBuf, MutationError> {
        self.coordinator
            .vault_root
            .lock()
            .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))
            .map(|root| root.canonical_path().to_path_buf())
    }

    pub(crate) fn adopt_durable_root(
        &self,
        vault_path: &Path,
        lease: &ActiveMarkdownRootLeaseV1,
    ) -> Result<(), MutationError> {
        crate::services::twin_events::validate_real_directory(
            vault_path,
            "trusted Markdown vault root",
        )?;
        let canonical = fs::canonicalize(vault_path)?;
        validate_disjoint_roots(
            &canonical,
            self.coordinator
                .data_root
                .canonical_path()
                .join("canvas")
                .as_path(),
            self.coordinator
                .data_root
                .canonical_path()
                .join("twin")
                .as_path(),
        )?;
        let durable = load_active_root_lease(&self.coordinator.data_root)?;
        if &durable != lease || lease.root_scope != root_scope_for_lease(&canonical, lease)? {
            return Err(MutationError::RecoveryConflict(
                "root-transition-lease-mismatch".into(),
            ));
        }
        let capability = crate::services::twin_events::AnchoredRoot::open(&canonical)?;
        crate::services::vault_namespace::initialize_locked(
            &self.coordinator.data_path,
            lease,
            &self._process_lock,
        )?;
        ensure_overlay_target_root(&self.coordinator.data_root, &lease.root_scope)?;
        if lease.is_stable() {
            self.coordinator
                .store
                .activate_vault_scope_locked(lease.root_scope.clone(), &self._process_lock)
                .map_err(MutationError::Store)?;
        }
        *self
            .coordinator
            .vault_root
            .lock()
            .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))? =
            capability;
        *self
            .coordinator
            .root_lease
            .lock()
            .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))? =
            lease.clone();
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetClassification {
    Before,
    After,
    Third,
}

fn target_is_exact_precondition(
    intent: &crate::services::twin_events::MutationIntentV1,
    target: &crate::services::twin_events::MutationTargetV1,
) -> bool {
    intent.schema_version == 3
        && intent.retain_commit_receipt
        && matches!(
            (&target.before, &target.after),
            (
                crate::services::twin_events::BeforeImage::Sha256(before),
                crate::services::twin_events::DesiredImage::Utf8Bytes(_),
            ) if before == &target.after_digest
        )
}

fn target_matches_after(
    current: &crate::services::twin_events::BeforeImage,
    desired: &crate::services::twin_events::DesiredImage,
    after_digest: &crate::models::twin_event::ContentDigest,
) -> bool {
    match (current, desired) {
        (
            crate::services::twin_events::BeforeImage::Absent,
            crate::services::twin_events::DesiredImage::Tombstone,
        ) => true,
        (
            crate::services::twin_events::BeforeImage::Sha256(current),
            crate::services::twin_events::DesiredImage::Utf8Bytes(_),
        ) => current == after_digest,
        _ => false,
    }
}

#[allow(private_interfaces)] // The public trait is implemented by an app-internal coordinator.
impl crate::services::twin_events::EventRecorder for MutationCoordinator {
    fn current_authority_token(
        &self,
    ) -> Result<crate::services::vault_namespace::VaultAuthorityTokenV1, MutationError> {
        MutationCoordinator::current_authority_token(self)
    }

    fn classify_witnessed_mutation(
        &self,
        mutation_id: &crate::models::twin_event::ContentDigest,
        expected_authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
        target_kind: crate::services::twin_events::TargetKind,
        target_key: &str,
        expected_before: &crate::services::twin_events::BeforeImage,
        expected_after: &crate::services::twin_events::BeforeImage,
    ) -> Result<WitnessedMutationRecovery, MutationError> {
        self.begin_root_transition()?.classify_witnessed_mutation(
            mutation_id,
            expected_authority,
            target_kind,
            target_key,
            expected_before,
            expected_after,
        )
    }

    fn consume_witnessed_mutation_receipt(
        &self,
        mutation_id: &crate::models::twin_event::ContentDigest,
    ) -> Result<(), MutationError> {
        self.begin_root_transition()?
            .consume_witnessed_mutation_receipt(mutation_id)
    }

    fn commit_mutation(
        &self,
        origin: MutationOrigin,
        stream: CausalStream,
        source_channel: crate::models::twin_event::SourceChannel,
        targets: Vec<crate::services::twin_events::TargetMutation>,
        drafts: Vec<TwinEventDraft>,
    ) -> Result<MutationCommit, MutationError> {
        match origin {
            MutationOrigin::Local => self.commit_local(stream, source_channel, targets, drafts),
            MutationOrigin::Remote | MutationOrigin::Recovery => {
                self.apply_nonlocal(origin, targets)
            }
        }
    }

    fn commit_planned_mutation(
        &self,
        origin: MutationOrigin,
        planner: &mut dyn FnMut() -> Result<Option<MutationPlan>, MutationError>,
    ) -> Result<MutationCommit, MutationError> {
        self.commit_planned(origin, planner)
    }

    fn commit_planned_mutation_with_hooks(
        &self,
        origin: MutationOrigin,
        planner: &mut dyn FnMut() -> Result<Option<MutationPlan>, MutationError>,
        prepared: &mut dyn FnMut(
            &crate::services::twin_events::MutationIntentV1,
        ) -> Result<(), MutationError>,
        committed: &mut dyn FnMut(&MutationCommit) -> Result<(), MutationError>,
    ) -> Result<MutationCommit, MutationError> {
        self.commit_planned_with_hooks(origin, planner, prepared, committed)
    }

    fn recover_pending_mutations(&self) -> Result<usize, MutationError> {
        self.recover_pending()
    }

    #[cfg(test)]
    fn retarget_markdown_root(&self, vault_path: &Path) -> Result<(), MutationError> {
        MutationCoordinator::retarget_markdown_root(self, vault_path)
    }

    fn recorded_events(&self) -> Result<Vec<TwinEvent>, MutationError> {
        self.store.ordered_events().map_err(MutationError::from)
    }
}

#[cfg(test)]
#[path = "mutation_coordinator_tests.rs"]
mod tests;
