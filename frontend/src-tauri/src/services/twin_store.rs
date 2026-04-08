use crate::models::twin::{
    ExportBundle, ExportFileSummary, PromotionState, RecordOrigin, SessionTrace, TraceEvent,
    TraceEventType, TwinExportRequest, UserRecord, UserRecordCreate, UserRecordUpdate,
};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const DEFAULT_EVAL_PERCENTAGE: u8 = 10;
const DEFAULT_HOLDOUT_PERCENTAGE: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportSplit {
    Train,
    Eval,
    Holdout,
}

#[derive(Debug)]
pub struct TwinStore {
    root_path: PathBuf,
    traces_path: PathBuf,
    records_path: PathBuf,
    exports_path: PathBuf,
    trace_cache: HashMap<String, SessionTrace>,
    record_cache: HashMap<String, UserRecord>,
    records_cache_ready: bool,
}

impl TwinStore {
    pub fn new(root_path: PathBuf) -> Self {
        let traces_path = root_path.join("traces");
        let records_path = root_path.join("records");
        let exports_path = root_path.join("exports");

        std::fs::create_dir_all(&traces_path).ok();
        std::fs::create_dir_all(&records_path).ok();
        std::fs::create_dir_all(&exports_path).ok();

        Self {
            root_path,
            traces_path,
            records_path,
            exports_path,
            trace_cache: HashMap::new(),
            record_cache: HashMap::new(),
            records_cache_ready: false,
        }
    }

    pub fn append_trace_event(
        &mut self,
        session_id: &str,
        event_type: TraceEventType,
        payload: serde_json::Value,
    ) -> Result<TraceEvent> {
        Self::validate_file_id(session_id)?;
        let now = Utc::now();
        let event = TraceEvent {
            id: uuid::Uuid::new_v4().to_string(),
            event_type,
            created_at: now,
            payload,
        };

        let trace = self.get_or_create_trace_mut(session_id)?;
        trace.updated_at = now;
        trace.events.push(event.clone());
        let trace = trace.clone();
        self.write_trace_file(&trace)?;

        Ok(event)
    }

    pub fn get_session_trace(&mut self, session_id: &str) -> Result<SessionTrace> {
        Self::validate_file_id(session_id)?;
        if let Some(trace) = self.trace_cache.get(session_id) {
            return Ok(trace.clone());
        }

        let path = self.trace_file_path(session_id);
        if !path.exists() {
            let trace = SessionTrace::new(session_id);
            self.trace_cache
                .insert(session_id.to_string(), trace.clone());
            return Ok(trace);
        }

        let trace = self.read_trace_file(&path)?;
        self.trace_cache
            .insert(session_id.to_string(), trace.clone());
        Ok(trace)
    }

    pub fn list_user_records(&mut self) -> Result<Vec<UserRecord>> {
        self.ensure_record_cache()?;
        let mut records: Vec<UserRecord> = self.record_cache.values().cloned().collect();
        records.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(records)
    }

    pub fn get_user_record(&mut self, id: &str) -> Result<UserRecord> {
        Self::validate_file_id(id)?;
        if let Some(record) = self.record_cache.get(id) {
            return Ok(record.clone());
        }

        let path = self.record_file_path(id);
        let record = self.read_record_file(&path)?;
        self.record_cache.insert(id.to_string(), record.clone());
        Ok(record)
    }

    pub fn create_user_record(&mut self, create: UserRecordCreate) -> Result<UserRecord> {
        self.ensure_record_cache()?;

        let now = Utc::now();
        let promotion_state = create
            .promotion_state
            .unwrap_or_else(|| PromotionState::default_for_origin(&create.origin));

        let record = UserRecord {
            id: uuid::Uuid::new_v4().to_string(),
            kind: create.kind,
            content: create.content,
            evidence_refs: create.evidence_refs,
            confidence: create.confidence.clamp(0.0, 1.0),
            origin: create.origin,
            promotion_state,
            created_at: now,
            updated_at: now,
            valid_from: create.valid_from,
            valid_until: create.valid_until,
            links: create.links,
            metadata: create.metadata,
        };

        self.record_cache.insert(record.id.clone(), record.clone());
        self.write_record_file(&record)?;

        Ok(record)
    }

