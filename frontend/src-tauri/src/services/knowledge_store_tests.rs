use super::*;
use crate::models::note::NoteStatus;
use crate::services::atomic_io::assert_no_tmp_siblings;
use tempfile::tempdir;

#[test]
fn sync_policy_is_opt_out_and_fails_closed_on_invalid_frontmatter() {
    assert!(note_allows_sync("# No frontmatter\n"));
    assert!(note_allows_sync(
        "---\ngrafyn_sync: inherit\ntitle: Shared\n---\n\nShared"
    ));
    assert!(!note_allows_sync(
        "---\ngrafyn_sync: local_only\ntitle: Private\n---\n\nPrivate"
    ));
    assert!(!note_allows_sync(
        "---\ngrafyn_sync: unexpected\n---\n\nUnknown policy"
    ));
    assert!(!note_allows_sync("---\ngrafyn_sync: [broken\n---\n"));
    assert_eq!(
        note_identity_from_markdown("---\nnote_id: private-memory\ngrafyn_sync: local_only\n---\n")
            .as_deref(),
        Some("private-memory")
    );
}

#[test]
fn sync_policy_fails_closed_on_unterminated_frontmatter() {
    for markdown in [
        "---\ngrafyn_sync: local_only\nprivate body\n",
        "---\r\ntitle: Unterminated\r\nshared body\r\n",
        "---\ngrafyn_sync: [broken\nprivate body\n",
    ] {
        assert!(!note_allows_sync(markdown), "accepted {markdown:?}");
    }
}

fn task_seven_note_create(title: &str, content: &str, relative_path: &str) -> NoteCreate {
    NoteCreate {
        title: title.into(),
        content: content.into(),
        relative_path: Some(relative_path.into()),
        aliases: Vec::new(),
        status: crate::models::note::NoteStatus::Draft,
        tags: Vec::new(),
        schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
        migration_source: None,
        optimizer_managed: false,
        properties: HashMap::new(),
    }
}

#[test]
fn coordinated_note_crud_emits_once_and_move_is_one_compound_event() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut store = KnowledgeStore::with_event_recorder(vault.clone(), data, coordinator);
    let created = store
        .create_note(task_seven_note_create("Captured", "one", "captured.md"))
        .unwrap();
    assert_eq!(event_store.ordered_events().unwrap().len(), 1);
    store
        .update_note(
            &created.id,
            NoteUpdate {
                content: Some("two".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(event_store.ordered_events().unwrap().len(), 2);
    store
        .update_note(
            &created.id,
            NoteUpdate {
                content: Some("two".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(event_store.ordered_events().unwrap().len(), 2);
    store
        .update_note(
            &created.id,
            NoteUpdate {
                relative_path: Some("moved/captured.md".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(event_store.ordered_events().unwrap().len(), 3);
    assert!(!vault.join("captured.md").exists());
    assert!(vault.join("moved/captured.md").exists());
    store.delete_note(&created.id).unwrap();
    let events = event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 4);
    assert!(matches!(
        events.last().unwrap().payload,
        crate::models::twin_event::TwinEventPayload::NoteChanged(
            crate::models::twin_event::NoteChanged {
                change: crate::models::twin_event::NoteChangeKind::Deleted,
                ..
            }
        )
    ));
}

#[test]
fn coordinated_vault_switch_retargets_markdown_mutations_before_store_path() {
    let root = tempdir().unwrap();
    let vault_a = root.path().join("vault-a");
    let vault_b = root.path().join("vault-b");
    let data = root.path().join("data");
    std::fs::create_dir(&vault_a).unwrap();
    std::fs::create_dir(&vault_b).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault_a,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut store = KnowledgeStore::with_event_recorder(vault_a.clone(), data, coordinator);

    store.set_vault_path(vault_b.clone()).unwrap();
    store
        .create_note(task_seven_note_create(
            "After switch",
            "new vault only",
            "switched.md",
        ))
        .unwrap();

    assert!(!vault_a.join("switched.md").exists());
    assert!(vault_b.join("switched.md").exists());
    assert_eq!(event_store.ordered_events().unwrap().len(), 1);
}

#[test]
fn coordinated_persistence_failure_leaves_cache_and_events_unchanged() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    std::fs::write(vault.join("blocked"), "regular file").unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut store = KnowledgeStore::with_event_recorder(vault, data, coordinator);
    assert!(store
        .create_note(task_seven_note_create(
            "Must fail",
            "never committed",
            "blocked/note.md",
        ))
        .is_err());
    assert!(store.list_notes().unwrap().is_empty());
    assert!(event_store.ordered_events().unwrap().is_empty());
}

#[test]
fn interrupted_note_target_refreshes_cache_from_durable_bytes_before_recovery() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut store = KnowledgeStore::with_event_recorder(vault.clone(), data, coordinator.clone());
    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));
    let (created, commit) = store
        .create_note_from_source_with_commit(
            task_seven_note_create(
                "Durable before event",
                "survives restart",
                "durable-before-event.md",
            ),
            "note_editor",
        )
        .expect("the exact durable note effect must converge in-call");
    assert!(commit.postcommit_warning);
    assert!(commit.mutation_id.is_some());
    assert!(commit.authority_token.is_some());
    assert_eq!(commit.events.len(), 1);
    assert_eq!(created.id, "durable-before-event");

    assert!(vault.join("durable-before-event.md").exists());
    let cached = store.list_notes().unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].id, "durable-before-event");
    assert_eq!(event_store.ordered_events().unwrap().len(), 1);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(coordinator.recover_pending().unwrap(), 0);
    assert_eq!(event_store.ordered_events().unwrap().len(), 1);
}

#[test]
fn interrupted_note_delete_recovers_markdown_overlay_and_generation_together() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let scoped_data = coordinator.current_namespace_path().unwrap();
    let mut store =
        KnowledgeStore::with_event_recorder(vault.clone(), scoped_data, coordinator.clone());
    let created = store
        .create_note(task_seven_note_create(
            "Compound delete",
            "body",
            "compound.md",
        ))
        .unwrap();
    store
        .write_overlay(
            &created.id,
            &serde_json::json!({"aliases": ["Compound Alias"]}),
        )
        .unwrap();
    let overlay_path = store.overlay_path(&created.id);
    assert!(overlay_path.is_file());
    let before_delete = coordinator.current_authority_token().unwrap();

    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));
    let commit = store
        .delete_note_from_source_with_commit(&created.id, "note_editor")
        .expect("the exact partial delete must converge in-call");
    assert!(commit.postcommit_warning);
    assert!(commit.mutation_id.is_some());
    assert_eq!(commit.events.len(), 1);
    let committed = commit.authority_token.unwrap();
    assert_eq!(
        committed.authority_generation,
        before_delete.authority_generation + 1
    );
    assert!(!vault.join("compound.md").exists());
    assert!(!overlay_path.exists());
    assert_eq!(coordinator.pending_count().unwrap(), 0);

    assert_eq!(coordinator.recover_pending().unwrap(), 0);
    assert!(!overlay_path.exists());
    assert_eq!(event_store.ordered_events().unwrap().len(), 2);
    assert_eq!(coordinator.current_authority_token().unwrap(), committed);
    assert_eq!(coordinator.recover_pending().unwrap(), 0);
    assert_eq!(coordinator.current_authority_token().unwrap(), committed);
}

