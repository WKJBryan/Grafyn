use crate::commands::rebuild_all_indexes;
use crate::models::note::SearchResult;
use crate::AppState;
use tauri::State;

/// Search notes by query string, with priority scoring applied
#[tauri::command]
pub async fn search_notes(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let limit = limit.unwrap_or(20);
    let mut results = {
        let search = state.search_service.read().await;
        search.search(&query, limit).map_err(|e| e.to_string())?
    };

    // Apply priority scoring (recency, status, tag boosts)
    let priority = state.priority_service.read().await;
    priority.score_results(&mut results);
    drop(priority);
    root_ticket.finish(state.inner()).await?;

    Ok(results)
}

/// Find notes similar to a given note (enhanced with graph-aware retrieval)
#[tauri::command]
pub async fn find_similar(
    note_id: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let limit = limit.unwrap_or(10);

    // Get the note content for keyword extraction
    let content = {
        let store = state.knowledge_store.read().await;
        store
            .get_note(&note_id)
            .map(|n| n.content)
            .map_err(|e| e.to_string())?
    };

    // Extract keywords from content
    let query_words: Vec<&str> = content
        .split_whitespace()
        .filter(|w| w.len() > 4)
        .take(20)
        .collect();

    if query_words.is_empty() {
        root_ticket.finish(state.inner()).await?;
        return Ok(Vec::new());
    }

    let query_str = query_words.join(" ");

    // Use retrieval pipeline with the source note as context
    let results = {
        let search = state.search_service.read().await;
        let graph = state.graph_index.read().await;
        let priority = state.priority_service.read().await;
        let retrieval = state.retrieval_service.read().await;

        retrieval.retrieve(
            &search,
            &graph,
            &priority,
            &query_str,
            limit + 1,
            &[note_id.clone()],
        )?
    };

    // Convert RetrievalResult → SearchResult, filtering out the source note
    let search_results: Vec<SearchResult> = results
        .into_iter()
        .filter(|r| r.note.id != note_id)
        .take(limit)
        .map(|r| SearchResult {
            note: r.note,
            score: r.score,
            snippet: if r.snippet.is_empty() {
                None
            } else {
                Some(r.snippet)
            },
        })
        .collect();
    root_ticket.finish(state.inner()).await?;

    Ok(search_results)
}

/// Reindex all notes
#[tauri::command]
pub async fn reindex(state: State<'_, AppState>) -> Result<(), String> {
    reindex_inner(state.inner()).await
}

async fn reindex_inner(state: &AppState) -> Result<(), String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state).await?;
    let expected = root_ticket.authority().clone();
    let sync = rebuild_all_indexes(state, expected).await?;
    let root_ticket = if sync.latest_commit.is_some() {
        drop(root_ticket);
        crate::commands::acquire_expected_root_epoch(state, &sync.continuation_authority).await?
    } else {
        root_ticket
    };
    root_ticket.finish(state).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{NoteCreate, NoteStatus};

    #[tokio::test]
    async fn reindex_repairs_post_authority_topic_commit_before_finishing() {
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
                    title: "Search rebuild topic source".into(),
                    content: "search-rebuild-marker-4821".into(),
                    relative_path: Some("search-rebuild-topic-source.md".into()),
                    aliases: Vec::new(),
                    status: NoteStatus::Draft,
                    tags: vec!["rust".into()],
                    schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                    migration_source: None,
                    optimizer_managed: false,
                    properties: Default::default(),
                },
                "test",
                coordinator.current_authority_token().unwrap(),
            )
            .unwrap();
        let expected = create_commit.authority_token.as_ref().unwrap();
        assert!(matches!(
            crate::commands::repair_after_migration_authority_token(
                &state,
                expected,
                "search test setup"
            )
            .await,
            crate::commands::PostAuthorityRepair::Ready(_)
        ));
        coordinator.fail_next_replays_before_targets(2);

        reindex_inner(&state)
            .await
            .expect("search rebuild must recover the exact topic commit");

        assert!(state
            .search_service
            .read()
            .await
            .search("search-rebuild-marker-4821", 5)
            .unwrap()
            .iter()
            .any(|result| result.note.id == created.id));
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        coordinator.require_namespace_ready().unwrap();
        let current = coordinator.current_authority_token().unwrap();
        assert_eq!(state.loaded_authority.read().await.as_ref(), Some(&current));
    }
}
