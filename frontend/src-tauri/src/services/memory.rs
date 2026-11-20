use crate::models::memory::{Contradiction, ConversationMessage, ExtractedClaim, RecallResult};
use crate::services::graph_index::GraphIndex;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::search::SearchService;
use regex::Regex;
use std::collections::HashSet;

/// Memory service for AI-powered knowledge recall and extraction
pub struct MemoryService;

impl MemoryService {
    pub fn new() -> Self {
        Self
    }

    /// Recall relevant notes with graph-aware boosting
    pub fn recall_relevant(
        &self,
        search: &SearchService,
        graph: &GraphIndex,
        query: &str,
        context_note_ids: &[String],
        limit: usize,
    ) -> Result<Vec<RecallResult>, String> {
        // Search with double limit for graph reranking
        let search_results = search.search(query, limit * 2).map_err(|e| e.to_string())?;

        // Get graph neighbors of context notes for boosting
        let mut neighbor_ids = HashSet::new();
        for ctx_id in context_note_ids {
            for meta in graph.get_backlinks(ctx_id) {
                neighbor_ids.insert(meta.id);
            }
            for meta in graph.get_outgoing(ctx_id) {
                neighbor_ids.insert(meta.id);
            }
        }

        // Build results with graph boost
        let mut results: Vec<RecallResult> = search_results
            .into_iter()
            .map(|r| {
                let graph_boost = if neighbor_ids.contains(&r.note.id) {
                    0.15
                } else {
                    0.0
                };
                // BM25 scores are unbounded; clamping to 1.0 would erase the boost.
                let total = r.score + graph_boost;
                RecallResult {
                    note_id: r.note.id,
                    title: r.note.title,
                    snippet: r.snippet.unwrap_or_default(),
                    score: r.score,
                    tags: r.note.tags,
                    graph_boost,
                    total_score: total,
                }
            })
            .collect();

        // Sort by total_score descending
        results.sort_by(|a, b| {
            b.total_score
                .partial_cmp(&a.total_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        Ok(results)
    }

    /// Find potential contradictions for a note
    pub fn find_contradictions(
        &self,
        search: &SearchService,
        store: &KnowledgeStore,
        note_id: &str,
    ) -> Result<Vec<Contradiction>, String> {
        let note = store.get_note(note_id).map_err(|e| e.to_string())?;

        // Find similar notes
        let similar = search
            .find_similar(note_id, &note.content, 10)
            .map_err(|e| e.to_string())?;

        // BM25 scores aren't 0-1: judge similarity relative to the best match.
        let max_score = similar.iter().map(|r| r.score).fold(0.0_f32, f32::max);
        let mut contradictions = Vec::new();
        let note_status = note.status.to_string();
        let note_tags: HashSet<String> = note.tags.iter().cloned().collect();

        for result in similar {
            if result.note.id == note_id {
                continue;
            }

            let similarity = if max_score > 0.0 {
                result.score / max_score
            } else {
                0.0
            };

            // Status mismatch on similar content
            if similarity > 0.7 && result.note.status.to_string() != note_status {
                contradictions.push(Contradiction {
                    note_id: result.note.id.clone(),
                    title: result.note.title.clone(),
                    snippet: result.snippet.clone().unwrap_or_default(),
                    similarity_score: similarity,
                    conflict_type: "status_mismatch".to_string(),
                    details: format!(
                        "Status '{}' vs '{}' on similar content",
                        note_status, result.note.status
                    ),
                });
            }

            // Tag mismatch on highly similar content
            if similarity > 0.8 {
                let similar_tags: HashSet<String> = result.note.tags.iter().cloned().collect();
                if !note_tags.is_empty()
                    && !similar_tags.is_empty()
                    && note_tags.is_disjoint(&similar_tags)
                {
                    contradictions.push(Contradiction {
                        note_id: result.note.id.clone(),
                        title: result.note.title.clone(),
                        snippet: result.snippet.unwrap_or_default(),
                        similarity_score: similarity,
                        conflict_type: "tag_mismatch".to_string(),
                        details: format!(
                            "Tags {:?} vs {:?} on similar content",
                            note_tags.iter().collect::<Vec<_>>(),
                            similar_tags.iter().collect::<Vec<_>>()
                        ),
                    });
                }
            }
        }

        Ok(contradictions)
    }

    /// Extract claims from conversation messages
    pub fn extract_from_conversation(
        &self,
        messages: &[ConversationMessage],
    ) -> Vec<ExtractedClaim> {
        let mut claims = Vec::new();

        for msg in messages {
            if msg.content.len() < 20 {
                continue;
            }

            // Split into paragraphs
            let paragraphs: Vec<&str> = msg
                .content
                .split("\n\n")
                .filter(|p| p.trim().len() > 20)
                .take(5)
                .collect();

            for para in paragraphs {
                if let Some(claim_type) = classify_claim(para) {
                    let title = para.split('.').next().unwrap_or(para);
                    let title: String = title.chars().take(100).collect();

                    let mut tags = extract_tags(para);
                    if msg.role == "assistant" {
                        tags.push("ai-generated".to_string());
                    }

                    claims.push(ExtractedClaim {
                        title,
                        content: para.to_string(),
                        tags,
                        claim_type: claim_type.to_string(),
                        confidence: if msg.role == "user" { 0.7 } else { 0.8 },
                    });
                }
            }
        }

        claims
    }
}

fn classify_claim(text: &str) -> Option<&str> {
    let lower = text.to_lowercase();

    if ["decided", "we will", "the plan is", "going with", "chosen"]
        .iter()
        .any(|kw| lower.contains(kw))
    {
        return Some("decision");
    }

    if text.trim().ends_with('?')
        || ["how ", "what ", "why ", "when ", "where "]
            .iter()
            .any(|kw| lower.starts_with(kw))
    {
        return Some("question");
    }

    if [
        "insight",
        "learned",
        "realized",
        "key takeaway",
        "important",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
    {
        return Some("insight");
    }

    if text.len() > 50 {
        return Some("claim");
    }

    None
}

fn extract_tags(text: &str) -> Vec<String> {
    // Note: Rust regex does not support lookbehinds, so we use a word-boundary approach.
    // Match # followed by word chars/hyphens, but only when # is at start or preceded by whitespace.
    let re = Regex::new(r"(?:^|\s)#([\w-]+)").unwrap();
    let mut tags = Vec::new();
    for cap in re.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            let tag = m.as_str();
            if tag.len() > 1 {
                let lower = tag.to_lowercase();
                if !tags.contains(&lower) {
                    tags.push(lower);
                }
            }
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{NoteCreate, NoteStatus};
    use tempfile::TempDir;

    fn create(
        store: &mut KnowledgeStore,
        search: &mut SearchService,
        title: &str,
        content: &str,
        status: NoteStatus,
    ) -> String {
        let note = store
            .create_note(NoteCreate {
                title: title.into(),
                content: content.into(),
                relative_path: None,
                aliases: Vec::new(),
                status,
                tags: Vec::new(),
                schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: None,
                optimizer_managed: false,
                properties: Default::default(),
            })
            .unwrap();
        search.index_note(&note).unwrap();
        note.id
    }

    #[test]
    fn claim_title_truncation_is_char_safe() {
        // 99 ASCII bytes then a 2-byte char straddling byte 100.
        let para = format!("{}é and more text after it for length", "a".repeat(99));
        let claims = MemoryService::new().extract_from_conversation(&[ConversationMessage {
            role: "user".into(),
            content: para,
        }]);
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].title.chars().count(), 100);
        assert!(claims[0].title.ends_with("aé"));
    }

    #[test]
    fn recall_graph_boost_is_not_saturated() {
        let tmp = TempDir::new().unwrap();
        let mut store = KnowledgeStore::new(tmp.path().join("vault"), tmp.path().join("data"));
        let mut search = SearchService::new(tmp.path().join("data")).unwrap();
        let ctx = create(
            &mut store,
            &mut search,
            "Context",
            "See [[Target]]",
            NoteStatus::Draft,
        );
        create(
            &mut store,
            &mut search,
            "Target",
            "zebra zebra zebra",
            NoteStatus::Draft,
        );
        search.commit().unwrap();
        let mut graph = GraphIndex::new();
        graph.build_from_notes(&store.list_full_notes().unwrap());

        let results = MemoryService::new()
            .recall_relevant(&search, &graph, "zebra", &[ctx], 5)
            .unwrap();
        let hit = &results[0];
        assert_eq!(hit.graph_boost, 0.15);
        assert!((hit.total_score - (hit.score + 0.15)).abs() < 1e-6);
    }

    #[test]
    fn contradictions_use_relative_similarity() {
        let tmp = TempDir::new().unwrap();
        let mut store = KnowledgeStore::new(tmp.path().join("vault"), tmp.path().join("data"));
        let mut search = SearchService::new(tmp.path().join("data")).unwrap();
        let words = "alpha bravo charlie delta echoes foxtrot golfer hotel indigo juliet";
        let src = create(&mut store, &mut search, "Source", words, NoteStatus::Draft);
        let close = create(
            &mut store,
            &mut search,
            "Close",
            words,
            NoteStatus::Canonical,
        );
        let weak = create(
            &mut store,
            &mut search,
            "Weak",
            "alpha plus unrelated filler words entirely",
            NoteStatus::Canonical,
        );
        search.commit().unwrap();

        let found = MemoryService::new()
            .find_contradictions(&search, &store, &src)
            .unwrap();
        assert!(found.iter().any(|c| c.note_id == close));
        assert!(!found.iter().any(|c| c.note_id == weak), "{found:?}");
        assert!(found.iter().all(|c| c.similarity_score <= 1.0));
    }
}
