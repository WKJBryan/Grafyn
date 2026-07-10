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
use super::shared::{lexical_terms};
use super::{AUTO_PROMOTE_CONFIDENCE, AUTO_PROMOTE_SUPPORT_COUNT};
use super::constitution::normalize_key_text;

const MAX_MEMORY_DIGEST_ITEMS: usize = 5;

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


impl TwinStore {
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

}
