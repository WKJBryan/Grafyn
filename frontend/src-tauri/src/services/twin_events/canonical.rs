use crate::models::twin_event::*;
use chrono::{DateTime, SecondsFormat, Utc};
use sha2::{Digest, Sha256};

pub fn derive_event_id(event: &TwinEvent) -> EventId {
    let digest = Sha256::digest(semantic_bytes(event));
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    EventId::parse(hex).expect("SHA-256 always produces a valid event id")
}

pub(crate) fn semantic_bytes(event: &TwinEvent) -> Vec<u8> {
    let mut event = event.clone();
    event.normalize();
    let mut out = Encoder::new("grafyn.twin_event.semantic.v1");
    out.u16("schema_version", event.schema_version);
    out.tag("event_type", event_type(&event.event_type));
    out.text("actor_id", event.actor_id.as_str());
    out.text("device_id", event.device_id.as_str());
    out.tag(
        "causal_stream",
        match event.causal_stream {
            CausalStream::LocalOnly => "local_only",
            CausalStream::SyncEligible => "sync_eligible",
        },
    );
    out.u64("device_sequence", event.device_sequence);
    out.ids("causal_parents", &event.causal_parents);
    out.time("recorded_at", &event.recorded_at);
    out.time("observed_at", &event.observed_at);
    out.optional_time("occurred_at", event.occurred_at.as_ref());
    out.optional_time("valid_from", event.valid_from.as_ref());
    out.optional_time("valid_to", event.valid_to.as_ref());
    out.ids("supersedes", &event.supersedes);
    out.ids("reinforces", &event.reinforces);
    encode_context(&mut out, &event.context);
    encode_evidence_list(&mut out, "evidence", &event.evidence);
    encode_governance(&mut out, "governance", &event.governance);
    encode_payload(&mut out, &event.payload);
    out.finish()
}

fn event_type(value: &TwinEventType) -> &'static str {
    match value {
        TwinEventType::ObservationRecorded => "observation_recorded",
        TwinEventType::NoteChanged => "note_changed",
        TwinEventType::ConversationTurnRecorded => "conversation_turn_recorded",
        TwinEventType::CanvasResponseRecorded => "canvas_response_recorded",
        TwinEventType::MemoryProposed => "memory_proposed",
        TwinEventType::MemoryReviewed => "memory_reviewed",
        TwinEventType::DecisionRecorded => "decision_recorded",
        TwinEventType::DecisionOutcomeRecorded => "decision_outcome_recorded",
        TwinEventType::FeedbackRecorded => "feedback_recorded",
        TwinEventType::RelationshipContextObserved => "relationship_context_observed",
    }
}

fn encode_context(out: &mut Encoder, context: &EventContext) {
    out.tag("context", "grafyn.event_context.v1");
    out.count("context.entities", context.entities.len());
    for entity in &context.entities {
        out.tag("context.entity", "grafyn.context_entity.v1");
        out.text("entity_id", entity.entity_id.as_str());
        out.text("entity_type", entity.entity_type.as_str());
        out.optional_text(
            "display_label",
            entity.display_label.as_ref().map(BoundedLabel::as_str),
        );
        out.optional_text(
            "role_in_event",
            entity.role_in_event.as_ref().map(BoundedRole::as_str),
        );
    }
    out.count("context.relationships", context.relationships.len());
    for relation in &context.relationships {
        out.tag("context.relationship", "grafyn.relationship_assertion.v1");
        out.text("subject_id", relation.subject_id.as_str());
        out.text("predicate", relation.predicate.as_str());
        out.text("object_id", relation.object_id.as_str());
        out.tag(
            "direction",
            match relation.direction {
                RelationshipDirection::Directed => "directed",
                RelationshipDirection::Bidirectional => "bidirectional",
            },
        );
        out.optional_time("relationship.valid_from", relation.valid_from.as_ref());
        out.optional_time("relationship.valid_to", relation.valid_to.as_ref());
        encode_evidence_list(out, "relationship.evidence", &relation.evidence);
        encode_governance(out, "relationship.governance", &relation.governance);
    }
    out.texts("context.environments", &context.environments);
    out.texts("context.activities", &context.activities);
    out.texts("context.goals", &context.goals);
    out.text("context.source_channel", context.source_channel.as_str());
    out.texts("context.tags", &context.tags);
}

