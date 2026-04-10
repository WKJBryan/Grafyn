use crate::models::note::{
    Note, NoteCreate, NoteStatus, NoteUpdate, PROP_IS_TOPIC_HUB, PROP_TOPIC_ALIASES, PROP_TOPIC_KEY,
};
use crate::services::knowledge_store::KnowledgeStore;
use anyhow::Result;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct TopicHubSyncResult {
    pub all_notes: Vec<Note>,
    pub changed_note_ids: Vec<String>,
    pub removed_note_ids: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct TopicHubMembership {
    pub hub_id: String,
    pub hub_title: String,
    pub topic_key: String,
    pub membership_source: String,
}

#[derive(Debug, Clone)]
struct HubRecord {
    hub_id: String,
    topic_key: String,
    aliases: HashSet<String>,
}

#[derive(Debug, Clone, Default)]
struct HubRegistry {
    by_id: HashMap<String, HubRecord>,
    by_alias: HashMap<String, String>,
}

impl HubRegistry {
    fn insert(&mut self, note: &Note) {
        let topic_key = note
            .topic_key()
            .unwrap_or_else(|| normalize_topic_key(&strip_hub_prefix(&note.title)));
        let aliases = note
            .topic_aliases()
            .into_iter()
            .chain(std::iter::once(topic_key.clone()))
            .chain(std::iter::once(normalize_topic_key(&strip_hub_prefix(
                &note.title,
            ))))
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();

        let record = HubRecord {
            hub_id: note.id.clone(),
            topic_key: topic_key.clone(),
            aliases: aliases.clone(),
        };

        for alias in &aliases {
            self.by_alias
                .entry(alias.clone())
                .or_insert_with(|| note.id.clone());
        }
        self.by_alias
            .entry(topic_key)
            .or_insert_with(|| note.id.clone());
        self.by_id.insert(note.id.clone(), record);
    }

    fn get(&self, hub_id: &str) -> Option<&HubRecord> {
        self.by_id.get(hub_id)
    }

    fn resolve(&self, seed: &str) -> Option<(String, String)> {
        let normalized = normalize_topic_key(seed);
        if normalized.is_empty() {
            return None;
        }

        if let Some(hub_id) = self.by_alias.get(&normalized) {
            return Some((hub_id.clone(), "exact".to_string()));
        }

        let mut best_match: Option<(&HubRecord, f64)> = None;
        for hub in self.by_id.values() {
            let score = hub
                .aliases
                .iter()
                .map(|alias| fuzzy_topic_score(&normalized, alias))
                .fold(0.0_f64, f64::max);
            if score < 0.74 {
                continue;
            }

            match best_match {
                Some((_, current_score)) if current_score >= score => {}
                _ => best_match = Some((hub, score)),
            }
        }

        best_match.map(|(hub, _)| (hub.hub_id.clone(), "fuzzy".to_string()))
    }
}

pub fn sync_topic_hubs(store: &mut KnowledgeStore) -> Result<TopicHubSyncResult> {
    let mut notes_by_id = store
        .list_full_notes()?
        .into_iter()
        .map(|note| (note.id.clone(), note))
        .collect::<HashMap<_, _>>();
    let mut changed_note_ids = BTreeSet::new();

    let existing_hub_ids = notes_by_id
        .values()
        .filter(|note| note.is_topic_hub())
        .map(|note| note.id.clone())
        .collect::<Vec<_>>();

    for hub_id in existing_hub_ids {
        let Some(current) = notes_by_id.get(&hub_id).cloned() else {
            continue;
        };
        let desired = standardize_existing_hub(&current);
        if let Some(updated) = persist_if_changed(store, &current, desired)? {
            changed_note_ids.insert(updated.id.clone());
            notes_by_id.insert(updated.id.clone(), updated);
        }
    }

    let mut registry = build_hub_registry(notes_by_id.values());

    let regular_note_ids = notes_by_id
        .values()
        .filter(|note| !note.is_topic_hub())
        .map(|note| note.id.clone())
        .collect::<Vec<_>>();

    for note_id in regular_note_ids {
        let Some(current) = notes_by_id.get(&note_id).cloned() else {
            continue;
        };

        let topic_seeds = candidate_topic_seeds(&current);
        let mut resolved_hub_ids = Vec::new();
        let mut primary_topic_key = None;

        for seed in topic_seeds {
            let (hub_id, _) = if let Some(resolved) = registry.resolve(&seed) {
                resolved
            } else {
                let created = create_topic_hub_note(store, &seed)?;
                let hub_id = created.id.clone();
                changed_note_ids.insert(hub_id.clone());
                notes_by_id.insert(hub_id.clone(), created.clone());
                registry.insert(&created);
                (hub_id, "created".to_string())
            };

            if resolved_hub_ids.contains(&hub_id) {
                continue;
            }

            if primary_topic_key.is_none() {
                primary_topic_key = registry.get(&hub_id).map(|hub| hub.topic_key.clone());
            }
            resolved_hub_ids.push(hub_id);
        }

        let mut desired = current.clone();
        desired.set_topic_hub_metadata(false, primary_topic_key, resolved_hub_ids, Vec::new());
        if let Some(updated) = persist_if_changed(store, &current, desired)? {
            changed_note_ids.insert(updated.id.clone());
            notes_by_id.insert(updated.id.clone(), updated);
        }
    }

    let refreshed_notes = store.list_full_notes()?;
    notes_by_id = refreshed_notes
        .into_iter()
        .map(|note| (note.id.clone(), note))
        .collect::<HashMap<_, _>>();
    registry = build_hub_registry(notes_by_id.values());

    let memberships = build_memberships(notes_by_id.values());
    let related_hubs = build_related_hubs(&notes_by_id, &memberships);

    let current_hub_ids = registry.by_id.keys().cloned().collect::<Vec<_>>();
    for hub_id in current_hub_ids {
        let Some(current) = notes_by_id.get(&hub_id).cloned() else {
            continue;
        };
        let members = memberships.get(&hub_id).cloned().unwrap_or_default();
        let related = related_hubs.get(&hub_id).cloned().unwrap_or_default();
        let desired = rewrite_hub_note(&current, &members, &related, &notes_by_id);
        if let Some(updated) = persist_if_changed(store, &current, desired)? {
            changed_note_ids.insert(updated.id.clone());
            notes_by_id.insert(updated.id.clone(), updated);
        }
    }

    Ok(TopicHubSyncResult {
        all_notes: store.list_full_notes()?,
        changed_note_ids: changed_note_ids.into_iter().collect(),
        removed_note_ids: Vec::new(),
    })
}

pub fn collect_note_topic_hubs(notes: &[Note], note_id: &str) -> Vec<TopicHubMembership> {
    let notes_by_id = notes
        .iter()
        .map(|note| (note.id.clone(), note))
        .collect::<HashMap<_, _>>();
    let Some(note) = notes_by_id.get(note_id) else {
        return Vec::new();
    };

    note.topic_hub_ids()
        .into_iter()
        .filter_map(|hub_id| {
            let hub = notes_by_id.get(&hub_id)?;
            Some(TopicHubMembership {
                hub_id: hub.id.clone(),
                hub_title: hub.title.clone(),
                topic_key: hub
                    .topic_key()
                    .unwrap_or_else(|| normalize_topic_key(&strip_hub_prefix(&hub.title))),
                membership_source: "auto".to_string(),
            })
        })
        .collect()
}

pub fn normalize_topic_key(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut previous_dash = false;

    for character in value.trim().chars() {
        let mapped = if character.is_ascii_alphanumeric() {
            previous_dash = false;
            character.to_ascii_lowercase()
        } else {
            if previous_dash {
                continue;
            }
            previous_dash = true;
            '-'
        };
        normalized.push(mapped);
    }

    normalized.trim_matches('-').to_string()
}

fn display_topic_name(topic_key: &str) -> String {
    topic_key
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

fn standardize_existing_hub(note: &Note) -> Note {
    let mut desired = note.clone();
    let topic_key = note
        .topic_key()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let from_title = normalize_topic_key(&strip_hub_prefix(&note.title));
            if from_title.is_empty() {
                note.tags
                    .iter()
                    .find(|tag| !is_structural_tag(tag))
                    .map(|tag| normalize_topic_key(tag))
                    .unwrap_or_default()
            } else {
                from_title
            }
        });

    let mut tags = note.tags.clone();
    ensure_tag(&mut tags, "hub");
    ensure_tag(&mut tags, "grafyn");
    if !topic_key.is_empty() {
        ensure_tag(&mut tags, &topic_key);
    }
    desired.tags = sorted_unique(tags);
    desired.set_topic_hub_metadata(
        true,
        if topic_key.is_empty() {
            None
        } else {
            Some(topic_key.clone())
        },
        Vec::new(),
        sorted_unique(vec![
            topic_key,
            normalize_topic_key(&strip_hub_prefix(&note.title)),
        ]),
    );
    desired
}

