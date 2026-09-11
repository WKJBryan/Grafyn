//! Vault- and person-scoped, source-grounded evidence with durable revision history.
mod context;
mod jobs;
mod models;
mod paths;
pub use context::{context_from_snapshot, without_goal_paths};
mod discovery;
mod embedding;
mod assessment;
pub mod benchmark;
pub use assessment::{assess_pair, assess_next, relationship_for_context, PairAssessment};
use crate::services::atomic_io::write_atomic;
use anyhow::{ensure, Context, Result};
use chrono::Utc;
pub use discovery::{discover_relationships, DiscoveryOutput};
pub use models::*;
use std::path::PathBuf;

const EXTRACTOR_VERSION: &str = "source-grounded-v1";

#[derive(Debug)]
pub struct EvidenceStore {
    root: PathBuf,
    state: EvidenceSnapshot,
}

impl EvidenceStore {
    pub fn new(root: PathBuf, subject_id: String, subject_name: String) -> Result<Self> {
        ensure!(!subject_id.trim().is_empty(), "Target person is required");
        std::fs::create_dir_all(&root)?;
        let path = root.join("evidence.json");
        let state: EvidenceSnapshot = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path)?)
                .context("Evidence file is invalid; original preserved")?
        } else {
            EvidenceSnapshot {
                schema_version: 1,
                subject_id: subject_id.clone(),
                subject_name,
                embedding_status: "pending: embeddinggemma has not been checked".into(),
                ..Default::default()
            }
        };
        ensure!(state.schema_version == 1, "Unsupported evidence schema");
        ensure!(
            state.subject_id == subject_id,
            "Evidence store belongs to another target person"
        );
        Ok(Self { root, state })
    }

    pub fn snapshot(&self) -> Result<EvidenceSnapshot> {
        let mut result = self.state.clone();
        assessment::refresh_flags(&mut result);
        Ok(result)
    }

    fn commit(&mut self, next: EvidenceSnapshot) -> Result<EvidenceSnapshot> {
        write_atomic(
            &self.root.join("evidence.json"),
            &serde_json::to_vec_pretty(&next)?,
        )?;
        self.state = next;
        self.snapshot()
    }

    pub fn save_interview(
        &mut self,
        mut draft: InterviewDraft,
        submit: bool,
    ) -> Result<EvidenceSnapshot> {
        ensure!(
            draft.subject_id == self.state.subject_id,
            "Interview target does not match this twin"
        );
        if draft.id.is_empty() {
            draft.id = uuid::Uuid::new_v4().to_string();
        }
        draft.updated_at = now();
        let mut next = self.state.clone();
        if !submit {
            next.interview_draft = Some(draft);
            return self.commit(next);
        }
        ensure!(!draft.situation.trim().is_empty() && !draft.chosen.trim().is_empty(), "A submitted case needs a situation and your actual choice; save an incomplete draft instead");
        let source_id = format!("interview:{}", draft.id);
        ensure!(
            matches!(
                draft.expected_goal_relation,
                None | Some(RelationshipKind::ContributesTo | RelationshipKind::Inhibits)
            ),
            "Interview goal effect must be contributes_to, inhibits, or unknown"
        );
        let text = format!("Situation: {}\nOptions: {}\nWanted: {}\nExpected: {}\nChosen: {}\nRejected: {}\nActual: {}\nRationale: {}\nConstraints: {}\nExpected effect on wanted goal: {}",
            draft.situation, draft.options.join("; "), draft.wanted, draft.expected, draft.chosen,
            draft.rejected.join("; "), draft.actual, draft.rationale, draft.constraints.join("; "),
            draft.expected_goal_relation.as_ref().map(|r| serde_json::to_string(r).unwrap_or_default()).unwrap_or_else(|| "unknown".into()));
        let input = SourceInput {
            id: source_id.clone(),
            title: "Guided decision interview".into(),
            text,
            subject_id: draft.subject_id.clone(),
            role: EvidenceRole::TargetStatement,
            source_group: source_id.clone(),
            ..Default::default()
        };
        let source = upsert_source(&mut next, input, true, &self.vault_id())?;
        let wanted_goal = paths::capture_interview_path(&mut next, &draft, &source)?;
        let id = format!("case:{}:{}", source_id, source.revision);
        if !next.cases.iter().any(|case| case.id == id) {
            let mut goal_revisions: Vec<GoalReference> = latest_goals(&next, None)
                .into_iter()
                .filter(|g| draft.goal_ids.contains(&g.input.id))
                .map(|g| GoalReference {
                    goal_id: g.input.id.clone(),
                    revision: g.revision,
                })
                .collect();
            if let Some(goal) = wanted_goal {
                if !goal_revisions.iter().any(|g| g.goal_id == goal.input.id) {
                    goal_revisions.push(GoalReference {
                        goal_id: goal.input.id,
                        revision: goal.revision,
                    });
                }
            }
            next.cases.push(DecisionCase {
                id,
                subject_id: draft.subject_id,
                domain: draft.domain,
                situation: draft.situation,
                options: draft.options,
                wanted: draft.wanted,
                expected: draft.expected,
                chosen: draft.chosen,
                rejected: draft.rejected,
                actual: draft.actual,
                rationale: draft.rationale,
                constraints: draft.constraints,
                goal_revisions,
                receipts: vec![whole_receipt(&source)],
                review_status: ReviewStatus::Tentative,
                provenance: "direct_interview".into(),
                case_kind: "observed_action".into(),
                recorded_at: now(),
                invalidated: false,
                conflict: false,
            });
        }
        next.interview_draft = None;
        self.commit(next)
    }

    /// Inventory is authoritative for note-backed sources only. Interview originals remain local.
    pub fn reconcile_sources(&mut self, sources: Vec<SourceInput>) -> Result<EvidenceSnapshot> {
        let mut next = self.state.clone();
        let incoming: std::collections::HashSet<_> = sources.iter().map(|s| s.id.clone()).collect();
        ensure!(
            incoming.len() == sources.len(),
            "Source inventory contains duplicate IDs"
        );
        let removed: Vec<_> = next
            .sources
            .iter()
            .filter(|s| !s.deleted && !s.interview && !incoming.contains(&s.input.id))
            .map(|s| s.input.id.clone())
            .collect();
        for id in removed {
            invalidate_source(&mut next, &id);
        }
        for source in sources {
            upsert_source(&mut next, source, false, &self.vault_id())?;
        }
        self.commit(next)
    }

    fn vault_id(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    pub fn save_goal(&mut self, mut input: GoalInput) -> Result<GoalRevision> {
        ensure!(
            input.subject_id == self.state.subject_id,
            "Goal target does not match this twin"
        );
        ensure!(
            !input.label.trim().is_empty() && !input.definition.trim().is_empty(),
            "A goal needs a label and definition"
        );
        if input.id.is_empty() {
            input.id = uuid::Uuid::new_v4().to_string();
        }
        if let Some(date) = &input.effective_at {
            validate_date(date)?;
        }
        for receipt in input
            .receipts
            .iter()
            .chain(input.criteria.iter().flat_map(|c| &c.receipts))
        {
            validate_receipt(&self.state, receipt, true)?;
        }
        let mut next = self.state.clone();
        let goal = append_goal(&mut next, input)?;
        self.commit(next)?;
        Ok(goal)
    }
    pub fn review_relationship(
        &mut self,
        id: &str,
        status: ReviewStatus,
        relation: Option<RelationshipKind>,
    ) -> Result<Relationship> {
        let mut next = self.state.clone();
        let index = next
            .relationships
            .iter()
            .position(|r| r.id == id)
            .context("Unknown relationship")?;
        let mut updated = next.relationships[index].clone();
        ensure!(
            !updated.invalidated,
            "Source revision changed; this relationship is historical"
        );
        if let Some(kind) = relation {
            updated.relation = kind;
            if matches!(updated.relation, RelationshipKind::Related | RelationshipKind::Equivalent | RelationshipKind::Conflicts | RelationshipKind::Contradicts) {
                updated.directed = false;
                updated.causal_basis = None;
            }
        }
        jobs::validate_relationship(&next, &updated)?;
        if status == ReviewStatus::Confirmed && updated.assessment.is_some() {
            updated.reviewed_context_hash = Some(assessment::review_fingerprint(&next,&updated)?);
        }
        updated.review_status = status;
        next.relationships[index] = updated.clone();
        self.commit(next)?;
        Ok(updated)
    }

    /// Review the interpretation without rewriting the source statement or its receipts.
    pub fn review_statement(&mut self, id: &str, status: ReviewStatus) -> Result<PersonalEvidence> {
        let mut next = self.state.clone();
        let index = next
            .statements
            .iter()
            .position(|statement| statement.id == id)
            .context("Unknown personal statement")?;
        ensure!(
            !next.statements[index].invalidated,
            "Source changed; review the current interpretation instead"
        );
        for receipt in &next.statements[index].receipts {
            validate_receipt(&next, receipt, true)?;
        }
        next.statements[index].review_status = status;
        let result = next.statements[index].clone();
        self.commit(next)?;
        Ok(result)
    }
}

