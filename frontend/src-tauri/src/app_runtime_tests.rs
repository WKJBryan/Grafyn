use crate::app_runtime::{RuntimeBootstrap, RuntimePaths};
use crate::models::runtime::{
    RuntimeFeatureStatusV1, RuntimeKind, RuntimeStatusV1, RuntimeVaultKind,
};
use crate::models::settings::SettingsUpdate;
use crate::services::settings::SettingsService;
use crate::services::sync::secrets::{SecretAccount, SecretBytes, SecretStore, SecretStoreError};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

#[derive(Debug, Default)]
struct CountingUnavailableSecretStore {
    calls: AtomicUsize,
}

impl SecretStore for CountingUnavailableSecretStore {
    fn put(&self, _account: &SecretAccount, _secret: &SecretBytes) -> Result<(), SecretStoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(SecretStoreError::BackendUnavailable)
    }

    fn get(&self, _account: &SecretAccount) -> Result<Option<SecretBytes>, SecretStoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(SecretStoreError::BackendUnavailable)
    }

    fn delete(&self, _account: &SecretAccount) -> Result<(), SecretStoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(SecretStoreError::BackendUnavailable)
    }
}

#[derive(Debug, Default)]
struct RecordingSecretStore {
    inner: crate::services::root_transition::MemoryVersionedSecretStore,
    accounts: Mutex<Vec<String>>,
}

impl RecordingSecretStore {
    fn record(&self, operation: &str, account: &SecretAccount) {
        self.accounts
            .lock()
            .unwrap()
            .push(format!("{operation}:{}", account.as_str()));
    }
}

impl SecretStore for RecordingSecretStore {
    fn put(&self, account: &SecretAccount, secret: &SecretBytes) -> Result<(), SecretStoreError> {
        self.record("put", account);
        self.inner.put(account, secret)
    }

    fn get(&self, account: &SecretAccount) -> Result<Option<SecretBytes>, SecretStoreError> {
        self.record("get", account);
        self.inner.get(account)
    }

    fn delete(&self, account: &SecretAccount) -> Result<(), SecretStoreError> {
        self.record("delete", account);
        self.inner.delete(account)
    }
}

fn memory_secrets() -> Arc<dyn SecretStore> {
    Arc::new(crate::services::root_transition::MemoryVersionedSecretStore::default())
}

fn android_bootstrap(temp: &tempfile::TempDir) -> RuntimeBootstrap {
    RuntimeBootstrap::new(
        RuntimeKind::Android,
        RuntimePaths::android(temp.path().join("app-data"), temp.path().join("app-cache")),
        memory_secrets(),
        RuntimeFeatureStatusV1::ready(),
        RuntimeFeatureStatusV1::ready(),
    )
}

#[test]
fn android_paths_are_app_private_and_share_only_from_the_fileprovider_cache_root() {
    let temp = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::android(temp.path().join("app-data"), temp.path().join("app-cache"));

    assert_eq!(paths.config_dir, temp.path().join("app-data/Grafyn/config"));
    assert_eq!(paths.data_dir, temp.path().join("app-data/Grafyn/data"));
    assert_eq!(paths.vault_dir, temp.path().join("app-data/Grafyn/vault"));
    assert_eq!(paths.cache_dir, temp.path().join("app-cache/Grafyn"));
    assert_eq!(
        paths.share_dir,
        temp.path().join("app-cache/Grafyn/grafyn-share-v1")
    );
    let rendered = format!("{paths:?}").to_ascii_lowercase();
    assert!(!rendered.contains("documents"));

    paths.prepare().unwrap();
    for path in [
        &paths.config_dir,
        &paths.data_dir,
        &paths.vault_dir,
        &paths.cache_dir,
        &paths.share_dir,
    ] {
        assert!(
            path.is_dir(),
            "{} should be a real directory",
            path.display()
        );
    }
}

