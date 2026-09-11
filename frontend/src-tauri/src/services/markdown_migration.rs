use crate::models::migration::{
    ExpectedProgramTarget, MarkdownMigrationApplyResult, MarkdownMigrationMode,
    MarkdownMigrationNoteProposal, MarkdownMigrationPreview, MarkdownMigrationPreviewSummary,
    MarkdownMigrationRequest, MarkdownMigrationStatus, MarkdownMigrationTopicCandidate,
    MigrationAuthoritySnapshotV1,
};
#[cfg(test)]
use crate::models::note::NoteCreate;
use crate::models::note::{
    Note, NoteUpdate, CURRENT_NOTE_SCHEMA_VERSION, PROP_AUTO_INSERTED_LINK_IDS,
    PROP_INFERRED_LINK_IDS, PROP_TOPIC_ALIASES, PROP_TOPIC_KEY,
};
#[cfg(test)]
use crate::services::atomic_io::write_atomic;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::topic_hub::normalize_topic_key;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

mod recovery;
mod semantics;
mod state;
mod status;
mod transaction;
mod validation;
#[cfg(test)]
pub(crate) use transaction::MigrationMutationOutcome;

#[cfg(test)]
#[path = "markdown_migration_transaction_tests.rs"]
mod transaction_tests;

const MIGRATION_SOURCE_MARKDOWN: &str = "markdown_migration";
const MIGRATION_SOURCE_BACKFILL: &str = "grafyn_schema_backfill";
const MIGRATION_PREVIEW_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredManifest {
    #[serde(default)]
    schema_version: u16,
    run_id: String,
    preview_id: String,
    #[serde(default)]
    root_scope: Option<crate::models::twin_event::ContentDigest>,
    vault_path: String,
    mode: MarkdownMigrationMode,
    created_at: DateTime<Utc>,
    #[serde(default)]
    applied_at: Option<DateTime<Utc>>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    created_files: Vec<String>,
    #[serde(default)]
    backup_files: Vec<String>,
    #[serde(default)]
    overlay_note_ids: Vec<String>,
    #[serde(default)]
    touched_note_ids: Vec<String>,
    #[serde(default)]
    created_hub_note_ids: Vec<String>,
    /// Notes skipped by apply because their original frontmatter is unparsable
    /// and preserved verbatim (`Note::frontmatter_raw_fallback`).
    #[serde(default)]
    skipped_fallback_note_ids: Vec<String>,
    #[serde(default)]
    expected_program_target: Option<ExpectedProgramTarget>,
    #[serde(default)]
    program_after_digest: Option<crate::models::twin_event::ContentDigest>,
    #[serde(default)]
    program_path: Option<String>,
    #[serde(default)]
    request: Option<MarkdownMigrationRequest>,
    #[serde(default)]
    source_inventory: Vec<crate::models::migration::MarkdownMigrationSourceV1>,
    #[serde(default)]
    overlay_inventory: Vec<crate::models::migration::MarkdownMigrationOverlaySourceV1>,
    #[serde(default)]
    starting_authority: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    #[serde(default)]
    final_authority: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    #[serde(default)]
    operations: Vec<MigrationOperationV1>,
    #[serde(default)]
    #[serde(
        serialize_with = "serialize_usize_as_u64",
        deserialize_with = "deserialize_u64_as_usize"
    )]
    apply_next: usize,
    #[serde(default)]
    #[serde(
        serialize_with = "serialize_usize_as_u64",
        deserialize_with = "deserialize_u64_as_usize"
    )]
    rollback_next: usize,
    #[serde(default)]
    #[serde(
        serialize_with = "serialize_usize_as_u64",
        deserialize_with = "deserialize_u64_as_usize"
    )]
    rollback_total: usize,
    #[serde(default)]
    authority_only_advances: u64,
    #[serde(default)]
    active_step: Option<MigrationStepWitnessV1>,
    #[serde(default)]
    last_commit: Option<MigrationCommitProofV1>,
}

const MIGRATION_MANIFEST_SCHEMA_VERSION: u16 = 1;
const MAX_MIGRATION_RECORD_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationOperationV1 {
    #[serde(
        serialize_with = "serialize_usize_as_u64",
        deserialize_with = "deserialize_u64_as_usize"
    )]
    index: usize,
    target_kind: crate::services::twin_events::TargetKind,
    target_key: String,
    before: crate::services::twin_events::BeforeImage,
    after: crate::services::twin_events::BeforeImage,
    after_digest: crate::models::twin_event::ContentDigest,
    after_blob_key: Option<String>,
    before_blob_key: Option<String>,
    note_event: Option<StoredMigrationNoteEventV1>,
    rollback_note_event: Option<StoredMigrationNoteEventV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredMigrationNoteEventV1 {
    note_id: String,
    change: crate::models::twin_event::NoteChangeKind,
    observed_at: DateTime<Utc>,
    governance: crate::models::twin_event::Governance,
    payload_digest: crate::models::twin_event::ContentDigest,
    evidence_digest: crate::models::twin_event::ContentDigest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationStepWitnessV1 {
    direction: MigrationDirectionV1,
    #[serde(
        serialize_with = "serialize_usize_as_u64",
        deserialize_with = "deserialize_u64_as_usize"
    )]
    operation_index: usize,
    expected_authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
    #[serde(default)]
    intent: Option<crate::services::twin_events::MutationIntentV1>,
    #[serde(default)]
    committed_authority: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    #[serde(default)]
    progress_recorded: bool,
    #[serde(default)]
    receipt_consumed: bool,
    #[serde(default)]
    abort_recorded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MigrationDirectionV1 {
    Apply,
    Rollback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationCommitProofV1 {
    mutation_id: crate::models::twin_event::ContentDigest,
    authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
}

const MIGRATION_COMMIT_RECORD_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationCommitRecordV1 {
    schema_version: u16,
    run_id: String,
    mutation_id: crate::models::twin_event::ContentDigest,
    authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
    direction: MigrationDirectionV1,
    #[serde(
        serialize_with = "serialize_usize_as_u64",
        deserialize_with = "deserialize_u64_as_usize"
    )]
    operation_index: usize,
    outcome: MigrationCommitOutcomeV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MigrationCommitOutcomeV1 {
    Committed,
    AuthorityOnly,
}

