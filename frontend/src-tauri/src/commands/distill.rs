use crate::models::note::{
    DistillRequest, DistillResponse, HubUpdate, NoteCreate, NoteStatus, NoteUpdate,
};
use crate::services::openrouter::ChatMessage;
use crate::AppState;
use lazy_static::lazy_static;
use regex::Regex;
use tauri::State;

lazy_static! {
    /// Inline tag pattern: # followed by letter, then letters/digits/hyphens/underscores
    /// Must not be preceded by ` or # (to skip code and headings)
    static ref INLINE_TAG_RE: Regex =
        Regex::new(r"(?<![`#])#([a-zA-Z][a-zA-Z0-9_/\-]+)").unwrap();
    /// Fenced code block removal
    static ref FENCED_CODE_RE: Regex = Regex::new(r"(?s)```.*?```").unwrap();
    /// Inline code removal
    static ref INLINE_CODE_RE: Regex = Regex::new(r"`[^`]+`").unwrap();
}

// ── Tag utilities ──────────────────────────────────────────────────────────

/// Normalize a single tag: lowercase, strip leading #, spaces→hyphens
fn normalize_tag(tag: &str) -> String {
    tag.trim_start_matches('#')
        .to_lowercase()
        .trim()
        .replace(' ', "-")
}

/// Normalize and deduplicate a list of tags
fn normalize_all_tags(tags: &[String]) -> Vec<String> {
    let mut set: std::collections::HashSet<String> = tags.iter().map(|t| normalize_tag(t)).collect();
    // Remove empty strings
    set.remove("");
    let mut sorted: Vec<String> = set.into_iter().collect();
    sorted.sort();
    sorted
}

/// Parse inline #tags from markdown, ignoring code blocks and headings
fn parse_inline_tags(content: &str) -> Vec<String> {
    // 1. Remove fenced code blocks
    let clean = FENCED_CODE_RE.replace_all(content, "");
    // 2. Remove inline code
    let clean = INLINE_CODE_RE.replace_all(&clean, "");
    // 3. Find tags
    let tags: std::collections::HashSet<String> = INLINE_TAG_RE
        .captures_iter(&clean)
        .filter_map(|cap| cap.get(1).map(|m| normalize_tag(m.as_str())))
        .collect();
    tags.into_iter().collect()
}

/// Suggest a hub title from tags, preferring the tag most relevant to the section title.
///
/// Priority: tag whose hyphen-split words appear most often in the title wins.
/// Ties broken by tag length (longer = more specific). Falls back to first significant tag.
fn suggest_hub(title: &str, tags: &[String]) -> Option<String> {
    let title_lower = title.to_lowercase();

    let significant_tags: Vec<&String> = tags
        .iter()
        .filter(|t| *t != "grafyn" && *t != "draft")
        .collect();

    if significant_tags.is_empty() {
        return None;
    }

    // Score each tag by how many of its words appear in the title
    let best = significant_tags
        .iter()
        .map(|tag| {
            let word_matches = tag
                .split('-')
                .filter(|w| !w.is_empty() && title_lower.contains(*w))
                .count();
            (tag, word_matches, tag.len())
        })
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)))
        .map(|(tag, _, _)| *tag);

    let chosen = best.unwrap_or(&significant_tags[0]);

    // Convert "my-tag" → "Hub: My Tag"
    let title_case: String = chosen
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some(format!("Hub: {}", title_case))
}

// ── Candidate extraction (rules-based) ─────────────────────────────────────

struct AtomicCandidate {
    title: String,
    summary: Vec<String>,
    recommended_tags: Vec<String>,
    suggested_hub: Option<String>,
}

/// META_SECTIONS that should be skipped during extraction
const META_SECTIONS: &[&str] = &[
    "metadata",
    "extracted atomic notes",
    "sources",
    "updates",
];