fn encode_evidence_list(out: &mut Encoder, field: &str, evidence: &[EvidenceRef]) {
    out.count(field, evidence.len());
    for item in evidence {
        out.tag("evidence", "grafyn.evidence_ref.v1");
        out.tag(
            "evidence_type",
            match item.evidence_type {
                EvidenceType::Note => "note",
                EvidenceType::Conversation => "conversation",
                EvidenceType::CanvasSession => "canvas_session",
                EvidenceType::CanvasResponse => "canvas_response",
                EvidenceType::TwinRecord => "twin_record",
                EvidenceType::Decision => "decision",
                EvidenceType::Feedback => "feedback",
                EvidenceType::Import => "import",
                EvidenceType::Event => "event",
                EvidenceType::Attachment => "attachment",
            },
        );
        out.text("source_id", item.source_id.as_str());
        out.optional_text("digest", item.digest.as_ref().map(ContentDigest::as_str));
    }
}

fn encode_governance(out: &mut Encoder, field: &str, value: &Governance) {
    out.tag(field, "grafyn.governance.v1");
    out.tag(
        "review",
        match value.review {
            ReviewState::NotApplicable => "not_applicable",
            ReviewState::Pending => "pending",
            ReviewState::Accepted => "accepted",
            ReviewState::Rejected => "rejected",
            ReviewState::Superseded => "superseded",
        },
    );
    match &value.authority {
        AuthorityClass::EvidenceObservation => out.tag("authority", "evidence_observation"),
        AuthorityClass::ReviewedMemory => out.tag("authority", "reviewed_memory"),
        AuthorityClass::CanonicalUserRule => out.tag("authority", "canonical_user_rule"),
        AuthorityClass::DeterministicallyVerified { method } => {
            out.tag("authority", "deterministically_verified");
            out.tag(
                "verification_method",
                match method {
                    VerificationMethod::HumanReview => "human_review",
                    VerificationMethod::SourceChecksum => "source_checksum",
                    VerificationMethod::SignedImport => "signed_import",
                    VerificationMethod::RecordedOutcomeMatch => "recorded_outcome_match",
                },
            );
        }
    }
    out.tag(
        "sensitivity",
        match value.sensitivity {
            Sensitivity::Standard => "standard",
            Sensitivity::Sensitive => "sensitive",
            Sensitivity::Restricted => "restricted",
        },
    );
    out.tag(
        "visibility",
        match value.visibility {
            Visibility::LocalOnly => "local_only",
            Visibility::SyncedVault => "synced_vault",
        },
    );
    out.bool("allowed.recall", value.allowed_uses.recall);
    out.bool("allowed.twin_advisor", value.allowed_uses.twin_advisor);
    out.bool(
        "allowed.twin_simulation",
        value.allowed_uses.twin_simulation,
    );
    out.bool("allowed.export", value.allowed_uses.export);
    out.bool("allowed.training", value.allowed_uses.training);
    out.bool("allowed.sync", value.allowed_uses.sync);
}

