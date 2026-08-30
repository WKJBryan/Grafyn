use crate::models::canvas::{CanvasSession, ResponseStatus};
use crate::models::twin_event::{
    ActorId, CanvasResponseRecorded, ConversationTurnRecorded, DecimalCost, ModelId,
    ProvenanceLabel,
};
use crate::models::twin_event::{
    AllowedUses, AuthorityClass, BoundedContent, BoundedRole, ContentDigest, EvidenceRef,
    EvidenceType, FeedbackRecorded, Governance, Identifier, NoteChangeKind, NoteChanged,
    ObservationRecorded, ReviewState, Sensitivity, SourceChannel, TwinEventPayload, Visibility,
};
use crate::services::twin_events::TwinEventDraft;
use chrono::{DateTime, Utc};

pub fn standard_capture_governance() -> Governance {
    Governance {
        review: ReviewState::NotApplicable,
        authority: AuthorityClass::EvidenceObservation,
        sensitivity: Sensitivity::Standard,
        visibility: Visibility::SyncedVault,
        allowed_uses: AllowedUses {
            recall: true,
            twin_advisor: true,
            twin_simulation: false,
            export: true,
            training: false,
            sync: true,
        },
    }
}

pub fn local_capture_governance(sensitivity: Sensitivity) -> Governance {
    let mut governance = standard_capture_governance();
    governance.sensitivity = sensitivity;
    governance.visibility = Visibility::LocalOnly;
    governance.allowed_uses.export = false;
    governance.allowed_uses.sync = false;
    governance
}

pub fn imported_capture_governance() -> Governance {
    let mut governance = standard_capture_governance();
    governance.sensitivity = Sensitivity::Sensitive;
    governance.allowed_uses.export = false;
    governance.allowed_uses.training = false;
    governance
}

pub fn note_changed_draft(
    note_id: &str,
    change: NoteChangeKind,
    payload_digest: ContentDigest,
    evidence_digest: ContentDigest,
    observed_at: DateTime<Utc>,
    source_channel: SourceChannel,
    governance: Governance,
) -> Result<TwinEventDraft, String> {
    let note_id = Identifier::parse(note_id)?;
    let mut draft = TwinEventDraft::observed(
        TwinEventPayload::NoteChanged(NoteChanged {
            note_id: note_id.clone(),
            change,
            content_digest: Some(payload_digest),
        }),
        observed_at,
        source_channel,
        governance,
    );
    draft.evidence.push(EvidenceRef {
        evidence_type: EvidenceType::Note,
        source_id: note_id,
        digest: Some(evidence_digest),
    });
    Ok(draft)
}

pub fn import_container_observation_draft(
    observation_id: &str,
    import_id: &str,
    import_digest: ContentDigest,
    note_id: &str,
    note_digest: ContentDigest,
    observed_at: DateTime<Utc>,
) -> Result<TwinEventDraft, String> {
    import_container_observation_draft_for_notes(
        observation_id,
        import_id,
        import_digest,
        &[(note_id.to_string(), note_digest)],
        observed_at,
    )
}

pub fn import_container_observation_draft_for_notes(
    observation_id: &str,
    import_id: &str,
    import_digest: ContentDigest,
    notes: &[(String, ContentDigest)],
    observed_at: DateTime<Utc>,
) -> Result<TwinEventDraft, String> {
    if notes.is_empty() || notes.len() > 63 {
        return Err("an import container must cite 1..=63 persisted notes".into());
    }
    let mut draft = TwinEventDraft::observed(
        TwinEventPayload::ObservationRecorded(ObservationRecorded {
            observation_id: Identifier::parse(observation_id)?,
            claims: Vec::new(),
            summary: None,
            content_digest: Some(import_digest.clone()),
        }),
        observed_at,
        SourceChannel::parse("import")?,
        imported_capture_governance(),
    );
    draft.evidence.push(EvidenceRef {
        evidence_type: EvidenceType::Import,
        source_id: Identifier::parse(import_id)?,
        digest: Some(import_digest),
    });
    for (note_id, note_digest) in notes {
        draft.evidence.push(EvidenceRef {
            evidence_type: EvidenceType::Note,
            source_id: Identifier::parse(note_id)?,
            digest: Some(note_digest.clone()),
        });
    }
    draft.evidence.sort();
    Ok(draft)
}

