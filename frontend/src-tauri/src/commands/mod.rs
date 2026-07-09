//! Canonical lock order: `knowledge_store` before `vault_optimizer`, always.
//!
//! The background vault-optimizer worker (`main.rs::start_vault_optimizer_worker`)
//! acquires `state.knowledge_store.write()` then `state.vault_optimizer.write()`.
//! Every other call site that needs both locks (e.g.
//! `commands::migration::rollback_vault_optimizer_change`) must acquire them in
//! the same order. Acquiring them in reverse order risks an ABBA deadlock: the
//! worker fires every 30s, so a caller that takes `vault_optimizer` first and
//! blocks on `knowledge_store` can cross with the worker holding
//! `knowledge_store` and blocking on `vault_optimizer`, wedging both locks
//! (and every command that touches either) until the app restarts.

pub mod boot;
pub mod canvas;
pub mod distill;
pub mod feedback;
pub mod graph;
pub mod import;
pub mod mcp;
pub mod memory;
pub mod migration;
pub mod notes;
pub mod priority;
pub mod retrieval;
pub mod search;
pub mod settings;
pub mod twin;
pub mod zettelkasten;

use crate::models::note::Note;
use crate::services::retrieval::RetrievalResult;
use crate::AppState;
use std::collections::HashSet;

/// Shared retrieval helper — acquires the 4 retrieval-pipeline read locks, calls
/// `retrieval.retrieve()`, and releases all locks before returning. Used by
/// `retrieve_relevant`, `recall_relevant`, and the canvas context resolvers to
/// avoid duplicating the same lock sequence in each caller.
pub(crate) async fn run_retrieval(
    state: &AppState,
    query: &str,
    limit: usize,
    context_ids: &[String],
) -> Result<Vec<RetrievalResult>, String> {
    let search = state.search_service.read().await;
    let graph = state.graph_index.read().await;
    let priority = state.priority_service.read().await;
    let retrieval = state.retrieval_service.read().await;
    retrieval.retrieve(&search, &graph, &priority, query, limit, context_ids)
}

pub(crate) async fn sync_chunk_index_for_note(state: &AppState, note: &Note) {
    sync_chunk_index_for_notes(state, std::slice::from_ref(note)).await;
}

pub(crate) async fn sync_chunk_index_for_notes(state: &AppState, notes: &[Note]) {
    if notes.is_empty() {
        return;
    }

    let mut chunks = state.chunk_index.write().await;
    for note in notes {
        if let Err(error) = chunks.index_note_chunks(note) {
            log::error!("Failed to index chunks for note '{}': {}", note.id, error);
        }
    }
    if let Err(error) = chunks.commit() {
        log::error!("Failed to commit chunk index: {}", error);
    }
}

pub(crate) async fn remove_note_chunks_from_index(state: &AppState, note_id: &str) {
    let mut chunks = state.chunk_index.write().await;
    if let Err(error) = chunks.remove_note_chunks(note_id) {
        log::error!("Failed to remove chunks for note '{}': {}", note_id, error);
    }
    if let Err(error) = chunks.commit() {
        log::error!("Failed to commit chunk index: {}", error);
    }
}

pub(crate) async fn rebuild_link_discovery(state: &AppState, notes: &[Note]) {
    let mut discovery = state.link_discovery.write().await;
    discovery.bootstrap(notes);
}

pub(crate) async fn bootstrap_vault_optimizer(state: &AppState, notes: &[Note]) {
    let mut optimizer = state.vault_optimizer.write().await;
    optimizer.bootstrap(notes);
}

pub(crate) async fn enqueue_vault_optimizer_note(state: &AppState, note_id: &str, reason: &str) {
    let mut optimizer = state.vault_optimizer.write().await;
    optimizer.enqueue_note(note_id, reason);
}

pub(crate) async fn remove_link_discovery_note(state: &AppState, note_id: &str) {
    let mut discovery = state.link_discovery.write().await;
    discovery.remove_note(note_id);
}

