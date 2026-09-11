use super::*;

fn snapshot_tree(root: &std::path::Path) -> Vec<(String, Option<Vec<u8>>)> {
    let mut snapshot = walkdir::WalkDir::new(root)
        .min_depth(1)
        .into_iter()
        .map(|entry| {
            let entry = entry.unwrap();
            let relative = entry
                .path()
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let contents = entry.file_type().is_file().then(|| {
                std::fs::read(entry.path())
                    .unwrap_or_else(|error| format!("unreadable:{:?}", error.kind()).into_bytes())
            });
            (relative, contents)
        })
        .collect::<Vec<_>>();
    snapshot.sort_by(|left, right| left.0.cmp(&right.0));
    snapshot
}

fn stable_boot_settings(
    temp: &tempfile::TempDir,
    vault_path: &std::path::Path,
) -> (SettingsService, crate::models::twin_event::ContentDigest) {
    let data_path = temp.path().join("data");
    std::fs::create_dir(&data_path).unwrap();
    std::fs::create_dir(vault_path).unwrap();
    let identity =
        crate::services::sync::identity::load_or_create_vault_identity(vault_path).unwrap();
    let durable_settings = crate::models::settings::UserSettings {
        vault_path: Some(vault_path.to_string_lossy().into_owned()),
        ..crate::models::settings::UserSettings::default()
    };
    let settings =
        SettingsService::for_test(temp.path().join("settings.json"), durable_settings.clone());
    let transition_store = settings.root_transition_store().unwrap();
    transition_store
        .write_settings_guarded(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(durable_settings),
        )
        .unwrap();
    transition_store
        .write_lease(
            &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                identity.root_scope.clone(),
            ),
        )
        .unwrap();
    (settings, identity.root_scope)
}

#[test]
fn provisioned_desktop_startup_promotes_local_markdown_to_the_sync_outbox() {
    let temp = tempfile::tempdir().unwrap();
    let vault_path = temp.path().join("vault");
    std::fs::create_dir(&vault_path).unwrap();
    let identity =
        crate::services::sync::identity::load_or_create_vault_identity(&vault_path).unwrap();
    let root_scope = identity.root_scope.clone();
    let durable_settings = crate::models::settings::UserSettings {
        vault_path: Some(vault_path.to_string_lossy().into_owned()),
        ..crate::models::settings::UserSettings::default()
    };
    let settings =
        SettingsService::for_test(temp.path().join("settings.json"), durable_settings.clone());
    settings
        .root_transition_store()
        .unwrap()
        .write_settings_guarded(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(durable_settings),
        )
        .unwrap();
    crate::services::sync::vault_keys::provision_vault_root_key(
        settings.secret_store().as_ref(),
        &identity.descriptor.vault_id().to_string(),
        &grafyn_sync_protocol::VaultRootKey::from_bytes([0x51; 32]),
    )
    .unwrap();
    std::fs::write(vault_path.join("startup-sync.md"), b"startup lifecycle\r\n").unwrap();

    let runtime = initialize_stable_mutation_runtime(
        temp.path().join("data"),
        &vault_path,
        Arc::new(TwinEventStore::new(temp.path().join("data"))),
        settings.secret_store(),
    )
    .unwrap();
    let knowledge_store = KnowledgeStore::with_event_recorder(
        vault_path.clone(),
        temp.path().join("derived"),
        runtime.coordinator.clone(),
    );

    bootstrap_sync_engine_before_service(
        runtime.sync_engine.as_ref(),
        Some(&runtime.coordinator),
        &knowledge_store,
    )
    .unwrap();

    assert_eq!(
        runtime
            .sync_engine
            .as_ref()
            .unwrap()
            .status()
            .unwrap()
            .outbox_operations,
        1
    );
    assert_eq!(
        std::fs::read(vault_path.join("startup-sync.md")).unwrap(),
        b"startup lifecycle\r\n"
    );
    assert_eq!(
        runtime.coordinator.current_root_epoch().unwrap().root_scope,
        root_scope
    );
}

