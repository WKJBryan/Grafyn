use crate::models::note::{
    GraphEdgeKind, GraphEdgeProvenance, GraphNeighbor, GraphNodeKind, LinkDirection, Note,
    NoteMeta, ParsedLink, RelationType, TypedEdge,
};
use std::collections::{HashMap, HashSet};

/// Service for managing the note link graph (backlinks and outgoing links)
#[derive(Debug, Clone, Default)]
pub struct GraphIndex {
    /// Cached full notes so incremental updates can rebuild the graph accurately.
    notes: HashMap<String, Note>,
    /// Map from note ID to typed edges it links to
    outgoing: HashMap<String, Vec<TypedEdge>>,
    /// Map from note ID to typed edges that link to it (with reverse relation)
    backlinks: HashMap<String, Vec<TypedEdge>>,
    /// Map from note title to note ID for resolving wikilinks
    title_to_id: HashMap<String, String>,
    /// Map aliases to note IDs for resolving plain-text mentions
    alias_to_id: HashMap<String, String>,
    /// Map relative markdown paths to note IDs
    path_to_id: HashMap<String, String>,
    /// Cached note metadata for quick lookups
    note_meta: HashMap<String, NoteMeta>,
    /// Regular note ID -> topic hub IDs
    topic_memberships: HashMap<String, Vec<String>>,
    /// Topic hub ID -> member note IDs
    topic_hub_members: HashMap<String, Vec<String>>,
    /// Topic hub ID -> adjacent topic hub IDs
    topic_related: HashMap<String, Vec<String>>,
}