pub(crate) async fn sync_topic_hubs(state: &AppState) -> Result<Vec<Note>, String> {
    let sync_result = {
        let mut store = state.knowledge_store.write().await;
        crate::services::topic_hub::sync_topic_hubs(&mut store)
            .map_err(|error| error.to_string())?
    };

    let changed_ids = sync_result
        .changed_note_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let changed_notes = sync_result
        .all_notes
        .iter()
        .filter(|note| changed_ids.contains(&note.id))
        .cloned()
        .collect::<Vec<_>>();

    for removed_note_id in &sync_result.removed_note_ids {
        remove_note_chunks_from_index(state, removed_note_id).await;
        remove_link_discovery_note(state, removed_note_id).await;

        let mut search = state.search_service.write().await;
        if let Err(error) = search.remove_note(removed_note_id) {
            log::error!(
                "Failed to remove topic-hub note '{}' from search index: {}",
                removed_note_id,
                error
            );
        }
        if let Err(error) = search.commit() {
            log::error!(
                "Failed to commit search index after topic-hub removal: {}",
                error
            );
        }
    }

    if !changed_notes.is_empty() {
        {
            let mut search = state.search_service.write().await;
            for note in &changed_notes {
                if let Err(error) = search.index_note(note) {
                    log::error!("Failed to index topic-hub note '{}': {}", note.id, error);
                }
            }
            if let Err(error) = search.commit() {
                log::error!("Failed to commit search index after topic sync: {}", error);
            }
        }

        sync_chunk_index_for_notes(state, &changed_notes).await;
    }

    {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(&sync_result.all_notes);
    }

    rebuild_link_discovery(state, &sync_result.all_notes).await;
    bootstrap_vault_optimizer(state, &sync_result.all_notes).await;

    Ok(sync_result.all_notes)
}

pub(crate) async fn rebuild_all_indexes(state: &AppState) -> Result<Vec<Note>, String> {
    let notes = sync_topic_hubs(state).await?;

    {
        let mut search = state.search_service.write().await;
        search
            .reindex_all(&notes)
            .map_err(|error| error.to_string())?;
    }

    {
        let mut graph = state.graph_index.write().await;
        graph.build_from_notes(&notes);
    }

    {
        let mut chunks = state.chunk_index.write().await;
        if let Err(error) = chunks.reindex_all(&notes) {
            log::error!("Failed to rebuild chunk index: {}", error);
        }
    }

    bootstrap_vault_optimizer(state, &notes).await;
    Ok(notes)
}

/// Single chokepoint for "a note was just created or edited on disk and needs to
/// become visible everywhere else": the search index, the chunk index, topic hubs
/// (which also rebuilds the graph + link-discovery bootstrap), and the vault
/// optimizer queue.
///
/// Before this helper existed, five call sites (`notes::create_note`,
/// `notes::update_note`, `zettelkasten::apply_links`, `zettelkasten::create_link`,
/// plus ad-hoc copies in `canvas::export_to_note`, `import.rs`, `distill.rs`)
/// each hand-rolled a subset of this sequence. `notes.rs` and `zettelkasten.rs`
/// called only `sync_topic_hubs` + `enqueue_vault_optimizer_note` — but
/// `sync_topic_hubs` only reindexes notes whose *hub* metadata changed, so a
/// plain content edit was never reindexed into search or chunks until the next
/// full rebuild (app restart). This was the reindex regression. The other three
/// sites got search+chunk indexing right but never synced topic hubs or
/// enqueued the vault optimizer.
///
/// `reason` is forwarded to `enqueue_vault_optimizer_note` for its audit trail
/// (e.g. `"note_created"`, `"links_applied"`).
///
/// Returns the full up-to-date note list from `sync_topic_hubs` (the same
/// value `sync_topic_hubs` itself returns) so callers that need it — e.g.
/// `distill::distill_note`, which builds hub-update summaries from it — don't
/// have to call `sync_topic_hubs` a second time. Callers that don't need it
/// simply discard the return value.
///
/// Error handling: search/chunk indexing failures are logged and do not abort
/// the call, matching what all the previously-correct call sites already did —
/// the note is already durably written to disk by the time this runs, so
/// losing that fact over an index hiccup would be worse than a temporarily
/// stale index (which self-heals on the next `rebuild_all_indexes`).
/// `sync_topic_hubs` failures do propagate, matching the pre-existing behavior
/// of `notes::create_note` / `notes::update_note` / the zettelkasten commands.
pub(crate) async fn commit_note_write(
    state: &AppState,
    note_id: &str,
    reason: &str,
) -> Result<Vec<Note>, String> {
    commit_note_writes(state, std::slice::from_ref(&note_id.to_string()), reason).await
}