#[derive(Debug, Clone)]
pub struct MarkdownMigrationService {
    data_path: PathBuf,
    runs_dir: PathBuf,
    migration_root: Option<std::sync::Arc<crate::services::twin_events::AnchoredRoot>>,
    #[cfg(test)]
    fail_manifest_write_at: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    blob_byte_limit: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    #[cfg(test)]
    status_scan_byte_limit: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl MarkdownMigrationService {
    pub fn new(data_path: PathBuf) -> Self {
        let fallback = data_path.clone();
        Self::try_new(data_path).unwrap_or_else(|error| {
            log::error!("Failed to initialize Markdown migration state: {error}");
            Self {
                runs_dir: fallback.join("vault_migration").join("runs"),
                data_path: fallback,
                migration_root: None,
                #[cfg(test)]
                fail_manifest_write_at: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                #[cfg(test)]
                blob_byte_limit: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(
                    transaction::MAX_MIGRATION_BLOB_BYTES,
                )),
                #[cfg(test)]
                status_scan_byte_limit: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(
                    status::MAX_MIGRATION_STATUS_SCAN_BYTES,
                )),
            }
        })
    }

    pub(crate) fn try_new(data_path: PathBuf) -> Result<Self> {
        let base_dir = data_path.join("vault_migration");
        let runs_dir = base_dir.join("runs");
        std::fs::create_dir_all(&runs_dir)?;
        std::fs::create_dir_all(base_dir.join("overlay").join("notes"))?;
        let migration_root = crate::services::twin_events::AnchoredRoot::open(&base_dir)
            .map_err(anyhow::Error::new)?;
        migration_root
            .open_directory("runs", true)
            .map_err(anyhow::Error::new)?;
        migration_root
            .open_directory("staging", true)
            .map_err(anyhow::Error::new)?;
        Ok(Self {
            data_path,
            runs_dir,
            migration_root: Some(std::sync::Arc::new(migration_root)),
            #[cfg(test)]
            fail_manifest_write_at: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            #[cfg(test)]
            blob_byte_limit: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(
                transaction::MAX_MIGRATION_BLOB_BYTES,
            )),
            #[cfg(test)]
            status_scan_byte_limit: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(
                status::MAX_MIGRATION_STATUS_SCAN_BYTES,
            )),
        })
    }

    pub(crate) fn uses_data_path(&self, data_path: &Path) -> bool {
        self.data_path == data_path
    }

    pub fn preview(
        &self,
        vault_path: PathBuf,
        request: MarkdownMigrationRequest,
    ) -> Result<MarkdownMigrationPreview> {
        let mut store = KnowledgeStore::new(vault_path.clone(), self.data_path.clone());
        let root_scope = crate::services::twin_events::root_identity_for_path(&vault_path)
            .map_err(anyhow::Error::new)?;
        // Compatibility helper for isolated service tests. Production callers
        // must use `preview_scoped` with the coordinator's exact authority.
        self.preview_scoped_with_schema(
            &mut store,
            crate::services::vault_namespace::VaultAuthorityTokenV1 {
                root_scope,
                lease_epoch_uuid: Uuid::nil().to_string(),
                authority_generation: 0,
            },
            request,
            0,
        )
    }

    pub(crate) fn preview_scoped(
        &self,
        store: &mut KnowledgeStore,
        authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
        request: MarkdownMigrationRequest,
    ) -> Result<MarkdownMigrationPreview> {
        self.preview_scoped_with_schema(store, authority, request, MIGRATION_PREVIEW_SCHEMA_VERSION)
    }

    fn preview_scoped_with_schema(
        &self,
        store: &mut KnowledgeStore,
        authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
        request: MarkdownMigrationRequest,
        schema_version: u16,
    ) -> Result<MarkdownMigrationPreview> {
        let vault_path = store.vault_path().to_path_buf();
        let created_at = Utc::now();
        let preview_id = Uuid::new_v4().to_string();
        let migration_root = self
            .migration_root
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("retained migration capability is unavailable"))?;
        let _transaction_lock = migration_root
            .lock_exclusive("runs/transaction.lock")
            .map_err(anyhow::Error::new)?;
        if schema_version == MIGRATION_PREVIEW_SCHEMA_VERSION
            && store.current_migration_authority()? != authority
        {
            anyhow::bail!("migration authority changed before preview scan");
        }
        let captured_authority = authority.clone();
        let hub_folder =
            canonical_hub_folder(request.hub_folder.as_deref().unwrap_or("_grafyn/hubs"))?;
        let program_path = canonical_program_path(
            request
                .program_path
                .as_deref()
                .unwrap_or("_grafyn/program.md"),
        )?;
        let request = canonical_migration_request(&request, &hub_folder, &program_path);
        let (source_snapshots, overlay_inventory) =
            store.migration_source_snapshot(&program_path, created_at)?;
        let source_inventory = source_snapshots
            .iter()
            .map(|snapshot| snapshot.source.clone())
            .collect::<Vec<_>>();
        let derived = semantics::derive_preview_semantics(
            &source_snapshots,
            &request,
            &hub_folder,
            &program_path,
        )?;

        let preview = MarkdownMigrationPreview {
            schema_version,
            preview_id: preview_id.clone(),
            root_scope: Some(authority.root_scope.clone()),
            authority: Some(MigrationAuthoritySnapshotV1 {
                root_scope: authority.root_scope,
                lease_epoch_uuid: authority.lease_epoch_uuid,
                authority_generation: authority.authority_generation,
            }),
            request: Some(request.clone()),
            vault_path: vault_path.to_string_lossy().to_string(),
            created_at: Some(created_at),
            mode: request.mode,
            hub_folder,
            program_path: program_path.clone(),
            expected_program_target: Some(derived.expected_program_target),
            program_after_digest: Some(derived.program_after_digest),
            summary: derived.summary,
            topic_candidates: derived.topic_candidates,
            note_proposals: derived.note_proposals,
            source_inventory,
            overlay_inventory,
            ambiguous_titles: derived.ambiguous_titles,
        };

        let (finish_sources, finish_overlays) =
            store.migration_source_snapshot(&program_path, created_at)?;
        let finish_inventory = finish_sources
            .into_iter()
            .map(|snapshot| snapshot.source)
            .collect::<Vec<_>>();
        if finish_inventory != preview.source_inventory
            || finish_overlays != preview.overlay_inventory
        {
            anyhow::bail!("migration sources changed while preview was being derived");
        }
        if schema_version == MIGRATION_PREVIEW_SCHEMA_VERSION
            && store.current_migration_authority()? != captured_authority
        {
            anyhow::bail!("migration authority changed while preview was being derived");
        }
        let preview_bytes = serde_json::to_vec_pretty(&preview)?;
        if preview_bytes.len() > MAX_MIGRATION_RECORD_BYTES {
            anyhow::bail!("migration preview exceeds its bounded record limit");
        }
        migration_root
            .put_atomic(&format!("runs/{preview_id}/preview.json"), &preview_bytes)
            .map_err(anyhow::Error::new)?;

        Ok(preview)
    }

    #[cfg(test)]
    pub fn apply(
        &self,
        preview_id: &str,
        request: MarkdownMigrationRequest,
        store: &mut KnowledgeStore,
    ) -> Result<MarkdownMigrationApplyResult> {
        let root_scope = crate::services::twin_events::root_identity_for_path(store.vault_path())
            .map_err(anyhow::Error::new)?;
        self.apply_legacy_scoped(preview_id, request, store, &root_scope)
    }

    #[cfg(test)]
    fn apply_legacy_scoped(
        &self,
        preview_id: &str,
        request: MarkdownMigrationRequest,
        store: &mut KnowledgeStore,
        expected_root_scope: &crate::models::twin_event::ContentDigest,
    ) -> Result<MarkdownMigrationApplyResult> {
        let preview = self.load_preview(preview_id)?;
        require_current_preview_scope(&preview, expected_root_scope, store.vault_path())?;
        let expected_program_target = preview
            .expected_program_target
            .clone()
            .ok_or_else(|| anyhow::anyhow!("legacy migration preview is audit-only"))?;
        let expected_before = expected_program_before_image(&expected_program_target);
        let program_contents =
            default_program_file_contents(&preview.hub_folder, &preview.program_path);
        let program_after_digest =
            crate::services::twin_events::digest_bytes(program_contents.as_bytes());
        if preview.program_after_digest.as_ref() != Some(&program_after_digest) {
            anyhow::bail!("migration preview program digest is missing or invalid");
        }

        // This validation/conditional create is deliberately the first apply action.
        // Its planner runs after journal recovery while the shared mutation lock is held.
        let program_created = matches!(expected_program_target, ExpectedProgramTarget::Absent);
        if program_created {
            store.put_vault_file_target_only_expected(
                &preview.program_path,
                program_contents.as_bytes(),
                "migration",
                Some(expected_before),
            )?;
        } else {
            store.validate_vault_file_target(&preview.program_path, expected_before)?;
        }
        let run_id = preview.preview_id.clone();
        let run_dir = self.runs_dir.join(&run_id);
        std::fs::create_dir_all(run_dir.join("backups"))?;

        let mut manifest = StoredManifest {
            run_id: run_id.clone(),
            preview_id: preview.preview_id.clone(),
            root_scope: preview.root_scope.clone(),
            vault_path: preview.vault_path.clone(),
            mode: request.mode.clone(),
            created_at: preview.created_at.unwrap_or_else(Utc::now),
            applied_at: Some(Utc::now()),
            status: "applied".to_string(),
            expected_program_target: preview.expected_program_target.clone(),
            program_after_digest: preview.program_after_digest.clone(),
            program_path: Some(preview.program_path.clone()),
            ..Default::default()
        };
        if program_created {
            manifest.created_files.push(preview.program_path.clone());
        }

        let mut touched_note_ids = Vec::new();
        let mut overlay_note_ids = Vec::new();
        let mut skipped_fallback_note_ids = Vec::new();

        for proposal in &preview.note_proposals {
            if request.mode == MarkdownMigrationMode::SidecarFirst {
                let overlay = json!({
                    "aliases": proposal.aliases,
                    "tags": proposal.inferred_tags,
                    "schema_version": CURRENT_NOTE_SCHEMA_VERSION,
                    "migration_source": MIGRATION_SOURCE_MARKDOWN,
                    "optimizer_managed": false,
                    "properties": {
                        PROP_TOPIC_KEY: proposal.topic_key,
                        PROP_INFERRED_LINK_IDS: proposal.inferred_link_ids,
                    }
                });
                store.write_overlay_from_source(&proposal.note_id, &overlay, "migration")?;
                overlay_note_ids.push(proposal.note_id.clone());
                continue;
            }

            let note = store.get_note(&proposal.note_id)?;

            // The note's original frontmatter failed to parse and is preserved
            // verbatim (see `Note::frontmatter_raw_fallback`). The update below
            // explicitly sets frontmatter-backed fields, which would clear the
            // fallback and permanently replace the unparsable original with
            // defaulted frontmatter. Skip it (consistent with
            // `backfill_legacy_grafyn_notes` and `vault_optimizer::run_next`) and
            // report it in the apply result/manifest.
            if note.frontmatter_raw_fallback.is_some() {
                log::warn!(
                    "Skipping markdown migration for note '{}': original frontmatter is unparsable and preserved verbatim",
                    note.id
                );
                skipped_fallback_note_ids.push(note.id.clone());
                continue;
            }

            if self.backup_note(&run_dir, &preview.vault_path, &note.relative_path)?
                && !manifest.backup_files.contains(&note.relative_path)
            {
                manifest.backup_files.push(note.relative_path.clone());
            }
            let mut properties = note.properties.clone();

            if let Some(topic_key) = &proposal.topic_key {
                properties.insert(PROP_TOPIC_KEY.to_string(), Value::String(topic_key.clone()));
                properties.insert(
                    PROP_TOPIC_ALIASES.to_string(),
                    Value::Array(
                        proposal
                            .aliases
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                );
            }
            if !proposal.inferred_link_ids.is_empty() {
                properties.insert(
                    PROP_INFERRED_LINK_IDS.to_string(),
                    Value::Array(
                        proposal
                            .inferred_link_ids
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                );
            }

            let new_content = if request.auto_insert_links.unwrap_or(false) {
                let (content, auto_inserted_ids) =
                    append_related_links(&note.content, &proposal.inferred_link_ids, store)?;
                if !auto_inserted_ids.is_empty() {
                    properties.insert(
                        PROP_AUTO_INSERTED_LINK_IDS.to_string(),
                        Value::Array(auto_inserted_ids.into_iter().map(Value::String).collect()),
                    );
                }
                Some(normalize_rewritten_content(
                    &note.title,
                    &content,
                    request.mode == MarkdownMigrationMode::FullRewrite,
                ))
            } else if request.mode == MarkdownMigrationMode::FullRewrite {
                Some(normalize_rewritten_content(
                    &note.title,
                    &note.content,
                    true,
                ))
            } else {
                None
            };

            let updated = store.update_note_from_source(
                &proposal.note_id,
                NoteUpdate {
                    title: None,
                    content: new_content,
                    relative_path: Some(note.relative_path.clone()),
                    aliases: Some(merge_unique_strings(note.aliases, proposal.aliases.clone())),
                    status: None,
                    tags: Some(merge_unique_strings(
                        note.tags,
                        proposal.inferred_tags.clone(),
                    )),
                    schema_version: Some(CURRENT_NOTE_SCHEMA_VERSION),
                    migration_source: Some(MIGRATION_SOURCE_MARKDOWN.to_string()),
                    optimizer_managed: Some(false),
                    properties: Some(properties),
                },
                "migration",
            )?;
            touched_note_ids.push(updated.id.clone());
        }

        let mut created_hub_note_ids = Vec::new();
        for topic in &preview.topic_candidates {
            if topic.reuse_existing_hub_id.is_some() {
                continue;
            }
            let content = format!(
                "# Hub: {}\n\nGrafyn will keep this topic hub updated from its member notes.\n",
                topic.display_name
            );
            let created = store.create_note_from_source(
                NoteCreate {
                    title: format!("Hub: {}", topic.display_name),
                    content,
                    relative_path: Some(format!(
                        "{}/{}.md",
                        preview.hub_folder,
                        slugify(&topic.display_name)
                    )),
                    aliases: vec![topic.display_name.clone()],
                    status: crate::models::note::NoteStatus::Canonical,
                    tags: vec!["hub".to_string()],
                    schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                    migration_source: Some(MIGRATION_SOURCE_MARKDOWN.to_string()),
                    optimizer_managed: true,
                    properties: HashMap::from([
                        (
                            PROP_TOPIC_KEY.to_string(),
                            Value::String(topic.topic_key.clone()),
                        ),
                        (
                            PROP_TOPIC_ALIASES.to_string(),
                            Value::Array(vec![Value::String(topic.display_name.clone())]),
                        ),
                        ("is_topic_hub".to_string(), Value::Bool(true)),
                    ]),
                },
                "migration",
            )?;
            created_hub_note_ids.push(created.id.clone());
            manifest.created_files.push(created.relative_path.clone());
        }

        manifest.overlay_note_ids = overlay_note_ids.clone();
        manifest.touched_note_ids = touched_note_ids.clone();
        manifest.created_hub_note_ids = created_hub_note_ids.clone();
        manifest.skipped_fallback_note_ids = skipped_fallback_note_ids.clone();
        write_atomic(
            &run_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest)?.as_bytes(),
        )?;

        let message = if skipped_fallback_note_ids.is_empty() {
            "Markdown migration applied".to_string()
        } else {
            format!(
                "Markdown migration applied ({} note(s) skipped: unparsable frontmatter preserved verbatim)",
                skipped_fallback_note_ids.len()
            )
        };

        Ok(MarkdownMigrationApplyResult {
            run_id,
            status: "applied".to_string(),
            created_hub_note_ids,
            touched_note_ids,
            overlay_note_ids,
            skipped_fallback_note_ids,
            message,
            warning: None,
            accepted_request: manifest.request.clone(),
        })
    }

    pub fn status(&self, run_id: Option<&str>) -> Result<MarkdownMigrationStatus> {
        let Some(target_id) = run_id
            .map(ToOwned::to_owned)
            .or_else(|| self.latest_run_id().ok().flatten())
        else {
            return Ok(MarkdownMigrationStatus {
                status: "idle".to_string(),
                ..Default::default()
            });
        };

        let preview = self.load_preview(&target_id).ok();
        let manifest = self.load_manifest(&target_id).ok();
        Ok(MarkdownMigrationStatus {
            run_id: Some(target_id.clone()),
            preview_id: Some(target_id),
            status: manifest
                .as_ref()
                .map(|value| value.status.clone())
                .unwrap_or_else(|| "previewed".to_string()),
            mode: preview.as_ref().map(|value| value.mode.clone()),
            created_at: preview.as_ref().and_then(|value| value.created_at),
            applied_at: manifest.as_ref().and_then(|value| value.applied_at),
            rollback_available: manifest.is_some(),
            summary: preview.map(|value| value.summary),
        })
    }

    pub(crate) fn status_scoped(
        &self,
        run_id: Option<&str>,
        expected_authority: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<MarkdownMigrationStatus> {
        self.status_transaction(run_id, expected_authority)
    }

    #[cfg(test)]
    pub fn rollback(&self, run_id: &str, store: &mut KnowledgeStore) -> Result<()> {
        let root_scope = crate::services::twin_events::root_identity_for_path(store.vault_path())
            .map_err(anyhow::Error::new)?;
        self.rollback_legacy_scoped(run_id, store, &root_scope)
    }

    #[cfg(test)]
    fn rollback_legacy_scoped(
        &self,
        run_id: &str,
        store: &mut KnowledgeStore,
        expected_root_scope: &crate::models::twin_event::ContentDigest,
    ) -> Result<()> {
        let manifest = self.load_manifest(run_id)?;
        if manifest.root_scope.as_ref() != Some(expected_root_scope) {
            anyhow::bail!("legacy or cross-vault migration manifests are audit-only");
        }
        let manifest_vault = std::fs::canonicalize(&manifest.vault_path)?;
        let current_vault = std::fs::canonicalize(store.vault_path())?;
        if manifest_vault != current_vault {
            anyhow::bail!("migration manifest vault does not match the active vault");
        }

        // Guard against ever reporting a no-op rollback as success. `touched_note_ids` is
        // populated in `apply()` for every note that went through the backup+rewrite path
        // (Hybrid/FullRewrite modes), so a manifest that recorded touched notes but has no
        // backups is either corrupt or — as was the case for the historical bug this fix
        // closes — was written by pre-fix code that never persisted `backup_files` at all.
        // Either way, there is nothing safe to restore; refuse rather than silently
        // "succeeding" at doing nothing.
        if manifest.backup_files.is_empty() && !manifest.touched_note_ids.is_empty() {
            anyhow::bail!(
                "Refusing to report rollback success for run '{}': manifest recorded {} rewritten note(s) but no backup files were captured. This run's backups cannot be restored automatically; check for an external vault backup.",
                run_id,
                manifest.touched_note_ids.len()
            );
        }

        let vault_path = PathBuf::from(&manifest.vault_path);
        let run_dir = self.runs_dir.join(run_id);
        let backups_dir = run_dir.join("backups");

        let mut failures = Vec::new();

        for relative_path in &manifest.backup_files {
            let backup_path = backups_dir.join(relative_path);
            let target_path = vault_path.join(relative_path);
            if !backup_path.exists() {
                failures.push(format!(
                    "backup file missing for '{}' (expected at '{}')",
                    relative_path,
                    backup_path.display()
                ));
                continue;
            }
            match std::fs::read(&backup_path) {
                Ok(bytes) => {
                    if let Err(error) =
                        store.restore_note_bytes_from_source(relative_path, &bytes, "migration")
                    {
                        failures.push(format!(
                            "failed to restore '{}' -> '{}': {}",
                            backup_path.display(),
                            target_path.display(),
                            error
                        ));
                    }
                }
                Err(error) => {
                    failures.push(format!(
                        "failed to read backup '{}': {}",
                        backup_path.display(),
                        error
                    ));
                }
            }
        }

        for relative_path in &manifest.created_files {
            if manifest.program_path.as_deref() == Some(relative_path.as_str()) {
                let Some(after_digest) = manifest.program_after_digest.clone() else {
                    failures.push(format!(
                        "program digest missing for created file '{}'",
                        relative_path
                    ));
                    continue;
                };
                if let Err(error) = store.delete_vault_file_target_only_expected(
                    relative_path,
                    "migration",
                    Some(crate::services::twin_events::BeforeImage::Sha256(
                        after_digest,
                    )),
                ) {
                    failures.push(format!(
                        "failed to remove created program file '{}': {}",
                        relative_path, error
                    ));
                }
                continue;
            }
            let target_path = vault_path.join(relative_path);
            if target_path.exists() {
                match store.find_note_by_relative_path(relative_path) {
                    Ok(Some(note)) => {
                        if let Err(error) = store.delete_note_from_source(&note.id, "migration") {
                            failures.push(format!(
                                "failed to remove created note '{}': {}",
                                relative_path, error
                            ));
                        }
                    }
                    Ok(None) => {
                        if let Err(error) =
                            store.delete_vault_file_target_only(relative_path, "migration")
                        {
                            failures.push(format!(
                                "failed to remove derived migration file '{}': {}",
                                relative_path, error
                            ));
                        }
                    }
                    Err(error) => failures.push(format!(
                        "failed to classify created file '{}': {}",
                        relative_path, error
                    )),
                }
            }
        }

        for note_id in &manifest.overlay_note_ids {
            if let Err(error) = store.delete_overlay_from_source(note_id, "migration") {
                failures.push(format!(
                    "failed to delete overlay for note '{}': {}",
                    note_id, error
                ));
            }
        }

        let mut updated_manifest = manifest;
        updated_manifest.status = if failures.is_empty() {
            "rolled_back".to_string()
        } else {
            "rolled_back_with_errors".to_string()
        };
        write_atomic(
            &run_dir.join("manifest.json"),
            serde_json::to_string_pretty(&updated_manifest)?.as_bytes(),
        )?;

        if !failures.is_empty() {
            anyhow::bail!(
                "Rollback for run '{}' completed with {} failure(s) and could not fully restore the vault: {}",
                run_id,
                failures.len(),
                failures.join("; ")
            );
        }

        Ok(())
    }

    pub fn backfill_legacy_grafyn_notes(&self, store: &mut KnowledgeStore) -> Result<Vec<String>> {
        let mut updated_ids = Vec::new();
        let notes = store.list_full_notes()?;
        for note in notes {
            // The note's original frontmatter failed to parse and is being preserved
            // verbatim (see `Note::frontmatter_raw_fallback`). Backfilling here would
            // explicitly set frontmatter-backed fields (aliases/schema_version/
            // migration_source/optimizer_managed/properties), which clears the
            // fallback and permanently destroys the unparsable original on write.
            // Skip it — it must not be rewritten until a human/editor fixes the YAML.
            if note.frontmatter_raw_fallback.is_some() {
                log::warn!(
                    "Skipping boot backfill for note '{}': original frontmatter is unparsable and preserved verbatim",
                    note.id
                );
                continue;
            }

            // Aliases deliberately play NO part in the skip decision. `note.aliases`
            // as loaded is *always* the union of the raw frontmatter aliases plus
            // freshly recomputed `alias_candidates(title, file_stem)` (see
            // `KnowledgeStore::read_note_file`), whether or not that union has ever
            // been persisted. Two consequences:
            //   1. Any alias this pass could persist is already present in
            //      `note.aliases` in memory, so `aliases: Some(note.aliases.clone())`
            //      below writes exactly what a reload would recompute anyway —
            //      alias state can never be the thing that makes a write necessary.
            //   2. The old skip requirement `!note.aliases.is_empty()` was therefore
            //      a broken proxy: for a note whose title matches its filename
            //      (single word, e.g. "Foo" / "foo.md") `alias_candidates` is empty
            //      forever, the requirement could never be met, and the note was
            //      rewritten (bumping `updated_at`) on every single boot.
            // The skip decision instead checks the fields this pass actually exists
            // to converge: schema version, migration provenance, and the
            // optimizer_managed flag (which can drift if a note becomes a topic hub
            // after it was first processed).
            let target_migration_source = note
                .migration_source
                .clone()
                .unwrap_or_else(|| MIGRATION_SOURCE_BACKFILL.to_string());
            let target_optimizer_managed = note.optimizer_managed || note.is_topic_hub();

            let already_processed = note.schema_version >= CURRENT_NOTE_SCHEMA_VERSION
                && note.migration_source.is_some()
                && note.optimizer_managed == target_optimizer_managed;

            if already_processed {
                continue;
            }

            let updated = store.update_note_preserving_timestamp(
                &note.id,
                NoteUpdate {
                    title: None,
                    content: None,
                    relative_path: Some(note.relative_path.clone()),
                    aliases: Some(note.aliases.clone()),
                    status: None,
                    tags: None,
                    schema_version: Some(CURRENT_NOTE_SCHEMA_VERSION),
                    migration_source: Some(target_migration_source),
                    optimizer_managed: Some(target_optimizer_managed),
                    properties: Some(note.properties.clone()),
                },
            )?;
            updated_ids.push(updated.id);
        }
        Ok(updated_ids)
    }

    fn latest_run_id(&self) -> Result<Option<String>> {
        let mut newest: Option<(std::time::SystemTime, String)> = None;
        for entry in std::fs::read_dir(&self.runs_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let modified = entry.metadata()?.modified()?;
            let id = entry.file_name().to_string_lossy().to_string();
            match &newest {
                Some((current, _)) if current >= &modified => {}
                _ => newest = Some((modified, id)),
            }
        }
        Ok(newest.map(|(_, id)| id))
    }

    fn load_preview(&self, preview_id: &str) -> Result<MarkdownMigrationPreview> {
        let path = self.runs_dir.join(preview_id).join("preview.json");
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read migration preview '{}'", path.display()))?;
        Ok(serde_json::from_str(&data)?)
    }

    fn load_manifest(&self, run_id: &str) -> Result<StoredManifest> {
        let path = self.runs_dir.join(run_id).join("manifest.json");
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read migration manifest '{}'", path.display()))?;
        Ok(serde_json::from_str(&data)?)
    }

    /// Copies `relative_path` into this run's `backups/` directory so `rollback()` can
    /// later restore it. Returns `Ok(true)` if a backup was actually captured (the source
    /// existed on disk), or `Ok(false)` if there was nothing to back up (e.g. the note
    /// file is missing). The caller is responsible for recording a successful backup into
    /// the in-memory manifest — this function intentionally does not touch manifest.json,
    /// since the manifest for a run is only written once, after `apply()` finishes, from
    /// the single in-memory `StoredManifest` it accumulates.
    #[cfg(test)]
    fn backup_note(&self, run_dir: &Path, vault_path: &str, relative_path: &str) -> Result<bool> {
        let source_path = Path::new(vault_path).join(relative_path);
        if !source_path.exists() {
            return Ok(false);
        }

        let backup_path = run_dir.join("backups").join(relative_path);
        if let Some(parent) = backup_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source_path, &backup_path).with_context(|| {
            format!(
                "Failed to backup note '{}' -> '{}'",
                source_path.display(),
                backup_path.display()
            )
        })?;

        Ok(true)
    }
}

