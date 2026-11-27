//! Session working memory and conversation compaction for canvas prompts.
//!
//! Compact mode injects compiled session state plus the last two parent-chain
//! turns. It does not truncate older turns to a character budget.
//! KnowledgeSearch, Semantic, and Twin follow-ups send the full parent chain
//! when estimated conversation tokens are below the compact threshold, and
//! compact on demand into a dreamed document once the chain is at/over it.

use crate::models::canvas::{
    CanvasSession, CanvasStreamEvent, CanvasWorkingMemory, ContextMode, ModelPosition, PromptRequest,
};
use crate::models::twin::TraceEventType;
use crate::services::openrouter::{ChatMessage, OpenRouterChatRequest};
use crate::AppState;
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;

pub(super) const COMPACT_HISTORY_RECENT_TURNS: usize = 2;
const WORKING_MEMORY_TOKEN_BUDGET: usize = 800;
const COMPACT_LLM_TIMEOUT: Duration = Duration::from_secs(60);
const KNOWLEDGE_SEEKING_TOKEN_THRESHOLD: usize = 12;
const VAULT_FETCH_STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "do", "for", "from", "how", "i",
    "if", "in", "into", "is", "it", "its", "just", "like", "make", "more", "my", "next", "not", "now",
    "of", "ok", "okay", "on", "option", "options", "or", "other", "our", "please", "so", "still",
    "sure", "thanks", "that", "the", "their", "them", "then", "there", "these", "they", "this", "to",
    "up", "use", "want", "was", "we", "well", "what", "when", "where", "which", "who", "why", "with",
    "would", "yeah", "yes", "you", "your", "go", "another", "instead", "continue", "proceed",
    "anyway", "whatever",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompactAction {
    Skipped,
    Reused,
    Compiled,
    Failed,
}

struct CompactCompile {
    user_prompt: String,
    previous: CanvasWorkingMemory,
    branch_key: String,
    chain_len: usize,
    parent_tile_id: String,
    parent_model_id: String,
}

enum CompactDecision {
    Skip,
    /// Some((parent_key, memory)) when the reused dream was seeded from the
    /// previous turn and must be copied to the parent key.
    Reuse(Option<(String, CanvasWorkingMemory)>),
    Compile(Box<CompactCompile>),
}

#[derive(Debug, Clone)]
pub(super) struct ConversationTurn {
    pub(super) tile_id: String,
    pub(super) prompt: String,
    pub(super) response: String,
    pub(super) model_id: String,
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

pub(super) fn conversation_tokens(turns: &[ConversationTurn]) -> usize {
    turns
        .iter()
        .map(|turn| estimate_tokens(&turn.prompt) + estimate_tokens(&turn.response))
        .sum()
}

pub(super) fn build_canvas_messages(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ChatMessage>, String> {
    build_canvas_messages_with_threshold(session, request, 32_000)
}

pub(super) fn build_canvas_messages_with_threshold(
    session: &CanvasSession,
    request: &PromptRequest,
    compact_token_threshold: usize,
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
                let turns = build_selected_parent_chain(session, request)?;
                if conversation_tokens(&turns) < compact_token_threshold {
                    Ok(turns_to_messages(turns, &request.prompt))
                } else {
                    // Dream is injected via compose_system_prompt; messages keep last 2.
                    let recent = recent_turns(&turns);
                    Ok(turns_to_messages(recent.to_vec(), &request.prompt))
                }
            } else {
                Ok(vec![ChatMessage {
                    role: "user".to_string(),
                    content: request.prompt.clone(),
                }])
            }
        }
    }
}

/// KnowledgeSearch/Semantic/Twin send the whole parent chain under the
/// threshold, so a parent dream in the system prompt would duplicate it.
pub(super) fn sends_full_parent_chain(
    session: &CanvasSession,
    request: &PromptRequest,
    compact_token_threshold: usize,
) -> bool {
    request.parent_debate_id.is_none()
        && matches!(
            request.context_mode,
            ContextMode::KnowledgeSearch | ContextMode::Semantic | ContextMode::Twin
        )
        && build_selected_parent_chain(session, request)
            .is_ok_and(|turns| conversation_tokens(&turns) < compact_token_threshold)
}

pub(super) fn compose_system_prompt(
    session: &CanvasSession,
    request: &PromptRequest,
    inner: Option<String>,
    include_memory: bool,
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
    } else if include_memory {
        if let Some(memory) =
            format_working_memory_for_prompt(&memory_for_follow_up(session, request))
        {
            parts.push(memory);
        }
    }
    if let Some(inner) = inner.map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
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
    let mut retrieval_cues = memory.retrieval_cues.clone();

    let goal = if !memory.goal.trim().is_empty() {
        memory.goal.trim().to_string()
    } else {
        memory.question.trim().to_string()
    };
    let discussed = if !memory.discussed.trim().is_empty() {
        memory.discussed.trim().to_string()
    } else {
        memory.summary.trim().to_string()
    };
    let mut nuances = if !memory.nuances.trim().is_empty() {
        memory.nuances.trim().to_string()
    } else {
        memory
            .open_questions
            .iter()
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>()
            .join("; ")
    };

    let mut body = String::from("## Session working memory\n");
    if !goal.is_empty() {
        body.push_str(&format!("\nGoal: {}\n", goal));
    }
    if !discussed.is_empty() {
        body.push_str(&format!("\nDiscussed:\n{}\n", discussed));
    }

    loop {
        let mut candidate = body.clone();
        if !nuances.trim().is_empty() {
            candidate.push_str(&format!("\nNuances:\n{}\n", nuances.trim()));
        }
        append_list(&mut candidate, "Constraints", &constraints);
        append_list(&mut candidate, "Open questions", &open_questions);
        append_list(&mut candidate, "Decisions", &decisions);
        append_positions(&mut candidate, &positions);
        append_list(&mut candidate, "Tried", &tried);
        append_list(&mut candidate, "Retrieval cues", &retrieval_cues);

        if estimate_tokens(&candidate) <= WORKING_MEMORY_TOKEN_BUDGET {
            sections.push(candidate);
            break;
        }

        if !tried.is_empty() {
            tried.pop();
            continue;
        }
        if !retrieval_cues.is_empty() {
            retrieval_cues.pop();
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
        if nuances.chars().count() > 40 {
            let keep = nuances.chars().count() / 2;
            nuances = nuances.chars().take(keep).collect();
            continue;
        }
        if !nuances.trim().is_empty() {
            nuances.clear();
            continue;
        }
        sections.push(candidate);
        break;
    }

    sections.pop()
}

