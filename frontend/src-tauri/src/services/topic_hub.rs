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
        let normalized = canonical_topic_key(seed)?;

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
    let mut removed_note_ids = BTreeSet::new();

    let existing_hub_ids = notes_by_id
        .values()
        .filter(|note| note.is_topic_hub())
        .map(|note| note.id.clone())
        .collect::<Vec<_>>();

    for hub_id in existing_hub_ids {
        let Some(current) = notes_by_id.get(&hub_id).cloned() else {
            continue;
        };
        if should_remove_noisy_auto_hub(&current) {
            store.delete_note(&current.id)?;
            notes_by_id.remove(&current.id);
            removed_note_ids.insert(current.id);
            continue;
        }

        let desired = standardize_existing_hub(&current);
        if let Some(updated) = persist_if_changed(store, &current, desired)? {
            changed_note_ids.insert(updated.id.clone());
            notes_by_id.insert(updated.id.clone(), updated);
        }
    }

    for removed_id in remove_duplicate_auto_hubs(store, &mut notes_by_id)? {
        removed_note_ids.insert(removed_id);
    }

    let mut registry = build_hub_registry(notes_by_id.values());
    let graph_cluster_seeds = infer_graph_cluster_topic_seeds(&notes_by_id);

    let regular_note_ids = notes_by_id
        .values()
        .filter(|note| !note.is_topic_hub())
        .map(|note| note.id.clone())
        .collect::<Vec<_>>();

    for note_id in regular_note_ids {
        let Some(current) = notes_by_id.get(&note_id).cloned() else {
            continue;
        };

        let topic_seeds = candidate_topic_seeds(&current, graph_cluster_seeds.get(&current.id));
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
        removed_note_ids: removed_note_ids.into_iter().collect(),
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

pub fn suggest_topic_hub_title(value: &str) -> Option<String> {
    canonical_topic_key(value).map(|topic_key| hub_title_from_key(&topic_key))
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
    let raw_topic_key = raw_topic_key_for_hub(note);
    let topic_key = canonical_topic_key(&raw_topic_key).unwrap_or(raw_topic_key.clone());

    let mut tags = note.tags.clone();
    ensure_tag(&mut tags, "hub");
    ensure_tag(&mut tags, "grafyn");
    if !topic_key.is_empty() {
        ensure_tag(&mut tags, &topic_key);
    }
    desired.tags = sorted_unique(tags);
    if is_auto_topic_hub(note) && !topic_key.is_empty() {
        desired.title = hub_title_from_key(&topic_key);
        desired.aliases = sorted_unique(vec![
            display_topic_name(&topic_key),
            display_topic_name(&raw_topic_key),
        ]);
    }
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
            raw_topic_key,
            normalize_topic_key(&strip_hub_prefix(&note.title)),
        ]),
    );
    desired
}

fn remove_duplicate_auto_hubs(
    store: &mut KnowledgeStore,
    notes_by_id: &mut HashMap<String, Note>,
) -> Result<Vec<String>> {
    let mut by_topic_key: HashMap<String, Vec<String>> = HashMap::new();
    for note in notes_by_id.values().filter(|note| note.is_topic_hub()) {
        if let Some(topic_key) = note.topic_key() {
            by_topic_key
                .entry(topic_key)
                .or_default()
                .push(note.id.clone());
        }
    }

    let mut removed = Vec::new();
    for (topic_key, mut hub_ids) in by_topic_key {
        if hub_ids.len() < 2 {
            continue;
        }

        hub_ids.sort_by(|left, right| {
            let left_note = notes_by_id.get(left);
            let right_note = notes_by_id.get(right);
            hub_keep_score(left_note, &topic_key)
                .cmp(&hub_keep_score(right_note, &topic_key))
                .then_with(|| left.cmp(right))
        });

        for duplicate_id in hub_ids.into_iter().skip(1) {
            let Some(duplicate) = notes_by_id.get(&duplicate_id).cloned() else {
                continue;
            };
            if !is_auto_topic_hub(&duplicate) {
                continue;
            }

            store.delete_note(&duplicate.id)?;
            notes_by_id.remove(&duplicate.id);
            removed.push(duplicate.id);
        }
    }

    Ok(removed)
}

