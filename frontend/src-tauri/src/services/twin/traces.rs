use super::shared::{
    excerpt, extract_event_model_id, extract_event_tile_id, first_payload_string,
    payload_string,
};
use super::TwinStore;
use crate::models::twin::{
    EvidenceRef, ResolvedEvidenceRef, SessionTrace, TraceEvent, TraceEventType,
};
#[cfg(test)]
use crate::models::twin::{RecordOrigin, UserRecordCreate};
use anyhow::Result;
use chrono::Utc;
#[cfg(test)]
use std::collections::HashMap;
use std::path::PathBuf;

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
        source_type: evidence_ref.source_type.clone(),
        source_id: evidence_ref.source_id.clone(),
        source_label: evidence_ref.source_label.clone(),
        excerpt: evidence_ref.excerpt.clone(),
        speaker_role: evidence_ref.speaker_role.clone(),
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

impl TwinStore {
    pub fn append_trace_event(
        &mut self,
        session_id: &str,
        event_type: TraceEventType,
        payload: serde_json::Value,
    ) -> Result<TraceEvent> {
        self.append_trace_event_internal(session_id, event_type, payload, None)
            .map(|(event, _)| event)
    }

    pub(crate) fn append_trace_event_expecting_authority(
        &mut self,
        session_id: &str,
        event_type: TraceEventType,
        payload: serde_json::Value,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(
        TraceEvent,
        crate::services::twin_events::MutationCommit,
    )> {
        self.append_trace_event_internal(session_id, event_type, payload, Some(expected))
    }

    fn append_trace_event_internal(
        &mut self,
        session_id: &str,
        event_type: TraceEventType,
        payload: serde_json::Value,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<(
        TraceEvent,
        crate::services::twin_events::MutationCommit,
    )> {
        if !self.event_recorder.is_noop() {
            Self::validate_file_id(session_id)?;
            let recorder = self.event_recorder.clone();
            let mut committed = None;
            let mut planner = || {
                let (event, trace) = self
                    .plan_trace_event(session_id, event_type.clone(), payload.clone())
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                let target = self
                    .serialized_trace_target(&trace)
                    .and_then(|value| self.governed_json_targets(vec![value]))
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                committed = Some((event, trace));
                let mut plan = crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::LocalOnly,
                    crate::models::twin_event::SourceChannel::parse("legacy_twin")
                        .map_err(crate::services::twin_events::MutationError::Invalid)?,
                    target,
                    Vec::new(),
                );
                if let Some(expected) = expected.clone() {
                    plan = plan.expecting_authority(expected);
                }
                Ok(Some(plan))
            };
            let commit = match recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            ) {
                Ok(commit) => commit,
                Err(error) => {
                    self.invalidate_mutation_caches();
                    return Err(anyhow::Error::new(error));
                }
            };
            let (event, trace) = committed
                .ok_or_else(|| anyhow::anyhow!("trace append was not planned"))?;
            self.cache_committed_trace(trace);
            return Ok((event, commit));
        }
        let (event, trace) = self.plan_trace_event(session_id, event_type, payload)?;
        self.write_trace_file(&trace)?;
        self.cache_committed_trace(trace);
        Ok((
            event,
            crate::services::twin_events::MutationCommit {
                mutation_id: None,
                events: Vec::new(),
                authority_token: None,
            },
        ))
    }

    pub(super) fn plan_trace_event(
        &self,
        session_id: &str,
        event_type: TraceEventType,
        payload: serde_json::Value,
    ) -> Result<(TraceEvent, SessionTrace)> {
        Self::validate_file_id(session_id)?;
        let now = Utc::now();
        let event = TraceEvent {
            id: uuid::Uuid::new_v4().to_string(),
            event_type,
            created_at: now,
            payload,
        };
        let path = self.trace_file_path(session_id);
        let mut trace = self
            .read_twin_json_bounded(&path)?
            .unwrap_or_else(|| SessionTrace::new(session_id));
        trace.updated_at = now;
        trace.events.push(event.clone());
        Ok((event, trace))
    }

    pub(super) fn serialized_trace_target(
        &self,
        trace: &SessionTrace,
    ) -> Result<(PathBuf, String)> {
        Ok((
            self.trace_file_path(&trace.session_id),
            serde_json::to_string_pretty(trace)?,
        ))
    }