#[tokio::test]
async fn provisioned_companion_capture_seals_inherited_group_and_excludes_local_only_group() {
    use crate::commands::twin_state::{
        create_companion_capture_inner, CompanionCaptureContextInput, CompanionCaptureKind,
        CompanionSyncPolicy, CreateCompanionCaptureRequest,
    };
    use crate::models::twin_event::CausalStream;
    use chrono::TimeZone;

    for (policy, expected_outbox, expected_stream) in [
        (CompanionSyncPolicy::Inherit, 3, CausalStream::SyncEligible),
        (CompanionSyncPolicy::LocalOnly, 0, CausalStream::LocalOnly),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let vault_path = temp.path().join("vault");
        let (settings, _) = stable_boot_settings(&temp, &vault_path);
        crate::services::twin_events::PersistedMutationIdentityProvider::load_or_create(
            temp.path().join("data"),
        )
        .unwrap();
        let identity =
            crate::services::sync::identity::load_or_create_vault_identity(&vault_path).unwrap();
        crate::services::sync::vault_keys::provision_vault_root_key(
            settings.secret_store().as_ref(),
            &identity.descriptor.vault_id().to_string(),
            &grafyn_sync_protocol::VaultRootKey::from_bytes([0x61; 32]),
        )
        .unwrap();
        let state = build_app_state(settings, None).unwrap();
        let sync_engine = state.sync_engine.as_ref().unwrap_or_else(|| {
            panic!(
                "provisioned test runtime failed: {:?}",
                state.mutation_startup_error.try_read().unwrap().as_deref()
            )
        });
        assert_eq!(sync_engine.status().unwrap().outbox_operations, 0);

        create_companion_capture_inner(
            &state,
            CreateCompanionCaptureRequest {
                content: "Private or inherited thought".into(),
                capture_kind: CompanionCaptureKind::Text,
                context: CompanionCaptureContextInput::default(),
                attachment_digests: Vec::new(),
                grafyn_sync: policy,
            },
            chrono::Utc.with_ymd_and_hms(2026, 9, 1, 4, 5, 0).unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(
            sync_engine.status().unwrap().outbox_operations,
            expected_outbox
        );
        let events = state.twin_event_store.ordered_events().unwrap();
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .all(|event| event.causal_stream == expected_stream));
    }
}