fn hub_keep_score(note: Option<&Note>, topic_key: &str) -> (u8, u8) {
    let Some(note) = note else {
        return (3, 3);
    };
    let expected_title = hub_title_from_key(topic_key);
    (
        if is_auto_topic_hub(note) { 1 } else { 0 },
        if note.title == expected_title { 0 } else { 1 },
    )
}

fn infer_graph_cluster_topic_seeds(notes_by_id: &HashMap<String, Note>) -> HashMap<String, String> {
    let regular_notes = notes_by_id
        .values()
        .filter(|note| !note.is_topic_hub())
        .collect::<Vec<_>>();
    if regular_notes.len() < 3 {
        return HashMap::new();
    }

    let adjacency = build_graph_cluster_adjacency(notes_by_id, &regular_notes);
    if adjacency.is_empty() {
        return HashMap::new();
    }

    let communities = label_propagation_communities(&regular_notes, &adjacency);
    let mut seeds = HashMap::new();
    for member_ids in communities.values() {
        if member_ids.len() < 3 {
            continue;
        }
        let Some(topic_key) = choose_cluster_topic_key(member_ids, notes_by_id) else {
            continue;
        };
        for member_id in member_ids {
            seeds.insert(member_id.clone(), topic_key.clone());
        }
    }

    seeds
}

fn build_graph_cluster_adjacency(
    notes_by_id: &HashMap<String, Note>,
    regular_notes: &[&Note],
) -> HashMap<String, HashMap<String, usize>> {
    let regular_ids = regular_notes
        .iter()
        .map(|note| note.id.clone())
        .collect::<HashSet<_>>();
    let ref_to_id = build_note_reference_index(notes_by_id);
    let mut adjacency = HashMap::<String, HashMap<String, usize>>::new();

    for note in regular_notes {
        for parsed_link in &note.parsed_links {
            let Some(target_id) = resolve_link_target(notes_by_id, &ref_to_id, parsed_link) else {
                continue;
            };
            if !regular_ids.contains(target_id) || target_id == &note.id {
                continue;
            }
            add_cluster_edge(&mut adjacency, &note.id, target_id, 5);
        }
    }

    let mut notes_by_seed = HashMap::<String, Vec<String>>::new();
    for note in regular_notes {
        for seed in graph_cluster_note_seeds(note) {
            notes_by_seed.entry(seed).or_default().push(note.id.clone());
        }
    }

    for member_ids in notes_by_seed.values_mut() {
        member_ids.sort();
        member_ids.dedup();
        if member_ids.len() < 2 || member_ids.len() > 80 {
            continue;
        }
        for left_index in 0..member_ids.len() {
            for right_index in (left_index + 1)..member_ids.len() {
                add_cluster_edge(
                    &mut adjacency,
                    &member_ids[left_index],
                    &member_ids[right_index],
                    2,
                );
            }
        }
    }

    adjacency
}

