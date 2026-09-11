//! Session working memory and conversation compaction for canvas prompts.
//!
//! Compact mode injects compiled session state plus the last two parent-chain
//! turns. It does not truncate older turns to a character budget.

use super::streaming::CanvasEventSink;
use crate::models::canvas::{
    CanvasSession, CanvasStreamEvent, CanvasWorkingMemory, ContextMode, ModelPosition,
    PromptRequest,
};
use crate::models::twin::TraceEventType;
use crate::services::canvas_store::CanvasStore;
use crate::services::ollama::OllamaService;
use crate::services::openrouter::{ChatMessage, OpenRouterService};
use crate::services::settings::SettingsService;
use crate::services::twin::TwinStore;
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

pub(super) const COMPACT_HISTORY_RECENT_TURNS: usize = 2;
const WORKING_MEMORY_TOKEN_BUDGET: usize = 800;

#[derive(Debug, Clone)]
pub(super) struct ConversationTurn {
    pub(super) prompt: String,
    pub(super) response: String,
    pub(super) _model_id: String,
}

pub(super) fn branch_memory_key(tile_id: &str, model_id: &str) -> String {
    format!("{tile_id}::{model_id}")
}

pub(super) fn memory_for_follow_up(
    session: &CanvasSession,
    request: &PromptRequest,
) -> CanvasWorkingMemory {
    match (&request.parent_tile_id, &request.parent_model_id) {
        (Some(tile_id), Some(model_id)) => session
            .branch_memories
            .get(&branch_memory_key(tile_id, model_id))
            .cloned()
            .unwrap_or_default(),
        _ => CanvasWorkingMemory::default(),
    }
}

fn debate_recap_message(session: &CanvasSession, debate_id: &str) -> Option<ChatMessage> {
    session
        .debates
        .iter()
        .find(|debate| debate.id == debate_id)
        .and_then(|debate| debate.recap.clone())
        .filter(|recap| !recap.trim().is_empty())
        .map(|recap| ChatMessage {
            role: "user".to_string(),
            content: recap,
        })
}

pub(super) fn build_canvas_messages(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ChatMessage>, String> {
    if let Some(debate_id) = request.parent_debate_id.as_deref() {
        let mut messages = Vec::new();
        if let Some(recap) = debate_recap_message(session, debate_id) {
            messages.push(recap);
        }
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: request.prompt.clone(),
        });
        return Ok(messages);
    }

    match request.context_mode {
        ContextMode::None => Ok(vec![ChatMessage {
            role: "user".to_string(),
            content: request.prompt.clone(),
        }]),
        ContextMode::FullHistory => build_full_history_messages(session, request),
        ContextMode::Compact => build_compact_history_messages(session, request),
        ContextMode::KnowledgeSearch | ContextMode::Semantic | ContextMode::Twin => {
            if request.parent_tile_id.is_some() && request.parent_model_id.is_some() {
                build_recent_turn_messages(session, request)
            } else {
                Ok(vec![ChatMessage {
                    role: "user".to_string(),
                    content: request.prompt.clone(),
                }])
            }
        }
        ContextMode::TwinHistory => {
            if request.parent_tile_id.is_none() && request.parent_model_id.is_none() {
                Ok(vec![ChatMessage {
                    role: "user".to_string(),
                    content: request.prompt.clone(),
                }])
            } else {
                build_compact_history_messages(session, request)
            }
        }
    }
}

pub(super) fn compose_system_prompt(
    session: &CanvasSession,
    request: &PromptRequest,
    inner: Option<String>,
) -> Option<String> {
    if request.context_mode == ContextMode::None {
        return request
            .system_prompt
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }

    let mut parts = Vec::new();
    if let Some(debate_id) = request.parent_debate_id.as_deref() {
        if let Some(recap) = session
            .debates
            .iter()
            .find(|debate| debate.id == debate_id)
            .and_then(|debate| debate.recap.as_deref())
            .map(str::trim)
            .filter(|recap| !recap.is_empty())
        {
            parts.push(recap.to_string());
        }
    } else if let Some(memory) =
        format_working_memory_for_prompt(&memory_for_follow_up(session, request))
    {
        parts.push(memory);
    }
    if let Some(inner) = inner
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        parts.push(inner);
    }
    if let Some(user) = request
        .system_prompt
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        parts.push(user);
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