fn require_current_preview_scope(
    preview: &MarkdownMigrationPreview,
    expected_root_scope: &crate::models::twin_event::ContentDigest,
    current_vault_path: &Path,
) -> Result<()> {
    if preview.root_scope.as_ref() != Some(expected_root_scope) {
        anyhow::bail!("legacy or cross-vault migration previews are audit-only");
    }
    if std::fs::canonicalize(&preview.vault_path)? != std::fs::canonicalize(current_vault_path)? {
        anyhow::bail!("migration preview vault does not match the active vault");
    }
    Ok(())
}

fn serialize_usize_as_u64<S>(value: &usize, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_u64(
        u64::try_from(*value).map_err(|_| serde::ser::Error::custom("index exceeds u64"))?,
    )
}

fn deserialize_u64_as_usize<'de, D>(deserializer: D) -> std::result::Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u64::deserialize(deserializer)?;
    usize::try_from(value).map_err(|_| serde::de::Error::custom("index exceeds this platform"))
}

#[cfg(test)]
fn expected_program_before_image(
    expected: &ExpectedProgramTarget,
) -> crate::services::twin_events::BeforeImage {
    match expected {
        ExpectedProgramTarget::Absent => crate::services::twin_events::BeforeImage::Absent,
        ExpectedProgramTarget::Present { digest } => {
            crate::services::twin_events::BeforeImage::Sha256(digest.clone())
        }
    }
}

