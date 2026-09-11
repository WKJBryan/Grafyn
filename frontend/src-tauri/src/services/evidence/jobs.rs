use super::*;

impl EvidenceStore {
    pub fn start_job(&mut self, id: &str) -> Result<ProcessingJob> {
        ensure!(
            !self
                .state
                .jobs
                .iter()
                .any(|j| j.status == JobStatus::Processing),
            "Another extraction request is already in flight"
        );
        let mut next = self.state.clone();
        let job = next
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .context("Unknown extraction job")?;
        ensure!(
            matches!(
                job.status,
                JobStatus::Queued | JobStatus::Failed | JobStatus::NeedsReview
            ),
            "Job is not available for processing"
        );
        job.status = JobStatus::Processing;
        job.attempts += 1;
        job.error = None;
        let result = job.clone();
        self.commit(next)?;
        Ok(result)
    }

    pub fn fail_job(&mut self, id: &str, error: String) -> Result<()> {
        let mut next = self.state.clone();
        let job = next
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .context("Unknown extraction job")?;
        if job.status == JobStatus::Superseded {
            return Ok(());
        }
        job.status = JobStatus::Failed;
        job.error = Some(error);
        self.commit(next)?;
        Ok(())
    }

    /// Call once after startup while holding the vault worker lock, never on every store load.
    pub fn recover_jobs(&mut self) -> Result<()> {
        let mut next = self.state.clone();
        for job in &mut next.jobs {
            if job.status == JobStatus::Processing {
                job.status = JobStatus::Queued;
                job.error = Some("Interrupted processing; ready to resume".into());
            }
        }
        self.commit(next)?;
        Ok(())
    }

