use super::shared::{
    event_text, evidence_note, excerpt, extract_event_model_id, extract_event_tile_id,
    lexical_terms, payload_string, text_contains_any, value_contains_key,
};
use super::TwinStore;
#[cfg(test)]
use crate::models::twin::TwinExportRequest;
use crate::models::twin::{
    EvidenceRef, PromotionState, RecordOrigin, ResolvedEvidenceRef, SessionTrace, TraceEvent,
    TraceEventType, TwinContextRecord, TwinInferenceRunSummary, TwinReviewRecord, UserRecord,
    UserRecordCreate, UserRecordKind, UserRecordUpdate,
};
use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

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

pub(super) fn append_promotion_history(
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
    match state.effective() {
        PromotionState::Candidate | PromotionState::AutoPromoted => 0,
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
        promotion_state: record.promotion_state.effective(),
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

fn apply_user_record_update(
    mut record: UserRecord,
    update: &UserRecordUpdate,
) -> Result<UserRecord> {
    let before = serde_json::to_value(&record)?;
    if let Some(content) = &update.content {
        record.content = content.clone();
    }
    if let Some(confidence) = update.confidence {
        record.confidence = confidence.clamp(0.0, 1.0);
    }
    if let Some(valid_from) = update.valid_from {
        record.valid_from = Some(valid_from);
    }
    if let Some(valid_until) = update.valid_until {
        record.valid_until = Some(valid_until);
    }
    if let Some(links) = &update.links {
        record.links = links.clone();
    }
    if let Some(metadata) = &update.metadata {
        record.metadata = metadata.clone();
    }
    if serde_json::to_value(&record)? != before {
        record.updated_at = Utc::now();
    }
    Ok(record)
}

fn apply_record_promotion(
    mut record: UserRecord,
    promotion_state: PromotionState,
    rationale: Option<&str>,
) -> (UserRecord, &'static str) {
    let previous_state = record.promotion_state.clone();
    let now = Utc::now();
    record.promotion_state = promotion_state.clone();
    record.updated_at = now;
    append_promotion_history(
        &mut record.metadata,
        &previous_state,
        &promotion_state,
        rationale,
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
    let action = match promotion_state {
        PromotionState::Candidate | PromotionState::AutoPromoted => "candidate",
        PromotionState::Endorsed => "endorse",
        PromotionState::Rejected => "reject",
        PromotionState::Private => "private",
        PromotionState::NoTrain => "no_train",
    };
    (record, action)
}

impl TwinStore {
    fn record_capture_governance(record: &UserRecord) -> crate::models::twin_event::Governance {
        if record.promotion_state.effective() == PromotionState::Private {
            crate::services::twin_events::local_capture_governance(
                crate::models::twin_event::Sensitivity::Restricted,
            )
        } else {
            crate::services::twin_events::standard_capture_governance()
        }
    }

    fn write_record_observation(
        &self,
        record: &UserRecord,
        automatic: bool,
        tag: Option<&str>,
    ) -> Result<()> {
        let draft = self.record_observation_draft(record, automatic, tag)?;
        self.write_governed_json(&self.record_file_path(&record.id), record, vec![draft])
    }

    pub(super) fn record_observation_draft(
        &self,
        record: &UserRecord,
        automatic: bool,
        tag: Option<&str>,
    ) -> Result<crate::services::twin_events::TwinEventDraft> {
        let digest = Self::governed_json_digest(record)?;
        let actor = automatic
            .then(|| crate::models::twin_event::ActorId::parse("grafyn"))
            .transpose()
            .map_err(anyhow::Error::msg)?;
        crate::services::twin_events::legacy_observation_draft(
            &format!("legacy-observation-{}", digest.as_str()),
            &record.id,
            digest,
            record.updated_at,
            crate::models::twin_event::SourceChannel::parse("legacy_twin")
                .map_err(anyhow::Error::msg)?,
            actor,
            tag,
            Self::record_capture_governance(record),
        )
        .map_err(anyhow::Error::msg)
    }

    fn write_record_feedback(
        &self,
        record: &UserRecord,
        action: &str,
        rationale: Option<&str>,
    ) -> Result<()> {
        let draft = self.record_feedback_draft(record, action, rationale)?;
        self.write_governed_json(&self.record_file_path(&record.id), record, vec![draft])
    }

    fn record_feedback_draft(
        &self,
        record: &UserRecord,
        action: &str,
        rationale: Option<&str>,
    ) -> Result<crate::services::twin_events::TwinEventDraft> {
        let digest = Self::governed_json_digest(record)?;
        let feedback_seed = format!(
            "{}\0{}\0{}",
            record.id,
            action,
            record.updated_at.to_rfc3339()
        );
        let feedback_id = format!(
            "feedback-{}",
            crate::services::twin_events::digest_bytes(feedback_seed.as_bytes()).as_str()
        );
        let mut draft = crate::services::twin_events::feedback_draft(
            crate::services::twin_events::FeedbackDraft {
                feedback_id: &feedback_id,
                target_id: &record.id,
                kind: action,
                content: None,
                rationale,
                rank: None,
                observed_at: record.updated_at,
                source_channel: crate::models::twin_event::SourceChannel::parse("legacy_twin")
                    .map_err(anyhow::Error::msg)?,
                governance: Self::record_capture_governance(record),
            },
        )
        .map_err(anyhow::Error::msg)?;
        draft.evidence.push(crate::models::twin_event::EvidenceRef {
            evidence_type: crate::models::twin_event::EvidenceType::TwinRecord,
            source_id: crate::models::twin_event::Identifier::parse(&record.id)
                .map_err(anyhow::Error::msg)?,
            digest: Some(digest),
        });
        Ok(draft)
    }

    /// Read-time compatibility overlay for artifacts materialized from the
    /// retired AutoPromoted state. Raw records and artifacts remain unchanged.
    pub(super) fn artifact_has_only_legacy_auto_support(&self, record_ids: &[String]) -> bool {
        let mut saw_legacy_auto = false;
        for id in record_ids {
            let record = Self::validate_file_id(id)
                .ok()
                .and_then(|_| self.read_record_file(&self.record_file_path(id)).ok());
            match record.map(|record| record.promotion_state) {
                Some(PromotionState::Endorsed) => return false,
                Some(PromotionState::AutoPromoted) => saw_legacy_auto = true,
                _ => {}
            }
        }
        saw_legacy_auto
    }

    pub fn list_user_records(&mut self) -> Result<Vec<UserRecord>> {
        self.ensure_record_cache()?;
        let mut records: Vec<UserRecord> = self.record_cache.values().cloned().collect();
        records.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(records)
    }

    pub fn get_user_record(&mut self, id: &str) -> Result<UserRecord> {
        Self::validate_file_id(id)?;
        self.ensure_record_cache()?;
        if let Some(record) = self.record_cache.get(id) {
            return Ok(record.clone());
        }
        anyhow::bail!("User record not found: {id}")
    }

    pub fn create_user_record(&mut self, create: UserRecordCreate) -> Result<UserRecord> {
        if create
            .promotion_state
            .as_ref()
            .is_some_and(|state| state.effective() != PromotionState::Candidate)
        {
            return Err(anyhow::anyhow!(
                "governance state must be changed through the explicit promotion action"
            ));
        }
        if self.event_recorder.is_noop() {
            self.ensure_record_cache()?;
            let record = Self::materialize_user_record(create);
            let automatic = record.origin == RecordOrigin::Inferred;
            self.write_record_observation(
                &record,
                automatic,
                automatic.then_some("legacy_inference"),
            )?;
            self.invalidate_mutation_caches();
            return Ok(record);
        }

        let recorder = self.event_recorder.clone();
        let mut committed = None;
        let mut planner = || {
            let record = loop {
                let candidate = Self::materialize_user_record(create.clone());
                let path = self.record_file_path(&candidate.id);
                if self
                    .read_twin_json_bounded::<UserRecord>(&path)
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?
                    .is_none()
                {
                    break candidate;
                }
            };
            let automatic = record.origin == RecordOrigin::Inferred;
            let draft = self
                .record_observation_draft(
                    &record,
                    automatic,
                    automatic.then_some("legacy_inference"),
                )
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            let content = serde_json::to_string_pretty(&record).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
            let targets = self
                .governed_json_targets(vec![(self.record_file_path(&record.id), content)])
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            committed = Some(record);
            Ok(Some(crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::SyncEligible,
                crate::models::twin_event::SourceChannel::parse("legacy_twin")
                    .map_err(crate::services::twin_events::MutationError::Invalid)?,
                targets,
                vec![draft],
            )))
        };
        if let Err(error) = recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut planner,
        ) {
            self.invalidate_mutation_caches();
            return Err(anyhow::Error::new(error));
        }
        let record = committed.ok_or_else(|| anyhow::anyhow!("record create was not planned"))?;
        self.invalidate_mutation_caches();
        Ok(record)
    }

    pub(super) fn materialize_user_record(create: UserRecordCreate) -> UserRecord {
        let now = Utc::now();
        let promotion_state = create
            .promotion_state
            .unwrap_or_else(|| PromotionState::default_for_origin(&create.origin))
            .effective();

        UserRecord {
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
        }
    }

    pub fn update_user_record(&mut self, id: &str, update: UserRecordUpdate) -> Result<UserRecord> {
        if update.promotion_state.is_some() {
            return Err(anyhow::anyhow!(
                "governance state must be changed through the explicit promotion action"
            ));
        }
        Self::validate_file_id(id)?;
        if self.event_recorder.is_noop() {
            self.ensure_record_cache()?;
            let record = apply_user_record_update(self.get_user_record(id)?, &update)?;
            if record.updated_at != self.get_user_record(id)?.updated_at {
                self.write_record_observation(&record, false, None)?;
                self.invalidate_mutation_caches();
            }
            return Ok(record);
        }

        let recorder = self.event_recorder.clone();
        let path = self.record_file_path(id);
        let mut committed = None;
        let mut planner = || {
            let before = self.read_record_file(&path).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
            let record = apply_user_record_update(before.clone(), &update).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
            committed = Some(record.clone());
            if record.updated_at == before.updated_at {
                return Ok(None);
            }
            let draft = self
                .record_observation_draft(&record, false, None)
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            let values = vec![(
                path.clone(),
                serde_json::to_string_pretty(&record).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?,
            )];
            let targets = self.governed_json_targets(values).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
            Ok(Some(crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::SyncEligible,
                crate::models::twin_event::SourceChannel::parse("legacy_twin")
                    .map_err(crate::services::twin_events::MutationError::Invalid)?,
                targets,
                vec![draft],
            )))
        };
        if let Err(error) = recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut planner,
        ) {
            self.invalidate_mutation_caches();
            return Err(anyhow::Error::new(error));
        }
        let record = committed.ok_or_else(|| anyhow::anyhow!("record update was not planned"))?;
        self.invalidate_mutation_caches();
        Ok(record)
    }

    pub fn run_twin_inference(&mut self) -> Result<TwinInferenceRunSummary> {
        if !self.event_recorder.is_noop() {
            let recorder = self.event_recorder.clone();
            let mut committed = None;
            let mut planner = || {
                let traces = self.list_session_traces_durable().map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                let scanned_events = traces.iter().map(|trace| trace.events.len()).sum::<usize>();
                let inferred = infer_behavioral_records(&traces);
                let records = self.list_user_records_durable().map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                let mut records_by_id = records
                    .into_iter()
                    .map(|record| (record.id.clone(), record))
                    .collect::<HashMap<_, _>>();
                let mut existing_by_key = HashMap::new();
                let mut rejected_keys = HashSet::new();
                for record in records_by_id.values() {
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

                let mut created_records = 0usize;
                let mut updated_records = 0usize;
                let mut skipped_rejected_records = 0usize;
                let mut changed = Vec::new();
                for draft in inferred {
                    let mut metadata = build_inference_metadata(&draft, false);
                    let record = if let Some(existing_id) =
                        existing_by_key.get(&draft.inference_key).cloned()
                    {
                        let mut record = records_by_id.get(&existing_id).cloned().ok_or_else(|| {
                            crate::services::twin_events::MutationError::Invalid(
                                "inference record index is inconsistent".into(),
                            )
                        })?;
                        let previous_state = record.promotion_state.clone();
                        record.kind = draft.kind.clone();
                        record.content = draft.content.clone();
                        record.evidence_refs = draft.evidence_refs.clone();
                        record.confidence = draft.confidence;
                        record.origin = RecordOrigin::Inferred;
                        if rejected_keys.contains(&draft.inference_key) {
                            metadata.insert("auto_promoted".to_string(), Value::Bool(false));
                            record.promotion_state = PromotionState::Rejected;
                            skipped_rejected_records += 1;
                        } else if record.promotion_state.effective() == PromotionState::Candidate {
                            record.promotion_state = PromotionState::Candidate;
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
                        updated_records += 1;
                        record
                    } else {
                        let record = loop {
                            let candidate = Self::materialize_user_record(UserRecordCreate {
                                kind: draft.kind.clone(),
                                content: draft.content.clone(),
                                evidence_refs: draft.evidence_refs.clone(),
                                confidence: draft.confidence,
                                origin: RecordOrigin::Inferred,
                                promotion_state: Some(PromotionState::Candidate),
                                valid_from: None,
                                valid_until: None,
                                links: Vec::new(),
                                metadata: metadata.clone(),
                            });
                            if !records_by_id.contains_key(&candidate.id) {
                                break candidate;
                            }
                        };
                        existing_by_key.insert(draft.inference_key.clone(), record.id.clone());
                        created_records += 1;
                        record
                    };
                    records_by_id.insert(record.id.clone(), record.clone());
                    changed.push(record);
                }
                changed.sort_by(|left, right| left.id.cmp(&right.id));
                if changed.len() > crate::services::twin_events::MAX_INTENT_TARGETS {
                    return Err(crate::services::twin_events::MutationError::Invalid(
                        "Twin inference can update at most 64 records per run".into(),
                    ));
                }
                let candidate_records = records_by_id
                    .values()
                    .filter(|record| {
                        record.origin == RecordOrigin::Inferred
                            && matches!(
                                record.promotion_state.effective(),
                                PromotionState::AutoPromoted | PromotionState::Candidate
                            )
                    })
                    .count();
                let summary = TwinInferenceRunSummary {
                    inference_version: TWIN_INFERENCE_VERSION.to_string(),
                    scanned_traces: traces.len(),
                    scanned_events,
                    created_records,
                    updated_records,
                    auto_promoted_records: 0,
                    candidate_records,
                    skipped_rejected_records,
                    generated_at: Utc::now(),
                };
                let all_records = records_by_id.into_values().collect::<Vec<_>>();
                if changed.is_empty() {
                    committed = Some((summary, all_records));
                    return Ok(None);
                }
                let mut values = Vec::with_capacity(changed.len());
                let mut event_drafts = Vec::with_capacity(changed.len());
                for record in &changed {
                    values.push((
                        self.record_file_path(&record.id),
                        serde_json::to_string_pretty(record).map_err(|error| {
                            crate::services::twin_events::MutationError::Invalid(error.to_string())
                        })?,
                    ));
                    event_drafts.push(
                        self.record_observation_draft(
                            record,
                            true,
                            Some("legacy_inference"),
                        )
                        .map_err(|error| {
                            crate::services::twin_events::MutationError::Invalid(error.to_string())
                        })?,
                    );
                }
                let targets = self.governed_json_targets(values).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                committed = Some((summary, all_records));
                Ok(Some(crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::SyncEligible,
                    crate::models::twin_event::SourceChannel::parse("legacy_twin")
                        .map_err(crate::services::twin_events::MutationError::Invalid)?,
                    targets,
                    event_drafts,
                )))
            };
            if let Err(error) = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            ) {
                self.invalidate_mutation_caches();
                return Err(anyhow::Error::new(error));
            }
            let (summary, _records) = committed
                .ok_or_else(|| anyhow::anyhow!("Twin inference was not planned"))?;
            self.invalidate_mutation_caches();
            return Ok(summary);
        }

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
        let auto_promoted_records = 0_usize;
        let mut candidate_records = 0_usize;
        let mut skipped_rejected_records = 0_usize;

        for draft in drafts {
            let desired_state = PromotionState::Candidate;
            let mut metadata = build_inference_metadata(&draft, false);

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
                } else if record.promotion_state.effective() == PromotionState::Candidate {
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
                self.write_record_observation(&record, true, Some("legacy_inference"))?;
                self.invalidate_mutation_caches();
                updated_records += 1;
            } else {
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

        self.ensure_record_cache()?;
        for record in self.record_cache.values() {
            if record.origin != RecordOrigin::Inferred {
                continue;
            }
            match record.promotion_state.effective() {
                PromotionState::AutoPromoted | PromotionState::Candidate => candidate_records += 1,
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
            match &record.promotion_state {
                PromotionState::Endorsed => {
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
                PromotionState::AutoPromoted
                | PromotionState::Rejected
                | PromotionState::Private
                | PromotionState::NoTrain => {}
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
        Self::validate_file_id(id)?;
        let promotion_state = promotion_state.effective();
        if self.event_recorder.is_noop() {
            self.ensure_record_cache()?;
            let (record, action) = apply_record_promotion(
                self.get_user_record(id)?,
                promotion_state,
                rationale.as_deref(),
            );
            self.write_record_feedback(&record, action, rationale.as_deref())?;
            self.invalidate_mutation_caches();
            return Ok(record);
        }

        let recorder = self.event_recorder.clone();
        let path = self.record_file_path(id);
        let mut committed = None;
        let mut planner = || {
            let before = self.read_record_file(&path).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
            let (record, action) =
                apply_record_promotion(before, promotion_state.clone(), rationale.as_deref());
            let draft = self
                .record_feedback_draft(&record, action, rationale.as_deref())
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            let content = serde_json::to_string_pretty(&record).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
            let targets = self
                .governed_json_targets(vec![(path.clone(), content)])
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            committed = Some(record);
            Ok(Some(crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::SyncEligible,
                crate::models::twin_event::SourceChannel::parse("legacy_twin")
                    .map_err(crate::services::twin_events::MutationError::Invalid)?,
                targets,
                vec![draft],
            )))
        };
        if let Err(error) = recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut planner,
        ) {
            self.invalidate_mutation_caches();
            return Err(anyhow::Error::new(error));
        }
        let record = committed.ok_or_else(|| anyhow::anyhow!("promotion was not planned"))?;
        self.invalidate_mutation_caches();
        Ok(record)
    }

    pub(super) fn ensure_record_cache(&mut self) -> Result<()> {
        if self.records_cache_ready {
            return Ok(());
        }

        let records = self
            .list_user_records_durable()?
            .into_iter()
            .map(|record| (record.id.clone(), record))
            .collect();
        self.record_cache = records;
        self.records_cache_ready = true;
        Ok(())
    }

    pub(super) fn record_file_path(&self, record_id: &str) -> PathBuf {
        self.records_path.join(format!("{}.json", record_id))
    }

    fn read_record_file(&self, path: &Path) -> Result<UserRecord> {
        self.read_twin_json_bounded(path)?.ok_or_else(|| {
            anyhow::anyhow!("Failed to read record file: {}", path.display())
        })
    }

    pub(super) fn list_user_records_durable(&self) -> Result<Vec<UserRecord>> {
        self.list_twin_json_bounded("records")
    }

    #[cfg(test)]
    pub(super) fn write_record_file(&self, record: &UserRecord) -> Result<()> {
        let path = self.record_file_path(&record.id);
        self.write_pretty_json(&path, record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin::{
        default_record_confidence, PromotionState, RecordLink, RecordLinkType, UserRecordKind,
    };
    use tempfile::tempdir;

    fn coordinated_twin_store(
        root: &std::path::Path,
    ) -> (
        TwinStore,
        std::sync::Arc<crate::services::twin_events::TwinEventStore>,
        std::sync::Arc<crate::services::twin_events::MutationCoordinator>,
    ) {
        let data = root.join("data");
        let vault = root.join("vault");
        let twin_root = data.join("twin").join("scope-one");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                events.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        (
            TwinStore::with_event_recorder(twin_root, data.join("twin"), coordinator.clone()),
            events,
            coordinator,
        )
    }

    fn record_create(origin: RecordOrigin) -> UserRecordCreate {
        UserRecordCreate {
            kind: UserRecordKind::Preference,
            content: "Prefers reviewable evidence".to_string(),
            evidence_refs: Vec::new(),
            confidence: 0.8,
            origin,
            promotion_state: Some(PromotionState::Candidate),
            valid_from: None,
            valid_until: None,
            links: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn generic_record_mutations_cannot_change_governance_state() {
        for state in [
            PromotionState::Endorsed,
            PromotionState::Private,
            PromotionState::Rejected,
            PromotionState::NoTrain,
        ] {
            let root = tempdir().unwrap();
            let mut store = TwinStore::new(root.path().join("twin"));
            let mut create = record_create(RecordOrigin::User);
            create.promotion_state = Some(state.clone());
            assert!(
                store.create_user_record(create).is_err(),
                "create {state:?}"
            );

            let record = store
                .create_user_record(record_create(RecordOrigin::User))
                .unwrap();
            assert!(
                store
                    .update_user_record(
                        &record.id,
                        UserRecordUpdate {
                            promotion_state: Some(state.clone()),
                            ..Default::default()
                        },
                    )
                    .is_err(),
                "update {state:?}"
            );
            assert_eq!(
                store.get_user_record(&record.id).unwrap().promotion_state,
                PromotionState::Candidate
            );
        }
    }

    #[test]
    fn coordinated_legacy_record_mutations_emit_observations_and_explicit_feedback_only() {
        let root = tempdir().unwrap();
        let (mut store, events, _) = coordinated_twin_store(root.path());
        let record = store
            .create_user_record(record_create(RecordOrigin::User))
            .unwrap();
        store
            .update_user_record(
                &record.id,
                UserRecordUpdate {
                    content: Some("Prefers primary evidence".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .set_user_record_promotion(
                &record.id,
                PromotionState::Endorsed,
                Some("Reviewed against source".to_string()),
            )
            .unwrap();
        let captured = events.ordered_events().unwrap();
        assert_eq!(captured.len(), 3);
        assert!(matches!(
            captured[0].payload,
            crate::models::twin_event::TwinEventPayload::ObservationRecorded(_)
        ));
        assert!(matches!(
            captured[1].payload,
            crate::models::twin_event::TwinEventPayload::ObservationRecorded(_)
        ));
        let crate::models::twin_event::TwinEventPayload::FeedbackRecorded(feedback) =
            &captured[2].payload
        else {
            panic!("an explicit promotion is feedback, never a memory review");
        };
        assert_eq!(feedback.target_id.as_str(), record.id);
        assert_eq!(feedback.kind.as_str(), "endorse");
        assert_eq!(feedback.content, None);
        assert_eq!(
            feedback.rationale.as_ref().unwrap().as_str(),
            "Reviewed against source"
        );
        assert!(captured.iter().all(|event| !matches!(
            event.payload,
            crate::models::twin_event::TwinEventPayload::MemoryProposed(_)
                | crate::models::twin_event::TwinEventPayload::MemoryReviewed(_)
        )));
    }

    #[test]
    fn inferred_legacy_record_is_tagged_as_automatic_grafyn_observation() {
        let root = tempdir().unwrap();
        let (mut store, events, _) = coordinated_twin_store(root.path());
        store
            .create_user_record(record_create(RecordOrigin::Inferred))
            .unwrap();
        let captured = events.ordered_events().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].actor_id.as_str(), "grafyn");
        assert_eq!(captured[0].context.tags, vec!["legacy_inference"]);
    }

    #[test]
    fn failed_legacy_record_persistence_does_not_poison_the_cache() {
        let root = tempdir().unwrap();
        let twin_root = root.path().join("twin").join("scope-one");
        let mut store = TwinStore::with_event_recorder(
            twin_root,
            root.path().join("twin"),
            std::sync::Arc::new(crate::services::twin_events::UnavailableEventRecorder::new(
                "injected failure",
            )),
        );
        assert!(store
            .create_user_record(record_create(RecordOrigin::User))
            .is_err());
        assert!(store.list_user_records().unwrap().is_empty());
    }

    #[test]
    fn interrupted_record_update_is_recovered_before_the_next_fresh_plan() {
        let root = tempdir().unwrap();
        let (mut store, events, coordinator) = coordinated_twin_store(root.path());
        let record = store
            .create_user_record(record_create(RecordOrigin::User))
            .unwrap();
        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));
        assert!(store
            .update_user_record(
                &record.id,
                UserRecordUpdate {
                    content: Some("Recovered field".into()),
                    ..Default::default()
                },
            )
            .is_err());

        let updated = store
            .update_user_record(
                &record.id,
                UserRecordUpdate {
                    confidence: Some(0.55),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.content, "Recovered field");
        assert_eq!(updated.confidence, 0.55);
        assert_eq!(events.ordered_events().unwrap().len(), 3);
    }

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
            .set_user_record_promotion(&synthetic.id, PromotionState::Endorsed, None)
            .expect("synthetic record should update");

        let bundle = store
            .export_bundle(TwinExportRequest::default())
            .expect("endorsed export should succeed");
        assert_eq!(bundle.included_records, 1);
    }

    #[test]
    fn user_records_start_pending_and_require_endorsement_before_export() {
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

        assert_eq!(record.promotion_state, PromotionState::Candidate);

        let bundle = store
            .export_bundle(TwinExportRequest::default())
            .expect("export should succeed");
        assert_eq!(bundle.included_records, 0);
        assert_eq!(bundle.approved_user_records.count, 0);

        store
            .set_user_record_promotion(&record.id, PromotionState::Endorsed, None)
            .expect("explicit endorsement should succeed");
        let bundle = store
            .export_bundle(TwinExportRequest::default())
            .expect("endorsed export should succeed");
        assert_eq!(bundle.included_records, 1);
    }

    #[test]
    fn inference_remains_candidate_even_above_the_legacy_threshold() {
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
        assert_eq!(record.promotion_state, PromotionState::Candidate);
        assert_eq!(
            record
                .metadata
                .get("auto_promoted")
                .and_then(Value::as_bool),
            Some(false)
        );
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
    fn rejected_inference_keys_remain_rejected() {
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
        assert_eq!(record.promotion_state, PromotionState::Candidate);

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

        let approved = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "Prefers evidence-backed implementation detail.".to_string(),
                origin: RecordOrigin::User,
                evidence_refs: Vec::new(),
                confidence: 0.9,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("approved record should create");
        store
            .set_user_record_promotion(&approved.id, PromotionState::Endorsed, None)
            .expect("approved record should be endorsed explicitly");
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
            let record = store
                .create_user_record(UserRecordCreate {
                    kind: UserRecordKind::Preference,
                    content: format!("Excluded record for red-team decision: {:?}", state),
                    origin: RecordOrigin::User,
                    evidence_refs: Vec::new(),
                    confidence: 0.9,
                    promotion_state: Some(PromotionState::Candidate),
                    valid_from: None,
                    valid_until: None,
                    links: Vec::new(),
                    metadata: HashMap::new(),
                })
                .expect("record should create");
            store
                .set_user_record_promotion(&record.id, state, None)
                .expect("record governance should change explicitly");
        }

        let (approved, candidates) = store
            .select_context_records("red-team decision")
            .expect("context records should select");

        assert!(approved.is_empty());
        assert!(candidates.is_empty());
    }

    #[test]
    fn legacy_auto_promoted_is_auditable_but_never_selected_as_twin_context() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let now = Utc::now();
        let legacy = UserRecord {
            id: "legacy-auto".to_string(),
            kind: UserRecordKind::Preference,
            content: "red-team every shipping decision".to_string(),
            evidence_refs: Vec::new(),
            confidence: 1.0,
            origin: RecordOrigin::Inferred,
            promotion_state: PromotionState::AutoPromoted,
            created_at: now,
            updated_at: now,
            valid_from: None,
            valid_until: None,
            links: Vec::new(),
            metadata: HashMap::new(),
        };
        store.record_cache.insert(legacy.id.clone(), legacy);
        store.records_cache_ready = true;

        let (approved, candidates) = store
            .select_context_records("red-team shipping decision")
            .expect("selection should succeed");
        assert!(approved.is_empty());
        assert!(candidates.is_empty());
        assert_eq!(
            store
                .get_user_record("legacy-auto")
                .unwrap()
                .promotion_state,
            PromotionState::AutoPromoted
        );
    }

    #[test]
    fn new_mutations_cannot_recreate_auto_promoted_authority() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let record = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "explicit legacy request".to_string(),
                evidence_refs: Vec::new(),
                confidence: 1.0,
                origin: RecordOrigin::User,
                promotion_state: Some(PromotionState::AutoPromoted),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .unwrap();
        assert_eq!(record.promotion_state, PromotionState::Candidate);
        let updated = store
            .set_user_record_promotion(&record.id, PromotionState::AutoPromoted, None)
            .unwrap();
        assert_eq!(updated.promotion_state, PromotionState::Candidate);
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
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("old record should be created");
        let old_record = store
            .set_user_record_promotion(&old_record.id, PromotionState::Endorsed, None)
            .expect("old record should be endorsed explicitly");

        let new_record = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "Now prefers blunt, longer answers when accuracy matters.".to_string(),
                origin: RecordOrigin::User,
                evidence_refs: Vec::new(),
                confidence: 0.9,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: vec![RecordLink {
                    relation: RecordLinkType::Supersedes,
                    target_record_id: old_record.id.clone(),
                }],
                metadata: HashMap::new(),
            })
            .expect("new record should be created");
        let new_record = store
            .set_user_record_promotion(&new_record.id, PromotionState::Endorsed, None)
            .expect("new record should be endorsed explicitly");

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
        let record = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: content.to_string(),
                origin: RecordOrigin::User,
                evidence_refs: Vec::new(),
                confidence,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("record should be created");
        store
            .set_user_record_promotion(&record.id, PromotionState::Endorsed, None)
            .expect("record should be endorsed explicitly")
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