#[test]
fn android_private_twin_roots_are_excluded_from_backup_and_device_transfer() {
    let manifest = include_str!("../gen/android/app/src/main/AndroidManifest.xml");
    let modern_rules =
        include_str!("../gen/android/app/src/main/res/xml/data_extraction_rules.xml");
    let legacy_rules = include_str!("../gen/android/app/src/main/res/xml/full_backup_content.xml");

    assert!(manifest.contains("android:allowBackup=\"false\""));
    assert!(manifest.contains("android:dataExtractionRules=\"@xml/data_extraction_rules\""));
    assert!(manifest.contains("android:fullBackupContent=\"@xml/full_backup_content\""));
    assert!(modern_rules.contains("<cloud-backup>"));
    assert!(modern_rules.contains("<device-transfer>"));
    assert!(legacy_rules.contains("<full-backup-content>"));

    for domain in ["root", "file", "database", "sharedpref", "external"] {
        let exclusion = format!("<exclude domain=\"{domain}\" path=\".\" />");
        assert_eq!(
            modern_rules.matches(&exclusion).count(),
            2,
            "modern rules must exclude {domain} from cloud backup and device transfer"
        );
        assert_eq!(
            legacy_rules.matches(&exclusion).count(),
            1,
            "legacy rules must exclude {domain} from backup"
        );
    }
}

#[test]
fn app_private_runtime_path_collision_fails_without_replacing_user_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::android(temp.path().join("app-data"), temp.path().join("app-cache"));
    std::fs::create_dir_all(paths.data_dir.parent().unwrap()).unwrap();
    std::fs::write(&paths.data_dir, b"canonical-user-bytes").unwrap();

    let error = paths.prepare().unwrap_err();

    assert!(error.contains("data"));
    assert_eq!(
        std::fs::read(&paths.data_dir).unwrap(),
        b"canonical-user-bytes"
    );
}

#[test]
fn android_runtime_status_is_typed_path_free_and_capability_gated() {
    let temp = tempfile::tempdir().unwrap();
    let bootstrap = android_bootstrap(&temp);
    bootstrap.paths.prepare().unwrap();

    let status = bootstrap.status();
    assert_eq!(status.schema_version, 1);
    assert_eq!(status.runtime, RuntimeKind::Android);
    assert_eq!(status.vault.kind, RuntimeVaultKind::AppPrivate);
    assert!(status.vault.available);
    assert!(status.capabilities.notes_read);
    assert!(status.capabilities.notes_write);
    assert!(status.capabilities.recall);
    assert!(status.capabilities.twin_review);
    assert!(status.capabilities.twin_chat);
    assert!(status.capabilities.linear_canvas);
    assert!(!status.capabilities.image_generation);
    assert!(!status.capabilities.native_image_share);
    assert!(status.capabilities.sync);
    assert!(!status.capabilities.spatial_canvas);
    assert!(!status.capabilities.native_vault_picker);
    assert!(!status.capabilities.import_by_path);
    assert!(!status.capabilities.local_ollama);
    assert!(!status.capabilities.mcp);
    assert!(!status.capabilities.vault_migration);
    assert!(!status.capabilities.optimizer_admin);
    assert!(!status.capabilities.desktop_updater);

    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["runtime"], "android");
    assert_eq!(json["vault"]["kind"], "app_private");
    assert_eq!(json["secureSecrets"]["status"], "ready");
    assert_eq!(json["nativeImageShare"]["status"], "unavailable");
    assert_eq!(
        json["nativeImageShare"]["code"],
        "android_image_receipt_unavailable"
    );
    assert!(!serde_json::to_string(&json)
        .unwrap()
        .contains(&temp.path().to_string_lossy().to_string()));
}

