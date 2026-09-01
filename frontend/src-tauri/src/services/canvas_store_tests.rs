use super::*;
use crate::models::canvas::{
    Debate, DebateResponse, DebateRound, ModelResponse, PromptTile, ResponseStatus, SessionCreate,
    TwinEvidenceSnapshot,
};
use crate::services::atomic_io::assert_no_tmp_siblings;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;

#[derive(Debug)]
struct AuthorityBearingCanvasRecorder {
    authority_token: crate::services::vault_namespace::VaultAuthorityTokenV1,
}

impl crate::services::twin_events::EventRecorder for AuthorityBearingCanvasRecorder {
    fn commit_mutation(
        &self,
        _origin: crate::services::twin_events::MutationOrigin,
        _stream: crate::models::twin_event::CausalStream,
        _source_channel: crate::models::twin_event::SourceChannel,
        _targets: Vec<crate::services::twin_events::TargetMutation>,
        _drafts: Vec<crate::services::twin_events::TwinEventDraft>,
    ) -> Result<
        crate::services::twin_events::MutationCommit,
        crate::services::twin_events::MutationError,
    > {
        Ok(crate::services::twin_events::MutationCommit {
            mutation_id: None,
            events: Vec::new(),
            authority_token: Some(self.authority_token.clone()),
            postcommit_warning: false,
        })
    }
}

#[derive(Debug)]
struct CountingCanvasRecorder(Arc<AtomicUsize>);

impl crate::services::twin_events::EventRecorder for CountingCanvasRecorder {
    fn commit_mutation(
        &self,
        _origin: crate::services::twin_events::MutationOrigin,
        _stream: crate::models::twin_event::CausalStream,
        _source_channel: crate::models::twin_event::SourceChannel,
        _targets: Vec<crate::services::twin_events::TargetMutation>,
        _drafts: Vec<crate::services::twin_events::TwinEventDraft>,
    ) -> Result<
        crate::services::twin_events::MutationCommit,
        crate::services::twin_events::MutationError,
    > {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(crate::services::twin_events::MutationCommit {
            mutation_id: None,
            events: Vec::new(),
            authority_token: None,
            postcommit_warning: false,
        })
    }
}

fn canvas_scope(hex: char) -> crate::models::twin_event::ContentDigest {
    crate::models::twin_event::ContentDigest::parse(hex.to_string().repeat(64)).unwrap()
}

fn session_named(title: &str) -> SessionCreate {
    SessionCreate {
        title: title.into(),
        description: None,
        tags: Vec::new(),
    }
}

fn try_symlink_directory(original: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link).unwrap();
        true
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_dir(original, link) {
            Ok(()) => true,
            Err(error) if error.raw_os_error() == Some(1314) => false,
            Err(error) => panic!("failed to create directory symlink: {error}"),
        }
    }
}

#[test]
fn scoped_canvas_roots_are_isolated_and_retargeting_reloads_each_scope() {
    let temp = tempdir().unwrap();
    let root_a = scoped_canvas_path(temp.path(), &canvas_scope('a'));
    let root_b = scoped_canvas_path(temp.path(), &canvas_scope('b'));
    assert_eq!(root_a, temp.path().join("canvas/v1").join("a".repeat(64)));
    let mut store = CanvasStore::new(root_a.clone());
    let session_a = store.create_session(session_named("Vault A")).unwrap();
    let mut peer_b = CanvasStore::new(root_b.clone());
    peer_b.create_session(session_named("Vault B")).unwrap();
    store.list_sessions().unwrap();
    store.get_session_mut(&session_a.id).unwrap();

    store.replace_root_path(root_b).unwrap();

    assert!(store.session_cache.is_empty());
    assert!(store.pending_bases.is_empty());
    assert!(!store.list_cache_ready);
    assert_eq!(store.list_sessions().unwrap()[0].title, "Vault B");
    store.replace_root_path(root_a).unwrap();
    assert_eq!(store.list_sessions().unwrap()[0].title, "Vault A");
    let empty_root = scoped_canvas_path(temp.path(), &canvas_scope('f'));
    store.replace_root_path(empty_root.clone()).unwrap();
    assert!(empty_root.is_dir());
    assert!(store.list_sessions().unwrap().is_empty());
}

#[test]
fn invalid_or_symlink_retarget_preserves_the_current_root_and_cache() {
    let temp = tempdir().unwrap();
    let root = scoped_canvas_path(temp.path(), &canvas_scope('c'));
    let mut store = CanvasStore::new(root.clone());
    let session = store
        .create_session(session_named("Cached current vault"))
        .unwrap();
    store.list_sessions().unwrap();
    let root_capability = store.root_capability.as_ref().unwrap().clone();
    let invalid = temp.path().join("not-a-directory");
    std::fs::write(&invalid, b"file").unwrap();

    assert!(store.replace_root_path(invalid).is_err());
    assert_eq!(store.data_path, root);
    assert!(Arc::ptr_eq(
        store.root_capability.as_ref().unwrap(),
        &root_capability
    ));
    assert_eq!(
        store.get_session(&session.id).unwrap().title,
        "Cached current vault"
    );

    let real = temp.path().join("real-directory");
    let symlink = temp.path().join("directory-symlink");
    std::fs::create_dir(&real).unwrap();
    if try_symlink_directory(&real, &symlink) {
        assert!(store.replace_root_path(symlink).is_err());
        assert_eq!(store.data_path, root);
        assert_eq!(
            store.get_session(&session.id).unwrap().title,
            "Cached current vault"
        );
    }
}

