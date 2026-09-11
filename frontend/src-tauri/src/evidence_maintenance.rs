//! Explicit-path, headless evidence inspection and reversible derived-state repair.
#![allow(dead_code)]
mod models;
mod services;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use serde_json::json;
use services::{atomic_io::write_atomic, evidence_repair, knowledge_store::KnowledgeStore};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "grafyn-evidence-maintenance")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct RepairPaths {
    #[arg(long)]
    vault: PathBuf,
    #[arg(long)]
    twin_root: PathBuf,
    /// Isolated cache directory, alongside the copied artifacts.
    #[arg(long)]
    data: PathBuf,
    #[arg(long)]
    manifest: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    /// Run a synthetic, file-backed service smoke in a new directory.
    Smoke {
        #[arg(long)]
        data: PathBuf,
    },
    /// Evaluate synthetic contextual pairs locally; data must be a new folder outside a vault.
    RelationshipBenchmark {
        #[arg(long)]
        data: PathBuf,
    },
    /// Inspect only; writes a reviewable manifest, no derived-state changes.
    Preview(RepairPaths),
    /// Snapshot and apply an existing manifest; conflicts never overwrite edits.
    Apply(RepairPaths),
    /// Restore a persisted snapshot when repaired bytes are still unchanged.
    Rollback(RepairPaths),
    /// Parse an import through the shared importer; output counts and issue codes only.
    InspectImport {
        #[arg(long)]
        file: PathBuf,
        /// JSON file containing columns, first_data_row and optional sheet.
        #[arg(long)]
        mapping_json: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Smoke { data } => smoke(data)?,
        Command::RelationshipBenchmark { data } => {
            println!("{}", services::evidence::benchmark::run(data).await?);
        }
        Command::InspectImport { file, mapping_json } => {
            let mapping = mapping_json
                .map(|path| -> Result<models::import::ImportTableMapping> {
                    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
                })
                .transpose()?;
            let parsed = services::import::file::parse_import_file(
                &file.to_string_lossy(),
                mapping.as_ref(),
            )
            .await
            .map_err(anyhow::Error::msg)?;
            match parsed {
                services::import::file::ParsedImport::Conversations {
                    platform,
                    conversations,
                } => println!(
                    "{}",
                    json!({"platform":platform,"items":conversations.len()})
                ),
                services::import::file::ParsedImport::Document(batch) => {
                    let mut issues = BTreeMap::<String, usize>::new();
                    let mut answered = 0;
                    let mut unanswered = 0;
                    let mut receipts = 0;
                    let mut model_outputs = 0;
                    let mut review_cases = 0;
                    for item in &batch.items {
                        for case in item
                            .metadata
                            .get("decision_cases")
                            .and_then(|v| v.as_array())
                            .into_iter()
                            .flatten()
                        {
                            if case["target_answer"].is_null() {
                                unanswered += 1;
                            } else {
                                answered += 1;
                            }
                            receipts += case["receipts"].as_array().map_or(0, |v| v.len());
                            model_outputs +=
                                case["model_outputs"].as_array().map_or(0, |v| v.len());
                            let case_issues =
                                case["issues"].as_array().cloned().unwrap_or_default();
                            if !case_issues.is_empty() {
                                review_cases += 1;
                            }
                            for issue in case_issues {
                                if let Some(issue) = issue.as_str() {
                                    *issues.entry(issue.into()).or_default() += 1;
                                }
                            }
                        }
                    }
                    println!(
                        "{}",
                        json!({"items":batch.items.len(),"answered":answered,"unanswered":unanswered,"review_cases":review_cases,"receipts":receipts,"model_outputs":model_outputs,"issues":issues})
                    );
                }
            }
        }
        command => {
            let (paths, mode) = match command {
                Command::Preview(paths) => (paths, "preview"),
                Command::Apply(paths) => (paths, "apply"),
                Command::Rollback(paths) => (paths, "rollback"),
                _ => unreachable!(),
            };
            let vault = paths
                .vault
                .canonicalize()
                .context("Vault must already exist")?;
            let twin_root = paths
                .twin_root
                .canonicalize()
                .context("Twin root must already exist")?;
            std::fs::create_dir_all(&paths.data)?;
            let data = paths.data.canonicalize()?;
            if data.starts_with(&vault) || data.starts_with(&twin_root) {
                anyhow::bail!("Use an isolated --data directory outside the vault and twin root");
            }
            if mode == "preview" {
                let store = KnowledgeStore::new(vault, data);
                let notes = store
                    .list_notes()?
                    .into_iter()
                    .map(|meta| store.get_note(&meta.id))
                    .collect::<Result<Vec<_>>>()?;
                let manifest = evidence_repair::preview(&twin_root, &notes)?;
                if let Some(parent) = paths.manifest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                write_atomic(&paths.manifest, &serde_json::to_vec_pretty(&manifest)?)?;
                println!(
                    "{}",
                    json!({"changes":manifest.changes.len(),"review_only":manifest.review_only.len(),"manifest":paths.manifest})
                );
            } else {
                let manifest: evidence_repair::RepairManifest =
                    serde_json::from_slice(&std::fs::read(&paths.manifest)?)?;
                let lock = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(twin_root.join(".evidence-repair.lock"))?;
                fs2::FileExt::try_lock_exclusive(&lock).context("Another repair is running")?;
                let result = if mode == "apply" {
                    evidence_repair::apply(&twin_root, &manifest)?
                } else {
                    evidence_repair::rollback(&twin_root, &manifest)?
                };
                println!("{}", serde_json::to_string(&result)?);
                if !result.conflicts.is_empty() {
                    anyhow::bail!("Repair conflicts require review; edited records were preserved");
                }
            }
        }
    }
    Ok(())
}

