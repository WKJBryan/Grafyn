use crate::models::twin::{
    EvidenceRef, ExportBundle, ExportFileSummary, PromotionState, RecordOrigin,
    ResolvedEvidenceRef, SessionTrace, TraceEvent, TraceEventType, TwinContextRecord,
    TwinExportRequest, TwinInferenceRunSummary, TwinReviewRecord, UserRecord, UserRecordCreate,
    UserRecordKind, UserRecordUpdate,
};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const DEFAULT_EVAL_PERCENTAGE: u8 = 10;
const DEFAULT_HOLDOUT_PERCENTAGE: u8 = 10;
const TWIN_INFERENCE_VERSION: &str = "local-signal-v1";
const AUTO_PROMOTE_CONFIDENCE: f32 = 0.75;
const AUTO_PROMOTE_SUPPORT_COUNT: usize = 3;
const EXCERPT_MAX_CHARS: usize = 220;
const MAX_TWIN_CANDIDATE_CONTEXT_RECORDS: usize = 8;

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

    pub fn run_twin_inference(&mut self) -> Result<TwinInferenceRunSummary> {
        self.ensure_record_cache()?;
        let traces = self.list_session_traces()?;
        let scanned_events = traces.iter().map(|trace| trace.events.len()).sum::<usize>();
        let drafts = infer_behavioral_records(&traces);

        let mut existing_by_key = HashMap::new();
        let mut rejected_keys = HashSet::new();
        for record in self.record_cache.values() {
            if let Some(inference_key) = record
                .metadata
                .get("inference_key")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
            {
                existing_by_key.insert(inference_key.clone(), record.id.clone());
                if record.promotion_state == PromotionState::Rejected {
                    rejected_keys.insert(inference_key);
                }
            }
        }

        let mut created_records = 0_usize;
        let mut updated_records = 0_usize;
        let mut auto_promoted_records = 0_usize;
        let mut candidate_records = 0_usize;
        let mut skipped_rejected_records = 0_usize;

        for draft in drafts {
            let desired_state = if draft.confidence >= AUTO_PROMOTE_CONFIDENCE
                && draft.support_count >= AUTO_PROMOTE_SUPPORT_COUNT
            {
                PromotionState::AutoPromoted
            } else {
                PromotionState::Candidate
            };
            let mut metadata =
                build_inference_metadata(&draft, desired_state == PromotionState::AutoPromoted);

            if let Some(existing_id) = existing_by_key.get(&draft.inference_key).cloned() {
                let mut record = self.get_user_record(&existing_id)?;
                let previous_state = record.promotion_state.clone();
                let is_rejected_key = rejected_keys.contains(&draft.inference_key);

                record.kind = draft.kind.clone();
                record.content = draft.content.clone();
                record.evidence_refs = draft.evidence_refs.clone();
                record.confidence = draft.confidence;
                record.origin = RecordOrigin::Inferred;

                if is_rejected_key {
                    metadata.insert("auto_promoted".to_string(), Value::Bool(false));
                    record.promotion_state = PromotionState::Rejected;
                    skipped_rejected_records += 1;
                } else if matches!(
                    record.promotion_state,
                    PromotionState::Candidate | PromotionState::AutoPromoted
                ) {
                    record.promotion_state = desired_state.clone();
                }

                metadata = merge_promotion_history(record.metadata.clone(), metadata);
                if previous_state != record.promotion_state {
                    append_promotion_history(
                        &mut metadata,
                        &previous_state,
                        &record.promotion_state,
                        Some("local signal inference threshold"),
                        true,
                    );
                }

                record.metadata = metadata;
                record.updated_at = Utc::now();
                self.record_cache.insert(record.id.clone(), record.clone());
                self.write_record_file(&record)?;
                updated_records += 1;
            } else {
                let mut metadata = metadata;
                if desired_state == PromotionState::AutoPromoted {
                    append_promotion_history(
                        &mut metadata,
                        &PromotionState::Candidate,
                        &PromotionState::AutoPromoted,
                        Some("confidence >= 0.75 and support_count >= 3"),
                        true,
                    );
                }

                self.create_user_record(UserRecordCreate {
                    kind: draft.kind,
                    content: draft.content,
                    evidence_refs: draft.evidence_refs,
                    confidence: draft.confidence,
                    origin: RecordOrigin::Inferred,
                    promotion_state: Some(desired_state),
                    valid_from: None,
                    valid_until: None,
                    links: Vec::new(),
                    metadata,
                })?;
                created_records += 1;
            }
        }

        for record in self.record_cache.values() {
            if record.origin != RecordOrigin::Inferred {
                continue;
            }
            match record.promotion_state {
                PromotionState::AutoPromoted => auto_promoted_records += 1,
                PromotionState::Candidate => candidate_records += 1,
                _ => {}
            }
        }

        Ok(TwinInferenceRunSummary {
            inference_version: TWIN_INFERENCE_VERSION.to_string(),
            scanned_traces: traces.len(),
            scanned_events,
            created_records,
            updated_records,
            auto_promoted_records,
            candidate_records,
            skipped_rejected_records,
            generated_at: Utc::now(),
        })
    }

    pub fn get_twin_review(&mut self) -> Result<Vec<TwinReviewRecord>> {
        let records = self.list_user_records()?;
        let mut review = Vec::with_capacity(records.len());

        for record in records {
            let evidence = self.resolve_evidence_refs(&record.evidence_refs)?;
            review.push(TwinReviewRecord {
                evidence_count: record.evidence_refs.len(),
                latest_evidence: evidence.into_iter().max_by_key(|item| item.created_at),
                record,
            });
        }

        review.sort_by(|a, b| {
            promotion_state_sort_key(&a.record.promotion_state)
                .cmp(&promotion_state_sort_key(&b.record.promotion_state))
                .then_with(|| b.record.updated_at.cmp(&a.record.updated_at))
        });

        Ok(review)
    }

    pub fn select_context_records(
        &mut self,
        query: &str,
    ) -> Result<(Vec<TwinContextRecord>, Vec<TwinContextRecord>)> {
        self.ensure_record_cache()?;

        let query_terms = lexical_terms(query);
        let mut approved = Vec::new();
        let mut candidates = Vec::new();

        for record in self.record_cache.values() {
            match record.promotion_state {
                PromotionState::Endorsed | PromotionState::AutoPromoted => {
                    approved.push(twin_context_record(record, "approved"));
                }
                PromotionState::Candidate => {
                    let relevance = twin_record_relevance(record, &query_terms);
                    if relevance > 0 {
                        candidates.push((
                            relevance,
                            record.updated_at,
                            twin_context_record(record, "candidate"),
                        ));
                    }
                }
                PromotionState::Rejected | PromotionState::Private | PromotionState::NoTrain => {}
            }
        }

        approved.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.evidence_count.cmp(&a.evidence_count))
        });

        candidates.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)).then_with(|| {
                b.2.confidence
                    .partial_cmp(&a.2.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });

        Ok((
            approved,
            candidates
                .into_iter()
                .take(MAX_TWIN_CANDIDATE_CONTEXT_RECORDS)
                .map(|(_, _, record)| record)
                .collect(),
        ))
    }

    pub fn resolve_user_record_evidence(&mut self, id: &str) -> Result<Vec<ResolvedEvidenceRef>> {
        let record = self.get_user_record(id)?;
        self.resolve_evidence_refs(&record.evidence_refs)
    }

    pub fn set_user_record_promotion(
        &mut self,
        id: &str,
        promotion_state: PromotionState,
        rationale: Option<String>,
    ) -> Result<UserRecord> {
        self.ensure_record_cache()?;
        let mut record = self.get_user_record(id)?;
        let previous_state = record.promotion_state.clone();
        let now = Utc::now();

        record.promotion_state = promotion_state.clone();
        record.updated_at = now;

        append_promotion_history(
            &mut record.metadata,
            &previous_state,
            &promotion_state,
            rationale.as_deref(),
            false,
        );

        if promotion_state == PromotionState::Rejected {
            record
                .metadata
                .insert("reverted_at".to_string(), json!(now));
            record
                .metadata
                .insert("revert_reason".to_string(), json!(rationale));
            record
                .metadata
                .insert("auto_promoted".to_string(), Value::Bool(false));
        }

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

        let approved_path = output_dir.join("approved_user_records.jsonl");
        let candidate_path = output_dir.join("candidate_user_records.jsonl");
        let rejected_path = output_dir.join("rejected_user_records.jsonl");
        let train_path = output_dir.join("train.jsonl");
        let eval_path = output_dir.join("eval.jsonl");
        let holdout_path = output_dir.join("holdout.jsonl");
        let manifest_path = output_dir.join("manifest.json");

        let mut records: Vec<UserRecord> = self.record_cache.values().cloned().collect();
        records.sort_by(|a, b| a.id.cmp(&b.id));

        let mut approved_lines = Vec::new();
        let mut candidate_lines = Vec::new();
        let mut rejected_lines = Vec::new();
        let mut train_lines = Vec::new();
        let mut eval_lines = Vec::new();
        let mut holdout_lines = Vec::new();
        let mut included_record_ids = Vec::new();
        let mut private_or_no_train_record_ids = Vec::new();

        for record in records {
            let line = serde_json::to_string(&Self::record_to_export_value(&record))?;

            match record.promotion_state {
                PromotionState::AutoPromoted | PromotionState::Endorsed => {
                    included_record_ids.push(record.id.clone());
                    approved_lines.push(line.clone());
                    match Self::split_for_record(&record, eval_percentage, holdout_percentage) {
                        ExportSplit::Train => train_lines.push(line),
                        ExportSplit::Eval => eval_lines.push(line),
                        ExportSplit::Holdout => holdout_lines.push(line),
                    }
                }
                PromotionState::Candidate => {
                    candidate_lines.push(line);
                }
                PromotionState::Rejected => {
                    rejected_lines.push(line);
                }
                PromotionState::Private | PromotionState::NoTrain => {
                    private_or_no_train_record_ids.push(record.id.clone());
                }
            }
        }

        self.write_jsonl_file(&approved_path, &approved_lines)?;
        self.write_jsonl_file(&candidate_path, &candidate_lines)?;
        self.write_jsonl_file(&rejected_path, &rejected_lines)?;
        self.write_jsonl_file(&train_path, &train_lines)?;
        self.write_jsonl_file(&eval_path, &eval_lines)?;
        self.write_jsonl_file(&holdout_path, &holdout_lines)?;

        let manifest = serde_json::json!({
            "generated_at": Utc::now(),
            "root_path": self.root_path.display().to_string(),
            "eval_percentage": eval_percentage,
            "holdout_percentage": holdout_percentage,
            "included_record_ids": included_record_ids,
            "record_files": {
                "approved_user_records": {
                    "path": approved_path.display().to_string(),
                    "count": approved_lines.len(),
                },
                "candidate_user_records": {
                    "path": candidate_path.display().to_string(),
                    "count": candidate_lines.len(),
                },
                "rejected_user_records": {
                    "path": rejected_path.display().to_string(),
                    "count": rejected_lines.len(),
                },
            },
            "excluded_counts": {
                "private_or_no_train": private_or_no_train_record_ids.len(),
            }
        });
        self.write_pretty_json(&manifest_path, &manifest)?;

        Ok(ExportBundle {
            output_dir: output_dir.display().to_string(),
            approved_user_records: ExportFileSummary {
                path: approved_path.display().to_string(),
                count: approved_lines.len(),
            },
            candidate_user_records: ExportFileSummary {
                path: candidate_path.display().to_string(),
                count: candidate_lines.len(),
            },
            rejected_user_records: ExportFileSummary {
                path: rejected_path.display().to_string(),
                count: rejected_lines.len(),
            },
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
            excluded_records: private_or_no_train_record_ids.len(),
        })
    }

    fn list_session_traces(&mut self) -> Result<Vec<SessionTrace>> {
        for entry in WalkDir::new(&self.traces_path)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let trace = self.read_trace_file(path)?;
                self.trace_cache.insert(trace.session_id.clone(), trace);
            }
        }

        let mut traces = self.trace_cache.values().cloned().collect::<Vec<_>>();
        traces.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        Ok(traces)
    }

    fn resolve_evidence_refs(&mut self, refs: &[EvidenceRef]) -> Result<Vec<ResolvedEvidenceRef>> {
        let mut resolved = Vec::new();
        for evidence_ref in refs {
            let trace_id = if evidence_ref.trace_id.trim().is_empty() {
                &evidence_ref.session_id
            } else {
                &evidence_ref.trace_id
            };
            let trace = self.get_session_trace(trace_id)?;
            let Some(event) = trace
                .events
                .iter()
                .find(|event| event.id == evidence_ref.event_id)
            else {
                continue;
            };

            resolved.push(resolve_event_evidence(&trace, event, evidence_ref));
        }

        resolved.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(resolved)
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

#[derive(Debug, Clone)]
struct InferredRecordDraft {
    inference_key: String,
    signal_family: String,
    kind: UserRecordKind,
    content: String,
    evidence_refs: Vec<EvidenceRef>,
    evidence_event_ids: Vec<String>,
    support_count: usize,
    confidence: f32,
}

#[derive(Debug, Default)]
struct SignalAccumulator {
    evidence_refs: Vec<EvidenceRef>,
    evidence_event_ids: HashSet<String>,
}

impl SignalAccumulator {
    fn add_event(&mut self, trace: &SessionTrace, event: &TraceEvent) {
        if !self.evidence_event_ids.insert(event.id.clone()) {
            return;
        }

        self.evidence_refs.push(EvidenceRef {
            trace_id: trace.id.clone(),
            event_id: event.id.clone(),
            session_id: trace.session_id.clone(),
            tile_id: extract_event_tile_id(&event.payload),
            model_id: extract_event_model_id(&event.payload),
            note: evidence_note(event),
        });
    }
}

fn infer_behavioral_records(traces: &[SessionTrace]) -> Vec<InferredRecordDraft> {
    let mut signals: HashMap<String, SignalAccumulator> = HashMap::new();

    for trace in traces {
        for event in &trace.events {
            match &event.event_type {
                TraceEventType::PromptSubmitted => {
                    let prompt = payload_string(&event.payload, &["prompt"]).unwrap_or_default();
                    if event
                        .payload
                        .get("parent_tile_id")
                        .is_some_and(|value| !value.is_null())
                    {
                        add_signal(&mut signals, "reasoning.iterative_deepening", trace, event);
                    }
                    if text_contains_any(
                        &prompt,
                        &["think harder", "revisit", "improve your previous answer"],
                    ) {
                        add_signal(&mut signals, "reasoning.iterative_deepening", trace, event);
                    }
                    if text_contains_any(
                        &prompt,
                        &[
                            "implement",
                            "fix",
                            "test",
                            "build",
                            "file",
                            "branch",
                            "worktree",
                        ],
                    ) {
                        add_signal(
                            &mut signals,
                            "preference.implementation_detail",
                            trace,
                            event,
                        );
                    }
                    if payload_string(&event.payload, &["context_mode"])
                        .is_some_and(|mode| mode == "knowledge_search" || mode == "semantic")
                    {
                        add_signal(&mut signals, "preference.grounded_context", trace, event);
                    }
                    if value_contains_key(&event.payload, "twin_domain") {
                        add_signal(&mut signals, "fact.twin_domain_metadata", trace, event);
                    }
                }
                TraceEventType::FeedbackRecorded => {
                    let feedback_type =
                        payload_string(&event.payload, &["feedback_type"]).unwrap_or_default();
                    let text = event_text(event);

                    match feedback_type.as_str() {
                        "accept" => {
                            if looks_structured(&text) {
                                add_signal(
                                    &mut signals,
                                    "preference.structured_answers",
                                    trace,
                                    event,
                                );
                            }
                            if looks_evidence_backed(&text) {
                                add_signal(
                                    &mut signals,
                                    "preference.evidence_backed_detail",
                                    trace,
                                    event,
                                );
                            }
                            if looks_implementation_detailed(&text) {
                                add_signal(
                                    &mut signals,
                                    "preference.implementation_detail",
                                    trace,
                                    event,
                                );
                            }
                        }
                        "reject" => {
                            add_signal(&mut signals, "preference.rejects_mismatch", trace, event);
                        }
                        "correction" => {
                            add_signal(&mut signals, "preference.rejects_mismatch", trace, event);
                            add_signal(&mut signals, "reasoning.corrects_ai_outputs", trace, event);
                        }
                        _ => {}
                    }
                }
                TraceEventType::RankingRecorded => {
                    add_signal(&mut signals, "reasoning.model_comparison", trace, event);
                    let text = event_text(event);
                    if looks_structured(&text) {
                        add_signal(&mut signals, "preference.structured_answers", trace, event);
                    }
                    if looks_evidence_backed(&text) {
                        add_signal(
                            &mut signals,
                            "preference.evidence_backed_detail",
                            trace,
                            event,
                        );
                    }
                    if looks_implementation_detailed(&text) {
                        add_signal(
                            &mut signals,
                            "preference.implementation_detail",
                            trace,
                            event,
                        );
                    }
                }
                TraceEventType::InsightCaptured => {
                    add_signal(
                        &mut signals,
                        "reasoning.captures_self_knowledge",
                        trace,
                        event,
                    );
                }
                TraceEventType::ModelsAdded => {
                    add_signal(&mut signals, "reasoning.model_comparison", trace, event);
                }
                TraceEventType::DebateStarted | TraceEventType::DebateContinued => {
                    add_signal(&mut signals, "reasoning.model_comparison", trace, event);
                    add_signal(&mut signals, "reasoning.uses_debate", trace, event);
                }
                TraceEventType::NoteExported => {
                    add_signal(
                        &mut signals,
                        "reasoning.curates_evidence_notes",
                        trace,
                        event,
                    );
                }
                TraceEventType::NoteCanonicalPromoted => {
                    add_signal(&mut signals, "reasoning.canonical_validation", trace, event);
                }
                TraceEventType::NoteCreated | TraceEventType::NoteUpdated => {
                    if value_contains_key(&event.payload, "twin_domain") {
                        add_signal(&mut signals, "fact.twin_domain_metadata", trace, event);
                    }
                    if payload_string(&event.payload, &["status"])
                        .is_some_and(|status| status == "canonical")
                    {
                        add_signal(&mut signals, "reasoning.canonical_validation", trace, event);
                    }
                }
                _ => {}
            }
        }
    }

    let mut drafts = signals
        .into_iter()
        .filter_map(|(key, accumulator)| {
            let support_count = accumulator.evidence_refs.len();
            if support_count == 0 {
                return None;
            }

            let (kind, signal_family, content) = signal_definition(&key)?;
            let mut evidence_event_ids = accumulator
                .evidence_event_ids
                .into_iter()
                .collect::<Vec<_>>();
            evidence_event_ids.sort();

            Some(InferredRecordDraft {
                inference_key: key,
                signal_family: signal_family.to_string(),
                kind,
                content: content.to_string(),
                confidence: confidence_for_support(support_count),
                support_count,
                evidence_refs: accumulator.evidence_refs,
                evidence_event_ids,
            })
        })
        .collect::<Vec<_>>();

    drafts.sort_by(|a, b| a.inference_key.cmp(&b.inference_key));
    drafts
}

fn add_signal(
    signals: &mut HashMap<String, SignalAccumulator>,
    key: &str,
    trace: &SessionTrace,
    event: &TraceEvent,
) {
    signals
        .entry(key.to_string())
        .or_default()
        .add_event(trace, event);
}

fn signal_definition(key: &str) -> Option<(UserRecordKind, &'static str, &'static str)> {
    match key {
        "fact.twin_domain_metadata" => Some((
            UserRecordKind::Fact,
            "twin_domain",
            "Uses twin_domain metadata to separate captured knowledge by the domain it belongs to.",
        )),
        "preference.structured_answers" => Some((
            UserRecordKind::Preference,
            "explicit_feedback",
            "Prefers structured answers with headings, lists, or clear steps when judging model output.",
        )),
        "preference.evidence_backed_detail" => Some((
            UserRecordKind::Preference,
            "explicit_feedback",
            "Prefers evidence-backed implementation detail over generic summary.",
        )),
        "preference.implementation_detail" => Some((
            UserRecordKind::Preference,
            "passive_prompting",
            "Prefers answers that include concrete implementation details such as files, commands, tests, or code.",
        )),
        "preference.grounded_context" => Some((
            UserRecordKind::Preference,
            "passive_prompting",
            "Uses existing notes as grounding context when asking models to reason.",
        )),
        "preference.rejects_mismatch" => Some((
            UserRecordKind::Preference,
            "explicit_feedback",
            "Rejects or corrects responses that do not match their knowledge instead of saving them as truth.",
        )),
        "reasoning.corrects_ai_outputs" => Some((
            UserRecordKind::ReasoningPattern,
            "explicit_feedback",
            "Provides corrections when model output conflicts with what they know.",
        )),
        "reasoning.iterative_deepening" => Some((
            UserRecordKind::ReasoningPattern,
            "branching",
            "Revisits answers through branches or think-harder passes before treating them as settled.",
        )),
        "reasoning.model_comparison" => Some((
            UserRecordKind::ReasoningPattern,
            "model_selection",
            "Compares multiple model outputs before selecting what matches their thinking.",
        )),
        "reasoning.uses_debate" => Some((
            UserRecordKind::ReasoningPattern,
            "debate_selection",
            "Uses model debate or disagreement to test alternatives before deciding what to keep.",
        )),
        "reasoning.captures_self_knowledge" => Some((
            UserRecordKind::ReasoningPattern,
            "explicit_feedback",
            "Captures durable facts, preferences, or reasoning patterns as explicit twin records.",
        )),
        "reasoning.curates_evidence_notes" => Some((
            UserRecordKind::ReasoningPattern,
            "note_export",
            "Turns useful canvas work into durable evidence notes.",
        )),
        "reasoning.canonical_validation" => Some((
            UserRecordKind::ReasoningPattern,
            "canonical_promotion",
            "Promotes knowledge to canonical notes after review or repeated validation.",
        )),
        _ => None,
    }
}

fn confidence_for_support(support_count: usize) -> f32 {
    (0.45 + support_count as f32 * 0.10).min(0.95)
}

fn build_inference_metadata(
    draft: &InferredRecordDraft,
    auto_promoted: bool,
) -> HashMap<String, Value> {
    json!({
        "inference_key": draft.inference_key,
        "inference_version": TWIN_INFERENCE_VERSION,
        "signal_family": draft.signal_family,
        "support_count": draft.support_count,
        "evidence_event_ids": draft.evidence_event_ids,
        "auto_promoted": auto_promoted,
    })
    .as_object()
    .cloned()
    .unwrap_or_default()
    .into_iter()
    .collect()
}

fn merge_promotion_history(
    existing: HashMap<String, Value>,
    mut next: HashMap<String, Value>,
) -> HashMap<String, Value> {
    for (key, value) in existing {
        next.entry(key).or_insert(value);
    }
    next
}

fn append_promotion_history(
    metadata: &mut HashMap<String, Value>,
    from: &PromotionState,
    to: &PromotionState,
    rationale: Option<&str>,
    automatic: bool,
) {
    let mut history = metadata
        .get("promotion_history")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    history.push(json!({
        "from": from,
        "to": to,
        "at": Utc::now(),
        "rationale": rationale,
        "automatic": automatic,
    }));
    metadata.insert("promotion_history".to_string(), Value::Array(history));
}

fn promotion_state_sort_key(state: &PromotionState) -> u8 {
    match state {
        PromotionState::AutoPromoted => 0,
        PromotionState::Candidate => 1,
        PromotionState::Endorsed => 2,
        PromotionState::Rejected => 3,
        PromotionState::Private => 4,
        PromotionState::NoTrain => 5,
    }
}

fn twin_context_record(record: &UserRecord, source_label: &str) -> TwinContextRecord {
    TwinContextRecord {
        id: record.id.clone(),
        kind: record.kind.clone(),
        content: record.content.clone(),
        confidence: record.confidence,
        promotion_state: record.promotion_state.clone(),
        evidence_count: record.evidence_refs.len(),
        source_label: Some(source_label.to_string()),
    }
}

fn twin_record_relevance(record: &UserRecord, query_terms: &HashSet<String>) -> usize {
    if query_terms.is_empty() {
        return 0;
    }

    let mut haystack = record.content.clone();
    for evidence in &record.evidence_refs {
        if let Some(note) = &evidence.note {
            haystack.push(' ');
            haystack.push_str(note);
        }
    }
    for value in record.metadata.values() {
        haystack.push(' ');
        haystack.push_str(&value.to_string());
    }

    let record_terms = lexical_terms(&haystack);
    query_terms.intersection(&record_terms).count()
}

fn lexical_terms(text: &str) -> HashSet<String> {
    const STOPWORDS: &[&str] = &[
        "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "for", "from", "how", "i",
        "if", "in", "into", "is", "it", "its", "me", "my", "of", "on", "or", "our", "so", "that",
        "the", "their", "them", "this", "to", "up", "use", "was", "we", "what", "when", "where",
        "which", "who", "why", "with", "would", "you", "your",
    ];

    text.split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|token| {
            let token = token.trim().to_lowercase();
            if token.len() < 3 || STOPWORDS.contains(&token.as_str()) {
                None
            } else {
                Some(token)
            }
        })
        .collect()
}

