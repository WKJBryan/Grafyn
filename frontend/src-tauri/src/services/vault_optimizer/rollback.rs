use super::*;
use crate::models::twin_event::{ContentDigest, Governance};
use crate::services::twin_events::{BeforeImage, TargetKind};

pub(super) const EXACT_OPTIMIZER_ROLLBACK_SCHEMA_VERSION: u16 = 1;
pub(super) const PENDING_ROLLBACKS_DIRECTORY: &str = "pending-rollbacks-v1";
pub(super) const MAX_PENDING_ROLLBACKS: usize = 64;
const MAX_PENDING_ROLLBACK_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum OptimizerRollbackPhaseV1 {
    RetryFenced,
    Prepared,
    Aborted,
    Committed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingOptimizerRollbackV1 {
    schema_version: u16,
    phase: OptimizerRollbackPhaseV1,
    rollback_id: String,
    change_id: String,
    change_digest: ContentDigest,
    note_id: String,
    requested_at: DateTime<Utc>,
    expected_authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
    material: ExactOptimizerRollbackMaterialV1,
    intent: Option<crate::services::twin_events::MutationIntentV1>,
    mutation_id: Option<ContentDigest>,
    committed_authority: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    audit_written: bool,
    counted: bool,
    proof_consumed: bool,
}

/// Immutable byte-level evidence captured with a new optimizer change.
/// Legacy change records intentionally deserialize this field as `None` and
/// remain audit-only: parsed `Note`/`Value` fields cannot prove byte-exact
/// restoration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExactOptimizerRollbackMaterialV1 {
    pub(super) schema_version: u16,
    pub(super) target_kind: TargetKind,
    pub(super) target_key: String,
    pub(super) restore_before: BeforeImage,
    pub(super) restore_utf8: Option<String>,
    pub(super) apply_after: BeforeImage,
    pub(super) source_relative_path: String,
    pub(super) source_digest: ContentDigest,
    pub(super) apply_payload_digest: ContentDigest,
    pub(super) apply_evidence_digest: ContentDigest,
    pub(super) rollback_payload_digest: ContentDigest,
    pub(super) rollback_evidence_digest: ContentDigest,
    pub(super) apply_governance: Governance,
    pub(super) rollback_governance: Governance,
}

pub(super) fn effective_sidecar_digest(
    markdown_digest: &ContentDigest,
    overlay: &BeforeImage,
) -> ContentDigest {
    let mut bytes = b"grafyn.optimizer.effective-note-state.v1\0".to_vec();
    bytes.extend_from_slice(markdown_digest.as_str().as_bytes());
    bytes.push(0);
    match overlay {
        BeforeImage::Absent => bytes.extend_from_slice(b"absent"),
        BeforeImage::Sha256(digest) => {
            bytes.extend_from_slice(b"sha256\0");
            bytes.extend_from_slice(digest.as_str().as_bytes());
        }
    }
    crate::services::twin_events::digest_bytes(&bytes)
}