fn label_propagation_communities(
    regular_notes: &[&Note],
    adjacency: &HashMap<String, HashMap<String, usize>>,
) -> HashMap<String, Vec<String>> {
    let mut labels = regular_notes
        .iter()
        .map(|note| (note.id.clone(), note.id.clone()))
        .collect::<HashMap<_, _>>();
    let mut ordered_ids = regular_notes
        .iter()
        .map(|note| note.id.clone())
        .collect::<Vec<_>>();
    ordered_ids.sort();

    for _ in 0..8 {
        let mut changed = false;
        for note_id in &ordered_ids {
            let Some(neighbors) = adjacency.get(note_id) else {
                continue;
            };

            let mut scores = HashMap::<String, usize>::new();
            for (neighbor_id, weight) in neighbors {
                let Some(label) = labels.get(neighbor_id) else {
                    continue;
                };
                *scores.entry(label.clone()).or_default() += *weight;
            }

            let Some((best_label, _)) = scores
                .into_iter()
                .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
            else {
                continue;
            };

            if labels.get(note_id) != Some(&best_label) {
                labels.insert(note_id.clone(), best_label);
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    let mut communities = HashMap::<String, Vec<String>>::new();
    for (note_id, label) in labels {
        communities.entry(label).or_default().push(note_id);
    }
    for member_ids in communities.values_mut() {
        member_ids.sort();
    }
    communities
}

fn choose_cluster_topic_key(
    member_ids: &[String],
    notes_by_id: &HashMap<String, Note>,
) -> Option<String> {
    let mut counts = HashMap::<String, usize>::new();
    for member_id in member_ids {
        let Some(note) = notes_by_id.get(member_id) else {
            continue;
        };
        for seed in graph_cluster_note_seeds(note) {
            *counts.entry(seed).or_default() += 1;
        }
    }

    counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .max_by(|left, right| {
            left.1
                .cmp(&right.1)
                .then_with(|| right.0.len().cmp(&left.0.len()))
                .then_with(|| right.0.cmp(&left.0))
        })
        .map(|(seed, _)| seed)
}

fn graph_cluster_note_seeds(note: &Note) -> Vec<String> {
    let mut seeds = note
        .tags
        .iter()
        .filter(|tag| !is_structural_tag(tag))
        .filter_map(|tag| canonical_topic_key(tag))
        .collect::<Vec<_>>();

    seeds.extend(
        ordered_topic_tokens(&note.title)
            .into_iter()
            .filter(|token| is_graph_cluster_token(token))
            .filter_map(|token| canonical_topic_key(&token)),
    );

    sorted_unique(seeds)
}

fn add_cluster_edge(
    adjacency: &mut HashMap<String, HashMap<String, usize>>,
    left: &str,
    right: &str,
    weight: usize,
) {
    if left == right {
        return;
    }
    *adjacency
        .entry(left.to_string())
        .or_default()
        .entry(right.to_string())
        .or_default() += weight;
    *adjacency
        .entry(right.to_string())
        .or_default()
        .entry(left.to_string())
        .or_default() += weight;
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
    let subtopic_lines = build_subtopic_lines(&topic_key, member_ids, notes_by_id);
    let member_lines = build_member_lines(member_ids, notes_by_id);
    let debate_lines = build_debate_lines(member_ids, notes_by_id);
    let related_lines = build_related_lines(related_hub_ids, notes_by_id);

    let mut content = Vec::new();
    content.push(format!("# {}", display_name));
    content.push(String::new());
    content.push("## Summary".to_string());
    content.push(summary);
    content.push(String::new());
    content.push("## Subtopics".to_string());
    content.extend(subtopic_lines);
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

fn build_subtopic_lines(
    topic_key: &str,
    member_ids: &[String],
    notes_by_id: &HashMap<String, Note>,
) -> Vec<String> {
    let mut counts = HashMap::<String, usize>::new();
    for member_id in member_ids {
        let Some(note) = notes_by_id.get(member_id) else {
            continue;
        };
        for tag in &note.tags {
            if is_structural_tag(tag) {
                continue;
            }
            let normalized = normalize_topic_key(tag);
            if normalized.is_empty() || normalized == topic_key {
                continue;
            }
            *counts.entry(normalized).or_default() += 1;
        }
    }

    if counts.is_empty() {
        return vec!["- No stable subtopics detected yet.".to_string()];
    }

    let mut ordered = counts.into_iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    ordered
        .into_iter()
        .take(8)
        .map(|(tag, count)| format!("- {} ({})", display_topic_name(&tag), count))
        .collect()
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
        relative_path: Some(format!("_grafyn/hubs/{}/index.md", topic_key)),
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
        && left.relative_path == right.relative_path
        && left.aliases == right.aliases
        && left.content == right.content
        && left.status == right.status
        && left.tags == right.tags
        && left.schema_version == right.schema_version
        && left.migration_source == right.migration_source
        && left.optimizer_managed == right.optimizer_managed
        && left.properties == right.properties
}

fn candidate_topic_seeds(note: &Note, graph_cluster_seed: Option<&String>) -> Vec<String> {
    if let Some(seed) = graph_cluster_seed.and_then(|seed| canonical_topic_key(seed)) {
        return vec![seed];
    }

    let mut seeds = note
        .tags
        .iter()
        .filter(|tag| !is_structural_tag(tag))
        .filter_map(|tag| canonical_topic_key(tag))
        .collect::<Vec<_>>();

    if seeds.is_empty() {
        if let Some(topic_key) = note.topic_key() {
            if let Some(normalized) = canonical_topic_key(&topic_key) {
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
    if matches!(
        word,
        "ai" | "dai" | "llm" | "mcp" | "rag" | "ui" | "ux" | "vr"
    ) {
        return word.to_ascii_uppercase();
    }

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

fn raw_topic_key_for_hub(note: &Note) -> String {
    note.topic_key()
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
        })
}

fn canonical_topic_key(value: &str) -> Option<String> {
    let normalized = normalize_topic_key(&strip_hub_prefix(value));
    if normalized.is_empty() {
        return None;
    }

    let tokens = ordered_topic_tokens(&normalized);
    if tokens.is_empty() || tokens.iter().all(|token| is_generic_topic_token(token)) {
        return None;
    }
    if is_model_only_topic(&tokens) || is_transcript_artifact_topic(&tokens) {
        return None;
    }
    if tokens.len() == 1 && tokens[0].len() <= 3 && !is_allowed_short_topic(&tokens[0]) {
        return None;
    }

    if tokens.iter().any(|token| token == "dai") {
        return Some("dai".to_string());
    }
    if tokens
        .iter()
        .any(|token| matches!(token.as_str(), "course" | "courses" | "credit" | "transfer"))
    {
        return Some("courses".to_string());
    }
    if tokens
        .iter()
        .any(|token| matches!(token.as_str(), "design" | "immersive" | "vr"))
    {
        return Some("design".to_string());
    }
    if tokens
        .iter()
        .any(|token| matches!(token.as_str(), "project" | "task" | "tasks"))
    {
        return Some("tasks".to_string());
    }
    if tokens.iter().any(|token| token == "device") {
        return Some("devices".to_string());
    }
    if tokens
        .iter()
        .any(|token| matches!(token.as_str(), "token" | "tokens"))
    {
        return Some("tokens".to_string());
    }
    if tokens
        .iter()
        .any(|token| matches!(token.as_str(), "llm" | "model" | "models" | "openrouter"))
    {
        return Some("ai-models".to_string());
    }
    if tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "decision" | "decisions" | "choice" | "choices"
        )
    }) {
        return Some("decision-making".to_string());
    }

    Some(normalized)
}

fn ordered_topic_tokens(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn is_allowed_short_topic(token: &str) -> bool {
    matches!(token, "ai" | "dai" | "mcp" | "rag" | "ui" | "ux" | "vr")
}

fn is_graph_cluster_token(token: &str) -> bool {
    token.len() >= 4 && !is_generic_topic_token(token) && !is_model_or_provider_token(token)
}

fn is_model_only_topic(tokens: &[String]) -> bool {
    if tokens.len() <= 2
        && tokens.iter().any(|token| is_model_or_provider_token(token))
        && tokens.iter().all(|token| {
            is_model_or_provider_token(token)
                || matches!(token.as_str(), "api" | "assistant" | "code")
        })
    {
        return true;
    }

    tokens
        .iter()
        .all(|token| is_model_or_provider_token(token) || is_generic_topic_token(token))
}

fn is_model_or_provider_token(token: &str) -> bool {
    matches!(
        token,
        "anthropic"
            | "claude"
            | "codex"
            | "copilot"
            | "cursor"
            | "gemini"
            | "gpt"
            | "grok"
            | "llama"
            | "mistral"
            | "openai"
            | "qwen"
            | "sonnet"
    )
}

fn is_transcript_artifact_topic(tokens: &[String]) -> bool {
    tokens
        .iter()
        .any(|token| token.chars().all(|c| c.is_ascii_digit()))
        && tokens
            .iter()
            .any(|token| matches!(token.as_str(), "message" | "assistant" | "user"))
}

fn is_generic_topic_token(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "answer"
            | "answers"
            | "approach"
            | "assistant"
            | "best"
            | "chat"
            | "comparison"
            | "correct"
            | "create"
            | "define"
            | "detailed"
            | "different"
            | "draft"
            | "example"
            | "good"
            | "hub"
            | "insight"
            | "message"
            | "note"
            | "notes"
            | "previous"
            | "question"
            | "response"
            | "results"
            | "since"
            | "summary"
            | "sure"
            | "task"
            | "the"
            | "topic"
            | "user"
            | "whole"
    )
}

fn is_auto_topic_hub(note: &Note) -> bool {
    note.optimizer_managed || note.migration_source.as_deref() == Some("topic_hub")
}

fn should_remove_noisy_auto_hub(note: &Note) -> bool {
    is_auto_topic_hub(note) && canonical_topic_key(&raw_topic_key_for_hub(note)).is_none()
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
            | "anthropic"
            | "openai"
            | "gemini"
            | "grok"
            | "qwen"
            | "chat"
            | "assistant"
            | "user"
            | "message"
            | "model"
            | "models"
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
    fn canonicalizes_noisy_minor_topics_into_major_hubs() {
        assert_eq!(
            suggest_topic_hub_title("dai-asd").as_deref(),
            Some("Hub: DAI")
        );
        assert_eq!(
            suggest_topic_hub_title("dai-courses").as_deref(),
            Some("Hub: DAI")
        );
        assert_eq!(
            suggest_topic_hub_title("design-immersive").as_deref(),
            Some("Hub: Design")
        );
        assert_eq!(
            suggest_topic_hub_title("design-vr").as_deref(),
            Some("Hub: Design")
        );
        assert_eq!(suggest_topic_hub_title("claude"), None);
        assert_eq!(suggest_topic_hub_title("message-12-user"), None);
        assert_eq!(suggest_topic_hub_title("create-detailed"), None);
        assert_eq!(suggest_topic_hub_title("ddd"), None);
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
    fn merges_duplicate_auto_hubs_by_canonical_topic() -> Result<()> {
        let dir = tempdir()?;
        let mut store = KnowledgeStore::new(dir.path().to_path_buf(), dir.path().to_path_buf());
        write_note(
            &mut store,
            "DAI course option",
            "DAI course notes.",
            &["dai-courses"],
        )?;
        write_note(
            &mut store,
            "DAI assessment",
            "DAI assessment notes.",
            &["dai-asd"],
        )?;

        let result = sync_topic_hubs(&mut store)?;
        let hubs = result
            .all_notes
            .iter()
            .filter(|note| note.is_topic_hub())
            .collect::<Vec<_>>();

        assert_eq!(hubs.len(), 1);
        assert_eq!(hubs[0].topic_key().as_deref(), Some("dai"));
        assert_eq!(hubs[0].title, "Hub: DAI");
        Ok(())
    }

    #[test]
    fn does_not_create_model_name_hubs() -> Result<()> {
        let dir = tempdir()?;
        let mut store = KnowledgeStore::new(dir.path().to_path_buf(), dir.path().to_path_buf());
        let note = write_note(
            &mut store,
            "Claude output",
            "Claude response review.",
            &["claude"],
        )?;

        let result = sync_topic_hubs(&mut store)?;
        let updated = store.get_note(&note.id)?;

        assert!(updated.topic_hub_ids().is_empty());
        assert!(result.all_notes.iter().all(|note| !note.is_topic_hub()));
        Ok(())
    }

    #[test]
    fn graph_clustering_prefers_major_community_hub_over_narrow_tags() -> Result<()> {
        let dir = tempdir()?;
        let mut store = KnowledgeStore::new(dir.path().to_path_buf(), dir.path().to_path_buf());
        let market = write_note(
            &mut store,
            "Alpha Market",
            "Market signal.\n- [[Alpha Budget]] (supports)",
            &["pricing"],
        )?;
        let budget = write_note(
            &mut store,
            "Alpha Budget",
            "Budget signal.\n- [[Alpha Timeline]] (supports)",
            &["finance"],
        )?;
        let timeline = write_note(
            &mut store,
            "Alpha Timeline",
            "Timeline signal.",
            &["planning"],
        )?;

        let result = sync_topic_hubs(&mut store)?;
        let hubs = result
            .all_notes
            .iter()
            .filter(|note| note.is_topic_hub())
            .collect::<Vec<_>>();

        assert_eq!(hubs.len(), 1);
        assert_eq!(hubs[0].topic_key().as_deref(), Some("alpha"));
        assert_eq!(hubs[0].title, "Hub: Alpha");

        for note in [market, budget, timeline] {
            let updated = store.get_note(&note.id)?;
            assert_eq!(updated.topic_hub_ids().len(), 1);
            let hub = store.get_note(&updated.topic_hub_ids()[0])?;
            assert_eq!(hub.topic_key().as_deref(), Some("alpha"));
        }
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
        assert!(hub.content.contains("## Subtopics"));
        assert!(hub.content.contains("## Notes In This Topic"));
        assert!(hub.content.contains("## Debates And Questions"));
        assert!(hub.content.contains("contradicts"));
        Ok(())
    }
}