fn encode_payload(out: &mut Encoder, payload: &TwinEventPayload) {
    out.tag("payload", "grafyn.twin_event_payload.v1");
    out.tag("payload_type", event_type(&payload.event_type()));
    match payload {
        TwinEventPayload::ObservationRecorded(v) => {
            out.text("observation_id", v.observation_id.as_str());
            encode_claims(out, "claims", &v.claims);
            out.optional_text("summary", v.summary.as_ref().map(BoundedSummary::as_str));
            out.optional_text(
                "content_digest",
                v.content_digest.as_ref().map(ContentDigest::as_str),
            );
        }
        TwinEventPayload::NoteChanged(v) => {
            out.text("note_id", v.note_id.as_str());
            out.tag(
                "change",
                match v.change {
                    NoteChangeKind::Created => "created",
                    NoteChangeKind::Updated => "updated",
                    NoteChangeKind::Deleted => "deleted",
                },
            );
            out.optional_text(
                "content_digest",
                v.content_digest.as_ref().map(ContentDigest::as_str),
            );
        }
        TwinEventPayload::ConversationTurnRecorded(v) => {
            out.text("conversation_id", v.conversation_id.as_str());
            out.text("turn_id", v.turn_id.as_str());
            out.text("role", v.role.as_str());
            out.text("content", v.content.as_str());
            out.optional_text("model_id", v.model_id.as_ref().map(ModelId::as_str));
            out.optional_text(
                "provenance",
                v.provenance.as_ref().map(ProvenanceLabel::as_str),
            );
            out.optional_text(
                "content_digest",
                v.content_digest.as_ref().map(ContentDigest::as_str),
            );
        }
        TwinEventPayload::CanvasResponseRecorded(v) => {
            out.text("session_id", v.session_id.as_str());
            out.text("tile_id", v.tile_id.as_str());
            out.text("response_id", v.response_id.as_str());
            out.text("prompt", v.prompt.as_str());
            out.text("response", v.response.as_str());
            out.text("model_id", v.model_id.as_str());
            out.optional_text("provider", v.provider.as_ref().map(Identifier::as_str));
            out.optional_text(
                "provenance",
                v.provenance.as_ref().map(ProvenanceLabel::as_str),
            );
            out.optional_u64("tokens_used", v.tokens_used);
            out.optional_text(
                "cost_usd_decimal",
                v.cost_usd_decimal.as_ref().map(DecimalCost::as_str),
            );
            out.optional_text(
                "prompt_digest",
                v.prompt_digest.as_ref().map(ContentDigest::as_str),
            );
            out.optional_text(
                "response_digest",
                v.response_digest.as_ref().map(ContentDigest::as_str),
            );
        }
        TwinEventPayload::MemoryProposed(v) => {
            out.text("memory_id", v.memory_id.as_str());
            encode_claim(out, &v.claim);
            out.optional_text("summary", v.summary.as_ref().map(BoundedSummary::as_str));
            out.text("proposal_source", v.proposal_source.as_str());
        }
        TwinEventPayload::MemoryReviewed(v) => {
            out.text("memory_id", v.memory_id.as_str());
            out.tag(
                "decision",
                match v.decision {
                    MemoryReviewDecision::Accept => "accept",
                    MemoryReviewDecision::Reject => "reject",
                    MemoryReviewDecision::Supersede => "supersede",
                },
            );
            match &v.reviewed_claim {
                Some(claim) => {
                    out.bool("reviewed_claim.present", true);
                    encode_claim(out, claim);
                }
                None => out.bool("reviewed_claim.present", false),
            };
            out.optional_text(
                "rationale",
                v.rationale.as_ref().map(BoundedContent::as_str),
            );
        }
        TwinEventPayload::DecisionRecorded(v) => {
            out.text("decision_id", v.decision_id.as_str());
            out.text("decision", v.decision.as_str());
            out.bounded_texts("options", &v.options);
            out.optional_text("stakes", v.stakes.as_ref().map(BoundedContent::as_str));
            out.optional_text(
                "initial_leaning",
                v.initial_leaning.as_ref().map(BoundedContent::as_str),
            );
            out.optional_text(
                "review_date",
                v.review_date.as_ref().map(BoundedLabel::as_str),
            );
        }
        TwinEventPayload::DecisionOutcomeRecorded(v) => {
            out.text("decision_id", v.decision_id.as_str());
            out.optional_text("outcome", v.outcome.as_ref().map(BoundedContent::as_str));
            out.optional_text(
                "chosen_option",
                v.chosen_option.as_ref().map(BoundedContent::as_str),
            );
            out.optional_text(
                "selected_response_id",
                v.selected_response_id.as_ref().map(Identifier::as_str),
            );
            out.optional_u16("confidence_basis_points", v.confidence_basis_points);
            out.optional_text(
                "review_date",
                v.review_date.as_ref().map(BoundedLabel::as_str),
            );
            out.optional_text(
                "correction_note",
                v.correction_note.as_ref().map(BoundedContent::as_str),
            );
            out.optional_u8("regret_score", v.regret_score);
            out.optional_text("lesson", v.lesson.as_ref().map(BoundedContent::as_str));
            out.optional_text(
                "missed_something",
                v.missed_something.as_ref().map(BoundedContent::as_str),
            );
        }
        TwinEventPayload::FeedbackRecorded(v) => {
            out.text("feedback_id", v.feedback_id.as_str());
            out.text("target_id", v.target_id.as_str());
            out.text("kind", v.kind.as_str());
            out.optional_text("content", v.content.as_ref().map(BoundedContent::as_str));
            out.optional_text(
                "rationale",
                v.rationale.as_ref().map(BoundedContent::as_str),
            );
            out.optional_u16("rank", v.rank);
        }
        TwinEventPayload::RelationshipContextObserved(_) => {}
    }
}

