use super::*;

impl TwinEventStore {
    pub(crate) fn adopt_or_validate_integrity(
        &self,
        applied_event_ids: Option<&BTreeSet<EventId>>,
    ) -> Result<(), StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event store lock poisoned".into()))?;
        if !state.initialized {
            return Err(StoreError::NotInitialized);
        }
        let root = self.root_capability()?;
        let lock = self.acquire_process_lock(&root, &state.namespace)?;
        let result = (|| {
            state.events = self.load_records(&root, &state.namespace)?;
            validate_applied_event_inventory(&state.events, applied_event_ids)?;
            let expected = lane_heads(&state.events)?;
            match self.read_integrity_marker(&root, &state.namespace)? {
                None => {
                    if !self
                        .read_integrity_heads(&root, &state.namespace)?
                        .is_empty()
                    {
                        return Err(StoreError::Invalid(
                            "Twin event integrity adoption marker is missing beside retained heads"
                                .into(),
                        ));
                    }
                    // Applied sync operations expose missing legacy sync events above. A
                    // local-only event deleted before this first durable anchor leaves no
                    // independent evidence and is therefore unknowable at adoption time.
                    self.write_integrity_marker(
                        &root,
                        &state.namespace,
                        EventIntegrityPhaseV1::Adopting,
                        &expected,
                    )?;
                    self.write_integrity_heads(&root, &state.namespace, &expected)?;
                    self.write_integrity_marker(
                        &root,
                        &state.namespace,
                        EventIntegrityPhaseV1::Enabled,
                        &expected,
                    )?;
                }
                Some(marker) if marker.phase == EventIntegrityPhaseV1::Adopting => {
                    if marker.heads != expected {
                        return Err(StoreError::Invalid(
                            "Twin event history changed during integrity adoption".into(),
                        ));
                    }
                    let durable = self.read_integrity_heads(&root, &state.namespace)?;
                    validate_partial_adoption_heads(&durable, &expected)?;
                    self.write_integrity_heads(&root, &state.namespace, &expected)?;
                    self.write_integrity_marker(
                        &root,
                        &state.namespace,
                        EventIntegrityPhaseV1::Enabled,
                        &expected,
                    )?;
                }
                Some(marker) => validate_enabled_integrity(
                    &marker,
                    &self.read_integrity_heads(&root, &state.namespace)?,
                    &expected,
                )?,
            }
            state.integrity_ready = true;
            Ok(())
        })();
        lock.unlock()?;
        result
    }

    pub(crate) fn validate_integrity(
        &self,
        applied_event_ids: Option<&BTreeSet<EventId>>,
    ) -> Result<(), StoreError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Invalid("Twin event store lock poisoned".into()))?;
        if !state.initialized {
            return Err(StoreError::NotInitialized);
        }
        if !state.integrity_ready {
            return Err(StoreError::Invalid(
                "Twin event integrity has not completed adoption".into(),
            ));
        }
        let root = self.root_capability()?;
        let lock = self.acquire_process_lock(&root, &state.namespace)?;
        let result = (|| {
            state.events = self.load_records(&root, &state.namespace)?;
            validate_applied_event_inventory(&state.events, applied_event_ids)?;
            let expected = lane_heads(&state.events)?;
            let marker = self
                .read_integrity_marker(&root, &state.namespace)?
                .ok_or_else(|| {
                    StoreError::Invalid("Twin event integrity adoption marker is missing".into())
                })?;
            validate_enabled_integrity(
                &marker,
                &self.read_integrity_heads(&root, &state.namespace)?,
                &expected,
            )
        })();
        lock.unlock()?;
        result
    }

    pub(crate) fn advance_integrity_heads(
        &self,
        appended_group: &[TwinEvent],
    ) -> Result<(), StoreError> {
        if appended_group.is_empty() {
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
        let lock = self.acquire_process_lock(&root, &state.namespace)?;
        let result = (|| {
            state.events = self.load_records(&root, &state.namespace)?;
            let expected = lane_heads(&state.events)?;
            match self.read_integrity_marker(&root, &state.namespace)? {
                // Legacy recovery may need to append a retained WAL before the
                // lifecycle can expose its applied TwinEvent inventory. Leave
                // adoption to `adopt_or_validate_integrity`, which performs
                // that cross-check first.
                None if !state.integrity_ready => {}
                None => {
                    return Err(StoreError::Invalid(
                        "Twin event integrity adoption marker disappeared".into(),
                    ))
                }
                Some(marker) if marker.phase != EventIntegrityPhaseV1::Enabled => {
                    return Err(StoreError::Invalid(
                        "Twin event integrity adoption is incomplete".into(),
                    ))
                }
                Some(marker) => {
                    validate_integrity_extension(
                        &state.events,
                        &marker.heads,
                        &expected,
                        appended_group,
                    )?;
                    validate_advancing_head_files(
                        &self.read_integrity_heads(&root, &state.namespace)?,
                        &marker.heads,
                        &expected,
                    )?;
                    self.write_integrity_heads(&root, &state.namespace, &expected)?;
                    self.write_integrity_marker(
                        &root,
                        &state.namespace,
                        EventIntegrityPhaseV1::Enabled,
                        &expected,
                    )?;
                }
            }
            Ok(())
        })();
        lock.unlock()?;
        result
    }

    fn read_integrity_marker(
        &self,
        root: &super::super::AnchoredRoot,
        namespace: &EventNamespace,
    ) -> Result<Option<EventIntegrityMarkerV1>, StoreError> {
        let Some(bytes) = root
            .read_bounded(
                &namespace.integrity_marker_key(),
                EVENT_INTEGRITY_MARKER_LIMIT,
            )
            .map_err(store_capability_error)?
        else {
            return Ok(None);
        };
        let marker: EventIntegrityMarkerV1 = serde_json::from_slice(&bytes).map_err(|error| {
            StoreError::Invalid(format!("invalid Twin event integrity marker: {error}"))
        })?;
        validate_integrity_marker(&marker)?;
        Ok(Some(marker))
    }

    fn read_integrity_heads(
        &self,
        root: &super::super::AnchoredRoot,
        namespace: &EventNamespace,
    ) -> Result<BTreeMap<EventLaneIdentityV1, EventLaneHeadV1>, StoreError> {
        let directory = namespace.integrity_heads_dir();
        let mut heads = BTreeMap::new();
        for (name, kind) in root
            .directory_entries(&directory)
            .map_err(store_capability_error)?
        {
            match kind {
                super::super::AnchoredEntryKind::Directory => {
                    return Err(StoreError::Invalid(
                        "Twin event integrity heads directory contains a nested directory".into(),
                    ))
                }
                super::super::AnchoredEntryKind::File if name.ends_with(".json") => {}
                super::super::AnchoredEntryKind::File => continue,
            }
            let key = format!("{directory}/{name}");
            let bytes = root
                .read_bounded(&key, EVENT_LANE_HEAD_LIMIT)
                .map_err(store_capability_error)?
                .ok_or_else(|| {
                    StoreError::Invalid("Twin event integrity head disappeared".into())
                })?;
            let head: EventLaneHeadV1 = serde_json::from_slice(&bytes).map_err(|error| {
                StoreError::Invalid(format!("invalid Twin event integrity head: {error}"))
            })?;
            validate_lane_head(&head)?;
            if name != integrity_head_filename(&head.lane) {
                return Err(StoreError::Invalid(
                    "Twin event integrity head occupies a noncanonical key".into(),
                ));
            }
            if heads.insert(head.lane.clone(), head).is_some() {
                return Err(StoreError::Invalid(
                    "duplicate Twin event integrity lane head".into(),
                ));
            }
        }
        Ok(heads)
    }

    fn write_integrity_heads(
        &self,
        root: &super::super::AnchoredRoot,
        namespace: &EventNamespace,
        heads: &[EventLaneHeadV1],
    ) -> Result<(), StoreError> {
        for head in heads {
            let bytes =
                serde_json::to_vec(head).map_err(|error| StoreError::Invalid(error.to_string()))?;
            if bytes.len() > EVENT_LANE_HEAD_LIMIT {
                return Err(StoreError::Invalid(
                    "Twin event integrity head exceeds its storage limit".into(),
                ));
            }
            root.put_atomic(
                &format!(
                    "{}/{}",
                    namespace.integrity_heads_dir(),
                    integrity_head_filename(&head.lane)
                ),
                &bytes,
            )
            .map_err(store_capability_error)?;
        }
        Ok(())
    }

    fn write_integrity_marker(
        &self,
        root: &super::super::AnchoredRoot,
        namespace: &EventNamespace,
        phase: EventIntegrityPhaseV1,
        heads: &[EventLaneHeadV1],
    ) -> Result<(), StoreError> {
        let marker = EventIntegrityMarkerV1 {
            schema_version: 1,
            phase,
            heads: heads.to_vec(),
        };
        validate_integrity_marker(&marker)?;
        let bytes =
            serde_json::to_vec(&marker).map_err(|error| StoreError::Invalid(error.to_string()))?;
        if bytes.len() > EVENT_INTEGRITY_MARKER_LIMIT {
            return Err(StoreError::Invalid(
                "Twin event integrity marker exceeds its storage limit".into(),
            ));
        }
        root.put_atomic(&namespace.integrity_marker_key(), &bytes)
            .map_err(store_capability_error)
    }
}

