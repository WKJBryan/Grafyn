use super::*;

pub(super) fn collect_conflicts(state: &EngineState) -> Vec<SyncConflict> {
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

pub(super) fn reset_logical_state(state: &mut EngineState) {
    state.operations.clear();
    state.note_heads.clear();
    state.note_revisions.clear();
    state.event_operations.clear();
    state.applied.clear();
    state.pending_graph = CausalGraph::new();
}

pub(super) fn engine_ledger_key(scope: &ContentDigest) -> String {
    format!("sync/vaults/v1/{}/engine-ledger-v1.json", scope.as_str())
}

pub(super) fn engine_ledger_lock_key(scope: &ContentDigest) -> String {
    format!("sync/vaults/v1/{}/engine-ledger-v1.lock", scope.as_str())
}

pub(super) fn remote_materialization_lock_key(scope: &ContentDigest) -> String {
    format!(
        "sync/vaults/v1/{}/remote-materialization-v1.lock",
        scope.as_str()
    )
}

pub(super) fn load_ledger(root: &AnchoredRoot, key: &str) -> Result<EngineLedgerV1, MutationError> {
    let Some(bytes) = root.read_bounded(key, ENGINE_LEDGER_LIMIT)? else {
        return Ok(EngineLedgerV1::default());
    };
    let ledger: EngineLedgerV1 = serde_json::from_slice(&bytes)
        .map_err(|error| MutationError::Invalid(format!("invalid sync engine ledger: {error}")))?;
    validate_ledger(&ledger)?;
    Ok(ledger)
}

pub(super) fn persist_ledger(
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

pub(super) fn validate_ledger(ledger: &EngineLedgerV1) -> Result<(), MutationError> {
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

pub(super) fn register_trusted_record(
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

pub(super) fn trusted_devices_from_ledger(
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

pub(super) fn decode_public_key(value: &str) -> Result<DevicePublicKey, MutationError> {
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

pub(super) fn hex_nibble(byte: u8) -> Result<u8, MutationError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(MutationError::Invalid(
            "trusted device public key is invalid".into(),
        )),
    }
}

pub(super) fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

pub(super) fn operation_store_error(error: impl fmt::Display) -> MutationError {
    MutationError::Io(format!("sync operation store: {error}"))
}

pub(super) fn graph_error(error: impl fmt::Display) -> MutationError {
    MutationError::Invalid(format!("sync causal graph: {error}"))
}

pub(super) fn terminal_remote_error(error: &MutationError) -> bool {
    matches!(
        error,
        MutationError::Invalid(_)
            | MutationError::Store(crate::services::twin_events::StoreError::Invalid(_))
    )
}

pub(super) fn conditional_note_conflict(error: &MutationError) -> bool {
    matches!(
        error,
        MutationError::RecoveryConflict(message)
            if message.starts_with("conditional mutation target changed:")
    )
}

pub(super) fn choose_note_winner<'a>(
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
