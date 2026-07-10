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

mod shared;
mod traces;

use shared::*;

const AUTO_PROMOTE_CONFIDENCE: f32 = 0.75;
const AUTO_PROMOTE_SUPPORT_COUNT: usize = 3;
const TWIN_INFERENCE_VERSION: &str = "local-signal-v1";
const MAX_TWIN_CANDIDATE_CONTEXT_RECORDS: usize = 8;
const MAX_TWIN_APPROVED_CONTEXT_RECORDS: usize = 12;
const MAX_TWIN_APPROVED_FALLBACK_RECORDS: usize = 3;
const MAX_MEMORY_DIGEST_ITEMS: usize = 5;
const DEFAULT_EVAL_PERCENTAGE: u8 = 10;
const DEFAULT_HOLDOUT_PERCENTAGE: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportSplit {
    Train,
    Eval,
    Holdout,
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

fn clean_setup_entries(entries: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .filter(|entry| seen.insert(entry.to_lowercase()))
        .collect()
}

fn clean_optional_setup_entry(entry: Option<String>) -> Option<String> {
    entry
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn constitution_key(dimension: &str, claim: &str) -> String {
    format!(
        "{}::{}",
        dimension.trim().to_lowercase(),
        normalize_key_text(claim)
    )
}

fn action_gap_key(stated_value: &str, revealed_behavior: &str) -> String {
    format!(
        "{}::{}",
        normalize_key_text(stated_value),
        normalize_key_text(revealed_behavior)
    )
}

fn normalize_key_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn constitution_allows_record(record: &UserRecord) -> bool {
    !matches!(
        record.promotion_state,
        PromotionState::Rejected | PromotionState::Private | PromotionState::NoTrain
    ) && record.kind != UserRecordKind::Fact
}

fn behavior_event_for_constitution(event_type: &TraceEventType) -> bool {
    matches!(
        event_type,
        TraceEventType::PromptSubmitted
            | TraceEventType::FeedbackRecorded
            | TraceEventType::RankingRecorded
            | TraceEventType::InsightCaptured
            | TraceEventType::DebateStarted
            | TraceEventType::DebateContinued
            | TraceEventType::NoteExported
            | TraceEventType::NoteCanonicalPromoted
            | TraceEventType::DecisionEpisodeCreated
            | TraceEventType::OutcomeFollowUpRecorded
    )
}

fn constitution_status_from_record(record: &UserRecord) -> ConstitutionStatus {
    let support_count = record.evidence_refs.len();
    match record.promotion_state {
        PromotionState::AutoPromoted | PromotionState::Endorsed
            if support_count >= AUTO_PROMOTE_SUPPORT_COUNT =>
        {
            ConstitutionStatus::Active
        }
        _ => ConstitutionStatus::Candidate,
    }
}

fn constitution_context_allowed(status: &ConstitutionStatus) -> bool {
    matches!(
        status,
        ConstitutionStatus::Active | ConstitutionStatus::Candidate | ConstitutionStatus::Softened
    )
}

fn constitution_status_for_action(action: &MemoryDigestAction) -> ConstitutionStatus {
    match action {
        MemoryDigestAction::Keep => ConstitutionStatus::Active,
        MemoryDigestAction::Soften => ConstitutionStatus::Softened,
        MemoryDigestAction::NotMe => ConstitutionStatus::NotMe,
        MemoryDigestAction::Private => ConstitutionStatus::Private,
        MemoryDigestAction::NoTrain => ConstitutionStatus::NoTrain,
        MemoryDigestAction::Reject => ConstitutionStatus::Rejected,
    }
}

fn constitution_status_sort_key(status: &ConstitutionStatus) -> u8 {
    match status {
        ConstitutionStatus::Active => 0,
        ConstitutionStatus::Candidate => 1,
        ConstitutionStatus::Softened => 2,
        ConstitutionStatus::NotMe => 3,
        ConstitutionStatus::Private => 4,
        ConstitutionStatus::NoTrain => 5,
        ConstitutionStatus::Rejected => 6,
    }
}

fn infer_constitution_dimension(content: &str, kind: &UserRecordKind) -> String {
    let lower = content.to_lowercase();
    if text_contains_any(&lower, &["taste", "aesthetic", "style", "design", "feel"]) {
        "taste".to_string()
    } else if text_contains_any(
        &lower,
        &["constraint", "deadline", "budget", "time", "cost"],
    ) {
        "constraints".to_string()
    } else if text_contains_any(&lower, &["somatic", "gut", "body", "energy", "fatigue"]) {
        "somatic".to_string()
    } else if text_contains_any(&lower, &["value", "principle", "mission", "care about"]) {
        "values".to_string()
    } else {
        match kind {
            UserRecordKind::Preference => "preferences".to_string(),
            UserRecordKind::ReasoningPattern => "reasoning".to_string(),
            UserRecordKind::Fact => "facts".to_string(),
        }
    }
}

fn note_is_interview(note: &Note) -> bool {
    note.properties
        .get("source_type")
        .or_else(|| note.properties.get("source"))
        .and_then(Value::as_str)
        .is_some_and(|value| value.eq_ignore_ascii_case("interview"))
        || note
            .tags
            .iter()
            .any(|tag| tag.eq_ignore_ascii_case("interview"))
}

#[derive(Debug, Clone)]
struct InterviewTurn {
    role: String,
    content: String,
}

enum InterviewExtraction {
    Unlabeled,
    Labeled {
        interviewer_turns: Vec<InterviewTurn>,
        interviewee_turns: Vec<InterviewTurn>,
    },
}

fn extract_interview_note_evidence(note: &Note) -> InterviewExtraction {
    let turns = extract_markdown_interview_turns(&note.content)
        .or_else(|| extract_colon_labeled_interview_turns(&note.content));
    let Some(turns) = turns else {
        return InterviewExtraction::Unlabeled;
    };
    let interviewer_turns = turns
        .iter()
        .filter(|turn| turn.role == "user")
        .cloned()
        .collect::<Vec<_>>();
    let interviewee_turns = turns
        .into_iter()
        .filter(|turn| turn.role == "interviewee")
        .collect::<Vec<_>>();
    if interviewer_turns.is_empty() || interviewee_turns.is_empty() {
        return InterviewExtraction::Unlabeled;
    }
    InterviewExtraction::Labeled {
        interviewer_turns,
        interviewee_turns,
    }
}

fn extract_markdown_interview_turns(content: &str) -> Option<Vec<InterviewTurn>> {
    let mut turns = Vec::new();
    let mut current_role: Option<String> = None;
    let mut current_content = String::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(role) = markdown_message_role(trimmed) {
            flush_interview_turn(&mut turns, &mut current_role, &mut current_content);
            current_role = Some(role);
            continue;
        }
        if current_role.is_some() {
            current_content.push_str(line);
            current_content.push('\n');
        }
    }
    flush_interview_turn(&mut turns, &mut current_role, &mut current_content);
    if turns.len() >= 2 {
        Some(turns)
    } else {
        None
    }
}

fn markdown_message_role(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    if !(lower.starts_with("### message") || lower.starts_with("## message")) {
        return None;
    }
    if lower.contains("interviewee") || lower.contains("participant") {
        Some("interviewee".to_string())
    } else if lower.contains("user")
        || lower.contains("interviewer")
        || lower.contains("researcher")
    {
        Some("user".to_string())
    } else {
        None
    }
}

fn extract_colon_labeled_interview_turns(content: &str) -> Option<Vec<InterviewTurn>> {
    let mut turns = Vec::new();
    for line in content.lines() {
        let Some((speaker, body)) = line.split_once(':') else {
            continue;
        };
        let speaker = speaker.trim().to_lowercase();
        let role = match speaker.as_str() {
            "interviewer" | "researcher" | "moderator" | "facilitator" | "me" | "user" => "user",
            "interviewee" | "participant" | "respondent" | "customer" | "student" | "teacher" => {
                "interviewee"
            }
            _ => continue,
        };
        let content = body.trim();
        if !content.is_empty() {
            turns.push(InterviewTurn {
                role: role.to_string(),
                content: content.to_string(),
            });
        }
    }
    if turns.len() >= 2 {
        Some(turns)
    } else {
        None
    }
}

fn flush_interview_turn(
    turns: &mut Vec<InterviewTurn>,
    current_role: &mut Option<String>,
    current_content: &mut String,
) {
    let Some(role) = current_role.take() else {
        return;
    };
    let content = current_content.trim().to_string();
    current_content.clear();
    if content.is_empty() {
        return;
    }
    turns.push(InterviewTurn { role, content });
}

fn constitution_claim_from_interviewer_turn(content: &str) -> Option<(String, String)> {
    let lower = content.to_lowercase();
    if text_contains_any(
        &lower,
        &[
            "concrete example",
            "compare",
            "why",
            "how do you",
            "what makes",
            "walk me through",
        ],
    ) {
        return Some((
            "reasoning".to_string(),
            "Uses interview questions to probe for concrete examples and comparisons before treating claims as useful.".to_string(),
        ));
    }
    None
}

fn looks_like_research_finding(content: &str) -> bool {
    content.split_whitespace().count() >= 8
        && text_contains_any(
            content,
            &[
                "need", "want", "trust", "frustrat", "prefer", "hard", "easy", "useful", "demo",
                "workflow",
            ],
        )
}

fn constitution_claim_from_note(note: &Note) -> Option<(String, String)> {
    if note.is_topic_hub() {
        return None;
    }
    let content = note.content.trim();
    if content.len() < 24 {
        return None;
    }
    let lower = content.to_lowercase();
    if !text_contains_any(
        &lower,
        &[
            "i prefer",
            "i need",
            "i value",
            "my reasoning",
            "works for me",
            "not me",
            "i reject",
            "i decide",
        ],
    ) {
        return None;
    }
    if text_contains_any(&lower, &["prefer"]) {
        Some((
            "preferences".to_string(),
            format!("Note-backed preference: {}", excerpt(content)),
        ))
    } else {
        Some((
            "reasoning".to_string(),
            format!("Note-backed reasoning pattern: {}", excerpt(content)),
        ))
    }
}

fn note_evidence_ref(
    note: &Note,
    source_type: &str,
    source_label: &str,
    speaker_role: Option<&str>,
    content: &str,
) -> EvidenceRef {
    EvidenceRef {
        trace_id: format!("note-{}", note.id),
        event_id: note.id.clone(),
        session_id: "vault-note".to_string(),
        tile_id: None,
        model_id: None,
        note: Some(note.title.clone()),
        source_type: Some(source_type.to_string()),
        source_id: Some(note.id.clone()),
        source_label: Some(source_label.to_string()),
        excerpt: Some(excerpt(content.trim())),
        speaker_role: speaker_role.map(ToOwned::to_owned),
    }
}

fn distill_constitution_setup_from_notes(notes: &[Note]) -> ConstitutionSetup {
    let mut setup = ConstitutionSetup::default();
    let mut saw_interview_question = false;
    let mut saw_values_question = false;
    let mut saw_tradeoff_question = false;
    let mut saw_risk_question = false;
    let mut saw_concrete_question = false;
    let mut saw_followup_question = false;
    let mut saw_user_note_preference = false;
    let mut interviewer_question_count = 0_usize;

    for note in notes {
        if note.is_topic_hub() {
            continue;
        }

        if note_is_interview(note) {
            if let InterviewExtraction::Labeled {
                interviewer_turns, ..
            } = extract_interview_note_evidence(note)
            {
                for turn in interviewer_turns {
                    let lower = turn.content.to_lowercase();
                    if turn.content.split_whitespace().count() < 5 {
                        continue;
                    }
                    saw_interview_question = true;
                    interviewer_question_count += 1;
                    if text_contains_any(&lower, &["value", "cultural", "drives your decision"]) {
                        saw_values_question = true;
                    }
                    if text_contains_any(&lower, &["trade off", "trade-off", "balance"]) {
                        saw_tradeoff_question = true;
                    }
                    if text_contains_any(&lower, &["risk", "comfortable taking", "actively avoid"])
                    {
                        saw_risk_question = true;
                    }
                    if text_contains_any(
                        &lower,
                        &[
                            "example",
                            "walk us through",
                            "walk through",
                            "concrete",
                            "top two",
                        ],
                    ) {
                        saw_concrete_question = true;
                    }
                    if text_contains_any(
                        &lower,
                        &["following up", "if i understand", "drill", "how would"],
                    ) {
                        saw_followup_question = true;
                    }
                }
            }
            continue;
        }

        if let Some((dimension, claim)) = constitution_claim_from_note(note) {
            saw_user_note_preference = true;
            match dimension.as_str() {
                "preferences" => push_unique(&mut setup.tastes, claim),
                "constraints" => push_unique(&mut setup.constraints, claim),
                "action_tendency" => push_unique(&mut setup.action_tendencies, claim),
                _ => push_unique(&mut setup.values, claim),
            }
        }
    }

    if saw_interview_question {
        push_unique(
            &mut setup.constraints,
            "Keeps interviewee answers as research evidence rather than treating them as the user's own Constitution.".to_string(),
        );
        push_unique(
            &mut setup.constraints,
            "Requires speaker-labelled evidence before extracting Constitution or research findings.".to_string(),
        );
    }
    if saw_values_question {
        push_unique(
            &mut setup.values,
            "Probes for the values and cultural assumptions behind decisions, not only the visible decision outcome.".to_string(),
        );
    }
    if saw_tradeoff_question {
        push_unique(
            &mut setup.values,
            "Treats important choices as tradeoffs between innovation, stability, impact, and institutional responsibility.".to_string(),
        );
    }
    if saw_risk_question {
        push_unique(
            &mut setup.constraints,
            "Separates acceptable experiments from risks that harm people, courses, or institutional trust.".to_string(),
        );
    }
    if saw_concrete_question {
        push_unique(
            &mut setup.tastes,
            "Prefers concrete walkthroughs, examples, and decision stories over abstract positioning.".to_string(),
        );
    }
    if saw_followup_question || (interviewer_question_count > 1 && saw_concrete_question) {
        push_unique(
            &mut setup.action_tendencies,
            "Uses follow-up questions to move from broad narrative into specific decisions, assumptions, criteria, and consequences.".to_string(),
        );
    }
    if saw_user_note_preference {
        push_unique(
            &mut setup.action_tendencies,
            "Turns user-authored note evidence into reviewable Constitution candidates instead of accepting it silently.".to_string(),
        );
    }

    setup
}

fn constitution_setup_entry_count(setup: &ConstitutionSetup) -> usize {
    setup.values.len()
        + setup.tastes.len()
        + setup.constraints.len()
        + setup.somatic_cues.len()
        + setup.action_tendencies.len()
}

fn constitution_setup_claims(setup: &ConstitutionSetup) -> HashSet<String> {
    setup
        .values
        .iter()
        .chain(setup.tastes.iter())
        .chain(setup.constraints.iter())
        .chain(setup.somatic_cues.iter())
        .chain(setup.action_tendencies.iter())
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn push_unique(entries: &mut Vec<String>, entry: String) {
    let normalized = entry.trim();
    if normalized.is_empty() {
        return;
    }
    if !entries
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(normalized))
    {
        entries.push(normalized.to_string());
    }
}

fn constitution_item_is_vault_derived(item: &ConstitutionItem) -> bool {
    matches!(
        item.source.as_deref(),
        Some("note_inference")
            | Some("interview_behavior_inference")
            | Some("interview_research_inference")
    ) || item.evidence_refs.iter().any(|evidence| {
        matches!(
            evidence.source_type.as_deref(),
            Some("note") | Some("interview-question") | Some("interview-answer")
        )
    })
}

fn record_is_vault_derived(record: &UserRecord) -> bool {
    record.metadata.get("source_note_id").is_some()
        || matches!(
            record.metadata.get("source_type").and_then(Value::as_str),
            Some("interview_answer") | Some("note")
        )
        || record.evidence_refs.iter().any(|evidence| {
            matches!(
                evidence.source_type.as_deref(),
                Some("note") | Some("interview-question") | Some("interview-answer")
            )
        })
}

fn split_action_gap_claim(content: &str) -> Option<(String, String)> {
    let lowered = content.to_lowercase();
    for separator in [" but ", " however ", " yet ", " although "] {
        if let Some(index) = lowered.find(separator) {
            let left = content[..index].trim();
            let right = content[index + separator.len()..].trim();
            if left.len() >= 8 && right.len() >= 8 {
                return Some((left.to_string(), right.to_string()));
            }
        }
    }
    None
}

fn constitution_item_relevance(item: &ConstitutionItem, query_terms: &HashSet<String>) -> usize {
    if query_terms.is_empty() {
        return 0;
    }
    let mut haystack = format!("{} {} {}", item.claim, item.dimension, item.scope.join(" "));
    for tension in &item.tensions {
        haystack.push(' ');
        haystack.push_str(tension);
    }
    let item_terms = lexical_terms(&haystack);
    query_terms.intersection(&item_terms).count()
}

fn action_gap_relevance(gap: &ActionGap, query_terms: &HashSet<String>) -> usize {
    if query_terms.is_empty() {
        return 0;
    }
    let mut haystack = format!(
        "{} {} {}",
        gap.stated_value, gap.revealed_behavior, gap.decision_risk
    );
    if let Some(driver) = &gap.driver_hypothesis {
        haystack.push(' ');
        haystack.push_str(driver);
    }
    if let Some(signal) = &gap.somatic_taste_signal {
        haystack.push(' ');
        haystack.push_str(signal);
    }
    let gap_terms = lexical_terms(&haystack);
    query_terms.intersection(&gap_terms).count()
}

fn clamp_decision_mirror_weights(mut weights: DecisionMirrorWeights) -> DecisionMirrorWeights {
    weights.notes_weight = clamp_weight(weights.notes_weight);
    weights.approved_records_weight = clamp_weight(weights.approved_records_weight);
    weights.candidate_records_weight = clamp_weight(weights.candidate_records_weight);
    weights.constitution_weight = clamp_weight(weights.constitution_weight);
    weights.action_gaps_weight = clamp_weight(weights.action_gaps_weight);
    weights.recency_weight = clamp_weight(weights.recency_weight);
    weights.evidence_count_weight = clamp_weight(weights.evidence_count_weight);
    weights.outcome_history_weight = clamp_weight(weights.outcome_history_weight);
    weights.contradiction_weight = clamp_weight(weights.contradiction_weight);
    weights.breadth_weight = clamp_weight(weights.breadth_weight);
    weights.depth_weight = clamp_weight(weights.depth_weight);
    weights.evidence_grounding_weight = clamp_weight(weights.evidence_grounding_weight);
    weights.blind_spot_weight = clamp_weight(weights.blind_spot_weight);
    weights.counter_position_weight = clamp_weight(weights.counter_position_weight);
    weights.actionability_weight = clamp_weight(weights.actionability_weight);
    weights.uncertainty_weight = clamp_weight(weights.uncertainty_weight);
    weights.privacy_weight = clamp_weight(weights.privacy_weight);
    weights.unsupported_penalty_weight = clamp_weight(weights.unsupported_penalty_weight);
    weights
}

fn clamp_weight(weight: f32) -> f32 {
    if weight.is_finite() {
        weight.clamp(0.0, 3.0)
    } else {
        1.0
    }
}

fn promotion_state_label(state: &PromotionState) -> &'static str {
    match state {
        PromotionState::Candidate => "Candidate",
        PromotionState::AutoPromoted => "Auto-promoted",
        PromotionState::Endorsed => "Endorsed",
        PromotionState::Rejected => "Rejected",
        PromotionState::Private => "Private",
        PromotionState::NoTrain => "No-train",
    }
}

