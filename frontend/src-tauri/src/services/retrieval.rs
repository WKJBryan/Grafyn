//! Temporal + graph-aware retrieval service
//!
//! Orchestrates a multi-stage retrieval pipeline that combines keyword search
//! (Tantivy BM25), priority scoring (recency/status/tags), graph expansion
//! (N-hop wikilink neighbors with relation-type weighting), and hub detection
//! (highly-connected notes).
//!
//! This replaces pure BM25 similarity with context-aware ranking that
//! leverages the Zettelkasten graph structure.

use crate::models::note::{ChunkResult, NoteMeta, RelationType};
use crate::services::chunk_index::ChunkIndex;
use crate::services::graph_index::GraphIndex;
use crate::services::priority::PriorityScoringService;
use crate::services::search::SearchService;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Per-relation-type weights for graph expansion scoring.
///
/// Edges with stronger relationships (like `supports`) get higher
/// boosts than generic `related` or `untyped` links.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationWeights {
    pub supports: f32,
    pub contradicts: f32,
    pub expands: f32,
    pub questions: f32,
    pub answers: f32,
    pub example: f32,
    pub part_of: f32,
    pub related: f32,
    pub untyped: f32,
}

impl Default for RelationWeights {
    fn default() -> Self {
        Self {
            supports: 1.5,
            contradicts: 1.2,
            expands: 1.3,
            questions: 1.1,
            answers: 1.2,
            example: 1.1,
            part_of: 1.2,
            related: 1.0,
            untyped: 1.0,
        }
    }
}

impl RelationWeights {
    pub fn weight_for(&self, relation: &RelationType) -> f32 {
        match relation {
            RelationType::Supports => self.supports,
            RelationType::Contradicts => self.contradicts,
            RelationType::Expands => self.expands,
            RelationType::Questions => self.questions,
            RelationType::Answers => self.answers,
            RelationType::Example => self.example,
            RelationType::PartOf => self.part_of,
            RelationType::Related => self.related,
            RelationType::Untyped => self.untyped,
        }
    }
}

fn default_token_budget() -> usize {
    4000
}

fn default_chunk_retrieval_enabled() -> bool {
    true
}

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
    /// Default token budget for chunk-level retrieval (1000-32000)
    #[serde(default = "default_token_budget")]
    pub default_token_budget: usize,
    /// Whether chunk-level retrieval is enabled for canvas context
    #[serde(default = "default_chunk_retrieval_enabled")]
    pub chunk_retrieval_enabled: bool,
    /// Per-relation-type weights for graph expansion
    #[serde(default)]
    pub relation_weights: RelationWeights,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            graph_hop_depth: 2,
            graph_proximity_weight: 0.2,
            hub_boost_weight: 0.15,
            hub_threshold: 3,
            base_search_limit: 50,
            default_token_budget: 4000,
            chunk_retrieval_enabled: true,
            relation_weights: RelationWeights::default(),
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

