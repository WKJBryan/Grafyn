use super::embedding::valid_vector;
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryOutput {
    pub relationships: Vec<Relationship>,
    pub embedding_status: String,
    #[serde(default)]
    pub embedding_version: Option<String>,
}

/// Local semantic evidence only. Lexical overlap never creates a relationship.
pub async fn discover_relationships(
    snapshot: &EvidenceSnapshot,
    cache_dir: PathBuf,
    ollama_url: &str,
) -> Result<DiscoveryOutput> {
    let passages = passages(snapshot);
    let texts: Vec<_> = passages.iter().map(|p| p.receipt.quote.as_str()).collect();
    let output = embedding::embed_passages(&texts, cache_dir, ollama_url).await?;
    let relationships =
        semantic_candidates(snapshot, &passages, &output.vectors, &output.model_version);
    Ok(DiscoveryOutput {
        relationships,
        embedding_version: (output.status.starts_with("ready:")
            && !output.model_version.is_empty())
        .then_some(output.model_version),
        embedding_status: format!(
            "{}; {} passages compared (limit 500), up to 10 candidates per passage",
            output.status,
            passages.len()
        ),
    })
}
fn pending(reason: &str) -> DiscoveryOutput {
    DiscoveryOutput {
        relationships: vec![],
        embedding_version: None,
        embedding_status: format!("pending: {reason}; no semantic relationships generated"),
    }
}

struct Passage {
    node_id: String,
    receipt: Receipt,
}
fn passages(snapshot: &EvidenceSnapshot) -> Vec<Passage> {
    let mut result = vec![];
    let mut seen = HashSet::new();
    for source in snapshot.sources.iter().filter(|s| {
        !s.deleted
            && !s.input.restricted
            && !s.input.held_out
            && s.input.subject_id == snapshot.subject_id
    }) {
        if snapshot.sources.iter().any(|other| {
            (other.input.restricted || other.input.held_out)
                && (other.input.source_group == source.input.source_group
                    || other.input.text == source.input.text)
        }) {
            continue;
        }
        let text = &source.input.text;
        let mut start = 0;
        for paragraph in text.split_inclusive("\n\n") {
            let quote = paragraph.trim();
            let offset = paragraph.find(quote).unwrap_or(0);
            let navigation = quote.lines().all(|line| {
                ["Previous:", "Next:", "Part of:"]
                    .iter()
                    .any(|prefix| line.trim().starts_with(prefix))
            });
            if quote.len() >= 24
                && quote.len() <= 12000
                && !navigation
                && seen.insert(quote.to_string())
            {
                result.push(Passage {
                    node_id: if source.input.note_id.is_empty() {
                        source.input.id.clone()
                    } else {
                        source.input.note_id.clone()
                    },
                    receipt: Receipt {
                        source_id: source.input.id.clone(),
                        source_revision: source.revision,
                        start: start + offset,
                        end: start + offset + quote.len(),
                        quote: quote.into(),
                        locator: format!(
                            "{} bytes {}..{}",
                            source.input.title,
                            start + offset,
                            start + offset + quote.len()
                        ),
                    },
                });
            }
            start += paragraph.len();
            if result.len() >= 500 {
                return result;
            }
        }
    }
    result
}

fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || !valid_vector(a) || !valid_vector(b) {
        return None;
    }
    let dot: f32 = a.iter().zip(b).map(|(a, b)| a * b).sum();
    Some(
        (dot / (a.iter().map(|x| x * x).sum::<f32>() * b.iter().map(|x| x * x).sum::<f32>())
            .sqrt())
        .clamp(-1.0, 1.0),
    )
}

fn semantic_candidates(
    snapshot: &EvidenceSnapshot,
    passages: &[Passage],
    vectors: &[Vec<f32>],
    version: &str,
) -> Vec<Relationship> {
    let mut result = vec![];
    let mut emitted = HashSet::new();
    for (i, left) in passages.iter().enumerate() {
        let mut candidates: Vec<_> = passages
            .iter()
            .enumerate()
            .filter_map(|(j, right)| {
                if i == j || left.node_id == right.node_id {
                    return None;
                }
                let score = cosine(vectors.get(i)?, vectors.get(j)?)?;
                // Conservative discovery threshold, a ranking heuristic, never a truth probability.
                (score >= 0.78).then_some((j, score))
            })
            .collect();
        candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (j, score) in candidates.into_iter().take(10) {
            let pair = (i.min(j), i.max(j));
            if !emitted.insert(pair) {
                continue;
            }
            let left = &passages[pair.0];
            let right = &passages[pair.1];
            let mut relation = Relationship { subject_id: snapshot.subject_id.clone(), from_id: left.node_id.clone(), to_id: right.node_id.clone(),
                relation: RelationshipKind::Related, provenance: "local_semantic_similarity".into(), similarity: Some(score), model_version: Some(version.into()),
                explanation: "Local embedding similarity suggests related passage meaning; this does not establish support, agreement, or a causal effect.".into(),
                from_receipt: left.receipt.clone(), to_receipt: right.receipt.clone(), recorded_at: now(), ..Default::default() };
            relation.id = jobs::relationship_id(&relation);
            result.push(relation);
        }
    }
    result
}