/// Extract atomic candidates by splitting on H2 headings (rules-based)
fn extract_candidates_rules(
    content: &str,
    container_tags: &[String],
) -> Vec<AtomicCandidate> {
    let mut candidates = Vec::new();

    // Split on \n## (keeping content before first ## is skipped)
    let sections: Vec<&str> = content.split("\n## ").collect();

    for section in sections.iter().skip(1) {
        let lines: Vec<&str> = section.lines().collect();
        if lines.is_empty() {
            continue;
        }

        let title = lines[0].trim().to_string();
        let body = lines[1..].join("\n").trim().to_string();

        // Skip meta sections
        if META_SECTIONS.contains(&title.to_lowercase().as_str()) {
            continue;
        }

        // Skip if too short
        if body.len() < 50 {
            continue;
        }

        // Extract summary bullets (up to 5)
        let summary: Vec<String> = body
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if trimmed.starts_with("- ") {
                    Some(trimmed[2..].to_string())
                } else if trimmed.starts_with("* ") {
                    Some(trimmed[2..].to_string())
                } else {
                    None
                }
            })
            .take(5)
            .collect();

        // Extract inline tags from section
        let section_tags = parse_inline_tags(&body);

        // Inherit container tags (excluding meta tags)
        let inherited: Vec<String> = container_tags
            .iter()
            .filter(|t| !["chat", "canvas-export", "evidence"].contains(&t.as_str()))
            .cloned()
            .collect();

        // Merge and limit to 5
        let mut all_tags: std::collections::HashSet<String> = section_tags.into_iter().collect();
        all_tags.extend(inherited);
        let mut recommended_tags: Vec<String> = all_tags.into_iter().collect();
        recommended_tags.sort();
        recommended_tags.truncate(5);

        let suggested_hub = suggest_hub(&title, &recommended_tags);

        let summary = if summary.is_empty() {
            if body.len() > 200 {
                vec![format!("{}...", &body[..200])]
            } else {
                vec![body.clone()]
            }
        } else {
            summary
        };

        candidates.push(AtomicCandidate {
            title: format!("Atomic: {}", title),
            summary,
            recommended_tags,
            suggested_hub,
        });
    }

    candidates
}

// ── LLM summarization ──────────────────────────────────────────────────────

/// Summarize note content using LLM, returning structured markdown with H2 headings
async fn summarize_with_llm(
    content: &str,
    state: &AppState,
) -> Result<String, String> {
    let prompt = format!(
        "Summarize this note into atomic knowledge units. For each distinct concept or insight, create a section with:\n\
        - A clear descriptive title (not \"Prompt 1\" or generic names)\n\
        - 3-5 bullet point summary\n\
        - Key claims or insights\n\
        - Any open questions\n\
        \n\
        Note content:\n\
        ---\n\
        {}\n\
        ---\n\
        \n\
        Format your response as markdown with ## headings for each atomic unit.",
        content
    );

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let openrouter = state.openrouter.read().await;
    openrouter
        .chat(
            "anthropic/claude-3-haiku",
            messages,
            None,
            Some(0.7),
            Some(2048),
        )
        .await
        .map_err(|e| format!("LLM summarization failed: {}", e))
}

// ── Hub management ─────────────────────────────────────────────────────────