#[allow(clippy::too_many_arguments)]
pub fn legacy_observation_draft(
    observation_id: &str,
    record_id: &str,
    digest: ContentDigest,
    observed_at: DateTime<Utc>,
    source_channel: SourceChannel,
    actor: Option<crate::models::twin_event::ActorId>,
    tag: Option<&str>,
    governance: Governance,
) -> Result<TwinEventDraft, String> {
    let mut draft = TwinEventDraft::observed(
        TwinEventPayload::ObservationRecorded(ObservationRecorded {
            observation_id: Identifier::parse(observation_id)?,
            claims: Vec::new(),
            summary: None,
            content_digest: Some(digest.clone()),
        }),
        observed_at,
        source_channel,
        governance,
    );
    draft.actor_id = actor;
    if let Some(tag) = tag {
        draft.context.tags.push(tag.to_string());
    }
    draft.evidence.push(EvidenceRef {
        evidence_type: EvidenceType::TwinRecord,
        source_id: Identifier::parse(record_id)?,
        digest: Some(digest),
    });
    Ok(draft)
}

pub struct FeedbackDraft<'a> {
    pub feedback_id: &'a str,
    pub target_id: &'a str,
    pub kind: &'a str,
    pub content: Option<&'a str>,
    pub rationale: Option<&'a str>,
    pub rank: Option<u16>,
    pub observed_at: DateTime<Utc>,
    pub source_channel: SourceChannel,
    pub governance: Governance,
}

pub fn feedback_draft(input: FeedbackDraft<'_>) -> Result<TwinEventDraft, String> {
    let payload = TwinEventPayload::FeedbackRecorded(FeedbackRecorded {
        feedback_id: Identifier::parse(input.feedback_id)?,
        target_id: Identifier::parse(input.target_id)?,
        kind: BoundedRole::parse(input.kind)?,
        content: input.content.map(BoundedContent::parse).transpose()?,
        rationale: input.rationale.map(BoundedContent::parse).transpose()?,
        rank: input.rank,
    });
    payload.validate()?;
    Ok(TwinEventDraft::observed(
        payload,
        input.observed_at,
        input.source_channel,
        input.governance,
    ))
}