fn lane_heads(events: &BTreeMap<EventId, TwinEvent>) -> Result<Vec<EventLaneHeadV1>, StoreError> {
    let mut heads = BTreeMap::<EventLaneIdentityV1, EventLaneHeadV1>::new();
    for event in events.values() {
        let lane = EventLaneIdentityV1 {
            device_id: event.device_id.clone(),
            causal_stream: event.causal_stream,
        };
        let candidate = EventLaneHeadV1 {
            schema_version: 1,
            lane: lane.clone(),
            device_sequence: event.device_sequence,
            event_id: event.event_id.clone(),
        };
        if heads
            .get(&lane)
            .is_none_or(|current| current.device_sequence < candidate.device_sequence)
        {
            heads.insert(lane, candidate);
        }
    }
    if heads.len() > MAX_EVENT_INTEGRITY_LANES {
        return Err(StoreError::Invalid(format!(
            "Twin event integrity exceeds its {MAX_EVENT_INTEGRITY_LANES}-lane limit"
        )));
    }
    Ok(heads.into_values().collect())
}

fn validate_lane_head(head: &EventLaneHeadV1) -> Result<(), StoreError> {
    if head.schema_version != 1 || head.device_sequence == 0 {
        return Err(StoreError::Invalid(
            "Twin event integrity head has an unsupported schema or sequence".into(),
        ));
    }
    Ok(())
}

