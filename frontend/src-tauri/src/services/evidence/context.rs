use super::*;

pub fn without_goal_paths(packet: &ContextPacket) -> ContextPacket {
    let mut result = packet.clone();
    result.goals.clear();
    result.nodes.clear();
    result.relationships.clear();
    result.unresolved.clear();
    result.source_revisions.clear();
    for case in &mut result.cases {
        case.goal_revisions.clear();
    }
    for receipt in result
        .cases
        .iter()
        .flat_map(|c| &c.receipts)
        .chain(result.statements.iter().flat_map(|s| &s.receipts))
    {
        if !result.source_revisions.contains(receipt) {
            result.source_revisions.push(receipt.clone());
        }
    }
    result
}

impl EvidenceStore {
    /// Shared by desktop and Lab. Restriction and historical availability are enforced here.
    pub fn context_packet(&self, request: ContextRequest) -> Result<ContextPacket> {
        context_from_snapshot(&self.state, request)
    }
}

pub fn context_from_snapshot(
    state: &EvidenceSnapshot,
    request: ContextRequest,
) -> Result<ContextPacket> {
    ensure!(
        request.subject_id.is_empty() || request.subject_id == state.subject_id,
        "Context target mismatch"
    );
    if let Some(date) = &request.as_of {
        validate_date(date)?;
    }
    let blocked_groups: std::collections::HashSet<_> = state
        .sources
        .iter()
        .filter(|s| {
            s.input.restricted
                || s.input.held_out
                || request
                    .excluded_source_groups
                    .contains(&s.input.source_group)
        })
        .map(|s| s.input.source_group.as_str())
        .collect();
    let allowed = |receipt: &Receipt| -> bool {
        validate_receipt(state, receipt, false).is_ok_and(|source| {
            !blocked_groups.contains(source.input.source_group.as_str())
                && source.input.subject_id == state.subject_id
                && source.input.role == EvidenceRole::TargetStatement
                && request
                    .as_of
                    .as_deref()
                    .is_none_or(|date| at_or_before(&source.recorded_at, date))
        })
    };
    let query_words: Vec<_> = request
        .query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let mut cases: Vec<_> = state
        .cases
        .iter()
        .filter(|case| {
            !case.invalidated
                && !case.conflict
                && case.review_status != ReviewStatus::Rejected
                && !case.chosen.trim().is_empty()
                && !case.receipts.is_empty()
                && case.receipts.iter().all(&allowed)
                && request
                    .as_of
                    .as_deref()
                    .is_none_or(|date| at_or_before(&case.recorded_at, date))
        })
        .cloned()
        .collect();
    cases.sort_by_cached_key(|case| {
        let text = format!(
            "{} {} {}",
            case.situation,
            case.rationale,
            case.constraints.join(" ")
        )
        .to_lowercase();
        std::cmp::Reverse(
            query_words
                .iter()
                .filter(|word| text.contains(word.as_str()))
                .count(),
        )
    });
    cases.truncate(request.max_cases.unwrap_or(12).min(32));
    let goals: Vec<_> = latest_goals(state, request.as_of.as_deref())
        .into_iter()
        .filter(|goal| {
            !goal.invalidated
                && goal.input.review_status != ReviewStatus::Rejected
                && goal
                    .input
                    .receipts
                    .iter()
                    .chain(goal.input.criteria.iter().flat_map(|c| &c.receipts))
                    .all(&allowed)
        })
        .cloned()
        .collect();
    let relationships: Vec<_> = state
        .relationships
        .iter()
        .filter(|r| {
            assessment::usable_at(state, r, request.as_of.as_deref())
                && r.review_status != ReviewStatus::Rejected
                && allowed(&r.from_receipt)
                && allowed(&r.to_receipt)
                && request
                    .as_of
                    .as_deref()
                    .is_none_or(|date| at_or_before(&r.recorded_at, date))
        })
        .take(64)
        .map(assessment::relationship_for_context)
        .collect();
    let nodes: Vec<_> = state
        .nodes
        .iter()
        .filter(|node| {
            !node.invalidated
                && node.review_status != ReviewStatus::Rejected
                && !node.receipts.is_empty()
                && node.receipts.iter().all(&allowed)
                && request
                    .as_of
                    .as_deref()
                    .is_none_or(|date| at_or_before(&node.recorded_at, date))
        })
        .take(64)
        .cloned()
        .collect();
    let statements: Vec<_> = state
        .statements
        .iter()
        .filter(|statement| {
            !statement.invalidated
                && statement.review_status != ReviewStatus::Rejected
                && !statement.receipts.is_empty()
                && statement.receipts.iter().all(&allowed)
                && request
                    .as_of
                    .as_deref()
                    .is_none_or(|date| at_or_before(&statement.recorded_at, date))
        })
        .take(32)
        .cloned()
        .collect();
    let mut source_revisions: Vec<Receipt> = vec![];
    for receipt in cases
        .iter()
        .flat_map(|c| &c.receipts)
        .chain(nodes.iter().flat_map(|n| &n.receipts))
        .chain(statements.iter().flat_map(|s| &s.receipts))
        .chain(goals.iter().flat_map(|g| {
            g.input
                .receipts
                .iter()
                .chain(g.input.criteria.iter().flat_map(|c| &c.receipts))
        }))
        .chain(
            relationships
                .iter()
                .flat_map(|r| [&r.from_receipt, &r.to_receipt]),
        )
    {
        if !source_revisions.contains(receipt) {
            source_revisions.push(receipt.clone());
        }
    }
    let unresolved = goals.iter().flat_map(|goal| goal.input.criteria.iter().enumerate().filter_map(|(i,c)| {
            if c.target.is_none() || (c.deadline.is_none() && c.duration.is_none()) {
                Some(format!("Goal {} criterion {} has an unresolved target or timeframe; do not invent one", goal.input.label, i + 1))
            } else { None }
        })).collect();
    Ok(ContextPacket {
        statements,
        nodes,
        subject_id: state.subject_id.clone(),
        assembled_at: now(),
        cases,
        goals,
        relationships,
        source_revisions,
        unresolved,
    })
}
