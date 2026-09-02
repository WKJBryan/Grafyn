pub(super) use super::prediction_terminality::fail_requested_prediction_if_same_root;
mod twin_history;
use super::shared::{
    preserve_canvas_mutation_error, repair_canvas_trace_error, ModelProviderRoute, ModelRoute,
};
use crate::commands::run_retrieval;
use crate::models::canvas::{
    CanvasSession, ContextMode, DecisionPromptMetadata, PromptRequest, PromptTile, PromptType,
    ResponseStatus, TileContextNote, TwinAnswerMode, TwinEvidenceSnapshot,
};
use crate::models::note::ChunkResult;
use crate::models::twin::{
    ActionGap, ConstitutionItem, ConstitutionSetup, DecisionEpisode, TwinContextRecord,
};
use crate::models::twin_state::SelectionDestination;
use crate::services::ollama::OllamaService;
use crate::services::openrouter::{ChatMessage, OpenRouterService};
use crate::services::retrieval::RetrievalResult;
use crate::services::twin::{parse_twin_prediction, TwinStore};
use crate::AppState;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use twin_history::build_compact_history_messages;

const MIN_RETRIEVAL_SCORE_FOR_NOTES: f32 = 5.0;
const MIN_CANVAS_QUERY_TOKEN_LEN: usize = 3;
const CANVAS_RETRIEVAL_STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "do", "for", "from", "how", "i",
    "if", "in", "into", "is", "it", "its", "like", "make", "more", "my", "not", "of", "on", "or",
    "our", "real", "so", "that", "the", "their", "them", "there", "these", "they", "this", "to",
    "up", "use", "want", "was", "we", "what", "when", "where", "which", "who", "why", "with",
    "works", "would", "you", "your",
];

// Twin context assembly
pub(super) const TWIN_CONTEXT_VERSION: &str = "ctx-v3-reviewed-authority";
const TWIN_CONTEXT_TOKEN_BUDGET: usize = 4000;
const MAX_TWIN_CASE_CONTEXT: usize = 5;
const TWIN_CASE_FIELD_MAX_CHARS: usize = 800;
const TWIN_CASE_CORRECTION_MAX_CHARS: usize = 500;