#[tokio::test]
async fn provisioned_generated_image_save_seals_one_inherited_attachment_group() {
    use crate::commands::image_generation::{
        load_generated_image_inner, save_generated_image_inner,
    };
    use crate::models::image_generation::{
        GeneratedImageSyncPolicy, ImageMetadataRetentionPolicy, LoadGeneratedImageRequest,
        SaveGeneratedImageRequest,
    };
    use crate::models::twin_event::{CausalStream, EvidenceType};
    use crate::services::openrouter::{GeneratedImageReceipt, OpenRouterService};
    use chrono::TimeZone;

    let temp = tempfile::tempdir().unwrap();
    let vault_path = temp.path().join("vault");
    let (settings, _) = stable_boot_settings(&temp, &vault_path);
    crate::services::twin_events::PersistedMutationIdentityProvider::load_or_create(
        temp.path().join("data"),
    )
    .unwrap();
    let identity =
        crate::services::sync::identity::load_or_create_vault_identity(&vault_path).unwrap();
    crate::services::sync::vault_keys::provision_vault_root_key(
        settings.secret_store().as_ref(),
        &identity.descriptor.vault_id().to_string(),
        &grafyn_sync_protocol::VaultRootKey::from_bytes([0x62; 32]),
    )
    .unwrap();
    let state = build_app_state(settings, None).unwrap();
    let sync_engine = state.sync_engine.as_ref().unwrap();
    assert_eq!(sync_engine.status().unwrap().outbox_operations, 0);
    let image = image::DynamicImage::new_rgba8(2, 2);
    let mut encoded = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let bytes = encoded.into_inner();
    let service = OpenRouterService::new(String::new());
    let receipt_id = service
        .insert_generated_image_for_tests(GeneratedImageReceipt {
            bytes: bytes.clone(),
            media_type: "image/png".into(),
            width: 2,
            height: 2,
            prompt: "Inherited companion image".into(),
            model_id: "author/model".into(),
            resolution: "1024x1024".into(),
            aspect_ratio: "1:1".into(),
            vault_scope: state
                .mutation_coordinator
                .as_ref()
                .unwrap()
                .current_authority_token()
                .unwrap()
                .root_scope,
            gateway: "openrouter".into(),
            provider_tag: "test-provider".into(),
        })
        .unwrap();
    *state.openrouter.write().await = service;

    let saved = save_generated_image_inner(
        &state,
        SaveGeneratedImageRequest {
            receipt_id,
            annotation: Some("Inherited image evidence".into()),
            retention_policy: ImageMetadataRetentionPolicy::RetainOriginal,
            grafyn_sync: GeneratedImageSyncPolicy::Inherit,
        },
        chrono::Utc.with_ymd_and_hms(2026, 9, 1, 9, 0, 0).unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(sync_engine.status().unwrap().outbox_operations, 5);
    assert_eq!(
        serde_json::to_value(saved.sync_disposition).unwrap(),
        serde_json::json!({
            "status": "queued",
            "manifestCount": 1,
            "chunkCount": 1,
            "operationCount": 2
        })
    );
    let events = state.twin_event_store.ordered_events().unwrap();
    assert_eq!(events.len(), 2);
    assert!(events
        .iter()
        .all(|event| event.causal_stream == CausalStream::SyncEligible));
    assert_eq!(
        events
            .iter()
            .flat_map(|event| &event.evidence)
            .filter(|evidence| evidence.evidence_type == EvidenceType::Attachment)
            .count(),
        1
    );
    let loaded = load_generated_image_inner(
        &state,
        LoadGeneratedImageRequest {
            attachment_digest: saved.attachment_digest,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            loaded.base64_data
        )
        .unwrap(),
        bytes
    );
}

fn assert_recoverable_detached_validation_state(
    state: &AppState,
    active_data_path: &std::path::Path,
    source_error: &str,
) {
    assert!(state.mutation_coordinator.is_none());
    let startup_error = state
        .mutation_startup_error
        .try_read()
        .unwrap()
        .clone()
        .expect("detached validation must keep authoritative commands unavailable");
    assert!(startup_error.contains(source_error));
    assert!(startup_error.contains("reattach"));
    let boot = state.boot_state.try_read().unwrap().clone();
    assert_eq!(boot.phase, "failed");
    assert!(!boot.ready);
    assert!(boot.error.as_deref().unwrap().contains(source_error));
    assert!(boot.error.as_deref().unwrap().contains("reattach"));
    let recovery_root = state
        ._recovery_runtime
        .as_ref()
        .expect("recoverable boot must retain isolated service roots")
        .path();
    assert!(state
        .twin_event_store
        .data_path()
        .starts_with(recovery_root));
    assert!(!state
        .twin_event_store
        .data_path()
        .starts_with(active_data_path));
    assert!(state
        .knowledge_store
        .try_read()
        .unwrap()
        .vault_path()
        .starts_with(recovery_root));
}

#[test]
fn descriptor_mismatch_constructs_failed_app_state_on_isolated_roots() {
    let temp = tempfile::tempdir().unwrap();
    let vault_path = temp.path().join("configured-vault");
    let (settings, expected_scope) = stable_boot_settings(&temp, &vault_path);
    let replacement_vault = temp.path().join("replacement-vault");
    std::fs::create_dir(&replacement_vault).unwrap();
    let replacement =
        crate::services::sync::identity::load_or_create_vault_identity(&replacement_vault).unwrap();
    assert_ne!(replacement.root_scope, expected_scope);
    std::fs::copy(
        replacement_vault.join("_grafyn/vault.json"),
        vault_path.join("_grafyn/vault.json"),
    )
    .unwrap();
    let before = snapshot_tree(temp.path());

    let state = build_app_state(settings, None)
        .expect("descriptor replacement must leave the reattach UI available");

    assert_recoverable_detached_validation_state(
        &state,
        &temp.path().join("data"),
        "configured stable vault descriptor was replaced",
    );
    assert_eq!(snapshot_tree(temp.path()), before);
}

#[test]
fn configured_vault_file_constructs_failed_app_state_on_isolated_roots() {
    let temp = tempfile::tempdir().unwrap();
    let vault_path = temp.path().join("configured-vault");
    let (settings, _scope) = stable_boot_settings(&temp, &vault_path);
    std::fs::remove_dir_all(&vault_path).unwrap();
    std::fs::write(&vault_path, b"not a vault directory").unwrap();
    let before = snapshot_tree(temp.path());

    let state = build_app_state(settings, None)
        .expect("invalid configured path must leave the reattach UI available");

    assert_recoverable_detached_validation_state(
        &state,
        &temp.path().join("data"),
        "configured stable vault path is not a real directory",
    );
    assert_eq!(snapshot_tree(temp.path()), before);
}

#[test]
fn invalid_root_transition_still_aborts_app_state_construction() {
    let temp = tempfile::tempdir().unwrap();
    let vault_path = temp.path().join("configured-vault");
    let (settings, _scope) = stable_boot_settings(&temp, &vault_path);
    std::fs::write(
        temp.path().join("data/twin/events/root-transition-v1.json"),
        b"not a root transition",
    )
    .unwrap();
    let before = snapshot_tree(temp.path());

    let error = match build_app_state(settings, None) {
        Ok(_) => panic!("root-transition recovery errors must remain strict"),
        Err(error) => error,
    };

    assert!(error.contains("invalid root transition"));
    assert_eq!(snapshot_tree(temp.path()), before);
}

#[test]
fn optimizer_worker_failure_preserves_exact_authority_for_repair() {
    let (state, _vault, _data) = crate::commands::commit_note_write_tests::build_test_state();
    let authority = state
        .mutation_coordinator
        .as_ref()
        .unwrap()
        .current_authority_token()
        .unwrap();
    let mutation_id = crate::models::twin_event::ContentDigest::parse("b".repeat(64)).unwrap();
    let error = anyhow::Error::new(
        crate::services::twin_events::MutationError::AuthorityAdvanced {
            mutation_id: mutation_id.clone(),
            authority_token: authority.clone(),
            target_aborted: false,
            reason: "injected optimizer publication failure".into(),
        },
    );

    let commit = optimizer_worker_failure_commit(&error)
        .expect("the worker must retain the direct non-retryable commit");
    assert_eq!(commit.mutation_id, Some(mutation_id));
    assert_eq!(commit.authority_token, Some(authority));
}

fn build_warm_start_state() -> (AppState, tempfile::TempDir, tempfile::TempDir) {
    let (mut state, vault, data) = crate::commands::commit_note_write_tests::build_test_state();
    let discarded = data.path().join("discarded-derived");
    state.search_service = Arc::new(RwLock::new(
        crate::services::search::SearchService::new(discarded.clone()).unwrap(),
    ));
    state.chunk_index = Arc::new(RwLock::new(
        crate::services::chunk_index::ChunkIndex::new(discarded.clone()).unwrap(),
    ));
    state.link_discovery = Some(Arc::new(RwLock::new(
        crate::services::link_discovery::LinkDiscoveryService::new(discarded.clone()),
    )));
    state.markdown_migration = Some(Arc::new(RwLock::new(
        crate::services::markdown_migration::MarkdownMigrationService::new(discarded.clone()),
    )));
    state.vault_optimizer = Some(Arc::new(RwLock::new(
        crate::services::vault_optimizer::VaultOptimizerService::new(discarded),
    )));
    let coordinator = state
        .mutation_coordinator
        .as_ref()
        .expect("test state should include a mutation coordinator")
        .clone();
    let namespace = coordinator.current_namespace_path().unwrap();
    state.knowledge_store = Arc::new(RwLock::new(
        crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
            vault.path().to_path_buf(),
            namespace.clone(),
            coordinator.clone(),
        ),
    ));
    state.search_service = Arc::new(RwLock::new(
        crate::services::search::SearchService::new(namespace.clone()).unwrap(),
    ));
    state.chunk_index = Arc::new(RwLock::new(
        crate::services::chunk_index::ChunkIndex::new(namespace.clone()).unwrap(),
    ));
    state.link_discovery = Some(Arc::new(RwLock::new(
        crate::services::link_discovery::LinkDiscoveryService::try_new(namespace.clone()).unwrap(),
    )));
    state.markdown_migration = Some(Arc::new(RwLock::new(
        crate::services::markdown_migration::MarkdownMigrationService::try_new(namespace.clone())
            .unwrap(),
    )));
    state.vault_optimizer = Some(Arc::new(RwLock::new(
        crate::services::vault_optimizer::VaultOptimizerService::try_new(namespace).unwrap(),
    )));
    state.twin_store = Arc::new(RwLock::new(
        crate::services::twin::TwinStore::with_event_recorder(
            crate::models::settings::twin_data_path_for_vault(data.path(), vault.path()).unwrap(),
            data.path().join("twin"),
            coordinator.clone(),
        ),
    ));
    state.mutation_coordinator = Some(coordinator);
    (state, vault, data)
}