fn validate_integrity_marker(marker: &EventIntegrityMarkerV1) -> Result<(), StoreError> {
    if marker.schema_version != 1 || marker.heads.len() > MAX_EVENT_INTEGRITY_LANES {
        return Err(StoreError::Invalid(
            "Twin event integrity marker has an unsupported schema or lane count".into(),
        ));
    }
    let mut previous = None;
    for head in &marker.heads {
        validate_lane_head(head)?;
        if previous.as_ref().is_some_and(|lane| lane >= &head.lane) {
            return Err(StoreError::Invalid(
                "Twin event integrity marker lanes must be sorted and unique".into(),
            ));
        }
        previous = Some(head.lane.clone());
    }
    Ok(())
}

fn validate_applied_event_inventory(
    events: &BTreeMap<EventId, TwinEvent>,
    applied_event_ids: Option<&BTreeSet<EventId>>,
) -> Result<(), StoreError> {
    let Some(applied_event_ids) = applied_event_ids else {
        return Ok(());
    };
    if let Some(missing) = applied_event_ids
        .iter()
        .find(|event_id| !events.contains_key(*event_id))
    {
        return Err(StoreError::Invalid(format!(
            "applied Twin event operation has no canonical event record: {missing}"
        )));
    }
    Ok(())
}

fn integrity_head_filename(lane: &EventLaneIdentityV1) -> String {
    let mut bytes = b"grafyn.twin-event-lane-head.v1\0".to_vec();
    bytes.extend_from_slice(&(lane.device_id.as_str().len() as u64).to_be_bytes());
    bytes.extend_from_slice(lane.device_id.as_str().as_bytes());
    bytes.push(match lane.causal_stream {
        CausalStream::LocalOnly => 0,
        CausalStream::SyncEligible => 1,
    });
    format!("{}.json", super::super::digest_bytes(&bytes).as_str())
}