fn smoke(data: PathBuf) -> Result<()> {
    use anyhow::ensure;
    use services::evidence::{Domain, InterviewDraft, RelationshipKind};
    use services::evidence_bridge::{pilot_location, shared_context, transaction};
    // Refuse an occupied target, then leave all evidence available for inspection.
    if data.exists() {
        ensure!(
            std::fs::read_dir(&data)?.next().is_none(),
            "Smoke --data must be empty or absent"
        );
    }
    std::fs::create_dir_all(&data)?;
    let run = data.join(format!("smoke-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&run)?;
    let location = pilot_location(&run);
    let mut product = InterviewDraft {
        id: "synthetic-product".into(),
        subject_id: location.subject_id.clone(),
        subject_name: location.subject_name.clone(),
        domain: Domain::ProductProject,
        situation: "A synthetic prototype has uncertain reliability before a demonstration.".into(),
        options: vec![
            "Demonstrate immediately".into(),
            "Test the prototype first".into(),
        ],
        wanted: "Demonstrate dependable behavior".into(),
        expected: "Find defects before the demonstration".into(),
        rationale: "The demonstration should use a working prototype.".into(),
        expected_goal_relation: Some(RelationshipKind::ContributesTo),
        step: 3,
        ..Default::default()
    };
    transaction(&location, |store| {
        store.save_interview(product.clone(), false)
    })?;
    let partial = transaction(&location, |store| store.snapshot())?;
    ensure!(
        partial
            .interview_draft
            .as_ref()
            .is_some_and(|d| d.step == 3)
            && partial.cases.is_empty(),
        "Partial interview did not resume without inventing a choice"
    );
    product.chosen = "Test the prototype first".into();
    let first = transaction(&location, |store| {
        store.save_interview(product.clone(), true)
    })?;
    ensure!(
        first.cases.len() == 1
            && first.sources.len() == 1
            && first.nodes.len() == 2
            && first.relationships.len() == 2,
        "Product decision path was not captured"
    );
    let again = transaction(&location, |store| {
        store.save_interview(product.clone(), true)
    })?;
    ensure!(
        again.cases.len() == first.cases.len()
            && again.sources.len() == first.sources.len()
            && again.relationships.len() == first.relationships.len(),
        "Repeated interview submission duplicated evidence"
    );
    let everyday = InterviewDraft {
        id: "synthetic-everyday".into(),
        subject_id: location.subject_id.clone(),
        subject_name: location.subject_name.clone(),
        domain: Domain::Everyday,
        situation: "A synthetic free evening after a tiring day.".into(),
        options: vec!["Watch a film".into(), "Take a walk".into()],
        wanted: "Feel rested".into(),
        expected: "Spend time outdoors away from a screen".into(),
        chosen: "Take a walk".into(),
        rationale: "I wanted quiet time outside.".into(),
        expected_goal_relation: Some(RelationshipKind::ContributesTo),
        ..Default::default()
    };
    let both = transaction(&location, |store| store.save_interview(everyday, true))?;
    ensure!(
        both.cases.len() == 2 && both.goals.len() == 2,
        "Second domain replaced an existing decision or goal"
    );
    let mut revised = both.goals[0].input.clone();
    revised.contextual_priority = Some("Prefer reliability for this demonstration".into());
    revised.effective_at = Some("2026-01-02T00:00:00Z".into());
    revised.reason = "Synthetic stated priority clarification".into();
    let revision = transaction(&location, |store| store.save_goal(revised))?;
    ensure!(
        revision.revision == 2,
        "Goal update did not create a revision"
    );
    let reloaded = transaction(&location, |store| store.snapshot())?;
    ensure!(
        reloaded.goals.len() == 3 && reloaded.cases[0].goal_revisions[0].revision == 1,
        "Later goal revision rewrote a historical case"
    );
    let context = shared_context(&location, "prototype demonstration evening")?;
    ensure!(
        context.cases.len() == 2
            && context.relationships.len() == 4
            && !context.source_revisions.is_empty(),
        "Shared context omitted captured evidence or paths"
    );
    for receipt in &context.source_revisions {
        let source = reloaded
            .sources
            .iter()
            .find(|s| s.input.id == receipt.source_id && s.revision == receipt.source_revision)
            .context("Missing source receipt")?;
        ensure!(
            source.input.text.get(receipt.start..receipt.end) == Some(receipt.quote.as_str()),
            "Context receipt does not match exact persisted source bytes"
        );
    }
    let mut restricted = reloaded
        .sources
        .iter()
        .find(|s| s.input.id == "interview:synthetic-product")
        .context("Missing product source")?
        .input
        .clone();
    restricted.restricted = true;
    transaction(&location, |store| store.reconcile_sources(vec![restricted]))?;
    let after = shared_context(&location, "prototype demonstration evening")?;
    ensure!(
        after.cases.len() == 1
            && after.cases[0].domain == Domain::Everyday
            && after
                .source_revisions
                .iter()
                .all(|r| r.source_id != "interview:synthetic-product"),
        "Restriction did not invalidate the source and derived context"
    );
    println!(
        "{}",
        json!({"status":"passed","data":run,"domains":2,"cases_before_restriction":context.cases.len(),"goal_revisions":reloaded.goals.len(),"relationships":context.relationships.len(),"cases_after_restriction":after.cases.len(),"partial_resume":true,"idempotence":true,"exact_receipts":true})
    );
    Ok(())
}