fn build_stable_warm_start_state() -> (AppState, tempfile::TempDir, tempfile::TempDir) {
    let vault = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let identity =
        crate::services::sync::identity::load_or_create_vault_identity(vault.path()).unwrap();
    let transition_store = crate::services::root_transition::RootTransitionStore::new(
        data.path(),
        data.path().join("settings.json"),
        Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    crate::services::twin_events::PersistedMutationIdentityProvider::load_or_create(data.path())
        .unwrap();
    let durable_settings = crate::models::settings::UserSettings {
        vault_path: Some(vault.path().to_string_lossy().into_owned()),
        ..crate::models::settings::UserSettings::default()
    };
    transition_store
        .write_settings(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                durable_settings.clone(),
            ),
        )
        .unwrap();
    transition_store
        .write_lease(
            &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                identity.root_scope,
            ),
        )
        .unwrap();
    transition_store
        .write_key_authority(
            crate::services::root_transition::OpenRouterKeySource::Unset,
            None,
        )
        .unwrap();
    let event_store = Arc::new(TwinEventStore::new(data.path()));
    let coordinator = Arc::new(
        MutationCoordinator::new_stable(
            data.path(),
            vault.path(),
            event_store.clone(),
            Arc::new(NoopMutationLifecycle),
        )
        .unwrap(),
    );
    let namespace = coordinator.current_namespace_path().unwrap();
    let scope = coordinator.current_root_epoch().unwrap().root_scope;
    let twin_path = {
        let guard = coordinator.begin_root_transition().unwrap();
        let lease = guard.current_lease().unwrap();
        guard.prepare_twin_data_path(vault.path(), &lease).unwrap()
    };
    let settings = crate::services::settings::SettingsService::for_test(
        data.path().join("settings.json"),
        durable_settings,
    );
    let state = AppState {
        knowledge_store: Arc::new(RwLock::new(KnowledgeStore::with_event_recorder(
            vault.path().to_path_buf(),
            namespace.clone(),
            coordinator.clone(),
        ))),
        graph_index: Arc::new(RwLock::new(GraphIndex::new())),
        search_service: Arc::new(RwLock::new(SearchService::new(namespace.clone()).unwrap())),
        canvas_store: Arc::new(RwLock::new(CanvasStore::with_event_recorder(
            crate::services::canvas_store::scoped_canvas_path(data.path(), &scope),
            coordinator.clone(),
        ))),
        openrouter: Arc::new(RwLock::new(OpenRouterService::new(String::new()))),
        ollama: Some(Arc::new(RwLock::new(OllamaService::new(String::new())))),
        feedback_service: Arc::new(RwLock::new(FeedbackService::new(
            data.path().join("feedback"),
        ))),
        settings_service: Arc::new(RwLock::new(settings)),
        priority_service: Arc::new(RwLock::new(PriorityScoringService::new(
            data.path().to_path_buf(),
        ))),
        retrieval_service: Arc::new(RwLock::new(RetrievalService::new(
            data.path().to_path_buf(),
        ))),
        chunk_index: Arc::new(RwLock::new(ChunkIndex::new(namespace.clone()).unwrap())),
        link_discovery: Some(Arc::new(RwLock::new(
            LinkDiscoveryService::try_new(namespace.clone()).unwrap(),
        ))),
        markdown_migration: Some(Arc::new(RwLock::new(
            MarkdownMigrationService::try_new(namespace.clone()).unwrap(),
        ))),
        vault_optimizer: Some(Arc::new(RwLock::new(
            VaultOptimizerService::try_new(namespace).unwrap(),
        ))),
        twin_store: Arc::new(RwLock::new(TwinStore::with_event_recorder_scoped(
            twin_path,
            data.path().join("twin"),
            scope,
            coordinator.clone(),
        ))),
        twin_event_store: event_store,
        mutation_coordinator: Some(coordinator),
        sync_engine: None,
        mutation_startup_error: Arc::new(RwLock::new(None)),
        loaded_authority: Arc::new(RwLock::new(None)),
        authority_repair: Arc::new(tokio::sync::Mutex::new(())),
        committed_warning_app: None,
        vault_transition: Arc::new(RwLock::new(())),
        memory_service: Arc::new(MemoryService::new()),
        boot_state: Arc::new(RwLock::new(BootStatus::default())),
        _recovery_runtime: None,
    };
    (state, vault, data)
}