#[test]
fn unavailable_mobile_security_disables_only_secret_dependent_capabilities() {
    let temp = tempfile::tempdir().unwrap();
    let bootstrap = RuntimeBootstrap::new(
        RuntimeKind::Android,
        RuntimePaths::android(temp.path().join("app-data"), temp.path().join("app-cache")),
        memory_secrets(),
        RuntimeFeatureStatusV1::unavailable(
            "keystore\nmissing",
            format!("{}\nsecret path", "x".repeat(400)),
        ),
        RuntimeFeatureStatusV1::unavailable("share_missing", "Native share unavailable"),
    );
    bootstrap.paths.prepare().unwrap();

    let status = bootstrap.status();
    assert!(status.capabilities.notes_write);
    assert!(status.capabilities.recall);
    assert!(status.capabilities.twin_review);
    assert!(status.capabilities.linear_canvas);
    assert!(!status.capabilities.twin_chat);
    assert!(!status.capabilities.image_generation);
    assert!(!status.capabilities.sync);
    assert!(!status.capabilities.native_image_share);
    assert_eq!(status.diagnostics.len(), 2);
    for diagnostic in &status.diagnostics {
        assert!(diagnostic.code.chars().count() <= 64);
        assert!(diagnostic.message.chars().count() <= 256);
        assert!(!diagnostic.code.chars().any(char::is_control));
        assert!(!diagnostic.message.chars().any(char::is_control));
    }
}

#[test]
fn desktop_runtime_status_preserves_the_existing_wide_surface() {
    let temp = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::desktop(
        temp.path().join("config"),
        temp.path().join("data"),
        temp.path().join("vault"),
        temp.path().join("cache"),
    );
    paths.prepare().unwrap();
    let bootstrap = RuntimeBootstrap::new(
        RuntimeKind::Desktop,
        paths,
        memory_secrets(),
        RuntimeFeatureStatusV1::ready(),
        RuntimeFeatureStatusV1::unavailable("desktop_save_as", "Desktop uses Save As"),
    );

    let RuntimeStatusV1 {
        capabilities,
        vault,
        ..
    } = bootstrap.status();
    assert_eq!(vault.kind, RuntimeVaultKind::UserSelected);
    assert!(capabilities.spatial_canvas);
    assert!(capabilities.native_vault_picker);
    assert!(capabilities.import_by_path);
    assert!(capabilities.local_ollama);
    assert!(capabilities.mcp);
    assert!(capabilities.vault_migration);
    assert!(capabilities.optimizer_admin);
    assert!(capabilities.desktop_updater);
    assert!(!capabilities.native_image_share);
}

#[test]
fn android_settings_use_only_injected_paths_and_never_allow_environment_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let bootstrap = android_bootstrap(&temp);

    let settings = SettingsService::load_for_runtime(&bootstrap).unwrap();

    assert_eq!(settings.runtime_kind(), RuntimeKind::Android);
    assert_eq!(settings.data_path(), bootstrap.paths.data_dir);
    assert_eq!(
        std::fs::canonicalize(settings.vault_path()).unwrap(),
        std::fs::canonicalize(&bootstrap.paths.vault_dir).unwrap()
    );
    assert!(!settings.allows_environment_fallback());
    assert!(!settings.needs_setup());
    assert!(bootstrap.paths.config_dir.join("settings.json").is_file());
}

#[test]
fn android_settings_and_sync_bootstrap_share_one_injected_secure_adapter() {
    const KEY_VERSION: &str = "123e4567-e89b-42d3-a456-426614174000";
    let temp = tempfile::tempdir().unwrap();
    let recording = Arc::new(RecordingSecretStore::default());
    let injected: Arc<dyn SecretStore> = recording.clone();
    let bootstrap = RuntimeBootstrap::new(
        RuntimeKind::Android,
        RuntimePaths::android(temp.path().join("app-data"), temp.path().join("app-cache")),
        injected.clone(),
        RuntimeFeatureStatusV1::ready(),
        RuntimeFeatureStatusV1::ready(),
    );
    let settings = SettingsService::load_for_runtime(&bootstrap).unwrap();
    assert!(Arc::ptr_eq(&injected, &settings.secret_store()));
    settings
        .root_transition_store()
        .unwrap()
        .stage_secret(KEY_VERSION, "openrouter-secret")
        .unwrap();
    let identity =
        crate::services::sync::identity::load_or_create_vault_identity(&bootstrap.paths.vault_dir)
            .unwrap();
    crate::services::sync::vault_keys::provision_vault_root_key(
        recording.as_ref(),
        &identity.descriptor.vault_id().to_string(),
        &grafyn_sync_protocol::VaultRootKey::from_bytes([7; 32]),
    )
    .unwrap();

    let state = crate::build_app_state_with_secure_runtime(settings, None, true).unwrap();

    assert!(state.sync_engine.is_some());
    let accounts = recording.accounts.lock().unwrap();
    assert!(accounts
        .iter()
        .any(|entry| entry.contains("openrouter_api_key/")));
    assert!(accounts.iter().any(|entry| entry.contains("sync.vault.")));
    assert!(accounts
        .iter()
        .any(|entry| entry.contains("sync.device.ed25519.v1")));
}

