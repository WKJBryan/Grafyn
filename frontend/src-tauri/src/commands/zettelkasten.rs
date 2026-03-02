use crate::models::note::{
    ApplyLinksRequest, ApplyLinksResponse, CreateLinkResponse, DiscoverLinksResponse, NoteStatus,
    NoteUpdate, ZettelLinkCandidate,
};
use crate::services::openrouter::ChatMessage;
use crate::AppState;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use tauri::State;

lazy_static! {
    /// Multi-word proper nouns (e.g. "Machine Learning")
    static ref PROPER_NOUN_RE: Regex =
        Regex::new(r"\b[A-Z][a-z]+(?:\s+[A-Z][a-z]+)+\b").unwrap();
    /// Acronyms (e.g. "API", "LLM")
    static ref ACRONYM_RE: Regex = Regex::new(r"\b[A-Z]{2,}\b").unwrap();
    /// Long words likely to be meaningful concepts
    static ref LONG_WORD_RE: Regex = Regex::new(r"\b[a-z]{4,}\b").unwrap();
    /// Wikilink pattern for removal during term extraction
    static ref WIKILINK_RE: Regex = Regex::new(r"\[\[.+?\]\]").unwrap();
    /// Fenced code block removal
    static ref FENCED_CODE_RE: Regex = Regex::new(r"(?s)```.*?```").unwrap();
    /// Inline code removal
    static ref INLINE_CODE_RE: Regex = Regex::new(r"`[^`]+`").unwrap();
}

/// Stopwords to exclude from key term extraction
const STOPWORDS: &[&str] = &[
    "this", "that", "with", "from", "have", "been", "will", "would", "should", "could", "might",
    "their", "there", "these", "those", "they", "them", "then", "than", "what", "when", "where",
    "which", "while", "about", "after", "before", "between", "into", "through", "during", "each",
    "some", "other", "more", "most", "also", "only", "very", "just", "like", "such", "well",
    "here", "even", "back", "over", "much", "many", "make", "made", "does", "done", "take",
    "note", "notes",
];

// ── Link type definitions ────────────────────────────────────────────────

/// Hardcoded link type definitions matching the Python API
fn link_type_definitions() -> Vec<serde_json::Value> {
    serde_json::json!([
        { "id": "related",     "label": "Related",     "description": "General topical relationship", "reverse": "related" },
        { "id": "supports",    "label": "Supports",    "description": "Provides evidence or backing", "reverse": "related" },
        { "id": "contradicts", "label": "Contradicts", "description": "Presents opposing evidence",  "reverse": "contradicts" },
        { "id": "expands",     "label": "Expands",     "description": "Elaborates on the concept",   "reverse": "related" },
        { "id": "questions",   "label": "Questions",   "description": "Raises questions about",      "reverse": "answers" },
        { "id": "answers",     "label": "Answers",     "description": "Answers questions from",      "reverse": "questions" },
        { "id": "example",     "label": "Example",     "description": "Provides a concrete example", "reverse": "related" },
        { "id": "part_of",     "label": "Part Of",     "description": "Is a component of",           "reverse": "related" }
    ])
    .as_array()
    .unwrap()
    .clone()
}

/// Get the reverse link type for bidirectional linking
fn reverse_link_type(link_type: &str) -> &'static str {
    match link_type {
        "supports" => "related",
        "contradicts" => "contradicts",
        "expands" => "related",
        "questions" => "answers",
        "answers" => "questions",
        "example" => "related",
        "part_of" => "related",
        _ => "related",
    }
}

// ── Key term extraction ──────────────────────────────────────────────────

