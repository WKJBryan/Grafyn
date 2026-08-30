use crate::models::note::{GraphNeighbor, NoteMeta};
use crate::services::graph_index::{FullGraph, GraphStats};
use crate::AppState;
use tauri::State;

/// Get all notes that link to the given note (backlinks)
#[tauri::command]
pub async fn get_backlinks(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<NoteMeta>, String> {
    let ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = state.graph_index.read().await.get_backlinks(&note_id);
    ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Get all notes that the given note links to (outgoing links)
#[tauri::command]
pub async fn get_outgoing(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<NoteMeta>, String> {
    let ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = state.graph_index.read().await.get_outgoing(&note_id);
    ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Get all neighbors (both backlinks and outgoing) for graph visualization
#[tauri::command]
pub async fn get_neighbors(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<GraphNeighbor>, String> {
    let ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = state.graph_index.read().await.get_neighbors(&note_id);
    ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Get notes with no incoming or outgoing links
#[tauri::command]
pub async fn get_unlinked(state: State<'_, AppState>) -> Result<Vec<NoteMeta>, String> {
    let ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = state.graph_index.read().await.get_unlinked();
    ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Get the full graph structure (nodes + links) for visualization
#[tauri::command]
pub async fn get_full_graph(state: State<'_, AppState>) -> Result<FullGraph, String> {
    let ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let result = state.graph_index.read().await.get_full_graph();
    ticket.finish(state.inner()).await?;
    Ok(result)
}

/// Rebuild the graph index from all notes
#[tauri::command]
pub async fn rebuild_graph(state: State<'_, AppState>) -> Result<GraphStats, String> {
    rebuild_graph_inner(state.inner()).await
}

async fn rebuild_graph_inner(state: &AppState) -> Result<GraphStats, String> {
    let ticket = crate::commands::acquire_root_epoch(state).await?;
    let expected = ticket.authority().clone();
    let sync = crate::commands::sync_topic_hubs_at_authority(state, expected).await?;
    let ticket = if sync.latest_commit.is_some() {
        drop(ticket);
        crate::commands::acquire_expected_root_epoch(state, &sync.continuation_authority).await?
    } else {
        ticket
    };

    // Rebuild graph
    let stats = {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(&sync.notes);
        graph.stats()
    };
    ticket.finish(state).await?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{NoteCreate, NoteStatus};

    #[tokio::test]
    async fn rebuild_graph_repairs_post_authority_topic_commit_before_finishing() {
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
        let (_, create_commit) = state
            .knowledge_store
            .write()
            .await
            .create_note_expecting_authority(
                NoteCreate {
                    title: "Graph rebuild topic source".into(),
                    content: "graph-rebuild-marker".into(),
                    relative_path: Some("graph-rebuild-topic-source.md".into()),
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
                "graph test setup"
            )
            .await,
            crate::commands::PostAuthorityRepair::Ready(_)
        ));
        coordinator.fail_next_replays_before_targets(2);

        let stats = rebuild_graph_inner(&state)
            .await
            .expect("graph rebuild must recover the exact topic commit");

        assert!(
            stats.total_notes >= 2,
            "source and topic hub must be indexed"
        );
        assert_eq!(coordinator.pending_count().unwrap(), 0);
        coordinator.require_namespace_ready().unwrap();
        let current = coordinator.current_authority_token().unwrap();
        assert_eq!(state.loaded_authority.read().await.as_ref(), Some(&current));
    }
}