#[test]
fn android_settings_dtos_hide_private_paths_and_reject_non_openrouter_providers() {
    let temp = tempfile::tempdir().unwrap();
    let bootstrap = android_bootstrap(&temp);
    let settings = SettingsService::load_for_runtime(&bootstrap).unwrap();

    let public_settings = crate::commands::settings::redact_settings_for_runtime(
        settings.get(),
        settings.runtime_kind(),
    );
    let public_status = crate::commands::settings::redact_status_for_runtime(
        settings.status(),
        settings.runtime_kind(),
    );
    assert_eq!(public_settings.vault_path, None);
    assert_eq!(public_status.vault_path, None);
    assert!(public_status.has_vault_path);

    for provider in ["ollama", "local", ""] {
        let update = SettingsUpdate {
            vault_path: None,
            openrouter_api_key: None,
            setup_completed: None,
            theme: None,
            mcp_enabled: None,
            llm_model: None,
            twin_llm_provider: Some(provider.to_string()),
            ollama_base_url: None,
            ollama_model: None,
            smart_web_search: None,
            background_link_discovery_enabled: None,
            background_link_discovery_llm_enabled: None,
            background_vault_optimizer_enabled: None,
            background_vault_optimizer_llm_enabled: None,
            background_vault_optimizer_budget_monthly: None,
            background_vault_optimizer_max_daily_writes: None,
            background_vault_optimizer_edit_mode: None,
            background_vault_optimizer_program_enabled: None,
            vault_optimizer_program_path: None,
            canvas_model_presets: None,
        };
        assert!(settings.validate_update_for_runtime(&update).is_err());
    }
}

#[test]
fn android_app_state_omits_desktop_only_services() {
    let temp = tempfile::tempdir().unwrap();
    let bootstrap = android_bootstrap(&temp);
    let settings = SettingsService::load_for_runtime(&bootstrap).unwrap();

    let state = crate::build_app_state(settings, None).unwrap();

    assert!(state.ollama.is_none());
    assert!(state.link_discovery.is_none());
    assert!(state.markdown_migration.is_none());
    assert!(state.vault_optimizer.is_none());
}

