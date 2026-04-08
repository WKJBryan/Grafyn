use crate::commands::{sync_chunk_index_for_notes, sync_link_discovery_for_notes};
use crate::models::link_discovery::{
    DismissLinkSuggestionResponse, LinkDiscoveryStatus, LinkSuggestionQueueEntry,
};
use crate::models::note::{
    ApplyLinksRequest, ApplyLinksResponse, CreateLinkResponse, DiscoverLinksResponse, NoteUpdate,
    RelationType, ZettelLinkCandidate,
};
use crate::services::link_discovery::discover_for_note;
use crate::AppState;
use std::collections::{HashMap, HashSet};
use tauri::State;

// ── Link type definitions ────────────────────────────────────────────────

/// Link type definitions derived from the RelationType enum
fn link_type_definitions() -> Vec<serde_json::Value> {
    let types = [
        (
            RelationType::Related,
            "Related",
            "General topical relationship",
        ),
        (
            RelationType::Supports,
            "Supports",
            "Provides evidence or backing",
        ),
        (
            RelationType::Contradicts,
            "Contradicts",
            "Presents opposing evidence",
        ),
        (
            RelationType::Expands,
            "Expands",
            "Elaborates on the concept",
        ),
        (
            RelationType::Questions,
            "Questions",
            "Raises questions about",
        ),
        (RelationType::Answers, "Answers", "Answers questions from"),
        (
            RelationType::Example,
            "Example",
            "Provides a concrete example",
        ),
        (RelationType::PartOf, "Part Of", "Is a component of"),
    ];
    types
        .iter()
        .map(|(rt, label, desc)| {
            serde_json::json!({
                "id": rt.to_string(),
                "label": label,
                "description": desc,
                "reverse": rt.reverse().to_string(),
            })
        })
        .collect()
}

