use crate::models::twin::{
    ActionGap, ActionGapCreate, ConstitutionInferenceSummary, ConstitutionItem,
    ConstitutionItemCreate, ConstitutionItemUpdate, ConstitutionReviewRequest, ConstitutionSetup,
    ConstitutionStatus, DecisionEpisode, DecisionEpisodeCreate, DecisionEpisodeWithReflections,
    DecisionEvidencePacket, DecisionEvidenceSource, DecisionMirrorConfig,
    DecisionMirrorConfigUpdate, DecisionMirrorPreset, DecisionMirrorWeights, DecisionOutcomeUpdate,
    EvidenceRef, ExportBundle, ExportFileSummary, MemoryDigestAction, MemoryDigestItem,
    MemoryDigestReviewRequest, MemoryDigestState, PrimitiveDecisionAssessment, PromotionState,
    RecordOrigin, ReflectionCard, ReflectionCardCreate, ReflectionScores, ResolvedEvidenceRef,
    SessionTrace, TraceEvent, TraceEventType, TwinContextRecord, TwinExportRequest,
    TwinInferenceRunSummary, TwinReviewRecord, UserRecord, UserRecordCreate, UserRecordKind,
    UserRecordUpdate,
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
const MAX_MEMORY_DIGEST_ITEMS: usize = 5;

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

    pub fn record_decision_episode(
        &mut self,
        create: DecisionEpisodeCreate,
    ) -> Result<DecisionEpisode> {
        Self::validate_file_id(&create.id)?;
        Self::validate_file_id(&create.session_id)?;
        Self::validate_file_id(&create.tile_id)?;

        let now = Utc::now();
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
                episodes.push(self.read_decision_file(path)?);
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
            episode.chosen_option = Some(chosen_option);
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
            }),
        )?;

        Ok(episode)
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
            .map(|episode| {
                let mut reflection_cards = cards_by_episode.remove(&episode.id).unwrap_or_default();
                reflection_cards.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                let feedback_events = self.decision_feedback_events(&episode).unwrap_or_default();
                DecisionEpisodeWithReflections {
                    episode,
                    reflection_cards,
                    feedback_events,
                }
            })
            .collect::<Vec<_>>();
        episodes.sort_by(|a, b| b.episode.updated_at.cmp(&a.episode.updated_at));
        Ok(episodes)
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
                items.push(self.read_constitution_file(path)?);
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
                gaps.push(self.read_action_gap_file(path)?);
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
        };

        self.seed_constitution_setup_items(&setup, evidence_ref)?;
        Ok(setup)
    }

    pub fn run_constitution_inference(&mut self) -> Result<ConstitutionInferenceSummary> {
        self.ensure_record_cache()?;
        let records = self
            .record_cache
            .values()
            .filter(|record| constitution_allows_record(record))
            .cloned()
            .collect::<Vec<_>>();
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

        for record in &records {
            let dimension = infer_constitution_dimension(&record.content, &record.kind);
            let claim = record.content.trim().to_string();
            if claim.is_empty() {
                continue;
            }
            let key = constitution_key(&dimension, &claim);
            if item_keys.insert(key) {
                self.create_constitution_item(ConstitutionItemCreate {
                    claim,
                    dimension,
                    scope: vec!["general".to_string()],
                    priority: record.confidence,
                    confidence: record.confidence,
                    status: ConstitutionStatus::Candidate,
                    evidence_refs: record.evidence_refs.clone(),
                    tensions: Vec::new(),
                    linked_record_ids: vec![record.id.clone()],
                    source: Some("constitution_inference".to_string()),
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

        let summary = ConstitutionInferenceSummary {
            scanned_records: records.len(),
            scanned_decisions: decisions.len(),
            created_constitution_items,
            created_action_gaps,
            generated_at: Utc::now(),
        };
        self.append_trace_event(
            "constitution-inference",
            TraceEventType::ConstitutionInferenceRun,
            serde_json::to_value(&summary)?,
        )?;
        Ok(summary)
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

    fn decision_file_path(&self, decision_id: &str) -> PathBuf {
        self.decisions_path.join(format!("{}.json", decision_id))
    }

    fn reflection_file_path(&self, reflection_id: &str) -> PathBuf {
        self.reflections_path
            .join(format!("{}.json", reflection_id))
    }

    fn constitution_file_path(&self, item_id: &str) -> PathBuf {
        self.constitution_path.join(format!("{}.json", item_id))
    }

    fn action_gap_file_path(&self, gap_id: &str) -> PathBuf {
        self.action_gaps_path.join(format!("{}.json", gap_id))
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

    fn read_decision_file(&self, path: &Path) -> Result<DecisionEpisode> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read decision file: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse decision file: {}", path.display()))
    }

    fn read_reflection_file(&self, path: &Path) -> Result<ReflectionCard> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read reflection file: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse reflection file: {}", path.display()))
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

    fn write_decision_mirror_config_file(&self, config: &DecisionMirrorConfig) -> Result<()> {
        self.write_pretty_json(&self.decision_mirror_config_path, config)
    }

    fn write_trace_file(&self, trace: &SessionTrace) -> Result<()> {
        let path = self.trace_file_path(&trace.session_id);
        self.write_pretty_json(&path, trace)
    }

    fn write_record_file(&self, record: &UserRecord) -> Result<()> {
        let path = self.record_file_path(&record.id);
        self.write_pretty_json(&path, record)
    }

    fn write_decision_file(&self, episode: &DecisionEpisode) -> Result<()> {
        let path = self.decision_file_path(&episode.id);
        self.write_pretty_json(&path, episode)
    }

    fn write_reflection_file(&self, card: &ReflectionCard) -> Result<()> {
        let path = self.reflection_file_path(&card.id);
        self.write_pretty_json(&path, card)
    }

    fn write_constitution_file(&self, item: &ConstitutionItem) -> Result<()> {
        let path = self.constitution_file_path(&item.id);
        self.write_pretty_json(&path, item)
    }

    fn write_action_gap_file(&self, gap: &ActionGap) -> Result<()> {
        let path = self.action_gap_file_path(&gap.id);
        self.write_pretty_json(&path, gap)
    }

    fn write_memory_digest_file(&self, items: &[MemoryDigestItem]) -> Result<()> {
        self.write_pretty_json(&self.digest_path, &items)
    }

    fn list_reflection_cards(&self) -> Result<Vec<ReflectionCard>> {
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
                cards.push(self.read_reflection_file(path)?);
            }
        }

        cards.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(cards)
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

    fn decision_evidence_refs(&self, decision_id: &str) -> Result<Vec<EvidenceRef>> {
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
            let trace = self.read_trace_file(path)?;
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

fn clean_setup_entries(entries: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty())
        .filter(|entry| seen.insert(entry.to_lowercase()))
        .collect()
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
                        },
                        EvidenceRef {
                            trace_id: "session-1".to_string(),
                            event_id: format!("event-{}-2", index),
                            session_id: "session-1".to_string(),
                            tile_id: Some(format!("tile-{}", index)),
                            model_id: None,
                            note: None,
                        },
                        EvidenceRef {
                            trace_id: "session-1".to_string(),
                            event_id: format!("event-{}-3", index),
                            session_id: "session-1".to_string(),
                            tile_id: Some(format!("tile-{}", index)),
                            model_id: None,
                            note: None,
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
                        },
                        EvidenceRef {
                            trace_id: "session-1".to_string(),
                            event_id: format!("cluster-event-{}-2", index),
                            session_id: "session-1".to_string(),
                            tile_id: Some(format!("cluster-tile-{}", index)),
                            model_id: None,
                            note: None,
                        },
                        EvidenceRef {
                            trace_id: "session-1".to_string(),
                            event_id: format!("cluster-event-{}-3", index),
                            session_id: "session-1".to_string(),
                            tile_id: Some(format!("cluster-tile-{}", index)),
                            model_id: None,
                            note: None,
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
