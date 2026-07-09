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
}

/// Service for managing markdown notes with YAML frontmatter and migration overlays.
#[derive(Debug, Clone)]
pub struct KnowledgeStore {
    vault_path: PathBuf,
    overlay_notes_dir: PathBuf,
    /// In-memory cache of note metadata, kept in sync with disk.
    meta_cache: Vec<NoteMeta>,
    path_index: HashMap<String, PathBuf>,
    title_index: HashMap<String, String>,
    alias_index: HashMap<String, String>,
    relative_path_index: HashMap<String, String>,
}

impl KnowledgeStore {
    pub fn new(vault_path: PathBuf, data_path: PathBuf) -> Self {
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
        };
        store.refresh_cache();
        store
    }

    /// Update the vault path at runtime (e.g., after settings change).
    pub fn set_vault_path(&mut self, vault_path: PathBuf) {
        if let Err(error) = std::fs::create_dir_all(&vault_path) {
            log::error!(
                "Failed to create vault directory {}: {}",
                vault_path.display(),
                error
            );
        }
        self.vault_path = vault_path;
        self.refresh_cache();
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

        self.write_note_file(&note)?;
        self.refresh_cache();
        self.get_note(&note.id)
    }

    pub fn update_note(&mut self, id: &str, update: NoteUpdate) -> Result<Note> {
        Self::validate_note_id(id)?;
        let mut note = self.get_note(id)?;
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

        note.updated_at = Utc::now();
        note.wikilinks = self.extract_wikilinks(&note.content);
        note.parsed_links = self.extract_links(&note.content, &note.relative_path);

        self.write_note_file(&note)?;
        let new_path = self.resolve_vault_relative_path(&note.relative_path)?;
        if old_path != new_path && old_path.exists() {
            std::fs::remove_file(&old_path).with_context(|| {
                format!(
                    "Failed to remove original note after move: {}",
                    old_path.display()
                )
            })?;
        }

        self.refresh_cache();
        self.get_note(&note.id)
    }

    pub fn delete_note(&mut self, id: &str) -> Result<()> {
        Self::validate_note_id(id)?;
        let path = self.note_path(id)?;
        std::fs::remove_file(&path).with_context(|| format!("Failed to delete note: {}", id))?;
        let overlay_path = self.overlay_path(id);
        if overlay_path.exists() {
            let _ = std::fs::remove_file(&overlay_path);
        }
        self.refresh_cache();
        Ok(())
    }

    pub fn overlay_path(&self, note_id: &str) -> PathBuf {
        self.overlay_notes_dir.join(format!("{}.json", note_id))
    }

    pub fn write_overlay(&self, note_id: &str, overlay: &serde_json::Value) -> Result<()> {
        Self::validate_note_id(note_id)?;
        if let Some(parent) = self.overlay_path(note_id).parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(
            &self.overlay_path(note_id),
            serde_json::to_string_pretty(overlay)?.as_bytes(),
        )
        .with_context(|| format!("Failed to write overlay for '{}'", note_id))?;
        Ok(())
    }

    pub fn delete_overlay(&self, note_id: &str) -> Result<()> {
        let path = self.overlay_path(note_id);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
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
        let slug = slugify(title);
        let base = if slug.is_empty() {
            "note".to_string()
        } else {
            slug
        };
        let mut id = base.clone();
        let mut counter = 1;
        let existing_ids = self
            .meta_cache
            .iter()
            .map(|note| note.id.as_str())
            .collect::<HashSet<_>>();
        while existing_ids.contains(id.as_str()) {
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

        let matter = Matter::<YAML>::new();
        let parsed = matter.parse(&content);

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

        let file_metadata = std::fs::metadata(path)?;
        let file_modified = file_metadata.modified().ok();
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

        let mut note = Note {
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

        self.merge_overlay(&mut note);
        Ok(note)
    }

    fn merge_overlay(&self, note: &mut Note) {
        let overlay_path = self.overlay_path(&note.id);
        let overlay = std::fs::read_to_string(&overlay_path)
            .ok()
            .and_then(|content| serde_json::from_str::<OverlayNoteData>(&content).ok());
        let Some(overlay) = overlay else {
            return;
        };

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

    fn write_note_file(&self, note: &Note) -> Result<()> {
        let relative_path = if note.relative_path.trim().is_empty() {
            format!("{}.md", note.id)
        } else {
            normalize_note_relative_path(&note.relative_path)?
        };
        let path = self.resolve_vault_relative_path(&relative_path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

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
            write_atomic(&path, file_content.as_bytes())
                .with_context(|| format!("Failed to write note: {}", path.display()))?;
            return Ok(());
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

        for (key, value) in &note.properties {
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
        write_atomic(&path, file_content.as_bytes())
            .with_context(|| format!("Failed to write note: {}", path.display()))?;

        Ok(())
    }

    fn make_unique_relative_path(&self, preferred_path: &str) -> String {
        let normalized = normalize_note_relative_path(preferred_path).unwrap_or_else(|_| {
            let filename = Path::new(preferred_path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("note.md");
            filename.to_string()
        });

        if !self.vault_path.join(&normalized).exists() {
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
            if !self.vault_path.join(&candidate).exists() {
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
mod tests {
    use super::*;
    use crate::models::note::NoteStatus;
    use crate::services::atomic_io::assert_no_tmp_siblings;
    use tempfile::tempdir;

    #[test]
    fn note_and_overlay_writes_are_atomic_with_no_tmp_litter() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let note = store
            .create_note(NoteCreate {
                title: "Atomic Adoption".to_string(),
                content: "Body content survives the temp+rename write.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: Default::default(),
                tags: vec!["adoption".to_string()],
                schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: None,
                optimizer_managed: false,
                properties: HashMap::new(),
            })
            .expect("note should be created");

        let note_path = vault_dir.path().join(&note.relative_path);
        let persisted = std::fs::read_to_string(&note_path).expect("note file should exist");
        assert!(persisted.contains("Atomic Adoption"));
        assert!(persisted.contains("Body content survives the temp+rename write."));
        assert_no_tmp_siblings(vault_dir.path());

        store
            .write_overlay(
                &note.id,
                &serde_json::json!({"aliases": ["Adoption Alias"]}),
            )
            .expect("overlay should be written");
        let overlay = std::fs::read_to_string(store.overlay_path(&note.id))
            .expect("overlay file should exist");
        assert!(overlay.contains("Adoption Alias"));
        assert_no_tmp_siblings(&store.overlay_notes_dir);
    }

    #[test]
    fn validate_note_id_rejects_colon_variants() {
        assert!(
            KnowledgeStore::validate_note_id("C:foo").is_err(),
            "drive-relative id must be rejected"
        );
        assert!(
            KnowledgeStore::validate_note_id("foo:bar").is_err(),
            "alternate-data-stream id must be rejected"
        );
    }

    #[test]
    fn validate_note_id_rejects_reserved_device_stems_case_insensitively() {
        for candidate in [
            "con",
            "CON",
            "con.md",
            "CON.backup",
            "prn",
            "PRN.md",
            "aux",
            "AUX",
            "nul",
            "NUL.md",
            "com1",
            "COM1.md",
            "com9",
            "COM9",
            "lpt1",
            "LPT1.md",
            "lpt9",
            "LPT9",
            // Windows strips trailing spaces and dots before device-name
            // resolution, so these variants still reach the device.
            "con ",
            "con .md",
            "CON. .md",
            "nul.",
            "aux . .md",
        ] {
            assert!(
                KnowledgeStore::validate_note_id(candidate).is_err(),
                "expected '{}' to be rejected as a reserved device name",
                candidate
            );
        }
    }

    #[test]
    fn validate_note_id_accepts_normal_unicode_titles() {
        for candidate in [
            "my-note",
            "笔记-notes",
            "my.note.v2",
            "project-plan-2026",
            "console-notes",
            "company",
            // Trailing space on a non-reserved stem must stay accepted:
            // trimmed stem is "console", which is not a device name.
            "console ",
        ] {
            assert!(
                KnowledgeStore::validate_note_id(candidate).is_ok(),
                "expected '{}' to be accepted",
                candidate
            );
        }
    }

    #[test]
    fn normalize_note_relative_path_rejects_colon_variants() {
        assert!(
            normalize_note_relative_path("C:foo").is_err(),
            "drive-relative path must be rejected"
        );
        assert!(
            normalize_note_relative_path("foo:bar.md").is_err(),
            "alternate-data-stream path must be rejected"
        );
        assert!(
            normalize_note_relative_path("sub/c:d.md").is_err(),
            "colon in a nested component must be rejected"
        );
    }

    #[test]
    fn normalize_note_relative_path_rejects_reserved_device_components() {
        for candidate in [
            "con",
            "CON.md",
            "prn.md",
            "aux",
            "nul.md",
            "com1.md",
            "lpt9",
            "con.backup.md",
            "sub/CON/note.md",
            "sub/prn.md/note.md",
            // Trailing spaces/dots are stripped by Windows before device-name
            // resolution, so these still reach the device.
            "con .md",
            "sub/CON. .md",
            "sub/nul ./note.md",
        ] {
            assert!(
                normalize_note_relative_path(candidate).is_err(),
                "expected '{}' to be rejected",
                candidate
            );
        }
    }

    #[test]
    fn normalize_note_relative_path_accepts_normal_unicode_paths() {
        for candidate in [
            "笔记-notes.md",
            "folder/my.note.v2.md",
            "notes/2026/plan.md",
            "console-notes.md",
            "folder/console .md",
        ] {
            assert!(
                normalize_note_relative_path(candidate).is_ok(),
                "expected '{}' to be accepted",
                candidate
            );
        }
    }

    #[test]
    fn ensure_path_within_vault_rejects_parent_dir_escape() {
        let vault_dir = tempdir().expect("vault tempdir");
        let vault_path = vault_dir.path();
        let joined = vault_path.join(Path::new("../../outside.md"));
        assert!(
            ensure_path_within_vault(vault_path, &joined).is_err(),
            "parent-dir escape must be rejected"
        );
    }

    #[cfg(windows)]
    #[test]
    fn ensure_path_within_vault_rejects_drive_relative_escape_on_windows() {
        let vault_dir = tempdir().expect("vault tempdir");
        let vault_path = vault_dir.path();
        // On Windows, PathBuf::join replaces the base entirely when the
        // argument carries its own drive prefix (drive-relative path).
        let joined = vault_path.join("C:secret.md");
        assert!(
            ensure_path_within_vault(vault_path, &joined).is_err(),
            "drive-relative escape must be rejected"
        );
    }

    #[test]
    fn store_rejects_hostile_note_ids_and_paths_end_to_end() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        assert!(store.get_note("C:secret").is_err());
        assert!(store.get_note("con").is_err());
        assert!(store.delete_note("con").is_err());
        assert!(store
            .create_note(NoteCreate {
                title: "Hostile".to_string(),
                content: "x".to_string(),
                relative_path: Some("C:evil.md".to_string()),
                aliases: Vec::new(),
                status: Default::default(),
                tags: Vec::new(),
                schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: None,
                optimizer_managed: false,
                properties: HashMap::new(),
            })
            .is_err());
    }

    /// Frontmatter block with a tab character used as block-sequence indentation,
    /// which yaml-rust2 rejects ("tab cannot be used as indentation"). This is not
    /// well-formed YAML, so `YamlLoader::load_from_str` errors and the gray_matter
    /// engine falls back to `Pod::Null`, which then fails to deserialize into
    /// `NoteFrontmatter` (a non-map `Pod` is an invalid type for the struct,
    /// regardless of field defaults).
    const MALFORMED_FRONTMATTER_NOTE: &str = "---\ntitle: Original Title\ntags:\n\t- alpha\nstatus: canonical\n---\n\nOriginal body content.";

    #[test]
    fn read_note_with_malformed_yaml_frontmatter_survives_without_error() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");

        let note_path = vault_dir.path().join("broken.md");
        std::fs::write(&note_path, MALFORMED_FRONTMATTER_NOTE)
            .expect("note file should be written");

        let store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let note = store
            .find_note_by_relative_path("broken.md")
            .expect("lookup should not error")
            .expect("malformed note should still be readable");
        assert!(
            note.content.contains("Original body content."),
            "body content should be preserved even though frontmatter failed to parse"
        );
        assert!(
            note.frontmatter_raw_fallback.is_some(),
            "unparsable frontmatter should be retained as a raw fallback"
        );
        assert!(
            note.frontmatter_raw_fallback
                .as_ref()
                .unwrap()
                .contains("Original Title"),
            "raw fallback should contain the original frontmatter text"
        );
    }

    #[test]
    fn content_only_update_preserves_original_raw_frontmatter_verbatim() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");

        let note_path = vault_dir.path().join("broken.md");
        std::fs::write(&note_path, MALFORMED_FRONTMATTER_NOTE)
            .expect("note file should be written");

        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );
        let note_id = store
            .find_note_by_relative_path("broken.md")
            .expect("lookup should not error")
            .expect("malformed note should still be readable")
            .id;

        store
            .update_note(
                &note_id,
                NoteUpdate {
                    content: Some("Updated body content.".to_string()),
                    ..Default::default()
                },
            )
            .expect("content-only update should succeed on a malformed-frontmatter note");

        let persisted = std::fs::read_to_string(&note_path).expect("note file should still exist");
        assert!(
            persisted.contains("title: Original Title\ntags:\n\t- alpha\nstatus: canonical"),
            "original raw frontmatter block should be preserved byte-for-byte:\n{persisted}"
        );
        assert!(
            persisted.contains("Updated body content."),
            "new content should be written:\n{persisted}"
        );
        assert!(
            !persisted.contains("Original body content."),
            "old content should be replaced, not appended:\n{persisted}"
        );
    }

    #[test]
    fn explicit_frontmatter_update_replaces_malformed_original() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");

        let note_path = vault_dir.path().join("broken.md");
        std::fs::write(&note_path, MALFORMED_FRONTMATTER_NOTE)
            .expect("note file should be written");

        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );
        let note_id = store
            .find_note_by_relative_path("broken.md")
            .expect("lookup should not error")
            .expect("malformed note should still be readable")
            .id;

        let updated = store
            .update_note(
                &note_id,
                NoteUpdate {
                    status: Some(NoteStatus::Evidence),
                    ..Default::default()
                },
            )
            .expect("explicit frontmatter update should succeed");

        assert!(
            updated.frontmatter_raw_fallback.is_none(),
            "explicitly editing a frontmatter field should clear the raw fallback"
        );

        let persisted = std::fs::read_to_string(&note_path).expect("note file should still exist");
        assert!(
            !persisted.contains("\t- alpha"),
            "malformed original frontmatter should no longer be present:\n{persisted}"
        );
        assert!(
            persisted.contains("status: evidence"),
            "newly serialized frontmatter should reflect the explicit edit:\n{persisted}"
        );
    }

    #[test]
    fn well_formed_frontmatter_has_no_raw_fallback() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");
        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let note = store
            .create_note(NoteCreate {
                title: "Well Formed".to_string(),
                content: "Body.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: Default::default(),
                tags: Vec::new(),
                schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: None,
                optimizer_managed: false,
                properties: HashMap::new(),
            })
            .expect("note should be created");

        assert!(
            note.frontmatter_raw_fallback.is_none(),
            "a freshly created, well-formed note must not carry a raw fallback"
        );

        let fetched = store.get_note(&note.id).expect("note should be readable");
        assert!(
            fetched.frontmatter_raw_fallback.is_none(),
            "re-reading a well-formed note must not carry a raw fallback"
        );
    }

    /// Valid YAML frontmatter that simply omits `title:` — the norm for
    /// Obsidian-style and imported vaults. This must NOT be treated as a parse
    /// failure: title falls back to the H1 heading downstream, and custom fields
    /// must flow into `properties` and round-trip through writes.
    const TITLE_LESS_VALID_FRONTMATTER_NOTE: &str = "---\nstatus: evidence\ntags:\n  - alpha\ncustom_field: keep-me\n---\n\n# Heading Title\n\nBody text.";

    #[test]
    fn title_less_valid_frontmatter_parses_without_fallback_and_round_trips() {
        let vault_dir = tempdir().expect("vault tempdir");
        let data_dir = tempdir().expect("data tempdir");

        let note_path = vault_dir.path().join("no-title.md");
        std::fs::write(&note_path, TITLE_LESS_VALID_FRONTMATTER_NOTE)
            .expect("note file should be written");

        let mut store = KnowledgeStore::new(
            vault_dir.path().to_path_buf(),
            data_dir.path().to_path_buf(),
        );

        let note = store
            .find_note_by_relative_path("no-title.md")
            .expect("lookup should not error")
            .expect("title-less note should be readable");
        assert!(
            note.frontmatter_raw_fallback.is_none(),
            "valid frontmatter without a title must NOT be treated as a parse failure"
        );
        assert_eq!(
            note.title, "Heading Title",
            "title should fall back to the H1 heading"
        );
        assert_eq!(note.status, NoteStatus::Evidence);
        assert!(note.tags.contains(&"alpha".to_string()));
        assert_eq!(
            note.properties.get("custom_field").and_then(|v| v.as_str()),
            Some("keep-me"),
            "custom frontmatter fields should flow into properties"
        );

        // Content-only edit: custom field must survive on disk.
        store
            .update_note(
                &note.id,
                NoteUpdate {
                    content: Some("# Heading Title\n\nEdited body.".to_string()),
                    ..Default::default()
                },
            )
            .expect("content-only update should succeed");
        let persisted = std::fs::read_to_string(&note_path).expect("note file should still exist");
        assert!(
            persisted.contains("custom_field: keep-me"),
            "custom field should round-trip through a content edit:\n{persisted}"
        );
        assert!(persisted.contains("Edited body."));

        // Explicit frontmatter edit: custom field must STILL survive, because the
        // frontmatter deserialized successfully and custom fields live in properties.
        store
            .update_note(
                &note.id,
                NoteUpdate {
                    status: Some(NoteStatus::Canonical),
                    ..Default::default()
                },
            )
            .expect("explicit status update should succeed");
        let persisted = std::fs::read_to_string(&note_path).expect("note file should still exist");
        assert!(
            persisted.contains("custom_field: keep-me"),
            "custom field should round-trip through an explicit status edit:\n{persisted}"
        );
        assert!(
            persisted.contains("status: canonical"),
            "status edit should be reflected:\n{persisted}"
        );
    }
}
