//! Temporal + graph-aware retrieval service
//!
//! Orchestrates a multi-stage retrieval pipeline that combines keyword search
//! (Tantivy BM25), priority scoring (recency/status/tags), graph expansion
//! (N-hop wikilink neighbors), and hub detection (highly-connected notes).
//!
//! This replaces pure BM25 similarity with context-aware ranking that
//! leverages the Zettelkasten graph structure.

use crate::models::note::{ChunkResult, NoteMeta};
use crate::services::chunk_index::ChunkIndex;
use crate::services::graph_index::GraphIndex;
use crate::services::priority::PriorityScoringService;
use crate::services::search::SearchService;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Configuration for the temporal + graph-aware retrieval pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalConfig {
    /// How many hops to expand via the link graph (1-3)
    pub graph_hop_depth: usize,
    /// Weight for graph proximity boost (0.0-1.0) — how much being a neighbor matters
    pub graph_proximity_weight: f32,
    /// Weight for connection density (hub) boost (0.0-1.0)
    pub hub_boost_weight: f32,
    /// Minimum backlink count to be considered a hub
    pub hub_threshold: usize,
    /// Maximum base search results before graph expansion
    pub base_search_limit: usize,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            graph_hop_depth: 2,
            graph_proximity_weight: 0.2,
            hub_boost_weight: 0.15,
            hub_threshold: 3,
            base_search_limit: 50,
        }
    }
}

/// A single retrieval result with scoring breakdown and relevance reasons
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalResult {
    pub note: NoteMeta,
    pub score: f32,
    pub snippet: String,
    pub relevance_reasons: Vec<String>,
}

/// Internal candidate entry during scoring
struct CandidateEntry {
    meta: NoteMeta,
    score: f32,
    snippet: String,
    reasons: Vec<String>,
}

/// Temporal + graph-aware retrieval service
pub struct RetrievalService {
    config: RetrievalConfig,
    config_path: PathBuf,
}

impl RetrievalService {
    pub fn new(data_path: PathBuf) -> Self {
        let config_path = data_path.join("retrieval_config.json");
        let config = Self::load_from_disk(&config_path).unwrap_or_default();
        Self {
            config,
            config_path,
        }
    }

    fn load_from_disk(path: &PathBuf) -> Result<RetrievalConfig> {
        let data = std::fs::read_to_string(path)?;
        let config: RetrievalConfig = serde_json::from_str(&data)?;
        Ok(config)
    }

