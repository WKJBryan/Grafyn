use super::{derive_event_id, semantic_bytes};
use crate::models::twin_event::{
    CausalStream, DeviceId, EventId, EvidenceRef, EvidenceType, TwinEvent,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;
#[cfg(test)]
use walkdir::WalkDir;

pub const MAX_TWIN_EVENT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    Appended,
    Duplicate,
    Ignored,
}

#[derive(Debug)]
pub enum StoreError {
    NotInitialized,
    Io(String),
    Invalid(String),
    Collision(EventId),
    MissingParent(EventId),
    CausalCycle,
    WrongEventId {
        supplied: EventId,
        expected: EventId,
    },
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInitialized => f.write_str("Twin event store is not initialized"),
            Self::Io(message) | Self::Invalid(message) => f.write_str(message),
            Self::Collision(id) => write!(f, "Twin event ID collision: {id}"),
            Self::MissingParent(id) => write!(f, "missing causal parent: {id}"),
            Self::CausalCycle => f.write_str("causal event cycle detected"),
            Self::WrongEventId { supplied, expected } => write!(
                f,
                "event ID {supplied} does not match canonical ID {expected}"
            ),
        }
    }
}

impl std::error::Error for StoreError {}
impl From<io::Error> for StoreError {
    fn from(value: io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

#[derive(Default)]
struct StoreState {
    initialized: bool,
    events: BTreeMap<EventId, TwinEvent>,
}

pub struct TwinEventStore {
    data_path: PathBuf,
    state: Mutex<StoreState>,
    root: Mutex<Option<super::AnchoredRoot>>,
    #[cfg(test)]
    after_enumeration_hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl TwinEventStore {
    pub fn new(data_path: impl AsRef<Path>) -> Self {
        Self {
            data_path: data_path.as_ref().to_path_buf(),
            state: Mutex::new(StoreState::default()),
            root: Mutex::new(None),
            #[cfg(test)]
            after_enumeration_hook: Mutex::new(None),
        }
    }

    pub fn events_dir(&self) -> PathBuf {
        self.data_path.join("twin").join("events").join("v1")
    }
    pub fn quarantine_dir(&self) -> PathBuf {
        self.data_path
            .join("twin")
            .join("events")
            .join("quarantine")
            .join("v1")
    }
    pub(crate) fn data_path(&self) -> &Path {
        &self.data_path
    }
    #[cfg(test)]
    fn set_after_enumeration_hook(&self, hook: impl FnOnce() + Send + 'static) {
        *self.after_enumeration_hook.lock().unwrap() = Some(Box::new(hook));
    }
    #[cfg(test)]
    fn lock_path(&self) -> PathBuf {
        self.data_path
            .join("twin")
            .join("events")
            .join("append-v1.lock")
    }

    #[cfg(test)]
    fn lock_durability_directory(&self) -> Result<PathBuf, StoreError> {
        self.lock_path()
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| StoreError::Invalid("Twin event lock has no parent directory".into()))
    }

    pub fn initialize(&self) -> Result<(), StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event store lock poisoned".into()))?;
        let root = self.ensure_store_layout()?;
        let lock = self.acquire_process_lock(&root)?;
        let events = self.load_records(&root)?;
        lock.unlock()?;
        *self
            .root
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event capability lock poisoned".into()))? =
            Some(root);
        state.events = events;
        state.initialized = true;
        Ok(())
    }

    pub(crate) fn append(&self, mut event: TwinEvent) -> Result<AppendOutcome, StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event store lock poisoned".into()))?;
        if !state.initialized {
            return Err(StoreError::NotInitialized);
        }
        let root = self.root_capability()?;
        let lock = self.acquire_process_lock(&root)?;
        state.events = self.load_records(&root)?;
        let outcome = validate_candidate_append(&state.events, &mut event);
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                lock.unlock()?;
                return Err(error);
            }
        };
        if outcome == AppendOutcome::Duplicate {
            lock.unlock()?;
            return Ok(outcome);
        }
        let bytes = serialize_record(&event)?;
        self.install_no_clobber(&root, &event, &bytes)?;
        state.events.insert(event.event_id.clone(), event);
        lock.unlock()?;
        Ok(outcome)
    }

    pub fn preflight_append_group(&self, group: &[TwinEvent]) -> Result<(), StoreError> {
        if group.is_empty() {
            return Ok(());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event store lock poisoned".into()))?;
        if !state.initialized {
            return Err(StoreError::NotInitialized);
        }
        let root = self.root_capability()?;
        let lock = self.acquire_process_lock(&root)?;
        state.events = self.load_records(&root)?;
        let result = (|| {
            let mut simulated = state.events.clone();
            for candidate in group {
                let mut candidate = candidate.clone();
                if validate_candidate_append(&simulated, &mut candidate)? == AppendOutcome::Appended
                {
                    simulated.insert(candidate.event_id.clone(), candidate);
                }
            }
            Ok(())
        })();
        lock.unlock()?;
        result
    }

    pub fn ordered_events(&self) -> Result<Vec<TwinEvent>, StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event store lock poisoned".into()))?;
        if !state.initialized {
            return Err(StoreError::NotInitialized);
        }
        let root = self.root_capability()?;
        let lock = self.acquire_process_lock(&root)?;
        state.events = self.load_records(&root)?;
        lock.unlock()?;
        topological_order(&state.events.values().cloned().collect::<Vec<_>>())
    }

    pub fn ordered_events_for_stream(
        &self,
        causal_stream: CausalStream,
    ) -> Result<Vec<TwinEvent>, StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event store lock poisoned".into()))?;
        if !state.initialized {
            return Err(StoreError::NotInitialized);
        }
        let root = self.root_capability()?;
        let lock = self.acquire_process_lock(&root)?;
        state.events = self.load_records(&root)?;
        lock.unlock()?;
        topological_order(
            &state
                .events
                .values()
                .filter(|event| event.causal_stream == causal_stream)
                .cloned()
                .collect::<Vec<_>>(),
        )
    }

    fn acquire_process_lock(
        &self,
        root: &super::AnchoredRoot,
    ) -> Result<super::AnchoredExclusiveLock, StoreError> {
        self.validate_store_layout(root)?;
        root.lock_exclusive("twin/events/append-v1.lock")
            .map_err(store_capability_error)
    }

    fn ensure_store_layout(&self) -> Result<super::AnchoredRoot, StoreError> {
        if !self.data_path.exists() {
            fs::create_dir_all(&self.data_path)?;
        }
        validate_real_directory(&self.data_path, "trusted app-data root")?;
        let root = super::AnchoredRoot::open(&self.data_path).map_err(store_capability_error)?;
        root.open_directory("twin", true)
            .map_err(store_capability_error)?;
        root.open_directory("twin/events", true)
            .map_err(store_capability_error)?;
        root.open_directory("twin/events/v1", true)
            .map_err(|error| {
                StoreError::Io(format!(
                    "failed to initialize canonical Twin event directory: {error}"
                ))
            })?;
        root.open_directory("twin/events/quarantine", true)
            .map_err(store_capability_error)?;
        root.open_directory("twin/events/quarantine/v1", true)
            .map_err(|error| {
                StoreError::Io(format!(
                    "failed to initialize Twin event quarantine: {error}"
                ))
            })?;
        root.open_directory("twin/events/staging/v1", true)
            .map_err(store_capability_error)?;
        Ok(root)
    }

    fn validate_store_layout(&self, root: &super::AnchoredRoot) -> Result<(), StoreError> {
        for directory in [
            "twin",
            "twin/events",
            "twin/events/v1",
            "twin/events/quarantine",
            "twin/events/quarantine/v1",
            "twin/events/staging/v1",
        ] {
            root.open_directory(directory, false)
                .map_err(store_capability_error)?;
        }
        Ok(())
    }

    fn root_capability(&self) -> Result<super::AnchoredRoot, StoreError> {
        self.root
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event capability lock poisoned".into()))?
            .as_ref()
            .ok_or(StoreError::NotInitialized)?
            .try_clone()
            .map_err(store_capability_error)
    }

    fn load_records(
        &self,
        root: &super::AnchoredRoot,
    ) -> Result<BTreeMap<EventId, TwinEvent>, StoreError> {
        self.validate_store_layout(root)?;
        let mut parsed = Vec::new();
        let keys = self.event_record_keys(root)?;
        #[cfg(test)]
        if let Some(hook) = self.after_enumeration_hook.lock().unwrap().take() {
            hook();
        }
        for key in keys {
            let bytes = match root.read_bounded(&key, MAX_TWIN_EVENT_BYTES) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    return Err(StoreError::Invalid(format!(
                        "Twin event disappeared between enumeration and read: {key}"
                    )))
                }
                Err(super::MutationError::Invalid(message)) if message.contains("exceeds its") => {
                    self.quarantine(root, &key)?;
                    continue;
                }
                Err(error) => return Err(store_capability_error(error)),
            };
            let event = match serde_json::from_slice::<TwinEvent>(&bytes) {
                Ok(mut event) => {
                    if event.validate().is_err() || derive_event_id(&event) != event.event_id {
                        self.quarantine(root, &key)?;
                        continue;
                    }
                    if key != canonical_event_key(&event.event_id) {
                        self.quarantine(root, &key)?;
                        continue;
                    }
                    event.normalize();
                    event
                }
                Err(_) => {
                    self.quarantine(root, &key)?;
                    continue;
                }
            };
            parsed.push((key, event));
        }
        let mut known: BTreeMap<_, _> = parsed
            .iter()
            .map(|(_, event)| (event.event_id.clone(), event.causal_stream))
            .collect();
        loop {
            let invalid: BTreeSet<_> = parsed
                .iter()
                .filter(|(_, event)| validate_event_references(&known, event).is_err())
                .map(|(_, event)| event.event_id.clone())
                .collect();
            if invalid.is_empty() {
                break;
            }
            let mut retained = Vec::with_capacity(parsed.len() - invalid.len());
            for (path, event) in parsed {
                if invalid.contains(&event.event_id) {
                    self.quarantine(root, &path)?;
                } else {
                    retained.push((path, event));
                }
            }
            for event_id in &invalid {
                known.remove(event_id);
            }
            parsed = retained;
        }
        let mut events = BTreeMap::new();
        for (path, event) in parsed {
            if let Some(existing) = events.get(&event.event_id) {
                if semantic_bytes(existing) != semantic_bytes(&event) {
                    return Err(StoreError::Collision(event.event_id));
                }
                self.quarantine(root, &path)?;
                continue;
            }
            events.insert(event.event_id.clone(), event);
        }
        validate_all_sequences(&events)?;
        topological_order(&events.values().cloned().collect::<Vec<_>>())?;
        Ok(events)
    }

    fn event_record_keys(&self, root: &super::AnchoredRoot) -> Result<Vec<String>, StoreError> {
        let mut keys = Vec::new();
        for (name, kind) in root
            .directory_entries("twin/events/v1")
            .map_err(store_capability_error)?
        {
            match kind {
                super::AnchoredEntryKind::File if name.ends_with(".json") => {
                    keys.push(format!("twin/events/v1/{name}"));
                }
                super::AnchoredEntryKind::File => {}
                super::AnchoredEntryKind::Directory => {
                    let directory = format!("twin/events/v1/{name}");
                    for (leaf, leaf_kind) in root
                        .directory_entries(&directory)
                        .map_err(store_capability_error)?
                    {
                        match leaf_kind {
                            super::AnchoredEntryKind::File if leaf.ends_with(".json") => {
                                keys.push(format!("{directory}/{leaf}"));
                            }
                            super::AnchoredEntryKind::File => {}
                            super::AnchoredEntryKind::Directory => {
                                return Err(StoreError::Invalid(format!(
                                    "canonical Twin event tree is deeper than one prefix: {directory}/{leaf}"
                                )))
                            }
                        }
                    }
                }
            }
        }
        keys.sort();
        Ok(keys)
    }

    fn quarantine(&self, root: &super::AnchoredRoot, source: &str) -> Result<(), StoreError> {
        self.validate_store_layout(root)?;
        let name = source.rsplit('/').next().unwrap_or("event.json");
        let target = format!("twin/events/quarantine/v1/{}-{name}", Uuid::new_v4());
        root.rename(source, &target, false)
            .map_err(store_capability_error)
    }

    fn install_no_clobber(
        &self,
        root: &super::AnchoredRoot,
        event: &TwinEvent,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        self.validate_store_layout(root)?;
        let target = canonical_event_key(&event.event_id);
        root.install_no_clobber(&target, "twin/events/staging/v1", bytes)
            .map_err(store_capability_error)?;
        let existing = self.read_existing_canonical(root, &target, &event.event_id)?;
        if semantic_bytes(&existing) == semantic_bytes(event) {
            Ok(())
        } else {
            Err(StoreError::Collision(event.event_id.clone()))
        }
    }

    fn read_existing_canonical(
        &self,
        root: &super::AnchoredRoot,
        key: &str,
        expected_id: &EventId,
    ) -> Result<TwinEvent, StoreError> {
        let bytes = root
            .read_bounded(key, MAX_TWIN_EVENT_BYTES)
            .map_err(store_capability_error)?
            .ok_or_else(|| StoreError::Invalid("installed Twin event disappeared".into()))?;
        let mut event: TwinEvent = serde_json::from_slice(&bytes).map_err(|error| {
            StoreError::Invalid(format!("existing Twin event is malformed: {error}"))
        })?;
        event.validate().map_err(StoreError::Invalid)?;
        if &event.event_id != expected_id
            || derive_event_id(&event) != event.event_id
            || key != canonical_event_key(&event.event_id)
        {
            return Err(StoreError::Invalid(
                "existing Twin event is not a canonical record".into(),
            ));
        }
        event.normalize();
        Ok(event)
    }
}

