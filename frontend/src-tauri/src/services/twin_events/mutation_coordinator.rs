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

#[derive(Debug)]
pub enum MutationError {
    Store(StoreError),
    Io(String),
    Invalid(String),
    RecoveryConflict(String),
}

impl std::fmt::Display for MutationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => error.fmt(formatter),
            Self::Io(message) | Self::Invalid(message) => formatter.write_str(message),
            Self::RecoveryConflict(id) => {
                write!(formatter, "recoverable local mutation conflict: {id}")
            }
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
    pub fn load_or_create(data_path: impl AsRef<Path>) -> Result<Self, MutationError> {
        const WRITER_KEY: &str = "twin/events/writer-v1.json";
        const STAGING_KEY: &str = "twin/events/staging/v1";
        let root = crate::services::twin_events::AnchoredRoot::open(data_path)?;
        root.open_directory("twin/events", false)?;
        root.open_directory(STAGING_KEY, true)?;
        if let Some(bytes) = root.read_bounded(WRITER_KEY, WRITER_FILE_LIMIT as usize)? {
            return Self::load(&bytes);
        }

        let identity = WriterIdentityV1 {
            schema_version: WRITER_SCHEMA_VERSION,
            device_id: DeviceId::parse(Uuid::new_v4().to_string())
                .map_err(MutationError::Invalid)?,
            actor_id: ActorId::parse("owner").map_err(MutationError::Invalid)?,
        };
        let mut bytes = serde_json::to_vec_pretty(&identity)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        bytes.push(b'\n');
        root.install_no_clobber(WRITER_KEY, STAGING_KEY, &bytes)?;
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
}

impl StoreEventGroupFinalizer {
    pub fn new(store: Arc<TwinEventStore>, identity: Arc<dyn MutationIdentityProvider>) -> Self {
        Self { store, identity }
    }

    pub(crate) fn acquire_coordinator_lock(&self) -> Result<CoordinatorProcessLock, MutationError> {
        acquire_shared_coordinator_process_lock(self.store.data_path())
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
    pub(crate) expected_authority:
        Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
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
        }
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
    AfterAuthorityAdvance,
    AfterStage,
    AfterTarget(usize),
    AfterTargets,
    AfterEvent(usize),
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

#[derive(Debug, Clone)]
pub struct MutationCommit {
    pub mutation_id: Option<crate::models::twin_event::ContentDigest>,
    pub events: Vec<TwinEvent>,
    pub(crate) authority_token:
        Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
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
    root_lease: std::sync::Mutex<ActiveMarkdownRootLeaseV1>,
}

pub(crate) struct MutationRootTransitionGuard<'a> {
    coordinator: &'a MutationCoordinator,
    _process_lock: CoordinatorProcessLock,
}

const ACTIVE_ROOT_LEASE_SCHEMA_VERSION: u16 = 1;
const ACTIVE_ROOT_LEASE_LIMIT: u64 = 4096;

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
}

impl MutationCoordinator {
    pub fn new(
        data_path: impl AsRef<Path>,
        vault_path: impl AsRef<Path>,
        store: Arc<TwinEventStore>,
        lifecycle: Arc<dyn MutationLifecycle>,
    ) -> Result<Self, MutationError> {
        let data_path = data_path.as_ref().to_path_buf();
        let vault_path = vault_path.as_ref().to_path_buf();
        crate::services::twin_events::validate_real_directory(&data_path, "trusted app-data root")?;
        crate::services::twin_events::validate_real_directory(
            &vault_path,
            "trusted Markdown vault root",
        )?;
        let data_path = fs::canonicalize(data_path)?;
        let vault_path = fs::canonicalize(vault_path)?;
        let data_root = crate::services::twin_events::AnchoredRoot::open(&data_path)?;
        data_root.open_directory("canvas", true)?;
        let twin_path = fs::canonicalize(data_path.join("twin"))?;
        let canvas_path = fs::canonicalize(data_path.join("canvas"))?;
        validate_disjoint_roots(&vault_path, &canvas_path, &twin_path)?;
        let vault_root = crate::services::twin_events::AnchoredRoot::open(&vault_path)?;
        let identity = Arc::new(PersistedMutationIdentityProvider::load_or_create(
            &data_path,
        )?);
        let finalizer = StoreEventGroupFinalizer::new(store.clone(), identity);
        let journal = crate::services::twin_events::LocalMutationJournal::initialize(&data_path)?;
        let process_lock = finalizer.acquire_coordinator_lock()?;
        journal.cleanup_orphan_temps_locked(&process_lock)?;
        let root_scope = markdown_root_scope_for(&vault_path)?;
        let root_lease = load_or_create_active_root_lease(&data_root, root_scope)?;
        crate::services::vault_namespace::initialize_locked(
            &data_path,
            &root_lease,
            &process_lock,
        )?;
        ensure_overlay_target_root(&data_root, &root_lease.root_scope)?;
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
            root_lease: std::sync::Mutex::new(root_lease),
        };
        // Version-1 intents predate content generations. Replay them first so an
        // upgrade cannot advance authority ahead of a still-pending mutation.
        for (_, pending) in coordinator.journal.load_pending(&process_lock)? {
            if pending.schema_version == 1 {
                coordinator.replay_intent_locked(&process_lock, &pending, false)?;
            }
        }
        crate::services::vault_namespace::initialize_authority_locked(
            &coordinator.data_path,
            &process_lock,
        )?;
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