#[tokio::test]
async fn update_boot_state_replaces_existing_status() {
    let boot_state = Arc::new(RwLock::new(BootStatus::default()));
    let next = BootStatus::new("building_search_index", "Building search index");

    update_boot_state(&boot_state, &next).await;

    assert_eq!(*boot_state.read().await, next);
}

#[tokio::test]
async fn warm_start_waits_behind_root_transition_before_reading_services() {
    let (state, _vault, _data) = crate::commands::commit_note_write_tests::build_test_state();
    let transition = state.vault_transition.write().await;
    let task_state = state.clone();
    let task = tokio::spawn(async move { acquire_warm_start_root_gate(&task_state).await });
    tokio::task::yield_now().await;
    assert!(
        !task.is_finished(),
        "warm start must wait behind root transition"
    );
    drop(transition);
    drop(task.await.unwrap().unwrap());
}

#[tokio::test]
async fn peer_root_wal_in_warm_start_gap_fails_before_any_further_data_change() {
    let (state, vault, data) = build_stable_warm_start_state();
    let data_snapshot = Arc::new(std::sync::Mutex::new(None));
    let vault_snapshot = Arc::new(std::sync::Mutex::new(None));
    let data_snapshot_hook = data_snapshot.clone();
    let vault_snapshot_hook = vault_snapshot.clone();
    let data_path = data.path().to_path_buf();
    let vault_path = vault.path().to_path_buf();

    let error = warm_start_services_inner_with_gap(None, &state, None, move || {
        std::fs::write(
            data_path.join("twin/events/root-transition-v1.json"),
            b"peer-prepared",
        )
        .map_err(|error| error.to_string())?;
        *data_snapshot_hook.lock().unwrap() = Some(snapshot_tree(&data_path));
        *vault_snapshot_hook.lock().unwrap() = Some(snapshot_tree(&vault_path));
        Ok(())
    })
    .await
    .unwrap_err();

    assert!(error.contains("root-transition"));
    assert_eq!(
        snapshot_tree(data.path()),
        data_snapshot.lock().unwrap().clone().unwrap()
    );
    assert_eq!(
        snapshot_tree(vault.path()),
        vault_snapshot.lock().unwrap().clone().unwrap()
    );
    assert!(state.twin_event_store.ordered_events().unwrap().is_empty());
}

#[test]
fn attached_boot_namespace_initialization_does_not_self_deadlock() {
    let (state, _vault, _data) = crate::commands::commit_note_write_tests::build_test_state();
    let coordinator = state.mutation_coordinator.unwrap();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(initialize_attached_namespace(&coordinator));
    });

    let result = receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("attached namespace initialization must not self-deadlock");
    let (namespace, scope) = result.unwrap();
    assert!(namespace.is_dir());
    assert_eq!(
        namespace,
        std::fs::canonicalize(crate::services::vault_namespace::scoped_data_path(
            state.twin_event_store.data_path(),
            &scope,
        ))
        .unwrap()
    );
}