impl GraphIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the graph index from a list of notes
    pub fn build_index(&mut self, notes: &[NoteMeta]) {
        self.clear();

        // Build title-to-ID mapping
        for note in notes {
            self.title_to_id
                .insert(note.title.to_lowercase(), note.id.clone());
            self.path_to_id
                .insert(note.relative_path.to_lowercase(), note.id.clone());
            for alias in &note.aliases {
                self.alias_to_id
                    .entry(alias.to_lowercase())
                    .or_insert_with(|| note.id.clone());
            }
            self.note_meta.insert(note.id.clone(), note.clone());
        }
    }

    /// Build the graph from full notes (with wikilinks)
    pub fn build_from_notes(&mut self, notes: &[Note]) {
        self.clear();
        for note in notes {
            self.notes.insert(note.id.clone(), note.clone());
        }
        self.rebuild_from_cached_notes();
    }

    fn rebuild_from_cached_notes(&mut self) {
        self.outgoing.clear();
        self.backlinks.clear();
        self.title_to_id.clear();
        self.alias_to_id.clear();
        self.path_to_id.clear();
        self.note_meta.clear();
        self.topic_memberships.clear();
        self.topic_hub_members.clear();
        self.topic_related.clear();

        for note in self.notes.values() {
            self.title_to_id
                .insert(note.title.to_lowercase(), note.id.clone());
            self.path_to_id
                .insert(note.relative_path.to_lowercase(), note.id.clone());
            for alias in &note.aliases {
                self.alias_to_id
                    .entry(alias.to_lowercase())
                    .or_insert_with(|| note.id.clone());
            }
            self.note_meta.insert(note.id.clone(), NoteMeta::from(note));
        }

        for note in self.notes.values() {
            let auto_inserted = note.auto_inserted_link_ids();
            let explicit_targets = note
                .parsed_links
                .iter()
                .filter_map(|parsed_link| {
                    self.resolve_target_id(parsed_link).map(|target_id| {
                        let provenance = if auto_inserted.iter().any(|id| id == &target_id) {
                            GraphEdgeProvenance::AutoInserted
                        } else {
                            GraphEdgeProvenance::Explicit
                        };
                        (target_id, parsed_link.relation.clone(), provenance)
                    })
                })
                .collect::<Vec<_>>();
            let outgoing_edges = self.outgoing.entry(note.id.clone()).or_default();

            for (target_id, relation, provenance) in explicit_targets {
                if !outgoing_edges.iter().any(|edge| edge.target_id == target_id) {
                    outgoing_edges.push(TypedEdge {
                        target_id: target_id.clone(),
                        relation: relation.clone(),
                        provenance: provenance.clone(),
                    });

                    self.backlinks
                        .entry(target_id.clone())
                        .or_default()
                        .push(TypedEdge {
                            target_id: note.id.clone(),
                            relation: relation.reverse(),
                            provenance,
                        });
                }
            }

            for inferred_target_id in note.inferred_link_ids() {
                if inferred_target_id == note.id
                    || outgoing_edges
                        .iter()
                        .any(|edge| edge.target_id == inferred_target_id)
                {
                    continue;
                }

                outgoing_edges.push(TypedEdge {
                    target_id: inferred_target_id.clone(),
                    relation: RelationType::Untyped,
                    provenance: GraphEdgeProvenance::Inferred,
                });
                self.backlinks
                    .entry(inferred_target_id)
                    .or_default()
                    .push(TypedEdge {
                        target_id: note.id.clone(),
                        relation: RelationType::Untyped,
                        provenance: GraphEdgeProvenance::Inferred,
                    });
            }
        }

        let hub_ids = self
            .notes
            .values()
            .filter(|note| note.is_topic_hub())
            .map(|note| note.id.clone())
            .collect::<HashSet<_>>();

        for note in self.notes.values().filter(|note| !note.is_topic_hub()) {
            let membership = note
                .topic_hub_ids()
                .into_iter()
                .filter(|hub_id| hub_ids.contains(hub_id))
                .collect::<Vec<_>>();
            if membership.is_empty() {
                continue;
            }

            self.topic_memberships
                .insert(note.id.clone(), membership.clone());
            for hub_id in membership {
                self.topic_hub_members
                    .entry(hub_id)
                    .or_default()
                    .push(note.id.clone());
            }
        }

        let mut topic_pairs: HashMap<(String, String), usize> = HashMap::new();
        for note in self.notes.values().filter(|note| !note.is_topic_hub()) {
            let source_hubs = self
                .topic_memberships
                .get(&note.id)
                .cloned()
                .unwrap_or_default();
            for index in 0..source_hubs.len() {
                for next in (index + 1)..source_hubs.len() {
                    bump_topic_pair(&mut topic_pairs, &source_hubs[index], &source_hubs[next], 1);
                }
            }

            for parsed_link in &note.parsed_links {
                let Some(target_id) = self.resolve_target_id(parsed_link) else {
                    continue;
                };

                let target_hubs = self.effective_topic_hubs(&target_id);
                for source_hub in &source_hubs {
                    for target_hub in &target_hubs {
                        if source_hub != target_hub {
                            bump_topic_pair(&mut topic_pairs, source_hub, target_hub, 2);
                        }
                    }
                }
            }
        }

        for ((left, right), score) in topic_pairs {
            if score == 0 {
                continue;
            }
            self.topic_related
                .entry(left.clone())
                .or_default()
                .push(right.clone());
            self.topic_related.entry(right).or_default().push(left);
        }

        for neighbors in self.topic_related.values_mut() {
            neighbors.sort();
            neighbors.dedup();
        }
    }

    /// Clear the index
    pub fn clear(&mut self) {
        self.notes.clear();
        self.outgoing.clear();
        self.backlinks.clear();
        self.title_to_id.clear();
        self.alias_to_id.clear();
        self.path_to_id.clear();
        self.note_meta.clear();
        self.topic_memberships.clear();
        self.topic_hub_members.clear();
        self.topic_related.clear();
    }

    /// Add or update a note in the index
    pub fn update_note(&mut self, note: &Note) {
        self.notes.insert(note.id.clone(), note.clone());
        self.rebuild_from_cached_notes();
    }

    /// Remove a note from the index
    pub fn remove_note(&mut self, note_id: &str) {
        self.notes.remove(note_id);
        self.rebuild_from_cached_notes();
    }

    /// Get all notes that link to the given note (backlinks)
    pub fn get_backlinks(&self, note_id: &str) -> Vec<NoteMeta> {
        self.backlinks
            .get(note_id)
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|e| self.note_meta.get(&e.target_id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all notes that the given note links to (outgoing links)
    pub fn get_outgoing(&self, note_id: &str) -> Vec<NoteMeta> {
        self.outgoing
            .get(note_id)
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|e| self.note_meta.get(&e.target_id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get typed backlinks with relationship information
    pub fn get_typed_backlinks(&self, note_id: &str) -> Vec<(NoteMeta, RelationType)> {
        self.backlinks
            .get(note_id)
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|e| {
                        self.note_meta
                            .get(&e.target_id)
                            .map(|meta| (meta.clone(), e.relation.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get typed outgoing links with relationship information
    pub fn get_typed_outgoing(&self, note_id: &str) -> Vec<(NoteMeta, RelationType)> {
        self.outgoing
            .get(note_id)
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|e| {
                        self.note_meta
                            .get(&e.target_id)
                            .map(|meta| (meta.clone(), e.relation.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all neighbors (both backlinks and outgoing) with typed relationships
    pub fn get_neighbors(&self, note_id: &str) -> Vec<GraphNeighbor> {
        let mut neighbors = Vec::new();

        // Add outgoing links
        if let Some(outgoing) = self.outgoing.get(note_id) {
            for edge in outgoing {
                if let Some(meta) = self.note_meta.get(&edge.target_id) {
                    neighbors.push(GraphNeighbor {
                        note: meta.clone(),
                        direction: LinkDirection::Outgoing,
                        relation: edge.relation.clone(),
                    });
                }
            }
        }

        // Add backlinks
        if let Some(backlinks) = self.backlinks.get(note_id) {
            for edge in backlinks {
                if let Some(meta) = self.note_meta.get(&edge.target_id) {
                    neighbors.push(GraphNeighbor {
                        note: meta.clone(),
                        direction: LinkDirection::Backlink,
                        relation: edge.relation.clone(),
                    });
                }
            }
        }

        neighbors
    }

    /// Get notes with no incoming or outgoing links
    pub fn get_unlinked(&self) -> Vec<NoteMeta> {
        self.note_meta
            .iter()
            .filter(|(id, _)| {
                let has_outgoing = self
                    .outgoing
                    .get(*id)
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let has_backlinks = self
                    .backlinks
                    .get(*id)
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let has_topic_membership = self
                    .topic_memberships
                    .get(*id)
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let has_topic_members = self
                    .topic_hub_members
                    .get(*id)
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let has_topic_related = self
                    .topic_related
                    .get(*id)
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                !has_outgoing
                    && !has_backlinks
                    && !has_topic_membership
                    && !has_topic_members
                    && !has_topic_related
            })
            .map(|(_, meta)| meta.clone())
            .collect()
    }

    /// Resolve a wikilink title to a note ID
    pub fn resolve_link(&self, title: &str) -> Option<String> {
        self.title_to_id.get(&title.to_lowercase()).cloned()
    }

    /// Get note metadata by ID
    pub fn get_note_meta(&self, note_id: &str) -> Option<NoteMeta> {
        self.note_meta.get(note_id).cloned()
    }

    pub fn get_note_topic_hub_ids(&self, note_id: &str) -> Vec<String> {
        if self
            .notes
            .get(note_id)
            .map(|note| note.is_topic_hub())
            .unwrap_or(false)
        {
            return vec![note_id.to_string()];
        }

        self.topic_memberships
            .get(note_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn get_note_topic_hubs(&self, note_id: &str) -> Vec<NoteMeta> {
        self.get_note_topic_hub_ids(note_id)
            .into_iter()
            .filter_map(|hub_id| self.note_meta.get(&hub_id).cloned())
            .collect()
    }

    pub fn get_related_topic_hubs(&self, note_id: &str) -> Vec<NoteMeta> {
        let mut related = Vec::new();
        let mut seen = HashSet::new();

        for hub_id in self.get_note_topic_hub_ids(note_id) {
            for related_hub_id in self.topic_related.get(&hub_id).cloned().unwrap_or_default() {
                if seen.insert(related_hub_id.clone()) {
                    if let Some(meta) = self.note_meta.get(&related_hub_id) {
                        related.push(meta.clone());
                    }
                }
            }
        }

        related
    }

    pub fn topic_relation_score(&self, source_note_id: &str, target_note_id: &str) -> f64 {
        let source_hubs = self.effective_topic_hubs(source_note_id);
        let target_hubs = self.effective_topic_hubs(target_note_id);

        if source_hubs.is_empty() || target_hubs.is_empty() {
            return 0.0;
        }

        if source_hubs
            .iter()
            .any(|hub_id| target_hubs.contains(hub_id))
        {
            return 1.0;
        }

        for source_hub in &source_hubs {
            for target_hub in &target_hubs {
                if self
                    .topic_related
                    .get(source_hub)
                    .map(|neighbors| neighbors.contains(target_hub))
                    .unwrap_or(false)
                {
                    return 0.55;
                }
            }
        }

        0.0
    }

    /// Get the full graph structure for visualization
    pub fn get_full_graph(&self) -> FullGraph {
        let mut nodes = Vec::new();
        let mut links = Vec::new();

        // Build nodes from all known notes
        for (id, meta) in &self.note_meta {
            let backlink_count = self.backlinks.get(id).map(|s| s.len()).unwrap_or(0);
            let is_topic_hub = self
                .notes
                .get(id)
                .map(|note| note.is_topic_hub())
                .unwrap_or(false);
            let topic_key = self.notes.get(id).and_then(|note| note.topic_key());

            nodes.push(GraphNode {
                id: id.clone(),
                label: meta.title.clone(),
                val: backlink_count + 1,
                note_type: if is_topic_hub {
                    "hub".to_string()
                } else {
                    "general".to_string()
                },
                tags: meta.tags.clone(),
                group: if is_topic_hub {
                    "#f59e0b".to_string()
                } else {
                    "#6b7280".to_string()
                },
                node_kind: if is_topic_hub {
                    GraphNodeKind::TopicHub
                } else {
                    GraphNodeKind::Note
                },
                topic_key,
            });
        }

        // Build links from outgoing map
        for (source, edges) in &self.outgoing {
            for edge in edges {
                if self.is_topic_membership_pair(source, &edge.target_id) {
                    continue;
                }
                links.push(GraphLink {
                    source: source.clone(),
                    target: edge.target_id.clone(),
                    relation: edge.relation.to_string(),
                    edge_kind: GraphEdgeKind::NoteLink,
                    provenance: edge.provenance.clone(),
                });
            }
        }

        for (note_id, hub_ids) in &self.topic_memberships {
            for hub_id in hub_ids {
                links.push(GraphLink {
                    source: note_id.clone(),
                    target: hub_id.clone(),
                    relation: "topic_membership".to_string(),
                    edge_kind: GraphEdgeKind::TopicMembership,
                    provenance: GraphEdgeProvenance::Topic,
                });
            }
        }

        let mut seen_topic_pairs = HashSet::new();
        for (hub_id, neighbors) in &self.topic_related {
            for neighbor_id in neighbors {
                let pair = if hub_id <= neighbor_id {
                    (hub_id.clone(), neighbor_id.clone())
                } else {
                    (neighbor_id.clone(), hub_id.clone())
                };
                if !seen_topic_pairs.insert(pair.clone()) {
                    continue;
                }

                links.push(GraphLink {
                    source: pair.0,
                    target: pair.1,
                    relation: "topic_related".to_string(),
                    edge_kind: GraphEdgeKind::TopicRelated,
                    provenance: GraphEdgeProvenance::Topic,
                });
            }
        }

        FullGraph { nodes, links }
    }

    /// Get statistics about the graph
    pub fn stats(&self) -> GraphStats {
        let total_notes = self.note_meta.len();
        let note_links: usize = self.outgoing.values().map(|edges| edges.len()).sum();
        let membership_links: usize = self
            .topic_memberships
            .values()
            .map(|edges| edges.len())
            .sum();
        let topic_links: usize = self
            .topic_related
            .values()
            .map(|edges| edges.len())
            .sum::<usize>()
            / 2;
        let total_links = note_links + membership_links + topic_links;
        let notes_with_backlinks = self.backlinks.values().filter(|s| !s.is_empty()).count();
        let orphan_notes = self.get_unlinked().len();

        GraphStats {
            total_notes,
            total_links,
            notes_with_backlinks,
            orphan_notes,
        }
    }

    fn effective_topic_hubs(&self, note_id: &str) -> Vec<String> {
        if self
            .notes
            .get(note_id)
            .map(|note| note.is_topic_hub())
            .unwrap_or(false)
        {
            return vec![note_id.to_string()];
        }

        self.topic_memberships
            .get(note_id)
            .cloned()
            .unwrap_or_default()
    }

    fn is_topic_membership_pair(&self, source_id: &str, target_id: &str) -> bool {
        self.topic_memberships
            .get(source_id)
            .map(|hub_ids| hub_ids.iter().any(|hub_id| hub_id == target_id))
            .unwrap_or(false)
            || self
                .topic_hub_members
                .get(source_id)
                .map(|member_ids| member_ids.iter().any(|member_id| member_id == target_id))
                .unwrap_or(false)
    }

    fn resolve_target_id(&self, parsed_link: &ParsedLink) -> Option<String> {
        if let Some(target_path) = &parsed_link.target_path {
            if let Some(target_id) = self.path_to_id.get(&target_path.to_lowercase()) {
                return Some(target_id.clone());
            }
        }

        if let Some(target_id) = self.title_to_id.get(&parsed_link.target_title.to_lowercase()) {
            return Some(target_id.clone());
        }

        self.alias_to_id
            .get(&parsed_link.target_title.to_lowercase())
            .cloned()
    }
}

/// Full graph structure for visualization (nodes + links)
#[derive(Debug, Clone, serde::Serialize)]
pub struct FullGraph {
    pub nodes: Vec<GraphNode>,
    pub links: Vec<GraphLink>,
}

/// A node in the full graph visualization
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub val: usize,
    pub note_type: String,
    pub tags: Vec<String>,
    pub group: String,
    pub node_kind: GraphNodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
}

/// A directed link between two nodes with relationship type
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphLink {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub edge_kind: GraphEdgeKind,
    pub provenance: GraphEdgeProvenance,
}

fn bump_topic_pair(
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

/// Statistics about the note graph
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphStats {
    pub total_notes: usize,
    pub total_links: usize,
    pub notes_with_backlinks: usize,
    pub orphan_notes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{NoteStatus, ParsedLink};
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_note(id: &str, title: &str, links: Vec<(&str, RelationType)>) -> Note {
        Note {
            id: id.to_string(),
            title: title.to_string(),
            content: format!("Content of {}", title),
            relative_path: format!("{}.md", id),
            aliases: Vec::new(),
            status: NoteStatus::Draft,
            tags: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            wikilinks: links.iter().map(|(target, _)| target.to_string()).collect(),
            parsed_links: links
                .into_iter()
                .map(|(target_title, relation)| ParsedLink {
                    target_title: target_title.to_string(),
                    target_path: None,
                    relation,
                })
                .collect(),
            properties: HashMap::new(),
        }
    }

    #[test]
    fn update_note_resolves_links_from_existing_notes() {
        let mut graph = GraphIndex::new();
        let source = make_note("source", "Source", vec![("Target", RelationType::Related)]);

        graph.build_from_notes(&[source]);
        assert!(graph.get_outgoing("source").is_empty());

        graph.update_note(&make_note("target", "Target", vec![]));

        let outgoing = graph.get_outgoing("source");
        let backlinks = graph.get_backlinks("target");

        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].id, "target");
        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].id, "source");
    }

    #[test]
    fn update_note_rebuilds_links_after_title_change() {
        let mut graph = GraphIndex::new();
        let source = make_note(
            "source",
            "Source",
            vec![("Renamed Target", RelationType::Supports)],
        );
        let target = make_note("target", "Old Target", vec![]);

        graph.build_from_notes(&[source, target]);
        assert!(graph.get_backlinks("target").is_empty());

        graph.update_note(&make_note("target", "Renamed Target", vec![]));

        let backlinks = graph.get_backlinks("target");
        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].id, "source");
    }
}