pub fn canvas_transition_drafts(
    before: Option<&CanvasSession>,
    after: &CanvasSession,
    persisted_events: &[crate::models::twin_event::TwinEvent],
    session_digest: ContentDigest,
) -> Result<Vec<TwinEventDraft>, String> {
    let source = SourceChannel::parse("canvas")?;
    let governance = standard_capture_governance();
    let session_evidence = EvidenceRef {
        evidence_type: EvidenceType::CanvasSession,
        source_id: Identifier::parse(&after.id)?,
        digest: Some(session_digest),
    };
    let mut drafts = Vec::new();

    let mut new_tiles = after
        .prompt_tiles
        .iter()
        .filter(|tile| {
            before.is_none_or(|before| !before.prompt_tiles.iter().any(|old| old.id == tile.id))
        })
        .collect::<Vec<_>>();
    new_tiles.sort_by(|left, right| left.id.cmp(&right.id));
    for tile in new_tiles {
        if tile.prompt.trim().is_empty() {
            continue;
        }
        let mut prompt = TwinEventDraft::observed(
            TwinEventPayload::ConversationTurnRecorded(ConversationTurnRecorded {
                conversation_id: Identifier::parse(&after.id)?,
                turn_id: Identifier::parse(&tile.id)?,
                role: BoundedRole::parse("user")?,
                content: BoundedContent::parse(&tile.prompt)?,
                model_id: None,
                provenance: Some(ProvenanceLabel::parse("canvas_prompt")?),
                content_digest: Some(crate::services::twin_events::digest_bytes(
                    tile.prompt.as_bytes(),
                )),
            }),
            tile.created_at,
            source.clone(),
            governance.clone(),
        );
        prompt.evidence.push(session_evidence.clone());
        drafts.push(prompt);
    }

    let mut completed = Vec::new();
    for tile in &after.prompt_tiles {
        let old_tile =
            before.and_then(|value| value.prompt_tiles.iter().find(|old| old.id == tile.id));
        for response in tile.responses.values() {
            if response.status != ResponseStatus::Completed || response.content.trim().is_empty() {
                continue;
            }
            let old = old_tile
                .and_then(|value| value.responses.values().find(|old| old.id == response.id));
            let changed = old.is_none_or(|old| {
                old.status != ResponseStatus::Completed
                    || old.content != response.content
                    || old.cost_usd != response.cost_usd
                    || old.tokens_used != response.tokens_used
                    || old.model_id != response.model_id
            });
            if changed {
                completed.push((response.id.clone(), tile, response));
            }
        }
    }
    completed.sort_by(|left, right| left.0.cmp(&right.0));
    for (_, tile, response) in completed {
        let mut draft = canvas_response_draft(
            &after.id,
            &tile.id,
            &response.id,
            &tile.prompt,
            &response.content,
            &response.model_id,
            response.provider.as_deref(),
            response.provenance.as_deref(),
            response.tokens_used.map(u64::from),
            response.cost_usd,
            response.created_at,
            session_evidence.digest.clone(),
            governance.clone(),
        )?;
        if let Some(previous) =
            latest_canvas_response_event(persisted_events, &after.id, &tile.id, &response.id)
        {
            draft.supersedes.push(previous);
        }
        drafts.push(draft);
    }

    let mut new_rounds = Vec::new();
    let mut debate_responses = Vec::new();
    for debate in &after.debates {
        let old = before.and_then(|value| value.debates.iter().find(|old| old.id == debate.id));
        for round in &debate.rounds {
            let old_round = old.and_then(|old| {
                old.rounds
                    .iter()
                    .find(|candidate| candidate.round_number == round.round_number)
            });
            if old_round.is_none() {
                new_rounds.push((debate, round));
            }
            for response in &round.responses {
                if response.content.trim().is_empty() {
                    continue;
                }
                let old_response = old_round.and_then(|old| {
                    old.responses
                        .iter()
                        .find(|candidate| candidate.model_id == response.model_id)
                });
                if old_response.is_none_or(|old| {
                    old.content != response.content || old.cost_usd != response.cost_usd
                }) {
                    let response_id = stable_canvas_id(&[
                        "debate-response",
                        &after.id,
                        &debate.id,
                        &round.round_number.to_string(),
                        &response.model_id,
                    ]);
                    debate_responses.push((response_id, debate, round, response));
                }
            }
        }
    }
    new_rounds.sort_by(|(left_debate, left_round), (right_debate, right_round)| {
        (left_debate.id.as_str(), left_round.round_number)
            .cmp(&(right_debate.id.as_str(), right_round.round_number))
    });
    for (debate, round) in new_rounds {
        if !round.topic.trim().is_empty() {
            let turn_id = stable_canvas_id(&[
                "debate-turn",
                &after.id,
                &debate.id,
                &round.round_number.to_string(),
            ]);
            let mut turn = TwinEventDraft::observed(
                TwinEventPayload::ConversationTurnRecorded(ConversationTurnRecorded {
                    conversation_id: Identifier::parse(&after.id)?,
                    turn_id: Identifier::parse(&turn_id)?,
                    role: BoundedRole::parse("user")?,
                    content: BoundedContent::parse(&round.topic)?,
                    model_id: None,
                    provenance: Some(ProvenanceLabel::parse("canvas_debate_prompt")?),
                    content_digest: Some(crate::services::twin_events::digest_bytes(
                        round.topic.as_bytes(),
                    )),
                }),
                round.created_at,
                source.clone(),
                governance.clone(),
            );
            turn.evidence.push(session_evidence.clone());
            drafts.push(turn);
        }
    }
    debate_responses.sort_by(|left, right| left.0.cmp(&right.0));
    for (response_id, debate, round, response) in debate_responses {
        let mut draft = canvas_response_draft(
            &after.id,
            &debate.id,
            &response_id,
            &round.topic,
            &response.content,
            &response.model_id,
            response.provider.as_deref(),
            response.provenance.as_deref(),
            None,
            response.cost_usd,
            round.created_at,
            session_evidence.digest.clone(),
            governance.clone(),
        )?;
        if let Some(previous) =
            latest_canvas_response_event(persisted_events, &after.id, &debate.id, &response_id)
        {
            draft.supersedes.push(previous);
        }
        drafts.push(draft);
    }
    Ok(drafts)
}