fn resolve_event_evidence(
    trace: &SessionTrace,
    event: &TraceEvent,
    evidence_ref: &EvidenceRef,
) -> ResolvedEvidenceRef {
    let prompt_excerpt = first_payload_string(
        &event.payload,
        &[
            &["prompt"],
            &["response", "prompt"],
            &["evidence", "prompt"],
            &["ranked_responses", "0", "response", "prompt"],
        ],
    )
    .map(|value| excerpt(&value));

    let response_excerpt = first_payload_string(
        &event.payload,
        &[
            &["content"],
            &["response", "response_content"],
            &["response", "response_excerpt"],
            &["evidence", "response_content"],
            &["ranked_responses", "0", "response", "response_content"],
            &["ranked_responses", "0", "response_excerpt"],
        ],
    )
    .map(|value| excerpt(&value));

    let model_name = first_payload_string(
        &event.payload,
        &[
            &["model_name"],
            &["response", "model_name"],
            &["evidence", "model_name"],
            &["ranked_responses", "0", "response", "model_name"],
        ],
    );

    ResolvedEvidenceRef {
        trace_id: evidence_ref.trace_id.clone(),
        event_id: event.id.clone(),
        session_id: trace.session_id.clone(),
        tile_id: evidence_ref
            .tile_id
            .clone()
            .or_else(|| extract_event_tile_id(&event.payload)),
        model_id: evidence_ref
            .model_id
            .clone()
            .or_else(|| extract_event_model_id(&event.payload)),
        event_type: event.event_type.clone(),
        created_at: event.created_at,
        note: evidence_ref.note.clone(),
        summary: Some(event_summary(
            event,
            prompt_excerpt.as_deref(),
            response_excerpt.as_deref(),
        )),
        prompt_excerpt,
        response_excerpt,
        model_name,
        payload: event.payload.clone(),
    }
}