#[test]
fn pending_update_is_recovered_before_fresh_note_fields_are_planned() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut first =
        KnowledgeStore::with_event_recorder(vault.clone(), data.clone(), coordinator.clone());
    let created = first
        .create_note(task_seven_note_create(
            "Concurrent",
            "original",
            "concurrent.md",
        ))
        .unwrap();
    let mut stale = KnowledgeStore::with_event_recorder(vault.clone(), data, coordinator.clone());

    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterStage);
    let (first_update, commit) = first
        .update_note_from_source_with_commit(
            &created.id,
            NoteUpdate {
                content: Some("recovered content".into()),
                ..Default::default()
            },
            "note_editor",
        )
        .expect("the staged update must converge in-call");
    assert_eq!(first_update.content, "recovered content");
    assert!(commit.postcommit_warning);
    assert!(commit.mutation_id.is_some());
    assert!(commit.authority_token.is_some());
    assert_eq!(commit.events.len(), 1);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(coordinator.recover_pending().unwrap(), 0);
    let updated = stale
        .update_note(
            &created.id,
            NoteUpdate {
                tags: Some(vec!["fresh-tag".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(updated.content, "recovered content");
    assert_eq!(updated.tags, vec!["fresh-tag"]);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(event_store.ordered_events().unwrap().len(), 3);
}

#[test]
fn pending_same_title_create_allocates_a_fresh_id_after_recovery() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut first =
        KnowledgeStore::with_event_recorder(vault.clone(), data.clone(), coordinator.clone());
    let mut stale = KnowledgeStore::with_event_recorder(vault.clone(), data, coordinator.clone());

    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterStage);
    let (first_created, commit) = first
        .create_note_from_source_with_commit(
            task_seven_note_create("Same title", "first", "same-title.md"),
            "note_editor",
        )
        .expect("the staged create must converge in-call");
    assert!(commit.postcommit_warning);
    assert!(commit.mutation_id.is_some());
    assert!(commit.authority_token.is_some());
    assert_eq!(commit.events.len(), 1);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(coordinator.recover_pending().unwrap(), 0);
    let second = stale
        .create_note(task_seven_note_create(
            "Same title",
            "second",
            "same-title.md",
        ))
        .unwrap();

    let notes = stale.list_full_notes().unwrap();
    assert_eq!(notes.len(), 2);
    assert_ne!(notes[0].id, notes[1].id);
    assert!(notes.iter().any(|note| note.content == "first"));
    assert!(notes.iter().any(|note| note.content == "second"));
    assert_eq!(first_created.relative_path, "same-title.md");
    assert_ne!(second.relative_path, "same-title.md");
    assert_eq!(event_store.ordered_events().unwrap().len(), 2);
}

#[test]
fn pending_create_is_recovered_before_import_allocates_ids_and_paths() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut first =
        KnowledgeStore::with_event_recorder(vault.clone(), data.clone(), coordinator.clone());
    let mut stale = KnowledgeStore::with_event_recorder(vault, data, coordinator.clone());

    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterStage);
    let (first_created, commit) = first
        .create_note_from_source_with_commit(
            task_seven_note_create("Shared title", "first", "shared.md"),
            "note_editor",
        )
        .expect("the staged create must converge in-call");
    assert!(commit.postcommit_warning);
    assert!(commit.mutation_id.is_some());
    assert!(commit.authority_token.is_some());
    assert_eq!(commit.events.len(), 1);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(coordinator.recover_pending().unwrap(), 0);
    let imported = stale
        .import_note_container(
            vec![task_seven_note_create(
                "Shared title",
                "imported",
                "shared.md",
            )],
            "fresh-import",
            b"fresh source",
        )
        .unwrap();

    let notes = stale.list_full_notes().unwrap();
    assert_eq!(notes.len(), 2);
    assert_ne!(notes[0].id, notes[1].id);
    assert_eq!(first_created.relative_path, "shared.md");
    assert_ne!(imported[0].relative_path, "shared.md");
    assert!(notes.iter().any(|note| note.content == "first"));
    assert!(notes.iter().any(|note| note.content == "imported"));
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(event_store.ordered_events().unwrap().len(), 3);
}

