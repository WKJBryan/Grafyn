use super::shared::{
    excerpt, extract_event_model_id, extract_event_tile_id, lexical_terms, payload_string,
};
use super::TwinStore;
use crate::models::twin::{
    ConstitutionStatus, DecisionEpisode, DecisionEpisodeCreate, DecisionEpisodeWithReflections,
    DecisionEvidencePacket, DecisionEvidenceSource, DecisionMirrorConfig,
    DecisionMirrorConfigUpdate, DecisionMirrorWeights, DecisionOutcomeUpdate, EvidenceRef,
    PromotionState, ReflectionCard, ReflectionCardCreate, ReflectionScores, SessionTrace,
    TraceEvent, TraceEventType, TwinPrediction, TwinPredictionDraft,
};
#[cfg(test)]
use crate::models::twin::{DecisionMirrorPreset, PrimitiveDecisionAssessment};
use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

type DecisionEpisodeMutationPlan = (
    DecisionEpisode,
    SessionTrace,
    Vec<(PathBuf, String)>,
    Vec<crate::services::twin_events::TwinEventDraft>,
);
type TwinJsonWrites = Vec<(PathBuf, String)>;
type DecisionOutcomeMutation = (
    SessionTrace,
    TwinJsonWrites,
    Vec<crate::services::twin_events::TwinEventDraft>,
);
type DecisionOutcomePlan = (DecisionEpisode, Option<DecisionOutcomeMutation>);
type TwinPredictionPlan = (DecisionEpisode, Option<SessionTrace>, TwinJsonWrites);
type ReflectionCardPlan = (ReflectionCard, SessionTrace, TwinJsonWrites);

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

fn apply_decision_mirror_config_update(
    config: &mut DecisionMirrorConfig,
    update: &DecisionMirrorConfigUpdate,
) {
    if let Some(preset) = update.preset.clone() {
        config.weights = DecisionMirrorWeights::for_preset(&preset);
        config.preset = preset;
    }
    if let Some(weights) = update.weights.clone() {
        config.weights = clamp_decision_mirror_weights(weights);
    }
    if let Some(advanced_enabled) = update.advanced_enabled {
        config.advanced_enabled = advanced_enabled;
    }
}

