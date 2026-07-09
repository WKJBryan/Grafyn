use crate::models::migration::{
    VaultOptimizerDecision, VaultOptimizerInboxEntry, VaultOptimizerRollbackResult,
    VaultOptimizerStatus,
};
use crate::models::note::{
    Note, NoteUpdate, CURRENT_NOTE_SCHEMA_VERSION, PROP_INFERRED_LINK_IDS, PROP_TOPIC_ALIASES,
    PROP_TOPIC_KEY,
};
use crate::models::settings::UserSettings;
use crate::services::atomic_io::write_atomic;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::topic_hub::normalize_topic_key;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct QueuedOptimizerNote {
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct OptimizerState {
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
    created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct VaultOptimizerService {
    optimizer_dir: PathBuf,
    queue_path: PathBuf,
    decisions_path: PathBuf,
    inbox_path: PathBuf,
    events_path: PathBuf,
    changes_dir: PathBuf,
    state: OptimizerState,
}

impl VaultOptimizerService {
    pub fn new(data_path: PathBuf) -> Self {
        let optimizer_dir = data_path.join("vault_migration").join("optimizer");
        let queue_path = optimizer_dir.join("queue.json");
        let decisions_path = optimizer_dir.join("decisions.json");
        let inbox_path = optimizer_dir.join("inbox.json");
        let events_path = optimizer_dir.join("events.jsonl");
        let changes_dir = optimizer_dir.join("changes");
        let _ = std::fs::create_dir_all(&changes_dir);

        let state = std::fs::read_to_string(&queue_path)
            .ok()
            .and_then(|content| serde_json::from_str::<OptimizerState>(&content).ok())
            .unwrap_or_default();

        Self {
            optimizer_dir,
            queue_path,
            decisions_path,
            inbox_path,
            events_path,
            changes_dir,
            state,
        }
    }

    pub fn bootstrap(&mut self, notes: &[Note]) {
        if self.state.queue.is_empty() {
            for note in notes.iter().filter(|note| !note.is_topic_hub()) {
                self.enqueue_note(&note.id, "bootstrap");
            }
            let _ = self.persist_state();
        }
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
            note_id: note_id.to_string(),
            reason: reason.to_string(),
            enqueued_at: Utc::now(),
            attempts: 0,
        });
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
        let path = self.changes_dir.join(format!("{}.json", change_id));
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("Optimizer change '{}' not found", change_id))?;
        let change: OptimizerChange = serde_json::from_str(&data)?;

        if let Some(overlay_before) = change.overlay_before {
            if overlay_before.is_null() {
                store.delete_overlay(&change.note_id)?;
            } else {
                store.write_overlay(&change.note_id, &overlay_before)?;
            }
        } else if let Some(note_before) = change.note_before {
            store.update_note(
                &change.note_id,
                NoteUpdate {
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
                },
            )?;
        }

        self.state.rollback_count += 1;
        self.persist_state()?;
        self.append_event(json!({
            "type": "rollback",
            "change_id": change_id,
            "at": Utc::now(),
        }))?;

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
    /// `Some(PendingOptimizerWrite)` for the caller to apply via
    /// [`Self::apply_pending`] under a write lock. This mirrors
    /// `link_discovery::discover_for_note`'s snapshot-under-read-lock /
    /// work-lock-free / write-lock-only-to-apply pattern: the background
    /// worker (`main.rs::start_vault_optimizer_worker`) no longer needs to hold
    /// `knowledge_store.write()` for the whole run, only for the narrow
    /// `apply_pending` step when one is actually needed.
    pub fn prepare_next(
        &mut self,
        store: &KnowledgeStore,
        settings: &UserSettings,
    ) -> Result<Option<PendingOptimizerWrite>> {
        if !settings.background_vault_optimizer_enabled {
            return Ok(None);
        }

        let Some(job) = self.state.queue.first().cloned() else {
            return Ok(None);
        };

        let note = match store.get_note(&job.note_id) {
            Ok(note) => note,
            Err(error) => {
                log::warn!("Skipping optimizer note '{}': {}", job.note_id, error);
                self.remove_queued_job(&job.note_id);
                self.persist_state()?;
                return Ok(None);
            }
        };

        if note.is_topic_hub() {
            self.complete_noop_job(&job.note_id)?;
            return Ok(None);
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
            return Ok(None);
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
                return Ok(None);
            }
        };

        if proposal.is_empty() {
            self.complete_noop_job(&job.note_id)?;
            return Ok(None);
        }

        if self.daily_write_cap_reached(settings) {
            log::info!(
                "Vault optimizer deferring note '{}': daily write cap ({}) reached",
                note.id,
                settings.background_vault_optimizer_max_daily_writes
            );
            // Leave the job queued untouched; it's retried on a later tick
            // (possibly after the daily counter rolls over to a new date).
            return Ok(None);
        }

        let change_id = Uuid::new_v4().to_string();
        let decision = VaultOptimizerDecision {
            id: change_id.clone(),
            note_id: Some(note.id.clone()),
            kind: "optimizer_update".to_string(),
            confidence: proposal.confidence,
            reason: proposal.reason.clone(),
            diff_preview: proposal.diff_preview.clone(),
            created_at: Some(Utc::now()),
            change_id: Some(change_id.clone()),
        };

        if settings.background_vault_optimizer_edit_mode == "sidecar_first" {
            let write_result = (|| -> Result<OptimizerChange> {
                let overlay_before = read_overlay_value(store, &note.id);
                let overlay_after = json!({
                    "aliases": proposal.aliases,
                    "tags": proposal.tags,
                    "schema_version": CURRENT_NOTE_SCHEMA_VERSION,
                    "migration_source": "vault_optimizer",
                    "optimizer_managed": false,
                    "properties": proposal.properties,
                });
                store.write_overlay(&note.id, &overlay_after)?;
                Ok(OptimizerChange {
                    change_id: change_id.clone(),
                    note_id: note.id.clone(),
                    mode: "sidecar_first".to_string(),
                    overlay_before,
                    overlay_after: Some(overlay_after),
                    created_at: Some(Utc::now()),
                    ..Default::default()
                })
            })();

            match write_result {
                Ok(change) => {
                    self.finalize_applied_change(&job, &note, &change_id, decision, change)?
                }
                Err(error) => self.defer_or_park_job(job, error)?,
            }

            return Ok(None);
        }

        Ok(Some(PendingOptimizerWrite {
            job,
            note,
            proposal,
            change_id,
            decision,
            edit_mode: settings.background_vault_optimizer_edit_mode.clone(),
        }))
    }

    /// Applies a `PendingOptimizerWrite` returned by [`Self::prepare_next`] for
    /// non-`sidecar_first` edit modes. This is the only step in the optimizer
    /// pipeline that needs a mutable, cache-rebuilding `KnowledgeStore` write
    /// lock (`update_note` walks and reparses the vault), so callers should
    /// hold that lock only around this call — not around `prepare_next`.
    pub fn apply_pending(
        &mut self,
        store: &mut KnowledgeStore,
        pending: PendingOptimizerWrite,
    ) -> Result<()> {
        let PendingOptimizerWrite {
            job,
            note,
            proposal,
            change_id,
            decision,
            edit_mode,
        } = pending;

        let write_result = store.update_note(
            &note.id,
            NoteUpdate {
                title: None,
                content: None,
                relative_path: Some(note.relative_path.clone()),
                aliases: Some(merge_unique_strings(
                    note.aliases.clone(),
                    proposal.aliases.clone(),
                )),
                status: None,
                tags: Some(merge_unique_strings(
                    note.tags.clone(),
                    proposal.tags.clone(),
                )),
                schema_version: Some(CURRENT_NOTE_SCHEMA_VERSION),
                migration_source: Some("vault_optimizer".to_string()),
                optimizer_managed: Some(false),
                properties: Some(merge_note_properties(
                    note.properties.clone(),
                    proposal.properties.clone(),
                )),
            },
        );

        match write_result {
            Ok(updated) => {
                let change = OptimizerChange {
                    change_id: change_id.clone(),
                    note_id: note.id.clone(),
                    mode: edit_mode,
                    note_before: Some(note.clone()),
                    note_after: Some(updated),
                    created_at: Some(Utc::now()),
                    ..Default::default()
                };
                self.finalize_applied_change(&job, &note, &change_id, decision, change)
            }
            Err(error) => self.defer_or_park_job(job, error),
        }
    }

    /// Persists an applied change (sidecar overlay or full-rewrite note
    /// update): writes the change record, decision, and inbox entry, bumps
    /// the accepted/daily-write counters, and — only now that the write has
    /// actually succeeded — removes the job from the queue.
    fn finalize_applied_change(
        &mut self,
        job: &QueuedOptimizerNote,
        note: &Note,
        change_id: &str,
        decision: VaultOptimizerDecision,
        change: OptimizerChange,
    ) -> Result<()> {
        let inbox_entry = VaultOptimizerInboxEntry {
            id: change_id.to_string(),
            note_id: Some(note.id.clone()),
            status: "applied".to_string(),
            title: note.title.clone(),
            reason: decision.reason.clone(),
            diff_preview: decision.diff_preview.clone(),
            confidence: decision.confidence,
            created_at: decision.created_at,
            change_id: Some(change_id.to_string()),
        };

        self.write_change(&change)?;
        self.push_decision(decision.clone())?;
        self.push_inbox(inbox_entry)?;
        self.append_event(json!({
            "type": "optimizer_apply",
            "note_id": note.id,
            "change_id": change_id,
            "at": Utc::now(),
            "confidence": decision.confidence,
        }))?;

        self.state.accepted_count += 1;
        self.state.last_run_at = Some(Utc::now());
        self.record_daily_write();
        self.remove_queued_job(&job.note_id);
        self.persist_state()?;
        Ok(())
    }

    /// Records a processing failure for `job`. Below `MAX_OPTIMIZER_ATTEMPTS`
    /// the job stays queued (in its original position) with `attempts`
    /// incremented, so it's retried on a later tick. At the limit it's parked:
    /// removed from the queue and recorded in the inbox with status `"failed"`
    /// so a human can see it, instead of spinning on a poisoned entry forever.
    fn defer_or_park_job(
        &mut self,
        mut job: QueuedOptimizerNote,
        error: anyhow::Error,
    ) -> Result<()> {
        job.attempts += 1;
        log::warn!(
            "Vault optimizer job for note '{}' failed (attempt {}/{}): {}",
            job.note_id,
            job.attempts,
            MAX_OPTIMIZER_ATTEMPTS,
            error
        );

        if job.attempts >= MAX_OPTIMIZER_ATTEMPTS {
            self.remove_queued_job(&job.note_id);
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
            self.append_event(json!({
                "type": "optimizer_parked",
                "note_id": job.note_id,
                "attempts": job.attempts,
                "error": error.to_string(),
                "at": Utc::now(),
            }))?;
        } else if let Some(entry) = self
            .state
            .queue
            .iter_mut()
            .find(|entry| entry.note_id == job.note_id)
        {
            entry.attempts = job.attempts;
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

    fn complete_noop_job(&mut self, note_id: &str) -> Result<()> {
        self.remove_queued_job(note_id);
        self.state.last_run_at = Some(Utc::now());
        self.persist_state()
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

    /// Records that a write happened "now", rolling the counter over to 1 if
    /// the stored date isn't today. Persisted as part of `OptimizerState` so
    /// the cap survives an app restart.
    fn record_daily_write(&mut self) {
        let today = Utc::now().date_naive().to_string();
        if self.state.daily_write_date.as_deref() == Some(today.as_str()) {
            self.state.daily_write_count += 1;
        } else {
            self.state.daily_write_date = Some(today);
            self.state.daily_write_count = 1;
        }
    }

    fn persist_state(&self) -> Result<()> {
        std::fs::create_dir_all(&self.optimizer_dir)?;
        write_atomic(
            &self.queue_path,
            serde_json::to_string_pretty(&self.state)?.as_bytes(),
        )?;
        Ok(())
    }

    fn load_decisions(&self) -> Result<Vec<VaultOptimizerDecision>> {
        let decisions = std::fs::read_to_string(&self.decisions_path)
            .ok()
            .and_then(|content| serde_json::from_str::<Vec<VaultOptimizerDecision>>(&content).ok())
            .unwrap_or_default();
        Ok(decisions)
    }

    fn push_decision(&self, decision: VaultOptimizerDecision) -> Result<()> {
        let mut decisions = self.load_decisions()?;
        decisions.push(decision);
        write_atomic(
            &self.decisions_path,
            serde_json::to_string_pretty(&decisions)?.as_bytes(),
        )?;
        Ok(())
    }

    fn load_inbox(&self) -> Result<Vec<VaultOptimizerInboxEntry>> {
        let inbox = std::fs::read_to_string(&self.inbox_path)
            .ok()
            .and_then(|content| {
                serde_json::from_str::<Vec<VaultOptimizerInboxEntry>>(&content).ok()
            })
            .unwrap_or_default();
        Ok(inbox)
    }

    fn push_inbox(&self, entry: VaultOptimizerInboxEntry) -> Result<()> {
        let mut inbox = self.load_inbox()?;
        inbox.push(entry);
        write_atomic(
            &self.inbox_path,
            serde_json::to_string_pretty(&inbox)?.as_bytes(),
        )?;
        Ok(())
    }

    fn write_change(&self, change: &OptimizerChange) -> Result<()> {
        std::fs::create_dir_all(&self.changes_dir)?;
        write_atomic(
            &self.changes_dir.join(format!("{}.json", change.change_id)),
            serde_json::to_string_pretty(change)?.as_bytes(),
        )?;
        Ok(())
    }

    fn append_event(&self, event: Value) -> Result<()> {
        std::fs::create_dir_all(&self.optimizer_dir)?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.events_path)?;
        writeln!(file, "{}", serde_json::to_string(&event)?)?;
        Ok(())
    }
}

/// A rules-based proposal that was accepted but needs a mutable, cache-
/// rebuilding `KnowledgeStore::update_note` write to apply (i.e. edit_mode is
/// something other than `sidecar_first`, which applies inline within
/// `prepare_next` instead). Returned by [`VaultOptimizerService::prepare_next`]
/// and consumed by [`VaultOptimizerService::apply_pending`].
#[derive(Debug, Clone)]
pub struct PendingOptimizerWrite {
    job: QueuedOptimizerNote,
    note: Note,
    proposal: OptimizerProposal,
    change_id: String,
    decision: VaultOptimizerDecision,
    edit_mode: String,
}

#[derive(Debug, Clone, Default)]
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

fn read_overlay_value(store: &KnowledgeStore, note_id: &str) -> Option<Value> {
    let path = store.overlay_path(note_id);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{NoteCreate, NoteStatus};
    use crate::services::atomic_io::assert_no_tmp_siblings;
    use tempfile::tempdir;

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
    fn daily_write_cap_defers_third_write_in_same_day() {
        let vault_dir = tempdir().expect("vault tempdir should be created");
        let data_dir = tempdir().expect("data tempdir should be created");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
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

        assert!(service
            .prepare_next(&store, &settings)
            .expect("tick 1 should not error")
            .is_none());
        assert!(service
            .prepare_next(&store, &settings)
            .expect("tick 2 should not error")
            .is_none());
        assert_eq!(
            service.state.queue.len(),
            1,
            "two notes should have been dequeued after being written"
        );
        assert_eq!(service.state.accepted_count, 2);
        assert_eq!(service.state.daily_write_count, 2);

        let queue_before_cap = service.state.queue.clone();
        assert!(service
            .prepare_next(&store, &settings)
            .expect("tick 3 (capped) should not error")
            .is_none());
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
            assert!(service
                .prepare_next(&store, &settings)
                .expect("tick should not error")
                .is_none());

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

    /// Forces `write_overlay` to fail by replacing the overlay directory
    /// (auto-created by `KnowledgeStore::new`) with a regular file, so
    /// `create_dir_all(parent)` inside `write_overlay` errors instead of
    /// writing the sidecar JSON. Returns the poisoned store and the note that
    /// will always fail to process.
    fn seed_poisoned_note(
        vault_dir: &std::path::Path,
        data_dir: &std::path::Path,
    ) -> (KnowledgeStore, Note) {
        let mut store = KnowledgeStore::new(vault_dir.to_path_buf(), data_dir.to_path_buf());
        let note = store
            .create_note(make_note_create("Poison Topic"))
            .expect("note should be created");

        let overlay_dir = store
            .overlay_path(&note.id)
            .parent()
            .expect("overlay path should have a parent")
            .to_path_buf();
        std::fs::remove_dir_all(&overlay_dir).expect("overlay dir should be removable");
        std::fs::write(&overlay_dir, b"blocking file")
            .expect("blocking file should be writable in place of the overlay dir");

        (store, note)
    }

    #[test]
    fn processing_error_keeps_job_queued_and_increments_attempts() {
        let vault_dir = tempdir().expect("vault tempdir should be created");
        let data_dir = tempdir().expect("data tempdir should be created");
        let (store, note) = seed_poisoned_note(vault_dir.path(), data_dir.path());

        let mut service = VaultOptimizerService::new(data_dir.path().to_path_buf());
        service.bootstrap(std::slice::from_ref(&note));

        let settings = UserSettings::default();

        assert!(service
            .prepare_next(&store, &settings)
            .expect("a processing error must not bubble up as Err")
            .is_none());

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
            assert!(service
                .prepare_next(&store, &settings)
                .expect("a processing error must not bubble up as Err")
                .is_none());
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
}