    pub(crate) fn cache_committed_trace(&mut self, _trace: SessionTrace) {
        self.invalidate_mutation_caches();
    }

    pub fn get_session_trace(&mut self, session_id: &str) -> Result<SessionTrace> {
        Self::validate_file_id(session_id)?;
        self.ensure_trace_cache()?;
        if let Some(trace) = self.trace_cache.get(session_id) {
            return Ok(trace.clone());
        }
        Ok(SessionTrace::new(session_id))
    }

    pub(super) fn list_session_traces(&mut self) -> Result<Vec<SessionTrace>> {
        self.ensure_trace_cache()?;
        let mut traces = self.trace_cache.values().cloned().collect::<Vec<_>>();
        traces.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        Ok(traces)
    }

    pub(super) fn list_session_traces_durable(&self) -> Result<Vec<SessionTrace>> {
        let mut traces = self.list_twin_json_bounded("traces")?;
        traces.sort_by(|a: &SessionTrace, b: &SessionTrace| a.session_id.cmp(&b.session_id));
        Ok(traces)
    }

    fn ensure_trace_cache(&mut self) -> Result<()> {
        if self.traces_cache_ready {
            return Ok(());
        }
        let traces = self
            .list_session_traces_durable()?
            .into_iter()
            .map(|trace| (trace.session_id.clone(), trace))
            .collect();
        self.trace_cache = traces;
        self.traces_cache_ready = true;
        Ok(())
    }

    pub(super) fn resolve_evidence_refs(
        &mut self,
        refs: &[EvidenceRef],
    ) -> Result<Vec<ResolvedEvidenceRef>> {
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

    pub(super) fn resolve_evidence_refs_durable(
        &self,
        refs: &[EvidenceRef],
    ) -> Result<Vec<ResolvedEvidenceRef>> {
        let mut resolved = Vec::new();
        for evidence_ref in refs {
            let trace_id = if evidence_ref.trace_id.trim().is_empty() {
                &evidence_ref.session_id
            } else {
                &evidence_ref.trace_id
            };
            let Some(trace) = self
                .read_twin_json_bounded::<SessionTrace>(&self.trace_file_path(trace_id))?
            else {
                continue;
            };
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

    pub(super) fn trace_file_path(&self, session_id: &str) -> PathBuf {
        self.traces_path.join(format!("{}.json", session_id))
    }

    fn write_trace_file(&self, trace: &SessionTrace) -> Result<()> {
        let path = self.trace_file_path(&trace.session_id);
        self.write_pretty_json(&path, trace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin::{PromotionState, UserRecordKind};
    use tempfile::tempdir;

    fn coordinated_store(
        root: &std::path::Path,
    ) -> (
        TwinStore,
        std::sync::Arc<crate::services::twin_events::MutationCoordinator>,
    ) {
        let data = root.join("data");
        let vault = root.join("vault");
        let twin_root = data.join("twin").join("scope-one");
        std::fs::create_dir_all(&twin_root).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                events,
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        (
            TwinStore::with_event_recorder(
                twin_root,
                data.join("twin"),
                coordinator.clone(),
            ),
            coordinator,
        )
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
                    source_type: Some("behavior".to_string()),
                    source_id: None,
                    source_label: None,
                    excerpt: None,
                    speaker_role: None,
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

    #[test]
    fn interrupted_trace_append_is_recovered_before_the_next_fresh_plan() {
        let root = tempdir().unwrap();
        let (mut store, coordinator) = coordinated_store(root.path());
        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterStage);

        assert!(store
            .append_trace_event(
                "session-1",
                TraceEventType::PromptSubmitted,
                serde_json::json!({ "prompt": "first" }),
            )
            .is_err());
        let second = store
            .append_trace_event(
                "session-1",
                TraceEventType::PromptSubmitted,
                serde_json::json!({ "prompt": "second" }),
            )
            .unwrap();

        let trace = store.get_session_trace("session-1").unwrap();
        assert_eq!(trace.events.len(), 2);
        assert_eq!(trace.events[1].id, second.id);
        assert_eq!(coordinator.pending_count().unwrap(), 0);
    }
}