fn append_goal(state: &mut EvidenceSnapshot, mut input: GoalInput) -> Result<GoalRevision> {
    ensure!(
        input.subject_id == state.subject_id,
        "Goal subject mismatch"
    );
    if input.id.is_empty() {
        let provenance = input
            .receipts
            .iter()
            .map(|r| format!("{}:{}:{}", r.source_id, r.source_revision, r.start))
            .collect::<Vec<_>>()
            .join("|");
        input.id = format!(
            "goal:{}",
            content_hash(&format!(
                "{}:{}:{}:{}",
                input.subject_id, input.label, input.scope, provenance
            ))
        );
    }
    let previous = state
        .goals
        .iter()
        .filter(|g| g.input.id == input.id)
        .max_by_key(|g| g.revision);
    if let Some(previous) = previous {
        if !previous.invalidated
            && serde_json::to_value(&previous.input)? == serde_json::to_value(&input)?
        {
            return Ok(previous.clone());
        }
    }
    let revision = previous.map_or(1, |g| g.revision + 1);
    let goal = GoalRevision {
        input,
        revision,
        recorded_at: now(),
        invalidated: false,
    };
    state.goals.push(goal.clone());
    Ok(goal)
}

fn validate_date(value: &str) -> Result<()> {
    ensure!(
        chrono::DateTime::parse_from_rfc3339(value).is_ok()
            || chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok(),
        "Use an ISO date or timestamp"
    );
    Ok(())
}