fn build_hub_registry<'a>(notes: impl Iterator<Item = &'a Note>) -> HubRegistry {
    let mut registry = HubRegistry::default();
    for note in notes.filter(|note| note.is_topic_hub()) {
        registry.insert(note);
    }
    registry
}

fn build_memberships<'a>(notes: impl Iterator<Item = &'a Note>) -> HashMap<String, Vec<String>> {
    let mut memberships: HashMap<String, Vec<String>> = HashMap::new();
    for note in notes.filter(|note| !note.is_topic_hub()) {
        for hub_id in note.topic_hub_ids() {
            memberships.entry(hub_id).or_default().push(note.id.clone());
        }
    }
    memberships
}

fn build_related_hubs(
    notes_by_id: &HashMap<String, Note>,
    memberships: &HashMap<String, Vec<String>>,
) -> HashMap<String, Vec<String>> {
    let ref_to_id = build_note_reference_index(notes_by_id);
    let mut pair_scores: HashMap<(String, String), usize> = HashMap::new();

    for note in notes_by_id.values().filter(|note| !note.is_topic_hub()) {
        let source_hubs = note.topic_hub_ids();
        if source_hubs.len() > 1 {
            for index in 0..source_hubs.len() {
                for next in (index + 1)..source_hubs.len() {
                    bump_pair(&mut pair_scores, &source_hubs[index], &source_hubs[next], 1);
                }
            }
        }

        for parsed_link in &note.parsed_links {
            let Some(target_id) = resolve_link_target(notes_by_id, &ref_to_id, parsed_link) else {
                continue;
            };
            let Some(target_note) = notes_by_id.get(target_id) else {
                continue;
            };

            let target_hubs = if target_note.is_topic_hub() {
                vec![target_note.id.clone()]
            } else {
                target_note.topic_hub_ids()
            };

            for source_hub in &source_hubs {
                for target_hub in &target_hubs {
                    if source_hub != target_hub {
                        bump_pair(&mut pair_scores, source_hub, target_hub, 2);
                    }
                }
            }
        }
    }

    let mut related: HashMap<String, Vec<String>> = HashMap::new();
    for ((left, right), score) in pair_scores {
        if score == 0 {
            continue;
        }
        related.entry(left.clone()).or_default().push(right.clone());
        related.entry(right).or_default().push(left);
    }

    for values in related.values_mut() {
        values.sort();
        values.dedup();
    }

    for hub_id in memberships.keys() {
        related.entry(hub_id.clone()).or_default();
    }

    related
}

