use super::*;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::sync::operation_store::{
    MAX_OPERATIONS_PER_AREA, MAX_OPERATION_BYTES_PER_AREA,
};
use grafyn_sync_protocol::MAX_ENVELOPE_JSON_BYTES;
use serde_json::Value;

const BOOTSTRAP_SCHEMA_VERSION: u16 = 1;
const BOOTSTRAP_WITNESS_EXTRA_BYTES: usize = 8 * 1024 * 1024;
const MAX_BOOTSTRAP_WITNESS_BYTES: usize =
    MAX_OPERATION_BYTES_PER_AREA + BOOTSTRAP_WITNESS_EXTRA_BYTES;
const MAX_BOOTSTRAP_COMPLETION_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct PreparedBootstrapV1 {
    schema_version: u16,
    vault_scope: ContentDigest,
    vault_id: String,
    device_id: String,
    base_ledger_policy_digest: ContentDigest,
    inventory_digest: ContentDigest,
    local_only_notes: BTreeMap<String, String>,
    note_paths: BTreeMap<String, String>,
    envelopes: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct BootstrapReceiptV1 {
    operation_id: String,
    envelope_sha256: ContentDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CompletedBootstrapV1 {
    schema_version: u16,
    vault_scope: ContentDigest,
    vault_id: String,
    device_id: String,
    prepared_sha256: ContentDigest,
    operations: Vec<BootstrapReceiptV1>,
}

#[derive(Debug, Clone, Serialize)]
struct InventoryFingerprintV1 {
    schema_version: u16,
    notes: Vec<(String, String, ContentDigest)>,
    events: Vec<String>,
    local_only_notes: BTreeMap<String, String>,
    note_paths: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct LedgerPolicyFingerprintV1<'a> {
    schema_version: u16,
    local_only_notes: &'a BTreeMap<String, String>,
    note_paths: &'a BTreeMap<String, String>,
}

struct BootstrapNote {
    note_id: String,
    relative_key: String,
    markdown: String,
}

struct BootstrapInventory {
    notes: Vec<BootstrapNote>,
    local_only_notes: BTreeMap<String, String>,
    note_paths: BTreeMap<String, String>,
}

impl SyncEngine {
    /// Durably seals a pre-sync vault snapshot without making any outbox entry
    /// visible. A restart can therefore reuse the exact random nonces.
    pub(crate) fn prepare_existing_vault_bootstrap(
        &self,
        coordinator: &MutationCoordinator,
        knowledge_store: &KnowledgeStore,
    ) -> Result<bool, MutationError> {
        let _coordinator_guard = coordinator.begin_root_transition()?;
        let (state, _engine_lock) = self.lock_fresh_state()?;
        self.prepare_existing_vault_bootstrap_locked(knowledge_store, &state, false)
    }

    /// Promotes every operation from the durable bootstrap witness, then
    /// publishes a compact completion receipt and removes the large witness.
    pub(crate) fn finish_existing_vault_bootstrap(
        &self,
        coordinator: &MutationCoordinator,
        knowledge_store: &KnowledgeStore,
    ) -> Result<(), MutationError> {
        let _coordinator_guard = coordinator.begin_root_transition()?;
        let (mut state, _engine_lock) = self.lock_fresh_state()?;
        self.finish_existing_vault_bootstrap_locked(knowledge_store, &mut state)
    }

    pub(crate) fn bootstrap_existing_vault(
        &self,
        coordinator: &MutationCoordinator,
        knowledge_store: &KnowledgeStore,
    ) -> Result<(), MutationError> {
        let _coordinator_guard = coordinator.begin_root_transition()?;
        let (mut state, _engine_lock) = self.lock_fresh_state()?;
        if self.prepare_existing_vault_bootstrap_locked(knowledge_store, &state, true)? {
            self.finish_existing_vault_bootstrap_locked(knowledge_store, &mut state)?;
        }
        Ok(())
    }

    fn prepare_existing_vault_bootstrap_locked(
        &self,
        knowledge_store: &KnowledgeStore,
        state: &EngineState,
        regenerate_drifted: bool,
    ) -> Result<bool, MutationError> {
        let Some(root_key) = state.root_key.as_ref() else {
            return Ok(false);
        };
        let device = state
            .device
            .as_ref()
            .ok_or_else(|| MutationError::Invalid("sync device identity is not attached".into()))?;
        if let Some(completed) = load_completed(&self.data_root, &state.vault_scope)? {
            validate_completed(&state, &completed)?;
            cleanup_completed_prepared(&self.data_root, &state.vault_scope, &completed)?;
            return Ok(false);
        }
        if let Some(prepared) = load_prepared(&self.data_root, &state.vault_scope)? {
            validate_prepared(&state, root_key, &prepared)?;
            let policy_drifted = prepared_policy_has_drifted(state, &prepared)?;
            let inventory_drifted = prepared_inventory_has_drifted(
                knowledge_store,
                state,
                &prepared,
                &self.event_store,
            )?;
            if !policy_drifted && !inventory_drifted {
                return Ok(true);
            }
            if !regenerate_drifted {
                return Err(if policy_drifted {
                    prepared_policy_drift_error()
                } else {
                    prepared_inventory_drift_error()
                });
            }
            self.data_root.delete(&prepared_key(&state.vault_scope))?;
        }

        let inventory =
            scan_markdown_inventory(knowledge_store, &state.vault_scope, &state.ledger)?;
        let prepared = build_prepared(&state, root_key, device, inventory, &self.event_store)?;
        install_prepared(&self.data_root, &state.vault_scope, &prepared)?;
        let durable = load_prepared(&self.data_root, &state.vault_scope)?.ok_or_else(|| {
            MutationError::RecoveryConflict(
                "sync bootstrap witness disappeared after installation".into(),
            )
        })?;
        validate_prepared(&state, root_key, &durable)?;
        ensure_prepared_policy_is_current(state, &durable)?;
        Ok(true)
    }

    fn finish_existing_vault_bootstrap_locked(
        &self,
        knowledge_store: &KnowledgeStore,
        state: &mut EngineState,
    ) -> Result<(), MutationError> {
        let Some(root_key) = state.root_key.as_ref() else {
            return Ok(());
        };
        if let Some(completed) = load_completed(&self.data_root, &state.vault_scope)? {
            validate_completed(&state, &completed)?;
            cleanup_completed_prepared(&self.data_root, &state.vault_scope, &completed)?;
            return rebuild_engine_state(&self.data_root, state);
        }
        let prepared = load_prepared(&self.data_root, &state.vault_scope)?.ok_or_else(|| {
            MutationError::RecoveryConflict("sync bootstrap witness is missing".into())
        })?;
        let envelopes = validate_prepared(&state, root_key, &prepared)?;
        ensure_prepared_policy_is_current(state, &prepared)?;
        ensure_prepared_inventory_is_current(knowledge_store, state, &prepared, &self.event_store)?;

        state.ledger.local_only_notes = prepared.local_only_notes.clone();
        state.ledger.note_paths = prepared.note_paths.clone();
        persist_ledger(
            &self.data_root,
            &engine_ledger_key(&state.vault_scope),
            &state.ledger,
        )?;
        for envelope in &envelopes {
            state
                .operation_store
                .bootstrap_outbox(envelope)
                .map_err(operation_store_error)?;
        }
        let prepared_bytes = serialize_prepared(&prepared)?;
        let completed = completed_from_prepared(&prepared, &prepared_bytes, &envelopes)?;
        install_completed(&self.data_root, &state.vault_scope, &completed)?;
        let durable = load_completed(&self.data_root, &state.vault_scope)?.ok_or_else(|| {
            MutationError::RecoveryConflict(
                "sync bootstrap completion disappeared after installation".into(),
            )
        })?;
        if durable != completed {
            return Err(MutationError::RecoveryConflict(
                "sync bootstrap completion collided with different durable bytes".into(),
            ));
        }
        validate_completed(&state, &durable)?;
        self.data_root.delete(&prepared_key(&state.vault_scope))?;
        rebuild_engine_state(&self.data_root, state)
    }
}

fn scan_markdown_inventory(
    knowledge_store: &KnowledgeStore,
    scope: &ContentDigest,
    ledger: &EngineLedgerV1,
) -> Result<BootstrapInventory, MutationError> {
    let vault_path = knowledge_store.vault_path();
    let root = AnchoredRoot::open(vault_path)?;
    let mut local_only_notes = ledger.local_only_notes.clone();
    let mut note_paths = ledger.note_paths.clone();
    let mut notes = Vec::new();
    let mut current_note_ids = BTreeMap::<String, String>::new();
    let mut markdown_count = 0usize;
    for entry in walkdir::WalkDir::new(vault_path).min_depth(1) {
        let entry = entry.map_err(|error| {
            MutationError::Io(format!("sync bootstrap vault inventory: {error}"))
        })?;
        if entry.file_type().is_symlink() {
            return Err(MutationError::Invalid(format!(
                "symlinked vault entry is not eligible for sync bootstrap: {}",
                entry.path().display()
            )));
        }
        let is_markdown = entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
        if !entry.file_type().is_file() || !is_markdown {
            continue;
        }
        markdown_count = markdown_count.checked_add(1).ok_or_else(|| {
            MutationError::Invalid("sync bootstrap Markdown count overflowed".into())
        })?;
        if markdown_count > MAX_OPERATIONS_PER_AREA {
            return Err(MutationError::Invalid(
                "sync bootstrap Markdown inventory exceeds its operation limit".into(),
            ));
        }
        let relative = entry.path().strip_prefix(vault_path).map_err(|_| {
            MutationError::Invalid("sync bootstrap Markdown escaped the vault".into())
        })?;
        let relative_key = normalize_note_key(&relative.to_string_lossy());
        crate::services::twin_events::validate_target_key(TargetKind::Markdown, &relative_key)?;
        if is_reserved_program(&relative_key) {
            continue;
        }
        let bytes = root
            .read_bounded(
                &relative_key,
                crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES,
            )?
            .ok_or_else(|| {
                MutationError::RecoveryConflict(format!(
                    "sync bootstrap Markdown disappeared: {relative_key}"
                ))
            })?;
        let markdown = match String::from_utf8(bytes) {
            Ok(markdown) => markdown,
            Err(_) => {
                let note_id = local_note_id(scope, ledger, &relative_key, None)?;
                insert_local_only(&mut local_only_notes, &relative_key, note_id)?;
                continue;
            }
        };
        if crate::services::knowledge_store::note_allows_sync(&markdown) {
            let note_id = local_note_id(scope, ledger, &relative_key, Some(&markdown))?;
            if let Some(existing_path) =
                current_note_ids.insert(note_id.clone(), relative_key.clone())
            {
                if existing_path != relative_key {
                    return Err(MutationError::RecoveryConflict(
                        "one sync note identity resolves to multiple local paths".into(),
                    ));
                }
            }
            note_paths.retain(|existing_id, path| existing_id == &note_id || path != &relative_key);
            note_paths.insert(note_id.clone(), relative_key.clone());
            validate_note_paths(&note_paths)?;
            local_only_notes.remove(&relative_key);
            notes.push(BootstrapNote {
                note_id,
                relative_key,
                markdown,
            });
        } else {
            let note_id = local_note_id(scope, ledger, &relative_key, Some(&markdown))?;
            insert_local_only(&mut local_only_notes, &relative_key, note_id)?;
        }
    }
    notes.sort_by(|left, right| left.relative_key.cmp(&right.relative_key));
    validate_note_paths(&note_paths)?;
    Ok(BootstrapInventory {
        notes,
        local_only_notes,
        note_paths,
    })
}

fn insert_local_only(
    policy: &mut BTreeMap<String, String>,
    relative_key: &str,
    note_id: String,
) -> Result<(), MutationError> {
    if policy.len() >= MAX_LOCAL_ONLY_NOTES && !policy.contains_key(relative_key) {
        return Err(MutationError::Invalid(
            "local-only note policy limit exceeded".into(),
        ));
    }
    policy.insert(relative_key.to_owned(), note_id);
    Ok(())
}

fn load_bootstrap_events(event_store: &TwinEventStore) -> Result<Vec<TwinEvent>, MutationError> {
    event_store.ordered_events().map_err(|error| {
        MutationError::Invalid(format!(
            "sync bootstrap Twin event inventory failed: {error}"
        ))
    })
}

fn bootstrap_event_is_eligible(event: &TwinEvent, local_only_notes: &BTreeSet<String>) -> bool {
    event.causal_stream == CausalStream::SyncEligible
        && governance_allows_sync(&event.governance)
        && event
            .context
            .relationships
            .iter()
            .all(|relationship| governance_allows_sync(&relationship.governance))
        && !event_references_local_only_note(event, local_only_notes)
}

fn bootstrap_inventory_digest(
    inventory: &BootstrapInventory,
    events: &[TwinEvent],
) -> Result<ContentDigest, MutationError> {
    let notes = inventory
        .notes
        .iter()
        .map(|note| {
            (
                note.note_id.clone(),
                note.relative_key.clone(),
                crate::services::twin_events::digest_bytes(note.markdown.as_bytes()),
            )
        })
        .collect();
    let local_only_references =
        local_only_references_with_paths(&inventory.local_only_notes, &inventory.note_paths);
    let mut event_ids = events
        .iter()
        .filter(|event| bootstrap_event_is_eligible(event, &local_only_references))
        .map(|event| event.event_id.as_str().to_owned())
        .collect::<Vec<_>>();
    event_ids.sort();
    event_ids.dedup();
    let fingerprint = InventoryFingerprintV1 {
        schema_version: BOOTSTRAP_SCHEMA_VERSION,
        notes,
        events: event_ids,
        local_only_notes: inventory.local_only_notes.clone(),
        note_paths: inventory.note_paths.clone(),
    };
    let bytes = serde_json::to_vec(&fingerprint)
        .map_err(|error| MutationError::Invalid(format!("invalid bootstrap inventory: {error}")))?;
    Ok(crate::services::twin_events::digest_bytes(&bytes))
}

fn current_bootstrap_inventory_digest(
    knowledge_store: &KnowledgeStore,
    state: &EngineState,
    event_store: &TwinEventStore,
) -> Result<ContentDigest, MutationError> {
    let inventory = scan_markdown_inventory(knowledge_store, &state.vault_scope, &state.ledger)?;
    let events = load_bootstrap_events(event_store)?;
    bootstrap_inventory_digest(&inventory, &events)
}

fn prepared_inventory_has_drifted(
    knowledge_store: &KnowledgeStore,
    state: &EngineState,
    prepared: &PreparedBootstrapV1,
    event_store: &TwinEventStore,
) -> Result<bool, MutationError> {
    Ok(
        current_bootstrap_inventory_digest(knowledge_store, state, event_store)?
            != prepared.inventory_digest,
    )
}

fn ensure_prepared_inventory_is_current(
    knowledge_store: &KnowledgeStore,
    state: &EngineState,
    prepared: &PreparedBootstrapV1,
    event_store: &TwinEventStore,
) -> Result<(), MutationError> {
    if prepared_inventory_has_drifted(knowledge_store, state, prepared, event_store)? {
        return Err(prepared_inventory_drift_error());
    }
    Ok(())
}

fn prepared_inventory_drift_error() -> MutationError {
    MutationError::RecoveryConflict(
        "sync bootstrap note or Twin event inventory changed after its witness was prepared".into(),
    )
}

fn build_prepared(
    state: &EngineState,
    root_key: &VaultRootKey,
    device: &DeviceCrypto,
    inventory: BootstrapInventory,
    event_store: &TwinEventStore,
) -> Result<PreparedBootstrapV1, MutationError> {
    let recorded_at_unix_ms = u64::try_from(chrono::Utc::now().timestamp_millis())
        .map_err(|_| MutationError::Invalid("bootstrap timestamp predates Unix time".into()))?;
    let events = load_bootstrap_events(event_store)?;
    let inventory_digest = bootstrap_inventory_digest(&inventory, &events)?;
    let mut envelopes = Vec::new();
    for note in &inventory.notes {
        if note_is_already_represented(state, &note.note_id, &note.markdown) {
            continue;
        }
        let parents = state
            .note_heads
            .get(&note.note_id)
            .map(|heads| heads.iter().copied().collect())
            .unwrap_or_default();
        let revision = NoteRevisionV1::put(note.note_id.clone(), note.markdown.clone())
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
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

    let local_only_references =
        local_only_references_with_paths(&inventory.local_only_notes, &inventory.note_paths);
    let mut event_operations = state.event_operations.clone();
    let mut event_candidates = Vec::new();
    for event in events {
        if event_operations.contains_key(event.event_id.as_str()) {
            continue;
        }
        if !bootstrap_event_is_eligible(&event, &local_only_references) {
            continue;
        }
        if event.device_id.as_str() != device.device_id.to_string() {
            return Err(MutationError::RecoveryConflict(
                "unsynchronized local Twin event belongs to a different signing device".into(),
            ));
        }
        event_candidates.push(event);
    }
    let available_event_ids = event_operations.keys().cloned().collect::<BTreeSet<_>>();
    for event in dependency_order_bootstrap_events(event_candidates, &available_event_ids)? {
        let event_dependencies = event_dependency_ids(&event)?;
        let mut parents = event_dependencies
            .iter()
            .map(|dependency| event_operations.get(dependency.as_str()).copied())
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                MutationError::RecoveryConflict(
                    "dependency-ordered bootstrap event parent is unavailable".into(),
                )
            })?;
        parents.sort();
        parents.dedup();
        let event_json = serde_json::to_string(&event)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let event_id = Digest32::parse_hex(event.event_id.as_str())
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let payload = TwinEventV1::new(event_id, event_json)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let event_recorded_at =
            u64::try_from(event.recorded_at.timestamp_millis()).map_err(|_| {
                MutationError::Invalid("Twin event timestamp predates Unix time".into())
            })?;
        let operation = OperationV1::new(
            event_recorded_at,
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
    if envelopes.len() > MAX_OPERATIONS_PER_AREA {
        return Err(MutationError::Invalid(
            "sync bootstrap operation count exceeds its durable limit".into(),
        ));
    }
    let envelope_values = envelopes
        .iter()
        .map(|envelope| {
            serde_json::to_value(envelope)
                .map_err(|error| MutationError::Invalid(format!("invalid envelope: {error}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PreparedBootstrapV1 {
        schema_version: BOOTSTRAP_SCHEMA_VERSION,
        vault_scope: state.vault_scope.clone(),
        vault_id: state.vault_id.to_string(),
        device_id: device.device_id.to_string(),
        base_ledger_policy_digest: ledger_policy_digest(&state.ledger)?,
        inventory_digest,
        local_only_notes: inventory.local_only_notes,
        note_paths: inventory.note_paths,
        envelopes: envelope_values,
    })
}

fn dependency_order_bootstrap_events(
    events: Vec<TwinEvent>,
    available_event_ids: &BTreeSet<String>,
) -> Result<Vec<TwinEvent>, MutationError> {
    let mut indices = BTreeMap::new();
    for (index, event) in events.iter().enumerate() {
        if indices
            .insert(event.event_id.as_str().to_owned(), index)
            .is_some()
        {
            return Err(MutationError::RecoveryConflict(
                "sync bootstrap Twin event inventory contains a duplicate ID".into(),
            ));
        }
    }

    let mut unresolved_dependencies = vec![0usize; events.len()];
    let mut blocked = vec![false; events.len()];
    let mut dependents = BTreeMap::<String, Vec<usize>>::new();
    for (index, event) in events.iter().enumerate() {
        for dependency in event_dependency_ids(event)? {
            if available_event_ids.contains(dependency.as_str()) {
                continue;
            }
            if indices.contains_key(dependency.as_str()) {
                unresolved_dependencies[index] = unresolved_dependencies[index]
                    .checked_add(1)
                    .ok_or_else(|| {
                        MutationError::Invalid(
                            "sync bootstrap Twin event dependency count overflowed".into(),
                        )
                    })?;
                dependents
                    .entry(dependency.as_str().to_owned())
                    .or_default()
                    .push(index);
            } else {
                blocked[index] = true;
            }
        }
    }

    let mut ready = unresolved_dependencies
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0 && !blocked[index]).then_some(index))
        .collect::<BTreeSet<_>>();
    let mut ordered = Vec::new();
    while let Some(index) = ready.pop_first() {
        let event = events[index].clone();
        if let Some(children) = dependents.get(event.event_id.as_str()) {
            for child in children {
                unresolved_dependencies[*child] -= 1;
                if unresolved_dependencies[*child] == 0 && !blocked[*child] {
                    ready.insert(*child);
                }
            }
        }
        ordered.push(event);
    }
    Ok(ordered)
}

fn note_is_already_represented(state: &EngineState, note_key: &str, markdown: &str) -> bool {
    state
        .note_heads
        .get(note_key)
        .and_then(|heads| choose_note_winner(heads, &state.note_revisions))
        .is_some_and(|(_, revision)| {
            matches!(revision.kind(), NoteRevisionKind::Put { markdown: current } if current == markdown)
        })
}

fn ledger_policy_digest(ledger: &EngineLedgerV1) -> Result<ContentDigest, MutationError> {
    let fingerprint = LedgerPolicyFingerprintV1 {
        schema_version: BOOTSTRAP_SCHEMA_VERSION,
        local_only_notes: &ledger.local_only_notes,
        note_paths: &ledger.note_paths,
    };
    let bytes = serde_json::to_vec(&fingerprint).map_err(|error| {
        MutationError::Invalid(format!("invalid sync bootstrap policy: {error}"))
    })?;
    Ok(crate::services::twin_events::digest_bytes(&bytes))
}

fn prepared_policy_has_drifted(
    state: &EngineState,
    prepared: &PreparedBootstrapV1,
) -> Result<bool, MutationError> {
    let policy_is_prepared = state.ledger.local_only_notes == prepared.local_only_notes
        && state.ledger.note_paths == prepared.note_paths;
    Ok(!policy_is_prepared
        && ledger_policy_digest(&state.ledger)? != prepared.base_ledger_policy_digest)
}

fn ensure_prepared_policy_is_current(
    state: &EngineState,
    prepared: &PreparedBootstrapV1,
) -> Result<(), MutationError> {
    if prepared_policy_has_drifted(state, prepared)? {
        return Err(prepared_policy_drift_error());
    }
    Ok(())
}

fn prepared_policy_drift_error() -> MutationError {
    MutationError::RecoveryConflict(
        "sync bootstrap ledger policy changed after its witness was prepared".into(),
    )
}

fn validate_prepared(
    state: &EngineState,
    root_key: &VaultRootKey,
    prepared: &PreparedBootstrapV1,
) -> Result<Vec<EnvelopeV1>, MutationError> {
    let device = state
        .device
        .as_ref()
        .ok_or_else(|| MutationError::Invalid("sync device identity is not attached".into()))?;
    if prepared.schema_version != BOOTSTRAP_SCHEMA_VERSION
        || prepared.vault_scope != state.vault_scope
        || prepared.vault_id != state.vault_id.to_string()
        || prepared.device_id != device.device_id.to_string()
        || prepared.envelopes.len() > MAX_OPERATIONS_PER_AREA
        || prepared.local_only_notes.len() > MAX_LOCAL_ONLY_NOTES
        || prepared.note_paths.len() > MAX_NOTE_PROJECTIONS
    {
        return Err(MutationError::RecoveryConflict(
            "sync bootstrap witness does not match the active vault/device".into(),
        ));
    }
    for note_key in prepared.local_only_notes.keys() {
        crate::services::twin_events::validate_target_key(TargetKind::Markdown, note_key)?;
    }
    validate_note_paths(&prepared.note_paths)?;
    let local_only =
        local_only_references_with_paths(&prepared.local_only_notes, &prepared.note_paths);
    let mut envelopes = Vec::with_capacity(prepared.envelopes.len());
    let mut total_bytes = 0usize;
    let mut operation_ids = BTreeSet::new();
    let mut known_operations = state.operations.keys().copied().collect::<BTreeSet<_>>();
    let mut event_operations = state.event_operations.clone();
    let mut represented_notes = BTreeSet::new();
    for value in &prepared.envelopes {
        let bytes = serde_json::to_vec(value).map_err(|error| {
            MutationError::Invalid(format!("invalid sync bootstrap envelope: {error}"))
        })?;
        if bytes.len() > MAX_ENVELOPE_JSON_BYTES {
            return Err(MutationError::Invalid(
                "sync bootstrap envelope exceeds its protocol limit".into(),
            ));
        }
        let envelope = EnvelopeV1::from_json_bytes(&bytes)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let canonical = envelope
            .to_json()
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        total_bytes = total_bytes.checked_add(canonical.len()).ok_or_else(|| {
            MutationError::Invalid("sync bootstrap envelope byte count overflowed".into())
        })?;
        if total_bytes > MAX_OPERATION_BYTES_PER_AREA
            || !operation_ids.insert(*envelope.operation_id())
        {
            return Err(MutationError::Invalid(
                "sync bootstrap envelopes violate their durable bounds".into(),
            ));
        }
        let verified = verify_envelope(state, root_key, &envelope)?;
        if verified
            .operation()
            .causal_parents()
            .iter()
            .any(|parent| !known_operations.contains(parent))
        {
            return Err(MutationError::RecoveryConflict(
                "sync bootstrap operation has an unavailable causal parent".into(),
            ));
        }
        match verified.operation().payload() {
            OperationPayloadV1::NoteRevision(revision) => match revision.kind() {
                NoteRevisionKind::Put { markdown }
                    if prepared.note_paths.contains_key(revision.note_id())
                        && represented_notes.insert(revision.note_id().to_owned())
                        && crate::services::knowledge_store::note_allows_sync(markdown)
                        && crate::services::knowledge_store::note_identity_from_markdown(
                            markdown,
                        )
                        .is_none_or(|embedded| embedded == revision.note_id())
                        && !local_only.contains(revision.note_id())
                        && !local_only.contains(&prepared.note_paths[revision.note_id()]) => {}
                _ => {
                    return Err(MutationError::Invalid(
                        "sync bootstrap witness contains a disallowed note".into(),
                    ))
                }
            },
            OperationPayloadV1::TwinEvent(payload) => {
                let event = parse_synced_event(payload, verified.device_id())?;
                if event_references_local_only_note(&event, &local_only) {
                    return Err(MutationError::Invalid(
                        "sync bootstrap event references a local-only note".into(),
                    ));
                }
                let mut parents = event_dependency_ids(&event)?
                    .into_iter()
                    .map(|dependency| event_operations.get(dependency.as_str()).copied())
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| {
                        MutationError::RecoveryConflict(
                            "sync bootstrap event parent is unavailable".into(),
                        )
                    })?;
                parents.sort();
                parents.dedup();
                if parents != verified.operation().causal_parents() {
                    return Err(MutationError::Invalid(
                        "sync bootstrap event parents do not match its operation".into(),
                    ));
                }
                event_operations
                    .insert(event.event_id.as_str().to_owned(), *verified.operation_id());
            }
            OperationPayloadV1::AttachmentManifest(_) | OperationPayloadV1::AttachmentChunk(_) => {
                return Err(MutationError::Invalid(
                    "sync bootstrap witness contains a non-bootstrap payload".into(),
                ))
            }
        }
        known_operations.insert(*verified.operation_id());
        envelopes.push(envelope);
    }
    Ok(envelopes)
}

fn completed_from_prepared(
    prepared: &PreparedBootstrapV1,
    prepared_bytes: &[u8],
    envelopes: &[EnvelopeV1],
) -> Result<CompletedBootstrapV1, MutationError> {
    let mut operations = envelopes
        .iter()
        .map(|envelope| {
            let bytes = envelope
                .to_json()
                .map_err(|error| MutationError::Invalid(error.to_string()))?
                .into_bytes();
            Ok(BootstrapReceiptV1 {
                operation_id: envelope.operation_id().to_string(),
                envelope_sha256: crate::services::twin_events::digest_bytes(&bytes),
            })
        })
        .collect::<Result<Vec<_>, MutationError>>()?;
    operations.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
    Ok(CompletedBootstrapV1 {
        schema_version: BOOTSTRAP_SCHEMA_VERSION,
        vault_scope: prepared.vault_scope.clone(),
        vault_id: prepared.vault_id.clone(),
        device_id: prepared.device_id.clone(),
        prepared_sha256: crate::services::twin_events::digest_bytes(prepared_bytes),
        operations,
    })
}

fn validate_completed(
    state: &EngineState,
    completed: &CompletedBootstrapV1,
) -> Result<(), MutationError> {
    let device = state
        .device
        .as_ref()
        .ok_or_else(|| MutationError::Invalid("sync device identity is not attached".into()))?;
    if completed.schema_version != BOOTSTRAP_SCHEMA_VERSION
        || completed.vault_scope != state.vault_scope
        || completed.vault_id != state.vault_id.to_string()
        || completed.device_id != device.device_id.to_string()
        || completed.operations.len() > MAX_OPERATIONS_PER_AREA
        || completed
            .operations
            .windows(2)
            .any(|pair| pair[0].operation_id >= pair[1].operation_id)
    {
        return Err(MutationError::RecoveryConflict(
            "sync bootstrap completion does not match the active vault/device".into(),
        ));
    }
    for receipt in &completed.operations {
        let operation_id = OperationId::parse_hex(&receipt.operation_id)
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        let durable = state
            .operation_store
            .load(OperationArea::Outbox, &operation_id)
            .map_err(operation_store_error)?
            .ok_or_else(|| {
                MutationError::RecoveryConflict(format!(
                    "completed sync bootstrap operation is missing: {operation_id}"
                ))
            })?;
        if crate::services::twin_events::digest_bytes(durable.bytes()) != receipt.envelope_sha256 {
            return Err(MutationError::RecoveryConflict(format!(
                "completed sync bootstrap operation changed: {operation_id}"
            )));
        }
    }
    Ok(())
}

fn cleanup_completed_prepared(
    root: &AnchoredRoot,
    scope: &ContentDigest,
    completed: &CompletedBootstrapV1,
) -> Result<(), MutationError> {
    if let Some(bytes) = root.read_bounded(&prepared_key(scope), MAX_BOOTSTRAP_WITNESS_BYTES)? {
        if crate::services::twin_events::digest_bytes(&bytes) != completed.prepared_sha256 {
            return Err(MutationError::RecoveryConflict(
                "completed sync bootstrap has a different retained witness".into(),
            ));
        }
        root.delete(&prepared_key(scope))?;
    }
    Ok(())
}

fn load_prepared(
    root: &AnchoredRoot,
    scope: &ContentDigest,
) -> Result<Option<PreparedBootstrapV1>, MutationError> {
    root.read_bounded(&prepared_key(scope), MAX_BOOTSTRAP_WITNESS_BYTES)?
        .map(|bytes| {
            serde_json::from_slice(&bytes).map_err(|error| {
                MutationError::Invalid(format!("invalid sync bootstrap witness: {error}"))
            })
        })
        .transpose()
}

fn load_completed(
    root: &AnchoredRoot,
    scope: &ContentDigest,
) -> Result<Option<CompletedBootstrapV1>, MutationError> {
    root.read_bounded(&completed_key(scope), MAX_BOOTSTRAP_COMPLETION_BYTES)?
        .map(|bytes| {
            serde_json::from_slice(&bytes).map_err(|error| {
                MutationError::Invalid(format!("invalid sync bootstrap completion: {error}"))
            })
        })
        .transpose()
}

fn install_prepared(
    root: &AnchoredRoot,
    scope: &ContentDigest,
    prepared: &PreparedBootstrapV1,
) -> Result<(), MutationError> {
    let bytes = serialize_prepared(prepared)?;
    root.install_no_clobber(&prepared_key(scope), &bootstrap_directory(scope), &bytes)
}

fn install_completed(
    root: &AnchoredRoot,
    scope: &ContentDigest,
    completed: &CompletedBootstrapV1,
) -> Result<(), MutationError> {
    let mut bytes = serde_json::to_vec_pretty(completed).map_err(|error| {
        MutationError::Invalid(format!("invalid sync bootstrap completion: {error}"))
    })?;
    bytes.push(b'\n');
    if bytes.len() > MAX_BOOTSTRAP_COMPLETION_BYTES {
        return Err(MutationError::Invalid(
            "sync bootstrap completion exceeds its storage limit".into(),
        ));
    }
    root.install_no_clobber(&completed_key(scope), &bootstrap_directory(scope), &bytes)
}

fn serialize_prepared(prepared: &PreparedBootstrapV1) -> Result<Vec<u8>, MutationError> {
    let mut bytes = serde_json::to_vec_pretty(prepared).map_err(|error| {
        MutationError::Invalid(format!("invalid sync bootstrap witness: {error}"))
    })?;
    bytes.push(b'\n');
    if bytes.len() > MAX_BOOTSTRAP_WITNESS_BYTES {
        return Err(MutationError::Invalid(
            "sync bootstrap witness exceeds its storage limit".into(),
        ));
    }
    Ok(bytes)
}

fn validate_note_paths(note_paths: &BTreeMap<String, String>) -> Result<(), MutationError> {
    if note_paths.len() > MAX_NOTE_PROJECTIONS {
        return Err(MutationError::Invalid(
            "sync note projection limit exceeded".into(),
        ));
    }
    let mut projected_paths = BTreeSet::new();
    for (note_id, relative_key) in note_paths {
        NoteRevisionV1::tombstone(note_id.clone())
            .map_err(|error| MutationError::Invalid(error.to_string()))?;
        crate::services::twin_events::validate_target_key(TargetKind::Markdown, relative_key)?;
        if is_reserved_program(relative_key) || !projected_paths.insert(relative_key) {
            return Err(MutationError::Invalid(
                "sync note projections contain a reserved or duplicate path".into(),
            ));
        }
    }
    Ok(())
}

fn normalize_note_key(value: &str) -> String {
    value.replace('\\', "/")
}

fn bootstrap_directory(scope: &ContentDigest) -> String {
    format!("sync/vaults/v1/{}/bootstrap/v1", scope.as_str())
}

fn prepared_key(scope: &ContentDigest) -> String {
    format!("{}/prepared.json", bootstrap_directory(scope))
}

fn completed_key(scope: &ContentDigest) -> String {
    format!("{}/completed.json", bootstrap_directory(scope))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin_event::{
        EntityId, EvidenceRef, EvidenceType, Governance, RelationshipAssertion,
        RelationshipDirection, RelationshipPredicate,
    };
    use crate::services::twin_events::{derive_event_id, test_support};

    #[derive(Debug, Clone, Copy)]
    enum SemanticDependency {
        Supersedes,
        Reinforces,
        EventEvidence,
        RelationshipEventEvidence,
    }

    #[test]
    fn bootstrap_event_order_waits_for_every_later_semantic_dependency() {
        for dependency in [
            SemanticDependency::Supersedes,
            SemanticDependency::Reinforces,
            SemanticDependency::EventEvidence,
            SemanticDependency::RelationshipEventEvidence,
        ] {
            let parent = test_support::valid_event_for_device_and_stream(
                "device-a",
                CausalStream::SyncEligible,
                1,
                Vec::new(),
            );
            let mut child = test_support::valid_event_for_device_and_stream(
                "device-a",
                CausalStream::SyncEligible,
                2,
                Vec::new(),
            );
            let event_evidence = || EvidenceRef {
                evidence_type: EvidenceType::Event,
                source_id: crate::models::twin_event::Identifier::parse(
                    parent.event_id.as_str().to_owned(),
                )
                .unwrap(),
                digest: None,
            };
            match dependency {
                SemanticDependency::Supersedes => {
                    child.supersedes.push(parent.event_id.clone());
                }
                SemanticDependency::Reinforces => {
                    child.reinforces.push(parent.event_id.clone());
                }
                SemanticDependency::EventEvidence => child.evidence.push(event_evidence()),
                SemanticDependency::RelationshipEventEvidence => {
                    child.context.relationships.push(RelationshipAssertion {
                        subject_id: EntityId::parse("owner").unwrap(),
                        predicate: RelationshipPredicate::parse("works_with").unwrap(),
                        object_id: EntityId::parse("person-1").unwrap(),
                        direction: RelationshipDirection::Directed,
                        valid_from: None,
                        valid_to: None,
                        evidence: vec![event_evidence()],
                        governance: Governance::direct_observation(),
                    });
                }
            }
            child.normalize();
            child.event_id = derive_event_id(&child);

            let ordered = dependency_order_bootstrap_events(
                vec![child.clone(), parent.clone()],
                &BTreeSet::new(),
            )
            .unwrap();

            assert_eq!(
                ordered
                    .into_iter()
                    .map(|event| event.event_id)
                    .collect::<Vec<_>>(),
                vec![parent.event_id.clone(), child.event_id.clone()],
                "{dependency:?}"
            );
        }
    }
}
