use super::{derive_event_id, semantic_bytes};
use crate::models::twin_event::{DeviceId, EventId, TwinEvent};
use fs2::FileExt;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;
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
}

impl TwinEventStore {
    pub fn new(data_path: impl AsRef<Path>) -> Self {
        Self {
            data_path: data_path.as_ref().to_path_buf(),
            state: Mutex::new(StoreState::default()),
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
    fn lock_path(&self) -> PathBuf {
        self.data_path
            .join("twin")
            .join("events")
            .join("append-v1.lock")
    }

    fn canonical_event_path(&self, event_id: &EventId) -> PathBuf {
        self.events_dir()
            .join(&event_id.as_str()[..2])
            .join(format!("{event_id}.json"))
    }

    pub fn initialize(&self) -> Result<(), StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event store lock poisoned".into()))?;
        self.ensure_store_layout()?;
        let lock = self.acquire_process_lock()?;
        let events = self.load_records()?;
        FileExt::unlock(&lock)?;
        state.events = events;
        state.initialized = true;
        Ok(())
    }

    pub fn append(&self, mut event: TwinEvent) -> Result<AppendOutcome, StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event store lock poisoned".into()))?;
        if !state.initialized {
            return Err(StoreError::NotInitialized);
        }
        event.validate().map_err(StoreError::Invalid)?;
        event.normalize();
        let bytes = serialize_record(&event)?;
        let lock = self.acquire_process_lock()?;
        state.events = self.load_records()?;

        if let Some(existing) = state.events.get(&event.event_id) {
            let result = if semantic_bytes(existing) == semantic_bytes(&event) {
                Ok(AppendOutcome::Duplicate)
            } else {
                Err(StoreError::Collision(event.event_id.clone()))
            };
            FileExt::unlock(&lock)?;
            return result;
        }

        let expected = derive_event_id(&event);
        if event.event_id != expected {
            FileExt::unlock(&lock)?;
            return Err(StoreError::WrongEventId {
                supplied: event.event_id,
                expected,
            });
        }
        validate_append_sequence(&state.events, &event)?;
        for parent in &event.causal_parents {
            if !state.events.contains_key(parent) {
                FileExt::unlock(&lock)?;
                return Err(StoreError::MissingParent(parent.clone()));
            }
        }
        self.install_no_clobber(&event, &bytes)?;
        state.events.insert(event.event_id.clone(), event);
        FileExt::unlock(&lock)?;
        Ok(AppendOutcome::Appended)
    }