#[tokio::test]
async fn unavailable_android_secrets_keep_canonical_local_capture_without_secret_fallback() {
    use crate::commands::twin_state::{
        CompanionCaptureContextInput, CompanionCaptureKind, CompanionSyncPolicy,
        CreateCompanionCaptureRequest,
    };
    use chrono::{TimeZone, Utc};

    let temp = tempfile::tempdir().unwrap();
    let counting = Arc::new(CountingUnavailableSecretStore::default());
    let bootstrap = RuntimeBootstrap::new(
        RuntimeKind::Android,
        RuntimePaths::android(temp.path().join("app-data"), temp.path().join("app-cache")),
        counting.clone(),
        RuntimeFeatureStatusV1::unavailable(
            "android_keystore_unavailable",
            "Android secure storage is unavailable.",
        ),
        RuntimeFeatureStatusV1::unavailable(
            "android_image_share_unavailable",
            "Android image sharing is unavailable.",
        ),
    );
    let settings = SettingsService::load_for_runtime(&bootstrap).unwrap();
    let state = crate::build_app_state_with_secure_runtime(settings, None, false).unwrap();

    assert!(state.mutation_coordinator.is_some());
    assert!(state.sync_engine.is_none());
    assert!(state.mutation_startup_error.try_read().unwrap().is_none());
    let capture = crate::commands::twin_state::create_companion_capture_inner(
        &state,
        CreateCompanionCaptureRequest {
            content: "Remember the offline companion state".to_string(),
            capture_kind: CompanionCaptureKind::Text,
            context: CompanionCaptureContextInput::default(),
            attachment_digests: Vec::new(),
            grafyn_sync: CompanionSyncPolicy::Inherit,
        },
        Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        state
            .knowledge_store
            .read()
            .await
            .get_note(&capture.note.id)
            .unwrap()
            .id,
        capture.note.id
    );
    assert!(state
        .twin_event_store
        .ordered_events()
        .unwrap()
        .iter()
        .any(|event| event.event_id == capture.observation_event_id));
    assert_eq!(counting.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn available_android_secrets_fail_closed_on_corrupt_vault_key() {
    let temp = tempfile::tempdir().unwrap();
    let recording = Arc::new(RecordingSecretStore::default());
    let bootstrap = RuntimeBootstrap::new(
        RuntimeKind::Android,
        RuntimePaths::android(temp.path().join("app-data"), temp.path().join("app-cache")),
        recording.clone(),
        RuntimeFeatureStatusV1::ready(),
        RuntimeFeatureStatusV1::ready(),
    );
    let settings = SettingsService::load_for_runtime(&bootstrap).unwrap();
    let identity =
        crate::services::sync::identity::load_or_create_vault_identity(&bootstrap.paths.vault_dir)
            .unwrap();
    recording
        .put(
            &SecretAccount::sync_vault_root(&identity.descriptor.vault_id().to_string()).unwrap(),
            &SecretBytes::new(vec![0x51; 31]).unwrap(),
        )
        .unwrap();

    let state = crate::build_app_state_with_secure_runtime(settings, None, true).unwrap();

    assert!(state.mutation_coordinator.is_none());
    assert!(state.sync_engine.is_none());
    let error = state
        .mutation_startup_error
        .try_read()
        .unwrap()
        .clone()
        .expect("authoritative corruption must keep canonical commands unavailable");
    assert!(error.contains("stored vault root key is invalid"));
    let boot = state.boot_state.try_read().unwrap().clone();
    assert_eq!(boot.phase, "failed");
    assert!(!boot.ready);
    assert_eq!(boot.error.as_deref(), Some(error.as_str()));
}

#[test]
fn android_plaintext_openrouter_key_fails_before_secret_store_access_and_preserves_file() {
    let temp = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::android(temp.path().join("app-data"), temp.path().join("app-cache"));
    paths.prepare().unwrap();
    let config_path = paths.config_dir.join("settings.json");
    let mut settings =
        serde_json::to_value(crate::models::settings::UserSettings::default()).unwrap();
    settings["openrouter_api_key"] =
        serde_json::Value::String("legacy-plaintext-must-not-migrate".to_string());
    let original = serde_json::to_vec_pretty(&settings).unwrap();
    std::fs::write(&config_path, &original).unwrap();
    let counting = Arc::new(CountingUnavailableSecretStore::default());
    let bootstrap = RuntimeBootstrap::new(
        RuntimeKind::Android,
        paths,
        counting.clone(),
        RuntimeFeatureStatusV1::unavailable("keystore_unavailable", "Keystore unavailable"),
        RuntimeFeatureStatusV1::unavailable("share_unavailable", "Share unavailable"),
    );

    let error = match SettingsService::load_for_runtime(&bootstrap) {
        Ok(_) => panic!("Android plaintext settings must fail closed"),
        Err(error) => error,
    };

    assert!(error
        .to_string()
        .contains("android-plaintext-openrouter-key-rejected"));
    assert_eq!(std::fs::read(config_path).unwrap(), original);
    assert_eq!(counting.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn failed_boot_state_uses_isolated_empty_twin_roots_and_blocks_authoritative_runtime() {
    let host = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::android(host.path().join("app-data"), host.path().join("app-cache"));
    let state =
        crate::build_recoverable_app_state(&paths, None, "canonical event replay failed").unwrap();
    let recovery_root = state
        ._recovery_runtime
        .as_ref()
        .expect("failed boot keeps isolated roots alive")
        .path();

    assert!(state.mutation_coordinator.is_none());
    assert!(state.sync_engine.is_none());
    assert!(state
        .twin_event_store
        .data_path()
        .starts_with(recovery_root));
    let twin_root = state
        .twin_store
        .try_read()
        .unwrap()
        .root_path()
        .to_path_buf();
    let canonical_twin_root = std::fs::canonicalize(&twin_root).unwrap();
    let canonical_recovery_root = std::fs::canonicalize(recovery_root).unwrap();
    assert!(
        canonical_twin_root.starts_with(&canonical_recovery_root),
        "isolated Twin root {} must remain under {}",
        canonical_twin_root.display(),
        canonical_recovery_root.display()
    );
    assert!(state.twin_event_store.ordered_events().unwrap().is_empty());
    assert!(recovery_root.starts_with(host.path().join("app-cache")));
    let boot = state.boot_state.try_read().unwrap().clone();
    assert_eq!(boot.phase, "failed");
    assert!(!boot.ready);
    assert_eq!(boot.error.as_deref(), Some("canonical event replay failed"));
}

#[test]
fn invalid_registered_android_health_enters_recoverable_failed_boot_without_secret_access() {
    let host = tempfile::tempdir().unwrap();
    let counting = Arc::new(CountingUnavailableSecretStore::default());
    let mut bootstrap = RuntimeBootstrap::new(
        RuntimeKind::Android,
        RuntimePaths::android(host.path().join("app-data"), host.path().join("app-cache")),
        counting.clone(),
        RuntimeFeatureStatusV1::unavailable(
            crate::ANDROID_NATIVE_HEALTH_INVALID,
            "Android native health is invalid.",
        ),
        RuntimeFeatureStatusV1::unavailable(
            crate::ANDROID_NATIVE_HEALTH_INVALID,
            "Android native health is invalid.",
        ),
    )
    .with_startup_error(crate::ANDROID_NATIVE_HEALTH_INVALID);

    let state = crate::initialize_runtime_state(&mut bootstrap, None).unwrap();

    assert!(state.mutation_coordinator.is_none());
    assert!(state.sync_engine.is_none());
    assert_eq!(counting.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        state.mutation_startup_error.try_read().unwrap().as_deref(),
        Some(crate::ANDROID_NATIVE_HEALTH_INVALID),
    );
    let boot = state.boot_state.try_read().unwrap().clone();
    assert_eq!(boot.phase, "failed");
    assert!(!boot.ready);
    assert_eq!(
        boot.error.as_deref(),
        Some(crate::ANDROID_NATIVE_HEALTH_INVALID),
    );
}

#[test]
fn failed_canonical_runtime_disables_local_capabilities_and_vault_availability() {
    let temp = tempfile::tempdir().unwrap();
    let bootstrap = android_bootstrap(&temp);
    bootstrap.paths.prepare().unwrap();

    let status = bootstrap.status_for_state(false, false);

    assert!(!status.vault.available);
    assert!(!status.capabilities.notes_read);
    assert!(!status.capabilities.notes_write);
    assert!(!status.capabilities.recall);
    assert!(!status.capabilities.twin_review);
    assert!(!status.capabilities.linear_canvas);
    assert!(!status.capabilities.native_image_share);
    assert!(!status.capabilities.sync);
    assert!(!status.secure_secrets.is_ready());
    assert!(!status.native_image_share.is_ready());
    assert!(status
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "canonical_runtime_unavailable"));

    let ready = bootstrap.status_for_state(true, true);
    assert_eq!(
        crate::app_runtime::mask_canonical_runtime_unavailable(ready),
        status
    );
}

#[test]
fn mobile_and_desktop_command_registrations_are_separate_and_workers_are_desktop_only() {
    let source = include_str!("lib.rs");
    let mobile_start = source
        .find("fn register_mobile_commands")
        .expect("mobile command registration");
    let desktop_start = source
        .find("fn register_desktop_commands")
        .expect("desktop command registration");
    let (mobile, desktop) = if mobile_start < desktop_start {
        (
            &source[mobile_start..desktop_start],
            &source[desktop_start..],
        )
    } else {
        (
            &source[mobile_start..],
            &source[desktop_start..mobile_start],
        )
    };

    let registered = mobile
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("commands::"))
        .map(|line| line.trim_end_matches(',').to_string())
        .collect::<Vec<_>>();
    let expected = [
        "commands::runtime::get_runtime_status",
        "commands::boot::get_boot_status",
        "commands::notes::list_notes",
        "commands::notes::get_note",
        "commands::notes::create_note",
        "commands::notes::update_note",
        "commands::notes::delete_note",
        "commands::canvas::list_sessions",
        "commands::canvas::get_session",
        "commands::canvas::create_session",
        "commands::canvas::update_session",
        "commands::canvas::delete_session",
        "commands::canvas::get_available_models",
        "commands::canvas::send_prompt",
        "commands::canvas::regenerate_response",
        "commands::twin::get_twin_review",
        "commands::twin::record_canvas_feedback",
        "commands::twin::list_decision_episodes",
        "commands::twin::get_decision_mirror_config",
        "commands::twin::list_memory_digest",
        "commands::twin::review_memory_digest_item",
        "commands::twin::list_constitution_items",
        "commands::twin::list_action_gaps",
        "commands::twin::get_constitution_setup",
        "commands::twin_state::list_twin_observations",
        "commands::twin_state::list_twin_proposals",
        "commands::twin_state::create_companion_capture",
        "commands::twin_state::review_twin_proposal",
        "commands::twin_state::get_twin_state_projection",
        "commands::twin_state::rank_twin_attention",
        "commands::twin_state::get_twin_event_timeline",
        "commands::image_generation::discover_image_models",
        "commands::image_generation::get_image_model_capability",
        "commands::image_generation::generate_image",
        "commands::image_generation::discard_generated_image_receipt",
        "commands::image_generation::save_generated_image",
        "commands::image_generation::load_generated_image",
        "commands::image_generation::share_generated_image",
        "commands::settings::get_settings",
        "commands::settings::get_settings_status",
        "commands::settings::update_settings",
        "commands::settings::get_openrouter_status",
        "commands::sync::get_sync_status",
        "commands::memory::recall_relevant",
    ];
    assert_eq!(
        registered,
        expected.map(str::to_string),
        "Android IPC must remain an exact compact allowlist"
    );

    for desktop_only in [
        "commands::mcp::",
        "commands::import::",
        "commands::migration::",
        "commands::settings::pick_vault_folder",
        "commands::settings::get_ollama_status",
        "commands::settings::list_ollama_models",
        "commands::canvas::start_debate",
        "commands::canvas::continue_debate",
        "commands::canvas::update_viewport",
        "start_link_discovery_worker",
        "start_vault_optimizer_worker",
    ] {
        assert!(
            desktop.contains(desktop_only),
            "desktop lost {desktop_only}"
        );
    }
    assert!(desktop.contains("commands::runtime::get_runtime_status"));
}

#[test]
fn startup_failures_are_reported_through_boot_state_without_exiting_the_process() {
    let source = include_str!("lib.rs");

    assert!(!source.contains("std::process::exit"));
    assert!(source.contains("build_recoverable_app_state"));
    assert!(source.contains("BootStatus::failed"));
}