fn score_reflection_card(
    content: &str,
    cited_note_ids: &[String],
    cited_user_record_ids: &[String],
    cited_constitution_item_ids: &[String],
    cited_action_gap_ids: &[String],
    config: &DecisionMirrorConfig,
) -> ReflectionScores {
    let lower = content.to_lowercase();
    let section_checks: &[&[&str]] = &[
        &["decision frame", "actual decision"],
        &["reasoning pattern", "likely reasoning", "default pattern"],
        &["evidence", "vault", "record"],
        &["blind spot", "missing", "underweighting"],
        &["counter-position", "counter position", "counterargument"],
        &["recommendation", "would do next"],
        &["confidence", "uncertain", "would change my mind"],
        &["next action", "smallest", "follow-up"],
    ];
    let present_sections = section_checks
        .iter()
        .filter(|aliases| aliases.iter().any(|alias| lower.contains(alias)))
        .count();
    let breadth_score = present_sections as f32 / section_checks.len() as f32;

    let word_count = content.split_whitespace().count() as f32;
    let depth_score = ((word_count / 450.0).min(1.0) * 0.5)
        + (phrase_score(
            &lower,
            &[
                "because",
                "tradeoff",
                "evidence",
                "alternative",
                "would change",
                "unsupported",
            ],
        ) * 0.5);

    let evidence_grounding_score = if !cited_note_ids.is_empty()
        || !cited_user_record_ids.is_empty()
        || !cited_constitution_item_ids.is_empty()
        || !cited_action_gap_ids.is_empty()
    {
        1.0
    } else if lower.contains("evidence")
        || lower.contains("note")
        || lower.contains("record")
        || lower.contains("based on")
    {
        0.5
    } else {
        0.0
    };

    let blind_spot_score = phrase_score(
        &lower,
        &[
            "blind spot",
            "missing",
            "underweight",
            "bias",
            "avoid",
            "overfit",
        ],
    );
    let actionability_score = phrase_score(
        &lower,
        &["next action", "smallest", "experiment", "step", "by "],
    );
    let counterargument_score = phrase_score(
        &lower,
        &[
            "counter",
            "strongest argument",
            "against",
            "alternative frame",
        ],
    );
    let uncertainty_score = phrase_score(
        &lower,
        &[
            "hypothesis",
            "may",
            "seem",
            "confidence",
            "uncertain",
            "would change",
        ],
    );
    let privacy_score = 1.0;
    let unsupported_claim_count = unsupported_self_claim_count(
        &lower,
        evidence_grounding_score,
        cited_note_ids,
        cited_user_record_ids,
        cited_constitution_item_ids,
        cited_action_gap_ids,
    );
    let (overall_score, weighted_breakdown) = weighted_reflection_score(
        config,
        &[
            ("breadth", breadth_score),
            ("depth", depth_score.min(1.0)),
            ("evidence_grounding", evidence_grounding_score),
            ("blind_spot", blind_spot_score),
            ("counter_position", counterargument_score),
            ("actionability", actionability_score),
            ("uncertainty", uncertainty_score),
            ("privacy", privacy_score),
        ],
        unsupported_claim_count,
    );

    ReflectionScores {
        breadth_score,
        depth_score: depth_score.min(1.0),
        evidence_grounding_score,
        blind_spot_score,
        actionability_score,
        counterargument_score,
        uncertainty_score,
        privacy_score,
        unsupported_claim_count,
        overall_score,
        weighted_breakdown,
    }
}

fn weighted_reflection_score(
    config: &DecisionMirrorConfig,
    scores: &[(&str, f32)],
    unsupported_claim_count: u32,
) -> (f32, HashMap<String, f32>) {
    let weights = &config.weights;
    let weight_for = |key: &str| match key {
        "breadth" => weights.breadth_weight,
        "depth" => weights.depth_weight,
        "evidence_grounding" => weights.evidence_grounding_weight,
        "blind_spot" => weights.blind_spot_weight,
        "counter_position" => weights.counter_position_weight,
        "actionability" => weights.actionability_weight,
        "uncertainty" => weights.uncertainty_weight,
        "privacy" => weights.privacy_weight,
        _ => 1.0,
    };

    let mut weighted_breakdown = HashMap::new();
    let mut weighted_sum = 0.0_f32;
    let mut weight_sum = 0.0_f32;
    for (key, score) in scores {
        let weight = clamp_weight(weight_for(key));
        let contribution = score.clamp(0.0, 1.0) * weight;
        weighted_breakdown.insert((*key).to_string(), contribution);
        weighted_sum += contribution;
        weight_sum += weight;
    }

    let unsupported_penalty =
        (unsupported_claim_count.min(5) as f32 / 5.0) * weights.unsupported_penalty_weight;
    weighted_breakdown.insert("unsupported_penalty".to_string(), -unsupported_penalty);

    if weight_sum <= 0.0 {
        return (0.0, weighted_breakdown);
    }

    let overall = ((weighted_sum - unsupported_penalty) / weight_sum).clamp(0.0, 1.0);
    (overall, weighted_breakdown)
}

fn phrase_score(content: &str, phrases: &[&str]) -> f32 {
    let hits = phrases
        .iter()
        .filter(|phrase| content.contains(**phrase))
        .count();
    (hits as f32 / phrases.len().max(1) as f32).min(1.0)
}

fn unsupported_self_claim_count(
    lower: &str,
    evidence_grounding_score: f32,
    cited_note_ids: &[String],
    cited_user_record_ids: &[String],
    cited_constitution_item_ids: &[String],
    cited_action_gap_ids: &[String],
) -> u32 {
    let has_structural_evidence = !cited_note_ids.is_empty()
        || !cited_user_record_ids.is_empty()
        || !cited_constitution_item_ids.is_empty()
        || !cited_action_gap_ids.is_empty()
        || evidence_grounding_score >= 0.5;
    if has_structural_evidence {
        return 0;
    }

    [
        "you seem",
        "you may",
        "you often",
        "you tend",
        "your likely",
        "based on your",
        "you prefer",
        "you avoid",
    ]
    .iter()
    .map(|needle| lower.matches(needle).count() as u32)
    .sum()
}

const PREDICTION_OPTION_MAX_CHARS: usize = 500;

const PREDICTION_RATIONALE_MAX_CHARS: usize = 2000;

/// Normalize an option string for comparison: trim, strip wrapping quotes,
/// lowercase, collapse whitespace. Label forms ("Option 2", "B") are handled
/// separately by `match_option_index` so meaningful digits survive.
pub fn normalize_option(text: &str) -> String {
    text.trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        text.chars().take(max_chars).collect()
    }
}

fn match_option_index(text: &str, options: &[String]) -> Option<usize> {
    let normalized = normalize_option(text);
    if normalized.is_empty() {
        return None;
    }

    // Label forms: "option 2" / "choice b" / bare "2" / bare "b".
    let label = ["option", "choice"]
        .iter()
        .find_map(|prefix| normalized.strip_prefix(prefix))
        .map(str::trim)
        .unwrap_or(&normalized)
        .trim_matches(|c: char| c == '.' || c == ')' || c == ':' || c.is_whitespace());
    if label.len() == 1 {
        if let Some(letter) = label.chars().next() {
            if letter.is_ascii_lowercase() {
                let index = (letter as usize) - ('a' as usize);
                if index < options.len() {
                    return Some(index);
                }
            }
        }
    }
    // Bare number labels: humans count from 1.
    if let Ok(number) = label.parse::<usize>() {
        if (1..=options.len()).contains(&number) {
            return Some(number - 1);
        }
    }

    options
        .iter()
        .position(|option| normalize_option(option) == normalized)
}

fn sanitize_confidence(raw: Option<f64>) -> Option<f32> {
    let value = raw?;
    if !value.is_finite() {
        return None;
    }
    let value = if value > 2.0 && value <= 100.0 {
        // Percent-style answer ("73" meaning 73%).
        value / 100.0
    } else {
        // Near-misses like 1.2 are over-confident, not percentages.
        value
    };
    Some(value.clamp(0.0, 1.0) as f32)
}

/// Extract the first balanced `{...}` span from `raw`, for best-effort JSON parsing of
/// model output. Returns `None` if there's no opening brace, no closing brace, or the
/// closing brace appears before the opening one (e.g. truncated/malformed model output
/// like `"Option A} — but {incomplete"`) — guarding this instead of blindly slicing
/// `&raw[start..=end]` is what prevents a panic on malformed input.
fn extract_json_slice(raw: &str) -> Option<&str> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if start > end {
        return None;
    }
    Some(&raw[start..=end])
}

/// Parse the raw model output of a sealed-prediction call into a draft.
/// Fallback chain: fenced/embedded strict JSON -> normalized string match
/// against `options` -> raw text (manual adjudication later).
pub fn parse_twin_prediction(raw: &str, options: &[String]) -> TwinPredictionDraft {
    #[derive(serde::Deserialize)]
    struct ParsedPrediction {
        predicted_option: Option<String>,
        option_index: Option<Value>,
        confidence: Option<f64>,
        rationale: Option<String>,
    }

    let json_slice = extract_json_slice(raw);

    if let Some(slice) = json_slice {
        if let Ok(parsed) = serde_json::from_str::<ParsedPrediction>(slice) {
            let text = parsed.predicted_option.unwrap_or_default();
            let text_index = match_option_index(&text, options);
            let field_index = parsed.option_index.as_ref().and_then(|value| match value {
                Value::Number(number) => number.as_u64().map(|n| n as usize),
                Value::String(text) => text.trim().parse::<usize>().ok(),
                _ => None,
            });
            // The prompt displays a 1-based option list, so prefer the
            // 1-based reading; tolerate 0-based answers, and let the option
            // text disambiguate when both readings are valid.
            let field_index = field_index.and_then(|index| {
                let one_based = index.checked_sub(1).filter(|i| *i < options.len());
                let zero_based = Some(index).filter(|i| *i < options.len());
                match (one_based, zero_based) {
                    (Some(ob), Some(zb)) => match text_index {
                        Some(text_idx) if text_idx == zb => Some(zb),
                        _ => Some(ob),
                    },
                    (Some(ob), None) => Some(ob),
                    (None, Some(zb)) => Some(zb),
                    (None, None) => None,
                }
            });
            // The option text is what the model actually said; a conflicting
            // numeric index loses.
            let matched_option_index = match (text_index, field_index) {
                (Some(text_idx), _) => Some(text_idx),
                (None, Some(field_idx)) if text.is_empty() => Some(field_idx),
                _ => None,
            };
            let predicted_option = if text.is_empty() {
                matched_option_index
                    .map(|index| options[index].clone())
                    .unwrap_or_else(|| truncate_chars(raw.trim(), PREDICTION_OPTION_MAX_CHARS))
            } else {
                truncate_chars(&text, PREDICTION_OPTION_MAX_CHARS)
            };
            return TwinPredictionDraft {
                predicted_option,
                matched_option_index,
                confidence: sanitize_confidence(parsed.confidence),
                rationale: parsed
                    .rationale
                    .map(|text| truncate_chars(&text, PREDICTION_RATIONALE_MAX_CHARS)),
                parse_mode: "json".to_string(),
            };
        }
    }

    if let Some(index) = match_option_index(raw, options) {
        return TwinPredictionDraft {
            predicted_option: options[index].clone(),
            matched_option_index: Some(index),
            confidence: None,
            rationale: None,
            parse_mode: "string_match".to_string(),
        };
    }

    TwinPredictionDraft {
        predicted_option: truncate_chars(raw.trim(), PREDICTION_OPTION_MAX_CHARS),
        matched_option_index: None,
        confidence: None,
        rationale: None,
        parse_mode: "raw".to_string(),
    }
}

fn compute_agreement(prediction: &TwinPrediction, chosen: &str, options: &[String]) -> bool {
    let chosen_index = match_option_index(chosen, options);
    match (prediction.matched_option_index, chosen_index) {
        (Some(predicted), Some(chosen)) => predicted == chosen,
        _ => normalize_option(&prediction.predicted_option) == normalize_option(chosen),
    }
}

fn decision_case_relevance(episode: &DecisionEpisode, query_terms: &HashSet<String>) -> usize {
    if query_terms.is_empty() {
        return 0;
    }

    let mut haystack = episode.decision.clone();
    haystack.push(' ');
    haystack.push_str(&episode.options.join(" "));
    for text in [
        episode.chosen_option.as_deref(),
        episode.initial_leaning.as_deref(),
        episode.lesson.as_deref(),
        episode.outcome.as_deref(),
        episode.correction_note.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        haystack.push(' ');
        haystack.push_str(text);
    }

    let episode_terms = lexical_terms(&haystack);
    query_terms.intersection(&episode_terms).count()
}

fn memory_digest_trigger(record: &UserRecord) -> Option<&'static str> {
    if record.kind == UserRecordKind::Fact {
        return None;
    }

    match record.promotion_state {
        PromotionState::Rejected | PromotionState::Private | PromotionState::NoTrain => None,
        PromotionState::AutoPromoted
            if record.evidence_refs.len() >= AUTO_PROMOTE_SUPPORT_COUNT =>
        {
            Some("3+ evidence points support this durable pattern")
        }
        PromotionState::Candidate if record.evidence_refs.len() >= AUTO_PROMOTE_SUPPORT_COUNT => {
            Some("candidate pattern has enough evidence for review")
        }
        PromotionState::Candidate
            if record.confidence >= AUTO_PROMOTE_CONFIDENCE && !record.evidence_refs.is_empty() =>
        {
            Some("high-confidence candidate pattern needs review")
        }
        PromotionState::Endorsed if stale_for_review(record) => {
            Some("endorsed pattern may be stale")
        }
        PromotionState::AutoPromoted => None,
        PromotionState::Endorsed => None,
        PromotionState::Candidate => None,
    }
}