fn at_or_before(value: &str, cutoff: &str) -> bool {
    let parse = |value: &str| {
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|date| date.with_timezone(&Utc))
            .ok()
            .or_else(|| {
                chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                    .ok()
                    .and_then(|date| date.and_hms_opt(0, 0, 0))
                    .map(|date| date.and_utc())
            })
    };
    matches!((parse(value), parse(cutoff)), (Some(value), Some(cutoff)) if value <= cutoff)
}

fn validate_receipt<'a>(
    state: &'a EvidenceSnapshot,
    receipt: &Receipt,
    target: bool,
) -> Result<&'a SourceRecord> {
    let source = state
        .sources
        .iter()
        .find(|s| {
            s.input.id == receipt.source_id && s.revision == receipt.source_revision && !s.deleted
        })
        .context("Receipt references a missing or superseded source revision")?;
    ensure!(
        !source.input.restricted && !source.input.held_out,
        "Source is restricted or held out"
    );
    ensure!(
        !state
            .sources
            .iter()
            .any(|other| (other.input.restricted || other.input.held_out)
                && (other.input.source_group == source.input.source_group
                    || other.input.text == source.input.text)),
        "A duplicate or source-group member is restricted or held out"
    );
    ensure!(
        !receipt.quote.trim().is_empty()
            && source.input.text.get(receipt.start..receipt.end) == Some(receipt.quote.as_str()),
        "Exact quotation and UTF-8 byte offsets do not match source"
    );
    if target {
        ensure!(
            source.input.subject_id == state.subject_id
                && source.input.role == EvidenceRole::TargetStatement,
            "Personal evidence requires an attributed target statement"
        );
    }
    Ok(source)
}

/// Validate selected frozen receipts against live revisions and newly restricted duplicates.
pub fn validate_receipts(state: &EvidenceSnapshot, receipts: &[Receipt]) -> Result<()> {
    for receipt in receipts {
        validate_receipt(state, receipt, false)?;
    }
    Ok(())
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

/// Stable content key; validity additionally compares the exact original bytes.
fn content_hash(text: &str) -> String {
    let hash = text
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}")
}