impl ExactOptimizerRollbackMaterialV1 {
    pub(super) fn validate(&self, change: &OptimizerChange) -> Result<()> {
        if self.schema_version != EXACT_OPTIMIZER_ROLLBACK_SCHEMA_VERSION {
            anyhow::bail!("unsupported exact optimizer rollback schema");
        }
        if !matches!(
            self.target_kind,
            TargetKind::Markdown | TargetKind::OverlayJson
        ) {
            anyhow::bail!("unsupported exact optimizer rollback target");
        }
        crate::services::twin_events::validate_target_key(self.target_kind, &self.target_key)
            .map_err(anyhow::Error::new)?;
        crate::services::twin_events::validate_target_key(
            TargetKind::Markdown,
            &self.source_relative_path,
        )
        .map_err(anyhow::Error::new)?;
        let restore_digest = self
            .restore_utf8
            .as_ref()
            .map(|raw| crate::services::twin_events::digest_bytes(raw.as_bytes()));
        match (&self.restore_before, restore_digest) {
            (BeforeImage::Absent, None) => {}
            (BeforeImage::Sha256(expected), Some(actual)) if expected == &actual => {}
            _ => anyhow::bail!("exact optimizer rollback before-image is inconsistent"),
        }
        let BeforeImage::Sha256(_) = &self.apply_after else {
            anyhow::bail!("optimizer apply after-image must be present");
        };
        match self.target_kind {
            TargetKind::OverlayJson => {
                if self.target_key != format!("{}.json", change.note_id)
                    || change.mode != "sidecar_first"
                    || change.markdown_relative_path.as_deref()
                        != Some(self.source_relative_path.as_str())
                    || change.markdown_before_digest.as_ref() != Some(&self.source_digest)
                {
                    anyhow::bail!("exact optimizer sidecar rollback binding is inconsistent");
                }
            }
            TargetKind::Markdown => {
                if self.target_key != self.source_relative_path
                    || change.mode != "full_rewrite"
                    || change.markdown_relative_path.as_deref()
                        != Some(self.source_relative_path.as_str())
                    || change.markdown_before_digest.as_ref() != Some(&self.source_digest)
                    || self.restore_before != BeforeImage::Sha256(self.source_digest.clone())
                {
                    anyhow::bail!("exact optimizer Markdown rollback binding is inconsistent");
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}

pub(super) fn initialize_root(root: &crate::services::twin_events::AnchoredRoot) -> Result<()> {
    root.open_directory(PENDING_ROLLBACKS_DIRECTORY, true)
        .map_err(anyhow::Error::new)?;
    Ok(())
}

fn pending_rollback_key(change_id: &str) -> Result<String> {
    parse_canonical_uuid(change_id, "optimizer change ID")?;
    Ok(format!("{PENDING_ROLLBACKS_DIRECTORY}/{change_id}.json"))
}

fn write_pending_rollback(
    root: &crate::services::twin_events::AnchoredRoot,
    pending: &PendingOptimizerRollbackV1,
    change: &OptimizerChange,
) -> Result<()> {
    validate_pending_rollback(pending, change)?;
    let bytes = serde_json::to_vec_pretty(pending)?;
    if bytes.len() > MAX_PENDING_ROLLBACK_BYTES {
        anyhow::bail!("optimizer rollback witness exceeds its 4 MiB limit");
    }
    let names = root
        .regular_file_names_bounded(PENDING_ROLLBACKS_DIRECTORY, MAX_PENDING_ROLLBACKS)
        .map_err(anyhow::Error::new)?;
    let filename = format!("{}.json", pending.change_id);
    if names.len() >= MAX_PENDING_ROLLBACKS && !names.contains(&filename) {
        anyhow::bail!("optimizer rollback witnesses have reached 64 entries");
    }
    root.put_atomic(&pending_rollback_key(&pending.change_id)?, &bytes)
        .map_err(anyhow::Error::new)
}

fn remove_pending_rollback(
    root: &crate::services::twin_events::AnchoredRoot,
    change_id: &str,
) -> Result<()> {
    root.delete(&pending_rollback_key(change_id)?)
        .map_err(anyhow::Error::new)
}

fn load_pending_rollbacks(
    service: &VaultOptimizerService,
) -> Result<Vec<(PendingOptimizerRollbackV1, OptimizerChange)>> {
    let root = service.retained_optimizer_root()?;
    let mut names = root
        .regular_file_names_bounded(PENDING_ROLLBACKS_DIRECTORY, MAX_PENDING_ROLLBACKS)
        .map_err(anyhow::Error::new)?;
    names.sort();
    let mut loaded = Vec::with_capacity(names.len());
    let mut rollback_ids = HashSet::new();
    for name in names {
        let change_id = name
            .strip_suffix(".json")
            .ok_or_else(|| anyhow::anyhow!("invalid optimizer rollback witness filename"))?;
        parse_canonical_uuid(change_id, "optimizer rollback witness change ID")?;
        let key = format!("{PENDING_ROLLBACKS_DIRECTORY}/{name}");
        let bytes = root
            .read_bounded(&key, MAX_PENDING_ROLLBACK_BYTES)
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow::anyhow!("optimizer rollback witness disappeared"))?;
        let pending: PendingOptimizerRollbackV1 =
            serde_json::from_slice(&bytes).context("invalid optimizer rollback witness")?;
        if pending.change_id != change_id || !rollback_ids.insert(pending.rollback_id.clone()) {
            anyhow::bail!("invalid optimizer rollback witness identity");
        }
        let (change, digest) = read_change_with_digest(service, change_id)?;
        if digest != pending.change_digest {
            anyhow::bail!("optimizer rollback change evidence changed");
        }
        validate_pending_rollback(&pending, &change)?;
        loaded.push((pending, change));
    }
    Ok(loaded)
}

pub(super) fn validate_pending_rollbacks(service: &VaultOptimizerService) -> Result<()> {
    load_pending_rollbacks(service).map(|_| ())
}

fn read_change_with_digest(
    service: &VaultOptimizerService,
    change_id: &str,
) -> Result<(OptimizerChange, ContentDigest)> {
    parse_canonical_uuid(change_id, "optimizer change ID")?;
    let key = format!("{CHANGES_DIRECTORY}/{change_id}.json");
    let bytes = service
        .retained_optimizer_root()?
        .read_bounded(&key, MAX_OPTIMIZER_CHANGE_BYTES)
        .map_err(anyhow::Error::new)?
        .ok_or_else(|| anyhow::anyhow!("optimizer change does not exist"))?;
    let digest = crate::services::twin_events::digest_bytes(&bytes);
    let change: OptimizerChange =
        serde_json::from_slice(&bytes).context("invalid optimizer change audit")?;
    if change.change_id != change_id {
        anyhow::bail!("optimizer change identity mismatch");
    }
    let material = change
        .exact_rollback
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("legacy optimizer changes are audit-only"))?;
    material.validate(&change)?;
    Ok((change, digest))
}

fn restore_desired(
    material: &ExactOptimizerRollbackMaterialV1,
) -> crate::services::twin_events::DesiredImage {
    material.restore_utf8.as_ref().map_or(
        crate::services::twin_events::DesiredImage::Tombstone,
        |raw| crate::services::twin_events::DesiredImage::Utf8Bytes(raw.clone()),
    )
}

fn validate_pending_rollback(
    pending: &PendingOptimizerRollbackV1,
    change: &OptimizerChange,
) -> Result<()> {
    if pending.schema_version != 1
        || pending.change_id != change.change_id
        || pending.note_id != change.note_id
    {
        anyhow::bail!("invalid optimizer rollback witness identity");
    }
    parse_canonical_uuid(&pending.rollback_id, "optimizer rollback ID")?;
    parse_canonical_uuid(&pending.change_id, "optimizer change ID")?;
    pending.material.validate(change)?;
    if change.exact_rollback.as_ref() != Some(&pending.material) {
        anyhow::bail!("optimizer rollback witness material changed");
    }
    let lease = Uuid::parse_str(&pending.expected_authority.lease_epoch_uuid)
        .context("invalid optimizer rollback authority lease")?;
    if lease.to_string() != pending.expected_authority.lease_epoch_uuid {
        anyhow::bail!("noncanonical optimizer rollback authority lease");
    }

    match pending.phase {
        OptimizerRollbackPhaseV1::RetryFenced => {
            if pending.intent.is_some()
                || pending.mutation_id.is_some()
                || pending.committed_authority.is_some()
                || pending.audit_written
                || pending.counted
                || pending.proof_consumed
            {
                anyhow::bail!("invalid RetryFenced optimizer rollback witness");
            }
        }
        OptimizerRollbackPhaseV1::Prepared => {
            if pending.intent.is_none()
                || pending.mutation_id.is_none()
                || pending.committed_authority.is_some()
                || pending.audit_written
                || pending.counted
                || pending.proof_consumed
            {
                anyhow::bail!("invalid Prepared optimizer rollback witness");
            }
        }
        OptimizerRollbackPhaseV1::Aborted => {
            if pending.intent.is_none()
                || pending.mutation_id.is_none()
                || pending.audit_written
                || pending.counted
            {
                anyhow::bail!("invalid Aborted optimizer rollback witness");
            }
        }
        OptimizerRollbackPhaseV1::Committed => {
            if pending.intent.is_none()
                || pending.mutation_id.is_none()
                || pending.committed_authority.is_none()
                || pending.counted && !pending.audit_written
                || pending.proof_consumed && !pending.counted
            {
                anyhow::bail!("invalid Committed optimizer rollback witness");
            }
        }
    }

    if let Some(committed) = pending.committed_authority.as_ref() {
        if committed.root_scope != pending.expected_authority.root_scope
            || committed.lease_epoch_uuid != pending.expected_authority.lease_epoch_uuid
            || Some(committed.authority_generation)
                != pending
                    .expected_authority
                    .authority_generation
                    .checked_add(1)
        {
            anyhow::bail!("optimizer rollback committed authority is inconsistent");
        }
    }

    if let Some(intent) = pending.intent.as_ref() {
        intent.validate().map_err(anyhow::Error::new)?;
        let intended_generation = pending
            .expected_authority
            .authority_generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("optimizer rollback authority exhausted"))?;
        if intent.schema_version != 3
            || !intent.retain_commit_receipt
            || intent.origin != crate::services::twin_events::MutationOrigin::Local
            || intent.source_channel.as_str() != "vault_optimizer"
            || intent.markdown_root_scope.as_ref() != Some(&pending.expected_authority.root_scope)
            || intent.content_authority_generation != Some(intended_generation)
            || pending.mutation_id.as_ref() != Some(&intent.mutation_id)
        {
            anyhow::bail!("optimizer rollback finalized intent identity is inconsistent");
        }
        validate_rollback_intent(pending, intent)?;
    }
    Ok(())
}

fn validate_rollback_intent(
    pending: &PendingOptimizerRollbackV1,
    intent: &crate::services::twin_events::MutationIntentV1,
) -> Result<()> {
    let material = &pending.material;
    let desired = restore_desired(material);
    let desired_digest = crate::services::twin_events::desired_digest(&desired);
    let writable_matches = |target: &crate::services::twin_events::MutationTargetV1| {
        target.kind == material.target_kind
            && target.relative_key == material.target_key
            && target.before == material.apply_after
            && target.after == desired
            && target.after_digest == desired_digest
    };
    let target_binding = match material.target_kind {
        TargetKind::Markdown => {
            intent.targets.len() == 1 && intent.targets.iter().all(writable_matches)
        }
        TargetKind::OverlayJson => {
            intent.targets.len() == 2
                && intent.targets.iter().any(writable_matches)
                && intent.targets.iter().any(|target| {
                    target.kind == TargetKind::Markdown
                        && target.relative_key == material.source_relative_path
                        && target.before == BeforeImage::Sha256(material.source_digest.clone())
                        && target.after_digest == material.source_digest
                        && matches!(
                            &target.after,
                            crate::services::twin_events::DesiredImage::Utf8Bytes(_)
                        )
                })
        }
        _ => false,
    };
    if !target_binding || intent.events.len() != 1 {
        anyhow::bail!("optimizer rollback intent target/event binding is inconsistent");
    }
    let event = &intent.events[0];
    let payload_matches = matches!(
        &event.payload,
        crate::models::twin_event::TwinEventPayload::NoteChanged(changed)
            if changed.note_id.as_str() == pending.note_id
                && changed.change == crate::models::twin_event::NoteChangeKind::Updated
                && changed.content_digest.as_ref()
                    == Some(&material.rollback_payload_digest)
    );
    let evidence_matches = event.evidence.len() == 1
        && event.evidence[0].source_id.as_str() == pending.note_id
        && event.evidence[0].digest.as_ref() == Some(&material.rollback_evidence_digest);
    if !payload_matches
        || !evidence_matches
        || event.observed_at != pending.requested_at
        || event.context.source_channel.as_str() != "vault_optimizer"
        || event.governance != material.rollback_governance
    {
        anyhow::bail!("optimizer rollback intent governed event binding is inconsistent");
    }
    Ok(())
}

enum StagedRollback {
    AlreadyRolledBack(crate::models::migration::VaultOptimizerRollbackResult),
    Pending(PendingOptimizerRollbackV1, OptimizerChange),
}

enum RecoveredRollback {
    Committed(crate::services::twin_events::MutationCommit),
    AbortedBeforeAuthority,
    AbortedAfterAuthority(crate::services::twin_events::MutationCommit),
}

pub(super) fn rollback_change(
    service: &mut VaultOptimizerService,
    change_id: &str,
    store: &mut KnowledgeStore,
    expected_authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> Result<OptimizerRollbackMutationOutcome> {
    parse_canonical_uuid(change_id, "optimizer change ID")?;
    if store.current_migration_authority()? != expected_authority {
        anyhow::bail!("root authority changed before optimizer rollback");
    }

    let staged = service.with_locked_fresh_state(|service| {
        stage_or_adopt_rollback(service, change_id, store, &expected_authority)
    })?;
    let StagedRollback::Pending(pending, change) = staged else {
        let StagedRollback::AlreadyRolledBack(result) = staged else {
            unreachable!()
        };
        return Ok(OptimizerRollbackMutationOutcome::NoWrite(result));
    };

    execute_rollback(service, store, pending, change)
}

fn stage_or_adopt_rollback(
    service: &mut VaultOptimizerService,
    change_id: &str,
    store: &KnowledgeStore,
    expected_authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> Result<StagedRollback> {
    let events = service.load_events()?;
    if events.iter().any(|event| {
        matches!(
            event,
            OptimizerAuditEventV1::Rollback {
                change_id: existing,
                ..
            } if existing == change_id
        )
    }) {
        return Ok(StagedRollback::AlreadyRolledBack(rollback_result(
            change_id,
            true,
            "Optimizer change was already rolled back",
            None,
        )));
    }

    if let Some((pending, change)) = load_pending_rollbacks(service)?
        .into_iter()
        .find(|(pending, _)| pending.change_id == change_id)
    {
        if pending.expected_authority.root_scope != expected_authority.root_scope
            || pending.expected_authority.lease_epoch_uuid != expected_authority.lease_epoch_uuid
            || pending.expected_authority.authority_generation
                > expected_authority.authority_generation
        {
            anyhow::bail!("optimizer rollback authority changed before resume");
        }
        return Ok(StagedRollback::Pending(pending, change));
    }

    let (change, change_digest) = read_change_with_digest(service, change_id)
        .with_context(|| format!("Optimizer change '{change_id}' is not exactly rollbackable"))?;
    let material = change
        .exact_rollback
        .clone()
        .ok_or_else(|| anyhow::anyhow!("legacy optimizer changes are audit-only"))?;
    verify_live_after_images(store, &material)?;

    let pending = PendingOptimizerRollbackV1 {
        schema_version: 1,
        phase: OptimizerRollbackPhaseV1::RetryFenced,
        rollback_id: Uuid::new_v4().to_string(),
        change_id: change_id.to_string(),
        change_digest,
        note_id: change.note_id.clone(),
        requested_at: Utc::now(),
        expected_authority: expected_authority.clone(),
        material,
        intent: None,
        mutation_id: None,
        committed_authority: None,
        audit_written: false,
        counted: false,
        proof_consumed: false,
    };
    preflight_rollback_audit(service, &pending)?;
    write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)?;
    Ok(StagedRollback::Pending(pending, change))
}

fn verify_live_after_images(
    store: &KnowledgeStore,
    material: &ExactOptimizerRollbackMaterialV1,
) -> Result<()> {
    if store.migration_target_before_image(material.target_kind, &material.target_key)?
        != material.apply_after
    {
        anyhow::bail!("optimizer rollback target changed after apply");
    }
    if material.target_kind == TargetKind::OverlayJson
        && store
            .migration_target_before_image(TargetKind::Markdown, &material.source_relative_path)?
            != BeforeImage::Sha256(material.source_digest.clone())
    {
        anyhow::bail!("optimizer rollback source Markdown changed after apply");
    }
    Ok(())
}

fn preflight_rollback_audit(
    service: &VaultOptimizerService,
    candidate: &PendingOptimizerRollbackV1,
) -> Result<()> {
    let mut events = service.load_events()?;
    for (pending, _) in load_pending_rollbacks(service)? {
        if pending.change_id != candidate.change_id && !pending.audit_written {
            merge_optimizer_event(&mut events, rollback_audit_event(&pending))?;
        }
    }
    merge_optimizer_event(&mut events, rollback_audit_event(candidate))?;
    serialize_optimizer_events(&events).map(|_| ())
}

fn rollback_audit_event(pending: &PendingOptimizerRollbackV1) -> OptimizerAuditEventV1 {
    OptimizerAuditEventV1::Rollback {
        change_id: pending.change_id.clone(),
        rollback_id: pending.rollback_id.clone(),
        at: pending.requested_at,
    }
}

fn execute_rollback(
    service: &mut VaultOptimizerService,
    store: &mut KnowledgeStore,
    pending: PendingOptimizerRollbackV1,
    change: OptimizerChange,
) -> Result<OptimizerRollbackMutationOutcome> {
    if pending.phase != OptimizerRollbackPhaseV1::RetryFenced {
        return resume_rollback(service, store, pending, change);
    }
    if store.current_migration_authority()? != pending.expected_authority {
        service.with_locked_fresh_state(|service| {
            let current = load_pending_rollbacks(service)?
                .into_iter()
                .find(|(candidate, _)| candidate.change_id == pending.change_id)
                .map(|(candidate, _)| candidate);
            if current.as_ref().is_some_and(|candidate| {
                candidate.phase == OptimizerRollbackPhaseV1::RetryFenced
                    && candidate.rollback_id == pending.rollback_id
            }) {
                remove_pending_rollback(service.retained_optimizer_root()?, &pending.change_id)?;
            }
            Ok(())
        })?;
        anyhow::bail!("root authority changed before optimizer rollback commit");
    }
    verify_live_after_images(store, &pending.material)?;

    let desired = restore_desired(&pending.material);
    let source_guard = if pending.material.target_kind == TargetKind::OverlayJson {
        Some(
            store
                .optimizer_markdown_precondition(
                    &pending.material.source_relative_path,
                    pending.material.source_digest.clone(),
                )?
                .retained_target()
                .map_err(anyhow::Error::new)?,
        )
    } else {
        None
    };
    let target = crate::services::knowledge_store::ExactMigrationTarget {
        kind: pending.material.target_kind,
        relative_key: pending.material.target_key.clone(),
        expected_before: pending.material.apply_after.clone(),
        desired,
        note_event: Some(crate::services::knowledge_store::ExactMigrationNoteEvent {
            note_id: pending.note_id.clone(),
            change: crate::models::twin_event::NoteChangeKind::Updated,
            observed_at: pending.requested_at,
            governance: pending.material.rollback_governance.clone(),
            payload_digest: pending.material.rollback_payload_digest.clone(),
            evidence_digest: pending.material.rollback_evidence_digest.clone(),
        }),
    };

    let root = service.retained_optimizer_root_handle()?;
    let hook_state = std::cell::RefCell::new(pending.clone());
    let prepared_change = change.clone();
    let prepared_root = root.clone();
    let mut prepared_hook = |intent: &crate::services::twin_events::MutationIntentV1| {
        let mut prepared = hook_state.borrow().clone();
        if prepared.phase != OptimizerRollbackPhaseV1::RetryFenced {
            return Err(crate::services::twin_events::MutationError::Invalid(
                "optimizer rollback owner changed before Prepared publication".into(),
            ));
        }
        prepared.phase = OptimizerRollbackPhaseV1::Prepared;
        prepared.intent = Some(intent.clone());
        prepared.mutation_id = Some(intent.mutation_id.clone());
        write_pending_rollback(&prepared_root, &prepared, &prepared_change)
            .map_err(|error| crate::services::twin_events::MutationError::Io(error.to_string()))?;
        *hook_state.borrow_mut() = prepared;
        Ok(())
    };
    let committed_change = change.clone();
    let committed_root = root;
    let mut committed_hook = |commit: &crate::services::twin_events::MutationCommit| {
        let mut committed = hook_state.borrow().clone();
        if committed.phase != OptimizerRollbackPhaseV1::Prepared
            || committed.mutation_id != commit.mutation_id
        {
            return Err(crate::services::twin_events::MutationError::Invalid(
                "optimizer rollback owner changed before Committed publication".into(),
            ));
        }
        committed.phase = OptimizerRollbackPhaseV1::Committed;
        committed.committed_authority = commit.authority_token.clone();
        write_pending_rollback(&committed_root, &committed, &committed_change)
            .map_err(|error| crate::services::twin_events::MutationError::Io(error.to_string()))?;
        *hook_state.borrow_mut() = committed;
        Ok(())
    };

    let write_result = store.commit_exact_optimizer_target_with_hooks(
        target,
        source_guard,
        pending.expected_authority.clone(),
        &mut prepared_hook,
        &mut committed_hook,
    );
    match write_result {
        Ok(commit) => finish_committed_outcome(service, store, &pending.change_id, commit, None),
        Err(error) => {
            let authority_advanced = error
                .downcast_ref::<crate::services::twin_events::MutationError>()
                .and_then(crate::services::twin_events::MutationError::authority_advanced_commit);
            match resume_rollback(service, store, hook_state.into_inner(), change) {
                Ok(outcome) => Ok(outcome),
                Err(recovery_error) => {
                    if let Some(commit) = authority_advanced {
                        log::error!(
                            "Optimizer rollback {} remains recoverable after authority: {}",
                            pending.change_id,
                            recovery_error
                        );
                        return Ok(partial_outcome(&pending.change_id, commit));
                    }
                    if matches!(
                        hook_state_phase(service, &pending.change_id)?,
                        Some(OptimizerRollbackPhaseV1::RetryFenced)
                    ) {
                        service.with_locked_fresh_state(|service| {
                            remove_pending_rollback(
                                service.retained_optimizer_root()?,
                                &pending.change_id,
                            )
                        })?;
                    }
                    Err(error.context(recovery_error.to_string()))
                }
            }
        }
    }
}

fn hook_state_phase(
    service: &mut VaultOptimizerService,
    change_id: &str,
) -> Result<Option<OptimizerRollbackPhaseV1>> {
    service.with_locked_fresh_state(|service| {
        Ok(load_pending_rollbacks(service)?
            .into_iter()
            .find(|(pending, _)| pending.change_id == change_id)
            .map(|(pending, _)| pending.phase))
    })
}

fn resume_rollback(
    service: &mut VaultOptimizerService,
    store: &KnowledgeStore,
    pending: PendingOptimizerRollbackV1,
    change: OptimizerChange,
) -> Result<OptimizerRollbackMutationOutcome> {
    if pending.phase == OptimizerRollbackPhaseV1::RetryFenced {
        anyhow::bail!("optimizer rollback has not reached its Prepared owner");
    }
    let change_id = pending.change_id.clone();
    let recovered = recover_rollback_owner(service, store, pending, change)?;
    match recovered {
        RecoveredRollback::Committed(commit) => {
            finish_committed_outcome(service, store, &change_id, commit, None)
        }
        RecoveredRollback::AbortedBeforeAuthority => {
            anyhow::bail!("optimizer rollback was aborted before authority")
        }
        RecoveredRollback::AbortedAfterAuthority(commit) => {
            Ok(OptimizerRollbackMutationOutcome::Partial {
                result: rollback_result(
                    &change_id,
                    false,
                    "Optimizer rollback was aborted after authority; no target bytes were restored",
                    Some(aborted_after_authority_warning()),
                ),
                commit,
                warning: aborted_after_authority_warning(),
                recovery_pending: false,
            })
        }
    }
}

fn recover_rollback_owner(
    service: &mut VaultOptimizerService,
    store: &KnowledgeStore,
    pending_hint: PendingOptimizerRollbackV1,
    _change_hint: OptimizerChange,
) -> Result<RecoveredRollback> {
    store.recover_coordinated_mutations()?;
    let (mut pending, change) = service.with_locked_fresh_state(|service| {
        load_pending_rollbacks(service)?
            .into_iter()
            .find(|(pending, _)| pending.change_id == pending_hint.change_id)
            .ok_or_else(|| anyhow::anyhow!("optimizer rollback owner disappeared"))
    })?;
    if pending.phase == OptimizerRollbackPhaseV1::Committed {
        let commit = commit_from_pending(&pending)?;
        return Ok(RecoveredRollback::Committed(commit));
    }
    if pending.phase == OptimizerRollbackPhaseV1::Aborted {
        let commit = pending
            .committed_authority
            .as_ref()
            .map(|_| commit_from_pending(&pending))
            .transpose()?;
        acknowledge_aborted(service, store, &mut pending, &change)?;
        return Ok(commit.map_or(
            RecoveredRollback::AbortedBeforeAuthority,
            RecoveredRollback::AbortedAfterAuthority,
        ));
    }
    let mutation_id = pending
        .mutation_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("optimizer rollback Prepared owner lacks mutation ID"))?;
    let recovery = store.classify_migration_witness(
        &mutation_id,
        &pending.expected_authority,
        pending.material.target_kind,
        &pending.material.target_key,
        &pending.material.apply_after,
        &pending.material.restore_before,
    )?;
    match recovery {
        crate::services::twin_events::WitnessedMutationRecovery::NotCommitted => {
            service.with_locked_fresh_state(|service| {
                remove_pending_rollback(service.retained_optimizer_root()?, &pending.change_id)
            })?;
            Ok(RecoveredRollback::AbortedBeforeAuthority)
        }
        crate::services::twin_events::WitnessedMutationRecovery::Aborted => {
            pending.phase = OptimizerRollbackPhaseV1::Aborted;
            pending.committed_authority = None;
            service.with_locked_fresh_state(|service| {
                write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)
            })?;
            acknowledge_aborted(service, store, &mut pending, &change)?;
            Ok(RecoveredRollback::AbortedBeforeAuthority)
        }
        crate::services::twin_events::WitnessedMutationRecovery::AbortedAfterAuthority(commit) => {
            pending.phase = OptimizerRollbackPhaseV1::Aborted;
            pending.committed_authority = commit.authority_token.clone();
            service.with_locked_fresh_state(|service| {
                write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)
            })?;
            acknowledge_aborted(service, store, &mut pending, &change)?;
            Ok(RecoveredRollback::AbortedAfterAuthority(commit))
        }
        crate::services::twin_events::WitnessedMutationRecovery::Committed(commit) => {
            pending.phase = OptimizerRollbackPhaseV1::Committed;
            pending.committed_authority = commit.authority_token.clone();
            service.with_locked_fresh_state(|service| {
                write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)
            })?;
            Ok(RecoveredRollback::Committed(commit))
        }
    }
}