#[allow(clippy::too_many_arguments)]
fn canvas_response_draft(
    session_id: &str,
    tile_id: &str,
    response_id: &str,
    prompt: &str,
    response: &str,
    model_id: &str,
    provider: Option<&str>,
    provenance: Option<&str>,
    tokens_used: Option<u64>,
    cost_usd: Option<f64>,
    observed_at: DateTime<Utc>,
    session_digest: Option<ContentDigest>,
    governance: Governance,
) -> Result<TwinEventDraft, String> {
    let cost_usd_decimal = cost_usd
        .map(|value| DecimalCost::parse(value.to_string()))
        .transpose()?;
    let response_digest = crate::services::twin_events::digest_bytes(response.as_bytes());
    let mut draft = TwinEventDraft::observed(
        TwinEventPayload::CanvasResponseRecorded(CanvasResponseRecorded {
            session_id: Identifier::parse(session_id)?,
            tile_id: Identifier::parse(tile_id)?,
            response_id: Identifier::parse(response_id)?,
            prompt: BoundedContent::parse(prompt)?,
            response: BoundedContent::parse(response)?,
            model_id: ModelId::parse(model_id)?,
            provider: provider.map(Identifier::parse).transpose()?,
            provenance: provenance.map(ProvenanceLabel::parse).transpose()?,
            tokens_used,
            cost_usd_decimal,
            prompt_digest: Some(crate::services::twin_events::digest_bytes(
                prompt.as_bytes(),
            )),
            response_digest: Some(response_digest.clone()),
        }),
        observed_at,
        SourceChannel::parse("canvas")?,
        governance,
    );
    draft.actor_id = Some(ActorId::parse("model")?);
    draft.evidence = vec![
        EvidenceRef {
            evidence_type: EvidenceType::CanvasSession,
            source_id: Identifier::parse(session_id)?,
            digest: session_digest,
        },
        EvidenceRef {
            evidence_type: EvidenceType::CanvasResponse,
            source_id: Identifier::parse(response_id)?,
            digest: Some(response_digest),
        },
    ];
    draft.evidence.sort();
    Ok(draft)
}

fn latest_canvas_response_event(
    events: &[crate::models::twin_event::TwinEvent],
    session_id: &str,
    tile_id: &str,
    response_id: &str,
) -> Option<crate::models::twin_event::EventId> {
    events.iter().rev().find_map(|event| match &event.payload {
        TwinEventPayload::CanvasResponseRecorded(value)
            if value.session_id.as_str() == session_id
                && value.tile_id.as_str() == tile_id
                && value.response_id.as_str() == response_id =>
        {
            Some(event.event_id.clone())
        }
        _ => None,
    })
}

