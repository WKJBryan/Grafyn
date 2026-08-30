use super::constitution::normalize_key_text;
use super::shared::lexical_terms;
use super::TwinStore;
use super::{AUTO_PROMOTE_CONFIDENCE, AUTO_PROMOTE_SUPPORT_COUNT};
#[cfg(test)]
use crate::models::twin::{EvidenceRef, RecordOrigin, UserRecordCreate};
use crate::models::twin::{
    MemoryDigestAction, MemoryDigestItem, MemoryDigestReviewRequest, MemoryDigestState,
    PromotionState, UserRecord, UserRecordKind, UserRecordUpdate,
};
use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

const MAX_MEMORY_DIGEST_ITEMS: usize = 5;

type MemoryDigestReviewPlan = (
    MemoryDigestItem,
    Vec<UserRecord>,
    Vec<(std::path::PathBuf, String)>,
    crate::services::twin_events::TwinEventDraft,
);

fn memory_digest_trigger(record: &UserRecord) -> Option<&'static str> {
    if record.kind == UserRecordKind::Fact {
        return None;
    }

    match record.promotion_state {
        PromotionState::Rejected | PromotionState::Private | PromotionState::NoTrain => None,
        PromotionState::Candidate | PromotionState::AutoPromoted
            if record.evidence_refs.len() >= AUTO_PROMOTE_SUPPORT_COUNT =>
        {
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

fn memory_digest_action_label(action: &MemoryDigestAction) -> &'static str {
    match action {
        MemoryDigestAction::Keep => "keep",
        MemoryDigestAction::Soften => "soften",
        MemoryDigestAction::NotMe => "not_me",
        MemoryDigestAction::Private => "private",
        MemoryDigestAction::NoTrain => "no_train",
        MemoryDigestAction::Reject => "reject",
    }
}

impl TwinStore {
    pub fn list_memory_digest(&mut self) -> Result<Vec<MemoryDigestItem>> {
        if !self.event_recorder.is_noop() {
            let recorder = self.event_recorder.clone();
            let mut committed = None;
            let mut planner = || {
                let (all_items, pending) = self.plan_memory_digest_items().map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                let content = serde_json::to_string_pretty(&all_items).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                let targets = self
                    .governed_json_targets(vec![(self.digest_path.clone(), content)])
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                committed = Some(pending);
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
            self.finish_mutation_commit(result)?;
            let pending = committed.ok_or_else(|| anyhow::anyhow!("memory digest was not planned"))?;
            self.invalidate_mutation_caches();
            return Ok(pending);
        }
        let (all_items, pending) = self.plan_memory_digest_items()?;
        self.write_memory_digest_file(&all_items)?;
        self.invalidate_mutation_caches();
        Ok(pending)
    }

    fn plan_memory_digest_items(
        &self,
    ) -> Result<(Vec<MemoryDigestItem>, Vec<MemoryDigestItem>)> {
        let mut existing = self.read_memory_digest_file()?;
        let mut existing_by_id = existing
            .iter()
            .cloned()
            .map(|item| (item.id.clone(), item))
            .collect::<HashMap<_, _>>();
        let now = Utc::now();
        let records = self
            .list_user_records_durable()?
            .into_iter()
            .filter(|record| memory_digest_trigger(record).is_some())
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
                .resolve_evidence_refs_durable(&primary.evidence_refs)
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
        let pending = existing
            .iter()
            .filter(|item| item.state == MemoryDigestState::Pending)
            .take(MAX_MEMORY_DIGEST_ITEMS)
            .cloned()
            .collect();
        Ok((existing, pending))
    }

    pub fn review_memory_digest_item(
        &mut self,
        id: &str,
        request: MemoryDigestReviewRequest,
    ) -> Result<MemoryDigestItem> {
        Self::validate_file_id(id)?;
        if !self.event_recorder.is_noop() {
            let recorder = self.event_recorder.clone();
            let mut committed = None;
            let mut planner = || {
                let (item, records, values, draft) = self
                    .plan_memory_digest_review(id, &request)
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                let targets = self.governed_json_targets(values).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                committed = Some((item, records));
                Ok(Some(crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::SyncEligible,
                    crate::models::twin_event::SourceChannel::parse("legacy_twin")
                        .map_err(crate::services::twin_events::MutationError::Invalid)?,
                    targets,
                    vec![draft],
                )))
            };
            let result = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            );
            self.finish_mutation_commit(result)?;
            let (item, _records) = committed
                .ok_or_else(|| anyhow::anyhow!("memory digest review was not planned"))?;
            self.invalidate_mutation_caches();
            return Ok(item);
        }
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

        if !self.event_recorder.is_noop() {
            if item.record_ids.len() > 63 {
                anyhow::bail!("a digest review can update at most 63 records");
            }
            let mut updated_records = Vec::new();
            for record_id in &item.record_ids {
                let Ok(mut record) = self.get_user_record(record_id) else {
                    continue;
                };
                let next_state = match request.action {
                    MemoryDigestAction::Keep => PromotionState::Endorsed,
                    MemoryDigestAction::Soften => PromotionState::Candidate,
                    MemoryDigestAction::NotMe | MemoryDigestAction::Reject => {
                        PromotionState::Rejected
                    }
                    MemoryDigestAction::Private => PromotionState::Private,
                    MemoryDigestAction::NoTrain => PromotionState::NoTrain,
                };
                let previous_state = record.promotion_state.clone();
                record.promotion_state = next_state.clone();
                record.updated_at = item.updated_at;
                super::records::append_promotion_history(
                    &mut record.metadata,
                    &previous_state,
                    &next_state,
                    request.rationale.as_deref(),
                    false,
                );
                if next_state == PromotionState::Rejected {
                    record
                        .metadata
                        .insert("reverted_at".to_string(), json!(item.updated_at));
                    record.metadata.insert(
                        "revert_reason".to_string(),
                        json!(request.rationale.clone()),
                    );
                    record
                        .metadata
                        .insert("auto_promoted".to_string(), Value::Bool(false));
                }
                if request.action == MemoryDigestAction::Soften {
                    record.confidence = (record.confidence * 0.85).max(0.35);
                }
                updated_records.push(record);
            }

            let digest_content = serde_json::to_string_pretty(&items)?;
            let digest_after =
                crate::services::twin_events::digest_bytes(digest_content.as_bytes());
            let mut values = vec![(self.digest_path.clone(), digest_content)];
            let action = memory_digest_action_label(&request.action);
            let feedback_seed = format!("{}\0{}\0{}", item.id, action, item.updated_at);
            let feedback_id = format!(
                "feedback-{}",
                crate::services::twin_events::digest_bytes(feedback_seed.as_bytes()).as_str()
            );
            let governance = if request.action == MemoryDigestAction::Private {
                crate::services::twin_events::local_capture_governance(
                    crate::models::twin_event::Sensitivity::Restricted,
                )
            } else {
                crate::services::twin_events::standard_capture_governance()
            };
            let mut draft = crate::services::twin_events::feedback_draft(
                crate::services::twin_events::FeedbackDraft {
                    feedback_id: &feedback_id,
                    target_id: &item.id,
                    kind: action,
                    content: None,
                    rationale: request.rationale.as_deref(),
                    rank: None,
                    observed_at: item.updated_at,
                    source_channel: crate::models::twin_event::SourceChannel::parse("legacy_twin")
                        .map_err(anyhow::Error::msg)?,
                    governance,
                },
            )
            .map_err(anyhow::Error::msg)?;
            draft.evidence.push(crate::models::twin_event::EvidenceRef {
                evidence_type: crate::models::twin_event::EvidenceType::TwinRecord,
                source_id: crate::models::twin_event::Identifier::parse(&item.id)
                    .map_err(anyhow::Error::msg)?,
                digest: Some(digest_after),
            });
            for record in &updated_records {
                let content = serde_json::to_string_pretty(record)?;
                let digest = crate::services::twin_events::digest_bytes(content.as_bytes());
                values.push((self.record_file_path(&record.id), content));
                draft.evidence.push(crate::models::twin_event::EvidenceRef {
                    evidence_type: crate::models::twin_event::EvidenceType::TwinRecord,
                    source_id: crate::models::twin_event::Identifier::parse(&record.id)
                        .map_err(anyhow::Error::msg)?,
                    digest: Some(digest),
                });
            }
            draft.evidence.sort();
            self.commit_governed_json_targets(values, vec![draft])?;
            self.invalidate_mutation_caches();
            return Ok(item);
        }

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

    fn plan_memory_digest_review(
        &self,
        id: &str,
        request: &MemoryDigestReviewRequest,
    ) -> Result<MemoryDigestReviewPlan> {
        let mut items = self.read_memory_digest_file()?;
        if !items.iter().any(|item| item.id == id) {
            items = self.plan_memory_digest_items()?.0;
        }
        let item_index = items
            .iter()
            .position(|item| item.id == id)
            .ok_or_else(|| anyhow::anyhow!("Memory digest item not found: {}", id))?;
        let mut item = items[item_index].clone();
        item.state = memory_digest_state_for_action(&request.action);
        item.updated_at = Utc::now();
        items[item_index] = item.clone();
        if item.record_ids.len() > 63 {
            anyhow::bail!("a digest review can update at most 63 records");
        }

        let mut updated_records = Vec::new();
        for record_id in &item.record_ids {
            let Some(mut record) = self
                .read_twin_json_bounded::<UserRecord>(&self.record_file_path(record_id))?
            else {
                continue;
            };
            let next_state = match request.action {
                MemoryDigestAction::Keep => PromotionState::Endorsed,
                MemoryDigestAction::Soften => PromotionState::Candidate,
                MemoryDigestAction::NotMe | MemoryDigestAction::Reject => {
                    PromotionState::Rejected
                }
                MemoryDigestAction::Private => PromotionState::Private,
                MemoryDigestAction::NoTrain => PromotionState::NoTrain,
            };
            let previous_state = record.promotion_state.clone();
            record.promotion_state = next_state.clone();
            record.updated_at = item.updated_at;
            super::records::append_promotion_history(
                &mut record.metadata,
                &previous_state,
                &next_state,
                request.rationale.as_deref(),
                false,
            );
            if next_state == PromotionState::Rejected {
                record
                    .metadata
                    .insert("reverted_at".to_string(), json!(item.updated_at));
                record.metadata.insert(
                    "revert_reason".to_string(),
                    json!(request.rationale.clone()),
                );
                record
                    .metadata
                    .insert("auto_promoted".to_string(), Value::Bool(false));
            }
            if request.action == MemoryDigestAction::Soften {
                record.confidence = (record.confidence * 0.85).max(0.35);
            }
            updated_records.push(record);
        }

        let digest_content = serde_json::to_string_pretty(&items)?;
        let digest_after = crate::services::twin_events::digest_bytes(digest_content.as_bytes());
        let mut values = vec![(self.digest_path.clone(), digest_content)];
        let action = memory_digest_action_label(&request.action);
        let feedback_seed = format!("{}\0{}\0{}", item.id, action, item.updated_at);
        let feedback_id = format!(
            "feedback-{}",
            crate::services::twin_events::digest_bytes(feedback_seed.as_bytes()).as_str()
        );
        let governance = if request.action == MemoryDigestAction::Private {
            crate::services::twin_events::local_capture_governance(
                crate::models::twin_event::Sensitivity::Restricted,
            )
        } else {
            crate::services::twin_events::standard_capture_governance()
        };
        let mut draft = crate::services::twin_events::feedback_draft(
            crate::services::twin_events::FeedbackDraft {
                feedback_id: &feedback_id,
                target_id: &item.id,
                kind: action,
                content: None,
                rationale: request.rationale.as_deref(),
                rank: None,
                observed_at: item.updated_at,
                source_channel: crate::models::twin_event::SourceChannel::parse("legacy_twin")
                    .map_err(anyhow::Error::msg)?,
                governance,
            },
        )
        .map_err(anyhow::Error::msg)?;
        draft.evidence.push(crate::models::twin_event::EvidenceRef {
            evidence_type: crate::models::twin_event::EvidenceType::TwinRecord,
            source_id: crate::models::twin_event::Identifier::parse(&item.id)
                .map_err(anyhow::Error::msg)?,
            digest: Some(digest_after),
        });
        for record in &updated_records {
            let content = serde_json::to_string_pretty(record)?;
            let digest = crate::services::twin_events::digest_bytes(content.as_bytes());
            values.push((self.record_file_path(&record.id), content));
            draft.evidence.push(crate::models::twin_event::EvidenceRef {
                evidence_type: crate::models::twin_event::EvidenceType::TwinRecord,
                source_id: crate::models::twin_event::Identifier::parse(&record.id)
                    .map_err(anyhow::Error::msg)?,
                digest: Some(digest),
            });
        }
        draft.evidence.sort();
        Ok((item, updated_records, values, draft))
    }

    fn read_memory_digest_file(&self) -> Result<Vec<MemoryDigestItem>> {
        Ok(self
            .read_twin_json_bounded(&self.digest_path)?
            .unwrap_or_default())
    }

    fn write_memory_digest_file(&self, items: &[MemoryDigestItem]) -> Result<()> {
        self.write_pretty_json(&self.digest_path, &items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin::{PromotionState, UserRecordKind};
    use tempfile::tempdir;

    #[test]
    fn coordinated_digest_review_is_one_feedback_event_over_digest_and_records() {
        let root = tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        let twin_root = data.join("twin").join("scope-one");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                events.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store = TwinStore::with_event_recorder(twin_root, data.join("twin"), coordinator);
        let evidence = (0..3)
            .map(|index| EvidenceRef {
                trace_id: "trace-one".to_string(),
                event_id: format!("event-{index}"),
                session_id: "session-one".to_string(),
                tile_id: None,
                model_id: None,
                note: None,
                source_type: Some("behavior".to_string()),
                source_id: None,
                source_label: None,
                excerpt: None,
                speaker_role: None,
            })
            .collect();
        store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "Review evidence first".to_string(),
                origin: RecordOrigin::Inferred,
                evidence_refs: evidence,
                confidence: 0.8,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .unwrap();
        let item = store.list_memory_digest().unwrap().remove(0);
        assert_eq!(events.ordered_events().unwrap().len(), 1);
        store
            .review_memory_digest_item(
                &item.id,
                MemoryDigestReviewRequest {
                    action: MemoryDigestAction::Keep,
                    rationale: Some("This is me".to_string()),
                },
            )
            .unwrap();
        let captured = events.ordered_events().unwrap();
        assert_eq!(captured.len(), 2);
        let crate::models::twin_event::TwinEventPayload::FeedbackRecorded(feedback) =
            &captured[1].payload
        else {
            panic!("digest review needs one explicit feedback event");
        };
        assert_eq!(feedback.target_id.as_str(), item.id);
        assert_eq!(feedback.kind.as_str(), "keep");
        assert_eq!(feedback.content, None);
        assert_eq!(feedback.rationale.as_ref().unwrap().as_str(), "This is me");
    }

    #[test]
    fn legacy_auto_promoted_is_only_a_pending_review_trigger() {
        let temp_dir = tempdir().expect("temp dir should be created");
        let mut store = TwinStore::new(temp_dir.path().to_path_buf());
        let mut record = store
            .create_user_record(UserRecordCreate {
                kind: UserRecordKind::Preference,
                content: "legacy review pattern".to_string(),
                origin: RecordOrigin::Inferred,
                evidence_refs: Vec::new(),
                confidence: 1.0,
                promotion_state: Some(PromotionState::Candidate),
                valid_from: None,
                valid_until: None,
                links: Vec::new(),
                metadata: HashMap::new(),
            })
            .unwrap();
        record.promotion_state = PromotionState::AutoPromoted;
        let evidence = EvidenceRef {
            trace_id: "trace".to_string(),
            event_id: "event".to_string(),
            session_id: "session".to_string(),
            tile_id: None,
            model_id: None,
            note: None,
            source_type: Some("behavior".to_string()),
            source_id: None,
            source_label: None,
            excerpt: None,
            speaker_role: None,
        };
        record.evidence_refs = vec![evidence; AUTO_PROMOTE_SUPPORT_COUNT];
        assert_eq!(
            memory_digest_trigger(&record),
            Some("candidate pattern has enough evidence for review")
        );
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
}
