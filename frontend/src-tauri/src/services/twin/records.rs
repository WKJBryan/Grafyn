use crate::models::note::Note;
use crate::models::twin::{
    ActionGap, ActionGapCreate, ConstitutionInferenceSummary, ConstitutionItem,
    ConstitutionItemCreate, ConstitutionItemUpdate, ConstitutionReviewRequest, ConstitutionSetup,
    ConstitutionStatus, DecisionEpisode, DecisionEpisodeCreate, DecisionEpisodeWithReflections,
    DecisionEvidencePacket, DecisionEvidenceSource, DecisionMirrorConfig,
    DecisionMirrorConfigUpdate, DecisionMirrorWeights, DecisionOutcomeUpdate, EvidenceRef,
    ExportBundle, ExportFileSummary, MemoryDigestAction, MemoryDigestItem,
    MemoryDigestReviewRequest, MemoryDigestState, PromotionState, RecordOrigin, ReflectionCard,
    ReflectionCardCreate, ReflectionScores, ResolvedEvidenceRef, SessionTrace, TraceEvent,
    TraceEventType, TwinContextRecord, TwinExportRequest, TwinInferenceRunSummary, TwinPrediction,
    TwinPredictionDraft, TwinReviewRecord, UserRecord, UserRecordCreate, UserRecordKind,
    UserRecordUpdate,
};
#[cfg(test)]
use crate::models::twin::{DecisionMirrorPreset, PrimitiveDecisionAssessment};
use crate::services::atomic_io::write_atomic;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;
use super::TwinStore;
use super::shared::{event_text, evidence_note, excerpt, extract_event_model_id, extract_event_tile_id, lexical_terms, load_or_quarantine, payload_string, text_contains_any, value_contains_key};
use super::{AUTO_PROMOTE_CONFIDENCE, AUTO_PROMOTE_SUPPORT_COUNT};

const TWIN_INFERENCE_VERSION: &str = "local-signal-v1";
const MAX_TWIN_CANDIDATE_CONTEXT_RECORDS: usize = 8;
const MAX_TWIN_APPROVED_CONTEXT_RECORDS: usize = 12;
const MAX_TWIN_APPROVED_FALLBACK_RECORDS: usize = 3;

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
            source_type: Some("behavior".to_string()),
            source_id: Some(event.id.clone()),
            source_label: evidence_note(event),
            excerpt: Some(excerpt(&event_text(event))),
            speaker_role: None,
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


impl TwinStore {
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
        let mut approved_fallback = Vec::new();
        let mut candidates = Vec::new();

        for record in self.record_cache.values() {
            match record.promotion_state {
                PromotionState::Endorsed | PromotionState::AutoPromoted => {
                    let relevance = twin_record_relevance(record, &query_terms);
                    if relevance > 0 {
                        approved.push((relevance, twin_context_record(record, "approved")));
                    } else {
                        approved_fallback.push(twin_context_record(record, "approved"));
                    }
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
            b.0.cmp(&a.0).then_with(|| {
                b.1.confidence
                    .partial_cmp(&a.1.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.1.evidence_count.cmp(&a.1.evidence_count))
            })
        });

        let approved: Vec<TwinContextRecord> = if approved.is_empty() {
            // Never assemble an empty behavioral context just because the
            // query shares no keywords with any approved record.
            approved_fallback.sort_by(|a, b| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.evidence_count.cmp(&a.evidence_count))
            });
            approved_fallback
                .into_iter()
                .take(MAX_TWIN_APPROVED_FALLBACK_RECORDS)
                .collect()
        } else {
            approved
                .into_iter()
                .take(MAX_TWIN_APPROVED_CONTEXT_RECORDS)
                .map(|(_, record)| record)
                .collect()
        };

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

    pub(super) fn ensure_record_cache(&mut self) -> Result<()> {
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
                if let Some(record) = load_or_quarantine::<UserRecord>(path, "record") {
                    self.record_cache.insert(record.id.clone(), record);
                }
            }
        }

        self.records_cache_ready = true;
        Ok(())
    }

    fn record_file_path(&self, record_id: &str) -> PathBuf {
        self.records_path.join(format!("{}.json", record_id))
    }

    fn read_record_file(&self, path: &Path) -> Result<UserRecord> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read record file: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse record file: {}", path.display()))
    }

    pub(super) fn write_record_file(&self, record: &UserRecord) -> Result<()> {
        let path = self.record_file_path(&record.id);
        self.write_pretty_json(&path, record)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{Note, NoteStatus};
    use crate::models::twin::{
        default_record_confidence, PromotionState, RecordLink, RecordLinkType, UserRecordKind,
    };
    use tempfile::tempdir;

    #[test]
    fn user_record_writes_are_atomic_with_no_tmp_litter() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let record = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "Prefers atomic writes".to_string(),
                origin: RecordOrigin::User,
                evidence_refs: Vec::new(),
                confidence: default_record_confidence(),
                promotion_state: None,
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("record should be created");

        let record_file = store.records_path.join(format!("{}.json", record.id));
        let persisted = std::fs::read_to_string(&record_file).expect("record file should exist");
        assert!(persisted.contains("Prefers atomic writes"));
        crate::services::atomic_io::assert_no_tmp_siblings(&store.records_path);
    }

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

    fn endorsed_record(store: &mut TwinStore, content: &str, confidence: f32) -> UserRecord {
        store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: content.to_string(),
                origin: RecordOrigin::User,
                evidence_refs: Vec::new(),
                confidence,
                promotion_state: Some(PromotionState::Endorsed),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("record should be created")
    }

    #[test]
    fn approved_records_are_relevance_gated_with_confidence_fallback() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        endorsed_record(
            &mut store,
            "Prefers remote work over relocation for salary",
            0.6,
        );
        endorsed_record(&mut store, "Enjoys woodworking podcasts on weekends", 0.9);

        let (approved, _) = store
            .select_context_records("Should I accept the relocation offer for more salary?")
            .expect("selection should succeed");
        assert_eq!(approved.len(), 1);
        assert!(approved[0].content.contains("relocation"));

        // No keyword overlap at all: fall back to top-confidence records
        // instead of an empty behavioral context.
        let (fallback, _) = store
            .select_context_records("zzz qqq xyzzy")
            .expect("fallback selection should succeed");
        assert!(!fallback.is_empty());
        assert!(fallback.len() <= MAX_TWIN_APPROVED_FALLBACK_RECORDS);
        assert!(fallback[0].content.contains("woodworking"));
    }

}