/// Create or update a hub note, returning a HubUpdate if successful
async fn update_hub(
    hub_title: &str,
    atomic_id: &str,
    state: &AppState,
) -> Option<HubUpdate> {
    let hub_id = hub_title
        .replace(' ', "_")
        .replace(':', "")
        .to_lowercase();

    // Check if hub exists
    let hub_exists = {
        let store = state.knowledge_store.read().await;
        store.get_note(&hub_id).ok()
    };

    if let Some(hub) = hub_exists {
        // Update existing hub — add link if not present
        let link = format!("[[{}]]", atomic_id);
        if hub.content.contains(&link) {
            return Some(HubUpdate {
                hub_id,
                hub_title: hub_title.to_string(),
                action: "unchanged".to_string(),
                atomic_ids_added: vec![atomic_id.to_string()],
            });
        }

        let new_content = if hub.content.contains("## Atomic Notes") {
            hub.content.replace(
                "## Atomic Notes",
                &format!("## Atomic Notes\n- {}", link),
            )
        } else {
            format!("{}\n\n## Atomic Notes\n- {}", hub.content, link)
        };

        let update = NoteUpdate {
            content: Some(new_content),
            ..Default::default()
        };

        let updated = {
            let mut store = state.knowledge_store.write().await;
            store.update_note(&hub_id, update).ok()
        };

        if let Some(updated_note) = updated {
            // Update search + graph
            {
                let mut search = state.search_service.write().await;
                let _ = search.index_note(&updated_note);
                let _ = search.commit();
            }
            {
                let mut graph = state.graph_index.write().await;
                graph.update_note(&updated_note);
            }
        }

        Some(HubUpdate {
            hub_id,
            hub_title: hub_title.to_string(),
            action: "updated".to_string(),
            atomic_ids_added: vec![atomic_id.to_string()],
        })
    } else {
        // Create new hub
        let content = format!(
            "# {}\n\n\
            ## Stance / Current Summary\n\
            <!-- Brief overview of this topic -->\n\n\
            ## Atomic Notes\n\
            - [[{}]]\n\n\
            ## Open Questions\n\
            <!-- Unanswered questions -->\n\n\
            ## Related Hubs\n\
            <!-- Links to adjacent topic hubs -->\n",
            hub_title, atomic_id
        );

        let hub_create = NoteCreate {
            title: hub_title.to_string(),
            content,
            status: NoteStatus::Draft,
            tags: vec!["hub".to_string(), "grafyn".to_string()],
            properties: Default::default(),
        };

        let created = {
            let mut store = state.knowledge_store.write().await;
            store.create_note(hub_create).ok()
        };

        if let Some(created_note) = created {
            // Update search + graph
            {
                let mut search = state.search_service.write().await;
                let _ = search.index_note(&created_note);
                let _ = search.commit();
            }
            {
                let mut graph = state.graph_index.write().await;
                graph.update_note(&created_note);
            }

            Some(HubUpdate {
                hub_id: created_note.id,
                hub_title: hub_title.to_string(),
                action: "created".to_string(),
                atomic_ids_added: vec![atomic_id.to_string()],
            })
        } else {
            log::error!("Failed to create hub: {}", hub_title);
            None
        }
    }
}

// ── Main distill command ───────────────────────────────────────────────────

