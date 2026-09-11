use super::*;

/// These links express the person's interview expectation, never measured effects.
pub(super) fn capture_interview_path(
    state: &mut EvidenceSnapshot,
    draft: &InterviewDraft,
    source: &SourceRecord,
) -> Result<Option<GoalRevision>> {
    let node = |kind: EvidenceNodeKind, label: &str, field: &str| -> Result<EvidenceNode> {
        Ok(EvidenceNode {
            id: format!("node:{}:{}:{}", source.input.id, source.revision, field),
            subject_id: state.subject_id.clone(),
            kind,
            label: label.into(),
            receipts: vec![field_receipt(source, field, label)?],
            recorded_at: now(),
            ..Default::default()
        })
    };
    let action = node(EvidenceNodeKind::Action, &draft.chosen, "Chosen")?;
    let consequence = if draft.expected.trim().is_empty() {
        None
    } else {
        Some(node(
            EvidenceNodeKind::Consequence,
            &draft.expected,
            "Expected",
        )?)
    };
    for item in std::iter::once(&action).chain(consequence.iter()) {
        if !state
            .nodes
            .iter()
            .any(|n| n.id == item.id && !n.invalidated)
        {
            state.nodes.push(item.clone());
        }
    }
    let wanted = if draft.wanted.trim().is_empty() {
        None
    } else {
        Some(append_goal(
            state,
            GoalInput {
                id: format!("goal:{}:wanted", source.input.id),
                subject_id: state.subject_id.clone(),
                label: draft.wanted.chars().take(80).collect(),
                definition: draft.wanted.clone(),
                scope: draft.situation.clone(),
                receipts: vec![field_receipt(source, "Wanted", &draft.wanted)?],
                ..Default::default()
            },
        )?)
    };
    if let Some(consequence) = consequence {
        let first = Relationship { subject_id: state.subject_id.clone(), from_id: action.id, to_id: consequence.id.clone(),
            relation: RelationshipKind::Enables, directed: true, provenance: "direct_interview".into(),
            explanation: "The person reported this as the expected consequence of their chosen action; it is an expectation, not a measured effect.".into(),
            from_receipt: action.receipts[0].clone(), to_receipt: consequence.receipts[0].clone(), causal_basis: Some("target_stated_belief".into()),
            conditions: vec![draft.situation.clone()], recorded_at: now(), ..Default::default() };
        append_relationship(state, first)?;
        if let (Some(relation), Some(goal)) = (&draft.expected_goal_relation, &wanted) {
            let second = Relationship { subject_id: state.subject_id.clone(), from_id: consequence.id, to_id: goal.input.id.clone(),
                relation: relation.clone(), directed: true, provenance: "direct_interview".into(),
                explanation: "The person explicitly labeled how the expected result would help or hinder what they wanted.".into(),
                from_receipt: consequence.receipts[0].clone(), to_receipt: goal.input.receipts[0].clone(),
                causal_basis: Some("target_stated_belief".into()), conditions: vec![draft.situation.clone()],
                goal_criterion_id: Some(format!("{}:definition", goal.input.id)), recorded_at: now(), ..Default::default() };
            append_relationship(state, second)?;
        }
    }
    Ok(wanted)
}

fn field_receipt(source: &SourceRecord, field: &str, value: &str) -> Result<Receipt> {
    let marker = format!("\n{field}: ");
    let start = source
        .input
        .text
        .find(&marker)
        .context("Interview field not found in original")?
        + marker.len();
    let receipt = Receipt {
        source_id: source.input.id.clone(),
        source_revision: source.revision,
        start,
        end: start + value.len(),
        quote: value.into(),
        locator: format!("Guided interview / {field}"),
    };
    ensure!(
        source.input.text.get(receipt.start..receipt.end) == Some(value),
        "Interview receipt does not match original"
    );
    Ok(receipt)
}

fn append_relationship(state: &mut EvidenceSnapshot, mut relation: Relationship) -> Result<()> {
    jobs::validate_relationship(state, &relation)?;
    relation.id = jobs::relationship_id(&relation);
    if !state
        .relationships
        .iter()
        .any(|r| r.id == relation.id && !r.invalidated)
    {
        state.relationships.push(relation);
    }
    Ok(())
}