fn encode_claims(out: &mut Encoder, field: &str, claims: &[ClaimAssertion]) {
    out.count(field, claims.len());
    for claim in claims {
        encode_claim(out, claim);
    }
}

fn encode_claim(out: &mut Encoder, claim: &ClaimAssertion) {
    out.tag("claim", "grafyn.claim_assertion.v1");
    out.text("claim.subject_id", claim.subject_id.as_str());
    out.text("claim.predicate", claim.predicate.as_str());
    out.text("claim.object", claim.object.as_str());
    out.tag(
        "claim.polarity",
        match claim.polarity {
            ClaimPolarity::Affirmed => "affirmed",
            ClaimPolarity::Denied => "denied",
        },
    );
}

struct Encoder {
    bytes: Vec<u8>,
}
impl Encoder {
    fn new(domain: &str) -> Self {
        let mut value = Self { bytes: Vec::new() };
        value.tag("domain", domain);
        value
    }
    fn finish(self) -> Vec<u8> {
        self.bytes
    }
    fn raw(&mut self, value: &[u8]) {
        self.bytes
            .extend_from_slice(&(value.len() as u64).to_be_bytes());
        self.bytes.extend_from_slice(value);
    }
    fn field(&mut self, name: &str, value: &[u8]) {
        self.raw(name.as_bytes());
        self.raw(value);
    }
    fn tag(&mut self, name: &str, value: &str) {
        self.field(name, value.as_bytes());
    }
    fn text(&mut self, name: &str, value: &str) {
        self.field(name, value.as_bytes());
    }
    fn u16(&mut self, name: &str, value: u16) {
        self.field(name, &value.to_be_bytes());
    }
    fn optional_u16(&mut self, name: &str, value: Option<u16>) {
        match value {
            Some(value) => {
                self.bool(&format!("{name}.present"), true);
                self.u16(name, value);
            }
            None => self.bool(&format!("{name}.present"), false),
        }
    }
    fn u64(&mut self, name: &str, value: u64) {
        self.field(name, &value.to_be_bytes());
    }
    fn bool(&mut self, name: &str, value: bool) {
        self.field(name, &[u8::from(value)]);
    }
    fn count(&mut self, name: &str, value: usize) {
        self.u64(name, value as u64);
    }
    fn ids(&mut self, name: &str, values: &[EventId]) {
        self.count(name, values.len());
        for value in values {
            self.text("event_id", value.as_str());
        }
    }
    fn texts(&mut self, name: &str, values: &[String]) {
        self.count(name, values.len());
        for value in values {
            self.text("text", value);
        }
    }
    fn bounded_texts(&mut self, name: &str, values: &[BoundedContent]) {
        self.count(name, values.len());
        for value in values {
            self.text("text", value.as_str());
        }
    }
    fn time(&mut self, name: &str, value: &DateTime<Utc>) {
        self.text(name, &value.to_rfc3339_opts(SecondsFormat::Nanos, true));
    }
    fn optional_time(&mut self, name: &str, value: Option<&DateTime<Utc>>) {
        match value {
            Some(value) => {
                self.bool(&format!("{name}.present"), true);
                self.time(name, value);
            }
            None => self.bool(&format!("{name}.present"), false),
        }
    }
    fn optional_text(&mut self, name: &str, value: Option<&str>) {
        match value {
            Some(value) => {
                self.bool(&format!("{name}.present"), true);
                self.text(name, value);
            }
            None => self.bool(&format!("{name}.present"), false),
        }
    }
    fn optional_u64(&mut self, name: &str, value: Option<u64>) {
        match value {
            Some(value) => {
                self.bool(&format!("{name}.present"), true);
                self.u64(name, value);
            }
            None => self.bool(&format!("{name}.present"), false),
        }
    }
    fn optional_u8(&mut self, name: &str, value: Option<u8>) {
        match value {
            Some(value) => {
                self.bool(&format!("{name}.present"), true);
                self.field(name, &[value]);
            }
            None => self.bool(&format!("{name}.present"), false),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::models::twin_event::CausalStream;
    use crate::services::twin_events::test_support::valid_event;

    #[test]
    fn canonical_id_ignores_json_object_field_order_and_formatting() {
        let event = valid_event(1, Vec::new());
        let compact = serde_json::to_string(&event).unwrap();
        let pretty = serde_json::to_string_pretty(&event).unwrap();
        let value = serde_json::to_value(&event).unwrap();
        let reversed = value
            .as_object()
            .unwrap()
            .iter()
            .rev()
            .map(|(key, value)| {
                format!(
                    "{}:{}",
                    serde_json::to_string(key).unwrap(),
                    serde_json::to_string(value).unwrap()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let reordered = format!("{{{reversed}}}");
        let compact_event = serde_json::from_str(&compact).unwrap();
        let pretty_event = serde_json::from_str(&pretty).unwrap();
        let reordered_event = serde_json::from_str(&reordered).unwrap();
        assert_eq!(
            super::derive_event_id(&compact_event),
            super::derive_event_id(&pretty_event)
        );
        assert_eq!(
            super::derive_event_id(&compact_event),
            super::derive_event_id(&reordered_event)
        );
    }

    #[test]
    fn causal_stream_is_part_of_event_identity() {
        let local = valid_event(1, Vec::new());
        let mut shared = local.clone();
        shared.causal_stream = CausalStream::SyncEligible;
        assert_ne!(
            super::derive_event_id(&local),
            super::derive_event_id(&shared)
        );
    }

    #[test]
    fn nested_relationship_evidence_order_is_canonicalized_before_relationship_order() {
        use crate::models::twin_event::*;
        let evidence = |id: &str| EvidenceRef {
            evidence_type: EvidenceType::Note,
            source_id: Identifier::parse(id).unwrap(),
            digest: None,
        };
        let relationship = |items| RelationshipAssertion {
            subject_id: EntityId::parse("owner").unwrap(),
            predicate: RelationshipPredicate::parse("works_with").unwrap(),
            object_id: EntityId::parse("person-1").unwrap(),
            direction: RelationshipDirection::Directed,
            valid_from: None,
            valid_to: None,
            evidence: items,
            governance: Governance::direct_observation(),
        };
        let mut first = valid_event(1, Vec::new());
        first.context.relationships = vec![relationship(vec![evidence("b"), evidence("a")])];
        let mut second = first.clone();
        second.context.relationships = vec![relationship(vec![evidence("a"), evidence("b")])];
        assert_eq!(
            super::derive_event_id(&first),
            super::derive_event_id(&second)
        );
    }

    #[test]
    fn task_seven_canvas_and_decision_learning_fields_are_canonical() {
        use crate::models::twin_event::*;
        use crate::services::twin_events::test_support::{all_payloads, event_for_payload};
        let payloads = all_payloads();
        let canvas = payloads
            .iter()
            .find_map(|payload| match payload {
                TwinEventPayload::CanvasResponseRecorded(value) => Some(value.clone()),
                _ => None,
            })
            .unwrap();
        let baseline = super::derive_event_id(&event_for_payload(
            TwinEventPayload::CanvasResponseRecorded(canvas.clone()),
        ));
        let mut changed = canvas;
        changed.cost_usd_decimal = Some(DecimalCost::parse("0.25").unwrap());
        assert_ne!(
            baseline,
            super::derive_event_id(&event_for_payload(
                TwinEventPayload::CanvasResponseRecorded(changed)
            ))
        );

        let decision = payloads
            .iter()
            .find_map(|payload| match payload {
                TwinEventPayload::DecisionRecorded(value) => Some(value.clone()),
                _ => None,
            })
            .unwrap();
        let baseline = super::derive_event_id(&event_for_payload(
            TwinEventPayload::DecisionRecorded(decision.clone()),
        ));
        let mut changed = decision;
        changed.initial_leaning = Some(BoundedContent::parse("option a").unwrap());
        assert_ne!(
            baseline,
            super::derive_event_id(&event_for_payload(TwinEventPayload::DecisionRecorded(
                changed
            )))
        );

        let outcome = payloads
            .iter()
            .find_map(|payload| match payload {
                TwinEventPayload::DecisionOutcomeRecorded(value) => Some(value.clone()),
                _ => None,
            })
            .unwrap();
        let baseline = super::derive_event_id(&event_for_payload(
            TwinEventPayload::DecisionOutcomeRecorded(outcome.clone()),
        ));
        let mut changed = outcome;
        changed.missed_something = Some(BoundedContent::parse("a constraint").unwrap());
        assert_ne!(
            baseline,
            super::derive_event_id(&event_for_payload(
                TwinEventPayload::DecisionOutcomeRecorded(changed)
            ))
        );
    }

    #[test]
    fn decision_option_order_and_duplicates_are_semantic_and_preserved() {
        use crate::models::twin_event::*;
        use crate::services::twin_events::test_support::event_for_payload;
        let decision = |options: &[&str]| {
            event_for_payload(TwinEventPayload::DecisionRecorded(DecisionRecorded {
                decision_id: Identifier::parse("decision-1").unwrap(),
                decision: BoundedContent::parse("choose").unwrap(),
                options: options
                    .iter()
                    .map(|value| BoundedContent::parse(*value).unwrap())
                    .collect(),
                stakes: None,
                initial_leaning: None,
                review_date: None,
            }))
        };
        let first = decision(&["alpha", "alpha", "beta"]);
        let reversed = decision(
            &["alpha", "alpha", "beta"]
                .into_iter()
                .rev()
                .collect::<Vec<_>>(),
        );
        assert_ne!(
            super::derive_event_id(&first),
            super::derive_event_id(&reversed)
        );

        let mut normalized = first.clone();
        normalized.normalize();
        let TwinEventPayload::DecisionRecorded(payload) = normalized.payload else {
            unreachable!()
        };
        assert_eq!(
            payload
                .options
                .iter()
                .map(BoundedContent::as_str)
                .collect::<Vec<_>>(),
            vec!["alpha", "alpha", "beta"]
        );
        let round_trip: TwinEvent =
            serde_json::from_str(&serde_json::to_string(&first).unwrap()).unwrap();
        assert_eq!(round_trip, first);
    }
}