#[derive(Debug, Clone)]
struct ConversationTurn {
    prompt: String,
    response: String,
    model_id: String,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedPromptContext {
    pub(super) messages: Vec<ChatMessage>,
    pub(super) context_notes: Vec<TileContextNote>,
    pub(super) approved_twin_records: Vec<TwinContextRecord>,
    pub(super) candidate_twin_records: Vec<TwinContextRecord>,
    pub(super) constitution_items: Vec<ConstitutionItem>,
    pub(super) action_gaps: Vec<ActionGap>,
    pub(super) system_prompt: Option<String>,
    /// Raw twin system prompt before any user system prompt is merged in;
    /// reused verbatim by the sealed-prediction call.
    pub(super) twin_context_prompt: Option<String>,
    pub(super) context_version: Option<String>,
    pub(super) decision_case_ids: Vec<String>,
    pub(super) twin_evidence_snapshot: Option<TwinEvidenceSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetrievalDecisionReason {
    NoResults,
    WeakTopScore,
    NoKeywordMatch,
    NoSnippet,
    NoLexicalOverlap,
    UseRetrievedNotes,
}

fn truncate_note_context_content(content: &str, max_chars: usize) -> String {
    let mut truncated = String::new();
    let mut chars = content.chars();
    for _ in 0..max_chars {
        match chars.next() {
            Some(ch) => truncated.push(ch),
            None => return truncated,
        }
    }

    if chars.next().is_some() {
        truncated.push_str("...");
    }

    truncated
}

fn normalize_canvas_query_tokens(text: &str) -> Vec<String> {
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
        .filter(|token| token.len() >= MIN_CANVAS_QUERY_TOKEN_LEN)
        .filter(|token| !CANVAS_RETRIEVAL_STOPWORDS.contains(token))
        .map(str::to_string)
        .collect()
}

fn matching_canvas_query_tokens(
    query_tokens: &[String],
    title: &str,
    snippet: &str,
) -> HashSet<String> {
    let candidate_tokens: HashSet<String> =
        normalize_canvas_query_tokens(&format!("{} {}", title, snippet))
            .into_iter()
            .collect();

    query_tokens
        .iter()
        .filter(|token| candidate_tokens.contains(token.as_str()))
        .cloned()
        .collect()
}

fn should_use_retrieved_notes(
    prompt: &str,
    retrieval_results: &[RetrievalResult],
) -> RetrievalDecisionReason {
    let Some(top_result) = retrieval_results.first() else {
        return RetrievalDecisionReason::NoResults;
    };

    if top_result.score < MIN_RETRIEVAL_SCORE_FOR_NOTES {
        return RetrievalDecisionReason::WeakTopScore;
    }

    if !top_result
        .relevance_reasons
        .iter()
        .any(|reason| reason == "keyword match")
    {
        return RetrievalDecisionReason::NoKeywordMatch;
    }

    let query_tokens = normalize_canvas_query_tokens(prompt);

    let mut saw_snippet = false;
    for result in retrieval_results {
        if result.snippet.trim().is_empty() {
            continue;
        }

        saw_snippet = true;
        let matched_tokens =
            matching_canvas_query_tokens(&query_tokens, &result.note.title, &result.snippet);
        let has_long_match = matched_tokens.iter().any(|token| token.len() >= 6);

        if matched_tokens.len() >= 2 || (has_long_match && !matched_tokens.is_empty()) {
            return RetrievalDecisionReason::UseRetrievedNotes;
        }
    }

    if !saw_snippet {
        RetrievalDecisionReason::NoSnippet
    } else {
        RetrievalDecisionReason::NoLexicalOverlap
    }
}

pub(super) async fn resolve_prompt_context(
    state: &AppState,
    session: &CanvasSession,
    request: &PromptRequest,
    replay_tile: Option<&PromptTile>,
    model_route: &ModelRoute,
) -> Result<ResolvedPromptContext, String> {
    let messages = build_canvas_messages(session, request)?;

    if request.context_mode == ContextMode::TwinHistory {
        return twin_history::resolve_twin_history_prompt_context(
            state,
            messages,
            session,
            request,
            replay_tile,
            match model_route.provider {
                ModelProviderRoute::Ollama => SelectionDestination::Local,
                ModelProviderRoute::OpenRouter => SelectionDestination::Network,
            },
        )
        .await;
    }

    if request.context_mode == ContextMode::Twin {
        return resolve_twin_prompt_context(state, messages, session, request).await;
    }

    if matches!(
        request.context_mode,
        ContextMode::KnowledgeSearch | ContextMode::Semantic
    ) {
        let pinned_ids = session.pinned_note_ids.clone();

        // Quality gate: note-level retrieval to check if vault has relevant content
        let retrieval_results = run_retrieval(state, &request.prompt, 5, &pinned_ids)
            .await
            .unwrap_or_default();

        let retrieval_decision = should_use_retrieved_notes(&request.prompt, &retrieval_results);
        if retrieval_decision != RetrievalDecisionReason::UseRetrievedNotes {
            log::info!(
                "Canvas knowledge search fallback for prompt {:?}: {:?}",
                request.prompt,
                retrieval_decision
            );
            return Ok(ResolvedPromptContext {
                messages,
                context_notes: Vec::new(),
                approved_twin_records: Vec::new(),
                candidate_twin_records: Vec::new(),
                constitution_items: Vec::new(),
                action_gaps: Vec::new(),
                system_prompt: request.system_prompt.clone(),
                twin_context_prompt: None,
                context_version: None,
                decision_case_ids: Vec::new(),
                twin_evidence_snapshot: None,
            });
        }

        // Check if chunk-level retrieval is enabled
        let chunk_enabled = {
            let retrieval = state.retrieval_service.read().await;
            retrieval.get_config().chunk_retrieval_enabled
        };

        if chunk_enabled {
            // Chunk-level context: retrieve relevant paragraphs within token budget
            let chunks = {
                let retrieval = state.retrieval_service.read().await;
                let chunk_index = state.chunk_index.read().await;
                let graph = state.graph_index.read().await;
                let priority = state.priority_service.read().await;
                let token_budget = retrieval.get_config().default_token_budget;
                retrieval
                    .retrieve_chunks(
                        &chunk_index,
                        &graph,
                        &priority,
                        &request.prompt,
                        token_budget,
                        &pinned_ids,
                    )
                    .unwrap_or_default()
            };

            if chunks.is_empty() {
                log::info!("Chunk retrieval returned no results, falling back to note-level");
                return resolve_note_level_context(
                    state,
                    messages,
                    &retrieval_results,
                    &pinned_ids,
                    &request.system_prompt,
                )
                .await;
            }

            let total_tokens: usize = chunks.iter().map(|c| c.token_estimate).sum();
            let parent_count = chunks
                .iter()
                .map(|c| &c.parent_note_id)
                .collect::<HashSet<_>>()
                .len();
            log::info!(
                "Canvas using chunk retrieval: {} chunks from {} notes (~{} tokens)",
                chunks.len(),
                parent_count,
                total_tokens
            );

            // Build context notes from chunk parent notes (deduped)
            let mut seen_notes: HashSet<String> = HashSet::new();
            let context_notes: Vec<TileContextNote> = chunks
                .iter()
                .filter_map(|c| {
                    if seen_notes.insert(c.parent_note_id.clone()) {
                        Some(TileContextNote {
                            id: c.parent_note_id.clone(),
                            title: c.parent_title.clone(),
                            snippet: truncate_note_context_content(&c.text, 200),
                            score: c.search_score,
                            pinned: pinned_ids.contains(&c.parent_note_id),
                        })
                    } else {
                        None
                    }
                })
                .collect();

            let note_prompt = build_chunk_context_prompt(&chunks);
            let system_prompt = match &request.system_prompt {
                Some(user_sp) if !user_sp.is_empty() => format!("{}\n\n{}", note_prompt, user_sp),
                _ => note_prompt,
            };

            Ok(ResolvedPromptContext {
                messages,
                context_notes,
                approved_twin_records: Vec::new(),
                candidate_twin_records: Vec::new(),
                constitution_items: Vec::new(),
                action_gaps: Vec::new(),
                system_prompt: Some(system_prompt),
                twin_context_prompt: None,
                context_version: None,
                decision_case_ids: Vec::new(),
                twin_evidence_snapshot: None,
            })
        } else {
            log::info!("Canvas using note-level context (chunk retrieval disabled)");
            resolve_note_level_context(
                state,
                messages,
                &retrieval_results,
                &pinned_ids,
                &request.system_prompt,
            )
            .await
        }
    } else {
        Ok(ResolvedPromptContext {
            messages,
            context_notes: Vec::new(),
            approved_twin_records: Vec::new(),
            candidate_twin_records: Vec::new(),
            constitution_items: Vec::new(),
            action_gaps: Vec::new(),
            system_prompt: request.system_prompt.clone(),
            twin_context_prompt: None,
            context_version: None,
            decision_case_ids: Vec::new(),
            twin_evidence_snapshot: None,
        })
    }
}

async fn resolve_twin_prompt_context(
    state: &AppState,
    messages: Vec<ChatMessage>,
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<ResolvedPromptContext, String> {
    let pinned_ids = session.pinned_note_ids.clone();
    let retrieval_results = run_retrieval(state, &request.prompt, 5, &pinned_ids)
        .await
        .unwrap_or_default();

    let should_use_notes = should_use_retrieved_notes(&request.prompt, &retrieval_results)
        == RetrievalDecisionReason::UseRetrievedNotes;
    let mut context_notes = Vec::new();
    let mut note_contexts = Vec::new();

    if should_use_notes {
        let chunk_enabled = {
            let retrieval = state.retrieval_service.read().await;
            retrieval.get_config().chunk_retrieval_enabled
        };

        if chunk_enabled {
            let chunks = {
                let retrieval = state.retrieval_service.read().await;
                let chunk_index = state.chunk_index.read().await;
                let graph = state.graph_index.read().await;
                let priority = state.priority_service.read().await;
                let token_budget = retrieval.get_config().default_token_budget;
                retrieval
                    .retrieve_chunks(
                        &chunk_index,
                        &graph,
                        &priority,
                        &request.prompt,
                        token_budget,
                        &pinned_ids,
                    )
                    .unwrap_or_default()
            };

            if !chunks.is_empty() {
                let mut seen_notes: HashSet<String> = HashSet::new();
                for chunk in &chunks {
                    note_contexts.push((
                        chunk.parent_note_id.clone(),
                        chunk.parent_title.clone(),
                        chunk.text.clone(),
                    ));
                    if seen_notes.insert(chunk.parent_note_id.clone()) {
                        context_notes.push(TileContextNote {
                            id: chunk.parent_note_id.clone(),
                            title: chunk.parent_title.clone(),
                            snippet: truncate_note_context_content(&chunk.text, 200),
                            score: chunk.search_score,
                            pinned: pinned_ids.contains(&chunk.parent_note_id),
                        });
                    }
                }
            }
        }

        if note_contexts.is_empty() {
            note_contexts = fetch_note_contexts(state, &retrieval_results).await;
            let found_ids: HashSet<&str> =
                note_contexts.iter().map(|(id, _, _)| id.as_str()).collect();
            for r in &retrieval_results {
                if found_ids.contains(r.note.id.as_str()) {
                    context_notes.push(TileContextNote {
                        id: r.note.id.clone(),
                        title: r.note.title.clone(),
                        snippet: r.snippet.clone(),
                        score: r.score,
                        pinned: pinned_ids.contains(&r.note.id),
                    });
                }
            }
        }
    }

    let constitution_query =
        decision_context_query(&request.prompt, request.decision_metadata.as_ref());
    let (setup, approved_twin_records, constitution_items, action_gaps, decision_cases) = {
        let mut twin_store = state.twin_store.write().await;
        let setup = twin_store
            .get_constitution_setup()
            .map_err(|error| error.to_string())?;
        validate_twin_identity_for_answer_mode(&setup, &request.twin_answer_mode)?;
        let (approved, _) = twin_store
            .select_context_records(&request.prompt)
            .map_err(|error| error.to_string())?;
        let (constitution_items, action_gaps) = twin_store
            .select_constitution_context(&constitution_query)
            .map_err(|error| error.to_string())?;
        let decision_cases = twin_store
            .select_decision_cases(&constitution_query, None, MAX_TWIN_CASE_CONTEXT)
            .map_err(|error| error.to_string())?;
        (
            setup,
            approved
                .into_iter()
                .filter(is_model_authoritative_twin_record)
                .collect(),
            constitution_items,
            action_gaps,
            decision_cases,
        )
    };

    let selection = apply_twin_context_budget(
        decision_cases,
        constitution_items,
        approved_twin_records,
        Vec::new(),
        action_gaps,
        note_contexts,
        TWIN_CONTEXT_TOKEN_BUDGET,
    );
    let decision_case_ids = selection
        .cases
        .iter()
        .map(|episode| episode.id.clone())
        .collect::<Vec<_>>();

    let twin_prompt = build_twin_context_prompt(
        &setup,
        &selection.cases,
        &selection.notes,
        &selection.approved,
        &selection.candidates,
        &selection.constitution_items,
        &selection.action_gaps,
        &request.twin_answer_mode,
        &request.prompt_type,
        request.decision_metadata.as_ref(),
    );
    let system_prompt = match &request.system_prompt {
        Some(user_sp) if !user_sp.is_empty() => format!("{}\n\n{}", twin_prompt, user_sp),
        _ => twin_prompt.clone(),
    };

    Ok(ResolvedPromptContext {
        messages,
        context_notes,
        approved_twin_records: selection.approved,
        candidate_twin_records: selection.candidates,
        constitution_items: selection.constitution_items,
        action_gaps: selection.action_gaps,
        system_prompt: Some(system_prompt),
        twin_context_prompt: Some(twin_prompt),
        context_version: Some(TWIN_CONTEXT_VERSION.to_string()),
        decision_case_ids,
        twin_evidence_snapshot: None,
    })
}

/// Fetch full note content for retrieval results, returning (id, title, truncated_content) tuples.
/// Skips notes that can't be read from the store. Called by both the semantic and twin note-level paths.
async fn fetch_note_contexts(
    state: &AppState,
    results: &[RetrievalResult],
) -> Vec<(String, String, String)> {
    let store = state.knowledge_store.read().await;
    results
        .iter()
        .filter_map(|r| {
            store.get_note(&r.note.id).ok().map(|note| {
                let truncated = truncate_note_context_content(&note.content, 1500);
                (note.id.clone(), note.title.clone(), truncated)
            })
        })
        .collect()
}

/// Fall back to note-level context when chunk retrieval is disabled or returns nothing.
async fn resolve_note_level_context(
    state: &AppState,
    messages: Vec<ChatMessage>,
    retrieval_results: &[RetrievalResult],
    pinned_ids: &[String],
    user_system_prompt: &Option<String>,
) -> Result<ResolvedPromptContext, String> {
    let note_contexts = fetch_note_contexts(state, retrieval_results).await;

    let context_notes: Vec<TileContextNote> = retrieval_results
        .iter()
        .map(|r| TileContextNote {
            id: r.note.id.clone(),
            title: r.note.title.clone(),
            snippet: r.snippet.clone(),
            score: r.score,
            pinned: pinned_ids.contains(&r.note.id),
        })
        .collect();

    let note_prompt = build_note_context_prompt(&note_contexts);
    let system_prompt = match user_system_prompt {
        Some(user_sp) if !user_sp.is_empty() => format!("{}\n\n{}", note_prompt, user_sp),
        _ => note_prompt,
    };

    Ok(ResolvedPromptContext {
        messages,
        context_notes,
        approved_twin_records: Vec::new(),
        candidate_twin_records: Vec::new(),
        constitution_items: Vec::new(),
        action_gaps: Vec::new(),
        system_prompt: Some(system_prompt),
        twin_context_prompt: None,
        context_version: None,
        decision_case_ids: Vec::new(),
        twin_evidence_snapshot: None,
    })
}

fn build_canvas_messages(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ChatMessage>, String> {
    match request.context_mode {
        ContextMode::FullHistory => build_full_history_messages(session, request),
        ContextMode::Compact => build_compact_history_messages(session, request),
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
        _ => Ok(vec![ChatMessage {
            role: "user".to_string(),
            content: request.prompt.clone(),
        }]),
    }
}

fn build_full_history_messages(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ChatMessage>, String> {
    let turns = build_selected_parent_chain(session, request)?;
    let mut messages = Vec::with_capacity((turns.len() * 2) + 1);

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
        content: request.prompt.clone(),
    });

    Ok(messages)
}

fn build_selected_parent_chain(
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
        tile.validate_twin_relationship_context(Some(&request.twin_relationship_variant))?;
        let response = tile.responses.get(&model_id).ok_or_else(|| {
            format!(
                "Parent response not found for tile {} and model {}",
                tile_id, model_id
            )
        })?;
        if response.status != ResponseStatus::Completed {
            return Err(format!(
                "Parent response must be completed for tile {} and model {}",
                tile_id, model_id
            ));
        }
        if response.content.trim().is_empty() {
            return Err(format!(
                "Parent response must contain non-empty content for tile {} and model {}",
                tile_id, model_id
            ));
        }

        turns.push(ConversationTurn {
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

/// Build a system prompt that includes retrieved note context.
/// Used by send_prompt when context_mode is Semantic.
fn build_chunk_context_prompt(chunks: &[ChunkResult]) -> String {
    let mut prompt = String::from(
        "You are a helpful knowledge assistant for the user's personal note-taking system (Grafyn). \
         Answer questions using the context from the user's notes below. \
         Reference specific notes by title when citing information. \
         If the notes don't contain relevant information, say so honestly.\n\n",
    );

    if chunks.is_empty() {
        prompt.push_str("No relevant notes were found for this query.\n");
        return prompt;
    }

    // Group chunks by parent note, preserving insertion order
    let mut note_order: Vec<String> = Vec::new();
    let mut note_map: HashMap<String, (String, Vec<&str>)> = HashMap::new();

    for chunk in chunks {
        let entry = note_map
            .entry(chunk.parent_note_id.clone())
            .or_insert_with(|| {
                note_order.push(chunk.parent_note_id.clone());
                (chunk.parent_title.clone(), Vec::new())
            });
        entry.1.push(&chunk.text);
    }

    prompt.push_str("## Relevant Notes\n\n");
    for note_id in &note_order {
        if let Some((title, texts)) = note_map.get(note_id) {
            prompt.push_str(&format!("### {} (id: {})\n", title, note_id));
            for text in texts {
                prompt.push_str(text);
                prompt.push_str("\n\n");
            }
        }
    }

    prompt
}

fn build_note_context_prompt(notes: &[(String, String, String)]) -> String {
    let mut prompt = String::from(
        "You are a helpful knowledge assistant for the user's personal note-taking system (Grafyn). \
         Answer questions using the context from the user's notes below. \
         Reference specific notes by title when citing information. \
         If the notes don't contain relevant information, say so honestly.\n\n",
    );

    if notes.is_empty() {
        prompt.push_str("No relevant notes were found for this query.\n");
    } else {
        prompt.push_str("## Relevant Notes\n\n");
        for (id, title, content) in notes {
            prompt.push_str(&format!("### {} (id: {})\n{}\n\n", title, id, content));
        }
    }

    prompt
}

/// Rough token estimate matching the chunk-index convention (words * 4/3).
fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace().count() * 4 / 3 + 1
}

/// Render one past decision episode as a verbatim behavioral case.
fn format_decision_case(episode: &DecisionEpisode) -> String {
    let mut case = format!("### Past decision: {}\n", episode.decision.trim());
    if !episode.options.is_empty() {
        case.push_str(&format!("- Options: {}\n", episode.options.join(" | ")));
    }
    if let Some(chosen) = episode.chosen_option.as_deref() {
        case.push_str(&format!("- Chose: {}\n", chosen.trim()));
    }
    if let Some(leaning) = episode.initial_leaning.as_deref() {
        if !leaning.trim().is_empty() {
            case.push_str(&format!("- Initial leaning: {}\n", leaning.trim()));
        }
    }
    if let Some(outcome) = episode.outcome.as_deref() {
        if !outcome.trim().is_empty() {
            case.push_str(&format!(
                "- Outcome: {}\n",
                truncate_note_context_content(outcome.trim(), TWIN_CASE_FIELD_MAX_CHARS)
            ));
        }
    }
    if let Some(lesson) = episode.lesson.as_deref() {
        if !lesson.trim().is_empty() {
            case.push_str(&format!(
                "- Lesson (verbatim): {}\n",
                truncate_note_context_content(lesson.trim(), TWIN_CASE_FIELD_MAX_CHARS)
            ));
        }
    }
    if let Some(note) = episode.correction_note.as_deref() {
        if !note.trim().is_empty() {
            case.push_str(&format!(
                "- Correction note (recorded when an earlier sealed twin guess missed): {}\n",
                truncate_note_context_content(note.trim(), TWIN_CASE_CORRECTION_MAX_CHARS)
            ));
        }
    }
    case.push('\n');
    case
}

struct TwinContextSelection {
    cases: Vec<DecisionEpisode>,
    constitution_items: Vec<ConstitutionItem>,
    approved: Vec<TwinContextRecord>,
    candidates: Vec<TwinContextRecord>,
    action_gaps: Vec<ActionGap>,
    notes: Vec<(String, String, String)>,
}

/// Greedy-fill the variable twin context sections into a hard token budget,
/// in priority order: cases > constitution > approved records > candidate
/// records > action gaps > evidence notes. Fixed scaffolding (operating
/// contract, identity, answer instructions, decision metadata) sits outside
/// the budget.
#[allow(clippy::too_many_arguments)]
fn apply_twin_context_budget(
    cases: Vec<DecisionEpisode>,
    constitution_items: Vec<ConstitutionItem>,
    approved: Vec<TwinContextRecord>,
    candidates: Vec<TwinContextRecord>,
    action_gaps: Vec<ActionGap>,
    notes: Vec<(String, String, String)>,
    budget: usize,
) -> TwinContextSelection {
    let mut remaining = budget as isize;
    let mut take_within_budget = move |cost: usize| -> bool {
        if remaining - cost as isize >= 0 {
            remaining -= cost as isize;
            true
        } else {
            false
        }
    };

    let cases = cases
        .into_iter()
        .filter(|episode| take_within_budget(estimate_tokens(&format_decision_case(episode))))
        .collect();
    let constitution_items = constitution_items
        .into_iter()
        .filter(|item| take_within_budget(estimate_tokens(&format_constitution_item(item))))
        .collect();
    let approved = approved
        .into_iter()
        .filter(|record| take_within_budget(estimate_tokens(&format_twin_record(record))))
        .collect();
    let candidates = candidates
        .into_iter()
        .filter(|record| take_within_budget(estimate_tokens(&format_twin_record(record))))
        .collect();
    let action_gaps = action_gaps
        .into_iter()
        .filter(|gap| take_within_budget(estimate_tokens(&format_action_gap(gap))))
        .collect();
    let notes = notes
        .into_iter()
        .filter(|(_, title, content)| {
            take_within_budget(estimate_tokens(title) + estimate_tokens(content))
        })
        .collect();

    TwinContextSelection {
        cases,
        constitution_items,
        approved,
        candidates,
        action_gaps,
        notes,
    }
}

/// User message for the hidden sealed-prediction call. With a configured
/// Twin Identity the framing is immersed first person; the fallback is a
/// neutral decision-support instruction. Disclosure language lives in the
/// app UI, never in model-facing prompts.
fn build_twin_prediction_user_message(
    setup: &ConstitutionSetup,
    decision: &str,
    options: &[String],
    stakes: Option<&str>,
) -> String {
    let immersed = has_twin_identity(setup);
    let mut message = String::new();
    if immersed {
        let name = setup
            .twin_name
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string();
        message.push_str(&format!(
            "I am {}. The decision in front of me: {}\n",
            name,
            decision.trim()
        ));
    } else {
        message.push_str(&format!(
            "Decision under consideration: {}\n",
            decision.trim()
        ));
    }
    if let Some(stakes) = stakes {
        if !stakes.trim().is_empty() {
            message.push_str(&format!("Stakes: {}\n", stakes.trim()));
        }
    }
    message.push_str("My options:\n");
    for (index, option) in options.iter().enumerate() {
        message.push_str(&format!("{}. {}\n", index + 1, option));
    }
    if immersed {
        message.push_str(
            "\nWhich option do I choose? I answer with only this JSON object and nothing else:\n",
        );
    } else {
        message.push_str(
            "\nGiven the context above, determine which option best fits this decision-maker's \
             documented values, constitution, and past decisions. Respond with only this JSON \
             object and nothing else:\n",
        );
    }
    message.push_str(
        "{\"predicted_option\": \"<option text>\", \"option_index\": <option number from the list above>, \
         \"confidence\": <0.0 to 1.0>, \"rationale\": \"<one or two sentences>\"}",
    );
    message
}

/// The hidden sealed-prediction call. Runs in its own spawned task, never
/// touches the window, and never blocks the visible streaming flow. Lock
/// discipline: collect owned data under the store lock, drop it, await the
/// provider, then re-lock to attach.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_sealed_twin_prediction(
    root_state: AppState,
    root_epoch: crate::services::vault_namespace::VaultAuthorityTokenV1,
    twin_store: Arc<RwLock<TwinStore>>,
    openrouter: Arc<RwLock<OpenRouterService>>,
    ollama: Option<Arc<RwLock<OllamaService>>>,
    provider_route: ModelProviderRoute,
    prediction_model: String,
    episode_id: String,
    prompt: String,
    decision: String,
    options: Vec<String>,
    stakes: Option<String>,
    twin_context_prompt: Option<String>,
    context_version: String,
    decision_metadata: Option<DecisionPromptMetadata>,
) {
    let initial_root_guard = match crate::commands::acquire_expected_derived_root_epoch(
        &root_state,
        &root_epoch,
    )
    .await
    {
        Ok(guard) => guard,
        Err(error) => {
            log::warn!("Sealed prediction abandoned after root transition: {error}");
            fail_requested_prediction_if_same_root(
                &root_state,
                &twin_store,
                &episode_id,
                &root_epoch,
                "initial authority validation",
            )
            .await;
            return;
        }
    };
    let built: Result<(String, String), String> = {
        let mut store = twin_store.write().await;
        (|| {
            let setup = store
                .get_constitution_setup()
                .map_err(|error| format!("sealed prediction setup load failed: {error}"))?;

            let system_prompt = if let Some(prompt) = twin_context_prompt {
                prompt
            } else {
                // Non-Twin-context decision tile: build a twin-only context on
                // the spot so predictions cover every decision, not just
                // Twin-mode ones (skipping them would bias the eval sample).
                let query = decision_context_query(&prompt, decision_metadata.as_ref());
                let (approved, constitution_items, action_gaps, cases) = store
                    .select_context_records(&prompt)
                    .and_then(|(approved, _)| {
                        let (constitution_items, action_gaps) =
                            store.select_constitution_context(&query)?;
                        let cases = store.select_decision_cases(
                            &query,
                            Some(&episode_id),
                            MAX_TWIN_CASE_CONTEXT,
                        )?;
                        Ok((approved, constitution_items, action_gaps, cases))
                    })
                    .map_err(|error| format!("sealed prediction context build failed: {error}"))?;
                let selection = apply_twin_context_budget(
                    cases,
                    constitution_items,
                    approved
                        .into_iter()
                        .filter(is_model_authoritative_twin_record)
                        .collect(),
                    Vec::new(),
                    action_gaps,
                    Vec::new(),
                    TWIN_CONTEXT_TOKEN_BUDGET,
                );
                let answer_mode = if has_twin_identity(&setup) {
                    TwinAnswerMode::Simulation
                } else {
                    TwinAnswerMode::Advisor
                };
                build_twin_context_prompt(
                    &setup,
                    &selection.cases,
                    &selection.notes,
                    &selection.approved,
                    &selection.candidates,
                    &selection.constitution_items,
                    &selection.action_gaps,
                    &answer_mode,
                    &PromptType::Decision,
                    decision_metadata.as_ref(),
                )
            };

            let user_message =
                build_twin_prediction_user_message(&setup, &decision, &options, stakes.as_deref());
            Ok((system_prompt, user_message))
        })()
    };
    let (system_prompt, user_message) = match built {
        Ok(built) => built,
        Err(error) => {
            log::warn!("Sealed prediction input failed for {episode_id}: {error}");
            drop(initial_root_guard);
            fail_requested_prediction_if_same_root(
                &root_state,
                &twin_store,
                &episode_id,
                &root_epoch,
                "input construction",
            )
            .await;
            return;
        }
    };
    if let Err(error) = initial_root_guard.finish(&root_state).await {
        log::warn!("Sealed prediction context discarded after authority change: {error}");
        fail_requested_prediction_if_same_root(
            &root_state,
            &twin_store,
            &episode_id,
            &root_epoch,
            "post-context authority validation",
        )
        .await;
        return;
    }

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: user_message,
    }];
    let result = match provider_route {
        ModelProviderRoute::Ollama => {
            if let Some(ollama) = ollama {
                let ollama = ollama.read().await;
                ollama
                    .chat(&prediction_model, messages, Some(&system_prompt), Some(0.2))
                    .await
            } else {
                Err(anyhow::anyhow!(
                    "Local Ollama is unavailable on this runtime"
                ))
            }
        }
        ModelProviderRoute::OpenRouter => {
            let openrouter = openrouter.read().await;
            openrouter
                .chat(
                    &prediction_model,
                    messages,
                    Some(&system_prompt),
                    Some(0.2),
                    Some(600),
                    Some("none"),
                    false,
                    0,
                )
                .await
        }
    };

    let root_guard =
        match crate::commands::acquire_expected_root_epoch(&root_state, &root_epoch).await {
            Ok(guard) => guard,
            Err(error) => {
                log::warn!("Sealed prediction result discarded after root transition: {error}");
                fail_requested_prediction_if_same_root(
                    &root_state,
                    &twin_store,
                    &episode_id,
                    &root_epoch,
                    "post-network authority validation",
                )
                .await;
                return;
            }
        };
    match result {
        Ok(raw) => {
            let draft = parse_twin_prediction(&raw, &options);
            let mut store = twin_store.write().await;
            let commit = match store.attach_twin_prediction_expecting_authority(
                &episode_id,
                draft,
                &prediction_model,
                &context_version,
                root_epoch.clone(),
            ) {
                Ok((_, commit)) => commit,
                Err(error) => {
                    let error = preserve_canvas_mutation_error(error);
                    log::warn!("Failed to seal twin prediction for {episode_id}: {error}");
                    drop(store);
                    drop(root_guard);
                    let repair = repair_canvas_trace_error(
                        &root_state,
                        &error,
                        "sealed prediction persistence",
                    )
                    .await;
                    let failed_epoch = match &repair {
                        crate::commands::PostAuthorityRepair::Ready(epoch) => epoch.clone(),
                        _ => error
                            .repair_commit()
                            .and_then(|commit| commit.authority_token.clone())
                            .unwrap_or_else(|| root_epoch.clone()),
                    };
                    crate::commands::acknowledge_reported_repair(repair);
                    fail_requested_prediction_if_same_root(
                        &root_state,
                        &twin_store,
                        &episode_id,
                        &failed_epoch,
                        "prediction persistence",
                    )
                    .await;
                    return;
                }
            };
            if commit.authority_token.is_none() {
                drop(store);
                drop(root_guard);
                fail_requested_prediction_if_same_root(
                    &root_state,
                    &twin_store,
                    &episode_id,
                    &root_epoch,
                    "prediction persistence without terminal authority",
                )
                .await;
                return;
            }
            drop(store);
            drop(root_guard);
            if let crate::commands::PostAuthorityRepair::Unavailable(error) =
                crate::commands::repair_after_authority_mutation(
                    &root_state,
                    &commit,
                    "sealed prediction",
                )
                .await
            {
                // The sealed state is already durable; the repair seam leaves
                // derived state unavailable without inviting a duplicate write.
                log::warn!("Failed to publish sealed prediction authority: {error}");
            }
        }
        Err(error) => {
            log::warn!("Sealed twin prediction call failed for {episode_id}: {error}");
            drop(root_guard);
            fail_requested_prediction_if_same_root(
                &root_state,
                &twin_store,
                &episode_id,
                &root_epoch,
                "provider failure",
            )
            .await;
        }
    }
}

fn is_model_authoritative_twin_record(record: &TwinContextRecord) -> bool {
    record.promotion_state == crate::models::twin::PromotionState::Endorsed
        && (record.kind != crate::models::twin::UserRecordKind::Preference
            || record.source_label.as_deref() == Some("governed_projection"))
}

#[allow(clippy::too_many_arguments)]
fn build_twin_context_prompt(
    setup: &ConstitutionSetup,
    decision_cases: &[DecisionEpisode],
    notes: &[(String, String, String)],
    approved_records: &[TwinContextRecord],
    _candidate_records: &[TwinContextRecord],
    constitution_items: &[ConstitutionItem],
    action_gaps: &[ActionGap],
    answer_mode: &TwinAnswerMode,
    prompt_type: &PromptType,
    decision_metadata: Option<&DecisionPromptMetadata>,
) -> String {
    let approved_records = approved_records
        .iter()
        .filter(|record| is_model_authoritative_twin_record(record))
        .collect::<Vec<_>>();
    let mut prompt = String::from(
        "## Twin Operating Contract\n\n\
         You are Grafyn's native RAG twin mode. Use only the provided Constitution, action gaps, vault evidence, and user-reviewed twin records as context. \
         Keep uncertainty visible. Do not use evidence to justify a preselected answer; use evidence to constrain the answer before choosing. \
         Use interviewee answers as evidence about the interviewee, institution, product, or research context. \
         Use interviewer questions and follow-ups as evidence about the user's reasoning pattern. \
         Keep these roles separate.\n\n",
    );

    prompt.push_str(&format_twin_identity_section(setup, answer_mode));

    prompt.push_str("## Past Decision Cases\n\n");
    if decision_cases.is_empty() {
        prompt.push_str("No similar past decisions were selected for this prompt.\n\n");
    } else {
        prompt.push_str(
            "These are this person's actual past decisions, verbatim. \
             Weight them above abstracted records: they show how tradeoffs were really made.\n",
        );
        for episode in decision_cases {
            prompt.push_str(&format_decision_case(episode));
        }
    }

    let reviewed_constitution = constitution_items
        .iter()
        .filter(|item| {
            matches!(
                item.status,
                crate::models::twin::ConstitutionStatus::Active
                    | crate::models::twin::ConstitutionStatus::Softened
            )
        })
        .collect::<Vec<_>>();
    let candidate_constitution = constitution_items
        .iter()
        .filter(|item| item.status == crate::models::twin::ConstitutionStatus::Candidate)
        .collect::<Vec<_>>();

    prompt.push_str("## Reviewed Constitution\n\n");
    if reviewed_constitution.is_empty() {
        prompt.push_str("No reviewed Constitution items were selected for this prompt.\n\n");
    } else {
        prompt.push_str("These are the governing principles for the answer. Apply them before weighing evidence.\n");
        for item in reviewed_constitution {
            prompt.push_str(&format_constitution_item(item));
        }
        prompt.push('\n');
    }

    prompt.push_str("## Action Gap Risks\n\n");
    if action_gaps.is_empty() {
        prompt
            .push_str("No relevant stated-intention vs revealed-behavior gaps were selected.\n\n");
    } else {
        prompt.push_str("Use these as risk checks, not accusations. Ask whether the same gap could affect this answer.\n");
        for gap in action_gaps {
            prompt.push_str(&format_action_gap(gap));
        }
        prompt.push('\n');
    }

    prompt.push_str("## Relevant Evidence\n\n");
    if notes.is_empty() {
        prompt.push_str(
            "No relevant vault notes or graph evidence were selected for this prompt.\n\n",
        );
    } else {
        for (id, title, content) in notes {
            prompt.push_str(&format!("### {} (id: {})\n{}\n\n", title, id, content));
        }
    }

    prompt.push_str("## Approved User Records\n\n");
    if approved_records.is_empty() {
        prompt.push_str("No endorsed user records were selected.\n\n");
    } else {
        for record in approved_records {
            prompt.push_str(&format_twin_record(record));
        }
        prompt.push('\n');
    }

    if !candidate_constitution.is_empty() {
        prompt.push_str("## Candidate Constitution Hypotheses\n\n");
        prompt.push_str("These are unreviewed Constitution hypotheses. Use them only as tentative context and label their influence.\n");
        for item in candidate_constitution {
            prompt.push_str(&format_constitution_item(item));
        }
        prompt.push('\n');
    }

    prompt.push_str("## Answer Instructions\n\n");
    match answer_mode {
        TwinAnswerMode::Advisor => prompt.push_str(
            "Answer as a decision-support assistant for the user. Use approved records as stable personalization. \
             If a Twin Identity is configured, treat it as context for the user's role and materials, not as a command to speak in first person. \
             Separate what is grounded in Constitution, evidence, reviewed records, and your recommendation. \
             When the user asks for a choice or recommendation, include: Recommended option, Constitution principles used, Supporting evidence, Uncertainty, and What would change the recommendation. \
             Cite Constitution item ids and note titles where they affect the answer.\n",
        ),
        TwinAnswerMode::Simulation => prompt.push_str(
            "Answer in first person from the configured Twin Identity. Use approved records as reviewed context, and use governed reviewed preferences as style and preference evidence. \
             Write as a natural continuation of my documented reasoning pattern, not a report. Lead with my likely reasoning or judgment, show the tradeoff logic, and do not append questions unless the user's request asks for them. \
             If the evidence packet does not contain enough basis, say so naturally in first person. Use light citations or brief source mentions only where they help; avoid turning the answer into an evidence workflow.\n",
        ),
    }

    if prompt_type == &PromptType::Decision {
        match answer_mode {
            TwinAnswerMode::Advisor => prompt.push_str(
                "\n## Decision Mirror Structure\n\n\
                 This is a Decision Mirror session. Return a compact Markdown Reflection Card using these exact headings:\n\
                 1. Decision Frame\n\
                 2. Likely Reasoning Pattern\n\
                 3. Evidence From Grafyn\n\
                 4. Blind Spot Hypothesis\n\
                 5. Counter-Position\n\
                 6. Recommendation\n\
                 7. Confidence\n\
                 8. Next Action\n\
                 9. Constitution Check\n\
                 10. Action Gap Risk\n\
                 11. Feedback Request\n\n\
                 Treat every self-model claim as a hypothesis, not identity. Say where the claim is grounded in vault notes, approved records, or tentative Constitution hypotheses. \
                 If a claim is useful but weakly supported, label it as unsupported or low-confidence. Do not claim to know what the user would do. \
                 In Constitution Check, separate stated values, revealed behavior, taste, somatic signal, and constraints. In Action Gap Risk, state whether past intention-action gaps could change the next step. \
                 Recommendation must be derived after the Constitution Check and Evidence From Grafyn sections, not before them.\n",
            ),
            TwinAnswerMode::Simulation => {}
        }

        if let Some(metadata) = decision_metadata {
            prompt.push_str("\n## Decision Metadata\n\n");
            prompt.push_str(&format!("Decision: {}\n", metadata.decision));
            if !metadata.options.is_empty() {
                prompt.push_str("Options:\n");
                for option in &metadata.options {
                    prompt.push_str(&format!("- {}\n", option));
                }
            }
            if let Some(stakes) = metadata.stakes.as_deref().filter(|value| !value.is_empty()) {
                prompt.push_str(&format!("Stakes: {}\n", stakes));
            }
            if let Some(leaning) = metadata
                .initial_leaning
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                prompt.push_str(&format!("Initial leaning: {}\n", leaning));
            }
            if let Some(review_date) = metadata
                .review_date
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                prompt.push_str(&format!("Follow-up review date: {}\n", review_date));
            }
            prompt.push('\n');
        }
    }

