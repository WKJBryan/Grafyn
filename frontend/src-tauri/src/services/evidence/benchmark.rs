//! Developer-authored synthetic relationship evaluation; never a personal accuracy study.

use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

const FIXTURE: &str = include_str!("../../../tests/fixtures/contextual_relationships.json");
const OLLAMA_URL: &str = "http://127.0.0.1:11434";
const SUBJECT: &str = "synthetic-relationship-benchmark";

#[derive(Serialize, Deserialize)]
struct FixtureSuite {
    version: String,
    authorship: String,
    pairs: Vec<LabeledPair>,
}

#[derive(Serialize, Deserialize)]
struct LabeledPair {
    id: String,
    category: String,
    left: String,
    right: String,
    expected_verdict: String,
    expected_direction: String,
    label_reason: String,
}

#[derive(Serialize)]
struct PairRun {
    fixture_id: String,
    discovered: Option<bool>,
    assessment: Option<Value>,
    call_error: Option<String>,
}

fn fixtures() -> Result<FixtureSuite> {
    let suite: FixtureSuite = serde_json::from_str(FIXTURE)?;
    let mut ids = BTreeSet::new();
    for pair in &suite.pairs {
        ensure!(ids.insert(&pair.id), "Duplicate fixture ID");
        ensure!(
            !pair.left.trim().is_empty() && !pair.right.trim().is_empty(),
            "Empty fixture passage"
        );
        ensure!(
            matches!(
                pair.expected_verdict.as_str(),
                "equivalent"
                    | "related"
                    | "conflicts"
                    | "enables"
                    | "inhibits"
                    | "requires"
                    | "unrelated"
                    | "insufficient"
            ),
            "Unknown fixture verdict"
        );
        let effect = matches!(
            pair.expected_verdict.as_str(),
            "enables" | "inhibits" | "requires"
        );
        ensure!(
            if effect {
                matches!(
                    pair.expected_direction.as_str(),
                    "left_to_right" | "right_to_left"
                )
            } else {
                pair.expected_direction == "symmetric"
            },
            "Invalid fixture direction"
        );
    }
    Ok(suite)
}

fn create_output(data: &Path) -> Result<()> {
    // create_dir is the reservation: even an existing empty folder is refused.
    std::fs::create_dir(data).context(
        "Benchmark --data must be a new directory outside the vault, with an existing parent",
    )
}

fn write_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    Ok(())
}

fn corpus(pairs: &[LabeledPair]) -> EvidenceSnapshot {
    let mut state = EvidenceSnapshot {
        schema_version: 1,
        subject_id: SUBJECT.into(),
        subject_name: "Synthetic fixture subject".into(),
        ..Default::default()
    };
    for (index, pair) in pairs.iter().enumerate() {
        for (side, text) in [("a", &pair.left), ("b", &pair.right)] {
            // Gold labels, categories and descriptive fixture IDs never enter model input.
            let id = format!("source-{index:02}-{side}");
            state.sources.push(SourceRecord {
                input: SourceInput {
                    id: id.clone(),
                    title: id.clone(),
                    text: text.clone(),
                    subject_id: SUBJECT.into(),
                    role: EvidenceRole::TargetStatement,
                    source_group: id,
                    ..Default::default()
                },
                revision: 1,
                content_hash: content_hash(text),
                recorded_at: now(),
                deleted: false,
                interview: false,
            });
        }
    }
    state
}

fn candidate(state: &EvidenceSnapshot, index: usize) -> Result<Relationship> {
    let source = |side: &str| {
        state
            .sources
            .iter()
            .find(|s| s.input.id == format!("source-{index:02}-{side}"))
            .context("Fixture source missing")
    };
    let (left, right) = (source("a")?, source("b")?);
    Ok(Relationship {
        id: format!("pair-{index:02}"),
        subject_id: SUBJECT.into(),
        from_id: left.input.id.clone(),
        to_id: right.input.id.clone(),
        from_receipt: whole_receipt(left),
        to_receipt: whole_receipt(right),
        provenance: "local_semantic_similarity".into(),
        explanation: "Synthetic passage pair supplied for assessment; no relationship is assumed."
            .into(),
        recorded_at: now(),
        ..Default::default()
    })
}

