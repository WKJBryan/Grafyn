use crate::models::note::{GraphNeighbor, LinkType, Note, NoteMeta};
use std::collections::{HashMap, HashSet};

/// Service for managing the note link graph (backlinks and outgoing links)
#[derive(Debug, Clone, Default)]
pub struct GraphIndex {
    /// Map from note ID to set of note IDs it links to
    outgoing: HashMap<String, HashSet<String>>,
    /// Map from note ID to set of note IDs that link to it
    backlinks: HashMap<String, HashSet<String>>,
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

        // Build link graph
        for note in notes {
            let outgoing_set = self.outgoing.entry(note.id.clone()).or_default();

            for link_title in &note.wikilinks {
                // Try to resolve the wikilink to a note ID
                if let Some(target_id) = self.title_to_id.get(&link_title.to_lowercase()) {
                    outgoing_set.insert(target_id.clone());

                    // Add backlink
                    self.backlinks
                        .entry(target_id.clone())
                        .or_default()
                        .insert(note.id.clone());
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

        // Add new links
        let outgoing_set = self.outgoing.entry(note.id.clone()).or_default();

        for link_title in &note.wikilinks {
            if let Some(target_id) = self.title_to_id.get(&link_title.to_lowercase()) {
                outgoing_set.insert(target_id.clone());

                self.backlinks
                    .entry(target_id.clone())
                    .or_default()
                    .insert(note.id.clone());
            }
        }
    }

    /// Remove a note from the index
    pub fn remove_note(&mut self, note_id: &str) {
        // Remove outgoing links
        if let Some(outgoing) = self.outgoing.remove(note_id) {
            for target_id in outgoing {
                if let Some(backlinks) = self.backlinks.get_mut(&target_id) {
                    backlinks.remove(note_id);
                }
            }
        }

        // Remove backlinks pointing to this note
        if let Some(backlinks) = self.backlinks.remove(note_id) {
            for source_id in backlinks {
                if let Some(outgoing) = self.outgoing.get_mut(&source_id) {
                    outgoing.remove(note_id);
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
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.note_meta.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all notes that the given note links to (outgoing links)
    pub fn get_outgoing(&self, note_id: &str) -> Vec<NoteMeta> {
        self.outgoing
            .get(note_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.note_meta.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all neighbors (both backlinks and outgoing) for graph visualization
    pub fn get_neighbors(&self, note_id: &str) -> Vec<GraphNeighbor> {
        let mut neighbors = Vec::new();

        // Add outgoing links
        if let Some(outgoing) = self.outgoing.get(note_id) {
            for target_id in outgoing {
                if let Some(meta) = self.note_meta.get(target_id) {
                    neighbors.push(GraphNeighbor {
                        note: meta.clone(),
                        link_type: LinkType::Outgoing,
                    });
                }
            }
        }

        // Add backlinks
        if let Some(backlinks) = self.backlinks.get(note_id) {
            for source_id in backlinks {
                if let Some(meta) = self.note_meta.get(source_id) {
                    neighbors.push(GraphNeighbor {
                        note: meta.clone(),
                        link_type: LinkType::Backlink,
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

/// Statistics about the note graph
#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphStats {
    pub total_notes: usize,
    pub total_links: usize,
    pub notes_with_backlinks: usize,
    pub orphan_notes: usize,
}