/// Extract key terms from markdown content (proper nouns, acronyms, long words)
fn extract_key_terms(content: &str) -> HashSet<String> {
    let stopwords: HashSet<&str> = STOPWORDS.iter().copied().collect();

    // Clean content: remove code blocks, inline code, wikilinks
    let clean = FENCED_CODE_RE.replace_all(content, "");
    let clean = INLINE_CODE_RE.replace_all(&clean, "");
    let clean = WIKILINK_RE.replace_all(&clean, "");

    let mut terms = HashSet::new();

    // Proper nouns (lowercased)
    for m in PROPER_NOUN_RE.find_iter(&clean) {
        let t = m.as_str().to_lowercase();
        if t.len() >= 3 && !stopwords.contains(t.as_str()) {
            terms.insert(t);
        }
    }

    // Acronyms (lowercased)
    for m in ACRONYM_RE.find_iter(&clean) {
        let t = m.as_str().to_lowercase();
        if t.len() >= 2 {
            terms.insert(t);
        }
    }

    // Long words
    for m in LONG_WORD_RE.find_iter(&clean) {
        let t = m.as_str().to_lowercase();
        if t.len() >= 4 && !stopwords.contains(t.as_str()) {
            terms.insert(t);
        }
    }

    terms
}

// ── Wikilink insertion ───────────────────────────────────────────────────

/// Add a wikilink to a note's content in the appropriate section.
/// Returns the new content if a link was added, or None if it already exists.
fn add_wikilink_to_content(content: &str, target_title: &str, link_type: &str) -> Option<String> {
    // Check if link already exists
    let link_marker = format!("[[{}]]", target_title);
    if content.contains(&link_marker) {
        return None;
    }

    let link_line = format!("- [[{}]] ({})", target_title, link_type);

    // Strategy: find existing section, or insert before ## Sources, or append
    let lines: Vec<&str> = content.lines().collect();
    let mut result_lines: Vec<String> = Vec::new();
    let mut inserted = false;

    // Look for "## Related Concepts" or "## See Also"
    let related_idx = lines
        .iter()
        .position(|l| l.trim() == "## Related Concepts");
    let see_also_idx = lines.iter().position(|l| l.trim() == "## See Also");
    let sources_idx = lines.iter().position(|l| l.trim() == "## Sources");

    if let Some(idx) = related_idx.or(see_also_idx) {
        // Insert after the section header (and any existing list items)
        for (i, line) in lines.iter().enumerate() {
            result_lines.push(line.to_string());
            if i == idx {
                // Skip past existing list items under this heading
                let mut insert_pos = i + 1;
                while insert_pos < lines.len() && lines[insert_pos].starts_with("- ") {
                    insert_pos += 1;
                }
                // We'll insert when we reach insert_pos
            }
        }
        // Re-do: simpler approach — find end of list under the heading
        result_lines.clear();
        let mut insert_after = idx;
        for j in (idx + 1)..lines.len() {
            if lines[j].starts_with("- ") || lines[j].trim().is_empty() {
                insert_after = j;
            } else {
                break;
            }
        }
        for (i, line) in lines.iter().enumerate() {
            result_lines.push(line.to_string());
            if i == insert_after && !inserted {
                result_lines.push(link_line.clone());
                inserted = true;
            }
        }
    } else if let Some(idx) = sources_idx {
        // Insert before ## Sources with a new Related Concepts section
        for (i, line) in lines.iter().enumerate() {
            if i == idx && !inserted {
                result_lines.push("## Related Concepts".to_string());
                result_lines.push(link_line.clone());
                result_lines.push(String::new());
                inserted = true;
            }
            result_lines.push(line.to_string());
        }
    }

    if !inserted {
        // Append to end with a new section
        result_lines = lines.iter().map(|l| l.to_string()).collect();
        result_lines.push(String::new());
        result_lines.push("## Related Concepts".to_string());
        result_lines.push(link_line);
    }

    Some(result_lines.join("\n"))
}

// ── Discovery strategies ─────────────────────────────────────────────────

