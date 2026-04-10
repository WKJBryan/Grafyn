use crate::models::migration::{
    VaultOptimizerDecision, VaultOptimizerInboxEntry, VaultOptimizerRollbackResult,
    VaultOptimizerStatus,
};
use crate::models::note::{
    Note, NoteUpdate, PROP_INFERRED_LINK_IDS, PROP_TOPIC_ALIASES, PROP_TOPIC_KEY,
    CURRENT_NOTE_SCHEMA_VERSION,
};
use crate::models::settings::UserSettings;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::topic_hub::normalize_topic_key;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QueuedOptimizerNote {
    note_id: String,
    reason: String,
    enqueued_at: DateTime<Utc>,
}

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

    pub fn inbox(&self, status: Option<&str>, limit: usize) -> Result<Vec<VaultOptimizerInboxEntry>> {
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

    pub fn run_next(&mut self, store: &mut KnowledgeStore, settings: &UserSettings) -> Result<()> {
        if !settings.background_vault_optimizer_enabled {
            return Ok(());
        }

        let Some(job) = self.state.queue.first().cloned() else {
            return Ok(());
        };
        self.state.queue.remove(0);

        let note = match store.get_note(&job.note_id) {
            Ok(note) => note,
            Err(error) => {
                log::warn!("Skipping optimizer note '{}': {}", job.note_id, error);
                self.persist_state()?;
                return Ok(());
            }
        };

        if note.is_topic_hub() {
            self.state.last_run_at = Some(Utc::now());
            self.persist_state()?;
            return Ok(());
        }

        let proposal = build_optimizer_proposal(&note, store)?;
        if proposal.is_empty() {
            self.state.last_run_at = Some(Utc::now());
            self.persist_state()?;
            return Ok(());
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

        let change = if settings.background_vault_optimizer_edit_mode == "sidecar_first" {
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
            OptimizerChange {
                change_id: change_id.clone(),
                note_id: note.id.clone(),
                mode: "sidecar_first".to_string(),
                overlay_before,
                overlay_after: Some(overlay_after),
                created_at: Some(Utc::now()),
                ..Default::default()
            }
        } else {
            let note_before = note.clone();
            let updated = store.update_note(
                &note.id,
                NoteUpdate {
                    title: None,
                    content: None,
                    relative_path: Some(note.relative_path.clone()),
                    aliases: Some(merge_unique_strings(note.aliases.clone(), proposal.aliases)),
                    status: None,
                    tags: Some(merge_unique_strings(note.tags.clone(), proposal.tags)),
                    schema_version: Some(CURRENT_NOTE_SCHEMA_VERSION),
                    migration_source: Some("vault_optimizer".to_string()),
                    optimizer_managed: Some(false),
                    properties: Some(merge_note_properties(note.properties.clone(), proposal.properties)),
                },
            )?;
            OptimizerChange {
                change_id: change_id.clone(),
                note_id: note.id.clone(),
                mode: settings.background_vault_optimizer_edit_mode.clone(),
                note_before: Some(note_before),
                note_after: Some(updated),
                created_at: Some(Utc::now()),
                ..Default::default()
            }
        };

        let inbox_entry = VaultOptimizerInboxEntry {
            id: change_id.clone(),
            note_id: Some(note.id.clone()),
            status: "applied".to_string(),
            title: note.title.clone(),
            reason: decision.reason.clone(),
            diff_preview: decision.diff_preview.clone(),
            confidence: decision.confidence,
            created_at: decision.created_at,
            change_id: Some(change_id.clone()),
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
        self.persist_state()?;
        Ok(())
    }

    fn persist_state(&self) -> Result<()> {
        std::fs::create_dir_all(&self.optimizer_dir)?;
        std::fs::write(&self.queue_path, serde_json::to_string_pretty(&self.state)?)?;
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
        std::fs::write(&self.decisions_path, serde_json::to_string_pretty(&decisions)?)?;
        Ok(())
    }

    fn load_inbox(&self) -> Result<Vec<VaultOptimizerInboxEntry>> {
        let inbox = std::fs::read_to_string(&self.inbox_path)
            .ok()
            .and_then(|content| serde_json::from_str::<Vec<VaultOptimizerInboxEntry>>(&content).ok())
            .unwrap_or_default();
        Ok(inbox)
    }

    fn push_inbox(&self, entry: VaultOptimizerInboxEntry) -> Result<()> {
        let mut inbox = self.load_inbox()?;
        inbox.push(entry);
        std::fs::write(&self.inbox_path, serde_json::to_string_pretty(&inbox)?)?;
        Ok(())
    }

    fn write_change(&self, change: &OptimizerChange) -> Result<()> {
        std::fs::create_dir_all(&self.changes_dir)?;
        std::fs::write(
            self.changes_dir.join(format!("{}.json", change.change_id)),
            serde_json::to_string_pretty(change)?,
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
        if note.content.to_lowercase().contains(&candidate.title.to_lowercase()) {
            inferred_link_ids.push(candidate.id);
        }
    }
    inferred_link_ids.truncate(3);

    let merged_aliases = merge_unique_strings(Vec::new(), aliases);
    let merged_tags = merge_unique_strings(Vec::new(), tags);
    let new_aliases = merged_aliases
        .into_iter()
        .filter(|alias| !note.aliases.iter().any(|existing| existing.eq_ignore_ascii_case(alias)))
        .collect::<Vec<_>>();
    let new_tags = merged_tags
        .into_iter()
        .filter(|tag| !note.tags.iter().any(|existing| existing.eq_ignore_ascii_case(tag)))
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
            if note.content.len() > 400 { "updated" } else { "checked" }
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