impl EvidenceStore {
    pub fn apply_discovery(&mut self, output: DiscoveryOutput) -> Result<EvidenceSnapshot> {
        let mut next = self.state.clone();
        if let Some(version) = output
            .embedding_version
            .as_ref()
            .filter(|version| output.embedding_status.starts_with("ready:") && !version.is_empty())
        {
            for previous in &mut next.relationships {
                if previous.provenance == "local_semantic_similarity"
                    && previous.review_status != ReviewStatus::Rejected
                    && !previous.invalidated
                    && previous.model_version.as_ref() != Some(version)
                {
                    // A reviewed interpretation remains evidence, but its old score cannot
                    // be ranked as if it came from the current embedding representation.
                    previous.similarity = None;
                    previous.model_version = Some(version.clone());
                }
            }
        }
        for relation in output.relationships {
            if jobs::rejected_pair(&next, &relation) {
                continue;
            }
            // A source edit while encoding discards that pair, preserving unrelated valid pairs.
            if jobs::validate_relationship(&next, &relation).is_err() {
                continue;
            }
            if let Some(previous) = next
                .relationships
                .iter_mut()
                .find(|r| r.id == relation.id && !r.invalidated)
            {
                previous.similarity = relation.similarity;
                previous.model_version = relation.model_version;
            } else {
                next.relationships.push(relation);
            }
        }
        next.embedding_status = output.embedding_status;
        self.commit(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_candidates_do_not_enter_personal_context() {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            EvidenceStore::new(dir.path().into(), "test".into(), "Test".into()).unwrap();
        let state = store
            .reconcile_sources(vec![
                SourceInput {
                    id: "a".into(),
                    subject_id: "test".into(),
                    role: EvidenceRole::TargetStatement,
                    text: "Reach ten thousand people this month".into(),
                    ..Default::default()
                },
                SourceInput {
                    id: "b".into(),
                    subject_id: "test".into(),
                    role: EvidenceRole::TargetStatement,
                    text: "Acquire ten thousand paying customers this month".into(),
                    ..Default::default()
                },
            ])
            .unwrap();
        let passages = passages(&state);
        let relationships =
            semantic_candidates(&state, &passages, &[vec![1.0, 0.0], vec![1.0, 0.0]], "test");
        assert_eq!(relationships.len(), 1);
        store
            .apply_discovery(DiscoveryOutput {
                relationships,
                embedding_status: "test".into(),
                embedding_version: None,
            })
            .unwrap();
        assert!(store.context_packet(ContextRequest::default()).unwrap().relationships.is_empty(),
            "Raw candidate similarity must be assessed before becoming personal relationship context");
    }
    #[test]
    fn semantic_discovery_has_two_exact_receipts_and_never_invents_causality() {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            EvidenceStore::new(dir.path().to_path_buf(), "bryan".into(), "Bryan".into()).unwrap();
        let make = |id: &str, text: &str| SourceInput {
            id: id.into(),
            text: text.into(),
            subject_id: "bryan".into(),
            role: EvidenceRole::TargetStatement,
            ..Default::default()
        };
        let snapshot = store
            .reconcile_sources(vec![
                make("a", "Protect customer trust when choosing release quality"),
                make(
                    "b",
                    "Release quality protects customer trust through careful review",
                ),
                make("nav", "Previous: [[Chapter One]]\nNext: [[Chapter Three]]"),
            ])
            .unwrap();
        let passages = passages(&snapshot);
        let relations = semantic_candidates(
            &snapshot,
            &passages,
            &vec![vec![1.0, 0.0]; passages.len()],
            "test-version",
        );
        assert!(!relations.is_empty());
        assert!(relations
            .iter()
            .all(|r| r.relation == RelationshipKind::Related
                && !r.from_id.starts_with("nav")
                && !r.to_id.starts_with("nav")));
        for relationship in &relations {
            validate_receipt(&snapshot, &relationship.from_receipt, false).unwrap();
            validate_receipt(&snapshot, &relationship.to_receipt, false).unwrap();
        }
        let output = DiscoveryOutput {
            relationships: relations,
            embedding_version: None,
            embedding_status: "ready test-version".into(),
        };
        let saved = store.apply_discovery(output.clone()).unwrap();
        let id = &saved.relationships[0].id;
        store
            .review_relationship(id, ReviewStatus::Rejected, None)
            .unwrap();
        let again = store.apply_discovery(output.clone()).unwrap();
        assert_eq!(again.relationships[0].review_status, ReviewStatus::Rejected);
        let mut relabeled = output;
        relabeled.relationships[0].relation = RelationshipKind::Expands;
        relabeled.relationships[0].id = jobs::relationship_id(&relabeled.relationships[0]);
        assert_eq!(
            store
                .apply_discovery(relabeled)
                .unwrap()
                .relationships
                .len(),
            1
        );
    }

    #[test]
    fn missing_embedding_runtime_emits_no_keyword_edges() {
        let output = pending("embeddinggemma not installed");
        assert!(output.relationships.is_empty());
        assert!(output.embedding_status.starts_with("pending:"));
        assert!(cosine(&[1.0], &[1.0, 2.0]).is_none());
    }
}

#[cfg(test)]
mod model_lifecycle_tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, EvidenceStore, Relationship) {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            EvidenceStore::new(dir.path().into(), "test".into(), "Test".into()).unwrap();
        let state = store
            .reconcile_sources(vec![
                SourceInput {
                    id: "a".into(),
                    subject_id: "test".into(),
                    role: EvidenceRole::TargetStatement,
                    text: "Protect customer trust when choosing release quality".into(),
                    ..Default::default()
                },
                SourceInput {
                    id: "b".into(),
                    subject_id: "test".into(),
                    role: EvidenceRole::TargetStatement,
                    text: "Release quality protects customer trust through careful review".into(),
                    ..Default::default()
                },
            ])
            .unwrap();
        let passages = passages(&state);
        let relation = semantic_candidates(
            &state,
            &passages,
            &[vec![1.0, 0.0], vec![1.0, 0.0]],
            "model-a",
        )
        .remove(0);
        store
            .apply_discovery(output(vec![relation.clone()], "model-a"))
            .unwrap();
        (dir, store, relation)
    }

