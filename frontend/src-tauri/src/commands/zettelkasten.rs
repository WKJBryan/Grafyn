use crate::commands::enqueue_vault_optimizer_note;
use crate::models::link_discovery::{
    DismissLinkSuggestionResponse, LinkDiscoveryStatus, LinkSuggestionQueueEntry,
};
use crate::models::note::{
    ApplyLinksRequest, ApplyLinksResponse, CreateLinkResponse, DiscoverLinksResponse, NoteUpdate,
    RelationType, ZettelLinkCandidate,
};
use crate::services::link_discovery::{discover_for_note, DiscoverMode};
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

// ── Tauri commands ───────────────────────────────────────────────────────

async fn complete_link_note_mutation(
    state: &AppState,
    expected: &crate::services::vault_namespace::VaultAuthorityTokenV1,
    prior_commit: Option<&crate::services::twin_events::MutationCommit>,
    mutation: anyhow::Result<(
        crate::models::note::Note,
        crate::services::twin_events::MutationCommit,
    )>,
    operation: &str,
) -> Result<crate::commands::CompletedKnowledgeNoteMutation, String> {
    crate::commands::complete_knowledge_note_mutation(
        state,
        expected,
        prior_commit,
        mutation,
        operation,
    )
    .await
}

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
    discover_for_note(state.inner(), &noteId, discover_mode, max_links, true).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{NoteCreate, NoteStatus};
    use crate::services::link_discovery::DiscoverMode;

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

    #[tokio::test]
    async fn link_mutation_recovers_exact_post_authority_commit_and_marks_note_dirty() {
        let (mut state, vault, _data) =
            crate::commands::commit_note_write_tests::build_test_state();
        let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
        state.knowledge_store = std::sync::Arc::new(tokio::sync::RwLock::new(
            crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
                vault.path().to_path_buf(),
                coordinator.current_namespace_path().unwrap(),
                coordinator.clone(),
            ),
        ));
        let (created, create_commit) = {
            let expected = coordinator.current_authority_token().unwrap();
            state
                .knowledge_store
                .write()
                .await
                .create_note_expecting_authority(
                    NoteCreate {
                        title: "Recovered link source".into(),
                        content: "before".into(),
                        relative_path: Some("recovered-link-source.md".into()),
                        aliases: Vec::new(),
                        status: NoteStatus::Draft,
                        tags: Vec::new(),
                        schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                        migration_source: None,
                        optimizer_managed: false,
                        properties: Default::default(),
                    },
                    "note_editor",
                    expected,
                )
                .unwrap()
        };
        assert!(matches!(
            crate::commands::repair_after_authority_mutation(
                &state,
                &create_commit,
                "link test setup"
            )
            .await,
            crate::commands::PostAuthorityRepair::Ready(_)
        ));
        let expected = coordinator.current_authority_token().unwrap();
        coordinator.fail_next_replays_before_targets(2);
        let mutation = {
            let mut store = state.knowledge_store.write().await;
            store.update_note_expecting_authority(
                &created.id,
                NoteUpdate {
                    content: Some("before\n\n## Related Concepts\n- [[Target]] (related)".into()),
                    ..Default::default()
                },
                "note_editor",
                expected.clone(),
            )
        };
        let exact = crate::services::knowledge_store::knowledge_authority_advanced_outcome(
            mutation.as_ref().unwrap_err(),
        )
        .unwrap()
        .commit;

        let completed =
            complete_link_note_mutation(&state, &expected, None, mutation, "applied note link")
                .await
                .expect("the committed link should recover without retry");

        assert_eq!(completed.note.id, created.id);
        let recovered_commit = completed.commit.as_ref().unwrap();
        assert_eq!(recovered_commit.mutation_id, exact.mutation_id);
        assert_eq!(recovered_commit.authority_token, exact.authority_token);
        assert_eq!(
            recovered_commit.postcommit_warning,
            exact.postcommit_warning
        );
        assert!(completed.repaired);
        assert!(completed.note.content.contains("[[Target]]"));
        assert_eq!(coordinator.pending_count().unwrap(), 0);
    }

    #[tokio::test]
    async fn reverse_link_failure_repairs_forward_commit_and_forbids_blind_retry() {
        let (mut state, vault, _data) =
            crate::commands::commit_note_write_tests::build_test_state();
        let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
        state.knowledge_store = std::sync::Arc::new(tokio::sync::RwLock::new(
            crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
                vault.path().to_path_buf(),
                coordinator.current_namespace_path().unwrap(),
                coordinator.clone(),
            ),
        ));
        let (created, create_commit) = state
            .knowledge_store
            .write()
            .await
            .create_note_expecting_authority(
                NoteCreate {
                    title: "Partial link source".into(),
                    content: "before".into(),
                    relative_path: Some("partial-link-source.md".into()),
                    aliases: Vec::new(),
                    status: NoteStatus::Draft,
                    tags: Vec::new(),
                    schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                    migration_source: None,
                    optimizer_managed: false,
                    properties: Default::default(),
                },
                "note_editor",
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap();
        assert!(matches!(
            crate::commands::repair_after_authority_mutation(
                &state,
                &create_commit,
                "partial link setup"
            )
            .await,
            crate::commands::PostAuthorityRepair::Ready(_)
        ));
        let (_, forward_commit) = state
            .knowledge_store
            .write()
            .await
            .update_note_expecting_authority(
                &created.id,
                NoteUpdate {
                    content: Some("before\n\n## Related Concepts\n- [[Target]] (related)".into()),
                    ..Default::default()
                },
                "note_editor",
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap();
        let expected = forward_commit.authority_token.clone().unwrap();
        let reverse_failure: anyhow::Result<(
            crate::models::note::Note,
            crate::services::twin_events::MutationCommit,
        )> = Err(anyhow::anyhow!("injected reverse precommit failure"));

        let error = complete_link_note_mutation(
            &state,
            &expected,
            Some(&forward_commit),
            reverse_failure,
            "created reverse note link",
        )
        .await
        .expect_err("partial bidirectional link must not look safely retryable");

        assert!(error.contains("partially committed"));
        assert!(error.contains("do not retry"));
        coordinator.require_namespace_ready().unwrap();
        assert!(state
            .knowledge_store
            .read()
            .await
            .get_note(&created.id)
            .unwrap()
            .content
            .contains("[[Target]]"));
    }
}