fn rewrite_hub_note(
    note: &Note,
    member_ids: &[String],
    related_hub_ids: &[String],
    notes_by_id: &HashMap<String, Note>,
) -> Note {
    let mut desired = standardize_existing_hub(note);
    let topic_key = desired
        .topic_key()
        .unwrap_or_else(|| normalize_topic_key(&strip_hub_prefix(&desired.title)));
    let display_name = if desired.title.trim().is_empty() {
        hub_title_from_key(&topic_key)
    } else {
        desired.title.clone()
    };
    let summary = build_topic_summary(&display_name, &topic_key, member_ids, notes_by_id);
    let member_lines = build_member_lines(member_ids, notes_by_id);
    let debate_lines = build_debate_lines(member_ids, notes_by_id);
    let related_lines = build_related_lines(related_hub_ids, notes_by_id);

    let mut content = Vec::new();
    content.push(format!("# {}", display_name));
    content.push(String::new());
    content.push("## Summary".to_string());
    content.push(summary);
    content.push(String::new());
    content.push("## Notes In This Topic".to_string());
    content.extend(member_lines);
    content.push(String::new());
    content.push("## Debates And Questions".to_string());
    content.extend(debate_lines);
    content.push(String::new());
    content.push("## Go Deeper".to_string());
    content.extend(related_lines);

    desired.content = content.join("\n");
    desired
}