pub(super) fn rewrite_retrieval_query(prompt: &str, memory: &CanvasWorkingMemory) -> String {
    let mut parts: Vec<String> = memory
        .retrieval_cues
        .iter()
        .map(|cue| cue.trim().to_string())
        .filter(|cue| !cue.is_empty())
        .collect();
    let goal = if !memory.goal.trim().is_empty() {
        memory.goal.trim()
    } else {
        memory.question.trim()
    };
    if !goal.is_empty() {
        parts.push(goal.to_string());
    }
    let prompt = prompt.trim();
    if !prompt.is_empty() {
        parts.push(prompt.to_string());
    }
    parts.join(" ")
}

pub(super) fn should_fetch_vault(prompt: &str, parent_memory: &CanvasWorkingMemory) -> bool {
    if parent_memory.is_empty() || parent_memory.needs_vault.is_none() {
        return true;
    }
    if parent_memory.needs_vault == Some(true)
        || parent_memory
            .retrieval_cues
            .iter()
            .any(|cue| !cue.trim().is_empty())
    {
        return true;
    }
    is_knowledge_seeking_prompt(prompt, parent_memory)
}

fn is_knowledge_seeking_prompt(prompt: &str, memory: &CanvasWorkingMemory) -> bool {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("what-did-i") || lower.contains("what did i") {
        return true;
    }

    let tokens = vault_prompt_tokens(prompt);
    if tokens
        .iter()
        .any(|token| matches!(token.as_str(), "note" | "notes" | "vault" | "remember"))
    {
        return true;
    }
    if tokens.len() >= KNOWLEDGE_SEEKING_TOKEN_THRESHOLD {
        return true;
    }

    let goal = if !memory.goal.trim().is_empty() {
        memory.goal.trim()
    } else {
        memory.question.trim()
    };
    let discussed = if !memory.discussed.trim().is_empty() {
        memory.discussed.trim()
    } else {
        memory.summary.trim()
    };
    let dream_tokens: HashSet<String> = vault_prompt_tokens(&format!(
        "{goal} {discussed} {}",
        memory.nuances.trim()
    ))
    .into_iter()
    .collect();
    tokens
        .iter()
        .any(|token| token.len() >= 6 && !dream_tokens.contains(token))
}