fn event_summary(
    event: &TraceEvent,
    prompt_excerpt: Option<&str>,
    response_excerpt: Option<&str>,
) -> String {
    match &event.event_type {
        TraceEventType::PromptSubmitted => {
            format!(
                "Prompt submitted: {}",
                prompt_excerpt.unwrap_or("no prompt excerpt")
            )
        }
        TraceEventType::ResponseCompleted => {
            format!(
                "Response completed: {}",
                response_excerpt.unwrap_or("no response excerpt")
            )
        }
        TraceEventType::FeedbackRecorded => {
            let feedback_type = payload_string(&event.payload, &["feedback_type"])
                .unwrap_or_else(|| "feedback".to_string());
            format!("{} feedback recorded", feedback_type)
        }
        TraceEventType::RankingRecorded => "Response ranking recorded".to_string(),
        TraceEventType::InsightCaptured => "Explicit twin insight captured".to_string(),
        TraceEventType::ModelsAdded => "Additional models added for comparison".to_string(),
        TraceEventType::DebateStarted => "Model debate started".to_string(),
        TraceEventType::DebateContinued => "Model debate continued".to_string(),
        TraceEventType::NoteExported => {
            let title =
                payload_string(&event.payload, &["title"]).unwrap_or_else(|| "note".to_string());
            format!("Canvas exported to note: {}", title)
        }
        TraceEventType::NoteCanonicalPromoted => "Note promoted to canonical".to_string(),
        TraceEventType::NoteCreated => "Note created".to_string(),
        TraceEventType::NoteUpdated => "Note updated".to_string(),
        _ => format!("{:?}", &event.event_type),
    }
}