fn build_reference_index(notes: &[Note]) -> HashMap<String, Vec<String>> {
    let mut index: HashMap<String, Vec<String>> = HashMap::new();
    for note in notes {
        index
            .entry(note.title.trim().to_lowercase())
            .or_default()
            .push(note.id.clone());
        for alias in &note.aliases {
            index
                .entry(alias.trim().to_lowercase())
                .or_default()
                .push(note.id.clone());
        }
        index
            .entry(note.relative_path.trim().to_lowercase())
            .or_default()
            .push(note.id.clone());
    }
    index
}

fn resolve_reference(
    reference: &str,
    resolution_index: &HashMap<String, Vec<String>>,
) -> Option<String> {
    resolution_index
        .get(&reference.trim().to_lowercase())
        .and_then(|matches| {
            if matches.len() == 1 {
                matches.first().cloned()
            } else {
                None
            }
        })
}

fn infer_topic_tags(note: &Note) -> Vec<String> {
    if !note.tags.is_empty() {
        return note.tags.clone();
    }

    let mut tokens = Vec::new();
    tokens.extend(
        note.title
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|token| token.len() >= 4)
            .map(|token| token.to_lowercase()),
    );
    tokens.extend(
        note.content
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|token| token.len() >= 5)
            .take(24)
            .map(|token| token.to_lowercase()),
    );

    let mut counts: HashMap<String, usize> = HashMap::new();
    for token in tokens {
        if is_stopword(&token) {
            continue;
        }
        *counts.entry(token).or_default() += 1;
    }

    let mut ranked = counts.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    ranked.into_iter().take(3).map(|(token, _)| token).collect()
}