fn heads_by_lane(heads: &[EventLaneHeadV1]) -> BTreeMap<EventLaneIdentityV1, EventLaneHeadV1> {
    heads
        .iter()
        .cloned()
        .map(|head| (head.lane.clone(), head))
        .collect()
}

fn validate_partial_adoption_heads(
    durable: &BTreeMap<EventLaneIdentityV1, EventLaneHeadV1>,
    expected: &[EventLaneHeadV1],
) -> Result<(), StoreError> {
    let expected = heads_by_lane(expected);
    if durable
        .iter()
        .any(|(lane, head)| expected.get(lane) != Some(head))
    {
        return Err(StoreError::Invalid(
            "Twin event integrity adoption contains an unexpected durable head".into(),
        ));
    }
    Ok(())
}

fn validate_enabled_integrity(
    marker: &EventIntegrityMarkerV1,
    durable: &BTreeMap<EventLaneIdentityV1, EventLaneHeadV1>,
    expected: &[EventLaneHeadV1],
) -> Result<(), StoreError> {
    if marker.phase != EventIntegrityPhaseV1::Enabled {
        return Err(StoreError::Invalid(
            "Twin event integrity adoption did not reach enabled state".into(),
        ));
    }
    let marker_heads = heads_by_lane(&marker.heads);
    let expected_heads = heads_by_lane(expected);
    if marker_heads != expected_heads || durable != &expected_heads {
        return Err(StoreError::Invalid(
            "Twin event history does not match its durable lane heads".into(),
        ));
    }
    Ok(())
}

fn validate_integrity_extension(
    events: &BTreeMap<EventId, TwinEvent>,
    previous: &[EventLaneHeadV1],
    current: &[EventLaneHeadV1],
    appended_group: &[TwinEvent],
) -> Result<(), StoreError> {
    let previous = heads_by_lane(previous);
    let current = heads_by_lane(current);
    let appended = appended_group
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<BTreeSet<_>>();
    for (lane, prior) in &previous {
        let durable = events.get(&prior.event_id).ok_or_else(|| {
            StoreError::Invalid("previous Twin event integrity head disappeared".into())
        })?;
        if durable.device_id != lane.device_id
            || durable.causal_stream != lane.causal_stream
            || durable.device_sequence != prior.device_sequence
            || current
                .get(lane)
                .is_none_or(|head| head.device_sequence < prior.device_sequence)
        {
            return Err(StoreError::Invalid(
                "previous Twin event integrity head changed or moved backwards".into(),
            ));
        }
    }
    for event in events.values() {
        let lane = EventLaneIdentityV1 {
            device_id: event.device_id.clone(),
            causal_stream: event.causal_stream,
        };
        let prior_sequence = previous.get(&lane).map_or(0, |head| head.device_sequence);
        if event.device_sequence > prior_sequence && !appended.contains(&event.event_id) {
            return Err(StoreError::Invalid(
                "Twin event lane advanced outside the retained mutation group".into(),
            ));
        }
    }
    if appended
        .iter()
        .any(|event_id| !events.contains_key(event_id))
    {
        return Err(StoreError::Invalid(
            "retained mutation event disappeared before head advancement".into(),
        ));
    }
    Ok(())
}