    #[cfg(test)]
    pub fn fail_once_at(&self, point: MutationFaultPoint) {
        *self.fault_once.lock().expect("fault lock") = Some(point);
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
        for (_, pending) in self.journal.load_pending(&process_lock)? {
            if let Err(error) = self.replay_intent_locked(&process_lock, &pending, false) {
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
        ) {
            Ok(Some(intent)) => intent,
            Ok(None) => {
                process_lock.unlock()?;
                return Ok(MutationCommit {
                    mutation_id: None,
                    events: Vec::new(),
                    authority_token: None,
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
        if invoke_lifecycle {
            if let Err(error) = self.lifecycle.stage_before_local(&prepared) {
                process_lock.unlock()?;
                self.lifecycle
                    .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
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
                    process_lock.unlock()?;
                    if invoke_lifecycle {
                        self.lifecycle.known_failure(
                            Some(prepared.mutation_id.as_str()),
                            &error.to_string(),
                        );
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
            if let Err(error) = self.inject(MutationFaultPoint::AfterAuthorityAdvance) {
                process_lock.unlock()?;
                if invoke_lifecycle {
                    self.lifecycle
                        .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
                }
                return Err(error);
            }
        }
        if let Err(error) = self.journal.stage(&process_lock, &prepared) {
            process_lock.unlock()?;
            if invoke_lifecycle {
                self.lifecycle
                    .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
            }
            return Err(error);
        }
        let committed = self
            .inject(MutationFaultPoint::AfterStage)
            .and_then(|()| self.replay_intent_locked(&process_lock, &prepared, true));
        let unlock = process_lock.unlock();
        if let Err(error) = unlock {
            return Err(error.into());
        }
        if let Err(error) = committed {
            if invoke_lifecycle {
                self.lifecycle
                    .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
            }
            return Err(error);
        }
        if invoke_lifecycle {
            self.lifecycle.committed(&prepared)?;
        }
        Ok(MutationCommit {
            mutation_id: Some(prepared.mutation_id.clone()),
            events: prepared.events,
            authority_token,
        })
    }

    pub fn apply_nonlocal(
        &self,
        origin: MutationOrigin,
        targets: Vec<crate::services::twin_events::TargetMutation>,
    ) -> Result<(), MutationError> {
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
        self.commit_planned(origin, &mut || Ok(plan.take()))?;
        Ok(())
    }

    pub fn recover_pending(&self) -> Result<usize, MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        self.verify_root_lease_locked()?;
        self.journal.cleanup_orphan_temps_locked(&process_lock)?;
        let pending = self.journal.load_pending(&process_lock)?;
        let mut recovered = 0usize;
        for (_, intent) in pending {
            if let Err(error) = self.replay_intent_locked(&process_lock, &intent, false) {
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
            return Err(MutationError::RecoveryConflict(
                error.to_string(),
            ));
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
            for (_, pending) in self.journal.load_pending(&process_lock)? {
                self.replay_intent_locked(&process_lock, &pending, false)?;
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
            for (_, pending) in self.journal.load_pending(&process_lock)? {
                self.replay_intent_locked(&process_lock, &pending, false)?;
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
            for (_, pending) in self.journal.load_pending(&process_lock)? {
                self.replay_intent_locked(&process_lock, &pending, false)?;
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

    pub fn quarantine_count(&self) -> Result<usize, MutationError> {
        let _in_process = self
            .in_process
            .lock()
            .map_err(|_| MutationError::Invalid("mutation coordinator lock poisoned".into()))?;
        let process_lock = self.finalizer.acquire_coordinator_lock()?;
        self.journal.quarantine_count(&process_lock)
    }

    fn prepare_intent(
        &self,
        process_lock: &CoordinatorProcessLock,
        origin: MutationOrigin,
        requested_stream: CausalStream,
        source_channel: crate::models::twin_event::SourceChannel,
        targets: Vec<crate::services::twin_events::TargetMutation>,
        drafts: Vec<TwinEventDraft>,
    ) -> Result<Option<crate::services::twin_events::MutationIntentV1>, MutationError> {
        if targets.len() > crate::services::twin_events::MAX_INTENT_TARGETS {
            return Err(MutationError::Invalid(
                "local mutation must contain at most 64 targets".into(),
            ));
        }
        if targets.is_empty() && drafts.is_empty() {
            return Err(MutationError::Invalid(
                "mutation must contain a target or event draft".into(),
            ));
        }
        let event_only = targets.is_empty();
        let mut targets = targets;
        targets.sort_by(|left, right| {
            (left.kind, left.relative_key.as_str()).cmp(&(right.kind, right.relative_key.as_str()))
        });
        let mut physical_keys = std::collections::BTreeSet::new();
        for target in &targets {
            let key = self.physical_target_id(target.kind, &target.relative_key)?;
            if !physical_keys.insert(key) {
                return Err(MutationError::Invalid(
                    "duplicate or aliased mutation target".into(),
                ));
            }
        }
        let mut prepared_targets = Vec::new();
        for target in targets {
            crate::services::twin_events::validate_target_key(
                target.kind,
                &target.relative_key,
            )?;
            let before = self.before_image(target.kind, &target.relative_key)?;
            let after_digest = crate::services::twin_events::desired_digest(&target.after);
            if target_matches_after(&before, &target.after, &after_digest) {
                continue;
            }
            if target
                .expected_before
                .as_ref()
                .is_some_and(|expected| expected != &before)
            {
                return Err(MutationError::RecoveryConflict(format!(
                    "conditional mutation target changed: {}",
                    target.relative_key
                )));
            }
            prepared_targets.push(crate::services::twin_events::MutationTargetV1 {
                kind: target.kind,
                relative_key: target.relative_key,
                before,
                after: target.after,
                after_digest,
            });
        }
        if prepared_targets.is_empty() && !event_only {
            return Ok(None);
        }
        if origin != MutationOrigin::Local && !drafts.is_empty() {
            return Err(MutationError::Invalid(
                "nonlocal mutations cannot generate local events".into(),
            ));
        }
        let drafts = drafts
            .into_iter()
            .map(|mut draft| {
                draft.context.source_channel = source_channel.clone();
                draft
            })
            .collect::<Vec<_>>();
        let events = if drafts.is_empty() {
            Vec::new()
        } else {
            self.finalizer.finalize_locked(requested_stream, &drafts)?
        };
        let stream = events
            .first()
            .map_or(requested_stream, |event| event.causal_stream);
        let markdown_root_scope = if prepared_targets
            .iter()
            .any(|target| {
                matches!(
                    target.kind,
                    crate::services::twin_events::TargetKind::Markdown
                        | crate::services::twin_events::TargetKind::OverlayJson
                )
            })
        {
            Some(self.current_markdown_root_scope()?)
        } else {
            None
        };
        let changes_authority = !events.is_empty()
            || prepared_targets.iter().any(|target| {
                matches!(
                    target.kind,
                    crate::services::twin_events::TargetKind::Markdown
                        | crate::services::twin_events::TargetKind::OverlayJson
                        | crate::services::twin_events::TargetKind::TwinJson
                )
            });
        let content_authority_generation = if changes_authority {
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let current = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                process_lock,
            )?;
            Some(current.authority_generation.checked_add(1).ok_or_else(|| {
                MutationError::RecoveryConflict("authority-generation-exhausted".into())
            })?)
        } else {
            None
        };
        let mut intent = crate::services::twin_events::MutationIntentV1 {
            schema_version: 2,
            mutation_id: crate::services::twin_events::digest_bytes(b"placeholder"),
            origin,
            actor_id: self.finalizer.actor_id(),
            device_id: self.finalizer.device_id(),
            causal_stream: stream,
            source_channel,
            markdown_root_scope,
            content_authority_generation,
            targets: prepared_targets,
            events,
            created_at: Utc::now(),
        };
        intent.mutation_id = crate::services::twin_events::derive_mutation_id(&intent);
        intent.validate()?;
        self.store.preflight_append_group(&intent.events)?;
        let serialized = serde_json::to_vec(&intent)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        if serialized.len() > crate::services::twin_events::MAX_SERIALIZED_INTENT_BYTES {
            return Err(MutationError::Invalid(
                "serialized mutation intent exceeds the 32 MiB limit".into(),
            ));
        }
        Ok(Some(intent))
    }

    fn replay_intent_locked(
        &self,
        process_lock: &CoordinatorProcessLock,
        intent: &crate::services::twin_events::MutationIntentV1,
        inject_faults: bool,
    ) -> Result<(), MutationError> {
        intent.validate()?;
        self.store.preflight_append_group(&intent.events)?;
        if let Some(intent_scope) = &intent.markdown_root_scope {
            if &self.current_markdown_root_scope()? != intent_scope {
                self.journal.quarantine_intent(process_lock, intent)?;
                return Err(MutationError::RecoveryConflict(
                    intent.mutation_id.as_str().to_string(),
                ));
            }
        }
        if intent.schema_version == 2 && intent_changes_authority(intent) {
            let lease = self
                .root_lease
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
                .clone();
            let current = crate::services::vault_namespace::capture_authority_token_locked(
                &self.data_path,
                &lease,
                process_lock,
            )?;
            if Some(current.authority_generation) != intent.content_authority_generation {
                self.journal.quarantine_intent(process_lock, intent)?;
                return Err(MutationError::RecoveryConflict(
                    intent.mutation_id.as_str().to_string(),
                ));
            }
        }
        let mut classifications = Vec::with_capacity(intent.targets.len());
        for target in &intent.targets {
            let current = self.before_image(target.kind, &target.relative_key)?;
            let classification = if current == target.before {
                TargetClassification::Before
            } else if target_matches_after(&current, &target.after, &target.after_digest) {
                TargetClassification::After
            } else {
                TargetClassification::Third
            };
            classifications.push(classification);
        }
        if classifications.contains(&TargetClassification::Third) {
            self.journal.quarantine_intent(process_lock, intent)?;
            return Err(MutationError::RecoveryConflict(
                intent.mutation_id.as_str().to_string(),
            ));
        }

        let mut application_order = intent
            .targets
            .iter()
            .enumerate()
            .filter(|(index, target)| {
                classifications[*index] == TargetClassification::Before
                    && !matches!(
                        target.after,
                        crate::services::twin_events::DesiredImage::Tombstone
                    )
            })
            .chain(intent.targets.iter().enumerate().filter(|(index, target)| {
                classifications[*index] == TargetClassification::Before
                    && matches!(
                        target.after,
                        crate::services::twin_events::DesiredImage::Tombstone
                    )
            }))
            .collect::<Vec<_>>();
        for (applied_index, (_, target)) in application_order.drain(..).enumerate() {
            self.apply_intent_target(target)?;
            if inject_faults {
                self.inject(MutationFaultPoint::AfterTarget(applied_index))?;
            }
        }
        if inject_faults {
            self.inject(MutationFaultPoint::AfterTargets)?;
        }
        for (index, event) in intent.events.iter().enumerate() {
            self.store.append(event.clone())?;
            if inject_faults {
                self.inject(MutationFaultPoint::AfterEvent(index))?;
            }
        }
        if inject_faults {
            self.inject(MutationFaultPoint::BeforeCleanup)?;
        }
        self.journal.remove(process_lock, intent)?;
        if inject_faults {
            self.inject(MutationFaultPoint::AfterCleanupBeforeFanout)?;
        }
        Ok(())
    }

    fn apply_intent_target(
        &self,
        target: &crate::services::twin_events::MutationTargetV1,
    ) -> Result<(), MutationError> {
        self.apply_target_mutation(&crate::services::twin_events::TargetMutation {
            kind: target.kind,
            relative_key: target.relative_key.clone(),
            after: target.after.clone(),
            expected_before: None,
        })
    }

    fn apply_target_mutation(
        &self,
        target: &crate::services::twin_events::TargetMutation,
    ) -> Result<(), MutationError> {
        crate::services::twin_events::validate_target_key(target.kind, &target.relative_key)?;
        match &target.after {
            crate::services::twin_events::DesiredImage::Utf8Bytes(content) => {
                self.put_target(target.kind, &target.relative_key, content.as_bytes())?;
            }
            crate::services::twin_events::DesiredImage::Tombstone => {
                self.delete_target(target.kind, &target.relative_key)?;
            }
        }
        Ok(())
    }

    fn target_root(
        &self,
        kind: crate::services::twin_events::TargetKind,
    ) -> Result<PathBuf, MutationError> {
        Ok(match kind {
            crate::services::twin_events::TargetKind::Markdown => self
                .vault_root
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))?
                .canonical_path()
                .to_path_buf(),
            crate::services::twin_events::TargetKind::OverlayJson => {
                let lease = self
                    .root_lease
                    .lock()
                    .map_err(|_| {
                        MutationError::Invalid("Markdown root lease lock poisoned".into())
                    })?
                    .clone();
                crate::services::vault_namespace::scoped_data_path(
                    &self.data_path,
                    &lease.root_scope,
                )
                .join("vault_migration")
                .join("overlay")
                .join("notes")
            }
            crate::services::twin_events::TargetKind::TwinJson => self.data_path.join("twin"),
            crate::services::twin_events::TargetKind::CanvasJson => self.data_path.join("canvas"),
        })
    }

    fn current_markdown_root_scope(
        &self,
    ) -> Result<crate::models::twin_event::ContentDigest, MutationError> {
        markdown_root_scope_for(
            &self.target_root(crate::services::twin_events::TargetKind::Markdown)?,
        )
    }

    fn verify_root_lease_locked(&self) -> Result<(), MutationError> {
        let expected = self
            .root_lease
            .lock()
            .map_err(|_| MutationError::Invalid("Markdown root lease lock poisoned".into()))?
            .clone();
        let durable = load_active_root_lease(&self.data_root)?;
        if durable != expected || durable.root_scope != self.current_markdown_root_scope()? {
            return Err(MutationError::RecoveryConflict(
                "stale-markdown-root-lease".into(),
            ));
        }
        Ok(())
    }

    fn target_key(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
    ) -> Result<String, MutationError> {
        crate::services::twin_events::validate_target_key(kind, relative_key)?;
        Ok(match kind {
            crate::services::twin_events::TargetKind::Markdown => relative_key.to_string(),
            crate::services::twin_events::TargetKind::OverlayJson => {
                let root = self.target_root(kind)?;
                let relative_root = root.strip_prefix(&self.data_path).map_err(|_| {
                    MutationError::Invalid("overlay target root escaped app data".into())
                })?;
                format!(
                    "{}/{}",
                    relative_root.to_string_lossy().replace('\\', "/"),
                    relative_key
                )
            }
            crate::services::twin_events::TargetKind::TwinJson => {
                format!("twin/{relative_key}")
            }
            crate::services::twin_events::TargetKind::CanvasJson => {
                format!("canvas/{relative_key}")
            }
        })
    }

    fn before_image(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
    ) -> Result<crate::services::twin_events::BeforeImage, MutationError> {
        let limit = match kind {
            crate::services::twin_events::TargetKind::Markdown
            | crate::services::twin_events::TargetKind::OverlayJson
            | crate::services::twin_events::TargetKind::TwinJson => {
                crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES
            }
            crate::services::twin_events::TargetKind::CanvasJson => {
                crate::services::twin_events::MAX_CANVAS_BYTES
            }
        };
        let bytes = self.read_target(kind, relative_key, limit)?;
        match bytes {
            Some(bytes) => Ok(crate::services::twin_events::BeforeImage::Sha256(
                crate::services::twin_events::digest_bytes(&bytes),
            )),
            None => Ok(crate::services::twin_events::BeforeImage::Absent),
        }
    }

    fn read_target(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, MutationError> {
        match kind {
            crate::services::twin_events::TargetKind::Markdown => self
                .vault_root
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))?
                .read_bounded(relative_key, limit),
            crate::services::twin_events::TargetKind::OverlayJson
            | crate::services::twin_events::TargetKind::TwinJson
            | crate::services::twin_events::TargetKind::CanvasJson => self
                .data_root
                .read_bounded(&self.target_key(kind, relative_key)?, limit),
        }
    }

    fn put_target(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
        bytes: &[u8],
    ) -> Result<(), MutationError> {
        match kind {
            crate::services::twin_events::TargetKind::Markdown => self
                .vault_root
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))?
                .put_atomic(relative_key, bytes),
            crate::services::twin_events::TargetKind::OverlayJson
            | crate::services::twin_events::TargetKind::TwinJson
            | crate::services::twin_events::TargetKind::CanvasJson => self
                .data_root
                .put_atomic(&self.target_key(kind, relative_key)?, bytes),
        }
    }

    fn delete_target(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
    ) -> Result<(), MutationError> {
        match kind {
            crate::services::twin_events::TargetKind::Markdown => self
                .vault_root
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))?
                .delete(relative_key),
            crate::services::twin_events::TargetKind::OverlayJson
            | crate::services::twin_events::TargetKind::TwinJson
            | crate::services::twin_events::TargetKind::CanvasJson => {
                self.data_root.delete(&self.target_key(kind, relative_key)?)
            }
        }
    }

    fn physical_target_id(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
    ) -> Result<Vec<u8>, MutationError> {
        crate::services::twin_events::validate_target_key(kind, relative_key)?;
        let mut target = platform_canonical_path_bytes(&self.target_root(kind)?)?;
        target.push(b'/');
        #[cfg(windows)]
        target.extend_from_slice(relative_key.to_lowercase().as_bytes());
        #[cfg(not(windows))]
        target.extend_from_slice(relative_key.as_bytes());
        Ok(target)
    }

    fn inject(&self, point: MutationFaultPoint) -> Result<(), MutationError> {
        let mut configured = self
            .fault_once
            .lock()
            .map_err(|_| MutationError::Invalid("fault injector lock poisoned".into()))?;
        if configured.as_ref() == Some(&point) {
            *configured = None;
            return Err(MutationError::Io(format!(
                "injected mutation crash at {point:?}"
            )));
        }
        Ok(())
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
        if &durable != lease || lease.root_scope != markdown_root_scope_for(&canonical)? {
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

fn ensure_overlay_target_root(
    data_root: &crate::services::twin_events::AnchoredRoot,
    root_scope: &crate::models::twin_event::ContentDigest,
) -> Result<(), MutationError> {
    data_root.open_directory(
        &format!(
            "vault_derived/v1/{}/vault_migration/overlay/notes",
            root_scope.as_str()
        ),
        true,
    )?;
    Ok(())
}

fn markdown_root_scope_for(
    root: &Path,
) -> Result<crate::models::twin_event::ContentDigest, MutationError> {
    root_identity_for_path(root)
}

pub(crate) fn root_identity_for_path(
    root: &Path,
) -> Result<crate::models::twin_event::ContentDigest, MutationError> {
    crate::services::twin_events::validate_real_directory(root, "trusted Markdown vault root")?;
    let encoded = platform_canonical_path_bytes(root)?;
    let platform: &[u8] = if cfg!(windows) { b"windows" } else { b"unix" };
    let mut scoped = Vec::with_capacity(encoded.len() + 64);
    scoped.extend_from_slice(b"grafyn.root_identity.v1");
    scoped.extend_from_slice(&(platform.len() as u64).to_be_bytes());
    scoped.extend_from_slice(platform);
    scoped.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
    scoped.extend_from_slice(&encoded);
    Ok(crate::services::twin_events::digest_bytes(&scoped))
}

fn validate_disjoint_roots(
    markdown: &Path,
    canvas: &Path,
    twin: &Path,
) -> Result<(), MutationError> {
    let roots = [
        ("Markdown", platform_canonical_path_bytes(markdown)?),
        ("Canvas", platform_canonical_path_bytes(canvas)?),
        ("Twin", platform_canonical_path_bytes(twin)?),
    ];
    for left in 0..roots.len() {
        for right in left + 1..roots.len() {
            if physical_paths_overlap(&roots[left].1, &roots[right].1) {
                return Err(MutationError::Invalid(format!(
                    "{} and {} roots overlap",
                    roots[left].0, roots[right].0
                )));
            }
        }
    }
    Ok(())
}

fn physical_paths_overlap(left: &[u8], right: &[u8]) -> bool {
    fn is_ancestor(ancestor: &[u8], descendant: &[u8]) -> bool {
        descendant.starts_with(ancestor)
            && (descendant.len() == ancestor.len()
                || descendant
                    .get(ancestor.len())
                    .is_some_and(|separator| *separator == b'/' || *separator == b'\\'))
    }
    is_ancestor(left, right) || is_ancestor(right, left)
}

#[cfg(unix)]
fn platform_canonical_path_bytes(path: &Path) -> Result<Vec<u8>, MutationError> {
    use std::os::unix::ffi::OsStrExt;
    Ok(fs::canonicalize(path)?.as_os_str().as_bytes().to_vec())
}

#[cfg(windows)]
fn platform_canonical_path_bytes(path: &Path) -> Result<Vec<u8>, MutationError> {
    let canonical = fs::canonicalize(path)?;
    let encoded = canonical
        .to_str()
        .ok_or_else(|| MutationError::Invalid("canonical root path must be Unicode".into()))?;
    let normalized = encoded
        .strip_prefix(r"\\?\")
        .unwrap_or(encoded)
        .replace('/', "\\")
        .to_lowercase();
    Ok(normalized.into_bytes())
}

const ACTIVE_ROOT_LEASE_KEY: &str = "twin/events/active-markdown-root-v1.json";

fn load_or_create_active_root_lease(
    data_root: &crate::services::twin_events::AnchoredRoot,
    root_scope: crate::models::twin_event::ContentDigest,
) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
    if let Some(bytes) =
        data_root.read_bounded(ACTIVE_ROOT_LEASE_KEY, ACTIVE_ROOT_LEASE_LIMIT as usize)?
    {
        let lease = parse_active_root_lease(&bytes)?;
        if lease.root_scope != root_scope {
            return Err(MutationError::RecoveryConflict(
                "configured-markdown-root-is-not-active".into(),
            ));
        }
        return Ok(lease);
    }
    let lease = ActiveMarkdownRootLeaseV1 {
        schema_version: ACTIVE_ROOT_LEASE_SCHEMA_VERSION,
        root_scope,
        epoch_uuid: Uuid::new_v4().to_string(),
    };
    write_active_root_lease(data_root, &lease)?;
    Ok(lease)
}

fn load_active_root_lease(
    data_root: &crate::services::twin_events::AnchoredRoot,
) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
    let bytes = data_root
        .read_bounded(ACTIVE_ROOT_LEASE_KEY, ACTIVE_ROOT_LEASE_LIMIT as usize)?
        .ok_or_else(|| MutationError::Invalid("active Markdown root lease is missing".into()))?;
    parse_active_root_lease(&bytes)
}

fn parse_active_root_lease(bytes: &[u8]) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
    let lease: ActiveMarkdownRootLeaseV1 = serde_json::from_slice(bytes)
        .map_err(|error| MutationError::Invalid(format!("invalid root lease: {error}")))?;
    if lease.schema_version != ACTIVE_ROOT_LEASE_SCHEMA_VERSION
        || Uuid::parse_str(&lease.epoch_uuid).is_err()
    {
        return Err(MutationError::Invalid(
            "invalid active Markdown root lease".into(),
        ));
    }
    Ok(lease)
}

fn write_active_root_lease(
    data_root: &crate::services::twin_events::AnchoredRoot,
    lease: &ActiveMarkdownRootLeaseV1,
) -> Result<(), MutationError> {
    let mut bytes = serde_json::to_vec_pretty(lease)
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() > ACTIVE_ROOT_LEASE_LIMIT as usize {
        return Err(MutationError::Invalid(
            "active Markdown root lease exceeds its size limit".into(),
        ));
    }
    data_root.put_atomic(ACTIVE_ROOT_LEASE_KEY, &bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetClassification {
    Before,
    After,
    Third,
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

impl crate::services::twin_events::EventRecorder for MutationCoordinator {
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
                self.apply_nonlocal(origin, targets)?;
                Ok(MutationCommit {
                    mutation_id: None,
                    events: Vec::new(),
                    authority_token: None,
                })
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
mod tests {
    use super::*;
    use crate::models::twin_event::{
        CausalStream, Governance, NoteChangeKind, NoteChanged, TwinEventPayload, Visibility,
    };
    use chrono::{TimeZone, Utc};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn draft(label: &str) -> TwinEventDraft {
        TwinEventDraft {
            actor_id: None,
            causal_parents: Vec::new(),
            recorded_at: Utc.with_ymd_and_hms(2026, 8, 30, 1, 0, 0).unwrap(),
            observed_at: Utc.with_ymd_and_hms(2026, 8, 30, 1, 0, 0).unwrap(),
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

    #[test]
    fn writer_identity_is_stable_nonsecret_and_finalizer_chains_one_group() {
        let temp = tempdir().unwrap();
        let store = Arc::new(crate::services::twin_events::TwinEventStore::new(
            temp.path(),
        ));
        store.initialize().unwrap();
        let first_identity =
            PersistedMutationIdentityProvider::load_or_create(temp.path()).unwrap();
        let reopened = PersistedMutationIdentityProvider::load_or_create(temp.path()).unwrap();
        assert_eq!(first_identity.actor_id(), reopened.actor_id());
        assert_eq!(first_identity.device_id(), reopened.device_id());
        uuid::Uuid::parse_str(first_identity.device_id().as_str()).unwrap();
        let identity_json = std::fs::read_to_string(
            temp.path()
                .join("twin")
                .join("events")
                .join("writer-v1.json"),
        )
        .unwrap();
        assert!(!identity_json.to_ascii_lowercase().contains("secret"));
        assert!(!identity_json.to_ascii_lowercase().contains("key"));

        let finalizer = StoreEventGroupFinalizer::new(store.clone(), Arc::new(first_identity));
        let events = finalizer
            .finalize(
                CausalStream::SyncEligible,
                &[draft("note-a"), draft("note-b")],
            )
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].device_sequence, 1);
        assert_eq!(events[1].device_sequence, 2);
        assert!(events[1].causal_parents.contains(&events[0].event_id));
        assert_eq!(events[0].causal_stream, CausalStream::SyncEligible);

        for event in events {
            store.append(event).unwrap();
        }
        let next = finalizer
            .finalize(CausalStream::SyncEligible, &[draft("note-c")])
            .unwrap();
        assert_eq!(next[0].device_sequence, 3);
    }

    #[test]
    fn concurrent_writer_identity_installers_converge_without_staging_litter() {
        let temp = tempdir().unwrap();
        let store = crate::services::twin_events::TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let barrier = barrier.clone();
            let data_path = temp.path().to_path_buf();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                PersistedMutationIdentityProvider::load_or_create(data_path).unwrap()
            }));
        }
        barrier.wait();
        let first = threads.remove(0).join().unwrap();
        let second = threads.remove(0).join().unwrap();
        assert_eq!(first.actor_id(), second.actor_id());
        assert_eq!(first.device_id(), second.device_id());
        assert_eq!(
            std::fs::read_dir(temp.path().join("twin/events/staging/v1"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn restrictive_member_downgrades_the_entire_group_to_local_only() {
        let temp = tempdir().unwrap();
        let store = Arc::new(crate::services::twin_events::TwinEventStore::new(
            temp.path(),
        ));
        store.initialize().unwrap();
        let identity =
            Arc::new(PersistedMutationIdentityProvider::load_or_create(temp.path()).unwrap());
        let finalizer = StoreEventGroupFinalizer::new(store, identity);
        let mut private = draft("private");
        private.governance.visibility = Visibility::LocalOnly;
        private.governance.allowed_uses.sync = false;
        let events = finalizer
            .finalize(CausalStream::SyncEligible, &[draft("shared"), private])
            .unwrap();
        assert!(events
            .iter()
            .all(|event| event.causal_stream == CausalStream::LocalOnly));
    }

    #[test]
    fn coordinator_rejects_overlapping_markdown_canvas_or_twin_roots() {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let store = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        store.initialize().unwrap();

        let twin_overlap = MutationCoordinator::new(
            &data,
            data.join("twin"),
            store.clone(),
            Arc::new(NoopMutationLifecycle),
        );
        let twin_error = twin_overlap.err().expect("Twin overlap must fail");
        assert!(
            twin_error.to_string().contains("overlap"),
            "unexpected construction failure: {twin_error}"
        );
        let canvas_overlap = MutationCoordinator::new(
            &data,
            data.join("canvas"),
            store,
            Arc::new(NoopMutationLifecycle),
        );
        let canvas_error = canvas_overlap.err().expect("Canvas overlap must fail");
        assert!(
            canvas_error.to_string().contains("overlap"),
            "unexpected construction failure: {canvas_error}"
        );
    }

    #[test]
    fn rejected_overlapping_retarget_keeps_old_root_and_lease_authoritative() {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        store.initialize().unwrap();
        let coordinator =
            MutationCoordinator::new(&data, &vault, store, Arc::new(NoopMutationLifecycle))
                .unwrap();

        assert!(coordinator
            .retarget_markdown_root(&data.join("canvas"))
            .is_err());
        coordinator
            .commit_local(
                CausalStream::SyncEligible,
                crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "still-old.md",
                    "old root",
                )],
                vec![draft("still-old")],
            )
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(vault.join("still-old.md")).unwrap(),
            "old root"
        );
        assert!(!data.join("canvas").join("still-old.md").exists());
    }

    #[test]
    fn coordinated_event_only_group_appends_distinct_primitive_assessments() {
        use crate::models::twin_event::{
            BoundedContent, DecisionRecorded, Identifier, PrimitiveDecisionAssessmentPayload,
        };
        let temp = tempdir().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(crate::services::twin_events::TwinEventStore::new(
            temp.path(),
        ));
        store.initialize().unwrap();
        let coordinator = MutationCoordinator::new(
            temp.path(),
            &vault,
            store.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        let payload = |field: &str| {
            let mut assessment = PrimitiveDecisionAssessmentPayload::default();
            let value = Some(BoundedContent::parse(format!("{field} value")).unwrap());
            match field {
                "stakes" => assessment.stakes = value,
                "reversibility" => assessment.reversibility = value,
                _ => unreachable!(),
            }
            crate::models::twin_event::TwinEventPayload::DecisionRecorded(DecisionRecorded {
                decision_id: Identifier::parse("primitive-decision").unwrap(),
                decision: BoundedContent::parse("choose").unwrap(),
                options: vec![BoundedContent::parse("a").unwrap()],
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: assessment,
            })
        };
        let mut first = draft("placeholder-a");
        first.payload = payload("stakes");
        let mut second = draft("placeholder-b");
        second.payload = payload("reversibility");

        let committed = coordinator
            .commit_local(
                CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("legacy_twin").unwrap(),
                Vec::new(),
                vec![first, second],
            )
            .unwrap();
        assert_eq!(committed.events.len(), 2);
        assert_ne!(committed.events[0].event_id, committed.events[1].event_id);
        assert_eq!(store.ordered_events().unwrap().len(), 2);
    }

    #[test]
    fn authority_mutations_invalidate_ready_before_journal_stage_but_canvas_layout_does_not() {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(&data));
        store.initialize().unwrap();
        let coordinator = MutationCoordinator::new(
            &data,
            &vault,
            store,
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        coordinator.require_namespace_ready().unwrap();
        let before = coordinator.current_authority_token().unwrap();

        coordinator
            .apply_nonlocal(
                MutationOrigin::Recovery,
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::CanvasJson,
                    "layout.json",
                    "{}",
                )],
            )
            .unwrap();
        coordinator.require_namespace_ready().unwrap();

        coordinator.fail_once_at(MutationFaultPoint::AfterStage);
        assert!(coordinator
            .commit_local(
                CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "authority.md",
                    "changed",
                )],
                vec![draft("authority")],
            )
            .is_err());
        assert!(coordinator.require_namespace_ready().is_err());
        assert!(!vault.join("authority.md").exists());
        let staged = coordinator.current_authority_token().unwrap();
        assert_eq!(
            staged.authority_generation,
            before.authority_generation + 1
        );

        coordinator.recover_pending().unwrap();
        assert_eq!(
            std::fs::read_to_string(vault.join("authority.md")).unwrap(),
            "changed"
        );
        assert!(coordinator.require_namespace_ready().is_err());
        assert_eq!(coordinator.current_authority_token().unwrap(), staged);
    }

    #[test]
    fn commit_returns_the_exact_post_mutation_authority_token() {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(&data));
        store.initialize().unwrap();
        let coordinator = MutationCoordinator::new(
            &data,
            &vault,
            store,
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();

        let canvas = coordinator
            .commit_local(
                CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("canvas").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::CanvasJson,
                    "layout.json",
                    "{}",
                )],
                Vec::new(),
            )
            .unwrap();
        assert!(canvas.authority_token.is_none());

        let authority = coordinator
            .commit_local(
                CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "token.md",
                    "changed",
                )],
                Vec::new(),
            )
            .unwrap();
        assert_eq!(
            authority.authority_token.as_ref(),
            Some(&coordinator.current_authority_token().unwrap())
        );
    }

    #[test]
    fn planned_mutation_rejects_a_stale_expected_authority_before_staging() {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(&data));
        store.initialize().unwrap();
        let coordinator = MutationCoordinator::new(
            &data,
            &vault,
            store,
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        let stale = coordinator.current_authority_token().unwrap();
        coordinator
            .commit_local(
                CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "peer.md",
                    "peer",
                )],
                Vec::new(),
            )
            .unwrap();

        let mut plan = Some(
            MutationPlan::new(
                CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("canvas").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::CanvasJson,
                    "stale.json",
                    "{}",
                )],
                Vec::new(),
            )
            .expecting_authority(stale),
        );
        let error = coordinator
            .commit_planned(MutationOrigin::Local, &mut || Ok(plan.take()))
            .unwrap_err();
        assert!(error.to_string().contains("authority changed"));
        assert!(!data.join("canvas/stale.json").exists());
        assert_eq!(
            std::fs::read_dir(data.join("twin/mutations/pending/v1"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn overlay_target_recovery_uses_the_staged_generation_once() {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(&data));
        store.initialize().unwrap();
        let coordinator = MutationCoordinator::new(
            &data,
            &vault,
            store.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        let before = coordinator.current_authority_token().unwrap();
        coordinator.fail_once_at(MutationFaultPoint::AfterStage);
        assert!(coordinator
            .commit_local(
                CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("vault_optimizer").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::OverlayJson,
                    "captured.json",
                    "{}",
                )],
                Vec::new(),
            )
            .is_err());
        let staged = coordinator.current_authority_token().unwrap();
        assert_eq!(staged.authority_generation, before.authority_generation + 1);
        let overlay = crate::services::vault_namespace::scoped_data_path(
            &data,
            &staged.root_scope,
        )
        .join("vault_migration/overlay/notes/captured.json");
        assert!(!overlay.exists());
        let pending_path = std::fs::read_dir(data.join("twin/mutations/pending/v1"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let pending: crate::services::twin_events::MutationIntentV1 =
            serde_json::from_slice(&std::fs::read(pending_path).unwrap()).unwrap();
        assert_eq!(pending.markdown_root_scope.as_ref(), Some(&staged.root_scope));
        assert!(pending.targets.iter().all(|target| {
            target.kind == crate::services::twin_events::TargetKind::OverlayJson
        }));

        assert_eq!(coordinator.recover_pending().unwrap(), 1);
        assert_eq!(std::fs::read_to_string(&overlay).unwrap(), "{}");
        assert_eq!(coordinator.current_authority_token().unwrap(), staged);
        assert_eq!(coordinator.recover_pending().unwrap(), 0);
        assert_eq!(coordinator.current_authority_token().unwrap(), staged);

        drop(coordinator);
        let restarted = MutationCoordinator::new(
            &data,
            &vault,
            store,
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        assert_eq!(restarted.recover_pending().unwrap(), 0);
        assert_eq!(restarted.current_authority_token().unwrap(), staged);
        assert_eq!(std::fs::read_to_string(overlay).unwrap(), "{}");
    }

    #[test]
    fn crash_after_authority_advance_before_wal_is_false_dirty_and_restart_has_zero_work() {
        let temp = tempdir().unwrap();
        let data = temp.path().join("data");
        let vault = temp.path().join("vault");
        std::fs::create_dir(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let store = Arc::new(TwinEventStore::new(&data));
        store.initialize().unwrap();
        let coordinator = MutationCoordinator::new(
            &data,
            &vault,
            store.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        let before = coordinator.current_authority_token().unwrap();

        coordinator.fail_once_at(MutationFaultPoint::AfterAuthorityAdvance);
        assert!(coordinator
            .commit_local(
                CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("note_editor").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "false-dirty.md",
                    "not staged",
                )],
                vec![draft("false-dirty")],
            )
            .is_err());
        let dirty = coordinator.current_authority_token().unwrap();
        assert_eq!(dirty.authority_generation, before.authority_generation + 1);
        assert!(coordinator.require_namespace_ready().is_err());
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert!(!vault.join("false-dirty.md").exists());
        assert_eq!(coordinator.recover_pending().unwrap(), 0);

        drop(coordinator);
        let restarted = MutationCoordinator::new(
            &data,
            &vault,
            store,
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap();
        assert_eq!(restarted.current_authority_token().unwrap(), dirty);
        assert_eq!(restarted.recover_pending().unwrap(), 0);
        assert!(!vault.join("false-dirty.md").exists());
    }

    #[test]
    fn production_desktop_and_mcp_use_coordinated_non_noop_construction() {
        let desktop = include_str!("../../lib.rs");
        let mcp = include_str!("../../mcp.rs");
        assert!(desktop.contains("KnowledgeStore::with_event_recorder"));
        assert!(desktop.contains("CanvasStore::with_event_recorder"));
        assert!(desktop.contains("recover_pending()"));
        assert!(desktop.contains("UnavailableEventRecorder"));
        assert!(mcp.contains("KnowledgeStore::with_event_recorder"));
        let recover = mcp.find("recover_pending()").unwrap();
        let serve = mcp.find(".serve(rmcp::transport::stdio())").unwrap();
        assert!(recover < serve);
        assert!(!mcp.contains("KnowledgeStore::new(vault_path"));
    }
}
