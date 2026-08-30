use crate::models::migration::{
    VaultOptimizerDecision, VaultOptimizerInboxEntry, VaultOptimizerRollbackResult,
    VaultOptimizerStatus,
};
use crate::models::note::{
    Note, NoteUpdate, CURRENT_NOTE_SCHEMA_VERSION, PROP_INFERRED_LINK_IDS, PROP_TOPIC_ALIASES,
    PROP_TOPIC_KEY,
};
use crate::models::settings::UserSettings;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::topic_hub::normalize_topic_key;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use uuid::Uuid;

mod publication;
#[path = "vault_optimizer/rollback.rs"]
mod rollback;
#[cfg(test)]
#[path = "vault_optimizer_rollback_tests.rs"]
mod rollback_tests;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueuedOptimizerNote {
    #[serde(default)]
    job_id: String,
    note_id: String,
    reason: String,
    enqueued_at: DateTime<Utc>,
    /// Number of times processing this job has failed. Incremented by
    /// `defer_or_park_job`; once it reaches `MAX_OPTIMIZER_ATTEMPTS` the job is
    /// parked into the inbox as a failed decision instead of retried forever.
    #[serde(default)]
    attempts: u32,
    #[serde(default)]
    pending_parking: Option<PendingOptimizerParkingV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingOptimizerParkingV1 {
    error: String,
    at: DateTime<Utc>,
}

/// A processing failure gets `MAX_OPTIMIZER_ATTEMPTS` tries (each a separate
/// background-worker tick) before the job is parked into the inbox as a
/// failed decision and dropped from the queue, so a permanently-poisoned
/// entry (e.g. a note whose overlay path can never be written) can't spin
/// forever and starve the rest of the queue.
const MAX_OPTIMIZER_ATTEMPTS: u32 = 3;
const OPTIMIZER_STATE_SCHEMA_VERSION: u16 = 1;
const PENDING_PUBLICATIONS_DIRECTORY: &str = "pending-publications-v1";
const MAX_PENDING_PUBLICATIONS: usize = 64;
const MAX_PENDING_PUBLICATION_BYTES: usize = 16 * 1024 * 1024;
const MAX_OPTIMIZER_STATE_BYTES: usize = 4 * 1024 * 1024;
const MAX_OPTIMIZER_AUDIT_BYTES: usize = 4 * 1024 * 1024;
const MAX_OPTIMIZER_AUDIT_ENTRIES: usize = 4096;
const MAX_OPTIMIZER_CHANGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_OPTIMIZER_ORPHAN_TEMPS: usize = 64;
const CHANGES_DIRECTORY: &str = "changes";
const QUEUE_KEY: &str = "queue.json";
const DECISIONS_KEY: &str = "decisions.json";
const INBOX_KEY: &str = "inbox.json";
const EVENTS_KEY: &str = "events.jsonl";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct OptimizerState {
    #[serde(default)]
    schema_version: u16,
    #[serde(default)]
    state_revision: u64,
    #[serde(default)]
    queue: Vec<QueuedOptimizerNote>,
    #[serde(default)]
    last_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    accepted_count: usize,
    #[serde(default)]
    rejected_count: usize,
    #[serde(default)]
    rollback_count: usize,
    /// UTC calendar date (`YYYY-MM-DD`) the `daily_write_count` below applies
    /// to. Persisted so the daily cap survives an app restart instead of
    /// resetting for free.
    #[serde(default)]
    daily_write_date: Option<String>,
    /// Number of optimizer writes (sidecar overlay or full-rewrite note
    /// update) already applied on `daily_write_date`. Reset to 1 whenever a
    /// write happens on a new calendar date.
    #[serde(default)]
    daily_write_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct OptimizerChange {
    change_id: String,
    note_id: String,
    #[serde(default)]
    mode: String,
    #[serde(default)]
    overlay_before: Option<Value>,
    #[serde(default)]
    overlay_after: Option<Value>,
    #[serde(default)]
    note_before: Option<Note>,
    #[serde(default)]
    note_after: Option<Note>,
    #[serde(default)]
    markdown_before_digest: Option<crate::models::twin_event::ContentDigest>,
    #[serde(default)]
    markdown_relative_path: Option<String>,
    #[serde(default)]
    created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    exact_rollback: Option<rollback::ExactOptimizerRollbackMaterialV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingOptimizerPublication {
    schema_version: u16,
    phase: OptimizerPublicationPhase,
    retry_fenced: bool,
    audit_written: bool,
    counted: bool,
    queue_removed: bool,
    #[serde(default)]
    abort_queue_reconciled: bool,
    expected_authority: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    mutation_id: Option<crate::models::twin_event::ContentDigest>,
    committed_authority: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    target: OptimizerPublicationTarget,
    job: QueuedOptimizerNote,
    note: Note,
    change_id: String,
    decision: VaultOptimizerDecision,
    change: OptimizerChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OptimizerPublicationPhase {
    RetryFenced,
    Prepared,
    Aborted,
    Committed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum OptimizerAuditEventV1 {
    Rollback {
        change_id: String,
        #[serde(default)]
        rollback_id: String,
        at: DateTime<Utc>,
    },
    OptimizerParked {
        #[serde(default)]
        job_id: String,
        note_id: String,
        attempts: u32,
        error: String,
        at: DateTime<Utc>,
    },
    OptimizerApply {
        note_id: String,
        change_id: String,
        at: DateTime<Utc>,
        confidence: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum OptimizerPublicationTarget {
    Overlay {
        note_id: String,
        before_digest: Option<crate::models::twin_event::ContentDigest>,
        after_digest: crate::models::twin_event::ContentDigest,
        #[serde(default)]
        source_relative_path: String,
        #[serde(default)]
        source_digest: Option<crate::models::twin_event::ContentDigest>,
    },
    Markdown {
        relative_path: String,
        before_digest: crate::models::twin_event::ContentDigest,
        after_digest: crate::models::twin_event::ContentDigest,
    },
}

#[derive(Debug, Clone)]
pub struct VaultOptimizerService {
    optimizer_dir: PathBuf,
    optimizer_root: Option<std::sync::Arc<crate::services::twin_events::AnchoredRoot>>,
    #[cfg_attr(not(test), allow(dead_code))]
    queue_path: PathBuf,
    #[cfg_attr(not(test), allow(dead_code))]
    decisions_path: PathBuf,
    #[cfg_attr(not(test), allow(dead_code))]
    events_path: PathBuf,
    #[cfg_attr(not(test), allow(dead_code))]
    changes_dir: PathBuf,
    state: OptimizerState,
    #[cfg(test)]
    fail_prepared_publication_once: bool,
    #[cfg(test)]
    fail_committed_publication_once: bool,
    #[cfg(test)]
    fail_retry_fence_stage_after_write_once: bool,
    #[cfg(test)]
    fail_terminal_parking_after_publication_once: bool,
    #[cfg(test)]
    pause_after_retry_fence_once: Option<(
        std::sync::Arc<std::sync::Barrier>,
        std::sync::Arc<std::sync::Barrier>,
    )>,
    #[cfg(test)]
    pause_before_markdown_digest_once: Option<(
        std::sync::Arc<std::sync::Barrier>,
        std::sync::Arc<std::sync::Barrier>,
    )>,
    #[cfg(test)]
    pause_before_prepared_hook_once: Option<(
        std::sync::Arc<std::sync::Barrier>,
        std::sync::Arc<std::sync::Barrier>,
    )>,
    #[cfg(test)]
    pause_after_prepared_publication_once: Option<(
        std::sync::Arc<std::sync::Barrier>,
        std::sync::Arc<std::sync::Barrier>,
    )>,
}

impl VaultOptimizerService {
    pub fn new(data_path: PathBuf) -> Self {
        let fallback_path = data_path.clone();
        Self::try_new(data_path).unwrap_or_else(|error| {
            log::error!("Failed to initialize vault optimizer state: {error}");
            Self::empty_at(fallback_path)
        })
    }

    pub(crate) fn try_new(data_path: PathBuf) -> Result<Self> {
        let optimizer_dir = data_path.join("vault_migration").join("optimizer");
        let queue_path = optimizer_dir.join("queue.json");
        let decisions_path = optimizer_dir.join("decisions.json");
        let events_path = optimizer_dir.join("events.jsonl");
        let changes_dir = optimizer_dir.join("changes");
        std::fs::create_dir_all(&optimizer_dir)?;
        let root = crate::services::twin_events::AnchoredRoot::open(&optimizer_dir)
            .map_err(anyhow::Error::new)?;
        root.open_directory(CHANGES_DIRECTORY, true)
            .map_err(anyhow::Error::new)?;
        root.open_directory(PENDING_PUBLICATIONS_DIRECTORY, true)
            .map_err(anyhow::Error::new)?;
        rollback::initialize_root(&root)?;

        let mut service = Self {
            optimizer_dir,
            optimizer_root: Some(std::sync::Arc::new(root)),
            queue_path,
            decisions_path,
            events_path,
            changes_dir,
            state: OptimizerState::default(),
            #[cfg(test)]
            fail_prepared_publication_once: false,
            #[cfg(test)]
            fail_committed_publication_once: false,
            #[cfg(test)]
            fail_retry_fence_stage_after_write_once: false,
            #[cfg(test)]
            fail_terminal_parking_after_publication_once: false,
            #[cfg(test)]
            pause_after_retry_fence_once: None,
            #[cfg(test)]
            pause_before_markdown_digest_once: None,
            #[cfg(test)]
            pause_before_prepared_hook_once: None,
            #[cfg(test)]
            pause_after_prepared_publication_once: None,
        };
        let lock = service.acquire_state_lock()?;
        let load_result = service.reload_from_disk_checked();
        let unlock_result = lock.unlock().map_err(anyhow::Error::new);
        load_result?;
        unlock_result?;
        Ok(service)
    }

    fn empty_at(data_path: PathBuf) -> Self {
        let optimizer_dir = data_path.join("vault_migration").join("optimizer");
        let optimizer_root = std::fs::create_dir_all(&optimizer_dir)
            .ok()
            .and_then(|()| crate::services::twin_events::AnchoredRoot::open(&optimizer_dir).ok())
            .map(std::sync::Arc::new);
        if let Some(root) = optimizer_root.as_deref() {
            let _ = root.open_directory(CHANGES_DIRECTORY, true);
            let _ = root.open_directory(PENDING_PUBLICATIONS_DIRECTORY, true);
            let _ = rollback::initialize_root(root);
        }
        Self {
            queue_path: optimizer_dir.join("queue.json"),
            decisions_path: optimizer_dir.join("decisions.json"),
            events_path: optimizer_dir.join("events.jsonl"),
            changes_dir: optimizer_dir.join("changes"),
            optimizer_dir,
            optimizer_root,
            state: OptimizerState::default(),
            #[cfg(test)]
            fail_prepared_publication_once: false,
            #[cfg(test)]
            fail_committed_publication_once: false,
            #[cfg(test)]
            fail_retry_fence_stage_after_write_once: false,
            #[cfg(test)]
            fail_terminal_parking_after_publication_once: false,
            #[cfg(test)]
            pause_after_retry_fence_once: None,
            #[cfg(test)]
            pause_before_markdown_digest_once: None,
            #[cfg(test)]
            pause_before_prepared_hook_once: None,
            #[cfg(test)]
            pause_after_prepared_publication_once: None,
        }
    }

    pub(crate) fn uses_data_path(&self, data_path: &std::path::Path) -> bool {
        self.optimizer_dir == data_path.join("vault_migration").join("optimizer")
    }

    #[cfg(test)]
    pub(crate) fn state_revision(&self) -> u64 {
        self.state.state_revision
    }

    fn retained_optimizer_root(&self) -> Result<&crate::services::twin_events::AnchoredRoot> {
        self.optimizer_root
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("vault optimizer filesystem root is unavailable"))
    }

    fn retained_optimizer_root_handle(
        &self,
    ) -> Result<std::sync::Arc<crate::services::twin_events::AnchoredRoot>> {
        self.optimizer_root
            .clone()
            .ok_or_else(|| anyhow::anyhow!("vault optimizer filesystem root is unavailable"))
    }

    pub(crate) fn with_locked_fresh_state<T>(
        &mut self,
        action: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let lock = self
            .retained_optimizer_root()?
            .lock_exclusive("state.lock")
            .map_err(anyhow::Error::new)?;
        let result = self.reload_from_disk_checked().and_then(|()| action(self));
        lock.unlock()?;
        result
    }

    pub(crate) fn acquire_state_lock(
        &self,
    ) -> Result<crate::services::twin_events::AnchoredExclusiveLock> {
        self.retained_optimizer_root()?
            .lock_exclusive("state.lock")
            .map_err(anyhow::Error::new)
    }

    pub(crate) fn reload_from_disk_checked(&mut self) -> Result<()> {
        let loaded = {
            let root = self.retained_optimizer_root()?;
            cleanup_optimizer_orphan_temps(root)?;
            load_optimizer_state(root)?
        };
        self.state = loaded;
        normalize_optimizer_job_ids(&mut self.state)?;
        if self.state.schema_version > OPTIMIZER_STATE_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported vault optimizer state schema {}",
                self.state.schema_version
            );
        }
        self.validate_persisted_state()?;
        self.recover_pending_parkings()
    }

    #[cfg(test)]
    fn fail_next_prepared_publication(&mut self) {
        self.fail_prepared_publication_once = true;
    }

    #[cfg(test)]
    fn fail_next_committed_publication(&mut self) {
        self.fail_committed_publication_once = true;
    }

    #[cfg(test)]
    fn fail_next_retry_fence_stage_after_write(&mut self) {
        self.fail_retry_fence_stage_after_write_once = true;
    }

    #[cfg(test)]
    fn fail_next_terminal_parking_after_publication(&mut self) {
        self.fail_terminal_parking_after_publication_once = true;
    }

    #[cfg(test)]
    fn pause_after_retry_fence_once(
        &mut self,
        entered: std::sync::Arc<std::sync::Barrier>,
        resume: std::sync::Arc<std::sync::Barrier>,
    ) {
        self.pause_after_retry_fence_once = Some((entered, resume));
    }

    #[cfg(test)]
    fn pause_before_markdown_digest_once(
        &mut self,
        entered: std::sync::Arc<std::sync::Barrier>,
        resume: std::sync::Arc<std::sync::Barrier>,
    ) {
        self.pause_before_markdown_digest_once = Some((entered, resume));
    }

    #[cfg(test)]
    fn pause_before_prepared_hook_once(
        &mut self,
        entered: std::sync::Arc<std::sync::Barrier>,
        resume: std::sync::Arc<std::sync::Barrier>,
    ) {
        self.pause_before_prepared_hook_once = Some((entered, resume));
    }

    #[cfg(test)]
    fn pause_after_prepared_publication_once(
        &mut self,
        entered: std::sync::Arc<std::sync::Barrier>,
        resume: std::sync::Arc<std::sync::Barrier>,
    ) {
        self.pause_after_prepared_publication_once = Some((entered, resume));
    }

    pub fn bootstrap(&mut self, notes: &[Note]) {
        if let Err(error) = self.bootstrap_checked(notes) {
            log::error!("Failed to persist vault optimizer bootstrap: {error}");
        }
    }

    pub(crate) fn bootstrap_checked(&mut self, notes: &[Note]) -> Result<()> {
        if self.state.queue.is_empty() {
            for note in notes.iter().filter(|note| !note.is_topic_hub()) {
                self.enqueue_note(&note.id, "bootstrap");
            }
            self.persist_state()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn reset_for_vault(&mut self, notes: &[Note]) {
        if let Err(error) = self.reset_for_vault_checked(notes) {
            log::error!("Failed to reset vault optimizer state: {error}");
        }
    }

    pub(crate) fn reset_for_vault_checked(&mut self, notes: &[Note]) -> Result<()> {
        self.state.queue.clear();
        for note in notes.iter().filter(|note| !note.is_topic_hub()) {
            self.enqueue_note(&note.id, "bootstrap");
        }
        self.persist_state()
    }

    fn validate_persisted_state(&self) -> Result<()> {
        if self.state.queue.len() > MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer queue exceeds 4096 entries");
        }
        let mut job_ids = HashSet::new();
        let mut note_ids = HashSet::new();
        for job in &self.state.queue {
            parse_canonical_uuid(&job.job_id, "optimizer job ID")?;
            if !job_ids.insert(job.job_id.as_str()) || !note_ids.insert(job.note_id.as_str()) {
                anyhow::bail!("optimizer queue contains duplicate job identity");
            }
            if job.pending_parking.is_some() && job.attempts < MAX_OPTIMIZER_ATTEMPTS {
                anyhow::bail!("optimizer parking witness precedes the terminal attempt");
            }
        }
        self.load_decisions()?;
        self.load_inbox()?;
        self.load_events()?;
        let root = self.retained_optimizer_root()?;
        let names = root
            .regular_file_names(CHANGES_DIRECTORY)
            .map_err(anyhow::Error::new)?;
        if names.len() > MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer change audit exceeds 4096 entries");
        }
        for name in names {
            let change_id = name
                .strip_suffix(".json")
                .ok_or_else(|| anyhow::anyhow!("invalid optimizer change filename"))?;
            parse_canonical_uuid(change_id, "optimizer change ID")?;
            let change = self.read_change(change_id)?;
            if change.change_id != change_id {
                anyhow::bail!("optimizer change filename identity mismatch");
            }
        }
        for (_, pending) in self.load_pending_publications()? {
            validate_pending_publication(&pending)?;
        }
        rollback::validate_pending_rollbacks(self)?;
        Ok(())
    }

    pub fn enqueue_note(&mut self, note_id: &str, reason: &str) {
        if self
            .state
            .queue
            .iter()
            .any(|entry| entry.note_id == note_id)
        {
            return;
        }

        self.state.queue.push(QueuedOptimizerNote {
            job_id: Uuid::new_v4().to_string(),
            note_id: note_id.to_string(),
            reason: reason.to_string(),
            enqueued_at: Utc::now(),
            attempts: 0,
            pending_parking: None,
        });
    }

    pub(crate) fn enqueue_note_checked(&mut self, note_id: &str, reason: &str) -> Result<bool> {
        let before = self.state.queue.len();
        self.enqueue_note(note_id, reason);
        if self.state.queue.len() == before {
            return Ok(false);
        }
        self.persist_state()?;
        Ok(true)
    }

    pub fn status(&self, settings: &UserSettings) -> VaultOptimizerStatus {
        let decisions = self.load_decisions().unwrap_or_default();
        let inbox = self.load_inbox().unwrap_or_default();
        let accepted = self.state.accepted_count;
        let rejected = self.state.rejected_count;
        let rollbacks = self.state.rollback_count;
        let total_completed = accepted + rejected;
        VaultOptimizerStatus {
            enabled: settings.background_vault_optimizer_enabled,
            llm_enabled: settings.background_vault_optimizer_llm_enabled,
            edit_mode: settings.background_vault_optimizer_edit_mode.clone(),
            queue_size: self.state.queue.len(),
            last_run_at: self.state.last_run_at,
            accepted_count: accepted,
            rejected_count: rejected,
            rollback_count: rollbacks,
            inbox_count: inbox.len(),
            recent_auto_edits: decisions.into_iter().rev().take(5).collect(),
            rollback_rate: if total_completed == 0 {
                0.0
            } else {
                rollbacks as f64 / total_completed as f64
            },
        }
    }

    pub fn list_decisions(&self, limit: usize) -> Result<Vec<VaultOptimizerDecision>> {
        let mut decisions = self.load_decisions()?;
        decisions.reverse();
        decisions.truncate(limit);
        Ok(decisions)
    }

    pub fn inbox(
        &self,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<VaultOptimizerInboxEntry>> {
        let mut inbox = self.load_inbox()?;
        if let Some(status) = status {
            inbox.retain(|entry| entry.status.eq_ignore_ascii_case(status));
        }
        inbox.reverse();
        inbox.truncate(limit);
        Ok(inbox)
    }

    pub(crate) fn rollback_change_expecting_authority(
        &mut self,
        change_id: &str,
        store: &mut KnowledgeStore,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<OptimizerRollbackMutationOutcome> {
        rollback::rollback_change(self, change_id, store, expected)
    }

    /// Advances the optimizer queue using only *read* access to the vault.
    ///
    /// This resolves every case that doesn't require a mutable, cache-rebuilding
    /// `KnowledgeStore::update_note` write: disabled optimizer, empty queue,
    /// missing/topic-hub/unparsable-frontmatter/no-op notes (all terminal —
    /// dequeued here), the daily write cap, and the `sidecar_first` write path
    /// itself (`KnowledgeStore::write_overlay` takes `&self`, so it's safe under
    /// a read lock). Only when `edit_mode` is something other than
    /// `sidecar_first` (i.e. a real note rewrite is needed) does this return
    /// `OptimizerTick::Pending` for the caller to apply via
    /// [`Self::apply_pending`] under a write lock. This mirrors
    /// `link_discovery::discover_for_note`'s snapshot-under-read-lock /
    /// work-lock-free / write-lock-only-to-apply pattern: the background
    /// worker (`main.rs::start_vault_optimizer_worker`) no longer needs to hold
    /// `knowledge_store.write()` for the whole run, only for the narrow
    /// `apply_pending` step when one is actually needed.
    ///
    /// When a `sidecar_first` write is applied inline, the return value is
    /// `OptimizerTick::Applied(note_id)` rather than a bare "done" signal —
    /// the write already changed the note's tags/aliases/properties on disk,
    /// and the caller (`main.rs::start_vault_optimizer_worker`) MUST refresh
    /// that note's search/chunk/topic-hub state afterward (via
    /// `commands::commit_note_index_refresh`, once every lock this call held
    /// has been released) or the change is invisible to search until the next
    /// full index rebuild. See `commit_note_index_refresh`'s doc comment for
    /// why that's a narrower helper than `commit_note_write` and not just a
    /// call to it.
    pub fn prepare_next(
        &mut self,
        store: &KnowledgeStore,
        settings: &UserSettings,
    ) -> Result<OptimizerTick> {
        self.prepare_next_internal(store, settings, None)
    }

    pub(crate) fn prepare_next_expecting_authority(
        &mut self,
        store: &KnowledgeStore,
        settings: &UserSettings,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<OptimizerTick> {
        self.prepare_next_internal(store, settings, Some(expected))
    }

    fn prepare_next_internal(
        &mut self,
        store: &KnowledgeStore,
        settings: &UserSettings,
        expected_authority: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<OptimizerTick> {
        if !settings.background_vault_optimizer_enabled {
            return Ok(OptimizerTick::NoWrite);
        }

        let Some(job) = self.state.queue.first().cloned() else {
            return Ok(OptimizerTick::NoWrite);
        };
        if job.pending_parking.is_some() {
            self.finish_pending_parking(&job)?;
            return Ok(OptimizerTick::NoWrite);
        }
        if let Some((_, pending)) = self
            .load_pending_publications()?
            .into_iter()
            .find(|(_, pending)| pending.job.job_id == job.job_id)
        {
            if pending.phase == OptimizerPublicationPhase::RetryFenced {
                return Ok(OptimizerTick::RetryFenced(Box::new(
                    RetryFencedOptimizerWrite {
                        publication: pending,
                    },
                )));
            }
            // Prepared and Committed witnesses must be classified/finalized
            // by a complete authority rebuild. They are never replayed as a
            // fresh write with a new change identity.
            return Ok(OptimizerTick::NoWrite);
        }

        let note = match store.optimizer_note_snapshot(&job.note_id) {
            Ok(Some(snapshot)) => snapshot.note,
            Ok(None) => {
                log::warn!("Skipping missing optimizer note '{}'", job.note_id);
                self.remove_queued_job(&job.note_id);
                self.persist_state()?;
                return Ok(OptimizerTick::NoWrite);
            }
            Err(error) => {
                self.defer_or_park_job(job, error)?;
                return Ok(OptimizerTick::NoWrite);
            }
        };

        if note.is_topic_hub() {
            self.complete_noop_job(&job.note_id)?;
            return Ok(OptimizerTick::NoWrite);
        }

        // The note's original frontmatter failed to parse and is preserved verbatim
        // (see `Note::frontmatter_raw_fallback`). `full_rewrite` mode would explicitly
        // set frontmatter-backed fields via `update_note`, clearing the fallback and
        // permanently destroying the unparsable original on write. Skip optimizing it
        // entirely until a human/editor fixes the YAML.
        if note.frontmatter_raw_fallback.is_some() {
            log::warn!(
                "Skipping vault optimizer run for note '{}': original frontmatter is unparsable and preserved verbatim",
                note.id
            );
            self.complete_noop_job(&job.note_id)?;
            return Ok(OptimizerTick::NoWrite);
        }

        // `background_vault_optimizer_llm_enabled` is meant to gate LLM-based
        // proposal enrichment. No such enrichment exists yet anywhere in this
        // service — `build_optimizer_proposal` below is purely rule-based and
        // never calls `OpenRouterService` or any other network client. The
        // flag is read here (rather than silently ignored) so the intended
        // integration point is explicit: a future LLM-backed enrichment step
        // MUST check this before making a network call. Today it has no
        // effect on behavior — see `run_next_ignores_llm_enabled_because_no_llm_path_exists`
        // below, which characterizes and locks in that fact.
        let _llm_enabled = settings.background_vault_optimizer_llm_enabled;

        let proposal = match build_optimizer_proposal(&note, store) {
            Ok(proposal) => proposal,
            Err(error) => {
                self.defer_or_park_job(job, error)?;
                return Ok(OptimizerTick::NoWrite);
            }
        };

        if proposal.is_empty() {
            self.complete_noop_job(&job.note_id)?;
            return Ok(OptimizerTick::NoWrite);
        }

        if self.daily_write_cap_reached(settings) {
            log::info!(
                "Vault optimizer deferring note '{}': daily write cap ({}) reached",
                note.id,
                settings.background_vault_optimizer_max_daily_writes
            );
            // Leave the job queued untouched; it's retried on a later tick
            // (possibly after the daily counter rolls over to a new date).
            return Ok(OptimizerTick::NoWrite);
        }

        let change_id = Uuid::new_v4().to_string();
        let publication_time = Utc::now();
        let decision = VaultOptimizerDecision {
            id: change_id.clone(),
            note_id: Some(note.id.clone()),
            kind: "optimizer_update".to_string(),
            confidence: proposal.confidence,
            reason: proposal.reason.clone(),
            diff_preview: proposal.diff_preview.clone(),
            created_at: Some(publication_time),
            change_id: Some(change_id.clone()),
        };

        Ok(OptimizerTick::Pending(Box::new(PendingOptimizerWrite {
            job,
            proposal,
            change_id,
            decision,
            edit_mode: settings.background_vault_optimizer_edit_mode.clone(),
            expected_authority,
            prepared_state_revision: self.state.state_revision,
        })))
    }

    /// Applies a `PendingOptimizerWrite` returned by [`Self::prepare_next`] for
    /// non-`sidecar_first` edit modes. This is the only step in the optimizer
    /// pipeline that needs a mutable, cache-rebuilding `KnowledgeStore` write
    /// lock (`update_note` walks and reparses the vault), so callers should
    /// hold that lock only around this call — not around `prepare_next`.
    ///
    /// Between `prepare_next` (read lock) and this call (write lock) the
    /// worker suspends at a real await point, so a concurrent user edit can
    /// land in the gap. The snapshot captured in `prepare_next` is therefore
    /// treated as stale by construction: the note is RE-FETCHED here under
    /// the write lock, and the proposal contributes only its additive deltas
    /// merged against the CURRENT `aliases`/`tags`/`properties` (and the
    /// current `relative_path` is left untouched). Applying against the
    /// snapshot instead would silently drop a tag the user just added or
    /// revert a rename they just made. If the re-fetch shows the note gone
    /// (deleted in the gap), the job is dropped like any missing note.
    ///
    /// Returns `Ok(Some(note_id))` when a write actually happened — the
    /// caller (`main.rs::start_vault_optimizer_worker`) must then refresh
    /// that note's search/chunk/topic-hub state (via
    /// `commands::commit_note_index_refresh`) once it has released the
    /// `knowledge_store`/`vault_optimizer` locks this call needed. Returns
    /// `Ok(None)` for every terminal no-write case (note deleted in the gap,
    /// frontmatter became unparsable in the gap, or the write itself failed
    /// and was deferred/parked) — nothing to reindex.
    pub fn apply_pending(
        &mut self,
        store: &mut KnowledgeStore,
        pending: PendingOptimizerWrite,
    ) -> Result<OptimizerMutationResult<OptimizerAppliedResult>> {
        let PendingOptimizerWrite {
            job,
            proposal,
            change_id,
            decision,
            edit_mode,
            expected_authority,
            prepared_state_revision,
        } = pending;

        if self.with_locked_fresh_state(|service| {
            Ok(service
                .load_pending_publications()?
                .into_iter()
                .any(|(_, pending)| pending.job.job_id == job.job_id))
        })? {
            return Ok(OptimizerMutationResult::NoWrite);
        }

        let snapshot = match store.optimizer_note_snapshot(&job.note_id) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                log::warn!(
                    "Skipping optimizer apply for note '{}': note disappeared between prepare and apply",
                    job.note_id
                );
                self.complete_noop_job_fresh(&job.job_id)?;
                return Ok(OptimizerMutationResult::NoWrite);
            }
            Err(error) => {
                self.defer_or_park_job_fresh(&job.job_id, &error)?;
                return Ok(OptimizerMutationResult::NoWrite);
            }
        };
        let crate::services::knowledge_store::OptimizerNoteSnapshot {
            note: current,
            markdown_precondition,
            markdown_raw_bytes,
            overlay_value,
            overlay_digest,
            overlay_raw_bytes,
        } = snapshot;

        // Same guard as `prepare_next`: an external edit in the gap may have
        // (re)introduced unparsable frontmatter that must be preserved
        // verbatim, and a full-rewrite `update_note` would destroy it.
        if current.frontmatter_raw_fallback.is_some() {
            log::warn!(
                "Skipping optimizer apply for note '{}': frontmatter became unparsable between prepare and apply",
                current.id
            );
            self.complete_noop_job_fresh(&job.job_id)?;
            return Ok(OptimizerMutationResult::NoWrite);
        }

        let refreshed_proposal = match build_optimizer_proposal(&current, store) {
            Ok(proposal) => proposal,
            Err(error) => {
                self.defer_or_park_job_fresh(&job.job_id, &error)?;
                return Ok(OptimizerMutationResult::NoWrite);
            }
        };
        let bound_sidecar_overlay = (edit_mode == "sidecar_first").then(|| {
            optimizer_sidecar_overlay(
                &proposal,
                markdown_precondition.relative_path(),
                markdown_precondition.expected_digest(),
            )
        });
        let sidecar_target_is_already_exact =
            bound_sidecar_overlay.as_ref().is_some_and(|overlay| {
                serde_json::to_vec_pretty(overlay)
                    .ok()
                    .map(|bytes| crate::services::twin_events::digest_bytes(&bytes))
                    .as_ref()
                    == overlay_digest.as_ref()
            });
        if refreshed_proposal != proposal && !sidecar_target_is_already_exact {
            let error =
                anyhow::anyhow!("optimizer source changed between proposal preparation and apply");
            self.defer_or_park_job_fresh(&job.job_id, &error)?;
            return Ok(OptimizerMutationResult::NoWrite);
        }
        if sidecar_target_is_already_exact {
            // The external writer satisfied the exact proposal before this
            // optimizer acquired an owner fence. Complete the queue item as a
            // no-op; the strict coordinator path intentionally refuses to
            // claim already-present bytes without an owned receipt.
            self.complete_noop_job_fresh(&job.job_id)?;
            return Ok(OptimizerMutationResult::NoWrite);
        }
        let proposal = refreshed_proposal;

        #[cfg(test)]
        if edit_mode != "sidecar_first" {
            if let Some((entered, resume)) = self.pause_before_markdown_digest_once.take() {
                entered.wait();
                resume.wait();
            }
        }

        let update = optimizer_note_update(&current, &proposal);
        let publication_inputs = (|| -> Result<(
            OptimizerPublicationTarget,
            OptimizerChange,
            Option<Note>,
            Option<Value>,
            NoteUpdate,
            Note,
        )> {

            if edit_mode == "sidecar_first" {
                let overlay_before = overlay_value;
                let before_digest = overlay_digest;
                let source_relative_path = markdown_precondition.relative_path().to_string();
                let source_digest = markdown_precondition.expected_digest().clone();
                let overlay_after = bound_sidecar_overlay
                    .clone()
                    .expect("sidecar overlay was constructed above");
                let after_bytes = serde_json::to_vec_pretty(&overlay_after)?;
                let after_digest = crate::services::twin_events::digest_bytes(&after_bytes);
                let restore_before = before_digest.as_ref().map_or(
                    crate::services::twin_events::BeforeImage::Absent,
                    |digest| crate::services::twin_events::BeforeImage::Sha256(digest.clone()),
                );
                let apply_after =
                    crate::services::twin_events::BeforeImage::Sha256(after_digest.clone());
                let restore_utf8 = overlay_raw_bytes
                    .map(String::from_utf8)
                    .transpose()
                    .context("optimizer overlay source is not UTF-8")?;
                let rollback_governance = store.optimizer_overlay_governance(
                    &source_relative_path,
                    &markdown_raw_bytes,
                    restore_utf8.as_deref().map(str::as_bytes),
                )?;
                let apply_governance = store.optimizer_overlay_governance(
                    &source_relative_path,
                    &markdown_raw_bytes,
                    Some(&after_bytes),
                )?;
                let apply_payload_digest = rollback::effective_sidecar_digest(
                    &source_digest,
                    &apply_after,
                );
                let apply_evidence_digest = rollback::effective_sidecar_digest(
                    &source_digest,
                    &restore_before,
                );
                Ok((
                    OptimizerPublicationTarget::Overlay {
                        note_id: current.id.clone(),
                        before_digest,
                        after_digest,
                        source_relative_path: source_relative_path.clone(),
                        source_digest: Some(source_digest.clone()),
                    },
                    OptimizerChange {
                        change_id: change_id.clone(),
                        note_id: current.id.clone(),
                        mode: edit_mode,
                        overlay_before,
                        overlay_after: Some(overlay_after.clone()),
                        markdown_before_digest: Some(source_digest),
                        markdown_relative_path: Some(source_relative_path),
                        created_at: decision.created_at,
                        exact_rollback: Some(rollback::ExactOptimizerRollbackMaterialV1 {
                            schema_version:
                                rollback::EXACT_OPTIMIZER_ROLLBACK_SCHEMA_VERSION,
                            target_kind:
                                crate::services::twin_events::TargetKind::OverlayJson,
                            target_key: format!("{}.json", current.id),
                            restore_before,
                            restore_utf8,
                            apply_after,
                            source_relative_path: markdown_precondition
                                .relative_path()
                                .to_string(),
                            source_digest: markdown_precondition.expected_digest().clone(),
                            apply_payload_digest: apply_payload_digest.clone(),
                            apply_evidence_digest: apply_evidence_digest.clone(),
                            rollback_payload_digest: apply_evidence_digest,
                            rollback_evidence_digest: apply_payload_digest,
                            apply_governance,
                            rollback_governance,
                        }),
                        ..Default::default()
                    },
                    None,
                    Some(overlay_after),
                    update.clone(),
                    current.clone(),
                ))
            } else {
                let before_path = store.serialized_note_bytes(&current)?.0;
                let before_digest = markdown_precondition.expected_digest().clone();
                let source_update = optimizer_note_update(&current, &proposal);
                let exact =
                    store.materialize_note_update_at(&current, source_update.clone(), Utc::now())?;
                let (after_path, after_bytes) = store.serialized_note_bytes(&exact)?;
                if before_path != after_path {
                    anyhow::bail!("optimizer rewrite unexpectedly changed the note path");
                }
                let after_digest = crate::services::twin_events::digest_bytes(&after_bytes);
                let restore_utf8 = String::from_utf8(markdown_raw_bytes)
                    .context("optimizer Markdown source is not UTF-8")?;
                Ok((
                    OptimizerPublicationTarget::Markdown {
                        relative_path: before_path.clone(),
                        before_digest: before_digest.clone(),
                        after_digest: after_digest.clone(),
                    },
                    OptimizerChange {
                        change_id: change_id.clone(),
                        note_id: current.id.clone(),
                        mode: edit_mode,
                        note_before: Some(current.clone()),
                        note_after: Some(exact.clone()),
                        markdown_before_digest: Some(before_digest.clone()),
                        markdown_relative_path: Some(before_path.clone()),
                        created_at: decision.created_at,
                        exact_rollback: Some(rollback::ExactOptimizerRollbackMaterialV1 {
                            schema_version:
                                rollback::EXACT_OPTIMIZER_ROLLBACK_SCHEMA_VERSION,
                            target_kind: crate::services::twin_events::TargetKind::Markdown,
                            target_key: before_path.clone(),
                            restore_before:
                                crate::services::twin_events::BeforeImage::Sha256(
                                    before_digest.clone(),
                                ),
                            restore_utf8: Some(restore_utf8),
                            apply_after:
                                crate::services::twin_events::BeforeImage::Sha256(
                                    after_digest.clone(),
                                ),
                            source_relative_path: before_path,
                            source_digest: before_digest.clone(),
                            apply_payload_digest: after_digest.clone(),
                            apply_evidence_digest: before_digest.clone(),
                            rollback_payload_digest: before_digest,
                            rollback_evidence_digest: after_digest,
                            apply_governance: store.optimizer_note_governance(&exact),
                            rollback_governance: store.optimizer_note_governance(&current),
                        }),
                        ..Default::default()
                    },
                    Some(exact),
                    None,
                    source_update,
                    current.clone(),
                ))
            }
        })();
        let (target, change, _exact_note, _overlay_after, publication_update, publication_note) =
            match publication_inputs {
                Ok(inputs) => inputs,
                Err(error) => {
                    self.defer_or_park_job_fresh(&job.job_id, &error)?;
                    return Ok(OptimizerMutationResult::NoWrite);
                }
            };

        let publication = PendingOptimizerPublication {
            schema_version: 1,
            phase: OptimizerPublicationPhase::RetryFenced,
            retry_fenced: true,
            audit_written: false,
            counted: false,
            queue_removed: false,
            abort_queue_reconciled: false,
            expected_authority: expected_authority.clone(),
            mutation_id: None,
            committed_authority: None,
            target,
            job: job.clone(),
            note: publication_note,
            change_id: change_id.to_string(),
            decision,
            change,
        };
        let stage_result = self.with_locked_fresh_state(|service| {
            if let Some((_, existing)) = service
                .load_pending_publications()?
                .into_iter()
                .find(|(_, pending)| pending.job.job_id == job.job_id)
            {
                if existing.change_id == publication.change_id
                    && serde_json::to_value(&existing)? != serde_json::to_value(&publication)?
                {
                    anyhow::bail!("optimizer retry-fence identity collision");
                }
                return Ok(false);
            }
            if service.state.state_revision != prepared_state_revision
                || !service
                    .state
                    .queue
                    .iter()
                    .any(|queued| queued.job_id == job.job_id && queued.note_id == job.note_id)
            {
                anyhow::bail!("vault optimizer state changed before pending apply");
            }
            if expected_authority.is_some() {
                service.preflight_publication_audit(&publication)?;
                service.stage_pending_publication(&publication)?;
                #[cfg(test)]
                if std::mem::take(&mut service.fail_retry_fence_stage_after_write_once) {
                    anyhow::bail!("injected optimizer RetryFenced stage ambiguity");
                }
            }
            Ok(true)
        });
        match stage_result {
            Ok(false) => return Ok(OptimizerMutationResult::NoWrite),
            Ok(true) => {}
            Err(error) => {
                if expected_authority.is_some() {
                    self.preserve_retry_fence_or_defer(&change_id, &job.job_id, &error)?;
                } else {
                    self.defer_or_park_job_fresh(&job.job_id, &error)?;
                }
                return Ok(OptimizerMutationResult::NoWrite);
            }
        }
        #[cfg(test)]
        if let Some((entered, resume)) = self.pause_after_retry_fence_once.take() {
            entered.wait();
            resume.wait();
        }
        self.commit_staged_publication(store, publication, Some(publication_update))
    }

    pub fn apply_retry_fenced(
        &mut self,
        store: &mut KnowledgeStore,
        pending: RetryFencedOptimizerWrite,
    ) -> Result<OptimizerMutationResult<OptimizerAppliedResult>> {
        let publication = pending.publication;
        self.with_locked_fresh_state(|service| {
            let persisted = service
                .load_pending_publications()?
                .into_iter()
                .find(|(_, candidate)| candidate.change_id == publication.change_id)
                .map(|(_, candidate)| candidate)
                .ok_or_else(|| anyhow::anyhow!("optimizer retry fence no longer exists"))?;
            if persisted.phase != OptimizerPublicationPhase::RetryFenced
                || serde_json::to_value(&persisted)? != serde_json::to_value(&publication)?
            {
                anyhow::bail!("optimizer retry fence changed before resume");
            }
            if !service
                .state
                .queue
                .iter()
                .any(|job| job.job_id == publication.job.job_id)
            {
                anyhow::bail!("optimizer retry-fenced queue job no longer exists");
            }
            service.preflight_publication_audit(&publication)
        })?;
        self.commit_staged_publication(store, publication, None)
    }
}

fn parse_canonical_uuid(value: &str, label: &str) -> Result<Uuid> {
    let parsed = Uuid::parse_str(value).with_context(|| format!("invalid {label}"))?;
    if parsed.to_string() != value {
        anyhow::bail!("noncanonical {label}");
    }
    Ok(parsed)
}

fn publication_inbox_entry(
    pending: &PendingOptimizerPublication,
) -> Result<VaultOptimizerInboxEntry> {
    let created_at = pending
        .decision
        .created_at
        .ok_or_else(|| anyhow::anyhow!("optimizer publication lacks its stable time"))?;
    Ok(VaultOptimizerInboxEntry {
        id: pending.change_id.clone(),
        note_id: Some(pending.note.id.clone()),
        status: "applied".to_string(),
        title: pending.note.title.clone(),
        reason: pending.decision.reason.clone(),
        diff_preview: pending.decision.diff_preview.clone(),
        confidence: pending.decision.confidence,
        created_at: Some(created_at),
        change_id: Some(pending.change_id.clone()),
    })
}

fn publication_audit_event(pending: &PendingOptimizerPublication) -> Result<OptimizerAuditEventV1> {
    let created_at = pending
        .decision
        .created_at
        .ok_or_else(|| anyhow::anyhow!("optimizer publication lacks its stable time"))?;
    Ok(OptimizerAuditEventV1::OptimizerApply {
        note_id: pending.note.id.clone(),
        change_id: pending.change_id.clone(),
        at: created_at,
        confidence: pending.decision.confidence,
    })
}

fn merge_optimizer_decision(
    values: &mut Vec<VaultOptimizerDecision>,
    candidate: VaultOptimizerDecision,
) -> Result<()> {
    if let Some(existing) = values
        .iter()
        .find(|existing| existing.id == candidate.id || existing.change_id == candidate.change_id)
    {
        if existing == &candidate {
            return Ok(());
        }
        anyhow::bail!("optimizer decision identity collision");
    }
    values.push(candidate);
    Ok(())
}

fn merge_optimizer_inbox(
    values: &mut Vec<VaultOptimizerInboxEntry>,
    candidate: VaultOptimizerInboxEntry,
) -> Result<()> {
    if let Some(existing) = values.iter().find(|existing| {
        existing.id == candidate.id
            || (candidate.change_id.is_some() && existing.change_id == candidate.change_id)
    }) {
        if existing == &candidate {
            return Ok(());
        }
        anyhow::bail!("optimizer inbox identity collision");
    }
    values.push(candidate);
    Ok(())
}

fn merge_optimizer_event(
    values: &mut Vec<OptimizerAuditEventV1>,
    candidate: OptimizerAuditEventV1,
) -> Result<()> {
    let candidate_change_id = match &candidate {
        OptimizerAuditEventV1::OptimizerApply { change_id, .. }
        | OptimizerAuditEventV1::Rollback { change_id, .. } => Some(change_id.as_str()),
        OptimizerAuditEventV1::OptimizerParked { job_id, .. } if !job_id.is_empty() => {
            Some(job_id.as_str())
        }
        OptimizerAuditEventV1::OptimizerParked { .. } => None,
    };
    if let Some(change_id) = candidate_change_id {
        if let Some(existing) = values.iter().find(|existing| {
            matches!(
                (existing, &candidate),
                (
                    OptimizerAuditEventV1::OptimizerApply {
                        change_id: existing_id,
                        ..
                    },
                    OptimizerAuditEventV1::OptimizerApply { .. }
                ) if existing_id == change_id
            ) || matches!(
                (existing, &candidate),
                (
                    OptimizerAuditEventV1::Rollback {
                        change_id: existing_id,
                        ..
                    },
                    OptimizerAuditEventV1::Rollback { .. }
                ) if existing_id == change_id
            ) || matches!(
                (existing, &candidate),
                (
                    OptimizerAuditEventV1::OptimizerParked {
                        job_id: existing_id,
                        ..
                    },
                    OptimizerAuditEventV1::OptimizerParked { .. }
                ) if !existing_id.is_empty() && existing_id == change_id
            )
        }) {
            if existing == &candidate {
                return Ok(());
            }
            anyhow::bail!("optimizer event audit identity collision");
        }
    }
    values.push(candidate);
    Ok(())
}

fn merge_optimizer_change(
    values: &mut HashMap<String, OptimizerChange>,
    candidate: OptimizerChange,
) -> Result<()> {
    if let Some(existing) = values.get(&candidate.change_id) {
        if serde_json::to_value(existing)? == serde_json::to_value(&candidate)? {
            return Ok(());
        }
        anyhow::bail!("optimizer change identity collision");
    }
    values.insert(candidate.change_id.clone(), candidate);
    Ok(())
}

fn validate_json_audit_capacity<T: Serialize>(values: &[T], label: &str) -> Result<()> {
    if values.len() > MAX_OPTIMIZER_AUDIT_ENTRIES {
        anyhow::bail!("{label} exceed 4096 entries");
    }
    if serde_json::to_vec_pretty(values)?.len() > MAX_OPTIMIZER_AUDIT_BYTES {
        anyhow::bail!("{label} exceed the 4 MiB limit");
    }
    Ok(())
}

fn serialize_optimizer_events(events: &[OptimizerAuditEventV1]) -> Result<Vec<u8>> {
    if events.len() > MAX_OPTIMIZER_AUDIT_ENTRIES {
        anyhow::bail!("optimizer event audit exceeds 4096 entries");
    }
    let mut bytes = Vec::new();
    for event in events {
        serde_json::to_writer(&mut bytes, event)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_OPTIMIZER_AUDIT_BYTES {
            anyhow::bail!("optimizer event audit exceeds its 4 MiB limit");
        }
    }
    Ok(bytes)
}

fn normalize_optimizer_job_ids(state: &mut OptimizerState) -> Result<()> {
    let mut seen = HashSet::new();
    for (index, job) in state.queue.iter_mut().enumerate() {
        if job.job_id.is_empty() {
            let mut hasher = Sha256::new();
            hasher.update(b"grafyn.optimizer.legacy-job.v1");
            hasher.update((index as u64).to_be_bytes());
            hasher.update((job.note_id.len() as u64).to_be_bytes());
            hasher.update(job.note_id.as_bytes());
            hasher.update((job.reason.len() as u64).to_be_bytes());
            hasher.update(job.reason.as_bytes());
            hasher.update(job.enqueued_at.timestamp_millis().to_be_bytes());
            let digest = hasher.finalize();
            let mut bytes = [0_u8; 16];
            bytes.copy_from_slice(&digest[..16]);
            bytes[6] = (bytes[6] & 0x0f) | 0x50;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            job.job_id = Uuid::from_bytes(bytes).to_string();
        } else {
            parse_canonical_uuid(&job.job_id, "optimizer job ID")?;
        }
        if !seen.insert(job.job_id.clone()) {
            anyhow::bail!("optimizer queue contains duplicate job IDs");
        }
    }
    Ok(())
}

fn load_optimizer_state(
    root: &crate::services::twin_events::AnchoredRoot,
) -> Result<OptimizerState> {
    let Some(bytes) = root
        .read_bounded(QUEUE_KEY, MAX_OPTIMIZER_STATE_BYTES)
        .map_err(anyhow::Error::new)?
    else {
        return Ok(OptimizerState::default());
    };
    serde_json::from_slice(&bytes).context("invalid optimizer queue state")
}

fn load_bounded_json_vec<T: serde::de::DeserializeOwned>(
    root: &crate::services::twin_events::AnchoredRoot,
    key: &str,
    label: &str,
) -> Result<Vec<T>> {
    let Some(bytes) = root
        .read_bounded(key, MAX_OPTIMIZER_AUDIT_BYTES)
        .map_err(anyhow::Error::new)?
    else {
        return Ok(Vec::new());
    };
    let values: Vec<T> =
        serde_json::from_slice(&bytes).with_context(|| format!("invalid {label}"))?;
    if values.len() > MAX_OPTIMIZER_AUDIT_ENTRIES {
        anyhow::bail!("{label} exceed 4096 entries");
    }
    Ok(values)
}

fn write_bounded_json_vec<T: Serialize>(
    root: &crate::services::twin_events::AnchoredRoot,
    key: &str,
    values: &[T],
) -> Result<()> {
    if values.len() > MAX_OPTIMIZER_AUDIT_ENTRIES {
        anyhow::bail!("optimizer audit exceeds 4096 entries");
    }
    let bytes = serde_json::to_vec_pretty(values)?;
    if bytes.len() > MAX_OPTIMIZER_AUDIT_BYTES {
        anyhow::bail!("optimizer audit exceeds its 4 MiB limit");
    }
    root.put_atomic(key, &bytes).map_err(anyhow::Error::new)
}

fn pending_publication_key(change_id: &str) -> Result<String> {
    parse_canonical_uuid(change_id, "optimizer change ID")?;
    Ok(format!("{PENDING_PUBLICATIONS_DIRECTORY}/{change_id}.json"))
}

fn cleanup_optimizer_orphan_temps(root: &crate::services::twin_events::AnchoredRoot) -> Result<()> {
    for (directory, durable_limit) in [
        (PENDING_PUBLICATIONS_DIRECTORY, MAX_PENDING_PUBLICATIONS),
        (
            rollback::PENDING_ROLLBACKS_DIRECTORY,
            rollback::MAX_PENDING_ROLLBACKS,
        ),
        (CHANGES_DIRECTORY, MAX_OPTIMIZER_AUDIT_ENTRIES),
    ] {
        let names = root
            .regular_file_names_bounded(directory, durable_limit + MAX_OPTIMIZER_ORPHAN_TEMPS)
            .map_err(anyhow::Error::new)?;
        if names.len() > durable_limit + MAX_OPTIMIZER_ORPHAN_TEMPS {
            anyhow::bail!("optimizer {directory} directory exceeds its bounded entry limit");
        }
        let orphans = names
            .into_iter()
            .filter(|name| {
                name.strip_prefix('.')
                    .and_then(|name| name.strip_suffix(".tmp"))
                    .is_some_and(|id| parse_canonical_uuid(id, "optimizer orphan temp ID").is_ok())
            })
            .collect::<Vec<_>>();
        if orphans.len() > MAX_OPTIMIZER_ORPHAN_TEMPS {
            anyhow::bail!("optimizer {directory} has too many orphan temporary files");
        }
        for name in orphans {
            root.delete(&format!("{directory}/{name}"))
                .map_err(anyhow::Error::new)?;
        }
    }
    Ok(())
}

fn validate_pending_publication(pending: &PendingOptimizerPublication) -> Result<()> {
    if pending.schema_version != 1 {
        anyhow::bail!("unsupported optimizer pending-publication schema");
    }
    pending_publication_key(&pending.change_id)?;
    parse_canonical_uuid(&pending.job.job_id, "optimizer job ID")?;
    if pending.job.note_id != pending.note.id
        || pending.decision.id != pending.change_id
        || pending.decision.change_id.as_deref() != Some(pending.change_id.as_str())
        || pending.decision.note_id.as_deref() != Some(pending.note.id.as_str())
        || pending.decision.kind != "optimizer_update"
        || pending.change.change_id != pending.change_id
        || pending.change.note_id != pending.note.id
        || pending.expected_authority.is_none()
        || !pending.retry_fenced
        || pending.decision.created_at.is_none()
        || pending.change.created_at != pending.decision.created_at
        || pending.counted && !pending.audit_written
        || pending.queue_removed && !pending.counted
    {
        anyhow::bail!("invalid optimizer pending-publication identity");
    }
    match (&pending.target, &pending.change) {
        (
            OptimizerPublicationTarget::Overlay {
                note_id,
                before_digest,
                after_digest,
                source_relative_path,
                source_digest,
            },
            change,
        ) => {
            let after = change.overlay_after.as_ref().ok_or_else(|| {
                anyhow::anyhow!("optimizer overlay publication lacks its exact after image")
            })?;
            let after_bytes = serde_json::to_vec_pretty(after)?;
            let source_digest = source_digest.as_ref().ok_or_else(|| {
                anyhow::anyhow!("optimizer overlay publication lacks its source digest")
            })?;
            let provenance = after
                .get("_grafyn_optimizer_source_v1")
                .and_then(Value::as_object);
            if note_id != &pending.note.id
                || source_relative_path != &pending.note.relative_path
                || change.mode != "sidecar_first"
                || change.note_before.is_some()
                || change.note_after.is_some()
                || change.markdown_before_digest.as_ref() != Some(source_digest)
                || change.markdown_relative_path.as_deref() != Some(source_relative_path.as_str())
                || change.overlay_before.is_some() != before_digest.is_some()
                || provenance
                    .and_then(|value| value.get("relative_path"))
                    .and_then(Value::as_str)
                    != Some(source_relative_path.as_str())
                || provenance
                    .and_then(|value| value.get("sha256"))
                    .and_then(Value::as_str)
                    != Some(source_digest.as_str())
                || crate::services::twin_events::digest_bytes(&after_bytes) != *after_digest
            {
                anyhow::bail!("invalid optimizer overlay-publication binding");
            }
        }
        (
            OptimizerPublicationTarget::Markdown {
                relative_path,
                before_digest,
                after_digest,
            },
            change,
        ) => {
            let before = change.note_before.as_ref().ok_or_else(|| {
                anyhow::anyhow!("optimizer Markdown publication lacks its exact before note")
            })?;
            let after = change.note_after.as_ref().ok_or_else(|| {
                anyhow::anyhow!("optimizer Markdown publication lacks its exact after note")
            })?;
            let (after_path, after_bytes) = KnowledgeStore::canonical_serialized_note_bytes(after)?;
            if change.mode != "full_rewrite"
                || change.overlay_before.is_some()
                || change.overlay_after.is_some()
                || change.markdown_relative_path.as_deref() != Some(relative_path.as_str())
                || serde_json::to_value(before)? != serde_json::to_value(&pending.note)?
                || change.markdown_before_digest.as_ref() != Some(before_digest)
                || before.id != pending.note.id
                || after.id != pending.note.id
                || &after_path != relative_path
                || crate::services::twin_events::digest_bytes(&after_bytes) != *after_digest
            {
                anyhow::bail!("invalid optimizer Markdown-publication binding");
            }
        }
    }
    if let Some(material) = pending.change.exact_rollback.as_ref() {
        material.validate(&pending.change)?;
        let target_matches = match &pending.target {
            OptimizerPublicationTarget::Overlay {
                note_id,
                after_digest,
                ..
            } => {
                material.target_kind == crate::services::twin_events::TargetKind::OverlayJson
                    && material.target_key == format!("{note_id}.json")
                    && material.apply_after
                        == crate::services::twin_events::BeforeImage::Sha256(after_digest.clone())
            }
            OptimizerPublicationTarget::Markdown {
                relative_path,
                after_digest,
                ..
            } => {
                material.target_kind == crate::services::twin_events::TargetKind::Markdown
                    && material.target_key == *relative_path
                    && material.apply_after
                        == crate::services::twin_events::BeforeImage::Sha256(after_digest.clone())
            }
        };
        if !target_matches {
            anyhow::bail!("optimizer exact rollback does not bind its publication target");
        }
    }
    match (
        pending.phase,
        pending.mutation_id.as_ref(),
        pending.committed_authority.as_ref(),
    ) {
        (OptimizerPublicationPhase::RetryFenced, None, None)
            if !pending.audit_written
                && !pending.counted
                && !pending.queue_removed
                && !pending.abort_queue_reconciled => {}
        (OptimizerPublicationPhase::Prepared, Some(_), None)
            if !pending.audit_written
                && !pending.counted
                && !pending.queue_removed
                && !pending.abort_queue_reconciled => {}
        (OptimizerPublicationPhase::Aborted, Some(_), committed)
            if !pending.audit_written && !pending.counted && !pending.queue_removed =>
        {
            if let Some(committed) = committed {
                let expected = pending.expected_authority.as_ref().unwrap();
                if committed.root_scope != expected.root_scope
                    || committed.lease_epoch_uuid != expected.lease_epoch_uuid
                    || Some(committed.authority_generation)
                        != expected.authority_generation.checked_add(1)
                {
                    anyhow::bail!("invalid optimizer aborted-publication authority");
                }
            }
        }
        (OptimizerPublicationPhase::Committed, Some(_), Some(committed)) => {
            if pending.abort_queue_reconciled {
                anyhow::bail!("invalid optimizer committed abort reconciliation");
            }
            let expected = pending.expected_authority.as_ref().unwrap();
            if committed.root_scope != expected.root_scope
                || committed.lease_epoch_uuid != expected.lease_epoch_uuid
                || Some(committed.authority_generation)
                    != expected.authority_generation.checked_add(1)
            {
                anyhow::bail!("invalid optimizer committed-publication authority");
            }
        }
        _ => anyhow::bail!("invalid optimizer pending-publication phase fields"),
    }
    Ok(())
}

fn write_pending_publication(
    root: &crate::services::twin_events::AnchoredRoot,
    pending: &PendingOptimizerPublication,
) -> Result<()> {
    validate_pending_publication(pending)?;
    let bytes = serde_json::to_vec_pretty(pending)?;
    if bytes.len() > MAX_PENDING_PUBLICATION_BYTES {
        anyhow::bail!("optimizer pending publication exceeds its 16 MiB limit");
    }
    root.open_directory(PENDING_PUBLICATIONS_DIRECTORY, true)
        .map_err(anyhow::Error::new)?;
    let names = root
        .regular_file_names(PENDING_PUBLICATIONS_DIRECTORY)
        .map_err(anyhow::Error::new)?;
    let key = pending_publication_key(&pending.change_id)?;
    let filename = format!("{}.json", pending.change_id);
    if names.len() >= MAX_PENDING_PUBLICATIONS && !names.contains(&filename) {
        anyhow::bail!("optimizer pending publications have reached 64 entries");
    }
    root.put_atomic(&key, &bytes).map_err(anyhow::Error::new)
}

fn remove_pending_publication(
    root: &crate::services::twin_events::AnchoredRoot,
    change_id: &str,
) -> Result<()> {
    root.delete(&pending_publication_key(change_id)?)
        .map_err(anyhow::Error::new)
}

fn load_pending_publications(
    root: &crate::services::twin_events::AnchoredRoot,
) -> Result<Vec<(String, PendingOptimizerPublication)>> {
    let mut names = root
        .regular_file_names(PENDING_PUBLICATIONS_DIRECTORY)
        .map_err(anyhow::Error::new)?;
    if names.len() > MAX_PENDING_PUBLICATIONS {
        anyhow::bail!("optimizer pending publications exceed 64 entries");
    }
    names.sort();
    let mut loaded = Vec::with_capacity(names.len());
    let mut owned_jobs = HashSet::new();
    for name in names {
        let change_id = name
            .strip_suffix(".json")
            .ok_or_else(|| anyhow::anyhow!("invalid optimizer pending-publication filename"))?;
        parse_canonical_uuid(change_id, "optimizer pending-publication filename")?;
        let key = format!("{PENDING_PUBLICATIONS_DIRECTORY}/{name}");
        let bytes = root
            .read_bounded(&key, MAX_PENDING_PUBLICATION_BYTES)
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow::anyhow!("optimizer pending publication disappeared"))?;
        let pending: PendingOptimizerPublication =
            serde_json::from_slice(&bytes).context("invalid optimizer pending publication")?;
        if pending.change_id != change_id {
            anyhow::bail!("invalid optimizer pending-publication filename identity");
        }
        validate_pending_publication(&pending)?;
        if !owned_jobs.insert(pending.job.job_id.clone()) {
            anyhow::bail!("optimizer job has multiple pending-publication owners");
        }
        loaded.push((key, pending));
    }
    Ok(loaded)
}

/// Outcome of a single [`VaultOptimizerService::prepare_next`] tick.
///
/// `prepare_next` handles every case that's resolvable under a read lock on
/// `KnowledgeStore` — including the common `sidecar_first` edit mode, which
/// applies its overlay write inline rather than deferring to a second stage.
/// Only a non-`sidecar_first` edit mode needs the caller to come back with a
/// write lock via [`VaultOptimizerService::apply_pending`].
#[derive(Debug)]
pub enum OptimizerTick {
    /// Nothing to do this tick: optimizer disabled, empty queue, a
    /// missing/topic-hub/unparsable-frontmatter/no-op note (all terminal —
    /// dequeued inside `prepare_next`), the daily write cap reached, or a
    /// transient processing error that was recorded and deferred/parked.
    /// Nothing was written, so there is nothing to reindex.
    NoWrite,
    /// A durable pre-effect witness owns the queue job. Any process may
    /// resume this exact publication; the shared mutation coordinator
    /// serializes the authority CAS, while the stable witness keeps its audit
    /// reservation across crashes and rebuilds.
    RetryFenced(Box<RetryFencedOptimizerWrite>),
    /// The governed authority write committed. The caller must repair from
    /// this exact commit token even when best-effort optimizer publication
    /// returned a warning.
    Committed {
        result: OptimizerAppliedResult,
        commit: crate::services::twin_events::MutationCommit,
        warning: Option<crate::models::mutation::CommittedMutationWarningV1>,
    },
    /// A non-`sidecar_first` edit mode computed a proposal but needs a real
    /// `KnowledgeStore::update_note` rewrite to apply it. Pass this to
    /// [`VaultOptimizerService::apply_pending`] under a write lock.
    Pending(Box<PendingOptimizerWrite>),
}

#[derive(Debug, Clone)]
pub struct RetryFencedOptimizerWrite {
    publication: PendingOptimizerPublication,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizerAppliedResult {
    note_id: String,
    change_id: String,
}

impl OptimizerAppliedResult {
    pub fn note_id(&self) -> &str {
        &self.note_id
    }

    pub fn change_id(&self) -> &str {
        &self.change_id
    }
}

#[must_use = "committed optimizer writes must consume their exact mutation commit"]
#[derive(Debug)]
pub enum OptimizerMutationResult<T> {
    NoWrite,
    Committed {
        result: T,
        commit: crate::services::twin_events::MutationCommit,
        warning: Option<crate::models::mutation::CommittedMutationWarningV1>,
    },
}

#[must_use = "optimizer rollback authority changes must consume their exact mutation commit"]
#[derive(Debug)]
pub(crate) enum OptimizerRollbackMutationOutcome {
    NoWrite(VaultOptimizerRollbackResult),
    Committed {
        result: VaultOptimizerRollbackResult,
        commit: crate::services::twin_events::MutationCommit,
        warning: Option<crate::models::mutation::CommittedMutationWarningV1>,
    },
    Partial {
        result: VaultOptimizerRollbackResult,
        commit: crate::services::twin_events::MutationCommit,
        warning: crate::models::mutation::CommittedMutationWarningV1,
        recovery_pending: bool,
    },
}

/// A rules-based proposal that was accepted but needs a mutable, cache-
/// rebuilding `KnowledgeStore::update_note` write to apply (i.e. edit_mode is
/// something other than `sidecar_first`, which applies inline within
/// `prepare_next` instead). Returned by [`VaultOptimizerService::prepare_next`]
/// (wrapped in [`OptimizerTick::Pending`]) and consumed by
/// [`VaultOptimizerService::apply_pending`].
///
/// Deliberately does NOT carry the note snapshot from the prepare stage:
/// `apply_pending` re-fetches the note under the write lock and merges the
/// proposal's additive deltas against the note's current state, so an
/// interleaved user edit between the two stages is never overwritten.
#[derive(Debug, Clone)]
pub struct PendingOptimizerWrite {
    job: QueuedOptimizerNote,
    proposal: OptimizerProposal,
    change_id: String,
    decision: VaultOptimizerDecision,
    edit_mode: String,
    expected_authority: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    prepared_state_revision: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct OptimizerProposal {
    aliases: Vec<String>,
    tags: Vec<String>,
    properties: HashMap<String, Value>,
    confidence: f64,
    reason: String,
    diff_preview: String,
}

impl OptimizerProposal {
    fn is_empty(&self) -> bool {
        self.aliases.is_empty() && self.tags.is_empty() && self.properties.is_empty()
    }
}

fn optimizer_sidecar_overlay(
    proposal: &OptimizerProposal,
    source_relative_path: &str,
    source_digest: &crate::models::twin_event::ContentDigest,
) -> Value {
    json!({
        "aliases": proposal.aliases,
        "tags": proposal.tags,
        "schema_version": CURRENT_NOTE_SCHEMA_VERSION,
        "migration_source": "vault_optimizer",
        "optimizer_managed": false,
        "properties": proposal.properties,
        "_grafyn_optimizer_source_v1": {
            "relative_path": source_relative_path,
            "sha256": source_digest,
        },
    })
}

fn build_optimizer_proposal(note: &Note, store: &KnowledgeStore) -> Result<OptimizerProposal> {
    let mut aliases = note.aliases.clone();
    let title_alias = note.title.replace(':', " ").replace("  ", " ");
    if !title_alias.eq_ignore_ascii_case(&note.title) {
        aliases.push(title_alias);
    }

    let mut tags = note.tags.clone();
    let topic_key = normalize_topic_key(&note.title);
    if !topic_key.is_empty() && !tags.iter().any(|tag| normalize_topic_key(tag) == topic_key) {
        tags.push(topic_key.replace('-', "_"));
    }

    let mut inferred_link_ids = Vec::new();
    for candidate in store.list_notes()? {
        if candidate.id == note.id || candidate.title.len() < 6 {
            continue;
        }
        if note
            .content
            .to_lowercase()
            .contains(&candidate.title.to_lowercase())
        {
            inferred_link_ids.push(candidate.id);
        }
    }
    inferred_link_ids.truncate(3);

    let merged_aliases = merge_unique_strings(Vec::new(), aliases);
    let merged_tags = merge_unique_strings(Vec::new(), tags);
    let new_aliases = merged_aliases
        .into_iter()
        .filter(|alias| {
            !note
                .aliases
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(alias))
        })
        .collect::<Vec<_>>();
    let new_tags = merged_tags
        .into_iter()
        .filter(|tag| {
            !note
                .tags
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(tag))
        })
        .collect::<Vec<_>>();

    let mut properties = HashMap::new();
    if !topic_key.is_empty() {
        properties.insert(PROP_TOPIC_KEY.to_string(), Value::String(topic_key.clone()));
        properties.insert(
            PROP_TOPIC_ALIASES.to_string(),
            Value::Array(vec![Value::String(note.title.clone())]),
        );
    }
    if !inferred_link_ids.is_empty() {
        properties.insert(
            PROP_INFERRED_LINK_IDS.to_string(),
            Value::Array(inferred_link_ids.into_iter().map(Value::String).collect()),
        );
    }

    Ok(OptimizerProposal {
        aliases: new_aliases.clone(),
        tags: new_tags.clone(),
        properties,
        confidence: 0.82,
        reason: "Inferred aliases, tags, and note relationships from vault context".to_string(),
        diff_preview: format!(
            "aliases +{} | tags +{} | inferred signals {}",
            new_aliases.len(),
            new_tags.len(),
            if note.content.len() > 400 {
                "updated"
            } else {
                "checked"
            }
        ),
    })
}

fn merge_note_properties(
    mut existing: HashMap<String, Value>,
    additions: HashMap<String, Value>,
) -> HashMap<String, Value> {
    for (key, value) in additions {
        existing.insert(key, value);
    }
    existing
}

fn optimizer_note_update(current: &Note, proposal: &OptimizerProposal) -> NoteUpdate {
    NoteUpdate {
        title: None,
        content: None,
        relative_path: None,
        aliases: Some(merge_unique_strings(
            current.aliases.clone(),
            proposal.aliases.clone(),
        )),
        status: None,
        tags: Some(merge_unique_strings(
            current.tags.clone(),
            proposal.tags.clone(),
        )),
        schema_version: Some(CURRENT_NOTE_SCHEMA_VERSION),
        migration_source: Some("vault_optimizer".to_string()),
        optimizer_managed: Some(false),
        properties: Some(merge_note_properties(
            current.properties.clone(),
            proposal.properties.clone(),
        )),
    }
}

fn merge_unique_strings(existing: Vec<String>, additions: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for value in existing.into_iter().chain(additions) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let owned = trimmed.to_string();
        if seen.insert(owned.to_lowercase()) {
            values.push(owned);
        }
    }
    values
}

#[cfg(test)]
#[path = "vault_optimizer_tests.rs"]
mod tests;