fn vault_prompt_tokens(text: &str) -> Vec<String> {
    let normalized: String = text
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    normalized
        .split_whitespace()
        .filter(|token| !VAULT_FETCH_STOPWORDS.contains(token))
        .map(str::to_string)
        .collect()
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
    // System-managed; compile JSON omits it and serde defaults to 0.
    if parsed.compacted_turn_count == 0 {
        parsed.compacted_turn_count = previous.compacted_turn_count;
    }
    if parsed.question.trim().is_empty() {
        parsed.question = previous.question.clone();
    }
    if parsed.summary.trim().is_empty() {
        parsed.summary = previous.summary.clone();
    }
    if parsed.goal.trim().is_empty() {
        parsed.goal = previous.goal.clone();
    }
    if parsed.discussed.trim().is_empty() {
        parsed.discussed = previous.discussed.clone();
    }
    if parsed.nuances.trim().is_empty() {
        parsed.nuances = previous.nuances.clone();
    }
    // Old compile JSON only had question/summary — promote into dreamed fields.
    if parsed.goal.trim().is_empty() && !parsed.question.trim().is_empty() {
        parsed.goal = parsed.question.clone();
    }
    if parsed.discussed.trim().is_empty() && !parsed.summary.trim().is_empty() {
        parsed.discussed = parsed.summary.clone();
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
    Ok(turns_to_messages(turns, &request.prompt))
}

fn build_compact_history_messages(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ChatMessage>, String> {
    let turns = build_selected_parent_chain(session, request)?;
    let recent = recent_turns(&turns);
    Ok(turns_to_messages(recent.to_vec(), &request.prompt))
}

fn recent_turns(turns: &[ConversationTurn]) -> &[ConversationTurn] {
    if turns.len() > COMPACT_HISTORY_RECENT_TURNS {
        &turns[turns.len() - COMPACT_HISTORY_RECENT_TURNS..]
    } else {
        turns
    }
}

fn turns_to_messages(turns: Vec<ConversationTurn>, new_prompt: &str) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
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
            tile_id: tile_id.clone(),
            prompt: tile.prompt.clone(),
            response: response.content.clone(),
            model_id: model_id.clone(),
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
    // chars/4 floor keeps minified JSON / CJK (few whitespace splits) from counting as ~1 word.
    std::cmp::max(text.split_whitespace().count() * 4 / 3, text.chars().count() / 4) + 1
}

const COMPILE_SYSTEM_PROMPT: &str = "\
You compile a short working-memory document for a multi-model canvas session. \
Return ONLY JSON with keys: goal, discussed, nuances, retrieval_cues, needs_vault, \
question, summary, constraints, open_questions, decisions, tried, model_positions, note_ids. \
model_positions is an array of {model_id, stance}. retrieval_cues is an array of short search phrases. \
needs_vault is true/false/null. Keep discussed under 120 words. Do not invent facts. \
Do not write constitution or identity claims.";

pub(super) fn compact_branch_memory(
    session: &mut CanvasSession,
    request: &PromptRequest,
    threshold: usize,
    compile: impl FnOnce(&str) -> Result<String, String>,
) -> CompactAction {
    match prepare_branch_compact(session, request, threshold) {
        CompactDecision::Skip => CompactAction::Skipped,
        CompactDecision::Reuse(seeded) => {
            if let Some((key, memory)) = seeded {
                session.branch_memories.insert(key, memory);
            }
            CompactAction::Reused
        }
        CompactDecision::Compile(plan) => match compile(&plan.user_prompt) {
            Ok(raw) => match apply_compact_compile(
                session,
                &plan.previous,
                &raw,
                &plan.branch_key,
                plan.chain_len,
                &plan.parent_tile_id,
            ) {
                Ok(_) => CompactAction::Compiled,
                Err(error) => {
                    log::warn!("Canvas compact parse failed: {error}");
                    CompactAction::Failed
                }
            },
            Err(error) => {
                log::warn!("Canvas working-memory compact failed: {error}");
                CompactAction::Failed
            }
        },
    }
}

pub(super) fn build_canvas_messages_after_compact(
    session: &CanvasSession,
    request: &PromptRequest,
    threshold: usize,
    action: CompactAction,
) -> Result<Vec<ChatMessage>, String> {
    if action == CompactAction::Failed
        && request.parent_debate_id.is_none()
        && matches!(
            request.context_mode,
            ContextMode::KnowledgeSearch
                | ContextMode::Semantic
                | ContextMode::Twin
                | ContextMode::Compact
        )
        && request.parent_tile_id.is_some()
        && request.parent_model_id.is_some()
    {
        let turns = build_selected_parent_chain(session, request)?;
        let tail = turns_fitting_threshold(&turns, threshold);
        return Ok(turns_to_messages(tail, &request.prompt));
    }
    build_canvas_messages_with_threshold(session, request, threshold)
}

pub(super) async fn compact_branch_memory_live(
    state: &AppState,
    session: &mut CanvasSession,
    request: &PromptRequest,
    threshold: usize,
    window: Option<&tauri::Window>,
) -> CompactAction {
    match prepare_branch_compact(session, request, threshold) {
        CompactDecision::Skip => CompactAction::Skipped,
        CompactDecision::Reuse(seeded) => {
            if let Some((key, memory)) = seeded {
                session.branch_memories.insert(key.clone(), memory.clone());
                persist_branch_memory(state, window, session, &key, &memory).await;
            }
            CompactAction::Reused
        }
        CompactDecision::Compile(plan) => match compile_dream_with_llm(state, &plan.user_prompt).await {
            Ok(raw) => match apply_compact_compile(
                session,
                &plan.previous,
                &raw,
                &plan.branch_key,
                plan.chain_len,
                &plan.parent_tile_id,
            ) {
                Ok(memory) => {
                    persist_branch_memory(state, window, session, &plan.branch_key, &memory).await;
                    let mut twin = state.twin_store.write().await;
                    let _ = twin.append_trace_event(
                        &session.id,
                        TraceEventType::WorkingMemoryCompiled,
                        json!({
                            "tile_id": plan.parent_tile_id,
                            "model_id": plan.parent_model_id,
                            "version": memory.version,
                            "summary": memory.summary.chars().take(240).collect::<String>(),
                            "branch_key": plan.branch_key,
                        }),
                    );
                    CompactAction::Compiled
                }
                Err(error) => {
                    log::warn!("Canvas compact parse failed: {error}");
                    CompactAction::Failed
                }
            },
            Err(error) => {
                log::warn!("Canvas working-memory compact failed: {error}");
                CompactAction::Failed
            }
        },
    }
}

fn prepare_branch_compact(
    session: &CanvasSession,
    request: &PromptRequest,
    threshold: usize,
) -> CompactDecision {
    if request.parent_debate_id.is_some() {
        return CompactDecision::Skip;
    }
    if !matches!(
        request.context_mode,
        ContextMode::Compact
            | ContextMode::KnowledgeSearch
            | ContextMode::Semantic
            | ContextMode::Twin
    ) {
        return CompactDecision::Skip;
    }
    let (Some(parent_tile_id), Some(parent_model_id)) =
        (&request.parent_tile_id, &request.parent_model_id)
    else {
        return CompactDecision::Skip;
    };
    let Ok(chain) = build_selected_parent_chain(session, request) else {
        return CompactDecision::Skip;
    };
    let force = request.context_mode == ContextMode::Compact;
    if !force && conversation_tokens(&chain) < threshold {
        return CompactDecision::Skip;
    }
    let branch_key = branch_memory_key(parent_tile_id, parent_model_id);
    // Dreams live at the key of the request's parent, so on a linear chain the
    // latest one sits one turn back; seed from it to stay incremental.
    let (previous, seeded) = match session.branch_memories.get(&branch_key) {
        Some(memory) => (memory.clone(), false),
        None => match chain.len().checked_sub(2).map(|idx| &chain[idx]) {
            Some(turn) => (
                session
                    .branch_memories
                    .get(&branch_memory_key(&turn.tile_id, &turn.model_id))
                    .cloned()
                    .unwrap_or_default(),
                true,
            ),
            None => (CanvasWorkingMemory::default(), false),
        },
    };
    if covering_dream_reusable(&previous, &chain, threshold) {
        return CompactDecision::Reuse(seeded.then(|| (branch_key, previous)));
    }
    CompactDecision::Compile(Box::new(CompactCompile {
        user_prompt: build_compact_user_prompt(&previous, &chain),
        previous,
        branch_key,
        chain_len: chain.len(),
        parent_tile_id: parent_tile_id.clone(),
        parent_model_id: parent_model_id.clone(),
    }))
}

fn covering_dream_reusable(
    memory: &CanvasWorkingMemory,
    chain: &[ConversationTurn],
    threshold: usize,
) -> bool {
    if memory.is_empty() {
        return false;
    }
    let covered_needed = chain.len().saturating_sub(COMPACT_HISTORY_RECENT_TURNS);
    if (memory.compacted_turn_count as usize) < covered_needed {
        return false;
    }
    let after = turns_after_compiled_from(chain, memory.compiled_from_tile_id.as_deref());
    estimate_dream_tokens(memory) + conversation_tokens(after) < threshold
}

fn estimate_dream_tokens(memory: &CanvasWorkingMemory) -> usize {
    format_working_memory_for_prompt(memory)
        .map(|text| estimate_tokens(&text))
        .unwrap_or(0)
}

fn turns_after_compiled_from<'a>(
    chain: &'a [ConversationTurn],
    compiled_from_tile_id: Option<&str>,
) -> &'a [ConversationTurn] {
    let Some(from_id) = compiled_from_tile_id else {
        return chain;
    };
    match chain.iter().position(|turn| turn.tile_id == from_id) {
        Some(idx) if idx + 1 < chain.len() => &chain[idx + 1..],
        Some(_) => &[],
        None => chain,
    }
}