#[tokio::test]
async fn every_readiness_component_failure_keeps_marker_unready_and_commands_unavailable() {
    for component in [
        WarmStartComponent::Migration,
        WarmStartComponent::Overlay,
        WarmStartComponent::Graph,
        WarmStartComponent::Search,
        WarmStartComponent::Chunk,
        WarmStartComponent::LinkDiscovery,
        WarmStartComponent::Optimizer,
        WarmStartComponent::TwinCaches,
    ] {
        let (state, _vault, data) = build_warm_start_state();
        let runtime_bootstrap = crate::app_runtime::RuntimeBootstrap::new(
            crate::models::runtime::RuntimeKind::Android,
            crate::app_runtime::RuntimePaths::android(
                data.path().join("runtime-data"),
                data.path().join("runtime-cache"),
            ),
            Arc::new(crate::app_runtime::UnavailableSecretStore),
            crate::models::runtime::RuntimeFeatureStatusV1::ready(),
            crate::models::runtime::RuntimeFeatureStatusV1::ready(),
        );
        runtime_bootstrap.paths.prepare().unwrap();
        let ready_runtime_status = runtime_bootstrap.status_for_state(true, true);
        let coordinator = state.mutation_coordinator.as_ref().unwrap();
        let namespace_path = coordinator.current_namespace_path().unwrap();

        let error = warm_start_services_inner(None, &state, Some(component))
            .await
            .unwrap_err();

        assert!(
            error.contains(&format!("{component:?}")),
            "expected injected {component:?} failure, got: {error}"
        );
        let marker = std::fs::read_to_string(namespace_path.join("ready-v1.json"))
            .expect("failed rebuild should retain the durable unready marker");
        assert!(marker.contains("\"ready\": false"));
        assert!(coordinator.require_namespace_ready().is_err());
        assert_eq!(
            state.mutation_startup_error.read().await.as_deref(),
            Some(error.as_str()),
            "a warm-start failure must revoke the canonical command gate"
        );
        let boot = state.boot_state.read().await.clone();
        assert_eq!(boot.phase, "failed");
        assert!(!boot.ready);
        assert_eq!(boot.error.as_deref(), Some(error.as_str()));
        assert!(crate::commands::acquire_root_epoch(&state).await.is_err());
        assert!(crate::commands::acquire_derived_root_epoch(&state)
            .await
            .is_err());
        let runtime_status =
            crate::commands::runtime::runtime_status_inner(&ready_runtime_status, &state).await;
        assert!(!runtime_status.vault.available);
        assert!(!runtime_status.capabilities.notes_read);
        assert!(!runtime_status.capabilities.notes_write);
        assert!(!runtime_status.capabilities.recall);
        assert!(!runtime_status.capabilities.twin_review);
        assert!(!runtime_status.capabilities.linear_canvas);
        assert!(!runtime_status.capabilities.sync);
        assert!(runtime_status
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "canonical_runtime_unavailable"));
    }
}

#[test]
fn canonical_directory_path_is_file_surfaces_as_recoverable_boot_failure() {
    let temp = tempfile::tempdir().unwrap();
    let store = TwinEventStore::new(temp.path());
    let events_path = store.events_dir();
    std::fs::create_dir_all(events_path.parent().unwrap()).unwrap();
    std::fs::write(&events_path, b"not a directory").unwrap();

    let error = initialize_twin_event_store_for_boot(&store).unwrap_err();
    let status = BootStatus::failed("failed", "Startup failed", error);
    assert_eq!(status.phase, "failed");
    assert!(!status.ready);
    assert!(status
        .error
        .unwrap()
        .contains("canonical Twin event directory"));
}

#[test]
fn production_twin_store_never_falls_back_to_the_noop_constructor() {
    let desktop = include_str!("lib.rs");
    let settings = include_str!("commands/settings.rs");
    assert!(desktop.contains("TwinStore::with_event_recorder_scoped("));
    let desktop_noop = [
        "let twin_store = TwinStore::",
        "new(settings_service.get().effective_twin_data_path())",
    ]
    .concat();
    assert!(!desktop.contains(&desktop_noop));
    let retarget = settings.find("replace_twin_and_canvas_roots(").unwrap();
    let publish = settings
        .find("settings.publish_runtime_authority(")
        .unwrap();
    assert!(retarget < publish);
    assert!(settings.contains("twin.replace_root_path_scoped("));
    let settings_noop = ["TwinStore::", "new(new_twin_path)"].concat();
    assert!(!settings.contains(&settings_noop));
}

#[test]
fn first_install_without_a_stable_lease_creates_the_configured_vault() {
    let temp = tempfile::tempdir().unwrap();
    let data_path = temp.path().join("data");
    let configured_vault = temp.path().join("first-install-vault");
    std::fs::create_dir(&data_path).unwrap();
    let transition_store = crate::services::root_transition::RootTransitionStore::new(
        &data_path,
        temp.path().join("settings.json"),
        Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    transition_store
        .write_settings_guarded(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                crate::models::settings::UserSettings {
                    vault_path: Some(configured_vault.to_string_lossy().into_owned()),
                    ..crate::models::settings::UserSettings::default()
                },
            ),
        )
        .unwrap();

    let runtime = transition_store
        .prepare_runtime_vault_path(&configured_vault)
        .unwrap();

    assert_eq!(runtime, configured_vault);
    assert!(runtime.is_dir());
}

