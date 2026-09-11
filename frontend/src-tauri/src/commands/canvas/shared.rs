use crate::models::canvas::{CanvasSession, ContextMode, PromptType};
use crate::models::settings::UserSettings;
use crate::models::twin::TraceEventType;
use crate::services::twin::TwinStore;
use crate::AppState;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModelRoute {
    pub(super) provider: ModelProviderRoute,
    pub(super) model_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ModelProviderRoute {
    OpenRouter,
    Ollama,
}

impl ModelProviderRoute {
    pub(super) fn provider_label(&self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::Ollama => "ollama",
        }
    }

    pub(super) fn provenance_label(&self, debate: bool) -> &'static str {
        match (self, debate) {
            (Self::OpenRouter, false) => "canvas_openrouter",
            (Self::Ollama, false) => "canvas_ollama",
            (Self::OpenRouter, true) => "canvas_debate_openrouter",
            (Self::Ollama, true) => "canvas_debate_ollama",
        }
    }
}

pub(super) fn resolve_model_route(
    prompt_type: &PromptType,
    context_mode: &ContextMode,
    twin_provider_override: Option<&str>,
    settings: &UserSettings,
) -> Result<ModelRoute, String> {
    let vault_context = is_vault_context_prompt(prompt_type, context_mode);
    let twin_provider = if let Some(provider) = twin_provider_override {
        let provider = provider.trim().to_ascii_lowercase();
        if provider != "ollama" && provider != "openrouter" {
            return Err(format!(
                "Unsupported Canvas provider override: {}",
                provider
            ));
        }
        provider
    } else if vault_context {
        let provider = settings.twin_llm_provider.trim().to_ascii_lowercase();
        if provider != "ollama" && provider != "openrouter" {
            return Err(format!("Unsupported Canvas provider setting: {}", provider));
        }
        provider
    } else {
        "openrouter".to_string()
    };

    if twin_provider == "ollama" {
        let model = settings.ollama_model.trim();
        if model.is_empty() {
            return Err(
                "Select an Ollama model for local vault/twin responses before sending vault context"
                    .to_string(),
            );
        }

        return Ok(ModelRoute {
            provider: ModelProviderRoute::Ollama,
            model_ids: vec![model.to_string()],
        });
    }

    Ok(ModelRoute {
        provider: ModelProviderRoute::OpenRouter,
        model_ids: Vec::new(),
    })
}

pub(super) fn source_tile_context_provider(
    session: &CanvasSession,
    source_tile_ids: &[String],
) -> (bool, Option<String>) {
    for tile_id in source_tile_ids {
        if let Some(tile) = session.prompt_tiles.iter().find(|tile| &tile.id == tile_id) {
            if is_vault_context_prompt(&tile.prompt_type, &tile.context_mode) {
                return (true, tile.twin_llm_provider.clone());
            }
        }
    }

    (false, None)
}

pub(super) fn is_vault_context_prompt(
    prompt_type: &PromptType,
    context_mode: &ContextMode,
) -> bool {
    prompt_type == &PromptType::Decision
        || matches!(
            context_mode,
            ContextMode::KnowledgeSearch
                | ContextMode::Semantic
                | ContextMode::Twin
                | ContextMode::TwinHistory
                | ContextMode::FullHistory
                | ContextMode::Compact
        )
}

pub(super) fn effective_model_ids(
    route: &ModelRoute,
    requested_model_ids: &[String],
) -> Vec<String> {
    match route.provider {
        ModelProviderRoute::Ollama => {
            if requested_model_ids.is_empty() {
                route.model_ids.clone()
            } else {
                requested_model_ids.to_vec()
            }
        }
        ModelProviderRoute::OpenRouter => requested_model_ids.to_vec(),
    }
}

/// Best-effort structural audit used only after an independently durable
/// session/layout mutation. Governed traces must use the `Result`-returning
/// `append_canvas_trace_expecting_authority` path below.
pub(super) async fn append_optional_canvas_audit_trace(
    twin_store_arc: Arc<RwLock<TwinStore>>,
    session_id: &str,
    event_type: TraceEventType,
    payload: serde_json::Value,
) -> Option<crate::services::twin_events::MutationCommit> {
    let mut twin_store = twin_store_arc.write().await;
    match twin_store.append_trace_event_with_commit(session_id, event_type, payload) {
        Ok((_, commit)) => Some(commit),
        Err(error) => {
            if let Some(commit) = error
                .downcast_ref::<crate::services::twin_events::MutationError>()
                .and_then(|error| error.authority_advanced_commit())
            {
                return Some(commit);
            }
            log::error!(
                "Failed to append twin trace for session '{}': {}",
                session_id,
                error
            );
            None
        }
    }
}