fn turns_fitting_threshold(turns: &[ConversationTurn], threshold: usize) -> Vec<ConversationTurn> {
    if turns.is_empty() {
        return Vec::new();
    }
    let mut start = turns.len();
    let mut tokens = 0;
    while start > 0 {
        let turn = &turns[start - 1];
        let cost = estimate_tokens(&turn.prompt) + estimate_tokens(&turn.response);
        if tokens + cost > threshold && start < turns.len() {
            break;
        }
        tokens += cost;
        start -= 1;
    }
    turns[start..].to_vec()
}

fn build_compact_user_prompt(previous: &CanvasWorkingMemory, chain: &[ConversationTurn]) -> String {
    let mut body = String::from("Previous working memory JSON for this branch only:\n");
    body.push_str(&serde_json::to_string_pretty(previous).unwrap_or_else(|_| "{}".to_string()));
    body.push('\n');

    let incremental = turns_after_compiled_from(chain, previous.compiled_from_tile_id.as_deref());
    let (label, turns) = if previous.compiled_from_tile_id.is_some() && !incremental.is_empty() {
        ("Turns since last compile:", incremental)
    } else {
        ("Full parent chain:", chain)
    };
    body.push('\n');
    body.push_str(label);
    body.push('\n');
    for turn in turns {
        body.push_str(&format!(
            "\nPrompt:\n{}\nAnswer ({}):\n{}\n",
            turn.prompt, turn.model_id, turn.response
        ));
    }
    body
}

fn apply_compact_compile(
    session: &mut CanvasSession,
    previous: &CanvasWorkingMemory,
    raw: &str,
    branch_key: &str,
    chain_len: usize,
    parent_tile_id: &str,
) -> Result<CanvasWorkingMemory, String> {
    if extract_memory_json(raw).is_none() {
        return Err("Compact compile returned no JSON".to_string());
    }
    let mut compiled =
        parse_working_memory_response(raw, previous, Some(parent_tile_id.to_string()));
    if compiled.is_empty() {
        return Err("Compact compile produced empty working memory".to_string());
    }
    compiled.compacted_turn_count = chain_len as u32;
    compiled.compiled_from_tile_id = Some(parent_tile_id.to_string());
    session
        .branch_memories
        .insert(branch_key.to_string(), compiled.clone());
    Ok(compiled)
}

async fn compile_dream_with_llm(state: &AppState, user_prompt: &str) -> Result<String, String> {
    let settings = state.settings_service.read().await.get().clone();
    let use_ollama = settings.twin_llm_provider.eq_ignore_ascii_case("ollama");
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: user_prompt.to_string(),
    }];

    let compile_future = async {
        if use_ollama {
            let model = settings.ollama_model.trim();
            if model.is_empty() {
                return Err("Select an Ollama model before compiling session memory".to_string());
            }
            let ollama = state.ollama.read().await;
            ollama
                .chat(model, messages, Some(COMPILE_SYSTEM_PROMPT), Some(0.2))
                .await
                .map_err(|e| e.to_string())
        } else {
            let model = settings.llm_model.trim();
            if model.is_empty() {
                return Err("No compile model configured".to_string());
            }
            let openrouter = state.openrouter.read().await;
            openrouter
                .chat(OpenRouterChatRequest {
                    model,
                    messages,
                    system_prompt: Some(COMPILE_SYSTEM_PROMPT),
                    temperature: Some(0.2),
                    max_tokens: Some(800),
                    reasoning_effort: Some("none"),
                    web_search: false,
                    web_search_max_results: 0,
                })
                .await
                .map_err(|e| e.to_string())
        }
    };

    match tokio::time::timeout(COMPACT_LLM_TIMEOUT, compile_future).await {
        Ok(result) => result,
        Err(_) => Err("Canvas compact timed out".to_string()),
    }
}