#[test]
fn stable_attached_vault_removed_after_detachment_probe_is_not_recreated() {
    let temp = tempfile::tempdir().unwrap();
    let data_path = temp.path().join("data");
    let configured_vault = temp.path().join("stable-vault");
    std::fs::create_dir(&data_path).unwrap();
    std::fs::create_dir(&configured_vault).unwrap();
    let identity =
        crate::services::sync::identity::load_or_create_vault_identity(&configured_vault).unwrap();
    let transition_store = crate::services::root_transition::RootTransitionStore::new(
        &data_path,
        temp.path().join("settings.json"),
        Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    let settings = crate::models::settings::UserSettings {
        vault_path: Some(configured_vault.to_string_lossy().into_owned()),
        ..crate::models::settings::UserSettings::default()
    };
    transition_store
        .write_settings_guarded(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(settings),
        )
        .unwrap();
    transition_store
        .write_lease(
            &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                identity.root_scope,
            ),
        )
        .unwrap();

    assert!(transition_store.detached_stable_vault().unwrap().is_none());
    std::fs::remove_dir_all(&configured_vault).unwrap();

    let error = transition_store
        .prepare_runtime_vault_path(&configured_vault)
        .unwrap_err();
    assert!(matches!(
        error,
        crate::services::twin_events::MutationError::Io(_)
            | crate::services::twin_events::MutationError::Store(_)
    ));
    assert!(
        !configured_vault.exists(),
        "a stable vault removed after the probe must remain available for reattachment"
    );
}

#[test]
fn production_boot_probes_detachment_before_any_runtime_root_preparation() {
    let desktop = include_str!("lib.rs");
    let probe = desktop.find(".detached_stable_vault()").unwrap();
    let prepare = desktop
        .find(".prepare_runtime_vault_path(&vault_path)")
        .unwrap();

    assert!(probe < prepare);
    let unconditional_vault_create = ["std::fs::create_dir_all", "(&vault_path)"].concat();
    assert!(!desktop.contains(&unconditional_vault_create));
}

#[test]
fn peer_wal_after_a_no_lease_probe_prevents_first_install_creation() {
    let temp = tempfile::tempdir().unwrap();
    let data_path = temp.path().join("data");
    let configured_vault = temp.path().join("first-install-vault");
    std::fs::create_dir(&data_path).unwrap();
    let transition_store = crate::services::root_transition::RootTransitionStore::new(
        &data_path,
        temp.path().join("settings.json"),
        Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    transition_store
        .write_settings_guarded(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                crate::models::settings::UserSettings {
                    vault_path: Some(configured_vault.to_string_lossy().into_owned()),
                    ..crate::models::settings::UserSettings::default()
                },
            ),
        )
        .unwrap();
    assert!(transition_store.detached_stable_vault().unwrap().is_none());
    std::fs::write(
        data_path.join("twin/events/root-transition-v1.json"),
        b"peer-prepared",
    )
    .unwrap();

    let error = transition_store
        .prepare_runtime_vault_path(&configured_vault)
        .unwrap_err();
    assert!(error.to_string().contains("root-transition"));
    assert!(!configured_vault.exists());
}

#[test]
fn peer_stable_lease_after_a_no_lease_probe_prevents_first_install_creation() {
    let temp = tempfile::tempdir().unwrap();
    let data_path = temp.path().join("data");
    let configured_vault = temp.path().join("first-install-vault");
    std::fs::create_dir(&data_path).unwrap();
    let transition_store = crate::services::root_transition::RootTransitionStore::new(
        &data_path,
        temp.path().join("settings.json"),
        Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    transition_store
        .write_settings_guarded(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(
                crate::models::settings::UserSettings {
                    vault_path: Some(configured_vault.to_string_lossy().into_owned()),
                    ..crate::models::settings::UserSettings::default()
                },
            ),
        )
        .unwrap();
    assert!(transition_store.detached_stable_vault().unwrap().is_none());
    transition_store
        .write_lease(
            &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                crate::models::twin_event::ContentDigest::parse("a".repeat(64)).unwrap(),
            ),
        )
        .unwrap();

    let error = transition_store
        .prepare_runtime_vault_path(&configured_vault)
        .unwrap_err();
    assert!(matches!(
        error,
        crate::services::twin_events::MutationError::Io(_)
            | crate::services::twin_events::MutationError::Store(_)
    ));
    assert!(!configured_vault.exists());
}

#[test]
fn coordinator_failure_branch_never_creates_or_adopts_a_vault_descriptor() {
    let desktop = include_str!("lib.rs");
    let coordinator_result = desktop
        .find("let (event_recorder, mutation_coordinator")
        .unwrap();
    let services = desktop.find("// Initialize services").unwrap();
    assert!(
        !desktop[coordinator_result..services].contains("load_or_create_vault_identity"),
        "a failed coordinator must enter isolated recovery without touching vault identity"
    );
}

#[test]
fn warm_start_does_not_reinitialize_events_after_releasing_the_root_guard() {
    let desktop = include_str!("lib.rs");
    let warm_start = desktop.find("async fn warm_start_services_inner(").unwrap();
    let root_gate = desktop
        .find("async fn acquire_warm_start_root_gate(")
        .unwrap();
    assert!(
        !desktop[warm_start..root_gate]
            .contains("initialize_twin_event_store_for_boot(&state.twin_event_store)"),
        "coordinator construction already initializes and activates the event store"
    );
}

