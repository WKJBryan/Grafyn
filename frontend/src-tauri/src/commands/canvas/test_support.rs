//! Test-only builder helpers shared by more than one canvas submodule's test
//! suite (currently `context` and `streaming`). Kept separate from either
//! module so neither has to duplicate `build_tile`/`build_response`.
//! Gated by the `#[cfg(test)] mod test_support;` declaration in `mod.rs`.

use crate::models::canvas::{
    ContextMode, ModelResponse, PromptTile, PromptType, ResponseStatus, TilePosition,
    TwinAnswerMode,
};
use std::collections::HashMap;

pub(super) fn build_response(model_id: &str, content: &str) -> ModelResponse {
    ModelResponse {
        id: format!("resp-{}", model_id),
        model_id: model_id.to_string(),
        model_name: model_id.to_string(),
        content: content.to_string(),
        status: ResponseStatus::Completed,
        error: None,
        tokens_used: None,
        cost_usd: None,
        created_at: chrono::Utc::now(),
        position: TilePosition::default(),
    }
}

pub(super) fn build_tile(
    id: &str,
    prompt: &str,
    model_id: &str,
    response: &str,
    parent_tile_id: Option<&str>,
    parent_model_id: Option<&str>,
) -> PromptTile {
    let mut responses = HashMap::new();
    responses.insert(model_id.to_string(), build_response(model_id, response));

    PromptTile {
        id: id.to_string(),
        prompt_type: PromptType::Standard,
        prompt: prompt.to_string(),
        system_prompt: None,
        models: vec![model_id.to_string()],
        responses,
        position: TilePosition::default(),
        created_at: chrono::Utc::now(),
        context_mode: ContextMode::default(),
        parent_tile_id: parent_tile_id.map(str::to_string),
        parent_model_id: parent_model_id.map(str::to_string),
        context_notes: Vec::new(),
        approved_twin_records: Vec::new(),
        candidate_twin_records: Vec::new(),
        twin_answer_mode: TwinAnswerMode::default(),
        twin_context_policy: None,
        twin_llm_provider: None,
        decision_metadata: None,
        decision_episode_id: None,
        web_search: false,
        web_search_max_results: 5,
        reasoning_effort: "none".to_string(),
    }
}
