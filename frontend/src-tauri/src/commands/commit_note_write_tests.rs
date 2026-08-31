use super::*;
use crate::models::boot::BootStatus;
use crate::models::note::{NoteCreate, NoteStatus, NoteUpdate};
use crate::models::settings::UserSettings;
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
use crate::services::topic_hub::normalize_topic_key;
use crate::services::twin::TwinStore;
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
pub(crate) fn build_test_state() -> (AppState, TempDir, TempDir) {
    let vault_dir = TempDir::new().expect("vault tempdir should be created");
    let data_dir = TempDir::new().expect("data tempdir should be created");
    let vault_path = vault_dir.path().to_path_buf();
    let data_path = data_dir.path().to_path_buf();

    std::fs::create_dir_all(data_path.join("canvas")).expect("canvas directory should initialize");
    let twin_event_store = Arc::new(crate::services::twin_events::TwinEventStore::new(
        data_path.clone(),
    ));
    twin_event_store
        .initialize()
        .expect("event store should initialize");
    let mutation_coordinator = Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data_path,
            &vault_path,
            twin_event_store.clone(),
            Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .expect("mutation coordinator should initialize"),
    );
    let namespace = mutation_coordinator
        .current_namespace_path()
        .expect("vault namespace should initialize");
    let knowledge_store = KnowledgeStore::new(vault_path, namespace.clone());
    let search_service =
        SearchService::new(namespace.clone()).expect("search service should initialize");
    let chunk_index = ChunkIndex::new(namespace.clone()).expect("chunk index should initialize");

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
        link_discovery: Arc::new(RwLock::new(LinkDiscoveryService::new(namespace.clone()))),
        markdown_migration: Arc::new(RwLock::new(MarkdownMigrationService::new(
            namespace.clone(),
        ))),
        vault_optimizer: Arc::new(RwLock::new(VaultOptimizerService::new(namespace))),
        twin_store: Arc::new(RwLock::new(TwinStore::new(data_path.join("twin")))),
        twin_event_store,
        mutation_coordinator: Some(mutation_coordinator),
        sync_engine: None,
        mutation_startup_error: Arc::new(RwLock::new(None)),
        loaded_authority: Arc::new(RwLock::new(None)),
        authority_repair: Arc::new(tokio::sync::Mutex::new(())),
        committed_warning_app: None,
        vault_transition: Arc::new(tokio::sync::RwLock::new(())),
        memory_service: Arc::new(MemoryService::new()),
        boot_state: Arc::new(RwLock::new(BootStatus::default())),
        _recovery_runtime: None,
    };

    (state, vault_dir, data_dir)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authority_rebuild_retains_process_lock_through_derived_publication() {
    let (state, _vault_dir, _data_dir) = build_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
    let expected = coordinator.current_authority_token().unwrap();
    *state.loaded_authority.write().await = Some(expected.clone());

    // Pause only after the rebuild owns every local service guard and the
    // coordinator process guard. A peer process-lock acquisition must stay
    // blocked until the retained window reaches ready publication.
    let entered = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    let hook_entered = entered.clone();
    let hook_release = release.clone();
    let repair_state = state.clone();
    let rebuild = tokio::spawn(async move {
        rebuild_and_publish_current_authority_with_checkpoint(
            &repair_state,
            &expected,
            move |step| {
                if step == AuthorityRepairStep::Captured {
                    hook_entered.wait();
                    hook_release.wait();
                }
                Ok(())
            },
        )
        .await
    });
    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .unwrap();

    let mut peer = tokio::task::spawn_blocking(move || coordinator.current_authority_token());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut peer)
            .await
            .is_err(),
        "a peer acquired the mutation process lock while derived publication was incomplete"
    );

    tokio::task::spawn_blocking(move || release.wait())
        .await
        .unwrap();
    rebuild
        .await
        .expect("rebuild task should not panic")
        .expect("rebuild should complete after the graph lock is released");
    peer.await
        .expect("peer task should not panic")
        .expect("peer should acquire after rebuild publication");
}