/// Distill a container note into atomic draft notes (AUTO mode)
#[tauri::command]
pub async fn distill_note(
    id: String,
    request: DistillRequest,
    state: State<'_, AppState>,
) -> Result<DistillResponse, String> {
    // 1. Get the container note
    let note = {
        let store = state.knowledge_store.read().await;
        store.get_note(&id).map_err(|e| e.to_string())?
    };

    // 2. Determine extraction method
    let openrouter_configured = {
        let or = state.openrouter.read().await;
        or.is_configured()
    };

    let use_llm = match request.extraction_method.as_str() {
        "llm" => {
            if !openrouter_configured {
                return Err("LLM extraction requested but OpenRouter API key not configured".into());
            }
            true
        }
        "rules" => false,
        _ => {
            // "auto" — prefer LLM if available
            openrouter_configured
        }
    };

    let extraction_method_used = if use_llm { "llm" } else { "rules" };

    // 3. Extract candidates
    let candidates = if use_llm {
        // LLM path: summarize then extract from summary
        match summarize_with_llm(&note.content, &state).await {
            Ok(summary) => extract_candidates_rules(&summary, &note.tags),
            Err(e) => {
                log::warn!("LLM summarization failed, falling back to rules: {}", e);
                // Fallback to rules
                extract_candidates_rules(&note.content, &note.tags)
            }
        }
    } else {
        extract_candidates_rules(&note.content, &note.tags)
    };

    if candidates.is_empty() {
        return Ok(DistillResponse {
            created_note_ids: vec![],
            hub_updates: vec![],
            container_updated: false,
            message: "No atomic note candidates found in this note".to_string(),
            extraction_method_used: extraction_method_used.to_string(),
            status: "completed".to_string(),
        });
    }

    // 4. Create atomic notes
    let mut created_ids: Vec<String> = Vec::new();
    let mut hub_updates: Vec<HubUpdate> = Vec::new();

    for candidate in &candidates {
        let mut tags = candidate.recommended_tags.clone();
        tags.push("draft".to_string());
        let tags = normalize_all_tags(&tags);

        // Build content matching Python's _create_atomic_note template
        let summary_list: String = candidate
            .summary
            .iter()
            .map(|s| format!("- {}", s))
            .collect::<Vec<_>>()
            .join("\n");

        let content = format!(
            "# {}\n\n\
            ## TL;DR\n\
            {}\n\n\
            ## Details\n\
            <!-- Expand on the key points here -->\n\n\
            ## Sources\n\
            - [[{}]]\n\n\
            ## Updates\n\
            <!-- Future updates appended here with date headers -->\n",
            candidate.title, summary_list, note.title
        );

        let note_create = NoteCreate {
            title: candidate.title.clone(),
            content,
            status: NoteStatus::Draft,
            tags,
            properties: Default::default(),
        };

        // Create the note
        let created = {
            let mut store = state.knowledge_store.write().await;
            store.create_note(note_create).map_err(|e| e.to_string())?
        };

        // Index in search
        {
            let mut search = state.search_service.write().await;
            if let Err(e) = search.index_note(&created) {
                log::error!("Failed to index atomic note '{}': {}", created.id, e);
            }
            if let Err(e) = search.commit() {
                log::error!("Failed to commit after indexing '{}': {}", created.id, e);
            }
        }

        // Update graph
        {
            let mut graph = state.graph_index.write().await;
            graph.update_note(&created);
        }

        created_ids.push(created.id.clone());

        // Auto-create/update hub
        if let Some(ref hub_title) = candidate.suggested_hub {
            if let Some(hu) = update_hub(hub_title, &created.id, &state).await {
                hub_updates.push(hu);
            }
        }
    }

    // 5. Update container note with extracted links
    let container_updated = if !created_ids.is_empty() {
        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string();
        let links: String = created_ids
            .iter()
            .map(|aid| format!("- [[{}]]", aid))
            .collect::<Vec<_>>()
            .join("\n");

        let section_content = format!("\n*Last distilled: {}*\n\n{}\n", timestamp, links);
        let section_header = "## Extracted Atomic Notes";

        let new_content = if let Some(start) = note.content.find(section_header) {
            // Find where this section ends (next ## or end of string)
            let after_header = start + section_header.len();
            let end = note.content[after_header..]
                .find("\n## ")
                .map(|pos| after_header + pos)
                .unwrap_or(note.content.len());
            format!(
                "{}\n{}{}",
                &note.content[..after_header],
                section_content,
                &note.content[end..]
            )
        } else {
            format!("{}\n\n{}\n{}", note.content, section_header, section_content)
        };

        let update = NoteUpdate {
            content: Some(new_content),
            ..Default::default()
        };

        let updated = {
            let mut store = state.knowledge_store.write().await;
            store.update_note(&id, update).ok()
        };

        if let Some(updated_note) = updated {
            {
                let mut search = state.search_service.write().await;
                let _ = search.index_note(&updated_note);
                let _ = search.commit();
            }
            {
                let mut graph = state.graph_index.write().await;
                graph.update_note(&updated_note);
            }
            true
        } else {
            false
        }
    } else {
        false
    };

    let count = created_ids.len();
    Ok(DistillResponse {
        created_note_ids: created_ids,
        hub_updates,
        container_updated,
        message: format!(
            "Auto-created {} draft atomic notes using {}",
            count, extraction_method_used
        ),
        extraction_method_used: extraction_method_used.to_string(),
        status: "completed".to_string(),
    })
}

/// Normalize tags for a note (merges inline #tags into YAML frontmatter)
#[tauri::command]
pub async fn normalize_tags(
    id: String,
    state: State<'_, AppState>,
) -> Result<crate::models::note::Note, String> {
    // Get the note
    let note = {
        let store = state.knowledge_store.read().await;
        store.get_note(&id).map_err(|e| e.to_string())?
    };

    // Parse inline tags from content
    let inline_tags = parse_inline_tags(&note.content);

    // Merge with existing tags
    let mut all_tags: std::collections::HashSet<String> =
        note.tags.iter().map(|t| normalize_tag(t)).collect();
    all_tags.extend(inline_tags);
    all_tags.remove("");

    let merged: Vec<String> = {
        let mut v: Vec<String> = all_tags.into_iter().collect();
        v.sort();
        v
    };

    // Only update if tags changed
    let existing_normalized: Vec<String> = normalize_all_tags(&note.tags);
    if merged == existing_normalized {
        return Ok(note);
    }

    let update = NoteUpdate {
        tags: Some(merged),
        ..Default::default()
    };

    let updated = {
        let mut store = state.knowledge_store.write().await;
        store.update_note(&id, update).map_err(|e| e.to_string())?
    };

    // Update search + graph
    {
        let mut search = state.search_service.write().await;
        let _ = search.index_note(&updated);
        let _ = search.commit();
    }
    {
        let mut graph = state.graph_index.write().await;
        graph.update_note(&updated);
    }

    Ok(updated)
}