/// Batch form of [`commit_note_write`] — indexes every note individually but
/// runs the (expensive, full-vault) `sync_topic_hubs` pass only once. Used by
/// call sites that write several notes in one command (conversation/document
/// import, zettelkasten's bidirectional link updates, distillation) so the
/// cost doesn't scale with the number of notes touched.
///
/// `sync_topic_hubs` always runs, even if `note_ids` is empty — callers such
/// as `distill_note` rely on getting the full post-sync note list back
/// regardless of whether this particular call touched any notes.
pub(crate) async fn commit_note_writes(
    state: &AppState,
    note_ids: &[String],
    reason: &str,
) -> Result<Vec<Note>, String> {
    let mut notes = Vec::with_capacity(note_ids.len());
    {
        let store = state.knowledge_store.read().await;
        for note_id in note_ids {
            match store.get_note(note_id) {
                Ok(note) => notes.push(note),
                Err(error) => {
                    log::error!(
                        "commit_note_write: note '{}' could not be read for indexing: {}",
                        note_id,
                        error
                    );
                }
            }
        }
    }

    if !notes.is_empty() {
        let mut search = state.search_service.write().await;
        for note in &notes {
            if let Err(error) = search.index_note(note) {
                log::error!("Failed to index note '{}': {}", note.id, error);
            }
        }
        if let Err(error) = search.commit() {
            log::error!(
                "Failed to commit search index after note write(s): {}",
                error
            );
        }
    }

    sync_chunk_index_for_notes(state, &notes).await;
    let all_notes = sync_topic_hubs(state).await?;

    for note_id in note_ids {
        enqueue_vault_optimizer_note(state, note_id, reason).await;
    }

    Ok(all_notes)
}

#[cfg(test)]
mod commit_note_write_tests {
    use super::*;
    use crate::models::boot::BootStatus;
    use crate::models::note::{NoteCreate, NoteStatus, NoteUpdate};
    use crate::services::canvas_store::CanvasStore;
    use crate::services::chunk_index::ChunkIndex;
    use crate::services::feedback::FeedbackService;
    use crate::services::graph_index::GraphIndex;
    use crate::services::knowledge_store::KnowledgeStore;
    use crate::services::link_discovery::LinkDiscoveryService;
    use crate::services::markdown_migration::MarkdownMigrationService;
    use crate::services::memory::MemoryService;
    use crate::services::ollama::OllamaService;
    use crate::services::openrouter::OpenRouterService;
    use crate::services::priority::PriorityScoringService;
    use crate::services::retrieval::RetrievalService;
    use crate::services::search::SearchService;
    use crate::services::settings::SettingsService;
    use crate::services::twin_store::TwinStore;
    use crate::services::vault_optimizer::VaultOptimizerService;
    use crate::AppState;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    /// Builds a fully-wired `AppState` over fresh tempdirs, mirroring
    /// `main.rs`'s setup block. Kept local to this test module: no other test
    /// in the crate currently needs a whole `AppState`, and command-level
    /// tests can't cheaply construct `tauri::State` outside a running app, so
    /// this exercises `commit_note_write` directly against real services.
    fn build_test_state() -> (AppState, TempDir, TempDir) {
        let vault_dir = TempDir::new().expect("vault tempdir should be created");
        let data_dir = TempDir::new().expect("data tempdir should be created");
        let vault_path = vault_dir.path().to_path_buf();
        let data_path = data_dir.path().to_path_buf();

        let knowledge_store = KnowledgeStore::new(vault_path, data_path.clone());
        let search_service =
            SearchService::new(data_path.clone()).expect("search service should initialize");
        let chunk_index =
            ChunkIndex::new(data_path.clone()).expect("chunk index should initialize");

        let state = AppState {
            knowledge_store: Arc::new(RwLock::new(knowledge_store)),
            graph_index: Arc::new(RwLock::new(GraphIndex::new())),
            search_service: Arc::new(RwLock::new(search_service)),
            canvas_store: Arc::new(RwLock::new(CanvasStore::new(data_path.join("canvas")))),
            openrouter: Arc::new(RwLock::new(OpenRouterService::new(String::new()))),
            ollama: Arc::new(RwLock::new(OllamaService::new(String::new()))),
            feedback_service: Arc::new(RwLock::new(FeedbackService::new(
                data_path.join("feedback"),
            ))),
            settings_service: Arc::new(RwLock::new(SettingsService::load_defaults())),
            priority_service: Arc::new(RwLock::new(PriorityScoringService::new(data_path.clone()))),
            retrieval_service: Arc::new(RwLock::new(RetrievalService::new(data_path.clone()))),
            chunk_index: Arc::new(RwLock::new(chunk_index)),
            link_discovery: Arc::new(RwLock::new(LinkDiscoveryService::new(data_path.clone()))),
            markdown_migration: Arc::new(RwLock::new(MarkdownMigrationService::new(
                data_path.clone(),
            ))),
            vault_optimizer: Arc::new(RwLock::new(VaultOptimizerService::new(data_path.clone()))),
            twin_store: Arc::new(RwLock::new(TwinStore::new(data_path.join("twin")))),
            memory_service: Arc::new(MemoryService::new()),
            boot_state: Arc::new(RwLock::new(BootStatus::default())),
        };

        (state, vault_dir, data_dir)
    }