async fn persist_branch_memory(
    state: &AppState,
    window: Option<&tauri::Window>,
    session: &CanvasSession,
    branch_key: &str,
    memory: &CanvasWorkingMemory,
) {
    {
        let mut store = state.canvas_store.write().await;
        if let Err(error) = store.update_branch_memory(&session.id, branch_key, memory.clone()) {
            log::warn!("Failed to persist compacted branch memory: {error}");
        }
    }
    if let Some(window) = window {
        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::WorkingMemoryUpdated {
                session_id: session.id.clone(),
                working_memory: memory.clone(),
                branch_key: branch_key.to_string(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_support::build_tile;
    use crate::models::canvas::{CanvasViewport, PromptType, TwinAnswerMode};
    use crate::models::canvas::PromptTile;

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
            build_tile("tile-1", "Prompt 1", "openai/gpt-4", "Response 1", None, None),
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
        assert!(!messages
            .iter()
            .any(|message| message.content.contains("Conversation summary before the most recent turns")));
        assert!(!messages.iter().any(|message| message.content.contains("Prompt 1")));
        assert!(!messages.iter().any(|message| message.content.contains("Response 1")));
    }

    #[test]
    fn compact_with_working_memory_uses_compiled_summary_not_truncation() {
        let mut session = four_turn_session();
        session.branch_memories.insert(
            branch_memory_key("tile-4", "openai/gpt-4"),
            CanvasWorkingMemory {
            version: 2,
            question: "Should we compact instead of truncate?".to_string(),
            summary: "Earlier turns decided compiled state beats 240-character chops.".to_string(),
            model_positions: vec![ModelPosition {
                model_id: "openai/gpt-4".to_string(),
                stance: "Prefer a session summary.".to_string(),
            }],
            ..CanvasWorkingMemory::default()
            },
        );
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::Compact);
        let messages = build_compact_history_messages(&session, &request).unwrap();

        // Dream lives in compose_system_prompt, not as a leading user message.
        assert_eq!(messages[0].content, "Prompt 3");
        assert_eq!(messages[1].content, "Response 3");
        assert_eq!(messages[2].content, "Prompt 4");
        assert_eq!(messages[3].content, "Response 4");
        assert_eq!(messages.last().unwrap().content, "Prompt 5");
        assert!(!messages.iter().any(|message| {
            message.content.contains("Should we compact instead of truncate?")
                || message.content.contains("compiled state beats 240-character chops")
        }));
        assert!(!messages
            .iter()
            .any(|message| message.content.contains("Conversation summary before the most recent turns")));
        assert!(!messages.iter().any(|message| message.content.contains("Response 1")));
    }

    #[test]
    fn compose_system_prompt_includes_parent_dream_for_compact() {
        let mut session = four_turn_session();
        session.branch_memories.insert(
            branch_memory_key("tile-4", "openai/gpt-4"),
            CanvasWorkingMemory {
                goal: "Keep the dream in the system prompt".to_string(),
                discussed: "Earlier turns decided compiled state beats 240-character chops."
                    .to_string(),
                ..CanvasWorkingMemory::default()
            },
        );
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::Compact);
        let composed = compose_system_prompt(&session, &request, None, true).expect("dream in system");
        assert!(composed.contains("Keep the dream in the system prompt"));
        assert!(composed.contains("compiled state beats 240-character chops"));
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

        assert!(messages.iter().any(|message| message.content.contains("Prompt 1")));
        assert!(messages.iter().any(|message| message.content.contains("Prompt 4")));
        assert_eq!(messages.last().unwrap().content, "What about the other option?");
    }

    #[test]
    fn knowledge_search_over_threshold_keeps_last_two_turns() {
        let session = four_turn_session();
        let request = build_request(
            "What about the other option?",
            "tile-4",
            "openai/gpt-4",
            ContextMode::KnowledgeSearch,
        );
        let messages = build_canvas_messages_with_threshold(&session, &request, 1).unwrap();

        assert!(!messages.iter().any(|message| message.content.contains("Prompt 1")));
        assert_eq!(messages[0].content, "Prompt 3");
        assert_eq!(messages[1].content, "Response 3");
        assert_eq!(messages.last().unwrap().content, "What about the other option?");
    }

    #[test]
    fn conversation_tokens_sums_prompt_and_response_estimates() {
        let turns = vec![ConversationTurn {
            tile_id: "tile-1".to_string(),
            prompt: "Prompt 1".to_string(),
            response: "Response 1".to_string(),
            model_id: "openai/gpt-4".to_string(),
        }];
        assert_eq!(conversation_tokens(&turns), 6);
        assert_eq!(conversation_tokens(&[]), 0);
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
    fn rewrite_retrieval_query_uses_cues_and_goal_not_lists() {
        let memory = CanvasWorkingMemory {
            goal: "Pick a quiet camera".to_string(),
            question: "Legacy question ignored when goal set".to_string(),
            retrieval_cues: vec!["ricoh gr".to_string(), "leaf shutter".to_string()],
            open_questions: vec!["TTL vs manual strobes".to_string()],
            constraints: vec!["Stay under $500".to_string()],
            ..CanvasWorkingMemory::default()
        };
        let query = rewrite_retrieval_query("the other option", &memory);
        assert!(query.contains("ricoh gr"));
        assert!(query.contains("leaf shutter"));
        assert!(query.contains("Pick a quiet camera"));
        assert!(query.contains("the other option"));
        assert!(!query.contains("TTL vs manual strobes"));
        assert!(!query.contains("Stay under $500"));
        assert!(!query.contains("Legacy question"));
    }

    #[test]
    fn rewrite_retrieval_query_falls_back_to_question_when_goal_empty() {
        let memory = CanvasWorkingMemory {
            question: "Mirofish social architecture".to_string(),
            open_questions: vec!["TTL vs manual strobes".to_string()],
            constraints: vec!["Stay under $500".to_string()],
            ..CanvasWorkingMemory::default()
        };
        let query = rewrite_retrieval_query("the other option", &memory);
        assert!(query.contains("the other option"));
        assert!(query.contains("Mirofish social architecture"));
        assert!(!query.contains("TTL vs manual strobes"));
        assert!(!query.contains("Stay under $500"));
    }

    fn skip_vault_memory() -> CanvasWorkingMemory {
        CanvasWorkingMemory {
            goal: "Choose a quiet camera".to_string(),
            discussed: "Compared two bodies.".to_string(),
            nuances: "Keep shutter noise down.".to_string(),
            needs_vault: Some(false),
            ..CanvasWorkingMemory::default()
        }
    }

    #[test]
    fn should_fetch_vault_skip_vs_fetch_table() {
        let empty = CanvasWorkingMemory::default();
        assert!(
            should_fetch_vault("ok go", &empty),
            "empty memory fetches"
        );

        let needs_none = CanvasWorkingMemory {
            goal: "Choose a quiet camera".to_string(),
            discussed: "Compared two bodies.".to_string(),
            needs_vault: None,
            ..CanvasWorkingMemory::default()
        };
        assert!(
            should_fetch_vault("ok go", &needs_none),
            "needs_vault None fetches"
        );

        let flagged_true = CanvasWorkingMemory {
            goal: "Choose a quiet camera".to_string(),
            discussed: "Compared two bodies.".to_string(),
            needs_vault: Some(true),
            ..CanvasWorkingMemory::default()
        };
        assert!(should_fetch_vault("ok go", &flagged_true));

        let with_cues = CanvasWorkingMemory {
            goal: "Choose a quiet camera".to_string(),
            discussed: "Compared two bodies.".to_string(),
            needs_vault: Some(false),
            retrieval_cues: vec!["ricoh".to_string()],
            ..CanvasWorkingMemory::default()
        };
        assert!(should_fetch_vault("ok go", &with_cues));

        let skip = skip_vault_memory();
        assert!(
            !should_fetch_vault("ok go", &skip),
            "short anaphoric skips"
        );
        assert!(
            !should_fetch_vault("the other option", &skip),
            "short anaphoric skips"
        );

        assert!(should_fetch_vault("check the vault", &skip));
        assert!(should_fetch_vault("please remember the prior decision", &skip));
        assert!(should_fetch_vault("what did i write about this", &skip));
        assert!(should_fetch_vault("What about Mirofish?", &skip));
        assert!(should_fetch_vault(
            "compare sony nikon canon fuji ricoh leica pentax olympus hasselblad sigma tamron street photography bodies",
            &skip,
        ));
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
        assert_eq!(parsed.constraints, vec!["Keep Twin review-gated".to_string()]);
        assert_eq!(parsed.compiled_from_tile_id.as_deref(), Some("tile-9"));
    }

    #[test]
    fn parse_working_memory_response_keeps_previous_on_garbage() {
        let previous = CanvasWorkingMemory {
            version: 3,
            summary: "Keep me".to_string(),
            ..CanvasWorkingMemory::default()
        };
        let parsed = parse_working_memory_response("not json at all", &previous, Some("tile-1".to_string()));
        assert_eq!(parsed.version, 3);
        assert_eq!(parsed.summary, "Keep me");
    }

    #[test]
    fn old_working_memory_json_deserializes_and_format_falls_back_to_question_summary() {
        let memory: CanvasWorkingMemory = serde_json::from_str(r#"{"question":"Q","summary":"S"}"#)
            .expect("old JSON should deserialize");
        assert!(memory.goal.is_empty());
        assert!(memory.discussed.is_empty());
        assert_eq!(memory.question, "Q");
        assert_eq!(memory.summary, "S");

        let formatted = format_working_memory_for_prompt(&memory).expect("should format");
        assert!(formatted.contains("## Session working memory"));
        assert!(formatted.contains("Goal: Q"));
        assert!(formatted.contains("Discussed:\nS") || formatted.contains("Discussed: S"));
        assert!(!formatted.contains("Question:"));
        assert!(!formatted.contains("Summary:"));
    }

    #[test]
    fn format_prefers_dreamed_fields_over_legacy_fallbacks() {
        let memory = CanvasWorkingMemory {
            goal: "Dreamed goal".to_string(),
            discussed: "Dreamed discussed".to_string(),
            nuances: "Dreamed nuances".to_string(),
            question: "Legacy question".to_string(),
            summary: "Legacy summary".to_string(),
            open_questions: vec!["Legacy open Q".to_string()],
            retrieval_cues: vec!["cue-a".to_string()],
            ..CanvasWorkingMemory::default()
        };
        let formatted = format_working_memory_for_prompt(&memory).expect("should format");
        assert!(formatted.contains("Goal: Dreamed goal"));
        assert!(formatted.contains("Dreamed discussed"));
        assert!(formatted.contains("Dreamed nuances"));
        assert!(!formatted.contains("Legacy question"));
        assert!(!formatted.contains("Legacy summary"));
        assert!(formatted.contains("Retrieval cues:"));
        assert!(formatted.contains("cue-a"));
    }

    #[test]
    fn is_empty_is_false_when_only_goal_is_set() {
        let memory = CanvasWorkingMemory {
            goal: "Only goal".to_string(),
            ..CanvasWorkingMemory::default()
        };
        assert!(!memory.is_empty());

        let cues_only = CanvasWorkingMemory {
            retrieval_cues: vec!["ignored-for-emptiness".to_string()],
            needs_vault: Some(true),
            ..CanvasWorkingMemory::default()
        };
        assert!(cues_only.is_empty());
    }

    #[test]
    fn parse_preserves_previous_compacted_turn_count_when_omitted() {
        let previous = CanvasWorkingMemory {
            version: 2,
            compacted_turn_count: 7,
            goal: "Prior goal".to_string(),
            ..CanvasWorkingMemory::default()
        };
        let parsed = parse_working_memory_response(
            r#"{"goal":"Fresh goal","discussed":"Fresh discussed"}"#,
            &previous,
            Some("tile-3".to_string()),
        );
        assert_eq!(parsed.goal, "Fresh goal");
        assert_eq!(parsed.compacted_turn_count, 7);
    }

    #[test]
    fn parse_sets_goal_and_keeps_previous_goal_on_garbage() {
        let previous = CanvasWorkingMemory {
            version: 1,
            goal: "Keep goal".to_string(),
            discussed: "Keep discussed".to_string(),
            nuances: "Keep nuances".to_string(),
            question: "Old Q".to_string(),
            summary: "Old S".to_string(),
            ..CanvasWorkingMemory::default()
        };
        let parsed = parse_working_memory_response(
            r#"{"goal":"New goal","discussed":"New discussed","nuances":"New nuances","question":"New Q","summary":"New S"}"#,
            &previous,
            Some("tile-9".to_string()),
        );
        assert_eq!(parsed.goal, "New goal");
        assert_eq!(parsed.discussed, "New discussed");
        assert_eq!(parsed.nuances, "New nuances");

        let garbage = parse_working_memory_response("not json", &previous, Some("tile-1".to_string()));
        assert_eq!(garbage.goal, "Keep goal");
        assert_eq!(garbage.discussed, "Keep discussed");
        assert_eq!(garbage.nuances, "Keep nuances");

        let legacy = parse_working_memory_response(
            r#"{"question":"Mapped Q","summary":"Mapped S"}"#,
            &CanvasWorkingMemory::default(),
            None,
        );
        assert_eq!(legacy.goal, "Mapped Q");
        assert_eq!(legacy.discussed, "Mapped S");
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
        assert!(!messages.iter().any(|message| message.content.contains("Prompt 1")));
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
            build_tile("tile-1", "Root prompt", "openai/gpt-4", "Root response", None, None),
            build_tile(
                "tile-2",
                "Branch prompt",
                "openai/gpt-4",
                "Branch response",
                Some("tile-1"),
                Some("openai/gpt-4"),
            ),
        ]);
        let request = build_request("Final prompt", "tile-2", "openai/gpt-4", ContextMode::FullHistory);
        let messages = build_full_history_messages(&session, &request).unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].content, "Root prompt");
        assert_eq!(messages[1].content, "Root response");
        assert_eq!(messages[4].content, "Final prompt");
    }

    fn covering_parent_dream() -> CanvasWorkingMemory {
        CanvasWorkingMemory {
            version: 3,
            goal: "Keep this dream".to_string(),
            discussed: "Covering summary of earlier turns.".to_string(),
            compiled_from_tile_id: Some("tile-4".to_string()),
            compacted_turn_count: 4,
            ..CanvasWorkingMemory::default()
        }
    }

    #[test]
    fn reuse_covering_dream_skips_compile() {
        let mut session = four_turn_session();
        session.branch_memories.insert(
            branch_memory_key("tile-4", "openai/gpt-4"),
            covering_parent_dream(),
        );
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::Compact);
        let calls = std::cell::Cell::new(0);
        let action = compact_branch_memory(&mut session, &request, 32_000, |_prompt| {
            calls.set(calls.get() + 1);
            Ok(r#"{"goal":"should not run"}"#.to_string())
        });
        assert_eq!(action, CompactAction::Reused);
        assert_eq!(calls.get(), 0);
        let memory = session
            .branch_memories
            .get(&branch_memory_key("tile-4", "openai/gpt-4"))
            .expect("parent dream stays on the parent key");
        assert_eq!(memory.goal, "Keep this dream");
        assert_eq!(memory.compacted_turn_count, 4);
    }

    #[test]
    fn over_threshold_without_dream_invokes_compile_once() {
        let mut session = four_turn_session();
        let request = build_request(
            "What about the other option?",
            "tile-4",
            "openai/gpt-4",
            ContextMode::KnowledgeSearch,
        );
        let calls = std::cell::Cell::new(0);
        let action = compact_branch_memory(&mut session, &request, 1, |_prompt| {
            calls.set(calls.get() + 1);
            Ok(r#"{"goal":"Compacted goal","discussed":"All four turns"}"#.to_string())
        });
        assert_eq!(action, CompactAction::Compiled);
        assert_eq!(calls.get(), 1);
        let memory = session
            .branch_memories
            .get(&branch_memory_key("tile-4", "openai/gpt-4"))
            .expect("dream is stored on the parent key");
        assert_eq!(memory.goal, "Compacted goal");
        assert_eq!(memory.discussed, "All four turns");
        assert_eq!(memory.compacted_turn_count, 4);
        assert_eq!(memory.compiled_from_tile_id.as_deref(), Some("tile-4"));

        let composed = compose_system_prompt(&session, &request, None, true).expect("dream in system");
        assert!(composed.contains("Compacted goal"));
        let messages = build_canvas_messages_after_compact(&session, &request, 1, action).unwrap();
        assert!(!messages.iter().any(|message| message.content.contains("Prompt 1")));
        assert_eq!(messages[0].content, "Prompt 3");
        assert_eq!(messages.last().unwrap().content, "What about the other option?");
    }

    #[test]
    fn incremental_compile_prompt_contains_previous_goal_and_later_turns_only() {
        let mut session = four_turn_session();
        session.branch_memories.insert(
            branch_memory_key("tile-4", "openai/gpt-4"),
            CanvasWorkingMemory {
                goal: "Find a quiet camera".to_string(),
                discussed: "Compared two bodies.".to_string(),
                compiled_from_tile_id: Some("tile-2".to_string()),
                compacted_turn_count: 1,
                ..CanvasWorkingMemory::default()
            },
        );
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::KnowledgeSearch);
        let captured = std::cell::RefCell::new(String::new());
        let action = compact_branch_memory(&mut session, &request, 1, |prompt| {
            captured.replace(prompt.to_string());
            Ok(r#"{"goal":"Find a quiet camera","discussed":"Updated with later turns"}"#.to_string())
        });
        assert_eq!(action, CompactAction::Compiled);
        let user_prompt = captured.into_inner();
        assert!(user_prompt.contains("Find a quiet camera"));
        assert!(user_prompt.contains("Prompt 3"));
        assert!(user_prompt.contains("Prompt 4"));
        assert!(
            !user_prompt.contains("Prompt 1"),
            "incremental prompt must not resend turns at/before compiled_from_tile_id"
        );
        assert!(
            !user_prompt.contains("Prompt 2"),
            "turns after compiled_from_tile_id start at the next tile"
        );
        let memory = session
            .branch_memories
            .get(&branch_memory_key("tile-4", "openai/gpt-4"))
            .unwrap();
        assert_eq!(memory.compacted_turn_count, 4);
        assert_eq!(memory.compiled_from_tile_id.as_deref(), Some("tile-4"));
    }

    #[test]
    fn compact_compile_failure_sends_fitting_tail_and_does_not_write() {
        let mut session = four_turn_session();
        let request = build_request(
            "What about the other option?",
            "tile-4",
            "openai/gpt-4",
            ContextMode::KnowledgeSearch,
        );
        let action = compact_branch_memory(&mut session, &request, 1, |_prompt| {
            Err("compile exploded".to_string())
        });
        assert_eq!(action, CompactAction::Failed);
        assert!(session.branch_memories.is_empty());
        let messages = build_canvas_messages_after_compact(&session, &request, 1, action).unwrap();
        assert!(!messages.iter().any(|message| message.content.contains("Prompt 1")));
        assert!(!messages.iter().any(|message| message.content.contains("Prompt 3")));
        assert_eq!(messages[0].content, "Prompt 4");
        assert_eq!(messages[1].content, "Response 4");
        assert_eq!(messages.last().unwrap().content, "What about the other option?");
    }

    fn dream_compiled_at_tile_3() -> CanvasWorkingMemory {
        CanvasWorkingMemory {
            version: 1,
            goal: "Seeded dream".to_string(),
            discussed: "Covers turns one through three.".to_string(),
            compiled_from_tile_id: Some("tile-3".to_string()),
            compacted_turn_count: 3,
            ..CanvasWorkingMemory::default()
        }
    }

    #[test]
    fn follow_up_one_tile_deeper_compiles_incrementally_from_previous_turn_dream() {
        let mut session = four_turn_session();
        session.branch_memories.insert(
            branch_memory_key("tile-3", "openai/gpt-4"),
            dream_compiled_at_tile_3(),
        );
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::KnowledgeSearch);
        let captured = std::cell::RefCell::new(String::new());
        let action = compact_branch_memory(&mut session, &request, 1, |prompt| {
            captured.replace(prompt.to_string());
            Ok(r#"{"goal":"Seeded dream","discussed":"Now covers four"}"#.to_string())
        });
        assert_eq!(action, CompactAction::Compiled);
        let user_prompt = captured.into_inner();
        assert!(user_prompt.contains("Seeded dream"));
        assert!(user_prompt.contains("Turns since last compile:"));
        assert!(user_prompt.contains("Prompt 4"));
        assert!(!user_prompt.contains("Prompt 3"), "seeded dream already covers tile-3");
        assert!(!user_prompt.contains("Prompt 1"));
        let memory = session
            .branch_memories
            .get(&branch_memory_key("tile-4", "openai/gpt-4"))
            .unwrap();
        assert_eq!(memory.version, 2);
        assert_eq!(memory.compacted_turn_count, 4);
    }

    #[test]
    fn follow_up_one_tile_deeper_reuses_previous_turn_dream_and_stores_it_at_parent_key() {
        let long = "word ".repeat(300);
        let mut session = build_session(vec![
            build_tile("tile-1", "Prompt 1", "openai/gpt-4", &long, None, None),
            build_tile("tile-2", "Prompt 2", "openai/gpt-4", &long, Some("tile-1"), Some("openai/gpt-4")),
            build_tile("tile-3", "Prompt 3", "openai/gpt-4", &long, Some("tile-2"), Some("openai/gpt-4")),
            build_tile("tile-4", "Prompt 4", "openai/gpt-4", "Response 4", Some("tile-3"), Some("openai/gpt-4")),
        ]);
        session.branch_memories.insert(
            branch_memory_key("tile-3", "openai/gpt-4"),
            dream_compiled_at_tile_3(),
        );
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::KnowledgeSearch);
        let calls = std::cell::Cell::new(0);
        let action = compact_branch_memory(&mut session, &request, 500, |_prompt| {
            calls.set(calls.get() + 1);
            Ok(r#"{"goal":"should not run"}"#.to_string())
        });
        assert_eq!(action, CompactAction::Reused);
        assert_eq!(calls.get(), 0);
        let memory = session
            .branch_memories
            .get(&branch_memory_key("tile-4", "openai/gpt-4"))
            .expect("seeded dream is carried forward to the parent key");
        assert_eq!(memory.goal, "Seeded dream");
        let composed = compose_system_prompt(&session, &request, None, true).expect("dream in system");
        assert!(composed.contains("Seeded dream"));
    }

    #[test]
    fn full_chain_under_threshold_omits_duplicate_parent_dream() {
        let mut session = four_turn_session();
        session.branch_memories.insert(
            branch_memory_key("tile-4", "openai/gpt-4"),
            covering_parent_dream(),
        );
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::KnowledgeSearch);
        assert!(sends_full_parent_chain(&session, &request, 32_000));
        assert!(!sends_full_parent_chain(&session, &request, 1), "over threshold sends last two");
        let full_history = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::FullHistory);
        assert!(
            !sends_full_parent_chain(&session, &full_history, 32_000),
            "FullHistory keeps its documented dream prefix"
        );
        assert_eq!(compose_system_prompt(&session, &request, None, false), None);
    }

    #[test]
    fn estimate_tokens_counts_unspaced_text_by_characters() {
        assert_eq!(estimate_tokens(&"x".repeat(400)), 101);
        assert_eq!(estimate_tokens("Prompt 1"), 3);
        assert_eq!(estimate_tokens("one two three four five six"), 9);
    }

    #[test]
    fn compact_mode_under_threshold_without_dream_invokes_compile() {
        let mut session = four_turn_session();
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::Compact);
        let calls = std::cell::Cell::new(0);
        let action = compact_branch_memory(&mut session, &request, 32_000, |_prompt| {
            calls.set(calls.get() + 1);
            Ok(r#"{"goal":"Summarize now","discussed":"Forced compact"}"#.to_string())
        });
        assert_eq!(action, CompactAction::Compiled);
        assert_eq!(calls.get(), 1);
    }
}