fn infer_unlinked_note_mentions(
    note: &Note,
    resolution_index: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let lower_content = note.content.to_lowercase();
    let existing = note
        .parsed_links
        .iter()
        .filter_map(|link| resolve_reference(&link.target_title, resolution_index))
        .collect::<HashSet<_>>();

    let mut matches = Vec::new();
    for (key, ids) in resolution_index {
        if ids.len() != 1 || key == &note.title.to_lowercase() {
            continue;
        }
        let candidate_id = ids.first().cloned().unwrap_or_default();
        if existing.contains(&candidate_id) || candidate_id == note.id {
            continue;
        }
        if key.len() >= 6 && lower_content.contains(key) {
            matches.push(candidate_id);
        }
    }
    matches.sort();
    matches.dedup();
    matches.into_iter().take(3).collect()
}

fn find_ambiguous_references(
    note: &Note,
    resolution_index: &HashMap<String, Vec<String>>,
) -> HashMap<String, Vec<String>> {
    let mut collisions = HashMap::new();
    for parsed_link in &note.parsed_links {
        let key = parsed_link.target_title.trim().to_lowercase();
        let Some(matches) = resolution_index.get(&key) else {
            continue;
        };
        if matches.len() > 1 {
            collisions.insert(key, matches.clone());
        }
    }
    collisions
}

#[cfg(test)]
fn append_related_links(
    content: &str,
    inferred_link_ids: &[String],
    store: &KnowledgeStore,
) -> Result<(String, Vec<String>)> {
    if inferred_link_ids.is_empty() {
        return Ok((content.to_string(), Vec::new()));
    }

    let mut titles = Vec::new();
    let mut inserted_ids = Vec::new();
    for note_id in inferred_link_ids.iter().take(3) {
        let note = store.get_note(note_id)?;
        let marker = format!("[[{}]]", note.title);
        if content.contains(&marker) {
            continue;
        }
        titles.push(note.title);
        inserted_ids.push(note_id.clone());
    }

    if titles.is_empty() {
        return Ok((content.to_string(), Vec::new()));
    }

    let mut rewritten = content.trim_end().to_string();
    rewritten.push_str("\n\n## Related Notes\n");
    for title in &titles {
        rewritten.push_str(&format!("- [[{}]]\n", title));
    }
    Ok((rewritten, inserted_ids))
}

fn normalize_rewritten_content(title: &str, content: &str, ensure_h1: bool) -> String {
    if !ensure_h1 {
        return content.to_string();
    }
    if content
        .lines()
        .any(|line| line.trim() == format!("# {}", title))
    {
        return content.to_string();
    }
    format!("# {}\n\n{}", title, content.trim_start())
}

fn normalize_hub_folder(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim_matches('/')
        .trim()
        .to_string()
}

fn normalize_program_path(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim_matches('/')
        .trim()
        .to_string()
}

fn canonical_hub_folder(value: &str) -> Result<String> {
    let normalized = normalize_hub_folder(value);
    if normalized.is_empty() {
        anyhow::bail!("migration hub folder cannot be empty");
    }
    crate::services::twin_events::validate_target_key(
        crate::services::twin_events::TargetKind::Markdown,
        &format!("{normalized}/probe.md"),
    )
    .map_err(anyhow::Error::new)?;
    Ok(normalized)
}

fn canonical_program_path(value: &str) -> Result<String> {
    let mut normalized = normalize_program_path(value);
    if !normalized.to_ascii_lowercase().ends_with(".md") {
        normalized.push_str(".md");
    }
    crate::services::twin_events::validate_target_key(
        crate::services::twin_events::TargetKind::Markdown,
        &normalized,
    )
    .map_err(anyhow::Error::new)?;
    Ok(normalized)
}

fn migration_path_key(value: &str) -> String {
    let normalized = value.trim().replace('\\', "/");
    #[cfg(windows)]
    {
        normalized.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        normalized
    }
}

fn merge_unique_strings(existing: Vec<String>, additions: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for value in existing.into_iter().chain(additions) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let owned = trimmed.to_string();
        if seen.insert(owned.to_lowercase()) {
            values.push(owned);
        }
    }
    values
}

