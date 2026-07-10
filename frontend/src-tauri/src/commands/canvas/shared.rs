use crate::models::canvas::{CanvasSession, ContextMode, PromptType};
use crate::models::settings::UserSettings;
use crate::models::twin::TraceEventType;
use crate::services::twin_store::TwinStore;
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

pub(super) fn resolve_model_route(
    prompt_type: &PromptType,
    context_mode: &ContextMode,
    twin_provider_override: Option<&str>,
    settings: &UserSettings,
) -> Result<ModelRoute, String> {
    let twin_provider = twin_provider_override
        .map(|provider| provider.trim().to_ascii_lowercase())
        .filter(|provider| provider == "ollama" || provider == "openrouter")
        .unwrap_or_else(|| settings.twin_llm_provider.to_ascii_lowercase());

    if is_vault_context_prompt(prompt_type, context_mode) && twin_provider == "ollama" {
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

pub(super) async fn append_canvas_trace(
    twin_store_arc: Arc<RwLock<TwinStore>>,
    session_id: &str,
    event_type: TraceEventType,
    payload: serde_json::Value,
) {
    let mut twin_store = twin_store_arc.write().await;
    if let Err(error) = twin_store.append_trace_event(session_id, event_type, payload) {
        log::error!(
            "Failed to append twin trace for session '{}': {}",
            session_id,
            error
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let normal_route =
            resolve_model_route(&PromptType::Standard, &ContextMode::None, None, &settings)
                .unwrap();

        assert_eq!(decision_route.provider, ModelProviderRoute::Ollama);
        assert_eq!(decision_route.model_ids, vec!["llama3.1:8b".to_string()]);
        assert_eq!(twin_route.provider, ModelProviderRoute::Ollama);
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
}