#[test]
fn retarget_preserves_the_event_recorder_for_governed_writes() {
    let temp = tempdir().unwrap();
    let root_a = scoped_canvas_path(temp.path(), &canvas_scope('d'));
    let root_b = scoped_canvas_path(temp.path(), &canvas_scope('e'));
    let session_b = CanvasStore::new(root_b.clone())
        .create_session(session_named("Delete through recorder"))
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut store =
        CanvasStore::with_event_recorder(root_a, Arc::new(CountingCanvasRecorder(calls.clone())));

    store.replace_root_path(root_b).unwrap();
    store.delete_session(&session_b.id).unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn session_writes_are_atomic_with_no_tmp_litter() {
    let temp_dir = tempdir().expect("temp dir should be created");
    let mut store = CanvasStore::new(temp_dir.path().to_path_buf());

    let session = store
        .create_session(SessionCreate {
            title: "Atomic Session".to_string(),
            description: Some("adoption test".to_string()),
            tags: Vec::new(),
        })
        .expect("session should be created");

    let session_file = temp_dir.path().join(format!("{}.json", session.id));
    let persisted = std::fs::read_to_string(&session_file).expect("session file should exist");
    assert!(persisted.contains("Atomic Session"));
    assert_no_tmp_siblings(temp_dir.path());
}

#[test]
fn canvas_rejects_noncanonical_twin_evidence_before_persisting() {
    let temp_dir = tempdir().unwrap();
    let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
    let session = store
        .create_session(SessionCreate {
            title: "Evidence validation".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    let mut tile = PromptTile::default();
    tile.twin_evidence_snapshot = Some(TwinEvidenceSnapshot {
        projection_snapshot_id: crate::models::twin_state::SnapshotId::parse("a".repeat(64))
            .unwrap(),
        reference_time: Utc::now(),
        prompt_context_digest: crate::models::twin_event::ContentDigest::parse("d".repeat(64))
            .unwrap(),
        prompt_context_version: None,
        twin_relationship_variant: crate::models::twin_state::RelationshipVariant::global(),
        evidence_event_ids: Vec::new(),
        note_ids: vec!["note-b".into(), "note-a".into()],
    });

    let error = store.add_tile(&session.id, tile).unwrap_err();

    assert!(error.to_string().contains("sorted and unique"));
    store.reload_authoritative_state();
    assert!(store
        .get_session(&session.id)
        .unwrap()
        .prompt_tiles
        .is_empty());
}

#[test]
fn canvas_rejects_mismatched_tile_and_evidence_relationship_context() {
    let temp_dir = tempdir().unwrap();
    let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
    let session = store
        .create_session(SessionCreate {
            title: "Relationship validation".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    let alex = crate::models::twin_state::RelationshipVariant::new(vec![
        crate::models::twin_state::RelationshipKey {
            subject_id: crate::models::twin_event::EntityId::parse("owner").unwrap(),
            predicate: crate::models::twin_event::RelationshipPredicate::parse("works_with")
                .unwrap(),
            object_id: crate::models::twin_event::EntityId::parse("alex").unwrap(),
            direction: crate::models::twin_event::RelationshipDirection::Directed,
        },
    ]);
    let tile = PromptTile {
        twin_relationship_variant: alex,
        twin_evidence_snapshot: Some(TwinEvidenceSnapshot {
            projection_snapshot_id: crate::models::twin_state::SnapshotId::parse("a".repeat(64))
                .unwrap(),
            reference_time: Utc::now(),
            prompt_context_digest: crate::models::twin_event::ContentDigest::parse("d".repeat(64))
                .unwrap(),
            prompt_context_version: None,
            twin_relationship_variant: crate::models::twin_state::RelationshipVariant::global(),
            evidence_event_ids: Vec::new(),
            note_ids: Vec::new(),
        }),
        ..PromptTile::default()
    };

    let error = store.add_tile(&session.id, tile).unwrap_err();

    assert!(error.to_string().contains("relationship context"));
    store.reload_authoritative_state();
    assert!(store
        .get_session(&session.id)
        .unwrap()
        .prompt_tiles
        .is_empty());
}

#[test]
fn canvas_session_load_rejects_both_context_version_relationship_mismatches() {
    let alex = crate::models::twin_state::RelationshipVariant::new(vec![
        crate::models::twin_state::RelationshipKey {
            subject_id: crate::models::twin_event::EntityId::parse("owner").unwrap(),
            predicate: crate::models::twin_event::RelationshipPredicate::parse("works_with")
                .unwrap(),
            object_id: crate::models::twin_event::EntityId::parse("alex").unwrap(),
            direction: crate::models::twin_event::RelationshipDirection::Directed,
        },
    ]);
    for (variant, context_version) in [
        (
            crate::models::twin_state::RelationshipVariant::global(),
            "relationship-v4-reviewed-projection-history",
        ),
        (alex, "global-v4-reviewed-projection-history"),
    ] {
        let temp_dir = tempdir().unwrap();
        let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
        let mut session = store
            .create_session(SessionCreate {
                title: "Invalid persisted context version".into(),
                description: None,
                tags: Vec::new(),
            })
            .unwrap();
        session.prompt_tiles.push(PromptTile {
            twin_relationship_variant: variant.clone(),
            twin_evidence_snapshot: Some(TwinEvidenceSnapshot {
                projection_snapshot_id: crate::models::twin_state::SnapshotId::parse(
                    "a".repeat(64),
                )
                .unwrap(),
                reference_time: Utc::now(),
                prompt_context_digest: crate::models::twin_event::ContentDigest::parse(
                    "d".repeat(64),
                )
                .unwrap(),
                prompt_context_version: Some(context_version.into()),
                twin_relationship_variant: variant,
                evidence_event_ids: Vec::new(),
                note_ids: Vec::new(),
            }),
            ..PromptTile::default()
        });
        let path = temp_dir.path().join(format!("{}.json", session.id));
        std::fs::write(&path, serde_json::to_vec_pretty(&session).unwrap()).unwrap();
        store.reload_authoritative_state();

        let error = store.get_session(&session.id).unwrap_err();

        assert!(format!("{error:#}").contains("context version"));
    }
}

#[test]
fn canvas_round_trips_twin_evidence_for_replay() {
    let temp_dir = tempdir().unwrap();
    let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
    let session = store
        .create_session(SessionCreate {
            title: "Reproducible evidence".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    let expected = TwinEvidenceSnapshot {
        projection_snapshot_id: crate::models::twin_state::SnapshotId::parse("a".repeat(64))
            .unwrap(),
        reference_time: Utc::now(),
        prompt_context_digest: crate::models::twin_event::ContentDigest::parse("d".repeat(64))
            .unwrap(),
        prompt_context_version: None,
        twin_relationship_variant: crate::models::twin_state::RelationshipVariant::global(),
        evidence_event_ids: vec![
            crate::models::twin_event::EventId::parse("b".repeat(64)).unwrap(),
            crate::models::twin_event::EventId::parse("c".repeat(64)).unwrap(),
        ],
        note_ids: vec!["notes/project/a".into()],
    };
    let tile = PromptTile {
        twin_evidence_snapshot: Some(expected.clone()),
        ..PromptTile::default()
    };
    store.add_tile(&session.id, tile).unwrap();

    store.reload_authoritative_state();
    let reloaded = store.get_session(&session.id).unwrap();
    assert_eq!(
        reloaded.prompt_tiles[0].twin_evidence_snapshot.as_ref(),
        Some(&expected)
    );
}

#[test]
fn authoritative_reload_discards_a_session_cached_before_a_peer_write() {
    let temp_dir = tempdir().expect("temp dir should be created");
    let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
    let session = store
        .create_session(SessionCreate {
            title: "Before".to_string(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    assert_eq!(store.get_session(&session.id).unwrap().title, "Before");

    let mut peer = CanvasStore::new(temp_dir.path().to_path_buf());
    peer.update_session(
        &session.id,
        crate::models::canvas::SessionUpdate {
            title: Some("After".to_string()),
            ..Default::default()
        },
    )
    .unwrap();

    store.reload_authoritative_state();
    assert_eq!(store.get_session(&session.id).unwrap().title, "After");
}

#[test]
fn pure_canvas_json_commit_does_not_advance_authority() {
    let root = tempdir().unwrap();
    let data = root.path().join("data");
    let vault = root.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store,
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut store = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
    let mut session = store
        .create_session(SessionCreate {
            title: "Canvas authority invariant".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    session.viewport.x = 42.0;

    let commit = store
        .save_session_expecting_authority(&session, expected.clone())
        .unwrap();

    assert!(commit.authority_token.is_none());
    assert_eq!(coordinator.current_authority_token().unwrap(), expected);
}

#[test]
fn canvas_delete_rejects_an_authority_bearing_commit() {
    let root = tempdir().unwrap();
    let canvas = root.path().join("canvas");
    let session = CanvasStore::new(canvas.clone())
        .create_session(SessionCreate {
            title: "Reject unexpected authority".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    let recorder = AuthorityBearingCanvasRecorder {
        authority_token: crate::services::vault_namespace::VaultAuthorityTokenV1 {
            root_scope: crate::services::twin_events::digest_bytes(b"canvas-test-scope"),
            lease_epoch_uuid: "00000000-0000-4000-8000-000000000001".into(),
            authority_generation: 1,
        },
    };
    let mut store = CanvasStore::with_event_recorder(canvas.clone(), Arc::new(recorder));

    let error = store.delete_session(&session.id).unwrap_err();

    assert!(error
        .to_string()
        .contains("Canvas-only mutation unexpectedly advanced content authority"));
    assert!(canvas.join(format!("{}.json", session.id)).exists());
}

fn build_response(model_id: &str) -> ModelResponse {
    ModelResponse {
        model_id: model_id.to_string(),
        model_name: model_id.to_string(),
        status: ResponseStatus::Completed,
        ..ModelResponse::default()
    }
}

fn build_tile(
    id: &str,
    parent_tile_id: Option<&str>,
    parent_model_id: Option<&str>,
    model_ids: &[&str],
) -> PromptTile {
    let mut tile = PromptTile {
        id: id.to_string(),
        parent_tile_id: parent_tile_id.map(str::to_string),
        parent_model_id: parent_model_id.map(str::to_string),
        ..PromptTile::default()
    };

    tile.models = model_ids
        .iter()
        .map(|model_id| model_id.to_string())
        .collect();
    for model_id in model_ids {
        tile.responses
            .insert((*model_id).to_string(), build_response(model_id));
    }

    tile
}

fn build_debate(id: &str, source_tile_ids: &[&str], participating_models: &[&str]) -> Debate {
    Debate {
        id: id.to_string(),
        source_tile_ids: source_tile_ids
            .iter()
            .map(|tile_id| tile_id.to_string())
            .collect(),
        participating_models: participating_models
            .iter()
            .map(|model_id| model_id.to_string())
            .collect(),
        ..Debate::default()
    }
}

#[test]
fn delete_tile_removes_all_descendants() {
    let temp_dir = tempdir().expect("temp dir should be created");
    let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
    let session = store
        .create_session(SessionCreate {
            title: "Tree".to_string(),
            description: None,
            tags: Vec::new(),
        })
        .expect("session should be created");

    let root = PromptTile {
        id: "root".to_string(),
        ..PromptTile::default()
    };
    let child = PromptTile {
        id: "child".to_string(),
        parent_tile_id: Some("root".to_string()),
        ..PromptTile::default()
    };
    let grandchild = PromptTile {
        id: "grandchild".to_string(),
        parent_tile_id: Some("child".to_string()),
        ..PromptTile::default()
    };
    let unrelated = PromptTile {
        id: "unrelated".to_string(),
        ..PromptTile::default()
    };

    store
        .add_tile(&session.id, root)
        .expect("root should be added");
    store
        .add_tile(&session.id, child)
        .expect("child should be added");
    store
        .add_tile(&session.id, grandchild)
        .expect("grandchild should be added");
    store
        .add_tile(&session.id, unrelated)
        .expect("unrelated should be added");

    store
        .delete_tile(&session.id, "root")
        .expect("delete should succeed");

    let remaining_ids: Vec<String> = store
        .get_session(&session.id)
        .expect("session should still load")
        .prompt_tiles
        .into_iter()
        .map(|tile| tile.id)
        .collect();

    assert_eq!(remaining_ids, vec!["unrelated".to_string()]);
}

#[test]
fn delete_tile_removes_debates_for_removed_subtree() {
    let temp_dir = tempdir().expect("temp dir should be created");
    let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
    let session = store
        .create_session(SessionCreate {
            title: "Debates".to_string(),
            description: None,
            tags: Vec::new(),
        })
        .expect("session should be created");

    store
        .add_tile(&session.id, build_tile("root", None, None, &["model-a"]))
        .expect("root should be added");
    store
        .add_tile(
            &session.id,
            build_tile("child", Some("root"), Some("model-a"), &["model-a"]),
        )
        .expect("child should be added");
    store
        .add_tile(&session.id, build_tile("sibling", None, None, &["model-b"]))
        .expect("sibling should be added");
    store
        .add_debate(
            &session.id,
            build_debate("debate-root", &["child"], &["model-a"]),
        )
        .expect("root debate should be added");
    store
        .add_debate(
            &session.id,
            build_debate("debate-sibling", &["sibling"], &["model-b"]),
        )
        .expect("sibling debate should be added");

    store
        .delete_tile(&session.id, "root")
        .expect("delete should succeed");

    let session = store
        .get_session(&session.id)
        .expect("session should still load");
    let remaining_tile_ids: Vec<String> = session
        .prompt_tiles
        .into_iter()
        .map(|tile| tile.id)
        .collect();
    let remaining_debate_ids: Vec<String> = session
        .debates
        .into_iter()
        .map(|debate| debate.id)
        .collect();

    assert_eq!(remaining_tile_ids, vec!["sibling".to_string()]);
    assert_eq!(remaining_debate_ids, vec!["debate-sibling".to_string()]);
}

#[test]
fn delete_response_removes_only_the_deleted_model_branch() {
    let temp_dir = tempdir().expect("temp dir should be created");
    let mut store = CanvasStore::new(temp_dir.path().to_path_buf());
    let session = store
        .create_session(SessionCreate {
            title: "Responses".to_string(),
            description: None,
            tags: Vec::new(),
        })
        .expect("session should be created");

    store
        .add_tile(
            &session.id,
            build_tile("root", None, None, &["model-a", "model-b"]),
        )
        .expect("root should be added");
    store
        .add_tile(
            &session.id,
            build_tile("branch-a", Some("root"), Some("model-a"), &["model-a"]),
        )
        .expect("branch-a should be added");
    store
        .add_tile(
            &session.id,
            build_tile(
                "branch-a-child",
                Some("branch-a"),
                Some("model-a"),
                &["model-a"],
            ),
        )
        .expect("branch-a-child should be added");
    store
        .add_tile(
            &session.id,
            build_tile("branch-b", Some("root"), Some("model-b"), &["model-b"]),
        )
        .expect("branch-b should be added");
    store
        .add_debate(
            &session.id,
            build_debate("debate-a", &["root"], &["model-a"]),
        )
        .expect("debate-a should be added");
    store
        .add_debate(
            &session.id,
            build_debate("debate-branch-a", &["branch-a"], &["model-a"]),
        )
        .expect("debate-branch-a should be added");
    store
        .add_debate(
            &session.id,
            build_debate("debate-b", &["root"], &["model-b"]),
        )
        .expect("debate-b should be added");

    store
        .delete_response(&session.id, "root", "model-a")
        .expect("delete response should succeed");

    let session = store
        .get_session(&session.id)
        .expect("session should still load");
    let remaining_tile_ids: Vec<String> = session
        .prompt_tiles
        .iter()
        .map(|tile| tile.id.clone())
        .collect();
    let remaining_debate_ids: Vec<String> = session
        .debates
        .iter()
        .map(|debate| debate.id.clone())
        .collect();
    let root = session
        .prompt_tiles
        .iter()
        .find(|tile| tile.id == "root")
        .expect("root tile should remain");

    assert_eq!(
        remaining_tile_ids,
        vec!["root".to_string(), "branch-b".to_string()]
    );
    assert_eq!(root.models, vec!["model-b".to_string()]);
    assert!(!root.responses.contains_key("model-a"));
    assert_eq!(remaining_debate_ids, vec!["debate-b".to_string()]);
}

#[test]
fn coordinated_canvas_captures_persisted_prompt_and_completion_but_not_structure() {
    let root = tempdir().unwrap();
    let data = root.path().join("data");
    let vault = root.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
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
    let mut store = CanvasStore::with_event_recorder(data.join("canvas"), coordinator);
    let session = store
        .create_session(SessionCreate {
            title: "Captured".to_string(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    assert!(event_store.ordered_events().unwrap().is_empty());

    let mut tile = PromptTile {
        id: "tile-captured".to_string(),
        prompt: "What next?".to_string(),
        models: vec!["model-a".to_string()],
        ..PromptTile::default()
    };
    tile.responses.insert(
        "model-a".to_string(),
        ModelResponse {
            id: "response-captured".to_string(),
            model_id: "model-a".to_string(),
            model_name: "Model A".to_string(),
            status: ResponseStatus::Pending,
            ..ModelResponse::default()
        },
    );
    store.add_tile(&session.id, tile).unwrap();
    let events = event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].payload,
        crate::models::twin_event::TwinEventPayload::ConversationTurnRecorded(_)
    ));

    store
        .update_tile_response(
            &session.id,
            "tile-captured",
            "model-a",
            "A durable answer",
            ResponseStatus::Completed,
            None,
            Some(0.01),
        )
        .unwrap();
    let events = event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[1].payload,
        crate::models::twin_event::TwinEventPayload::CanvasResponseRecorded(_)
    ));

    store
        .update_viewport(
            &session.id,
            CanvasViewport {
                x: 12.0,
                y: 24.0,
                zoom: 0.8,
            },
        )
        .unwrap();
    store
        .delete_response(&session.id, "tile-captured", "model-a")
        .unwrap();
    assert_eq!(event_store.ordered_events().unwrap().len(), 2);
}

#[test]
fn decision_submission_recovers_canvas_episode_and_trace_as_one_event_group() {
    let root = tempdir().unwrap();
    let data = root.path().join("data");
    let vault = root.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
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
    let mut canvas = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
    let mut twin = crate::services::twin::TwinStore::with_event_recorder(
        data.join("twin").join("scope-one"),
        data.join("twin"),
        coordinator.clone(),
    );
    let session = canvas
        .create_session(SessionCreate {
            title: "Decision".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    let tile = PromptTile {
        id: "decision-tile".into(),
        prompt_type: crate::models::canvas::PromptType::Decision,
        prompt: "Ship now?".into(),
        decision_episode_id: Some("decision-episode".into()),
        decision_metadata: Some(crate::models::canvas::DecisionPromptMetadata {
            decision: "Ship now?".into(),
            options: vec!["Ship".into(), "Wait".into()],
            stakes: Some("Launch quality".into()),
            initial_leaning: Some("Ship".into()),
            review_date: None,
        }),
        ..PromptTile::default()
    };
    let create = crate::models::twin::DecisionEpisodeCreate {
        id: "decision-episode".into(),
        session_id: session.id.clone(),
        tile_id: tile.id.clone(),
        decision: "Ship now?".into(),
        options: vec!["Ship".into(), "Wait".into()],
        stakes: Some("Launch quality".into()),
        initial_leaning: Some("Ship".into()),
        review_date: None,
        primitive_assessment: Default::default(),
        context_version: Some("test-v1".into()),
    };

    let expected = coordinator.current_authority_token().unwrap();
    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));
    let (_, commit) = canvas
        .add_decision_tile_expecting_authority(&session.id, tile, &mut twin, create, expected)
        .unwrap();
    assert!(commit.postcommit_warning);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(coordinator.recover_pending().unwrap(), 0);

    let captured = event_store.ordered_events().unwrap();
    assert_eq!(captured.len(), 2);
    assert!(matches!(
        captured[0].payload,
        crate::models::twin_event::TwinEventPayload::ConversationTurnRecorded(_)
    ));
    assert!(matches!(
        captured[1].payload,
        crate::models::twin_event::TwinEventPayload::DecisionRecorded(_)
    ));
    assert_eq!(
        captured[1].causal_parents,
        vec![captured[0].event_id.clone()]
    );
    assert_eq!(
        twin.get_decision_episode("decision-episode")
            .unwrap()
            .tile_id,
        "decision-tile"
    );
    assert!(twin
        .get_session_trace(&session.id)
        .unwrap()
        .events
        .iter()
        .any(
            |event| event.event_type == crate::models::twin::TraceEventType::DecisionEpisodeCreated
        ));
}

#[test]
fn visible_decision_response_precedes_sealed_prediction_or_explicit_failure() {
    let root = tempdir().unwrap();
    let data = root.path().join("data");
    let vault = root.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
    let event_store = std::sync::Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
    event_store.initialize().unwrap();
    let coordinator = std::sync::Arc::new(
        crate::services::twin_events::MutationCoordinator::new(
            &data,
            &vault,
            event_store,
            std::sync::Arc::new(crate::services::twin_events::NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let mut canvas = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
    let mut twin = crate::services::twin::TwinStore::with_event_recorder(
        data.join("twin/scope-one"),
        data.join("twin"),
        coordinator.clone(),
    );
    let session = canvas
        .create_session(SessionCreate {
            title: "Sequenced prediction".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();

    for (suffix, prediction_succeeds) in [("sealed", true), ("failed", false)] {
        let tile_id = format!("tile-{suffix}");
        let episode_id = format!("episode-{suffix}");
        let mut tile = PromptTile {
            id: tile_id.clone(),
            prompt_type: crate::models::canvas::PromptType::Decision,
            prompt: "Ship?".into(),
            models: vec!["model-a".into()],
            decision_episode_id: Some(episode_id.clone()),
            ..PromptTile::default()
        };
        tile.responses.insert(
            "model-a".into(),
            ModelResponse {
                id: format!("response-{suffix}"),
                model_id: "model-a".into(),
                model_name: "Model A".into(),
                status: ResponseStatus::Pending,
                ..ModelResponse::default()
            },
        );
        let create = crate::models::twin::DecisionEpisodeCreate {
            id: episode_id.clone(),
            session_id: session.id.clone(),
            tile_id: tile_id.clone(),
            decision: "Ship?".into(),
            options: vec!["Ship".into(), "Wait".into()],
            stakes: None,
            initial_leaning: None,
            review_date: None,
            primitive_assessment: Default::default(),
            context_version: Some("test-v1".into()),
        };
        let expected = coordinator.current_authority_token().unwrap();
        let (_, prompt_commit) = canvas
            .add_decision_tile_expecting_authority(&session.id, tile, &mut twin, create, expected)
            .unwrap();
        let prompt_epoch = prompt_commit.authority_token.unwrap();
        let response_commit = canvas
            .batch_update_tile_responses_expecting_authority(
                &session.id,
                &tile_id,
                &[(
                    "model-a".into(),
                    format!("visible-{suffix}"),
                    ResponseStatus::Completed,
                    None,
                    None,
                )],
                prompt_epoch,
            )
            .unwrap();
        let response_epoch = response_commit.authority_token.unwrap();

        if prediction_succeeds {
            let (_, commit) = twin
                .attach_twin_prediction_expecting_authority(
                    &episode_id,
                    crate::models::twin::TwinPredictionDraft {
                        predicted_option: "Ship".into(),
                        matched_option_index: Some(0),
                        confidence: Some(0.8),
                        rationale: Some("evidence".into()),
                        parse_mode: "strict_json".into(),
                    },
                    "model-a",
                    "test-v1",
                    response_epoch,
                )
                .unwrap();
            assert!(commit.authority_token.is_some());
        } else {
            let commit = twin
                .mark_twin_prediction_failed_expecting_authority(&episode_id, response_epoch)
                .unwrap();
            assert!(commit.authority_token.is_some());
        }

        let durable_session = canvas.get_session(&session.id).unwrap();
        assert_eq!(
            durable_session
                .prompt_tiles
                .iter()
                .find(|tile| tile.id == tile_id)
                .unwrap()
                .responses["model-a"]
                .content,
            format!("visible-{suffix}")
        );
        let episode = twin.get_decision_episode(&episode_id).unwrap();
        if prediction_succeeds {
            assert!(episode.twin_prediction.is_some());
        } else {
            assert_eq!(episode.prediction_status.as_deref(), Some("failed"));
            assert!(episode.twin_prediction.is_none());
            let after_failure = coordinator.current_authority_token().unwrap();
            let duplicate = twin
                .mark_twin_prediction_failed_expecting_authority(&episode_id, after_failure.clone())
                .unwrap();
            assert!(duplicate.authority_token.is_none());
            assert_eq!(
                coordinator.current_authority_token().unwrap(),
                after_failure
            );
        }
    }
}

#[test]
fn coordinated_canvas_failure_restores_cache_and_persisted_session() {
    let root = tempdir().unwrap();
    let canvas = root.path().join("canvas");
    let session = CanvasStore::new(canvas.clone())
        .create_session(SessionCreate {
            title: "Stable".to_string(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    let mut store = CanvasStore::with_event_recorder(
        canvas.clone(),
        std::sync::Arc::new(crate::services::twin_events::UnavailableEventRecorder::new(
            "injected persistence failure",
        )),
    );
    store.get_session(&session.id).unwrap();
    let before = std::fs::read(canvas.join(format!("{}.json", session.id))).unwrap();
    let result = store.add_tile(
        &session.id,
        PromptTile {
            id: "must-not-stick".to_string(),
            prompt: "Do not persist".to_string(),
            ..PromptTile::default()
        },
    );
    assert!(result.is_err());
    assert_eq!(
        std::fs::read(canvas.join(format!("{}.json", session.id))).unwrap(),
        before
    );
    assert!(store
        .get_session(&session.id)
        .unwrap()
        .prompt_tiles
        .is_empty());
}

#[test]
fn interrupted_canvas_target_keeps_cache_equal_to_durable_bytes_until_recovery() {
    let root = tempdir().unwrap();
    let data = root.path().join("data");
    let vault = root.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
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
    let mut store = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
    let session = store
        .create_session(SessionCreate {
            title: "Interrupted".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));
    let (_, commit) = store
        .add_tile_expecting_authority(
            &session.id,
            PromptTile {
                id: "durable-before-event".into(),
                prompt: "Persist me once".into(),
                ..PromptTile::default()
            },
            expected,
        )
        .unwrap();
    assert!(commit.postcommit_warning);

    let path = data.join("canvas").join(format!("{}.json", session.id));
    let durable: CanvasSession = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let cached = store.get_session(&session.id).unwrap();
    assert_eq!(cached.prompt_tiles.len(), durable.prompt_tiles.len());
    assert_eq!(cached.prompt_tiles[0].id, durable.prompt_tiles[0].id);
    assert_eq!(event_store.ordered_events().unwrap().len(), 1);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(coordinator.recover_pending().unwrap(), 0);
    assert_eq!(event_store.ordered_events().unwrap().len(), 1);
}

#[test]
fn target_only_canvas_write_uses_journal_and_recovers_without_event() {
    let root = tempdir().unwrap();
    let data = root.path().join("data");
    let vault = root.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
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
    let mut store = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
    let session = store
        .create_session(SessionCreate {
            title: "Layout journal".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();

    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));
    store
        .update_viewport(
            &session.id,
            CanvasViewport {
                x: 7.0,
                y: 9.0,
                zoom: 0.75,
            },
        )
        .unwrap();
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    assert_eq!(coordinator.recover_pending().unwrap(), 0);
    assert!(event_store.ordered_events().unwrap().is_empty());
    let durable = store.get_session(&session.id).unwrap();
    assert_eq!(durable.viewport.x, 7.0);
    assert_eq!(durable.viewport.y, 9.0);
}

#[test]
fn pending_canvas_change_is_recovered_before_fresh_session_plan() {
    let root = tempdir().unwrap();
    let data = root.path().join("data");
    let vault = root.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
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
    let mut first = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
    let session = first
        .create_session(SessionCreate {
            title: "Fresh plan".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    let mut stale = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
    stale.get_session(&session.id).unwrap();

    let expected = coordinator.current_authority_token().unwrap();
    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterStage);
    let (_, commit) = first
        .add_tile_expecting_authority(
            &session.id,
            PromptTile {
                id: "recovered-tile".into(),
                prompt: "first".into(),
                ..PromptTile::default()
            },
            expected,
        )
        .unwrap();
    assert!(commit.postcommit_warning);
    assert_eq!(coordinator.pending_count().unwrap(), 0);
    stale
        .add_tile(
            &session.id,
            PromptTile {
                id: "fresh-tile".into(),
                prompt: "second".into(),
                ..PromptTile::default()
            },
        )
        .unwrap();
    let durable = stale.get_session(&session.id).unwrap();
    let ids = durable
        .prompt_tiles
        .iter()
        .map(|tile| tile.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["recovered-tile", "fresh-tile"]);
    assert_eq!(event_store.ordered_events().unwrap().len(), 2);
}

#[test]
fn immediate_regeneration_supersedes_event_recovered_under_same_lock() {
    let root = tempdir().unwrap();
    let data = root.path().join("data");
    let vault = root.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
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
    let mut store = CanvasStore::with_event_recorder(data.join("canvas"), coordinator.clone());
    let session = store
        .create_session(SessionCreate {
            title: "Regenerate after crash".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    let mut tile = PromptTile {
        id: "tile".into(),
        prompt: "prompt".into(),
        models: vec!["model".into()],
        ..PromptTile::default()
    };
    tile.responses.insert(
        "model".into(),
        ModelResponse {
            id: "stable-response".into(),
            model_id: "model".into(),
            model_name: "Model".into(),
            status: ResponseStatus::Pending,
            ..ModelResponse::default()
        },
    );
    store.add_tile(&session.id, tile).unwrap();
    let expected = coordinator.current_authority_token().unwrap();
    coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterTarget(0));
    let commit = store
        .update_tile_response_expecting_authority(
            &session.id,
            "tile",
            "model",
            "first completion",
            ResponseStatus::Completed,
            None,
            None,
            expected,
        )
        .unwrap();
    assert!(commit.postcommit_warning);
    store
        .update_tile_response(
            &session.id,
            "tile",
            "model",
            "regenerated completion",
            ResponseStatus::Completed,
            None,
            None,
        )
        .unwrap();

    let response_events = event_store
        .ordered_events()
        .unwrap()
        .into_iter()
        .filter(|event| {
            matches!(
                event.payload,
                crate::models::twin_event::TwinEventPayload::CanvasResponseRecorded(_)
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(response_events.len(), 2);
    assert_eq!(
        response_events[1].supersedes,
        vec![response_events[0].event_id.clone()]
    );
    assert_eq!(coordinator.pending_count().unwrap(), 0);
}

#[test]
fn coordinated_canvas_batches_regeneration_and_debate_are_persisted_once_in_order() {
    let root = tempdir().unwrap();
    let data = root.path().join("data");
    let vault = root.path().join("vault");
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&vault).unwrap();
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
    let mut store = CanvasStore::with_event_recorder(data.join("canvas"), coordinator);
    let session = store
        .create_session(SessionCreate {
            title: "Batch and debate".into(),
            description: None,
            tags: Vec::new(),
        })
        .unwrap();
    let mut tile = PromptTile {
        id: "tile-batch".into(),
        prompt: "Compare both answers".into(),
        models: vec!["model-z".into(), "model-a".into()],
        ..PromptTile::default()
    };
    for (model_id, response_id) in [("model-z", "response-z"), ("model-a", "response-a")] {
        tile.responses.insert(
            model_id.into(),
            ModelResponse {
                id: response_id.into(),
                model_id: model_id.into(),
                model_name: model_id.into(),
                status: ResponseStatus::Pending,
                ..ModelResponse::default()
            },
        );
    }
    store.add_tile(&session.id, tile).unwrap();
    store
        .batch_update_tile_responses(
            &session.id,
            "tile-batch",
            &[
                (
                    "model-z".into(),
                    "z answer".into(),
                    ResponseStatus::Completed,
                    None,
                    Some(0.02),
                ),
                (
                    "model-a".into(),
                    "a answer".into(),
                    ResponseStatus::Completed,
                    None,
                    Some(0.01),
                ),
            ],
        )
        .unwrap();
    let events = event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 3);
    let response_ids = events[1..]
        .iter()
        .map(|event| match &event.payload {
            crate::models::twin_event::TwinEventPayload::CanvasResponseRecorded(payload) => {
                payload.response_id.as_str()
            }
            other => panic!("unexpected batch payload: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(response_ids, vec!["response-a", "response-z"]);

    store
        .update_tile_response(
            &session.id,
            "tile-batch",
            "model-a",
            "partial",
            ResponseStatus::Streaming,
            None,
            None,
        )
        .unwrap();
    assert_eq!(event_store.ordered_events().unwrap().len(), 3);
    let prior_a = events[1].event_id.clone();
    store
        .update_tile_response(
            &session.id,
            "tile-batch",
            "model-a",
            "regenerated answer",
            ResponseStatus::Completed,
            None,
            Some(0.03),
        )
        .unwrap();
    let events = event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(events[3].supersedes, vec![prior_a]);

    let mut debate = Debate {
        id: "debate-stable".into(),
        participating_models: vec!["model-z".into(), "model-a".into()],
        ..Debate::default()
    };
    store.add_debate(&session.id, debate.clone()).unwrap();
    assert_eq!(event_store.ordered_events().unwrap().len(), 4);
    debate.rounds.push(DebateRound {
        round_number: 1,
        topic: "Which answer is stronger?".into(),
        responses: vec![
            DebateResponse {
                model_id: "model-z".into(),
                model_name: "Model Z".into(),
                content: "Z case".into(),
                stance: None,
                cost_usd: Some(0.02),
                provider: Some("openrouter".into()),
                provenance: Some("canvas_debate_openrouter".into()),
            },
            DebateResponse {
                model_id: "model-a".into(),
                model_name: "Model A".into(),
                content: "A case".into(),
                stance: None,
                cost_usd: Some(0.01),
                provider: Some("openrouter".into()),
                provenance: Some("canvas_debate_openrouter".into()),
            },
        ],
        created_at: Utc::now(),
    });
    store.update_debate(&session.id, &debate).unwrap();
    let events = event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 7);
    assert!(matches!(
        events[4].payload,
        crate::models::twin_event::TwinEventPayload::ConversationTurnRecorded(_)
    ));
    let debate_rows = events[5..]
        .iter()
        .map(|event| match &event.payload {
            crate::models::twin_event::TwinEventPayload::CanvasResponseRecorded(payload) => {
                (payload.response_id.as_str(), payload.model_id.as_str())
            }
            other => panic!("unexpected debate payload: {other:?}"),
        })
        .collect::<Vec<_>>();
    let mut sorted_ids = debate_rows.iter().map(|row| row.0).collect::<Vec<_>>();
    sorted_ids.sort_unstable();
    assert_eq!(
        debate_rows.iter().map(|row| row.0).collect::<Vec<_>>(),
        sorted_ids
    );
    let mut debate_models = debate_rows.iter().map(|row| row.1).collect::<Vec<_>>();
    debate_models.sort_unstable();
    assert_eq!(debate_models, vec!["model-a", "model-z"]);
}