fn evidence_note(event: &TraceEvent) -> Option<String> {
    match &event.event_type {
        TraceEventType::NoteExported => payload_string(&event.payload, &["title"]),
        TraceEventType::FeedbackRecorded
        | TraceEventType::RankingRecorded
        | TraceEventType::InsightCaptured => payload_string(&event.payload, &["rationale"]),
        _ => None,
    }
}

fn event_text(event: &TraceEvent) -> String {
    let mut parts = Vec::new();
    collect_strings(&event.payload, &mut parts);
    parts.join("\n").to_lowercase()
}

fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(value) => out.push(value.clone()),
        Value::Array(items) => {
            for item in items {
                collect_strings(item, out);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_strings(value, out);
            }
        }
        _ => {}
    }
}

fn extract_event_tile_id(payload: &Value) -> Option<String> {
    first_payload_string(
        payload,
        &[
            &["tile_id"],
            &["response", "tile_id"],
            &["evidence", "tile_id"],
            &["ranked_responses", "0", "response", "tile_id"],
        ],
    )
}

fn extract_event_model_id(payload: &Value) -> Option<String> {
    first_payload_string(
        payload,
        &[
            &["model_id"],
            &["response", "model_id"],
            &["evidence", "model_id"],
            &["ranked_responses", "0", "response", "model_id"],
        ],
    )
}