#[test]
fn pending_move_is_recovered_before_delete_resolves_the_durable_path() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut first =
        KnowledgeStore::with_event_recorder(vault.clone(), data.clone(), coordinator.clone());
    let created = first
        .create_note(task_seven_note_create("Delete moved", "body", "old.md"))
        .unwrap();
    let mut stale = KnowledgeStore::with_event_recorder(vault.clone(), data, coordinator.clone());

    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterStage);
    let (moved, commit) = first
        .update_note_from_source_with_commit(
            &created.id,
            NoteUpdate {
                relative_path: Some("moved/new.md".into()),
                ..Default::default()
            },
            "note_editor",
        )
        .expect("the staged move must converge in-call");
    assert!(commit.postcommit_warning);
    assert!(commit.mutation_id.is_some());
    assert!(commit.authority_token.is_some());
    assert_eq!(commit.events.len(), 1);
    assert_eq!(moved.relative_path, "moved/new.md");
    assert!(!vault.join("old.md").exists());
    assert!(vault.join("moved/new.md").exists());
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(coordinator.recover_pending().unwrap(), 0);
    stale.delete_note(&created.id).unwrap();

    assert!(!vault.join("old.md").exists());
    assert!(!vault.join("moved/new.md").exists());
    assert!(stale.list_notes().unwrap().is_empty());
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(event_store.ordered_events().unwrap().len(), 3);
}

#[test]
fn pending_update_is_recovered_before_restore_records_fresh_evidence() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut first =
        KnowledgeStore::with_event_recorder(vault.clone(), data.clone(), coordinator.clone());
    let created = first
        .create_note(task_seven_note_create(
            "Restore fresh",
            "original body",
            "restore.md",
        ))
        .unwrap();
    let original = std::fs::read_to_string(vault.join("restore.md")).unwrap();
    let restored = original.replace("original body", "restored body");
    let mut stale =
        KnowledgeStore::with_event_recorder(vault.clone(), data.clone(), coordinator.clone());

    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterStage);
    let (updated, commit) = first
        .update_note_from_source_with_commit(
            &created.id,
            NoteUpdate {
                content: Some("pending recovered body".into()),
                ..Default::default()
            },
            "note_editor",
        )
        .expect("the staged update must converge in-call");
    assert_eq!(updated.content, "pending recovered body");
    assert!(commit.postcommit_warning);
    assert!(commit.mutation_id.is_some());
    assert!(commit.authority_token.is_some());
    assert_eq!(commit.events.len(), 1);
    let recovered_digest = crate::services::twin_events::digest_bytes(
        &std::fs::read(vault.join("restore.md")).unwrap(),
    );
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(coordinator.recover_pending().unwrap(), 0);

    let note = stale
        .restore_note_bytes_from_source("restore.md", restored.as_bytes(), "migration")
        .unwrap();
    assert_eq!(note.content, "restored body");
    let events = event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[2].evidence[0].digest, Some(recovered_digest));
    assert_eq!(coordinator.pending_count().unwrap(), 0);
}