fn validate_advancing_head_files(
    durable: &BTreeMap<EventLaneIdentityV1, EventLaneHeadV1>,
    previous: &[EventLaneHeadV1],
    current: &[EventLaneHeadV1],
) -> Result<(), StoreError> {
    let previous = heads_by_lane(previous);
    let current = heads_by_lane(current);
    for (lane, head) in durable {
        if previous.get(lane) != Some(head) && current.get(lane) != Some(head) {
            return Err(StoreError::Invalid(
                "durable Twin event lane head changed outside WAL recovery".into(),
            ));
        }
    }
    for (lane, prior) in &previous {
        if durable
            .get(lane)
            .is_none_or(|head| head != prior && current.get(lane) != Some(head))
        {
            return Err(StoreError::Invalid(
                "previous durable Twin event lane head disappeared".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::twin_events::test_support::{
        valid_event_for_device, valid_event_for_device_and_stream,
    };

    fn canonical_path(store: &TwinEventStore, event: &TwinEvent) -> std::path::PathBuf {
        store
            .events_dir()
            .join(&event.event_id.as_str()[..2])
            .join(format!("{}.json", event.event_id))
    }

    fn marker_path(root: &std::path::Path) -> std::path::PathBuf {
        root.join("twin/events/integrity/v1/adoption.json")
    }

    #[test]
    fn enabled_heads_detect_lone_and_tail_event_deletion() {
        for tail_only in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let store = TwinEventStore::new(temp.path());
            store.initialize().unwrap();
            let first = valid_event_for_device_and_stream(
                "integrity-device",
                CausalStream::LocalOnly,
                1,
                Vec::new(),
            );
            store.append(first.clone()).unwrap();
            let mut events = vec![first.clone()];
            if tail_only {
                let second = valid_event_for_device_and_stream(
                    "integrity-device",
                    CausalStream::LocalOnly,
                    2,
                    vec![first.event_id.clone()],
                );
                store.append(second.clone()).unwrap();
                events.push(second);
            }
            store.adopt_or_validate_integrity(None).unwrap();
            let marker_before = std::fs::read(marker_path(temp.path())).unwrap();

            std::fs::remove_file(canonical_path(&store, events.last().unwrap())).unwrap();

            assert!(matches!(
                store.validate_integrity(None),
                Err(StoreError::Invalid(message))
                    if message.contains("durable lane heads")
            ));
            assert_eq!(
                std::fs::read(marker_path(temp.path())).unwrap(),
                marker_before,
                "a failed audit must not rewrite its durable anchor"
            );
        }
    }

    #[test]
    fn canonical_corruption_is_quarantined_once_and_remains_a_startup_error() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let event = valid_event_for_device("corrupt-device", 1, Vec::new());
        store.append(event.clone()).unwrap();
        store.adopt_or_validate_integrity(None).unwrap();
        let marker_before = std::fs::read(marker_path(temp.path())).unwrap();
        std::fs::write(canonical_path(&store, &event), b"{not canonical json").unwrap();

        let reopened = TwinEventStore::new(temp.path());
        assert!(matches!(
            reopened.initialize(),
            Err(StoreError::Invalid(message)) if message.contains("canonical Twin event")
        ));
        assert_eq!(
            std::fs::read_dir(reopened.quarantine_dir())
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            std::fs::read(marker_path(temp.path())).unwrap(),
            marker_before
        );

        let retried = TwinEventStore::new(temp.path());
        assert!(matches!(
            retried.initialize(),
            Err(StoreError::Invalid(message)) if message.contains("explicit recovery")
        ));
    }

    #[test]
    fn legacy_adoption_rejects_an_applied_operation_whose_event_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let store = TwinEventStore::new(temp.path());
        store.initialize().unwrap();
        let missing = valid_event_for_device("missing-applied-device", 1, Vec::new()).event_id;
        let applied = BTreeSet::from([missing]);

        assert!(matches!(
            store.adopt_or_validate_integrity(Some(&applied)),
            Err(StoreError::Invalid(message))
                if message.contains("applied Twin event operation")
        ));
        assert!(!marker_path(temp.path()).exists());
    }
}