/// Strategy A: Tantivy full-text search
/// Uses the note's title + first paragraph as a query, normalizes BM25 scores to 0-1
async fn find_search_links(
    state: &AppState,
    note_id: &str,
    title: &str,
    content: &str,
    max_links: usize,
) -> Vec<ZettelLinkCandidate> {
    // Build query from title + first paragraph
    let first_para = content
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("---") && !l.trim().is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join(" ");

    let query = format!("{} {}", title, first_para);
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    let results = {
        let search = state.search_service.read().await;
        match search.search(query, max_links * 2) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("Tantivy search failed during link discovery: {}", e);
                return Vec::new();
            }
        }
    };

    if results.is_empty() {
        return Vec::new();
    }

    // Find max score for normalization (BM25 scores are unbounded)
    let max_score = results
        .iter()
        .map(|r| r.score)
        .fold(0.0_f32, f32::max)
        .max(0.001); // avoid division by zero

    results
        .into_iter()
        .filter(|r| {
            r.note.id != note_id && r.note.status != NoteStatus::Evidence && r.score > 0.0
        })
        .take(max_links)
        .map(|r| {
            let normalized = (r.score / max_score) as f64;
            let link_type = if normalized >= 0.85 {
                "expands"
            } else {
                "related"
            };
            ZettelLinkCandidate {
                target_id: r.note.id.clone(),
                target_title: r.note.title.clone(),
                link_type: link_type.to_string(),
                confidence: (normalized * 100.0).round() / 100.0, // 2 decimal places
                reason: format!("Search similarity: {:.2}", normalized),
            }
        })
        .collect()
}

/// Strategy B: Keyword and tag overlap
async fn find_keyword_links(
    state: &AppState,
    note_id: &str,
    content: &str,
    tags: &[String],
) -> Vec<ZettelLinkCandidate> {
    let note_terms = extract_key_terms(content);
    let note_tags: HashSet<String> = tags.iter().map(|t| t.to_lowercase()).collect();

    if note_terms.is_empty() && note_tags.is_empty() {
        return Vec::new();
    }

    // Get all notes to compare against
    let all_notes = {
        let store = state.knowledge_store.read().await;
        store.list_notes().unwrap_or_default()
    };

    let mut candidates = Vec::new();

    for meta in &all_notes {
        if meta.id == note_id || meta.status == NoteStatus::Evidence {
            continue;
        }

        // Get full note content for term extraction
        let other_content = {
            let store = state.knowledge_store.read().await;
            match store.get_note(&meta.id) {
                Ok(n) => n.content.clone(),
                Err(_) => continue,
            }
        };

        let other_terms = extract_key_terms(&other_content);
        let other_tags: HashSet<String> = meta.tags.iter().map(|t| t.to_lowercase()).collect();

        let term_overlap = note_terms.intersection(&other_terms).count();
        let tag_overlap = note_tags.intersection(&other_tags).count();

        // Require meaningful overlap
        if term_overlap >= 2 || (tag_overlap >= 1 && term_overlap >= 1) {
            let confidence = ((term_overlap as f64 * 0.15) + (tag_overlap as f64 * 0.25)).min(0.9);
            candidates.push(ZettelLinkCandidate {
                target_id: meta.id.clone(),
                target_title: meta.title.clone(),
                link_type: "related".to_string(),
                confidence: (confidence * 100.0).round() / 100.0,
                reason: format!("Shared {} terms, {} tags", term_overlap, tag_overlap),
            });
        }
    }

    candidates.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    candidates.truncate(5);
    candidates
}