    pub fn ordered_events(&self) -> Result<Vec<TwinEvent>, StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event store lock poisoned".into()))?;
        if !state.initialized {
            return Err(StoreError::NotInitialized);
        }
        let lock = self.acquire_process_lock()?;
        state.events = self.load_records()?;
        FileExt::unlock(&lock)?;
        topological_order(&state.events.values().cloned().collect::<Vec<_>>())
    }

    fn acquire_process_lock(&self) -> Result<File, StoreError> {
        self.validate_store_layout()?;
        let path = self.lock_path();
        let file = match OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => {
                sync_directory(&self.events_dir())?;
                file
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                validate_real_file(&path, "Twin event process lock")?;
                OpenOptions::new().read(true).write(true).open(&path)?
            }
            Err(error) => return Err(error.into()),
        };
        validate_real_file(&path, "Twin event process lock")?;
        file.lock_exclusive()?;
        Ok(file)
    }

    fn ensure_store_layout(&self) -> Result<(), StoreError> {
        if !self.data_path.exists() {
            fs::create_dir_all(&self.data_path)?;
        }
        let root = fs::metadata(&self.data_path)?;
        if !root.is_dir() {
            return Err(StoreError::Invalid(format!(
                "trusted app-data root is not a directory: {}",
                self.data_path.display()
            )));
        }
        let twin = ensure_real_child_directory(&self.data_path, "twin", true)?;
        let events = ensure_real_child_directory(&twin, "events", false)?;
        ensure_real_child_directory(&events, "v1", false).map_err(|error| {
            StoreError::Io(format!(
                "failed to initialize canonical Twin event directory: {error}"
            ))
        })?;
        let quarantine = ensure_real_child_directory(&events, "quarantine", false)?;
        ensure_real_child_directory(&quarantine, "v1", false).map_err(|error| {
            StoreError::Io(format!(
                "failed to initialize Twin event quarantine: {error}"
            ))
        })?;
        Ok(())
    }

    fn validate_store_layout(&self) -> Result<(), StoreError> {
        let root = fs::metadata(&self.data_path)?;
        if !root.is_dir() {
            return Err(StoreError::Invalid(format!(
                "trusted app-data root is not a directory: {}",
                self.data_path.display()
            )));
        }
        validate_real_directory(&self.data_path.join("twin"), "Twin directory")?;
        validate_real_directory(
            &self.data_path.join("twin").join("events"),
            "Twin events directory",
        )?;
        validate_real_directory(&self.events_dir(), "canonical Twin event directory")?;
        validate_real_directory(
            &self
                .data_path
                .join("twin")
                .join("events")
                .join("quarantine"),
            "Twin event quarantine directory",
        )?;
        validate_real_directory(&self.quarantine_dir(), "Twin event quarantine v1 directory")?;
        Ok(())
    }

    fn load_records(&self) -> Result<BTreeMap<EventId, TwinEvent>, StoreError> {
        self.validate_store_layout()?;
        let mut parsed = Vec::new();
        for entry in WalkDir::new(self.events_dir()).min_depth(1) {
            let entry = entry.map_err(|error| {
                StoreError::Io(format!("failed to read Twin event directory: {error}"))
            })?;
            if entry.file_type().is_symlink() {
                return Err(StoreError::Invalid(format!(
                    "canonical Twin event tree contains a symlink: {}",
                    entry.path().display()
                )));
            }
            if entry.file_type().is_dir() {
                validate_real_directory(entry.path(), "canonical Twin event tree directory")?;
                continue;
            }
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|v| v.to_str()) != Some("json")
            {
                continue;
            }
            validate_real_file(entry.path(), "canonical Twin event record")?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
                StoreError::Io(format!(
                    "failed to inspect Twin event {}: {error}",
                    entry.path().display()
                ))
            })?;
            if validate_record_size(metadata.len()).is_err() {
                self.quarantine(entry.path())?;
                continue;
            }
            let bytes = fs::read(entry.path()).map_err(|error| {
                StoreError::Io(format!(
                    "failed to read Twin event {}: {error}",
                    entry.path().display()
                ))
            })?;
            let event = match serde_json::from_slice::<TwinEvent>(&bytes) {
                Ok(mut event) => {
                    if event.validate().is_err() || derive_event_id(&event) != event.event_id {
                        self.quarantine(entry.path())?;
                        continue;
                    }
                    if entry.path() != self.canonical_event_path(&event.event_id) {
                        self.quarantine(entry.path())?;
                        continue;
                    }
                    event.normalize();
                    event
                }
                Err(_) => {
                    self.quarantine(entry.path())?;
                    continue;
                }
            };
            parsed.push((entry.path().to_path_buf(), event));
        }
        let mut known: BTreeSet<_> = parsed
            .iter()
            .map(|(_, event)| event.event_id.clone())
            .collect();
        loop {
            let invalid: BTreeSet<_> = parsed
                .iter()
                .filter(|(_, event)| {
                    event
                        .causal_parents
                        .iter()
                        .any(|parent| !known.contains(parent))
                })
                .map(|(_, event)| event.event_id.clone())
                .collect();
            if invalid.is_empty() {
                break;
            }
            let mut retained = Vec::with_capacity(parsed.len() - invalid.len());
            for (path, event) in parsed {
                if invalid.contains(&event.event_id) {
                    self.quarantine(&path)?;
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
                self.quarantine(&path)?;
                continue;
            }
            events.insert(event.event_id.clone(), event);
        }
        validate_all_sequences(&events)?;
        topological_order(&events.values().cloned().collect::<Vec<_>>())?;
        Ok(events)
    }

    fn quarantine(&self, source: &Path) -> Result<(), StoreError> {
        self.validate_store_layout()?;
        validate_real_file(source, "quarantined Twin event record")?;
        let name = source
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("event.json");
        let target = self
            .quarantine_dir()
            .join(format!("{}-{name}", Uuid::new_v4()));
        fs::rename(source, &target).map_err(|error| {
            StoreError::Io(format!(
                "failed to quarantine malformed Twin event {}: {error}",
                source.display()
            ))
        })?;
        sync_directory(source.parent().expect("quarantined record has parent"))?;
        sync_directory(&self.quarantine_dir())?;
        Ok(())
    }

    fn install_no_clobber(&self, event: &TwinEvent, bytes: &[u8]) -> Result<(), StoreError> {
        self.validate_store_layout()?;
        let prefix = &event.event_id.as_str()[..2];
        let directory = ensure_real_child_directory(&self.events_dir(), prefix, false)?;
        let target = directory.join(format!("{}.json", event.event_id));
        let temporary = directory.join(format!(".{}.{}.tmp", event.event_id, Uuid::new_v4()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            match fs::hard_link(&temporary, &target) {
                Ok(()) => {
                    sync_directory(&directory)?;
                    Ok(())
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let existing = self.read_existing_canonical(&target, &event.event_id)?;
                    if semantic_bytes(&existing) == semantic_bytes(event) {
                        Ok(())
                    } else {
                        Err(StoreError::Collision(event.event_id.clone()))
                    }
                }
                Err(error) => Err(StoreError::Io(format!(
                    "failed to install immutable Twin event: {error}"
                ))),
            }
        })();
        let cleanup = fs::remove_file(&temporary);
        if let Err(error) = cleanup {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(StoreError::Io(format!(
                    "failed to remove Twin event temporary file: {error}"
                )));
            }
        }
        sync_directory(&directory)?;
        result
    }

    fn read_existing_canonical(
        &self,
        path: &Path,
        expected_id: &EventId,
    ) -> Result<TwinEvent, StoreError> {
        validate_real_file(path, "existing canonical Twin event")?;
        let metadata = fs::symlink_metadata(path)?;
        validate_record_size(metadata.len())?;
        let mut event: TwinEvent = serde_json::from_slice(&fs::read(path)?).map_err(|error| {
            StoreError::Invalid(format!("existing Twin event is malformed: {error}"))
        })?;
        event.validate().map_err(StoreError::Invalid)?;
        if &event.event_id != expected_id
            || derive_event_id(&event) != event.event_id
            || path != self.canonical_event_path(&event.event_id)
        {
            return Err(StoreError::Invalid(
                "existing Twin event is not a canonical record".into(),
            ));
        }
        event.normalize();
        Ok(event)
    }
}

