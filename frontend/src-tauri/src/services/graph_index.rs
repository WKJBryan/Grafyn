use crate::models::note::{
    GraphNeighbor, LinkDirection, Note, NoteMeta, RelationType, TypedEdge,
};
use std::collections::HashMap;

/// Service for managing the note link graph (backlinks and outgoing links)
#[derive(Debug, Clone, Default)]
pub struct GraphIndex {
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

        // Build title-to-ID mapping and cache metadata
        for note in notes {
            self.title_to_id
                .insert(note.title.to_lowercase(), note.id.clone());
            self.note_meta.insert(note.id.clone(), NoteMeta::from(note));
        }

        // Build link graph from typed links
        for note in notes {
            let outgoing_edges = self.outgoing.entry(note.id.clone()).or_default();

            for parsed_link in &note.parsed_links {
                if let Some(target_id) =
                    self.title_to_id.get(&parsed_link.target_title.to_lowercase())
                {
                    // Avoid duplicate edges to the same target
                    if !outgoing_edges.iter().any(|e| e.target_id == *target_id) {
                        outgoing_edges.push(TypedEdge {
                            target_id: target_id.clone(),
                            relation: parsed_link.relation.clone(),
                        });

                        // Add backlink with reverse relation
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
        self.outgoing.clear();
        self.backlinks.clear();
        self.title_to_id.clear();
        self.note_meta.clear();
    }

    /// Add or update a note in the index
    pub fn update_note(&mut self, note: &Note) {
        // Remove old links
        self.remove_note(&note.id);

        // Update title mapping
        self.title_to_id
            .insert(note.title.to_lowercase(), note.id.clone());
        self.note_meta.insert(note.id.clone(), NoteMeta::from(note));

        // Add new typed links
        let outgoing_edges = self.outgoing.entry(note.id.clone()).or_default();

        for parsed_link in &note.parsed_links {
            if let Some(target_id) =
                self.title_to_id.get(&parsed_link.target_title.to_lowercase())
            {
                if !outgoing_edges.iter().any(|e| e.target_id == *target_id) {
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

    /// Remove a note from the index
    pub fn remove_note(&mut self, note_id: &str) {
        // Remove outgoing links and clean up corresponding backlinks
        if let Some(outgoing) = self.outgoing.remove(note_id) {
            for edge in outgoing {
                if let Some(backlinks) = self.backlinks.get_mut(&edge.target_id) {
                    backlinks.retain(|e| e.target_id != note_id);
                }
            }
        }

        // Remove backlinks pointing to this note and clean up corresponding outgoing
        if let Some(backlinks) = self.backlinks.remove(note_id) {
            for edge in backlinks {
                if let Some(outgoing) = self.outgoing.get_mut(&edge.target_id) {
                    outgoing.retain(|e| e.target_id != note_id);
                }
            }
        }

        // Remove from title mapping
        self.title_to_id.retain(|_, v| v != note_id);
        self.note_meta.remove(note_id);
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