fn first_payload_string(payload: &Value, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| payload_string(payload, path))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn payload_string(payload: &Value, path: &[&str]) -> Option<String> {
    let mut current = payload;
    for part in path {
        if let Ok(index) = part.parse::<usize>() {
            current = current.as_array()?.get(index)?;
        } else {
            current = current.get(*part)?;
        }
    }

    current
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| match current {
            Value::Bool(value) => Some(value.to_string()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
}

fn value_contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key(key) || map.values().any(|value| value_contains_key(value, key))
        }
        Value::Array(items) => items.iter().any(|value| value_contains_key(value, key)),
        _ => false,
    }
}

fn looks_structured(text: &str) -> bool {
    text.contains("\n- ")
        || text.contains("\n* ")
        || text.contains("\n1. ")
        || text.contains("##")
        || text.contains("```")
}

fn looks_evidence_backed(text: &str) -> bool {
    text_contains_any(
        text,
        &[
            "evidence",
            "source",
            "according",
            "because",
            "verify",
            "verified",
            "test",
            "logs",
            "tradeoff",
        ],
    )
}

fn looks_implementation_detailed(text: &str) -> bool {
    text_contains_any(
        text,
        &[
            ".rs",
            ".js",
            ".vue",
            "frontend/",
            "src/",
            "cargo ",
            "npm ",
            "test",
            "function ",
            "```",
            "command",
            "file",
        ],
    )
}