pub(super) fn format_working_memory_for_prompt(memory: &CanvasWorkingMemory) -> Option<String> {
    if memory.is_empty() {
        return None;
    }

    let mut sections = Vec::new();
    let mut tried = memory.tried.clone();
    let mut constraints = memory.constraints.clone();
    let mut open_questions = memory.open_questions.clone();
    let mut decisions = memory.decisions.clone();
    let mut positions = memory.model_positions.clone();

    let mut body = String::from("## Session working memory\n");
    if !memory.question.trim().is_empty() {
        body.push_str(&format!("\nQuestion: {}\n", memory.question.trim()));
    }
    if !memory.summary.trim().is_empty() {
        body.push_str(&format!("\nSummary:\n{}\n", memory.summary.trim()));
    }

    loop {
        let mut candidate = body.clone();
        append_list(&mut candidate, "Constraints", &constraints);
        append_list(&mut candidate, "Open questions", &open_questions);
        append_list(&mut candidate, "Decisions", &decisions);
        append_positions(&mut candidate, &positions);
        append_list(&mut candidate, "Tried", &tried);

        if estimate_tokens(&candidate) <= WORKING_MEMORY_TOKEN_BUDGET {
            sections.push(candidate);
            break;
        }

        if !tried.is_empty() {
            tried.pop();
            continue;
        }
        if constraints.len() > 1 {
            constraints.pop();
            continue;
        }
        if open_questions.len() > 1 {
            open_questions.pop();
            continue;
        }
        if decisions.len() > 1 {
            decisions.pop();
            continue;
        }
        if positions.len() > 1 {
            positions.pop();
            continue;
        }
        sections.push(candidate);
        break;
    }

    sections.pop()
}

pub(super) fn rewrite_retrieval_query(prompt: &str, memory: &CanvasWorkingMemory) -> String {
    let mut parts = vec![prompt.trim().to_string()];
    if !memory.question.trim().is_empty() {
        parts.push(memory.question.trim().to_string());
    }
    parts.extend(
        memory
            .open_questions
            .iter()
            .chain(memory.constraints.iter())
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty()),
    );
    parts.join(" ")
}

pub(super) fn parse_working_memory_response(
    raw: &str,
    previous: &CanvasWorkingMemory,
    compiled_from_tile_id: Option<String>,
) -> CanvasWorkingMemory {
    match extract_memory_json(raw) {
        Some(parsed) => merge_working_memory(previous, parsed, compiled_from_tile_id),
        None => previous.clone(),
    }
}

fn merge_working_memory(
    previous: &CanvasWorkingMemory,
    mut parsed: CanvasWorkingMemory,
    compiled_from_tile_id: Option<String>,
) -> CanvasWorkingMemory {
    parsed.version = previous.version.saturating_add(1).max(1);
    parsed.updated_at = Some(Utc::now());
    parsed.compiled_from_tile_id = compiled_from_tile_id.or(parsed.compiled_from_tile_id);
    if parsed.question.trim().is_empty() {
        parsed.question = previous.question.clone();
    }
    if parsed.summary.trim().is_empty() {
        parsed.summary = previous.summary.clone();
    }
    parsed
}

