use crate::models::note::{
    Note, NoteCreate, NoteFrontmatter, NoteMeta, NoteUpdate, ParsedLink, RelationType,
};
use anyhow::{Context, Result};
use chrono::Utc;
use gray_matter::{engine::YAML, Matter};
use lazy_static::lazy_static;
use regex::Regex;
use std::path::PathBuf;
use walkdir::WalkDir;

lazy_static! {
    /// Regex for extracting wikilinks: [[Target]] or [[Target|Display]]
    static ref WIKILINK_REGEX: Regex = Regex::new(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]").unwrap();

    /// Regex for extracting typed wikilinks: [[Target]] (relation_type)
    /// Captures: group 1 = target title, group 2 = optional relation type
    static ref TYPED_WIKILINK_REGEX: Regex =
        Regex::new(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]\s*(?:\((\w+)\))?").unwrap();
}

/// Service for managing markdown notes with YAML frontmatter
#[derive(Debug, Clone)]
pub struct KnowledgeStore {
    vault_path: PathBuf,
    /// In-memory cache of note metadata, kept in sync with disk.
    /// Eliminates WalkDir + YAML parse on every list_notes() call.
    meta_cache: Vec<NoteMeta>,
}

impl KnowledgeStore {
    pub fn new(vault_path: PathBuf) -> Self {
        // Ensure vault directory exists
        if let Err(e) = std::fs::create_dir_all(&vault_path) {
            log::error!("Failed to create vault directory {}: {}", vault_path.display(), e);
        }
        let mut store = Self {
            vault_path,
            meta_cache: Vec::new(),
        };
        store.refresh_cache();
        store
    }

    /// Update the vault path at runtime (e.g., after settings change)
    pub fn set_vault_path(&mut self, vault_path: PathBuf) {
        if let Err(e) = std::fs::create_dir_all(&vault_path) {
            log::error!("Failed to create vault directory {}: {}", vault_path.display(), e);
        }
        log::info!("Vault path updated to {:?}", vault_path);
        self.vault_path = vault_path;
        self.refresh_cache();
    }