fn acknowledge_aborted(
    service: &mut VaultOptimizerService,
    store: &KnowledgeStore,
    pending: &mut PendingOptimizerRollbackV1,
    change: &OptimizerChange,
) -> Result<()> {
    let mutation_id = pending
        .mutation_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("optimizer aborted rollback lacks mutation ID"))?;
    store.consume_migration_witness(&mutation_id)?;
    pending.proof_consumed = true;
    service.with_locked_fresh_state(|service| {
        write_pending_rollback(service.retained_optimizer_root()?, pending, change)?;
        remove_pending_rollback(service.retained_optimizer_root()?, &pending.change_id)
    })
}

fn finish_committed_outcome(
    service: &mut VaultOptimizerService,
    store: &KnowledgeStore,
    change_id: &str,
    commit: crate::services::twin_events::MutationCommit,
    warning: Option<crate::models::mutation::CommittedMutationWarningV1>,
) -> Result<OptimizerRollbackMutationOutcome> {
    match finalize_committed_rollback(service, store, change_id, &commit) {
        Ok(()) => Ok(OptimizerRollbackMutationOutcome::Committed {
            result: rollback_result(
                change_id,
                true,
                "Optimizer change rolled back",
                warning.clone(),
            ),
            commit,
            warning,
        }),
        Err(error) => {
            log::error!(
                "Optimizer rollback {change_id} committed but publication remains pending: {error}"
            );
            Ok(partial_outcome(change_id, commit))
        }
    }
}

