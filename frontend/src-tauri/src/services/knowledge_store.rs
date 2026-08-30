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

/// Service for managing markdown notes with YAML frontmatter and migration overlays.
#[derive(Clone)]
pub struct KnowledgeStore {
    vault_path: PathBuf,
    overlay_notes_dir: PathBuf,
    /// In-memory cache of note metadata, kept in sync with disk.
    meta_cache: Vec<NoteMeta>,
    path_index: HashMap<String, PathBuf>,
    title_index: HashMap<String, String>,
    alias_index: HashMap<String, String>,
    relative_path_index: HashMap<String, String>,
    event_recorder: Arc<dyn crate::services::twin_events::EventRecorder>,
    last_mutation_commit:
        Arc<std::sync::Mutex<Option<crate::services::twin_events::MutationCommit>>>,
}

pub(crate) struct OptimizerNoteSnapshot {
    pub note: Note,
    pub markdown_precondition: OptimizerMarkdownPrecondition,
    pub overlay_value: Option<Value>,
    pub overlay_digest: Option<crate::models::twin_event::ContentDigest>,
}

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

        let mut store = Self {
            vault_path,
            overlay_notes_dir,
            meta_cache: Vec::new(),
            path_index: HashMap::new(),
            title_index: HashMap::new(),
            alias_index: HashMap::new(),
            relative_path_index: HashMap::new(),
            event_recorder,
            last_mutation_commit: Arc::new(std::sync::Mutex::new(None)),
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
        self.event_recorder
            .retarget_markdown_root(&vault_path)
            .map_err(anyhow::Error::new)?;
        self.vault_path = vault_path;
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
        self.refresh_cache();
        Ok(())
    }

    pub fn vault_path(&self) -> &std::path::Path {
        &self.vault_path
    }

    fn remember_mutation_commit(&self, commit: &crate::services::twin_events::MutationCommit) {
        if let Ok(mut slot) = self.last_mutation_commit.lock() {
            *slot = Some(commit.clone());
        }
    }

    fn remember_commit_result(
        &self,
        result: Result<
            crate::services::twin_events::MutationCommit,
            crate::services::twin_events::MutationError,
        >,
    ) -> Result<
        crate::services::twin_events::MutationCommit,
        crate::services::twin_events::MutationError,
    > {
        if let Ok(commit) = &result {
            self.remember_mutation_commit(commit);
        }
        result
    }

    pub(crate) fn clear_last_mutation_commit(&self) {
        if let Ok(mut slot) = self.last_mutation_commit.lock() {
            *slot = None;
        }
    }

    pub(crate) fn take_last_mutation_commit(
        &self,
    ) -> Option<crate::services::twin_events::MutationCommit> {
        self.last_mutation_commit
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
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

    pub fn import_note_container(
        &mut self,
        creates: Vec<NoteCreate>,
        container_id: &str,
        source_bytes: &[u8],
    ) -> Result<Vec<Note>> {
        if creates.is_empty() || creates.len() > 63 {
            anyhow::bail!("an import container must persist 1..=63 notes");
        }
        if self.event_recorder.is_noop() {
            return creates
                .into_iter()
                .map(|create| self.create_note_from_source(create, "import"))
                .collect();
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
        let commit_result = recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut planner,
        );
        if let Err(error) = self.remember_commit_result(commit_result) {
            self.refresh_cache();
            return Err(anyhow::Error::new(error));
        }
        self.refresh_cache();
        return_ids
            .expect("import planner returned note IDs")
            .iter()
            .map(|note_id| self.get_note(note_id))
            .collect()
    }

    fn create_note_with_context(
        &mut self,
        create: NoteCreate,
        context: NoteMutationContext,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        if !self.event_recorder.is_noop() {
            return self.create_note_with_locked_plan(create, context, expected);
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
            let now = Utc::now();
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
            if let Some(expected) = expected.clone() {
                plan = plan.expecting_authority(expected);
            }
            planned_id = Some(id);
            Ok(Some(plan))
        };
        let result = recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut planner,
        );
        self.refresh_cache();
        let commit = self
            .remember_commit_result(result)
            .map_err(anyhow::Error::new)?;
        Ok((
            self.get_note(planned_id.as_deref().expect("planner returned a note ID"))?,
            commit,
        ))
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
        let commit = self
            .remember_commit_result(result)
            .map_err(anyhow::Error::new)?;
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

    pub(crate) fn materialize_note_update_at(
        &self,
        current: &Note,
        update: NoteUpdate,
        updated_at: chrono::DateTime<Utc>,
    ) -> Result<Note> {
        let mut note = current.clone();
        self.apply_note_update(&mut note, update, false)?;
        note.updated_at = updated_at;
        Ok(note)
    }

    pub(crate) fn serialized_note_bytes(&self, note: &Note) -> Result<(String, Vec<u8>)> {
        Self::canonical_serialized_note_bytes(note)
    }

    pub(crate) fn recover_coordinated_mutations(&self) -> Result<usize> {
        self.event_recorder
            .recover_pending_mutations()
            .map_err(anyhow::Error::new)
    }

    pub(crate) fn optimizer_markdown_snapshot(
        &self,
        relative_path: &str,
    ) -> Result<Option<(Note, OptimizerMarkdownPrecondition)>> {
        let normalized = normalize_note_relative_path(relative_path)?;
        let root = crate::services::twin_events::AnchoredRoot::open(&self.vault_path)
            .map_err(anyhow::Error::new)?;
        let Some(bytes) = root
            .read_bounded(
                &normalized,
                crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES,
            )
            .map_err(anyhow::Error::new)?
        else {
            return Ok(None);
        };
        let content = std::str::from_utf8(&bytes).context("optimizer Markdown is not UTF-8")?;
        let path = self.resolve_vault_relative_path(&normalized)?;
        let note = self.parse_note_content_without_overlay(&path, content, None)?;
        Ok(Some((
            note,
            OptimizerMarkdownPrecondition {
                relative_path: normalized,
                expected_digest: crate::services::twin_events::digest_bytes(&bytes),
                root: Arc::new(root),
            },
        )))
    }

    pub(crate) fn optimizer_note_snapshot(
        &self,
        note_id: &str,
    ) -> Result<Option<OptimizerNoteSnapshot>> {
        Self::validate_note_id(note_id)?;
        let path = self.note_path(note_id)?;
        let relative_path = path
            .strip_prefix(&self.vault_path)
            .map_err(|_| anyhow::anyhow!("optimizer note path escaped the vault"))?
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("optimizer note path is not UTF-8"))?;
        let Some((mut note, markdown_precondition)) =
            self.optimizer_markdown_snapshot(relative_path)?
        else {
            return Ok(None);
        };
        if note.id != note_id {
            anyhow::bail!("optimizer note identity changed before snapshot");
        }
        let (overlay_value, overlay_digest) = self.optimizer_overlay_snapshot(note_id)?;
        if let Some(value) = overlay_value.as_ref() {
            let overlay: OverlayNoteData = serde_json::from_value(value.clone())
                .context("invalid optimizer overlay source")?;
            Self::merge_overlay_data(
                &mut note,
                overlay,
                markdown_precondition.relative_path(),
                markdown_precondition.expected_digest(),
            );
        }
        Ok(Some(OptimizerNoteSnapshot {
            note,
            markdown_precondition,
            overlay_value,
            overlay_digest,
        }))
    }

    pub(crate) fn optimizer_markdown_precondition(
        &self,
        relative_path: &str,
        expected_digest: crate::models::twin_event::ContentDigest,
    ) -> Result<OptimizerMarkdownPrecondition> {
        let relative_path = normalize_note_relative_path(relative_path)?;
        let root = crate::services::twin_events::AnchoredRoot::open(&self.vault_path)
            .map_err(anyhow::Error::new)?;
        Ok(OptimizerMarkdownPrecondition {
            relative_path,
            expected_digest,
            root: Arc::new(root),
        })
    }

    pub(crate) fn optimizer_overlay_snapshot(
        &self,
        note_id: &str,
    ) -> Result<(
        Option<serde_json::Value>,
        Option<crate::models::twin_event::ContentDigest>,
    )> {
        Self::validate_note_id(note_id)?;
        let data_root = self
            .overlay_notes_dir
            .ancestors()
            .nth(3)
            .ok_or_else(|| anyhow::anyhow!("optimizer overlay root is invalid"))?;
        let root = crate::services::twin_events::AnchoredRoot::open(data_root)
            .map_err(anyhow::Error::new)?;
        let Some(bytes) = root
            .read_bounded(
                &format!("vault_migration/overlay/notes/{note_id}.json"),
                crate::services::twin_events::MAX_MARKDOWN_TWIN_BYTES,
            )
            .map_err(anyhow::Error::new)?
        else {
            return Ok((None, None));
        };
        let digest = crate::services::twin_events::digest_bytes(&bytes);
        let value = serde_json::from_slice(&bytes).context("invalid optimizer overlay source")?;
        Ok((Some(value), Some(digest)))
    }

    pub(crate) fn replace_note_exact_expecting_authority(
        &mut self,
        id: &str,
        exact: Note,
        source: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        let snapshot = self
            .optimizer_note_snapshot(id)?
            .ok_or_else(|| anyhow::anyhow!("optimizer source note does not exist"))?;
        self.replace_note_exact_expecting_authority_with_hooks(
            id,
            snapshot.note,
            snapshot.markdown_precondition.expected_digest().clone(),
            exact,
            source,
            expected,
            &mut |_| Ok(()),
            &mut |_| Ok(()),
        )
    }

    pub(crate) fn replace_note_exact_expecting_authority_with_hooks(
        &mut self,
        id: &str,
        before: Note,
        before_digest: crate::models::twin_event::ContentDigest,
        exact: Note,
        source: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
        prepared: &mut dyn FnMut(
            &crate::services::twin_events::MutationIntentV1,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
        committed: &mut dyn FnMut(
            &crate::services::twin_events::MutationCommit,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
    ) -> Result<(Note, crate::services::twin_events::MutationCommit)> {
        Self::validate_note_id(id)?;
        if exact.id != id {
            anyhow::bail!("exact replacement note identity does not match target");
        }
        if before.id != id || before.relative_path != exact.relative_path {
            anyhow::bail!("exact replacement source does not match target");
        }
        if self.event_recorder.is_noop() {
            self.write_note_file(&exact)?;
            self.cache_exact_note(&exact)?;
            return Ok((
                exact,
                crate::services::twin_events::MutationCommit {
                    mutation_id: None,
                    events: Vec::new(),
                    authority_token: None,
                    postcommit_warning: false,
                },
            ));
        }
        let recorder = self.event_recorder.clone();
        let source_channel =
            crate::models::twin_event::SourceChannel::parse(source).map_err(anyhow::Error::msg)?;
        let exact_for_plan = exact.clone();
        let mut planner = || {
            let (relative_path, exact_bytes) =
                Self::canonical_serialized_note_bytes(&exact_for_plan).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            let after_digest = crate::services::twin_events::digest_bytes(&exact_bytes);
            if before_digest == after_digest {
                return Ok(None);
            }
            let target = crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::Markdown,
                normalize_relative_path_for_output(&relative_path),
                String::from_utf8(exact_bytes).expect("Markdown serialization is UTF-8"),
            )
            .expecting(crate::services::twin_events::BeforeImage::Sha256(
                before_digest.clone(),
            ));
            let draft = crate::services::twin_events::note_changed_draft(
                &exact_for_plan.id,
                crate::models::twin_event::NoteChangeKind::Updated,
                after_digest,
                before_digest.clone(),
                exact_for_plan.updated_at,
                source_channel.clone(),
                note_capture_governance(&exact_for_plan, source_channel.as_str()),
            )
            .map_err(crate::services::twin_events::MutationError::Invalid)?;
            let plan = crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::SyncEligible,
                source_channel.clone(),
                vec![target],
                vec![draft],
            )
            .expecting_authority(expected.clone())
            .retaining_commit_receipt();
            Ok(Some(plan))
        };
        let result = recorder.commit_planned_mutation_with_hooks(
            crate::services::twin_events::MutationOrigin::Local,
            &mut planner,
            prepared,
            committed,
        );
        let commit = self
            .remember_commit_result(result)
            .map_err(anyhow::Error::new)?;
        self.cache_exact_note(&exact)?;
        Ok((exact, commit))
    }

    /// Administrative variant of `update_note` that preserves the note's existing
    /// `updated_at` timestamp. See `backfill_legacy_grafyn_notes` for the motivating
    /// case: schema/provenance bookkeeping writes should not bump recency ranking.
    pub(crate) fn update_note_preserving_timestamp(
        &mut self,
        id: &str,
        update: NoteUpdate,
    ) -> Result<Note> {
        self.update_note_with_options(
            id,
            update,
            false,
            NoteMutationContext::local("migration")?,
            None,
        )
        .map(|(note, _)| note)
    }

    pub(crate) fn restore_note_bytes_from_source(
        &mut self,
        relative_path: &str,
        bytes: &[u8],
        source: &str,
    ) -> Result<Note> {
        let relative_path = normalize_note_relative_path(relative_path)?;
        let after = std::str::from_utf8(bytes)
            .with_context(|| format!("restored Markdown is not UTF-8: {relative_path}"))?
            .to_string();
        if self.event_recorder.is_noop() {
            let path = self.resolve_vault_relative_path(&relative_path)?;
            let note = self
                .find_note_by_relative_path(&relative_path)?
                .ok_or_else(|| {
                    anyhow::anyhow!("note to restore is not indexed: {relative_path}")
                })?;
            let before = std::fs::read(&path)?;
            if before == bytes {
                return Ok(note);
            }
            write_atomic(&path, bytes)?;
            self.refresh_cache();
            self.get_note(&note.id)
        } else {
            let recorder = self.event_recorder.clone();
            let source = source.to_string();
            let mut restored_note_id = None;
            let mut planner = || {
                self.refresh_cache();
                let path = self
                    .resolve_vault_relative_path(&relative_path)
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                let note = self
                    .find_note_by_relative_path(&relative_path)
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?
                    .ok_or_else(|| {
                        crate::services::twin_events::MutationError::Invalid(format!(
                            "note to restore is not indexed: {relative_path}"
                        ))
                    })?;
                restored_note_id = Some(note.id.clone());
                let before = std::fs::read(&path).map_err(|error| {
                    crate::services::twin_events::MutationError::Io(error.to_string())
                })?;
                if before == bytes {
                    return Ok(None);
                }
                let source_channel = crate::models::twin_event::SourceChannel::parse(&source)
                    .map_err(crate::services::twin_events::MutationError::Invalid)?;
                let draft = crate::services::twin_events::note_changed_draft(
                    &note.id,
                    crate::models::twin_event::NoteChangeKind::Updated,
                    crate::services::twin_events::digest_bytes(bytes),
                    crate::services::twin_events::digest_bytes(&before),
                    Utc::now(),
                    source_channel.clone(),
                    note_capture_governance(&note, &source),
                )
                .map_err(crate::services::twin_events::MutationError::Invalid)?;
                Ok(Some(crate::services::twin_events::MutationPlan::new(
                    crate::models::twin_event::CausalStream::SyncEligible,
                    source_channel,
                    vec![crate::services::twin_events::TargetMutation::put(
                        crate::services::twin_events::TargetKind::Markdown,
                        relative_path.clone(),
                        after.clone(),
                    )],
                    vec![draft],
                )))
            };
            let commit_result = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            );
            if let Err(error) = self.remember_commit_result(commit_result) {
                self.refresh_cache();
                return Err(anyhow::Error::new(error));
            }
            self.refresh_cache();
            self.get_note(
                restored_note_id
                    .as_deref()
                    .expect("restore planner returned a note ID"),
            )
        }
    }

    pub(crate) fn put_vault_file_target_only_expected(
        &mut self,
        relative_path: &str,
        bytes: &[u8],
        source: &str,
        expected_before: Option<crate::services::twin_events::BeforeImage>,
    ) -> Result<()> {
        let relative_path = normalize_note_relative_path(relative_path)?;
        let after = std::str::from_utf8(bytes)
            .with_context(|| format!("Markdown target is not UTF-8: {relative_path}"))?
            .to_string();
        if self.event_recorder.is_noop() {
            let path = self.resolve_vault_relative_path(&relative_path)?;
            if let Some(expected) = expected_before.as_ref() {
                let current = before_image_for_path(&path)?;
                if current
                    == crate::services::twin_events::BeforeImage::Sha256(
                        crate::services::twin_events::digest_bytes(bytes),
                    )
                {
                    return Ok(());
                }
                if &current != expected {
                    anyhow::bail!("conditional Markdown target changed: {relative_path}");
                }
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            write_atomic(&path, bytes)?;
            self.refresh_cache();
            return Ok(());
        }
        let recorder = self.event_recorder.clone();
        let source_channel =
            crate::models::twin_event::SourceChannel::parse(source).map_err(anyhow::Error::msg)?;
        let target = crate::services::twin_events::TargetMutation::put(
            crate::services::twin_events::TargetKind::Markdown,
            relative_path,
            after,
        );
        let target = match expected_before {
            Some(expected) => target.expecting(expected),
            None => target,
        };
        let mut plan = Some(crate::services::twin_events::MutationPlan::new(
            crate::models::twin_event::CausalStream::LocalOnly,
            source_channel,
            vec![target],
            Vec::new(),
        ));
        let result = recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut || Ok(plan.take()),
        );
        self.refresh_cache();
        self.remember_commit_result(result)
            .map(|_| ())
            .map_err(anyhow::Error::new)
    }

    pub(crate) fn delete_vault_file_target_only(
        &mut self,
        relative_path: &str,
        source: &str,
    ) -> Result<()> {
        self.delete_vault_file_target_only_expected(relative_path, source, None)
    }

    pub(crate) fn delete_vault_file_target_only_expected(
        &mut self,
        relative_path: &str,
        source: &str,
        expected_before: Option<crate::services::twin_events::BeforeImage>,
    ) -> Result<()> {
        let relative_path = normalize_note_relative_path(relative_path)?;
        if self.event_recorder.is_noop() {
            let path = self.resolve_vault_relative_path(&relative_path)?;
            if let Some(expected) = expected_before.as_ref() {
                let current = before_image_for_path(&path)?;
                if current == crate::services::twin_events::BeforeImage::Absent {
                    return Ok(());
                }
                if &current != expected {
                    anyhow::bail!("conditional Markdown target changed: {relative_path}");
                }
            }
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            self.refresh_cache();
            return Ok(());
        }
        let recorder = self.event_recorder.clone();
        let source_channel =
            crate::models::twin_event::SourceChannel::parse(source).map_err(anyhow::Error::msg)?;
        let target = crate::services::twin_events::TargetMutation::tombstone(
            crate::services::twin_events::TargetKind::Markdown,
            relative_path,
        );
        let target = match expected_before {
            Some(expected) => target.expecting(expected),
            None => target,
        };
        let mut plan = Some(crate::services::twin_events::MutationPlan::new(
            crate::models::twin_event::CausalStream::LocalOnly,
            source_channel,
            vec![target],
            Vec::new(),
        ));
        let result = recorder.commit_planned_mutation(
            crate::services::twin_events::MutationOrigin::Local,
            &mut || Ok(plan.take()),
        );
        self.refresh_cache();
        self.remember_commit_result(result)
            .map(|_| ())
            .map_err(anyhow::Error::new)
    }

    pub(crate) fn validate_vault_file_target(
        &mut self,
        relative_path: &str,
        expected: crate::services::twin_events::BeforeImage,
    ) -> Result<()> {
        let relative_path = normalize_note_relative_path(relative_path)?;
        let recorder = self.event_recorder.clone();
        let mut planner = || {
            let path = self
                .resolve_vault_relative_path(&relative_path)
                .map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
            let current = before_image_for_path(&path).map_err(|error| {
                crate::services::twin_events::MutationError::Io(error.to_string())
            })?;
            if current != expected {
                return Err(
                    crate::services::twin_events::MutationError::RecoveryConflict(format!(
                        "conditional Markdown target changed: {relative_path}"
                    )),
                );
            }
            Ok(None)
        };
        recorder
            .commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            )
            .map(|_| ())
            .map_err(anyhow::Error::new)
    }

    pub fn delete_note(&mut self, id: &str) -> Result<()> {
        self.delete_note_from_source(id, "note_editor")
    }

    pub fn delete_note_from_source(&mut self, id: &str, source: &str) -> Result<()> {
        self.delete_note_from_source_with_commit(id, source)
            .map(|_| ())
    }

    pub(crate) fn delete_note_from_source_with_commit(
        &mut self,
        id: &str,
        source: &str,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.delete_note_with_context(id, source, None)
    }

    pub(crate) fn delete_note_expecting_authority(
        &mut self,
        id: &str,
        source: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.delete_note_with_context(id, source, Some(expected))
    }

    fn delete_note_with_context(
        &mut self,
        id: &str,
        source: &str,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        Self::validate_note_id(id)?;
        let context = NoteMutationContext::local(source)?;
        let commit = if self.event_recorder.is_noop() {
            let path = self.note_path(id)?;
            let note = self.get_note(id)?;
            self.persist_note_delete(&note, &path, context)?;
            crate::services::twin_events::MutationCommit {
                mutation_id: None,
                events: Vec::new(),
                authority_token: None,
                postcommit_warning: false,
            }
        } else {
            let recorder = self.event_recorder.clone();
            let note_id = id.to_string();
            let mut planner = || {
                self.refresh_cache();
                let note = self.get_note(&note_id).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                let path = self.note_path(&note_id).map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                let mut plan = self
                    .plan_note_delete(&note, &path, context.clone())
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })?;
                if let Some(expected) = expected.clone() {
                    plan = plan.expecting_authority(expected);
                }
                Ok(Some(plan))
            };
            let result = recorder.commit_planned_mutation(
                crate::services::twin_events::MutationOrigin::Local,
                &mut planner,
            );
            match self.remember_commit_result(result) {
                Ok(commit) => commit,
                Err(error) => {
                    self.refresh_cache();
                    return Err(anyhow::Error::new(error));
                }
            }
        };
        if self.event_recorder.is_noop() {
            let overlay_path = self.overlay_path(id);
            if overlay_path.exists() {
                let _ = std::fs::remove_file(&overlay_path);
            }
        }
        self.refresh_cache();
        Ok(commit)
    }

    pub fn overlay_path(&self, note_id: &str) -> PathBuf {
        self.overlay_notes_dir.join(format!("{}.json", note_id))
    }

    pub fn write_overlay(&self, note_id: &str, overlay: &serde_json::Value) -> Result<()> {
        self.write_overlay_from_source(note_id, overlay, "vault_optimizer")
    }

    pub fn write_overlay_from_source(
        &self,
        note_id: &str,
        overlay: &serde_json::Value,
        source: &str,
    ) -> Result<()> {
        self.write_overlay_from_source_with_authority(note_id, overlay, source, None)
            .map(|_| ())
    }

    pub(crate) fn write_overlay_from_source_expecting_authority(
        &self,
        note_id: &str,
        overlay: &serde_json::Value,
        source: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.write_overlay_from_source_with_authority(note_id, overlay, source, Some(expected))
    }

    pub(crate) fn write_overlay_from_source_with_authority(
        &self,
        note_id: &str,
        overlay: &serde_json::Value,
        source: &str,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.write_overlay_from_source_with_authority_and_hooks(
            note_id,
            overlay,
            source,
            expected,
            &mut |_| Ok(()),
            &mut |_| Ok(()),
            false,
            None,
        )
    }

    pub(crate) fn write_overlay_from_source_with_authority_and_hooks(
        &self,
        note_id: &str,
        overlay: &serde_json::Value,
        source: &str,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
        prepared: &mut dyn FnMut(
            &crate::services::twin_events::MutationIntentV1,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
        committed: &mut dyn FnMut(
            &crate::services::twin_events::MutationCommit,
        )
            -> Result<(), crate::services::twin_events::MutationError>,
        retain_commit_receipt: bool,
        source_guard: Option<crate::services::twin_events::TargetMutation>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        Self::validate_note_id(note_id)?;
        let content = serde_json::to_string_pretty(overlay)?;
        if !self.event_recorder.is_noop() {
            let source_channel = crate::models::twin_event::SourceChannel::parse(source)
                .map_err(anyhow::Error::msg)?;
            let target_key = format!("{note_id}.json");
            if source_guard.is_some() && !retain_commit_receipt {
                anyhow::bail!("optimizer source guards require retained schema-3 receipts");
            }
            let mut targets = Vec::with_capacity(1 + usize::from(source_guard.is_some()));
            if let Some(source_guard) = source_guard {
                targets.push(source_guard);
            }
            targets.push(crate::services::twin_events::TargetMutation::put(
                crate::services::twin_events::TargetKind::OverlayJson,
                target_key,
                content,
            ));
            let mut mutation_plan = crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::LocalOnly,
                source_channel,
                targets,
                Vec::new(),
            );
            if let Some(expected) = expected {
                mutation_plan = mutation_plan.expecting_authority(expected);
            }
            if retain_commit_receipt {
                mutation_plan = mutation_plan.retaining_commit_receipt();
            }
            let mut plan = Some(mutation_plan);
            let commit = self
                .event_recorder
                .commit_planned_mutation_with_hooks(
                    crate::services::twin_events::MutationOrigin::Local,
                    &mut || Ok(plan.take()),
                    prepared,
                    committed,
                )
                .map_err(anyhow::Error::new)?;
            self.remember_mutation_commit(&commit);
            return Ok(commit);
        }
        if let Some(parent) = self.overlay_path(note_id).parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(&self.overlay_path(note_id), content.as_bytes())
            .with_context(|| format!("Failed to write overlay for '{}'", note_id))?;
        Ok(crate::services::twin_events::MutationCommit {
            mutation_id: None,
            events: Vec::new(),
            authority_token: None,
            postcommit_warning: false,
        })
    }

    pub fn delete_overlay(&self, note_id: &str) -> Result<()> {
        self.delete_overlay_from_source(note_id, "vault_optimizer")
    }

    pub fn delete_overlay_from_source(&self, note_id: &str, source: &str) -> Result<()> {
        self.delete_overlay_from_source_with_authority(note_id, source, None)
            .map(|_| ())
    }

    pub(crate) fn delete_overlay_from_source_expecting_authority(
        &self,
        note_id: &str,
        source: &str,
        expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        self.delete_overlay_from_source_with_authority(note_id, source, Some(expected))
    }

    pub(crate) fn delete_overlay_from_source_with_authority(
        &self,
        note_id: &str,
        source: &str,
        expected: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<crate::services::twin_events::MutationCommit> {
        Self::validate_note_id(note_id)?;
        if !self.event_recorder.is_noop() {
            let source_channel = crate::models::twin_event::SourceChannel::parse(source)
                .map_err(anyhow::Error::msg)?;
            let target_key = format!("{note_id}.json");
            let mut mutation_plan = crate::services::twin_events::MutationPlan::new(
                crate::models::twin_event::CausalStream::LocalOnly,
                source_channel,
                vec![crate::services::twin_events::TargetMutation::tombstone(
                    crate::services::twin_events::TargetKind::OverlayJson,
                    target_key,
                )],
                Vec::new(),
            );
            if let Some(expected) = expected {
                mutation_plan = mutation_plan.expecting_authority(expected);
            }
            let mut plan = Some(mutation_plan);
            let commit = self
                .event_recorder
                .commit_planned_mutation(
                    crate::services::twin_events::MutationOrigin::Local,
                    &mut || Ok(plan.take()),
                )
                .map_err(anyhow::Error::new)?;
            self.remember_mutation_commit(&commit);
            return Ok(commit);
        }
        let path = self.overlay_path(note_id);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(crate::services::twin_events::MutationCommit {
            mutation_id: None,
            events: Vec::new(),
            authority_token: None,
            postcommit_warning: false,
        })
    }

    pub fn extract_wikilinks(&self, content: &str) -> Vec<String> {
        WIKILINK_REGEX
            .captures_iter(content)
            .filter_map(|cap| cap.get(1).map(|m| m.as_str().trim().to_string()))
            .filter(|value| !value.is_empty())
            .collect()
    }

    pub fn extract_links(&self, content: &str, source_relative_path: &str) -> Vec<ParsedLink> {
        let mut links = self.extract_typed_wikilinks(content);
        links.extend(self.extract_markdown_links(content, source_relative_path));
        links
    }

    /// Extract typed wikilinks with relationship information.
    pub fn extract_typed_wikilinks(&self, content: &str) -> Vec<ParsedLink> {
        TYPED_WIKILINK_REGEX
            .captures_iter(content)
            .filter_map(|cap| {
                let target_title = cap.get(1)?.as_str().trim().to_string();
                if target_title.is_empty() {
                    return None;
                }
                let relation = cap
                    .get(2)
                    .map(|m| RelationType::from_str_lossy(m.as_str()))
                    .unwrap_or(RelationType::Untyped);
                Some(ParsedLink {
                    target_title,
                    target_path: None,
                    relation,
                })
            })
            .collect()
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

        let now = Utc::now();
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
        let aliases = dedupe_strings(
            frontmatter_aliases
                .into_iter()
                .chain(alias_candidates(&title, file_stem)),
        );
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

fn note_update_changes(note: &Note, update: &NoteUpdate) -> Result<bool> {
    if update
        .title
        .as_ref()
        .is_some_and(|value| value != &note.title)
        || update
            .content
            .as_ref()
            .is_some_and(|value| value != &note.content)
        || update
            .aliases
            .as_ref()
            .is_some_and(|value| dedupe_strings(value.clone()) != note.aliases)
        || update
            .status
            .as_ref()
            .is_some_and(|value| value != &note.status)
        || update
            .tags
            .as_ref()
            .is_some_and(|value| dedupe_strings(value.clone()) != note.tags)
        || update
            .schema_version
            .is_some_and(|value| value.max(CURRENT_NOTE_SCHEMA_VERSION) != note.schema_version)
        || update
            .migration_source
            .as_ref()
            .is_some_and(|value| Some(value) != note.migration_source.as_ref())
        || update
            .optimizer_managed
            .is_some_and(|value| value != note.optimizer_managed)
        || update
            .properties
            .as_ref()
            .is_some_and(|value| value != &note.properties)
    {
        return Ok(true);
    }
    if let Some(relative_path) = &update.relative_path {
        return Ok(normalize_note_relative_path(relative_path)? != note.relative_path);
    }
    Ok(false)
}

fn note_capture_governance(
    note: &Note,
    source_channel: &str,
) -> crate::models::twin_event::Governance {
    let mut governance = if source_channel == "import" {
        crate::services::twin_events::imported_capture_governance()
    } else {
        crate::services::twin_events::standard_capture_governance()
    };
    let local_only = note
        .properties
        .get("grafyn_sync")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "local_only")
        || note
            .properties
            .get("private")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let sensitivity = note
        .properties
        .get("sensitivity")
        .and_then(Value::as_str)
        .map(|value| match value {
            "restricted" | "private" => crate::models::twin_event::Sensitivity::Restricted,
            "sensitive" => crate::models::twin_event::Sensitivity::Sensitive,
            _ => crate::models::twin_event::Sensitivity::Standard,
        })
        .unwrap_or(governance.sensitivity.clone());
    if local_only || sensitivity == crate::models::twin_event::Sensitivity::Restricted {
        governance = crate::services::twin_events::local_capture_governance(sensitivity);
    } else {
        governance.sensitivity = sensitivity;
    }
    governance
}

fn normalize_lookup_key(value: &str) -> String {
    value.trim().replace('\\', "/").to_lowercase()
}

fn normalize_relative_path_for_output(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

/// Windows reserved device names — invalid as a file/directory stem
/// regardless of extension (e.g. `con`, `CON.md`, `con.backup.md`).
/// Checked platform-independently: a vault synced across OSes must not
/// contain files that are unopenable on Windows.
const RESERVED_WINDOWS_STEMS: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Returns true if `component` (an id or a single path segment, with or
/// without an extension) is a Windows-reserved device name. The reserved
/// stem is the text before the *first* dot, matched case-insensitively, so
/// `con.backup.md` is still reserved. Windows additionally strips trailing
/// spaces and dots before device-name resolution (`con .md` still reaches
/// the CON device), so the stem is trimmed of those before comparison.
fn is_reserved_windows_component(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    let stem = stem.trim_end_matches([' ', '.']);
    RESERVED_WINDOWS_STEMS
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

/// Belt-and-braces check run after joining a (validated) relative path onto
/// the vault root: confirms the resolved path is still lexically nested
/// under `vault_path`. This is a pure component walk — no filesystem
/// canonicalize, since the target may not exist yet (e.g. a note being
/// created). Catches anything the string-level validators might miss,
/// including Windows drive-relative joins (`PathBuf::join` replaces the
/// base entirely when the argument carries its own drive prefix).
fn ensure_path_within_vault(vault_path: &Path, resolved: &Path) -> Result<()> {
    let remainder = resolved.strip_prefix(vault_path).map_err(|_| {
        anyhow::anyhow!(
            "Resolved note path escapes the vault: {}",
            resolved.display()
        )
    })?;
    for component in remainder.components() {
        match component {
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::ParentDir => {
                anyhow::bail!(
                    "Resolved note path escapes the vault: {}",
                    resolved.display()
                );
            }
            _ => {}
        }
    }
    Ok(())
}

fn normalize_note_relative_path(value: &str) -> Result<String> {
    let normalized = normalize_relative_path_for_output(value)
        .trim_matches('/')
        .to_string();
    if normalized.is_empty() {
        anyhow::bail!("Relative note path cannot be empty");
    }
    if Path::new(&normalized).is_absolute() {
        anyhow::bail!("Absolute note paths are not allowed");
    }
    if normalized.contains(':') {
        anyhow::bail!(
            "Note paths must not contain ':' (drive-relative or alternate-data-stream syntax is not allowed): {}",
            normalized
        );
    }
    for segment in normalized.split('/') {
        if segment.is_empty() || segment == ".." {
            anyhow::bail!("Path traversal is not allowed in note paths");
        }
        if is_reserved_windows_component(segment) {
            anyhow::bail!(
                "Note paths must not use a reserved device name: {}",
                segment
            );
        }
    }
    if normalized.to_lowercase().ends_with(".md") {
        Ok(normalized)
    } else {
        Ok(format!("{}.md", normalized))
    }
}

fn before_image_for_path(path: &Path) -> Result<crate::services::twin_events::BeforeImage> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(crate::services::twin_events::BeforeImage::Sha256(
            crate::services::twin_events::digest_bytes(&bytes),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(crate::services::twin_events::BeforeImage::Absent)
        }
        Err(error) => Err(error.into()),
    }
}

fn slugify(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else if character.is_whitespace()
                || character == '-'
                || character == '_'
                || character == '/'
            {
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

fn humanize_filename(value: &str) -> String {
    value
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn alias_candidates(title: &str, file_stem: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let humanized = humanize_filename(file_stem);
    if !humanized.trim().is_empty() && !humanized.eq_ignore_ascii_case(title.trim()) {
        candidates.push(humanized);
    }
    let compact = file_stem.replace(['-', '_'], "");
    if !compact.is_empty()
        && !compact.eq_ignore_ascii_case(file_stem)
        && !compact.eq_ignore_ascii_case(title)
    {
        candidates.push(compact);
    }
    candidates
}

fn extract_inline_hashtags(content: &str) -> Vec<String> {
    HASHTAG_REGEX
        .captures_iter(content)
        .filter_map(|caps| caps.get(1).map(|value| value.as_str().trim().to_string()))
        .filter(|value| !value.is_empty())
        .collect()
}

fn dedupe_strings<I>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let owned = trimmed.to_string();
        let key = owned.to_lowercase();
        if seen.insert(key) {
            result.push(owned);
        }
    }
    result
}

#[cfg(test)]
#[path = "knowledge_store_tests.rs"]
mod tests;