/// Apply discovered links to a note (creates bidirectional wikilinks)
#[tauri::command]
pub async fn apply_links(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] noteId: String,
    request: ApplyLinksRequest,
) -> Result<ApplyLinksResponse, String> {
    let mut root_guard = Some(crate::commands::acquire_root_epoch(state.inner()).await?);
    let mut root_epoch = crate::commands::capture_root_epoch(state.inner())?;
    let requested_candidates = if !request.candidates.is_empty() {
        deduplicate_links(request.candidates.clone())
    } else {
        drop(root_guard.take());
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
    let root_guard = match root_guard {
        Some(guard) => guard,
        None => crate::commands::acquire_expected_root_epoch(state.inner(), &root_epoch).await?,
    };
    let links_attempted = requested_candidates.len();

    let mut links_created = 0;
    let mut dirty_note_ids: HashSet<String> = HashSet::new();
    let mut latest_commit = None;
    let mut repair_pending = false;

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
                let mutation = store.update_note_expecting_authority(
                    &noteId,
                    NoteUpdate {
                        content: Some(new_content),
                        ..Default::default()
                    },
                    "note_editor",
                    root_epoch.clone(),
                );
                drop(store);
                let completed = complete_link_note_mutation(
                    state.inner(),
                    &root_epoch,
                    latest_commit.as_ref(),
                    mutation,
                    "applied forward note link",
                )
                .await?;
                root_epoch = completed.continuation_authority;
                if let Some(commit) = completed.commit {
                    latest_commit = Some(commit);
                    repair_pending = !completed.repaired;
                }
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
                let mutation = store.update_note_expecting_authority(
                    &candidate.target_id,
                    NoteUpdate {
                        content: Some(new_content),
                        ..Default::default()
                    },
                    "note_editor",
                    root_epoch.clone(),
                );
                drop(store);
                let completed = complete_link_note_mutation(
                    state.inner(),
                    &root_epoch,
                    latest_commit.as_ref(),
                    mutation,
                    "applied reverse note link",
                )
                .await?;
                root_epoch = completed.continuation_authority;
                if let Some(commit) = completed.commit {
                    latest_commit = Some(commit);
                    repair_pending = !completed.repaired;
                }
                target_updated = true;
            }
        }

        if source_updated {
            links_created += 1;
            dirty_note_ids.insert(noteId.clone());
        }
        if target_updated {
            dirty_note_ids.insert(candidate.target_id.clone());
        }
    }

    if !dirty_note_ids.is_empty() {
        if let Some(commit) = latest_commit.as_ref() {
            drop(root_guard);
            if repair_pending {
                crate::commands::acknowledge_reported_repair(
                    crate::commands::repair_after_authority_mutation(
                        state.inner(),
                        commit,
                        "applied note links",
                    )
                    .await,
                );
            }
        } else {
            root_guard.finish(state.inner()).await?;
        }
    } else {
        root_guard.finish(state.inner()).await?;
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
    let root_ticket = crate::commands::acquire_root_epoch(state.inner()).await?;
    let mut root_epoch = root_ticket.authority().clone();
    let link_type = linkType.unwrap_or_else(|| "related".to_string());
    let mut dirty_note_ids: HashSet<String> = HashSet::new();
    let mut latest_commit = None;
    let mut repair_pending = false;

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
            let mutation = store.update_note_expecting_authority(
                &sourceId,
                NoteUpdate {
                    content: Some(new_content),
                    ..Default::default()
                },
                "note_editor",
                root_epoch.clone(),
            );
            drop(store);
            let completed = complete_link_note_mutation(
                state.inner(),
                &root_epoch,
                latest_commit.as_ref(),
                mutation,
                "created forward note link",
            )
            .await?;
            root_epoch = completed.continuation_authority;
            if let Some(commit) = completed.commit {
                latest_commit = Some(commit);
                repair_pending = !completed.repaired;
            }
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
            let mutation = store.update_note_expecting_authority(
                &targetId,
                NoteUpdate {
                    content: Some(new_content),
                    ..Default::default()
                },
                "note_editor",
                root_epoch.clone(),
            );
            drop(store);
            let completed = complete_link_note_mutation(
                state.inner(),
                &root_epoch,
                latest_commit.as_ref(),
                mutation,
                "created reverse note link",
            )
            .await?;
            if let Some(commit) = completed.commit {
                latest_commit = Some(commit);
                repair_pending = !completed.repaired;
            }
            dirty_note_ids.insert(targetId.clone());
        }
    }

    if !dirty_note_ids.is_empty() {
        if let Some(commit) = latest_commit.as_ref() {
            drop(root_ticket);
            if repair_pending {
                crate::commands::acknowledge_reported_repair(
                    crate::commands::repair_after_authority_mutation(
                        state.inner(),
                        commit,
                        "created note link",
                    )
                    .await,
                );
            }
        } else {
            root_ticket.finish(state.inner()).await?;
        }
    } else {
        root_ticket.finish(state.inner()).await?;
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
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let service = state.link_discovery_service()?;
        let mut discovery = service.write().await;
        state
            .mutation_coordinator
            .as_ref()
            .ok_or_else(|| "mutation coordinator is unavailable".to_string())?
            .with_locked_derived_state(root_ticket.authority(), true, || {
                discovery.reload_from_disk_checked().map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                Ok(discovery.list_queue_entries(status.as_deref(), limit.unwrap_or(25)))
            })
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Dismiss a cached suggestion so it does not reappear until the note changes again.
#[tauri::command]
pub async fn dismiss_link_suggestion(
    state: State<'_, AppState>,
    #[allow(non_snake_case)] noteId: String,
    #[allow(non_snake_case)] targetId: String,
) -> Result<DismissLinkSuggestionResponse, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let response = {
        let service = state.link_discovery_service()?;
        let mut discovery = service.write().await;
        state
            .mutation_coordinator
            .as_ref()
            .ok_or_else(|| "mutation coordinator is unavailable".to_string())?
            .with_locked_derived_state(root_ticket.authority(), true, || {
                discovery.reload_from_disk_checked().map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                discovery
                    .dismiss_suggestion_checked(&noteId, &targetId)
                    .map_err(|error| {
                        crate::services::twin_events::MutationError::Invalid(error.to_string())
                    })
            })
            .map_err(|error| error.to_string())?
    };
    enqueue_vault_optimizer_note(state.inner(), &noteId, "link_dismissed").await?;
    root_ticket.finish(state.inner()).await?;
    Ok(response)
}

/// Get background discovery worker status and queue metrics.
#[tauri::command]
pub async fn get_link_discovery_status(
    state: State<'_, AppState>,
) -> Result<LinkDiscoveryStatus, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = {
        let settings = {
            let settings_service = state.settings_service.read().await;
            settings_service.get().clone()
        };
        let service = state.link_discovery_service()?;
        let mut discovery = service.write().await;
        state
            .mutation_coordinator
            .as_ref()
            .ok_or_else(|| "mutation coordinator is unavailable".to_string())?
            .with_locked_derived_state(root_ticket.authority(), true, || {
                discovery.reload_from_disk_checked().map_err(|error| {
                    crate::services::twin_events::MutationError::Invalid(error.to_string())
                })?;
                Ok(discovery.status(&settings))
            })
            .map_err(|error| error.to_string())?
    };
    root_ticket.finish(state.inner()).await?;
    Ok(result)
}