fn build_topic_summary(
    display_name: &str,
    topic_key: &str,
    member_ids: &[String],
    notes_by_id: &HashMap<String, Note>,
) -> String {
    let member_count = member_ids.len();
    let recurring_tags = member_ids
        .iter()
        .filter_map(|member_id| notes_by_id.get(member_id))
        .flat_map(|note| note.tags.iter())
        .filter(|tag| !is_structural_tag(tag) && normalize_topic_key(tag) != topic_key)
        .fold(HashMap::<String, usize>::new(), |mut acc, tag| {
            *acc.entry(tag.clone()).or_default() += 1;
            acc
        });

    let top_tags = recurring_tags.into_iter().collect::<Vec<_>>();
    let mut top_tags = top_tags;
    top_tags.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let tag_summary = top_tags
        .into_iter()
        .take(3)
        .map(|(tag, _)| tag)
        .collect::<Vec<_>>();

    if member_count == 0 {
        return format!(
            "{} exists as a topic hub, but there are no notes assigned to it yet.",
            display_name
        );
    }

    if tag_summary.is_empty() {
        format!(
            "{} currently summarizes {} linked note{} around this topic.",
            display_name,
            member_count,
            if member_count == 1 { "" } else { "s" }
        )
    } else {
        format!(
            "{} currently summarizes {} linked note{} around this topic. Recurring themes: {}.",
            display_name,
            member_count,
            if member_count == 1 { "" } else { "s" },
            tag_summary.join(", ")
        )
    }
}

fn build_member_lines(member_ids: &[String], notes_by_id: &HashMap<String, Note>) -> Vec<String> {
    if member_ids.is_empty() {
        return vec!["- No notes are assigned yet.".to_string()];
    }

    let mut lines = member_ids
        .iter()
        .filter_map(|member_id| notes_by_id.get(member_id))
        .map(|note| {
            let context = note_context_line(note);
            if context.is_empty() {
                format!("- [[{}]]", note.title)
            } else {
                format!("- [[{}]] — {}", note.title, context)
            }
        })
        .collect::<Vec<_>>();
    lines.sort();
    lines
}

fn build_debate_lines(member_ids: &[String], notes_by_id: &HashMap<String, Note>) -> Vec<String> {
    let members = member_ids.iter().cloned().collect::<HashSet<_>>();
    let ref_to_id = build_note_reference_index(notes_by_id);
    let mut lines = Vec::new();

    for member_id in member_ids {
        let Some(note) = notes_by_id.get(member_id) else {
            continue;
        };
        for parsed_link in &note.parsed_links {
            let Some(target_id) = resolve_link_target(notes_by_id, &ref_to_id, parsed_link) else {
                continue;
            };
            if !members.contains(target_id) {
                continue;
            }
            let Some(target) = notes_by_id.get(target_id) else {
                continue;
            };

            let line = match parsed_link.relation.to_string().as_str() {
                "contradicts" => Some(format!(
                    "- [[{}]] contradicts [[{}]]",
                    note.title, target.title
                )),
                "questions" => Some(format!(
                    "- [[{}]] raises a question for [[{}]]",
                    note.title, target.title
                )),
                "answers" => Some(format!(
                    "- [[{}]] answers a question from [[{}]]",
                    note.title, target.title
                )),
                "supports" => Some(format!(
                    "- [[{}]] supports [[{}]]",
                    note.title, target.title
                )),
                _ => None,
            };

            if let Some(line) = line {
                lines.push(line);
            }
        }
    }

    if lines.is_empty() {
        vec!["- No explicit debates or open questions are linked yet.".to_string()]
    } else {
        lines.sort();
        lines.dedup();
        lines
    }
}