fn promotion_state_label(state: &PromotionState) -> &'static str {
    match state {
        PromotionState::Candidate => "Candidate",
        PromotionState::AutoPromoted => "Candidate (legacy)",
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
        let mut config: DecisionMirrorConfig = self
            .read_twin_json_bounded(&self.decision_mirror_config_path)?
            .unwrap_or_default();
        config.weights = clamp_decision_mirror_weights(config.weights);
        Ok(config)
    }

    pub fn update_decision_mirror_config(
        &mut self,
        update: DecisionMirrorConfigUpdate,
    ) -> Result<DecisionMirrorConfig> {
        let (config, _commit) = self.update_decision_mirror_config_with_commit(update)?;
        Ok(config)
    }

    pub(crate) fn update_decision_mirror_config_with_commit(
        &mut self,
        update: DecisionMirrorConfigUpdate,
    ) -> Result<(
        DecisionMirrorConfig,
        crate::services::twin_events::MutationCommit,
    )> {
        if !self.event_recorder.is_noop() {
            let recorder = self.event_recorder.clone();
            let mut committed = None;
            let mut planner = || {
                let mut config = self.get_decision_mirror_config().map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                apply_decision_mirror_config_update(&mut config, &update);
                let content = serde_json::to_string_pretty(&config).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                let targets = self
                    .governed_json_targets(vec![(
                        self.decision_mirror_config_path.clone(),
                        content,
                    )])
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                committed = Some(config);
                Ok(Some(crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::LocalOnly,
                    crate::models::twin_event::SourceChannel::parse("legacy_twin")
                        .map_err(crate::services::twin_events::MutationError::Invalid)?,
                    targets,
                    Vec::new(),
                )))
            };
            let result = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            );
            let commit = self.finish_mutation_commit(result)?;
            let config =
                committed.ok_or_else(|| anyhow::anyhow!("config update was not planned"))?;
            self.invalidate_mutation_caches();
            return Ok((config, commit));
        }
        let mut config = self.get_decision_mirror_config()?;
        apply_decision_mirror_config_update(&mut config, &update);

        self.write_decision_mirror_config_file(&config)?;
        self.invalidate_mutation_caches();
        Ok((config, Self::tokenless_mutation_commit()))
    }

    pub fn reset_decision_mirror_config(&mut self) -> Result<DecisionMirrorConfig> {
        let (config, _commit) = self.reset_decision_mirror_config_with_commit()?;
        Ok(config)
    }

    pub(crate) fn reset_decision_mirror_config_with_commit(
        &mut self,
    ) -> Result<(
        DecisionMirrorConfig,
        crate::services::twin_events::MutationCommit,
    )> {
        let config = DecisionMirrorConfig::default();
        let commit = if self.event_recorder.is_noop() {
            self.write_decision_mirror_config_file(&config)?;
            Self::tokenless_mutation_commit()
        } else {
            let values = vec![(
                self.decision_mirror_config_path.clone(),
                serde_json::to_string_pretty(&config)?,
            )];
            let targets = self.governed_json_targets(values)?;
            let result = self.event_recorder.commit_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                crate::models::twin_event::CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("legacy_twin")
                    .map_err(anyhow::Error::msg)?,
                targets,
                Vec::new(),
            );
            self.finish_mutation_commit(result)?
        };
        self.invalidate_mutation_caches();
        Ok((config, commit))
    }

    pub fn record_decision_episode(
        &mut self,
        create: DecisionEpisodeCreate,
    ) -> Result<DecisionEpisode> {
        self.record_decision_episode_with_commit(create)
            .map(|(episode, _commit)| episode)
    }

    pub(crate) fn record_decision_episode_with_commit(
        &mut self,
        create: DecisionEpisodeCreate,
    ) -> Result<(
        DecisionEpisode,
        crate::services::twin_events::MutationCommit,
    )> {
        if !self.event_recorder.is_noop() {
            let recorder = self.event_recorder.clone();
            let mut committed = None;
            let mut planner = || {
                let (episode, trace, values, drafts) = self
                    .plan_decision_episode_mutation(create.clone())
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                let targets = self.governed_json_targets(values).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                committed = Some((episode, trace));
                Ok(Some(crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::SyncEligible,
                    crate::models::twin_event::SourceChannel::parse("legacy_twin")
                        .map_err(crate::services::twin_events::MutationError::Invalid)?,
                    targets,
                    drafts,
                )))
            };
            let result = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            );
            let commit = self.finish_mutation_commit(result)?;
            let (episode, trace) =
                committed.ok_or_else(|| anyhow::anyhow!("decision episode was not planned"))?;
            self.cache_committed_trace(trace);
            return Ok((episode, commit));
        }
        let (episode, trace, values, drafts) = self.plan_decision_episode_mutation(create)?;
        let commit = self.commit_governed_json_targets(values, drafts)?;
        self.cache_committed_trace(trace);
        Ok((episode, commit))
    }

    pub(crate) fn plan_decision_episode_mutation(
        &self,
        create: DecisionEpisodeCreate,
    ) -> Result<DecisionEpisodeMutationPlan> {
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

        let digest = Self::governed_json_digest(&episode)?;
        let mut draft = crate::services::twin_events::TwinEventDraft::observed(
            crate::models::twin_event::TwinEventPayload::DecisionRecorded(
                crate::models::twin_event::DecisionRecorded {
                    decision_id: crate::models::twin_event::Identifier::parse(&episode.id)
                        .map_err(anyhow::Error::msg)?,
                    decision: crate::models::twin_event::BoundedContent::parse(&episode.decision)
                        .map_err(anyhow::Error::msg)?,
                    options: episode
                        .options
                        .iter()
                        .map(crate::models::twin_event::BoundedContent::parse)
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .map_err(anyhow::Error::msg)?,
                    stakes: episode
                        .stakes
                        .as_deref()
                        .map(crate::models::twin_event::BoundedContent::parse)
                        .transpose()
                        .map_err(anyhow::Error::msg)?,
                    initial_leaning: episode
                        .initial_leaning
                        .as_deref()
                        .map(crate::models::twin_event::BoundedContent::parse)
                        .transpose()
                        .map_err(anyhow::Error::msg)?,
                    review_date: episode
                        .review_date
                        .as_deref()
                        .map(crate::models::twin_event::BoundedLabel::parse)
                        .transpose()
                        .map_err(anyhow::Error::msg)?,
                    primitive_assessment:
                        crate::models::twin_event::PrimitiveDecisionAssessmentPayload::from_legacy(
                            &episode.primitive_assessment,
                        )
                        .map_err(anyhow::Error::msg)?,
                },
            ),
            episode.updated_at,
            crate::models::twin_event::SourceChannel::parse("legacy_twin")
                .map_err(anyhow::Error::msg)?,
            crate::services::twin_events::standard_capture_governance(),
        );
        draft.evidence.push(crate::models::twin_event::EvidenceRef {
            evidence_type: crate::models::twin_event::EvidenceType::TwinRecord,
            source_id: crate::models::twin_event::Identifier::parse(&episode.id)
                .map_err(anyhow::Error::msg)?,
            digest: Some(digest),
        });
        let (_, trace) = self.plan_trace_event(
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
        let values = vec![
            (
                self.decision_file_path(&episode.id),
                serde_json::to_string_pretty(&episode)?,
            ),
            self.serialized_trace_target(&trace)?,
        ];

        Ok((episode, trace, values, vec![draft]))
    }

    pub fn list_decision_episodes(&self) -> Result<Vec<DecisionEpisode>> {
        let mut episodes = self.list_twin_json_bounded::<DecisionEpisode>("decisions")?;
        episodes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(episodes)
    }

    pub fn get_decision_episode(&self, id: &str) -> Result<DecisionEpisode> {
        Self::validate_file_id(id)?;
        self.read_decision_file(&self.decision_file_path(id))
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
        self.update_decision_outcome_with_response_id(id, update, None)
    }

    pub fn update_decision_outcome_with_response_id(
        &mut self,
        id: &str,
        update: DecisionOutcomeUpdate,
        selected_response_id: Option<String>,
    ) -> Result<DecisionEpisode> {
        let (episode, _commit) = self.update_decision_outcome_with_response_id_and_commit(
            id,
            update,
            selected_response_id,
        )?;
        Ok(episode)
    }

    pub(crate) fn update_decision_outcome_with_response_id_and_commit(
        &mut self,
        id: &str,
        update: DecisionOutcomeUpdate,
        selected_response_id: Option<String>,
    ) -> Result<(
        DecisionEpisode,
        crate::services::twin_events::MutationCommit,
    )> {
        Self::validate_file_id(id)?;
        if !self.event_recorder.is_noop() {
            let recorder = self.event_recorder.clone();
            let mut committed = None;
            let mut planner = || {
                let (episode, mutation) = self
                    .plan_decision_outcome_mutation(id, &update, selected_response_id.as_deref())
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                committed = Some((episode, mutation.as_ref().map(|value| value.0.clone())));
                let Some((_trace, values, drafts)) = mutation else {
                    return Ok(None);
                };
                let targets = self.governed_json_targets(values).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                Ok(Some(crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::SyncEligible,
                    crate::models::twin_event::SourceChannel::parse("legacy_twin")
                        .map_err(crate::services::twin_events::MutationError::Invalid)?,
                    targets,
                    drafts,
                )))
            };
            let result = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            );
            let commit = self.finish_mutation_commit(result)?;
            let (episode, trace) =
                committed.ok_or_else(|| anyhow::anyhow!("decision outcome was not planned"))?;
            if let Some(trace) = trace {
                self.cache_committed_trace(trace);
            } else {
                self.invalidate_mutation_caches();
            }
            return Ok((episode, commit));
        }

        let (episode, mutation) =
            self.plan_decision_outcome_mutation(id, &update, selected_response_id.as_deref())?;
        let commit = if let Some((trace, values, drafts)) = mutation {
            let commit = self.commit_governed_json_targets(values, drafts)?;
            self.cache_committed_trace(trace);
            commit
        } else {
            self.invalidate_mutation_caches();
            Self::tokenless_mutation_commit()
        };
        Ok((episode, commit))
    }

    fn plan_decision_outcome_mutation(
        &self,
        id: &str,
        update: &DecisionOutcomeUpdate,
        selected_response_id: Option<&str>,
    ) -> Result<DecisionOutcomePlan> {
        let path = self.decision_file_path(id);
        let mut episode = self.read_decision_file(&path)?;
        let before = serde_json::to_value(&episode)?;
        let explicit_outcome = update.outcome.clone();
        let explicit_choice = update.chosen_option.clone();
        let explicit_confidence = update.confidence;
        let explicit_review_date = update.review_date.clone();
        let explicit_correction = update.correction_note.clone();
        let explicit_regret = update.regret_score;
        let explicit_lesson = update.lesson.clone();
        let explicit_missed = update.missed_something.clone();
        let explicit_primitive = update.primitive_assessment.clone();

        if let Some(selected_response) = update.selected_response.clone() {
            episode.selected_response = Some(selected_response);
        }
        if let Some(chosen_option) = update.chosen_option.clone() {
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
        if let Some(review_date) = update.review_date.clone() {
            episode.review_date = Some(review_date);
        }
        if let Some(outcome) = update.outcome.clone() {
            episode.outcome = Some(outcome);
        }
        if let Some(regret_score) = update.regret_score {
            episode.regret_score = Some(regret_score.min(10));
        }
        if let Some(lesson) = update.lesson.clone() {
            episode.lesson = Some(lesson);
        }
        if let Some(missed_something) = update.missed_something.clone() {
            episode.missed_something = Some(missed_something);
        }
        if let Some(primitive_assessment) = update.primitive_assessment.clone() {
            episode.primitive_assessment = primitive_assessment;
        }
        if let Some(correction_note) = update.correction_note.clone() {
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

        if serde_json::to_value(&episode)? == before {
            return Ok((episode, None));
        }
        episode.updated_at = Utc::now();
        let drafts = {
            let confidence_basis_points =
                explicit_confidence.map(|value| (value.clamp(0.0, 1.0) * 10_000.0).round() as u16);
            if explicit_outcome.is_none()
                && explicit_choice.is_none()
                && selected_response_id.is_none()
                && confidence_basis_points.is_none()
                && explicit_review_date.is_none()
                && explicit_correction.is_none()
                && explicit_regret.is_none()
                && explicit_lesson.is_none()
                && explicit_missed.is_none()
                && explicit_primitive.is_none()
            {
                anyhow::bail!("decision outcome mutation has no governed follow-up field");
            }
            let payload = crate::models::twin_event::TwinEventPayload::DecisionOutcomeRecorded(
                crate::models::twin_event::DecisionOutcomeRecorded {
                    decision_id: crate::models::twin_event::Identifier::parse(&episode.id)
                        .map_err(anyhow::Error::msg)?,
                    outcome: explicit_outcome
                        .as_deref()
                        .map(crate::models::twin_event::BoundedContent::parse)
                        .transpose()
                        .map_err(anyhow::Error::msg)?,
                    chosen_option: explicit_choice
                        .as_deref()
                        .map(crate::models::twin_event::BoundedContent::parse)
                        .transpose()
                        .map_err(anyhow::Error::msg)?,
                    selected_response_id: selected_response_id
                        .map(crate::models::twin_event::Identifier::parse)
                        .transpose()
                        .map_err(anyhow::Error::msg)?,
                    confidence_basis_points,
                    review_date: explicit_review_date
                        .as_deref()
                        .map(crate::models::twin_event::BoundedLabel::parse)
                        .transpose()
                        .map_err(anyhow::Error::msg)?,
                    correction_note: explicit_correction
                        .as_deref()
                        .map(crate::models::twin_event::BoundedContent::parse)
                        .transpose()
                        .map_err(anyhow::Error::msg)?,
                    regret_score: explicit_regret,
                    lesson: explicit_lesson
                        .as_deref()
                        .map(crate::models::twin_event::BoundedContent::parse)
                        .transpose()
                        .map_err(anyhow::Error::msg)?,
                    missed_something: explicit_missed
                        .as_deref()
                        .map(crate::models::twin_event::BoundedContent::parse)
                        .transpose()
                        .map_err(anyhow::Error::msg)?,
                    primitive_assessment: explicit_primitive
                        .as_ref()
                        .map(
                            crate::models::twin_event::PrimitiveDecisionAssessmentPayload::from_legacy,
                        )
                        .transpose()
                        .map_err(anyhow::Error::msg)?,
                },
            );
            let digest = Self::governed_json_digest(&episode)?;
            let mut draft = crate::services::twin_events::TwinEventDraft::observed(
                payload,
                episode.updated_at,
                crate::models::twin_event::SourceChannel::parse("legacy_twin")
                    .map_err(anyhow::Error::msg)?,
                crate::services::twin_events::standard_capture_governance(),
            );
            draft.evidence.push(crate::models::twin_event::EvidenceRef {
                evidence_type: crate::models::twin_event::EvidenceType::TwinRecord,
                source_id: crate::models::twin_event::Identifier::parse(&episode.id)
                    .map_err(anyhow::Error::msg)?,
                digest: Some(digest),
            });
            vec![draft]
        };
        let (_, trace) = self.plan_trace_event(
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
        let trace_target = self.serialized_trace_target(&trace)?;
        Ok((
            episode.clone(),
            Some((
                trace,
                vec![
                    (path, serde_json::to_string_pretty(&episode)?),
                    trace_target,
                ],
                drafts,
            )),
        ))
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
        self.attach_twin_prediction_internal(episode_id, draft, model_id, context_version, None)
            .map(|(episode, _)| episode)
    }

    pub(crate) fn attach_twin_prediction_expecting_authority(
        &mut self,
        episode_id: &str,
        draft: TwinPredictionDraft,
        model_id: &str,
        context_version: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(
        DecisionEpisode,
        crate::services::twin_events::MutationCommit,
    )> {
        self.attach_twin_prediction_internal(
            episode_id,
            draft,
            model_id,
            context_version,
            Some(expected),
        )
    }

    fn attach_twin_prediction_internal(
        &mut self,
        episode_id: &str,
        draft: TwinPredictionDraft,
        model_id: &str,
        context_version: &str,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<(
        DecisionEpisode,
        crate::services::twin_events::MutationCommit,
    )> {
        Self::validate_file_id(episode_id)?;
        if !self.event_recorder.is_noop() {
            let recorder = self.event_recorder.clone();
            let mut committed = None;
            let mut planner = || {
                let (episode, trace, values) = self
                    .plan_twin_prediction_mutation(episode_id, &draft, model_id, context_version)
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                committed = Some((episode, trace));
                if values.is_empty() {
                    return Ok(None);
                }
                let targets = self.governed_json_targets(values).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                let mut plan = crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::LocalOnly,
                    crate::models::twin_event::SourceChannel::parse("legacy_twin")
                        .map_err(crate::services::twin_events::MutationError::Invalid)?,
                    targets,
                    Vec::new(),
                );
                if let Some(expected) = expected.clone() {
                    plan = plan.expecting_authority(expected);
                }
                Ok(Some(plan))
            };
            let result = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            );
            let commit = self.finish_mutation_commit(result)?;
            let (episode, trace) =
                committed.ok_or_else(|| anyhow::anyhow!("Twin prediction was not planned"))?;
            if let Some(trace) = trace {
                self.cache_committed_trace(trace);
            } else {
                self.invalidate_mutation_caches();
            }
            return Ok((episode, commit));
        }
        let (episode, trace, values) =
            self.plan_twin_prediction_mutation(episode_id, &draft, model_id, context_version)?;
        let commit = if values.is_empty() {
            Self::tokenless_mutation_commit()
        } else {
            self.commit_governed_json_targets(values, Vec::new())?
        };
        if let Some(trace) = trace {
            self.cache_committed_trace(trace);
        } else {
            self.invalidate_mutation_caches();
        }
        Ok((episode, commit))
    }

    fn plan_twin_prediction_mutation(
        &self,
        episode_id: &str,
        draft: &TwinPredictionDraft,
        model_id: &str,
        context_version: &str,
    ) -> Result<TwinPredictionPlan> {
        let path = self.decision_file_path(episode_id);
        let mut episode = self.read_decision_file(&path)?;

        if episode.chosen_option.is_some() {
            log::warn!(
                "Twin prediction for episode {} arrived after the outcome was recorded; discarding",
                episode_id
            );
            episode.prediction_status = Some("outcome_recorded_first".to_string());
            episode.updated_at = Utc::now();
            return Ok((
                episode.clone(),
                None,
                vec![(path, serde_json::to_string_pretty(&episode)?)],
            ));
        }
        if episode.twin_prediction.is_some() {
            log::warn!(
                "Episode {} already has a sealed twin prediction; ignoring duplicate",
                episode_id
            );
            return Ok((episode, None, Vec::new()));
        }

        let prediction = TwinPrediction {
            predicted_option: draft.predicted_option.clone(),
            matched_option_index: draft.matched_option_index,
            confidence: draft.confidence,
            rationale: draft.rationale.clone(),
            parse_mode: draft.parse_mode.clone(),
            model_id: model_id.to_string(),
            context_version: context_version.to_string(),
            sealed_at: Utc::now(),
        };
        let sealed_at = prediction.sealed_at;
        episode.twin_prediction = Some(prediction);
        episode.prediction_status = Some("sealed".to_string());
        episode.updated_at = Utc::now();
        let (_, trace) = self.plan_trace_event(
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
        let values = vec![
            (path, serde_json::to_string_pretty(&episode)?),
            self.serialized_trace_target(&trace)?,
        ];
        Ok((episode, Some(trace), values))
    }

    /// Record that the hidden prediction call failed, so exported episodes
    /// distinguish "no prediction because the call failed" from "agreed to
    /// not predict" — silent gaps would inflate measured accuracy.
    pub fn mark_twin_prediction_failed(&mut self, episode_id: &str) -> Result<()> {
        self.mark_twin_prediction_failed_internal(episode_id, None)
            .map(|_| ())
    }

    pub(crate) fn mark_twin_prediction_failed_with_commit(
        &mut self,
        episode_id: &str,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.mark_twin_prediction_failed_internal(episode_id, None)
    }

    #[cfg(test)]
    pub(crate) fn mark_twin_prediction_failed_expecting_authority(
        &mut self,
        episode_id: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.mark_twin_prediction_failed_internal(episode_id, Some(expected))
    }

    fn mark_twin_prediction_failed_internal(
        &mut self,
        episode_id: &str,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        Self::validate_file_id(episode_id)?;
        if !self.event_recorder.is_noop() {
            let recorder = self.event_recorder.clone();
            let mut planner = || {
                let path = self.decision_file_path(episode_id);
                let mut episode = self.read_decision_file(&path).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                if episode.twin_prediction.is_some()
                    || episode.chosen_option.is_some()
                    || episode.prediction_status.as_deref() != Some("requested")
                {
                    return Ok(None);
                }
                episode.prediction_status = Some("failed".to_string());
                episode.updated_at = Utc::now();
                let content = serde_json::to_string_pretty(&episode).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                let targets =
                    self.governed_json_targets(vec![(path, content)])
                        .map_err(|error| {
                            crate::services::twin_events::MutationError::Invalid(error.to_string())
                        })?;
                let mut plan = crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::LocalOnly,
                    crate::models::twin_event::SourceChannel::parse("legacy_twin")
                        .map_err(crate::services::twin_events::MutationError::Invalid)?,
                    targets,
                    Vec::new(),
                );
                if let Some(expected) = expected.clone() {
                    plan = plan.expecting_authority(expected);
                }
                Ok(Some(plan))
            };
            let result = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            );
            let commit = self.finish_mutation_commit(result)?;
            self.invalidate_mutation_caches();
            return Ok(commit);
        }
        let path = self.decision_file_path(episode_id);
        let mut episode = self.read_decision_file(&path)?;
        if episode.twin_prediction.is_some()
            || episode.chosen_option.is_some()
            || episode.prediction_status.as_deref() != Some("requested")
        {
            return Ok(crate::services::twin_events::MutationCommit {
                mutation_id: None,
                events: Vec::new(),
                authority_token: None,
                postcommit_warning: false,
            });
        }
        episode.prediction_status = Some("failed".to_string());
        episode.updated_at = Utc::now();
        self.write_decision_file(&episode)?;
        Ok(crate::services::twin_events::MutationCommit {
            mutation_id: None,
            events: Vec::new(),
            authority_token: None,
            postcommit_warning: false,
        })
    }

    fn constitution_citation_is_supported(&self, id: &str) -> bool {
        Self::validate_file_id(id).is_ok()
            && self
                .read_constitution_file(&self.constitution_file_path(id))
                .ok()
                .is_some_and(|item| {
                    !self.artifact_has_only_legacy_auto_support(&item.linked_record_ids)
                })
    }

    fn action_gap_citation_is_supported(&self, id: &str) -> bool {
        Self::validate_file_id(id).is_ok()
            && self
                .read_action_gap_file(&self.action_gap_file_path(id))
                .ok()
                .is_some_and(|gap| {
                    !self.artifact_has_only_legacy_auto_support(&gap.linked_record_ids)
                })
    }

    fn filter_artifact_citations(
        &self,
        constitution_ids: &mut Vec<String>,
        action_gap_ids: &mut Vec<String>,
        packet: &mut DecisionEvidencePacket,
    ) {
        constitution_ids.retain(|id| self.constitution_citation_is_supported(id));
        action_gap_ids.retain(|id| self.action_gap_citation_is_supported(id));
        packet
            .selected_sources
            .retain(|source| match source.source_type.as_str() {
                "constitution_item" => self.constitution_citation_is_supported(&source.id),
                "action_gap" => self.action_gap_citation_is_supported(&source.id),
                _ => true,
            });
    }

    fn overlay_legacy_artifacts_on_reflection(&self, card: &mut ReflectionCard) {
        self.filter_artifact_citations(
            &mut card.cited_constitution_item_ids,
            &mut card.cited_action_gap_ids,
            &mut card.evidence_packet,
        );
        let config = card
            .evidence_packet
            .config_snapshot
            .clone()
            .unwrap_or_default();
        card.scores = score_reflection_card(
            &card.content,
            &card.cited_note_ids,
            &card.cited_user_record_ids,
            &card.cited_constitution_item_ids,
            &card.cited_action_gap_ids,
            &config,
        );
    }

    pub fn record_reflection_card(
        &mut self,
        create: ReflectionCardCreate,
    ) -> Result<ReflectionCard> {
        self.record_reflection_card_internal(create, None)
            .map(|(card, _)| card)
    }

    pub(crate) fn record_reflection_card_expecting_authority(
        &mut self,
        create: ReflectionCardCreate,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(ReflectionCard, crate::services::twin_events::MutationCommit)> {
        self.record_reflection_card_internal(create, Some(expected))
    }

    fn record_reflection_card_internal(
        &mut self,
        create: ReflectionCardCreate,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<(ReflectionCard, crate::services::twin_events::MutationCommit)> {
        if !self.event_recorder.is_noop() {
            let recorder = self.event_recorder.clone();
            let mut committed = None;
            let mut planner = || {
                let (card, trace, values) = self
                    .plan_reflection_card_mutation(create.clone())
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                let targets = self.governed_json_targets(values).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                committed = Some((card, trace));
                let mut plan = crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::LocalOnly,
                    crate::models::twin_event::SourceChannel::parse("legacy_twin")
                        .map_err(crate::services::twin_events::MutationError::Invalid)?,
                    targets,
                    Vec::new(),
                );
                if let Some(expected) = expected.clone() {
                    plan = plan.expecting_authority(expected);
                }
                Ok(Some(plan))
            };
            let result = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            );
            let commit = self.finish_mutation_commit(result)?;
            let (card, trace) =
                committed.ok_or_else(|| anyhow::anyhow!("reflection card was not planned"))?;
            self.cache_committed_trace(trace);
            return Ok((card, commit));
        }
        let (card, trace, values) = self.plan_reflection_card_mutation(create)?;
        let commit = self.commit_governed_json_targets(values, Vec::new())?;
        self.cache_committed_trace(trace);
        Ok((card, commit))
    }

    fn plan_reflection_card_mutation(
        &mut self,
        mut create: ReflectionCardCreate,
    ) -> Result<ReflectionCardPlan> {
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
        self.filter_artifact_citations(
            &mut create.cited_constitution_item_ids,
            &mut create.cited_action_gap_ids,
            &mut evidence_packet,
        );
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

        let (_, trace) = self.plan_trace_event(
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
        let values = vec![
            (
                self.reflection_file_path(&card.id),
                serde_json::to_string_pretty(&card)?,
            ),
            self.serialized_trace_target(&trace)?,
        ];
        Ok((card, trace, values))
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
                    PromotionState::Endorsed => {
                        ("approved_record", weights.approved_records_weight)
                    }
                    PromotionState::Candidate => {
                        ("candidate_record", weights.candidate_records_weight)
                    }
                    PromotionState::AutoPromoted
                    | PromotionState::Rejected
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
            let Ok(item) = self.read_constitution_file(&self.constitution_file_path(id)) else {
                continue;
            };
            if self.artifact_has_only_legacy_auto_support(&item.linked_record_ids) {
                continue;
            }
            selected_sources.push(DecisionEvidenceSource {
                source_type: "constitution_item".to_string(),
                id: id.clone(),
                label: excerpt(&item.claim),
                weight: weights.constitution_weight,
                reason: "Higher-order constitution item selected for decision framing".to_string(),
            });
        }

        for id in cited_action_gap_ids {
            let Ok(gap) = self.read_action_gap_file(&self.action_gap_file_path(id)) else {
                continue;
            };
            if self.artifact_has_only_legacy_auto_support(&gap.linked_record_ids) {
                continue;
            }
            selected_sources.push(DecisionEvidenceSource {
                source_type: "action_gap".to_string(),
                id: id.clone(),
                label: excerpt(&gap.decision_risk),
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
        self.read_twin_json_bounded(path)?
            .ok_or_else(|| anyhow::anyhow!("Failed to read decision file: {}", path.display()))
    }

    fn write_decision_mirror_config_file(&self, config: &DecisionMirrorConfig) -> Result<()> {
        self.write_pretty_json(&self.decision_mirror_config_path, config)
            .map(|_commit| ())
    }

    fn write_decision_file(&self, episode: &DecisionEpisode) -> Result<()> {
        let path = self.decision_file_path(&episode.id);
        self.write_pretty_json(&path, episode).map(|_commit| ())
    }

    pub(super) fn list_reflection_cards(&self) -> Result<Vec<ReflectionCard>> {
        let mut cards = self.list_twin_json_bounded::<ReflectionCard>("reflections")?;
        for card in &mut cards {
            self.overlay_legacy_artifacts_on_reflection(card);
        }

        cards.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(cards)
    }

    pub(super) fn decision_evidence_refs(&self, decision_id: &str) -> Result<Vec<EvidenceRef>> {
        Self::validate_file_id(decision_id)?;
        let mut refs = Vec::new();
        for trace in self.list_session_traces_durable()? {
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
        let Some(trace) = self.read_twin_json_bounded::<SessionTrace>(&path)? else {
            return Ok(Vec::new());
        };
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
#[path = "decisions_tests.rs"]
mod tests;