fn canonical_event_key(event_id: &EventId) -> String {
    format!(
        "twin/events/v1/{}/{}.json",
        &event_id.as_str()[..2],
        event_id
    )
}

fn store_capability_error(error: super::MutationError) -> StoreError {
    match error {
        super::MutationError::Store(error) => error,
        super::MutationError::Io(message) => StoreError::Io(message),
        super::MutationError::Invalid(message) => StoreError::Invalid(message),
        super::MutationError::RecoveryConflict(message) => StoreError::Invalid(message),
    }
}

fn serialize_record(event: &TwinEvent) -> Result<Vec<u8>, StoreError> {
    let mut bytes =
        serde_json::to_vec_pretty(event).map_err(|error| StoreError::Invalid(error.to_string()))?;
    bytes.push(b'\n');
    validate_record_size(bytes.len() as u64)?;
    Ok(bytes)
}

pub(crate) fn validate_real_directory(path: &Path, label: &str) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StoreError::Invalid(format!(
            "{label} must be a real directory, not a symlink or other entry: {}",
            path.display()
        )));
    }
    Ok(())
}

fn validate_record_size(length: u64) -> Result<(), StoreError> {
    if length > MAX_TWIN_EVENT_BYTES as u64 {
        return Err(StoreError::Invalid(format!(
            "Twin event record exceeds the {MAX_TWIN_EVENT_BYTES}-byte limit"
        )));
    }
    Ok(())
}