fn extract_memory_json(raw: &str) -> Option<CanvasWorkingMemory> {
    let trimmed = raw.trim();
    let json_slice = if let Some(start) = trimmed.find('{') {
        let end = trimmed.rfind('}')?;
        &trimmed[start..=end]
    } else {
        return None;
    };
    serde_json::from_str::<Value>(json_slice)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn build_full_history_messages(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ChatMessage>, String> {
    let turns = build_selected_parent_chain(session, request)?;
    Ok(turns_to_messages(turns, &request.prompt, None))
}

fn build_compact_history_messages(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ChatMessage>, String> {
    let turns = build_selected_parent_chain(session, request)?;
    let recent = recent_turns(&turns);
    let memory_prefix = format_working_memory_for_prompt(&memory_for_follow_up(session, request));
    Ok(turns_to_messages(
        recent.to_vec(),
        &request.prompt,
        memory_prefix,
    ))
}

fn build_recent_turn_messages(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ChatMessage>, String> {
    let turns = build_selected_parent_chain(session, request)?;
    let recent = recent_turns(&turns);
    Ok(turns_to_messages(recent.to_vec(), &request.prompt, None))
}

fn recent_turns(turns: &[ConversationTurn]) -> &[ConversationTurn] {
    if turns.len() > COMPACT_HISTORY_RECENT_TURNS {
        &turns[turns.len() - COMPACT_HISTORY_RECENT_TURNS..]
    } else {
        turns
    }
}

fn turns_to_messages(
    turns: Vec<ConversationTurn>,
    new_prompt: &str,
    memory_prefix: Option<String>,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    if let Some(prefix) = memory_prefix {
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: prefix,
        });
    }
    for turn in turns {
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: turn.prompt,
        });
        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: turn.response,
        });
    }
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: new_prompt.to_string(),
    });
    messages
}

pub(super) fn build_selected_parent_chain(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ConversationTurn>, String> {
    let mut tile_id = request
        .parent_tile_id
        .clone()
        .ok_or_else(|| "Context mode requires a parent tile".to_string())?;
    let mut model_id = request
        .parent_model_id
        .clone()
        .ok_or_else(|| "Context mode requires a parent model".to_string())?;
    let mut visited = HashSet::new();
    let mut turns = Vec::new();

    loop {
        let visit_key = format!("{}::{}", tile_id, model_id);
        if !visited.insert(visit_key) {
            return Err("Detected a cycle while reconstructing canvas history".to_string());
        }

        let tile = session
            .prompt_tiles
            .iter()
            .find(|t| t.id == tile_id)
            .ok_or_else(|| format!("Parent tile not found in session: {}", tile_id))?;
        let response = tile.responses.get(&model_id).ok_or_else(|| {
            format!(
                "Parent response not found for tile {} and model {}",
                tile_id, model_id
            )
        })?;

        turns.push(ConversationTurn {
            prompt: tile.prompt.clone(),
            response: response.content.clone(),
            _model_id: model_id.clone(),
        });

        match (&tile.parent_tile_id, &tile.parent_model_id) {
            (Some(next_tile_id), Some(next_model_id)) => {
                tile_id = next_tile_id.clone();
                model_id = next_model_id.clone();
            }
            (None, None) => break,
            _ => {
                return Err(format!(
                    "Incomplete parent linkage for tile {} while reconstructing history",
                    tile.id
                ))
            }
        }
    }

    turns.reverse();
    Ok(turns)
}

fn append_list(body: &mut String, heading: &str, items: &[String]) {
    let kept: Vec<&str> = items
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .collect();
    if kept.is_empty() {
        return;
    }
    body.push_str(&format!("\n{heading}:\n"));
    for item in kept {
        body.push_str(&format!("- {}\n", item));
    }
}

fn append_positions(body: &mut String, positions: &[ModelPosition]) {
    if positions.is_empty() {
        return;
    }
    body.push_str("\nModel positions:\n");
    for position in positions {
        let stance = position.stance.trim();
        if stance.is_empty() {
            continue;
        }
        body.push_str(&format!("- {}: {}\n", position.model_id, stance));
    }
}

fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace().count() * 4 / 3 + 1
}

