use crate::models::migration::{
    MarkdownMigrationApplyResult, MarkdownMigrationMode, MarkdownMigrationNoteProposal,
    MarkdownMigrationPreview, MarkdownMigrationPreviewSummary, MarkdownMigrationRequest,
    MarkdownMigrationStatus, MarkdownMigrationTopicCandidate,
};
use crate::models::note::{
    Note, NoteCreate, NoteUpdate, PROP_AUTO_INSERTED_LINK_IDS, PROP_INFERRED_LINK_IDS,
    PROP_TOPIC_ALIASES, PROP_TOPIC_KEY, CURRENT_NOTE_SCHEMA_VERSION,
};
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::topic_hub::normalize_topic_key;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MIGRATION_SOURCE_MARKDOWN: &str = "markdown_migration";
const MIGRATION_SOURCE_BACKFILL: &str = "grafyn_schema_backfill";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredManifest {
    run_id: String,
    preview_id: String,
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
}

#[derive(Debug, Clone)]
pub struct MarkdownMigrationService {
    data_path: PathBuf,
    runs_dir: PathBuf,
}

impl MarkdownMigrationService {
    pub fn new(data_path: PathBuf) -> Self {
        let base_dir = data_path.join("vault_migration");
        let runs_dir = base_dir.join("runs");
        let _ = std::fs::create_dir_all(&runs_dir);
        let _ = std::fs::create_dir_all(base_dir.join("overlay").join("notes"));
        Self {
            data_path,
            runs_dir,
        }
    }

    pub fn preview(
        &self,
        vault_path: PathBuf,
        request: MarkdownMigrationRequest,
    ) -> Result<MarkdownMigrationPreview> {
        let store = KnowledgeStore::new(vault_path.clone(), self.data_path.clone());
        let notes = store.list_full_notes()?;
        let created_at = Utc::now();
        let preview_id = Uuid::new_v4().to_string();
        let hub_folder = normalize_hub_folder(request.hub_folder.as_deref().unwrap_or("_grafyn/hubs"));
        let program_path = normalize_program_path(request.program_path.as_deref().unwrap_or("_grafyn/program.md"));

        let resolution_index = build_reference_index(&notes);
        let existing_hubs = notes
            .iter()
            .filter(|note| note.is_topic_hub())
            .filter_map(|note| note.topic_key().map(|key| (key, note.id.clone())))
            .collect::<HashMap<_, _>>();

        let mut summary = MarkdownMigrationPreviewSummary::default();
        let mut topic_buckets: BTreeMap<String, MarkdownMigrationTopicCandidate> = BTreeMap::new();
        let mut note_proposals = Vec::new();
        let mut ambiguous_titles = HashMap::new();

        summary.total_scanned_notes = notes.len();

        for note in &notes {
            let raw_path = vault_path.join(&note.relative_path);
            let raw_content = std::fs::read_to_string(&raw_path).unwrap_or_default();
            let has_frontmatter = raw_content.trim_start().starts_with("---");
            if !has_frontmatter {
                summary.files_without_frontmatter += 1;
            }
            if !raw_content.trim_start().starts_with("---\n") || note.title == humanize_title(&note.relative_path) {
                summary.inferred_titles += 1;
            }
            if !note.aliases.is_empty() {
                summary.inferred_aliases += note.aliases.len();
            }

            let inferred_tags = infer_topic_tags(note);
            if !inferred_tags.is_empty() {
                summary.inferred_tags_or_topic_seeds += inferred_tags.len();
            }

            let mut markdown_resolved = 0usize;
            let mut wikilinks_resolved = 0usize;
            for parsed_link in &note.parsed_links {
                if let Some(target_path) = &parsed_link.target_path {
                    if store.find_note_by_relative_path(target_path)?.is_some() {
                        markdown_resolved += 1;
                    }
                } else if resolve_reference(parsed_link.target_title.as_str(), &resolution_index)
                    .is_some()
                {
                    wikilinks_resolved += 1;
                }
            }
            summary.markdown_links_resolved += markdown_resolved;
            summary.wikilinks_resolved += wikilinks_resolved;

            let collisions = find_ambiguous_references(note, &resolution_index);
            if !collisions.is_empty() {
                summary.ambiguous_matches += collisions.len();
                ambiguous_titles.extend(collisions);
            }

            let inferred_link_ids = infer_unlinked_note_mentions(note, &resolution_index);
            let topic_key = inferred_tags
                .first()
                .map(|value| normalize_topic_key(value))
                .filter(|value| !value.is_empty());

            let confidence = if note.tags.is_empty() { 0.78 } else { 0.91 };
            let write_required = request.mode.allows_user_note_writes()
                && (!inferred_link_ids.is_empty()
                    || note.schema_version < CURRENT_NOTE_SCHEMA_VERSION
                    || !inferred_tags.is_empty());

            if write_required {
                summary.files_to_rewrite += 1;
            }
            if !inferred_link_ids.is_empty() && request.mode.allows_user_note_writes() {
                summary.proposed_auto_link_edits += inferred_link_ids.len().min(3);
            }
            if note.schema_version < CURRENT_NOTE_SCHEMA_VERSION && note.migration_source.is_none() {
                summary.old_grafyn_notes_eligible_for_backfill += 1;
            }

            if let Some(topic_key) = &topic_key {
                let display_name = display_topic_name(topic_key);
                let entry = topic_buckets
                    .entry(topic_key.clone())
                    .or_insert_with(|| MarkdownMigrationTopicCandidate {
                        topic_key: topic_key.clone(),
                        display_name: display_name.clone(),
                        reuse_existing_hub_id: existing_hubs.get(topic_key).cloned(),
                        ..Default::default()
                    });
                entry.member_note_ids.push(note.id.clone());
                entry.member_note_titles.push(note.title.clone());
            }

            note_proposals.push(MarkdownMigrationNoteProposal {
                note_id: note.id.clone(),
                title: note.title.clone(),
                relative_path: note.relative_path.clone(),
                aliases: note.aliases.clone(),
                inferred_tags,
                inferred_link_ids,
                topic_key,
                confidence,
                write_required,
            });
        }

        let topic_candidates = topic_buckets.into_values().collect::<Vec<_>>();
        summary.proposed_hubs = topic_candidates
            .iter()
            .filter(|candidate| candidate.reuse_existing_hub_id.is_none())
            .count();
        summary.files_to_create = summary.proposed_hubs
            + usize::from(!vault_path.join(&program_path).exists());

        let preview = MarkdownMigrationPreview {
            preview_id: preview_id.clone(),
            vault_path: vault_path.to_string_lossy().to_string(),
            created_at: Some(created_at),
            mode: request.mode,
            hub_folder,
            program_path,
            summary,
            topic_candidates,
            note_proposals,
            ambiguous_titles,
        };

        let run_dir = self.runs_dir.join(&preview_id);
        std::fs::create_dir_all(&run_dir)?;
        std::fs::write(
            run_dir.join("preview.json"),
            serde_json::to_string_pretty(&preview)?,
        )?;

        Ok(preview)
    }