    /// Rebuild the metadata cache from disk
    fn refresh_cache(&mut self) {
        let mut notes = Vec::new();

        for entry in WalkDir::new(&self.vault_path)
            .min_depth(1)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "md") {
                if let Ok(note) = self.read_note_file(path) {
                    notes.push(NoteMeta::from(&note));
                }
            }
        }

        notes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        self.meta_cache = notes;
    }

    /// List all notes in the vault (metadata only, served from cache)
    pub fn list_notes(&self) -> Result<Vec<NoteMeta>> {
        Ok(self.meta_cache.clone())
    }

    /// Get a full note by ID
    pub fn get_note(&self, id: &str) -> Result<Note> {
        Self::validate_note_id(id)?;
        let path = self.note_path(id);
        self.read_note_file(&path)
            .with_context(|| format!("Note not found: {}", id))
    }

    /// Create a new note
    pub fn create_note(&mut self, create: NoteCreate) -> Result<Note> {
        let id = self.generate_note_id(&create.title);
        let now = Utc::now();

        let note = Note {
            id: id.clone(),
            title: create.title,
            content: create.content,
            status: create.status,
            tags: create.tags,
            created_at: now,
            updated_at: now,
            wikilinks: Vec::new(),
            parsed_links: Vec::new(),
            properties: create.properties,
        };

        // Extract wikilinks from content
        let mut note = note;
        note.wikilinks = self.extract_wikilinks(&note.content);
        note.parsed_links = self.extract_typed_wikilinks(&note.content);

        self.write_note_file(&note)?;

        // Update cache: insert at front (newest first)
        self.meta_cache.insert(0, NoteMeta::from(&note));

        Ok(note)
    }

    /// Update an existing note
    pub fn update_note(&mut self, id: &str, update: NoteUpdate) -> Result<Note> {
        Self::validate_note_id(id)?;
        let mut note = self.get_note(id)?;

        if let Some(title) = update.title {
            note.title = title;
        }
        if let Some(content) = update.content {
            note.content = content;
            note.wikilinks = self.extract_wikilinks(&note.content);
            note.parsed_links = self.extract_typed_wikilinks(&note.content);
        }
        if let Some(status) = update.status {
            note.status = status;
        }
        if let Some(tags) = update.tags {
            note.tags = tags;
        }
        if let Some(properties) = update.properties {
            note.properties = properties;
        }

        note.updated_at = Utc::now();
        self.write_note_file(&note)?;

        // Update cache: replace existing entry and re-sort to front
        let meta = NoteMeta::from(&note);
        if let Some(pos) = self.meta_cache.iter().position(|m| m.id == id) {
            self.meta_cache[pos] = meta.clone();
            // Move to front since it's the most recently updated
            let item = self.meta_cache.remove(pos);
            self.meta_cache.insert(0, item);
        } else {
            // Not in cache (shouldn't happen), insert at front
            self.meta_cache.insert(0, meta);
        }

        Ok(note)
    }

    /// Delete a note
    pub fn delete_note(&mut self, id: &str) -> Result<()> {
        Self::validate_note_id(id)?;
        let path = self.note_path(id);
        std::fs::remove_file(&path).with_context(|| format!("Failed to delete note: {}", id))?;

        // Remove from cache
        self.meta_cache.retain(|m| m.id != id);

        Ok(())
    }

    /// Extract wikilinks from markdown content (titles only, for backward compat)
    pub fn extract_wikilinks(&self, content: &str) -> Vec<String> {
        WIKILINK_REGEX
            .captures_iter(content)
            .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
            .collect()
    }

    /// Extract typed wikilinks with relationship information
    /// Parses: [[Target]] (supports), [[Target]], - [[Target]] (expands)
    pub fn extract_typed_wikilinks(&self, content: &str) -> Vec<ParsedLink> {
        TYPED_WIKILINK_REGEX
            .captures_iter(content)
            .filter_map(|cap| {
                let target_title = cap.get(1)?.as_str().to_string();
                let relation = cap
                    .get(2)
                    .map(|m| RelationType::from_str_lossy(m.as_str()))
                    .unwrap_or(RelationType::Untyped);
                Some(ParsedLink {
                    target_title,
                    relation,
                })
            })
            .collect()
    }

    /// Generate a note ID from title (slug format)
    fn generate_note_id(&self, title: &str) -> String {
        let slug: String = title
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() {
                    c
                } else if c.is_whitespace() || c == '-' || c == '_' {
                    '-'
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-");

        // Handle duplicates by adding a suffix
        let mut id = slug.clone();
        let mut counter = 1;
        while self.note_path(&id).exists() {
            id = format!("{}-{}", slug, counter);
            counter += 1;
        }
        id
    }

    /// Validate that a note ID doesn't contain path traversal sequences
    fn validate_note_id(id: &str) -> Result<()> {
        if id.is_empty()
            || id.contains('/')
            || id.contains('\\')
            || id.contains("..")
        {
            anyhow::bail!("Invalid note ID: {}", id);
        }
        Ok(())
    }

    /// Get the file path for a note ID
    fn note_path(&self, id: &str) -> PathBuf {
        self.vault_path.join(format!("{}.md", id))
    }

    /// Read and parse a note file
    fn read_note_file(&self, path: &std::path::Path) -> Result<Note> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {:?}", path))?;

        let matter = Matter::<YAML>::new();
        let parsed = matter.parse(&content);

        // Extract frontmatter
        let frontmatter: NoteFrontmatter = parsed
            .data
            .map(|d| d.deserialize())
            .transpose()
            .unwrap_or(None)
            .unwrap_or_default();

        // Get file metadata for timestamps
        let metadata = std::fs::metadata(path)?;
        let file_modified = metadata.modified().ok();

        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let now = Utc::now();
        let created_at = frontmatter.created_at.unwrap_or(now);
        let updated_at = frontmatter.updated_at.or_else(|| {
            file_modified.map(|t| chrono::DateTime::<Utc>::from(t))
        }).unwrap_or(now);

        let body = parsed.content;
        let wikilinks = self.extract_wikilinks(&body);
        let parsed_links = self.extract_typed_wikilinks(&body);

        Ok(Note {
            id,
            title: frontmatter.title,
            content: body,
            status: frontmatter.status.parse().unwrap_or_default(),
            tags: frontmatter.tags,
            created_at,
            updated_at,
            wikilinks,
            parsed_links,
            properties: frontmatter.extra,
        })
    }

    /// Write a note to file with YAML frontmatter
    fn write_note_file(&self, note: &Note) -> Result<()> {
        let path = self.note_path(&note.id);

        // Build frontmatter
        let mut frontmatter = serde_yaml::Mapping::new();
        frontmatter.insert(
            serde_yaml::Value::String("title".to_string()),
            serde_yaml::Value::String(note.title.clone()),
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
                    .map(|t| serde_yaml::Value::String(t.clone()))
                    .collect(),
            ),
        );
        frontmatter.insert(
            serde_yaml::Value::String("created_at".to_string()),
            serde_yaml::Value::String(note.created_at.to_rfc3339()),
        );
        frontmatter.insert(
            serde_yaml::Value::String("updated_at".to_string()),
            serde_yaml::Value::String(note.updated_at.to_rfc3339()),
        );

        // Add extra properties
        for (key, value) in &note.properties {
            if let Ok(yaml_value) = serde_yaml::to_value(value) {
                frontmatter.insert(serde_yaml::Value::String(key.clone()), yaml_value);
            }
        }

        let yaml = serde_yaml::to_string(&frontmatter)?;
        let file_content = format!("---\n{}---\n\n{}", yaml, note.content);

        std::fs::write(&path, file_content)
            .with_context(|| format!("Failed to write note: {:?}", path))?;

        Ok(())
    }
}