fn text_contains_any(text: &str, needles: &[&str]) -> bool {
    let text = text.to_lowercase();
    needles.iter().any(|needle| text.contains(needle))
}

fn excerpt(content: &str) -> String {
    if content.chars().count() <= EXCERPT_MAX_CHARS {
        return content.to_string();
    }

    let mut excerpt = content.chars().take(EXCERPT_MAX_CHARS).collect::<String>();
    excerpt.push_str("...");
    excerpt
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
        assert_eq!(bundle.approved_user_records.count, 1);
    }

    #[test]
    fn inference_creates_candidate_below_threshold_and_auto_promotes_at_threshold() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        for index in 0..2 {
            store
                .append_trace_event(
                    "session-1",
                    TraceEventType::PromptSubmitted,
                    serde_json::json!({
                        "tile_id": format!("tile-{}", index),
                        "prompt": "Please implement this with files and tests.",
                        "models": ["openai/gpt-4o"],
                    }),
                )
                .expect("trace event should append");
        }

        store.run_twin_inference().expect("inference should run");
        let records = store.list_user_records().expect("records should list");
        let record = records
            .iter()
            .find(|record| {
                record.metadata.get("inference_key").and_then(Value::as_str)
                    == Some("preference.implementation_detail")
            })
            .expect("implementation detail record should exist");
        assert_eq!(record.promotion_state, PromotionState::Candidate);
        assert_eq!(
            record.metadata.get("support_count").and_then(Value::as_u64),
            Some(2)
        );

        store
            .append_trace_event(
                "session-1",
                TraceEventType::PromptSubmitted,
                serde_json::json!({
                    "tile_id": "tile-3",
                    "prompt": "Fix the build and show exact commands.",
                    "models": ["openai/gpt-4o"],
                }),
            )
            .expect("trace event should append");

        store.run_twin_inference().expect("inference should rerun");
        let records = store.list_user_records().expect("records should list");
        let record = records
            .iter()
            .find(|record| {
                record.metadata.get("inference_key").and_then(Value::as_str)
                    == Some("preference.implementation_detail")
            })
            .expect("implementation detail record should exist");
        assert_eq!(record.promotion_state, PromotionState::AutoPromoted);
        assert!(record.confidence >= AUTO_PROMOTE_CONFIDENCE);
    }

    #[test]
    fn repeated_inference_updates_by_key_instead_of_duplicating() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        store
            .append_trace_event(
                "session-1",
                TraceEventType::ModelsAdded,
                serde_json::json!({
                    "tile_id": "tile-1",
                    "model_ids": ["openai/gpt-4o", "anthropic/claude"],
                }),
            )
            .expect("trace event should append");

        store
            .run_twin_inference()
            .expect("first inference should run");
        store
            .run_twin_inference()
            .expect("second inference should run");

        let inferred = store
            .list_user_records()
            .expect("records should list")
            .into_iter()
            .filter(|record| record.origin == RecordOrigin::Inferred)
            .collect::<Vec<_>>();
        assert_eq!(inferred.len(), 1);
        assert_eq!(
            inferred[0]
                .metadata
                .get("inference_key")
                .and_then(Value::as_str),
            Some("reasoning.model_comparison")
        );
    }

    #[test]
    fn rejected_inference_keys_are_not_auto_promoted_again() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        for index in 0..3 {
            store
                .append_trace_event(
                    "session-1",
                    TraceEventType::DebateStarted,
                    serde_json::json!({
                        "debate_id": format!("debate-{}", index),
                        "source_tile_ids": ["tile-1"],
                        "participating_models": ["a", "b"],
                    }),
                )
                .expect("trace event should append");
        }

        store.run_twin_inference().expect("inference should run");
        let record = store
            .list_user_records()
            .expect("records should list")
            .into_iter()
            .find(|record| {
                record.metadata.get("inference_key").and_then(Value::as_str)
                    == Some("reasoning.uses_debate")
            })
            .expect("debate record should exist");
        assert_eq!(record.promotion_state, PromotionState::AutoPromoted);

        store
            .set_user_record_promotion(
                &record.id,
                PromotionState::Rejected,
                Some("too broad".to_string()),
            )
            .expect("record should reject");
        let summary = store.run_twin_inference().expect("inference should rerun");
        let record = store
            .get_user_record(&record.id)
            .expect("record should load");

        assert_eq!(record.promotion_state, PromotionState::Rejected);
        assert!(summary.skipped_rejected_records >= 1);
    }

    #[test]
    fn export_separates_approved_candidate_and_rejected_records() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Fact,
                content: "Approved".to_string(),
                origin: RecordOrigin::User,
                evidence_refs: Vec::new(),
                confidence: 0.9,
                promotion_state: Some(PromotionState::Endorsed),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("approved record should be created");
        store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "Candidate".to_string(),
                origin: RecordOrigin::Inferred,
                evidence_refs: Vec::new(),
                confidence: 0.6,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("candidate record should be created");
        store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::ReasoningPattern,
                content: "Rejected".to_string(),
                origin: RecordOrigin::Inferred,
                evidence_refs: Vec::new(),
                confidence: 0.8,
                promotion_state: Some(PromotionState::Rejected),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("rejected record should be created");

        let bundle = store
            .export_bundle(TwinExportRequest::default())
            .expect("export should succeed");

        assert_eq!(bundle.approved_user_records.count, 1);
        assert_eq!(bundle.candidate_user_records.count, 1);
        assert_eq!(bundle.rejected_user_records.count, 1);
        assert!(std::path::Path::new(&bundle.approved_user_records.path).exists());
        assert!(std::path::Path::new(&bundle.candidate_user_records.path).exists());
        assert!(std::path::Path::new(&bundle.rejected_user_records.path).exists());
    }

    #[test]
    fn context_records_include_approved_and_only_relevant_candidates() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "Prefers evidence-backed implementation detail.".to_string(),
                origin: RecordOrigin::User,
                evidence_refs: Vec::new(),
                confidence: 0.9,
                promotion_state: Some(PromotionState::Endorsed),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("approved record should create");
        let relevant_candidate = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::ReasoningPattern,
                content: "May prefer red-team critique for shipping decisions.".to_string(),
                origin: RecordOrigin::Inferred,
                evidence_refs: Vec::new(),
                confidence: 0.6,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("candidate should create");
        store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::ReasoningPattern,
                content: "May prefer visual design exploration.".to_string(),
                origin: RecordOrigin::Inferred,
                evidence_refs: Vec::new(),
                confidence: 0.6,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("irrelevant candidate should create");

        let (approved, candidates) = store
            .select_context_records("Need red-team critique for this shipping decision")
            .expect("context records should select");

        assert_eq!(approved.len(), 1);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, relevant_candidate.id);
    }

    #[test]
    fn context_records_exclude_rejected_private_and_no_train() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        for state in [
            PromotionState::Rejected,
            PromotionState::Private,
            PromotionState::NoTrain,
        ] {
            store
                .create_user_record(UserRecordCreate {
                    kind: UserRecordKind::Preference,
                    content: format!("Excluded record for red-team decision: {:?}", state),
                    origin: RecordOrigin::User,
                    evidence_refs: Vec::new(),
                    confidence: 0.9,
                    promotion_state: Some(state),
                    valid_from: None,
                    valid_until: None,
                    links: Vec::new(),
                    metadata: HashMap::new(),
                })
                .expect("record should create");
        }

        let (approved, candidates) = store
            .select_context_records("red-team decision")
            .expect("context records should select");

        assert!(approved.is_empty());
        assert!(candidates.is_empty());
    }

    #[test]
    fn evidence_resolution_returns_trace_session_model_and_excerpts() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let event = store
            .append_trace_event(
                "session-1",
                TraceEventType::ResponseCompleted,
                serde_json::json!({
                    "tile_id": "tile-1",
                    "model_id": "openai/gpt-4o",
                    "model_name": "GPT-4o",
                    "prompt": "Implement this with tests.",
                    "content": "Updated frontend/src/api/client.js and ran npm run test:run.",
                }),
            )
            .expect("trace event should append");
        let record = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "Prefers implementation detail.".to_string(),
                origin: RecordOrigin::Inferred,
                evidence_refs: vec![EvidenceRef {
                    trace_id: "session-1".to_string(),
                    event_id: event.id,
                    session_id: "session-1".to_string(),
                    tile_id: Some("tile-1".to_string()),
                    model_id: Some("openai/gpt-4o".to_string()),
                    note: None,
                }],
                confidence: 0.8,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("record should be created");

        let evidence = store
            .resolve_user_record_evidence(&record.id)
            .expect("evidence should resolve");

        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].session_id, "session-1");
        assert_eq!(evidence[0].model_id.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(
            evidence[0].prompt_excerpt.as_deref(),
            Some("Implement this with tests.")
        );
        assert!(evidence[0]
            .response_excerpt
            .as_deref()
            .unwrap_or_default()
            .contains("frontend/src/api/client.js"));
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