#[derive(Debug)]
pub(super) struct CanvasTraceMutationError {
    message: String,
    repair_commit: Option<crate::services::twin_events::MutationCommit>,
}

impl CanvasTraceMutationError {
    pub(super) fn precommit(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            repair_commit: None,
        }
    }

    pub(super) fn repair_commit(&self) -> Option<&crate::services::twin_events::MutationCommit> {
        self.repair_commit.as_ref()
    }

    pub(super) fn with_fallback_commit(
        mut self,
        commit: crate::services::twin_events::MutationCommit,
    ) -> Self {
        if self.repair_commit.is_none() && commit.authority_token.is_some() {
            self.message = format!(
                "{}; earlier Canvas work committed, so do not retry automatically",
                self.message
            );
            self.repair_commit = Some(commit);
        }
        self
    }
}

impl std::fmt::Display for CanvasTraceMutationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub(super) fn preserve_canvas_mutation_error(error: anyhow::Error) -> CanvasTraceMutationError {
    let authority_outcome =
        crate::services::knowledge_store::knowledge_authority_advanced_outcome(&error);
    let repair_commit = authority_outcome
        .as_ref()
        .map(|outcome| outcome.commit.clone());
    let target_aborted = authority_outcome
        .as_ref()
        .map(|outcome| outcome.target_aborted)
        .unwrap_or(false);
    let message = if target_aborted {
        format!(
            "Canvas mutation authority advanced but its target was aborted; do not retry automatically: {error}"
        )
    } else if repair_commit.is_some() {
        format!("Canvas mutation authority advanced; do not retry automatically: {error}")
    } else {
        error.to_string()
    };

    CanvasTraceMutationError {
        message,
        repair_commit,
    }
}

pub(super) async fn repair_canvas_trace_error(
    state: &AppState,
    error: &CanvasTraceMutationError,
    operation: &str,
) -> crate::commands::PostAuthorityRepair {
    match error.repair_commit() {
        Some(commit) => {
            crate::commands::repair_after_authority_mutation(state, commit, operation).await
        }
        None => crate::commands::PostAuthorityRepair::NotRequired,
    }
}