#[cfg(test)]
fn sync_directory(path: &Path) -> Result<(), StoreError> {
    sync_directory_impl(path, false)
}

#[cfg(test)]
fn sync_directory_impl(path: &Path, trusted_boundary: bool) -> Result<(), StoreError> {
    let metadata = if trusted_boundary {
        fs::metadata(path)?
    } else {
        fs::symlink_metadata(path)?
    };
    if (!trusted_boundary && metadata.file_type().is_symlink()) || !metadata.is_dir() {
        return Err(StoreError::Invalid(format!(
            "directory sync target is not a real directory: {}",
            path.display()
        )));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        directory.sync_all()?;
        Ok(())
    }
    #[cfg(unix)]
    {
        std::fs::File::open(path)?.sync_all()?;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(StoreError::Io(
            "directory synchronization is unsupported on this platform".into(),
        ))
    }
}

fn validate_candidate_append(
    events: &BTreeMap<EventId, TwinEvent>,
    event: &mut TwinEvent,
) -> Result<AppendOutcome, StoreError> {
    event.validate().map_err(StoreError::Invalid)?;
    event.normalize();
    if let Some(existing) = events.get(&event.event_id) {
        return if semantic_bytes(existing) == semantic_bytes(event) {
            Ok(AppendOutcome::Duplicate)
        } else {
            Err(StoreError::Collision(event.event_id.clone()))
        };
    }
    let expected = derive_event_id(event);
    if event.event_id != expected {
        return Err(StoreError::WrongEventId {
            supplied: event.event_id.clone(),
            expected,
        });
    }
    validate_append_sequence(events, event)?;
    let known = events
        .values()
        .map(|known| (known.event_id.clone(), known.causal_stream))
        .collect();
    validate_event_references(&known, event)?;
    Ok(AppendOutcome::Appended)
}

fn validate_append_sequence(
    events: &BTreeMap<EventId, TwinEvent>,
    event: &TwinEvent,
) -> Result<(), StoreError> {
    let same_device: Vec<_> = events
        .values()
        .filter(|item| {
            item.device_id == event.device_id && item.causal_stream == event.causal_stream
        })
        .collect();
    let expected = same_device
        .iter()
        .map(|item| item.device_sequence)
        .max()
        .unwrap_or(0)
        + 1;
    if event.device_sequence != expected {
        return Err(StoreError::Invalid(format!(
            "device sequence must be {expected}"
        )));
    }
    if event.device_sequence > 1 {
        let predecessor = same_device
            .into_iter()
            .find(|item| item.device_sequence + 1 == event.device_sequence)
            .ok_or_else(|| {
                StoreError::Invalid("missing immediately preceding device sequence".into())
            })?;
        if !event.causal_parents.contains(&predecessor.event_id) {
            return Err(StoreError::Invalid(
                "causal parents must directly include the preceding same-device event".into(),
            ));
        }
    }
    Ok(())
}