#[tokio::test]
async fn same_generation_rebuild_failure_leaves_durable_namespace_unready() {
    let (state, _vault_dir, _data_dir) = build_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
    coordinator.require_namespace_ready().unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    *state.loaded_authority.write().await = Some(expected.clone());

    let result = rebuild_and_publish_current_authority_with_checkpoint(&state, &expected, |step| {
        if step == AuthorityRepairStep::Search {
            return Err("injected mid-rebuild failure".into());
        }
        Ok(())
    })
    .await;

    assert!(result.is_err());
    assert!(coordinator.require_namespace_ready().is_err());
    assert!(state.loaded_authority.read().await.is_none());
    assert_eq!(coordinator.current_authority_token().unwrap(), expected);
}

#[tokio::test]
async fn normalization_prelude_failure_leaves_durable_namespace_unready() {
    let (state, _vault_dir, _data_dir) = build_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
    coordinator.require_namespace_ready().unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    *state.loaded_authority.write().await = Some(expected.clone());

    let result = rebuild_and_publish_current_authority_with_checkpoint(&state, &expected, |step| {
        if step == AuthorityRepairStep::Normalized {
            return Err("injected post-normalization failure".into());
        }
        Ok(())
    })
    .await;

    assert!(result.is_err());
    assert!(coordinator.require_namespace_ready().is_err());
    assert!(state.loaded_authority.read().await.is_none());
    assert_eq!(coordinator.current_authority_token().unwrap(), expected);
}

