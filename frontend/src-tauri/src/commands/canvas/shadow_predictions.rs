//! Comparator ("shadow") sealed predictions, run after the primary sealed call
//! in the same hidden task when `twin_shadow_predictions_enabled` is on. They
//! exist only so the external evaluation harness can measure skill above
//! baseline; nothing in-app reads them.

use super::context::SEALED_PREDICTION_TEMPERATURE;
use super::shared::ModelProviderRoute;
use crate::services::ollama::OllamaService;
use crate::services::openrouter::{ChatMessage, OpenRouterChatRequest, OpenRouterService};
use crate::services::twin::{parse_twin_prediction, TwinStore};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Max tokens for the reasoning-on comparator, so reasoning does not
/// truncate the JSON answer.
const REASONING_ON_MAX_TOKENS: u32 = 4000;

/// Seal the `no_context` (same request, no system prompt) and `reasoning_on`
/// (primary request with reasoning effort "medium"; OpenRouter only)
/// comparators. Each failure is only logged. Lock discipline: no store lock
/// is held across a provider await.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_shadow_predictions(
    twin_store: &Arc<RwLock<TwinStore>>,
    openrouter: &Arc<RwLock<OpenRouterService>>,
    ollama: &Arc<RwLock<OllamaService>>,
    provider_route: &ModelProviderRoute,
    model: &str,
    episode_id: &str,
    options: &[String],
    context_version: &str,
    system_prompt: &str,
    user_message: &str,
    primary_reasoning_effort: Option<&str>,
) {
    let messages = || {
        vec![ChatMessage {
            role: "user".to_string(),
            content: user_message.to_string(),
        }]
    };

    let no_context = match provider_route {
        ModelProviderRoute::Ollama => {
            let ollama = ollama.read().await;
            ollama
                .chat(model, messages(), None, Some(SEALED_PREDICTION_TEMPERATURE))
                .await
        }
        ModelProviderRoute::OpenRouter => {
            let openrouter = openrouter.read().await;
            openrouter
                .chat(OpenRouterChatRequest {
                    model,
                    messages: messages(),
                    system_prompt: None,
                    temperature: Some(SEALED_PREDICTION_TEMPERATURE),
                    max_tokens: Some(600),
                    reasoning_effort: primary_reasoning_effort,
                    web_search: false,
                    web_search_max_results: 0,
                })
                .await
        }
    };
    seal_shadow(
        twin_store,
        episode_id,
        "no_context",
        no_context,
        options,
        model,
        context_version,
        user_message,
        primary_reasoning_effort,
    )
    .await;

    if *provider_route != ModelProviderRoute::OpenRouter {
        return;
    }
    let reasoning_on = {
        let openrouter = openrouter.read().await;
        openrouter
            .chat(OpenRouterChatRequest {
                model,
                messages: messages(),
                system_prompt: Some(system_prompt),
                temperature: Some(SEALED_PREDICTION_TEMPERATURE),
                max_tokens: Some(REASONING_ON_MAX_TOKENS),
                reasoning_effort: Some("medium"),
                web_search: false,
                web_search_max_results: 0,
            })
            .await
    };
    seal_shadow(
        twin_store,
        episode_id,
        "reasoning_on",
        reasoning_on,
        options,
        model,
        context_version,
        user_message,
        Some("medium"),
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn seal_shadow(
    twin_store: &Arc<RwLock<TwinStore>>,
    episode_id: &str,
    variant: &str,
    result: anyhow::Result<String>,
    options: &[String],
    model: &str,
    context_version: &str,
    user_message: &str,
    reasoning_effort: Option<&str>,
) {
    let raw = match result {
        Ok(raw) => raw,
        Err(error) => {
            log::warn!("Shadow prediction {variant} call failed for {episode_id}: {error}");
            return;
        }
    };
    let draft = parse_twin_prediction(&raw, options);
    let mut store = twin_store.write().await;
    if let Err(error) = store.attach_shadow_prediction(
        episode_id,
        variant,
        draft,
        model,
        context_version,
        Some(user_message),
        Some(SEALED_PREDICTION_TEMPERATURE as f32),
        reasoning_effort,
    ) {
        log::warn!("Failed to seal {variant} shadow prediction for {episode_id}: {error}");
    }
}