fn stable_canvas_id(parts: &[&str]) -> String {
    let mut bytes = Vec::new();
    for part in parts {
        bytes.extend_from_slice(&(part.len() as u64).to_be_bytes());
        bytes.extend_from_slice(part.as_bytes());
    }
    format!(
        "canvas-{}",
        crate::services::twin_events::digest_bytes(&bytes).as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::canvas::{
        Debate, DebateResponse, DebateRound, ModelResponse, PromptTile, ResponseStatus,
    };
    use crate::models::twin_event::{
        CanvasResponseRecorded, CausalStream, NoteChangeKind, TwinEventPayload,
    };
    use crate::services::twin_events::test_support::event_for_payload;
    use chrono::{TimeZone, Utc};

    fn digest(value: &str) -> ContentDigest {
        crate::services::twin_events::digest_bytes(value.as_bytes())
    }

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 30, 3, 0, 0).unwrap()
    }

    fn canvas_session() -> CanvasSession {
        CanvasSession {
            id: "session-one".to_string(),
            title: "Session".to_string(),
            created_at: at(),
            updated_at: at(),
            ..CanvasSession::default()
        }
    }

    fn response(id: &str, model_id: &str, content: &str, status: ResponseStatus) -> ModelResponse {
        ModelResponse {
            id: id.to_string(),
            model_id: model_id.to_string(),
            model_name: model_id.to_string(),
            content: content.to_string(),
            status,
            tokens_used: Some(21),
            cost_usd: Some(0.0125),
            provider: Some("openrouter".to_string()),
            provenance: Some("canvas_openrouter".to_string()),
            created_at: at(),
            ..ModelResponse::default()
        }
    }

    fn prompt_tile() -> PromptTile {
        PromptTile {
            id: "tile-one".to_string(),
            prompt: "Which path?".to_string(),
            models: vec!["model-a".to_string()],
            created_at: at(),
            ..PromptTile::default()
        }
    }

    #[test]
    fn table_driven_capture_rows_preserve_payload_evidence_source_and_governance() {
        let at = Utc.with_ymd_and_hms(2026, 8, 30, 3, 0, 0).unwrap();
        let rows = [
            (
                "create",
                NoteChangeKind::Created,
                digest("after-create"),
                digest("after-create"),
            ),
            (
                "update",
                NoteChangeKind::Updated,
                digest("after-update"),
                digest("before-update"),
            ),
            (
                "delete",
                NoteChangeKind::Deleted,
                digest("before-delete"),
                digest("before-delete"),
            ),
        ];
        for (label, change, payload_digest, evidence_digest) in rows {
            let draft = note_changed_draft(
                "note-one",
                change.clone(),
                payload_digest.clone(),
                evidence_digest.clone(),
                at,
                SourceChannel::parse("note_editor").unwrap(),
                standard_capture_governance(),
            )
            .unwrap();
            let TwinEventPayload::NoteChanged(payload) = draft.payload else {
                panic!("{label}");
            };
            assert_eq!(payload.change, change, "{label}");
            assert_eq!(payload.content_digest, Some(payload_digest), "{label}");
            assert_eq!(draft.evidence[0].digest, Some(evidence_digest), "{label}");
            assert_eq!(
                draft.context.source_channel.as_str(),
                "note_editor",
                "{label}"
            );
            assert!(!draft.governance.allowed_uses.training, "{label}");
            assert!(!draft.governance.allowed_uses.twin_simulation, "{label}");
        }

        let imported = import_container_observation_draft(
            "import-observation",
            "import-one",
            digest("raw-import"),
            "note-one",
            digest("note"),
            at,
        )
        .unwrap();
        let TwinEventPayload::ObservationRecorded(payload) = imported.payload else {
            panic!();
        };
        assert!(payload.claims.is_empty());
        assert_eq!(imported.evidence.len(), 2);
        assert_eq!(imported.governance.sensitivity, Sensitivity::Sensitive);
        assert!(!imported.governance.allowed_uses.export);
        assert_eq!(imported.context.source_channel.as_str(), "import");
        assert_eq!(CausalStream::SyncEligible, CausalStream::SyncEligible);
    }

    #[test]
    fn private_capture_cannot_be_shared_and_feedback_never_becomes_memory() {
        let governance = local_capture_governance(Sensitivity::Restricted);
        assert_eq!(governance.visibility, Visibility::LocalOnly);
        assert!(!governance.allowed_uses.sync);
        let feedback = feedback_draft(FeedbackDraft {
            feedback_id: "feedback-one",
            target_id: "record-one",
            kind: "reject",
            content: None,
            rationale: None,
            rank: None,
            observed_at: Utc.with_ymd_and_hms(2026, 8, 30, 3, 0, 0).unwrap(),
            source_channel: SourceChannel::parse("legacy_twin").unwrap(),
            governance,
        })
        .unwrap();
        assert!(matches!(
            feedback.payload,
            TwinEventPayload::FeedbackRecorded(_)
        ));
        assert!(!matches!(
            feedback.payload,
            TwinEventPayload::MemoryReviewed(_)
        ));
    }

    #[test]
    fn canvas_completed_batch_is_sorted_exact_and_excludes_transient_or_layout_changes() {
        let before = canvas_session();
        let mut prompted = before.clone();
        prompted.prompt_tiles.push(prompt_tile());
        let session_digest = digest("persisted-session");
        let prompt_drafts =
            canvas_transition_drafts(Some(&before), &prompted, &[], session_digest.clone())
                .unwrap();
        assert_eq!(prompt_drafts.len(), 1);
        let TwinEventPayload::ConversationTurnRecorded(prompt) = &prompt_drafts[0].payload else {
            panic!("new persisted tile must capture one prompt turn");
        };
        assert_eq!(
            prompt.provenance.as_ref().unwrap().as_str(),
            "canvas_prompt"
        );
        assert_eq!(prompt.model_id, None);

        let mut completed = prompted.clone();
        let tile = &mut completed.prompt_tiles[0];
        tile.responses.insert(
            "model-b".to_string(),
            response("response-b", "model-b", "B", ResponseStatus::Completed),
        );
        tile.responses.insert(
            "model-a".to_string(),
            response("response-a", "model-a", "A", ResponseStatus::Completed),
        );
        tile.responses.insert(
            "model-c".to_string(),
            response(
                "response-c",
                "model-c",
                "partial",
                ResponseStatus::Streaming,
            ),
        );
        tile.responses.insert(
            "model-d".to_string(),
            response("response-d", "model-d", "failed", ResponseStatus::Error),
        );
        let response_drafts =
            canvas_transition_drafts(Some(&prompted), &completed, &[], session_digest.clone())
                .unwrap();
        let responses = response_drafts
            .iter()
            .map(|draft| match &draft.payload {
                TwinEventPayload::CanvasResponseRecorded(response) => response,
                _ => panic!("only completed responses belong in this transition"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            responses
                .iter()
                .map(|response| response.response_id.as_str())
                .collect::<Vec<_>>(),
            vec!["response-a", "response-b"]
        );
        for (draft, response) in response_drafts.iter().zip(responses) {
            assert_eq!(draft.actor_id.as_ref().unwrap().as_str(), "model");
            assert_eq!(response.provider.as_ref().unwrap().as_str(), "openrouter");
            assert_eq!(
                response.provenance.as_ref().unwrap().as_str(),
                "canvas_openrouter"
            );
            assert_eq!(response.tokens_used, Some(21));
            assert_eq!(
                response.cost_usd_decimal.as_ref().unwrap().as_str(),
                "0.0125"
            );
            assert!(draft.evidence.iter().any(|evidence| {
                evidence.evidence_type == EvidenceType::CanvasSession
                    && evidence.digest.as_ref() == Some(&session_digest)
            }));
        }

        let mut layout_only = completed.clone();
        layout_only.viewport.x = 99.0;
        layout_only.prompt_tiles[0].position.x = 42.0;
        assert!(canvas_transition_drafts(
            Some(&completed),
            &layout_only,
            &[],
            digest("layout-session")
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn canvas_regeneration_supersedes_only_the_same_persisted_response_identity() {
        let mut before = canvas_session();
        let mut tile = prompt_tile();
        tile.responses.insert(
            "model-a".to_string(),
            response("response-one", "model-a", "old", ResponseStatus::Completed),
        );
        before.prompt_tiles.push(tile);
        let mut after = before.clone();
        after.prompt_tiles[0]
            .responses
            .get_mut("model-a")
            .unwrap()
            .content = "new".to_string();

        let old_same = event_for_payload(TwinEventPayload::CanvasResponseRecorded(
            CanvasResponseRecorded {
                session_id: Identifier::parse("session-one").unwrap(),
                tile_id: Identifier::parse("tile-one").unwrap(),
                response_id: Identifier::parse("response-one").unwrap(),
                prompt: BoundedContent::parse("Which path?").unwrap(),
                response: BoundedContent::parse("old").unwrap(),
                model_id: ModelId::parse("model-a").unwrap(),
                provider: None,
                provenance: None,
                tokens_used: Some(21),
                cost_usd_decimal: Some(DecimalCost::parse("0.0125").unwrap()),
                prompt_digest: None,
                response_digest: Some(digest("old")),
            },
        ));
        let unrelated = event_for_payload(TwinEventPayload::CanvasResponseRecorded(
            CanvasResponseRecorded {
                session_id: Identifier::parse("session-one").unwrap(),
                tile_id: Identifier::parse("tile-one").unwrap(),
                response_id: Identifier::parse("response-other").unwrap(),
                prompt: BoundedContent::parse("Which path?").unwrap(),
                response: BoundedContent::parse("other").unwrap(),
                model_id: ModelId::parse("model-a").unwrap(),
                provider: None,
                provenance: None,
                tokens_used: None,
                cost_usd_decimal: None,
                prompt_digest: None,
                response_digest: Some(digest("other")),
            },
        ));
        let drafts = canvas_transition_drafts(
            Some(&before),
            &after,
            &[old_same.clone(), unrelated],
            digest("regenerated-session"),
        )
        .unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].supersedes, vec![old_same.event_id]);
    }

    #[test]
    fn debate_completion_on_an_existing_round_emits_stable_sorted_model_responses_only() {
        let mut before = canvas_session();
        before.debates.push(Debate {
            id: "debate-one".to_string(),
            rounds: vec![DebateRound {
                round_number: 1,
                topic: "Tradeoffs".to_string(),
                responses: Vec::new(),
                created_at: at(),
            }],
            created_at: at(),
            ..Debate::default()
        });
        let mut after = before.clone();
        after.debates[0].rounds[0].responses = vec![
            DebateResponse {
                model_id: "model-b".to_string(),
                model_name: "B".to_string(),
                content: "Second".to_string(),
                stance: None,
                cost_usd: Some(0.02),
                provider: Some("openrouter".into()),
                provenance: Some("canvas_debate_openrouter".into()),
            },
            DebateResponse {
                model_id: "model-a".to_string(),
                model_name: "A".to_string(),
                content: "First".to_string(),
                stance: None,
                cost_usd: None,
                provider: Some("ollama".into()),
                provenance: Some("canvas_debate_ollama".into()),
            },
        ];

        let drafts =
            canvas_transition_drafts(Some(&before), &after, &[], digest("debate-session")).unwrap();
        assert_eq!(drafts.len(), 2);
        let rows = drafts
            .iter()
            .map(|draft| match &draft.payload {
                TwinEventPayload::CanvasResponseRecorded(response) => {
                    (response.model_id.as_str(), response.response_id.as_str())
                }
                _ => panic!("an already captured debate prompt must not be repeated"),
            })
            .collect::<Vec<_>>();
        let mut sorted_ids = rows.iter().map(|row| row.1).collect::<Vec<_>>();
        sorted_ids.sort_unstable();
        assert_eq!(rows.iter().map(|row| row.1).collect::<Vec<_>>(), sorted_ids);
        assert_ne!(rows[0].1, rows[1].1);

        let repeated =
            canvas_transition_drafts(Some(&before), &after, &[], digest("debate-session")).unwrap();
        let repeated_ids = repeated
            .iter()
            .map(|draft| match &draft.payload {
                TwinEventPayload::CanvasResponseRecorded(response) => response.response_id.as_str(),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows.iter().map(|row| row.1).collect::<Vec<_>>(),
            repeated_ids
        );
    }
}