pub(super) async fn append_canvas_trace_expecting_authority(
    twin_store_arc: Arc<RwLock<TwinStore>>,
    session_id: &str,
    event_type: TraceEventType,
    payload: serde_json::Value,
    expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
) -> Result<crate::services::twin_events::MutationCommit, CanvasTraceMutationError> {
    let mut twin_store = twin_store_arc.write().await;
    let (_, commit) = twin_store
        .append_trace_event_expecting_authority(session_id, event_type, payload, expected)
        .map_err(|error| {
            let aborted_commit = error
                .downcast_ref::<crate::services::twin_events::MutationError>()
                .filter(|error| error.authority_advanced_target_aborted())
                .and_then(|error| error.authority_advanced_commit());
            let message = if aborted_commit.is_some() {
                format!(
                    "Twin trace authority advanced but its target was aborted; do not retry automatically: {error}"
                )
            } else {
                error.to_string()
            };
            CanvasTraceMutationError {
                message,
                repair_commit: aborted_commit,
            }
        })?;
    if commit.authority_token.is_none() {
        return Err(CanvasTraceMutationError::precommit(
            "Twin trace mutation did not advance the content authority generation",
        ));
    }
    Ok(commit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_between<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        let start = source
            .find(start)
            .unwrap_or_else(|| panic!("missing source marker: {start}"));
        let source = &source[start..];
        let end = source
            .find(end)
            .unwrap_or_else(|| panic!("missing source marker: {end}"));
        &source[..end]
    }

    #[test]
    fn direct_canvas_abort_preserves_exact_commit_and_prohibits_retry() {
        let mutation_id = crate::services::twin_events::digest_bytes(b"canvas-abort");
        let authority_token = crate::services::vault_namespace::VaultAuthorityTokenV1 {
            root_scope: crate::services::twin_events::digest_bytes(b"canvas-root"),
            lease_epoch_uuid: uuid::Uuid::nil().to_string(),
            authority_generation: 7,
        };
        let error = anyhow::Error::new(
            crate::services::twin_events::MutationError::AuthorityAdvanced {
                mutation_id: mutation_id.clone(),
                authority_token: authority_token.clone(),
                target_aborted: true,
                reason: "guard drift".to_string(),
            },
        );

        let preserved = preserve_canvas_mutation_error(error);

        let repair_commit = preserved
            .repair_commit()
            .expect("the exact authority commit must survive the command boundary");
        assert_eq!(repair_commit.mutation_id.as_ref(), Some(&mutation_id));
        assert_eq!(
            repair_commit.authority_token.as_ref(),
            Some(&authority_token)
        );
        assert!(preserved.to_string().contains("target was aborted"));
        assert!(preserved.to_string().contains("do not retry automatically"));
    }

    #[test]
    fn knowledge_wrapped_canvas_commit_preserves_exact_commit_and_prohibits_retry() {
        let root = tempfile::tempdir().unwrap();
        let vault = root.path().join("vault");
        let data = root.path().join("data");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&data).unwrap();
        let events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                events,
                Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let expected = coordinator.current_authority_token().unwrap();
        let mut store = crate::services::knowledge_store::KnowledgeStore::with_event_recorder(
            vault,
            data,
            coordinator.clone(),
        );
        coordinator.fail_next_replays_before_targets(2);
        let error = store
            .create_note_expecting_authority(
                crate::models::note::NoteCreate {
                    title: "Wrapped Canvas export".to_string(),
                    content: "durable export".to_string(),
                    relative_path: Some("wrapped-canvas-export.md".to_string()),
                    aliases: Vec::new(),
                    status: crate::models::note::NoteStatus::Evidence,
                    tags: Vec::new(),
                    schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                    migration_source: None,
                    optimizer_managed: false,
                    properties: std::collections::HashMap::new(),
                },
                "canvas",
                expected,
            )
            .unwrap_err();
        assert!(error
            .downcast_ref::<crate::services::twin_events::MutationError>()
            .is_none());
        let exact = crate::services::knowledge_store::knowledge_authority_advanced_outcome(&error)
            .expect("KnowledgeStore must return its wrapped authority commit");

        let preserved = preserve_canvas_mutation_error(error);

        let repair_commit = preserved
            .repair_commit()
            .expect("the wrapped exact authority commit must survive the Canvas boundary");
        assert_eq!(repair_commit.mutation_id, exact.commit.mutation_id);
        assert_eq!(repair_commit.authority_token, exact.commit.authority_token);
        assert_eq!(
            repair_commit.authority_token,
            Some(coordinator.current_authority_token().unwrap())
        );
        assert!(!exact.target_aborted);
        assert!(preserved.to_string().contains("do not retry automatically"));
    }

    #[test]
    fn precommit_trace_failure_preserves_the_prior_response_commit() {
        let mutation_id = crate::services::twin_events::digest_bytes(b"canvas-response");
        let authority_token = crate::services::vault_namespace::VaultAuthorityTokenV1 {
            root_scope: crate::services::twin_events::digest_bytes(b"canvas-root"),
            lease_epoch_uuid: uuid::Uuid::nil().to_string(),
            authority_generation: 8,
        };
        let response_commit = crate::services::twin_events::MutationCommit {
            mutation_id: Some(mutation_id.clone()),
            events: Vec::new(),
            authority_token: Some(authority_token.clone()),
            postcommit_warning: false,
        };

        let preserved = CanvasTraceMutationError::precommit("trace failed before commit")
            .with_fallback_commit(response_commit);

        let repair_commit = preserved
            .repair_commit()
            .expect("the prior response commit must repair a precommit trace failure");
        assert_eq!(repair_commit.mutation_id.as_ref(), Some(&mutation_id));
        assert_eq!(
            repair_commit.authority_token.as_ref(),
            Some(&authority_token)
        );
        assert!(preserved.to_string().contains("do not retry automatically"));
    }

    #[test]
    fn newer_trace_abort_commit_wins_over_response_fallback() {
        let response_commit = crate::services::twin_events::MutationCommit {
            mutation_id: Some(crate::services::twin_events::digest_bytes(
                b"canvas-response",
            )),
            events: Vec::new(),
            authority_token: Some(crate::services::vault_namespace::VaultAuthorityTokenV1 {
                root_scope: crate::services::twin_events::digest_bytes(b"canvas-root"),
                lease_epoch_uuid: uuid::Uuid::nil().to_string(),
                authority_generation: 8,
            }),
            postcommit_warning: false,
        };
        let trace_mutation_id = crate::services::twin_events::digest_bytes(b"canvas-trace");
        let trace_authority = crate::services::vault_namespace::VaultAuthorityTokenV1 {
            root_scope: crate::services::twin_events::digest_bytes(b"canvas-root"),
            lease_epoch_uuid: uuid::Uuid::nil().to_string(),
            authority_generation: 9,
        };
        let trace_error = preserve_canvas_mutation_error(anyhow::Error::new(
            crate::services::twin_events::MutationError::AuthorityAdvanced {
                mutation_id: trace_mutation_id.clone(),
                authority_token: trace_authority.clone(),
                target_aborted: true,
                reason: "trace guard drift".to_string(),
            },
        ))
        .with_fallback_commit(response_commit);

        let repair_commit = trace_error.repair_commit().unwrap();
        assert_eq!(repair_commit.mutation_id.as_ref(), Some(&trace_mutation_id));
        assert_eq!(
            repair_commit.authority_token.as_ref(),
            Some(&trace_authority)
        );
    }

    #[test]
    fn governed_canvas_callers_preserve_exact_commits_before_repair() {
        let streaming = include_str!("streaming.rs");
        let reflection = source_between(
            streaming,
            "let (_, commit) = match twin_store.record_reflection_card_expecting_authority",
            "publication_commit = commit;",
        );
        assert!(reflection.contains("preserve_canvas_mutation_error"));
        assert!(reflection.contains("with_fallback_commit(publication_commit.clone())"));
        assert!(reflection.contains("repair_canvas_trace_error"));

        let context = include_str!("context.rs");
        let prediction = source_between(
            context,
            "pub(super) async fn run_sealed_twin_prediction",
            "fn build_twin_context_prompt",
        );
        assert!(prediction.contains("preserve_canvas_mutation_error"));
        assert!(prediction.contains("repair_canvas_trace_error"));
        assert!(prediction.contains("repair_after_authority_mutation"));
        assert!(!prediction.contains("repair_after_authority_token"));

        let session = include_str!("session.rs");
        let export = source_between(
            session,
            "pub async fn export_to_note",
            "Ok(serde_json::json!",
        );
        assert!(export.contains("preserve_canvas_mutation_error"));
        assert!(export.contains("repair_canvas_trace_error"));
    }

    #[test]
    fn required_trace_failures_emit_user_visible_non_retry_errors() {
        let streaming = include_str!("streaming.rs");
        for marker in [
            "Failed to append governed Canvas result trace",
            "Failed to persist governed models-added trace",
            "Failed to persist governed added-model result trace",
            "Failed to persist governed regeneration trace",
        ] {
            let branch = source_between(streaming, marker, "return;");
            assert!(
                branch.contains("emit_persistence_error"),
                "{marker} remains logs-only"
            );
        }

        let debate = include_str!("debate.rs");
        let start_failure = source_between(
            debate,
            "Canvas debate was saved, but its audit trace failed",
            "return Ok(debate_id);",
        );
        assert!(start_failure.contains("CanvasStreamEvent::DebateError"));

        let session = include_str!("session.rs");
        let export_failure = source_between(
            session,
            "Canvas note export committed, but its audit trace could not be appended",
            "return Ok(serde_json::json!",
        );
        assert!(export_failure.contains("CanvasStreamEvent::Error"));
    }

    #[tokio::test]
    async fn governed_trace_helper_preserves_the_full_exact_commit() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let vault = root.path().join("vault");
        let twin_root = data.join("twin/scope-one");
        std::fs::create_dir_all(&twin_root).unwrap();
        std::fs::create_dir(&vault).unwrap();
        let events = Arc::new(crate::services::twin_events::TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = Arc::new(
            crate::services::twin_events::MutationCoordinator::new(
                &data,
                &vault,
                events,
                Arc::new(crate::services::twin_events::NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let expected = coordinator.current_authority_token().unwrap();
        coordinator.fail_once_at(crate::services::twin_events::MutationFaultPoint::AfterStage);
        let store = Arc::new(RwLock::new(TwinStore::with_event_recorder(
            twin_root,
            data.join("twin"),
            coordinator,
        )));

        let commit = append_canvas_trace_expecting_authority(
            store,
            "session-exact-commit",
            TraceEventType::PromptSubmitted,
            serde_json::json!({ "prompt": "preserve warning" }),
            expected,
        )
        .await
        .unwrap();

        assert!(commit.mutation_id.is_some());
        assert!(commit.postcommit_warning);
        assert!(commit.authority_token.is_some());
    }

    #[test]
    fn model_route_uses_ollama_for_all_vault_context_when_configured() {
        let mut settings = crate::models::settings::UserSettings::default();
        settings.twin_llm_provider = "ollama".to_string();
        settings.ollama_model = "llama3.1:8b".to_string();

        let decision_route = resolve_model_route(
            &PromptType::Decision,
            &ContextMode::KnowledgeSearch,
            None,
            &settings,
        )
        .unwrap();
        let twin_route =
            resolve_model_route(&PromptType::Standard, &ContextMode::Twin, None, &settings)
                .unwrap();
        let twin_history_route = resolve_model_route(
            &PromptType::Standard,
            &ContextMode::TwinHistory,
            None,
            &settings,
        )
        .unwrap();
        let normal_route =
            resolve_model_route(&PromptType::Standard, &ContextMode::None, None, &settings)
                .unwrap();

        assert_eq!(decision_route.provider, ModelProviderRoute::Ollama);
        assert_eq!(decision_route.model_ids, vec!["llama3.1:8b".to_string()]);
        assert_eq!(twin_route.provider, ModelProviderRoute::Ollama);
        assert_eq!(twin_history_route.provider, ModelProviderRoute::Ollama);
        assert_eq!(normal_route.provider, ModelProviderRoute::OpenRouter);
        assert!(normal_route.model_ids.is_empty());
    }

    #[test]
    fn effective_model_ids_honors_requested_ollama_models() {
        let route = ModelRoute {
            provider: ModelProviderRoute::Ollama,
            model_ids: vec!["llama3.1:8b".to_string()],
        };

        let effective = effective_model_ids(&route, &["qwen3:14b".to_string()]);

        assert_eq!(effective, vec!["qwen3:14b".to_string()]);
    }

    #[test]
    fn effective_model_ids_falls_back_to_configured_ollama_model() {
        let route = ModelRoute {
            provider: ModelProviderRoute::Ollama,
            model_ids: vec!["llama3.1:8b".to_string()],
        };

        let effective = effective_model_ids(&route, &[]);

        assert_eq!(effective, vec!["llama3.1:8b".to_string()]);
    }

    #[test]
    fn model_route_allows_openrouter_for_vault_context_override() {
        let mut settings = crate::models::settings::UserSettings::default();
        settings.twin_llm_provider = "ollama".to_string();
        settings.ollama_model = "llama3.1:8b".to_string();

        let route = resolve_model_route(
            &PromptType::Decision,
            &ContextMode::Twin,
            Some("openrouter"),
            &settings,
        )
        .unwrap();

        assert_eq!(route.provider, ModelProviderRoute::OpenRouter);
        assert!(route.model_ids.is_empty());
    }

    #[test]
    fn model_route_fails_closed_when_local_twin_model_is_missing() {
        let mut settings = crate::models::settings::UserSettings::default();
        settings.twin_llm_provider = "ollama".to_string();

        let error = resolve_model_route(
            &PromptType::Decision,
            &ContextMode::KnowledgeSearch,
            None,
            &settings,
        )
        .unwrap_err();

        assert!(error.contains("Select an Ollama model"));
    }

    #[test]
    fn model_route_rejects_an_unknown_explicit_provider() {
        let mut settings = crate::models::settings::UserSettings::default();
        settings.twin_llm_provider = "openrouter".to_string();

        let error = resolve_model_route(
            &PromptType::Standard,
            &ContextMode::TwinHistory,
            Some("ollmaa"),
            &settings,
        )
        .unwrap_err();

        assert!(error.contains("Unsupported Canvas provider"));
    }

    #[test]
    fn model_route_honors_an_explicit_local_provider_for_plain_companion_prompts() {
        let mut settings = crate::models::settings::UserSettings::default();
        settings.twin_llm_provider = "openrouter".to_string();
        settings.ollama_model = "llama3.1:8b".to_string();

        let route = resolve_model_route(
            &PromptType::Standard,
            &ContextMode::None,
            Some("ollama"),
            &settings,
        )
        .unwrap();

        assert_eq!(route.provider, ModelProviderRoute::Ollama);
        assert_eq!(route.model_ids, vec!["llama3.1:8b".to_string()]);
    }
}