pub(super) fn spawn_working_memory_compile<S: CanvasEventSink>(
    window: S,
    session_id: String,
    tile_id: String,
    model_id: String,
    canvas_store: Arc<RwLock<CanvasStore>>,
    twin_store: Arc<RwLock<TwinStore>>,
    openrouter: Arc<RwLock<OpenRouterService>>,
    ollama: Option<Arc<RwLock<OllamaService>>>,
    settings: Arc<RwLock<SettingsService>>,
) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = compile_session_working_memory(
            window,
            session_id,
            tile_id,
            model_id,
            canvas_store,
            twin_store,
            openrouter,
            ollama,
            settings,
        )
        .await
        {
            log::warn!("Canvas working-memory compile failed: {error}");
        }
    });
}

async fn compile_session_working_memory<S: CanvasEventSink>(
    window: S,
    session_id: String,
    tile_id: String,
    model_id: String,
    canvas_store: Arc<RwLock<CanvasStore>>,
    twin_store: Arc<RwLock<TwinStore>>,
    openrouter: Arc<RwLock<OpenRouterService>>,
    ollama: Option<Arc<RwLock<OllamaService>>>,
    settings: Arc<RwLock<SettingsService>>,
) -> Result<(), String> {
    let session = {
        let mut store = canvas_store.write().await;
        store.get_session(&session_id).map_err(|e| e.to_string())?
    };
    let branch_key = branch_memory_key(&tile_id, &model_id);
    let previous = session
        .branch_memories
        .get(&branch_key)
        .cloned()
        .unwrap_or_default();
    if previous.compiled_from_tile_id.as_deref() == Some(tile_id.as_str()) && !previous.is_empty() {
        return Ok(());
    }

    let compile_user = build_compile_user_prompt(&session, &tile_id, &model_id);
    let settings_snapshot = settings.read().await.get().clone();
    let use_ollama = settings_snapshot
        .twin_llm_provider
        .eq_ignore_ascii_case("ollama");

    let raw = if use_ollama {
        let model = settings_snapshot.ollama_model.trim();
        if model.is_empty() {
            return Err("Select an Ollama model before compiling session memory".to_string());
        }
        let ollama =
            ollama.ok_or_else(|| "Local Ollama is unavailable on this runtime".to_string())?;
        let ollama = ollama.read().await;
        ollama
            .chat(
                model,
                vec![ChatMessage {
                    role: "user".to_string(),
                    content: compile_user,
                }],
                Some(COMPILE_SYSTEM_PROMPT),
                Some(0.2),
            )
            .await
            .map_err(|e| e.to_string())?
    } else {
        let model = settings_snapshot.llm_model.trim();
        if model.is_empty() {
            return Err("No compile model configured".to_string());
        }
        let openrouter = openrouter.read().await;
        openrouter
            .chat(
                model,
                vec![ChatMessage {
                    role: "user".to_string(),
                    content: compile_user,
                }],
                Some(COMPILE_SYSTEM_PROMPT),
                Some(0.2),
                Some(800),
                Some("none"),
                false,
                0,
            )
            .await
            .map_err(|e| e.to_string())?
    };

    let compiled = parse_working_memory_response(&raw, &previous, Some(tile_id.clone()));
    if compiled.is_empty() {
        return Ok(());
    }

    {
        let mut store = canvas_store.write().await;
        store
            .update_branch_memory(&session_id, &branch_key, compiled.clone())
            .map_err(|e| e.to_string())?;
    }

    let _ = window.emit_canvas(CanvasStreamEvent::WorkingMemoryUpdated {
        session_id: session_id.clone(),
        working_memory: compiled.clone(),
    });

    let mut twin = twin_store.write().await;
    let _ = twin.append_trace_event(
        &session_id,
        TraceEventType::WorkingMemoryCompiled,
        json!({
            "tile_id": tile_id,
            "model_id": model_id,
            "version": compiled.version,
            "summary": compiled.summary.chars().take(240).collect::<String>(),
        }),
    );

    Ok(())
}

