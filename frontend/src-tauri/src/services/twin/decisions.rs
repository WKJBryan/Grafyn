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
use super::shared::{excerpt, extract_event_model_id, extract_event_tile_id, lexical_terms, load_or_quarantine, payload_string};

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


impl TwinStore {
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

}
