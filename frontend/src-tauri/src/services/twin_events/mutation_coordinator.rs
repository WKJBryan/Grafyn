use crate::models::twin_event::{
    ActorId, CausalStream, DeviceId, EventContext, EventId, EvidenceRef, Governance, Sensitivity,
    TwinEvent, TwinEventPayload, Visibility,
};
use crate::services::twin_events::{derive_event_id, StoreError, TwinEventStore};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

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
        let events_dir = data_path.as_ref().join("twin").join("events");
        crate::services::twin_events::validate_real_directory(
            &events_dir,
            "Twin events directory for writer identity",
        )?;
        let path = events_dir.join("writer-v1.json");
        if path.exists() {
            return Self::load(&path);
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
        let temporary = events_dir.join(format!(".writer-v1.{}.tmp", Uuid::new_v4()));
        let result = (|| -> Result<(), MutationError> {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            match fs::hard_link(&temporary, &path) {
                Ok(()) => crate::services::twin_events::sync_directory(&events_dir)?,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            Ok(())
        })();
        let cleanup = fs::remove_file(&temporary);
        if let Err(error) = cleanup {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error.into());
            }
        }
        crate::services::twin_events::sync_directory(&events_dir)?;
        result?;
        Self::load(&path)
    }

    fn load(path: &Path) -> Result<Self, MutationError> {
        crate::services::twin_events::validate_real_file(path, "Twin writer identity")?;
        let metadata = fs::symlink_metadata(path)?;
        if metadata.len() > WRITER_FILE_LIMIT {
            return Err(MutationError::Invalid(
                "Twin writer identity exceeds its size limit".into(),
            ));
        }
        let identity: WriterIdentityV1 = serde_json::from_slice(&fs::read(path)?)
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

    fn coordinator_lock_path(&self) -> PathBuf {
        self.store
            .data_path()
            .join("twin")
            .join("events")
            .join("mutation-v1.lock")
    }

    pub(crate) fn acquire_coordinator_lock(&self) -> Result<File, MutationError> {
        let path = self.coordinator_lock_path();
        let parent = path
            .parent()
            .ok_or_else(|| MutationError::Invalid("coordinator lock has no parent".into()))?;
        crate::services::twin_events::validate_real_directory(
            parent,
            "Twin mutation coordinator directory",
        )?;
        let file = match OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => {
                crate::services::twin_events::sync_directory(parent)?;
                file
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                crate::services::twin_events::validate_real_file(
                    &path,
                    "Twin mutation coordinator lock",
                )?;
                OpenOptions::new().read(true).write(true).open(&path)?
            }
            Err(error) => return Err(error.into()),
        };
        file.lock_exclusive()?;
        Ok(file)
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

impl EventGroupFinalizer for StoreEventGroupFinalizer {
    fn finalize(
        &self,
        requested_stream: CausalStream,
        drafts: &[TwinEventDraft],
    ) -> Result<Vec<TwinEvent>, MutationError> {
        let lock = self.acquire_coordinator_lock()?;
        let result = self.finalize_locked(requested_stream, drafts);
        FileExt::unlock(&lock)?;
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
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationFaultPoint {
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
}

pub struct MutationCoordinator {
    data_path: PathBuf,
    vault_path: std::sync::Mutex<PathBuf>,
    store: Arc<TwinEventStore>,
    finalizer: StoreEventGroupFinalizer,
    journal: crate::services::twin_events::LocalMutationJournal,
    lifecycle: Arc<dyn MutationLifecycle>,
    in_process: std::sync::Mutex<()>,
    fault_once: std::sync::Mutex<Option<MutationFaultPoint>>,
    root_lease: std::sync::Mutex<ActiveMarkdownRootLeaseV1>,
}

const ACTIVE_ROOT_LEASE_SCHEMA_VERSION: u16 = 1;
const ACTIVE_ROOT_LEASE_LIMIT: u64 = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActiveMarkdownRootLeaseV1 {
    schema_version: u16,
    root_scope: crate::models::twin_event::ContentDigest,
    epoch_uuid: String,
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
        let vault_path = fs::canonicalize(vault_path)?;
        let identity = Arc::new(PersistedMutationIdentityProvider::load_or_create(
            &data_path,
        )?);
        let finalizer = StoreEventGroupFinalizer::new(store.clone(), identity);
        let journal = crate::services::twin_events::LocalMutationJournal::initialize(&data_path)?;
        let canvas_path = data_path.join("canvas");
        match fs::symlink_metadata(&canvas_path) {
            Ok(_) => crate::services::twin_events::validate_real_directory(
                &canvas_path,
                "Canvas data directory",
            )?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&canvas_path)?;
                crate::services::twin_events::sync_directory(&data_path)?;
            }
            Err(error) => return Err(error.into()),
        }
        let process_lock = finalizer.acquire_coordinator_lock()?;
        let root_scope = markdown_root_scope_for(&vault_path)?;
        let root_lease = load_or_create_active_root_lease(&data_path, root_scope)?;
        FileExt::unlock(&process_lock)?;
        Ok(Self {
            data_path,
            vault_path: std::sync::Mutex::new(vault_path),
            store,
            finalizer,
            journal,
            lifecycle,
            in_process: std::sync::Mutex::new(()),
            fault_once: std::sync::Mutex::new(None),
            root_lease: std::sync::Mutex::new(root_lease),
        })
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
            FileExt::unlock(&process_lock)?;
            return Err(error);
        }
        for (_, pending) in self.journal.load_pending()? {
            if let Err(error) = self.replay_intent_locked(&pending, false) {
                FileExt::unlock(&process_lock)?;
                return Err(error);
            }
        }
        let Some(plan) = (match planner() {
            Ok(plan) => plan,
            Err(error) => {
                FileExt::unlock(&process_lock)?;
                if origin == MutationOrigin::Local {
                    self.lifecycle.known_failure(None, &error.to_string());
                }
                return Err(error);
            }
        }) else {
            FileExt::unlock(&process_lock)?;
            return Ok(MutationCommit {
                mutation_id: None,
                events: Vec::new(),
            });
        };
        let prepared = match self.prepare_intent(
            origin,
            plan.requested_stream,
            plan.source_channel,
            plan.targets,
            plan.drafts,
        ) {
            Ok(Some(intent)) => intent,
            Ok(None) => {
                FileExt::unlock(&process_lock)?;
                return Ok(MutationCommit {
                    mutation_id: None,
                    events: Vec::new(),
                });
            }
            Err(error) => {
                FileExt::unlock(&process_lock)?;
                if origin == MutationOrigin::Local {
                    self.lifecycle.known_failure(None, &error.to_string());
                }
                return Err(error);
            }
        };

        let invoke_lifecycle = origin == MutationOrigin::Local && !prepared.events.is_empty();
        if invoke_lifecycle {
            if let Err(error) = self.lifecycle.stage_before_local(&prepared) {
                FileExt::unlock(&process_lock)?;
                self.lifecycle
                    .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
                return Err(error);
            }
        }
        if let Err(error) = self.journal.stage(&prepared) {
            FileExt::unlock(&process_lock)?;
            if invoke_lifecycle {
                self.lifecycle
                    .known_failure(Some(prepared.mutation_id.as_str()), &error.to_string());
            }
            return Err(error);
        }
        let committed = self
            .inject(MutationFaultPoint::AfterStage)
            .and_then(|()| self.replay_intent_locked(&prepared, true));
        let unlock = FileExt::unlock(&process_lock);
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
        let pending = self.journal.load_pending()?;
        let mut recovered = 0usize;
        for (_, intent) in pending {
            if let Err(error) = self.replay_intent_locked(&intent, false) {
                FileExt::unlock(&process_lock)?;
                return Err(error);
            }
            recovered += 1;
        }
        FileExt::unlock(&process_lock)?;
        Ok(recovered)
    }

    pub fn pending_count(&self) -> Result<usize, MutationError> {
        self.journal.pending_count()
    }

    pub fn retarget_markdown_root(&self, vault_path: &Path) -> Result<(), MutationError> {
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
            for (_, pending) in self.journal.load_pending()? {
                self.replay_intent_locked(&pending, false)?;
            }
            let next_lease = ActiveMarkdownRootLeaseV1 {
                schema_version: ACTIVE_ROOT_LEASE_SCHEMA_VERSION,
                root_scope: markdown_root_scope_for(&new_root)?,
                epoch_uuid: Uuid::new_v4().to_string(),
            };
            write_active_root_lease(&self.data_path, &next_lease)?;
            *self
                .vault_path
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))? =
                new_root;
            *self.root_lease.lock().map_err(|_| {
                MutationError::Invalid("Markdown root lease lock poisoned".into())
            })? = next_lease;
            Ok(())
        })();
        FileExt::unlock(&process_lock)?;
        result
    }

    pub fn quarantine_count(&self) -> Result<usize, MutationError> {
        self.journal.quarantine_count()
    }

    fn prepare_intent(
        &self,
        origin: MutationOrigin,
        requested_stream: CausalStream,
        source_channel: crate::models::twin_event::SourceChannel,
        targets: Vec<crate::services::twin_events::TargetMutation>,
        drafts: Vec<TwinEventDraft>,
    ) -> Result<Option<crate::services::twin_events::MutationIntentV1>, MutationError> {
        if targets.is_empty() || targets.len() > crate::services::twin_events::MAX_INTENT_TARGETS {
            return Err(MutationError::Invalid(
                "local mutation must contain 1..=64 targets".into(),
            ));
        }
        let mut targets = targets;
        targets.sort_by(|left, right| {
            (left.kind, left.relative_key.as_str()).cmp(&(right.kind, right.relative_key.as_str()))
        });
        let mut physical_keys = std::collections::BTreeSet::new();
        for target in &targets {
            let key = crate::services::twin_events::physical_target_key(
                target.kind,
                &target.relative_key,
            )?;
            if !physical_keys.insert(key) {
                return Err(MutationError::Invalid(
                    "duplicate or aliased mutation target".into(),
                ));
            }
        }
        let mut prepared_targets = Vec::new();
        for target in targets {
            crate::services::twin_events::validate_relative_key(&target.relative_key)?;
            let path = self.resolve_target_path(target.kind, &target.relative_key, false)?;
            let before = self.before_image(&path)?;
            let after_digest = crate::services::twin_events::desired_digest(&target.after);
            if target_matches_after(&before, &target.after, &after_digest) {
                continue;
            }
            prepared_targets.push(crate::services::twin_events::MutationTargetV1 {
                kind: target.kind,
                relative_key: target.relative_key,
                before,
                after: target.after,
                after_digest,
            });
        }
        if prepared_targets.is_empty() {
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
            .any(|target| target.kind == crate::services::twin_events::TargetKind::Markdown)
        {
            Some(self.current_markdown_root_scope()?)
        } else {
            None
        };
        let mut intent = crate::services::twin_events::MutationIntentV1 {
            schema_version: 1,
            mutation_id: crate::services::twin_events::digest_bytes(b"placeholder"),
            origin,
            actor_id: self.finalizer.actor_id(),
            device_id: self.finalizer.device_id(),
            causal_stream: stream,
            source_channel,
            markdown_root_scope,
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
        intent: &crate::services::twin_events::MutationIntentV1,
        inject_faults: bool,
    ) -> Result<(), MutationError> {
        intent.validate()?;
        self.store.preflight_append_group(&intent.events)?;
        if let Some(intent_scope) = &intent.markdown_root_scope {
            if &self.current_markdown_root_scope()? != intent_scope {
                self.journal.quarantine_intent(intent)?;
                return Err(MutationError::RecoveryConflict(
                    intent.mutation_id.as_str().to_string(),
                ));
            }
        }
        let mut classifications = Vec::with_capacity(intent.targets.len());
        for target in &intent.targets {
            let path = self.resolve_target_path(target.kind, &target.relative_key, false)?;
            let current = self.before_image(&path)?;
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
            self.journal.quarantine_intent(intent)?;
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
        self.journal.remove(intent)?;
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
        })
    }

    fn apply_target_mutation(
        &self,
        target: &crate::services::twin_events::TargetMutation,
    ) -> Result<(), MutationError> {
        crate::services::twin_events::validate_relative_key(&target.relative_key)?;
        let create_parents = matches!(
            target.after,
            crate::services::twin_events::DesiredImage::Utf8Bytes(_)
        );
        let path = self.resolve_target_path(target.kind, &target.relative_key, create_parents)?;
        match &target.after {
            crate::services::twin_events::DesiredImage::Utf8Bytes(content) => {
                if path.exists() {
                    crate::services::twin_events::validate_real_file(
                        &path,
                        "local mutation target",
                    )?;
                }
                crate::services::atomic_io::write_atomic(&path, content.as_bytes())?;
                crate::services::twin_events::sync_directory(
                    path.parent().expect("target path has parent"),
                )?;
            }
            crate::services::twin_events::DesiredImage::Tombstone => match fs::remove_file(&path) {
                Ok(()) => crate::services::twin_events::sync_directory(
                    path.parent().expect("target path has parent"),
                )?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            },
        }
        Ok(())
    }

    fn target_root(
        &self,
        kind: crate::services::twin_events::TargetKind,
    ) -> Result<PathBuf, MutationError> {
        Ok(match kind {
            crate::services::twin_events::TargetKind::Markdown => self
                .vault_path
                .lock()
                .map_err(|_| MutationError::Invalid("Markdown root lock poisoned".into()))?
                .clone(),
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
        let durable = load_active_root_lease(&self.data_path)?;
        if durable != expected || durable.root_scope != self.current_markdown_root_scope()? {
            return Err(MutationError::RecoveryConflict(
                "stale-markdown-root-lease".into(),
            ));
        }
        Ok(())
    }

    fn resolve_target_path(
        &self,
        kind: crate::services::twin_events::TargetKind,
        relative_key: &str,
        create_parents: bool,
    ) -> Result<PathBuf, MutationError> {
        crate::services::twin_events::validate_relative_key(relative_key)?;
        let root = self.target_root(kind)?;
        crate::services::twin_events::validate_real_directory(&root, "mutation target root")?;
        let components = Path::new(relative_key)
            .components()
            .map(|component| match component {
                std::path::Component::Normal(value) => value
                    .to_str()
                    .map(str::to_string)
                    .ok_or_else(|| MutationError::Invalid("target key is not UTF-8".into())),
                _ => Err(MutationError::Invalid("unsafe target key".into())),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut parent = root;
        for (index, component) in components[..components.len() - 1].iter().enumerate() {
            let candidate = parent.join(component);
            match fs::symlink_metadata(&candidate) {
                Ok(_) => crate::services::twin_events::validate_real_directory(
                    &candidate,
                    "mutation target path component",
                )?,
                Err(error) if error.kind() == io::ErrorKind::NotFound && create_parents => {
                    parent = crate::services::twin_events::ensure_real_child_directory(
                        &parent, component, false,
                    )?;
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    for remaining in &components[index..] {
                        parent.push(remaining);
                    }
                    return Ok(parent);
                }
                Err(error) => return Err(error.into()),
            }
            parent = candidate;
        }
        let path = parent.join(components.last().expect("validated key is nonempty"));
        if path.exists() {
            crate::services::twin_events::validate_real_file(&path, "mutation target")?;
        }
        Ok(path)
    }

    fn before_image(
        &self,
        path: &Path,
    ) -> Result<crate::services::twin_events::BeforeImage, MutationError> {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                crate::services::twin_events::validate_real_file(path, "mutation target")?;
                let mut file = File::open(path)?;
                let mut hasher = sha2::Sha256::new();
                let mut buffer = [0u8; 64 * 1024];
                loop {
                    let count = std::io::Read::read(&mut file, &mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    use sha2::Digest as _;
                    hasher.update(&buffer[..count]);
                }
                use sha2::Digest as _;
                Ok(crate::services::twin_events::BeforeImage::Sha256(
                    crate::models::twin_event::ContentDigest::parse(format!(
                        "{:x}",
                        hasher.finalize()
                    ))
                    .map_err(MutationError::Invalid)?,
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(crate::services::twin_events::BeforeImage::Absent)
            }
            Err(error) => Err(error.into()),
        }
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

fn markdown_root_scope_for(
    root: &Path,
) -> Result<crate::models::twin_event::ContentDigest, MutationError> {
    crate::services::twin_events::validate_real_directory(root, "trusted Markdown vault root")?;
    let canonical = fs::canonicalize(root)?;
    let encoded = canonical
        .to_str()
        .ok_or_else(|| MutationError::Invalid("Markdown vault root must be valid UTF-8".into()))?;
    let mut scoped = Vec::with_capacity(encoded.len() + 48);
    scoped.extend_from_slice(b"grafyn.markdown_root_scope.v1");
    scoped.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
    scoped.extend_from_slice(encoded.as_bytes());
    Ok(crate::services::twin_events::digest_bytes(&scoped))
}

fn active_root_lease_path(data_path: &Path) -> PathBuf {
    data_path
        .join("twin")
        .join("events")
        .join("active-markdown-root-v1.json")
}

fn load_or_create_active_root_lease(
    data_path: &Path,
    root_scope: crate::models::twin_event::ContentDigest,
) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
    let path = active_root_lease_path(data_path);
    if path.exists() {
        let lease = load_active_root_lease(data_path)?;
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
    write_active_root_lease(data_path, &lease)?;
    Ok(lease)
}

fn load_active_root_lease(data_path: &Path) -> Result<ActiveMarkdownRootLeaseV1, MutationError> {
    let path = active_root_lease_path(data_path);
    crate::services::twin_events::validate_real_file(&path, "active Markdown root lease")?;
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.len() > ACTIVE_ROOT_LEASE_LIMIT {
        return Err(MutationError::Invalid(
            "active Markdown root lease exceeds its size limit".into(),
        ));
    }
    let lease: ActiveMarkdownRootLeaseV1 = serde_json::from_slice(&fs::read(path)?)
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
    data_path: &Path,
    lease: &ActiveMarkdownRootLeaseV1,
) -> Result<(), MutationError> {
    let path = active_root_lease_path(data_path);
    let parent = path
        .parent()
        .ok_or_else(|| MutationError::Invalid("active root lease has no parent".into()))?;
    crate::services::twin_events::validate_real_directory(parent, "Twin events directory")?;
    if path.exists() {
        crate::services::twin_events::validate_real_file(&path, "active Markdown root lease")?;
    }
    let mut bytes = serde_json::to_vec_pretty(lease)
        .map_err(|error| MutationError::Invalid(error.to_string()))?;
    bytes.push(b'\n');
    crate::services::atomic_io::write_atomic(&path, &bytes)?;
    crate::services::twin_events::sync_directory(parent)?;
    Ok(())
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
    fn record(
        &self,
        event: TwinEvent,
    ) -> Result<crate::services::twin_events::AppendOutcome, StoreError> {
        self.store.append(event)
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
                self.apply_nonlocal(origin, targets)?;
                Ok(MutationCommit {
                    mutation_id: None,
                    events: Vec::new(),
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