    pub fn update_user_record(&mut self, id: &str, update: UserRecordUpdate) -> Result<UserRecord> {
        self.ensure_record_cache()?;
        let mut record = self.get_user_record(id)?;

        if let Some(content) = update.content {
            record.content = content;
        }
        if let Some(confidence) = update.confidence {
            record.confidence = confidence.clamp(0.0, 1.0);
        }
        if let Some(promotion_state) = update.promotion_state {
            record.promotion_state = promotion_state;
        }
        if let Some(valid_from) = update.valid_from {
            record.valid_from = Some(valid_from);
        }
        if let Some(valid_until) = update.valid_until {
            record.valid_until = Some(valid_until);
        }
        if let Some(links) = update.links {
            record.links = links;
        }
        if let Some(metadata) = update.metadata {
            record.metadata = metadata;
        }

        record.updated_at = Utc::now();
        self.record_cache.insert(record.id.clone(), record.clone());
        self.write_record_file(&record)?;

        Ok(record)
    }

    pub fn export_bundle(&mut self, request: TwinExportRequest) -> Result<ExportBundle> {
        self.ensure_record_cache()?;

        let eval_percentage = request.eval_percentage.unwrap_or(DEFAULT_EVAL_PERCENTAGE);
        let holdout_percentage = request
            .holdout_percentage
            .unwrap_or(DEFAULT_HOLDOUT_PERCENTAGE);

        if eval_percentage + holdout_percentage >= 100 {
            anyhow::bail!("Eval and holdout percentages must total less than 100");
        }

        let bundle_name = request.bundle_name.as_deref().unwrap_or("latest").trim();
        if bundle_name.is_empty() {
            anyhow::bail!("Bundle name cannot be empty");
        }
        Self::validate_file_id(bundle_name)?;

        let output_dir = self.exports_path.join(bundle_name);
        std::fs::create_dir_all(&output_dir).with_context(|| {
            format!(
                "Failed to create export directory: {}",
                output_dir.display()
            )
        })?;

        let train_path = output_dir.join("train.jsonl");
        let eval_path = output_dir.join("eval.jsonl");
        let holdout_path = output_dir.join("holdout.jsonl");
        let manifest_path = output_dir.join("manifest.json");

        let mut records: Vec<UserRecord> = self.record_cache.values().cloned().collect();
        records.sort_by(|a, b| a.id.cmp(&b.id));

        let mut train_lines = Vec::new();
        let mut eval_lines = Vec::new();
        let mut holdout_lines = Vec::new();
        let mut included_record_ids = Vec::new();
        let mut excluded_synthetic = 0_usize;
        let mut excluded_non_promoted = 0_usize;

        for record in records {
            if !Self::is_record_exportable(&record) {
                match record.origin {
                    RecordOrigin::Synthetic | RecordOrigin::Inferred => {
                        excluded_synthetic += 1;
                    }
                    RecordOrigin::User => {
                        excluded_non_promoted += 1;
                    }
                }
                continue;
            }

            included_record_ids.push(record.id.clone());
            let line = serde_json::to_string(&Self::record_to_export_value(&record))?;
            match Self::split_for_record(&record, eval_percentage, holdout_percentage) {
                ExportSplit::Train => train_lines.push(line),
                ExportSplit::Eval => eval_lines.push(line),
                ExportSplit::Holdout => holdout_lines.push(line),
            }
        }

        self.write_jsonl_file(&train_path, &train_lines)?;
        self.write_jsonl_file(&eval_path, &eval_lines)?;
        self.write_jsonl_file(&holdout_path, &holdout_lines)?;

        let manifest = serde_json::json!({
            "generated_at": Utc::now(),
            "root_path": self.root_path.display().to_string(),
            "eval_percentage": eval_percentage,
            "holdout_percentage": holdout_percentage,
            "included_record_ids": included_record_ids,
            "excluded_counts": {
                "synthetic_or_inferred_without_endorsement": excluded_synthetic,
                "not_promoted_or_disallowed": excluded_non_promoted,
            }
        });
        self.write_pretty_json(&manifest_path, &manifest)?;

        Ok(ExportBundle {
            output_dir: output_dir.display().to_string(),
            train: ExportFileSummary {
                path: train_path.display().to_string(),
                count: train_lines.len(),
            },
            eval: ExportFileSummary {
                path: eval_path.display().to_string(),
                count: eval_lines.len(),
            },
            holdout: ExportFileSummary {
                path: holdout_path.display().to_string(),
                count: holdout_lines.len(),
            },
            manifest_path: manifest_path.display().to_string(),
            included_records: train_lines.len() + eval_lines.len() + holdout_lines.len(),
            excluded_records: excluded_synthetic + excluded_non_promoted,
        })
    }