fn build_related_lines(
    related_hub_ids: &[String],
    notes_by_id: &HashMap<String, Note>,
) -> Vec<String> {
    if related_hub_ids.is_empty() {
        return vec!["- No adjacent topics found yet.".to_string()];
    }

    let mut lines = related_hub_ids
        .iter()
        .filter_map(|hub_id| notes_by_id.get(hub_id))
        .map(|hub| format!("- [[{}]]", hub.title))
        .collect::<Vec<_>>();
    lines.sort();
    lines.dedup();
    lines
}

fn create_topic_hub_note(store: &mut KnowledgeStore, topic_key: &str) -> Result<Note> {
    let title = hub_title_from_key(topic_key);
    let mut properties = HashMap::new();
    properties.insert(PROP_IS_TOPIC_HUB.to_string(), Value::Bool(true));
    properties.insert(
        PROP_TOPIC_KEY.to_string(),
        Value::String(topic_key.to_string()),
    );
    properties.insert(
        PROP_TOPIC_ALIASES.to_string(),
        Value::Array(vec![Value::String(topic_key.to_string())]),
    );

    store.create_note(NoteCreate {
        title,
        content: String::new(),
        relative_path: None,
        aliases: vec![display_topic_name(topic_key)],
        status: NoteStatus::Draft,
        tags: vec![
            "hub".to_string(),
            "grafyn".to_string(),
            topic_key.to_string(),
        ],
        schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
        migration_source: Some("topic_hub".to_string()),
        optimizer_managed: true,
        properties,
    })
}

fn persist_if_changed(
    store: &mut KnowledgeStore,
    current: &Note,
    desired: Note,
) -> Result<Option<Note>> {
    if notes_equal(current, &desired) {
        return Ok(None);
    }

    let updated = store.update_note(
        &current.id,
        NoteUpdate {
            title: (current.title != desired.title).then_some(desired.title.clone()),
            content: (current.content != desired.content).then_some(desired.content.clone()),
            relative_path: (current.relative_path != desired.relative_path)
                .then_some(desired.relative_path.clone()),
            aliases: (current.aliases != desired.aliases).then_some(desired.aliases.clone()),
            status: (current.status != desired.status).then_some(desired.status.clone()),
            tags: (current.tags != desired.tags).then_some(desired.tags.clone()),
            schema_version: (current.schema_version != desired.schema_version)
                .then_some(desired.schema_version),
            migration_source: if current.migration_source != desired.migration_source {
                desired.migration_source.clone()
            } else {
                None
            },
            optimizer_managed: (current.optimizer_managed != desired.optimizer_managed)
                .then_some(desired.optimizer_managed),
            properties: (current.properties != desired.properties)
                .then_some(desired.properties.clone()),
        },
    )?;

    Ok(Some(updated))
}

fn notes_equal(left: &Note, right: &Note) -> bool {
    left.title == right.title
        && left.content == right.content
        && left.status == right.status
        && left.tags == right.tags
        && left.properties == right.properties
}

fn candidate_topic_seeds(note: &Note) -> Vec<String> {
    let mut seeds = note
        .tags
        .iter()
        .filter(|tag| !is_structural_tag(tag))
        .map(|tag| normalize_topic_key(tag))
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();

    if seeds.is_empty() {
        if let Some(topic_key) = note.topic_key() {
            let normalized = normalize_topic_key(&topic_key);
            if !normalized.is_empty() {
                seeds.push(normalized);
            }
        }
    }

    sorted_unique(seeds)
}