#[test]
fn case_only_note_move_is_rejected_without_changing_bytes() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut store = KnowledgeStore::with_event_recorder(vault.clone(), data, coordinator);
    let note = store
        .create_note(task_seven_note_create("Case move", "body", "Case.md"))
        .unwrap();
    let before = std::fs::read(vault.join("Case.md")).unwrap();
    assert!(store
        .update_note(
            &note.id,
            NoteUpdate {
                relative_path: Some("case.md".into()),
                ..Default::default()
            },
        )
        .is_err());
    assert_eq!(std::fs::read(vault.join("Case.md")).unwrap(), before);
    assert_eq!(event_store.ordered_events().unwrap().len(), 1);
}

#[test]
fn coordinated_import_container_is_one_sorted_group_with_no_parser_noise() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut store = KnowledgeStore::with_event_recorder(vault, data, coordinator.clone());
    let before = coordinator.current_authority_token().unwrap();
    let (notes, commit) = store
        .import_note_container_with_commit(
            vec![
                task_seven_note_create("Zulu", "section z", "zulu.md"),
                task_seven_note_create("Alpha", "section a", "alpha.md"),
            ],
            "document-one",
            b"the original imported document",
        )
        .unwrap();
    assert_eq!(notes.len(), 2);
    assert!(commit.mutation_id.is_some());
    assert_eq!(commit.events.len(), 3);
    let committed = commit
        .authority_token
        .expect("import returns its exact authority");
    assert_eq!(committed.root_scope, before.root_scope);
    assert_eq!(
        committed.authority_generation,
        before.authority_generation + 1
    );
    assert_eq!(coordinator.current_authority_token().unwrap(), committed);
    let events = event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 3);
    let note_ids = events[..2]
        .iter()
        .map(|event| match &event.payload {
            crate::models::twin_event::TwinEventPayload::NoteChanged(value) => {
                value.note_id.as_str()
            }
            _ => panic!("only persisted notes precede the container observation"),
        })
        .collect::<Vec<_>>();
    assert_eq!(note_ids, vec!["alpha", "zulu"]);
    let crate::models::twin_event::TwinEventPayload::ObservationRecorded(observation) =
        &events[2].payload
    else {
        panic!("one container observation must close the import group");
    };
    assert!(observation.claims.is_empty());
    assert_eq!(events[2].evidence.len(), 3);
    assert_eq!(
        events[2]
            .evidence
            .iter()
            .filter(|evidence| evidence.evidence_type
                == crate::models::twin_event::EvidenceType::Import)
            .count(),
        1
    );
    assert!(events.iter().all(|event| {
        event.governance.sensitivity == crate::models::twin_event::Sensitivity::Sensitive
            && !event.governance.allowed_uses.export
            && !event.governance.allowed_uses.training
    }));
    assert_eq!(events[1].causal_parents, vec![events[0].event_id.clone()]);
    assert_eq!(events[2].causal_parents, vec![events[1].event_id.clone()]);
}

#[test]
fn coordinated_import_preserves_authority_advanced_commit_for_its_caller() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    events.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            events,
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut store = KnowledgeStore::with_event_recorder(vault, data, coordinator.clone());

    coordinator.fail_next_replays_before_targets(2);
    let error = store
        .import_note_container_with_commit(
            vec![task_seven_note_create(
                "Pending import",
                "exactly once",
                "pending-import.md",
            )],
            "pending-container",
            b"pending source",
        )
        .expect_err("the exact authority-advanced outcome must reach the caller");
    let outcome = knowledge_authority_advanced_outcome(&error)
        .expect("KnowledgeStore must preserve the exact non-retryable outcome");
    assert_eq!(
        outcome.commit.authority_token,
        Some(coordinator.current_authority_token().unwrap())
    );
    assert_eq!(outcome.note_ids, vec!["pending-import"]);
    assert!(!outcome.target_aborted);
    assert_eq!(coordinator.pending_count().unwrap(), 1);
}

#[test]
fn import_container_enforces_the_sixty_three_note_prewrite_boundary() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut store = KnowledgeStore::with_event_recorder(vault.clone(), data, coordinator);
    let creates = (0..63)
        .map(|index| {
            task_seven_note_create(
                &format!("Section {index:02}"),
                "body",
                &format!("section-{index:02}.md"),
            )
        })
        .collect();
    assert_eq!(
        store
            .import_note_container(creates, "document-63", b"source-63")
            .unwrap()
            .len(),
        63
    );
    assert_eq!(event_store.ordered_events().unwrap().len(), 64);

    let too_many = (0..64)
        .map(|index| {
            task_seven_note_create(
                &format!("Overflow {index:02}"),
                "body",
                &format!("overflow-{index:02}.md"),
            )
        })
        .collect();
    assert!(store
        .import_note_container(too_many, "document-64", b"source-64")
        .is_err());
    assert_eq!(event_store.ordered_events().unwrap().len(), 64);
    assert!(!vault.join("overflow-00.md").exists());
}