fn finalize_committed_rollback(
    service: &mut VaultOptimizerService,
    store: &KnowledgeStore,
    change_id: &str,
    commit: &crate::services::twin_events::MutationCommit,
) -> Result<()> {
    let mutation_id = commit
        .mutation_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("optimizer rollback commit lacks its mutation ID"))?;
    service.with_locked_fresh_state(|service| {
        let (mut pending, change) = load_pending_rollbacks(service)?
            .into_iter()
            .find(|(pending, _)| pending.change_id == change_id)
            .ok_or_else(|| anyhow::anyhow!("optimizer committed rollback owner disappeared"))?;
        if pending.phase != OptimizerRollbackPhaseV1::Committed
            || pending.mutation_id.as_ref() != Some(&mutation_id)
            || pending.committed_authority != commit.authority_token
        {
            anyhow::bail!("optimizer committed rollback owner changed");
        }
        finalize_rollback_audit_locked(service, &mut pending, &change)
    })?;

    store.consume_migration_witness(&mutation_id)?;
    service.with_locked_fresh_state(|service| {
        let Some((mut pending, change)) = load_pending_rollbacks(service)?
            .into_iter()
            .find(|(pending, _)| pending.change_id == change_id)
        else {
            return Ok(());
        };
        if pending.phase != OptimizerRollbackPhaseV1::Committed
            || !pending.audit_written
            || !pending.counted
        {
            anyhow::bail!("optimizer rollback publication regressed before proof consumption");
        }
        if !pending.proof_consumed {
            pending.proof_consumed = true;
            write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)?;
        }
        remove_pending_rollback(service.retained_optimizer_root()?, change_id)
    })
}

