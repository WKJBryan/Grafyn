//! Decision episodes: capture, freeze/void, outcomes, and storage.
//!
//! - `prediction` -- sealed-prediction parsing, sanitizing, and agreement
//! - `mirror` -- Decision Mirror config/weights and past-case selection
//! - `reflections` -- reflection cards, scoring, and evidence packets

mod mirror;
mod prediction;
mod reflections;
#[cfg(test)]
mod tests;

pub use prediction::parse_twin_prediction;
use prediction::{compute_agreement, match_option_index};

use super::shared::{
    extract_event_model_id, extract_event_tile_id, load_or_quarantine, payload_string,
    sealed_payload_hash,
};
use super::TwinStore;
use crate::models::twin::{
    DecisionEpisode, DecisionEpisodeCreate, DecisionOutcomeUpdate, EvidenceRef, ReflectionCard,
    SessionTrace, TraceEventType, TwinPrediction, TwinPredictionDraft,
};
#[cfg(test)]
use crate::models::twin::{
    DecisionMirrorConfigUpdate, DecisionMirrorPreset, PrimitiveDecisionAssessment,
    ReflectionCardCreate,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::json;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

impl TwinStore {
    pub fn record_decision_episode(
        &mut self,
        create: DecisionEpisodeCreate,
    ) -> Result<DecisionEpisode> {
        Self::validate_file_id(&create.id)?;
        Self::validate_file_id(&create.session_id)?;
        Self::validate_file_id(&create.tile_id)?;
        // Capture metadata degrades, it never blocks: an invalid repeat_of
        // (bad id, self-reference, or missing episode) is dropped with a
        // warning instead of failing the episode — and with it the prompt.
        let repeat_of = create
            .repeat_of
            .as_deref()
            .filter(|repeat_of| {
                let valid = Self::validate_file_id(repeat_of).is_ok()
                    && *repeat_of != create.id
                    && self.decision_file_path(repeat_of).exists();
                if !valid {
                    log::warn!(
                        "Dropping invalid repeat_of {repeat_of:?} on decision episode {}",
                        create.id
                    );
                }
                valid
            })
            .map(str::to_string);
        // Re-recording an existing id must not erase a sealed prediction.
        let previous_path = self.decision_file_path(&create.id);
        let previous = if previous_path.exists() {
            self.read_decision_file(&previous_path).ok()
        } else {
            None
        }
        .filter(|previous| {
            previous.twin_prediction.is_some() || !previous.shadow_predictions.is_empty()
        });

        let now = Utc::now();
        // A prediction is only attempted when there are at least two options
        // to choose between; record why one will not arrive otherwise.
        let prediction_status = if create.options.len() >= 2 {
            Some("requested".to_string())
        } else {
            None
        };
        let mut episode = DecisionEpisode {
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
            shadow_predictions: Vec::new(),
            prediction_status,
            agreement: None,
            correction_note: None,
            context_version: create.context_version,
            outcome_recorded_at: None,
            twin_answer_rendered_at: None,
            twin_answer_visible_before_choice: None,
            repeat_of,
            created_at: now,
            updated_at: now,
        };
        if let Some(previous) = previous {
            episode.twin_prediction = previous.twin_prediction;
            episode.shadow_predictions = previous.shadow_predictions;
            episode.prediction_status = previous.prediction_status;
            self.void_if_payload_drifted(&mut episode)?;
        }

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
                "repeat_of": episode.repeat_of,
            }),
        )?;

        Ok(episode)
    }

    /// Freeze rule: a sealed prediction whose decision/options no longer hash
    /// to the sealed payload is voided (kept for audit, never scored). Legacy
    /// seals without a hash cannot be checked and are left alone. Caller writes.
    fn void_if_payload_drifted(&mut self, episode: &mut DecisionEpisode) -> Result<()> {
        if episode.prediction_status.as_deref() != Some("sealed") {
            return Ok(());
        }
        let Some(sealed_hash) = episode
            .twin_prediction
            .as_ref()
            .and_then(|prediction| prediction.sealed_payload_hash.clone())
        else {
            return Ok(());
        };
        let current_hash = sealed_payload_hash(&episode.decision, &episode.options);
        if current_hash == sealed_hash {
            return Ok(());
        }
        episode.prediction_status = Some("voided".to_string());
        episode.agreement = None;
        self.append_trace_event(
            &episode.session_id,
            TraceEventType::TwinPredictionVoided,
            json!({
                "decision_episode_id": episode.id,
                "tile_id": episode.tile_id,
                "reason": "decision_or_options_changed_after_seal",
                "sealed_payload_hash": sealed_hash,
                "current_payload_hash": current_hash,
            }),
        )?;
        Ok(())
    }

    /// Stamp when the first visible answer on the decision tile finished
    /// streaming. First stamp wins.
    pub fn mark_twin_answer_rendered(
        &mut self,
        episode_id: &str,
        rendered_at: DateTime<Utc>,
    ) -> Result<()> {
        Self::validate_file_id(episode_id)?;
        let path = self.decision_file_path(episode_id);
        let mut episode = self.read_decision_file(&path)?;
        if episode.twin_answer_rendered_at.is_some() {
            return Ok(());
        }
        episode.twin_answer_rendered_at = Some(rendered_at);
        episode.updated_at = Utc::now();
        self.write_decision_file(&episode)
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
            if episode.chosen_option.is_none() {
                // First recorded choice: was the visible answer already on screen?
                let choice_at = Utc::now();
                episode.twin_answer_visible_before_choice = episode
                    .twin_answer_rendered_at
                    .map(|rendered_at| rendered_at < choice_at);
            }
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
        // sealed before the outcome was first recorded count; voided seals never do.
        self.void_if_payload_drifted(&mut episode)?;
        let voided = episode.prediction_status.as_deref() == Some("voided");
        if voided {
            episode.agreement = None;
        } else if let (Some(chosen), Some(prediction)) =
            (&episode.chosen_option, &episode.twin_prediction)
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
    #[allow(clippy::too_many_arguments)]
    pub fn attach_twin_prediction(
        &mut self,
        episode_id: &str,
        draft: TwinPredictionDraft,
        model_id: &str,
        context_version: &str,
        elicitation_prompt: Option<&str>,
        temperature: Option<f32>,
        reasoning_effort: Option<&str>,
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

        let parse_mode = draft.parse_mode.clone();
        let prediction = seal_prediction(
            &episode,
            draft,
            model_id,
            context_version,
            elicitation_prompt,
            temperature,
            reasoning_effort,
            None,
        );
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
                "parse_mode": parse_mode,
                "sealed_at": sealed_at,
            }),
        )?;

        Ok(episode)
    }

    /// Seal a comparator ("shadow") prediction alongside the primary. Same
    /// stamping as `attach_twin_prediction`, but appended to
    /// `shadow_predictions` under `variant`. Refuses (logged no-op) after the
    /// outcome is recorded or when the variant is already sealed. Never
    /// touches the primary, `prediction_status`, or `agreement`.
    #[allow(clippy::too_many_arguments)]
    pub fn attach_shadow_prediction(
        &mut self,
        episode_id: &str,
        variant: &str,
        draft: TwinPredictionDraft,
        model_id: &str,
        context_version: &str,
        elicitation_prompt: Option<&str>,
        temperature: Option<f32>,
        reasoning_effort: Option<&str>,
    ) -> Result<DecisionEpisode> {
        Self::validate_file_id(episode_id)?;
        let path = self.decision_file_path(episode_id);
        let mut episode = self.read_decision_file(&path)?;

        if episode.chosen_option.is_some() {
            log::warn!(
                "Shadow prediction {variant} for episode {episode_id} arrived after the outcome was recorded; discarding"
            );
            return Ok(episode);
        }
        if episode
            .shadow_predictions
            .iter()
            .any(|shadow| shadow.variant.as_deref() == Some(variant))
        {
            log::warn!(
                "Episode {episode_id} already has a sealed {variant} shadow prediction; ignoring duplicate"
            );
            return Ok(episode);
        }

        let parse_mode = draft.parse_mode.clone();
        let prediction = seal_prediction(
            &episode,
            draft,
            model_id,
            context_version,
            elicitation_prompt,
            temperature,
            reasoning_effort,
            Some(variant),
        );
        let sealed_at = prediction.sealed_at;
        episode.shadow_predictions.push(prediction);
        episode.updated_at = Utc::now();
        self.write_decision_file(&episode)?;

        self.append_trace_event(
            &episode.session_id,
            TraceEventType::TwinPredictionSealed,
            json!({
                "decision_episode_id": episode.id,
                "tile_id": episode.tile_id,
                "variant": variant,
                "model_id": model_id,
                "context_version": context_version,
                "parse_mode": parse_mode,
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

    fn decision_file_path(&self, decision_id: &str) -> PathBuf {
        self.decisions_path.join(format!("{}.json", decision_id))
    }

    fn read_decision_file(&self, path: &Path) -> Result<DecisionEpisode> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read decision file: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse decision file: {}", path.display()))
    }

    fn write_decision_file(&self, episode: &DecisionEpisode) -> Result<()> {
        let path = self.decision_file_path(&episode.id);
        self.write_pretty_json(&path, episode)
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
}

/// Stamp a parsed draft into a sealed prediction (shared by primary and shadows).
#[allow(clippy::too_many_arguments)]
fn seal_prediction(
    episode: &DecisionEpisode,
    draft: TwinPredictionDraft,
    model_id: &str,
    context_version: &str,
    elicitation_prompt: Option<&str>,
    temperature: Option<f32>,
    reasoning_effort: Option<&str>,
    variant: Option<&str>,
) -> TwinPrediction {
    TwinPrediction {
        predicted_option: draft.predicted_option,
        matched_option_index: draft.matched_option_index,
        confidence: draft.confidence,
        rationale: draft.rationale,
        parse_mode: draft.parse_mode,
        model_id: model_id.to_string(),
        context_version: context_version.to_string(),
        sealed_at: Utc::now(),
        option_probabilities: draft.option_probabilities,
        elicitation_prompt: elicitation_prompt.map(str::to_string),
        temperature,
        // Freeze the question + options exactly as stored at seal time.
        sealed_payload_hash: Some(sealed_payload_hash(&episode.decision, &episode.options)),
        variant: variant.map(str::to_string),
        reasoning_effort: reasoning_effort.map(str::to_string),
    }
}