#[test]
fn note_and_overlay_writes_are_atomic_with_no_tmp_litter() {
    let vault_dir = tempdir().expect("vault tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let mut store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );

    let note = store
        .create_note(NoteCreate {
            title: "Atomic Adoption".to_string(),
            content: "Body content survives the temp+rename write.".to_string(),
            relative_path: None,
            aliases: Vec::new(),
            status: Default::default(),
            tags: vec!["adoption".to_string()],
            schema_version: CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            properties: HashMap::new(),
        })
        .expect("note should be created");

    let note_path = vault_dir.path().join(&note.relative_path);
    let persisted = std::fs::read_to_string(&note_path).expect("note file should exist");
    assert!(persisted.contains("Atomic Adoption"));
    assert!(persisted.contains("Body content survives the temp+rename write."));
    assert_no_tmp_siblings(vault_dir.path());

    store
        .write_overlay(
            &note.id,
            &serde_json::json!({"aliases": ["Adoption Alias"]}),
        )
        .expect("overlay should be written");
    let overlay =
        std::fs::read_to_string(store.overlay_path(&note.id)).expect("overlay file should exist");
    assert!(overlay.contains("Adoption Alias"));
    assert_no_tmp_siblings(&store.overlay_notes_dir);
}

#[test]
fn optimizer_provenanced_overlay_is_ignored_after_markdown_source_changes() {
    let vault_dir = tempdir().expect("vault tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let mut store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );
    let note = store
        .create_note(task_seven_note_create(
            "Bound Overlay",
            "Original source",
            "bound-overlay.md",
        ))
        .unwrap();
    let markdown_path = vault_dir.path().join(&note.relative_path);
    let original = std::fs::read(&markdown_path).unwrap();
    let source_digest = crate::services::twin_events::digest_bytes(&original);
    store
        .write_overlay(
            &note.id,
            &serde_json::json!({
                "tags": ["optimizer-bound"],
                "_grafyn_optimizer_source_v1": {
                    "relative_path": note.relative_path,
                    "sha256": source_digest,
                }
            }),
        )
        .unwrap();
    assert!(store
        .get_note(&note.id)
        .unwrap()
        .tags
        .contains(&"optimizer-bound".to_string()));

    let mut edited = original;
    edited.extend_from_slice(b"\nExternal edit.\n");
    std::fs::write(&markdown_path, edited).unwrap();

    assert!(!store
        .get_note(&note.id)
        .unwrap()
        .tags
        .contains(&"optimizer-bound".to_string()));
}

#[test]
fn coordinated_overlay_write_invalidates_authority_without_emitting_an_event() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store.clone(),
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let before = coordinator.current_authority_token().unwrap();
    let scoped_data = crate::services::vault_namespace::scoped_data_path(&data, &before.root_scope);
    let store = KnowledgeStore::with_event_recorder(vault, scoped_data, coordinator.clone());

    store
        .write_overlay(
            "captured-overlay",
            &serde_json::json!({"aliases": ["Captured Alias"]}),
        )
        .unwrap();

    let after = coordinator.current_authority_token().unwrap();
    assert_eq!(after.authority_generation, before.authority_generation + 1);
    assert!(coordinator.require_namespace_ready().is_err());
    assert_eq!(event_store.ordered_events().unwrap().len(), 0);
    assert!(store.overlay_path("captured-overlay").is_file());
}

#[test]
fn optimizer_overlay_aborts_before_publication_when_its_source_authority_is_stale() {
    let root = tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    std::fs::create_dir(&vault).unwrap();
    std::fs::create_dir(&data).unwrap();
    let events = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    events.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            events,
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let stale = coordinator.current_authority_token().unwrap();
    let scoped = crate::services::vault_namespace::scoped_data_path(&data, &stale.root_scope);
    let mut store = KnowledgeStore::with_event_recorder(vault, scoped, coordinator.clone());
    store
        .create_note(task_seven_note_create("Peer", "changed", "peer.md"))
        .unwrap();

    let error = store
        .write_overlay_from_source_expecting_authority(
            "captured-overlay",
            &serde_json::json!({"aliases": ["Must not land"]}),
            "vault_optimizer",
            stale,
        )
        .unwrap_err();
    assert!(error.to_string().contains("authority"));
    assert!(!store.overlay_path("captured-overlay").exists());
}

#[test]
fn validate_note_id_rejects_colon_variants() {
    assert!(
        KnowledgeStore::validate_note_id("C:foo").is_err(),
        "drive-relative id must be rejected"
    );
    assert!(
        KnowledgeStore::validate_note_id("foo:bar").is_err(),
        "alternate-data-stream id must be rejected"
    );
}

#[test]
fn validate_note_id_rejects_reserved_device_stems_case_insensitively() {
    for candidate in [
        "con",
        "CON",
        "con.md",
        "CON.backup",
        "prn",
        "PRN.md",
        "aux",
        "AUX",
        "nul",
        "NUL.md",
        "com1",
        "COM1.md",
        "com9",
        "COM9",
        "lpt1",
        "LPT1.md",
        "lpt9",
        "LPT9",
        // Windows strips trailing spaces and dots before device-name
        // resolution, so these variants still reach the device.
        "con ",
        "con .md",
        "CON. .md",
        "nul.",
        "aux . .md",
    ] {
        assert!(
            KnowledgeStore::validate_note_id(candidate).is_err(),
            "expected '{}' to be rejected as a reserved device name",
            candidate
        );
    }
}

#[test]
fn validate_note_id_accepts_normal_unicode_titles() {
    for candidate in [
        "my-note",
        "笔记-notes",
        "my.note.v2",
        "project-plan-2026",
        "console-notes",
        "company",
        // Trailing space on a non-reserved stem must stay accepted:
        // trimmed stem is "console", which is not a device name.
        "console ",
    ] {
        assert!(
            KnowledgeStore::validate_note_id(candidate).is_ok(),
            "expected '{}' to be accepted",
            candidate
        );
    }
}

#[test]
fn normalize_note_relative_path_rejects_colon_variants() {
    assert!(
        normalize_note_relative_path("C:foo").is_err(),
        "drive-relative path must be rejected"
    );
    assert!(
        normalize_note_relative_path("foo:bar.md").is_err(),
        "alternate-data-stream path must be rejected"
    );
    assert!(
        normalize_note_relative_path("sub/c:d.md").is_err(),
        "colon in a nested component must be rejected"
    );
}

#[test]
fn normalize_note_relative_path_rejects_reserved_device_components() {
    for candidate in [
        "con",
        "CON.md",
        "prn.md",
        "aux",
        "nul.md",
        "com1.md",
        "lpt9",
        "con.backup.md",
        "sub/CON/note.md",
        "sub/prn.md/note.md",
        // Trailing spaces/dots are stripped by Windows before device-name
        // resolution, so these still reach the device.
        "con .md",
        "sub/CON. .md",
        "sub/nul ./note.md",
    ] {
        assert!(
            normalize_note_relative_path(candidate).is_err(),
            "expected '{}' to be rejected",
            candidate
        );
    }
}

#[test]
fn normalize_note_relative_path_accepts_normal_unicode_paths() {
    for candidate in [
        "笔记-notes.md",
        "folder/my.note.v2.md",
        "notes/2026/plan.md",
        "console-notes.md",
        "folder/console .md",
    ] {
        assert!(
            normalize_note_relative_path(candidate).is_ok(),
            "expected '{}' to be accepted",
            candidate
        );
    }
}

#[test]
fn ensure_path_within_vault_rejects_parent_dir_escape() {
    let vault_dir = tempdir().expect("vault tempdir");
    let vault_path = vault_dir.path();
    let joined = vault_path.join(Path::new("../../outside.md"));
    assert!(
        ensure_path_within_vault(vault_path, &joined).is_err(),
        "parent-dir escape must be rejected"
    );
}

#[cfg(windows)]
#[test]
fn ensure_path_within_vault_rejects_drive_relative_escape_on_windows() {
    let vault_dir = tempdir().expect("vault tempdir");
    let vault_path = vault_dir.path();
    // On Windows, PathBuf::join replaces the base entirely when the
    // argument carries its own drive prefix (drive-relative path).
    let joined = vault_path.join("C:secret.md");
    assert!(
        ensure_path_within_vault(vault_path, &joined).is_err(),
        "drive-relative escape must be rejected"
    );
}

#[test]
fn store_rejects_hostile_note_ids_and_paths_end_to_end() {
    let vault_dir = tempdir().expect("vault tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let mut store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );

    assert!(store.get_note("C:secret").is_err());
    assert!(store.get_note("con").is_err());
    assert!(store.delete_note("con").is_err());
    assert!(store
        .create_note(NoteCreate {
            title: "Hostile".to_string(),
            content: "x".to_string(),
            relative_path: Some("C:evil.md".to_string()),
            aliases: Vec::new(),
            status: Default::default(),
            tags: Vec::new(),
            schema_version: CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            properties: HashMap::new(),
        })
        .is_err());
}

#[test]
fn reserved_synced_materialization_preserves_only_frontmatter_aliases() {
    let vault_dir = tempdir().expect("vault tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let synced_dir = vault_dir.path().join("synced");
    std::fs::create_dir(&synced_dir).expect("synced directory should be created");
    let remote_stem = "a".repeat(64);
    let relative_path = format!("synced/{remote_stem}.md");
    std::fs::write(
        synced_dir.join(format!("{remote_stem}.md")),
        "---\ntitle: Shared Note\naliases:\n  - Shared Alias\n  - Existing Alias\n---\n\nShared body.",
    )
    .expect("synced note should be written");

    let store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );
    let note = store
        .find_note_by_relative_path(&relative_path)
        .expect("lookup should not error")
        .expect("synced note should be readable");

    assert_eq!(
        note.aliases,
        vec!["Shared Alias".to_string(), "Existing Alias".to_string()]
    );
}

#[test]
fn non_reserved_note_paths_still_derive_filename_aliases() {
    let vault_dir = tempdir().expect("vault tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let lowercase_hash = "b".repeat(64);
    let uppercase_hash = "A".repeat(64);
    let short_hash = "c".repeat(63);
    let cases = [
        (
            "synced/not-a-remote-hash.md".to_string(),
            "Ordinary Synced Note",
            "not a remote hash".to_string(),
        ),
        (
            format!("notes/{lowercase_hash}.md"),
            "User Hash Note",
            lowercase_hash.clone(),
        ),
        (
            format!("synced/{uppercase_hash}.md"),
            "Uppercase Hash Note",
            uppercase_hash,
        ),
        (
            format!("synced/{short_hash}.md"),
            "Short Hash Note",
            short_hash,
        ),
        (
            format!("synced/nested/{lowercase_hash}.md"),
            "Nested Synced Note",
            lowercase_hash,
        ),
    ];

    for (relative_path, title, _) in &cases {
        let note_path = vault_dir.path().join(relative_path);
        std::fs::create_dir_all(note_path.parent().expect("note path should have a parent"))
            .expect("note parent should be created");
        std::fs::write(
            note_path,
            format!("---\ntitle: {title}\naliases:\n  - Explicit Alias\n---\n\nBody."),
        )
        .expect("note should be written");
    }

    let store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );
    for (relative_path, _, expected_derived_alias) in cases {
        let note = store
            .find_note_by_relative_path(&relative_path)
            .expect("lookup should not error")
            .expect("note should be readable");
        assert!(
            note.aliases.contains(&expected_derived_alias),
            "{relative_path} should retain its path-derived alias: {:?}",
            note.aliases
        );
    }
}

/// Frontmatter block with a tab character used as block-sequence indentation,
/// which yaml-rust2 rejects ("tab cannot be used as indentation"). This is not
/// well-formed YAML, so `YamlLoader::load_from_str` errors and the gray_matter
/// engine falls back to `Pod::Null`, which then fails to deserialize into
/// `NoteFrontmatter` (a non-map `Pod` is an invalid type for the struct,
/// regardless of field defaults).
const MALFORMED_FRONTMATTER_NOTE: &str = "---\ntitle: Original Title\ntags:\n\t- alpha\nstatus: canonical\n---\n\nOriginal body content.";

#[test]
fn read_note_with_malformed_yaml_frontmatter_survives_without_error() {
    let vault_dir = tempdir().expect("vault tempdir");
    let data_dir = tempdir().expect("data tempdir");

    let note_path = vault_dir.path().join("broken.md");
    std::fs::write(&note_path, MALFORMED_FRONTMATTER_NOTE).expect("note file should be written");

    let store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );

    let note = store
        .find_note_by_relative_path("broken.md")
        .expect("lookup should not error")
        .expect("malformed note should still be readable");
    assert!(
        note.content.contains("Original body content."),
        "body content should be preserved even though frontmatter failed to parse"
    );
    assert!(
        note.frontmatter_raw_fallback.is_some(),
        "unparsable frontmatter should be retained as a raw fallback"
    );
    assert!(
        note.frontmatter_raw_fallback
            .as_ref()
            .unwrap()
            .contains("Original Title"),
        "raw fallback should contain the original frontmatter text"
    );
}

#[test]
fn content_only_update_preserves_original_raw_frontmatter_verbatim() {
    let vault_dir = tempdir().expect("vault tempdir");
    let data_dir = tempdir().expect("data tempdir");

    let note_path = vault_dir.path().join("broken.md");
    std::fs::write(&note_path, MALFORMED_FRONTMATTER_NOTE).expect("note file should be written");

    let mut store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );
    let note_id = store
        .find_note_by_relative_path("broken.md")
        .expect("lookup should not error")
        .expect("malformed note should still be readable")
        .id;

    store
        .update_note(
            &note_id,
            NoteUpdate {
                content: Some("Updated body content.".to_string()),
                ..Default::default()
            },
        )
        .expect("content-only update should succeed on a malformed-frontmatter note");

    let persisted = std::fs::read_to_string(&note_path).expect("note file should still exist");
    assert!(
        persisted.contains("title: Original Title\ntags:\n\t- alpha\nstatus: canonical"),
        "original raw frontmatter block should be preserved byte-for-byte:\n{persisted}"
    );
    assert!(
        persisted.contains("Updated body content."),
        "new content should be written:\n{persisted}"
    );
    assert!(
        !persisted.contains("Original body content."),
        "old content should be replaced, not appended:\n{persisted}"
    );
}

