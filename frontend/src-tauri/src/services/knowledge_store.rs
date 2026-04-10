use crate::models::note::{
    Note, NoteCreate, NoteFrontmatter, NoteMeta, NoteUpdate, ParsedLink, RelationType,
    CURRENT_NOTE_SCHEMA_VERSION,
};
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

        let overlay_notes_dir = data_path.join("vault_migration").join("overlay").join("notes");
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
                    self.relative_path_index.insert(
                        normalize_lookup_key(&note.relative_path),
                        note.id.clone(),
                    );
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
                    log::warn!("Failed to read markdown note '{}': {}", path.display(), error);
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
        let path = self
            .path_index
            .get(id)
            .cloned()
            .unwrap_or_else(|| self.vault_path.join(format!("{}.md", id)));
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
        let old_path = self.note_path(id);

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

        note.updated_at = Utc::now();
        note.wikilinks = self.extract_wikilinks(&note.content);
        note.parsed_links = self.extract_links(&note.content, &note.relative_path);

        self.write_note_file(&note)?;
        let new_path = self.vault_path.join(&note.relative_path);
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
        let path = self.note_path(id);
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
        std::fs::write(self.overlay_path(note_id), serde_json::to_string_pretty(overlay)?)
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
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            anyhow::bail!("Invalid note ID: {}", id);
        }
        Ok(())
    }

    fn note_path(&self, id: &str) -> PathBuf {
        self.path_index
            .get(id)
            .cloned()
            .unwrap_or_else(|| self.vault_path.join(format!("{}.md", id)))
    }

    fn read_note_file(&self, path: &Path) -> Result<Note> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;

        let matter = Matter::<YAML>::new();
        let parsed = matter.parse(&content);

        let frontmatter: NoteFrontmatter = parsed
            .data
            .map(|data| data.deserialize())
            .transpose()
            .unwrap_or(None)
            .unwrap_or_default();

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

        note.aliases = dedupe_strings(
            note.aliases
                .clone()
                .into_iter()
                .chain(overlay.aliases),
        );
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
        let path = self.vault_path.join(&relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
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
        std::fs::write(&path, file_content)
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
        let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("note");
        let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("md");
        let parent = path.parent().and_then(|value| value.to_str()).unwrap_or("");

        let mut counter = 1;
        loop {
            let filename = format!("{}-{}.{}", stem, counter, ext);
            let candidate = if parent.is_empty() {
                filename.clone()
            } else {
                format!("{}/{}", normalize_relative_path_for_output(parent), filename)
            };
            if !self.vault_path.join(&candidate).exists() {
                return candidate;
            }
            counter += 1;
        }
    }

    fn is_reserved_program_path(&self, path: &Path) -> bool {
        let Some(relative) = path.strip_prefix(&self.vault_path).ok().and_then(|p| p.to_str()) else {
            return false;
        };
        normalize_relative_path_for_output(relative).eq_ignore_ascii_case("_grafyn/program.md")
    }
}

fn normalize_lookup_key(value: &str) -> String {
    value.trim().replace('\\', "/").to_lowercase()
}

fn normalize_relative_path_for_output(value: &str) -> String {
    value.replace('\\', "/").trim_start_matches("./").to_string()
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
    if normalized.split('/').any(|segment| segment.is_empty() || segment == "..") {
        anyhow::bail!("Path traversal is not allowed in note paths");
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
            } else if character.is_whitespace() || character == '-' || character == '_' || character == '/' {
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