fn discovered_pair(candidate: &Relationship, discovered: &[Relationship]) -> bool {
    discovered.iter().any(|r| {
        let same = |a: &Receipt, b: &Receipt| {
            a.source_id == b.source_id && a.source_revision == b.source_revision
        };
        (same(&candidate.from_receipt, &r.from_receipt)
            && same(&candidate.to_receipt, &r.to_receipt))
            || (same(&candidate.from_receipt, &r.to_receipt)
                && same(&candidate.to_receipt, &r.from_receipt))
    })
}

fn ratio(numerator: usize, denominator: usize, available: bool) -> Value {
    json!({"numerator":if available {Some(numerator)} else {None}, "denominator":denominator,
        "value":if available && denominator > 0 {Some(numerator as f64 / denominator as f64)} else {None}})
}

fn metrics(pairs: &[LabeledPair], runs: &[PairRun], discovery_available: bool) -> Value {
    let relevant =
        |p: &&LabeledPair| !matches!(p.expected_verdict.as_str(), "unrelated" | "insufficient");
    let relevant_count = pairs.iter().filter(relevant).count();
    let discovered_relevant = pairs
        .iter()
        .filter(relevant)
        .filter(|p| {
            runs.iter()
                .any(|r| r.fixture_id == p.id && r.discovered == Some(true))
        })
        .count();
    let mut agreement = 0;
    let mut accepted = 0;
    let mut accepted_agreement = 0;
    let mut valid = 0;
    let mut abstentions = 0;
    let mut unrelated = 0;
    let mut direction_errors = 0;
    let mut direction_denominator = 0;
    let mut versions = BTreeSet::new();
    let mut prompt_versions = BTreeSet::new();
    let mut failures = BTreeMap::<String, usize>::new();
    let mut missing_identity = 0;
    for run in runs {
        let Some(pair) = pairs.iter().find(|p| p.id == run.fixture_id) else {
            continue;
        };
        let assessment = run.assessment.as_ref();
        if let Some(version) = assessment
            .and_then(|a| a["model_version"].as_str())
            .filter(|v| !v.is_empty())
        {
            versions.insert(version.to_owned());
        } else {
            missing_identity += 1;
        }
        if let Some(version) = assessment.and_then(|a| a["prompt_version"].as_str()) {
            prompt_versions.insert(version.to_owned());
        }
        if run.call_error.is_some() {
            *failures.entry("provider_failed".into()).or_default() += 1;
            continue;
        }
        let Some(a) = assessment else {
            *failures.entry("missing_assessment".into()).or_default() += 1;
            continue;
        };
        if !a["error"].is_null() || a["result"].is_null() {
            *failures
                .entry(
                    a["error_kind"]
                        .as_str()
                        .unwrap_or("unclassified_failure")
                        .into(),
                )
                .or_default() += 1;
            continue;
        }
        let result = &a["result"];
        let verdict = result["verdict"].as_str().unwrap_or("");
        valid += 1;
        agreement += usize::from(verdict == pair.expected_verdict);
        match verdict {
            "insufficient" => abstentions += 1,
            "unrelated" => unrelated += 1,
            _ => {
                accepted += 1;
                accepted_agreement += usize::from(verdict == pair.expected_verdict);
                if pair.expected_direction != "symmetric" {
                    direction_denominator += 1;
                    direction_errors +=
                        usize::from(result["direction"].as_str() != Some(&pair.expected_direction));
                }
            }
        }
    }
    let count = |key: &str| failures.get(key).copied().unwrap_or(0);
    let comparable = versions.len() == 1
        && prompt_versions.len() <= 1
        && missing_identity == 0
        && runs.len() == pairs.len()
        && count("stale_model") == 0;
    json!({
        "comparable": comparable,
        "comparability_note":"Comparable requires one unchanged reported model identity across all pairs; raw metrics remain diagnostic when false.",
        "model_versions":versions, "prompt_versions":prompt_versions, "missing_model_identity":missing_identity,
        "total_labeled_pairs":pairs.len(), "attempted_pairs":runs.len(), "unattempted_pairs":pairs.len().saturating_sub(runs.len()),
        "candidate_recall":ratio(discovered_relevant,relevant_count,discovery_available),
        "candidate_recall_definition":"Gold labels other than unrelated/insufficient are relevant. Only intended labeled pairs are scored; combined-corpus cross-pair candidates have no gold labels.",
        "typed_label_agreement_all_pairs":ratio(agreement,pairs.len(),true),
        "typed_label_agreement_accepted_only":ratio(accepted_agreement,accepted,true),
        "accepted_only_definition":"Valid relationship judgments excluding unrelated and insufficient; this is selective agreement, not human endorsement.",
        "valid_judgments":valid, "accepted_relationship_judgments":accepted, "abstentions":abstentions,"unrelated_judgments":unrelated,
        "provider_failures":count("provider_failed"),
        "parse_or_evidence_failures":count("parse_failed")+count("invalid_evidence"),
        "failures_by_kind":failures,
        "direction_errors":ratio(direction_errors,direction_denominator,true),
        "direction_error_definition":"Wrong direction among accepted relationship judgments on gold-directional pairs; abstentions/failures remain visible separately.",
        "gold_directional_pairs":pairs.iter().filter(|p|p.expected_direction != "symmetric").count()
    })
}