#[tokio::test]
async fn remote_sync_repair_rebuilds_and_publishes_without_hub_or_optimizer_echo() {
    let (state, vault_dir, _data_dir) = build_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
    *state.knowledge_store.write().await = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        coordinator.current_namespace_path().unwrap(),
        coordinator.clone(),
    );
    let (_, commit) = state
        .knowledge_store
        .write()
        .await
        .create_note_expecting_authority(
            NoteCreate {
                title: "Remote repair source".into(),
                content: "A remotely materialized note about Rust".into(),
                relative_path: Some("remote-repair-source.md".into()),
                aliases: Vec::new(),
                status: NoteStatus::Draft,
                tags: vec!["rust".into()],
                schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: None,
                optimizer_managed: false,
                properties: Default::default(),
            },
            "remote_sync_test",
            coordinator.current_authority_token().unwrap(),
        )
        .unwrap();
    let expected = commit.authority_token.unwrap();
    assert_eq!(
        state
            .vault_optimizer
            .read()
            .await
            .status(&UserSettings::default())
            .queue_size,
        0
    );

    let repaired = rebuild_and_publish_remote_authority(&state, &expected)
        .await
        .unwrap();

    let notes = state
        .knowledge_store
        .read()
        .await
        .list_full_notes()
        .unwrap();
    assert_eq!(notes.len(), 1);
    assert!(!notes[0].is_topic_hub());
    assert_eq!(
        state
            .vault_optimizer
            .read()
            .await
            .status(&UserSettings::default())
            .queue_size,
        0
    );
    coordinator.require_namespace_ready().unwrap();
    assert_eq!(
        state.loaded_authority.read().await.as_ref(),
        Some(&repaired)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topic_hub_partial_commit_is_repaired_before_do_not_retry_error() {
    let (state, vault_dir, _data_dir) = build_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
    *state.knowledge_store.write().await = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        coordinator.current_namespace_path().unwrap(),
        coordinator.clone(),
    );
    let (_, create_commit) = state
        .knowledge_store
        .write()
        .await
        .create_note_expecting_authority(
            NoteCreate {
                title: "Command partial topic source".into(),
                content: "Rust topic source".into(),
                relative_path: Some("command-partial-topic-source.md".into()),
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
    let expected = create_commit.authority_token.unwrap();
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    coordinator.pause_after_authority_advance_once(entered.clone(), resume.clone());
    let task_state = state.clone();
    let task =
        tokio::spawn(async move { sync_topic_hubs_at_authority(&task_state, expected).await });

    tokio::task::spawn_blocking(move || entered.wait())
        .await
        .unwrap();
    coordinator
        .fail_once_at(crate::services::twin_events::MutationFaultPoint::BeforePreAuthorityMarker);
    tokio::task::spawn_blocking(move || resume.wait())
        .await
        .unwrap();

    let error = task
        .await
        .expect("topic normalization task should not panic")
        .expect_err("the partial normalization must not look wholly successful");
    assert!(error.contains("partially committed"));
    assert!(error.contains("do not retry"));
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    coordinator.require_namespace_ready().unwrap();
    let current = coordinator.current_authority_token().unwrap();
    assert_eq!(state.loaded_authority.read().await.as_ref(), Some(&current));
    assert!(state
        .knowledge_store
        .read()
        .await
        .list_full_notes()
        .unwrap()
        .iter()
        .any(|note| note.is_topic_hub()));
}

#[tokio::test]
async fn authority_rebuild_recovers_wal_only_after_fail_closed_invalidation() {
    let (state, _vault_dir, _data_dir) = build_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
    let expected = coordinator.current_authority_token().unwrap();
    *state.loaded_authority.write().await = Some(expected.clone());
    coordinator
        .fail_once_at(crate::services::twin_events::MutationFaultPoint::BeforePendingRecovery);

    let result = rebuild_and_publish_current_authority(&state, &expected).await;

    assert!(
        result.is_err(),
        "rebuild must execute canonical WAL recovery"
    );
    assert!(coordinator.require_namespace_ready().is_err());
    assert!(state.loaded_authority.read().await.is_none());
}

#[tokio::test]
async fn live_rebuild_replays_optimizer_wal_before_witness_recovery() {
    let (state, vault_dir, _data_dir) = build_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
    let namespace = coordinator.current_namespace_path().unwrap();
    *state.knowledge_store.write().await = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        namespace.clone(),
        coordinator.clone(),
    );
    let created = {
        let mut store = state.knowledge_store.write().await;
        store
            .create_note(NoteCreate {
                title: "Prepared WAL Recovery".to_string(),
                content: "A staged optimizer mutation must keep its audit owner.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: NoteStatus::Draft,
                tags: Vec::new(),
                schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: None,
                optimizer_managed: false,
                properties: Default::default(),
            })
            .unwrap()
    };
    commit_note_write(&state, &created.id, "test_note_created")
        .await
        .unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    let settings = UserSettings::default();
    let pending = {
        let store = state.knowledge_store.read().await;
        let mut optimizer = state.vault_optimizer.write().await;
        match optimizer
            .prepare_next_expecting_authority(&store, &settings, expected)
            .unwrap()
        {
            crate::services::vault_optimizer::OptimizerTick::Pending(pending) => *pending,
            other => panic!("expected a pending optimizer write, got {other:?}"),
        }
    };
    let original_queue: serde_json::Value = serde_json::from_slice(
        &std::fs::read(namespace.join("vault_migration/optimizer/queue.json")).unwrap(),
    )
    .unwrap();
    let original_job_id = original_queue["queue"][0]["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    coordinator.fail_next_replays_before_targets(2);
    let apply = {
        let mut store = state.knowledge_store.write().await;
        let mut optimizer = state.vault_optimizer.write().await;
        optimizer.apply_pending(&mut store, pending)
    };
    assert!(apply.is_err());
    assert_eq!(coordinator.pending_count().unwrap(), 1);
    assert!(!state
        .knowledge_store
        .read()
        .await
        .overlay_path(&created.id)
        .exists());

    let repair_token = coordinator.current_authority_token().unwrap();
    rebuild_and_publish_current_authority(&state, &repair_token)
        .await
        .unwrap();

    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert!(state
        .knowledge_store
        .read()
        .await
        .overlay_path(&created.id)
        .exists());
    let optimizer = state.vault_optimizer.read().await;
    let status = optimizer.status(&settings);
    assert_eq!(status.accepted_count, 1);
    assert_eq!(optimizer.list_decisions(10).unwrap().len(), 1);
    assert_eq!(optimizer.inbox(None, 10).unwrap().len(), 1);
    drop(optimizer);
    let queue: serde_json::Value = serde_json::from_slice(
        &std::fs::read(namespace.join("vault_migration/optimizer/queue.json")).unwrap(),
    )
    .unwrap();
    assert!(queue["queue"]
        .as_array()
        .unwrap()
        .iter()
        .all(|job| { job["job_id"].as_str() != Some(original_job_id.as_str()) }));
    let pending_dir = namespace.join("vault_migration/optimizer/pending-publications-v1");
    assert_eq!(std::fs::read_dir(pending_dir).unwrap().count(), 0);
    coordinator.require_namespace_ready().unwrap();
}

#[tokio::test]
async fn post_cas_guard_abort_retires_optimizer_owner_before_ready_repair() {
    let (state, vault_dir, data_dir) = build_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
    let namespace = coordinator.current_namespace_path().unwrap();
    *state.knowledge_store.write().await = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        namespace.clone(),
        coordinator.clone(),
    );
    let created = {
        let mut store = state.knowledge_store.write().await;
        store
            .create_note(NoteCreate {
                title: "Post CAS Guard Abort".to_string(),
                content: "The source guard changes after authority advances.".to_string(),
                relative_path: None,
                aliases: Vec::new(),
                status: NoteStatus::Draft,
                tags: Vec::new(),
                schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: None,
                optimizer_managed: false,
                properties: Default::default(),
            })
            .unwrap()
    };
    commit_note_write(&state, &created.id, "test_note_created")
        .await
        .unwrap();
    let before = coordinator.current_authority_token().unwrap();
    coordinator.require_namespace_ready().unwrap();
    let settings = UserSettings::default();
    let pending = {
        let store = state.knowledge_store.read().await;
        let mut optimizer = state.vault_optimizer.write().await;
        match optimizer
            .prepare_next_expecting_authority(&store, &settings, before.clone())
            .unwrap()
        {
            crate::services::vault_optimizer::OptimizerTick::Pending(pending) => *pending,
            other => panic!("expected a pending optimizer write, got {other:?}"),
        }
    };
    let original_job_id = serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(namespace.join("vault_migration/optimizer/queue.json")).unwrap(),
    )
    .unwrap()["queue"][0]["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    coordinator.pause_after_authority_advance_once(entered.clone(), resume.clone());
    let knowledge = state.knowledge_store.clone();
    let optimizer = state.vault_optimizer.clone();
    let owner = std::thread::spawn(move || {
        let mut store = knowledge.blocking_write();
        let mut optimizer = optimizer.blocking_write();
        optimizer.apply_pending(&mut store, pending)
    });

    entered.wait();
    let pending_dir = namespace.join("vault_migration/optimizer/pending-publications-v1");
    let witness: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::fs::read_dir(&pending_dir)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(witness["phase"], "prepared");
    let markdown_path = vault_dir.path().join(&created.relative_path);
    let mut edited = std::fs::read(&markdown_path).unwrap();
    edited.extend_from_slice(b"\nExternal edit after authority CAS.\n");
    std::fs::write(&markdown_path, &edited).unwrap();
    resume.wait();

    assert!(owner.join().unwrap().is_err());
    let advanced = coordinator.current_authority_token().unwrap();
    assert_eq!(
        advanced.authority_generation,
        before.authority_generation + 1
    );
    assert!(coordinator.require_namespace_ready().is_err());
    // The optimizer owns this retained post-authority abort proof until
    // the exact repair seam resolves its durable witness.
    assert_eq!(coordinator.pending_count().unwrap(), 1);
    assert_eq!(std::fs::read_dir(&pending_dir).unwrap().count(), 1);
    assert!(!state
        .knowledge_store
        .read()
        .await
        .overlay_path(&created.id)
        .exists());
    assert_eq!(std::fs::read(&markdown_path).unwrap(), edited);
    {
        let optimizer = state.vault_optimizer.read().await;
        assert!(optimizer.list_decisions(10).unwrap().is_empty());
        assert!(optimizer.inbox(None, 10).unwrap().is_empty());
        let queue: serde_json::Value = serde_json::from_slice(
            &std::fs::read(namespace.join("vault_migration/optimizer/queue.json")).unwrap(),
        )
        .unwrap();
        let job = queue["queue"]
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["job_id"].as_str() == Some(original_job_id.as_str()))
            .unwrap();
        assert_eq!(job["attempts"], 0);
    }
    assert_eq!(
        std::fs::read_dir(data_dir.path().join("twin/mutations/receipts/v1"))
            .unwrap()
            .count(),
        0
    );

    rebuild_and_publish_current_authority(&state, &advanced)
        .await
        .unwrap();
    coordinator.require_namespace_ready().unwrap();
    assert_eq!(
        state.loaded_authority.read().await.as_ref(),
        Some(&advanced)
    );
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(std::fs::read_dir(&pending_dir).unwrap().count(), 0);
    let _restarted = VaultOptimizerService::try_new(namespace.clone()).unwrap();
    assert_eq!(std::fs::read_dir(&pending_dir).unwrap().count(), 0);
    let queue: serde_json::Value = serde_json::from_slice(
        &std::fs::read(namespace.join("vault_migration/optimizer/queue.json")).unwrap(),
    )
    .unwrap();
    assert!(queue["queue"].as_array().unwrap().iter().any(|job| {
        job["job_id"].as_str() == Some(original_job_id.as_str()) && job["attempts"] == 1
    }));
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
async fn note_delete_repairs_readiness_from_its_exact_commit_token() {
    let (state, vault_dir, _data_dir) = build_test_state();
    let coordinator = state.mutation_coordinator.as_ref().unwrap().clone();
    let namespace = coordinator.current_namespace_path().unwrap();
    *state.knowledge_store.write().await = KnowledgeStore::with_event_recorder(
        vault_dir.path().to_path_buf(),
        namespace,
        coordinator.clone(),
    );
    let create = NoteCreate {
        title: "Delete repair sentinel".into(),
        content: "repair-delete-marker".into(),
        relative_path: None,
        aliases: Vec::new(),
        status: NoteStatus::Draft,
        tags: Vec::new(),
        schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
        migration_source: None,
        optimizer_managed: false,
        properties: Default::default(),
    };
    let (created, create_commit) = {
        let expected = coordinator.current_authority_token().unwrap();
        state
            .knowledge_store
            .write()
            .await
            .create_note_expecting_authority(create, "note_editor", expected)
            .unwrap()
    };
    assert!(matches!(
        repair_after_authority_mutation(&state, &create_commit, "note create").await,
        PostAuthorityRepair::Ready(_)
    ));
    let delete_commit = {
        let expected = coordinator.current_authority_token().unwrap();
        state
            .knowledge_store
            .write()
            .await
            .delete_note_expecting_authority(&created.id, "note_editor", expected)
            .unwrap()
    };

    assert!(matches!(
        repair_after_authority_mutation(&state, &delete_commit, "note delete").await,
        PostAuthorityRepair::Ready(_)
    ));
    coordinator.require_namespace_ready().unwrap();
    assert!(state
        .knowledge_store
        .read()
        .await
        .get_note(&created.id)
        .is_err());
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

/// Regression coverage for the optimizer-reindex bug: the background
/// vault optimizer's `sidecar_first` write path (`VaultOptimizerService::
/// prepare_next`) changes a note's tags via an overlay file, and that
/// change IS visible through `KnowledgeStore::get_note` (which merges the
/// overlay), but the search index's STORED tags field is a snapshot
/// frozen at the last `index_note` call — it goes stale the moment the
/// optimizer writes, until some other code path happens to reindex the
/// note. Searching by the note's own (unchanged) title still matches via
/// the title field regardless of the tag staleness, so the returned
/// `SearchResult::note.tags` — reconstructed purely from the indexed
/// document, not from the vault — is a direct probe of whether a reindex
/// actually happened.
#[tokio::test]
async fn optimizer_sidecar_write_is_reindexed_into_search() {
    let (state, _vault_dir, _data_dir) = build_test_state();

    let created = {
        let mut store = state.knowledge_store.write().await;
        store
            .create_note(NoteCreate {
                title: "Quokka Alpha Habitat".to_string(),
                content: "Quokkas are found on Rottnest Island.".to_string(),
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
    // Indexes the note once (empty tags) and enqueues it into the
    // optimizer's queue, mirroring what actually happens on note
    // creation in the running app.
    commit_note_write(&state, &created.id, "test_note_created")
        .await
        .expect("commit_note_write should succeed");

    let expected_tag = normalize_topic_key(&created.title).replace('-', "_");
    let settings = UserSettings::default();
    assert_eq!(
        settings.background_vault_optimizer_edit_mode, "sidecar_first",
        "this test exercises the sidecar_first path specifically"
    );

    // One worker tick, mirroring `start_vault_optimizer_worker`:
    // preparation is read-only and returns an exact pending write, then
    // application runs under the knowledge-store write lock.
    let tick = {
        let store = state.knowledge_store.read().await;
        let mut optimizer = state.vault_optimizer.write().await;
        optimizer
            .prepare_next(&store, &settings)
            .expect("prepare_next should not error")
    };
    let pending = match tick {
        crate::services::vault_optimizer::OptimizerTick::Pending(pending) => pending,
        other => panic!("sidecar_first mode must return a pending write, got {other:?}"),
    };
    let applied_note_id = {
        let mut store = state.knowledge_store.write().await;
        let mut optimizer = state.vault_optimizer.write().await;
        match optimizer
            .apply_pending(&mut store, *pending)
            .expect("apply_pending should not error")
        {
            crate::services::vault_optimizer::OptimizerMutationResult::Committed {
                result, ..
            } => result.note_id().to_string(),
            crate::services::vault_optimizer::OptimizerMutationResult::NoWrite => {
                panic!("sidecar_first apply should report the committed note")
            }
        }
    };

    // The locks taken above are released by now (the block ended) —
    // `commit_note_index_refresh` is called exactly as the worker calls
    // it, only after releasing every optimizer-tick lock.
    commit_note_index_refresh(&state, &applied_note_id)
        .await
        .expect("commit_note_index_refresh should succeed");

    let search = state.search_service.read().await;
    let results = search
        .search(&created.title, 10)
        .expect("search should not error");
    let found = results
        .iter()
        .find(|r| r.note.id == created.id)
        .expect("note should still be findable by its unchanged title");
    assert!(
        found.note.tags.contains(&expected_tag),
        "expected the optimizer's sidecar overlay tag '{}' to be visible in \
         search results without any manual reindex, got tags: {:?}",
        expected_tag,
        found.note.tags
    );
}

/// Same regression as `optimizer_sidecar_write_is_reindexed_into_search`,
/// but for the `full_rewrite` edit mode: `prepare_next` returns a
/// `PendingOptimizerWrite` that the caller applies via `apply_pending`
/// under a write lock (a real `KnowledgeStore::update_note`, not a
/// sidecar overlay). That write is also invisible to search until
/// something reindexes it.
#[tokio::test]
async fn optimizer_full_rewrite_write_is_reindexed_into_search() {
    let (state, _vault_dir, _data_dir) = build_test_state();

    let created = {
        let mut store = state.knowledge_store.write().await;
        store
            .create_note(NoteCreate {
                title: "Narwhal Beta Colony".to_string(),
                content: "Narwhals gather near Baffin Island in summer.".to_string(),
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

    let expected_tag = normalize_topic_key(&created.title).replace('-', "_");
    let settings = UserSettings {
        background_vault_optimizer_edit_mode: "full_rewrite".to_string(),
        ..UserSettings::default()
    };

    let tick = {
        let store = state.knowledge_store.read().await;
        let mut optimizer = state.vault_optimizer.write().await;
        optimizer
            .prepare_next(&store, &settings)
            .expect("prepare_next should not error")
    };
    let pending = match tick {
        crate::services::vault_optimizer::OptimizerTick::Pending(pending) => pending,
        other => panic!(
            "full_rewrite mode must return a pending write, got {:?}",
            other
        ),
    };

    let applied_note_id = {
        let mut store = state.knowledge_store.write().await;
        let mut optimizer = state.vault_optimizer.write().await;
        match optimizer
            .apply_pending(&mut store, *pending)
            .expect("apply_pending should not error")
        {
            crate::services::vault_optimizer::OptimizerMutationResult::Committed {
                result, ..
            } => result.note_id().to_string(),
            crate::services::vault_optimizer::OptimizerMutationResult::NoWrite => {
                panic!("full_rewrite apply should report the committed note")
            }
        }
    };

    // The locks taken above are released by now (the block ended) —
    // `commit_note_index_refresh` is called exactly as the worker calls
    // it, only after releasing every optimizer-tick lock.
    commit_note_index_refresh(&state, &applied_note_id)
        .await
        .expect("commit_note_index_refresh should succeed");

    let search = state.search_service.read().await;
    let results = search
        .search(&created.title, 10)
        .expect("search should not error");
    let found = results
        .iter()
        .find(|r| r.note.id == created.id)
        .expect("note should still be findable by its unchanged title");
    assert!(
        found.note.tags.contains(&expected_tag),
        "expected the optimizer's full_rewrite tag '{}' to be visible in \
         search results without any manual reindex, got tags: {:?}",
        expected_tag,
        found.note.tags
    );
}

/// `commit_note_delete` is the symmetric counterpart to
/// `commit_note_write`: a note removed from the vault must stop being
/// findable via search immediately, without a manual reindex.
#[tokio::test]
async fn deleted_note_is_no_longer_search_indexed_after_commit_note_delete() {
    let (state, _vault_dir, _data_dir) = build_test_state();

    let created = {
        let mut store = state.knowledge_store.write().await;
        store
            .create_note(NoteCreate {
                title: "Platypus Gamma Burrow".to_string(),
                content: "Platypuses dig burrows along riverbanks in Tasmania.".to_string(),
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

    // Sanity check: the note is indeed searchable before deletion.
    {
        let search = state.search_service.read().await;
        let results = search
            .search(&created.title, 10)
            .expect("search should not error");
        assert!(
            results.iter().any(|r| r.note.id == created.id),
            "note should be search-indexed before deletion"
        );
    }

    {
        let mut store = state.knowledge_store.write().await;
        store
            .delete_note(&created.id)
            .expect("note should be deleted from the vault");
    }

    commit_note_delete(&state, &created.id, "test_note_deleted")
        .await
        .expect("commit_note_delete should succeed");

    let search = state.search_service.read().await;
    let results = search
        .search(&created.title, 10)
        .expect("search should not error");
    assert!(
        !results.iter().any(|r| r.note.id == created.id),
        "expected deleted note to be immediately removed from search results \
         without any manual reindex, found: {:?}",
        results.iter().map(|r| &r.note.id).collect::<Vec<_>>()
    );
}