    fn get_or_create_trace_mut(&mut self, session_id: &str) -> Result<&mut SessionTrace> {
        if !self.trace_cache.contains_key(session_id) {
            let path = self.trace_file_path(session_id);
            let trace = if path.exists() {
                self.read_trace_file(&path)?
            } else {
                SessionTrace::new(session_id)
            };
            self.trace_cache.insert(session_id.to_string(), trace);
        }

        Ok(self
            .trace_cache
            .get_mut(session_id)
            .expect("trace inserted"))
    }

    fn ensure_record_cache(&mut self) -> Result<()> {
        if self.records_cache_ready {
            return Ok(());
        }

        for entry in WalkDir::new(&self.records_path)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let record = self.read_record_file(path)?;
                self.record_cache.insert(record.id.clone(), record);
            }
        }

        self.records_cache_ready = true;
        Ok(())
    }

    fn is_record_exportable(record: &UserRecord) -> bool {
        match record.promotion_state {
            PromotionState::Rejected | PromotionState::Private | PromotionState::NoTrain => false,
            PromotionState::Candidate => false,
            PromotionState::AutoPromoted | PromotionState::Endorsed => match record.origin {
                RecordOrigin::User => true,
                RecordOrigin::Synthetic | RecordOrigin::Inferred => {
                    record.promotion_state == PromotionState::Endorsed
                }
            },
        }
    }

    fn split_for_record(
        record: &UserRecord,
        eval_percentage: u8,
        holdout_percentage: u8,
    ) -> ExportSplit {
        let key = format!(
            "{}:{}:{}:{}",
            record.id,
            serde_json::to_string(&record.kind).unwrap_or_default(),
            record.content,
            record.updated_at
        );
        let bucket = Self::stable_bucket(&key);

        if bucket < holdout_percentage {
            ExportSplit::Holdout
        } else if bucket < holdout_percentage + eval_percentage {
            ExportSplit::Eval
        } else {
            ExportSplit::Train
        }
    }

    fn stable_bucket(key: &str) -> u8 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() % 100) as u8
    }

    fn record_to_export_value(record: &UserRecord) -> serde_json::Value {
        serde_json::json!({
            "id": record.id,
            "kind": record.kind,
            "content": record.content,
            "origin": record.origin,
            "promotion_state": record.promotion_state,
            "confidence": record.confidence,
            "created_at": record.created_at,
            "updated_at": record.updated_at,
            "valid_from": record.valid_from,
            "valid_until": record.valid_until,
            "links": record.links,
            "evidence_refs": record.evidence_refs,
            "metadata": record.metadata,
        })
    }

    fn trace_file_path(&self, session_id: &str) -> PathBuf {
        self.traces_path.join(format!("{}.json", session_id))
    }

    fn record_file_path(&self, record_id: &str) -> PathBuf {
        self.records_path.join(format!("{}.json", record_id))
    }

    fn read_trace_file(&self, path: &Path) -> Result<SessionTrace> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read trace file: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse trace file: {}", path.display()))
    }

    fn read_record_file(&self, path: &Path) -> Result<UserRecord> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read record file: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse record file: {}", path.display()))
    }

    fn write_trace_file(&self, trace: &SessionTrace) -> Result<()> {
        let path = self.trace_file_path(&trace.session_id);
        self.write_pretty_json(&path, trace)
    }

    fn write_record_file(&self, record: &UserRecord) -> Result<()> {
        let path = self.record_file_path(&record.id);
        self.write_pretty_json(&path, record)
    }

    fn write_jsonl_file(&self, path: &Path, lines: &[String]) -> Result<()> {
        let mut content = lines.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write JSONL file: {}", path.display()))
    }

    fn write_pretty_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        let content = serde_json::to_string_pretty(value)?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write JSON file: {}", path.display()))
    }

    fn validate_file_id(id: &str) -> Result<()> {
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            anyhow::bail!("Invalid file id: {}", id);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin::{
        default_record_confidence, PromotionState, RecordLink, RecordLinkType, UserRecordKind,
    };
    use tempfile::tempdir;

    #[test]
    fn synthetic_records_require_endorsement_for_export() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let synthetic = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "Synthesized summary".to_string(),
                origin: RecordOrigin::Synthetic,
                evidence_refs: Vec::new(),
                confidence: default_record_confidence(),
                promotion_state: None,
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("synthetic record should be created");

        let bundle = store
            .export_bundle(TwinExportRequest::default())
            .expect("export should succeed");
        assert_eq!(bundle.included_records, 0);

        store
            .update_user_record(
                &synthetic.id,
                UserRecordUpdate {
                    promotion_state: Some(PromotionState::Endorsed),
                    ..UserRecordUpdate::default()
                },
            )
            .expect("synthetic record should update");

        let bundle = store
            .export_bundle(TwinExportRequest::default())
            .expect("endorsed export should succeed");
        assert_eq!(bundle.included_records, 1);
    }

    #[test]
    fn user_records_auto_promote_and_export() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let record = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Fact,
                content: "I prefer blunt feedback over flattery.".to_string(),
                origin: RecordOrigin::User,
                evidence_refs: Vec::new(),
                confidence: 0.9,
                promotion_state: None,
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("user record should be created");

        assert_eq!(record.promotion_state, PromotionState::AutoPromoted);

        let bundle = store
            .export_bundle(TwinExportRequest::default())
            .expect("export should succeed");
        assert_eq!(bundle.included_records, 1);
    }

    #[test]
    fn export_preserves_history_through_links_instead_of_overwriting() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let old_record = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "Preferred shorter answers earlier.".to_string(),
                origin: RecordOrigin::User,
                evidence_refs: Vec::new(),
                confidence: 0.7,
                promotion_state: Some(PromotionState::Endorsed),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("old record should be created");

        let new_record = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "Now prefers blunt, longer answers when accuracy matters.".to_string(),
                origin: RecordOrigin::User,
                evidence_refs: Vec::new(),
                confidence: 0.9,
                promotion_state: Some(PromotionState::Endorsed),
                valid_from: None,
                valid_until: None,
                links: vec![RecordLink {
                    relation: RecordLinkType::Supersedes,
                    target_record_id: old_record.id.clone(),
                }],
                metadata: HashMap::new(),
            })
            .expect("new record should be created");

        let listed = store.list_user_records().expect("records should list");
        let old = listed
            .iter()
            .find(|record| record.id == old_record.id)
            .expect("old record should remain present");
        let new = listed
            .iter()
            .find(|record| record.id == new_record.id)
            .expect("new record should remain present");

        assert!(old.links.is_empty());
        assert_eq!(new.links.len(), 1);
        assert_eq!(new.links[0].target_record_id, old_record.id);
    }

    #[test]
    fn append_trace_event_creates_trace_file() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let event = store
            .append_trace_event(
                "session-1",
                TraceEventType::PromptSubmitted,
                serde_json::json!({ "prompt": "hello" }),
            )
            .expect("trace event should append");

        let trace = store
            .get_session_trace("session-1")
            .expect("trace should load");

        assert_eq!(trace.events.len(), 1);
        assert_eq!(trace.events[0].id, event.id);
        assert_eq!(trace.events[0].event_type, TraceEventType::PromptSubmitted);
    }
}
