use crate::models::note::{
    Note, NoteCreate, NoteFrontmatter, NoteMeta, NoteUpdate, ParsedLink, RelationType,
    CURRENT_NOTE_SCHEMA_VERSION,
};
use crate::services::atomic_io::write_atomic;
use anyhow::{Context, Result};
use chrono::Utc;
use gray_matter::{engine::YAML, Matter};
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use walkdir::WalkDir;

mod exact_targets;
mod note_helpers;

use note_helpers::*;

lazy_static! {
    /// Regex for extracting wikilinks: [[Target]] or [[Target|Display]]
    static ref WIKILINK_REGEX: Regex = Regex::new(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]").unwrap();
    /// Regex for extracting typed wikilinks: [[Target]] (relation_type)
    static ref TYPED_WIKILINK_REGEX: Regex =
        Regex::new(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]\s*(?:\((\w+)\))?").unwrap();
    /// Regex for extracting relative markdown links like [text](../note.md)
    static ref MARKDOWN_LINK_REGEX: Regex =
        Regex::new(r"\[[^\]]+\]\(([^)]+?\.md(?:#[^)]+)?)\)").unwrap();
    /// First markdown H1 heading.
    static ref H1_REGEX: Regex = Regex::new(r"(?m)^\#\s+(.+?)\s*$").unwrap();
    /// Inline hashtag extraction for topic seeding.
    static ref HASHTAG_REGEX: Regex = Regex::new(r"(?m)(?:^|[^\w/])#([A-Za-z][\w/-]+)").unwrap();
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct OverlayNoteData {
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    schema_version: Option<u32>,
    #[serde(default)]
    migration_source: Option<String>,
    #[serde(default)]
    optimizer_managed: Option<bool>,
    #[serde(default)]
    properties: HashMap<String, Value>,
    #[serde(default, rename = "_grafyn_optimizer_source_v1")]
    optimizer_source: Option<OptimizerOverlaySourceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OptimizerOverlaySourceV1 {
    relative_path: String,
    sha256: crate::models::twin_event::ContentDigest,
}

#[derive(Deserialize)]
struct NoteSyncFrontmatter {
    #[serde(default)]
    grafyn_sync: Option<String>,
    #[serde(default)]
    note_id: Option<String>,
}

pub(crate) fn note_allows_sync(markdown: &str) -> bool {
    let starts_with_frontmatter = markdown
        .lines()
        .next()
        .is_some_and(|line| line.trim_end() == "---");
    let parsed = Matter::<YAML>::new().parse(markdown);
    if parsed.matter.trim().is_empty() {
        return !starts_with_frontmatter;
    }
    let Some(data) = parsed.data else {
        return false;
    };
    let Ok(frontmatter) = data.deserialize::<NoteSyncFrontmatter>() else {
        return false;
    };
    matches!(frontmatter.grafyn_sync.as_deref(), None | Some("inherit"))
}

pub(crate) fn note_identity_from_markdown(markdown: &str) -> Option<String> {
    let parsed = Matter::<YAML>::new().parse(markdown);
    parsed
        .data?
        .deserialize::<NoteSyncFrontmatter>()
        .ok()?
        .note_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(crate) fn default_note_identity_for_relative_path(relative_path: &str) -> String {
    slugify(relative_path)
}

/// Service for managing markdown notes with YAML frontmatter and migration overlays.
#[derive(Clone)]
pub struct KnowledgeStore {
    vault_path: PathBuf,
    overlay_notes_dir: PathBuf,
    vault_root: Option<Arc<crate::services::twin_events::AnchoredRoot>>,
    overlay_root: Option<Arc<crate::services::twin_events::AnchoredRoot>>,
    /// In-memory cache of note metadata, kept in sync with disk.
    meta_cache: Vec<NoteMeta>,
    path_index: HashMap<String, PathBuf>,
    title_index: HashMap<String, String>,
    alias_index: HashMap<String, String>,
    relative_path_index: HashMap<String, String>,
    event_recorder: Arc<dyn crate::services::twin_events::EventRecorder>,
}

pub(crate) struct OptimizerNoteSnapshot {
    pub note: Note,
    pub markdown_precondition: OptimizerMarkdownPrecondition,
    pub markdown_raw_bytes: Vec<u8>,
    pub overlay_value: Option<Value>,
    pub overlay_digest: Option<crate::models::twin_event::ContentDigest>,
    pub overlay_raw_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub(crate) struct KnowledgeAuthorityAdvancedOutcome {
    pub commit: crate::services::twin_events::MutationCommit,
    pub target_aborted: bool,
    pub note_ids: Vec<String>,
}

pub(crate) enum KnowledgeNotePersistenceAttempt {
    Committed((Note, crate::services::twin_events::MutationCommit)),
    ProvenPrecommit(anyhow::Error),
    AuthorityAdvanced(anyhow::Error),
    Uncertain(anyhow::Error),
}

impl KnowledgeNotePersistenceAttempt {
    pub(crate) fn is_proven_precommit(&self) -> bool {
        matches!(self, Self::ProvenPrecommit(_))
    }

    pub(crate) fn into_result(
        self,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        match self {
            Self::Committed(value) => Ok(value),
            Self::ProvenPrecommit(error)
            | Self::AuthorityAdvanced(error)
            | Self::Uncertain(error) => Err(error),
        }
    }
}

fn classify_generated_image_persistence(
    result: anyhow::Result<(Note, crate::services::twin_events::MutationCommit)>,
) -> KnowledgeNotePersistenceAttempt {
    match result {
        Ok(value) => KnowledgeNotePersistenceAttempt::Committed(value),
        Err(error) if knowledge_authority_advanced_outcome(&error).is_some() => {
            KnowledgeNotePersistenceAttempt::AuthorityAdvanced(error)
        }
        Err(error) if is_proven_precommit_knowledge_error(&error) => {
            KnowledgeNotePersistenceAttempt::ProvenPrecommit(error)
        }
        Err(error) => KnowledgeNotePersistenceAttempt::Uncertain(error),
    }
}

fn is_proven_precommit_knowledge_error(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<crate::services::twin_events::MutationError>(),
        Some(crate::services::twin_events::MutationError::Invalid(_))
            | Some(
                crate::services::twin_events::MutationError::AbortedPrecondition {
                    authority_advanced: false,
                    ..
                }
            )
    )
}

#[cfg(test)]
mod generated_image_persistence_attempt_tests {
    use super::*;

    #[test]
    fn unknown_persistence_error_is_not_classified_as_proven_precommit() {
        let attempt = classify_generated_image_persistence(Err(anyhow::anyhow!(
            "plain unknown persistence failure"
        )));

        assert!(!attempt.is_proven_precommit());
        assert!(attempt.into_result().is_err());
    }

    #[test]
    fn typed_invalid_persistence_error_is_classified_as_proven_precommit() {
        let attempt = classify_generated_image_persistence(Err(anyhow::Error::new(
            crate::services::twin_events::MutationError::Invalid(
                "planner rejected before authority".into(),
            ),
        )));

        assert!(attempt.is_proven_precommit());
    }
}

#[derive(Debug)]
struct KnowledgeAuthorityAdvancedError {
    outcome: KnowledgeAuthorityAdvancedOutcome,
    source: crate::services::twin_events::MutationError,
}

impl std::fmt::Display for KnowledgeAuthorityAdvancedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(formatter)
    }
}

impl std::error::Error for KnowledgeAuthorityAdvancedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

fn preserve_knowledge_authority_error(
    error: crate::services::twin_events::MutationError,
    note_ids: Vec<String>,
) -> anyhow::Error {
    preserve_knowledge_authority_error_with_events(error, note_ids, Vec::new())
}

fn preserve_knowledge_authority_error_with_events(
    error: crate::services::twin_events::MutationError,
    note_ids: Vec<String>,
    prepared_events: Vec<crate::models::twin_event::TwinEvent>,
) -> anyhow::Error {
    let Some(mut commit) = error.authority_advanced_commit() else {
        return anyhow::Error::new(error);
    };
    commit.events = prepared_events;
    anyhow::Error::new(KnowledgeAuthorityAdvancedError {
        outcome: KnowledgeAuthorityAdvancedOutcome {
            commit,
            target_aborted: error.authority_advanced_target_aborted(),
            note_ids,
        },
        source: error,
    })
}

fn preserve_committed_knowledge_result_error(
    commit: crate::services::twin_events::MutationCommit,
    note_ids: Vec<String>,
    error: anyhow::Error,
) -> anyhow::Error {
    anyhow::Error::new(KnowledgeAuthorityAdvancedError {
        outcome: KnowledgeAuthorityAdvancedOutcome {
            commit,
            target_aborted: false,
            note_ids,
        },
        source: crate::services::twin_events::MutationError::RecoveryConflict(format!(
            "committed knowledge result could not be loaded: {error}"
        )),
    })
}

pub(crate) fn knowledge_authority_advanced_outcome(
    error: &anyhow::Error,
) -> Option<KnowledgeAuthorityAdvancedOutcome> {
    if let Some(error) = error.downcast_ref::<KnowledgeAuthorityAdvancedError>() {
        return Some(error.outcome.clone());
    }
    let error = error.downcast_ref::<crate::services::twin_events::MutationError>()?;
    Some(KnowledgeAuthorityAdvancedOutcome {
        commit: error.authority_advanced_commit()?,
        target_aborted: error.authority_advanced_target_aborted(),
        note_ids: Vec::new(),
    })
}

#[derive(Debug, Clone)]
pub(crate) struct ExactMigrationTarget {
    pub kind: crate::services::twin_events::TargetKind,
    pub relative_key: String,
    pub expected_before: crate::services::twin_events::BeforeImage,
    pub desired: crate::services::twin_events::DesiredImage,
    pub note_event: Option<ExactMigrationNoteEvent>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExactMigrationNoteEvent {
    pub note_id: String,
    pub change: crate::models::twin_event::NoteChangeKind,
    pub observed_at: chrono::DateTime<Utc>,
    pub governance: crate::models::twin_event::Governance,
    pub payload_digest: crate::models::twin_event::ContentDigest,
    pub evidence_digest: crate::models::twin_event::ContentDigest,
}

pub(crate) struct MigrationMarkdownSnapshot {
    pub note: Option<Note>,
    pub source: crate::models::migration::MarkdownMigrationSourceV1,
    pub raw_bytes: Vec<u8>,
    pub overlay_raw_bytes: Option<Vec<u8>>,
}

const MAX_MIGRATION_SOURCE_COUNT: usize = 50_000;
const MAX_MIGRATION_SCAN_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct OptimizerMarkdownPrecondition {
    relative_path: String,
    expected_digest: crate::models::twin_event::ContentDigest,
    root: Arc<crate::services::twin_events::AnchoredRoot>,
}

impl OptimizerMarkdownPrecondition {
    pub(crate) fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub(crate) fn expected_digest(&self) -> &crate::models::twin_event::ContentDigest {
        &self.expected_digest
    }

    fn read_verified_bytes(&self) -> Result<Vec<u8>, crate::services::twin_events::MutationError> {
        let bytes = self
            .root
            .read_bounded(
                &self.relative_path,
                crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES,
            )?
            .ok_or_else(|| {
                crate::services::twin_events::MutationError::RecoveryConflict(
                    "optimizer source Markdown disappeared before commit".into(),
                )
            })?;
        if crate::services::twin_events::digest_bytes(&bytes) != self.expected_digest {
            return Err(
                crate::services::twin_events::MutationError::RecoveryConflict(
                    "optimizer source Markdown changed before commit".into(),
                ),
            );
        }
        Ok(bytes)
    }

    pub(crate) fn verify(&self) -> Result<(), crate::services::twin_events::MutationError> {
        self.read_verified_bytes().map(|_| ())
    }

    pub(crate) fn retained_target(
        &self,
    ) -> Result<
        crate::services::twin_events::TargetMutation,
        crate::services::twin_events::MutationError,
    > {
        let bytes = self.read_verified_bytes()?;
        let content = String::from_utf8(bytes).map_err(|_| {
            crate::services::twin_events::MutationError::Invalid(
                "optimizer source Markdown is not UTF-8".into(),
            )
        })?;
        Ok(crate::services::twin_events::TargetMutation::put(
            crate::services::twin_events::TargetKind::Markdown,
            self.relative_path.clone(),
            content,
        )
        .expecting(crate::services::twin_events::BeforeImage::Sha256(
            self.expected_digest.clone(),
        ))
        .retaining_exact_precondition())
    }
}

#[derive(Clone)]
struct NoteMutationContext {
    origin: crate::services::twin_events::MutationOrigin,
    source_channel: crate::models::twin_event::SourceChannel,
    capture_event: bool,
}

impl NoteMutationContext {
    fn local(source: &str) -> Result<Self> {
        Ok(Self {
            origin: crate::services::twin_events::MutationOrigin::Local,
            source_channel: crate::models::twin_event::SourceChannel::parse(source)
                .map_err(anyhow::Error::msg)?,
            capture_event: true,
        })
    }
}

impl KnowledgeStore {
    pub fn new(vault_path: PathBuf, data_path: PathBuf) -> Self {
        Self::with_event_recorder(
            vault_path,
            data_path,
            Arc::new(crate::services::twin_events::NoopEventRecorder),
        )
    }

    pub fn with_event_recorder(
        vault_path: PathBuf,
        data_path: PathBuf,
        event_recorder: Arc<dyn crate::services::twin_events::EventRecorder>,
    ) -> Self {
        if let Err(error) = std::fs::create_dir_all(&vault_path) {
            log::error!(
                "Failed to create vault directory {}: {}",
                vault_path.display(),
                error
            );
        }

        let overlay_notes_dir = data_path
            .join("vault_migration")
            .join("overlay")
            .join("notes");
        let _ = std::fs::create_dir_all(&overlay_notes_dir);
        let vault_root = crate::services::twin_events::AnchoredRoot::open(&vault_path)
            .map(Arc::new)
            .map_err(|error| log::error!("Failed to retain vault capability: {error}"))
            .ok();
        let overlay_root = crate::services::twin_events::AnchoredRoot::open(&overlay_notes_dir)
            .map(Arc::new)
            .map_err(|error| log::error!("Failed to retain overlay capability: {error}"))
            .ok();

        let mut store = Self {
            vault_path,
            overlay_notes_dir,
            vault_root,
            overlay_root,
            meta_cache: Vec::new(),
            path_index: HashMap::new(),
            title_index: HashMap::new(),
            alias_index: HashMap::new(),
            relative_path_index: HashMap::new(),
            event_recorder,
        };
        store.refresh_cache();
        store
    }

    /// Update the vault path at runtime (e.g., after settings change).
    #[cfg(test)]
    pub(crate) fn set_vault_path(&mut self, vault_path: PathBuf) -> Result<()> {
        crate::services::twin_events::validate_real_directory(&vault_path, "vault directory")
            .map_err(anyhow::Error::new)?;
        let vault_path = std::fs::canonicalize(&vault_path).with_context(|| {
            format!(
                "Failed to canonicalize vault directory {}",
                vault_path.display()
            )
        })?;
        self.vault_root = None;
        self.overlay_root = None;
        if let Err(error) = self.event_recorder.retarget_markdown_root(&vault_path) {
            self.vault_root = crate::services::twin_events::AnchoredRoot::open(&self.vault_path)
                .map(Arc::new)
                .ok();
            self.overlay_root =
                crate::services::twin_events::AnchoredRoot::open(&self.overlay_notes_dir)
                    .map(Arc::new)
                    .ok();
            return Err(anyhow::Error::new(error));
        }
        self.vault_path = vault_path;
        self.vault_root = Some(Arc::new(
            crate::services::twin_events::AnchoredRoot::open(&self.vault_path)
                .map_err(anyhow::Error::new)?,
        ));
        self.refresh_cache();
        Ok(())
    }

    pub(crate) fn adopt_coordinated_vault_path(
        &mut self,
        vault_path: PathBuf,
        derived_data_path: &Path,
    ) -> Result<()> {
        crate::services::twin_events::validate_real_directory(&vault_path, "vault directory")
            .map_err(anyhow::Error::new)?;
        self.vault_path = std::fs::canonicalize(&vault_path).with_context(|| {
            format!(
                "Failed to canonicalize vault directory {}",
                vault_path.display()
            )
        })?;
        self.overlay_notes_dir = derived_data_path
            .join("vault_migration")
            .join("overlay")
            .join("notes");
        std::fs::create_dir_all(&self.overlay_notes_dir).with_context(|| {
            format!(
                "Failed to prepare scoped overlay directory {}",
                self.overlay_notes_dir.display()
            )
        })?;
        self.vault_root = Some(Arc::new(
            crate::services::twin_events::AnchoredRoot::open(&self.vault_path)
                .map_err(anyhow::Error::new)?,
        ));
        self.overlay_root = Some(Arc::new(
            crate::services::twin_events::AnchoredRoot::open(&self.overlay_notes_dir)
                .map_err(anyhow::Error::new)?,
        ));
        self.refresh_cache();
        Ok(())
    }

    pub fn vault_path(&self) -> &std::path::Path {
        &self.vault_path
    }

    /// Builds the exact physical inputs used by Markdown migration previews.
    /// This deliberately bypasses the metadata cache for file discovery and
    /// bytes, then binds the freshly rebuilt note identity map and the complete
    /// overlay directory (including orphan overlays).
    pub(crate) fn migration_source_snapshot(
        &mut self,
        selected_program_path: &str,
        fallback_at: chrono::DateTime<Utc>,
    ) -> Result<(
        Vec<MigrationMarkdownSnapshot>,
        Vec<crate::models::migration::MarkdownMigrationOverlaySourceV1>,
    )> {
        let overlay_root = self
            .overlay_root
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("retained overlay capability is unavailable"))?;
        let mut overlays = Vec::new();
        let mut overlay_data_by_note = HashMap::new();
        let mut total_bytes = 0usize;
        if self.overlay_notes_dir.exists() {
            for entry in WalkDir::new(&self.overlay_notes_dir).min_depth(1) {
                let entry = entry?;
                if entry.file_type().is_symlink() {
                    anyhow::bail!(
                        "symlinked overlay source is not eligible for migration: {}",
                        entry.path().display()
                    );
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                if overlays.len() >= MAX_MIGRATION_SOURCE_COUNT {
                    anyhow::bail!("migration overlay inventory is too large");
                }
                let relative = entry.path().strip_prefix(&self.overlay_notes_dir)?;
                let relative_path = normalize_relative_path_for_output(&relative.to_string_lossy());
                if relative.components().count() != 1
                    || relative
                        .extension()
                        .and_then(|extension| extension.to_str())
                        != Some("json")
                {
                    anyhow::bail!(
                        "migration overlays must be direct <note-id>.json files: {relative_path}"
                    );
                }
                let note_id = relative
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .ok_or_else(|| anyhow::anyhow!("migration overlay name is not UTF-8"))?
                    .to_string();
                Self::validate_note_id(&note_id)?;
                let bytes = overlay_root
                    .read_bounded(
                        &relative_path,
                        crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES,
                    )
                    .map_err(anyhow::Error::new)?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "overlay disappeared during migration source scan: {relative_path}"
                        )
                    })?;
                total_bytes = total_bytes
                    .checked_add(bytes.len())
                    .ok_or_else(|| anyhow::anyhow!("migration scan byte count overflowed"))?;
                if total_bytes > MAX_MIGRATION_SCAN_BYTES {
                    anyhow::bail!("migration source inventory exceeds its aggregate byte limit");
                }
                let overlay_data: OverlayNoteData = serde_json::from_slice(&bytes)
                    .with_context(|| format!("invalid migration overlay: {relative_path}"))?;
                if overlay_data_by_note
                    .insert(
                        note_id,
                        (relative_path.clone(), overlay_data, bytes.clone()),
                    )
                    .is_some()
                {
                    anyhow::bail!("duplicate overlay note identity in migration source scan");
                }
                overlays.push(crate::models::migration::MarkdownMigrationOverlaySourceV1 {
                    relative_path,
                    digest: crate::services::twin_events::digest_bytes(&bytes),
                    byte_len: u64::try_from(bytes.len())?,
                });
            }
        }
        overlays.sort_by_key(|entry| migration_physical_path_key(&entry.relative_path));
        if overlays
            .windows(2)
            .any(|pair| migration_paths_equal(&pair[0].relative_path, &pair[1].relative_path))
        {
            anyhow::bail!("duplicate overlay path in migration source scan");
        }
        let overlays_by_path = overlays
            .iter()
            .map(|entry| (entry.relative_path.as_str(), entry))
            .collect::<HashMap<_, _>>();

        let vault_root = self
            .vault_root
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("retained vault capability is unavailable"))?;
        let selected_program_path = normalize_note_relative_path(selected_program_path)?;
        let mut seen_note_ids = HashSet::new();
        let mut snapshots = Vec::new();
        for entry in WalkDir::new(&self.vault_path).min_depth(1) {
            let entry = entry?;
            let is_markdown = entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
            if entry.file_type().is_symlink() {
                anyhow::bail!(
                    "symlinked vault entry is not eligible for migration: {}",
                    entry.path().display()
                );
            }
            if !entry.file_type().is_file() || !is_markdown {
                continue;
            }
            if snapshots.len() >= MAX_MIGRATION_SOURCE_COUNT {
                anyhow::bail!("migration Markdown inventory is too large");
            }
            let relative = entry.path().strip_prefix(&self.vault_path)?;
            let relative_path = normalize_note_relative_path(&normalize_relative_path_for_output(
                &relative.to_string_lossy(),
            ))?;
            let bytes = vault_root
                .read_bounded(
                    &relative_path,
                    crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES,
                )
                .map_err(anyhow::Error::new)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Markdown disappeared during migration source scan: {relative_path}"
                    )
                })?;
            total_bytes = total_bytes
                .checked_add(bytes.len())
                .ok_or_else(|| anyhow::anyhow!("migration scan byte count overflowed"))?;
            if total_bytes > MAX_MIGRATION_SCAN_BYTES {
                anyhow::bail!("migration source inventory exceeds its aggregate byte limit");
            }
            let markdown_digest = crate::services::twin_events::digest_bytes(&bytes);
            let note_id = if migration_paths_equal(&relative_path, &selected_program_path)
                || migration_paths_equal(&relative_path, "_grafyn/program.md")
            {
                None
            } else {
                let content = std::str::from_utf8(&bytes)
                    .with_context(|| format!("migration Markdown is not UTF-8: {relative_path}"))?;
                let path = self.vault_path.join(&relative_path);
                let mut note =
                    self.parse_note_content_without_overlay_at(&path, content, None, fallback_at)?;
                Self::validate_note_id(&note.id)?;
                if !seen_note_ids.insert(note.id.clone()) {
                    anyhow::bail!(
                        "duplicate note identity in migration source scan: {}",
                        note.id
                    );
                }
                if let Some((overlay_path, overlay_data, _)) = overlay_data_by_note.get(&note.id) {
                    Self::merge_overlay_data(
                        &mut note,
                        overlay_data.clone(),
                        &relative_path,
                        &markdown_digest,
                    );
                    debug_assert!(overlays_by_path.contains_key(overlay_path.as_str()));
                }
                let note_id = note.id.clone();
                Some((note_id, note))
            };
            let overlay = note_id.as_ref().map_or(
                crate::models::migration::MigrationSourceStateV1::Absent,
                |(note_id, _)| {
                    overlay_data_by_note.get(note_id).map_or(
                        crate::models::migration::MigrationSourceStateV1::Absent,
                        |(overlay_path, _, _)| {
                            let entry = overlays_by_path
                                .get(overlay_path.as_str())
                                .expect("validated overlay inventory entry exists");
                            crate::models::migration::MigrationSourceStateV1::Present {
                                digest: entry.digest.clone(),
                                byte_len: entry.byte_len,
                            }
                        },
                    )
                },
            );
            let source = crate::models::migration::MarkdownMigrationSourceV1 {
                relative_path,
                markdown_digest,
                byte_len: u64::try_from(bytes.len())?,
                note_id: note_id.as_ref().map(|(note_id, _)| note_id.clone()),
                overlay,
            };
            let overlay_raw_bytes = note_id
                .as_ref()
                .and_then(|(note_id, _)| overlay_data_by_note.get(note_id))
                .map(|(_, _, bytes)| bytes.clone());
            snapshots.push(MigrationMarkdownSnapshot {
                note: note_id.map(|(_, note)| note),
                source,
                raw_bytes: bytes,
                overlay_raw_bytes,
            });
        }
        snapshots.sort_by_key(|entry| migration_physical_path_key(&entry.source.relative_path));
        if snapshots.windows(2).any(|pair| {
            migration_paths_equal(&pair[0].source.relative_path, &pair[1].source.relative_path)
        }) {
            anyhow::bail!("duplicate Markdown path in migration source scan");
        }
        Ok((snapshots, overlays))
    }

    /// Commits one fully materialized migration step. The planner rechecks the
    /// exact physical before-image while holding the coordinator lock before it
    /// offers a plan, including when an external writer installed the desired
    /// bytes. Every real step retains a schema-3 receipt for owner recovery.
    pub(crate) fn commit_exact_migration_target_with_hooks(
        &mut self,
        target: ExactMigrationTarget,
        expected_authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
        prepared: &mut dyn FnMut(
            &crate::services::twin_events::MutationIntentV1,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
        committed: &mut dyn FnMut(
            &crate::services::twin_events::MutationCommit,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.commit_exact_target_with_hooks(
            target,
            None,
            "migration",
            expected_authority,
            prepared,
            committed,
        )
    }

    pub(crate) fn commit_exact_optimizer_target_with_hooks(
        &mut self,
        target: ExactMigrationTarget,
        source_guard: Option<crate::services::twin_events::TargetMutation>,
        expected_authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
        prepared: &mut dyn FnMut(
            &crate::services::twin_events::MutationIntentV1,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
        committed: &mut dyn FnMut(
            &crate::services::twin_events::MutationCommit,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.commit_exact_target_with_hooks(
            target,
            source_guard,
            "vault_optimizer",
            expected_authority,
            prepared,
            committed,
        )
    }

    fn commit_exact_target_with_hooks(
        &mut self,
        target: ExactMigrationTarget,
        source_guard: Option<crate::services::twin_events::TargetMutation>,
        source: &str,
        expected_authority: crate::services::vault_namespace::VaultAuthorityTokenV1,
        prepared: &mut dyn FnMut(
            &crate::services::twin_events::MutationIntentV1,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
        committed: &mut dyn FnMut(
            &crate::services::twin_events::MutationCommit,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        if !matches!(
            target.kind,
            crate::services::twin_events::TargetKind::Markdown
                | crate::services::twin_events::TargetKind::OverlayJson
        ) {
            anyhow::bail!("migration target kind is not supported");
        }
        let source_channel =
            crate::models::twin_event::SourceChannel::parse(source).map_err(anyhow::Error::msg)?;
        if self.event_recorder.is_noop() {
            anyhow::bail!("strict Markdown migration requires a mutation coordinator");
        }

        let root = match target.kind {
            crate::services::twin_events::TargetKind::Markdown => self.vault_root.clone(),
            crate::services::twin_events::TargetKind::OverlayJson => self.overlay_root.clone(),
            _ => unreachable!(),
        }
        .ok_or_else(|| anyhow::anyhow!("retained migration target capability is unavailable"))?;
        let recorder = self.event_recorder.clone();
        let target_for_plan = target.clone();
        let source_guard_for_plan = source_guard.clone();
        let mut planner = || {
            let current = root
                .read_bounded(
                    &target_for_plan.relative_key,
                    crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES,
                )?
                .map_or(crate::services::twin_events::BeforeImage::Absent, |bytes| {
                    crate::services::twin_events::BeforeImage::Sha256(
                        crate::services::twin_events::digest_bytes(&bytes),
                    )
                });
            if current != target_for_plan.expected_before {
                return Err(
                    crate::services::twin_events::MutationError::RecoveryConflict(
                        "exact migration target changed before commit".into(),
                    ),
                );
            }
            let after_digest =
                crate::services::twin_events::desired_digest(&target_for_plan.desired);
            if matches!(
                (&current, &target_for_plan.desired),
                (
                    crate::services::twin_events::BeforeImage::Absent,
                    crate::services::twin_events::DesiredImage::Tombstone
                )
            ) || matches!(
                &current,
                crate::services::twin_events::BeforeImage::Sha256(digest)
                    if digest == &after_digest
            ) {
                return Err(crate::services::twin_events::MutationError::RecoveryConflict(
                    "strict migration operation is already at its after-image without an owned receipt"
                        .into(),
                ));
            }
            let coordinator_target = crate::services::twin_events::TargetMutation {
                kind: target_for_plan.kind,
                relative_key: target_for_plan.relative_key.clone(),
                after: target_for_plan.desired.clone(),
                expected_before: Some(target_for_plan.expected_before.clone()),
                retain_exact_precondition: false,
                check_expected_before_before_after_elision: true,
            };
            let mut coordinator_targets =
                Vec::with_capacity(1 + usize::from(source_guard_for_plan.is_some()));
            if let Some(source_guard) = source_guard_for_plan.clone() {
                coordinator_targets.push(source_guard);
            }
            coordinator_targets.push(coordinator_target);
            let drafts = target_for_plan
                .note_event
                .as_ref()
                .map(|event| {
                    crate::services::twin_events::note_changed_draft(
                        &event.note_id,
                        event.change.clone(),
                        event.payload_digest.clone(),
                        event.evidence_digest.clone(),
                        event.observed_at,
                        source_channel.clone(),
                        event.governance.clone(),
                    )
                    .map_err(crate::services::twin_events::MutationError::Invalid)
                })
                .transpose()?
                .into_iter()
                .collect();
            Ok(Some(
                crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::SyncEligible,
                    source_channel.clone(),
                    coordinator_targets,
                    drafts,
                )
                .expecting_authority(expected_authority.clone())
                .retaining_commit_receipt(),
            ))
        };
        let commit = recorder
            .commit_planned_mutation_with_hooks(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
                prepared,
                committed,
            )
            .map_err(anyhow::Error::new)?;
        if commit.mutation_id.is_none() || commit.authority_token.is_none() {
            anyhow::bail!("strict mutation did not return an owned receipt and authority");
        }
        self.refresh_cache();
        Ok(commit)
    }

    /// Rebuild the metadata cache and lookups from disk.
    fn refresh_cache(&mut self) {
        let mut notes = Vec::new();
        self.path_index.clear();
        self.title_index.clear();
        self.alias_index.clear();
        self.relative_path_index.clear();

        for entry in WalkDir::new(&self.vault_path)
            .min_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }

            if self.is_reserved_program_path(path) {
                continue;
            }

            match self.read_note_file(path) {
                Ok(note) => {
                    self.path_index.insert(note.id.clone(), path.to_path_buf());
                    self.relative_path_index
                        .insert(normalize_lookup_key(&note.relative_path), note.id.clone());
                    self.title_index
                        .entry(normalize_lookup_key(&note.title))
                        .or_insert_with(|| note.id.clone());
                    for alias in &note.aliases {
                        self.alias_index
                            .entry(normalize_lookup_key(alias))
                            .or_insert_with(|| note.id.clone());
                    }
                    notes.push(NoteMeta::from(&note));
                }
                Err(error) => {
                    log::warn!(
                        "Failed to read markdown note '{}': {}",
                        path.display(),
                        error
                    );
                }
            }
        }

        notes.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        self.meta_cache = notes;
    }

    fn cache_exact_note(&mut self, note: &Note) -> Result<()> {
        let path = self.resolve_vault_relative_path(&note.relative_path)?;
        self.meta_cache.retain(|meta| meta.id != note.id);
        self.meta_cache.push(NoteMeta::from(note));
        self.meta_cache
            .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        self.path_index.insert(note.id.clone(), path);
        self.title_index.retain(|_, value| value != &note.id);
        self.alias_index.retain(|_, value| value != &note.id);
        self.relative_path_index
            .retain(|_, value| value != &note.id);
        self.title_index
            .entry(normalize_lookup_key(&note.title))
            .or_insert_with(|| note.id.clone());
        for alias in &note.aliases {
            self.alias_index
                .entry(normalize_lookup_key(alias))
                .or_insert_with(|| note.id.clone());
        }
        self.relative_path_index
            .insert(normalize_lookup_key(&note.relative_path), note.id.clone());
        Ok(())
    }

    pub(crate) fn reload_authoritative_state(&mut self) {
        self.refresh_cache();
    }

    pub fn list_notes(&self) -> Result<Vec<NoteMeta>> {
        Ok(self.meta_cache.clone())
    }

    pub fn list_full_notes(&self) -> Result<Vec<Note>> {
        let mut notes = Vec::with_capacity(self.meta_cache.len());
        for meta in &self.meta_cache {
            notes.push(self.get_note(&meta.id)?);
        }
        Ok(notes)
    }

    pub fn get_note(&self, id: &str) -> Result<Note> {
        Self::validate_note_id(id)?;
        let path = self.note_path(id)?;
        self.read_note_file(&path)
            .with_context(|| format!("Note not found: {}", id))
    }

    pub fn find_note_by_title_case_insensitive(&self, title: &str) -> Result<Option<Note>> {
        let normalized = normalize_lookup_key(title);
        if normalized.is_empty() {
            return Ok(None);
        }

        if let Some(note_id) = self.title_index.get(&normalized) {
            return self.get_note(note_id).map(Some);
        }

        if let Some(note_id) = self.alias_index.get(&normalized) {
            return self.get_note(note_id).map(Some);
        }

        Ok(None)
    }

    pub fn find_note_by_relative_path(&self, relative_path: &str) -> Result<Option<Note>> {
        let normalized = normalize_lookup_key(relative_path);
        let Some(note_id) = self.relative_path_index.get(&normalized) else {
            return Ok(None);
        };
        self.get_note(note_id).map(Some)
    }

    pub fn create_note(&mut self, create: NoteCreate) -> Result<Note> {
        self.create_note_from_source(create, "note_editor")
    }

    pub fn create_note_from_source(&mut self, create: NoteCreate, source: &str) -> Result<Note> {
        self.create_note_from_source_with_commit(create, source)
            .map(|(note, _)| note)
    }

    pub(crate) fn create_note_from_source_with_commit(
        &mut self,
        create: NoteCreate,
        source: &str,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        self.create_note_with_context(create, NoteMutationContext::local(source)?, None)
    }

    pub(crate) fn create_note_expecting_authority(
        &mut self,
        create: NoteCreate,
        source: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        self.create_note_with_context(create, NoteMutationContext::local(source)?, Some(expected))
    }

    pub(crate) fn create_companion_capture_expecting_authority(
        &mut self,
        create: NoteCreate,
        observation: crate::services::twin_events::CompanionObservationInput,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        self.create_companion_evidence_expecting_authority(
            create,
            observation,
            "companion_capture",
            expected,
        )
    }

    pub(crate) fn create_companion_evidence_expecting_authority(
        &mut self,
        create: NoteCreate,
        observation: crate::services::twin_events::CompanionObservationInput,
        source: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        if self.event_recorder.is_noop() {
            anyhow::bail!("companion capture requires the mutation coordinator");
        }
        self.create_note_with_locked_plan(
            create,
            NoteMutationContext::local(source)?,
            Some(expected),
            Some(observation),
        )
    }

    pub(crate) fn create_generated_image_capture_expecting_authority(
        &mut self,
        create: NoteCreate,
        observation: crate::services::twin_events::CompanionObservationInput,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> KnowledgeNotePersistenceAttempt {
        let result = self.create_companion_evidence_expecting_authority(
            create,
            observation,
            "image_generation",
            expected,
        );
        classify_generated_image_persistence(result)
    }

    pub fn import_note_container(
        &mut self,
        creates: Vec<NoteCreate>,
        container_id: &str,
        source_bytes: &[u8],
    ) -> Result<Vec<Note>> {
        self.import_note_container_with_commit(creates, container_id, source_bytes)
            .map(|(notes, _)| notes)
    }

    pub(crate) fn import_note_container_with_commit(
        &mut self,
        creates: Vec<NoteCreate>,
        container_id: &str,
        source_bytes: &[u8],
    ) -> Result<(Vec<Note>, crate::services::twin_events::MutationCommit)> {
        if creates.is_empty() || creates.len() > 63 {
            anyhow::bail!("an import container must persist 1..=63 notes");
        }
        if self.event_recorder.is_noop() {
            let notes = creates
                .into_iter()
                .map(|create| {
                    self.create_note_from_source_with_commit(create, "import")
                        .map(|(note, _)| note)
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok((
                notes,
                crate::services::twin_events::MutationCommit {
                    mutation_id: None,
                    events: Vec::new(),
                    authority_token: None,
                    postcommit_warning: false,
                },
            ));
        }

        let source_digest = crate::services::twin_events::digest_bytes(source_bytes);
        let container_digest = crate::services::twin_events::digest_bytes(
            format!("{container_id}\0{}", source_digest.as_str()).as_bytes(),
        );
        let import_id = format!("import-{}", container_digest.as_str());
        let observation_id = format!("import-observation-{}", container_digest.as_str());
        let recorder = self.event_recorder.clone();
        let mut return_ids = None;
        let mut planner = || {
            self.refresh_cache();
            let now = Utc::now();
            let mut reserved_ids = self
                .meta_cache
                .iter()
                .map(|note| note.id.clone())
                .collect::<HashSet<_>>();
            let mut reserved_paths = self
                .meta_cache
                .iter()
                .map(|note| normalize_lookup_key(&note.relative_path))
                .collect::<HashSet<_>>();
            let mut planned = Vec::with_capacity(creates.len());
            for create in &creates {
                let id = self.generate_note_id_with_reserved(&create.title, &reserved_ids);
                reserved_ids.insert(id.clone());
                let preferred_path = match &create.relative_path {
                    Some(path) => normalize_note_relative_path(path).map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?,
                    None => format!("{id}.md"),
                };
                let relative_path =
                    self.make_unique_relative_path_with_reserved(&preferred_path, &reserved_paths);
                reserved_paths.insert(normalize_lookup_key(&relative_path));
                let mut note = Note {
                    id,
                    title: create.title.clone(),
                    content: create.content.clone(),
                    relative_path,
                    aliases: dedupe_strings(create.aliases.clone()),
                    status: create.status.clone(),
                    tags: dedupe_strings(create.tags.clone()),
                    created_at: now,
                    updated_at: now,
                    schema_version: create.schema_version.max(CURRENT_NOTE_SCHEMA_VERSION),
                    migration_source: create.migration_source.clone(),
                    optimizer_managed: create.optimizer_managed,
                    wikilinks: Vec::new(),
                    parsed_links: Vec::new(),
                    properties: create.properties.clone(),
                    frontmatter_raw_fallback: None,
                };
                note.wikilinks = self.extract_wikilinks(&note.content);
                note.parsed_links = self.extract_links(&note.content, &note.relative_path);
                planned.push(note);
            }

            return_ids = Some(
                planned
                    .iter()
                    .map(|note| note.id.clone())
                    .collect::<Vec<_>>(),
            );
            planned.sort_by(|left, right| left.id.cmp(&right.id));
            let source = crate::models::twin_event::SourceChannel::parse("import")
                .map_err(crate::services::twin_events::MutationError::Invalid)?;
            let mut targets = Vec::with_capacity(planned.len());
            let mut drafts = Vec::with_capacity(planned.len() + 1);
            let mut note_evidence = Vec::with_capacity(planned.len());
            for note in &planned {
                let (relative_path, bytes) =
                    Self::canonical_serialized_note_bytes(note).map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                let digest = crate::services::twin_events::digest_bytes(&bytes);
                targets.push(crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    relative_path,
                    String::from_utf8(bytes).expect("Markdown serialization is UTF-8"),
                ));
                drafts.push(
                    crate::services::twin_events::note_changed_draft(
                        &note.id,
                        crate::models::twin_event::NoteChangeKind::Created,
                        digest.clone(),
                        digest.clone(),
                        note.updated_at,
                        source.clone(),
                        note_capture_governance(note, "import"),
                    )
                    .map_err(crate::services::twin_events::MutationError::Invalid)?,
                );
                note_evidence.push((note.id.clone(), digest));
            }
            drafts.push(
                crate::services::twin_events::import_container_observation_draft_for_notes(
                    &observation_id,
                    &import_id,
                    source_digest.clone(),
                    &note_evidence,
                    now,
                )
                .map_err(crate::services::twin_events::MutationError::Invalid)?,
            );
            Ok(Some(crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::SyncEligible,
                source,
                targets,
                drafts,
            )))
        };
        let commit = match recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut planner,
        ) {
            Ok(commit) => commit,
            Err(error) => {
                self.refresh_cache();
                return Err(preserve_knowledge_authority_error(
                    error,
                    return_ids.clone().unwrap_or_default(),
                ));
            }
        };
        self.refresh_cache();
        let notes = return_ids
            .expect("import planner returned note IDs")
            .iter()
            .map(|note_id| self.get_note(note_id))
            .collect::<Result<Vec<_>>>()?;
        Ok((notes, commit))
    }

    fn create_note_with_context(
        &mut self,
        create: NoteCreate,
        context: NoteMutationContext,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        if !self.event_recorder.is_noop() {
            return self.create_note_with_locked_plan(create, context, expected, None);
        }
        let id = self.generate_note_id(&create.title);
        let relative_path = match create.relative_path {
            Some(path) => self.make_unique_relative_path(&normalize_note_relative_path(&path)?),
            None => self.make_unique_relative_path(&format!("{}.md", id)),
        };
        let now = Utc::now();

        let mut note = Note {
            id,
            title: create.title,
            content: create.content,
            relative_path,
            aliases: dedupe_strings(create.aliases),
            status: create.status,
            tags: dedupe_strings(create.tags),
            created_at: now,
            updated_at: now,
            schema_version: create.schema_version.max(CURRENT_NOTE_SCHEMA_VERSION),
            migration_source: create.migration_source,
            optimizer_managed: create.optimizer_managed,
            wikilinks: Vec::new(),
            parsed_links: Vec::new(),
            properties: create.properties,
            frontmatter_raw_fallback: None,
        };

        note.wikilinks = self.extract_wikilinks(&note.content);
        note.parsed_links = self.extract_links(&note.content, &note.relative_path);

        if let Err(error) = self.persist_note_change(
            &note,
            None,
            crate::models::twin_event::NoteChangeKind::Created,
            context,
        ) {
            self.refresh_cache();
            return Err(error);
        }
        self.refresh_cache();
        Ok((
            self.get_note(&note.id)?,
            crate::services::twin_events::MutationCommit {
                mutation_id: None,
                events: Vec::new(),
                authority_token: None,
                postcommit_warning: false,
            },
        ))
    }

    fn create_note_with_locked_plan(
        &mut self,
        create: NoteCreate,
        context: NoteMutationContext,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
        companion_observation: Option<crate::services::twin_events::CompanionObservationInput>,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        let recorder = self.event_recorder.clone();
        let mut planned_id = None;
        let mut planner = || {
            self.refresh_cache();
            let id = self.generate_note_id(&create.title);
            let relative_path = match &create.relative_path {
                Some(path) => self.make_unique_relative_path(
                    &normalize_note_relative_path(path).map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?,
                ),
                None => self.make_unique_relative_path(&format!("{id}.md")),
            };
            let now = companion_observation
                .as_ref()
                .map(|observation| observation.observed_at)
                .unwrap_or_else(Utc::now);
            let mut note = Note {
                id: id.clone(),
                title: create.title.clone(),
                content: create.content.clone(),
                relative_path,
                aliases: dedupe_strings(create.aliases.clone()),
                status: create.status.clone(),
                tags: dedupe_strings(create.tags.clone()),
                created_at: now,
                updated_at: now,
                schema_version: create.schema_version.max(CURRENT_NOTE_SCHEMA_VERSION),
                migration_source: create.migration_source.clone(),
                optimizer_managed: create.optimizer_managed,
                wikilinks: Vec::new(),
                parsed_links: Vec::new(),
                properties: create.properties.clone(),
                frontmatter_raw_fallback: None,
            };
            note.wikilinks = self.extract_wikilinks(&note.content);
            note.parsed_links = self.extract_links(&note.content, &note.relative_path);
            let mut plan = self
                .plan_note_change(
                    &note,
                    None,
                    crate::models::twin_event::NoteChangeKind::Created,
                    context.clone(),
                )
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            if let Some(observation) = companion_observation.as_ref() {
                let (_, note_bytes) =
                    Self::canonical_serialized_note_bytes(&note).map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                let note_digest = crate::services::twin_events::digest_bytes(&note_bytes);
                plan.drafts.push(
                    crate::services::twin_events::companion_capture_observation_draft(
                        &note,
                        note_digest,
                        observation,
                    )
                    .map_err(crate::services::twin_events::MutationError::Invalid)?,
                );
            }
            if let Some(expected) = expected.clone() {
                plan = plan.expecting_authority(expected);
            }
            planned_id = Some(id);
            Ok(Some(plan))
        };
        let mut prepared_events = Vec::new();
        let result = if companion_observation.is_some() {
            recorder.commit_planned_mutation_with_hooks(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
                &mut |intent| {
                    prepared_events = intent.events.clone();
                    Ok(())
                },
                &mut |_| Ok(()),
            )
        } else {
            recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            )
        };
        self.refresh_cache();
        let commit = result.map_err(|error| {
            preserve_knowledge_authority_error_with_events(
                error,
                planned_id.iter().cloned().collect::<Vec<_>>(),
                prepared_events,
            )
        })?;
        let planned_id = planned_id.expect("planner returned a note ID");
        let note = self.get_note(&planned_id).map_err(|error| {
            preserve_committed_knowledge_result_error(
                commit.clone(),
                vec![planned_id.clone()],
                error,
            )
        })?;
        Ok((note, commit))
    }

    pub fn update_note(&mut self, id: &str, update: NoteUpdate) -> Result<Note> {
        self.update_note_from_source(id, update, "note_editor")
    }

    pub fn update_note_from_source(
        &mut self,
        id: &str,
        update: NoteUpdate,
        source: &str,
    ) -> Result<Note> {
        self.update_note_from_source_with_commit(id, update, source)
            .map(|(note, _)| note)
    }

    pub(crate) fn update_note_from_source_with_commit(
        &mut self,
        id: &str,
        update: NoteUpdate,
        source: &str,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        self.update_note_with_options(id, update, true, NoteMutationContext::local(source)?, None)
    }

    pub(crate) fn update_note_expecting_authority(
        &mut self,
        id: &str,
        update: NoteUpdate,
        source: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        self.update_note_with_options(
            id,
            update,
            true,
            NoteMutationContext::local(source)?,
            Some(expected),
        )
    }

    /// Same as `update_note`, but when `bump_updated_at` is false the note's existing
    /// `updated_at` is preserved instead of being stamped with `Utc::now()`. Used by
    /// `update_note_preserving_timestamp` (below) for the boot-time schema backfill
    /// (`markdown_migration::backfill_legacy_grafyn_notes`), so a purely administrative
    /// write (schema_version/migration_source bookkeeping) doesn't skew recency-based
    /// ranking on every app launch.
    fn update_note_with_options(
        &mut self,
        id: &str,
        update: NoteUpdate,
        bump_updated_at: bool,
        context: NoteMutationContext,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        if !self.event_recorder.is_noop() {
            return self.update_note_with_locked_plan(
                id,
                update,
                bump_updated_at,
                context,
                expected,
            );
        }
        Self::validate_note_id(id)?;
        let mut note = self.get_note(id)?;
        if !note_update_changes(&note, &update)? {
            return Ok((
                note,
                crate::services::twin_events::MutationCommit {
                    mutation_id: None,
                    events: Vec::new(),
                    authority_token: None,
                    postcommit_warning: false,
                },
            ));
        }
        let old_path = self.note_path(id)?;

        // A note may be carrying an unparsable original frontmatter block
        // (`frontmatter_raw_fallback`, see doc comment on `Note`). Any of these fields
        // being explicitly set means the caller consciously wants new frontmatter
        // written, so the fallback is cleared below and normal serialization takes
        // over. A content-only update (only `content` and/or `relative_path` set)
        // leaves the fallback in place, so the original frontmatter is re-emitted
        // verbatim by `write_note_file` instead of being replaced by defaults.
        let explicit_frontmatter_edit = update.title.is_some()
            || update.aliases.is_some()
            || update.status.is_some()
            || update.tags.is_some()
            || update.schema_version.is_some()
            || update.migration_source.is_some()
            || update.optimizer_managed.is_some()
            || update.properties.is_some();

        if let Some(title) = update.title {
            note.title = title;
        }
        if let Some(content) = update.content {
            note.content = content;
        }
        if let Some(relative_path) = update.relative_path {
            note.relative_path = normalize_note_relative_path(&relative_path)?;
        }
        if let Some(aliases) = update.aliases {
            note.aliases = dedupe_strings(aliases);
        }
        if let Some(status) = update.status {
            note.status = status;
        }
        if let Some(tags) = update.tags {
            note.tags = dedupe_strings(tags);
        }
        if let Some(schema_version) = update.schema_version {
            note.schema_version = schema_version.max(CURRENT_NOTE_SCHEMA_VERSION);
        }
        if update.migration_source.is_some() {
            note.migration_source = update.migration_source;
        }
        if let Some(optimizer_managed) = update.optimizer_managed {
            note.optimizer_managed = optimizer_managed;
        }
        if let Some(properties) = update.properties {
            note.properties = properties;
        }

        if explicit_frontmatter_edit {
            note.frontmatter_raw_fallback = None;
        }

        if bump_updated_at {
            note.updated_at = Utc::now();
        }
        note.wikilinks = self.extract_wikilinks(&note.content);
        note.parsed_links = self.extract_links(&note.content, &note.relative_path);

        if let Err(error) = self.persist_note_change(
            &note,
            Some(&old_path),
            crate::models::twin_event::NoteChangeKind::Updated,
            context,
        ) {
            self.refresh_cache();
            return Err(error);
        }

        self.refresh_cache();
        Ok((
            self.get_note(&note.id)?,
            crate::services::twin_events::MutationCommit {
                mutation_id: None,
                events: Vec::new(),
                authority_token: None,
                postcommit_warning: false,
            },
        ))
    }

    fn update_note_with_locked_plan(
        &mut self,
        id: &str,
        update: NoteUpdate,
        bump_updated_at: bool,
        context: NoteMutationContext,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        Self::validate_note_id(id)?;
        let note_id = id.to_string();
        let recorder = self.event_recorder.clone();
        let mut planned_id = None;
        let mut planner = || {
            self.refresh_cache();
            let mut note = self.get_note(&note_id).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
            if !note_update_changes(&note, &update).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })? {
                planned_id = Some(note.id.clone());
                return Ok(None);
            }
            let old_path = self.note_path(&note_id).map_err(|error| {
                crate::services::twin_events::MutationError::Invalid(error.to_string())
            })?;
            self.apply_note_update(&mut note, update.clone(), bump_updated_at)
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            let mut plan = self
                .plan_note_change(
                    &note,
                    Some(&old_path),
                    crate::models::twin_event::NoteChangeKind::Updated,
                    context.clone(),
                )
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            if let Some(expected) = expected.clone() {
                plan = plan.expecting_authority(expected);
            }
            planned_id = Some(note.id.clone());
            Ok(Some(plan))
        };
        let result = recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut planner,
        );
        self.refresh_cache();
        let commit = result.map_err(|error| {
            preserve_knowledge_authority_error(
                error,
                planned_id.iter().cloned().collect::<Vec<_>>(),
            )
        })?;
        Ok((
            self.get_note(planned_id.as_deref().expect("planner returned a note ID"))?,
            commit,
        ))
    }

    fn apply_note_update(
        &self,
        note: &mut Note,
        update: NoteUpdate,
        bump_updated_at: bool,
    ) -> Result<()> {
        let explicit_frontmatter_edit = update.title.is_some()
            || update.aliases.is_some()
            || update.status.is_some()
            || update.tags.is_some()
            || update.schema_version.is_some()
            || update.migration_source.is_some()
            || update.optimizer_managed.is_some()
            || update.properties.is_some();
        if let Some(title) = update.title {
            note.title = title;
        }
        if let Some(content) = update.content {
            note.content = content;
        }
        if let Some(relative_path) = update.relative_path {
            note.relative_path = normalize_note_relative_path(&relative_path)?;
        }
        if let Some(aliases) = update.aliases {
            note.aliases = dedupe_strings(aliases);
        }
        if let Some(status) = update.status {
            note.status = status;
        }
        if let Some(tags) = update.tags {
            note.tags = dedupe_strings(tags);
        }
        if let Some(schema_version) = update.schema_version {
            note.schema_version = schema_version.max(CURRENT_NOTE_SCHEMA_VERSION);
        }
        if update.migration_source.is_some() {
            note.migration_source = update.migration_source;
        }
        if let Some(optimizer_managed) = update.optimizer_managed {
            note.optimizer_managed = optimizer_managed;
        }
        if let Some(properties) = update.properties {
            note.properties = properties;
        }
        if explicit_frontmatter_edit {
            note.frontmatter_raw_fallback = None;
        }
        if bump_updated_at {
            note.updated_at = Utc::now();
        }
        note.wikilinks = self.extract_wikilinks(&note.content);
        note.parsed_links = self.extract_links(&note.content, &note.relative_path);
        Ok(())
    }

    fn extract_markdown_links(&self, content: &str, source_relative_path: &str) -> Vec<ParsedLink> {
        let source_path = Path::new(source_relative_path);
        let base_dir = source_path.parent().unwrap_or_else(|| Path::new(""));

        MARKDOWN_LINK_REGEX
            .captures_iter(content)
            .filter_map(|cap| {
                let raw_target = cap.get(1)?.as_str().split('#').next()?.trim();
                if raw_target.contains("://") || raw_target.starts_with("mailto:") {
                    return None;
                }
                let joined = base_dir.join(raw_target);
                let target_path = normalize_note_relative_path(&joined.to_string_lossy()).ok()?;
                let title = Path::new(&target_path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(humanize_filename)
                    .unwrap_or_default();
                Some(ParsedLink {
                    target_title: title,
                    target_path: Some(target_path),
                    relation: RelationType::Untyped,
                })
            })
            .collect()
    }

    fn generate_note_id(&self, title: &str) -> String {
        let existing_ids = self
            .meta_cache
            .iter()
            .map(|note| note.id.clone())
            .collect::<HashSet<_>>();
        self.generate_note_id_with_reserved(title, &existing_ids)
    }

    fn generate_note_id_with_reserved(
        &self,
        title: &str,
        reserved_ids: &HashSet<String>,
    ) -> String {
        let slug = slugify(title);
        let base = if slug.is_empty() {
            "note".to_string()
        } else {
            slug
        };
        let mut id = base.clone();
        let mut counter = 1;
        while reserved_ids.contains(&id) {
            id = format!("{}-{}", base, counter);
            counter += 1;
        }
        id
    }

    fn validate_note_id(id: &str) -> Result<()> {
        if id.is_empty()
            || id.contains('/')
            || id.contains('\\')
            || id.contains("..")
            || id.contains(':')
        {
            anyhow::bail!("Invalid note ID: {}", id);
        }
        if is_reserved_windows_component(id) {
            anyhow::bail!("Invalid note ID: {} (reserved device name)", id);
        }
        Ok(())
    }

    fn note_path(&self, id: &str) -> Result<PathBuf> {
        match self.path_index.get(id) {
            Some(cached) => Ok(cached.clone()),
            None => self.resolve_vault_relative_path(&format!("{}.md", id)),
        }
    }

    /// Joins `relative` onto the vault root and verifies the result cannot
    /// have escaped the vault (belt-and-braces on top of the string-level
    /// validators in `validate_note_id` / `normalize_note_relative_path`).
    fn resolve_vault_relative_path(&self, relative: &str) -> Result<PathBuf> {
        let joined = self.vault_path.join(relative);
        ensure_path_within_vault(&self.vault_path, &joined)?;
        Ok(joined)
    }

    fn read_note_file(&self, path: &Path) -> Result<Note> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;
        let file_modified = std::fs::metadata(path)?.modified().ok();
        self.parse_note_content(path, &content, file_modified)
    }

    fn parse_note_content(
        &self,
        path: &Path,
        content: &str,
        file_modified: Option<std::time::SystemTime>,
    ) -> Result<Note> {
        let mut note = self.parse_note_content_without_overlay(path, content, file_modified)?;
        let markdown_digest = crate::services::twin_events::digest_bytes(content.as_bytes());
        self.merge_overlay(&mut note, &markdown_digest);
        Ok(note)
    }

    fn parse_note_content_without_overlay(
        &self,
        path: &Path,
        content: &str,
        file_modified: Option<std::time::SystemTime>,
    ) -> Result<Note> {
        self.parse_note_content_without_overlay_at(path, content, file_modified, Utc::now())
    }

    fn parse_note_content_without_overlay_at(
        &self,
        path: &Path,
        content: &str,
        file_modified: Option<std::time::SystemTime>,
        fallback_at: chrono::DateTime<Utc>,
    ) -> Result<Note> {
        let matter = Matter::<YAML>::new();
        let parsed = matter.parse(content);

        // `parsed.matter` holds the raw text between the `---` delimiters verbatim,
        // regardless of whether it deserialized successfully. We capture it before
        // consuming `parsed.data` so a note can fall back to it below.
        let raw_frontmatter_block = parsed.matter.clone();
        let has_frontmatter_block = !raw_frontmatter_block.trim().is_empty();

        let deserialized_frontmatter: Option<NoteFrontmatter> = parsed
            .data
            .map(|data| data.deserialize())
            .transpose()
            .unwrap_or(None);

        // If a frontmatter block exists but failed to deserialize (malformed YAML,
        // missing required fields, etc.), preserve the raw block instead of silently
        // discarding it. See `frontmatter_raw_fallback` doc comment on `Note` for the
        // full preserve/clear contract.
        let frontmatter_raw_fallback = if has_frontmatter_block
            && deserialized_frontmatter.is_none()
        {
            log::warn!(
                "Frontmatter for note '{}' failed to parse; preserving raw block verbatim instead of defaulting metadata",
                path.display()
            );
            Some(raw_frontmatter_block)
        } else {
            None
        };

        let frontmatter = deserialized_frontmatter.unwrap_or_default();

        let relative_path = path
            .strip_prefix(&self.vault_path)
            .ok()
            .and_then(|value| value.to_str())
            .map(normalize_relative_path_for_output)
            .unwrap_or_else(|| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("note.md")
                    .to_string()
            });
        let file_stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("note");

        let now = fallback_at;
        let created_at = frontmatter.created_at.unwrap_or(now);
        let updated_at = frontmatter
            .updated_at
            .or_else(|| file_modified.map(chrono::DateTime::<Utc>::from))
            .unwrap_or(now);

        let body = parsed.content;
        let title = if !frontmatter.title.trim().is_empty() {
            frontmatter.title.trim().to_string()
        } else if let Some(caps) = H1_REGEX.captures(&body) {
            caps.get(1)
                .map(|value| value.as_str().trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| humanize_filename(file_stem))
        } else {
            humanize_filename(file_stem)
        };

        let frontmatter_aliases = dedupe_strings(frontmatter.aliases);
        let derived_aliases = if is_reserved_synced_materialization_path(&relative_path) {
            Vec::new()
        } else {
            alias_candidates(&title, file_stem)
        };
        let aliases = dedupe_strings(frontmatter_aliases.into_iter().chain(derived_aliases));
        let tags = dedupe_strings(
            frontmatter
                .tags
                .into_iter()
                .chain(extract_inline_hashtags(&body)),
        );
        let note_id = frontmatter
            .note_id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| slugify(&relative_path));

        let note = Note {
            id: note_id,
            title,
            content: body.clone(),
            relative_path: relative_path.clone(),
            aliases,
            status: frontmatter.status.parse().unwrap_or_default(),
            tags,
            created_at,
            updated_at,
            schema_version: frontmatter.schema_version.max(CURRENT_NOTE_SCHEMA_VERSION),
            migration_source: frontmatter.migration_source,
            optimizer_managed: frontmatter.optimizer_managed,
            wikilinks: self.extract_wikilinks(&body),
            parsed_links: self.extract_links(&body, &relative_path),
            properties: frontmatter.extra,
            frontmatter_raw_fallback,
        };

        Ok(note)
    }

    fn merge_overlay(
        &self,
        note: &mut Note,
        markdown_digest: &crate::models::twin_event::ContentDigest,
    ) {
        let overlay_path = self.overlay_path(&note.id);
        let overlay = std::fs::read_to_string(&overlay_path)
            .ok()
            .and_then(|content| serde_json::from_str::<OverlayNoteData>(&content).ok());
        let Some(overlay) = overlay else {
            return;
        };

        let relative_path = note.relative_path.clone();
        Self::merge_overlay_data(note, overlay, &relative_path, markdown_digest);
    }

    fn merge_overlay_data(
        note: &mut Note,
        overlay: OverlayNoteData,
        markdown_relative_path: &str,
        markdown_digest: &crate::models::twin_event::ContentDigest,
    ) {
        if overlay.optimizer_source.as_ref().is_some_and(|source| {
            source.relative_path != markdown_relative_path || &source.sha256 != markdown_digest
        }) {
            return;
        }
        note.aliases = dedupe_strings(note.aliases.clone().into_iter().chain(overlay.aliases));
        note.tags = dedupe_strings(note.tags.clone().into_iter().chain(overlay.tags));
        if let Some(schema_version) = overlay.schema_version {
            note.schema_version = note.schema_version.max(schema_version);
        }
        if overlay.migration_source.is_some() {
            note.migration_source = overlay.migration_source;
        }
        if let Some(optimizer_managed) = overlay.optimizer_managed {
            note.optimizer_managed = optimizer_managed;
        }
        for (key, value) in overlay.properties {
            note.properties.insert(key, value);
        }
    }

    fn plan_note_change(
        &self,
        note: &Note,
        old_path: Option<&Path>,
        change: crate::models::twin_event::NoteChangeKind,
        context: NoteMutationContext,
    ) -> Result<crate::services::twin_events::MutationPlan> {
        let (relative_path, after_bytes) = Self::canonical_serialized_note_bytes(note)?;
        let after_digest = crate::services::twin_events::digest_bytes(&after_bytes);
        let old_bytes = old_path
            .filter(|path| path.exists())
            .map(std::fs::read)
            .transpose()?;
        let evidence_digest = old_bytes
            .as_deref()
            .map(crate::services::twin_events::digest_bytes)
            .unwrap_or_else(|| after_digest.clone());
        let mut targets = vec![crate::services::twin_events::TargetMutation::put(
            crate::services::twin_events::TargetKind::Markdown,
            normalize_relative_path_for_output(&relative_path),
            String::from_utf8(after_bytes).expect("Markdown serialization is UTF-8"),
        )];
        if let Some(old_path) = old_path {
            let new_path = self.resolve_vault_relative_path(&relative_path)?;
            if old_path != new_path {
                let old_key = old_path
                    .strip_prefix(&self.vault_path)
                    .map_err(|_| anyhow::anyhow!("old note path escaped the vault"))?
                    .to_string_lossy();
                targets.push(crate::services::twin_events::TargetMutation::tombstone(
                    crate::services::twin_events::TargetKind::Markdown,
                    normalize_relative_path_for_output(&old_key),
                ));
            }
        }
        let drafts = if context.origin == crate::services::twin_events::MutationOrigin::Local
            && context.capture_event
        {
            let governance = note_capture_governance(note, context.source_channel.as_str());
            vec![crate::services::twin_events::note_changed_draft(
                &note.id,
                change,
                after_digest,
                evidence_digest,
                note.updated_at,
                context.source_channel.clone(),
                governance,
            )
            .map_err(anyhow::Error::msg)?]
        } else {
            Vec::new()
        };
        Ok(crate::services::twin_events::MutationPlan::new(
            crate::models::twin_event::CausalStream::SyncEligible,
            context.source_channel,
            targets,
            drafts,
        ))
    }

    fn persist_note_change(
        &self,
        note: &Note,
        old_path: Option<&Path>,
        _change: crate::models::twin_event::NoteChangeKind,
        _context: NoteMutationContext,
    ) -> Result<()> {
        if !self.event_recorder.is_noop() {
            anyhow::bail!("coordinated note persistence requires a locked planner");
        }
        let (relative_path, _) = Self::canonical_serialized_note_bytes(note)?;
        self.write_note_file(note)?;
        if let Some(old_path) = old_path {
            let new_path = self.resolve_vault_relative_path(&relative_path)?;
            if old_path != new_path && old_path.exists() {
                std::fs::remove_file(old_path).with_context(|| {
                    format!(
                        "Failed to remove original note after move: {}",
                        old_path.display()
                    )
                })?;
            }
        }
        Ok(())
    }

    fn persist_note_delete(
        &self,
        note: &Note,
        path: &Path,
        _context: NoteMutationContext,
    ) -> Result<()> {
        if !self.event_recorder.is_noop() {
            anyhow::bail!("coordinated note deletion requires a locked planner");
        }
        std::fs::remove_file(path)
            .with_context(|| format!("Failed to delete note: {}", note.id))?;
        Ok(())
    }

    fn plan_note_delete(
        &self,
        note: &Note,
        path: &Path,
        context: NoteMutationContext,
    ) -> Result<crate::services::twin_events::MutationPlan> {
        let before_bytes = std::fs::read(path)?;
        let before_digest = crate::services::twin_events::digest_bytes(&before_bytes);
        let key = path
            .strip_prefix(&self.vault_path)
            .map_err(|_| anyhow::anyhow!("note path escaped the vault"))?
            .to_string_lossy();
        let drafts = if context.origin == crate::services::twin_events::MutationOrigin::Local
            && context.capture_event
        {
            vec![crate::services::twin_events::note_changed_draft(
                &note.id,
                crate::models::twin_event::NoteChangeKind::Deleted,
                before_digest.clone(),
                before_digest,
                Utc::now(),
                context.source_channel.clone(),
                note_capture_governance(note, context.source_channel.as_str()),
            )
            .map_err(anyhow::Error::msg)?]
        } else {
            Vec::new()
        };
        let mut targets = vec![crate::services::twin_events::TargetMutation::tombstone(
            crate::services::twin_events::TargetKind::Markdown,
            normalize_relative_path_for_output(&key),
        )];
        targets.push(crate::services::twin_events::TargetMutation::tombstone(
            crate::services::twin_events::TargetKind::OverlayJson,
            format!("{}.json", note.id),
        ));
        Ok(crate::services::twin_events::MutationPlan::new(
            crate::models::twin_event::CausalStream::SyncEligible,
            context.source_channel,
            targets,
            drafts,
        ))
    }

    fn write_note_file(&self, note: &Note) -> Result<()> {
        let (relative_path, file_content) = Self::canonical_serialized_note_bytes(note)?;
        let path = self.resolve_vault_relative_path(&relative_path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        write_atomic(&path, &file_content)
            .with_context(|| format!("Failed to write note: {}", path.display()))?;

        Ok(())
    }

    pub(crate) fn canonical_serialized_note_bytes(note: &Note) -> Result<(String, Vec<u8>)> {
        let relative_path = if note.relative_path.trim().is_empty() {
            format!("{}.md", note.id)
        } else {
            normalize_note_relative_path(&note.relative_path)?
        };

        // If the note's original frontmatter couldn't be parsed on read and the
        // caller hasn't explicitly replaced it (see `update_note`, which clears
        // `frontmatter_raw_fallback` on any explicit frontmatter-field edit), re-emit
        // the original raw block byte-for-byte rather than serializing the (defaulted)
        // `NoteFrontmatter` struct. This is what prevents a content-only save from
        // silently destroying unparsable frontmatter.
        if let Some(raw_frontmatter) = &note.frontmatter_raw_fallback {
            log::warn!(
                "Writing note '{}' with its original unparsable frontmatter preserved verbatim",
                note.id
            );
            let file_content = format!("---\n{}\n---\n\n{}", raw_frontmatter.trim(), note.content);
            return Ok((relative_path, file_content.into_bytes()));
        }

        let mut frontmatter = serde_yaml::Mapping::new();
        frontmatter.insert(
            serde_yaml::Value::String("note_id".to_string()),
            serde_yaml::Value::String(note.id.clone()),
        );
        frontmatter.insert(
            serde_yaml::Value::String("title".to_string()),
            serde_yaml::Value::String(note.title.clone()),
        );
        frontmatter.insert(
            serde_yaml::Value::String("aliases".to_string()),
            serde_yaml::Value::Sequence(
                note.aliases
                    .iter()
                    .map(|alias| serde_yaml::Value::String(alias.clone()))
                    .collect(),
            ),
        );
        frontmatter.insert(
            serde_yaml::Value::String("status".to_string()),
            serde_yaml::Value::String(note.status.to_string()),
        );
        frontmatter.insert(
            serde_yaml::Value::String("tags".to_string()),
            serde_yaml::Value::Sequence(
                note.tags
                    .iter()
                    .map(|tag| serde_yaml::Value::String(tag.clone()))
                    .collect(),
            ),
        );
        frontmatter.insert(
            serde_yaml::Value::String("schema_version".to_string()),
            serde_yaml::Value::Number(note.schema_version.into()),
        );
        if let Some(migration_source) = &note.migration_source {
            frontmatter.insert(
                serde_yaml::Value::String("migration_source".to_string()),
                serde_yaml::Value::String(migration_source.clone()),
            );
        }
        if note.optimizer_managed {
            frontmatter.insert(
                serde_yaml::Value::String("optimizer_managed".to_string()),
                serde_yaml::Value::Bool(true),
            );
        }
        frontmatter.insert(
            serde_yaml::Value::String("created_at".to_string()),
            serde_yaml::Value::String(note.created_at.to_rfc3339()),
        );
        frontmatter.insert(
            serde_yaml::Value::String("updated_at".to_string()),
            serde_yaml::Value::String(note.updated_at.to_rfc3339()),
        );

        let mut properties = note.properties.iter().collect::<Vec<_>>();
        properties.sort_by(|(left, _), (right, _)| left.cmp(right));
        for (key, value) in properties {
            if matches!(
                key.as_str(),
                "note_id"
                    | "title"
                    | "aliases"
                    | "status"
                    | "tags"
                    | "schema_version"
                    | "migration_source"
                    | "optimizer_managed"
                    | "created_at"
                    | "updated_at"
            ) {
                continue;
            }
            if let Ok(yaml_value) = serde_yaml::to_value(value) {
                frontmatter.insert(serde_yaml::Value::String(key.clone()), yaml_value);
            }
        }

        let yaml = serde_yaml::to_string(&frontmatter)?;
        let file_content = format!("---\n{}---\n\n{}", yaml, note.content);
        Ok((relative_path, file_content.into_bytes()))
    }

    fn make_unique_relative_path(&self, preferred_path: &str) -> String {
        self.make_unique_relative_path_with_reserved(preferred_path, &HashSet::new())
    }

    fn make_unique_relative_path_with_reserved(
        &self,
        preferred_path: &str,
        reserved_paths: &HashSet<String>,
    ) -> String {
        let normalized = normalize_note_relative_path(preferred_path).unwrap_or_else(|_| {
            let filename = Path::new(preferred_path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("note.md");
            filename.to_string()
        });

        if !self.vault_path.join(&normalized).exists()
            && !reserved_paths.contains(&normalize_lookup_key(&normalized))
        {
            return normalized;
        }

        let path = Path::new(&normalized);
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("note");
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("md");
        let parent = path.parent().and_then(|value| value.to_str()).unwrap_or("");

        let mut counter = 1;
        loop {
            let filename = format!("{}-{}.{}", stem, counter, ext);
            let candidate = if parent.is_empty() {
                filename.clone()
            } else {
                format!(
                    "{}/{}",
                    normalize_relative_path_for_output(parent),
                    filename
                )
            };
            if !self.vault_path.join(&candidate).exists()
                && !reserved_paths.contains(&normalize_lookup_key(&candidate))
            {
                return candidate;
            }
            counter += 1;
        }
    }

    fn is_reserved_program_path(&self, path: &Path) -> bool {
        let Some(relative) = path
            .strip_prefix(&self.vault_path)
            .ok()
            .and_then(|p| p.to_str())
        else {
            return false;
        };
        normalize_relative_path_for_output(relative).eq_ignore_ascii_case("_grafyn/program.md")
    }
}

#[cfg(test)]
#[path = "knowledge_store_tests.rs"]
mod tests;