#[test]
fn explicit_frontmatter_update_replaces_malformed_original() {
    let vault_dir = tempdir().expect("vault tempdir");
    let data_dir = tempdir().expect("data tempdir");

    let note_path = vault_dir.path().join("broken.md");
    std::fs::write(&note_path, MALFORMED_FRONTMATTER_NOTE).expect("note file should be written");

    let mut store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );
    let note_id = store
        .find_note_by_relative_path("broken.md")
        .expect("lookup should not error")
        .expect("malformed note should still be readable")
        .id;

    let updated = store
        .update_note(
            &note_id,
            NoteUpdate {
                status: Some(NoteStatus::Evidence),
                ..Default::default()
            },
        )
        .expect("explicit frontmatter update should succeed");

    assert!(
        updated.frontmatter_raw_fallback.is_none(),
        "explicitly editing a frontmatter field should clear the raw fallback"
    );

    let persisted = std::fs::read_to_string(&note_path).expect("note file should still exist");
    assert!(
        !persisted.contains("\t- alpha"),
        "malformed original frontmatter should no longer be present:\n{persisted}"
    );
    assert!(
        persisted.contains("status: evidence"),
        "newly serialized frontmatter should reflect the explicit edit:\n{persisted}"
    );
}

#[test]
fn well_formed_frontmatter_has_no_raw_fallback() {
    let vault_dir = tempdir().expect("vault tempdir");
    let data_dir = tempdir().expect("data tempdir");
    let mut store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );

    let note = store
        .create_note(NoteCreate {
            title: "Well Formed".to_string(),
            content: "Body.".to_string(),
            relative_path: None,
            aliases: Vec::new(),
            status: Default::default(),
            tags: Vec::new(),
            schema_version: CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            properties: HashMap::new(),
        })
        .expect("note should be created");

    assert!(
        note.frontmatter_raw_fallback.is_none(),
        "a freshly created, well-formed note must not carry a raw fallback"
    );

    let fetched = store.get_note(&note.id).expect("note should be readable");
    assert!(
        fetched.frontmatter_raw_fallback.is_none(),
        "re-reading a well-formed note must not carry a raw fallback"
    );
}

