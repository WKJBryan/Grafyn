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

    pub fn initialize(&self) -> Result<(), StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event store lock poisoned".into()))?;
        fs::create_dir_all(self.events_dir()).map_err(|error| {
            StoreError::Io(format!(
                "failed to initialize canonical Twin event directory: {error}"
            ))
        })?;
        fs::create_dir_all(self.quarantine_dir()).map_err(|error| {
            StoreError::Io(format!(
                "failed to initialize Twin event quarantine: {error}"
            ))
        })?;
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
        self.install_no_clobber(&event)?;
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
        let parent = self
            .lock_path()
            .parent()
            .expect("lock path has parent")
            .to_path_buf();
        fs::create_dir_all(parent)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.lock_path())?;
        file.lock_exclusive()?;
        Ok(file)
    }

    fn load_records(&self) -> Result<BTreeMap<EventId, TwinEvent>, StoreError> {
        let mut parsed = Vec::new();
        for entry in WalkDir::new(self.events_dir()).min_depth(1) {
            let entry = entry.map_err(|error| {
                StoreError::Io(format!("failed to read Twin event directory: {error}"))
            })?;
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|v| v.to_str()) != Some("json")
            {
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
        let known: BTreeSet<_> = parsed
            .iter()
            .map(|(_, event)| event.event_id.clone())
            .collect();
        let mut events = BTreeMap::new();
        for (path, event) in parsed {
            if event
                .causal_parents
                .iter()
                .any(|parent| !known.contains(parent))
            {
                self.quarantine(&path)?;
                continue;
            }
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
        fs::create_dir_all(self.quarantine_dir())?;
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
        })
    }

    fn install_no_clobber(&self, event: &TwinEvent) -> Result<(), StoreError> {
        let prefix = &event.event_id.as_str()[..2];
        let directory = self.events_dir().join(prefix);
        fs::create_dir_all(&directory)?;
        let target = directory.join(format!("{}.json", event.event_id));
        let temporary = directory.join(format!(".{}.{}.tmp", event.event_id, Uuid::new_v4()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            let bytes = serde_json::to_vec_pretty(event)
                .map_err(|error| StoreError::Invalid(error.to_string()))?;
            file.write_all(&bytes)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            match fs::hard_link(&temporary, &target) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let existing: TwinEvent =
                        serde_json::from_slice(&fs::read(&target)?).map_err(|parse| {
                            StoreError::Invalid(format!(
                                "existing Twin event is malformed: {parse}"
                            ))
                        })?;
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
        result
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
    use crate::services::twin_events::test_support::{valid_event, valid_event_for_device};

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
}