    pub fn apply(
        &self,
        preview_id: &str,
        request: MarkdownMigrationRequest,
        store: &mut KnowledgeStore,
    ) -> Result<MarkdownMigrationApplyResult> {
        let preview = self.load_preview(preview_id)?;
        let run_id = preview.preview_id.clone();
        let run_dir = self.runs_dir.join(&run_id);
        std::fs::create_dir_all(run_dir.join("backups"))?;

        let mut manifest = StoredManifest {
            run_id: run_id.clone(),
            preview_id: preview.preview_id.clone(),
            vault_path: preview.vault_path.clone(),
            mode: request.mode.clone(),
            created_at: preview.created_at.unwrap_or_else(Utc::now),
            applied_at: Some(Utc::now()),
            status: "applied".to_string(),
            ..Default::default()
        };

        let mut touched_note_ids = Vec::new();
        let mut overlay_note_ids = Vec::new();

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
                store.write_overlay(&proposal.note_id, &overlay)?;
                overlay_note_ids.push(proposal.note_id.clone());
                continue;
            }

            let note = store.get_note(&proposal.note_id)?;
            self.backup_note(&run_dir, &preview.vault_path, &note.relative_path)?;
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

            let updated = store.update_note(
                &proposal.note_id,
                NoteUpdate {
                    title: None,
                    content: new_content,
                    relative_path: Some(note.relative_path.clone()),
                    aliases: Some(merge_unique_strings(note.aliases, proposal.aliases.clone())),
                    status: None,
                    tags: Some(merge_unique_strings(note.tags, proposal.inferred_tags.clone())),
                    schema_version: Some(CURRENT_NOTE_SCHEMA_VERSION),
                    migration_source: Some(MIGRATION_SOURCE_MARKDOWN.to_string()),
                    optimizer_managed: Some(false),
                    properties: Some(properties),
                },
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
            let created = store.create_note(NoteCreate {
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
                    (PROP_TOPIC_KEY.to_string(), Value::String(topic.topic_key.clone())),
                    (
                        PROP_TOPIC_ALIASES.to_string(),
                        Value::Array(vec![Value::String(topic.display_name.clone())]),
                    ),
                    ("is_topic_hub".to_string(), Value::Bool(true)),
                ]),
            })?;
            created_hub_note_ids.push(created.id.clone());
            manifest
                .created_files
                .push(created.relative_path.clone());
        }

        let program_path = Path::new(&preview.vault_path).join(&preview.program_path);
        if !program_path.exists() {
            if let Some(parent) = program_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(
                &program_path,
                default_program_file_contents(&preview.hub_folder, &preview.program_path),
            )?;
            manifest.created_files.push(preview.program_path.clone());
        }

        manifest.overlay_note_ids = overlay_note_ids.clone();
        manifest.touched_note_ids = touched_note_ids.clone();
        manifest.created_hub_note_ids = created_hub_note_ids.clone();
        std::fs::write(
            run_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest)?,
        )?;

        Ok(MarkdownMigrationApplyResult {
            run_id,
            status: "applied".to_string(),
            created_hub_note_ids,
            touched_note_ids,
            overlay_note_ids,
            message: "Markdown migration applied".to_string(),
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

    pub fn rollback(&self, run_id: &str, store: &mut KnowledgeStore) -> Result<()> {
        let manifest = self.load_manifest(run_id)?;
        let vault_path = PathBuf::from(&manifest.vault_path);
        let run_dir = self.runs_dir.join(run_id);
        let backups_dir = run_dir.join("backups");

        for relative_path in &manifest.backup_files {
            let backup_path = backups_dir.join(relative_path);
            let target_path = vault_path.join(relative_path);
            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&backup_path, &target_path).with_context(|| {
                format!(
                    "Failed to restore backup '{}' -> '{}'",
                    backup_path.display(),
                    target_path.display()
                )
            })?;
        }

        for relative_path in &manifest.created_files {
            let target_path = vault_path.join(relative_path);
            if target_path.exists() {
                std::fs::remove_file(&target_path)?;
            }
        }

        for note_id in &manifest.overlay_note_ids {
            store.delete_overlay(note_id)?;
        }

        let mut updated_manifest = manifest;
        updated_manifest.status = "rolled_back".to_string();
        std::fs::write(
            run_dir.join("manifest.json"),
            serde_json::to_string_pretty(&updated_manifest)?,
        )?;
        Ok(())
    }

    pub fn backfill_legacy_grafyn_notes(&self, store: &mut KnowledgeStore) -> Result<Vec<String>> {
        let mut updated_ids = Vec::new();
        let notes = store.list_full_notes()?;
        for note in notes {
            if note.schema_version >= CURRENT_NOTE_SCHEMA_VERSION
                && note.migration_source.is_some()
                && !note.aliases.is_empty()
            {
                continue;
            }

            let updated = store.update_note(
                &note.id,
                NoteUpdate {
                    title: None,
                    content: None,
                    relative_path: Some(note.relative_path.clone()),
                    aliases: Some(note.aliases.clone()),
                    status: None,
                    tags: None,
                    schema_version: Some(CURRENT_NOTE_SCHEMA_VERSION),
                    migration_source: Some(MIGRATION_SOURCE_BACKFILL.to_string()),
                    optimizer_managed: Some(note.optimizer_managed || note.is_topic_hub()),
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

    fn backup_note(&self, run_dir: &Path, vault_path: &str, relative_path: &str) -> Result<()> {
        let source_path = Path::new(vault_path).join(relative_path);
        if !source_path.exists() {
            return Ok(());
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

        let manifest_path = run_dir.join("manifest.json");
        if manifest_path.exists() {
            let mut manifest: StoredManifest =
                serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)?;
            if !manifest.backup_files.contains(&relative_path.to_string()) {
                manifest.backup_files.push(relative_path.to_string());
                std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
            }
        }

        Ok(())
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

fn resolve_reference(reference: &str, resolution_index: &HashMap<String, Vec<String>>) -> Option<String> {
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
    if content.lines().any(|line| line.trim() == format!("# {}", title)) {
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
