#[cfg(test)]
use super::decisions::parse_twin_prediction;
use super::TwinStore;
#[cfg(test)]
use crate::models::twin::{
    DecisionEpisodeCreate, DecisionOutcomeUpdate, EvidenceRef, PrimitiveDecisionAssessment,
    RecordOrigin, UserRecordCreate,
};
use crate::models::twin::{
    ExportBundle, ExportFileSummary, PromotionState, TraceEventType, TwinExportRequest, UserRecord,
};
use crate::services::atomic_io::write_atomic;
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::Path;

const DEFAULT_EVAL_PERCENTAGE: u8 = 10;
const DEFAULT_HOLDOUT_PERCENTAGE: u8 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportSplit {
    Train,
    Eval,
    Holdout,
}

impl TwinStore {
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
    use crate::models::twin::{default_record_confidence, PromotionState, UserRecordKind};
    use tempfile::tempdir;

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

    fn prediction_options() -> Vec<String> {
        vec![
            "Take the Denver job".to_string(),
            "Stay in Austin".to_string(),
        ]
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