fn validate_all_sequences(events: &BTreeMap<EventId, TwinEvent>) -> Result<(), StoreError> {
    let mut by_device: BTreeMap<(DeviceId, CausalStream), Vec<&TwinEvent>> = BTreeMap::new();
    for event in events.values() {
        by_device
            .entry((event.device_id.clone(), event.causal_stream))
            .or_default()
            .push(event);
    }
    for device_events in by_device.values_mut() {
        device_events.sort_by_key(|event| event.device_sequence);
        for (index, event) in device_events.iter().enumerate() {
            let expected = index as u64 + 1;
            if event.device_sequence != expected {
                return Err(StoreError::Invalid(format!(
                    "invalid or reused device sequence {0}",
                    event.device_sequence
                )));
            }
            if index > 0
                && !event
                    .causal_parents
                    .contains(&device_events[index - 1].event_id)
            {
                return Err(StoreError::Invalid(
                    "device sequence does not directly include predecessor".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_event_references(
    known: &BTreeMap<EventId, CausalStream>,
    event: &TwinEvent,
) -> Result<(), StoreError> {
    for parent in &event.causal_parents {
        let parent_stream = known
            .get(parent)
            .ok_or_else(|| StoreError::MissingParent(parent.clone()))?;
        if *parent_stream != event.causal_stream {
            return Err(StoreError::Invalid(
                "causal parents must be in the same causal stream".into(),
            ));
        }
    }
    let mut evidence_event_ids = event_event_ids(&event.evidence)?;
    for relationship in &event.context.relationships {
        evidence_event_ids.extend(event_event_ids(&relationship.evidence)?);
    }
    if event.causal_stream == CausalStream::LocalOnly {
        return Ok(());
    }
    for reference in event
        .supersedes
        .iter()
        .chain(&event.reinforces)
        .cloned()
        .chain(evidence_event_ids)
    {
        match known.get(&reference) {
            Some(CausalStream::SyncEligible) => {}
            Some(CausalStream::LocalOnly) => {
                return Err(StoreError::Invalid(
                    "sync-eligible events may not reference local-only events".into(),
                ))
            }
            None => return Err(StoreError::MissingParent(reference)),
        }
    }
    Ok(())
}

fn event_event_ids(evidence: &[EvidenceRef]) -> Result<Vec<EventId>, StoreError> {
    evidence
        .iter()
        .filter(|reference| reference.evidence_type == EvidenceType::Event)
        .map(|reference| EventId::parse(reference.source_id.as_str()).map_err(StoreError::Invalid))
        .collect()
}

pub fn topological_order(events: &[TwinEvent]) -> Result<Vec<TwinEvent>, StoreError> {
    let by_id: BTreeMap<_, _> = events
        .iter()
        .map(|event| (event.event_id.clone(), event.clone()))
        .collect();
    if by_id.len() != events.len() {
        return Err(StoreError::Invalid(
            "duplicate event IDs in ordering input".into(),
        ));
    }
    let mut indegree: BTreeMap<EventId, usize> = by_id.keys().cloned().map(|id| (id, 0)).collect();
    let mut children: BTreeMap<EventId, BTreeSet<EventId>> = BTreeMap::new();
    for event in events {
        for parent in &event.causal_parents {
            if !by_id.contains_key(parent) {
                return Err(StoreError::MissingParent(parent.clone()));
            }
            *indegree.get_mut(&event.event_id).expect("event exists") += 1;
            children
                .entry(parent.clone())
                .or_default()
                .insert(event.event_id.clone());
        }
    }
    let mut ready: BTreeSet<_> = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(id.clone()))
        .collect();
    let mut ordered = Vec::with_capacity(events.len());
    while let Some(id) = ready.pop_first() {
        ordered.push(by_id.get(&id).expect("ready event exists").clone());
        if let Some(next) = children.get(&id) {
            for child in next {
                let degree = indegree.get_mut(child).expect("child exists");
                *degree -= 1;
                if *degree == 0 {
                    ready.insert(child.clone());
                }
            }
        }
    }
    if ordered.len() != events.len() {
        return Err(StoreError::CausalCycle);
    }
    Ok(ordered)
}

pub trait EventRecorder: Send + Sync {
    fn commit_mutation(
        &self,
        _origin: crate::services::twin_events::MutationOrigin,
        _stream: CausalStream,
        _source_channel: crate::models::twin_event::SourceChannel,
        _targets: Vec<crate::services::twin_events::TargetMutation>,
        _drafts: Vec<crate::services::twin_events::TwinEventDraft>,
    ) -> Result<
        crate::services::twin_events::MutationCommit,
        crate::services::twin_events::MutationError,
    > {
        Err(crate::services::twin_events::MutationError::Invalid(
            "event recorder does not support coordinated mutations".into(),
        ))
    }

    fn commit_planned_mutation(
        &self,
        _origin: crate::services::twin_events::MutationOrigin,
        _planner: &mut dyn FnMut() -> Result<
            Option<crate::services::twin_events::MutationPlan>,
            crate::services::twin_events::MutationError,
        >,
    ) -> Result<
        crate::services::twin_events::MutationCommit,
        crate::services::twin_events::MutationError,
    > {
        Err(crate::services::twin_events::MutationError::Invalid(
            "event recorder does not support locked mutation planning".into(),
        ))
    }

    fn recover_pending_mutations(
        &self,
    ) -> Result<usize, crate::services::twin_events::MutationError> {
        Ok(0)
    }

    #[cfg(test)]
    fn retarget_markdown_root(
        &self,
        _vault_path: &std::path::Path,
    ) -> Result<(), crate::services::twin_events::MutationError> {
        Err(crate::services::twin_events::MutationError::Invalid(
            "event recorder does not support Markdown root retargeting".into(),
        ))
    }

    fn recorded_events(
        &self,
    ) -> Result<Vec<TwinEvent>, crate::services::twin_events::MutationError> {
        Ok(Vec::new())
    }

    fn is_noop(&self) -> bool {
        false
    }
}

#[derive(Debug, Default)]
pub struct NoopEventRecorder;
impl EventRecorder for NoopEventRecorder {
    fn commit_mutation(
        &self,
        _origin: crate::services::twin_events::MutationOrigin,
        _stream: CausalStream,
        _source_channel: crate::models::twin_event::SourceChannel,
        _targets: Vec<crate::services::twin_events::TargetMutation>,
        _drafts: Vec<crate::services::twin_events::TwinEventDraft>,
    ) -> Result<
        crate::services::twin_events::MutationCommit,
        crate::services::twin_events::MutationError,
    > {
        Ok(crate::services::twin_events::MutationCommit {
            mutation_id: None,
            events: Vec::new(),
        })
    }

    #[cfg(test)]
    fn retarget_markdown_root(
        &self,
        _vault_path: &std::path::Path,
    ) -> Result<(), crate::services::twin_events::MutationError> {
        Ok(())
    }

    fn commit_planned_mutation(
        &self,
        _origin: crate::services::twin_events::MutationOrigin,
        planner: &mut dyn FnMut() -> Result<
            Option<crate::services::twin_events::MutationPlan>,
            crate::services::twin_events::MutationError,
        >,
    ) -> Result<
        crate::services::twin_events::MutationCommit,
        crate::services::twin_events::MutationError,
    > {
        let _ = planner()?;
        Ok(crate::services::twin_events::MutationCommit {
            mutation_id: None,
            events: Vec::new(),
        })
    }

    fn is_noop(&self) -> bool {
        true
    }
}

#[derive(Debug)]
pub struct UnavailableEventRecorder {
    reason: String,
}

impl UnavailableEventRecorder {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl EventRecorder for UnavailableEventRecorder {
    fn commit_mutation(
        &self,
        _origin: crate::services::twin_events::MutationOrigin,
        _stream: CausalStream,
        _source_channel: crate::models::twin_event::SourceChannel,
        _targets: Vec<crate::services::twin_events::TargetMutation>,
        _drafts: Vec<crate::services::twin_events::TwinEventDraft>,
    ) -> Result<
        crate::services::twin_events::MutationCommit,
        crate::services::twin_events::MutationError,
    > {
        Err(crate::services::twin_events::MutationError::Invalid(
            self.reason.clone(),
        ))
    }

    #[cfg(test)]
    fn retarget_markdown_root(
        &self,
        _vault_path: &std::path::Path,
    ) -> Result<(), crate::services::twin_events::MutationError> {
        Err(crate::services::twin_events::MutationError::Invalid(
            self.reason.clone(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin_event::{
        BoundedContent, CausalStream, DecisionRecorded, EvidenceRef, EvidenceType, Governance,
        Identifier, RelationshipAssertion, RelationshipDirection, RelationshipPredicate,
        TwinEventPayload, MAX_DECISION_OPTIONS,
    };
    use crate::services::twin_events::test_support::{
        valid_event, valid_event_for_device, valid_event_for_device_and_stream,
    };

    #[test]
    fn event_recorder_has_no_raw_append_surface_or_store_implementation() {
        let source = include_str!("store.rs");
        assert!(!source.contains(&["fn rec", "ord(&self"].concat()));
        assert!(!source.contains(&["impl EventRecorder for ", "TwinEventStore"].concat()));
    }

    #[test]
    fn production_surface_cannot_retarget_the_markdown_root_outside_the_wal() {
        let recorder = include_str!("store.rs").replace("\r\n", "\n");
        let coordinator = include_str!("mutation_coordinator.rs").replace("\r\n", "\n");
        let knowledge = include_str!("../knowledge_store.rs").replace("\r\n", "\n");
        assert!(recorder.contains("#[cfg(test)]\n    fn retarget_markdown_root("));
        assert!(coordinator.contains("#[cfg(test)]\n    pub(crate) fn retarget_markdown_root("));
        assert!(knowledge.contains("#[cfg(test)]\n    pub(crate) fn set_vault_path("));
        assert!(!coordinator.contains("pub fn retarget_markdown_root("));
    }

    #[test]
    fn production_event_and_identity_io_stays_behind_anchored_capabilities() {
        let event_store = include_str!("store.rs");
        for required in [
            "root.lock_exclusive(\"twin/events/append-v1.lock\")",
            ".directory_entries(\"twin/events/v1\")",
            "root.read_bounded(&key, MAX_TWIN_EVENT_BYTES)",
            "root.rename(source, &target, false)",
            "root.install_no_clobber(&target, \"twin/events/staging/v1\", bytes)",
        ] {
            assert!(
                event_store.contains(required),
                "missing anchored event I/O: {required}"
            );
        }
        assert!(event_store.contains("#[cfg(test)]\nuse walkdir::WalkDir;"));

        let coordinator = include_str!("mutation_coordinator.rs");
        for required in [
            "root.lock_exclusive(\"twin/events/mutation-v1.lock\")",
            "root.read_bounded(WRITER_KEY, WRITER_FILE_LIMIT as usize)",
            "root.install_no_clobber(WRITER_KEY, STAGING_KEY, &bytes)",
        ] {
            assert!(
                coordinator.contains(required),
                "missing anchored identity/lock I/O: {required}"
            );
        }
    }

    fn write_event(path: &Path, event: &TwinEvent) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec_pretty(event).unwrap()).unwrap();
    }

    fn canonical_path(store: &TwinEventStore, event: &TwinEvent) -> PathBuf {
        store
            .events_dir()
            .join(&event.event_id.as_str()[..2])
            .join(format!("{}.json", event.event_id))
    }

    fn event_reference(event: &TwinEvent) -> EvidenceRef {
        EvidenceRef {
            evidence_type: EvidenceType::Event,
            source_id: Identifier::parse(event.event_id.as_str()).unwrap(),
            digest: None,
        }
    }

    fn relationship_with_event_reference(event: &TwinEvent) -> RelationshipAssertion {
        RelationshipAssertion {
            subject_id: crate::models::twin_event::EntityId::parse("owner").unwrap(),
            predicate: RelationshipPredicate::parse("works_with").unwrap(),
            object_id: crate::models::twin_event::EntityId::parse("person-1").unwrap(),
            direction: RelationshipDirection::Directed,
            valid_from: None,
            valid_to: None,
            evidence: vec![event_reference(event)],
            governance: Governance::direct_observation(),
        }
    }

    fn try_symlink_dir(original: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(original, link).unwrap();
            true
        }
        #[cfg(windows)]
        {
            match std::os::windows::fs::symlink_dir(original, link) {
                Ok(()) => true,
                Err(error) if error.raw_os_error() == Some(1314) => {
                    eprintln!("skipping symlink regression without Windows symlink privilege");
                    false
                }
                Err(error) => panic!("failed to create directory symlink: {error}"),
            }
        }
    }

    fn try_symlink_file(original: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(original, link).unwrap();
            true
        }
        #[cfg(windows)]
        {
            match std::os::windows::fs::symlink_file(original, link) {
                Ok(()) => true,
                Err(error) if error.raw_os_error() == Some(1314) => {
                    eprintln!("skipping symlink regression without Windows symlink privilege");
                    false
                }
                Err(error) => panic!("failed to create file symlink: {error}"),
            }
        }
    }

    #[test]
    fn duplicate_is_noop_and_collision_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let event = valid_event(1, Vec::new());
        assert_eq!(
            store.append(event.clone()).unwrap(),
            AppendOutcome::Appended
        );
        assert_eq!(
            store.append(event.clone()).unwrap(),
            AppendOutcome::Duplicate
        );
        let mut collision = event;
        collision.recorded_at += chrono::Duration::seconds(1);
        assert!(matches!(
            store.append(collision),
            Err(StoreError::Collision(_))
        ));
    }

    #[test]
    fn sequence_starts_at_one_and_requires_direct_predecessor_without_reuse_or_gaps() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        assert!(store.append(valid_event(2, Vec::new())).is_err());
        let first = valid_event(1, Vec::new());
        store.append(first.clone()).unwrap();
        let mut reused = valid_event(1, Vec::new());
        reused.observed_at += chrono::Duration::seconds(1);
        reused.event_id = crate::services::twin_events::derive_event_id(&reused);
        assert!(store.append(reused).is_err());
        assert!(store
            .append(valid_event(3, vec![first.event_id.clone()]))
            .is_err());
        assert!(store.append(valid_event(2, Vec::new())).is_err());
        store.append(valid_event(2, vec![first.event_id])).unwrap();
    }

    #[test]
    fn causal_lanes_sequence_independently_and_sync_peer_can_omit_local_lane() {
        let sync_one = valid_event_for_device_and_stream(
            "device-a",
            CausalStream::SyncEligible,
            1,
            Vec::new(),
        );
        let local_one =
            valid_event_for_device_and_stream("device-a", CausalStream::LocalOnly, 1, Vec::new());
        let sync_two = valid_event_for_device_and_stream(
            "device-a",
            CausalStream::SyncEligible,
            2,
            vec![sync_one.event_id.clone()],
        );

        let origin_dir = tempfile::tempdir().unwrap();
        let peer_dir = tempfile::tempdir().unwrap();
        let origin = TwinEventStore::new(origin_dir.path());
        let peer = TwinEventStore::new(peer_dir.path());
        origin.initialize().unwrap();
        peer.initialize().unwrap();
        for event in [sync_one.clone(), local_one, sync_two.clone()] {
            origin.append(event).unwrap();
        }
        for event in [sync_one.clone(), sync_two.clone()] {
            peer.append(event).unwrap();
        }

        let origin_shared = origin
            .ordered_events_for_stream(CausalStream::SyncEligible)
            .unwrap();
        let peer_shared = peer
            .ordered_events_for_stream(CausalStream::SyncEligible)
            .unwrap();
        assert_eq!(origin_shared, vec![sync_one, sync_two]);
        assert_eq!(peer_shared, origin_shared);
    }

    #[test]
    fn sequence_predecessors_are_lane_local_and_cross_stream_causal_parents_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let local_one =
            valid_event_for_device_and_stream("device-a", CausalStream::LocalOnly, 1, Vec::new());
        let sync_one = valid_event_for_device_and_stream(
            "device-a",
            CausalStream::SyncEligible,
            1,
            Vec::new(),
        );
        store.append(local_one.clone()).unwrap();
        store.append(sync_one.clone()).unwrap();

        let local_two_wrong = valid_event_for_device_and_stream(
            "device-a",
            CausalStream::LocalOnly,
            2,
            vec![sync_one.event_id.clone()],
        );
        assert!(store.append(local_two_wrong).is_err());
        let sync_two_wrong = valid_event_for_device_and_stream(
            "device-a",
            CausalStream::SyncEligible,
            2,
            vec![local_one.event_id.clone()],
        );
        assert!(store.append(sync_two_wrong).is_err());

        let local_two = valid_event_for_device_and_stream(
            "device-a",
            CausalStream::LocalOnly,
            2,
            vec![local_one.event_id],
        );
        let sync_two = valid_event_for_device_and_stream(
            "device-a",
            CausalStream::SyncEligible,
            2,
            vec![sync_one.event_id],
        );
        store.append(local_two).unwrap();
        store.append(sync_two).unwrap();
    }

    #[test]
    fn sync_eligible_rejects_every_reference_to_a_local_event_but_local_may_reference_shared() {
        for kind in [
            "causal",
            "supersedes",
            "reinforces",
            "evidence",
            "relationship",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let store = TwinEventStore::new(temp.path());
            store.initialize().unwrap();
            let local = valid_event_for_device_and_stream(
                "local-device",
                CausalStream::LocalOnly,
                1,
                Vec::new(),
            );
            store.append(local.clone()).unwrap();
            let mut shared = valid_event_for_device_and_stream(
                "shared-device",
                CausalStream::SyncEligible,
                1,
                Vec::new(),
            );
            match kind {
                "causal" => shared.causal_parents.push(local.event_id.clone()),
                "supersedes" => shared.supersedes.push(local.event_id.clone()),
                "reinforces" => shared.reinforces.push(local.event_id.clone()),
                "evidence" => shared.evidence.push(event_reference(&local)),
                "relationship" => shared
                    .context
                    .relationships
                    .push(relationship_with_event_reference(&local)),
                _ => unreachable!(),
            }
            shared.event_id = crate::services::twin_events::derive_event_id(&shared);
            assert!(store.append(shared).is_err(), "{kind}");
        }

        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let shared = valid_event_for_device_and_stream(
            "shared-device",
            CausalStream::SyncEligible,
            1,
            Vec::new(),
        );
        store.append(shared.clone()).unwrap();
        let mut local = valid_event_for_device_and_stream(
            "local-device",
            CausalStream::LocalOnly,
            1,
            Vec::new(),
        );
        local.supersedes.push(shared.event_id.clone());
        local.reinforces.push(shared.event_id.clone());
        local.evidence.push(event_reference(&shared));
        local
            .context
            .relationships
            .push(relationship_with_event_reference(&shared));
        local.event_id = crate::services::twin_events::derive_event_id(&local);
        store.append(local).unwrap();
    }

    #[test]
    fn event_evidence_requires_a_valid_event_id_in_both_lanes() {
        for causal_stream in [CausalStream::LocalOnly, CausalStream::SyncEligible] {
            let temp = tempfile::tempdir().unwrap();
            let store = TwinEventStore::new(temp.path());
            store.initialize().unwrap();
            let mut event =
                valid_event_for_device_and_stream("device-a", causal_stream, 1, Vec::new());
            event.evidence.push(EvidenceRef {
                evidence_type: EvidenceType::Event,
                source_id: Identifier::parse("not-an-event-id").unwrap(),
                digest: None,
            });
            event.event_id = crate::services::twin_events::derive_event_id(&event);
            assert!(store.append(event).is_err(), "{causal_stream:?}");
        }
    }

    #[test]
    fn loading_quarantines_sync_event_that_references_local_event() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        let local = valid_event_for_device_and_stream(
            "local-device",
            CausalStream::LocalOnly,
            1,
            Vec::new(),
        );
        let mut shared = valid_event_for_device_and_stream(
            "shared-device",
            CausalStream::SyncEligible,
            1,
            Vec::new(),
        );
        shared.evidence.push(event_reference(&local));
        shared.event_id = crate::services::twin_events::derive_event_id(&shared);
        write_event(&canonical_path(&store, &local), &local);
        write_event(&canonical_path(&store, &shared), &shared);

        store.initialize().unwrap();

        assert_eq!(store.ordered_events().unwrap(), vec![local]);
        assert_eq!(
            std::fs::read_dir(store.quarantine_dir()).unwrap().count(),
            1
        );
    }

    #[test]
    fn lock_creation_durability_uses_the_lock_files_actual_parent() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        assert_eq!(
            store.lock_durability_directory().unwrap(),
            store.lock_path().parent().unwrap()
        );
        assert_eq!(
            store.lock_durability_directory().unwrap(),
            temp.path().join("twin").join("events")
        );
    }

    #[test]
    fn ordering_is_parent_first_and_independent_of_cross_device_arrival() {
        let first_a = valid_event_for_device("device-a", 1, Vec::new());
        let child_a = valid_event_for_device("device-a", 2, vec![first_a.event_id.clone()]);
        let first_b = valid_event_for_device("device-b", 1, Vec::new());
        let temp_a = tempfile::tempdir().unwrap();
        let temp_b = tempfile::tempdir().unwrap();
        let store_a = TwinEventStore::new(temp_a.path());
        let store_b = TwinEventStore::new(temp_b.path());
        store_a.initialize().unwrap();
        store_b.initialize().unwrap();
        for event in [first_b.clone(), first_a.clone(), child_a.clone()] {
            store_a.append(event).unwrap();
        }
        for event in [first_a.clone(), child_a.clone(), first_b.clone()] {
            store_b.append(event).unwrap();
        }
        let ids_a: Vec<_> = store_a
            .ordered_events()
            .unwrap()
            .into_iter()
            .map(|e| e.event_id)
            .collect();
        let ids_b: Vec<_> = store_b
            .ordered_events()
            .unwrap()
            .into_iter()
            .map(|e| e.event_id)
            .collect();
        assert_eq!(ids_a, ids_b);
        assert!(
            ids_a.iter().position(|id| id == &first_a.event_id).unwrap()
                < ids_a.iter().position(|id| id == &child_a.event_id).unwrap()
        );
    }

    #[test]
    fn full_and_sync_filtered_order_are_stable_across_arrival_order() {
        let shared_a = valid_event_for_device_and_stream(
            "shared-a",
            CausalStream::SyncEligible,
            1,
            Vec::new(),
        );
        let shared_b = valid_event_for_device_and_stream(
            "shared-b",
            CausalStream::SyncEligible,
            1,
            Vec::new(),
        );
        let local =
            valid_event_for_device_and_stream("local-a", CausalStream::LocalOnly, 1, Vec::new());
        let first_dir = tempfile::tempdir().unwrap();
        let second_dir = tempfile::tempdir().unwrap();
        let first = TwinEventStore::new(first_dir.path());
        let second = TwinEventStore::new(second_dir.path());
        first.initialize().unwrap();
        second.initialize().unwrap();
        for event in [shared_b.clone(), local.clone(), shared_a.clone()] {
            first.append(event).unwrap();
        }
        for event in [shared_a, local, shared_b] {
            second.append(event).unwrap();
        }

        let full_ids = |store: &TwinEventStore| {
            store
                .ordered_events()
                .unwrap()
                .into_iter()
                .map(|event| event.event_id)
                .collect::<Vec<_>>()
        };
        let shared_ids = |store: &TwinEventStore| {
            store
                .ordered_events_for_stream(CausalStream::SyncEligible)
                .unwrap()
                .into_iter()
                .map(|event| event.event_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(full_ids(&first), full_ids(&second));
        assert_eq!(shared_ids(&first), shared_ids(&second));
        assert_eq!(
            shared_ids(&first),
            first
                .ordered_events()
                .unwrap()
                .into_iter()
                .filter(|event| event.causal_stream == CausalStream::SyncEligible)
                .map(|event| event.event_id)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn missing_parent_and_cycle_fail_closed() {
        let event = valid_event(1, Vec::new());
        let missing =
            EventId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .unwrap();
        assert!(topological_order(&[TwinEvent {
            causal_parents: vec![missing],
            ..event.clone()
        }])
        .is_err());
        assert!(topological_order(&[TwinEvent {
            causal_parents: vec![event.event_id.clone()],
            ..event
        }])
        .is_err());
    }

    #[test]
    fn malformed_records_are_quarantined_on_initialization() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        let events = store.events_dir();
        std::fs::create_dir_all(events.join("aa")).unwrap();
        std::fs::write(events.join("aa").join("bad.json"), b"not json").unwrap();
        store.initialize().unwrap();
        assert_eq!(
            std::fs::read_dir(store.quarantine_dir()).unwrap().count(),
            1
        );
    }

    #[test]
    fn wrong_new_event_id_and_unavailable_quarantine_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let mut wrong = valid_event(1, Vec::new());
        wrong.event_id =
            EventId::parse("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
                .unwrap();
        assert!(matches!(
            store.append(wrong),
            Err(StoreError::WrongEventId { .. })
        ));

        let second = tempfile::tempdir().unwrap();
        let blocked = TwinEventStore::new(second.path());
        let events = blocked.events_dir();
        std::fs::create_dir_all(events.join("aa")).unwrap();
        std::fs::write(events.join("aa").join("bad.json"), b"bad").unwrap();
        let quarantine = blocked.quarantine_dir();
        std::fs::create_dir_all(quarantine.parent().unwrap()).unwrap();
        std::fs::write(&quarantine, b"not a directory").unwrap();
        assert!(blocked.initialize().is_err());
    }

    #[test]
    fn duplicate_set_members_are_rejected_before_persistence() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let mut event = valid_event(1, Vec::new());
        event.context.tags = vec!["same".into(), "same".into()];
        event.event_id = crate::services::twin_events::derive_event_id(&event);
        assert!(matches!(store.append(event), Err(StoreError::Invalid(_))));
    }

    #[test]
    fn event_store_construction_does_no_io() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("missing");
        let _store = TwinEventStore::new(&data);
        assert!(!data.exists());
    }

    #[test]
    fn initialized_peer_refreshes_events_written_by_another_process_instance() {
        let temp = tempfile::tempdir().unwrap();
        let first = TwinEventStore::new(temp.path());
        let second = TwinEventStore::new(temp.path());
        first.initialize().unwrap();
        second.initialize().unwrap();
        second.append(valid_event(1, Vec::new())).unwrap();
        assert_eq!(first.ordered_events().unwrap().len(), 1);
    }

    #[test]
    fn record_size_limit_is_inclusive() {
        assert!(validate_record_size(MAX_TWIN_EVENT_BYTES as u64).is_ok());
        assert!(validate_record_size(MAX_TWIN_EVENT_BYTES as u64 + 1).is_err());
    }

    #[test]
    fn oversized_append_is_rejected_before_installation() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let options = (0..MAX_DECISION_OPTIONS)
            .map(|index| {
                BoundedContent::parse(format!("{index:02}{}", "x".repeat(32_760))).unwrap()
            })
            .collect();
        let mut event = crate::services::twin_events::test_support::event_for_payload(
            TwinEventPayload::DecisionRecorded(DecisionRecorded {
                decision_id: Identifier::parse("large-decision").unwrap(),
                decision: BoundedContent::parse("choose").unwrap(),
                options,
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: Default::default(),
            }),
        );
        event.event_id = crate::services::twin_events::derive_event_id(&event);
        assert!(matches!(store.append(event), Err(StoreError::Invalid(_))));
        assert_eq!(
            WalkDir::new(store.events_dir())
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().and_then(|v| v.to_str()) == Some("json"))
                .count(),
            0
        );
    }

    #[test]
    fn oversized_record_is_quarantined_before_parsing() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        let path = store
            .events_dir()
            .join("aa")
            .join(format!("{}.json", "a".repeat(64)));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![b' '; MAX_TWIN_EVENT_BYTES + 1]).unwrap();
        store.initialize().unwrap();
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_dir(store.quarantine_dir()).unwrap().count(),
            1
        );
    }

    #[test]
    fn directory_sync_helper_accepts_a_directory_and_rejects_a_file() {
        let temp = tempfile::tempdir().unwrap();
        sync_directory(temp.path()).unwrap();
        let file = temp.path().join("file");
        std::fs::write(&file, b"data").unwrap();
        assert!(sync_directory(&file).is_err());
    }

    #[test]
    fn wrong_path_is_quarantined_before_duplicates_and_canonical_copy_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        let event = (0..100)
            .map(|index| valid_event_for_device(&format!("device-{index}"), 1, Vec::new()))
            .find(|event| &event.event_id.as_str()[..2] != "00")
            .unwrap();
        let wrong = store
            .events_dir()
            .join("00")
            .join(format!("{}.json", event.event_id));
        let canonical = canonical_path(&store, &event);
        write_event(&wrong, &event);
        write_event(&canonical, &event);

        store.initialize().unwrap();

        assert_eq!(store.ordered_events().unwrap(), vec![event]);
        assert!(canonical.exists());
        assert!(!wrong.exists());
        assert_eq!(
            std::fs::read_dir(store.quarantine_dir()).unwrap().count(),
            1
        );
    }

    #[test]
    fn unknown_bytes_under_an_unchanged_event_id_are_quarantined() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        let event = valid_event(1, Vec::new());
        let mut json = serde_json::to_value(&event).unwrap();
        json["payload"]["data"]["api_key"] = serde_json::json!("must-not-pass");
        let path = canonical_path(&store, &event);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();

        store.initialize().unwrap();

        assert!(store.ordered_events().unwrap().is_empty());
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_dir(store.quarantine_dir()).unwrap().count(),
            1
        );
    }

    #[test]
    fn missing_parent_quarantine_is_transitively_closed_in_one_initialization() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        let missing =
            EventId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .unwrap();
        let child = valid_event_for_device("bad-child", 1, vec![missing]);
        let grandchild = valid_event_for_device("bad-grandchild", 1, vec![child.event_id.clone()]);
        let clean = valid_event_for_device("clean-sibling", 1, Vec::new());
        for event in [&child, &grandchild, &clean] {
            write_event(&canonical_path(&store, event), event);
        }

        store.initialize().unwrap();

        assert_eq!(store.ordered_events().unwrap(), vec![clean]);
        assert_eq!(
            std::fs::read_dir(store.quarantine_dir()).unwrap().count(),
            2
        );
        assert!(!canonical_path(&store, &child).exists());
        assert!(!canonical_path(&store, &grandchild).exists());
    }

    #[test]
    fn real_store_components_initialize_and_append_normally() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let event = valid_event(1, Vec::new());
        store.append(event.clone()).unwrap();
        assert!(canonical_path(&store, &event).is_file());
    }

    #[test]
    fn symlinked_twin_component_fails_initialization_closed() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        if !try_symlink_dir(outside.path(), &temp.path().join("twin")) {
            return;
        }
        let store = TwinEventStore::new(temp.path());
        assert!(store.initialize().is_err());
        assert!(!outside.path().join("events").exists());
    }

    #[test]
    fn symlinked_prefix_cannot_redirect_an_append() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let event = valid_event(1, Vec::new());
        let prefix = store.events_dir().join(&event.event_id.as_str()[..2]);
        if !try_symlink_dir(outside.path(), &prefix) {
            return;
        }
        assert!(store.append(event).is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn symlinked_canonical_target_and_encountered_record_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let event = valid_event(1, Vec::new());
        let target = canonical_path(&store, &event);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let external = outside.path().join("event.json");
        write_event(&external, &event);
        if !try_symlink_file(&external, &target) {
            return;
        }
        assert!(store.append(event).is_err());
        let peer = TwinEventStore::new(temp.path());
        assert!(peer.initialize().is_err());
    }

    #[test]
    fn event_replaced_between_enumeration_and_read_is_never_admitted() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let original = valid_event_for_device("original-device", 1, Vec::new());
        store.append(original.clone()).unwrap();
        let target = canonical_path(&store, &original);
        let displaced = target.parent().unwrap().join("enumerated-old.json");
        let replacement = valid_event_for_device("replacement-device", 1, Vec::new());
        let replacement_bytes = serde_json::to_vec_pretty(&replacement).unwrap();
        let hook_target = target.clone();
        store.set_after_enumeration_hook(move || {
            std::fs::rename(&hook_target, &displaced).unwrap();
            std::fs::write(&hook_target, replacement_bytes).unwrap();
        });

        assert!(store.ordered_events().unwrap().is_empty());
        assert!(store.ordered_events().unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(store.quarantine_dir()).unwrap().count(),
            2
        );
        assert!(!target.exists());
    }
}