fn memory_digest_cluster_key(record: &UserRecord) -> String {
    if let Some(signal_family) = record
        .metadata
        .get("signal_family")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return format!("{:?}::{}", record.kind, signal_family.trim().to_lowercase());
    }

    let mut terms = lexical_terms(&record.content)
        .into_iter()
        .collect::<Vec<_>>();
    terms.sort();
    terms.truncate(6);
    if terms.is_empty() {
        format!("{:?}::{}", record.kind, normalize_key_text(&record.content))
    } else {
        format!("{:?}::{}", record.kind, terms.join("-"))
    }
}

fn stable_digest_id(records: &[UserRecord]) -> String {
    let mut ids = records
        .iter()
        .map(|record| record.id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    let mut hasher = DefaultHasher::new();
    ids.hash(&mut hasher);
    format!("digest-cluster-{:x}", hasher.finish())
}

fn stale_for_review(record: &UserRecord) -> bool {
    Utc::now()
        .signed_duration_since(record.updated_at)
        .num_days()
        >= 90
}

fn digest_state_sort_key(state: &MemoryDigestState) -> u8 {
    match state {
        MemoryDigestState::Pending => 0,
        MemoryDigestState::Softened => 1,
        MemoryDigestState::Kept => 2,
        MemoryDigestState::NotMe => 3,
        MemoryDigestState::Private => 4,
        MemoryDigestState::NoTrain => 5,
        MemoryDigestState::Rejected => 6,
    }
}

fn memory_digest_state_for_action(action: &MemoryDigestAction) -> MemoryDigestState {
    match action {
        MemoryDigestAction::Keep => MemoryDigestState::Kept,
        MemoryDigestAction::Soften => MemoryDigestState::Softened,
        MemoryDigestAction::NotMe => MemoryDigestState::NotMe,
        MemoryDigestAction::Private => MemoryDigestState::Private,
        MemoryDigestAction::NoTrain => MemoryDigestState::NoTrain,
        MemoryDigestAction::Reject => MemoryDigestState::Rejected,
    }
}

pub struct TwinStore {
    root_path: PathBuf,
    traces_path: PathBuf,
    records_path: PathBuf,
    decisions_path: PathBuf,
    reflections_path: PathBuf,
    constitution_path: PathBuf,
    action_gaps_path: PathBuf,
    setup_path: PathBuf,
    decision_mirror_config_path: PathBuf,
    digest_path: PathBuf,
    exports_path: PathBuf,
    trace_cache: HashMap<String, SessionTrace>,
    record_cache: HashMap<String, UserRecord>,
    records_cache_ready: bool,
}

impl TwinStore {
    pub fn new(root_path: PathBuf) -> Self {
        let traces_path = root_path.join("traces");
        let records_path = root_path.join("records");
        let decisions_path = root_path.join("decisions");
        let reflections_path = root_path.join("reflections");
        let constitution_path = root_path.join("constitution");
        let action_gaps_path = root_path.join("action_gaps");
        let setup_path = root_path.join("constitution_setup.json");
        let decision_mirror_config_path = root_path.join("decision_mirror_config.json");
        let digest_path = root_path.join("memory_digest.json");
        let exports_path = root_path.join("exports");

        std::fs::create_dir_all(&traces_path).ok();
        std::fs::create_dir_all(&records_path).ok();
        std::fs::create_dir_all(&decisions_path).ok();
        std::fs::create_dir_all(&reflections_path).ok();
        std::fs::create_dir_all(&constitution_path).ok();
        std::fs::create_dir_all(&action_gaps_path).ok();
        std::fs::create_dir_all(&exports_path).ok();

        Self {
            root_path,
            traces_path,
            records_path,
            decisions_path,
            reflections_path,
            constitution_path,
            action_gaps_path,
            setup_path,
            decision_mirror_config_path,
            digest_path,
            exports_path,
            trace_cache: HashMap::new(),
            record_cache: HashMap::new(),
            records_cache_ready: false,
        }
    }

    fn write_pretty_json<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        let content = serde_json::to_string_pretty(value)?;
        write_atomic(path, content.as_bytes())
            .with_context(|| format!("Failed to write JSON file: {}", path.display()))
    }

    fn validate_file_id(id: &str) -> Result<()> {
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            anyhow::bail!("Invalid file id: {}", id);
        }

        Ok(())
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

    pub fn list_constitution_items(&self) -> Result<Vec<ConstitutionItem>> {
        let mut items = Vec::new();
        if !self.constitution_path.exists() {
            return Ok(items);
        }

        for entry in WalkDir::new(&self.constitution_path)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Some(item) =
                    load_or_quarantine::<ConstitutionItem>(path, "constitution item")
                {
                    items.push(item);
                }
            }
        }

        items.sort_by(|a, b| {
            constitution_status_sort_key(&a.status)
                .cmp(&constitution_status_sort_key(&b.status))
                .then_with(|| {
                    b.priority
                        .partial_cmp(&a.priority)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        Ok(items)
    }

    pub fn create_constitution_item(
        &self,
        create: ConstitutionItemCreate,
    ) -> Result<ConstitutionItem> {
        let now = Utc::now();
        let item = ConstitutionItem {
            id: uuid::Uuid::new_v4().to_string(),
            claim: create.claim,
            dimension: create.dimension,
            scope: create.scope,
            priority: create.priority.clamp(0.0, 1.0),
            confidence: create.confidence.clamp(0.0, 1.0),
            status: create.status,
            evidence_refs: create.evidence_refs,
            tensions: create.tensions,
            linked_record_ids: create.linked_record_ids,
            source: create.source,
            created_at: now,
            updated_at: now,
        };
        self.write_constitution_file(&item)?;
        Ok(item)
    }

    pub fn update_constitution_item(
        &self,
        id: &str,
        update: ConstitutionItemUpdate,
    ) -> Result<ConstitutionItem> {
        Self::validate_file_id(id)?;
        let path = self.constitution_file_path(id);
        let mut item = self.read_constitution_file(&path)?;
        if let Some(claim) = update.claim {
            item.claim = claim;
        }
        if let Some(dimension) = update.dimension {
            item.dimension = dimension;
        }
        if let Some(scope) = update.scope {
            item.scope = scope;
        }
        if let Some(priority) = update.priority {
            item.priority = priority.clamp(0.0, 1.0);
        }
        if let Some(confidence) = update.confidence {
            item.confidence = confidence.clamp(0.0, 1.0);
        }
        if let Some(status) = update.status {
            item.status = status;
        }
        if let Some(tensions) = update.tensions {
            item.tensions = tensions;
        }
        item.updated_at = Utc::now();
        self.write_constitution_file(&item)?;
        Ok(item)
    }

    pub fn review_constitution_item(
        &mut self,
        id: &str,
        request: ConstitutionReviewRequest,
    ) -> Result<ConstitutionItem> {
        Self::validate_file_id(id)?;
        let mut item = self.read_constitution_file(&self.constitution_file_path(id))?;
        item.status = constitution_status_for_action(&request.action);
        item.updated_at = Utc::now();
        self.write_constitution_file(&item)?;
        self.append_trace_event(
            "constitution-review",
            TraceEventType::ConstitutionItemReviewed,
            json!({
                "constitution_item_id": item.id,
                "action": request.action,
                "rationale": request.rationale,
            }),
        )?;
        Ok(item)
    }

    pub fn list_action_gaps(&self) -> Result<Vec<ActionGap>> {
        let mut gaps = Vec::new();
        if !self.action_gaps_path.exists() {
            return Ok(gaps);
        }

        for entry in WalkDir::new(&self.action_gaps_path)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Some(gap) = load_or_quarantine::<ActionGap>(path, "action gap") {
                    gaps.push(gap);
                }
            }
        }

        gaps.sort_by(|a, b| {
            constitution_status_sort_key(&a.status)
                .cmp(&constitution_status_sort_key(&b.status))
                .then_with(|| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        Ok(gaps)
    }

    pub fn create_action_gap(&self, create: ActionGapCreate) -> Result<ActionGap> {
        let now = Utc::now();
        let gap = ActionGap {
            id: uuid::Uuid::new_v4().to_string(),
            stated_value: create.stated_value,
            revealed_behavior: create.revealed_behavior,
            driver_hypothesis: create.driver_hypothesis,
            somatic_taste_signal: create.somatic_taste_signal,
            decision_risk: create.decision_risk,
            evidence_refs: create.evidence_refs,
            linked_record_ids: create.linked_record_ids,
            confidence: create.confidence.clamp(0.0, 1.0),
            status: create.status,
            created_at: now,
            updated_at: now,
        };
        self.write_action_gap_file(&gap)?;
        Ok(gap)
    }

    pub fn review_action_gap(
        &mut self,
        id: &str,
        request: ConstitutionReviewRequest,
    ) -> Result<ActionGap> {
        Self::validate_file_id(id)?;
        let mut gap = self.read_action_gap_file(&self.action_gap_file_path(id))?;
        gap.status = constitution_status_for_action(&request.action);
        gap.updated_at = Utc::now();
        self.write_action_gap_file(&gap)?;
        self.append_trace_event(
            "constitution-review",
            TraceEventType::ActionGapReviewed,
            json!({
                "action_gap_id": gap.id,
                "action": request.action,
                "rationale": request.rationale,
            }),
        )?;
        Ok(gap)
    }

    pub fn get_constitution_setup(&self) -> Result<ConstitutionSetup> {
        if !self.setup_path.exists() {
            return Ok(ConstitutionSetup::default());
        }

        let content = std::fs::read_to_string(&self.setup_path).with_context(|| {
            format!(
                "Failed to read constitution setup file: {}",
                self.setup_path.display()
            )
        })?;
        serde_json::from_str(&content).with_context(|| {
            format!(
                "Failed to parse constitution setup file: {}",
                self.setup_path.display()
            )
        })
    }

    pub fn save_constitution_setup(
        &mut self,
        mut setup: ConstitutionSetup,
    ) -> Result<ConstitutionSetup> {
        setup.twin_name = clean_optional_setup_entry(setup.twin_name);
        setup.twin_role = clean_optional_setup_entry(setup.twin_role);
        setup.source_boundaries = clean_setup_entries(setup.source_boundaries);
        setup.values = clean_setup_entries(setup.values);
        setup.tastes = clean_setup_entries(setup.tastes);
        setup.constraints = clean_setup_entries(setup.constraints);
        setup.somatic_cues = clean_setup_entries(setup.somatic_cues);
        setup.action_tendencies = clean_setup_entries(setup.action_tendencies);
        setup.updated_at = Some(Utc::now());
        self.write_pretty_json(&self.setup_path, &setup)?;

        let event = self.append_trace_event(
            "constitution-setup",
            TraceEventType::ConstitutionSetupSaved,
            json!({
                "twin_name": setup.twin_name,
                "twin_role": setup.twin_role,
                "source_boundaries": setup.source_boundaries,
                "values": setup.values,
                "tastes": setup.tastes,
                "constraints": setup.constraints,
                "somatic_cues": setup.somatic_cues,
                "action_tendencies": setup.action_tendencies,
            }),
        )?;
        let evidence_ref = EvidenceRef {
            trace_id: "constitution-setup".to_string(),
            event_id: event.id,
            session_id: "constitution-setup".to_string(),
            tile_id: None,
            model_id: None,
            note: Some("Guided Twin setup".to_string()),
            source_type: Some("setup".to_string()),
            source_id: Some("constitution-setup".to_string()),
            source_label: Some("Guided Twin setup".to_string()),
            excerpt: None,
            speaker_role: None,
        };

        self.seed_constitution_setup_items(&setup, evidence_ref)?;
        Ok(setup)
    }

    pub fn run_constitution_inference(&mut self) -> Result<ConstitutionInferenceSummary> {
        self.run_constitution_inference_with_notes(&[])
    }

    pub fn run_constitution_inference_with_notes(
        &mut self,
        notes: &[Note],
    ) -> Result<ConstitutionInferenceSummary> {
        self.ensure_record_cache()?;
        let current_note_ids = notes
            .iter()
            .map(|note| note.id.clone())
            .collect::<HashSet<_>>();
        let pruned_stale_records = self.prune_stale_note_records(&current_note_ids)?;
        let distilled_setup = distill_constitution_setup_from_notes(notes);
        let updated_setup_entries = constitution_setup_entry_count(&distilled_setup);
        let pruned_stale_constitution_items = self
            .prune_stale_note_constitution_items(&current_note_ids)?
            + self.prune_stale_setup_constitution_items(&distilled_setup)?;
        if updated_setup_entries > 0 || self.setup_path.exists() {
            self.write_auto_constitution_setup(distilled_setup.clone())?;
        }
        self.ensure_record_cache()?;
        let records = self
            .record_cache
            .values()
            .filter(|record| constitution_allows_record(record))
            .cloned()
            .collect::<Vec<_>>();
        let traces = self.list_session_traces()?;
        let scanned_behavior_events = traces
            .iter()
            .flat_map(|trace| trace.events.iter())
            .filter(|event| behavior_event_for_constitution(&event.event_type))
            .count();
        let decisions = self.list_decision_episodes()?;
        let existing_items = self.list_constitution_items()?;
        let existing_gaps = self.list_action_gaps()?;
        let mut item_keys = existing_items
            .iter()
            .map(|item| constitution_key(&item.dimension, &item.claim))
            .collect::<HashSet<_>>();
        let mut gap_keys = existing_gaps
            .iter()
            .map(|gap| action_gap_key(&gap.stated_value, &gap.revealed_behavior))
            .collect::<HashSet<_>>();

        let mut created_constitution_items = 0_usize;
        let mut created_action_gaps = 0_usize;
        let mut auto_active_items = 0_usize;
        let mut review_candidate_items = 0_usize;
        let mut skipped_domain_claims = 0_usize;
        let mut extracted_research_findings = 0_usize;

        for record in &records {
            let dimension = infer_constitution_dimension(&record.content, &record.kind);
            let claim = record.content.trim().to_string();
            if claim.is_empty() {
                continue;
            }
            let key = constitution_key(&dimension, &claim);
            if item_keys.insert(key) {
                let status = constitution_status_from_record(record);
                if status == ConstitutionStatus::Active {
                    auto_active_items += 1;
                } else if status == ConstitutionStatus::Candidate {
                    review_candidate_items += 1;
                }
                let source = if record.origin == RecordOrigin::Inferred {
                    "behavior_inference"
                } else {
                    "constitution_inference"
                };
                self.create_constitution_item(ConstitutionItemCreate {
                    claim,
                    dimension,
                    scope: vec!["general".to_string()],
                    priority: record.confidence,
                    confidence: record.confidence,
                    status,
                    evidence_refs: record.evidence_refs.clone(),
                    tensions: Vec::new(),
                    linked_record_ids: vec![record.id.clone()],
                    source: Some(source.to_string()),
                })?;
                created_constitution_items += 1;
            }

            if let Some((stated_value, revealed_behavior)) = split_action_gap_claim(&record.content)
            {
                let key = action_gap_key(&stated_value, &revealed_behavior);
                if gap_keys.insert(key) {
                    self.create_action_gap(ActionGapCreate {
                        stated_value,
                        revealed_behavior,
                        driver_hypothesis: Some(
                            "Inferred from a contradiction-style user record".to_string(),
                        ),
                        somatic_taste_signal: None,
                        decision_risk:
                            "The user's endorsed intent may diverge from what they repeatedly do."
                                .to_string(),
                        evidence_refs: record.evidence_refs.clone(),
                        linked_record_ids: vec![record.id.clone()],
                        confidence: record.confidence,
                        status: ConstitutionStatus::Candidate,
                    })?;
                    created_action_gaps += 1;
                }
            }
        }

        for decision in &decisions {
            let Some(initial_leaning) = decision.initial_leaning.as_ref() else {
                continue;
            };
            let Some(chosen_option) = decision.chosen_option.as_ref() else {
                continue;
            };
            if initial_leaning
                .trim()
                .eq_ignore_ascii_case(chosen_option.trim())
            {
                continue;
            }
            let stated_value = format!("Initial leaning: {}", initial_leaning.trim());
            let revealed_behavior = format!("Chosen option: {}", chosen_option.trim());
            let key = action_gap_key(&stated_value, &revealed_behavior);
            if gap_keys.insert(key) {
                self.create_action_gap(ActionGapCreate {
                    stated_value,
                    revealed_behavior,
                    driver_hypothesis: Some("Inferred from a decision where final action diverged from initial leaning.".to_string()),
                    somatic_taste_signal: decision.primitive_assessment.somatic_signal.clone(),
                    decision_risk: "Grafyn should check whether future similar decisions repeat this gap.".to_string(),
                    evidence_refs: self.decision_evidence_refs(&decision.id)?,
                    linked_record_ids: Vec::new(),
                    confidence: 0.68,
                    status: ConstitutionStatus::Candidate,
                })?;
                created_action_gaps += 1;
            }
        }

        for note in notes {
            if note_is_interview(note) {
                match extract_interview_note_evidence(note) {
                    InterviewExtraction::Unlabeled => {
                        skipped_domain_claims += 1;
                    }
                    InterviewExtraction::Labeled {
                        interviewer_turns,
                        interviewee_turns,
                    } => {
                        for turn in interviewer_turns {
                            if let Some((dimension, claim)) =
                                constitution_claim_from_interviewer_turn(&turn.content)
                            {
                                let key = constitution_key(&dimension, &claim);
                                if item_keys.insert(key) {
                                    self.create_constitution_item(ConstitutionItemCreate {
                                        claim,
                                        dimension,
                                        scope: vec!["interview".to_string()],
                                        priority: 0.62,
                                        confidence: 0.62,
                                        status: ConstitutionStatus::Candidate,
                                        evidence_refs: vec![note_evidence_ref(
                                            note,
                                            "interview-question",
                                            "Interviewer question",
                                            Some("user"),
                                            &turn.content,
                                        )],
                                        tensions: Vec::new(),
                                        linked_record_ids: Vec::new(),
                                        source: Some("interview_behavior_inference".to_string()),
                                    })?;
                                    created_constitution_items += 1;
                                    review_candidate_items += 1;
                                }
                            }
                        }
                        for turn in interviewee_turns {
                            if !looks_like_research_finding(&turn.content) {
                                skipped_domain_claims += 1;
                                continue;
                            }
                            if self.create_interview_research_record(note, &turn.content)? {
                                extracted_research_findings += 1;
                            }
                        }
                    }
                }
                continue;
            }

            if let Some((dimension, claim)) = constitution_claim_from_note(note) {
                let key = constitution_key(&dimension, &claim);
                if item_keys.insert(key) {
                    self.create_constitution_item(ConstitutionItemCreate {
                        claim,
                        dimension,
                        scope: vec!["note".to_string()],
                        priority: 0.58,
                        confidence: 0.58,
                        status: ConstitutionStatus::Candidate,
                        evidence_refs: vec![note_evidence_ref(
                            note,
                            "note",
                            "Vault note",
                            Some("user"),
                            &note.content,
                        )],
                        tensions: Vec::new(),
                        linked_record_ids: Vec::new(),
                        source: Some("note_inference".to_string()),
                    })?;
                    created_constitution_items += 1;
                    review_candidate_items += 1;
                }
            } else {
                skipped_domain_claims += 1;
            }
        }

        let summary = ConstitutionInferenceSummary {
            scanned_records: records.len(),
            scanned_decisions: decisions.len(),
            created_constitution_items,
            created_action_gaps,
            scanned_behavior_events,
            scanned_notes: notes.len(),
            scanned_interviews: notes.iter().filter(|note| note_is_interview(note)).count(),
            auto_active_items,
            review_candidate_items,
            skipped_domain_claims,
            extracted_research_findings,
            pruned_stale_constitution_items,
            pruned_stale_records,
            updated_setup_entries,
            generated_at: Utc::now(),
        };
        self.append_trace_event(
            "constitution-inference",
            TraceEventType::ConstitutionInferenceRun,
            serde_json::to_value(&summary)?,
        )?;
        Ok(summary)
    }

    fn create_interview_research_record(&mut self, note: &Note, content: &str) -> Result<bool> {
        self.ensure_record_cache()?;
        let finding = format!("Interview finding: {}", excerpt(content.trim()));
        let existing = self.record_cache.values().any(|record| {
            record.kind == UserRecordKind::Fact
                && record.content == finding
                && record.metadata.get("source_type").and_then(Value::as_str)
                    == Some("interview_answer")
        });
        if existing {
            return Ok(false);
        }

        self.create_user_record(UserRecordCreate {
            kind: UserRecordKind::Fact,
            content: finding,
            evidence_refs: vec![note_evidence_ref(
                note,
                "interview-answer",
                "Interviewee answer",
                Some("interviewee"),
                content,
            )],
            confidence: 0.66,
            origin: RecordOrigin::Inferred,
            promotion_state: Some(PromotionState::Candidate),
            valid_from: None,
            valid_until: None,
            links: Vec::new(),
            metadata: HashMap::from([
                ("source_type".to_string(), json!("interview_answer")),
                ("source_note_id".to_string(), json!(note.id.clone())),
            ]),
        })?;
        Ok(true)
    }

    fn write_auto_constitution_setup(&self, mut setup: ConstitutionSetup) -> Result<()> {
        if let Ok(existing) = self.get_constitution_setup() {
            setup.twin_name = existing.twin_name;
            setup.twin_role = existing.twin_role;
            setup.source_boundaries = existing.source_boundaries;
        }
        setup.twin_name = clean_optional_setup_entry(setup.twin_name);
        setup.twin_role = clean_optional_setup_entry(setup.twin_role);
        setup.source_boundaries = clean_setup_entries(setup.source_boundaries);
        setup.values = clean_setup_entries(setup.values);
        setup.tastes = clean_setup_entries(setup.tastes);
        setup.constraints = clean_setup_entries(setup.constraints);
        setup.somatic_cues = clean_setup_entries(setup.somatic_cues);
        setup.action_tendencies = clean_setup_entries(setup.action_tendencies);
        setup.updated_at = Some(Utc::now());
        self.write_pretty_json(&self.setup_path, &setup)
    }

    fn prune_stale_note_records(&mut self, current_note_ids: &HashSet<String>) -> Result<usize> {
        self.ensure_record_cache()?;
        let stale_ids = self
            .record_cache
            .values()
            .filter(|record| record_is_vault_derived(record))
            .filter(|record| {
                record
                    .metadata
                    .get("source_note_id")
                    .and_then(Value::as_str)
                    .is_some_and(|source_note_id| !current_note_ids.contains(source_note_id))
            })
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();

        for id in &stale_ids {
            self.record_cache.remove(id);
            let path = self.records_path.join(format!("{}.json", id));
            if path.exists() {
                std::fs::remove_file(&path).with_context(|| {
                    format!("Failed to remove stale Twin record: {}", path.display())
                })?;
            }
        }

        Ok(stale_ids.len())
    }

    fn prune_stale_note_constitution_items(
        &self,
        current_note_ids: &HashSet<String>,
    ) -> Result<usize> {
        let stale_items = self
            .list_constitution_items()?
            .into_iter()
            .filter(|item| constitution_item_is_vault_derived(item))
            .filter(|item| {
                item.evidence_refs
                    .iter()
                    .filter_map(|evidence| evidence.source_id.as_deref())
                    .any(|source_id| !current_note_ids.contains(source_id))
            })
            .collect::<Vec<_>>();

        for item in &stale_items {
            let path = self.constitution_path.join(format!("{}.json", item.id));
            if path.exists() {
                std::fs::remove_file(&path).with_context(|| {
                    format!(
                        "Failed to remove stale Constitution item: {}",
                        path.display()
                    )
                })?;
            }
        }

        Ok(stale_items.len())
    }

    fn prune_stale_setup_constitution_items(&self, setup: &ConstitutionSetup) -> Result<usize> {
        let setup_claims = constitution_setup_claims(setup);
        let stale_items = self
            .list_constitution_items()?
            .into_iter()
            .filter(|item| item.source.as_deref() == Some("guided_setup"))
            .filter(|item| !setup_claims.contains(item.claim.trim()))
            .collect::<Vec<_>>();

        for item in &stale_items {
            let path = self.constitution_path.join(format!("{}.json", item.id));
            if path.exists() {
                std::fs::remove_file(&path).with_context(|| {
                    format!(
                        "Failed to remove stale setup Constitution item: {}",
                        path.display()
                    )
                })?;
            }
        }

        Ok(stale_items.len())
    }

    pub fn select_constitution_context(
        &self,
        query: &str,
    ) -> Result<(Vec<ConstitutionItem>, Vec<ActionGap>)> {
        let query_terms = lexical_terms(query);
        let mut items = self
            .list_constitution_items()?
            .into_iter()
            .filter(|item| constitution_context_allowed(&item.status))
            .map(|item| (constitution_item_relevance(&item, &query_terms), item))
            .filter(|(score, item)| *score > 0 || item.status == ConstitutionStatus::Active)
            .collect::<Vec<_>>();
        items.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| {
                    b.1.priority
                        .partial_cmp(&a.1.priority)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| b.1.updated_at.cmp(&a.1.updated_at))
        });

        let mut gaps = self
            .list_action_gaps()?
            .into_iter()
            .filter(|gap| constitution_context_allowed(&gap.status))
            .map(|gap| (action_gap_relevance(&gap, &query_terms), gap))
            .filter(|(score, gap)| *score > 0 || gap.status == ConstitutionStatus::Active)
            .collect::<Vec<_>>();
        gaps.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| {
                    b.1.confidence
                        .partial_cmp(&a.1.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| b.1.updated_at.cmp(&a.1.updated_at))
        });

        Ok((
            items.into_iter().take(8).map(|(_, item)| item).collect(),
            gaps.into_iter().take(4).map(|(_, gap)| gap).collect(),
        ))
    }

    fn constitution_file_path(&self, item_id: &str) -> PathBuf {
        self.constitution_path.join(format!("{}.json", item_id))
    }

    fn action_gap_file_path(&self, gap_id: &str) -> PathBuf {
        self.action_gaps_path.join(format!("{}.json", gap_id))
    }

    fn read_constitution_file(&self, path: &Path) -> Result<ConstitutionItem> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read constitution file: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse constitution file: {}", path.display()))
    }

    fn read_action_gap_file(&self, path: &Path) -> Result<ActionGap> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read action gap file: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse action gap file: {}", path.display()))
    }

    fn write_constitution_file(&self, item: &ConstitutionItem) -> Result<()> {
        let path = self.constitution_file_path(&item.id);
        self.write_pretty_json(&path, item)
    }

    fn write_action_gap_file(&self, gap: &ActionGap) -> Result<()> {
        let path = self.action_gap_file_path(&gap.id);
        self.write_pretty_json(&path, gap)
    }

    fn seed_constitution_setup_items(
        &self,
        setup: &ConstitutionSetup,
        evidence_ref: EvidenceRef,
    ) -> Result<()> {
        let existing_keys = self
            .list_constitution_items()?
            .into_iter()
            .map(|item| constitution_key(&item.dimension, &item.claim))
            .collect::<HashSet<_>>();
        let mut created_keys = existing_keys;

        let groups: &[(&str, &[String])] = &[
            ("values", &setup.values),
            ("taste", &setup.tastes),
            ("constraints", &setup.constraints),
            ("somatic", &setup.somatic_cues),
            ("action_tendency", &setup.action_tendencies),
        ];

        for (dimension, entries) in groups {
            for entry in entries.iter().filter(|entry| !entry.trim().is_empty()) {
                let claim = entry.trim().to_string();
                let key = constitution_key(dimension, &claim);
                if !created_keys.insert(key) {
                    continue;
                }
                self.create_constitution_item(ConstitutionItemCreate {
                    claim,
                    dimension: (*dimension).to_string(),
                    scope: vec!["setup".to_string()],
                    priority: 0.9,
                    confidence: 0.9,
                    status: ConstitutionStatus::Active,
                    evidence_refs: vec![evidence_ref.clone()],
                    tensions: Vec::new(),
                    linked_record_ids: Vec::new(),
                    source: Some("guided_setup".to_string()),
                })?;
            }
        }

        Ok(())
    }

    pub fn get_decision_mirror_config(&self) -> Result<DecisionMirrorConfig> {
        if !self.decision_mirror_config_path.exists() {
            return Ok(DecisionMirrorConfig::default());
        }

        let content =
            std::fs::read_to_string(&self.decision_mirror_config_path).with_context(|| {
                format!(
                    "Failed to read decision mirror config {}",
                    self.decision_mirror_config_path.display()
                )
            })?;
        let mut config: DecisionMirrorConfig =
            serde_json::from_str(&content).context("Failed to parse decision mirror config")?;
        config.weights = clamp_decision_mirror_weights(config.weights);
        Ok(config)
    }

    pub fn update_decision_mirror_config(
        &self,
        update: DecisionMirrorConfigUpdate,
    ) -> Result<DecisionMirrorConfig> {
        let mut config = self.get_decision_mirror_config()?;
        if let Some(preset) = update.preset {
            config.weights = DecisionMirrorWeights::for_preset(&preset);
            config.preset = preset;
        }
        if let Some(weights) = update.weights {
            config.weights = clamp_decision_mirror_weights(weights);
        }
        if let Some(advanced_enabled) = update.advanced_enabled {
            config.advanced_enabled = advanced_enabled;
        }

        self.write_decision_mirror_config_file(&config)?;
        Ok(config)
    }

    pub fn reset_decision_mirror_config(&self) -> Result<DecisionMirrorConfig> {
        let config = DecisionMirrorConfig::default();
        self.write_decision_mirror_config_file(&config)?;
        Ok(config)
    }

    pub fn record_decision_episode(
        &mut self,
        create: DecisionEpisodeCreate,
    ) -> Result<DecisionEpisode> {
        Self::validate_file_id(&create.id)?;
        Self::validate_file_id(&create.session_id)?;
        Self::validate_file_id(&create.tile_id)?;

        let now = Utc::now();
        // A prediction is only attempted when there are at least two options
        // to choose between; record why one will not arrive otherwise.
        let prediction_status = if create.options.len() >= 2 {
            Some("requested".to_string())
        } else {
            None
        };
        let episode = DecisionEpisode {
            id: create.id,
            session_id: create.session_id,
            tile_id: create.tile_id,
            decision: create.decision,
            options: create.options,
            stakes: create.stakes,
            initial_leaning: create.initial_leaning,
            selected_response: None,
            chosen_option: None,
            confidence: None,
            review_date: create.review_date,
            outcome: None,
            regret_score: None,
            lesson: None,
            missed_something: None,
            primitive_assessment: create.primitive_assessment,
            twin_prediction: None,
            prediction_status,
            agreement: None,
            correction_note: None,
            context_version: create.context_version,
            outcome_recorded_at: None,
            created_at: now,
            updated_at: now,
        };

        self.write_decision_file(&episode)?;
        self.append_trace_event(
            &episode.session_id,
            TraceEventType::DecisionEpisodeCreated,
            json!({
                "decision_episode_id": episode.id,
                "tile_id": episode.tile_id,
                "decision": episode.decision,
                "options": episode.options,
                "stakes": episode.stakes,
                "initial_leaning": episode.initial_leaning,
                "review_date": episode.review_date,
                "primitive_assessment": episode.primitive_assessment,
            }),
        )?;

        Ok(episode)
    }

    pub fn list_decision_episodes(&self) -> Result<Vec<DecisionEpisode>> {
        let mut episodes = Vec::new();
        if !self.decisions_path.exists() {
            return Ok(episodes);
        }

        for entry in WalkDir::new(&self.decisions_path)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Some(episode) =
                    load_or_quarantine::<DecisionEpisode>(path, "decision episode")
                {
                    episodes.push(episode);
                }
            }
        }

        episodes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(episodes)
    }

    /// Select past decided episodes as verbatim behavioral cases for twin
    /// context. Correction notes (recorded when a sealed prediction missed)
    /// break ties between equally relevant cases but never outrank a more
    /// relevant case.
    pub fn select_decision_cases(
        &self,
        query: &str,
        exclude_episode_id: Option<&str>,
        max: usize,
    ) -> Result<Vec<DecisionEpisode>> {
        if max == 0 {
            return Ok(Vec::new());
        }

        let query_terms = lexical_terms(query);
        let mut scored = Vec::new();
        let mut fallback = Vec::new();

        for episode in self.list_decision_episodes()? {
            if episode.chosen_option.is_none() {
                continue;
            }
            if exclude_episode_id.is_some_and(|id| id == episode.id) {
                continue;
            }
            let score = decision_case_relevance(&episode, &query_terms);
            if score > 0 {
                scored.push((score, episode));
            } else {
                fallback.push(episode);
            }
        }

        if scored.is_empty() {
            // No keyword overlap at all: include the most recent decided
            // cases rather than an empty behavioral context.
            fallback.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            fallback.truncate(max.min(2));
            return Ok(fallback);
        }

        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| {
                    b.1.correction_note
                        .is_some()
                        .cmp(&a.1.correction_note.is_some())
                })
                .then_with(|| b.1.updated_at.cmp(&a.1.updated_at))
        });

        Ok(scored
            .into_iter()
            .take(max)
            .map(|(_, episode)| episode)
            .collect())
    }

    pub fn update_decision_outcome(
        &mut self,
        id: &str,
        update: DecisionOutcomeUpdate,
    ) -> Result<DecisionEpisode> {
        Self::validate_file_id(id)?;
        let path = self.decision_file_path(id);
        let mut episode = self.read_decision_file(&path)?;

        if let Some(selected_response) = update.selected_response {
            episode.selected_response = Some(selected_response);
        }
        if let Some(chosen_option) = update.chosen_option {
            // Canonicalize label/case variants ("b", "Option 2", extra
            // whitespace) against the recorded options; unmatched free text
            // is kept as-is (legacy and "other" outcomes stay recordable).
            let canonical = match_option_index(&chosen_option, &episode.options)
                .map(|index| episode.options[index].clone())
                .unwrap_or(chosen_option);
            episode.chosen_option = Some(canonical);
        }
        if let Some(confidence) = update.confidence {
            episode.confidence = Some(confidence.clamp(0.0, 1.0));
        }
        if let Some(review_date) = update.review_date {
            episode.review_date = Some(review_date);
        }
        if let Some(outcome) = update.outcome {
            episode.outcome = Some(outcome);
        }
        if let Some(regret_score) = update.regret_score {
            episode.regret_score = Some(regret_score.min(10));
        }
        if let Some(lesson) = update.lesson {
            episode.lesson = Some(lesson);
        }
        if let Some(missed_something) = update.missed_something {
            episode.missed_something = Some(missed_something);
        }
        if let Some(primitive_assessment) = update.primitive_assessment {
            episode.primitive_assessment = primitive_assessment;
        }
        if let Some(correction_note) = update.correction_note {
            episode.correction_note = Some(correction_note);
        }

        if episode.outcome_recorded_at.is_none()
            && (episode.chosen_option.is_some() || episode.outcome.is_some())
        {
            episode.outcome_recorded_at = Some(Utc::now());
        }

        // Agreement is recomputed whenever both sides exist, so a corrected
        // chosen_option keeps the stored agreement current. Only predictions
        // sealed before the outcome was first recorded count.
        if let (Some(chosen), Some(prediction)) = (&episode.chosen_option, &episode.twin_prediction)
        {
            let recorded_at = episode.outcome_recorded_at.unwrap_or_else(Utc::now);
            if prediction.sealed_at <= recorded_at {
                episode.agreement = Some(compute_agreement(prediction, chosen, &episode.options));
            }
        }

        episode.updated_at = Utc::now();
        self.write_decision_file(&episode)?;
        self.append_trace_event(
            &episode.session_id,
            TraceEventType::OutcomeFollowUpRecorded,
            json!({
                "decision_episode_id": episode.id,
                "tile_id": episode.tile_id,
                "selected_response": episode.selected_response,
                "chosen_option": episode.chosen_option,
                "confidence": episode.confidence,
                "review_date": episode.review_date,
                "outcome": episode.outcome,
                "regret_score": episode.regret_score,
                "lesson": episode.lesson,
                "missed_something": episode.missed_something,
                "primitive_assessment": episode.primitive_assessment,
                "agreement": episode.agreement,
                "correction_note": episode.correction_note,
                "prediction_context_version": episode
                    .twin_prediction
                    .as_ref()
                    .map(|prediction| prediction.context_version.clone()),
            }),
        )?;

        Ok(episode)
    }

    /// Seal a twin prediction onto an episode. Refuses (as a logged no-op
    /// returning the unchanged episode) when the outcome is already recorded
    /// or a prediction already exists, so `sealed_at` always precedes the
    /// outcome structurally. The trace payload never contains the predicted
    /// option — the trace viewer must not leak a sealed prediction.
    pub fn attach_twin_prediction(
        &mut self,
        episode_id: &str,
        draft: TwinPredictionDraft,
        model_id: &str,
        context_version: &str,
    ) -> Result<DecisionEpisode> {
        Self::validate_file_id(episode_id)?;
        let path = self.decision_file_path(episode_id);
        let mut episode = self.read_decision_file(&path)?;

        if episode.chosen_option.is_some() {
            log::warn!(
                "Twin prediction for episode {} arrived after the outcome was recorded; discarding",
                episode_id
            );
            episode.prediction_status = Some("outcome_recorded_first".to_string());
            episode.updated_at = Utc::now();
            self.write_decision_file(&episode)?;
            return Ok(episode);
        }
        if episode.twin_prediction.is_some() {
            log::warn!(
                "Episode {} already has a sealed twin prediction; ignoring duplicate",
                episode_id
            );
            return Ok(episode);
        }

        let prediction = TwinPrediction {
            predicted_option: draft.predicted_option,
            matched_option_index: draft.matched_option_index,
            confidence: draft.confidence,
            rationale: draft.rationale,
            parse_mode: draft.parse_mode.clone(),
            model_id: model_id.to_string(),
            context_version: context_version.to_string(),
            sealed_at: Utc::now(),
        };
        let sealed_at = prediction.sealed_at;
        episode.twin_prediction = Some(prediction);
        episode.prediction_status = Some("sealed".to_string());
        episode.updated_at = Utc::now();
        self.write_decision_file(&episode)?;

        self.append_trace_event(
            &episode.session_id,
            TraceEventType::TwinPredictionSealed,
            json!({
                "decision_episode_id": episode.id,
                "tile_id": episode.tile_id,
                "model_id": model_id,
                "context_version": context_version,
                "parse_mode": draft.parse_mode,
                "sealed_at": sealed_at,
            }),
        )?;

        Ok(episode)
    }

    /// Record that the hidden prediction call failed, so exported episodes
    /// distinguish "no prediction because the call failed" from "agreed to
    /// not predict" — silent gaps would inflate measured accuracy.
    pub fn mark_twin_prediction_failed(&mut self, episode_id: &str) -> Result<()> {
        Self::validate_file_id(episode_id)?;
        let path = self.decision_file_path(episode_id);
        let mut episode = self.read_decision_file(&path)?;
        if episode.twin_prediction.is_some() || episode.chosen_option.is_some() {
            return Ok(());
        }
        episode.prediction_status = Some("failed".to_string());
        episode.updated_at = Utc::now();
        self.write_decision_file(&episode)
    }

    pub fn record_reflection_card(
        &mut self,
        create: ReflectionCardCreate,
    ) -> Result<ReflectionCard> {
        Self::validate_file_id(&create.decision_episode_id)?;
        Self::validate_file_id(&create.session_id)?;
        Self::validate_file_id(&create.tile_id)?;

        let config = self.get_decision_mirror_config()?;
        let mut evidence_packet = match create.evidence_packet.clone() {
            Some(packet) => packet,
            None => self.build_decision_evidence_packet(
                &create.cited_note_ids,
                &create.cited_user_record_ids,
                &create.cited_constitution_item_ids,
                &create.cited_action_gap_ids,
                &config,
            )?,
        };
        if evidence_packet.config_snapshot.is_none() {
            evidence_packet.config_snapshot = Some(config.clone());
        }
        let scores = score_reflection_card(
            &create.content,
            &create.cited_note_ids,
            &create.cited_user_record_ids,
            &create.cited_constitution_item_ids,
            &create.cited_action_gap_ids,
            &config,
        );
        let card = ReflectionCard {
            id: uuid::Uuid::new_v4().to_string(),
            decision_episode_id: create.decision_episode_id,
            session_id: create.session_id,
            tile_id: create.tile_id,
            model_id: create.model_id,
            content: create.content,
            cited_note_ids: create.cited_note_ids,
            cited_user_record_ids: create.cited_user_record_ids,
            cited_constitution_item_ids: create.cited_constitution_item_ids,
            cited_action_gap_ids: create.cited_action_gap_ids,
            scores,
            evidence_packet,
            created_at: Utc::now(),
        };

        self.write_reflection_file(&card)?;
        self.append_trace_event(
            &card.session_id,
            TraceEventType::ReflectionCardRecorded,
            json!({
                "reflection_card_id": card.id,
                "decision_episode_id": card.decision_episode_id,
                "tile_id": card.tile_id,
                "model_id": card.model_id,
                "cited_note_ids": card.cited_note_ids,
                "cited_user_record_ids": card.cited_user_record_ids,
                "cited_constitution_item_ids": card.cited_constitution_item_ids,
                "cited_action_gap_ids": card.cited_action_gap_ids,
                "scores": card.scores,
                "evidence_packet": card.evidence_packet,
            }),
        )?;

        Ok(card)
    }

    fn build_decision_evidence_packet(
        &mut self,
        cited_note_ids: &[String],
        cited_user_record_ids: &[String],
        cited_constitution_item_ids: &[String],
        cited_action_gap_ids: &[String],
        config: &DecisionMirrorConfig,
    ) -> Result<DecisionEvidencePacket> {
        self.ensure_record_cache()?;
        let weights = &config.weights;
        let mut selected_sources = Vec::new();

        for id in cited_note_ids {
            selected_sources.push(DecisionEvidenceSource {
                source_type: "note".to_string(),
                id: id.clone(),
                label: format!("Note {}", id),
                weight: weights.notes_weight,
                reason: "Selected by vault retrieval for this decision".to_string(),
            });
        }

        for id in cited_user_record_ids {
            if let Ok(record) = self.get_user_record(id) {
                let (source_type, weight) = match &record.promotion_state {
                    PromotionState::Endorsed | PromotionState::AutoPromoted => {
                        ("approved_record", weights.approved_records_weight)
                    }
                    PromotionState::Candidate => {
                        ("candidate_record", weights.candidate_records_weight)
                    }
                    PromotionState::Rejected
                    | PromotionState::Private
                    | PromotionState::NoTrain => {
                        continue;
                    }
                };
                selected_sources.push(DecisionEvidenceSource {
                    source_type: source_type.to_string(),
                    id: record.id,
                    label: excerpt(&record.content),
                    weight,
                    reason: format!(
                        "{} user record selected for live twin context",
                        promotion_state_label(&record.promotion_state)
                    ),
                });
            }
        }

        for id in cited_constitution_item_ids {
            let label = self
                .read_constitution_file(&self.constitution_file_path(id))
                .ok()
                .map(|item| excerpt(&item.claim))
                .unwrap_or_else(|| format!("Constitution {}", id));
            selected_sources.push(DecisionEvidenceSource {
                source_type: "constitution_item".to_string(),
                id: id.clone(),
                label,
                weight: weights.constitution_weight,
                reason: "Higher-order constitution item selected for decision framing".to_string(),
            });
        }

        for id in cited_action_gap_ids {
            let label = self
                .read_action_gap_file(&self.action_gap_file_path(id))
                .ok()
                .map(|gap| excerpt(&gap.decision_risk))
                .unwrap_or_else(|| format!("Action Gap {}", id));
            selected_sources.push(DecisionEvidenceSource {
                source_type: "action_gap".to_string(),
                id: id.clone(),
                label,
                weight: weights.action_gaps_weight,
                reason: "Action gap selected as decision risk context".to_string(),
            });
        }

        let mut excluded_private_count = self
            .record_cache
            .values()
            .filter(|record| record.promotion_state == PromotionState::Private)
            .count();
        let mut excluded_rejected_count = self
            .record_cache
            .values()
            .filter(|record| record.promotion_state == PromotionState::Rejected)
            .count();
        let mut excluded_no_train_count = self
            .record_cache
            .values()
            .filter(|record| record.promotion_state == PromotionState::NoTrain)
            .count();

        for item in self.list_constitution_items()? {
            match item.status {
                ConstitutionStatus::Private => excluded_private_count += 1,
                ConstitutionStatus::Rejected | ConstitutionStatus::NotMe => {
                    excluded_rejected_count += 1
                }
                ConstitutionStatus::NoTrain => excluded_no_train_count += 1,
                ConstitutionStatus::Candidate
                | ConstitutionStatus::Active
                | ConstitutionStatus::Softened => {}
            }
        }
        for gap in self.list_action_gaps()? {
            match gap.status {
                ConstitutionStatus::Private => excluded_private_count += 1,
                ConstitutionStatus::Rejected | ConstitutionStatus::NotMe => {
                    excluded_rejected_count += 1
                }
                ConstitutionStatus::NoTrain => excluded_no_train_count += 1,
                ConstitutionStatus::Candidate
                | ConstitutionStatus::Active
                | ConstitutionStatus::Softened => {}
            }
        }

        Ok(DecisionEvidencePacket {
            selected_sources,
            excluded_private_count,
            excluded_rejected_count,
            excluded_no_train_count,
            created_at: Some(Utc::now()),
            config_snapshot: Some(config.clone()),
        })
    }

    pub fn list_decision_episodes_with_reflections(
        &self,
    ) -> Result<Vec<DecisionEpisodeWithReflections>> {
        let cards = self.list_reflection_cards()?;
        let mut cards_by_episode: HashMap<String, Vec<ReflectionCard>> = HashMap::new();
        for card in cards {
            cards_by_episode
                .entry(card.decision_episode_id.clone())
                .or_default()
                .push(card);
        }

        let mut episodes = self
            .list_decision_episodes()?
            .into_iter()
            .map(|mut episode| {
                let mut reflection_cards = cards_by_episode.remove(&episode.id).unwrap_or_default();
                reflection_cards.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                let feedback_events = self.decision_feedback_events(&episode).unwrap_or_default();
                // A sealed prediction must never cross IPC before the outcome
                // is recorded; the UI only learns that one exists.
                let prediction_sealed =
                    episode.twin_prediction.is_some() && episode.chosen_option.is_none();
                if prediction_sealed {
                    episode.twin_prediction = None;
                }
                DecisionEpisodeWithReflections {
                    episode,
                    reflection_cards,
                    feedback_events,
                    prediction_sealed,
                }
            })
            .collect::<Vec<_>>();
        episodes.sort_by(|a, b| b.episode.updated_at.cmp(&a.episode.updated_at));
        Ok(episodes)
    }

    fn decision_file_path(&self, decision_id: &str) -> PathBuf {
        self.decisions_path.join(format!("{}.json", decision_id))
    }

    fn reflection_file_path(&self, reflection_id: &str) -> PathBuf {
        self.reflections_path
            .join(format!("{}.json", reflection_id))
    }

    fn read_decision_file(&self, path: &Path) -> Result<DecisionEpisode> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read decision file: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse decision file: {}", path.display()))
    }

    fn write_decision_mirror_config_file(&self, config: &DecisionMirrorConfig) -> Result<()> {
        self.write_pretty_json(&self.decision_mirror_config_path, config)
    }

    fn write_decision_file(&self, episode: &DecisionEpisode) -> Result<()> {
        let path = self.decision_file_path(&episode.id);
        self.write_pretty_json(&path, episode)
    }

    fn write_reflection_file(&self, card: &ReflectionCard) -> Result<()> {
        let path = self.reflection_file_path(&card.id);
        self.write_pretty_json(&path, card)
    }

    pub(super) fn list_reflection_cards(&self) -> Result<Vec<ReflectionCard>> {
        let mut cards = Vec::new();
        if !self.reflections_path.exists() {
            return Ok(cards);
        }

        for entry in WalkDir::new(&self.reflections_path)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Some(card) = load_or_quarantine::<ReflectionCard>(path, "reflection card") {
                    cards.push(card);
                }
            }
        }

        cards.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(cards)
    }

    pub(super) fn decision_evidence_refs(&self, decision_id: &str) -> Result<Vec<EvidenceRef>> {
        Self::validate_file_id(decision_id)?;
        let mut refs = Vec::new();
        for entry in WalkDir::new(&self.traces_path)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if !path.extension().is_some_and(|ext| ext == "json") {
                continue;
            }
            let Some(trace) = load_or_quarantine::<SessionTrace>(path, "trace") else {
                // A corrupt trace only omits its own evidence refs; the
                // decision's evidence from healthy traces is still collected.
                continue;
            };
            for event in trace.events {
                let event_decision_id =
                    payload_string(&event.payload, &["decision_episode_id"]).unwrap_or_default();
                if event_decision_id != decision_id {
                    continue;
                }
                refs.push(EvidenceRef {
                    trace_id: trace.id.clone(),
                    event_id: event.id.clone(),
                    session_id: trace.session_id.clone(),
                    tile_id: extract_event_tile_id(&event.payload),
                    model_id: extract_event_model_id(&event.payload),
                    note: Some("Decision episode evidence".to_string()),
                    source_type: Some("decision".to_string()),
                    source_id: Some(decision_id.to_string()),
                    source_label: Some("Decision episode evidence".to_string()),
                    excerpt: None,
                    speaker_role: None,
                });
            }
        }
        refs.sort_by(|a, b| a.event_id.cmp(&b.event_id));
        Ok(refs)
    }

    fn decision_feedback_events(&self, episode: &DecisionEpisode) -> Result<Vec<TraceEvent>> {
        Self::validate_file_id(&episode.session_id)?;
        let path = self.trace_file_path(&episode.session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }

        let trace = self.read_trace_file(&path)?;
        let mut events = trace
            .events
            .into_iter()
            .filter(|event| {
                matches!(
                    event.event_type,
                    TraceEventType::FeedbackRecorded
                        | TraceEventType::RankingRecorded
                        | TraceEventType::InsightCaptured
                )
            })
            .filter(|event| {
                payload_string(&event.payload, &["decision_episode_id"])
                    .is_some_and(|id| id == episode.id)
                    || extract_event_tile_id(&event.payload)
                        .is_some_and(|tile_id| tile_id == episode.tile_id)
            })
            .collect::<Vec<_>>();
        events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(events)
    }

    pub fn list_memory_digest(&mut self) -> Result<Vec<MemoryDigestItem>> {
        self.ensure_record_cache()?;
        let mut existing = self.read_memory_digest_file()?;
        let mut existing_by_id = existing
            .iter()
            .cloned()
            .map(|item| (item.id.clone(), item))
            .collect::<HashMap<_, _>>();
        let now = Utc::now();
        let records = self
            .record_cache
            .values()
            .filter(|record| memory_digest_trigger(record).is_some())
            .cloned()
            .collect::<Vec<_>>();
        let mut clusters: HashMap<String, Vec<UserRecord>> = HashMap::new();

        for record in records {
            clusters
                .entry(memory_digest_cluster_key(&record))
                .or_default()
                .push(record);
        }

        for mut records in clusters.into_values() {
            records.sort_by(|a, b| {
                b.evidence_refs
                    .len()
                    .cmp(&a.evidence_refs.len())
                    .then_with(|| {
                        b.confidence
                            .partial_cmp(&a.confidence)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| b.updated_at.cmp(&a.updated_at))
            });
            let primary = match records.first() {
                Some(record) => record,
                None => continue,
            };
            let item_id = stable_digest_id(&records);
            if existing_by_id.contains_key(&item_id) {
                continue;
            }

            let record_ids = records
                .iter()
                .map(|record| record.id.clone())
                .collect::<Vec<_>>();
            let evidence_count = records
                .iter()
                .map(|record| record.evidence_refs.len())
                .sum::<usize>();
            let confidence = records
                .iter()
                .map(|record| record.confidence)
                .fold(0.0_f32, f32::max);
            let trigger_reason = if records.len() > 1 {
                format!("{} related patterns clustered for review", records.len())
            } else {
                memory_digest_trigger(primary)
                    .unwrap_or("pattern needs review")
                    .to_string()
            };
            let latest_evidence = self
                .resolve_evidence_refs(&primary.evidence_refs)
                .ok()
                .and_then(|mut refs| refs.drain(..).next());
            let item = MemoryDigestItem {
                id: item_id.clone(),
                pattern: primary.content.clone(),
                evidence_count,
                confidence,
                trigger_reason,
                latest_evidence,
                record_ids,
                state: MemoryDigestState::Pending,
                created_at: now,
                updated_at: now,
            };
            existing_by_id.insert(item_id, item);
        }

        existing = existing_by_id.into_values().collect();
        existing.sort_by(|a, b| {
            digest_state_sort_key(&a.state)
                .cmp(&digest_state_sort_key(&b.state))
                .then_with(|| b.evidence_count.cmp(&a.evidence_count))
                .then_with(|| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        self.write_memory_digest_file(&existing)?;

        Ok(existing
            .into_iter()
            .filter(|item| item.state == MemoryDigestState::Pending)
            .take(MAX_MEMORY_DIGEST_ITEMS)
            .collect())
    }

    pub fn review_memory_digest_item(
        &mut self,
        id: &str,
        request: MemoryDigestReviewRequest,
    ) -> Result<MemoryDigestItem> {
        Self::validate_file_id(id)?;
        let mut items = self.read_memory_digest_file()?;
        if !items.iter().any(|item| item.id == id) {
            let _ = self.list_memory_digest()?;
            items = self.read_memory_digest_file()?;
        }

        let item_index = items
            .iter()
            .position(|item| item.id == id)
            .ok_or_else(|| anyhow::anyhow!("Memory digest item not found: {}", id))?;
        let mut item = items[item_index].clone();
        item.state = memory_digest_state_for_action(&request.action);
        item.updated_at = Utc::now();
        items[item_index] = item.clone();
        self.write_memory_digest_file(&items)?;

        for record_id in &item.record_ids {
            if self.get_user_record(record_id).is_err() {
                continue;
            }

            let promotion_state = match request.action {
                MemoryDigestAction::Keep => Some(PromotionState::Endorsed),
                MemoryDigestAction::Soften => Some(PromotionState::Candidate),
                MemoryDigestAction::NotMe | MemoryDigestAction::Reject => {
                    Some(PromotionState::Rejected)
                }
                MemoryDigestAction::Private => Some(PromotionState::Private),
                MemoryDigestAction::NoTrain => Some(PromotionState::NoTrain),
            };

            if let Some(state) = promotion_state {
                let _ = self.set_user_record_promotion(
                    record_id,
                    state,
                    request
                        .rationale
                        .clone()
                        .or_else(|| Some(format!("Memory digest action: {:?}", request.action))),
                );
            }

            if request.action == MemoryDigestAction::Soften {
                if let Ok(record) = self.get_user_record(record_id) {
                    let _ = self.update_user_record(
                        record_id,
                        UserRecordUpdate {
                            confidence: Some((record.confidence * 0.85).max(0.35)),
                            ..UserRecordUpdate::default()
                        },
                    );
                }
            }
        }

        Ok(item)
    }

    fn read_memory_digest_file(&self) -> Result<Vec<MemoryDigestItem>> {
        if !self.digest_path.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&self.digest_path).with_context(|| {
            format!(
                "Failed to read memory digest file: {}",
                self.digest_path.display()
            )
        })?;
        serde_json::from_str(&content).with_context(|| {
            format!(
                "Failed to parse memory digest file: {}",
                self.digest_path.display()
            )
        })
    }

    fn write_memory_digest_file(&self, items: &[MemoryDigestItem]) -> Result<()> {
        self.write_pretty_json(&self.digest_path, &items)
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
        let benchmark_path = output_dir.join("decision_mirror_benchmark.jsonl");
        let constitution_path = output_dir.join("constitution_items.jsonl");
        let action_gaps_path = output_dir.join("action_gaps.jsonl");
        let decision_episodes_path = output_dir.join("decision_episodes.jsonl");
        let feedback_events_path = output_dir.join("feedback_events.jsonl");
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
        let decision_mirror_config = self.get_decision_mirror_config()?;
        let benchmark_lines = self
            .list_decision_episodes_with_reflections()?
            .into_iter()
            .map(|entry| {
                serde_json::to_string(&json!({
                    "variant": "decision_mirror",
                    "baseline_variants": [
                        "generic_llm",
                        "persona_prompt",
                        "vault_only_rag",
                        "twin_rag",
                        "decision_mirror"
                    ],
                    "config": decision_mirror_config.clone(),
                    "episode": entry.episode,
                    "reflection_cards": entry.reflection_cards,
                }))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.write_jsonl_file(&benchmark_path, &benchmark_lines)?;
        let constitution_lines = self
            .list_constitution_items()?
            .into_iter()
            .map(|item| serde_json::to_string(&item))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.write_jsonl_file(&constitution_path, &constitution_lines)?;
        let action_gap_lines = self
            .list_action_gaps()?
            .into_iter()
            .map(|gap| serde_json::to_string(&gap))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.write_jsonl_file(&action_gaps_path, &action_gap_lines)?;

        // Decision episodes with sealed-prediction integrity: while an
        // episode has no recorded choice, its prediction exports as a
        // non-revealing stub so the bet cannot leak through an early export.
        let decision_episode_lines = self
            .list_decision_episodes()?
            .into_iter()
            .map(|episode| {
                let still_sealed =
                    episode.twin_prediction.is_some() && episode.chosen_option.is_none();
                let mut value = serde_json::to_value(&episode)?;
                if still_sealed {
                    if let Some(object) = value.as_object_mut() {
                        let stub = episode.twin_prediction.as_ref().map(|prediction| {
                            serde_json::json!({
                                "sealed": true,
                                "sealed_at": prediction.sealed_at,
                                "model_id": prediction.model_id,
                                "context_version": prediction.context_version,
                                "parse_mode": prediction.parse_mode,
                            })
                        });
                        object.insert("twin_prediction".to_string(), stub.unwrap_or(Value::Null));
                    }
                }
                serde_json::to_string(&value).map_err(anyhow::Error::from)
            })
            .collect::<Result<Vec<_>>>()?;
        self.write_jsonl_file(&decision_episodes_path, &decision_episode_lines)?;

        // Ranking / Matches-Me / insight raw material. Privacy rule: skip
        // events that are evidence for records now marked Rejected, Private,
        // or NoTrain.
        let excluded_event_ids: HashSet<String> = self
            .record_cache
            .values()
            .filter(|record| {
                matches!(
                    record.promotion_state,
                    PromotionState::Rejected | PromotionState::Private | PromotionState::NoTrain
                )
            })
            .flat_map(|record| {
                record
                    .evidence_refs
                    .iter()
                    .map(|evidence| evidence.event_id.clone())
            })
            .collect();
        let feedback_event_lines = self
            .list_session_traces()?
            .into_iter()
            .flat_map(|trace| {
                let session_id = trace.session_id.clone();
                let trace_id = trace.id.clone();
                trace
                    .events
                    .into_iter()
                    .filter(|event| {
                        matches!(
                            event.event_type,
                            TraceEventType::FeedbackRecorded
                                | TraceEventType::RankingRecorded
                                | TraceEventType::InsightCaptured
                        ) && !excluded_event_ids.contains(&event.id)
                    })
                    .map(move |event| {
                        serde_json::to_string(&serde_json::json!({
                            "session_id": session_id,
                            "trace_id": trace_id,
                            "event": event,
                        }))
                        .map_err(anyhow::Error::from)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Result<Vec<_>>>()?;
        self.write_jsonl_file(&feedback_events_path, &feedback_event_lines)?;

        let manifest = serde_json::json!({
            "bundle_schema_version": 2,
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
                "decision_mirror_benchmark": {
                    "path": benchmark_path.display().to_string(),
                    "count": benchmark_lines.len(),
                },
                "constitution_items": {
                    "path": constitution_path.display().to_string(),
                    "count": constitution_lines.len(),
                },
                "action_gaps": {
                    "path": action_gaps_path.display().to_string(),
                    "count": action_gap_lines.len(),
                },
                "decision_episodes": {
                    "path": decision_episodes_path.display().to_string(),
                    "count": decision_episode_lines.len(),
                },
                "feedback_events": {
                    "path": feedback_events_path.display().to_string(),
                    "count": feedback_event_lines.len(),
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
            decision_mirror_benchmark: ExportFileSummary {
                path: benchmark_path.display().to_string(),
                count: benchmark_lines.len(),
            },
            constitution_items: ExportFileSummary {
                path: constitution_path.display().to_string(),
                count: constitution_lines.len(),
            },
            action_gaps: ExportFileSummary {
                path: action_gaps_path.display().to_string(),
                count: action_gap_lines.len(),
            },
            decision_episodes: ExportFileSummary {
                path: decision_episodes_path.display().to_string(),
                count: decision_episode_lines.len(),
            },
            feedback_events: ExportFileSummary {
                path: feedback_events_path.display().to_string(),
                count: feedback_event_lines.len(),
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

    fn write_jsonl_file(&self, path: &Path, lines: &[String]) -> Result<()> {
        let mut content = lines.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        write_atomic(path, content.as_bytes())
            .with_context(|| format!("Failed to write JSONL file: {}", path.display()))
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

    use crate::models::twin::ConstitutionStatus;
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

    fn test_note(id: &str, title: &str, content: &str, source_type: Option<&str>) -> Note {
        let now = Utc::now();
        let mut properties = HashMap::new();
        if let Some(source_type) = source_type {
            properties.insert(
                "source_type".to_string(),
                Value::String(source_type.to_string()),
            );
        }
        Note {
            id: id.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            relative_path: format!("{}.md", id),
            aliases: Vec::new(),
            status: NoteStatus::Evidence,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            wikilinks: Vec::new(),
            parsed_links: Vec::new(),
            properties,
            ..Default::default()
        }
    }

    #[test]
    fn constitution_inference_uses_repeated_behavior_as_primary_evidence() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        for index in 0..3 {
            store
                .append_trace_event(
                    "session-1",
                    TraceEventType::PromptSubmitted,
                    serde_json::json!({
                        "tile_id": format!("tile-{}", index),
                        "prompt": "Please implement this with exact files, commands, and tests.",
                        "models": ["openai/gpt-4o"],
                    }),
                )
                .expect("trace event should append");
        }

        store
            .run_twin_inference()
            .expect("record inference should run");
        let summary = store
            .run_constitution_inference_with_notes(&[])
            .expect("constitution inference should run");
        let items = store
            .list_constitution_items()
            .expect("constitution should list");
        let item = items
            .iter()
            .find(|item| item.claim.contains("concrete implementation details"))
            .expect("behavior-derived constitution item should exist");

        assert_eq!(summary.scanned_behavior_events, 3);
        assert_eq!(summary.auto_active_items, 1);
        assert_eq!(item.status, ConstitutionStatus::Active);
        assert_eq!(item.source.as_deref(), Some("behavior_inference"));
        assert!(item
            .evidence_refs
            .iter()
            .all(|evidence| evidence.source_type.as_deref() == Some("behavior")));
    }

    #[test]
    fn interviewee_answers_become_research_findings_not_personal_constitution() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let notes = vec![test_note(
            "interview-1",
            "Interview: onboarding",
            "### Message 1: User\n\nHow do you decide whether an AI workflow is useful?\n\n### Message 2: Interviewee\n\nI need to see a working demo before I trust the system.\n\n### Message 3: User\n\nCan you give a concrete example and compare it with your current workflow?",
            Some("interview"),
        )];

        let summary = store
            .run_constitution_inference_with_notes(&notes)
            .expect("constitution inference should run");
        let items = store
            .list_constitution_items()
            .expect("constitution should list");
        let records = store.list_user_records().expect("records should list");

        assert_eq!(summary.scanned_interviews, 1);
        assert_eq!(summary.extracted_research_findings, 1);
        assert!(items
            .iter()
            .any(|item| item.claim.contains("concrete examples")));
        assert!(!items
            .iter()
            .any(|item| item.claim.contains("working demo before I trust")));
        assert!(records.iter().any(|record| {
            record.kind == UserRecordKind::Fact
                && record.content.contains("working demo before I trust")
                && record.metadata.get("source_type").and_then(Value::as_str)
                    == Some("interview_answer")
        }));
    }

    #[test]
    fn unlabeled_interview_notes_import_but_do_not_extract() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let notes = vec![test_note(
            "interview-2",
            "Unlabeled interview",
            "How do you decide whether an AI workflow is useful?\nI need a working demo.",
            Some("interview"),
        )];

        let summary = store
            .run_constitution_inference_with_notes(&notes)
            .expect("constitution inference should run");

        assert_eq!(summary.scanned_interviews, 1);
        assert_eq!(summary.extracted_research_findings, 0);
        assert_eq!(summary.skipped_domain_claims, 1);
        assert!(store
            .list_constitution_items()
            .expect("constitution should list")
            .is_empty());
    }

    #[test]
    fn constitution_inference_prunes_stale_vault_derived_items_and_records() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        store
            .create_constitution_item(ConstitutionItemCreate {
                claim: "Old note-backed claim".to_string(),
                dimension: "reasoning".to_string(),
                scope: vec!["note".to_string()],
                priority: 0.58,
                confidence: 0.58,
                status: ConstitutionStatus::Candidate,
                evidence_refs: vec![EvidenceRef {
                    trace_id: "note-old-note".to_string(),
                    event_id: "old-note".to_string(),
                    session_id: "vault-note".to_string(),
                    tile_id: None,
                    model_id: None,
                    note: Some("Old note".to_string()),
                    source_type: Some("note".to_string()),
                    source_id: Some("old-note".to_string()),
                    source_label: Some("Vault note".to_string()),
                    excerpt: Some("old evidence".to_string()),
                    speaker_role: Some("user".to_string()),
                }],
                tensions: Vec::new(),
                linked_record_ids: Vec::new(),
                source: Some("note_inference".to_string()),
            })
            .expect("old constitution item should be created");
        store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Fact,
                content: "Interview finding: old vault finding".to_string(),
                origin: RecordOrigin::Inferred,
                evidence_refs: Vec::new(),
                confidence: 0.66,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::from([
                    ("source_type".to_string(), json!("interview_answer")),
                    ("source_note_id".to_string(), json!("old-note")),
                ]),
            })
            .expect("old record should be created");
        store
            .create_constitution_item(ConstitutionItemCreate {
                claim: "Old guided setup claim".to_string(),
                dimension: "values".to_string(),
                scope: vec!["setup".to_string()],
                priority: 0.9,
                confidence: 0.9,
                status: ConstitutionStatus::Active,
                evidence_refs: Vec::new(),
                tensions: Vec::new(),
                linked_record_ids: Vec::new(),
                source: Some("guided_setup".to_string()),
            })
            .expect("old setup constitution item should be created");

        let current_notes = vec![test_note(
            "current-interview",
            "Current interview",
            "### Message 1: User\n\nCan you give a concrete example of how you make tradeoffs?\n\n### Message 2: Interviewee\n\nI compare impact and risk before deciding.",
            Some("interview"),
        )];
        let summary = store
            .run_constitution_inference_with_notes(&current_notes)
            .expect("constitution inference should run");

        assert_eq!(summary.pruned_stale_constitution_items, 2);
        assert_eq!(summary.pruned_stale_records, 1);
        assert!(store
            .list_constitution_items()
            .expect("constitution should list")
            .iter()
            .all(|item| !item.claim.contains("Old note-backed claim")
                && !item.claim.contains("Old guided setup claim")));
        assert!(store
            .list_user_records()
            .expect("records should list")
            .iter()
            .all(|record| !record.content.contains("old vault finding")));
    }

    #[test]
    fn constitution_inference_rewrites_setup_from_current_interview_questions() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let notes = vec![test_note(
            "interview-setup",
            "Interview: strategy",
            "### Message 1: User\n\nWhat are some personal values or cultural values that drive your decisions?\n\n### Message 2: Interviewee\n\nI want work to make something better and different.\n\n### Message 3: User\n\nCan you walk us through a concrete example and how you balance innovation and stability?",
            Some("interview"),
        )];

        let summary = store
            .run_constitution_inference_with_notes(&notes)
            .expect("constitution inference should run");
        let setup = store
            .get_constitution_setup()
            .expect("setup should load after inference");

        assert!(summary.updated_setup_entries >= 4);
        assert!(setup
            .values
            .iter()
            .any(|entry| entry.contains("values and cultural assumptions")));
        assert!(setup
            .tastes
            .iter()
            .any(|entry| entry.contains("concrete walkthroughs")));
        assert!(setup
            .constraints
            .iter()
            .any(|entry| entry.contains("interviewee answers as research evidence")));
        assert!(setup
            .action_tendencies
            .iter()
            .any(|entry| entry.contains("follow-up questions")));
    }

    #[test]
    fn constitution_setup_accepts_legacy_json_without_identity() {
        let setup: ConstitutionSetup = serde_json::from_str(
            r#"{
                "values": ["evidence-backed work"],
                "tastes": ["clean UX"],
                "constraints": [],
                "somatic_cues": [],
                "action_tendencies": []
            }"#,
        )
        .expect("legacy setup should parse");

        assert_eq!(setup.twin_name, None);
        assert_eq!(setup.twin_role, None);
        assert!(setup.source_boundaries.is_empty());
        assert_eq!(setup.values, vec!["evidence-backed work"]);
    }

    #[test]
    fn save_constitution_setup_trims_identity_fields() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let saved = store
            .save_constitution_setup(ConstitutionSetup {
                twin_name: Some("  Alex Chen  ".to_string()),
                twin_role: Some("  founder deciding from product evidence  ".to_string()),
                source_boundaries: vec![
                    "  Use reviewed notes only.  ".to_string(),
                    "".to_string(),
                    " Uploaded interviews define domain context. ".to_string(),
                ],
                values: vec![" evidence-backed work ".to_string()],
                ..ConstitutionSetup::default()
            })
            .expect("setup should save");

        assert_eq!(saved.twin_name.as_deref(), Some("Alex Chen"));
        assert_eq!(
            saved.twin_role.as_deref(),
            Some("founder deciding from product evidence")
        );
        assert_eq!(
            saved.source_boundaries,
            vec![
                "Use reviewed notes only.".to_string(),
                "Uploaded interviews define domain context.".to_string()
            ]
        );
        assert_eq!(saved.values, vec!["evidence-backed work"]);
    }

    #[test]
    fn constitution_inference_preserves_configured_identity() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        store
            .save_constitution_setup(ConstitutionSetup {
                twin_name: Some("Alex Chen".to_string()),
                twin_role: Some("founder deciding from product evidence".to_string()),
                source_boundaries: vec!["Use reviewed notes only.".to_string()],
                values: vec!["evidence-backed work".to_string()],
                ..ConstitutionSetup::default()
            })
            .expect("identity setup should save");

        let notes = vec![test_note(
            "interview-setup",
            "Interview: strategy",
            "### Message 1: User\n\nCan you walk us through a concrete example and how you balance innovation and stability?",
            Some("interview"),
        )];
        store
            .run_constitution_inference_with_notes(&notes)
            .expect("constitution inference should run");
        let setup = store
            .get_constitution_setup()
            .expect("setup should load after inference");

        assert_eq!(setup.twin_name.as_deref(), Some("Alex Chen"));
        assert_eq!(
            setup.twin_role.as_deref(),
            Some("founder deciding from product evidence")
        );
        assert_eq!(
            setup.source_boundaries,
            vec!["Use reviewed notes only.".to_string()]
        );
    }

    #[test]
    fn decision_episode_and_reflection_card_persist_with_scores() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-1".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-1".to_string(),
                decision: "Should Grafyn build Decision Mirror first?".to_string(),
                options: vec!["Decision Mirror".to_string(), "Topology".to_string()],
                stakes: Some("Product direction".to_string()),
                initial_leaning: Some("Decision Mirror".to_string()),
                review_date: Some("2026-05-15".to_string()),
                primitive_assessment: PrimitiveDecisionAssessment {
                    stakes: Some("high".to_string()),
                    reversibility: Some("medium".to_string()),
                    time_horizon: Some("weeks".to_string()),
                    uncertainty: Some("medium".to_string()),
                    agency: Some("high".to_string()),
                    value_tension: Some("ambition vs proof".to_string()),
                    constraint_pressure: None,
                    taste_aesthetic_pull: None,
                    somatic_signal: None,
                    action_gap_risk: None,
                    outcome_feedback: None,
                },
                context_version: None,
            })
            .expect("decision episode should persist");

        let card = store
            .record_reflection_card(ReflectionCardCreate {
                decision_episode_id: episode.id.clone(),
                session_id: episode.session_id.clone(),
                tile_id: episode.tile_id.clone(),
                model_id: "openai/gpt-4".to_string(),
                content: [
                    "## Decision Frame",
                    "You seem pulled toward ambitious topology.",
                    "## Likely Reasoning Pattern",
                    "You tend to prefer large architecture.",
                    "## Recommendation",
                    "Build the smaller proof first.",
                ]
                .join("\n"),
                cited_note_ids: Vec::new(),
                cited_user_record_ids: Vec::new(),
                cited_constitution_item_ids: Vec::new(),
                cited_action_gap_ids: Vec::new(),
                evidence_packet: None,
            })
            .expect("reflection card should persist");

        assert_eq!(card.decision_episode_id, "decision-1");
        assert!(card.scores.unsupported_claim_count > 0);
        assert!(card.scores.overall_score <= 1.0);
        assert_eq!(card.evidence_packet.selected_sources.len(), 0);
        assert_eq!(
            card.evidence_packet
                .config_snapshot
                .as_ref()
                .expect("config snapshot should persist")
                .preset,
            DecisionMirrorPreset::Balanced
        );
        store
            .append_trace_event(
                &episode.session_id,
                TraceEventType::FeedbackRecorded,
                json!({
                    "feedback_type": "reject",
                    "response": {
                        "tile_id": episode.tile_id.clone(),
                        "model_id": "openai/gpt-4",
                    },
                    "rationale": "Decision Mirror reflection marked Not Me",
                }),
            )
            .expect("feedback event should persist");

        let decision_rows = store
            .list_decision_episodes_with_reflections()
            .expect("decision rows should list with traces");
        assert_eq!(decision_rows[0].reflection_cards.len(), 1);
        assert_eq!(decision_rows[0].feedback_events.len(), 1);
        assert_eq!(
            decision_rows[0].feedback_events[0]
                .payload
                .get("feedback_type")
                .and_then(|value| value.as_str()),
            Some("reject")
        );
        let episodes = store
            .list_decision_episodes()
            .expect("episodes should list");
        assert_eq!(episodes.len(), 1);

        let legacy_card: ReflectionCard = serde_json::from_str(
            r#"{
                "id": "legacy-card",
                "decision_episode_id": "decision-1",
                "session_id": "session-1",
                "tile_id": "tile-1",
                "model_id": "openai/gpt-4",
                "content": "legacy reflection",
                "scores": { "overall_score": 0.5 },
                "created_at": "2026-05-08T00:00:00Z"
            }"#,
        )
        .expect("legacy reflection cards should deserialize");
        assert!(legacy_card.evidence_packet.config_snapshot.is_none());
    }

    #[test]
    fn decision_mirror_config_presets_persist() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let store = TwinStore::new(temp_dir.path().to_path_buf());

        let default_config = store
            .get_decision_mirror_config()
            .expect("default config should load");
        assert_eq!(default_config.preset, DecisionMirrorPreset::Balanced);

        let updated = store
            .update_decision_mirror_config(DecisionMirrorConfigUpdate {
                preset: Some(DecisionMirrorPreset::EvidenceStrict),
                weights: None,
                advanced_enabled: Some(true),
            })
            .expect("config should update");
        assert_eq!(updated.preset, DecisionMirrorPreset::EvidenceStrict);
        assert!(
            updated.weights.evidence_grounding_weight
                > default_config.weights.evidence_grounding_weight
        );

        let persisted = store
            .get_decision_mirror_config()
            .expect("persisted config should load");
        assert_eq!(persisted.preset, DecisionMirrorPreset::EvidenceStrict);

        let reset = store
            .reset_decision_mirror_config()
            .expect("config should reset");
        assert_eq!(reset.preset, DecisionMirrorPreset::Balanced);
    }

    #[test]
    fn decision_episode_old_json_loads_with_default_prediction_fields() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let store = TwinStore::new(temp_dir.path().to_path_buf());

        let old_json = r#"{
            "id": "legacy-episode",
            "session_id": "session-1",
            "tile_id": "tile-1",
            "decision": "Ship now or wait?",
            "options": ["Ship now", "Wait"],
            "chosen_option": "Ship now",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }"#;
        std::fs::create_dir_all(&store.decisions_path).expect("decisions dir");
        std::fs::write(store.decisions_path.join("legacy-episode.json"), old_json)
            .expect("legacy episode should write");

        let episodes = store
            .list_decision_episodes()
            .expect("legacy episode should deserialize");
        assert_eq!(episodes.len(), 1);
        let episode = &episodes[0];
        assert!(episode.twin_prediction.is_none());
        assert!(episode.prediction_status.is_none());
        assert!(episode.agreement.is_none());
        assert!(episode.correction_note.is_none());
        assert!(episode.context_version.is_none());
        assert!(episode.outcome_recorded_at.is_none());
    }

    #[test]
    fn sealed_prediction_redacted_from_reflections_until_outcome() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-sealed".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-1".to_string(),
                decision: "Take the Denver job?".to_string(),
                options: vec!["Take it".to_string(), "Stay".to_string()],
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: Some("ctx-test".to_string()),
            })
            .expect("decision episode should persist");
        assert_eq!(episode.prediction_status.as_deref(), Some("requested"));

        let mut sealed = episode.clone();
        sealed.twin_prediction = Some(TwinPrediction {
            predicted_option: "Stay".to_string(),
            matched_option_index: Some(1),
            confidence: Some(0.7),
            rationale: Some("Family proximity outweighs salary here.".to_string()),
            parse_mode: "json".to_string(),
            model_id: "test/model".to_string(),
            context_version: "ctx-test".to_string(),
            sealed_at: Utc::now(),
        });
        sealed.prediction_status = Some("sealed".to_string());
        store
            .write_decision_file(&sealed)
            .expect("sealed episode should write");

        let listed = store
            .list_decision_episodes_with_reflections()
            .expect("episodes should list");
        let item = listed
            .iter()
            .find(|item| item.episode.id == "decision-sealed")
            .expect("episode should be listed");
        assert!(item.prediction_sealed);
        assert!(item.episode.twin_prediction.is_none());

        store
            .update_decision_outcome(
                "decision-sealed",
                DecisionOutcomeUpdate {
                    chosen_option: Some("Stay".to_string()),
                    ..DecisionOutcomeUpdate::default()
                },
            )
            .expect("outcome should record");

        let listed = store
            .list_decision_episodes_with_reflections()
            .expect("episodes should list after outcome");
        let item = listed
            .iter()
            .find(|item| item.episode.id == "decision-sealed")
            .expect("episode should be listed after outcome");
        assert!(!item.prediction_sealed);
        let prediction = item
            .episode
            .twin_prediction
            .as_ref()
            .expect("prediction should be revealed after outcome");
        assert_eq!(prediction.predicted_option, "Stay");
    }

    fn decided_episode(
        store: &mut TwinStore,
        id: &str,
        decision: &str,
        chosen: &str,
        correction_note: Option<&str>,
    ) -> DecisionEpisode {
        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: id.to_string(),
                session_id: "session-1".to_string(),
                tile_id: format!("tile-{id}"),
                decision: decision.to_string(),
                options: vec!["Option A".to_string(), "Option B".to_string()],
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: None,
            })
            .expect("episode should persist");
        let mut decided = episode.clone();
        decided.chosen_option = Some(chosen.to_string());
        decided.correction_note = correction_note.map(|note| note.to_string());
        store
            .write_decision_file(&decided)
            .expect("decided episode should write");
        decided
    }

    #[test]
    fn decision_cases_rank_relevance_above_correction_notes() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        decided_episode(
            &mut store,
            "case-relevant",
            "Accept the Denver relocation offer with higher salary?",
            "Option B",
            None,
        );
        decided_episode(
            &mut store,
            "case-correction",
            "Buy the relocation boxes early?",
            "Option A",
            Some("Twin guessed wrong here"),
        );
        // Undecided episode must never appear as a case.
        store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "case-undecided".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-undecided".to_string(),
                decision: "Relocation salary salary salary?".to_string(),
                options: vec!["A".to_string(), "B".to_string()],
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: None,
            })
            .expect("undecided episode should persist");

        let cases = store
            .select_decision_cases("Denver relocation salary decision", None, 5)
            .expect("cases should select");
        assert_eq!(cases.len(), 2);
        // The strongly relevant case outranks the weakly relevant one even
        // though the weak one carries a correction note.
        assert_eq!(cases[0].id, "case-relevant");
        assert_eq!(cases[1].id, "case-correction");
        assert!(cases.iter().all(|case| case.id != "case-undecided"));

        let excluded = store
            .select_decision_cases(
                "Denver relocation salary decision",
                Some("case-relevant"),
                5,
            )
            .expect("exclusion should apply");
        assert!(excluded.iter().all(|case| case.id != "case-relevant"));

        // Zero overlap: most recent decided cases, capped at two.
        let recent = store
            .select_decision_cases("zzz qqq xyzzy", None, 5)
            .expect("fallback should select");
        assert!(!recent.is_empty());
        assert!(recent.len() <= 2);
        assert!(recent.iter().all(|case| case.chosen_option.is_some()));
    }

    fn prediction_options() -> Vec<String> {
        vec![
            "Take the Denver job".to_string(),
            "Stay in Austin".to_string(),
        ]
    }

    #[test]
    fn parse_twin_prediction_strict_json() {
        let raw = r#"{"predicted_option": "Stay in Austin", "option_index": 2, "confidence": 0.8, "rationale": "Family proximity wins."}"#;
        let draft = parse_twin_prediction(raw, &prediction_options());
        assert_eq!(draft.parse_mode, "json");
        assert_eq!(draft.matched_option_index, Some(1));
        assert_eq!(draft.predicted_option, "Stay in Austin");
        assert_eq!(draft.confidence, Some(0.8));
        assert_eq!(draft.rationale.as_deref(), Some("Family proximity wins."));
    }

    #[test]
    fn parse_twin_prediction_fenced_json_with_language_tag() {
        let raw = "```json\n{\"predicted_option\": \"Take the Denver job\", \"option_index\": 1, \"confidence\": 0.6, \"rationale\": \"Growth.\"}\n```";
        let draft = parse_twin_prediction(raw, &prediction_options());
        assert_eq!(draft.parse_mode, "json");
        assert_eq!(draft.matched_option_index, Some(0));
    }

    #[test]
    fn parse_twin_prediction_index_as_string_and_one_based() {
        let raw = r#"{"option_index": "1", "confidence": 0.5}"#;
        let draft = parse_twin_prediction(raw, &prediction_options());
        // 1-based reading preferred: "1" means the first listed option.
        assert_eq!(draft.matched_option_index, Some(0));
        assert_eq!(draft.predicted_option, "Take the Denver job");
    }

    #[test]
    fn parse_twin_prediction_text_beats_conflicting_index() {
        let raw = r#"{"predicted_option": "Stay in Austin", "option_index": 1, "confidence": 0.9}"#;
        let draft = parse_twin_prediction(raw, &prediction_options());
        // The option text is what the model said; the conflicting index loses.
        assert_eq!(draft.matched_option_index, Some(1));
    }

    #[test]
    fn parse_twin_prediction_out_of_range_index_with_valid_text() {
        let raw = r#"{"predicted_option": "Stay in Austin", "option_index": 9}"#;
        let draft = parse_twin_prediction(raw, &prediction_options());
        assert_eq!(draft.matched_option_index, Some(1));
    }

    #[test]
    fn parse_twin_prediction_bare_text_and_labels() {
        let options = prediction_options();
        let bare = parse_twin_prediction("  Stay in Austin\n", &options);
        assert_eq!(bare.parse_mode, "string_match");
        assert_eq!(bare.matched_option_index, Some(1));

        let letter = parse_twin_prediction("B", &options);
        assert_eq!(letter.matched_option_index, Some(1));

        let labeled = parse_twin_prediction("Option 2", &options);
        assert_eq!(labeled.matched_option_index, Some(1));
    }

    #[test]
    fn parse_twin_prediction_garbage_falls_to_raw() {
        let long_garbage = "I think there are many considerations here ".repeat(40);
        let draft = parse_twin_prediction(&long_garbage, &prediction_options());
        assert_eq!(draft.parse_mode, "raw");
        assert!(draft.matched_option_index.is_none());
        assert!(draft.predicted_option.chars().count() <= 500);
    }

    #[test]
    fn parse_twin_prediction_reversed_braces_does_not_panic() {
        // The closing brace appears BEFORE the opening one, so `raw.find('{')` finds a
        // start index greater than `raw.rfind('}')`'s end index. Slicing `&raw[start..=end]`
        // without checking `start <= end` panics — which previously crashed the spawned
        // sealed-prediction task, silently skipping both `attach_twin_prediction` and
        // `mark_twin_prediction_failed`.
        let raw = "Option A} — but {incomplete";
        let draft = parse_twin_prediction(raw, &prediction_options());
        // No panic reaching here is the primary assertion. The malformed brace pair is
        // simply not treated as JSON, so it falls through the same fallback chain as any
        // other non-JSON text.
        assert_ne!(draft.parse_mode, "json");
    }

    #[test]
    fn extract_json_slice_rejects_reversed_braces() {
        assert_eq!(extract_json_slice("Option A} — but {incomplete"), None);
        assert_eq!(extract_json_slice("no braces here"), None);
        assert_eq!(extract_json_slice("only open {"), None);
        assert_eq!(extract_json_slice("only close }"), None);
        assert_eq!(
            extract_json_slice(r#"prefix {"a": 1} suffix"#),
            Some(r#"{"a": 1}"#)
        );
    }

    #[test]
    fn parse_twin_prediction_sanitizes_confidence() {
        let options = prediction_options();
        let percent = parse_twin_prediction(
            r#"{"predicted_option": "Stay in Austin", "confidence": 73}"#,
            &options,
        );
        assert_eq!(percent.confidence, Some(0.73));

        let overshoot = parse_twin_prediction(
            r#"{"predicted_option": "Stay in Austin", "confidence": 1.2}"#,
            &options,
        );
        assert_eq!(overshoot.confidence, Some(1.0));

        let negative = parse_twin_prediction(
            r#"{"predicted_option": "Stay in Austin", "confidence": -0.1}"#,
            &options,
        );
        assert_eq!(negative.confidence, Some(0.0));

        let null = parse_twin_prediction(
            r#"{"predicted_option": "Stay in Austin", "confidence": null}"#,
            &options,
        );
        assert!(null.confidence.is_none());
    }

    #[test]
    fn attach_twin_prediction_refuses_after_outcome_and_duplicates() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-attach".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-1".to_string(),
                decision: "Take the Denver job?".to_string(),
                options: prediction_options(),
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: Some("ctx-test".to_string()),
            })
            .expect("episode should persist");

        let draft = parse_twin_prediction("Stay in Austin", &prediction_options());
        let sealed = store
            .attach_twin_prediction(&episode.id, draft.clone(), "test/model", "ctx-test")
            .expect("first attach should seal");
        assert_eq!(sealed.prediction_status.as_deref(), Some("sealed"));
        let first_sealed_at = sealed.twin_prediction.as_ref().unwrap().sealed_at;

        // Duplicate attach is a no-op.
        let duplicate = store
            .attach_twin_prediction(&episode.id, draft.clone(), "other/model", "ctx-test")
            .expect("duplicate attach should not error");
        assert_eq!(
            duplicate.twin_prediction.as_ref().unwrap().sealed_at,
            first_sealed_at
        );
        assert_eq!(
            duplicate.twin_prediction.as_ref().unwrap().model_id,
            "test/model"
        );

        // Outcome-first race: attach after the choice is recorded.
        let late_episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-late".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-2".to_string(),
                decision: "Take the Denver job?".to_string(),
                options: prediction_options(),
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: None,
            })
            .expect("episode should persist");
        store
            .update_decision_outcome(
                &late_episode.id,
                DecisionOutcomeUpdate {
                    chosen_option: Some("Stay in Austin".to_string()),
                    ..DecisionOutcomeUpdate::default()
                },
            )
            .expect("outcome should record");
        let refused = store
            .attach_twin_prediction(&late_episode.id, draft, "test/model", "ctx-test")
            .expect("late attach should not error");
        assert!(refused.twin_prediction.is_none());
        assert_eq!(
            refused.prediction_status.as_deref(),
            Some("outcome_recorded_first")
        );
    }

    #[test]
    fn outcome_computes_agreement_and_canonicalizes_choice() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-agree".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-1".to_string(),
                decision: "Take the Denver job?".to_string(),
                options: prediction_options(),
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: Some("ctx-test".to_string()),
            })
            .expect("episode should persist");
        let draft = parse_twin_prediction("Stay in Austin", &prediction_options());
        store
            .attach_twin_prediction(&episode.id, draft, "test/model", "ctx-test")
            .expect("prediction should seal");

        // Label + case variant resolves to the canonical option text and
        // agreement computes via index comparison.
        let updated = store
            .update_decision_outcome(
                &episode.id,
                DecisionOutcomeUpdate {
                    chosen_option: Some("option 2".to_string()),
                    correction_note: None,
                    ..DecisionOutcomeUpdate::default()
                },
            )
            .expect("outcome should record");
        assert_eq!(updated.chosen_option.as_deref(), Some("Stay in Austin"));
        assert_eq!(updated.agreement, Some(true));
        assert!(updated.outcome_recorded_at.is_some());

        // Editing the choice recomputes agreement and accepts a correction
        // note; outcome_recorded_at is not reset.
        let first_recorded_at = updated.outcome_recorded_at;
        let edited = store
            .update_decision_outcome(
                &episode.id,
                DecisionOutcomeUpdate {
                    chosen_option: Some("  TAKE the denver JOB ".to_string()),
                    correction_note: Some("Twin overweighted family proximity.".to_string()),
                    ..DecisionOutcomeUpdate::default()
                },
            )
            .expect("edited outcome should record");
        assert_eq!(edited.chosen_option.as_deref(), Some("Take the Denver job"));
        assert_eq!(edited.agreement, Some(false));
        assert_eq!(
            edited.correction_note.as_deref(),
            Some("Twin overweighted family proximity.")
        );
        assert_eq!(edited.outcome_recorded_at, first_recorded_at);
    }

    #[test]
    fn outcome_without_prediction_records_no_agreement_and_keeps_free_text() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let episode = store
            .record_decision_episode(DecisionEpisodeCreate {
                id: "decision-free".to_string(),
                session_id: "session-1".to_string(),
                tile_id: "tile-1".to_string(),
                decision: "Take the Denver job?".to_string(),
                options: prediction_options(),
                stakes: None,
                initial_leaning: None,
                review_date: None,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                context_version: None,
            })
            .expect("episode should persist");

        let updated = store
            .update_decision_outcome(
                &episode.id,
                DecisionOutcomeUpdate {
                    chosen_option: Some("Negotiated a remote arrangement instead".to_string()),
                    ..DecisionOutcomeUpdate::default()
                },
            )
            .expect("outcome should record");
        // Free text that matches no option is preserved verbatim.
        assert_eq!(
            updated.chosen_option.as_deref(),
            Some("Negotiated a remote arrangement instead")
        );
        assert!(updated.agreement.is_none());
    }

    #[test]
    fn memory_digest_caps_review_items_and_updates_record_state() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        let mut record_ids = Vec::new();
        for index in 0..6 {
            let record = store
                .create_user_record(UserRecordCreate {
                    kind: UserRecordKind::ReasoningPattern,
                    content: format!("Pattern {} benefits from evidence gates.", index),
                    origin: RecordOrigin::Inferred,
                    evidence_refs: vec![
                        EvidenceRef {
                            trace_id: "session-1".to_string(),
                            event_id: format!("event-{}-1", index),
                            session_id: "session-1".to_string(),
                            tile_id: Some(format!("tile-{}", index)),
                            model_id: None,
                            note: None,
                            source_type: Some("behavior".to_string()),
                            source_id: None,
                            source_label: None,
                            excerpt: None,
                            speaker_role: None,
                        },
                        EvidenceRef {
                            trace_id: "session-1".to_string(),
                            event_id: format!("event-{}-2", index),
                            session_id: "session-1".to_string(),
                            tile_id: Some(format!("tile-{}", index)),
                            model_id: None,
                            note: None,
                            source_type: Some("behavior".to_string()),
                            source_id: None,
                            source_label: None,
                            excerpt: None,
                            speaker_role: None,
                        },
                        EvidenceRef {
                            trace_id: "session-1".to_string(),
                            event_id: format!("event-{}-3", index),
                            session_id: "session-1".to_string(),
                            tile_id: Some(format!("tile-{}", index)),
                            model_id: None,
                            note: None,
                            source_type: Some("behavior".to_string()),
                            source_id: None,
                            source_label: None,
                            excerpt: None,
                            speaker_role: None,
                        },
                    ],
                    confidence: 0.82,
                    promotion_state: Some(PromotionState::Candidate),
                    valid_from: None,
                    valid_until: None,
                    links: Vec::new(),
                    metadata: HashMap::from([(
                        "signal_family".to_string(),
                        serde_json::json!(format!("evidence_gate_family_{}", index)),
                    )]),
                })
                .expect("record should be created");
            record_ids.push(record.id);
        }

        let digest = store.list_memory_digest().expect("digest should list");
        assert_eq!(digest.len(), 5);
        assert!(digest.iter().all(|item| item.evidence_count == 3));

        let reviewed = store
            .review_memory_digest_item(
                &digest[0].id,
                MemoryDigestReviewRequest {
                    action: MemoryDigestAction::NoTrain,
                    rationale: Some("Do not use this in twin context".to_string()),
                },
            )
            .expect("digest item should update");
        assert_eq!(reviewed.state, MemoryDigestState::NoTrain);

        let linked_record = store
            .get_user_record(&reviewed.record_ids[0])
            .expect("linked record should still exist");
        assert_eq!(linked_record.promotion_state, PromotionState::NoTrain);

        let (approved, candidates) = store
            .select_context_records("evidence gates")
            .expect("context records should select");
        assert!(!approved
            .iter()
            .any(|record| record.id == reviewed.record_ids[0]));
        assert!(!candidates
            .iter()
            .any(|record| record.id == reviewed.record_ids[0]));
        assert_eq!(record_ids.len(), 6);
    }

    #[test]
    fn memory_digest_clusters_related_records() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        for index in 0..3 {
            store
                .create_user_record(UserRecordCreate {
                    kind: UserRecordKind::ReasoningPattern,
                    content: format!(
                        "Pattern {} says the user benefits from hard evidence gates before scaling.",
                        index
                    ),
                    origin: RecordOrigin::Inferred,
                    evidence_refs: vec![
                        EvidenceRef {
                            trace_id: "session-1".to_string(),
                            event_id: format!("cluster-event-{}-1", index),
                            session_id: "session-1".to_string(),
                            tile_id: Some(format!("cluster-tile-{}", index)),
                            model_id: None,
                            note: None,
                            source_type: Some("behavior".to_string()),
                            source_id: None,
                            source_label: None,
                            excerpt: None,
                            speaker_role: None,
                        },
                        EvidenceRef {
                            trace_id: "session-1".to_string(),
                            event_id: format!("cluster-event-{}-2", index),
                            session_id: "session-1".to_string(),
                            tile_id: Some(format!("cluster-tile-{}", index)),
                            model_id: None,
                            note: None,
                            source_type: Some("behavior".to_string()),
                            source_id: None,
                            source_label: None,
                            excerpt: None,
                            speaker_role: None,
                        },
                        EvidenceRef {
                            trace_id: "session-1".to_string(),
                            event_id: format!("cluster-event-{}-3", index),
                            session_id: "session-1".to_string(),
                            tile_id: Some(format!("cluster-tile-{}", index)),
                            model_id: None,
                            note: None,
                            source_type: Some("behavior".to_string()),
                            source_id: None,
                            source_label: None,
                            excerpt: None,
                            speaker_role: None,
                        },
                    ],
                    confidence: 0.84,
                    promotion_state: Some(PromotionState::Candidate),
                    valid_from: None,
                    valid_until: None,
                    links: Vec::new(),
                    metadata: HashMap::from([(
                        "signal_family".to_string(),
                        serde_json::json!("evidence_gates"),
                    )]),
                })
                .expect("record should be created");
        }

        let digest = store.list_memory_digest().expect("digest should list");
        assert_eq!(digest.len(), 1);
        assert_eq!(digest[0].record_ids.len(), 3);
        assert_eq!(digest[0].evidence_count, 9);
        assert!(digest[0].trigger_reason.contains("clustered"));
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
    fn export_redacts_sealed_predictions_and_filters_private_feedback_events() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());

        for (id, record_outcome) in [("episode-sealed", false), ("episode-revealed", true)] {
            store
                .record_decision_episode(DecisionEpisodeCreate {
                    id: id.to_string(),
                    session_id: "session-1".to_string(),
                    tile_id: format!("tile-{id}"),
                    decision: "Take the Denver job?".to_string(),
                    options: prediction_options(),
                    stakes: None,
                    initial_leaning: None,
                    review_date: None,
                    primitive_assessment: PrimitiveDecisionAssessment::default(),
                    context_version: Some("ctx-test".to_string()),
                })
                .expect("episode should persist");
            let draft = parse_twin_prediction("Stay in Austin", &prediction_options());
            store
                .attach_twin_prediction(id, draft, "test/model", "ctx-test")
                .expect("prediction should seal");
            if record_outcome {
                store
                    .update_decision_outcome(
                        id,
                        DecisionOutcomeUpdate {
                            chosen_option: Some("Stay in Austin".to_string()),
                            ..DecisionOutcomeUpdate::default()
                        },
                    )
                    .expect("outcome should record");
            }
        }

        // One feedback event tied to a record later marked Private, one free.
        let private_event = store
            .append_trace_event(
                "session-1",
                TraceEventType::FeedbackRecorded,
                json!({"rationale": "extremely sensitive rationale"}),
            )
            .expect("event should append");
        let exportable_event = store
            .append_trace_event(
                "session-1",
                TraceEventType::RankingRecorded,
                json!({"ranking": ["a", "b"]}),
            )
            .expect("event should append");
        store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "Private claim".to_string(),
                origin: RecordOrigin::User,
                evidence_refs: vec![EvidenceRef {
                    trace_id: "session-1".to_string(),
                    event_id: private_event.id.clone(),
                    session_id: "session-1".to_string(),
                    tile_id: None,
                    model_id: None,
                    note: None,
                    source_type: None,
                    source_id: None,
                    source_label: None,
                    excerpt: None,
                    speaker_role: None,
                }],
                confidence: default_record_confidence(),
                promotion_state: Some(PromotionState::Private),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .expect("private record should be created");

        let bundle = store
            .export_bundle(TwinExportRequest::default())
            .expect("export should succeed");
        assert_eq!(bundle.decision_episodes.count, 2);

        let episodes_content = std::fs::read_to_string(&bundle.decision_episodes.path)
            .expect("decision episodes file should read");
        let sealed_line = episodes_content
            .lines()
            .find(|line| line.contains("episode-sealed"))
            .expect("sealed episode should export");
        assert!(sealed_line.contains("\"sealed\":true"));
        assert!(!sealed_line.contains("predicted_option"));
        assert!(!sealed_line.contains("rationale"));
        let revealed_line = episodes_content
            .lines()
            .find(|line| line.contains("episode-revealed"))
            .expect("revealed episode should export");
        assert!(revealed_line.contains("predicted_option"));
        assert!(revealed_line.contains("\"agreement\":true"));
        assert!(revealed_line.contains("ctx-test"));

        let feedback_content = std::fs::read_to_string(&bundle.feedback_events.path)
            .expect("feedback events file should read");
        assert!(!feedback_content.contains("extremely sensitive rationale"));
        assert!(!feedback_content.contains(&private_event.id));
        assert!(feedback_content.contains(&exportable_event.id));
    }
}