fn slugify(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else if character.is_whitespace() || character == '-' || character == '_' {
                '-'
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

fn display_topic_name(value: &str) -> String {
    value
        .split('-')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn humanize_title(relative_path: &str) -> String {
    Path::new(relative_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("note")
        .replace(['-', '_'], " ")
}

fn is_stopword(value: &str) -> bool {
    matches!(
        value,
        "that"
            | "this"
            | "with"
            | "from"
            | "about"
            | "their"
            | "there"
            | "because"
            | "while"
            | "where"
            | "which"
            | "would"
            | "could"
            | "should"
            | "note"
            | "notes"
            | "topic"
            | "ideas"
    )
}

fn default_program_file_contents(hub_folder: &str, program_path: &str) -> String {
    format!(
        "# Grafyn Vault Program\n\n- Hub folder: `{}`\n- Program path: `{}`\n- Preferred hub title prefix: `Hub:`\n- Auto-edit boundaries: `frontmatter, hubs, sidecar overlays`\n- Link aggressiveness: `precision_first`\n- Ignore folders: `_grafyn/tmp`\n",
        hub_folder, program_path
    )
}

fn canonical_migration_request(
    request: &MarkdownMigrationRequest,
    hub_folder: &str,
    program_path: &str,
) -> MarkdownMigrationRequest {
    MarkdownMigrationRequest {
        mode: request.mode.clone(),
        hub_folder: Some(hub_folder.to_string()),
        start_optimizer: Some(request.start_optimizer.unwrap_or(true)),
        enable_llm: Some(request.enable_llm.unwrap_or(false)),
        auto_insert_links: Some(request.auto_insert_links.unwrap_or(false)),
        program_path: Some(program_path.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::atomic_io::assert_no_tmp_siblings;
    use tempfile::tempdir;

    #[test]
    fn migration_program_put_is_target_only_and_crash_recoverable() {
        let vault_dir = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
            data_dir.path(),
        ));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                data_dir.path(),
                vault_dir.path(),
                events.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());
        let request = MarkdownMigrationRequest::default();
        let preview = service
            .preview(vault_dir.path().to_path_buf(), request.clone())
            .unwrap();
        let authority_before = coordinator.current_authority_token().unwrap();
        let expected_program =
            default_program_file_contents(&preview.hub_folder, &preview.program_path);
        let mut store = KnowledgeStore::with_event_recorder(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
            coordinator.clone(),
        );
        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterStage);

        let applied = service
            .apply(&preview.preview_id, request, &mut store)
            .expect("the exact staged program mutation must converge in-call");
        assert_eq!(applied.run_id, preview.preview_id);
        assert_eq!(applied.status, "applied");
        assert!(applied.created_hub_note_ids.is_empty());
        assert!(applied.touched_note_ids.is_empty());
        assert!(applied.overlay_note_ids.is_empty());
        assert_eq!(
            std::fs::read_to_string(vault_dir.path().join("_grafyn/program.md")).unwrap(),
            expected_program
        );
        let authority_after = coordinator.current_authority_token().unwrap();
        assert_eq!(authority_after.root_scope, authority_before.root_scope);
        assert_eq!(
            authority_after.lease_epoch_uuid,
            authority_before.lease_epoch_uuid
        );
        assert_eq!(
            authority_after.authority_generation,
            authority_before.authority_generation + 1
        );
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        assert_eq!(coordinator.recover_pending().unwrap(), 0);
        assert_eq!(
            coordinator.current_authority_token().unwrap(),
            authority_after
        );
        assert!(events.ordered_events().unwrap().is_empty());
    }

    #[test]
    fn preview_writes_are_atomic_with_no_tmp_litter() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        std::fs::write(
            vault_dir.path().join("sample.md"),
            "---\ntitle: Sample\n---\n\nSample content.",
        )
        .expect("seed vault note");

        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());
        let preview = service
            .preview(
                vault_dir.path().to_path_buf(),
                MarkdownMigrationRequest::default(),
            )
            .expect("preview should succeed");

        let run_dir = service.runs_dir.join(&preview.preview_id);
        let persisted = std::fs::read_to_string(run_dir.join("preview.json"))
            .expect("preview.json should exist");
        assert!(persisted.contains(&preview.preview_id));
        assert_eq!(
            preview.root_scope,
            Some(crate::services::twin_events::root_identity_for_path(vault_dir.path()).unwrap())
        );
        assert_eq!(
            preview.expected_program_target,
            Some(ExpectedProgramTarget::Absent)
        );
        assert_eq!(
            preview.program_after_digest,
            Some(crate::services::twin_events::digest_bytes(
                default_program_file_contents(&preview.hub_folder, &preview.program_path)
                    .as_bytes()
            ))
        );
        let persisted_json: Value = serde_json::from_str(&persisted).unwrap();
        for key in [
            "root_scope",
            "expected_program_target",
            "program_after_digest",
        ] {
            assert!(persisted_json.get(key).is_some(), "missing {key}");
        }
        assert_no_tmp_siblings(&run_dir);
    }

    #[test]
    fn preview_resolves_markdown_links_from_the_same_fresh_snapshot() {
        let vault_dir = tempdir().unwrap();
        let data_dir = tempdir().unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
            data_dir.path(),
        ));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                data_dir.path(),
                vault_dir.path(),
                events,
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let authority = coordinator.current_authority_token().unwrap();
        let derived = crate::services::vault_namespace::scoped_data_path(
            data_dir.path(),
            &authority.root_scope,
        );
        let mut store = KnowledgeStore::with_event_recorder(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
            coordinator,
        );
        store
            .adopt_coordinated_vault_path(vault_dir.path().to_path_buf(), &derived)
            .unwrap();
        std::fs::write(
            vault_dir.path().join("source.md"),
            "# Source\n\n[Target](target.md)",
        )
        .unwrap();
        std::fs::write(vault_dir.path().join("target.md"), "# Target").unwrap();
        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());

        let preview = service
            .preview_scoped_with_schema(
                &mut store,
                authority,
                MarkdownMigrationRequest::default(),
                MIGRATION_PREVIEW_SCHEMA_VERSION,
            )
            .unwrap();

        assert_eq!(preview.summary.total_scanned_notes, 2);
        assert_eq!(preview.summary.markdown_links_resolved, 1);
    }

    #[test]
    fn apply_rejects_program_changed_after_preview_before_any_migration_write() {
        let root = tempdir().unwrap();
        let vault = root.path().join("vault");
        let data = root.path().join("data");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&data).unwrap();
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
        let service = MarkdownMigrationService::new(data.clone());
        let preview = service
            .preview(vault.clone(), MarkdownMigrationRequest::default())
            .unwrap();
        std::fs::create_dir(vault.join("_grafyn")).unwrap();
        std::fs::write(vault.join("_grafyn/program.md"), b"user edit").unwrap();
        let mut store =
            KnowledgeStore::with_event_recorder(vault.clone(), data.clone(), coordinator.clone());

        let error = service
            .apply(
                &preview.preview_id,
                MarkdownMigrationRequest::default(),
                &mut store,
            )
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("conditional mutation target changed"));
        assert_eq!(
            std::fs::read(vault.join("_grafyn/program.md")).unwrap(),
            b"user edit"
        );
        assert!(!service
            .runs_dir
            .join(&preview.preview_id)
            .join("manifest.json")
            .exists());
        assert_eq!(coordinator.pending_count().unwrap(), 0);
    }

    #[test]
    fn rollback_preserves_program_edited_after_apply() {
        let root = tempdir().unwrap();
        let vault = root.path().join("vault");
        let data = root.path().join("data");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&data).unwrap();
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
        let service = MarkdownMigrationService::new(data.clone());
        let preview = service
            .preview(vault.clone(), MarkdownMigrationRequest::default())
            .unwrap();
        let mut store =
            KnowledgeStore::with_event_recorder(vault.clone(), data, coordinator.clone());
        let applied = service
            .apply(
                &preview.preview_id,
                MarkdownMigrationRequest::default(),
                &mut store,
            )
            .unwrap();
        std::fs::write(vault.join("_grafyn/program.md"), b"user edit after apply").unwrap();

        assert!(service.rollback(&applied.run_id, &mut store).is_err());
        assert_eq!(
            std::fs::read(vault.join("_grafyn/program.md")).unwrap(),
            b"user edit after apply"
        );
        assert_eq!(coordinator.pending_count().unwrap(), 0);
    }

    #[test]
    fn preview_cannot_apply_to_another_vault_scope() {
        let root = tempdir().unwrap();
        let vault_a = root.path().join("vault-a");
        let vault_b = root.path().join("vault-b");
        let data = root.path().join("data");
        std::fs::create_dir(&vault_a).unwrap();
        std::fs::create_dir(&vault_b).unwrap();
        std::fs::create_dir(&data).unwrap();
        let service = MarkdownMigrationService::new(data.clone());
        let preview = service
            .preview(vault_a, MarkdownMigrationRequest::default())
            .unwrap();
        let mut store = KnowledgeStore::new(vault_b.clone(), data);

        assert!(service
            .apply(
                &preview.preview_id,
                MarkdownMigrationRequest::default(),
                &mut store,
            )
            .is_err());
        assert!(!vault_b.join("_grafyn/program.md").exists());
    }

    fn seed_two_notes(store: &mut KnowledgeStore) -> (Note, Note) {
        let note_a = store
            .create_note(NoteCreate {
                title: "Rollback Note A".to_string(),
                content: "Original body A.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: Default::default(),
                tags: Vec::new(),
                schema_version: 1,
                migration_source: None,
                optimizer_managed: false,
                properties: HashMap::new(),
            })
            .expect("note a should be created");
        let note_b = store
            .create_note(NoteCreate {
                title: "Rollback Note B".to_string(),
                content: "Original body B.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: Default::default(),
                tags: Vec::new(),
                schema_version: 1,
                migration_source: None,
                optimizer_managed: false,
                properties: HashMap::new(),
            })
            .expect("note b should be created");
        (note_a, note_b)
    }

    #[test]
    fn rollback_restores_backed_up_note_contents() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let (note_a, note_b) = seed_two_notes(&mut store);

        let path_a = vault_dir.path().join(&note_a.relative_path);
        let path_b = vault_dir.path().join(&note_b.relative_path);
        let original_a = std::fs::read_to_string(&path_a).expect("note a file should exist");
        let original_b = std::fs::read_to_string(&path_b).expect("note b file should exist");

        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());
        let preview = service
            .preview(
                vault_dir.path().to_path_buf(),
                MarkdownMigrationRequest {
                    mode: MarkdownMigrationMode::FullRewrite,
                    auto_insert_links: Some(true),
                    ..Default::default()
                },
            )
            .expect("preview should succeed");

        let apply_result = service
            .apply(
                &preview.preview_id,
                MarkdownMigrationRequest {
                    mode: MarkdownMigrationMode::FullRewrite,
                    auto_insert_links: Some(true),
                    ..Default::default()
                },
                &mut store,
            )
            .expect("apply should succeed");
        assert_eq!(apply_result.touched_note_ids.len(), 2);

        let rewritten_a = std::fs::read_to_string(&path_a).expect("note a should still exist");
        let rewritten_b = std::fs::read_to_string(&path_b).expect("note b should still exist");
        assert_ne!(
            rewritten_a, original_a,
            "apply should have rewritten note a"
        );
        assert_ne!(
            rewritten_b, original_b,
            "apply should have rewritten note b"
        );

        service
            .rollback(&apply_result.run_id, &mut store)
            .expect("rollback should succeed and restore backups");

        let restored_a =
            std::fs::read_to_string(&path_a).expect("note a should exist post-rollback");
        let restored_b =
            std::fs::read_to_string(&path_b).expect("note b should exist post-rollback");
        assert_eq!(
            restored_a, original_a,
            "note a should be byte-equal to its pre-migration original after rollback"
        );
        assert_eq!(
            restored_b, original_b,
            "note b should be byte-equal to its pre-migration original after rollback"
        );
    }

    #[test]
    fn coordinated_apply_and_rollback_emit_only_migration_note_changes() {
        let root = tempdir().unwrap();
        let vault = root.path().join("vault");
        let data = root.path().join("data");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&data).unwrap();
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                events.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store =
            KnowledgeStore::with_event_recorder(vault.clone(), data.clone(), coordinator);
        seed_two_notes(&mut store);
        let baseline = events.ordered_events().unwrap().len();
        let service = MarkdownMigrationService::new(data);
        let request = MarkdownMigrationRequest {
            mode: MarkdownMigrationMode::FullRewrite,
            auto_insert_links: Some(true),
            ..Default::default()
        };
        let preview = service.preview(vault, request.clone()).unwrap();
        let applied = service
            .apply(&preview.preview_id, request, &mut store)
            .unwrap();
        let after_apply = events.ordered_events().unwrap();
        assert!(after_apply.len() > baseline);
        assert!(after_apply[baseline..]
            .iter()
            .all(|event| event.context.source_channel.as_str() == "migration"));

        service.rollback(&applied.run_id, &mut store).unwrap();
        let after_rollback = events.ordered_events().unwrap();
        assert!(after_rollback.len() > after_apply.len());
        assert!(after_rollback[after_apply.len()..].iter().all(|event| event
            .context
            .source_channel
            .as_str()
            == "migration"));
    }

    #[test]
    fn rollback_is_idempotent() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let (note_a, note_b) = seed_two_notes(&mut store);
        let path_a = vault_dir.path().join(&note_a.relative_path);
        let path_b = vault_dir.path().join(&note_b.relative_path);

        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());
        let request = MarkdownMigrationRequest {
            mode: MarkdownMigrationMode::FullRewrite,
            ..Default::default()
        };
        let preview = service
            .preview(vault_dir.path().to_path_buf(), request.clone())
            .expect("preview should succeed");
        let apply_result = service
            .apply(&preview.preview_id, request, &mut store)
            .expect("apply should succeed");

        service
            .rollback(&apply_result.run_id, &mut store)
            .expect("first rollback should succeed");
        let first_a = std::fs::read_to_string(&path_a).expect("note a should exist");
        let first_b = std::fs::read_to_string(&path_b).expect("note b should exist");

        service
            .rollback(&apply_result.run_id, &mut store)
            .expect("second rollback should also succeed (idempotent)");
        let second_a = std::fs::read_to_string(&path_a).expect("note a should exist");
        let second_b = std::fs::read_to_string(&path_b).expect("note b should exist");

        assert_eq!(
            first_a, second_a,
            "repeated rollback must not change note a further"
        );
        assert_eq!(
            first_b, second_b,
            "repeated rollback must not change note b further"
        );
    }

    #[test]
    fn apply_skips_fallback_notes_and_reports_them() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");

        // Malformed frontmatter: tab used as YAML block-sequence indentation, which
        // yaml-rust2 rejects — the note carries a raw-frontmatter fallback on read.
        let malformed_path = vault_dir.path().join("broken.md");
        std::fs::write(
            &malformed_path,
            "---\ntitle: Broken Note\ntags:\n\t- alpha\n---\n\nBroken body.",
        )
        .expect("malformed note file should be written");

        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );
        let healthy = store
            .create_note(NoteCreate {
                title: "Healthy Note".to_string(),
                content: "Healthy body.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: Default::default(),
                tags: Vec::new(),
                schema_version: 1,
                migration_source: None,
                optimizer_managed: false,
                properties: HashMap::new(),
            })
            .expect("healthy note should be created");
        let broken_id = store
            .find_note_by_relative_path("broken.md")
            .expect("lookup should not error")
            .expect("malformed note should be readable")
            .id;

        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());
        let request = MarkdownMigrationRequest {
            mode: MarkdownMigrationMode::FullRewrite,
            ..Default::default()
        };
        let preview = service
            .preview(vault_dir.path().to_path_buf(), request.clone())
            .expect("preview should succeed");
        let apply_result = service
            .apply(&preview.preview_id, request, &mut store)
            .expect("apply should succeed");

        assert!(
            apply_result.touched_note_ids.contains(&healthy.id),
            "healthy note should be rewritten by the migration"
        );
        assert!(
            !apply_result.touched_note_ids.contains(&broken_id),
            "fallback note must not be counted as touched"
        );
        assert_eq!(
            apply_result.skipped_fallback_note_ids,
            vec![broken_id.clone()],
            "fallback note should be reported as skipped"
        );

        let persisted =
            std::fs::read_to_string(&malformed_path).expect("malformed note should still exist");
        assert!(
            persisted.contains("title: Broken Note\ntags:\n\t- alpha"),
            "original malformed frontmatter must be preserved byte-for-byte:\n{persisted}"
        );
        assert!(
            persisted.contains("Broken body."),
            "original content must be untouched:\n{persisted}"
        );
    }

    #[test]
    fn rollback_errors_when_backup_files_empty_but_notes_were_touched() {
        // Simulates a manifest written by the pre-fix code path: touched_note_ids is
        // populated (notes were rewritten) but backup_files was never populated, since
        // that was exactly the silent-no-op bug this fix closes. Rollback must refuse
        // to report success for such a manifest rather than silently doing nothing.
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());

        let run_id = "legacy-run".to_string();
        let run_dir = service.runs_dir.join(&run_id);
        std::fs::create_dir_all(run_dir.join("backups")).expect("run dir should be created");

        let legacy_manifest = StoredManifest {
            run_id: run_id.clone(),
            preview_id: run_id.clone(),
            vault_path: vault_dir.path().to_string_lossy().to_string(),
            mode: MarkdownMigrationMode::FullRewrite,
            created_at: Utc::now(),
            applied_at: Some(Utc::now()),
            status: "applied".to_string(),
            created_files: Vec::new(),
            backup_files: Vec::new(),
            overlay_note_ids: Vec::new(),
            touched_note_ids: vec!["note-1".to_string()],
            created_hub_note_ids: Vec::new(),
            skipped_fallback_note_ids: Vec::new(),
            ..Default::default()
        };
        write_atomic(
            &run_dir.join("manifest.json"),
            serde_json::to_string_pretty(&legacy_manifest)
                .expect("manifest should serialize")
                .as_bytes(),
        )
        .expect("legacy manifest should be written");

        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let result = service.rollback(&run_id, &mut store);
        assert!(
            result.is_err(),
            "rollback must error rather than report success for an empty-backup, touched-notes manifest"
        );
    }

    #[test]
    fn rollback_reports_failure_when_backup_file_missing_from_disk() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let (note_a, _note_b) = seed_two_notes(&mut store);

        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());
        let request = MarkdownMigrationRequest {
            mode: MarkdownMigrationMode::FullRewrite,
            ..Default::default()
        };
        let preview = service
            .preview(vault_dir.path().to_path_buf(), request.clone())
            .expect("preview should succeed");
        let apply_result = service
            .apply(&preview.preview_id, request, &mut store)
            .expect("apply should succeed");

        // Corrupt the run dir by deleting one of the captured backup files.
        let run_dir = service.runs_dir.join(&apply_result.run_id);
        let corrupted_backup = run_dir.join("backups").join(&note_a.relative_path);
        std::fs::remove_file(&corrupted_backup).expect("backup file should exist to delete");

        let result = service.rollback(&apply_result.run_id, &mut store);
        assert!(
            result.is_err(),
            "rollback must surface an error (not panic) when a backup file is missing"
        );
    }

    /// Reads a note file's mtime + full byte content, for before/after comparisons.
    fn snapshot(path: &Path) -> (std::time::SystemTime, Vec<u8>) {
        let metadata = std::fs::metadata(path).expect("note file should exist");
        let modified = metadata.modified().expect("mtime should be readable");
        let bytes = std::fs::read(path).expect("note file should be readable");
        (modified, bytes)
    }

    #[test]
    fn backfill_is_idempotent_for_single_word_title_notes() {
        // A single-word title matching its filename (e.g. "Foo" / "foo.md") makes
        // `alias_candidates` return zero candidates forever (see
        // `knowledge_store::alias_candidates`). The pre-fix skip condition required
        // `!note.aliases.is_empty()`, which such a note can never satisfy, so it was
        // rewritten (and `updated_at` bumped) on every single boot.
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(
            data_dir.path(),
        ));
        events.initialize().unwrap();
        let coordinator = std::sync::Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                data_dir.path(),
                vault_dir.path(),
                events.clone(),
                std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let mut store = KnowledgeStore::with_event_recorder(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
            coordinator,
        );

        let note = store
            .create_note(NoteCreate {
                title: "Foo".to_string(),
                content: "Single-word-title body.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: Default::default(),
                tags: Vec::new(),
                schema_version: 1,
                migration_source: None,
                optimizer_managed: false,
                properties: HashMap::new(),
            })
            .expect("note should be created");
        let note_path = vault_dir.path().join(&note.relative_path);
        let baseline = events.ordered_events().unwrap().len();

        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());

        // First run: migration_source is None, so this note is legitimately touched
        // once to record schema/provenance bookkeeping.
        let first_pass = service
            .backfill_legacy_grafyn_notes(&mut store)
            .expect("first backfill pass should succeed");
        assert!(
            first_pass.contains(&note.id),
            "first pass should backfill provenance for a never-migrated note"
        );
        let after_first_events = events.ordered_events().unwrap();
        assert_eq!(after_first_events.len(), baseline + 1);
        assert_eq!(
            after_first_events[baseline].context.source_channel.as_str(),
            "migration"
        );

        // Give the filesystem a chance to distinguish mtimes if a second write occurs.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let after_first = snapshot(&note_path);

        let second_pass = service
            .backfill_legacy_grafyn_notes(&mut store)
            .expect("second backfill pass should succeed");
        assert!(
            second_pass.is_empty(),
            "second pass must not touch an already-backfilled note with zero alias candidates, got: {second_pass:?}"
        );
        assert_eq!(events.ordered_events().unwrap().len(), baseline + 1);

        let after_second = snapshot(&note_path);
        assert_eq!(
            after_first.1, after_second.1,
            "note content must be byte-identical after a no-op second pass"
        );
        assert_eq!(
            after_first.0, after_second.0,
            "note mtime (and therefore frontmatter `updated_at`) must be unchanged by a no-op second pass"
        );
    }

    #[test]
    fn backfill_does_not_rewrite_an_already_processed_note_on_the_first_pass() {
        // If a single-word-title note is *already* fully processed (schema current,
        // migration_source recorded, optimizer flag correct) there is genuinely
        // nothing left to backfill — zero candidates, zero diffs — so even the very
        // first pass over it must be a no-op write.
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let note = store
            .create_note(NoteCreate {
                title: "Foo".to_string(),
                content: "Already processed body.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: Default::default(),
                tags: Vec::new(),
                schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: Some("markdown_migration".to_string()),
                optimizer_managed: false,
                properties: HashMap::new(),
            })
            .expect("note should be created");
        let note_path = vault_dir.path().join(&note.relative_path);
        let before = snapshot(&note_path);

        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());
        std::thread::sleep(std::time::Duration::from_millis(20));
        let updated_ids = service
            .backfill_legacy_grafyn_notes(&mut store)
            .expect("backfill should succeed");

        assert!(
            updated_ids.is_empty(),
            "an already-processed note must not be touched, got: {updated_ids:?}"
        );
        let after = snapshot(&note_path);
        assert_eq!(before.1, after.1, "content must be untouched");
        assert_eq!(before.0, after.0, "mtime must be untouched");
    }

    #[test]
    fn backfill_updates_stale_optimizer_managed_flag_then_settles() {
        // A note that is otherwise fully processed (schema current, migration_source
        // recorded) but whose persisted `optimizer_managed: false` is stale relative
        // to `is_topic_hub()` (e.g. it later gained the `hub` tag) must be updated by
        // backfill — with `updated_at` preserved — and a second run must do nothing.
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let note = store
            .create_note(NoteCreate {
                title: "Foo".to_string(),
                content: "Hub-tagged body.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: Default::default(),
                tags: vec!["hub".to_string()],
                schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: Some("markdown_migration".to_string()),
                optimizer_managed: false,
                properties: HashMap::new(),
            })
            .expect("note should be created");
        assert!(
            note.is_topic_hub() && !note.optimizer_managed,
            "precondition: note must be a hub with a stale optimizer_managed flag"
        );
        let original_updated_at = note.updated_at;
        let note_path = vault_dir.path().join(&note.relative_path);

        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());
        let first_pass = service
            .backfill_legacy_grafyn_notes(&mut store)
            .expect("first backfill pass should succeed");
        assert!(
            first_pass.contains(&note.id),
            "backfill must catch optimizer_managed drift on an otherwise-processed note, got: {first_pass:?}"
        );

        let reloaded = store.get_note(&note.id).expect("note should reload");
        assert!(
            reloaded.optimizer_managed,
            "optimizer_managed must be corrected to match is_topic_hub()"
        );
        assert_eq!(
            reloaded.updated_at, original_updated_at,
            "administrative backfill write must preserve updated_at"
        );

        std::thread::sleep(std::time::Duration::from_millis(20));
        let after_first = snapshot(&note_path);

        let second_pass = service
            .backfill_legacy_grafyn_notes(&mut store)
            .expect("second backfill pass should succeed");
        assert!(
            second_pass.is_empty(),
            "second pass must be a no-op once the drift is fixed, got: {second_pass:?}"
        );
        let after_second = snapshot(&note_path);
        assert_eq!(
            after_first, after_second,
            "second pass must not touch the file"
        );
    }

    #[test]
    fn backfill_does_zero_writes_on_a_second_pass_over_a_mixed_vault() {
        // Guards the O(N^2) boot cost: once a vault has been backfilled, a second
        // pass (e.g. the next app launch) must touch nothing at all, whether note
        // titles are single-word (zero alias candidates) or multi-word (non-empty
        // alias candidates).
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let single_word = store
            .create_note(NoteCreate {
                title: "Foo".to_string(),
                content: "Single word title.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: Default::default(),
                tags: Vec::new(),
                schema_version: 1,
                migration_source: None,
                optimizer_managed: false,
                properties: HashMap::new(),
            })
            .expect("single-word note should be created");
        let multi_word = store
            .create_note(NoteCreate {
                title: "Multi Word Title".to_string(),
                content: "Multi word title body.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: Default::default(),
                tags: Vec::new(),
                schema_version: 1,
                migration_source: None,
                optimizer_managed: false,
                properties: HashMap::new(),
            })
            .expect("multi-word note should be created");

        let single_path = vault_dir.path().join(&single_word.relative_path);
        let multi_path = vault_dir.path().join(&multi_word.relative_path);

        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());
        let first_pass = service
            .backfill_legacy_grafyn_notes(&mut store)
            .expect("first backfill pass should succeed");
        assert_eq!(
            first_pass.len(),
            2,
            "first pass should backfill both never-migrated notes, got: {first_pass:?}"
        );

        std::thread::sleep(std::time::Duration::from_millis(20));
        let single_before = snapshot(&single_path);
        let multi_before = snapshot(&multi_path);

        let second_pass = service
            .backfill_legacy_grafyn_notes(&mut store)
            .expect("second backfill pass should succeed");
        assert!(
            second_pass.is_empty(),
            "second pass over an already-backfilled vault must touch zero notes, got: {second_pass:?}"
        );

        let single_after = snapshot(&single_path);
        let multi_after = snapshot(&multi_path);
        assert_eq!(
            single_before, single_after,
            "single-word note must be untouched"
        );
        assert_eq!(
            multi_before, multi_after,
            "multi-word note must be untouched"
        );
    }

    #[test]
    fn backfill_skips_notes_with_unparsable_frontmatter() {
        // Task 1.6 regression guard: notes carrying a raw-frontmatter fallback
        // (`Note::frontmatter_raw_fallback`) must never be rewritten by boot
        // backfill — doing so would silently clear the fallback and destroy the
        // original unparsable YAML.
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");

        let malformed_path = vault_dir.path().join("broken.md");
        std::fs::write(
            &malformed_path,
            "---\ntitle: Broken Note\ntags:\n\t- alpha\n---\n\nBroken body.",
        )
        .expect("malformed note file should be written");
        let before = snapshot(&malformed_path);

        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );
        let broken_id = store
            .find_note_by_relative_path("broken.md")
            .expect("lookup should not error")
            .expect("malformed note should be readable")
            .id;

        let service = MarkdownMigrationService::new(data_dir.path().to_path_buf());
        std::thread::sleep(std::time::Duration::from_millis(20));
        let updated_ids = service
            .backfill_legacy_grafyn_notes(&mut store)
            .expect("backfill should succeed");

        assert!(
            !updated_ids.contains(&broken_id),
            "fallback note must not be counted as backfilled"
        );
        let after = snapshot(&malformed_path);
        assert_eq!(
            before, after,
            "fallback note's file must remain byte-identical after backfill"
        );
    }
}