fn finalize_rollback_audit_locked(
    service: &mut VaultOptimizerService,
    pending: &mut PendingOptimizerRollbackV1,
    change: &OptimizerChange,
) -> Result<()> {
    if !pending.audit_written {
        service.append_event_unique(&pending.change_id, rollback_audit_event(pending))?;
        pending.audit_written = true;
        write_pending_rollback(service.retained_optimizer_root()?, pending, change)?;
    }
    if !pending.counted {
        let mut unique = HashSet::new();
        for event in service.load_events()? {
            if let OptimizerAuditEventV1::Rollback { change_id, .. } = event {
                unique.insert(change_id);
            }
        }
        service.state.rollback_count = unique.len();
        service.persist_state()?;
        pending.counted = true;
        write_pending_rollback(service.retained_optimizer_root()?, pending, change)?;
    }
    Ok(())
}

fn commit_from_pending(
    pending: &PendingOptimizerRollbackV1,
) -> Result<crate::services::twin_events::MutationCommit> {
    Ok(crate::services::twin_events::MutationCommit {
        mutation_id: Some(
            pending
                .mutation_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("optimizer rollback lacks mutation ID"))?,
        ),
        events: Vec::new(),
        authority_token: pending.committed_authority.clone(),
        postcommit_warning: true,
    })
}