fn whole_receipt(source: &SourceRecord) -> Receipt {
    Receipt {
        source_id: source.input.id.clone(),
        source_revision: source.revision,
        start: 0,
        end: source.input.text.len(),
        quote: source.input.text.clone(),
        locator: source.input.title.clone(),
    }
}

fn upsert_source(
    state: &mut EvidenceSnapshot,
    mut input: SourceInput,
    interview: bool,
    vault_id: &str,
) -> Result<SourceRecord> {
    ensure!(!input.id.trim().is_empty(), "Source ID is required");
    if input.source_group.is_empty() {
        input.source_group = content_hash(&input.text);
    }
    let previous = state
        .sources
        .iter()
        .filter(|s| s.input.id == input.id)
        .max_by_key(|s| s.revision)
        .cloned();
    if let Some(old) = &previous {
        if !old.deleted && source_identity(&old.input) == source_identity(&input) {
            return Ok(old.clone());
        }
        invalidate_source(state, &input.id);
    }
    let source = SourceRecord {
        content_hash: content_hash(&source_identity(&input).to_string()),
        input,
        revision: previous.map_or(1, |s| s.revision + 1),
        recorded_at: now(),
        deleted: false,
        interview,
    };
    if !source.input.restricted && !source.input.held_out && !source.input.text.trim().is_empty() {
        state.jobs.push(ProcessingJob {
            id: format!(
                "{}:{}:{}:{}",
                source.input.id, source.revision, source.input.mapping_revision, EXTRACTOR_VERSION
            ),
            source_id: source.input.id.clone(),
            source_revision: source.revision,
            mapping_revision: source.input.mapping_revision,
            extractor_version: EXTRACTOR_VERSION.into(),
            vault_id: vault_id.into(),
            status: JobStatus::Queued,
            error: None,
            attempts: 0,
            recorded_at: now(),
        });
    }
    state.sources.push(source.clone());
    Ok(source)
}

fn source_identity(input: &SourceInput) -> serde_json::Value {
    serde_json::json!({"text":input.text,"structured_case":input.structured_case,
        "subject_id":input.subject_id,"role":input.role,"mapping_revision":input.mapping_revision,
        "restricted":input.restricted,"held_out":input.held_out,"source_group":input.source_group})
}

fn invalidate_source(state: &mut EvidenceSnapshot, id: &str) {
    for statement in &mut state.statements {
        if statement.receipts.iter().any(|r| r.source_id == id) {
            statement.invalidated = true;
        }
    }
    for node in &mut state.nodes {
        if node.receipts.iter().any(|r| r.source_id == id) {
            node.invalidated = true;
        }
    }
    for source in &mut state.sources {
        if source.input.id == id {
            source.deleted = true;
        }
    }
    for case in &mut state.cases {
        if case.receipts.iter().any(|r| r.source_id == id) {
            case.invalidated = true;
        }
    }
    for goal in &mut state.goals {
        if goal
            .input
            .receipts
            .iter()
            .chain(goal.input.criteria.iter().flat_map(|c| &c.receipts))
            .any(|r| r.source_id == id)
        {
            goal.invalidated = true;
        }
    }
    for relation in &mut state.relationships {
        if relation.from_receipt.source_id == id || relation.to_receipt.source_id == id {
            relation.invalidated = true;
        }
    }
    for job in &mut state.jobs {
        if job.source_id == id {
            job.status = JobStatus::Superseded;
        }
    }
}

fn latest_goals<'a>(state: &'a EvidenceSnapshot, as_of: Option<&str>) -> Vec<&'a GoalRevision> {
    let current = now();
    let as_of = Some(as_of.unwrap_or(&current));
    let mut latest = std::collections::BTreeMap::new();
    for goal in &state.goals {
        if as_of.is_some_and(|date| {
            !at_or_before(&goal.recorded_at, date)
                || goal
                    .input
                    .effective_at
                    .as_deref()
                    .is_some_and(|effective| !at_or_before(effective, date))
        }) {
            continue;
        }
        let entry = latest.entry(&goal.input.id).or_insert(goal);
        if entry.revision < goal.revision {
            *entry = goal;
        }
    }
    latest.into_values().collect()
}

#[cfg(test)]
mod tests;
