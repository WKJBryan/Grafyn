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
const MAX_PENDING_PUBLICATION_BYTES: usize = 1024 * 1024;
const MAX_OPTIMIZER_STATE_BYTES: usize = 4 * 1024 * 1024;
const MAX_OPTIMIZER_AUDIT_BYTES: usize = 4 * 1024 * 1024;
const MAX_OPTIMIZER_AUDIT_ENTRIES: usize = 4096;
const MAX_OPTIMIZER_CHANGE_BYTES: usize = 1024 * 1024;
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
    Committed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum OptimizerAuditEventV1 {
    Rollback {
        change_id: String,
        at: DateTime<Utc>,
    },
    OptimizerParked {
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
    queue_path: PathBuf,
    decisions_path: PathBuf,
    events_path: PathBuf,
    changes_dir: PathBuf,
    state: OptimizerState,
    #[cfg(test)]
    fail_prepared_publication_once: bool,
    #[cfg(test)]
    fail_committed_publication_once: bool,
    #[cfg(test)]
    fail_retry_fence_stage_after_write_once: bool,
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
        self.validate_persisted_state()
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

    pub fn rollback_change(
        &mut self,
        change_id: &str,
        store: &mut KnowledgeStore,
    ) -> Result<VaultOptimizerRollbackResult> {
        self.rollback_change_internal(change_id, store, None)
    }

    pub(crate) fn rollback_change_expecting_authority(
        &mut self,
        change_id: &str,
        store: &mut KnowledgeStore,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<VaultOptimizerRollbackResult> {
        self.rollback_change_internal(change_id, store, Some(expected))
    }

    fn rollback_change_internal(
        &mut self,
        change_id: &str,
        store: &mut KnowledgeStore,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<VaultOptimizerRollbackResult> {
        let change = self
            .read_change(change_id)
            .with_context(|| format!("Optimizer change '{}' not found", change_id))?;

        if change.mode == "sidecar_first" || change.overlay_after.is_some() {
            if change
                .overlay_before
                .as_ref()
                .is_none_or(serde_json::Value::is_null)
            {
                if let Some(expected) = expected.clone() {
                    store.delete_overlay_from_source_expecting_authority(
                        &change.note_id,
                        "vault_optimizer",
                        expected,
                    )?;
                } else {
                    store.delete_overlay(&change.note_id)?;
                }
            } else {
                let overlay_before = change
                    .overlay_before
                    .as_ref()
                    .expect("non-null overlay was checked above");
                if let Some(expected) = expected.clone() {
                    store.write_overlay_from_source_expecting_authority(
                        &change.note_id,
                        overlay_before,
                        "vault_optimizer",
                        expected,
                    )?;
                } else {
                    store.write_overlay(&change.note_id, overlay_before)?;
                }
            }
        } else if let Some(note_before) = change.note_before {
            let update = NoteUpdate {
                title: Some(note_before.title),
                content: Some(note_before.content),
                relative_path: Some(note_before.relative_path),
                aliases: Some(note_before.aliases),
                status: Some(note_before.status),
                tags: Some(note_before.tags),
                schema_version: Some(note_before.schema_version),
                migration_source: note_before.migration_source,
                optimizer_managed: Some(note_before.optimizer_managed),
                properties: Some(note_before.properties),
            };
            if let Some(expected) = expected {
                store.update_note_expecting_authority(
                    &change.note_id,
                    update,
                    "vault_optimizer",
                    expected,
                )?;
            } else {
                store.update_note_from_source(&change.note_id, update, "vault_optimizer")?;
            }
        }

        self.state.rollback_count += 1;
        self.append_event(OptimizerAuditEventV1::Rollback {
            change_id: change_id.to_string(),
            at: Utc::now(),
        })?;
        self.persist_state()?;

        Ok(VaultOptimizerRollbackResult {
            change_id: change_id.to_string(),
            rolled_back: true,
            message: "Optimizer change rolled back".to_string(),
        })
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
            overlay_value,
            overlay_digest,
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
        let proposal = if sidecar_target_is_already_exact {
            proposal
        } else {
            refreshed_proposal
        };

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
                Ok((
                    OptimizerPublicationTarget::Overlay {
                        note_id: current.id.clone(),
                        before_digest,
                        after_digest: crate::services::twin_events::digest_bytes(&after_bytes),
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
                Ok((
                    OptimizerPublicationTarget::Markdown {
                        relative_path: before_path.clone(),
                        before_digest: before_digest.clone(),
                        after_digest: crate::services::twin_events::digest_bytes(&after_bytes),
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

    fn commit_staged_publication(
        &mut self,
        store: &mut KnowledgeStore,
        mut publication: PendingOptimizerPublication,
        compatibility_update: Option<NoteUpdate>,
    ) -> Result<OptimizerMutationResult<OptimizerAppliedResult>> {
        let expected_authority = publication.expected_authority.clone();
        let change_id = publication.change_id.clone();
        let job_id = publication.job.job_id.clone();
        let note_id = publication.note.id.clone();
        let source_precondition = match &publication.target {
            OptimizerPublicationTarget::Overlay {
                source_relative_path,
                source_digest,
                ..
            } => {
                let source_digest = source_digest.clone().ok_or_else(|| {
                    anyhow::anyhow!("optimizer sidecar witness lacks its source digest")
                });
                match source_digest.and_then(|digest| {
                    store.optimizer_markdown_precondition(source_relative_path, digest)
                }) {
                    Ok(precondition) => Some(precondition),
                    Err(error) => {
                        if expected_authority.is_some() {
                            self.abort_retry_fence_and_defer(&change_id, &job_id, &error, true)?;
                        } else {
                            self.defer_or_park_job_fresh(&job_id, &error)?;
                        }
                        return Ok(OptimizerMutationResult::NoWrite);
                    }
                }
            }
            OptimizerPublicationTarget::Markdown { .. } => None,
        };
        if let Some(precondition) = source_precondition.as_ref() {
            if let Err(error) = precondition.verify().map_err(anyhow::Error::new) {
                if expected_authority.is_some() {
                    self.abort_retry_fence_and_defer(&change_id, &job_id, &error, true)?;
                } else {
                    self.defer_or_park_job_fresh(&job_id, &error)?;
                }
                return Ok(OptimizerMutationResult::NoWrite);
            }
        }
        let source_guard = if expected_authority.is_some() {
            match source_precondition.as_ref() {
                Some(precondition) => match precondition.retained_target() {
                    Ok(target) => Some(target),
                    Err(error) => {
                        let error = anyhow::Error::new(error);
                        self.abort_retry_fence_and_defer(&change_id, &job_id, &error, true)?;
                        return Ok(OptimizerMutationResult::NoWrite);
                    }
                },
                None => None,
            }
        } else {
            None
        };

        // No optimizer state lock is held across this authority CAS. The
        // coordinator hooks durably install Prepared after it has finalized
        // the exact intent and mark Committed before releasing the shared
        // process lock, closing the post-effect publication race.
        let optimizer_root = self.retained_optimizer_root_handle()?;
        let prepared_template = publication.clone();
        let committed_template = publication.clone();
        let fail_prepared_publication = {
            #[cfg(test)]
            {
                std::mem::take(&mut self.fail_prepared_publication_once)
            }
            #[cfg(not(test))]
            {
                false
            }
        };
        let fail_committed_publication = {
            #[cfg(test)]
            {
                std::mem::take(&mut self.fail_committed_publication_once)
            }
            #[cfg(not(test))]
            {
                false
            }
        };
        let post_publication = std::cell::RefCell::new(None);
        let post_error = std::cell::RefCell::new(None);
        #[cfg(test)]
        let mut pause_after_prepared_publication =
            self.pause_after_prepared_publication_once.take();
        let mut prepared_hook = |intent: &crate::services::twin_events::MutationIntentV1| {
            if fail_prepared_publication {
                return Err(crate::services::twin_events::MutationError::Io(
                    "injected optimizer Prepared publication failure".into(),
                ));
            }
            if let Some(precondition) = source_precondition.as_ref() {
                precondition.verify()?;
            }
            let mut prepared = prepared_template.clone();
            let expected = prepared.expected_authority.as_ref().ok_or_else(|| {
                crate::services::twin_events::MutationError::Invalid(
                    "optimizer witness requires an exact source authority".into(),
                )
            })?;
            let targets_match = match &prepared.target {
                OptimizerPublicationTarget::Overlay {
                    note_id,
                    before_digest,
                    after_digest,
                    source_relative_path,
                    source_digest,
                } => {
                    let overlay_before = before_digest.as_ref().map_or(
                        crate::services::twin_events::BeforeImage::Absent,
                        |digest| crate::services::twin_events::BeforeImage::Sha256(digest.clone()),
                    );
                    source_digest.as_ref().is_some_and(|source_digest| {
                        intent.targets.len() == 2
                            && intent.targets.iter().any(|target| {
                                target.kind == crate::services::twin_events::TargetKind::OverlayJson
                                    && target.relative_key == format!("{note_id}.json")
                                    && target.before == overlay_before
                                    && &target.after_digest == after_digest
                            })
                            && intent.targets.iter().any(|target| {
                                target.kind == crate::services::twin_events::TargetKind::Markdown
                                    && target.relative_key == *source_relative_path
                                    && target.before
                                        == crate::services::twin_events::BeforeImage::Sha256(
                                            source_digest.clone(),
                                        )
                                    && target.after_digest == *source_digest
                            })
                    })
                }
                OptimizerPublicationTarget::Markdown {
                    relative_path,
                    before_digest,
                    after_digest,
                } => {
                    intent.targets.len() == 1
                        && intent.targets.first().is_some_and(|target| {
                            target.kind == crate::services::twin_events::TargetKind::Markdown
                                && target.relative_key == *relative_path
                                && target.before
                                    == crate::services::twin_events::BeforeImage::Sha256(
                                        before_digest.clone(),
                                    )
                                && &target.after_digest == after_digest
                        })
                }
            };
            if intent.schema_version != 3
                || !intent.retain_commit_receipt
                || intent.markdown_root_scope.as_ref() != Some(&expected.root_scope)
                || !targets_match
            {
                return Err(crate::services::twin_events::MutationError::Invalid(
                    "optimizer witness does not bind the finalized mutation intent".into(),
                ));
            }
            prepared.phase = OptimizerPublicationPhase::Prepared;
            prepared.mutation_id = Some(intent.mutation_id.clone());
            if prepared.expected_authority.is_some()
                && intent.content_authority_generation
                    != prepared
                        .expected_authority
                        .as_ref()
                        .and_then(|token| token.authority_generation.checked_add(1))
            {
                return Err(crate::services::twin_events::MutationError::Invalid(
                    "optimizer publication authority generation mismatch".into(),
                ));
            }
            write_pending_publication(&optimizer_root, &prepared).map_err(|error| {
                crate::services::twin_events::MutationError::Io(error.to_string())
            })?;
            #[cfg(test)]
            if let Some((entered, resume)) = pause_after_prepared_publication.take() {
                entered.wait();
                resume.wait();
            }
            Ok(())
        };
        let committed_root = self.retained_optimizer_root_handle()?;
        let mut committed_hook = |commit: &crate::services::twin_events::MutationCommit| {
            if fail_committed_publication {
                let error = "injected optimizer Committed publication failure".to_string();
                *post_error.borrow_mut() = Some(error);
                return Err(crate::services::twin_events::MutationError::Io(
                    "optimizer committed publication could not be persisted".into(),
                ));
            }
            let mut committed = committed_template.clone();
            committed.phase = OptimizerPublicationPhase::Committed;
            committed.retry_fenced = true;
            committed.mutation_id = commit.mutation_id.clone();
            committed.committed_authority = commit.authority_token.clone();
            match write_pending_publication(&committed_root, &committed) {
                Ok(()) => {
                    *post_publication.borrow_mut() = Some(committed);
                    Ok(())
                }
                Err(error) => {
                    *post_error.borrow_mut() = Some(error.to_string());
                    Err(crate::services::twin_events::MutationError::Io(
                        "optimizer committed publication could not be persisted".into(),
                    ))
                }
            }
        };

        #[cfg(test)]
        if let Some((entered, resume)) = self.pause_before_prepared_hook_once.take() {
            entered.wait();
            resume.wait();
        }

        let write_result =
            if let Some(overlay_after) = publication.change.overlay_after.as_ref() {
                if expected_authority.is_some() {
                    store.write_overlay_from_source_with_authority_and_hooks(
                        &note_id,
                        overlay_after,
                        "vault_optimizer",
                        expected_authority.clone(),
                        &mut prepared_hook,
                        &mut committed_hook,
                        true,
                        source_guard,
                    )
                } else {
                    store.write_overlay_from_source_with_authority(
                        &note_id,
                        overlay_after,
                        "vault_optimizer",
                        None,
                    )
                }
            } else {
                let before =
                    publication.change.note_before.clone().ok_or_else(|| {
                        anyhow::anyhow!("optimizer publication has no source note")
                    })?;
                let before_digest = publication
                    .change
                    .markdown_before_digest
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("optimizer publication has no source digest"))?;
                let exact =
                    publication.change.note_after.clone().ok_or_else(|| {
                        anyhow::anyhow!("optimizer publication has no exact note")
                    })?;
                match expected_authority.clone() {
                    Some(expected) => store
                        .replace_note_exact_expecting_authority_with_hooks(
                            &note_id,
                            before,
                            before_digest,
                            exact,
                            "vault_optimizer",
                            expected,
                            &mut prepared_hook,
                            &mut committed_hook,
                        )
                        .map(|(_, commit)| commit),
                    None => store
                        .update_note_from_source_with_commit(
                            &note_id,
                            compatibility_update.ok_or_else(|| {
                                anyhow::anyhow!("optimizer compatibility update is missing")
                            })?,
                            "vault_optimizer",
                        )
                        .map(|(_, commit)| commit),
                }
            };
        let commit = match write_result {
            Ok(commit) => commit,
            Err(error) => {
                let aborted_precondition = error
                    .downcast_ref::<crate::services::twin_events::MutationError>()
                    .and_then(|error| match error {
                        crate::services::twin_events::MutationError::AbortedPrecondition {
                            mutation_id,
                            authority_advanced,
                        } => Some((mutation_id.clone(), *authority_advanced)),
                        _ => None,
                    });
                if let Some((mutation_id, authority_advanced)) = aborted_precondition {
                    if self.abort_precondition_owner_and_defer(
                        &change_id,
                        &job_id,
                        &mutation_id,
                        &error,
                    )? && !authority_advanced
                    {
                        return Ok(OptimizerMutationResult::NoWrite);
                    }
                    return Err(error);
                }
                // A RetryFenced witness proves the Prepared hook never
                // completed, so no authoritative effect could have started.
                // Prepared/Committed witnesses remain for exact rebuild
                // classification and are never cleared optimistically.
                if expected_authority.is_some() {
                    if self.abort_retry_fence_and_defer(&change_id, &job_id, &error, false)? {
                        return Ok(OptimizerMutationResult::NoWrite);
                    }
                }
                return Err(error);
            }
        };
        if expected_authority.is_some()
            && commit.mutation_id.is_none()
            && commit.authority_token.is_none()
        {
            self.complete_retry_fenced_noop(&change_id, &job_id)?;
            return Ok(OptimizerMutationResult::NoWrite);
        }
        publication.phase = OptimizerPublicationPhase::Committed;
        publication.retry_fenced = true;
        publication.mutation_id = commit.mutation_id.clone();
        publication.committed_authority = commit.authority_token.clone();
        let mark_result = if expected_authority.is_none() {
            Ok(())
        } else if let Some(error) = post_error.into_inner() {
            Err(anyhow::anyhow!(error))
        } else if let Some(committed_publication) = post_publication.into_inner() {
            publication = committed_publication;
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "optimizer committed hook did not publish its durable fence"
            ))
        };
        let warning = match mark_result {
            Ok(()) if expected_authority.is_some() && !commit.postcommit_warning => None,
            Ok(()) if expected_authority.is_some() => Some(
                crate::models::mutation::CommittedMutationWarningV1::derived_state_unavailable(),
            ),
            Ok(()) => match self.with_locked_fresh_state(|service| {
                service.finalize_compatibility_publication(&publication)
            }) {
                Ok(()) => None,
                Err(error) => {
                    log::error!("Optimizer compatibility publication {change_id} failed: {error}");
                    Some(
                        crate::models::mutation::CommittedMutationWarningV1::derived_state_unavailable(),
                    )
                }
            },
            Err(error) => {
                log::error!(
                "Optimizer authority change {change_id} committed but postwrite publication failed: {error}"
            );
                Some(
                    crate::models::mutation::CommittedMutationWarningV1::derived_state_unavailable(
                    ),
                )
            }
        };
        Ok(OptimizerMutationResult::Committed {
            result: OptimizerAppliedResult { note_id, change_id },
            commit,
            warning,
        })
    }

    fn stage_pending_publication(&self, pending: &PendingOptimizerPublication) -> Result<()> {
        write_pending_publication(self.retained_optimizer_root()?, pending)
    }

    fn load_pending_publications(&self) -> Result<Vec<(String, PendingOptimizerPublication)>> {
        load_pending_publications(self.retained_optimizer_root()?)
    }

    fn complete_retry_fenced_noop(&mut self, change_id: &str, job_id: &str) -> Result<()> {
        self.with_locked_fresh_state(|service| {
            let pending = service
                .load_pending_publications()?
                .into_iter()
                .find(|(_, pending)| pending.change_id == change_id)
                .map(|(_, pending)| pending)
                .ok_or_else(|| anyhow::anyhow!("optimizer no-op retry fence disappeared"))?;
            if pending.phase != OptimizerPublicationPhase::RetryFenced
                || pending.job.job_id != job_id
            {
                anyhow::bail!("optimizer no-op retry fence changed before cleanup");
            }
            remove_pending_publication(service.retained_optimizer_root()?, change_id)?;
            let before = service.state.queue.len();
            service.remove_queued_job_id(job_id);
            if service.state.queue.len() != before {
                service.state.last_run_at = Some(Utc::now());
                service.persist_state()?;
            }
            Ok(())
        })
    }

    fn abort_retry_fence_and_defer(
        &mut self,
        change_id: &str,
        job_id: &str,
        error: &anyhow::Error,
        missing_is_unprepared: bool,
    ) -> Result<bool> {
        let error_message = error.to_string();
        self.with_locked_fresh_state(|service| {
            let pending = service
                .load_pending_publications()?
                .into_iter()
                .find(|(_, pending)| pending.change_id == change_id);
            match pending {
                Some((_, pending))
                    if pending.phase == OptimizerPublicationPhase::RetryFenced
                        && pending.job.job_id == job_id =>
                {
                    remove_pending_publication(service.retained_optimizer_root()?, change_id)?;
                    service.defer_or_park_job_id(job_id, anyhow::anyhow!(error_message))?;
                    Ok(true)
                }
                None if missing_is_unprepared => {
                    service.defer_or_park_job_id(job_id, anyhow::anyhow!(error_message))?;
                    Ok(true)
                }
                _ => Ok(false),
            }
        })
    }

    fn abort_precondition_owner_and_defer(
        &mut self,
        change_id: &str,
        job_id: &str,
        mutation_id: &str,
        error: &anyhow::Error,
    ) -> Result<bool> {
        let error_message = error.to_string();
        self.with_locked_fresh_state(|service| {
            let pending = service
                .load_pending_publications()?
                .into_iter()
                .find(|(_, pending)| pending.change_id == change_id)
                .map(|(_, pending)| pending);
            let Some(pending) = pending else {
                return Ok(false);
            };
            if pending.phase != OptimizerPublicationPhase::Prepared
                || pending.job.job_id != job_id
                || pending.mutation_id.as_ref().map(|id| id.as_str()) != Some(mutation_id)
            {
                return Ok(false);
            }
            remove_pending_publication(service.retained_optimizer_root()?, change_id)?;
            service.defer_or_park_job_id(job_id, anyhow::anyhow!(error_message))?;
            Ok(true)
        })
    }

    fn preserve_retry_fence_or_defer(
        &mut self,
        change_id: &str,
        job_id: &str,
        error: &anyhow::Error,
    ) -> Result<()> {
        let error_message = error.to_string();
        self.with_locked_fresh_state(|service| {
            let owners = service
                .load_pending_publications()?
                .into_iter()
                .filter(|(_, pending)| pending.job.job_id == job_id)
                .collect::<Vec<_>>();
            if let Some((_, owner)) = owners.first() {
                if owner.change_id == change_id
                    && owner.phase == OptimizerPublicationPhase::RetryFenced
                {
                    return Ok(());
                }
                return Ok(());
            }
            service.defer_or_park_job_id(job_id, anyhow::anyhow!(error_message))
        })
    }

    fn finalize_pending_publication(
        &mut self,
        pending: &mut PendingOptimizerPublication,
    ) -> Result<()> {
        validate_pending_publication(pending)?;
        if pending.phase != OptimizerPublicationPhase::Committed {
            anyhow::bail!("optimizer publication is not durably committed");
        }

        if !pending.audit_written {
            let inbox_entry = publication_inbox_entry(pending)?;
            let event = publication_audit_event(pending)?;
            self.write_change(&pending.change)?;
            self.push_decision_unique(pending.decision.clone())?;
            self.push_inbox_unique(inbox_entry)?;
            self.append_event_unique(&pending.change_id, event)?;
            pending.audit_written = true;
            self.stage_pending_publication(pending)?;
        }

        if !pending.counted {
            let decisions = self.load_decisions()?;
            self.state.accepted_count = decisions
                .iter()
                .filter(|decision| decision.kind == "optimizer_update")
                .count();
            self.state.last_run_at = decisions
                .iter()
                .filter_map(|decision| decision.created_at)
                .max();
            let today = Utc::now().date_naive();
            self.state.daily_write_date = Some(today.to_string());
            self.state.daily_write_count = decisions
                .iter()
                .filter(|decision| {
                    decision.kind == "optimizer_update"
                        && decision
                            .created_at
                            .is_some_and(|created| created.date_naive() == today)
                })
                .count()
                .try_into()
                .map_err(|_| anyhow::anyhow!("optimizer daily write count overflow"))?;
            self.persist_state()?;
            pending.counted = true;
            self.stage_pending_publication(pending)?;
        }

        if !pending.queue_removed {
            self.remove_queued_job_id(&pending.job.job_id);
            self.persist_state()?;
            pending.queue_removed = true;
            self.stage_pending_publication(pending)?;
        }
        Ok(())
    }

    fn finalize_compatibility_publication(
        &mut self,
        pending: &PendingOptimizerPublication,
    ) -> Result<()> {
        let inbox_entry = publication_inbox_entry(pending)?;
        let event = publication_audit_event(pending)?;
        let audit_result = (|| -> Result<()> {
            self.write_change(&pending.change)?;
            self.push_decision_unique(pending.decision.clone())?;
            self.push_inbox_unique(inbox_entry)?;
            self.append_event_unique(&pending.change_id, event)
        })();

        if audit_result.is_ok() {
            let decisions = self.load_decisions()?;
            self.state.accepted_count = decisions
                .iter()
                .filter(|decision| decision.kind == "optimizer_update")
                .count();
            self.state.last_run_at = decisions
                .iter()
                .filter_map(|decision| decision.created_at)
                .max();
            let today = Utc::now().date_naive();
            self.state.daily_write_date = Some(today.to_string());
            self.state.daily_write_count = decisions
                .iter()
                .filter(|decision| {
                    decision.kind == "optimizer_update"
                        && decision
                            .created_at
                            .is_some_and(|created| created.date_naive() == today)
                })
                .count()
                .try_into()
                .map_err(|_| anyhow::anyhow!("optimizer daily write count overflow"))?;
        }
        // Even compatibility callers that do not supply an authority token
        // must never retry a write that already returned a commit.
        self.remove_queued_job_id(&pending.job.job_id);
        self.persist_state()?;
        audit_result
    }

    pub(crate) fn recover_pending_publications_locked(
        &mut self,
        _store: &KnowledgeStore,
        guard: &crate::services::twin_events::MutationRootTransitionGuard<'_>,
    ) -> Result<()> {
        cleanup_optimizer_orphan_temps(self.retained_optimizer_root()?)?;
        for (_, mut pending) in self.load_pending_publications()? {
            if pending.phase == OptimizerPublicationPhase::RetryFenced {
                // This pre-effect witness is both the stable retry identity
                // and the durable reservation for all four audit outputs.
                // It may belong to a live writer waiting for the coordinator,
                // or to a crashed writer; either way, deleting it here races
                // the former and strands the latter. Leave it adoptable by a
                // later optimizer tick.
                continue;
            }
            let expected = pending.expected_authority.as_ref().ok_or_else(|| {
                anyhow::anyhow!("optimizer publication is missing its source authority")
            })?;
            let mutation_id = pending.mutation_id.clone().ok_or_else(|| {
                anyhow::anyhow!("optimizer publication is missing its finalized mutation ID")
            })?;
            if pending.phase == OptimizerPublicationPhase::Committed
                && pending.audit_written
                && pending.counted
                && pending.queue_removed
            {
                // Publication completion is itself a durable proof. This
                // branch makes the receipt-delete / witness-delete crash gap
                // idempotent: a missing receipt can only be accepted after all
                // four monotonic phases were fsynced in the witness.
                guard
                    .consume_witnessed_mutation_receipt(&mutation_id)
                    .map_err(anyhow::Error::new)?;
                remove_pending_publication(self.retained_optimizer_root()?, &pending.change_id)?;
                continue;
            }
            let (kind, key, before, after) = match &pending.target {
                OptimizerPublicationTarget::Overlay {
                    note_id,
                    before_digest,
                    after_digest,
                    ..
                } => (
                    crate::services::twin_events::TargetKind::OverlayJson,
                    format!("{note_id}.json"),
                    before_digest.as_ref().map_or(
                        crate::services::twin_events::BeforeImage::Absent,
                        |digest| crate::services::twin_events::BeforeImage::Sha256(digest.clone()),
                    ),
                    after_digest.clone(),
                ),
                OptimizerPublicationTarget::Markdown {
                    relative_path,
                    before_digest,
                    after_digest,
                } => (
                    crate::services::twin_events::TargetKind::Markdown,
                    relative_path.clone(),
                    crate::services::twin_events::BeforeImage::Sha256(before_digest.clone()),
                    after_digest.clone(),
                ),
            };
            match guard
                .classify_witnessed_mutation(&mutation_id, expected, kind, &key, &before, &after)
                .map_err(anyhow::Error::new)?
            {
                crate::services::twin_events::WitnessedMutationRecovery::NotCommitted => {
                    remove_pending_publication(
                        self.retained_optimizer_root()?,
                        &pending.change_id,
                    )?;
                }
                crate::services::twin_events::WitnessedMutationRecovery::Committed(commit) => {
                    if pending.phase == OptimizerPublicationPhase::Committed
                        && pending.committed_authority != commit.authority_token
                    {
                        anyhow::bail!("optimizer committed witness authority mismatch");
                    }
                    pending.phase = OptimizerPublicationPhase::Committed;
                    pending.retry_fenced = true;
                    pending.committed_authority = commit.authority_token;
                    self.stage_pending_publication(&pending)?;
                    self.finalize_pending_publication(&mut pending)?;
                    guard
                        .consume_witnessed_mutation_receipt(&mutation_id)
                        .map_err(anyhow::Error::new)?;
                    remove_pending_publication(
                        self.retained_optimizer_root()?,
                        &pending.change_id,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Records a processing failure for `job`. Below `MAX_OPTIMIZER_ATTEMPTS`
    /// the job stays queued (in its original position) with `attempts`
    /// incremented, so it's retried on a later tick. At the limit it's parked:
    /// removed from the queue and recorded in the inbox with status `"failed"`
    /// so a human can see it, instead of spinning on a poisoned entry forever.
    fn defer_or_park_job(&mut self, job: QueuedOptimizerNote, error: anyhow::Error) -> Result<()> {
        self.defer_or_park_job_id(&job.job_id, error)
    }

    fn defer_or_park_job_fresh(&mut self, job_id: &str, error: &anyhow::Error) -> Result<()> {
        let error_message = error.to_string();
        self.with_locked_fresh_state(|service| {
            if service
                .load_pending_publications()?
                .into_iter()
                .any(|(_, pending)| pending.job.job_id == job_id)
            {
                return Ok(());
            }
            service.defer_or_park_job_id(job_id, anyhow::anyhow!(error_message))
        })
    }

    fn defer_or_park_job_id(&mut self, job_id: &str, error: anyhow::Error) -> Result<()> {
        let Some(position) = self
            .state
            .queue
            .iter()
            .position(|entry| entry.job_id == job_id)
        else {
            return Ok(());
        };
        let mut job = self.state.queue[position].clone();
        job.attempts = job
            .attempts
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("optimizer attempt count exhausted"))?;
        log::warn!(
            "Vault optimizer job for note '{}' failed (attempt {}/{}): {}",
            job.note_id,
            job.attempts,
            MAX_OPTIMIZER_ATTEMPTS,
            error
        );

        if job.attempts >= MAX_OPTIMIZER_ATTEMPTS {
            self.state.queue.remove(position);
            let inbox_entry = VaultOptimizerInboxEntry {
                id: Uuid::new_v4().to_string(),
                note_id: Some(job.note_id.clone()),
                status: "failed".to_string(),
                title: job.note_id.clone(),
                reason: format!(
                    "Vault optimizer parked after {} failed attempts: {}",
                    job.attempts, error
                ),
                diff_preview: String::new(),
                confidence: 0.0,
                created_at: Some(Utc::now()),
                change_id: None,
            };
            self.push_inbox(inbox_entry)?;
            self.append_event(OptimizerAuditEventV1::OptimizerParked {
                note_id: job.note_id,
                attempts: job.attempts,
                error: error.to_string(),
                at: Utc::now(),
            })?;
        } else {
            self.state.queue[position].attempts = job.attempts;
        }

        self.persist_state()?;
        Ok(())
    }

    fn remove_queued_job(&mut self, note_id: &str) {
        if let Some(pos) = self
            .state
            .queue
            .iter()
            .position(|entry| entry.note_id == note_id)
        {
            self.state.queue.remove(pos);
        }
    }

    fn remove_queued_job_id(&mut self, job_id: &str) {
        if let Some(position) = self
            .state
            .queue
            .iter()
            .position(|entry| entry.job_id == job_id)
        {
            self.state.queue.remove(position);
        }
    }

    fn complete_noop_job(&mut self, note_id: &str) -> Result<()> {
        self.remove_queued_job(note_id);
        self.state.last_run_at = Some(Utc::now());
        self.persist_state()
    }

    fn complete_noop_job_fresh(&mut self, job_id: &str) -> Result<()> {
        self.with_locked_fresh_state(|service| {
            if service
                .load_pending_publications()?
                .into_iter()
                .any(|(_, pending)| pending.job.job_id == job_id)
            {
                return Ok(());
            }
            let before = service.state.queue.len();
            service.remove_queued_job_id(job_id);
            if service.state.queue.len() == before {
                return Ok(());
            }
            service.state.last_run_at = Some(Utc::now());
            service.persist_state()
        })
    }

    /// Whether today's write count has already reached
    /// `background_vault_optimizer_max_daily_writes`. Only meaningful once at
    /// least one write has happened today; a fresh day always reports `false`
    /// regardless of yesterday's count.
    fn daily_write_cap_reached(&self, settings: &UserSettings) -> bool {
        let today = Utc::now().date_naive().to_string();
        self.state.daily_write_date.as_deref() == Some(today.as_str())
            && self.state.daily_write_count >= settings.background_vault_optimizer_max_daily_writes
    }

    fn persist_state(&mut self) -> Result<()> {
        let previous_revision = self.state.state_revision;
        self.state.schema_version = OPTIMIZER_STATE_SCHEMA_VERSION;
        self.state.state_revision = self
            .state
            .state_revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("vault optimizer state revision exhausted"))?;
        let bytes = serde_json::to_vec_pretty(&self.state)?;
        if bytes.len() > MAX_OPTIMIZER_STATE_BYTES {
            self.state.state_revision = previous_revision;
            anyhow::bail!("vault optimizer state exceeds its 4 MiB limit");
        }
        let result = self
            .retained_optimizer_root()?
            .put_atomic(QUEUE_KEY, &bytes)
            .map_err(anyhow::Error::new);
        if let Err(error) = result {
            self.state.state_revision = previous_revision;
            return Err(error);
        }
        Ok(())
    }

    fn pending_audit_reservations(
        &self,
        exclude_change_id: Option<&str>,
    ) -> Result<Vec<PendingOptimizerPublication>> {
        Ok(self
            .load_pending_publications()?
            .into_iter()
            .map(|(_, pending)| pending)
            .filter(|pending| {
                !pending.audit_written && exclude_change_id != Some(pending.change_id.as_str())
            })
            .collect())
    }

    fn ensure_decision_capacity(
        &self,
        additional: &VaultOptimizerDecision,
        exclude_change_id: Option<&str>,
    ) -> Result<()> {
        let mut decisions = self.load_decisions()?;
        for pending in self.pending_audit_reservations(exclude_change_id)? {
            merge_optimizer_decision(&mut decisions, pending.decision)?;
        }
        merge_optimizer_decision(&mut decisions, additional.clone())?;
        validate_json_audit_capacity(&decisions, "optimizer decisions")
    }

    fn ensure_inbox_capacity(
        &self,
        additional: &VaultOptimizerInboxEntry,
        exclude_change_id: Option<&str>,
    ) -> Result<()> {
        let mut inbox = self.load_inbox()?;
        for pending in self.pending_audit_reservations(exclude_change_id)? {
            merge_optimizer_inbox(&mut inbox, publication_inbox_entry(&pending)?)?;
        }
        merge_optimizer_inbox(&mut inbox, additional.clone())?;
        validate_json_audit_capacity(&inbox, "optimizer inbox")
    }

    fn ensure_event_capacity(
        &self,
        additional: &OptimizerAuditEventV1,
        exclude_change_id: Option<&str>,
    ) -> Result<()> {
        let mut events = self.load_events()?;
        for pending in self.pending_audit_reservations(exclude_change_id)? {
            merge_optimizer_event(&mut events, publication_audit_event(&pending)?)?;
        }
        merge_optimizer_event(&mut events, additional.clone())?;
        serialize_optimizer_events(&events).map(|_| ())
    }

    fn ensure_change_capacity(
        &self,
        additional: &OptimizerChange,
        exclude_change_id: Option<&str>,
    ) -> Result<()> {
        let root = self.retained_optimizer_root()?;
        let names = root
            .regular_file_names(CHANGES_DIRECTORY)
            .map_err(anyhow::Error::new)?;
        if names.len() > MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer change audit exceeds 4096 entries");
        }
        let mut changes = HashMap::new();
        for name in names {
            let change_id = name
                .strip_suffix(".json")
                .ok_or_else(|| anyhow::anyhow!("invalid optimizer change filename"))?;
            merge_optimizer_change(&mut changes, self.read_change(change_id)?)?;
        }
        for pending in self.pending_audit_reservations(exclude_change_id)? {
            merge_optimizer_change(&mut changes, pending.change)?;
        }
        merge_optimizer_change(&mut changes, additional.clone())?;
        if changes.len() > MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer change audit has reached 4096 entries");
        }
        for change in changes.values() {
            if serde_json::to_vec_pretty(change)?.len() > MAX_OPTIMIZER_CHANGE_BYTES {
                anyhow::bail!("optimizer change exceeds its 1 MiB limit");
            }
        }
        Ok(())
    }

    fn preflight_publication_audit(&self, pending: &PendingOptimizerPublication) -> Result<()> {
        validate_pending_publication(pending)?;
        let inbox = publication_inbox_entry(pending)?;
        let event = publication_audit_event(pending)?;
        self.ensure_decision_capacity(&pending.decision, None)?;
        self.ensure_inbox_capacity(&inbox, None)?;
        self.ensure_event_capacity(&event, None)?;
        self.ensure_change_capacity(&pending.change, None)
    }

    fn load_decisions(&self) -> Result<Vec<VaultOptimizerDecision>> {
        load_bounded_json_vec(
            self.retained_optimizer_root()?,
            DECISIONS_KEY,
            "optimizer decisions",
        )
    }

    fn push_decision_unique(&self, decision: VaultOptimizerDecision) -> Result<()> {
        self.ensure_decision_capacity(&decision, decision.change_id.as_deref())?;
        let mut decisions = self.load_decisions()?;
        if let Some(existing) = decisions
            .iter()
            .find(|existing| existing.id == decision.id || existing.change_id == decision.change_id)
        {
            if existing == &decision {
                return Ok(());
            }
            anyhow::bail!("optimizer decision identity collision");
        }
        if decisions.len() >= MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer decisions have reached 4096 entries");
        }
        decisions.push(decision);
        write_bounded_json_vec(self.retained_optimizer_root()?, DECISIONS_KEY, &decisions)
    }

    fn load_inbox(&self) -> Result<Vec<VaultOptimizerInboxEntry>> {
        load_bounded_json_vec(
            self.retained_optimizer_root()?,
            INBOX_KEY,
            "optimizer inbox",
        )
    }

    fn push_inbox(&self, entry: VaultOptimizerInboxEntry) -> Result<()> {
        self.ensure_inbox_capacity(&entry, None)?;
        let mut inbox = self.load_inbox()?;
        if inbox.len() >= MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer inbox has reached 4096 entries");
        }
        inbox.push(entry);
        write_bounded_json_vec(self.retained_optimizer_root()?, INBOX_KEY, &inbox)
    }

    fn push_inbox_unique(&self, entry: VaultOptimizerInboxEntry) -> Result<()> {
        self.ensure_inbox_capacity(&entry, entry.change_id.as_deref())?;
        let mut inbox = self.load_inbox()?;
        if let Some(existing) = inbox
            .iter()
            .find(|existing| existing.id == entry.id || existing.change_id == entry.change_id)
        {
            if existing == &entry {
                return Ok(());
            }
            anyhow::bail!("optimizer inbox identity collision");
        }
        if inbox.len() >= MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer inbox has reached 4096 entries");
        }
        inbox.push(entry);
        write_bounded_json_vec(self.retained_optimizer_root()?, INBOX_KEY, &inbox)
    }

    fn write_change(&self, change: &OptimizerChange) -> Result<()> {
        parse_canonical_uuid(&change.change_id, "optimizer change ID")?;
        self.ensure_change_capacity(change, Some(change.change_id.as_str()))?;
        let root = self.retained_optimizer_root()?;
        let key = format!("{CHANGES_DIRECTORY}/{}.json", change.change_id);
        if let Some(bytes) = root
            .read_bounded(&key, MAX_OPTIMIZER_CHANGE_BYTES)
            .map_err(anyhow::Error::new)?
        {
            let existing: OptimizerChange =
                serde_json::from_slice(&bytes).context("invalid optimizer change audit")?;
            if serde_json::to_value(existing)? == serde_json::to_value(change)? {
                return Ok(());
            }
            anyhow::bail!("optimizer change identity collision");
        }
        let bytes = serde_json::to_vec_pretty(change)?;
        if bytes.len() > MAX_OPTIMIZER_CHANGE_BYTES {
            anyhow::bail!("optimizer change exceeds its 1 MiB limit");
        }
        root.put_atomic(&key, &bytes).map_err(anyhow::Error::new)
    }

    fn read_change(&self, change_id: &str) -> Result<OptimizerChange> {
        parse_canonical_uuid(change_id, "optimizer change ID")?;
        let key = format!("{CHANGES_DIRECTORY}/{change_id}.json");
        let bytes = self
            .retained_optimizer_root()?
            .read_bounded(&key, MAX_OPTIMIZER_CHANGE_BYTES)
            .map_err(anyhow::Error::new)?
            .ok_or_else(|| anyhow::anyhow!("optimizer change does not exist"))?;
        let change: OptimizerChange =
            serde_json::from_slice(&bytes).context("invalid optimizer change audit")?;
        if change.change_id != change_id {
            anyhow::bail!("optimizer change identity mismatch");
        }
        Ok(change)
    }

    fn load_events(&self) -> Result<Vec<OptimizerAuditEventV1>> {
        let Some(bytes) = self
            .retained_optimizer_root()?
            .read_bounded(EVENTS_KEY, MAX_OPTIMIZER_AUDIT_BYTES)
            .map_err(anyhow::Error::new)?
        else {
            return Ok(Vec::new());
        };
        let contents = std::str::from_utf8(&bytes).context("optimizer event audit is not UTF-8")?;
        let mut events = Vec::new();
        for (index, line) in contents.lines().enumerate() {
            if index >= MAX_OPTIMIZER_AUDIT_ENTRIES {
                anyhow::bail!("optimizer event audit exceeds 4096 entries");
            }
            if line.is_empty() {
                anyhow::bail!("optimizer event audit contains an empty record");
            }
            events.push(
                serde_json::from_str::<OptimizerAuditEventV1>(line).with_context(|| {
                    format!("invalid optimizer event audit at line {}", index + 1)
                })?,
            );
        }
        Ok(events)
    }

    fn write_events(&self, events: &[OptimizerAuditEventV1]) -> Result<()> {
        let bytes = serialize_optimizer_events(events)?;
        self.retained_optimizer_root()?
            .put_atomic(EVENTS_KEY, &bytes)
            .map_err(anyhow::Error::new)
    }

    fn append_event(&self, event: OptimizerAuditEventV1) -> Result<()> {
        self.ensure_event_capacity(&event, None)?;
        let mut events = self.load_events()?;
        if events.len() >= MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer event audit has reached 4096 entries");
        }
        events.push(event);
        self.write_events(&events)
    }

    fn append_event_unique(&self, change_id: &str, event: OptimizerAuditEventV1) -> Result<()> {
        self.ensure_event_capacity(&event, Some(change_id))?;
        let mut events = self.load_events()?;
        if let Some(existing) = events.iter().find(|existing| {
            matches!(
                existing,
                OptimizerAuditEventV1::OptimizerApply {
                    change_id: existing_id,
                    ..
                } if existing_id == change_id
            )
        }) {
            if existing == &event {
                return Ok(());
            }
            anyhow::bail!("optimizer event audit identity collision");
        }
        if events.len() >= MAX_OPTIMIZER_AUDIT_ENTRIES {
            anyhow::bail!("optimizer event audit has reached 4096 entries");
        }
        events.push(event);
        self.write_events(&events)
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
        OptimizerAuditEventV1::OptimizerApply { change_id, .. } => Some(change_id.as_str()),
        _ => None,
    };
    if let Some(change_id) = candidate_change_id {
        if let Some(existing) = values.iter().find(|existing| {
            matches!(
                existing,
                OptimizerAuditEventV1::OptimizerApply {
                    change_id: existing_id,
                    ..
                } if existing_id == change_id
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
        (CHANGES_DIRECTORY, MAX_OPTIMIZER_AUDIT_ENTRIES),
    ] {
        let names = root
            .regular_file_names(directory)
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
    match (
        pending.phase,
        pending.mutation_id.as_ref(),
        pending.committed_authority.as_ref(),
    ) {
        (OptimizerPublicationPhase::RetryFenced, None, None)
            if !pending.audit_written && !pending.counted && !pending.queue_removed => {}
        (OptimizerPublicationPhase::Prepared, Some(_), None)
            if !pending.audit_written && !pending.counted && !pending.queue_removed => {}
        (OptimizerPublicationPhase::Committed, Some(_), Some(committed)) => {
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
        anyhow::bail!("optimizer pending publication exceeds its 1 MiB limit");
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
mod tests {
    use super::*;
    use crate::models::note::{NoteCreate, NoteStatus};
    use crate::services::atomic_io::assert_no_tmp_siblings;
    use tempfile::tempdir;

    fn try_symlink_file(target: &std::path::Path, link: &std::path::Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).unwrap();
            true
        }
        #[cfg(windows)]
        {
            match std::os::windows::fs::symlink_file(target, link) {
                Ok(()) => true,
                Err(error) if matches!(error.raw_os_error(), Some(5) | Some(1314)) => {
                    eprintln!("skipping symlink regression without Windows symlink privilege");
                    false
                }
                Err(error) => panic!("failed to create file symlink: {error}"),
            }
        }
    }

    fn make_note_create(title: &str) -> NoteCreate {
        NoteCreate {
            title: title.to_string(),
            content: format!("Content for {}", title),
            relative_path: None,
            aliases: Vec::new(),
            status: NoteStatus::Draft,
            tags: Vec::new(),
            schema_version: CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            properties: HashMap::new(),
        }
    }

    fn run_one_optimizer_write(
        service: &mut VaultOptimizerService,
        store: &mut KnowledgeStore,
        settings: &UserSettings,
    ) {
        let tick = service
            .prepare_next(store, settings)
            .expect("optimizer prepare should not error");
        match tick {
            OptimizerTick::Pending(pending) => assert!(matches!(
                service
                    .apply_pending(store, *pending)
                    .expect("optimizer apply should not error"),
                OptimizerMutationResult::Committed { .. }
            )),
            OptimizerTick::RetryFenced(pending) => assert!(matches!(
                service
                    .apply_retry_fenced(store, *pending)
                    .expect("optimizer retry fence should resume"),
                OptimizerMutationResult::Committed { .. }
            )),
            OptimizerTick::Committed { .. } => {}
            OptimizerTick::NoWrite => panic!("expected an optimizer authority write"),
        }
    }

    fn make_note(id: &str, title: &str) -> Note {
        let now = Utc::now();
        Note {
            id: id.to_string(),
            title: title.to_string(),
            content: format!("Content of {}", title),
            relative_path: format!("{}.md", id),
            aliases: Vec::new(),
            status: NoteStatus::Draft,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            wikilinks: Vec::new(),
            parsed_links: Vec::new(),
            properties: HashMap::new(),
            ..Default::default()
        }
    }

    fn authority_optimizer_fixture(
        title: &str,
    ) -> (
        tempfile::TempDir,
        tempfile::TempDir,
        std::sync::Arc<crate::services::twin_events::MutationCoordinator>,
        KnowledgeStore,
        VaultOptimizerService,
        Note,
    ) {
        let vault_dir = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
            data_dir.path(),
        ));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                data_dir.path(),
                vault_dir.path(),
                events,
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let namespace = coordinator.current_namespace_path().unwrap();
        let mut store = KnowledgeStore::with_event_recorder(
            vault_dir.path().to_path_buf(),
            namespace.clone(),
            coordinator.clone(),
        );
        let note = store.create_note(make_note_create(title)).unwrap();
        let mut service = VaultOptimizerService::try_new(namespace).unwrap();
        service
            .bootstrap_checked(std::slice::from_ref(&note))
            .unwrap();
        (vault_dir, data_dir, coordinator, store, service, note)
    }

    fn prepare_authority_write(
        service: &mut VaultOptimizerService,
        store: &KnowledgeStore,
        coordinator: &crate::services::twin_events::MutationCoordinator,
    ) -> PendingOptimizerWrite {
        match service
            .prepare_next_expecting_authority(
                store,
                &UserSettings::default(),
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap()
        {
            OptimizerTick::Pending(pending) => *pending,
            other => panic!("expected a pending authority write, got {other:?}"),
        }
    }

    #[test]
    fn locked_restart_cleans_only_canonical_optimizer_orphan_temps() {
        let data_dir = tempdir().unwrap();
        let service = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
        let change_temp = service.changes_dir.join(format!(".{}.tmp", Uuid::new_v4()));
        let pending_temp = service
            .optimizer_dir
            .join(PENDING_PUBLICATIONS_DIRECTORY)
            .join(format!(".{}.tmp", Uuid::new_v4()));
        std::fs::write(&change_temp, b"fsynced orphan").unwrap();
        std::fs::write(&pending_temp, b"fsynced orphan").unwrap();
        drop(service);

        let restarted = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();

        assert!(!change_temp.exists());
        assert!(!pending_temp.exists());
        assert!(restarted.load_pending_publications().unwrap().is_empty());
    }

    #[test]
    fn noncanonical_optimizer_temp_is_not_deleted_as_an_orphan() {
        let data_dir = tempdir().unwrap();
        let service = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
        let unknown = service.changes_dir.join(".not-a-uuid.tmp");
        std::fs::write(&unknown, b"untrusted file").unwrap();
        drop(service);

        assert!(VaultOptimizerService::try_new(data_dir.path().to_path_buf()).is_err());
        assert!(unknown.exists());
    }

    #[test]
    fn corrupt_optimizer_audits_fail_closed_on_restart() {
        let decisions_dir = tempdir().unwrap();
        let decisions = VaultOptimizerService::try_new(decisions_dir.path().to_path_buf()).unwrap();
        std::fs::write(&decisions.decisions_path, b"{not-json").unwrap();
        drop(decisions);
        assert!(VaultOptimizerService::try_new(decisions_dir.path().to_path_buf()).is_err());

        let events_dir = tempdir().unwrap();
        let events = VaultOptimizerService::try_new(events_dir.path().to_path_buf()).unwrap();
        std::fs::write(&events.events_path, b"{not-json\n").unwrap();
        drop(events);
        assert!(VaultOptimizerService::try_new(events_dir.path().to_path_buf()).is_err());
    }

    #[test]
    fn state_lock_and_protected_io_share_one_retained_root_capability() {
        let original = tempdir().unwrap();
        let replacement = tempdir().unwrap();
        let mut service = VaultOptimizerService::try_new(original.path().to_path_buf()).unwrap();
        service
            .bootstrap_checked(&[make_note("original-note", "Original")])
            .unwrap();
        let mut replacement_service =
            VaultOptimizerService::try_new(replacement.path().to_path_buf()).unwrap();
        replacement_service
            .bootstrap_checked(&[make_note("replacement-note", "Replacement")])
            .unwrap();

        let lock = service.acquire_state_lock().unwrap();
        service.optimizer_dir = replacement_service.optimizer_dir.clone();
        service.reload_from_disk_checked().unwrap();
        lock.unlock().unwrap();

        assert_eq!(service.state.queue.len(), 1);
        assert_eq!(service.state.queue[0].note_id, "original-note");
    }

    #[test]
    fn legacy_queue_job_id_is_stable_across_restarts() {
        let data_dir = tempdir().unwrap();
        let optimizer_dir = data_dir.path().join("vault_migration/optimizer");
        std::fs::create_dir_all(&optimizer_dir).unwrap();
        std::fs::write(
            optimizer_dir.join(QUEUE_KEY),
            serde_json::to_vec_pretty(&serde_json::json!({
                "queue": [{
                    "note_id": "legacy-note",
                    "reason": "legacy",
                    "enqueued_at": "2026-08-30T00:00:00Z",
                    "attempts": 0
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let first = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
        let first_id = first.state.queue[0].job_id.clone();
        parse_canonical_uuid(&first_id, "legacy optimizer job ID").unwrap();
        drop(first);
        let second = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();

        assert_eq!(second.state.queue[0].job_id, first_id);
    }

    #[test]
    fn prepared_hook_failure_releases_retry_fence_for_same_process_retry() {
        let (_vault, _data, coordinator, mut store, mut service, _) =
            authority_optimizer_fixture("Prepared Hook Failure");
        let first = prepare_authority_write(&mut service, &store, &coordinator);
        let first_change_id = first.change_id.clone();
        let job_id = first.job.job_id.clone();
        service.fail_next_prepared_publication();

        assert!(matches!(
            service.apply_pending(&mut store, first).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        assert!(service.load_pending_publications().unwrap().is_empty());

        let retry = prepare_authority_write(&mut service, &store, &coordinator);
        assert_eq!(retry.job.job_id, job_id);
        assert_ne!(retry.change_id, first_change_id);
    }

    #[test]
    fn receipt_capacity_failure_releases_retry_fence_for_same_process_retry() {
        let (_vault, data, coordinator, mut store, mut service, _) =
            authority_optimizer_fixture("Receipt Capacity Failure");
        let receipts = data.path().join("twin/mutations/receipts/v1");
        for index in 0..256_u16 {
            std::fs::write(receipts.join(format!("{index:064x}.json")), b"{}").unwrap();
        }
        let first = prepare_authority_write(&mut service, &store, &coordinator);
        let first_change_id = first.change_id.clone();
        let job_id = first.job.job_id.clone();

        assert!(matches!(
            service.apply_pending(&mut store, first).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        assert!(service.load_pending_publications().unwrap().is_empty());

        let retry = prepare_authority_write(&mut service, &store, &coordinator);
        assert_eq!(retry.job.job_id, job_id);
        assert_ne!(retry.change_id, first_change_id);
    }

    #[test]
    fn ambiguous_retry_fence_stage_failure_preserves_adoptable_owner() {
        let (_vault, _data, coordinator, mut store, mut service, note) =
            authority_optimizer_fixture("Retry Fence Stage Failure");
        let first = prepare_authority_write(&mut service, &store, &coordinator);
        let first_change_id = first.change_id.clone();
        let job_id = first.job.job_id.clone();
        service.fail_next_retry_fence_stage_after_write();

        assert!(matches!(
            service.apply_pending(&mut store, first).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        let pending = service.load_pending_publications().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.change_id, first_change_id);
        assert_eq!(pending[0].1.phase, OptimizerPublicationPhase::RetryFenced);
        assert_eq!(service.state.queue[0].attempts, 0);
        assert!(!store.overlay_path(&note.id).exists());

        let retry = match service
            .prepare_next_expecting_authority(
                &store,
                &UserSettings::default(),
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap()
        {
            OptimizerTick::RetryFenced(retry) => *retry,
            other => panic!("expected the stable retry fence, got {other:?}"),
        };
        assert_eq!(retry.publication.job.job_id, job_id);
        assert_eq!(retry.publication.change_id, first_change_id);
        assert!(matches!(
            service.apply_retry_fenced(&mut store, retry).unwrap(),
            OptimizerMutationResult::Committed { .. }
        ));
        assert!(store.overlay_path(&note.id).exists());
    }

    #[test]
    fn optimizer_source_snapshot_rejects_oversize_before_witness() {
        let (_vault, _data, coordinator, mut store, mut service, note) =
            authority_optimizer_fixture("Oversize Overlay Source");
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        let authority_before = coordinator.current_authority_token().unwrap();
        let oversized = serde_json::to_vec(&serde_json::json!({
            "tags": ["x".repeat(crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES)]
        }))
        .unwrap();
        std::fs::write(store.overlay_path(&note.id), oversized).unwrap();

        assert!(store.optimizer_overlay_snapshot(&note.id).is_err());
        assert!(matches!(
            service.apply_pending(&mut store, pending).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        assert_eq!(service.state.queue[0].attempts, 1);
        assert_eq!(
            coordinator.current_authority_token().unwrap(),
            authority_before
        );
        assert!(service.load_pending_publications().unwrap().is_empty());
        assert!(service.list_decisions(10).unwrap().is_empty());

        assert!(matches!(
            service
                .prepare_next_expecting_authority(
                    &store,
                    &UserSettings::default(),
                    authority_before.clone(),
                )
                .unwrap(),
            OptimizerTick::NoWrite
        ));
        assert_eq!(service.state.queue[0].attempts, 2);
        assert!(matches!(
            service
                .prepare_next_expecting_authority(
                    &store,
                    &UserSettings::default(),
                    authority_before.clone(),
                )
                .unwrap(),
            OptimizerTick::NoWrite
        ));
        assert!(service.state.queue.is_empty());
        assert_eq!(service.inbox(Some("failed"), 10).unwrap().len(), 1);
        assert!(service.list_decisions(10).unwrap().is_empty());
    }

    #[test]
    fn optimizer_prepare_rejects_oversize_markdown_without_authority_or_witness() {
        let (vault, _data, coordinator, store, mut service, note) =
            authority_optimizer_fixture("Oversize Markdown Source");
        let authority_before = coordinator.current_authority_token().unwrap();
        std::fs::write(
            vault.path().join(&note.relative_path),
            vec![b'x'; crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES + 1],
        )
        .unwrap();

        assert!(matches!(
            service
                .prepare_next_expecting_authority(
                    &store,
                    &UserSettings::default(),
                    authority_before.clone(),
                )
                .unwrap(),
            OptimizerTick::NoWrite
        ));
        assert_eq!(service.state.queue[0].attempts, 1);
        assert_eq!(
            coordinator.current_authority_token().unwrap(),
            authority_before
        );
        assert!(service.load_pending_publications().unwrap().is_empty());
    }

    #[test]
    fn optimizer_apply_rejects_symlinked_markdown_without_reading_outside() {
        let (vault, _data, coordinator, mut store, mut service, note) =
            authority_optimizer_fixture("Symlink Markdown Source");
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        let authority_before = coordinator.current_authority_token().unwrap();
        let outside = vault
            .path()
            .parent()
            .unwrap()
            .join(format!("optimizer-outside-{}.md", Uuid::new_v4()));
        let secret = "outside-markdown-must-not-be-read";
        std::fs::write(&outside, secret).unwrap();
        let markdown = vault.path().join(&note.relative_path);
        std::fs::remove_file(&markdown).unwrap();
        if !try_symlink_file(&outside, &markdown) {
            let _ = std::fs::remove_file(&outside);
            return;
        }

        assert!(matches!(
            service.apply_pending(&mut store, pending).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        assert_eq!(std::fs::read(&outside).unwrap(), secret.as_bytes());
        assert_eq!(service.state.queue[0].attempts, 1);
        assert_eq!(
            coordinator.current_authority_token().unwrap(),
            authority_before
        );
        assert!(service.load_pending_publications().unwrap().is_empty());
        std::fs::remove_file(&markdown).unwrap();
        std::fs::remove_file(&outside).unwrap();
    }

    #[test]
    fn optimizer_apply_rejects_symlinked_overlay_without_reading_outside() {
        let (vault, _data, coordinator, mut store, mut service, note) =
            authority_optimizer_fixture("Symlink Overlay Source");
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        let authority_before = coordinator.current_authority_token().unwrap();
        let outside = vault
            .path()
            .parent()
            .unwrap()
            .join(format!("optimizer-outside-{}.json", Uuid::new_v4()));
        let secret = "outside-overlay-must-not-be-read";
        std::fs::write(&outside, format!(r#"{{"tags":["{secret}"]}}"#)).unwrap();
        let overlay = store.overlay_path(&note.id);
        if overlay.exists() {
            std::fs::remove_file(&overlay).unwrap();
        }
        if !try_symlink_file(&outside, &overlay) {
            let _ = std::fs::remove_file(&outside);
            return;
        }

        assert!(matches!(
            service.apply_pending(&mut store, pending).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        assert_eq!(
            std::fs::read_to_string(&outside).unwrap(),
            format!(r#"{{"tags":["{secret}"]}}"#)
        );
        assert_eq!(service.state.queue[0].attempts, 1);
        assert_eq!(
            coordinator.current_authority_token().unwrap(),
            authority_before
        );
        assert!(service.load_pending_publications().unwrap().is_empty());
        std::fs::remove_file(&overlay).unwrap();
        std::fs::remove_file(&outside).unwrap();
    }

    #[test]
    fn pending_publication_rejects_cross_field_corruption() {
        let (_vault, _data, coordinator, mut store, mut service, _) =
            authority_optimizer_fixture("Witness Binding");
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        let _ = service.apply_pending(&mut store, pending).unwrap();
        let (_, valid) = service.load_pending_publications().unwrap().remove(0);

        let mut wrong_kind = valid.clone();
        wrong_kind.decision.kind = "other".into();
        assert!(validate_pending_publication(&wrong_kind).is_err());

        let mut wrong_note = valid.clone();
        wrong_note.decision.note_id = Some("other-note".into());
        assert!(validate_pending_publication(&wrong_note).is_err());

        let mut wrong_mode = valid.clone();
        wrong_mode.change.mode = "full_rewrite".into();
        assert!(validate_pending_publication(&wrong_mode).is_err());

        let mut wrong_after = valid;
        match &mut wrong_after.target {
            OptimizerPublicationTarget::Overlay { after_digest, .. }
            | OptimizerPublicationTarget::Markdown { after_digest, .. } => {
                *after_digest = crate::services::twin_events::digest_bytes(b"wrong-after");
            }
        }
        assert!(validate_pending_publication(&wrong_after).is_err());
    }

    #[test]
    fn exact_overlay_target_completes_as_noop_before_retry_fence() {
        let (_vault, _data, coordinator, mut store, mut service, _) =
            authority_optimizer_fixture("Exact Overlay Noop");
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        let snapshot = store
            .optimizer_note_snapshot(&pending.job.note_id)
            .unwrap()
            .unwrap();
        let overlay = optimizer_sidecar_overlay(
            &pending.proposal,
            snapshot.markdown_precondition.relative_path(),
            snapshot.markdown_precondition.expected_digest(),
        );
        std::fs::write(
            store.overlay_path(&pending.job.note_id),
            serde_json::to_string_pretty(&overlay).unwrap(),
        )
        .unwrap();
        let authority_before = coordinator.current_authority_token().unwrap();

        assert!(matches!(
            service.apply_pending(&mut store, pending).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        assert_eq!(
            coordinator.current_authority_token().unwrap(),
            authority_before
        );
        assert!(service.state.queue.is_empty());
        assert!(service.load_pending_publications().unwrap().is_empty());
        assert!(service.list_decisions(10).unwrap().is_empty());
        assert!(service.inbox(None, 10).unwrap().is_empty());

        let restarted =
            VaultOptimizerService::try_new(coordinator.current_namespace_path().unwrap()).unwrap();
        assert!(restarted.state.queue.is_empty());
        assert!(restarted.load_pending_publications().unwrap().is_empty());
    }

    #[test]
    fn externally_satisfied_markdown_target_cleans_retry_fence_as_noop() {
        let (vault, _data, coordinator, mut store, mut service, note) =
            authority_optimizer_fixture("Exact Markdown Noop");
        let settings = UserSettings {
            background_vault_optimizer_edit_mode: "full_rewrite".into(),
            ..UserSettings::default()
        };
        let pending = match service
            .prepare_next_expecting_authority(
                &store,
                &settings,
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap()
        {
            OptimizerTick::Pending(pending) => *pending,
            other => panic!("expected a pending full rewrite, got {other:?}"),
        };
        let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
        let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
        service.pause_after_retry_fence_once(entered.clone(), resume.clone());
        let owner = std::thread::spawn(move || {
            let result = service.apply_pending(&mut store, pending);
            (service, result)
        });

        entered.wait();
        let witness =
            VaultOptimizerService::try_new(coordinator.current_namespace_path().unwrap()).unwrap();
        let publication = witness.load_pending_publications().unwrap().remove(0).1;
        let exact = publication.change.note_after.unwrap();
        let (_, bytes) = KnowledgeStore::canonical_serialized_note_bytes(&exact).unwrap();
        std::fs::write(vault.path().join(&note.relative_path), bytes).unwrap();
        let authority_before = coordinator.current_authority_token().unwrap();
        resume.wait();

        let (service, result) = owner.join().unwrap();
        assert!(matches!(result.unwrap(), OptimizerMutationResult::NoWrite));
        assert_eq!(
            coordinator.current_authority_token().unwrap(),
            authority_before
        );
        assert!(service.state.queue.is_empty());
        assert!(service.load_pending_publications().unwrap().is_empty());
        assert!(service.list_decisions(10).unwrap().is_empty());
        assert!(service.inbox(None, 10).unwrap().is_empty());

        let restarted =
            VaultOptimizerService::try_new(coordinator.current_namespace_path().unwrap()).unwrap();
        assert!(restarted.state.queue.is_empty());
        assert!(restarted.load_pending_publications().unwrap().is_empty());
    }

    #[test]
    fn sidecar_source_edit_before_coordinator_has_no_authority_or_overlay_effect() {
        let (vault, _data, coordinator, mut store, mut service, note) =
            authority_optimizer_fixture("Sidecar Source Guard");
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        let overlay_path = store.overlay_path(&note.id);
        let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
        let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
        service.pause_before_prepared_hook_once(entered.clone(), resume.clone());
        let owner = std::thread::spawn(move || {
            let result = service.apply_pending(&mut store, pending);
            (service, result)
        });

        entered.wait();
        let markdown_path = vault.path().join(&note.relative_path);
        let mut external = std::fs::read_to_string(&markdown_path).unwrap();
        external.push_str("\n\nExternal edit before sidecar commit.\n");
        std::fs::write(&markdown_path, external).unwrap();
        let authority_before = coordinator.current_authority_token().unwrap();
        resume.wait();

        let (service, result) = owner.join().unwrap();
        assert!(matches!(result.unwrap(), OptimizerMutationResult::NoWrite));
        assert_eq!(
            coordinator.current_authority_token().unwrap(),
            authority_before
        );
        assert!(!overlay_path.exists());
        assert!(service.load_pending_publications().unwrap().is_empty());
        assert!(service.list_decisions(10).unwrap().is_empty());
        assert_eq!(service.state.queue[0].attempts, 1);
    }

    #[test]
    fn prepared_sidecar_guard_abort_retires_exact_owner_and_defers_without_authority() {
        let (vault, data, coordinator, mut store, mut service, note) =
            authority_optimizer_fixture("Prepared Sidecar Guard Abort");
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        let job_id = pending.job.job_id.clone();
        let overlay_path = store.overlay_path(&note.id);
        let namespace = coordinator.current_namespace_path().unwrap();
        let authority_before = coordinator.current_authority_token().unwrap();
        coordinator
            .begin_root_transition()
            .unwrap()
            .publish_namespace_ready(&authority_before)
            .unwrap();
        let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
        let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
        service.pause_after_prepared_publication_once(entered.clone(), resume.clone());
        let owner = std::thread::spawn(move || {
            let result = service.apply_pending(&mut store, pending);
            (service, result)
        });

        entered.wait();
        let mut observer = VaultOptimizerService::try_new(namespace.clone()).unwrap();
        let owners = observer.load_pending_publications().unwrap();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].1.phase, OptimizerPublicationPhase::Prepared);
        let prepared_change_id = owners[0].1.change_id.clone();
        let prepared_mutation_id = owners[0].1.mutation_id.clone().unwrap();
        assert!(!observer
            .abort_precondition_owner_and_defer(
                &prepared_change_id,
                &job_id,
                "wrong-mutation-id",
                &anyhow::anyhow!("must not retire a different owner"),
            )
            .unwrap());
        let unchanged_owners = observer.load_pending_publications().unwrap();
        assert_eq!(unchanged_owners.len(), 1);
        assert_eq!(
            unchanged_owners[0].1.mutation_id.as_ref(),
            Some(&prepared_mutation_id)
        );
        assert_eq!(observer.state.queue[0].attempts, 0);
        assert_eq!(
            std::fs::read_dir(data.path().join("twin/mutations/pending/v1"))
                .unwrap()
                .count(),
            0
        );
        let markdown_path = vault.path().join(&note.relative_path);
        let mut external = std::fs::read(&markdown_path).unwrap();
        external.extend_from_slice(b"\nExternal edit after Prepared publication.\n");
        std::fs::write(&markdown_path, &external).unwrap();
        resume.wait();

        let (service, result) = owner.join().unwrap();
        assert!(matches!(result.unwrap(), OptimizerMutationResult::NoWrite));
        assert_eq!(
            coordinator.current_authority_token().unwrap(),
            authority_before
        );
        coordinator.require_namespace_ready().unwrap();
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert!(!overlay_path.exists());
        assert_eq!(std::fs::read(&markdown_path).unwrap(), external);
        assert!(service.load_pending_publications().unwrap().is_empty());
        assert!(service.list_decisions(10).unwrap().is_empty());
        assert!(service.inbox(None, 10).unwrap().is_empty());
        assert_eq!(service.state.queue.len(), 1);
        assert_eq!(service.state.queue[0].job_id, job_id);
        assert_eq!(service.state.queue[0].attempts, 1);
        assert_eq!(
            std::fs::read_dir(data.path().join("twin/mutations/receipts/v1"))
                .unwrap()
                .count(),
            0
        );

        let restarted = VaultOptimizerService::try_new(namespace).unwrap();
        assert!(restarted.load_pending_publications().unwrap().is_empty());
        assert_eq!(restarted.state.queue[0].attempts, 1);
        assert!(!restarted
            .load_pending_publications()
            .unwrap()
            .iter()
            .any(|(_, owner)| owner.mutation_id.as_ref() == Some(&prepared_mutation_id)));
    }

    #[test]
    fn two_prepared_peers_keep_one_stable_job_owner_through_restart() {
        let (vault, _data, coordinator, mut owner_store, mut owner, _note) =
            authority_optimizer_fixture("Single Pending Owner");
        let namespace = coordinator.current_namespace_path().unwrap();
        let mut peer = VaultOptimizerService::try_new(namespace.clone()).unwrap();
        let mut peer_store = KnowledgeStore::with_event_recorder(
            vault.path().to_path_buf(),
            namespace.clone(),
            coordinator.clone(),
        );
        let expected = coordinator.current_authority_token().unwrap();
        let first = prepare_authority_write(&mut owner, &owner_store, &coordinator);
        let second = prepare_authority_write(&mut peer, &peer_store, &coordinator);
        assert_eq!(first.job.job_id, second.job.job_id);
        assert_ne!(first.change_id, second.change_id);
        let first_change_id = first.change_id.clone();
        let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
        let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
        owner.pause_after_retry_fence_once(entered.clone(), resume.clone());
        let owner_thread = std::thread::spawn(move || {
            let result = owner.apply_pending(&mut owner_store, first);
            (owner, result)
        });

        entered.wait();
        assert!(matches!(
            peer.apply_pending(&mut peer_store, second).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        let owners = peer.load_pending_publications().unwrap();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].1.change_id, first_change_id);
        assert_eq!(peer.state.queue[0].attempts, 0);
        assert!(peer.inbox(Some("failed"), 10).unwrap().is_empty());
        resume.wait();

        let (owner, result) = owner_thread.join().unwrap();
        assert!(matches!(
            result.unwrap(),
            OptimizerMutationResult::Committed { .. }
        ));
        drop(owner);
        let mut restarted = VaultOptimizerService::try_new(namespace).unwrap();
        let owners = restarted.load_pending_publications().unwrap();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].1.change_id, first_change_id);

        let state_lock = restarted.acquire_state_lock().unwrap();
        restarted.reload_from_disk_checked().unwrap();
        {
            let guard = coordinator.begin_root_transition().unwrap();
            restarted
                .recover_pending_publications_locked(&peer_store, &guard)
                .unwrap();
        }
        state_lock.unlock().unwrap();
        assert!(restarted.state.queue.is_empty());
        assert!(restarted.load_pending_publications().unwrap().is_empty());
        assert_eq!(restarted.list_decisions(10).unwrap().len(), 1);
        assert_eq!(
            coordinator.current_authority_token().unwrap(),
            crate::services::vault_namespace::VaultAuthorityTokenV1 {
                authority_generation: expected.authority_generation + 1,
                ..expected
            }
        );
    }

    #[test]
    fn duplicate_persisted_job_owners_fail_closed_on_restart() {
        let (_vault, _data, coordinator, mut store, mut service, _) =
            authority_optimizer_fixture("Duplicate Pending Owner");
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        service.fail_next_retry_fence_stage_after_write();
        assert!(matches!(
            service.apply_pending(&mut store, pending).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        let mut duplicate = service.load_pending_publications().unwrap().remove(0).1;
        let duplicate_id = Uuid::new_v4().to_string();
        duplicate.change_id = duplicate_id.clone();
        duplicate.decision.id = duplicate_id.clone();
        duplicate.decision.change_id = Some(duplicate_id.clone());
        duplicate.change.change_id = duplicate_id;
        write_pending_publication(service.retained_optimizer_root().unwrap(), &duplicate).unwrap();
        let namespace = coordinator.current_namespace_path().unwrap();
        drop(service);

        let error = VaultOptimizerService::try_new(namespace).unwrap_err();
        assert!(error
            .to_string()
            .contains("multiple pending-publication owners"));
    }

    #[test]
    fn full_change_audit_rejects_optimizer_before_authority_effect() {
        let (_vault, _data, coordinator, mut store, mut service, note) =
            authority_optimizer_fixture("Full Change Audit");
        let before = coordinator.current_authority_token().unwrap();
        for index in 0..MAX_OPTIMIZER_AUDIT_ENTRIES {
            let change_id = Uuid::from_u128(index as u128 + 1).to_string();
            std::fs::write(
                service.changes_dir.join(format!("{change_id}.json")),
                serde_json::to_vec(&OptimizerChange {
                    change_id,
                    note_id: format!("old-{index}"),
                    mode: "sidecar_first".into(),
                    ..Default::default()
                })
                .unwrap(),
            )
            .unwrap();
        }
        let pending = prepare_authority_write(&mut service, &store, &coordinator);

        assert!(matches!(
            service.apply_pending(&mut store, pending).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        assert_eq!(coordinator.current_authority_token().unwrap(), before);
        assert!(!store.overlay_path(&note.id).exists());
        assert!(service.load_pending_publications().unwrap().is_empty());
        assert_eq!(
            std::fs::read_dir(&service.changes_dir).unwrap().count(),
            4096
        );
    }

    #[test]
    fn pending_reservation_blocks_other_audit_writers_at_boundary() {
        let (_vault, _data, coordinator, mut store, mut service, _) =
            authority_optimizer_fixture("Reserved Audit Slot");
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        let _ = service.apply_pending(&mut store, pending).unwrap();
        let now = Utc::now();
        let inbox = (0..(MAX_OPTIMIZER_AUDIT_ENTRIES - 1))
            .map(|index| VaultOptimizerInboxEntry {
                id: format!("legacy-inbox-{index}"),
                status: "failed".into(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        write_bounded_json_vec(
            service.retained_optimizer_root().unwrap(),
            INBOX_KEY,
            &inbox,
        )
        .unwrap();
        assert!(service
            .push_inbox(VaultOptimizerInboxEntry {
                id: "would-consume-reserved-inbox".into(),
                status: "failed".into(),
                ..Default::default()
            })
            .is_err());

        let events = (0..(MAX_OPTIMIZER_AUDIT_ENTRIES - 1))
            .map(|index| OptimizerAuditEventV1::Rollback {
                change_id: format!("legacy-rollback-{index}"),
                at: now,
            })
            .collect::<Vec<_>>();
        service.write_events(&events).unwrap();
        assert!(service
            .append_event(OptimizerAuditEventV1::Rollback {
                change_id: "would-consume-reserved-event".into(),
                at: now,
            })
            .is_err());
    }

    #[test]
    fn peer_recovery_preserves_a_live_retry_fence_and_its_last_audit_slot() {
        let (vault, _data, coordinator, mut store, mut service, _) =
            authority_optimizer_fixture("Live Retry Fence");
        let inbox = (0..(MAX_OPTIMIZER_AUDIT_ENTRIES - 1))
            .map(|index| VaultOptimizerInboxEntry {
                id: format!("existing-inbox-{index}"),
                status: "failed".into(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        write_bounded_json_vec(
            service.retained_optimizer_root().unwrap(),
            INBOX_KEY,
            &inbox,
        )
        .unwrap();
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
        let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
        service.pause_after_retry_fence_once(entered.clone(), resume.clone());

        let owner = std::thread::spawn(move || {
            let outcome = service.apply_pending(&mut store, pending);
            (service, store, outcome)
        });
        entered.wait();

        let namespace = coordinator.current_namespace_path().unwrap();
        let peer_store = KnowledgeStore::with_event_recorder(
            vault.path().to_path_buf(),
            namespace.clone(),
            coordinator.clone(),
        );
        let mut peer = VaultOptimizerService::try_new(namespace).unwrap();
        let state_lock = peer.acquire_state_lock().unwrap();
        peer.reload_from_disk_checked().unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        peer.recover_pending_publications_locked(&peer_store, &guard)
            .unwrap();
        drop(guard);
        state_lock.unlock().unwrap();

        assert!(matches!(
            peer.prepare_next_expecting_authority(
                &peer_store,
                &UserSettings::default(),
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap(),
            OptimizerTick::RetryFenced(_)
        ));
        assert!(peer
            .push_inbox(VaultOptimizerInboxEntry {
                id: "would-steal-live-reservation".into(),
                status: "failed".into(),
                ..Default::default()
            })
            .is_err());

        resume.wait();
        let (_service, _store, outcome) = owner.join().unwrap();
        assert!(matches!(
            outcome.unwrap(),
            OptimizerMutationResult::Committed { .. }
        ));
    }

    #[test]
    fn committed_publication_replays_all_phases_once_after_restart() {
        let (_vault, _data, coordinator, mut store, mut service, note) =
            authority_optimizer_fixture("Restarted Publication");
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        let outcome = service.apply_pending(&mut store, pending).unwrap();
        assert!(matches!(outcome, OptimizerMutationResult::Committed { .. }));
        drop(service);

        let namespace = coordinator.current_namespace_path().unwrap();
        let mut restarted = VaultOptimizerService::try_new(namespace.clone()).unwrap();
        let state_lock = restarted.acquire_state_lock().unwrap();
        restarted.reload_from_disk_checked().unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        restarted
            .recover_pending_publications_locked(&store, &guard)
            .unwrap();
        state_lock.unlock().unwrap();
        drop(guard);

        assert!(restarted.load_pending_publications().unwrap().is_empty());
        assert!(restarted
            .state
            .queue
            .iter()
            .all(|job| job.note_id != note.id));
        assert_eq!(restarted.state.accepted_count, 1);
        assert_eq!(restarted.load_decisions().unwrap().len(), 1);
        assert_eq!(restarted.load_inbox().unwrap().len(), 1);
        assert_eq!(restarted.load_events().unwrap().len(), 1);

        drop(restarted);
        let replayed = VaultOptimizerService::try_new(namespace).unwrap();
        assert_eq!(replayed.state.accepted_count, 1);
        assert_eq!(replayed.load_decisions().unwrap().len(), 1);
        assert_eq!(replayed.load_inbox().unwrap().len(), 1);
        assert_eq!(replayed.load_events().unwrap().len(), 1);
    }

    #[test]
    fn completed_witness_survives_receipt_delete_before_witness_delete() {
        let (_vault, _data, coordinator, mut store, mut service, _) =
            authority_optimizer_fixture("Receipt Witness Gap");
        let pending = prepare_authority_write(&mut service, &store, &coordinator);
        let _ = service.apply_pending(&mut store, pending).unwrap();
        let state_lock = service.acquire_state_lock().unwrap();
        service.reload_from_disk_checked().unwrap();
        let (_, mut publication) = service.load_pending_publications().unwrap().remove(0);
        service
            .finalize_pending_publication(&mut publication)
            .unwrap();
        let mutation_id = publication.mutation_id.clone().unwrap();
        state_lock.unlock().unwrap();

        let guard = coordinator.begin_root_transition().unwrap();
        guard
            .consume_witnessed_mutation_receipt(&mutation_id)
            .unwrap();
        drop(guard);
        drop(service);

        let namespace = coordinator.current_namespace_path().unwrap();
        let mut restarted = VaultOptimizerService::try_new(namespace).unwrap();
        let state_lock = restarted.acquire_state_lock().unwrap();
        restarted.reload_from_disk_checked().unwrap();
        let guard = coordinator.begin_root_transition().unwrap();
        restarted
            .recover_pending_publications_locked(&store, &guard)
            .unwrap();
        state_lock.unlock().unwrap();

        assert!(restarted.load_pending_publications().unwrap().is_empty());
        assert_eq!(restarted.state.accepted_count, 1);
        assert_eq!(restarted.load_decisions().unwrap().len(), 1);
        assert_eq!(restarted.load_events().unwrap().len(), 1);
    }

    #[test]
    fn queue_state_writes_are_atomic_with_no_tmp_litter() {
        let data_dir = tempdir().expect("temp dir should be created");
        let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());

        service.bootstrap(&[make_note("note-1", "Optimizer Adoption")]);

        let persisted =
            std::fs::read_to_string(&service.queue_path).expect("queue.json should exist");
        assert!(persisted.contains("note-1"));
        assert_no_tmp_siblings(&service.optimizer_dir);
    }

    #[test]
    fn peer_instances_reload_revision_before_enqueuing() {
        let data_dir = tempdir().expect("temp dir should be created");
        let mut first = VaultOptimizerService::new(data_dir.path().to_path_buf());
        let mut peer = VaultOptimizerService::new(data_dir.path().to_path_buf());

        first
            .with_locked_fresh_state(|service| {
                service.enqueue_note_checked("note-a", "first").map(|_| ())
            })
            .unwrap();
        let first_revision = first.state_revision();
        peer.with_locked_fresh_state(|service| {
            service.enqueue_note_checked("note-b", "peer").map(|_| ())
        })
        .unwrap();
        let peer_revision = peer.state_revision();
        let queued = first
            .with_locked_fresh_state(|service| {
                Ok(service
                    .state
                    .queue
                    .iter()
                    .map(|entry| entry.note_id.clone())
                    .collect::<std::collections::BTreeSet<_>>())
            })
            .unwrap();

        assert_eq!(queued, ["note-a".to_string(), "note-b".to_string()].into());
        assert!(peer_revision > first_revision);
        assert_eq!(first.state_revision(), peer_revision);
    }

    #[test]
    fn daily_write_cap_defers_third_write_in_same_day() {
        let vault_dir = tempdir().expect("vault tempdir should be created");
        let data_dir = tempdir().expect("data tempdir should be created");
        let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
            data_dir.path(),
        ));
        event_store.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                data_dir.path(),
                vault_dir.path(),
                event_store.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store = KnowledgeStore::with_event_recorder(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
            coordinator,
        );

        let notes = vec![
            store
                .create_note(make_note_create("Alpha Topic"))
                .expect("note 1 should be created"),
            store
                .create_note(make_note_create("Beta Topic"))
                .expect("note 2 should be created"),
            store
                .create_note(make_note_create("Gamma Topic"))
                .expect("note 3 should be created"),
        ];

        let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
        service.bootstrap(&notes);
        assert_eq!(service.state.queue.len(), 3);

        let settings = UserSettings {
            background_vault_optimizer_max_daily_writes: 2,
            ..UserSettings::default()
        };

        run_one_optimizer_write(&mut service, &mut store, &settings);
        run_one_optimizer_write(&mut service, &mut store, &settings);
        assert_eq!(
            service.state.queue.len(),
            1,
            "two notes should have been dequeued after being written"
        );
        assert_eq!(service.state.accepted_count, 2);
        assert_eq!(service.state.daily_write_count, 2);

        let queue_before_cap = service.state.queue.clone();
        assert!(matches!(
            service
                .prepare_next(&store, &settings)
                .expect("tick 3 (capped) should not error"),
            OptimizerTick::NoWrite
        ));
        assert_eq!(
            service.state.queue.len(),
            1,
            "the third note must stay queued once the daily cap is hit"
        );
        assert_eq!(
            service.state.queue, queue_before_cap,
            "the deferred job must be untouched (no attempts bump, no removal)"
        );
        assert_eq!(
            service.state.accepted_count, 2,
            "no write should be recorded past the daily cap"
        );
        assert_eq!(
            event_store.ordered_events().unwrap().len(),
            3,
            "sidecar overlays and capped no-ops must not emit beyond note creation"
        );
    }

    #[test]
    fn stale_optimizer_source_aborts_before_overlay_and_queue_publication() {
        let vault_dir = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
            data_dir.path(),
        ));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                data_dir.path(),
                vault_dir.path(),
                events,
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let namespace = coordinator.current_namespace_path().unwrap();
        let mut store = KnowledgeStore::with_event_recorder(
            vault_dir.path().to_path_buf(),
            namespace.clone(),
            coordinator.clone(),
        );
        let note = store.create_note(make_note_create("Stale Topic")).unwrap();
        let source = coordinator.current_authority_token().unwrap();
        let mut service = VaultOptimizerService::new(namespace);
        service.bootstrap(std::slice::from_ref(&note));
        let queue_before = service.state.queue.clone();

        store
            .update_note(
                &note.id,
                NoteUpdate {
                    tags: Some(vec!["peer".into()]),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_ne!(
            coordinator.current_authority_token().unwrap(),
            source,
            "peer note update must advance the exact optimizer source authority"
        );
        let pending = match service
            .prepare_next_expecting_authority(&store, &UserSettings::default(), source)
            .unwrap()
        {
            OptimizerTick::Pending(pending) => *pending,
            other => panic!("expected a pending stale-authority write, got {other:?}"),
        };
        assert!(matches!(
            service.apply_pending(&mut store, pending).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        assert_eq!(service.state.queue.len(), queue_before.len());
        assert_eq!(service.state.queue[0].job_id, queue_before[0].job_id);
        assert_eq!(service.state.queue[0].attempts, 1);
        assert!(!store.overlay_path(&note.id).exists());
        assert_eq!(service.state.accepted_count, 0);
        assert!(service.load_pending_publications().unwrap().is_empty());
    }

    #[test]
    fn postwrite_publication_failure_returns_explicit_committed_result() {
        let vault_dir = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
            data_dir.path(),
        ));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                data_dir.path(),
                vault_dir.path(),
                events,
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let namespace = coordinator.current_namespace_path().unwrap();
        let mut store = KnowledgeStore::with_event_recorder(
            vault_dir.path().to_path_buf(),
            namespace.clone(),
            coordinator.clone(),
        );
        let note = store
            .create_note(make_note_create("Committed Optimizer Topic"))
            .unwrap();
        let expected = coordinator.current_authority_token().unwrap();
        let mut service = VaultOptimizerService::new(namespace);
        service.bootstrap(std::slice::from_ref(&note));

        // The governed overlay write and retained receipt succeed, while the
        // committed-witness publication is faulted before the coordinator
        // releases its retained process guard.
        service.fail_next_committed_publication();

        let tick = service
            .prepare_next_expecting_authority(&store, &UserSettings::default(), expected)
            .expect("a durable governed write must not be returned as retryable failure");
        let outcome = match tick {
            OptimizerTick::Pending(pending) => service
                .apply_pending(&mut store, *pending)
                .expect("a durable governed write must not be returned as retryable failure"),
            other => panic!("expected pending optimizer write, got {other:?}"),
        };
        match outcome {
            OptimizerMutationResult::Committed {
                result,
                commit,
                warning,
            } => {
                assert_eq!(result.note_id(), note.id);
                assert!(commit.authority_token.is_some());
                assert!(warning.is_some());
            }
            other => panic!("expected explicit committed optimizer result, got {other:?}"),
        }
        assert!(store.overlay_path(&note.id).exists());
        let pending = service.load_pending_publications().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].1.phase, OptimizerPublicationPhase::Prepared);
        assert!(pending[0].1.retry_fenced);
        assert!(
            matches!(
                service
                    .prepare_next_expecting_authority(
                        &store,
                        &UserSettings::default(),
                        coordinator.current_authority_token().unwrap(),
                    )
                    .unwrap(),
                OptimizerTick::NoWrite
            ),
            "the durable publication witness must fence the queued job from retry"
        );
    }

    #[test]
    fn stale_rollback_source_aborts_before_overlay_and_rollback_state_publication() {
        let vault_dir = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
            data_dir.path(),
        ));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                data_dir.path(),
                vault_dir.path(),
                events,
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let namespace = coordinator.current_namespace_path().unwrap();
        let mut store = KnowledgeStore::with_event_recorder(
            vault_dir.path().to_path_buf(),
            namespace.clone(),
            coordinator.clone(),
        );
        let note = store
            .create_note(make_note_create("Rollback Source"))
            .unwrap();
        let old_overlay = serde_json::json!({"tags": ["before"]});
        let overlay = serde_json::json!({"tags": ["optimizer"]});
        store.write_overlay(&note.id, &old_overlay).unwrap();
        store.write_overlay(&note.id, &overlay).unwrap();
        let source = coordinator.current_authority_token().unwrap();
        let mut service = VaultOptimizerService::new(namespace);
        let change_id = Uuid::new_v4().to_string();
        std::fs::write(
            service.changes_dir.join(format!("{change_id}.json")),
            serde_json::to_vec_pretty(&OptimizerChange {
                change_id: change_id.to_string(),
                note_id: note.id.clone(),
                mode: "sidecar_first".to_string(),
                overlay_before: Some(old_overlay),
                overlay_after: Some(overlay.clone()),
                note_before: None,
                note_after: None,
                markdown_before_digest: None,
                markdown_relative_path: None,
                created_at: Some(Utc::now()),
            })
            .unwrap(),
        )
        .unwrap();

        store.create_note(make_note_create("Peer Change")).unwrap();
        assert_ne!(coordinator.current_authority_token().unwrap(), source);
        let error = service
            .rollback_change_expecting_authority(&change_id, &mut store, source)
            .unwrap_err();

        assert!(error.to_string().contains("authority"));
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(store.overlay_path(&note.id)).unwrap())
                .unwrap(),
            overlay
        );
        assert_eq!(service.state.rollback_count, 0);
        assert!(!service.events_path.exists());
    }

    #[test]
    fn sidecar_rollback_restores_an_absent_overlay() {
        let vault_dir = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
            data_dir.path(),
        ));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                data_dir.path(),
                vault_dir.path(),
                events,
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let namespace = coordinator.current_namespace_path().unwrap();
        let mut store = KnowledgeStore::with_event_recorder(
            vault_dir.path().to_path_buf(),
            namespace.clone(),
            coordinator.clone(),
        );
        let note = store
            .create_note(make_note_create("Absent Overlay"))
            .unwrap();
        let overlay = serde_json::json!({"tags": ["optimizer"]});
        store.write_overlay(&note.id, &overlay).unwrap();
        let source = coordinator.current_authority_token().unwrap();
        let mut service = VaultOptimizerService::new(namespace);
        let change_id = Uuid::new_v4().to_string();
        std::fs::write(
            service.changes_dir.join(format!("{change_id}.json")),
            serde_json::to_vec_pretty(&OptimizerChange {
                change_id: change_id.to_string(),
                note_id: note.id.clone(),
                mode: "sidecar_first".to_string(),
                overlay_before: None,
                overlay_after: Some(overlay),
                note_before: None,
                note_after: None,
                markdown_before_digest: None,
                markdown_relative_path: None,
                created_at: Some(Utc::now()),
            })
            .unwrap(),
        )
        .unwrap();

        let result = service
            .rollback_change_expecting_authority(&change_id, &mut store, source)
            .unwrap();

        assert!(result.rolled_back);
        assert!(!store.overlay_path(&note.id).exists());
        assert_eq!(service.state.rollback_count, 1);
    }

    #[test]
    fn apply_pending_merges_against_current_note_not_stale_snapshot() {
        // Between `prepare_next` (read lock) and `apply_pending` (write lock)
        // there is a real await suspension in the background worker, so a
        // concurrent user `update_note` can land in the gap. The apply stage
        // must merge the proposal's ADDITIONS against the note's CURRENT
        // state, not the snapshot captured in `prepare_next` — otherwise it
        // silently drops the user's fresh tag and reverts their rename.
        let vault_dir = tempdir().expect("vault tempdir should be created");
        let data_dir = tempdir().expect("data tempdir should be created");
        let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
            data_dir.path(),
        ));
        event_store.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                data_dir.path(),
                vault_dir.path(),
                event_store.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store = KnowledgeStore::with_event_recorder(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
            coordinator,
        );

        let note = store
            .create_note(make_note_create("Interleaved Edit Topic"))
            .expect("note should be created");

        let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
        service.bootstrap(std::slice::from_ref(&note));

        let settings = UserSettings {
            background_vault_optimizer_edit_mode: "full_rewrite".to_string(),
            ..UserSettings::default()
        };

        let pending = match service
            .prepare_next(&store, &settings)
            .expect("prepare should not error")
        {
            OptimizerTick::Pending(pending) => pending,
            other => panic!(
                "full_rewrite mode must return a pending write, got {:?}",
                other
            ),
        };

        // Simulate the interleaved user edit landing between the read-locked
        // prepare stage and the write-locked apply stage: add a tag and move
        // the note to a new path.
        store
            .update_note(
                &note.id,
                NoteUpdate {
                    tags: Some(vec!["user-fresh-tag".to_string()]),
                    relative_path: Some("renamed-by-user.md".to_string()),
                    ..Default::default()
                },
            )
            .expect("interleaved user edit should succeed");

        let applied = service
            .apply_pending(&mut store, *pending)
            .expect("apply should not error");
        let OptimizerMutationResult::Committed { result, .. } = applied else {
            panic!("apply_pending must report its committed write")
        };
        assert_eq!(result.note_id(), note.id);

        let final_note = store.get_note(&note.id).expect("note should still exist");
        assert!(
            final_note.tags.iter().any(|tag| tag == "user-fresh-tag"),
            "the user's interleaved tag must survive the optimizer apply, got tags: {:?}",
            final_note.tags
        );
        assert!(
            final_note
                .tags
                .iter()
                .any(|tag| tag == "interleaved_edit_topic"),
            "the proposal's additive tag must still be applied, got tags: {:?}",
            final_note.tags
        );
        assert_eq!(
            final_note.relative_path, "renamed-by-user.md",
            "the user's interleaved rename must not be reverted to the snapshot path"
        );
        let events = event_store.ordered_events().unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].context.source_channel.as_str(), "note_editor");
        assert_eq!(events[1].context.source_channel.as_str(), "note_editor");
        assert_eq!(events[2].context.source_channel.as_str(), "vault_optimizer");
    }

    #[test]
    fn external_markdown_edit_between_refetch_and_digest_is_not_overwritten() {
        let (vault, _data, coordinator, mut store, mut service, note) =
            authority_optimizer_fixture("Torn Snapshot Topic");
        let settings = UserSettings {
            background_vault_optimizer_edit_mode: "full_rewrite".into(),
            ..UserSettings::default()
        };
        let pending = match service
            .prepare_next_expecting_authority(
                &store,
                &settings,
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap()
        {
            OptimizerTick::Pending(pending) => *pending,
            other => panic!("expected a pending full rewrite, got {other:?}"),
        };
        let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
        let resume = std::sync::Arc::new(std::sync::Barrier::new(2));
        service.pause_before_markdown_digest_once(entered.clone(), resume.clone());
        let owner = std::thread::spawn(move || {
            let outcome = service.apply_pending(&mut store, pending);
            (store, outcome)
        });

        entered.wait();
        let markdown_path = vault.path().join(&note.relative_path);
        let mut external = std::fs::read_to_string(&markdown_path).unwrap();
        external.push_str("\n\nExternal B survives.\n");
        std::fs::write(&markdown_path, external).unwrap();
        resume.wait();

        let (store, outcome) = owner.join().unwrap();
        assert!(matches!(outcome.unwrap(), OptimizerMutationResult::NoWrite));
        assert!(store
            .get_note(&note.id)
            .unwrap()
            .content
            .contains("External B survives."));
        let restarted =
            VaultOptimizerService::try_new(coordinator.current_namespace_path().unwrap()).unwrap();
        assert_eq!(restarted.state.queue[0].attempts, 1);
        assert!(restarted.load_pending_publications().unwrap().is_empty());
    }

    #[test]
    fn apply_pending_parks_job_when_note_deleted_in_the_gap() {
        // If the note is deleted between prepare and apply, the apply stage
        // must not resurrect it — the job is dropped like any missing note.
        let vault_dir = tempdir().expect("vault tempdir should be created");
        let data_dir = tempdir().expect("data tempdir should be created");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let note = store
            .create_note(make_note_create("Deleted In Gap Topic"))
            .expect("note should be created");

        let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
        service.bootstrap(std::slice::from_ref(&note));

        let settings = UserSettings {
            background_vault_optimizer_edit_mode: "full_rewrite".to_string(),
            ..UserSettings::default()
        };

        let pending = match service
            .prepare_next(&store, &settings)
            .expect("prepare should not error")
        {
            OptimizerTick::Pending(pending) => pending,
            other => panic!(
                "full_rewrite mode must return a pending write, got {:?}",
                other
            ),
        };

        store
            .delete_note(&note.id)
            .expect("interleaved delete should succeed");

        let applied = service
            .apply_pending(&mut store, *pending)
            .expect("apply of a deleted note must not error");
        assert!(matches!(applied, OptimizerMutationResult::NoWrite));

        assert!(
            store.get_note(&note.id).is_err(),
            "the optimizer must not resurrect a note deleted in the gap"
        );
        assert!(
            service.state.queue.is_empty(),
            "the job for a deleted note must be dropped from the queue"
        );
        assert_eq!(
            service.state.accepted_count, 0,
            "no write should be recorded for a deleted note"
        );
    }

    #[test]
    fn apply_noop_does_not_clobber_a_peer_enqueue() {
        let vault_dir = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );
        let removed = store.create_note(make_note_create("Removed job")).unwrap();
        let peer_note = store.create_note(make_note_create("Peer enqueue")).unwrap();
        let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
        service.bootstrap(std::slice::from_ref(&removed));
        let settings = UserSettings {
            background_vault_optimizer_edit_mode: "full_rewrite".into(),
            ..UserSettings::default()
        };
        let pending = match service.prepare_next(&store, &settings).unwrap() {
            OptimizerTick::Pending(pending) => *pending,
            other => panic!("expected pending optimizer write, got {other:?}"),
        };

        let mut peer = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
        peer.with_locked_fresh_state(|peer| {
            assert!(peer.enqueue_note_checked(&peer_note.id, "peer")?);
            Ok(())
        })
        .unwrap();
        store.delete_note(&removed.id).unwrap();

        assert!(matches!(
            service.apply_pending(&mut store, pending).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        let restarted = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
        assert_eq!(restarted.state.queue.len(), 1);
        assert_eq!(restarted.state.queue[0].note_id, peer_note.id);
    }

    #[test]
    fn apply_error_does_not_clobber_a_peer_enqueue() {
        let vault_dir = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );
        let note = store.create_note(make_note_create("Poison Topic")).unwrap();
        let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
        service.bootstrap(std::slice::from_ref(&note));
        let pending = match service
            .prepare_next(&store, &UserSettings::default())
            .unwrap()
        {
            OptimizerTick::Pending(pending) => *pending,
            other => panic!("expected pending optimizer write, got {other:?}"),
        };
        poison_overlay_directory(&store, &note);

        let mut peer = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
        peer.with_locked_fresh_state(|peer| {
            assert!(peer.enqueue_note_checked("peer-survivor", "peer")?);
            Ok(())
        })
        .unwrap();

        assert!(matches!(
            service.apply_pending(&mut store, pending).unwrap(),
            OptimizerMutationResult::NoWrite
        ));
        let restarted = VaultOptimizerService::try_new(data_dir.path().to_path_buf()).unwrap();
        assert_eq!(restarted.state.queue.len(), 2);
        assert_eq!(restarted.state.queue[0].note_id, note.id);
        assert_eq!(restarted.state.queue[0].attempts, 1);
        assert_eq!(restarted.state.queue[1].note_id, "peer-survivor");
    }

    #[test]
    fn run_next_ignores_llm_enabled_because_no_llm_path_exists() {
        // vault_optimizer has no LLM/network call path today:
        // `build_optimizer_proposal` is purely rule-based, and neither
        // `prepare_next` nor `apply_pending` reference `OpenRouterService` or
        // any network client anywhere in this file (confirmed by inspection —
        // there is no seam to stub). This test characterizes that fact:
        // toggling `background_vault_optimizer_llm_enabled` produces
        // identical decisions, proving enabling it doesn't silently add
        // behavior and disabling it doesn't block the rules pipeline. If an
        // LLM-backed enrichment step is ever added, it must be gated on this
        // flag and this test should then be replaced with one that exercises
        // the real seam.
        fn run_with_llm_flag(llm_enabled: bool) -> VaultOptimizerDecision {
            let vault_dir = tempdir().expect("vault tempdir should be created");
            let data_dir = tempdir().expect("data tempdir should be created");
            let mut store = KnowledgeStore::new(
                vault_dir.path().to_path_buf(),
                data_dir.path().to_path_buf(),
            );
            let note = store
                .create_note(make_note_create("Shared Topic"))
                .expect("note should be created");

            let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
            service.bootstrap(&[note]);

            let settings = UserSettings {
                background_vault_optimizer_llm_enabled: llm_enabled,
                ..UserSettings::default()
            };
            run_one_optimizer_write(&mut service, &mut store, &settings);

            service
                .list_decisions(1)
                .expect("decisions should be readable")
                .into_iter()
                .next()
                .expect("a decision should have been recorded")
        }

        let disabled = run_with_llm_flag(false);
        let enabled = run_with_llm_flag(true);

        assert_eq!(disabled.reason, enabled.reason);
        assert_eq!(disabled.diff_preview, enabled.diff_preview);
        assert_eq!(disabled.confidence, enabled.confidence);
    }

    /// Replaces the overlay directory with a regular file so the secure
    /// optimizer snapshot fails before any authority mutation. Returns the
    /// poisoned store and the note that will always fail to process.
    fn seed_poisoned_note(
        vault_dir: &std::path::Path,
        data_dir: &std::path::Path,
    ) -> (KnowledgeStore, Note) {
        let mut store = KnowledgeStore::new(vault_dir.to_path_buf(), data_dir.to_path_buf());
        let note = store
            .create_note(make_note_create("Poison Topic"))
            .expect("note should be created");

        poison_overlay_directory(&store, &note);

        (store, note)
    }

    fn poison_overlay_directory(store: &KnowledgeStore, note: &Note) {
        let overlay_dir = store
            .overlay_path(&note.id)
            .parent()
            .expect("overlay path should have a parent")
            .to_path_buf();
        std::fs::remove_dir_all(&overlay_dir).expect("overlay dir should be removable");
        std::fs::write(&overlay_dir, b"blocking file")
            .expect("blocking file should be writable in place of the overlay dir");
    }

    #[test]
    fn processing_error_keeps_job_queued_and_increments_attempts() {
        let vault_dir = tempdir().expect("vault tempdir should be created");
        let data_dir = tempdir().expect("data tempdir should be created");
        let (store, note) = seed_poisoned_note(vault_dir.path(), data_dir.path());

        let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
        service.bootstrap(std::slice::from_ref(&note));

        let settings = UserSettings::default();

        assert!(matches!(
            service
                .prepare_next(&store, &settings)
                .expect("preparing a processing attempt must not error"),
            OptimizerTick::NoWrite
        ));

        assert_eq!(
            service.state.queue.len(),
            1,
            "a failed job must stay queued, not be dropped"
        );
        assert_eq!(service.state.queue[0].note_id, note.id);
        assert_eq!(
            service.state.queue[0].attempts, 1,
            "the first failure should record exactly one attempt"
        );
        assert_eq!(
            service.state.accepted_count, 0,
            "no write should have been recorded for a failed job"
        );
    }

    #[test]
    fn poison_job_is_parked_after_max_attempts() {
        let vault_dir = tempdir().expect("vault tempdir should be created");
        let data_dir = tempdir().expect("data tempdir should be created");
        let (store, note) = seed_poisoned_note(vault_dir.path(), data_dir.path());

        let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
        service.bootstrap(std::slice::from_ref(&note));

        let settings = UserSettings::default();

        for attempt in 1..=MAX_OPTIMIZER_ATTEMPTS {
            assert!(matches!(
                service
                    .prepare_next(&store, &settings)
                    .expect("preparing a processing attempt must not error"),
                OptimizerTick::NoWrite
            ));
            if attempt < MAX_OPTIMIZER_ATTEMPTS {
                assert_eq!(
                    service.state.queue.len(),
                    1,
                    "job should still be queued before the attempt limit"
                );
            }
        }

        assert!(
            service.state.queue.is_empty(),
            "a poisoned job must be dropped from the queue after {} attempts",
            MAX_OPTIMIZER_ATTEMPTS
        );
        let inbox = service
            .inbox(Some("failed"), 10)
            .expect("inbox should be readable");
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].note_id.as_deref(), Some(note.id.as_str()));
        assert_eq!(inbox[0].status, "failed");
    }

    #[test]
    fn root_retarget_discards_old_queue_and_bootstraps_only_new_vault_ids() {
        let old_vault = tempdir().unwrap();
        let new_vault = tempdir().unwrap();
        let data = tempdir().unwrap();
        let mut old_store =
            KnowledgeStore::new(old_vault.path().to_path_buf(), data.path().to_path_buf());
        let mut new_store =
            KnowledgeStore::new(new_vault.path().to_path_buf(), data.path().to_path_buf());
        let old_note = old_store.create_note(make_note_create("Old root")).unwrap();
        let new_note = new_store.create_note(make_note_create("New root")).unwrap();
        let mut service = VaultOptimizerService::new(data.path().to_path_buf());
        service.bootstrap(std::slice::from_ref(&old_note));
        assert_eq!(service.state.queue[0].note_id, old_note.id);

        service.reset_for_vault(std::slice::from_ref(&new_note));
        assert_eq!(service.state.queue.len(), 1);
        assert_eq!(service.state.queue[0].note_id, new_note.id);
        assert!(service
            .state
            .queue
            .iter()
            .all(|entry| entry.note_id != old_note.id));
    }
}