    fn output(relationships: Vec<Relationship>, version: &str) -> DiscoveryOutput {
        DiscoveryOutput {
            relationships,
            embedding_status: format!("ready: {version}"),
            embedding_version: Some(version.into()),
        }
    }

    #[test]
    fn changed_model_and_same_model_refresh_scores_preserving_confirmed_correction() {
        let (_dir, mut store, mut candidate) = fixture();
        store.state.relationships[0].conditions =
            vec!["Same release-quality and customer-trust context".into()];
        store.state.relationships[0].assessment = Some(assessment::PairAssessment {
            model_version: "scorer-a".into(),
            raw_response: Some("Preserved audit payload".into()),
            ..Default::default()
        });
        store
            .review_relationship(
                &candidate.id,
                ReviewStatus::Confirmed,
                Some(RelationshipKind::Supports),
            )
            .unwrap();
        let reviewed = store.snapshot().unwrap().relationships[0].clone();
        for score in [0.86, 0.93] {
            candidate.similarity = Some(score);
            candidate.model_version = Some("model-b".into());
            let state = store
                .apply_discovery(output(vec![candidate.clone()], "model-b"))
                .unwrap();
            assert_eq!(state.relationships.len(), 1);
            let mut expected = reviewed.clone();
            expected.similarity = Some(score);
            expected.model_version = Some("model-b".into());
            assert_eq!(
                serde_json::to_value(&state.relationships[0]).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
    }

    #[test]
    fn model_change_without_edges_clears_scores_but_keeps_reviewed_evidence() {
        let (_dir, mut store, candidate) = fixture();
        store
            .review_relationship(&candidate.id, ReviewStatus::Confirmed, None)
            .unwrap();
        let before = store.context_packet(ContextRequest::default()).unwrap();
        assert_eq!(before.relationships.len(), 1);
        let state = store.apply_discovery(output(vec![], "model-b")).unwrap();
        assert_eq!(state.relationships[0].similarity, None);
        assert_eq!(
            state.relationships[0].model_version.as_deref(),
            Some("model-b")
        );
        assert!(!state.relationships[0].invalidated);
        assert_eq!(
            state.relationships[0].review_status,
            ReviewStatus::Confirmed
        );
        assert_eq!(
            store
                .context_packet(ContextRequest::default())
                .unwrap()
                .relationships
                .len(),
            1
        );
    }

    #[test]
    fn rejected_pair_and_pending_runtime_keep_previous_metadata() {
        let (_dir, mut store, mut candidate) = fixture();
        store.state.relationships[0].assessment = Some(assessment::PairAssessment {
            model_version: "scorer-a".into(),
            raw_response: Some("Preserved audit payload".into()),
            ..Default::default()
        });
        store
            .review_relationship(&candidate.id, ReviewStatus::Rejected, None)
            .unwrap();
        let before = serde_json::to_value(store.snapshot().unwrap().relationships).unwrap();
        candidate.model_version = Some("model-b".into());
        candidate.similarity = Some(0.85);
        let state = store
            .apply_discovery(output(vec![candidate], "model-b"))
            .unwrap();
        assert_eq!(serde_json::to_value(&state.relationships).unwrap(), before);
        let state = store
            .apply_discovery(pending("runtime unavailable"))
            .unwrap();
        assert_eq!(serde_json::to_value(&state.relationships).unwrap(), before);
        let legacy: DiscoveryOutput = serde_json::from_value(
            serde_json::json!({"relationships":[],"embedding_status":"ready legacy"}),
        )
        .unwrap();
        assert!(legacy.embedding_version.is_none());
    }
}