/// Valid YAML frontmatter that simply omits `title:` — the norm for
/// Obsidian-style and imported vaults. This must NOT be treated as a parse
/// failure: title falls back to the H1 heading downstream, and custom fields
/// must flow into `properties` and round-trip through writes.
const TITLE_LESS_VALID_FRONTMATTER_NOTE: &str = "---\nstatus: evidence\ntags:\n  - alpha\ncustom_field: keep-me\n---\n\n# Heading Title\n\nBody text.";

#[test]
fn title_less_valid_frontmatter_parses_without_fallback_and_round_trips() {
    let vault_dir = tempdir().expect("vault tempdir");
    let data_dir = tempdir().expect("data tempdir");

    let note_path = vault_dir.path().join("no-title.md");
    std::fs::write(&note_path, TITLE_LESS_VALID_FRONTMATTER_NOTE)
        .expect("note file should be written");

    let mut store = KnowledgeStore::new(
        vault_dir.path().to_path_buf(),
        data_dir.path().to_path_buf(),
    );

    let note = store
        .find_note_by_relative_path("no-title.md")
        .expect("lookup should not error")
        .expect("title-less note should be readable");
    assert!(
        note.frontmatter_raw_fallback.is_none(),
        "valid frontmatter without a title must NOT be treated as a parse failure"
    );
    assert_eq!(
        note.title, "Heading Title",
        "title should fall back to the H1 heading"
    );
    assert_eq!(note.status, NoteStatus::Evidence);
    assert!(note.tags.contains(&"alpha".to_string()));
    assert_eq!(
        note.properties.get("custom_field").and_then(|v| v.as_str()),
        Some("keep-me"),
        "custom frontmatter fields should flow into properties"
    );

    // Content-only edit: custom field must survive on disk.
    store
        .update_note(
            &note.id,
            NoteUpdate {
                content: Some("# Heading Title\n\nEdited body.".to_string()),
                ..Default::default()
            },
        )
        .expect("content-only update should succeed");
    let persisted = std::fs::read_to_string(&note_path).expect("note file should still exist");
    assert!(
        persisted.contains("custom_field: keep-me"),
        "custom field should round-trip through a content edit:\n{persisted}"
    );
    assert!(persisted.contains("Edited body."));

    // Explicit frontmatter edit: custom field must STILL survive, because the
    // frontmatter deserialized successfully and custom fields live in properties.
    store
        .update_note(
            &note.id,
            NoteUpdate {
                status: Some(NoteStatus::Canonical),
                ..Default::default()
            },
        )
        .expect("explicit status update should succeed");
    let persisted = std::fs::read_to_string(&note_path).expect("note file should still exist");
    assert!(
        persisted.contains("custom_field: keep-me"),
        "custom field should round-trip through an explicit status edit:\n{persisted}"
    );
    assert!(
        persisted.contains("status: canonical"),
        "status edit should be reflected:\n{persisted}"
    );
}