    pub fn process_job(&mut self, id: &str, output: ExtractionOutput) -> Result<EvidenceSnapshot> {
        let job = self
            .state
            .jobs
            .iter()
            .find(|j| j.id == id)
            .context("Unknown extraction job")?
            .clone();
        ensure!(
            job.vault_id == self.vault_id() && job.status != JobStatus::Superseded,
            "Extraction results belong to a stale vault or source revision"
        );
        let source = self
            .state
            .sources
            .iter()
            .find(|s| s.input.id == job.source_id && !s.deleted)
            .context("Source was removed")?;
        ensure!(
            source.revision == job.source_revision
                && source.input.mapping_revision == job.mapping_revision
                && !source.input.restricted
                && !source.input.held_out,
            "Source or speaker mapping changed while extracting"
        );
        if job.status == JobStatus::Completed {
            return self.snapshot();
        }
        ensure!(
            output.cases.len() <= 32
                && output.goals.len() <= 32
                && output.relationships.len() <= 64,
            "Extraction batch exceeds bounded proposal limits"
        );
        let mut next = self.state.clone();
        ensure!(
            output.nodes.len() <= 64 && output.statements.len() <= 64,
            "Extraction batch exceeds evidence limit"
        );
        for mut statement in output.statements {
            ensure!(
                statement.subject_id == next.subject_id
                    && !statement.statement.trim().is_empty()
                    && !statement.receipts.is_empty(),
                "Personal statements require the target identity and exact source receipts"
            );
            ensure!(
                matches!(
                    statement.kind.as_str(),
                    "statement" | "preference" | "constraint" | "decision_procedure"
                ),
                "Unknown personal evidence kind"
            );
            ensure!(
                statement
                    .receipts
                    .iter()
                    .any(|r| r.source_id == job.source_id),
                "Personal statement does not cite this processing source"
            );
            for receipt in &statement.receipts {
                validate_receipt(&next, receipt, true)?;
            }
            if next.statements.iter().any(|previous| {
                !previous.invalidated
                    && previous.review_status == ReviewStatus::Rejected
                    && previous.receipts.iter().any(|old| {
                        statement.receipts.iter().any(|receipt| {
                            old.source_id == receipt.source_id
                                && old.source_revision == receipt.source_revision
                                && old.start == receipt.start
                                && old.end == receipt.end
                        })
                    })
            }) {
                continue;
            }
            statement.id = format!(
                "statement:{}",
                content_hash(&format!(
                    "{}:{}:{}:{}",
                    statement.subject_id, job.source_id, job.source_revision, statement.statement
                ))
            );
            statement.provenance = "semantic_extraction".into();
            statement.review_status = ReviewStatus::Tentative;
            statement.recorded_at = now();
            statement.invalidated = false;
            if !next
                .statements
                .iter()
                .any(|s| s.id == statement.id && !s.invalidated)
            {
                next.statements.push(statement);
            }
        }
        for mut node in output.nodes {
            ensure!(
                node.subject_id == next.subject_id
                    && !node.id.trim().is_empty()
                    && !node.label.trim().is_empty()
                    && !node.receipts.is_empty(),
                "Effect nodes require identity, subject, label, and receipts"
            );
            ensure!(
                node.receipts.iter().any(|r| r.source_id == job.source_id),
                "Effect node does not cite this processing source"
            );
            for receipt in &node.receipts {
                validate_receipt(&next, receipt, true)?;
            }
            ensure!(
                node.receipts.iter().any(|r| r.quote.contains(&node.label)),
                "Effect node label is absent from quoted evidence"
            );
            node.review_status = ReviewStatus::Tentative;
            node.recorded_at = now();
            node.invalidated = false;
            if let Some(previous) = next
                .nodes
                .iter()
                .find(|n| n.id == node.id && !n.invalidated)
            {
                ensure!(
                    previous.receipts == node.receipts
                        && previous.label == node.label
                        && previous.kind == node.kind,
                    "Effect node ID conflicts with existing evidence"
                );
            } else {
                next.nodes.push(node);
            }
        }
        for mut case in output.cases {
            ensure!(
                case.subject_id == next.subject_id,
                "Case belongs to another person"
            );
            ensure!(
                !case.situation.trim().is_empty()
                    && !case.chosen.trim().is_empty()
                    && !case.receipts.is_empty(),
                "A decision case requires a source-grounded situation and human choice"
            );
            ensure!(
                case.receipts
                    .iter()
                    .any(|r| r.source_id == job.source_id
                        && r.source_revision == job.source_revision),
                "Case does not cite this processing source"
            );
            for receipt in &case.receipts {
                validate_receipt(&next, receipt, true)?;
            }
            for text in [
                &case.chosen,
                &case.rationale,
                &case.wanted,
                &case.expected,
                &case.actual,
            ]
            .into_iter()
            .chain(case.rejected.iter())
            {
                ensure!(
                    text.trim().is_empty() || case.receipts.iter().any(|r| r.quote.contains(text)),
                    "Choice, rationale, or experience was not quoted from the target"
                );
            }
            case.review_status = ReviewStatus::Tentative;
            case.provenance = if source.input.structured_case.is_some() {
                "structured_source"
            } else {
                "semantic_extraction"
            }
            .into();
            case.invalidated = false;
            case.conflict = false;
            case.recorded_at = now();
            if !matches!(
                case.case_kind.as_str(),
                "observed_action" | "hypothetical_response"
            ) {
                case.case_kind = "hypothetical_response".into();
            }
            for goal in &case.goal_revisions {
                ensure!(
                    next.goals.iter().any(|g| g.input.id == goal.goal_id
                        && g.revision == goal.revision
                        && !g.invalidated),
                    "Case cites an unknown goal revision"
                );
            }
            let signature = case_signature(&case);
            let same_response = next.cases.iter().find(|c| {
                !c.invalidated
                    && case_signature(c) == signature
                    && normalize(&c.chosen) == normalize(&case.chosen)
            });
            if same_response.is_some_and(|existing| {
                existing.receipts.iter().any(|r| {
                    case.receipts.iter().any(|n| {
                        r.source_id == n.source_id && r.source_revision == n.source_revision
                    })
                })
            }) {
                continue;
            }
            let group = next
                .sources
                .iter()
                .find(|s| {
                    s.input.id == case.receipts[0].source_id
                        && s.revision == case.receipts[0].source_revision
                })
                .unwrap()
                .input
                .source_group
                .clone();
            case.id = format!(
                "case:{}",
                content_hash(&format!(
                    "{}:{}:{}",
                    signature,
                    normalize(&case.chosen),
                    group
                ))
            );
            if next.cases.iter().any(|c| c.id == case.id && !c.invalidated) {
                continue;
            }
            for previous in &mut next.cases {
                if !previous.invalidated
                    && case_signature(previous) == signature
                    && normalize(&previous.chosen) != normalize(&case.chosen)
                {
                    previous.conflict = true;
                    case.conflict = true;
                }
            }
            next.cases.push(case);
        }
        for mut goal in output.goals {
            ensure!(
                !goal.receipts.is_empty(),
                "Extracted goals require exact source receipts"
            );
            ensure!(
                goal.receipts.iter().any(|r| r.source_id == job.source_id),
                "Goal does not cite this source"
            );
            for receipt in goal
                .receipts
                .iter()
                .chain(goal.criteria.iter().flat_map(|c| &c.receipts))
            {
                validate_receipt(&next, receipt, true)?;
            }
            for criterion in &goal.criteria {
                let quotes: Vec<_> = goal
                    .receipts
                    .iter()
                    .chain(&criterion.receipts)
                    .map(|r| r.quote.replace(',', ""))
                    .collect();
                for number in [
                    criterion.target,
                    criterion.upper_target,
                    criterion.baseline,
                    criterion.observed_progress,
                ]
                .into_iter()
                .flatten()
                {
                    ensure!(
                        number.is_finite()
                            && quotes.iter().any(|quote| quote
                                .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
                                .filter_map(|token| token.parse::<f64>().ok())
                                .any(|value| value == number)),
                        "Extracted quantity is absent from exact goal receipts"
                    );
                }
                for field in [
                    &criterion.deadline,
                    &criterion.duration,
                    &criterion.start_anchor,
                ]
                .into_iter()
                .flatten()
                {
                    ensure!(
                        quotes.iter().any(|quote| quote.contains(field)),
                        "Extracted timeframe or start anchor is absent from goal receipts"
                    );
                }
            }
            ensure!(
                !goal.label.trim().is_empty() && !goal.definition.trim().is_empty(),
                "Extracted goal requires a definition"
            );
            if let Some(date) = &goal.effective_at {
                validate_date(date)?;
            }
            goal.review_status = ReviewStatus::Tentative;
            append_goal(&mut next, goal)?;
        }
        for mut relation in output.relationships {
            if rejected_pair(&next, &relation) {
                continue;
            }
            ensure!(
                relation.from_receipt.source_id == job.source_id
                    || relation.to_receipt.source_id == job.source_id,
                "Relationship does not cite this source"
            );
            validate_relationship(&next, &relation)?;
            relation.id = relationship_id(&relation);
            relation.review_status = ReviewStatus::Tentative;
            relation.provenance = "semantic_extraction".into();
            relation.invalidated = false;
            relation.recorded_at = now();
            if !next
                .relationships
                .iter()
                .any(|r| r.id == relation.id && !r.invalidated)
            {
                next.relationships.push(relation);
            }
        }
        let has_conflict = next
            .cases
            .iter()
            .any(|c| c.conflict && c.receipts.iter().any(|r| r.source_id == job.source_id));
        let job = next.jobs.iter_mut().find(|j| j.id == id).unwrap();
        job.status = if output.needs_review.is_empty() && !has_conflict {
            JobStatus::Completed
        } else {
            JobStatus::NeedsReview
        };
        job.error = if output.needs_review.is_empty() {
            None
        } else {
            Some(output.needs_review.join("; "))
        };
        self.commit(next)
    }
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
fn case_signature(case: &DecisionCase) -> String {
    format!(
        "{}|{}|{}|{}",
        case.subject_id,
        normalize(&case.situation),
        normalize(&case.options.join("|")),
        normalize(&case.constraints.join("|"))
    )
}

pub(super) fn rejected_pair(state: &EvidenceSnapshot, relation: &Relationship) -> bool {
    let same = |a: &Receipt, b: &Receipt| {
        a.source_id == b.source_id
            && a.source_revision == b.source_revision
            && a.start == b.start
            && a.end == b.end
    };
    state.relationships.iter().any(|previous| {
        !previous.invalidated
            && previous.review_status == ReviewStatus::Rejected
            && ((same(&previous.from_receipt, &relation.from_receipt)
                && same(&previous.to_receipt, &relation.to_receipt))
                || (same(&previous.from_receipt, &relation.to_receipt)
                    && same(&previous.to_receipt, &relation.from_receipt)))
    })
}

pub(super) fn relationship_id(relation: &Relationship) -> String {
    format!(
        "relation:{}",
        content_hash(&format!(
            "{}:{}:{}:{}:{}:{}:{}:{:?}",
            relation.subject_id,
            relation.from_id,
            relation.from_receipt.source_revision,
            relation.to_id,
            relation.to_receipt.source_revision,
            relation.from_receipt.start,
            relation.to_receipt.start,
            relation.relation
        ))
    )
}

pub(super) fn validate_relationship(
    state: &EvidenceSnapshot,
    relation: &Relationship,
) -> Result<()> {
    ensure!(
        relation.subject_id == state.subject_id,
        "Relationship belongs to another person"
    );
    ensure!(
        !relation.from_id.is_empty()
            && !relation.to_id.is_empty()
            && relation.from_id != relation.to_id,
        "Relationship requires two different endpoints"
    );
    let causal = matches!(
        relation.relation,
        RelationshipKind::Enables
            | RelationshipKind::Inhibits
            | RelationshipKind::Requires
            | RelationshipKind::ContributesTo
    );
    validate_receipt(state, &relation.from_receipt, causal)?;
    validate_receipt(state, &relation.to_receipt, causal)?;
    for (id, receipt) in [
        (&relation.from_id, &relation.from_receipt),
        (&relation.to_id, &relation.to_receipt),
    ] {
        let source = validate_receipt(state, receipt, causal)?;
        let grounded = id == &source.input.id
            || (!source.input.note_id.is_empty() && id == &source.input.note_id)
            || state.cases.iter().any(|case| {
                &case.id == id
                    && !case.invalidated
                    && case.receipts.iter().any(|r| {
                        r.source_id == receipt.source_id
                            && r.source_revision == receipt.source_revision
                    })
            })
            || state.nodes.iter().any(|node| {
                &node.id == id
                    && !node.invalidated
                    && node.receipts.iter().any(|r| {
                        r.source_id == receipt.source_id
                            && r.source_revision == receipt.source_revision
                    })
            })
            || state.goals.iter().any(|goal| {
                &goal.input.id == id
                    && !goal.invalidated
                    && goal.input.receipts.iter().any(|r| {
                        r.source_id == receipt.source_id
                            && r.source_revision == receipt.source_revision
                    })
            });
        ensure!(
            grounded,
            "Relationship endpoint is not a source, case, or goal grounded by its receipt"
        );
    }
    if causal {
        ensure!(
            relation.directed,
            "Effect relationships must be directional"
        );
        ensure!(matches!(relation.causal_basis.as_deref(), Some("target_stated_belief" | "extracted_hypothesis")), "Empirical causality requires separately validated outcome evidence; extraction can only propose beliefs or hypotheses");
    }
    if matches!(
        relation.relation,
        RelationshipKind::Supports | RelationshipKind::Contradicts | RelationshipKind::Equivalent | RelationshipKind::Conflicts
    ) {
        ensure!(
            !relation.conditions.is_empty(),
            "Support/contradiction requires explicit compatible scope conditions"
        );
    }
    ensure!(
        !relation.explanation.trim().is_empty(),
        "Relationship needs a source-grounded explanation"
    );
    ensure!(
        relation
            .similarity
            .is_none_or(|score| score.is_finite() && (-1.0..=1.0).contains(&score)),
        "Invalid similarity score"
    );
    Ok(())
}