    prompt
}

fn validate_twin_identity_for_answer_mode(
    setup: &ConstitutionSetup,
    answer_mode: &TwinAnswerMode,
) -> Result<(), String> {
    if answer_mode != &TwinAnswerMode::Simulation {
        return Ok(());
    }

    if has_twin_identity(setup) {
        Ok(())
    } else {
        Err("Twin Identity requires Name and Role / context before Simulation can run.".to_string())
    }
}

fn has_twin_identity(setup: &ConstitutionSetup) -> bool {
    setup
        .twin_name
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        && setup
            .twin_role
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

fn format_twin_identity_section(setup: &ConstitutionSetup, answer_mode: &TwinAnswerMode) -> String {
    let mut section = String::from("## Twin Identity\n\n");
    let name = setup.twin_name.as_deref().map(str::trim).unwrap_or("");
    let role = setup.twin_role.as_deref().map(str::trim).unwrap_or("");

    if name.is_empty() || role.is_empty() {
        section.push_str("No Twin Identity is configured for this prompt.\n\n");
        return section;
    }

    match answer_mode {
        TwinAnswerMode::Simulation => {
            section.push_str(&format!("I am {}.\n", name));
            section.push_str(&format!("My role/context is {}.\n", role));
            section.push_str("I reason from supplied Knowledge materials, reviewed Constitution, selected evidence, and reviewed twin records.\n");
            section.push_str("I speak in first person. I do not explain myself as an outside analyst. Continue my documented reasoning pattern.\n");
            section.push_str("If the evidence packet does not contain enough basis, I say so naturally: I do not recall that specifically, I would need more details, or based on what I have done before.\n");
        }
        TwinAnswerMode::Advisor => {
            section.push_str(&format!("Twin name: {}\n", name));
            section.push_str(&format!("Role/context: {}\n", role));
            section.push_str("Use this identity as context for role, materials, and decision frame while answering as an advisor.\n");
        }
    }

    let boundaries = setup
        .source_boundaries
        .iter()
        .map(|boundary| boundary.trim())
        .filter(|boundary| !boundary.is_empty())
        .collect::<Vec<_>>();
    if !boundaries.is_empty() {
        section.push_str("Source boundaries:\n");
        for boundary in boundaries {
            section.push_str(&format!("- {}\n", boundary));
        }
    }

    section.push('\n');
    section
}

fn format_twin_record(record: &TwinContextRecord) -> String {
    format!(
        "- [{:?}; {:?}; confidence {:.2}; evidence {}] {}\n",
        record.kind,
        record.promotion_state,
        record.confidence,
        record.evidence_count,
        record.content
    )
}

fn format_constitution_item(item: &ConstitutionItem) -> String {
    let source_labels = constitution_source_labels(item);
    format!(
        "- [id {}; dimension {}; status {:?}; confidence {:.2}; priority {:.2}; evidence {}; sources: {}] {}\n",
        item.id,
        item.dimension,
        item.status,
        item.confidence,
        item.priority,
        item.evidence_refs.len(),
        source_labels,
        item.claim
    )
}

fn constitution_source_labels(item: &ConstitutionItem) -> String {
    let mut labels = item
        .evidence_refs
        .iter()
        .flat_map(|evidence| {
            [
                evidence.source_type.as_deref(),
                evidence.source_label.as_deref(),
                evidence.speaker_role.as_deref(),
            ]
        })
        .flatten()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    if labels.is_empty() {
        item.source
            .clone()
            .unwrap_or_else(|| "unspecified".to_string())
    } else {
        labels.join(", ")
    }
}

fn format_action_gap(gap: &ActionGap) -> String {
    format!(
        "- [id {}; status {:?}; confidence {:.2}; evidence {}] Stated: {} | Revealed: {} | Risk: {}\n",
        gap.id,
        gap.status,
        gap.confidence,
        gap.evidence_refs.len(),
        gap.stated_value,
        gap.revealed_behavior,
        gap.decision_risk
    )
}

fn decision_context_query(
    prompt: &str,
    decision_metadata: Option<&DecisionPromptMetadata>,
) -> String {
    let mut parts = vec![prompt.to_string()];
    if let Some(metadata) = decision_metadata {
        parts.push(metadata.decision.clone());
        parts.extend(metadata.options.clone());
        if let Some(stakes) = &metadata.stakes {
            parts.push(stakes.clone());
        }
        if let Some(leaning) = &metadata.initial_leaning {
            parts.push(leaning.clone());
        }
    }
    parts.join("\n")
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "context_prediction_tests.rs"]
mod prediction_tests;