    fn save_to_disk(&self) -> Result<()> {
        let data = serde_json::to_string_pretty(&self.config)?;
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.config_path, data)?;
        Ok(())
    }

    pub fn get_config(&self) -> &RetrievalConfig {
        &self.config
    }

    pub fn update_config(&mut self, update: RetrievalConfigUpdate) -> Result<RetrievalConfig> {
        if let Some(v) = update.graph_hop_depth {
            self.config.graph_hop_depth = v.clamp(1, 3);
        }
        if let Some(v) = update.graph_proximity_weight {
            self.config.graph_proximity_weight = v.clamp(0.0, 1.0);
        }
        if let Some(v) = update.hub_boost_weight {
            self.config.hub_boost_weight = v.clamp(0.0, 1.0);
        }
        if let Some(v) = update.hub_threshold {
            self.config.hub_threshold = v.max(1);
        }
        if let Some(v) = update.base_search_limit {
            self.config.base_search_limit = v.clamp(10, 200);
        }
        self.save_to_disk()?;
        Ok(self.config.clone())
    }

    /// Main retrieval pipeline:
    ///   1. Tantivy keyword search (base candidates)
    ///   2. Enrich with real timestamps from GraphIndex
    ///   3. Priority scoring (recency, status, tag boosts)
    ///   4. Graph expansion (N-hop neighbors of top results + context notes)
    ///   5. Hub boost (highly-connected notes score higher)
    ///   6. Sort by final score, truncate to limit
    pub fn retrieve(
        &self,
        search: &SearchService,
        graph: &GraphIndex,
        priority: &PriorityScoringService,
        query: &str,
        limit: usize,
        context_note_ids: &[String],
    ) -> Result<Vec<RetrievalResult>, String> {
        // Step 1: Tantivy keyword search (base candidates)
        let mut search_results = search
            .search(query, self.config.base_search_limit)
            .map_err(|e| e.to_string())?;

        // Step 2: Enrich search results with real timestamps from GraphIndex
        // (Tantivy doesn't store timestamps — SearchService uses Utc::now() placeholders)
        for r in &mut search_results {
            if let Some(real_meta) = graph.get_note_meta(&r.note.id) {
                r.note.created_at = real_meta.created_at;
                r.note.updated_at = real_meta.updated_at;
            }
        }

        // Step 3: Apply priority scoring (recency, status, tag boosts)
        priority.score_results(&mut search_results);

        // Build candidate map: note_id → CandidateEntry
        let mut candidates: HashMap<String, CandidateEntry> = HashMap::new();
        for r in &search_results {
            candidates.insert(
                r.note.id.clone(),
                CandidateEntry {
                    meta: r.note.clone(),
                    score: r.score,
                    snippet: r.snippet.clone().unwrap_or_default(),
                    reasons: vec!["keyword match".to_string()],
                },
            );
        }

        // Step 4: Graph expansion — add N-hop neighbors of top search results + context notes
        let seed_ids: HashSet<String> = search_results
            .iter()
            .take(10) // Only expand from top 10 to avoid combinatorial explosion
            .map(|r| r.note.id.clone())
            .chain(context_note_ids.iter().cloned())
            .collect();

        let graph_neighbors = self.expand_graph(&seed_ids, graph);

        for (note_id, (meta, hops)) in &graph_neighbors {
            let proximity_boost = self.config.graph_proximity_weight / (*hops as f32);

            if let Some(entry) = candidates.get_mut(note_id) {
                // Already a search result — add graph proximity boost
                entry.score += proximity_boost;
                entry.reasons.push(format!(
                    "graph neighbor ({} hop{})",
                    hops,
                    if *hops == 1 { "" } else { "s" }
                ));
            } else {
                // New candidate discovered via graph expansion
                candidates.insert(
                    note_id.clone(),
                    CandidateEntry {
                        meta: meta.clone(),
                        score: proximity_boost,
                        snippet: String::new(),
                        reasons: vec![format!(
                            "graph neighbor ({} hop{})",
                            hops,
                            if *hops == 1 { "" } else { "s" }
                        )],
                    },
                );
            }
        }

        // Step 5: Hub boost — notes with many backlinks get boosted
        for (note_id, entry) in candidates.iter_mut() {
            let backlink_count = graph.get_backlinks(note_id).len();
            if backlink_count >= self.config.hub_threshold {
                let hub_boost = self.config.hub_boost_weight
                    * (backlink_count as f32 / self.config.hub_threshold as f32).min(3.0);
                entry.score += hub_boost;
                entry
                    .reasons
                    .push(format!("hub ({} backlinks)", backlink_count));
            }
        }

        // Step 6: Sort by score descending, take top-K
        let mut results: Vec<RetrievalResult> = candidates
            .into_values()
            .map(|entry| RetrievalResult {
                note: entry.meta,
                score: entry.score,
                snippet: entry.snippet,
                relevance_reasons: entry.reasons,
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        Ok(results)
    }

    /// Chunk-level retrieval with token budgeting.
    ///
    /// Searches the chunk index, applies graph and hub boosts from the note-level
    /// graph, then greedily fills the token budget with the best-scoring chunks.
    pub fn retrieve_chunks(
        &self,
        chunk_index: &ChunkIndex,
        graph: &GraphIndex,
        priority: &PriorityScoringService,
        query: &str,
        token_budget: usize,
        context_note_ids: &[String],
    ) -> std::result::Result<Vec<ChunkResult>, String> {
        // Step 1: Search chunks (3x limit since chunks are smaller than notes)
        let search_limit = self.config.base_search_limit * 3;
        let mut chunks = chunk_index
            .search_chunks(query, search_limit)
            .map_err(|e| e.to_string())?;

        if chunks.is_empty() {
            return Ok(Vec::new());
        }

        // Step 2: Build seed set from context notes + top chunk parents
        let mut seed_ids: HashSet<String> = context_note_ids.iter().cloned().collect();
        for chunk in chunks.iter().take(10) {
            seed_ids.insert(chunk.parent_note_id.clone());
        }

        // Step 3: Expand graph for proximity boost
        let neighbors = self.expand_graph(&seed_ids, graph);

        // Step 4: Apply graph proximity and hub boosts to chunk scores
        for chunk in chunks.iter_mut() {
            let parent_id = &chunk.parent_note_id;

            // Priority scoring (recency, status, tags) via parent note
            if let Some(meta) = graph.get_note_meta(parent_id) {
                let priority_boost = priority.compute_boost(&meta);
                chunk.search_score += priority_boost;
            }

            // Graph proximity boost
            if let Some((_meta, hops)) = neighbors.get(parent_id) {
                let proximity_boost =
                    self.config.graph_proximity_weight / (*hops as f32);
                chunk.search_score += proximity_boost;
            }

            // Hub boost
            let backlink_count = graph.get_backlinks(parent_id).len();
            if backlink_count >= self.config.hub_threshold {
                let hub_boost = self.config.hub_boost_weight
                    * (backlink_count as f32 / self.config.hub_threshold as f32).min(3.0);
                chunk.search_score += hub_boost;
            }
        }

        // Step 5: Sort by score descending
        chunks.sort_by(|a, b| {
            b.search_score
                .partial_cmp(&a.search_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Step 6: Greedy token-budget fill
        let mut selected = Vec::new();
        let mut running_tokens = 0;

        for chunk in chunks {
            if running_tokens + chunk.token_estimate <= token_budget {
                running_tokens += chunk.token_estimate;
                selected.push(chunk);
            }
            // Stop early if budget is nearly exhausted
            if running_tokens >= token_budget {
                break;
            }
        }

        Ok(selected)
    }

    /// Expand the graph from seed IDs, returning a map of discovered neighbors
    /// with their minimum hop distance from any seed.
    ///
    /// Explores both outgoing links (wikilinks) and backlinks (reverse edges)
    /// for bidirectional graph traversal.
    fn expand_graph(
        &self,
        seed_ids: &HashSet<String>,
        graph: &GraphIndex,
    ) -> HashMap<String, (NoteMeta, usize)> {
        let mut neighbors: HashMap<String, (NoteMeta, usize)> = HashMap::new();
        let mut visited: HashSet<String> = seed_ids.clone();
        let mut frontier: HashSet<String> = seed_ids.clone();

        for hop in 1..=self.config.graph_hop_depth {
            let mut next_frontier = HashSet::new();

            for id in &frontier {
                // Outgoing links (this note links to...)
                for meta in graph.get_outgoing(id) {
                    if !visited.contains(&meta.id) {
                        let nid = meta.id.clone();
                        neighbors.entry(nid.clone()).or_insert((meta, hop));
                        next_frontier.insert(nid);
                    }
                }
                // Backlinks (linked by...)
                for meta in graph.get_backlinks(id) {
                    if !visited.contains(&meta.id) {
                        let nid = meta.id.clone();
                        neighbors.entry(nid.clone()).or_insert((meta, hop));
                        next_frontier.insert(nid);
                    }
                }
            }

            visited.extend(next_frontier.iter().cloned());
            frontier = next_frontier;
        }

        neighbors
    }
}

/// Partial update for retrieval config
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RetrievalConfigUpdate {
    pub graph_hop_depth: Option<usize>,
    pub graph_proximity_weight: Option<f32>,
    pub hub_boost_weight: Option<f32>,
    pub hub_threshold: Option<usize>,
    pub base_search_limit: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{Note, NoteStatus};
    use crate::services::graph_index::GraphIndex;
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_note(id: &str, title: &str, wikilinks: Vec<&str>) -> Note {
        use crate::models::note::{ParsedLink, RelationType};
        let wikilink_strings: Vec<String> = wikilinks.into_iter().map(String::from).collect();
        let parsed_links = wikilink_strings
            .iter()
            .map(|title| ParsedLink {
                target_title: title.clone(),
                relation: RelationType::Untyped,
            })
            .collect();
        Note {
            id: id.to_string(),
            title: title.to_string(),
            content: format!("Content of {}", title),
            status: NoteStatus::Draft,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            wikilinks: wikilink_strings,
            parsed_links,
            properties: HashMap::new(),
        }
    }

    #[test]
    fn test_default_config() {
        let config = RetrievalConfig::default();
        assert_eq!(config.graph_hop_depth, 2);
        assert_eq!(config.graph_proximity_weight, 0.2);
        assert_eq!(config.hub_boost_weight, 0.15);
        assert_eq!(config.hub_threshold, 3);
        assert_eq!(config.base_search_limit, 50);
    }

    #[test]
    fn test_expand_graph_one_hop() {
        let notes = vec![
            make_note("a", "Note A", vec!["Note B"]),
            make_note("b", "Note B", vec!["Note C"]),
            make_note("c", "Note C", vec![]),
            make_note("d", "Note D", vec![]),
        ];

        let mut graph = GraphIndex::new();
        graph.build_from_notes(&notes);

        let svc = RetrievalService {
            config: RetrievalConfig {
                graph_hop_depth: 1,
                ..Default::default()
            },
            config_path: PathBuf::from("/tmp/test_retrieval.json"),
        };

        let seeds: HashSet<String> = ["a".to_string()].into_iter().collect();
        let neighbors = svc.expand_graph(&seeds, &graph);

        // A→B (outgoing), so B is a 1-hop neighbor
        assert!(neighbors.contains_key("b"));
        assert_eq!(neighbors["b"].1, 1);
        // C is 2 hops away — shouldn't be reached at depth 1
        assert!(!neighbors.contains_key("c"));
        // D is disconnected
        assert!(!neighbors.contains_key("d"));
    }

    #[test]
    fn test_expand_graph_two_hops() {
        let notes = vec![
            make_note("a", "Note A", vec!["Note B"]),
            make_note("b", "Note B", vec!["Note C"]),
            make_note("c", "Note C", vec![]),
            make_note("d", "Note D", vec![]),
        ];

        let mut graph = GraphIndex::new();
        graph.build_from_notes(&notes);

        let svc = RetrievalService {
            config: RetrievalConfig::default(), // graph_hop_depth: 2
            config_path: PathBuf::from("/tmp/test_retrieval.json"),
        };

        let seeds: HashSet<String> = ["a".to_string()].into_iter().collect();
        let neighbors = svc.expand_graph(&seeds, &graph);

        // B is 1 hop (A→B), C is 2 hops (A→B→C)
        assert!(neighbors.contains_key("b"));
        assert_eq!(neighbors["b"].1, 1);
        assert!(neighbors.contains_key("c"));
        assert_eq!(neighbors["c"].1, 2);
        // D is disconnected
        assert!(!neighbors.contains_key("d"));
    }

    #[test]
    fn test_expand_graph_bidirectional() {
        // A→B (outgoing) and C→A (backlink from C)
        let notes = vec![
            make_note("a", "Note A", vec!["Note B"]),
            make_note("b", "Note B", vec![]),
            make_note("c", "Note C", vec!["Note A"]),
        ];

        let mut graph = GraphIndex::new();
        graph.build_from_notes(&notes);

        let svc = RetrievalService {
            config: RetrievalConfig {
                graph_hop_depth: 1,
                ..Default::default()
            },
            config_path: PathBuf::from("/tmp/test_retrieval.json"),
        };

        let seeds: HashSet<String> = ["a".to_string()].into_iter().collect();
        let neighbors = svc.expand_graph(&seeds, &graph);

        // B is reachable via outgoing link
        assert!(neighbors.contains_key("b"));
        // C is reachable via backlink (C→A means A has backlink from C)
        assert!(neighbors.contains_key("c"));
    }

    #[test]
    fn test_expand_graph_seeds_excluded() {
        let notes = vec![
            make_note("a", "Note A", vec!["Note B"]),
            make_note("b", "Note B", vec!["Note A"]),
        ];

        let mut graph = GraphIndex::new();
        graph.build_from_notes(&notes);

        let svc = RetrievalService {
            config: RetrievalConfig {
                graph_hop_depth: 2,
                ..Default::default()
            },
            config_path: PathBuf::from("/tmp/test_retrieval.json"),
        };

        let seeds: HashSet<String> = ["a".to_string()].into_iter().collect();
        let neighbors = svc.expand_graph(&seeds, &graph);

        // B is a neighbor, but A (the seed) should NOT appear in neighbors
        assert!(neighbors.contains_key("b"));
        assert!(!neighbors.contains_key("a"));
    }

    #[test]
    fn test_config_update_clamps_values() {
        let dir = std::env::temp_dir().join("test_retrieval_clamp");
        let _ = std::fs::create_dir_all(&dir);

        let mut svc = RetrievalService::new(dir.clone());

        let updated = svc
            .update_config(RetrievalConfigUpdate {
                graph_hop_depth: Some(10),       // max 3
                graph_proximity_weight: Some(5.0), // max 1.0
                hub_boost_weight: Some(-1.0),    // min 0.0
                hub_threshold: Some(0),          // min 1
                base_search_limit: Some(1),      // min 10
            })
            .unwrap();

        assert_eq!(updated.graph_hop_depth, 3);
        assert_eq!(updated.graph_proximity_weight, 1.0);
        assert_eq!(updated.hub_boost_weight, 0.0);
        assert_eq!(updated.hub_threshold, 1);
        assert_eq!(updated.base_search_limit, 10);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_hub_detection_via_graph() {
        let notes = vec![
            make_note("hub", "Hub Note", vec![]),
            make_note("a", "Note A", vec!["Hub Note"]),
            make_note("b", "Note B", vec!["Hub Note"]),
            make_note("c", "Note C", vec!["Hub Note"]),
            make_note("d", "Note D", vec!["Hub Note"]),
        ];

        let mut graph = GraphIndex::new();
        graph.build_from_notes(&notes);

        // Hub should have 4 backlinks (above default threshold of 3)
        assert_eq!(graph.get_backlinks("hub").len(), 4);
    }

    #[test]
    fn test_expand_graph_records_minimum_hops() {
        // A→B→C and A→C — C should be recorded as 1-hop (direct), not 2-hop
        let notes = vec![
            make_note("a", "Note A", vec!["Note B", "Note C"]),
            make_note("b", "Note B", vec!["Note C"]),
            make_note("c", "Note C", vec![]),
        ];

        let mut graph = GraphIndex::new();
        graph.build_from_notes(&notes);

        let svc = RetrievalService {
            config: RetrievalConfig::default(),
            config_path: PathBuf::from("/tmp/test_retrieval.json"),
        };

        let seeds: HashSet<String> = ["a".to_string()].into_iter().collect();
        let neighbors = svc.expand_graph(&seeds, &graph);

        // C is reachable directly from A (1 hop), even though B→C exists (2 hops)
        assert_eq!(neighbors["c"].1, 1);
    }
}