fn serialize_record(event: &TwinEvent) -> Result<Vec<u8>, StoreError> {
    let mut bytes =
        serde_json::to_vec_pretty(event).map_err(|error| StoreError::Invalid(error.to_string()))?;
    bytes.push(b'\n');
    validate_record_size(bytes.len() as u64)?;
    Ok(bytes)
}

fn ensure_real_child_directory(
    parent: &Path,
    name: &str,
    parent_is_trusted_root: bool,
) -> Result<PathBuf, StoreError> {
    if parent_is_trusted_root {
        if !fs::metadata(parent)?.is_dir() {
            return Err(StoreError::Invalid(format!(
                "trusted directory boundary is not a directory: {}",
                parent.display()
            )));
        }
    } else {
        validate_real_directory(parent, "Twin event directory parent")?;
    }
    let path = parent.join(name);
    match fs::symlink_metadata(&path) {
        Ok(_) => validate_real_directory(&path, "Twin event directory component")?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&path)?;
            if parent_is_trusted_root {
                sync_directory_impl(parent, true)?;
            } else {
                sync_directory(parent)?;
            }
            validate_real_directory(&path, "Twin event directory component")?;
        }
        Err(error) => return Err(error.into()),
    }
    Ok(path)
}

fn validate_real_directory(path: &Path, label: &str) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StoreError::Invalid(format!(
            "{label} must be a real directory, not a symlink or other entry: {}",
            path.display()
        )));
    }
    Ok(())
}

fn validate_real_file(path: &Path, label: &str) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StoreError::Invalid(format!(
            "{label} must be a real regular file, not a symlink or other entry: {}",
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

fn sync_directory(path: &Path) -> Result<(), StoreError> {
    sync_directory_impl(path, false)
}

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
        let directory = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        directory.sync_all()?;
        Ok(())
    }
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(StoreError::Io(
            "directory synchronization is unsupported on this platform".into(),
        ))
    }
}

fn validate_append_sequence(
    events: &BTreeMap<EventId, TwinEvent>,
    event: &TwinEvent,
) -> Result<(), StoreError> {
    let same_device: Vec<_> = events
        .values()
        .filter(|item| item.device_id == event.device_id)
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
    let mut by_device: BTreeMap<DeviceId, Vec<&TwinEvent>> = BTreeMap::new();
    for event in events.values() {
        by_device
            .entry(event.device_id.clone())
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

#[allow(dead_code)] // Task 7 injects this seam into mutation-owning services.
pub trait EventRecorder: Send + Sync {
    fn record(&self, event: TwinEvent) -> Result<AppendOutcome, StoreError>;
}

impl EventRecorder for TwinEventStore {
    fn record(&self, event: TwinEvent) -> Result<AppendOutcome, StoreError> {
        self.append(event)
    }
}

#[derive(Debug, Default)]
#[allow(dead_code)] // Compatibility recorder until Task 7 adds capture hooks.
pub struct NoopEventRecorder;
impl EventRecorder for NoopEventRecorder {
    fn record(&self, _event: TwinEvent) -> Result<AppendOutcome, StoreError> {
        Ok(AppendOutcome::Ignored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin_event::{
        BoundedContent, DecisionRecorded, Identifier, TwinEventPayload, MAX_DECISION_OPTIONS,
    };
    use crate::services::twin_events::test_support::{valid_event, valid_event_for_device};

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
}