fn note_context_line(note: &Note) -> String {
    note.content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("- "))
        .map(|line| truncate_text(line, 120))
        .next()
        .unwrap_or_default()
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let truncated = trimmed.chars().take(max_chars).collect::<String>();
    format!("{}...", truncated.trim_end())
}

fn fuzzy_topic_score(left: &str, right: &str) -> f64 {
    if left == right {
        return 1.0;
    }
    if left.contains(right) || right.contains(left) {
        return 0.82;
    }

    let left_tokens = topic_tokens(left);
    let right_tokens = topic_tokens(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0.0;
    }

    let overlap = left_tokens.intersection(&right_tokens).count() as f64;
    let union = left_tokens.union(&right_tokens).count().max(1) as f64;
    overlap / union
}

fn build_note_reference_index(notes_by_id: &HashMap<String, Note>) -> HashMap<String, String> {
    let mut refs = HashMap::new();
    for note in notes_by_id.values() {
        refs.entry(note.title.to_lowercase())
            .or_insert_with(|| note.id.clone());
        refs.entry(note.relative_path.to_lowercase())
            .or_insert_with(|| note.id.clone());
        for alias in &note.aliases {
            refs.entry(alias.to_lowercase())
                .or_insert_with(|| note.id.clone());
        }
    }
    refs
}

fn resolve_link_target<'a>(
    notes_by_id: &'a HashMap<String, Note>,
    ref_to_id: &'a HashMap<String, String>,
    parsed_link: &crate::models::note::ParsedLink,
) -> Option<&'a String> {
    if let Some(target_path) = &parsed_link.target_path {
        if let Some(target_id) = ref_to_id.get(&target_path.to_lowercase()) {
            return Some(target_id);
        }
    }

    if let Some(target_id) = ref_to_id.get(&parsed_link.target_title.to_lowercase()) {
        return Some(target_id);
    }

    notes_by_id
        .values()
        .find(|note| note.title.eq_ignore_ascii_case(&parsed_link.target_title))
        .map(|note| &note.id)
}

fn topic_tokens(value: &str) -> BTreeSet<String> {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn bump_pair(
    scores: &mut HashMap<(String, String), usize>,
    left: &str,
    right: &str,
    amount: usize,
) {
    let key = if left <= right {
        (left.to_string(), right.to_string())
    } else {
        (right.to_string(), left.to_string())
    };
    *scores.entry(key).or_default() += amount;
}

fn ensure_tag(tags: &mut Vec<String>, tag: &str) {
    if !tags.iter().any(|existing| existing == tag) {
        tags.push(tag.to_string());
    }
}

fn sorted_unique(values: Vec<String>) -> Vec<String> {
    let mut cleaned = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    cleaned.sort();
    cleaned.dedup();
    cleaned
}

fn strip_hub_prefix(title: &str) -> String {
    title
        .trim()
        .strip_prefix("Hub: ")
        .unwrap_or(title.trim())
        .to_string()
}

fn hub_title_from_key(topic_key: &str) -> String {
    let display = topic_key
        .split('-')
        .filter(|part| !part.is_empty())
        .map(title_case_word)
        .collect::<Vec<_>>()
        .join(" ");
    format!("Hub: {}", display)
}

fn title_case_word(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };

    format!(
        "{}{}",
        first.to_ascii_uppercase(),
        chars.as_str().to_ascii_lowercase()
    )
}