const COMPILE_SYSTEM_PROMPT: &str = "\
You compile a short working-memory document for a multi-model canvas session. \
Return ONLY JSON with keys: question, summary, constraints, open_questions, decisions, tried, model_positions, note_ids. \
model_positions is an array of {model_id, stance}. \
Keep summary under 120 words. Do not invent facts. Do not write constitution or identity claims.";

fn build_compile_user_prompt(session: &CanvasSession, tile_id: &str, model_id: &str) -> String {
    let previous = session
        .branch_memories
        .get(&branch_memory_key(tile_id, model_id))
        .cloned()
        .unwrap_or_default();
    let mut body = String::from("Previous working memory JSON for this branch only:\n");
    body.push_str(&serde_json::to_string_pretty(&previous).unwrap_or_else(|_| "{}".to_string()));
    body.push('\n');

    if let Some(tile) = session.prompt_tiles.iter().find(|tile| tile.id == tile_id) {
        body.push_str("\nLatest prompt:\n");
        body.push_str(&tile.prompt);
        if let Some(response) = tile.responses.get(model_id) {
            body.push_str(&format!(
                "\nThis model's answer ({model_id}):\n{}\n",
                response.content
            ));
        }
        body.push_str("\nDo not include sibling models on the same prompt.\n");
    }

    body
}

#[cfg(test)]
mod tests {
    use super::super::test_support::build_tile;
    use super::*;
    use crate::models::canvas::PromptTile;
    use crate::models::canvas::{CanvasViewport, PromptType, TwinAnswerMode};

    fn build_session(tiles: Vec<PromptTile>) -> CanvasSession {
        CanvasSession {
            id: "session-1".to_string(),
            title: "Canvas".to_string(),
            description: None,
            prompt_tiles: tiles,
            debates: Vec::new(),
            viewport: CanvasViewport::default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tags: Vec::new(),
            status: "draft".to_string(),
            pinned_note_ids: Vec::new(),
            working_memory: CanvasWorkingMemory::default(),
            branch_memories: std::collections::HashMap::new(),
        }
    }

    fn build_request(
        prompt: &str,
        parent_tile_id: &str,
        parent_model_id: &str,
        context_mode: ContextMode,
    ) -> PromptRequest {
        PromptRequest {
            prompt: prompt.to_string(),
            prompt_type: PromptType::Standard,
            system_prompt: None,
            models: vec!["openai/gpt-4".to_string()],
            position: None,
            context_mode,
            twin_answer_mode: TwinAnswerMode::default(),
            twin_relationship_variant: crate::models::twin_state::RelationshipVariant::global(),
            twin_context_policy: None,
            twin_llm_provider: None,
            decision_metadata: None,
            parent_tile_id: Some(parent_tile_id.to_string()),
            parent_model_id: Some(parent_model_id.to_string()),
            parent_debate_id: None,
            temperature: 0.7,
            max_tokens: None,
            web_search: false,
            web_search_max_results: 5,
            reasoning_effort: "none".to_string(),
        }
    }

    fn four_turn_session() -> CanvasSession {
        build_session(vec![
            build_tile(
                "tile-1",
                "Prompt 1",
                "openai/gpt-4",
                "Response 1",
                None,
                None,
            ),
            build_tile(
                "tile-2",
                "Prompt 2",
                "openai/gpt-4",
                "Response 2",
                Some("tile-1"),
                Some("openai/gpt-4"),
            ),
            build_tile(
                "tile-3",
                "Prompt 3",
                "openai/gpt-4",
                "Response 3",
                Some("tile-2"),
                Some("openai/gpt-4"),
            ),
            build_tile(
                "tile-4",
                "Prompt 4",
                "openai/gpt-4",
                "Response 4",
                Some("tile-3"),
                Some("openai/gpt-4"),
            ),
        ])
    }