    #[tokio::test]
    async fn create_note_is_search_indexed_after_commit_note_write() {
        let (state, _vault_dir, _data_dir) = build_test_state();

        let created = {
            let mut store = state.knowledge_store.write().await;
            store
                .create_note(NoteCreate {
                    title: "Quokka Habits".to_string(),
                    content: "The quokka forages for xylophonemarker9142 at dawn.".to_string(),
                    relative_path: None,
                    aliases: Vec::new(),
                    status: NoteStatus::Draft,
                    tags: Vec::new(),
                    schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                    migration_source: None,
                    optimizer_managed: false,
                    properties: Default::default(),
                })
                .expect("note should be created")
        };

        commit_note_write(&state, &created.id, "test_note_created")
            .await
            .expect("commit_note_write should succeed");

        let search = state.search_service.read().await;
        let results = search
            .search("xylophonemarker9142", 10)
            .expect("search should not error");
        assert!(
            results.iter().any(|r| r.note.id == created.id),
            "expected newly created note to be search-indexed immediately, found: {:?}",
            results.iter().map(|r| &r.note.id).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn update_note_content_is_search_indexed_after_commit_note_write() {
        let (state, _vault_dir, _data_dir) = build_test_state();

        let created = {
            let mut store = state.knowledge_store.write().await;
            store
                .create_note(NoteCreate {
                    title: "Wombat Notes".to_string(),
                    content: "Original content with no special markers.".to_string(),
                    relative_path: None,
                    aliases: Vec::new(),
                    status: NoteStatus::Draft,
                    tags: Vec::new(),
                    schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                    migration_source: None,
                    optimizer_managed: false,
                    properties: Default::default(),
                })
                .expect("note should be created")
        };
        commit_note_write(&state, &created.id, "test_note_created")
            .await
            .expect("initial commit_note_write should succeed");

        {
            let mut store = state.knowledge_store.write().await;
            store
                .update_note(
                    &created.id,
                    NoteUpdate {
                        content: Some(
                            "Updated content mentions zebrawhistle6784 explicitly.".to_string(),
                        ),
                        ..Default::default()
                    },
                )
                .expect("note should be updated");
        }

        commit_note_write(&state, &created.id, "test_note_updated")
            .await
            .expect("commit_note_write should succeed after update");

        let search = state.search_service.read().await;
        let results = search
            .search("zebrawhistle6784", 10)
            .expect("search should not error");
        assert!(
            results.iter().any(|r| r.note.id == created.id),
            "expected updated note content to be search-indexed immediately, found: {:?}",
            results.iter().map(|r| &r.note.id).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn create_note_is_chunk_indexed_after_commit_note_write() {
        let (state, _vault_dir, _data_dir) = build_test_state();

        let created = {
            let mut store = state.knowledge_store.write().await;
            store
                .create_note(NoteCreate {
                    title: "Narwhal Facts".to_string(),
                    content: "Narwhals communicate using kittywomble4471 clicks and whistles."
                        .to_string(),
                    relative_path: None,
                    aliases: Vec::new(),
                    status: NoteStatus::Draft,
                    tags: Vec::new(),
                    schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                    migration_source: None,
                    optimizer_managed: false,
                    properties: Default::default(),
                })
                .expect("note should be created")
        };

        commit_note_write(&state, &created.id, "test_note_created")
            .await
            .expect("commit_note_write should succeed");

        let chunks = state.chunk_index.read().await;
        let results = chunks
            .search_chunks("kittywomble4471", 10)
            .expect("chunk search should not error");
        assert!(
            results.iter().any(|r| r.parent_note_id == created.id),
            "expected newly created note's content to be chunk-indexed immediately, found: {:?}",
            results
                .iter()
                .map(|r| &r.parent_note_id)
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn commit_note_writes_batches_a_single_hub_sync_across_multiple_notes() {
        let (state, _vault_dir, _data_dir) = build_test_state();

        let mut ids = Vec::new();
        {
            let mut store = state.knowledge_store.write().await;
            for i in 0..3 {
                let note = store
                    .create_note(NoteCreate {
                        title: format!("Batch Note {}", i),
                        content: format!("Batch content marker batchmarker{}77 here.", i),
                        relative_path: None,
                        aliases: Vec::new(),
                        status: NoteStatus::Draft,
                        tags: Vec::new(),
                        schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                        migration_source: None,
                        optimizer_managed: false,
                        properties: Default::default(),
                    })
                    .expect("note should be created");
                ids.push(note.id);
            }
        }

        commit_note_writes(&state, &ids, "test_batch_created")
            .await
            .expect("commit_note_writes should succeed");

        let search = state.search_service.read().await;
        for (i, id) in ids.iter().enumerate() {
            let results = search
                .search(&format!("batchmarker{}77", i), 10)
                .expect("search should not error");
            assert!(
                results.iter().any(|r| &r.note.id == id),
                "expected batch note {} to be search-indexed",
                i
            );
        }
    }
}