/// Strategy C: LLM-based discovery via OpenRouter
async fn find_llm_links(
    state: &AppState,
    note_id: &str,
    title: &str,
    content: &str,
    max_links: usize,
) -> Vec<ZettelLinkCandidate> {
    // Check if OpenRouter is configured
    let is_configured = {
        let or = state.openrouter.read().await;
        or.is_configured()
    };
    if !is_configured {
        return Vec::new();
    }

    // Get context notes (up to 15, excluding self and evidence notes)
    let context_notes: Vec<(String, String)> = {
        let store = state.knowledge_store.read().await;
        store
            .list_notes()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.id != note_id && m.status != NoteStatus::Evidence)
            .take(15)
            .map(|m| (m.id.clone(), m.title.clone()))
            .collect()
    };

    if context_notes.is_empty() {
        return Vec::new();
    }

    // Build context string
    let context_list = context_notes
        .iter()
        .enumerate()
        .map(|(i, (_, t))| format!("{}. {}", i + 1, t))
        .collect::<Vec<_>>()
        .join("\n");

    // Extract TL;DR if present
    let summary = content
        .lines()
        .skip_while(|l| !l.contains("## TL;DR"))
        .skip(1)
        .take_while(|l| !l.starts_with("## "))
        .collect::<Vec<_>>()
        .join(" ");

    let note_desc = if summary.trim().is_empty() {
        format!("TITLE: {}", title)
    } else {
        format!("TITLE: {}\nSUMMARY: {}", title, summary.trim())
    };

    let prompt = format!(
        "Given this note:\n{}\n\n\
        And these existing notes in the knowledge base:\n{}\n\n\
        Identify up to {} notes that are conceptually related. For each, provide:\n\
        - \"title\": exact title from the list above\n\
        - \"type\": one of: related, supports, contradicts, expands, questions, answers, example, part_of\n\
        - \"reason\": brief explanation (max 100 chars)\n\n\
        Respond ONLY with a JSON array. Example:\n\
        [{{\"title\": \"Note Title\", \"type\": \"related\", \"reason\": \"Both discuss X\"}}]",
        note_desc, context_list, max_links
    );

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let response = {
        let or = state.openrouter.read().await;
        match or
            .chat("anthropic/claude-3.5-haiku", messages, None, Some(0.2), Some(800))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                log::warn!("LLM link discovery failed: {}", e);
                return Vec::new();
            }
        }
    };

    // Parse JSON array from response
    parse_llm_links(&response, &context_notes, state).await
}

/// Parse the LLM JSON response into link candidates
async fn parse_llm_links(
    response: &str,
    context_notes: &[(String, String)],
    state: &AppState,
) -> Vec<ZettelLinkCandidate> {
    // Extract JSON array from response (may have surrounding text)
    let json_re = Regex::new(r"(?s)\[.*\]").unwrap();
    let json_str = match json_re.find(response) {
        Some(m) => m.as_str(),
        None => return Vec::new(),
    };

    let parsed: Vec<serde_json::Value> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    // Build title→ID lookup from context notes
    let title_map: HashMap<String, String> = context_notes
        .iter()
        .map(|(id, title)| (title.to_lowercase(), id.clone()))
        .collect();

    // Also use graph index for resolution
    let graph = state.graph_index.read().await;

    let valid_types: HashSet<&str> = [
        "related",
        "supports",
        "contradicts",
        "expands",
        "questions",
        "answers",
        "example",
        "part_of",
    ]
    .iter()
    .copied()
    .collect();

    let mut links = Vec::new();

    for item in &parsed {
        let title = match item.get("title").and_then(|v| v.as_str()) {
            Some(t) => t.trim().to_string(),
            None => continue,
        };

        // Resolve title to ID
        let target_id = title_map
            .get(&title.to_lowercase())
            .cloned()
            .or_else(|| graph.resolve_link(&title));

        let target_id = match target_id {
            Some(id) => id,
            None => continue,
        };

        let link_type = item
            .get("type")
            .and_then(|v| v.as_str())
            .filter(|t| valid_types.contains(t))
            .unwrap_or("related")
            .to_string();

        let reason = item
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .take(100)
            .collect::<String>();

        links.push(ZettelLinkCandidate {
            target_id,
            target_title: title,
            link_type,
            confidence: 0.75,
            reason,
        });
    }

    links
}

/// Deduplicate candidates, keeping the highest confidence for each target
fn deduplicate_links(links: Vec<ZettelLinkCandidate>) -> Vec<ZettelLinkCandidate> {
    let mut seen: HashMap<String, ZettelLinkCandidate> = HashMap::new();

    for link in links {
        let entry = seen.entry(link.target_id.clone()).or_insert(link.clone());
        if link.confidence > entry.confidence {
            *entry = link;
        }
    }

    seen.into_values().collect()
}