fn partial_outcome(
    change_id: &str,
    commit: crate::services::twin_events::MutationCommit,
) -> OptimizerRollbackMutationOutcome {
    OptimizerRollbackMutationOutcome::Partial {
        result: rollback_result(
            change_id,
            false,
            "Optimizer rollback authority advanced; recovery remains pending",
            Some(fixed_warning()),
        ),
        commit,
        warning: fixed_warning(),
        recovery_pending: true,
    }
}

fn fixed_warning() -> crate::models::mutation::CommittedMutationWarningV1 {
    crate::models::mutation::CommittedMutationWarningV1::optimizer_rollback_recovery_pending()
}

fn aborted_after_authority_warning() -> crate::models::mutation::CommittedMutationWarningV1 {
    crate::models::mutation::CommittedMutationWarningV1::optimizer_rollback_not_applied()
}

fn rollback_result(
    change_id: &str,
    rolled_back: bool,
    message: &str,
    warning: Option<crate::models::mutation::CommittedMutationWarningV1>,
) -> crate::models::migration::VaultOptimizerRollbackResult {
    crate::models::migration::VaultOptimizerRollbackResult {
        change_id: change_id.to_string(),
        rolled_back,
        message: message.to_string(),
        warning,
    }
}

pub(super) fn recover_pending_rollbacks_locked(
    service: &mut VaultOptimizerService,
    guard: &crate::services::twin_events::MutationRootTransitionGuard<'_>,
) -> Result<()> {
    for (mut pending, change) in load_pending_rollbacks(service)? {
        if pending.phase == OptimizerRollbackPhaseV1::RetryFenced {
            continue;
        }
        let mutation_id = pending
            .mutation_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("optimizer rollback owner lacks mutation ID"))?;

        if pending.phase == OptimizerRollbackPhaseV1::Aborted {
            guard
                .consume_witnessed_mutation_receipt(&mutation_id)
                .map_err(anyhow::Error::new)?;
            pending.proof_consumed = true;
            write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)?;
            remove_pending_rollback(service.retained_optimizer_root()?, &pending.change_id)?;
            continue;
        }

        if pending.phase == OptimizerRollbackPhaseV1::Committed {
            finalize_rollback_audit_locked(service, &mut pending, &change)?;
            guard
                .consume_witnessed_mutation_receipt(&mutation_id)
                .map_err(anyhow::Error::new)?;
            pending.proof_consumed = true;
            write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)?;
            remove_pending_rollback(service.retained_optimizer_root()?, &pending.change_id)?;
            continue;
        }

        match guard
            .classify_witnessed_mutation(
                &mutation_id,
                &pending.expected_authority,
                pending.material.target_kind,
                &pending.material.target_key,
                &pending.material.apply_after,
                &pending.material.restore_before,
            )
            .map_err(anyhow::Error::new)?
        {
            crate::services::twin_events::WitnessedMutationRecovery::NotCommitted => {
                remove_pending_rollback(service.retained_optimizer_root()?, &pending.change_id)?;
            }
            crate::services::twin_events::WitnessedMutationRecovery::Aborted => {
                pending.phase = OptimizerRollbackPhaseV1::Aborted;
                pending.committed_authority = None;
                write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)?;
                guard
                    .consume_witnessed_mutation_receipt(&mutation_id)
                    .map_err(anyhow::Error::new)?;
                pending.proof_consumed = true;
                write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)?;
                remove_pending_rollback(service.retained_optimizer_root()?, &pending.change_id)?;
            }
            crate::services::twin_events::WitnessedMutationRecovery::AbortedAfterAuthority(
                commit,
            ) => {
                pending.phase = OptimizerRollbackPhaseV1::Aborted;
                pending.committed_authority = commit.authority_token;
                write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)?;
                guard
                    .consume_witnessed_mutation_receipt(&mutation_id)
                    .map_err(anyhow::Error::new)?;
                pending.proof_consumed = true;
                write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)?;
                remove_pending_rollback(service.retained_optimizer_root()?, &pending.change_id)?;
            }
            crate::services::twin_events::WitnessedMutationRecovery::Committed(commit) => {
                pending.phase = OptimizerRollbackPhaseV1::Committed;
                pending.committed_authority = commit.authority_token;
                write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)?;
                finalize_rollback_audit_locked(service, &mut pending, &change)?;
                guard
                    .consume_witnessed_mutation_receipt(&mutation_id)
                    .map_err(anyhow::Error::new)?;
                pending.proof_consumed = true;
                write_pending_rollback(service.retained_optimizer_root()?, &pending, &change)?;
                remove_pending_rollback(service.retained_optimizer_root()?, &pending.change_id)?;
            }
        }
    }
    Ok(())
}