/// Get the reverse link type for bidirectional linking
fn reverse_link_type(link_type: &str) -> String {
    RelationType::from_str_lossy(link_type)
        .reverse()
        .to_string()
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
    let related_idx = lines.iter().position(|l| l.trim() == "## Related Concepts");
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoverMode {
    Manual,
    Algorithm,
    Llm,
}

impl DiscoverMode {
    fn parse(mode: Option<&str>) -> Self {
        match mode.unwrap_or("suggested").to_ascii_lowercase().as_str() {
            "manual" => Self::Manual,
            "algorithm" => Self::Algorithm,
            "llm" | "suggested" => Self::Llm,
            _ => Self::Llm,
        }
    }

    #[cfg(test)]
    fn include_llm(self) -> bool {
        matches!(self, Self::Llm)
    }
}

fn to_service_mode(mode: DiscoverMode) -> crate::services::link_discovery::DiscoverMode {
    match mode {
        DiscoverMode::Manual => crate::services::link_discovery::DiscoverMode::Manual,
        DiscoverMode::Algorithm => crate::services::link_discovery::DiscoverMode::Algorithm,
        DiscoverMode::Llm => crate::services::link_discovery::DiscoverMode::Llm,
    }
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
    let discover_mode = DiscoverMode::parse(mode.as_deref());
    let max_links = maxLinks.unwrap_or(10);
    discover_for_note(
        state.inner(),
        &noteId,
        to_service_mode(discover_mode),
        max_links,
        true,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::DiscoverMode;

    #[test]
    fn parses_discover_modes() {
        assert_eq!(DiscoverMode::parse(None), DiscoverMode::Llm);
        assert_eq!(DiscoverMode::parse(Some("suggested")), DiscoverMode::Llm);
        assert_eq!(DiscoverMode::parse(Some("llm")), DiscoverMode::Llm);
        assert_eq!(
            DiscoverMode::parse(Some("algorithm")),
            DiscoverMode::Algorithm
        );
        assert_eq!(DiscoverMode::parse(Some("manual")), DiscoverMode::Manual);
    }

    #[test]
    fn unknown_modes_default_to_llm() {
        assert_eq!(DiscoverMode::parse(Some("unexpected")), DiscoverMode::Llm);
    }

    #[test]
    fn include_llm_only_for_llm_mode() {
        assert!(DiscoverMode::Llm.include_llm());
        assert!(!DiscoverMode::Algorithm.include_llm());
        assert!(!DiscoverMode::Manual.include_llm());
    }
}

/// Apply discovered links to a note (creates bidirectional wikilinks)
#[tauri::command]
pub async fn apply_links(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] noteId: String,
    request: ApplyLinksRequest,
) -> Result<ApplyLinksResponse, String> {
    let requested_candidates = if !request.candidates.is_empty() {
        deduplicate_links(request.candidates.clone())
    } else {
        // Backward-compatibility path for older callers that only send IDs.
        let response = discover_for_note(
            state.inner(),
            &noteId,
            crate::services::link_discovery::DiscoverMode::Llm,
            20,
            true,
        )
        .await?;
        let candidates = deduplicate_links(
            response
                .links
                .into_iter()
                .chain(response.exploratory_links.into_iter())
                .collect(),
        );

        let requested: HashSet<String> = request.link_ids.iter().cloned().collect();
        candidates
            .into_iter()
            .filter(|c| requested.contains(&c.target_id))
            .collect()
    };
    let links_attempted = requested_candidates.len();

    let mut links_created = 0;
    let mut dirty_note_ids: HashSet<String> = HashSet::new();

    for candidate in &requested_candidates {
        let (target_title, target_content) = {
            let store = state.knowledge_store.read().await;
            let target = match store.get_note(&candidate.target_id) {
                Ok(t) => t,
                Err(_) => continue,
            };
            (target.title.clone(), target.content.clone())
        };
        let mut target_updated = false;

        // Add forward link (source → target)
        let source_updated = {
            let store = state.knowledge_store.read().await;
            let source = store.get_note(&noteId).map_err(|e| e.to_string())?;

            if let Some(new_content) =
                add_wikilink_to_content(&source.content, &target_title, &candidate.link_type)
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
            if let Some(new_content) =
                add_wikilink_to_content(&target_content, &source_title, &reverse_type)
            {
                let mut store = state.knowledge_store.write().await;
                target_updated = store
                    .update_note(
                        &candidate.target_id,
                        NoteUpdate {
                            content: Some(new_content),
                            ..Default::default()
                        },
                    )
                    .is_ok();
            }
        }

        if source_updated {
            links_created += 1;
            dirty_note_ids.insert(noteId.clone());
        }
        if target_updated {
            dirty_note_ids.insert(candidate.target_id.clone());
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

    if !dirty_note_ids.is_empty() {
        let dirty_notes = {
            let store = state.knowledge_store.read().await;
            dirty_note_ids
                .iter()
                .filter_map(|id| store.get_note(id).ok())
                .collect::<Vec<_>>()
        };
        sync_chunk_index_for_notes(state.inner(), &dirty_notes).await;
        sync_link_discovery_for_notes(state.inner(), &dirty_notes).await;
        state.link_discovery.write().await.record_links_applied(
            &noteId,
            &requested_candidates
                .iter()
                .map(|candidate| candidate.target_id.clone())
                .collect::<Vec<_>>(),
        );
    }

    Ok(ApplyLinksResponse {
        note_id: noteId,
        links_created,
        links_attempted,
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
    let mut dirty_note_ids: HashSet<String> = HashSet::new();

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

        if let Some(new_content) =
            add_wikilink_to_content(&source.content, &target_title, &link_type)
        {
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
            dirty_note_ids.insert(sourceId.clone());
        }
    }

    // Reverse link: target → source
    let reverse = reverse_link_type(&link_type);
    {
        let store = state.knowledge_store.read().await;
        let target = store.get_note(&targetId).map_err(|e| e.to_string())?;

        if let Some(new_content) = add_wikilink_to_content(&target.content, &source_title, &reverse)
        {
            drop(store);
            let mut store = state.knowledge_store.write().await;
            let _ = store.update_note(
                &targetId,
                NoteUpdate {
                    content: Some(new_content),
                    ..Default::default()
                },
            );
            dirty_note_ids.insert(targetId.clone());
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

    if !dirty_note_ids.is_empty() {
        let dirty_notes = {
            let store = state.knowledge_store.read().await;
            dirty_note_ids
                .iter()
                .filter_map(|id| store.get_note(id).ok())
                .collect::<Vec<_>>()
        };
        sync_chunk_index_for_notes(state.inner(), &dirty_notes).await;
        sync_link_discovery_for_notes(state.inner(), &dirty_notes).await;
        state
            .link_discovery
            .write()
            .await
            .record_links_applied(&sourceId, std::slice::from_ref(&targetId));
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

/// List cached link suggestions for the global inbox.
#[tauri::command]
pub async fn list_link_suggestion_queue(
    state: State<'_, AppState>,
    status: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<LinkSuggestionQueueEntry>, String> {
    let discovery = state.link_discovery.read().await;
    Ok(discovery.list_queue_entries(status.as_deref(), limit.unwrap_or(25)))
}

/// Dismiss a cached suggestion so it does not reappear until the note changes again.
#[tauri::command]
pub async fn dismiss_link_suggestion(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] noteId: String,
    #[allow(non_snake_case)] targetId: String,
) -> Result<DismissLinkSuggestionResponse, String> {
    let mut discovery = state.link_discovery.write().await;
    Ok(discovery.dismiss_suggestion(&noteId, &targetId))
}

/// Get background discovery worker status and queue metrics.
#[tauri::command]
pub async fn get_link_discovery_status(
    state: State<'_, AppState>,
) -> Result<LinkDiscoveryStatus, String> {
    let settings = state.settings_service.read().await;
    let discovery = state.link_discovery.read().await;
    Ok(discovery.status(settings.get()))
}