// ── Tauri commands ───────────────────────────────────────────────────────

/// Discover potential links for a note using multiple strategies
#[tauri::command]
pub async fn discover_links(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] noteId: String,
    mode: Option<String>,
    #[allow(non_snake_case)] maxLinks: Option<usize>,
) -> Result<DiscoverLinksResponse, String> {
    let mode = mode.unwrap_or_else(|| "suggested".to_string());
    let max_links = maxLinks.unwrap_or(10);

    if mode == "manual" {
        return Ok(DiscoverLinksResponse {
            note_id: noteId,
            links: Vec::new(),
        });
    }

    // Get the source note
    let (title, content, tags) = {
        let store = state.knowledge_store.read().await;
        let note = store.get_note(&noteId).map_err(|e| e.to_string())?;
        (note.title.clone(), note.content.clone(), note.tags.clone())
    };

    // Run all three strategies
    let search_links = find_search_links(&state, &noteId, &title, &content, max_links).await;
    let keyword_links = find_keyword_links(&state, &noteId, &content, &tags).await;
    let llm_links = find_llm_links(&state, &noteId, &title, &content, max_links).await;

    // Merge, deduplicate, sort by confidence
    let mut all_links = Vec::new();
    all_links.extend(search_links);
    all_links.extend(keyword_links);
    all_links.extend(llm_links);

    let mut links = deduplicate_links(all_links);
    links.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    links.truncate(max_links);

    Ok(DiscoverLinksResponse {
        note_id: noteId,
        links,
    })
}

/// Apply discovered links to a note (creates bidirectional wikilinks)
#[tauri::command]
pub async fn apply_links(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] noteId: String,
    request: ApplyLinksRequest,
) -> Result<ApplyLinksResponse, String> {
    // First discover links to get the full candidate list
    let candidates = {
        let store = state.knowledge_store.read().await;
        let note = store.get_note(&noteId).map_err(|e| e.to_string())?;

        // Re-discover to get candidate details (the frontend only sends target IDs)
        let (title, content, tags) = (note.title.clone(), note.content.clone(), note.tags.clone());
        drop(store); // release lock before async calls

        let search_links = find_search_links(&state, &noteId, &title, &content, 20).await;
        let keyword_links = find_keyword_links(&state, &noteId, &content, &tags).await;
        let llm_links = find_llm_links(&state, &noteId, &title, &content, 20).await;

        let mut all = Vec::new();
        all.extend(search_links);
        all.extend(keyword_links);
        all.extend(llm_links);
        deduplicate_links(all)
    };

    // Build lookup of requested target IDs
    let requested: HashSet<String> = request.link_ids.iter().cloned().collect();
    let to_apply: Vec<&ZettelLinkCandidate> = candidates
        .iter()
        .filter(|c| requested.contains(&c.target_id))
        .collect();

    let mut links_created = 0;

    for candidate in &to_apply {
        // Add forward link (source → target)
        let source_updated = {
            let store = state.knowledge_store.read().await;
            let source = store.get_note(&noteId).map_err(|e| e.to_string())?;

            if let Some(new_content) =
                add_wikilink_to_content(&source.content, &candidate.target_title, &candidate.link_type)
            {
                drop(store);
                let mut store = state.knowledge_store.write().await;
                store
                    .update_note(
                        &noteId,
                        NoteUpdate {
                            content: Some(new_content),
                            ..Default::default()
                        },
                    )
                    .map_err(|e| e.to_string())?;
                true
            } else {
                false
            }
        };

        // Add reverse link (target → source)
        let reverse_type = reverse_link_type(&candidate.link_type);
        let source_title = {
            let store = state.knowledge_store.read().await;
            store
                .get_note(&noteId)
                .map(|n| n.title.clone())
                .unwrap_or_default()
        };

        {
            let store = state.knowledge_store.read().await;
            let target = match store.get_note(&candidate.target_id) {
                Ok(t) => t,
                Err(_) => continue,
            };

            if let Some(new_content) =
                add_wikilink_to_content(&target.content, &source_title, reverse_type)
            {
                drop(store);
                let mut store = state.knowledge_store.write().await;
                let _ = store.update_note(
                    &candidate.target_id,
                    NoteUpdate {
                        content: Some(new_content),
                        ..Default::default()
                    },
                );
            }
        }

        if source_updated {
            links_created += 1;
        }

        // Update search index for both notes
        {
            let store = state.knowledge_store.read().await;
            let mut search = state.search_service.write().await;

            if let Ok(source) = store.get_note(&noteId) {
                let _ = search.index_note(&source);
            }
            if let Ok(target) = store.get_note(&candidate.target_id) {
                let _ = search.index_note(&target);
            }
            let _ = search.commit();
        }

        // Update graph
        {
            let store = state.knowledge_store.read().await;
            let mut graph = state.graph_index.write().await;

            if let Ok(source) = store.get_note(&noteId) {
                graph.update_note(&source);
            }
            if let Ok(target) = store.get_note(&candidate.target_id) {
                graph.update_note(&target);
            }
        }
    }

    Ok(ApplyLinksResponse {
        note_id: noteId,
        links_created,
        links_attempted: request.link_ids.len(),
    })
}

