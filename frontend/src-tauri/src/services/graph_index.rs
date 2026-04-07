use crate::models::note::{
    GraphNeighbor, LinkDirection, Note, NoteMeta, RelationType, TypedEdge,
};
use std::collections::HashMap;

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
    /// Cached note metadata for quick lookups
    note_meta: HashMap<String, NoteMeta>,
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
        self.note_meta.clear();

        for note in self.notes.values() {
            self.title_to_id
                .insert(note.title.to_lowercase(), note.id.clone());
            self.note_meta.insert(note.id.clone(), NoteMeta::from(note));
        }

        for note in self.notes.values() {
            let outgoing_edges = self.outgoing.entry(note.id.clone()).or_default();

            for parsed_link in &note.parsed_links {
                if let Some(target_id) =
                    self.title_to_id.get(&parsed_link.target_title.to_lowercase())
                {
                    if !outgoing_edges.iter().any(|edge| edge.target_id == *target_id) {
                        outgoing_edges.push(TypedEdge {
                            target_id: target_id.clone(),
                            relation: parsed_link.relation.clone(),
                        });

                        self.backlinks
                            .entry(target_id.clone())
                            .or_default()
                            .push(TypedEdge {
                                target_id: note.id.clone(),
                                relation: parsed_link.relation.reverse(),
                            });
                    }
                }
            }
        }
    }

    /// Clear the index
    pub fn clear(&mut self) {
        self.notes.clear();
        self.outgoing.clear();
        self.backlinks.clear();
        self.title_to_id.clear();
        self.note_meta.clear();
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
                !has_outgoing && !has_backlinks
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

    /// Get the full graph structure for visualization
    pub fn get_full_graph(&self) -> FullGraph {
        let mut nodes = Vec::new();
        let mut links = Vec::new();

        // Build nodes from all known notes
        for (id, meta) in &self.note_meta {
            let backlink_count = self
                .backlinks
                .get(id)
                .map(|s| s.len())
                .unwrap_or(0);

            nodes.push(GraphNode {
                id: id.clone(),
                label: meta.title.clone(),
                val: backlink_count + 1,
                note_type: meta.status.to_string(),
                tags: meta.tags.clone(),
                group: "#6b7280".to_string(),
            });
        }

        // Build links from outgoing map
        for (source, edges) in &self.outgoing {
            for edge in edges {
                links.push(GraphLink {
                    source: source.clone(),
                    target: edge.target_id.clone(),
                    relation: edge.relation.to_string(),
                });
            }
        }

        FullGraph { nodes, links }
    }

    /// Get statistics about the graph
    pub fn stats(&self) -> GraphStats {
        let total_notes = self.note_meta.len();
        let total_links: usize = self.outgoing.values().map(|s| s.len()).sum();
        let notes_with_backlinks = self
            .backlinks
            .values()
            .filter(|s| !s.is_empty())
            .count();
        let orphan_notes = self.get_unlinked().len();

        GraphStats {
            total_notes,
            total_links,
            notes_with_backlinks,
            orphan_notes,
        }
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
}

/// A directed link between two nodes with relationship type
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphLink {
    pub source: String,
    pub target: String,
    pub relation: String,
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
            status: NoteStatus::Draft,
            tags: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            wikilinks: links.iter().map(|(target, _)| target.to_string()).collect(),
            parsed_links: links
                .into_iter()
                .map(|(target_title, relation)| ParsedLink {
                    target_title: target_title.to_string(),
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
        let source = make_note("source", "Source", vec![("Renamed Target", RelationType::Supports)]);
        let target = make_note("target", "Old Target", vec![]);

        graph.build_from_notes(&[source, target]);
        assert!(graph.get_backlinks("target").is_empty());

        graph.update_note(&make_note("target", "Renamed Target", vec![]));

        let backlinks = graph.get_backlinks("target");
        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].id, "source");
    }
}
