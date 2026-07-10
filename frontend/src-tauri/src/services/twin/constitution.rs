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
use super::shared::{excerpt, lexical_terms, load_or_quarantine, text_contains_any};
use super::{AUTO_PROMOTE_SUPPORT_COUNT};

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

pub(super) fn normalize_key_text(text: &str) -> String {
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


impl TwinStore {
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

    pub(super) fn constitution_file_path(&self, item_id: &str) -> PathBuf {
        self.constitution_path.join(format!("{}.json", item_id))
    }

    pub(super) fn action_gap_file_path(&self, gap_id: &str) -> PathBuf {
        self.action_gaps_path.join(format!("{}.json", gap_id))
    }

    pub(super) fn read_constitution_file(&self, path: &Path) -> Result<ConstitutionItem> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read constitution file: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse constitution file: {}", path.display()))
    }

    pub(super) fn read_action_gap_file(&self, path: &Path) -> Result<ActionGap> {
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

}