/// Create a single bidirectional link between two notes
#[tauri::command]
pub async fn create_link(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] sourceId: String,
    #[allow(non_snake_case)] targetId: String,
    #[allow(non_snake_case)] linkType: Option<String>,
) -> Result<CreateLinkResponse, String> {
    let link_type = linkType.unwrap_or_else(|| "related".to_string());

    // Get both notes
    let (source_title, target_title) = {
        let store = state.knowledge_store.read().await;
        let source = store.get_note(&sourceId).map_err(|e| e.to_string())?;
        let target = store.get_note(&targetId).map_err(|e| e.to_string())?;
        (source.title.clone(), target.title.clone())
    };

    // Forward link: source → target
    {
        let store = state.knowledge_store.read().await;
        let source = store.get_note(&sourceId).map_err(|e| e.to_string())?;

        if let Some(new_content) = add_wikilink_to_content(&source.content, &target_title, &link_type) {
            drop(store);
            let mut store = state.knowledge_store.write().await;
            store
                .update_note(
                    &sourceId,
                    NoteUpdate {
                        content: Some(new_content),
                        ..Default::default()
                    },
                )
                .map_err(|e| e.to_string())?;
        }
    }

    // Reverse link: target → source
    let reverse = reverse_link_type(&link_type);
    {
        let store = state.knowledge_store.read().await;
        let target = store.get_note(&targetId).map_err(|e| e.to_string())?;

        if let Some(new_content) = add_wikilink_to_content(&target.content, &source_title, reverse) {
            drop(store);
            let mut store = state.knowledge_store.write().await;
            let _ = store.update_note(
                &targetId,
                NoteUpdate {
                    content: Some(new_content),
                    ..Default::default()
                },
            );
        }
    }

    // Update search index + graph for both notes
    {
        let store = state.knowledge_store.read().await;
        let mut search = state.search_service.write().await;

        if let Ok(source) = store.get_note(&sourceId) {
            let _ = search.index_note(&source);
        }
        if let Ok(target) = store.get_note(&targetId) {
            let _ = search.index_note(&target);
        }
        let _ = search.commit();
    }

    {
        let store = state.knowledge_store.read().await;
        let mut graph = state.graph_index.write().await;

        if let Ok(source) = store.get_note(&sourceId) {
            graph.update_note(&source);
        }
        if let Ok(target) = store.get_note(&targetId) {
            graph.update_note(&target);
        }
    }

    Ok(CreateLinkResponse {
        status: "linked".to_string(),
        source: sourceId,
        target: targetId,
        link_type,
    })
}

/// Get available link type definitions
#[tauri::command]
pub async fn get_link_types() -> Result<Vec<serde_json::Value>, String> {
    Ok(link_type_definitions())
}