/// Runs fixed synthetic fixtures locally, outside any evidence vault. No downloads or training.
pub async fn run(data: PathBuf) -> Result<Value> {
    let suite = fixtures()?;
    create_output(&data)?;
    write_new(&data.join("fixtures.json"), &suite)?;
    let state = corpus(&suite.pairs);
    write_new(&data.join("sources.json"), &state)?;
    let discovery = discover_relationships(&state, data.join("embedding-cache"), OLLAMA_URL).await;
    let (relationships, discovery_status, discovery_available) = match discovery {
        Ok(output) => {
            let available = output.embedding_status.starts_with("ready");
            write_new(&data.join("discovery.json"), &output)?;
            (output.relationships, output.embedding_status, available)
        }
        Err(error) => {
            let status = format!("pending: {error}");
            write_new(
                &data.join("discovery.json"),
                &json!({"embedding_status":status,"error":error.to_string()}),
            )?;
            (vec![], status, false)
        }
    };
    let mut runs = Vec::new();
    for (index, pair) in suite.pairs.iter().enumerate() {
        // Score every intended pair, even when discovery missed it or could not run.
        let candidate = candidate(&state, index)?;
        let discovered = discovery_available.then(|| discovered_pair(&candidate, &relationships));
        let (assessment, call_error) =
            match super::assess_pair(&state, &candidate, OLLAMA_URL).await {
                Ok(result) => (Some(serde_json::to_value(result)?), None),
                Err(error) => (None, Some(error.to_string())),
            };
        let run = PairRun {
            fixture_id: pair.id.clone(),
            discovered,
            assessment,
            call_error,
        };
        write_new(
            &data.join(format!("pair-{:02}.json", index + 1)),
            &json!({"fixture":pair,"candidate":candidate,"run":run}),
        )?;
        runs.push(run);
        eprintln!(
            "Relationship assessment {}/{} saved",
            index + 1,
            suite.pairs.len()
        );
    }
    let summary = metrics(&suite.pairs, &runs, discovery_available);
    let report = json!({"schema_version":1,"fixture_version":suite.version,"authorship":suite.authorship,
        "completed_at":now(),"ollama_url":OLLAMA_URL,"discovery_status":discovery_status,
        "candidate_count_in_combined_corpus":relationships.len(),"metrics":summary,"pairs":runs,
        "scope":"Small developer-authored synthetic baseline, not private/person-specific accuracy, causal validation or training data."});
    write_new(&data.join("report.json"), &report)?;
    Ok(
        json!({"report":data.join("report.json"),"discovery_status":discovery_status,"metrics":summary}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(id: &str, verdict: &str, direction: &str) -> PairRun {
        PairRun {
            fixture_id: id.into(),
            discovered: Some(true),
            assessment: Some(json!({
                "result": {"verdict":verdict,"direction":direction}, "model_version":"fixed-digest",
                "error":null
            })),
            call_error: None,
        }
    }

    #[test]
    fn pending_discovery_is_unavailable_and_abstentions_keep_the_all_pair_denominator() {
        let suite = fixtures().unwrap();
        let runs = vec![
            run("equivalent_delivery", "equivalent", "symmetric"),
            run("equivalent_release", "insufficient", "symmetric"),
        ];
        let report = metrics(&suite.pairs, &runs, false);
        assert_eq!(report["candidate_recall"]["value"], serde_json::Value::Null);
        assert_eq!(report["candidate_recall"]["denominator"], 10);
        assert_eq!(report["typed_label_agreement_all_pairs"]["denominator"], 12);
        assert_eq!(report["typed_label_agreement_all_pairs"]["numerator"], 1);
        assert_eq!(
            report["typed_label_agreement_accepted_only"]["denominator"],
            1
        );
        assert_eq!(report["abstentions"], 1);
        assert_eq!(report["unattempted_pairs"], 10);
    }

    #[test]
    fn wrong_direction_and_model_changes_are_not_hidden_by_label_agreement() {
        let suite = fixtures().unwrap();
        let mut runs = vec![
            run("installation_enables_reports", "enables", "left_to_right"),
            run("reopening_requires_signoff", "requires", "right_to_left"),
        ];
        runs[1].assessment.as_mut().unwrap()["model_version"] = json!("changed-digest");
        let report = metrics(&suite.pairs, &runs, true);
        assert_eq!(report["direction_errors"]["numerator"], 1);
        assert_eq!(report["direction_errors"]["denominator"], 2);
        assert_eq!(report["typed_label_agreement_all_pairs"]["numerator"], 2);
        assert_eq!(report["comparable"], false);
    }

    #[test]
    fn invalid_receipts_provider_failures_and_abstention_have_separate_counts() {
        let suite = fixtures().unwrap();
        let runs = vec![
            PairRun {
                fixture_id: "equivalent_delivery".into(),
                discovered: None,
                assessment: Some(
                    json!({"result":null,"error":"Quote absent", "error_kind":"invalid_evidence",
                "model_version":"fixed-digest"}),
                ),
                call_error: None,
            },
            PairRun {
                fixture_id: "equivalent_release".into(),
                discovered: None,
                assessment: None,
                call_error: Some("Local model unavailable".into()),
            },
            run("unknown_mechanism", "insufficient", "symmetric"),
        ];
        let report = metrics(&suite.pairs, &runs, false);
        assert_eq!(report["parse_or_evidence_failures"], 1);
        assert_eq!(report["provider_failures"], 1);
        assert_eq!(report["abstentions"], 1);
        assert_eq!(report["typed_label_agreement_all_pairs"]["numerator"], 1);
    }

    #[test]
    fn fixtures_are_complete_synthetic_and_output_directory_is_never_reused() {
        let suite = fixtures().unwrap();
        assert_eq!(suite.pairs.len(), 12);
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("run");
        create_output(&output).unwrap();
        assert!(create_output(&output).is_err());
        assert!(suite
            .pairs
            .iter()
            .all(|p| p.left.len() >= 24 && p.right.len() >= 24));
    }

    #[test]
    fn combined_corpus_has_exact_receipts_without_label_leakage_and_matches_reversed_discovery() {
        let suite = fixtures().unwrap();
        let state = corpus(&suite.pairs);
        assert_eq!(state.sources.len(), 24);
        for (index, pair) in suite.pairs.iter().enumerate() {
            let relation = candidate(&state, index).unwrap();
            super::super::jobs::validate_relationship(&state, &relation).unwrap();
            assert!(!relation.explanation.is_empty());
            let serialized = serde_json::to_string(&relation).unwrap();
            assert!(
                !serialized.contains(&pair.id),
                "Gold fixture IDs must not reach the scorer"
            );
            assert!(!serialized.contains(&pair.label_reason));
            assert!(relation.from_receipt.source_id < relation.to_receipt.source_id);
            validate_receipt(&state, &relation.from_receipt, true).unwrap();
            validate_receipt(&state, &relation.to_receipt, true).unwrap();
            let mut reversed = relation.clone();
            std::mem::swap(&mut reversed.from_receipt, &mut reversed.to_receipt);
            assert!(discovered_pair(&relation, &[reversed]));
            let other = candidate(&state, (index + 1) % suite.pairs.len()).unwrap();
            assert!(!discovered_pair(&relation, &[other]));
        }
    }
}