#[test]
fn pending_root_wal_failure_uses_only_ephemeral_recovery_service_roots() {
    let temp = tempfile::tempdir().unwrap();
    let data_path = temp.path().join("active-data");
    let vault_path = temp.path().join("active-vault");
    std::fs::create_dir(&data_path).unwrap();
    std::fs::create_dir(&vault_path).unwrap();
    let identity =
        crate::services::sync::identity::load_or_create_vault_identity(&vault_path).unwrap();
    let transition_store = crate::services::root_transition::RootTransitionStore::new(
        &data_path,
        temp.path().join("settings.json"),
        Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default()),
    )
    .unwrap();
    crate::services::twin_events::PersistedMutationIdentityProvider::load_or_create(&data_path)
        .unwrap();
    let settings = crate::models::settings::UserSettings {
        vault_path: Some(vault_path.to_string_lossy().into_owned()),
        ..crate::models::settings::UserSettings::default()
    };
    transition_store
        .write_settings_guarded(
            &crate::services::root_transition::NonsecretSettingsV1::from_settings(settings),
        )
        .unwrap();
    transition_store
        .write_lease(
            &crate::services::twin_events::ActiveMarkdownRootLeaseV1::new_stable(
                identity.root_scope,
            ),
        )
        .unwrap();

    // Model the peer gap after the desktop's detachment/lease probes: the
    // descriptor disappears and a peer publishes a prepared root WAL before
    // this process enters coordinator construction.
    std::fs::remove_file(vault_path.join("_grafyn/vault.json")).unwrap();
    std::fs::write(
        data_path.join("twin/events/root-transition-v1.json"),
        b"peer-prepared",
    )
    .unwrap();
    let event_store = Arc::new(TwinEventStore::new(&data_path));
    let coordinator_error = match MutationCoordinator::new_stable(
        &data_path,
        &vault_path,
        event_store,
        Arc::new(NoopMutationLifecycle),
    ) {
        Ok(_) => panic!("the pending root WAL must reject coordinator construction"),
        Err(error) => error,
    };
    assert!(coordinator_error.to_string().contains("root-transition"));
    let data_before = snapshot_tree(&data_path);
    let vault_before = snapshot_tree(&vault_path);

    let recovery = isolated_recovery_runtime_roots(&data_path, &vault_path).unwrap();
    assert!(!recovery.data_path.starts_with(&data_path));
    assert!(!recovery.vault_path.starts_with(&vault_path));
    assert!(!recovery.derived_data_path.starts_with(&data_path));
    let recovery_path = recovery
        .recovery_runtime
        .as_ref()
        .unwrap()
        .path()
        .to_path_buf();
    let unavailable: Arc<dyn EventRecorder> =
        Arc::new(UnavailableEventRecorder::new(coordinator_error.to_string()));
    {
        let _knowledge = KnowledgeStore::with_event_recorder(
            recovery.vault_path.clone(),
            recovery.derived_data_path.clone(),
            unavailable.clone(),
        );
        let _search = SearchService::new(recovery.derived_data_path.clone()).unwrap();
        let _chunk = ChunkIndex::new(recovery.derived_data_path.clone()).unwrap();
        let _canvas = CanvasStore::with_event_recorder(
            crate::services::canvas_store::scoped_canvas_path(
                &recovery.data_path,
                &recovery.active_scope,
            ),
            unavailable.clone(),
        );
        let _twin = TwinStore::with_event_recorder_scoped(
            crate::models::settings::twin_data_path_for_scope(
                &recovery.data_path,
                &recovery.active_scope,
            ),
            recovery.data_path.join("twin"),
            recovery.active_scope.clone(),
            unavailable,
        );
        let _link = LinkDiscoveryService::try_new(recovery.derived_data_path.clone()).unwrap();
        let _migration =
            MarkdownMigrationService::try_new(recovery.derived_data_path.clone()).unwrap();
        let _optimizer =
            VaultOptimizerService::try_new(recovery.derived_data_path.clone()).unwrap();
        let _priority = PriorityScoringService::new(recovery.data_path.clone());
        let _retrieval = RetrievalService::new(recovery.data_path.clone());
        let _feedback = FeedbackService::new(recovery.data_path.join("feedback"));
    }

    assert_eq!(snapshot_tree(&data_path), data_before);
    assert_eq!(snapshot_tree(&vault_path), vault_before);
    assert!(!vault_path.join("_grafyn/vault.json").exists());
    let retained_runtime = recovery.recovery_runtime.as_ref().unwrap().clone();
    drop(recovery);
    assert!(recovery_path.exists());
    drop(retained_runtime);
    assert!(!recovery_path.exists());
}

#[test]
fn recovery_runtime_rejects_a_temporary_base_inside_the_active_vault() {
    let active_vault = tempfile::tempdir().unwrap();
    let active_data = tempfile::tempdir().unwrap();
    let before = snapshot_tree(active_vault.path());
    let source = include_str!("lib.rs");
    let precheck = source
        .find("canonical_path_is_within(temporary_base, active_root)")
        .unwrap();
    let create = source.find(".tempdir_in(temporary_base)").unwrap();
    assert!(
        precheck < create,
        "overlap must be rejected before creation"
    );

    let error = match isolated_recovery_runtime_roots_in(
        active_vault.path(),
        active_data.path(),
        active_vault.path(),
    ) {
        Ok(_) => panic!("recovery runtime must not overlap an active vault"),
        Err(error) => error,
    };

    assert!(error.contains("overlaps the active vault root"));
    assert_eq!(snapshot_tree(active_vault.path()), before);
}