/// A neighbor discovered during graph expansion
struct ExpandedNeighbor {
    meta: NoteMeta,
    hops: usize,
    best_relation: RelationType,
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
        if let Some(v) = update.default_token_budget {
            self.config.default_token_budget = v.clamp(1000, 32000);
        }
        if let Some(v) = update.chunk_retrieval_enabled {
            self.config.chunk_retrieval_enabled = v;
        }
        if let Some(v) = update.relation_weights {
            self.config.relation_weights = v;
        }
        self.save_to_disk()?;
        Ok(self.config.clone())
    }

    /// Main retrieval pipeline:
    ///   1. Tantivy keyword search (base candidates)
    ///   2. Enrich with real timestamps from GraphIndex
    ///   3. Priority scoring (recency, status, tag boosts)
    ///   4. Graph expansion (N-hop neighbors with relation-type weighting)
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

        self.apply_topic_hub_boosts(&mut candidates, graph, context_note_ids);

        // Step 4: Graph expansion — add N-hop neighbors with relation-type weighting
        let seed_ids: HashSet<String> = search_results
            .iter()
            .take(10) // Only expand from top 10 to avoid combinatorial explosion
            .map(|r| r.note.id.clone())
            .chain(context_note_ids.iter().cloned())
            .collect();

        let graph_neighbors = self.expand_graph(&seed_ids, graph);

        for (note_id, neighbor) in &graph_neighbors {
            let relation_weight = self
                .config
                .relation_weights
                .weight_for(&neighbor.best_relation);
            let proximity_boost =
                self.config.graph_proximity_weight / (neighbor.hops as f32) * relation_weight;

            if let Some(entry) = candidates.get_mut(note_id) {
                // Already a search result — add graph proximity boost
                entry.score += proximity_boost;
                entry.reasons.push(format!(
                    "graph neighbor ({} hop{}, {})",
                    neighbor.hops,
                    if neighbor.hops == 1 { "" } else { "s" },
                    neighbor.best_relation
                ));
            } else {
                // New candidate discovered via graph expansion
                candidates.insert(
                    note_id.clone(),
                    CandidateEntry {
                        meta: neighbor.meta.clone(),
                        score: proximity_boost,
                        snippet: String::new(),
                        reasons: vec![format!(
                            "graph neighbor ({} hop{}, {})",
                            neighbor.hops,
                            if neighbor.hops == 1 { "" } else { "s" },
                            neighbor.best_relation
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
    /// Searches the chunk index, applies graph boosts with relation-type weighting,
    /// then greedily fills the token budget with the best-scoring chunks.
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

            // Graph proximity boost with relation-type weighting
            if let Some(neighbor) = neighbors.get(parent_id) {
                let relation_weight = self
                    .config
                    .relation_weights
                    .weight_for(&neighbor.best_relation);
                let proximity_boost =
                    self.config.graph_proximity_weight / (neighbor.hops as f32) * relation_weight;
                chunk.search_score += proximity_boost;
            }

            let topic_boost = context_note_ids
                .iter()
                .map(|context_id| graph.topic_relation_score(context_id, parent_id))
                .fold(0.0_f64, f64::max) as f32;
            if topic_boost > 0.0 {
                chunk.search_score += self.config.graph_proximity_weight * topic_boost * 0.75;
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

    /// Expand the graph from seed IDs, returning discovered neighbors with
    /// their minimum hop distance and the relation type of the discovering edge.
    ///
    /// Explores both outgoing links and backlinks (bidirectional).
    /// Relation types are preserved so callers can weight edges differently
    /// (e.g., `supports` edges get a higher boost than `untyped`).
    fn expand_graph(
        &self,
        seed_ids: &HashSet<String>,
        graph: &GraphIndex,
    ) -> HashMap<String, ExpandedNeighbor> {
        let mut neighbors: HashMap<String, ExpandedNeighbor> = HashMap::new();
        let mut visited: HashSet<String> = seed_ids.clone();
        let mut frontier: HashSet<String> = seed_ids.clone();

        for hop in 1..=self.config.graph_hop_depth {
            let mut next_frontier = HashSet::new();

            for id in &frontier {
                // Outgoing links with relation types
                for (meta, relation) in graph.get_typed_outgoing(id) {
                    if !visited.contains(&meta.id) {
                        let nid = meta.id.clone();
                        neighbors.entry(nid.clone()).or_insert(ExpandedNeighbor {
                            meta,
                            hops: hop,
                            best_relation: relation,
                        });
                        next_frontier.insert(nid);
                    }
                }
                // Backlinks with relation types
                for (meta, relation) in graph.get_typed_backlinks(id) {
                    if !visited.contains(&meta.id) {
                        let nid = meta.id.clone();
                        neighbors.entry(nid.clone()).or_insert(ExpandedNeighbor {
                            meta,
                            hops: hop,
                            best_relation: relation,
                        });
                        next_frontier.insert(nid);
                    }
                }
            }

            visited.extend(next_frontier.iter().cloned());
            frontier = next_frontier;
        }

        neighbors
    }

    fn apply_topic_hub_boosts(
        &self,
        candidates: &mut HashMap<String, CandidateEntry>,
        graph: &GraphIndex,
        context_note_ids: &[String],
    ) {
        if context_note_ids.is_empty() {
            return;
        }

        for (candidate_id, entry) in candidates.iter_mut() {
            let topic_score = context_note_ids
                .iter()
                .map(|context_id| graph.topic_relation_score(context_id, candidate_id))
                .fold(0.0_f64, f64::max) as f32;

            if topic_score <= 0.0 {
                continue;
            }

            let shared_hub = context_note_ids.iter().any(|context_id| {
                let context_hubs = graph.get_note_topic_hub_ids(context_id);
                let candidate_hubs = graph.get_note_topic_hub_ids(candidate_id);
                context_hubs
                    .iter()
                    .any(|hub_id| candidate_hubs.contains(hub_id))
            });

            entry.score += self.config.graph_proximity_weight * topic_score * 0.75;
            entry.reasons.push(if shared_hub {
                "shared topic hub".to_string()
            } else {
                "adjacent topic hub".to_string()
            });
        }
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
    pub default_token_budget: Option<usize>,
    pub chunk_retrieval_enabled: Option<bool>,
    pub relation_weights: Option<RelationWeights>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{Note, NoteStatus, ParsedLink};
    use crate::services::graph_index::GraphIndex;
    use chrono::Utc;
    use std::collections::HashMap;

    fn make_note(id: &str, title: &str, wikilinks: Vec<&str>) -> Note {
        let wikilink_strings: Vec<String> = wikilinks.into_iter().map(String::from).collect();
        let parsed_links = wikilink_strings
            .iter()
            .map(|title| ParsedLink {
                target_title: title.clone(),
                target_path: None,
                relation: RelationType::Untyped,
            })
            .collect();
        Note {
            id: id.to_string(),
            title: title.to_string(),
            content: format!("Content of {}", title),
            relative_path: format!("{}.md", id),
            aliases: Vec::new(),
            status: NoteStatus::Draft,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            wikilinks: wikilink_strings,
            parsed_links,
            properties: HashMap::new(),
        }
    }

    fn make_typed_note(id: &str, title: &str, links: Vec<(&str, RelationType)>) -> Note {
        let wikilink_strings: Vec<String> = links.iter().map(|(t, _)| t.to_string()).collect();
        let parsed_links = links
            .into_iter()
            .map(|(target, relation)| ParsedLink {
                target_title: target.to_string(),
                target_path: None,
                relation,
            })
            .collect();
        Note {
            id: id.to_string(),
            title: title.to_string(),
            content: format!("Content of {}", title),
            relative_path: format!("{}.md", id),
            aliases: Vec::new(),
            status: NoteStatus::Draft,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
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
        assert_eq!(config.default_token_budget, 4000);
        assert!(config.chunk_retrieval_enabled);
        assert_eq!(config.relation_weights.supports, 1.5);
        assert_eq!(config.relation_weights.untyped, 1.0);
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
        assert_eq!(neighbors["b"].hops, 1);
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
        assert_eq!(neighbors["b"].hops, 1);
        assert!(neighbors.contains_key("c"));
        assert_eq!(neighbors["c"].hops, 2);
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
                graph_hop_depth: Some(10),         // max 3
                graph_proximity_weight: Some(5.0), // max 1.0
                hub_boost_weight: Some(-1.0),      // min 0.0
                hub_threshold: Some(0),            // min 1
                base_search_limit: Some(1),        // min 10
                default_token_budget: Some(100),   // min 1000
                chunk_retrieval_enabled: Some(false),
                relation_weights: None,
            })
            .unwrap();

        assert_eq!(updated.graph_hop_depth, 3);
        assert_eq!(updated.graph_proximity_weight, 1.0);
        assert_eq!(updated.hub_boost_weight, 0.0);
        assert_eq!(updated.hub_threshold, 1);
        assert_eq!(updated.base_search_limit, 10);
        assert_eq!(updated.default_token_budget, 1000);
        assert!(!updated.chunk_retrieval_enabled);

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
        assert_eq!(neighbors["c"].hops, 1);
    }

    #[test]
    fn test_expand_graph_preserves_relation_types() {
        let notes = vec![
            make_typed_note(
                "a",
                "Note A",
                vec![
                    ("Note B", RelationType::Supports),
                    ("Note C", RelationType::Contradicts),
                ],
            ),
            make_note("b", "Note B", vec![]),
            make_note("c", "Note C", vec![]),
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

        assert_eq!(neighbors["b"].best_relation, RelationType::Supports);
        assert_eq!(neighbors["c"].best_relation, RelationType::Contradicts);
    }

    #[test]
    fn test_relation_weights() {
        let weights = RelationWeights::default();
        assert!(
            weights.weight_for(&RelationType::Supports)
                > weights.weight_for(&RelationType::Untyped)
        );
        assert!(
            weights.weight_for(&RelationType::Contradicts)
                > weights.weight_for(&RelationType::Untyped)
        );
        assert_eq!(weights.weight_for(&RelationType::Untyped), 1.0);
        assert_eq!(weights.weight_for(&RelationType::Related), 1.0);
    }

    #[test]
    fn test_backward_compatible_config_deserialization() {
        // Old config without new fields should deserialize with defaults
        let old_json = r#"{
            "graph_hop_depth": 2,
            "graph_proximity_weight": 0.2,
            "hub_boost_weight": 0.15,
            "hub_threshold": 3,
            "base_search_limit": 50
        }"#;
        let config: RetrievalConfig = serde_json::from_str(old_json).unwrap();
        assert_eq!(config.default_token_budget, 4000);
        assert!(config.chunk_retrieval_enabled);
        assert_eq!(config.relation_weights.supports, 1.5);
    }
}
