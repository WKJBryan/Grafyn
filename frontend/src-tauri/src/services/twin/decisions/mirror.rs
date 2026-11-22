use super::super::shared::lexical_terms;
use super::super::TwinStore;
use crate::models::twin::{
    DecisionEpisode, DecisionMirrorConfig, DecisionMirrorConfigUpdate, DecisionMirrorWeights,
};
use anyhow::{Context, Result};
use std::collections::HashSet;

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

pub(super) fn clamp_weight(weight: f32) -> f32 {
    if weight.is_finite() {
        weight.clamp(0.0, 3.0)
    } else {
        1.0
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

    fn write_decision_mirror_config_file(&self, config: &DecisionMirrorConfig) -> Result<()> {
        self.write_pretty_json(&self.decision_mirror_config_path, config)
    }
}