    #[test]
    fn compact_without_working_memory_keeps_only_recent_turns() {
        let session = four_turn_session();
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::Compact);
        let messages = build_compact_history_messages(&session, &request).unwrap();

        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].content, "Prompt 3");
        assert_eq!(messages[1].content, "Response 3");
        assert_eq!(messages[2].content, "Prompt 4");
        assert_eq!(messages[3].content, "Response 4");
        assert_eq!(messages[4].content, "Prompt 5");
        assert!(!messages.iter().any(|message| message
            .content
            .contains("Conversation summary before the most recent turns")));
        assert!(!messages
            .iter()
            .any(|message| message.content.contains("Prompt 1")));
        assert!(!messages
            .iter()
            .any(|message| message.content.contains("Response 1")));
    }

    #[test]
    fn compact_with_working_memory_uses_compiled_summary_not_truncation() {
        let mut session = four_turn_session();
        session.branch_memories.insert(
            branch_memory_key("tile-4", "openai/gpt-4"),
            CanvasWorkingMemory {
                version: 2,
                question: "Should we compact instead of truncate?".to_string(),
                summary: "Earlier turns decided compiled state beats 240-character chops."
                    .to_string(),
                model_positions: vec![ModelPosition {
                    model_id: "openai/gpt-4".to_string(),
                    stance: "Prefer a session summary.".to_string(),
                }],
                ..CanvasWorkingMemory::default()
            },
        );
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::Compact);
        let messages = build_compact_history_messages(&session, &request).unwrap();

        assert!(messages[0]
            .content
            .contains("Should we compact instead of truncate?"));
        assert!(messages[0]
            .content
            .contains("compiled state beats 240-character chops"));
        assert!(!messages[0].content.contains("Response 1"));
        assert_eq!(messages[1].content, "Prompt 3");
        assert_eq!(messages[2].content, "Response 3");
        assert_eq!(messages.last().unwrap().content, "Prompt 5");
        assert!(!messages.iter().any(|message| message
            .content
            .contains("Conversation summary before the most recent turns")));
    }

    #[test]
    fn knowledge_search_branch_includes_recent_turns() {
        let session = four_turn_session();
        let request = build_request(
            "What about the other option?",
            "tile-4",
            "openai/gpt-4",
            ContextMode::KnowledgeSearch,
        );
        let messages = build_canvas_messages(&session, &request).unwrap();

        assert_eq!(messages[0].content, "Prompt 3");
        assert_eq!(messages[1].content, "Response 3");
        assert_eq!(
            messages.last().unwrap().content,
            "What about the other option?"
        );
        assert!(!messages
            .iter()
            .any(|message| message.content.contains("Prompt 1")));
    }

    #[test]
    fn none_mode_is_only_the_new_prompt() {
        let session = four_turn_session();
        let request = PromptRequest {
            prompt: "Fresh root prompt".to_string(),
            prompt_type: PromptType::Standard,
            system_prompt: None,
            models: vec!["openai/gpt-4".to_string()],
            position: None,
            context_mode: ContextMode::None,
            twin_answer_mode: TwinAnswerMode::default(),
            twin_relationship_variant: crate::models::twin_state::RelationshipVariant::global(),
            twin_context_policy: None,
            twin_llm_provider: None,
            decision_metadata: None,
            parent_tile_id: None,
            parent_model_id: None,
            parent_debate_id: None,
            temperature: 0.7,
            max_tokens: None,
            web_search: false,
            web_search_max_results: 5,
            reasoning_effort: "none".to_string(),
        };
        let messages = build_canvas_messages(&session, &request).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Fresh root prompt");
    }

    #[test]
    fn rewrite_retrieval_query_appends_working_memory_question() {
        let memory = CanvasWorkingMemory {
            question: "Mirofish social architecture".to_string(),
            open_questions: vec!["TTL vs manual strobes".to_string()],
            constraints: vec!["Stay under $500".to_string()],
            ..CanvasWorkingMemory::default()
        };
        let query = rewrite_retrieval_query("the other option", &memory);
        assert!(query.contains("the other option"));
        assert!(query.contains("Mirofish social architecture"));
        assert!(query.contains("TTL vs manual strobes"));
        assert!(query.contains("Stay under $500"));
    }

    #[test]
    fn parse_working_memory_response_merges_valid_json() {
        let previous = CanvasWorkingMemory {
            version: 1,
            question: "Old question".to_string(),
            summary: "Old summary".to_string(),
            ..CanvasWorkingMemory::default()
        };
        let raw = r#"Here you go:
{"question":"Should we compact?","summary":"Use compiled state.","constraints":["Keep Twin review-gated"]}"#;
        let parsed = parse_working_memory_response(raw, &previous, Some("tile-9".to_string()));
        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.question, "Should we compact?");
        assert_eq!(parsed.summary, "Use compiled state.");
        assert_eq!(
            parsed.constraints,
            vec!["Keep Twin review-gated".to_string()]
        );
        assert_eq!(parsed.compiled_from_tile_id.as_deref(), Some("tile-9"));
    }

    #[test]
    fn parse_working_memory_response_keeps_previous_on_garbage() {
        let previous = CanvasWorkingMemory {
            version: 3,
            summary: "Keep me".to_string(),
            ..CanvasWorkingMemory::default()
        };
        let parsed =
            parse_working_memory_response("not json at all", &previous, Some("tile-1".to_string()));
        assert_eq!(parsed.version, 3);
        assert_eq!(parsed.summary, "Keep me");
    }

    #[test]
    fn reply_from_debate_uses_recap_not_sibling_branches() {
        let mut session = four_turn_session();
        session.debates.push(crate::models::canvas::Debate {
            id: "debate-1".to_string(),
            recap: Some("The leftover fight is rent versus moving.".to_string()),
            ..crate::models::canvas::Debate::default()
        });
        let request = PromptRequest {
            prompt: "Draft the email.".to_string(),
            prompt_type: PromptType::Standard,
            system_prompt: None,
            models: vec!["openai/gpt-4".to_string()],
            position: None,
            context_mode: ContextMode::None,
            twin_answer_mode: TwinAnswerMode::default(),
            twin_relationship_variant: crate::models::twin_state::RelationshipVariant::global(),
            twin_context_policy: None,
            twin_llm_provider: None,
            decision_metadata: None,
            parent_tile_id: None,
            parent_model_id: None,
            parent_debate_id: Some("debate-1".to_string()),
            temperature: 0.7,
            max_tokens: None,
            web_search: false,
            web_search_max_results: 5,
            reasoning_effort: "none".to_string(),
        };
        let messages = build_canvas_messages(&session, &request).unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages[0].content.contains("rent versus moving"));
        assert_eq!(messages[1].content, "Draft the email.");
        assert!(!messages
            .iter()
            .any(|message| message.content.contains("Prompt 1")));
    }

    #[test]
    fn parent_chain_returns_root_to_leaf_order() {
        let session = four_turn_session();
        let request = build_request("Newest", "tile-4", "openai/gpt-4", ContextMode::FullHistory);
        let turns = build_selected_parent_chain(&session, &request).unwrap();
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[0].prompt, "Prompt 1");
        assert_eq!(turns[3].prompt, "Prompt 4");
    }

    #[test]
    fn full_history_interleaves_user_and_assistant_turns() {
        let session = build_session(vec![
            build_tile(
                "tile-1",
                "Root prompt",
                "openai/gpt-4",
                "Root response",
                None,
                None,
            ),
            build_tile(
                "tile-2",
                "Branch prompt",
                "openai/gpt-4",
                "Branch response",
                Some("tile-1"),
                Some("openai/gpt-4"),
            ),
        ]);
        let request = build_request(
            "Final prompt",
            "tile-2",
            "openai/gpt-4",
            ContextMode::FullHistory,
        );
        let messages = build_full_history_messages(&session, &request).unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].content, "Root prompt");
        assert_eq!(messages[1].content, "Root response");
        assert_eq!(messages[4].content, "Final prompt");
    }
}
