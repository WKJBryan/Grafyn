use super::super::shared::{excerpt, extract_event_tile_id, payload_string};
use super::super::TwinStore;
use super::mirror::clamp_weight;
use crate::models::twin::{
    ConstitutionStatus, DecisionEpisode, DecisionEpisodeWithReflections, DecisionEvidencePacket,
    DecisionEvidenceSource, DecisionMirrorConfig, PromotionState, ReflectionCard,
    ReflectionCardCreate, ReflectionScores, TraceEvent, TraceEventType,
};
use anyhow::Result;
use chrono::Utc;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

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

impl TwinStore {
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
                let prediction_sealed = episode.redact_sealed_predictions();
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

    fn reflection_file_path(&self, reflection_id: &str) -> PathBuf {
        self.reflections_path
            .join(format!("{}.json", reflection_id))
    }

    fn write_reflection_file(&self, card: &ReflectionCard) -> Result<()> {
        let path = self.reflection_file_path(&card.id);
        self.write_pretty_json(&path, card)
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