fn is_structural_tag(tag: &str) -> bool {
    matches!(
        tag.trim().to_ascii_lowercase().as_str(),
        "hub"
            | "grafyn"
            | "draft"
            | "evidence"
            | "canonical"
            | "import"
            | "chatgpt"
            | "claude"
            | "gemini"
            | "grok"
            | "chat"
            | "canvas-export"
            | "ai-generated"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_note(
        store: &mut KnowledgeStore,
        title: &str,
        content: &str,
        tags: &[&str],
    ) -> Result<Note> {
        store.create_note(NoteCreate {
            title: title.to_string(),
            content: content.to_string(),
            relative_path: None,
            aliases: Vec::new(),
            status: NoteStatus::Draft,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            properties: HashMap::new(),
        })
    }

    #[test]
    fn normalizes_topic_keys() {
        assert_eq!(normalize_topic_key("Rust / WASM"), "rust-wasm");
        assert_eq!(normalize_topic_key("  AI Research "), "ai-research");
    }

    #[test]
    fn reuses_existing_hub_on_exact_match() -> Result<()> {
        let dir = tempdir()?;
        let mut store = KnowledgeStore::new(dir.path().to_path_buf(), dir.path().to_path_buf());
        let mut hub_props = HashMap::new();
        hub_props.insert(PROP_IS_TOPIC_HUB.to_string(), Value::Bool(true));
        hub_props.insert(
            PROP_TOPIC_KEY.to_string(),
            Value::String("rust".to_string()),
        );
        store.create_note(NoteCreate {
            title: "Hub: Rust".to_string(),
            content: String::new(),
            relative_path: None,
            aliases: Vec::new(),
            status: NoteStatus::Draft,
            tags: vec!["hub".to_string(), "grafyn".to_string()],
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: true,
            properties: hub_props,
        })?;
        let note = write_note(&mut store, "Ownership", "Rust ownership rules.", &["rust"])?;

        let result = sync_topic_hubs(&mut store)?;
        let updated = store.get_note(&note.id)?;
        let hubs = updated.topic_hub_ids();

        assert_eq!(hubs.len(), 1);
        assert_eq!(
            result
                .all_notes
                .iter()
                .filter(|note| note.is_topic_hub())
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn creates_new_hub_for_missing_topic() -> Result<()> {
        let dir = tempdir()?;
        let mut store = KnowledgeStore::new(dir.path().to_path_buf(), dir.path().to_path_buf());
        let note = write_note(
            &mut store,
            "Memory Safety",
            "Rust focuses on memory safety.",
            &["rust"],
        )?;

        let result = sync_topic_hubs(&mut store)?;
        let updated = store.get_note(&note.id)?;

        assert_eq!(updated.topic_hub_ids().len(), 1);
        assert!(result
            .all_notes
            .iter()
            .any(|candidate| candidate.is_topic_hub()
                && candidate.topic_key().as_deref() == Some("rust")));
        Ok(())
    }

    #[test]
    fn reassigns_note_when_topics_change() -> Result<()> {
        let dir = tempdir()?;
        let mut store = KnowledgeStore::new(dir.path().to_path_buf(), dir.path().to_path_buf());
        let note = write_note(
            &mut store,
            "Systems Note",
            "Systems programming ideas.",
            &["rust"],
        )?;
        sync_topic_hubs(&mut store)?;

        store.update_note(
            &note.id,
            NoteUpdate {
                tags: Some(vec!["python".to_string()]),
                ..Default::default()
            },
        )?;

        sync_topic_hubs(&mut store)?;
        let updated = store.get_note(&note.id)?;
        let hubs = updated.topic_hub_ids();

        assert_eq!(hubs.len(), 1);
        let hub = store.get_note(&hubs[0])?;
        assert_eq!(hub.topic_key().as_deref(), Some("python"));
        Ok(())
    }

    #[test]
    fn hub_content_summarizes_members_and_debates() -> Result<()> {
        let dir = tempdir()?;
        let mut store = KnowledgeStore::new(dir.path().to_path_buf(), dir.path().to_path_buf());
        write_note(
            &mut store,
            "Claim A",
            "Claim A body.\n- [[Claim B]] (contradicts)",
            &["research"],
        )?;
        write_note(&mut store, "Claim B", "Claim B body.", &["research"])?;

        sync_topic_hubs(&mut store)?;
        let hub = store
            .list_full_notes()?
            .into_iter()
            .find(|note| note.is_topic_hub())
            .expect("hub should exist");

        assert!(hub.content.contains("## Summary"));
        assert!(hub.content.contains("## Notes In This Topic"));
        assert!(hub.content.contains("## Debates And Questions"));
        assert!(hub.content.contains("contradicts"));
        Ok(())
    }
}
