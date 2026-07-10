use crate::commands::{commit_note_write, run_retrieval};
use crate::models::canvas::{
    AddModelsRequest, AvailableModel, CanvasSession, CanvasStreamEvent, CanvasViewport,
    ContextMode, Debate, DebateContinueRequest, DebateResponse, DebateRound, DebateStartRequest,
    DecisionPromptMetadata, LLMNodePositionUpdate, ModelResponse, PromptRequest, PromptTile,
    PromptType, ResponseStatus, SessionCreate, SessionMeta, SessionUpdate, TileContextNote,
    TilePosition, TilePositionUpdate, TwinAnswerMode,
};
use crate::models::note::{ChunkResult, NoteCreate, NoteStatus};
use crate::models::settings::UserSettings;
use crate::models::twin::{
    ActionGap, ConstitutionItem, ConstitutionSetup, DecisionEpisode, DecisionEpisodeCreate,
    PrimitiveDecisionAssessment, ReflectionCardCreate, TraceEventType, TwinContextRecord,
};
use crate::services::ollama::OllamaService;
use crate::services::openrouter::{ChatMessage, OpenRouterService};
use crate::services::retrieval::RetrievalResult;
use crate::services::twin_store::{parse_twin_prediction, TwinStore};
use crate::AppState;
use chrono::Utc;
use futures::StreamExt;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use tokio::sync::RwLock;

mod shared;
use shared::{
    append_canvas_trace, effective_model_ids, is_vault_context_prompt, resolve_model_route,
    source_tile_context_provider, ModelProviderRoute, ModelRoute,
};

const COMPACT_HISTORY_RECENT_TURNS: usize = 2;
const COMPACT_HISTORY_EXCERPT_CHARS: usize = 240;
const EMPTY_MODEL_RESPONSE_ERROR: &str = "No response returned from model";
const MIN_RETRIEVAL_SCORE_FOR_NOTES: f32 = 5.0;
const MIN_CANVAS_QUERY_TOKEN_LEN: usize = 3;
const CANVAS_RETRIEVAL_STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "do", "for", "from", "how", "i",
    "if", "in", "into", "is", "it", "its", "like", "make", "more", "my", "not", "of", "on", "or",
    "our", "real", "so", "that", "the", "their", "them", "there", "these", "they", "this", "to",
    "up", "use", "want", "was", "we", "what", "when", "where", "which", "who", "why", "with",
    "works", "would", "you", "your",
];

// LLM node layout constants
const LLM_NODE_WIDTH: f64 = 280.0;
const LLM_NODE_HEIGHT: f64 = 200.0;
const LLM_NODE_Y_STEP: f64 = 300.0; // height(200) + 100px gap for content overflow
const LLM_NODE_X_GAP: f64 = 80.0;

// Twin context assembly
const TWIN_CONTEXT_VERSION: &str = "ctx-v2-cases-lexical";
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
struct ResolvedPromptContext {
    messages: Vec<ChatMessage>,
    context_notes: Vec<TileContextNote>,
    approved_twin_records: Vec<TwinContextRecord>,
    candidate_twin_records: Vec<TwinContextRecord>,
    constitution_items: Vec<ConstitutionItem>,
    action_gaps: Vec<ActionGap>,
    system_prompt: Option<String>,
    /// Raw twin system prompt before any user system prompt is merged in;
    /// reused verbatim by the sealed-prediction call.
    twin_context_prompt: Option<String>,
    context_version: Option<String>,
    decision_case_ids: Vec<String>,
}

type StreamedResponseUpdate = (String, String, ResponseStatus, Option<String>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetrievalDecisionReason {
    NoResults,
    WeakTopScore,
    NoKeywordMatch,
    NoSnippet,
    NoLexicalOverlap,
    UseRetrievedNotes,
}

/// List all canvas sessions
#[tauri::command]
pub async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionMeta>, String> {
    let mut store = state.canvas_store.write().await;
    store.list_sessions().map_err(|e| e.to_string())
}

/// Get a single session by ID
#[tauri::command]
pub async fn get_session(id: String, state: State<'_, AppState>) -> Result<CanvasSession, String> {
    let mut store = state.canvas_store.write().await;
    store.get_session(&id).map_err(|e| e.to_string())
}

/// Create a new canvas session
#[tauri::command]
pub async fn create_session(
    session: SessionCreate,
    state: State<'_, AppState>,
) -> Result<CanvasSession, String> {
    let mut store = state.canvas_store.write().await;
    let created = store.create_session(session).map_err(|e| e.to_string())?;
    drop(store);

    append_canvas_trace(
        state.twin_store.clone(),
        &created.id,
        TraceEventType::SessionCreated,
        json!({
            "title": created.title.clone(),
            "description": created.description.clone(),
            "tags": created.tags.clone(),
            "status": created.status.clone(),
        }),
    )
    .await;

    Ok(created)
}

/// Update a canvas session
#[tauri::command]
pub async fn update_session(
    id: String,
    update: SessionUpdate,
    state: State<'_, AppState>,
) -> Result<CanvasSession, String> {
    let mut store = state.canvas_store.write().await;
    let updated = store
        .update_session(&id, update)
        .map_err(|e| e.to_string())?;
    drop(store);

    append_canvas_trace(
        state.twin_store.clone(),
        &updated.id,
        TraceEventType::SessionUpdated,
        json!({
            "title": updated.title.clone(),
            "description": updated.description.clone(),
            "tags": updated.tags.clone(),
            "status": updated.status.clone(),
            "viewport": updated.viewport.clone(),
            "pinned_note_ids": updated.pinned_note_ids.clone(),
        }),
    )
    .await;

    Ok(updated)
}

/// Delete a canvas session
#[tauri::command]
pub async fn delete_session(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store.delete_session(&id).map_err(|e| e.to_string())?;
    drop(store);

    append_canvas_trace(
        state.twin_store.clone(),
        &id,
        TraceEventType::SessionDeleted,
        json!({
            "session_id": id,
        }),
    )
    .await;

    Ok(())
}

/// Get list of available LLM models
#[tauri::command]
pub async fn get_available_models(
    state: State<'_, AppState>,
) -> Result<Vec<AvailableModel>, String> {
    let openrouter = state.openrouter.read().await;
    openrouter
        .get_available_models()
        .await
        .map_err(|e| e.to_string())
}

/// Send a prompt to multiple models with streaming responses via Tauri events.
/// Returns the tile_id immediately; actual responses stream via "canvas-stream" events.
#[tauri::command]
pub async fn send_prompt(
    window: tauri::Window,
    session_id: String,
    mut request: PromptRequest,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let tile_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let model_route = {
        let settings = state.settings_service.read().await;
        resolve_model_route(
            &request.prompt_type,
            &request.context_mode,
            request.twin_llm_provider.as_deref(),
            settings.get(),
        )?
    };
    request.models = effective_model_ids(&model_route, &request.models);
    let session = {
        let mut store = state.canvas_store.write().await;
        store.get_session(&session_id).map_err(|e| e.to_string())?
    };
    let resolved_context = resolve_prompt_context(state.inner(), &session, &request).await?;
    let decision_episode_id = if request.prompt_type == PromptType::Decision {
        Some(uuid::Uuid::new_v4().to_string())
    } else {
        None
    };

    // Compute LLM node positions (offset from prompt tile)
    let prompt_pos = request.position.clone().unwrap_or_default();
    let llm_start_x = prompt_pos.x + prompt_pos.width + LLM_NODE_X_GAP;

    // Create initial responses map with positions
    let mut responses: HashMap<String, ModelResponse> = HashMap::new();
    for (i, model_id) in request.models.iter().enumerate() {
        let model_name = model_id.split('/').last().unwrap_or(model_id).to_string();
        let llm_y = prompt_pos.y + (i as f64) * LLM_NODE_Y_STEP;
        responses.insert(
            model_id.clone(),
            ModelResponse {
                id: uuid::Uuid::new_v4().to_string(),
                model_id: model_id.clone(),
                model_name,
                content: String::new(),
                status: ResponseStatus::Pending,
                error: None,
                tokens_used: None,
                created_at: now,
                position: TilePosition {
                    x: llm_start_x,
                    y: llm_y,
                    width: LLM_NODE_WIDTH,
                    height: LLM_NODE_HEIGHT,
                },
            },
        );
    }

    // Create the tile (with context notes attached)
    let tile = PromptTile {
        id: tile_id.clone(),
        prompt_type: request.prompt_type.clone(),
        prompt: request.prompt.clone(),
        system_prompt: request.system_prompt.clone(),
        models: request.models.clone(),
        responses: responses.clone(),
        position: prompt_pos,
        created_at: now,
        context_mode: request.context_mode,
        parent_tile_id: request.parent_tile_id,
        parent_model_id: request.parent_model_id,
        context_notes: resolved_context.context_notes.clone(),
        approved_twin_records: resolved_context.approved_twin_records.clone(),
        candidate_twin_records: resolved_context.candidate_twin_records.clone(),
        twin_answer_mode: request.twin_answer_mode.clone(),
        twin_context_policy: request.twin_context_policy.clone(),
        twin_llm_provider: request.twin_llm_provider.clone(),
        decision_metadata: request.decision_metadata.clone(),
        decision_episode_id: decision_episode_id.clone(),
        web_search: request.web_search,
        web_search_max_results: request.web_search_max_results,
        reasoning_effort: request.reasoning_effort.clone(),
    };

    // Save tile to session
    {
        let mut store = state.canvas_store.write().await;
        store
            .add_tile(&session_id, tile.clone())
            .map_err(|e| e.to_string())?;
    }

    if let Some(decision_episode_id) = decision_episode_id.clone() {
        let decision_metadata = request.decision_metadata.clone().unwrap_or_default();
        let decision = if decision_metadata.decision.trim().is_empty() {
            request.prompt.clone()
        } else {
            decision_metadata.decision
        };
        let mut twin_store = state.twin_store.write().await;
        twin_store
            .record_decision_episode(DecisionEpisodeCreate {
                id: decision_episode_id,
                session_id: session_id.clone(),
                tile_id: tile_id.clone(),
                decision,
                options: decision_metadata.options,
                stakes: decision_metadata.stakes,
                initial_leaning: decision_metadata.initial_leaning,
                review_date: decision_metadata.review_date,
                primitive_assessment: PrimitiveDecisionAssessment::default(),
                // Stamped on every decision episode — including non-Twin
                // context tiles — so attribution survives a failed or absent
                // hidden prediction call.
                context_version: Some(
                    resolved_context
                        .context_version
                        .clone()
                        .unwrap_or_else(|| TWIN_CONTEXT_VERSION.to_string()),
                ),
            })
            .map_err(|error| error.to_string())?;
    }

    append_canvas_trace(
        state.twin_store.clone(),
        &session_id,
        TraceEventType::PromptSubmitted,
        json!({
            "tile_id": tile.id.clone(),
            "prompt_type": tile.prompt_type.clone(),
            "decision_episode_id": tile.decision_episode_id.clone(),
            "decision_metadata": tile.decision_metadata.clone(),
            "prompt": tile.prompt.clone(),
            "models": tile.models.clone(),
            "context_mode": tile.context_mode.clone(),
            "parent_tile_id": tile.parent_tile_id.clone(),
            "parent_model_id": tile.parent_model_id.clone(),
            "context_note_ids": tile.context_notes.iter().map(|note| note.id.clone()).collect::<Vec<_>>(),
            "twin_answer_mode": tile.twin_answer_mode.clone(),
            "approved_twin_record_ids": tile.approved_twin_records.iter().map(|record| record.id.clone()).collect::<Vec<_>>(),
            "candidate_twin_record_ids": tile.candidate_twin_records.iter().map(|record| record.id.clone()).collect::<Vec<_>>(),
            "constitution_item_ids": resolved_context.constitution_items.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
            "action_gap_ids": resolved_context.action_gaps.iter().map(|gap| gap.id.clone()).collect::<Vec<_>>(),
            "context_version": resolved_context.context_version.clone(),
            "decision_case_ids": resolved_context.decision_case_ids.clone(),
            "web_search": tile.web_search,
            "web_search_max_results": tile.web_search_max_results,
        }),
    )
    .await;

    // Emit TileCreated event
    let _ = window.emit(
        "canvas-stream",
        CanvasStreamEvent::TileCreated {
            session_id: session_id.clone(),
            tile: tile.clone(),
        },
    );

    // Emit ContextNotes event (so frontend can display which notes were used)
    if !resolved_context.context_notes.is_empty() {
        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::ContextNotes {
                session_id: session_id.clone(),
                tile_id: tile_id.clone(),
                notes: resolved_context.context_notes.clone(),
            },
        );
    }

    // Clone what we need for the spawned task
    let openrouter_arc = state.openrouter.clone();
    let ollama_arc = state.ollama.clone();
    let canvas_store_arc = state.canvas_store.clone();
    let twin_store_arc = state.twin_store.clone();
    let models = request.models.clone();
    let messages = resolved_context.messages.clone();
    let system_prompt = resolved_context.system_prompt.clone();
    let temperature = request.temperature;
    let web_search = request.web_search;
    let web_search_max_results = request.web_search_max_results;
    let reasoning_effort = request.reasoning_effort.clone();
    let provider_route = model_route.provider.clone();
    let tile_id_clone = tile_id.clone();
    let session_id_clone = session_id.clone();
    let decision_episode_id_for_reflection = tile.decision_episode_id.clone();
    let reflection_note_ids = tile
        .context_notes
        .iter()
        .map(|note| note.id.clone())
        .collect::<Vec<_>>();
    let reflection_user_record_ids = tile
        .approved_twin_records
        .iter()
        .chain(tile.candidate_twin_records.iter())
        .map(|record| record.id.clone())
        .collect::<Vec<_>>();
    let reflection_constitution_item_ids = resolved_context
        .constitution_items
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let reflection_action_gap_ids = resolved_context
        .action_gaps
        .iter()
        .map(|gap| gap.id.clone())
        .collect::<Vec<_>>();

    // Spawn async task for streaming (doesn't block the IPC response)
    tauri::async_runtime::spawn(async move {
        // Stream all models concurrently using JoinSet
        let mut join_set = tokio::task::JoinSet::new();

        for model_id in models {
            let messages = messages.clone();
            let system_prompt = system_prompt.clone();
            let reasoning_effort = reasoning_effort.clone();
            let provider_route = provider_route.clone();
            let openrouter_arc = openrouter_arc.clone();
            let ollama_arc = ollama_arc.clone();
            let window = window.clone();
            let session_id = session_id_clone.clone();
            let tile_id = tile_id_clone.clone();

            join_set.spawn(async move {
                let stream_result = match provider_route {
                    ModelProviderRoute::Ollama => {
                        let ollama = ollama_arc.read().await;
                        let result = ollama
                            .chat_stream(
                                &model_id,
                                messages,
                                system_prompt.as_deref(),
                                Some(temperature),
                            )
                            .await;
                        drop(ollama);
                        result.map(|stream| {
                            Box::pin(stream)
                                as std::pin::Pin<
                                    Box<dyn futures::Stream<Item = anyhow::Result<String>> + Send>,
                                >
                        })
                    }
                    ModelProviderRoute::OpenRouter => {
                        let openrouter = openrouter_arc.read().await;
                        let result = openrouter
                            .chat_stream(
                                &model_id,
                                messages,
                                system_prompt.as_deref(),
                                Some(temperature),
                                None,
                                Some(reasoning_effort.as_str()),
                                web_search,
                                web_search_max_results,
                            )
                            .await;
                        drop(openrouter);
                        result.map(|stream| {
                            Box::pin(stream)
                                as std::pin::Pin<
                                    Box<dyn futures::Stream<Item = anyhow::Result<String>> + Send>,
                                >
                        })
                    }
                };

                match stream_result {
                    Ok(stream) => {
                        let mut stream = stream;
                        let mut full_content = String::new();

                        loop {
                            match tokio::time::timeout(Duration::from_secs(60), stream.next()).await
                            {
                                Ok(Some(Ok(chunk))) => {
                                    if !chunk.is_empty() {
                                        full_content.push_str(&chunk);
                                        let _ = window.emit(
                                            "canvas-stream",
                                            CanvasStreamEvent::Chunk {
                                                session_id: session_id.clone(),
                                                tile_id: tile_id.clone(),
                                                model_id: model_id.clone(),
                                                chunk,
                                            },
                                        );
                                    }
                                }
                                Ok(Some(Err(e))) => {
                                    let error = e.to_string();
                                    emit_canvas_error(
                                        &window,
                                        &session_id,
                                        &tile_id,
                                        &model_id,
                                        &error,
                                    );
                                    return (
                                        model_id,
                                        String::new(),
                                        ResponseStatus::Error,
                                        Some(error),
                                    );
                                }
                                Ok(None) => break, // Stream ended naturally
                                Err(_) => {
                                    let error = "Stream idle timeout (60s)".to_string();
                                    emit_canvas_error(
                                        &window,
                                        &session_id,
                                        &tile_id,
                                        &model_id,
                                        &error,
                                    );
                                    return (
                                        model_id,
                                        full_content,
                                        ResponseStatus::Error,
                                        Some(error),
                                    );
                                }
                            }
                        }

                        finalize_streamed_model_response(
                            &window,
                            &session_id,
                            &tile_id,
                            model_id,
                            full_content,
                        )
                    }
                    Err(e) => {
                        let error = e.to_string();
                        emit_canvas_error(&window, &session_id, &tile_id, &model_id, &error);
                        (model_id, String::new(), ResponseStatus::Error, Some(error))
                    }
                }
            });
        }

        // Wait for all models and collect results
        let mut results = Vec::new();
        while let Some(result) = join_set.join_next().await {
            if let Ok(r) = result {
                results.push(r);
            }
        }

        // Batch update store with all results in a single write
        let persistence_ok = {
            let mut store = canvas_store_arc.write().await;
            match store.batch_update_tile_responses(&session_id_clone, &tile_id_clone, &results) {
                Ok(()) => true,
                Err(error) => {
                    let model_ids: Vec<String> = results
                        .iter()
                        .map(|(model_id, _, _, _)| model_id.clone())
                        .collect();
                    emit_persistence_error(
                        &window,
                        &session_id_clone,
                        &tile_id_clone,
                        &model_ids,
                        &error,
                    );
                    false
                }
            }
        };

        append_model_result_traces(
            twin_store_arc.clone(),
            &session_id_clone,
            &tile_id_clone,
            "send_prompt",
            &results,
        )
        .await;

        if let Some(decision_episode_id) = decision_episode_id_for_reflection {
            let mut twin_store = twin_store_arc.write().await;
            for (model_id, content, status, _) in &results {
                if *status != ResponseStatus::Completed || content.trim().is_empty() {
                    continue;
                }

                let _ = twin_store.record_reflection_card(ReflectionCardCreate {
                    decision_episode_id: decision_episode_id.clone(),
                    session_id: session_id_clone.clone(),
                    tile_id: tile_id_clone.clone(),
                    model_id: model_id.clone(),
                    content: content.clone(),
                    cited_note_ids: reflection_note_ids.clone(),
                    cited_user_record_ids: reflection_user_record_ids.clone(),
                    cited_constitution_item_ids: reflection_constitution_item_ids.clone(),
                    cited_action_gap_ids: reflection_action_gap_ids.clone(),
                    evidence_packet: None,
                });
            }
        }

        // Emit session saved after all models complete — but only if the
        // batch persist above actually succeeded. If it failed, per-model
        // Error events were already emitted and the frontend must not
        // believe the (in-memory only) responses were saved to disk.
        if persistence_ok {
            let _ = window.emit(
                "canvas-stream",
                CanvasStreamEvent::SessionSaved {
                    session_id: session_id_clone,
                },
            );
        }
    });

    // Sealed twin prediction: one hidden, non-streaming call per decision
    // episode with at least two options. Fire-and-forget in its own task —
    // it never emits canvas-stream events and cannot block or fail the
    // visible flow above.
    if let Some(episode_id) = tile.decision_episode_id.clone() {
        let metadata = tile.decision_metadata.clone().unwrap_or_default();
        if metadata.options.len() >= 2 {
            let prediction_model = match model_route.provider {
                ModelProviderRoute::Ollama => tile.models.first().cloned().unwrap_or_default(),
                ModelProviderRoute::OpenRouter => {
                    let settings = state.settings_service.read().await;
                    settings.get().llm_model.clone()
                }
            };
            let decision = if metadata.decision.trim().is_empty() {
                tile.prompt.clone()
            } else {
                metadata.decision.clone()
            };
            let context_version = resolved_context
                .context_version
                .clone()
                .unwrap_or_else(|| TWIN_CONTEXT_VERSION.to_string());
            tauri::async_runtime::spawn(run_sealed_twin_prediction(
                state.twin_store.clone(),
                state.openrouter.clone(),
                state.ollama.clone(),
                model_route.provider.clone(),
                prediction_model,
                episode_id,
                tile.prompt.clone(),
                decision,
                metadata.options.clone(),
                metadata.stakes.clone(),
                resolved_context.twin_context_prompt.clone(),
                context_version,
                tile.decision_metadata.clone(),
            ));
        }
    }

    Ok(tile_id)
}

/// Update a tile's position
#[tauri::command]
pub async fn update_tile_position(
    session_id: String,
    tile_id: String,
    position: TilePositionUpdate,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .update_tile_position(&session_id, &tile_id, position)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete a prompt tile and its children
#[tauri::command]
pub async fn delete_tile(
    session_id: String,
    tile_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .delete_tile(&session_id, &tile_id)
        .map_err(|e| e.to_string())?;
    drop(store);

    append_canvas_trace(
        state.twin_store.clone(),
        &session_id,
        TraceEventType::TileDeleted,
        json!({
            "tile_id": tile_id,
        }),
    )
    .await;

    Ok(())
}

/// Delete a single model response from a tile
#[tauri::command]
pub async fn delete_response(
    session_id: String,
    tile_id: String,
    model_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .delete_response(&session_id, &tile_id, &model_id)
        .map_err(|e| e.to_string())?;
    drop(store);

    append_canvas_trace(
        state.twin_store.clone(),
        &session_id,
        TraceEventType::ResponseDeleted,
        json!({
            "tile_id": tile_id,
            "model_id": model_id,
        }),
    )
    .await;

    Ok(())
}

/// Update viewport zoom/pan state
#[tauri::command]
pub async fn update_viewport(
    session_id: String,
    viewport: CanvasViewport,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .update_viewport(&session_id, viewport)
        .map_err(|e| e.to_string())
}

/// Update an LLM response node's position
#[tauri::command]
pub async fn update_llm_node_position(
    session_id: String,
    tile_id: String,
    model_id: String,
    position: LLMNodePositionUpdate,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .update_llm_node_position(&session_id, &tile_id, &model_id, position)
        .map_err(|e| e.to_string())
}

/// Auto-arrange all nodes (batch position update)
#[tauri::command]
pub async fn auto_arrange(
    session_id: String,
    positions: HashMap<String, TilePosition>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store
        .batch_update_positions(&session_id, positions)
        .map_err(|e| e.to_string())
}

/// Export canvas session to a note (returns note info)
#[tauri::command]
pub async fn export_to_note(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let mut store = state.canvas_store.write().await;
    let session = store.get_session(&session_id).map_err(|e| e.to_string())?;
    drop(store);

    // Build markdown content from session
    let mut content = format!("# {}\n\n", session.title);

    if let Some(desc) = &session.description {
        content.push_str(&format!("{}\n\n", desc));
    }

    for tile in &session.prompt_tiles {
        content.push_str(&format!("## Prompt\n\n{}\n\n", tile.prompt));

        for (model_id, response) in &tile.responses {
            if response.status == ResponseStatus::Completed {
                content.push_str(&format!(
                    "### {} ({})\n\n{}\n\n",
                    response.model_name, model_id, response.content
                ));
            }
        }
    }

    for debate in &session.debates {
        content.push_str("## Debate\n\n");
        for round in &debate.rounds {
            content.push_str(&format!(
                "### Round {} - {}\n\n",
                round.round_number, round.topic
            ));
            for resp in &round.responses {
                content.push_str(&format!(
                    "**{} ({}):**\n\n{}\n\n",
                    resp.model_name, resp.model_id, resp.content
                ));
            }
        }
    }

    // Create note via knowledge store
    let mut ks = state.knowledge_store.write().await;

    let note = ks
        .create_note(NoteCreate {
            title: session.title.clone(),
            content,
            relative_path: None,
            aliases: Vec::new(),
            status: NoteStatus::Evidence,
            tags: session.tags.clone(),
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            properties: HashMap::new(),
        })
        .map_err(|e| e.to_string())?;

    drop(ks);

    commit_note_write(state.inner(), &note.id, "note_exported").await?;

    append_canvas_trace(
        state.twin_store.clone(),
        &session_id,
        TraceEventType::NoteExported,
        json!({
            "note_id": note.id.clone(),
            "title": note.title.clone(),
        }),
    )
    .await;

    Ok(serde_json::json!({
        "note_id": note.id,
        "title": note.title,
        "updated": false,
    }))
}

/// Start a debate between models with streaming via Tauri events
#[tauri::command]
pub async fn start_debate(
    window: tauri::Window,
    session_id: String,
    mut request: DebateStartRequest,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let debate_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();

    // Collect source content from tiles
    let mut store = state.canvas_store.write().await;
    let session = store.get_session(&session_id).map_err(|e| e.to_string())?;
    drop(store);

    let mut source_content = String::new();
    let (source_has_vault_context, source_twin_provider) =
        source_tile_context_provider(&session, &request.source_tile_ids);
    let model_route = {
        let settings = state.settings_service.read().await;
        if source_has_vault_context {
            resolve_model_route(
                &PromptType::Decision,
                &ContextMode::Twin,
                source_twin_provider.as_deref(),
                settings.get(),
            )?
        } else {
            resolve_model_route(
                &PromptType::Standard,
                &ContextMode::KnowledgeSearch,
                None,
                settings.get(),
            )?
        }
    };
    request.participating_models = effective_model_ids(&model_route, &request.participating_models);
    for tile_id in &request.source_tile_ids {
        if let Some(tile) = session.prompt_tiles.iter().find(|t| &t.id == tile_id) {
            source_content.push_str(&format!("Prompt: {}\n", tile.prompt));
            for model_id in &request.participating_models {
                if let Some(resp) = tile.responses.get(model_id) {
                    source_content.push_str(&format!(
                        "{} responded: {}\n",
                        resp.model_name, resp.content
                    ));
                }
            }
        }
    }

    // Calculate position (to the right of source tiles)
    let max_x = session
        .prompt_tiles
        .iter()
        .flat_map(|t| {
            t.responses
                .values()
                .map(|r| r.position.x + r.position.width)
        })
        .fold(0.0_f64, f64::max);

    let debate = Debate {
        id: debate_id.clone(),
        participating_models: request.participating_models.clone(),
        source_tile_ids: request.source_tile_ids.clone(),
        rounds: Vec::new(),
        status: "active".to_string(),
        position: TilePosition {
            x: max_x + 100.0,
            y: 100.0,
            width: 400.0,
            height: 300.0,
        },
        debate_mode: request.debate_mode.clone(),
        reasoning_effort: request.reasoning_effort.clone(),
        created_at: now,
    };

    // Save debate to session
    {
        let mut store = state.canvas_store.write().await;
        store
            .add_debate(&session_id, debate.clone())
            .map_err(|e| e.to_string())?;
    }

    append_canvas_trace(
        state.twin_store.clone(),
        &session_id,
        TraceEventType::DebateStarted,
        json!({
            "debate_id": debate.id.clone(),
            "source_tile_ids": debate.source_tile_ids.clone(),
            "participating_models": debate.participating_models.clone(),
            "debate_mode": debate.debate_mode.clone(),
            "max_rounds": request.max_rounds,
        }),
    )
    .await;

    // Emit debate created
    let _ = window.emit(
        "canvas-stream",
        CanvasStreamEvent::DebateCreated {
            session_id: session_id.clone(),
            debate: debate.clone(),
        },
    );

    // Spawn async task for debate streaming
    let openrouter_arc = state.openrouter.clone();
    let ollama_arc = state.ollama.clone();
    let canvas_store_arc = state.canvas_store.clone();
    let models = request.participating_models.clone();
    let max_rounds = request.max_rounds;
    let reasoning_effort = request.reasoning_effort.clone();
    let provider_route = model_route.provider.clone();
    let debate_id_clone = debate_id.clone();
    let session_id_clone = session_id.clone();

    tauri::async_runtime::spawn(async move {
        let mut debate_state = debate;

        for round_num in 1..=max_rounds {
            let _ = window.emit(
                "canvas-stream",
                CanvasStreamEvent::RoundStart {
                    session_id: session_id_clone.clone(),
                    debate_id: debate_id_clone.clone(),
                    round_number: round_num,
                },
            );

            // Build debate context
            let mut context = format!(
                "You are participating in a structured debate.\n\nOriginal context:\n{}\n\n",
                source_content
            );

            // Add previous rounds for context
            for prev_round in &debate_state.rounds {
                context.push_str(&format!("Round {}:\n", prev_round.round_number));
                for resp in &prev_round.responses {
                    context.push_str(&format!("{}: {}\n", resp.model_name, resp.content));
                }
                context.push('\n');
            }

            context.push_str(&format!(
                "Round {} - Present your analysis. Be concise and insightful. If other models have responded before you, engage with their points.",
                round_num
            ));

            // Stream all models concurrently within this round using JoinSet
            let mut join_set = tokio::task::JoinSet::new();

            for model_id in models.clone() {
                let model_name = model_id.split('/').last().unwrap_or(&model_id).to_string();
                let messages = vec![ChatMessage {
                    role: "user".to_string(),
                    content: context.clone(),
                }];
                let openrouter_arc = openrouter_arc.clone();
                let ollama_arc = ollama_arc.clone();
                let window = window.clone();
                let session_id = session_id_clone.clone();
                let debate_id = debate_id_clone.clone();
                let reasoning_effort = reasoning_effort.clone();
                let provider_route = provider_route.clone();

                join_set.spawn(async move {
                    let stream_result = match provider_route {
                        ModelProviderRoute::Ollama => {
                            let ollama = ollama_arc.read().await;
                            let result = ollama
                                .chat_stream(&model_id, messages, None, Some(0.7))
                                .await;
                            drop(ollama);
                            result.map(|stream| {
                                Box::pin(stream)
                                    as std::pin::Pin<
                                        Box<
                                            dyn futures::Stream<Item = anyhow::Result<String>>
                                                + Send,
                                        >,
                                    >
                            })
                        }
                        ModelProviderRoute::OpenRouter => {
                            let openrouter = openrouter_arc.read().await;
                            let result = openrouter
                                .chat_stream(
                                    &model_id,
                                    messages,
                                    None,
                                    Some(0.7),
                                    None,
                                    Some(reasoning_effort.as_str()),
                                    false,
                                    5,
                                )
                                .await;
                            drop(openrouter);
                            result.map(|stream| {
                                Box::pin(stream)
                                    as std::pin::Pin<
                                        Box<
                                            dyn futures::Stream<Item = anyhow::Result<String>>
                                                + Send,
                                        >,
                                    >
                            })
                        }
                    };

                    match stream_result {
                        Ok(stream) => {
                            let mut stream = stream;
                            let mut full_content = String::new();

                            loop {
                                match tokio::time::timeout(Duration::from_secs(60), stream.next())
                                    .await
                                {
                                    Ok(Some(Ok(chunk))) => {
                                        if !chunk.is_empty() {
                                            full_content.push_str(&chunk);
                                            let _ = window.emit(
                                                "canvas-stream",
                                                CanvasStreamEvent::DebateChunk {
                                                    session_id: session_id.clone(),
                                                    debate_id: debate_id.clone(),
                                                    model_id: model_id.clone(),
                                                    chunk,
                                                    round_number: round_num,
                                                },
                                            );
                                        }
                                    }
                                    Ok(Some(Err(e))) => {
                                        let _ = window.emit(
                                            "canvas-stream",
                                            CanvasStreamEvent::DebateError {
                                                session_id: session_id.clone(),
                                                debate_id: debate_id.clone(),
                                                model_id: model_id.clone(),
                                                error: e.to_string(),
                                                round_number: round_num,
                                            },
                                        );
                                        return DebateResponse {
                                            model_id,
                                            model_name,
                                            content: full_content,
                                            stance: None,
                                        };
                                    }
                                    Ok(None) => break, // Stream ended naturally
                                    Err(_) => {
                                        let _ = window.emit(
                                            "canvas-stream",
                                            CanvasStreamEvent::DebateError {
                                                session_id: session_id.clone(),
                                                debate_id: debate_id.clone(),
                                                model_id: model_id.clone(),
                                                error: "Stream idle timeout (60s)".to_string(),
                                                round_number: round_num,
                                            },
                                        );
                                        return DebateResponse {
                                            model_id,
                                            model_name,
                                            content: full_content,
                                            stance: None,
                                        };
                                    }
                                }
                            }

                            let _ = window.emit(
                                "canvas-stream",
                                CanvasStreamEvent::ModelComplete {
                                    session_id: session_id.clone(),
                                    debate_id: debate_id.clone(),
                                    model_id: model_id.clone(),
                                    round_number: round_num,
                                },
                            );

                            DebateResponse {
                                model_id,
                                model_name,
                                content: full_content,
                                stance: None,
                            }
                        }
                        Err(e) => {
                            let _ = window.emit(
                                "canvas-stream",
                                CanvasStreamEvent::DebateError {
                                    session_id: session_id.clone(),
                                    debate_id: debate_id.clone(),
                                    model_id: model_id.clone(),
                                    error: e.to_string(),
                                    round_number: round_num,
                                },
                            );
                            let _ = window.emit(
                                "canvas-stream",
                                CanvasStreamEvent::ModelComplete {
                                    session_id: session_id.clone(),
                                    debate_id: debate_id.clone(),
                                    model_id: model_id.clone(),
                                    round_number: round_num,
                                },
                            );
                            DebateResponse {
                                model_id,
                                model_name,
                                content: e.to_string(),
                                stance: None,
                            }
                        }
                    }
                });
            }

            // Collect all model responses from this round
            let mut round_responses = Vec::new();
            while let Some(result) = join_set.join_next().await {
                if let Ok(response) = result {
                    round_responses.push(response);
                }
            }

            // Save round
            let round = DebateRound {
                round_number: round_num,
                topic: format!("Round {}", round_num),
                responses: round_responses,
                created_at: Utc::now(),
            };
            debate_state.rounds.push(round);

            // Persist after each round. If this fails, surface it instead of
            // silently continuing to stream rounds that will never survive a
            // session reopen.
            let round_persisted = {
                let mut store = canvas_store_arc.write().await;
                match store.update_debate(&session_id_clone, &debate_state) {
                    Ok(()) => true,
                    Err(error) => {
                        emit_debate_persist_error(
                            &window,
                            &session_id_clone,
                            &debate_id_clone,
                            round_num,
                            &debate_state,
                            &error,
                        );
                        false
                    }
                }
            };

            if !round_persisted {
                return;
            }
        }

        // Mark debate as complete
        debate_state.status = "completed".to_string();
        let final_round_number = debate_state
            .rounds
            .last()
            .map(|round| round.round_number)
            .unwrap_or(max_rounds);
        let completion_persisted = {
            let mut store = canvas_store_arc.write().await;
            match store.update_debate(&session_id_clone, &debate_state) {
                Ok(()) => true,
                Err(error) => {
                    emit_debate_persist_error(
                        &window,
                        &session_id_clone,
                        &debate_id_clone,
                        final_round_number,
                        &debate_state,
                        &error,
                    );
                    false
                }
            }
        };

        if completion_persisted {
            let _ = window.emit(
                "canvas-stream",
                CanvasStreamEvent::DebateComplete {
                    session_id: session_id_clone,
                    debate_id: debate_id_clone,
                },
            );
        }
    });

    Ok(debate_id)
}

/// Continue a debate with a new round
#[tauri::command]
pub async fn continue_debate(
    window: tauri::Window,
    session_id: String,
    debate_id: String,
    request: DebateContinueRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    let session = store.get_session(&session_id).map_err(|e| e.to_string())?;
    drop(store);

    let debate = session
        .debates
        .iter()
        .find(|d| d.id == debate_id)
        .ok_or_else(|| "Debate not found".to_string())?
        .clone();
    let (source_has_vault_context, source_twin_provider) =
        source_tile_context_provider(&session, &debate.source_tile_ids);
    let model_route = {
        let settings = state.settings_service.read().await;
        if source_has_vault_context {
            resolve_model_route(
                &PromptType::Decision,
                &ContextMode::Twin,
                source_twin_provider.as_deref(),
                settings.get(),
            )?
        } else {
            resolve_model_route(
                &PromptType::Standard,
                &ContextMode::KnowledgeSearch,
                None,
                settings.get(),
            )?
        }
    };

    append_canvas_trace(
        state.twin_store.clone(),
        &session_id,
        TraceEventType::DebateContinued,
        json!({
            "debate_id": debate_id.clone(),
            "prompt": request.prompt.clone(),
            "participating_models": debate.participating_models.clone(),
        }),
    )
    .await;

    let openrouter_arc = state.openrouter.clone();
    let ollama_arc = state.ollama.clone();
    let canvas_store_arc = state.canvas_store.clone();
    let models = effective_model_ids(&model_route, &debate.participating_models);
    let reasoning_effort = request.reasoning_effort.clone();
    let provider_route = model_route.provider.clone();

    tauri::async_runtime::spawn(async move {
        let mut debate_state = debate;
        let round_num = debate_state.rounds.len() as u32 + 1;

        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::RoundStart {
                session_id: session_id.clone(),
                debate_id: debate_id.clone(),
                round_number: round_num,
            },
        );

        // Build context from previous rounds + new prompt
        let mut context = String::from("Previous debate rounds:\n\n");
        for round in &debate_state.rounds {
            context.push_str(&format!("Round {}:\n", round.round_number));
            for resp in &round.responses {
                context.push_str(&format!("{}: {}\n", resp.model_name, resp.content));
            }
            context.push('\n');
        }
        context.push_str(&format!(
            "New prompt: {}\n\nRespond to this new direction.",
            request.prompt
        ));

        // Stream all models concurrently using JoinSet
        let mut join_set = tokio::task::JoinSet::new();

        for model_id in models {
            let model_name = model_id.split('/').last().unwrap_or(&model_id).to_string();
            let messages = vec![ChatMessage {
                role: "user".to_string(),
                content: context.clone(),
            }];
            let openrouter_arc = openrouter_arc.clone();
            let ollama_arc = ollama_arc.clone();
            let window = window.clone();
            let session_id = session_id.clone();
            let debate_id = debate_id.clone();
            let reasoning_effort = reasoning_effort.clone();
            let provider_route = provider_route.clone();

            join_set.spawn(async move {
                let stream_result = match provider_route {
                    ModelProviderRoute::Ollama => {
                        let ollama = ollama_arc.read().await;
                        let result = ollama
                            .chat_stream(&model_id, messages, None, Some(0.7))
                            .await;
                        drop(ollama);
                        result.map(|stream| {
                            Box::pin(stream)
                                as std::pin::Pin<
                                    Box<dyn futures::Stream<Item = anyhow::Result<String>> + Send>,
                                >
                        })
                    }
                    ModelProviderRoute::OpenRouter => {
                        let openrouter = openrouter_arc.read().await;
                        let result = openrouter
                            .chat_stream(
                                &model_id,
                                messages,
                                None,
                                Some(0.7),
                                None,
                                Some(reasoning_effort.as_str()),
                                false,
                                5,
                            )
                            .await;
                        drop(openrouter);
                        result.map(|stream| {
                            Box::pin(stream)
                                as std::pin::Pin<
                                    Box<dyn futures::Stream<Item = anyhow::Result<String>> + Send>,
                                >
                        })
                    }
                };

                match stream_result {
                    Ok(stream) => {
                        let mut stream = stream;
                        let mut full_content = String::new();

                        loop {
                            match tokio::time::timeout(Duration::from_secs(60), stream.next()).await
                            {
                                Ok(Some(Ok(chunk))) => {
                                    if !chunk.is_empty() {
                                        full_content.push_str(&chunk);
                                        let _ = window.emit(
                                            "canvas-stream",
                                            CanvasStreamEvent::DebateChunk {
                                                session_id: session_id.clone(),
                                                debate_id: debate_id.clone(),
                                                model_id: model_id.clone(),
                                                chunk,
                                                round_number: round_num,
                                            },
                                        );
                                    }
                                }
                                Ok(Some(Err(e))) => {
                                    let _ = window.emit(
                                        "canvas-stream",
                                        CanvasStreamEvent::DebateError {
                                            session_id: session_id.clone(),
                                            debate_id: debate_id.clone(),
                                            model_id: model_id.clone(),
                                            error: e.to_string(),
                                            round_number: round_num,
                                        },
                                    );
                                    return DebateResponse {
                                        model_id,
                                        model_name,
                                        content: full_content,
                                        stance: None,
                                    };
                                }
                                Ok(None) => break, // Stream ended naturally
                                Err(_) => {
                                    let _ = window.emit(
                                        "canvas-stream",
                                        CanvasStreamEvent::DebateError {
                                            session_id: session_id.clone(),
                                            debate_id: debate_id.clone(),
                                            model_id: model_id.clone(),
                                            error: "Stream idle timeout (60s)".to_string(),
                                            round_number: round_num,
                                        },
                                    );
                                    return DebateResponse {
                                        model_id,
                                        model_name,
                                        content: full_content,
                                        stance: None,
                                    };
                                }
                            }
                        }

                        let _ = window.emit(
                            "canvas-stream",
                            CanvasStreamEvent::ModelComplete {
                                session_id: session_id.clone(),
                                debate_id: debate_id.clone(),
                                model_id: model_id.clone(),
                                round_number: round_num,
                            },
                        );

                        DebateResponse {
                            model_id,
                            model_name,
                            content: full_content,
                            stance: None,
                        }
                    }
                    Err(e) => {
                        let _ = window.emit(
                            "canvas-stream",
                            CanvasStreamEvent::DebateError {
                                session_id: session_id.clone(),
                                debate_id: debate_id.clone(),
                                model_id: model_id.clone(),
                                error: e.to_string(),
                                round_number: round_num,
                            },
                        );
                        let _ = window.emit(
                            "canvas-stream",
                            CanvasStreamEvent::ModelComplete {
                                session_id: session_id.clone(),
                                debate_id: debate_id.clone(),
                                model_id: model_id.clone(),
                                round_number: round_num,
                            },
                        );
                        DebateResponse {
                            model_id,
                            model_name,
                            content: e.to_string(),
                            stance: None,
                        }
                    }
                }
            });
        }

        // Collect all model responses from this round
        let mut round_responses = Vec::new();
        while let Some(result) = join_set.join_next().await {
            if let Ok(response) = result {
                round_responses.push(response);
            }
        }

        // Save round
        let round = DebateRound {
            round_number: round_num,
            topic: request.prompt,
            responses: round_responses,
            created_at: Utc::now(),
        };
        debate_state.rounds.push(round);

        let persisted = {
            let mut store = canvas_store_arc.write().await;
            match store.update_debate(&session_id, &debate_state) {
                Ok(()) => true,
                Err(error) => {
                    emit_debate_persist_error(
                        &window,
                        &session_id,
                        &debate_id,
                        round_num,
                        &debate_state,
                        &error,
                    );
                    false
                }
            }
        };

        if persisted {
            let _ = window.emit(
                "canvas-stream",
                CanvasStreamEvent::DebateComplete {
                    session_id,
                    debate_id,
                },
            );
        }
    });

    Ok(())
}

/// Add new models to an existing tile (same prompt, new model responses)
#[tauri::command]
pub async fn add_models_to_tile(
    window: tauri::Window,
    session_id: String,
    tile_id: String,
    request: AddModelsRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Get the tile's prompt
    let mut store = state.canvas_store.write().await;
    let session = store.get_session(&session_id).map_err(|e| e.to_string())?;
    drop(store);

    let tile = session
        .prompt_tiles
        .iter()
        .find(|t| t.id == tile_id)
        .ok_or_else(|| "Tile not found".to_string())?
        .clone();
    let model_route = {
        let settings = state.settings_service.read().await;
        resolve_model_route(
            &tile.prompt_type,
            &tile.context_mode,
            tile.twin_llm_provider.as_deref(),
            settings.get(),
        )?
    };
    let model_ids = effective_model_ids(&model_route, &request.model_ids);
    let prompt_request = prompt_request_from_tile(&tile, model_ids.clone(), 0.7);
    let resolved_context = resolve_prompt_context(state.inner(), &session, &prompt_request).await?;

    let now = Utc::now();

    // Calculate positions for new models
    let existing_count = tile.responses.len();
    let prompt_pos = &tile.position;
    let llm_start_x = prompt_pos.x + prompt_pos.width + LLM_NODE_X_GAP;

    // Add initial pending responses and notify frontend before streaming
    let mut new_responses: HashMap<String, ModelResponse> = HashMap::new();
    {
        let mut store = state.canvas_store.write().await;
        let mut session = store.get_session(&session_id).map_err(|e| e.to_string())?;
        if let Some(t) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
            for (i, model_id) in model_ids.iter().enumerate() {
                let model_name = model_id.split('/').last().unwrap_or(model_id).to_string();
                let llm_y = prompt_pos.y + ((existing_count + i) as f64) * LLM_NODE_Y_STEP;
                let response = ModelResponse {
                    id: uuid::Uuid::new_v4().to_string(),
                    model_id: model_id.clone(),
                    model_name,
                    content: String::new(),
                    status: ResponseStatus::Pending,
                    error: None,
                    tokens_used: None,
                    created_at: now,
                    position: TilePosition {
                        x: llm_start_x,
                        y: llm_y,
                        width: LLM_NODE_WIDTH,
                        height: LLM_NODE_HEIGHT,
                    },
                };
                new_responses.insert(model_id.clone(), response.clone());
                t.models.push(model_id.clone());
                t.responses.insert(model_id.clone(), response);
            }
            session.updated_at = Utc::now();
            store.save_session(&session).map_err(|e| e.to_string())?;
        }
    }

    // Notify frontend about new responses before streaming starts
    let _ = window.emit(
        "canvas-stream",
        CanvasStreamEvent::ModelsAdded {
            session_id: session_id.clone(),
            tile_id: tile_id.clone(),
            responses: new_responses,
        },
    );

    // Spawn streaming task
    let openrouter_arc = state.openrouter.clone();
    let ollama_arc = state.ollama.clone();
    let canvas_store_arc = state.canvas_store.clone();
    let twin_store_arc = state.twin_store.clone();
    let messages = resolved_context.messages.clone();
    let system_prompt = resolved_context.system_prompt.clone();
    let web_search = prompt_request.web_search;
    let web_search_max_results = prompt_request.web_search_max_results;
    let reasoning_effort = prompt_request.reasoning_effort.clone();
    let provider_route = model_route.provider.clone();

    tauri::async_runtime::spawn(async move {
        // Stream all new models concurrently using JoinSet
        let mut join_set = tokio::task::JoinSet::new();

        for model_id in model_ids {
            let messages = messages.clone();
            let system_prompt = system_prompt.clone();
            let openrouter_arc = openrouter_arc.clone();
            let window = window.clone();
            let session_id = session_id.clone();
            let tile_id = tile_id.clone();
            let reasoning_effort = reasoning_effort.clone();
            let provider_route = provider_route.clone();
            let ollama_arc = ollama_arc.clone();

            join_set.spawn(async move {
                let stream_result = match provider_route {
                    ModelProviderRoute::Ollama => {
                        let ollama = ollama_arc.read().await;
                        let result = ollama
                            .chat_stream(&model_id, messages, system_prompt.as_deref(), Some(0.7))
                            .await;
                        drop(ollama);
                        result.map(|stream| {
                            Box::pin(stream)
                                as std::pin::Pin<
                                    Box<dyn futures::Stream<Item = anyhow::Result<String>> + Send>,
                                >
                        })
                    }
                    ModelProviderRoute::OpenRouter => {
                        let openrouter = openrouter_arc.read().await;
                        let result = openrouter
                            .chat_stream(
                                &model_id,
                                messages,
                                system_prompt.as_deref(),
                                Some(0.7),
                                None,
                                Some(reasoning_effort.as_str()),
                                web_search,
                                web_search_max_results,
                            )
                            .await;
                        drop(openrouter);
                        result.map(|stream| {
                            Box::pin(stream)
                                as std::pin::Pin<
                                    Box<dyn futures::Stream<Item = anyhow::Result<String>> + Send>,
                                >
                        })
                    }
                };

                match stream_result {
                    Ok(stream) => {
                        let mut stream = stream;
                        let mut full_content = String::new();

                        loop {
                            match tokio::time::timeout(Duration::from_secs(60), stream.next()).await
                            {
                                Ok(Some(Ok(chunk))) => {
                                    if !chunk.is_empty() {
                                        full_content.push_str(&chunk);
                                        let _ = window.emit(
                                            "canvas-stream",
                                            CanvasStreamEvent::Chunk {
                                                session_id: session_id.clone(),
                                                tile_id: tile_id.clone(),
                                                model_id: model_id.clone(),
                                                chunk,
                                            },
                                        );
                                    }
                                }
                                Ok(Some(Err(e))) => {
                                    let error = e.to_string();
                                    emit_canvas_error(
                                        &window,
                                        &session_id,
                                        &tile_id,
                                        &model_id,
                                        &error,
                                    );
                                    return (
                                        model_id,
                                        String::new(),
                                        ResponseStatus::Error,
                                        Some(error),
                                    );
                                }
                                Ok(None) => break,
                                Err(_) => {
                                    let error = "Stream idle timeout (60s)".to_string();
                                    emit_canvas_error(
                                        &window,
                                        &session_id,
                                        &tile_id,
                                        &model_id,
                                        &error,
                                    );
                                    return (
                                        model_id,
                                        full_content,
                                        ResponseStatus::Error,
                                        Some(error),
                                    );
                                }
                            }
                        }

                        finalize_streamed_model_response(
                            &window,
                            &session_id,
                            &tile_id,
                            model_id,
                            full_content,
                        )
                    }
                    Err(e) => {
                        let error = e.to_string();
                        emit_canvas_error(&window, &session_id, &tile_id, &model_id, &error);
                        (model_id, String::new(), ResponseStatus::Error, Some(error))
                    }
                }
            });
        }

        // Wait for all models and collect results
        let mut results = Vec::new();
        while let Some(result) = join_set.join_next().await {
            if let Ok(r) = result {
                results.push(r);
            }
        }

        // Batch update store
        let persistence_ok = {
            let mut store = canvas_store_arc.write().await;
            match store.batch_update_tile_responses(&session_id, &tile_id, &results) {
                Ok(()) => true,
                Err(error) => {
                    let model_ids: Vec<String> = results
                        .iter()
                        .map(|(model_id, _, _, _)| model_id.clone())
                        .collect();
                    emit_persistence_error(&window, &session_id, &tile_id, &model_ids, &error);
                    false
                }
            }
        };

        append_canvas_trace(
            twin_store_arc.clone(),
            &session_id,
            TraceEventType::ModelsAdded,
            json!({
                "tile_id": tile_id.clone(),
                "model_ids": results.iter().map(|(model_id, _, _, _)| model_id.clone()).collect::<Vec<_>>(),
            }),
        )
        .await;
        append_model_result_traces(
            twin_store_arc.clone(),
            &session_id,
            &tile_id,
            "add_models_to_tile",
            &results,
        )
        .await;

        if persistence_ok {
            let _ = window.emit(
                "canvas-stream",
                CanvasStreamEvent::SessionSaved { session_id },
            );
        }
    });

    Ok(())
}

/// Regenerate a single model's response
#[tauri::command]
pub async fn regenerate_response(
    window: tauri::Window,
    session_id: String,
    tile_id: String,
    model_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Get the tile's prompt
    let mut store = state.canvas_store.write().await;
    let session = store.get_session(&session_id).map_err(|e| e.to_string())?;
    drop(store);

    let tile = session
        .prompt_tiles
        .iter()
        .find(|t| t.id == tile_id)
        .ok_or_else(|| "Tile not found".to_string())?;

    let model_route = {
        let settings = state.settings_service.read().await;
        resolve_model_route(
            &tile.prompt_type,
            &tile.context_mode,
            tile.twin_llm_provider.as_deref(),
            settings.get(),
        )?
    };
    let effective_model_id = effective_model_ids(&model_route, std::slice::from_ref(&model_id))
        .into_iter()
        .next()
        .unwrap_or_else(|| model_id.clone());

    if !tile.responses.contains_key(&effective_model_id) {
        return Err("Response not found".to_string());
    }

    let request = prompt_request_from_tile(tile, vec![effective_model_id.clone()], 0.7);
    let resolved_context = resolve_prompt_context(state.inner(), &session, &request).await?;

    // Reset response to streaming
    {
        let mut store = state.canvas_store.write().await;
        let _ = store.update_tile_response(
            &session_id,
            &tile_id,
            &effective_model_id,
            "",
            ResponseStatus::Streaming,
            None,
        );
    }

    let openrouter_arc = state.openrouter.clone();
    let ollama_arc = state.ollama.clone();
    let canvas_store_arc = state.canvas_store.clone();
    let twin_store_arc = state.twin_store.clone();
    let messages = resolved_context.messages.clone();
    let system_prompt = resolved_context.system_prompt.clone();
    let web_search = request.web_search;
    let web_search_max_results = request.web_search_max_results;
    let reasoning_effort = request.reasoning_effort.clone();
    let provider_route = model_route.provider.clone();

    tauri::async_runtime::spawn(async move {
        let model_id = effective_model_id;
        let stream_result = match provider_route {
            ModelProviderRoute::Ollama => {
                let ollama = ollama_arc.read().await;
                let result = ollama
                    .chat_stream(&model_id, messages, system_prompt.as_deref(), Some(0.7))
                    .await;
                drop(ollama);
                result.map(|stream| {
                    Box::pin(stream)
                        as std::pin::Pin<
                            Box<dyn futures::Stream<Item = anyhow::Result<String>> + Send>,
                        >
                })
            }
            ModelProviderRoute::OpenRouter => {
                let openrouter = openrouter_arc.read().await;
                let result = openrouter
                    .chat_stream(
                        &model_id,
                        messages,
                        system_prompt.as_deref(),
                        Some(0.7),
                        None,
                        Some(reasoning_effort.as_str()),
                        web_search,
                        web_search_max_results,
                    )
                    .await;
                drop(openrouter);
                result.map(|stream| {
                    Box::pin(stream)
                        as std::pin::Pin<
                            Box<dyn futures::Stream<Item = anyhow::Result<String>> + Send>,
                        >
                })
            }
        };

        let mut persistence_ok = true;

        match stream_result {
            Ok(stream) => {
                let mut stream = stream;
                let mut full_content = String::new();
                let mut final_error: Option<String> = None;

                loop {
                    match tokio::time::timeout(Duration::from_secs(60), stream.next()).await {
                        Ok(Some(Ok(chunk))) => {
                            if !chunk.is_empty() {
                                full_content.push_str(&chunk);
                                let _ = window.emit(
                                    "canvas-stream",
                                    CanvasStreamEvent::Chunk {
                                        session_id: session_id.clone(),
                                        tile_id: tile_id.clone(),
                                        model_id: model_id.clone(),
                                        chunk,
                                    },
                                );
                            }
                        }
                        Ok(Some(Err(e))) => {
                            let error = e.to_string();
                            emit_canvas_error(&window, &session_id, &tile_id, &model_id, &error);
                            final_error = Some(error);
                            break;
                        }
                        Ok(None) => break, // Stream ended naturally
                        Err(_) => {
                            let error = "Stream idle timeout (60s)".to_string();
                            emit_canvas_error(&window, &session_id, &tile_id, &model_id, &error);
                            final_error = Some(error);
                            break;
                        }
                    }
                }

                let (final_content, final_status) = if final_error.is_some() {
                    (full_content, ResponseStatus::Error)
                } else {
                    let update =
                        classify_streamed_model_response(model_id.clone(), full_content.clone());
                    final_error = update.3.clone();
                    if let Some(error) = final_error.as_deref() {
                        emit_canvas_error(&window, &session_id, &tile_id, &model_id, error);
                    } else {
                        emit_canvas_complete(&window, &session_id, &tile_id, &model_id);
                    }
                    (update.1, update.2)
                };

                {
                    let mut store = canvas_store_arc.write().await;
                    if let Err(persist_error) = store.update_tile_response(
                        &session_id,
                        &tile_id,
                        &model_id,
                        &final_content,
                        final_status.clone(),
                        final_error.as_deref(),
                    ) {
                        emit_persistence_error(
                            &window,
                            &session_id,
                            &tile_id,
                            std::slice::from_ref(&model_id),
                            &persist_error,
                        );
                        persistence_ok = false;
                    }
                }

                append_canvas_trace(
                    twin_store_arc.clone(),
                    &session_id,
                    TraceEventType::ResponseRegenerated,
                    json!({
                        "tile_id": tile_id.clone(),
                        "model_id": model_id.clone(),
                        "status": final_status,
                        "error": final_error.clone(),
                        "content": final_content.clone(),
                    }),
                )
                .await;
            }
            Err(e) => {
                let error = e.to_string();
                {
                    let mut store = canvas_store_arc.write().await;
                    if let Err(persist_error) = store.update_tile_response(
                        &session_id,
                        &tile_id,
                        &model_id,
                        "",
                        ResponseStatus::Error,
                        Some(error.as_str()),
                    ) {
                        emit_persistence_error(
                            &window,
                            &session_id,
                            &tile_id,
                            std::slice::from_ref(&model_id),
                            &persist_error,
                        );
                        persistence_ok = false;
                    }
                }
                emit_canvas_error(&window, &session_id, &tile_id, &model_id, &error);
                append_canvas_trace(
                    twin_store_arc.clone(),
                    &session_id,
                    TraceEventType::ResponseRegenerated,
                    json!({
                        "tile_id": tile_id.clone(),
                        "model_id": model_id.clone(),
                        "status": ResponseStatus::Error,
                        "error": error,
                        "content": "",
                    }),
                )
                .await;
            }
        }

        if persistence_ok {
            let _ = window.emit(
                "canvas-stream",
                CanvasStreamEvent::SessionSaved { session_id },
            );
        }
    });

    Ok(())
}

fn prompt_request_from_tile(
    tile: &PromptTile,
    models: Vec<String>,
    temperature: f64,
) -> PromptRequest {
    PromptRequest {
        prompt: tile.prompt.clone(),
        prompt_type: tile.prompt_type.clone(),
        system_prompt: tile.system_prompt.clone(),
        models,
        position: None,
        context_mode: tile.context_mode.clone(),
        twin_answer_mode: tile.twin_answer_mode.clone(),
        twin_context_policy: tile.twin_context_policy.clone(),
        twin_llm_provider: tile.twin_llm_provider.clone(),
        decision_metadata: tile.decision_metadata.clone(),
        parent_tile_id: tile.parent_tile_id.clone(),
        parent_model_id: tile.parent_model_id.clone(),
        temperature,
        max_tokens: None,
        web_search: tile.web_search,
        web_search_max_results: tile.web_search_max_results,
        reasoning_effort: tile.reasoning_effort.clone(),
    }
}

async fn append_model_result_traces(
    twin_store_arc: Arc<RwLock<TwinStore>>,
    session_id: &str,
    tile_id: &str,
    trigger: &str,
    results: &[StreamedResponseUpdate],
) {
    for (model_id, content, status, error) in results {
        let event_type = if *status == ResponseStatus::Error {
            TraceEventType::ResponseErrored
        } else {
            TraceEventType::ResponseCompleted
        };

        append_canvas_trace(
            twin_store_arc.clone(),
            session_id,
            event_type,
            json!({
                "tile_id": tile_id,
                "model_id": model_id,
                "trigger": trigger,
                "status": status,
                "content": content,
                "error": error,
            }),
        )
        .await;
    }
}

fn emit_canvas_error(
    window: &tauri::Window,
    session_id: &str,
    tile_id: &str,
    model_id: &str,
    error: &str,
) {
    let _ = window.emit(
        "canvas-stream",
        CanvasStreamEvent::Error {
            session_id: session_id.to_string(),
            tile_id: tile_id.to_string(),
            model_id: model_id.to_string(),
            error: error.to_string(),
        },
    );
}

/// Emit an Error event for every model whose response was streamed
/// successfully but then failed to persist to the session file (disk full,
/// file locked, etc.). Without this, the frontend has already rendered a
/// complete response, `SessionSaved` would otherwise still fire, and the
/// content silently reverts to an empty Pending stub the next time the
/// session is reopened — with no error anywhere in the UI.
fn emit_persistence_error(
    window: &tauri::Window,
    session_id: &str,
    tile_id: &str,
    model_ids: &[String],
    error: &anyhow::Error,
) {
    log::error!(
        "Failed to persist canvas tile '{}' responses for session '{}': {}",
        tile_id,
        session_id,
        error
    );

    let message = format!("Failed to save response: {}", error);
    for model_id in model_ids {
        emit_canvas_error(window, session_id, tile_id, model_id, &message);
    }
}

/// Same contract as `emit_persistence_error` but for debate rounds, which
/// don't have a single tile/model — a round can involve several models at
/// once. Emits a `DebateError` per participating model in the round that
/// failed to persist so the frontend has something concrete to render.
fn emit_debate_persist_error(
    window: &tauri::Window,
    session_id: &str,
    debate_id: &str,
    round_number: u32,
    debate_state: &Debate,
    error: &anyhow::Error,
) {
    log::error!(
        "Failed to persist debate '{}' round {} for session '{}': {}",
        debate_id,
        round_number,
        session_id,
        error
    );

    let model_ids: Vec<String> = debate_state
        .rounds
        .last()
        .map(|round| round.responses.iter().map(|r| r.model_id.clone()).collect())
        .unwrap_or_default();

    let message = format!("Failed to save debate round: {}", error);

    if model_ids.is_empty() {
        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::DebateError {
                session_id: session_id.to_string(),
                debate_id: debate_id.to_string(),
                model_id: "system".to_string(),
                error: message,
                round_number,
            },
        );
        return;
    }

    for model_id in model_ids {
        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::DebateError {
                session_id: session_id.to_string(),
                debate_id: debate_id.to_string(),
                model_id,
                error: message.clone(),
                round_number,
            },
        );
    }
}

fn emit_canvas_complete(window: &tauri::Window, session_id: &str, tile_id: &str, model_id: &str) {
    let _ = window.emit(
        "canvas-stream",
        CanvasStreamEvent::Complete {
            session_id: session_id.to_string(),
            tile_id: tile_id.to_string(),
            model_id: model_id.to_string(),
            tokens_used: None,
        },
    );
}

fn finalize_streamed_model_response(
    window: &tauri::Window,
    session_id: &str,
    tile_id: &str,
    model_id: String,
    full_content: String,
) -> StreamedResponseUpdate {
    let update = classify_streamed_model_response(model_id, full_content);

    if let Some(error) = &update.3 {
        emit_canvas_error(window, session_id, tile_id, &update.0, error);
    } else {
        emit_canvas_complete(window, session_id, tile_id, &update.0);
    }

    update
}

fn classify_streamed_model_response(
    model_id: String,
    full_content: String,
) -> StreamedResponseUpdate {
    if full_content.trim().is_empty() {
        (
            model_id,
            String::new(),
            ResponseStatus::Error,
            Some(EMPTY_MODEL_RESPONSE_ERROR.to_string()),
        )
    } else {
        (model_id, full_content, ResponseStatus::Completed, None)
    }
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

async fn resolve_prompt_context(
    state: &AppState,
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<ResolvedPromptContext, String> {
    let messages = build_canvas_messages(session, request)?;

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
    let (
        setup,
        approved_twin_records,
        candidate_twin_records,
        constitution_items,
        action_gaps,
        decision_cases,
    ) = {
        let mut twin_store = state.twin_store.write().await;
        let setup = twin_store
            .get_constitution_setup()
            .map_err(|error| error.to_string())?;
        validate_twin_identity_for_answer_mode(&setup, &request.twin_answer_mode)?;
        let (approved, candidate) = twin_store
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
            approved,
            candidate,
            constitution_items,
            action_gaps,
            decision_cases,
        )
    };

    let selection = apply_twin_context_budget(
        decision_cases,
        constitution_items,
        approved_twin_records,
        candidate_twin_records,
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
    })
}

fn build_canvas_messages(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ChatMessage>, String> {
    match request.context_mode {
        ContextMode::FullHistory => build_full_history_messages(session, request),
        ContextMode::Compact => build_compact_history_messages(session, request),
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

fn build_compact_history_messages(
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<Vec<ChatMessage>, String> {
    let turns = build_selected_parent_chain(session, request)?;
    let mut messages = Vec::new();

    if turns.len() > COMPACT_HISTORY_RECENT_TURNS {
        let split_at = turns.len() - COMPACT_HISTORY_RECENT_TURNS;
        let summary = build_compact_history_summary(&turns[..split_at]);
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: summary,
        });

        for turn in &turns[split_at..] {
            messages.push(ChatMessage {
                role: "user".to_string(),
                content: turn.prompt.clone(),
            });
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: turn.response.clone(),
            });
        }
    } else {
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
        let response = tile.responses.get(&model_id).ok_or_else(|| {
            format!(
                "Parent response not found for tile {} and model {}",
                tile_id, model_id
            )
        })?;

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

fn build_compact_history_summary(turns: &[ConversationTurn]) -> String {
    let mut summary = String::from("Conversation summary before the most recent turns:\n");

    for (index, turn) in turns.iter().enumerate() {
        summary.push_str(&format!(
            "\nTurn {}:\nUser: {}\nAssistant ({}): {}\n",
            index + 1,
            truncate_for_compact_history(&turn.prompt),
            turn.model_id,
            truncate_for_compact_history(&turn.response),
        ));
    }

    summary
}

fn truncate_for_compact_history(content: &str) -> String {
    if content.chars().count() <= COMPACT_HISTORY_EXCERPT_CHARS {
        return content.to_string();
    }

    let mut truncated = content
        .chars()
        .take(COMPACT_HISTORY_EXCERPT_CHARS)
        .collect::<String>();
    truncated.push_str("...");
    truncated
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
async fn run_sealed_twin_prediction(
    twin_store: Arc<RwLock<TwinStore>>,
    openrouter: Arc<RwLock<OpenRouterService>>,
    ollama: Arc<RwLock<OllamaService>>,
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
    let built = {
        let mut store = twin_store.write().await;
        let setup = match store.get_constitution_setup() {
            Ok(setup) => setup,
            Err(error) => {
                log::warn!("Sealed prediction setup load failed for {episode_id}: {error}");
                let _ = store.mark_twin_prediction_failed(&episode_id);
                return;
            }
        };

        let system_prompt = if let Some(prompt) = twin_context_prompt {
            prompt
        } else {
            // Non-Twin-context decision tile: build a twin-only context on
            // the spot so predictions cover every decision, not just
            // Twin-mode ones (skipping them would bias the eval sample).
            let query = decision_context_query(&prompt, decision_metadata.as_ref());
            let selections =
                store
                    .select_context_records(&prompt)
                    .and_then(|(approved, candidates)| {
                        let (constitution_items, action_gaps) =
                            store.select_constitution_context(&query)?;
                        let cases = store.select_decision_cases(
                            &query,
                            Some(&episode_id),
                            MAX_TWIN_CASE_CONTEXT,
                        )?;
                        Ok((approved, candidates, constitution_items, action_gaps, cases))
                    });
            match selections {
                Ok((approved, candidates, constitution_items, action_gaps, cases)) => {
                    let selection = apply_twin_context_budget(
                        cases,
                        constitution_items,
                        approved,
                        candidates,
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
                }
                Err(error) => {
                    log::warn!("Sealed prediction context build failed for {episode_id}: {error}");
                    let _ = store.mark_twin_prediction_failed(&episode_id);
                    return;
                }
            }
        };

        let user_message =
            build_twin_prediction_user_message(&setup, &decision, &options, stakes.as_deref());
        (system_prompt, user_message)
    };
    let (system_prompt, user_message) = built;

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: user_message,
    }];
    let result = match provider_route {
        ModelProviderRoute::Ollama => {
            let ollama = ollama.read().await;
            ollama
                .chat(&prediction_model, messages, Some(&system_prompt), Some(0.2))
                .await
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

    match result {
        Ok(raw) => {
            let draft = parse_twin_prediction(&raw, &options);
            let mut store = twin_store.write().await;
            if let Err(error) = store.attach_twin_prediction(
                &episode_id,
                draft,
                &prediction_model,
                &context_version,
            ) {
                log::warn!("Failed to seal twin prediction for {episode_id}: {error}");
            }
        }
        Err(error) => {
            log::warn!("Sealed twin prediction call failed for {episode_id}: {error}");
            let mut store = twin_store.write().await;
            let _ = store.mark_twin_prediction_failed(&episode_id);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_twin_context_prompt(
    setup: &ConstitutionSetup,
    decision_cases: &[DecisionEpisode],
    notes: &[(String, String, String)],
    approved_records: &[TwinContextRecord],
    candidate_records: &[TwinContextRecord],
    constitution_items: &[ConstitutionItem],
    action_gaps: &[ActionGap],
    answer_mode: &TwinAnswerMode,
    prompt_type: &PromptType,
    decision_metadata: Option<&DecisionPromptMetadata>,
) -> String {
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
        prompt.push_str("No endorsed or auto-promoted user records were selected.\n\n");
    } else {
        for record in approved_records {
            prompt.push_str(&format_twin_record(record));
        }
        prompt.push('\n');
    }

    prompt.push_str("## Tentative Candidate Records\n\n");
    if candidate_records.is_empty() {
        prompt.push_str("No relevant candidate records were selected.\n\n");
    } else {
        prompt.push_str("These are unreviewed hypotheses. Use them lightly and disclose when they affect the answer.\n");
        for record in candidate_records {
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
             Use candidate records only as tentative context. Separate what is grounded in Constitution, evidence, records, and your recommendation. \
             When the user asks for a choice or recommendation, include: Recommended option, Constitution principles used, Supporting evidence, Uncertainty, and What would change the recommendation. \
             Cite Constitution item ids and note titles where they affect the answer.\n",
        ),
        TwinAnswerMode::Simulation => prompt.push_str(
            "Answer in first person from the configured Twin Identity. Use approved records as stronger style and preference evidence; mention candidate influence as tentative when relevant. \
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
                 Treat every self-model claim as a hypothesis, not identity. Say where the claim is grounded in vault notes, approved records, or tentative records. \
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
mod tests {
    use super::*;
    use crate::models::twin::ConstitutionSetup;
    use chrono::Utc;

    fn build_response(model_id: &str, content: &str) -> ModelResponse {
        ModelResponse {
            id: format!("resp-{}", model_id),
            model_id: model_id.to_string(),
            model_name: model_id.to_string(),
            content: content.to_string(),
            status: ResponseStatus::Completed,
            error: None,
            tokens_used: None,
            created_at: Utc::now(),
            position: TilePosition::default(),
        }
    }

    fn build_tile(
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
            created_at: Utc::now(),
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
            temperature: 0.7,
            max_tokens: None,
            web_search: false,
            web_search_max_results: 5,
            reasoning_effort: "none".to_string(),
        }
    }

    fn build_root_request(prompt: &str, context_mode: ContextMode) -> PromptRequest {
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
            parent_tile_id: None,
            parent_model_id: None,
            temperature: 0.7,
            max_tokens: None,
            web_search: false,
            web_search_max_results: 5,
            reasoning_effort: "none".to_string(),
        }
    }

    fn build_retrieval_result(
        id: &str,
        title: &str,
        snippet: &str,
        score: f32,
        reasons: &[&str],
    ) -> RetrievalResult {
        RetrievalResult {
            note: crate::models::note::NoteMeta {
                id: id.to_string(),
                title: title.to_string(),
                relative_path: format!("{}.md", id),
                aliases: Vec::new(),
                status: NoteStatus::default(),
                tags: Vec::new(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: None,
                optimizer_managed: false,
            },
            score,
            snippet: snippet.to_string(),
            relevance_reasons: reasons.iter().map(|reason| reason.to_string()).collect(),
        }
    }

    fn build_constitution_item(
        id: &str,
        claim: &str,
        status: crate::models::twin::ConstitutionStatus,
        source_type: &str,
    ) -> ConstitutionItem {
        ConstitutionItem {
            id: id.to_string(),
            claim: claim.to_string(),
            dimension: "values".to_string(),
            scope: vec!["general".to_string()],
            priority: 0.8,
            confidence: 0.82,
            status,
            evidence_refs: vec![crate::models::twin::EvidenceRef {
                trace_id: format!("trace-{}", id),
                event_id: format!("event-{}", id),
                session_id: "session-1".to_string(),
                tile_id: None,
                model_id: None,
                note: Some("Evidence note".to_string()),
                source_type: Some(source_type.to_string()),
                source_id: Some(format!("source-{}", id)),
                source_label: Some("Interview question".to_string()),
                excerpt: Some("Can you give a concrete example?".to_string()),
                speaker_role: Some("user".to_string()),
            }],
            tensions: Vec::new(),
            linked_record_ids: Vec::new(),
            source: Some("interview_behavior_inference".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn build_action_gap(id: &str) -> ActionGap {
        ActionGap {
            id: id.to_string(),
            stated_value: "Protect mission alignment".to_string(),
            revealed_behavior: "Accepts attractive adjacent projects".to_string(),
            driver_hypothesis: Some("Funding pressure".to_string()),
            somatic_taste_signal: Some("Prestige pull".to_string()),
            decision_risk: "May divert faculty from core mission work".to_string(),
            evidence_refs: Vec::new(),
            linked_record_ids: Vec::new(),
            confidence: 0.72,
            status: crate::models::twin::ConstitutionStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn test_twin_identity_setup() -> ConstitutionSetup {
        ConstitutionSetup {
            twin_name: Some("Alex Chen".into()),
            twin_role: Some("founder deciding from product evidence".into()),
            source_boundaries: vec!["Use reviewed notes and uploaded interviews only.".into()],
            ..ConstitutionSetup::default()
        }
    }

    #[test]
    fn test_build_note_context_prompt_with_notes() {
        let notes = vec![
            ("id1".into(), "Note A".into(), "Content of A".into()),
            ("id2".into(), "Note B".into(), "Content of B".into()),
        ];
        let prompt = build_note_context_prompt(&notes);

        assert!(prompt.contains("Note A"));
        assert!(prompt.contains("Content of A"));
        assert!(prompt.contains("Note B"));
        assert!(prompt.contains("id: id1"));
    }

    #[test]
    fn test_build_note_context_prompt_empty() {
        let notes: Vec<(String, String, String)> = vec![];
        let prompt = build_note_context_prompt(&notes);

        assert!(prompt.contains("No relevant notes were found"));
    }

    #[test]
    fn test_build_chunk_context_prompt_groups_by_parent() {
        let chunks = vec![
            ChunkResult {
                chunk_id: "c1".into(),
                parent_note_id: "note-a".into(),
                parent_title: "Note A".into(),
                text: "First paragraph of A".into(),
                start_char: 0,
                end_char: 20,
                depth_score: 1.0,
                search_score: 5.0,
                token_estimate: 10,
            },
            ChunkResult {
                chunk_id: "c2".into(),
                parent_note_id: "note-b".into(),
                parent_title: "Note B".into(),
                text: "Content of B".into(),
                start_char: 0,
                end_char: 12,
                depth_score: 1.0,
                search_score: 4.0,
                token_estimate: 8,
            },
            ChunkResult {
                chunk_id: "c3".into(),
                parent_note_id: "note-a".into(),
                parent_title: "Note A".into(),
                text: "Second paragraph of A".into(),
                start_char: 21,
                end_char: 42,
                depth_score: 0.5,
                search_score: 3.5,
                token_estimate: 10,
            },
        ];
        let prompt = build_chunk_context_prompt(&chunks);

        // Both chunks from Note A should be under the same heading
        assert!(prompt.contains("### Note A (id: note-a)"));
        assert!(prompt.contains("First paragraph of A"));
        assert!(prompt.contains("Second paragraph of A"));
        assert!(prompt.contains("### Note B (id: note-b)"));
        assert!(prompt.contains("Content of B"));
        // Note A should appear before Note B (insertion order from chunks)
        let a_pos = prompt.find("Note A").unwrap();
        let b_pos = prompt.find("Note B").unwrap();
        assert!(a_pos < b_pos);
    }

    #[test]
    fn test_build_chunk_context_prompt_empty() {
        let chunks: Vec<ChunkResult> = vec![];
        let prompt = build_chunk_context_prompt(&chunks);
        assert!(prompt.contains("No relevant notes were found"));
    }

    #[test]
    fn twin_context_prompt_separates_approved_candidates_and_advisor_instructions() {
        let approved = vec![TwinContextRecord {
            id: "record-approved".into(),
            kind: crate::models::twin::UserRecordKind::Preference,
            content: "User prefers evidence-backed implementation detail.".into(),
            confidence: 0.9,
            promotion_state: crate::models::twin::PromotionState::Endorsed,
            evidence_count: 4,
            source_label: Some("approved".into()),
        }];
        let candidates = vec![TwinContextRecord {
            id: "record-candidate".into(),
            kind: crate::models::twin::UserRecordKind::ReasoningPattern,
            content: "User may prefer red-team critique before shipping.".into(),
            confidence: 0.62,
            promotion_state: crate::models::twin::PromotionState::Candidate,
            evidence_count: 1,
            source_label: Some("candidate".into()),
        }];
        let constitution = vec![
            build_constitution_item(
                "constitution-active",
                "Prefer mission alignment before opportunistic funding.",
                crate::models::twin::ConstitutionStatus::Active,
                "interview-question",
            ),
            build_constitution_item(
                "constitution-candidate",
                "May prefer negotiation before rejection.",
                crate::models::twin::ConstitutionStatus::Candidate,
                "behavior",
            ),
            build_constitution_item(
                "constitution-rejected",
                "Rejected claims must not leak.",
                crate::models::twin::ConstitutionStatus::Rejected,
                "note",
            ),
            build_constitution_item(
                "constitution-not-me",
                "Not-me claims must not leak.",
                crate::models::twin::ConstitutionStatus::NotMe,
                "note",
            ),
            build_constitution_item(
                "constitution-no-train",
                "No-train claims must not leak.",
                crate::models::twin::ConstitutionStatus::NoTrain,
                "note",
            ),
        ];
        let gaps = vec![build_action_gap("gap-1")];

        let prompt = build_twin_context_prompt(
            &ConstitutionSetup::default(),
            &[],
            &[(
                "note-1".into(),
                "Decision Notes".into(),
                "### Message 2: Interviewee\nExpert says accept grants when partnerships are strategic.".into(),
            )],
            &approved,
            &candidates,
            &constitution,
            &gaps,
            &TwinAnswerMode::Advisor,
            &PromptType::Standard,
            None,
        );

        assert!(prompt.contains("## Twin Operating Contract"));
        assert!(prompt.contains("## Reviewed Constitution"));
        assert!(prompt.contains("Prefer mission alignment before opportunistic funding."));
        assert!(prompt.contains("Interview question"));
        assert!(prompt.contains("## Action Gap Risks"));
        assert!(prompt.contains("May divert faculty from core mission work"));
        assert!(prompt.contains("## Relevant Evidence"));
        assert!(prompt.contains("Expert says accept grants when partnerships are strategic."));
        assert!(prompt.contains("## Approved User Records"));
        assert!(prompt.contains("## Tentative Candidate Records"));
        assert!(prompt.contains("Candidate Constitution Hypotheses"));
        assert!(prompt.contains("May prefer negotiation before rejection."));
        assert!(!prompt.contains("Rejected claims must not leak."));
        assert!(!prompt.contains("Not-me claims must not leak."));
        assert!(!prompt.contains("No-train claims must not leak."));
        assert!(prompt.contains("Use interviewee answers as evidence about the interviewee"));
        assert!(prompt.contains("Do not use evidence to justify a preselected answer"));
        assert!(prompt.contains("Recommended option"));
        assert!(prompt.contains("decision-support assistant"));
    }

    #[test]
    fn twin_context_prompt_labels_simulation_mode() {
        let setup = test_twin_identity_setup();
        let prompt = build_twin_context_prompt(
            &setup,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &TwinAnswerMode::Simulation,
            &PromptType::Standard,
            None,
        );

        assert!(prompt.contains("## Twin Identity"));
        assert!(prompt.contains("I am Alex Chen."));
        assert!(prompt.contains("My role/context is founder deciding from product evidence."));
        assert!(prompt.contains("Use reviewed notes and uploaded interviews only."));
        assert!(prompt.contains("Continue my documented reasoning pattern"));
        assert!(prompt.contains("do not append questions unless the user's request asks for them"));
        assert!(!prompt.contains("reflective questions"));
        assert!(!prompt.contains("next question"));
        assert!(!prompt.contains("not the user's actual view"));
        assert!(prompt.contains("## Reviewed Constitution"));
    }

    #[test]
    fn twin_context_prompt_rejects_simulation_without_identity() {
        let setup = ConstitutionSetup::default();

        let result = validate_twin_identity_for_answer_mode(&setup, &TwinAnswerMode::Simulation);

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Twin Identity requires Name and Role / context"));
    }

    #[test]
    fn decision_advisor_prompt_uses_reflection_card_structure() {
        let metadata = DecisionPromptMetadata {
            decision: "Should Grafyn build Decision Mirror first?".into(),
            options: vec!["Decision Mirror".into(), "Topology layer".into()],
            stakes: Some("Product direction".into()),
            initial_leaning: Some("Decision Mirror first".into()),
            review_date: Some("2026-05-15".into()),
        };

        let prompt = build_twin_context_prompt(
            &ConstitutionSetup::default(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &TwinAnswerMode::Advisor,
            &PromptType::Decision,
            Some(&metadata),
        );

        assert!(prompt.contains("Decision Mirror session"));
        assert!(prompt.contains("Reflection Card"));
        assert!(prompt.contains("Blind Spot Hypothesis"));
        assert!(prompt.contains("Recommendation must be derived after the Constitution Check and Evidence From Grafyn sections"));
        assert!(prompt.contains("Decision: Should Grafyn build Decision Mirror first?"));
        assert!(prompt.contains("Topology layer"));
    }

    #[test]
    fn decision_simulation_prompt_uses_base_simulation_without_decision_style_block() {
        let metadata = DecisionPromptMetadata {
            decision: "Should Grafyn build Decision Mirror first?".into(),
            options: vec!["Decision Mirror".into(), "Topology layer".into()],
            stakes: Some("Product direction".into()),
            initial_leaning: Some("Decision Mirror first".into()),
            review_date: Some("2026-05-15".into()),
        };

        let prompt = build_twin_context_prompt(
            &test_twin_identity_setup(),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &TwinAnswerMode::Simulation,
            &PromptType::Decision,
            Some(&metadata),
        );

        assert!(!prompt.contains("Reflection Card"));
        assert!(!prompt.contains("Evidence From Grafyn"));
        assert!(!prompt.contains("Blind Spot Hypothesis"));
        assert!(!prompt.contains("Decision Mirror Simulation Style"));
        assert!(!prompt.contains("Decision Mirror simulation session"));
        assert!(!prompt.contains("natural first-person reflection"));
        assert!(!prompt.contains("numbered headings"));
        assert!(prompt.contains("do not append questions unless the user's request asks for them"));
        assert!(!prompt.contains("not the user's actual view"));
        assert!(prompt.contains("Decision: Should Grafyn build Decision Mirror first?"));
    }

    fn test_decision_case(
        id: &str,
        decision: &str,
        lesson: Option<&str>,
        note: Option<&str>,
    ) -> DecisionEpisode {
        let now = Utc::now();
        DecisionEpisode {
            id: id.to_string(),
            session_id: "session-1".to_string(),
            tile_id: format!("tile-{id}"),
            decision: decision.to_string(),
            options: vec!["Ship now".to_string(), "Wait a sprint".to_string()],
            stakes: None,
            initial_leaning: Some("Ship now".to_string()),
            selected_response: None,
            chosen_option: Some("Wait a sprint".to_string()),
            confidence: None,
            review_date: None,
            outcome: None,
            regret_score: None,
            lesson: lesson.map(|text| text.to_string()),
            missed_something: None,
            primitive_assessment: PrimitiveDecisionAssessment::default(),
            twin_prediction: None,
            prediction_status: None,
            agreement: None,
            correction_note: note.map(|text| text.to_string()),
            context_version: None,
            outcome_recorded_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn twin_context_prompt_renders_past_decision_cases_verbatim() {
        let case = test_decision_case(
            "case-1",
            "Ship the importer before polish?",
            Some("I always regret shipping before the empty states are done."),
            Some("Twin assumed I optimize for speed; I optimize for trust."),
        );

        let prompt = build_twin_context_prompt(
            &ConstitutionSetup::default(),
            &[case],
            &[],
            &[],
            &[],
            &[],
            &[],
            &TwinAnswerMode::Advisor,
            &PromptType::Standard,
            None,
        );

        assert!(prompt.contains("## Past Decision Cases"));
        assert!(prompt.contains("Past decision: Ship the importer before polish?"));
        assert!(prompt.contains("Options: Ship now | Wait a sprint"));
        assert!(prompt.contains("Chose: Wait a sprint"));
        assert!(prompt.contains("I always regret shipping before the empty states are done."));
        assert!(prompt.contains("Twin assumed I optimize for speed; I optimize for trust."));
        assert!(prompt.contains("Weight them above abstracted records"));
    }

    #[test]
    fn twin_context_budget_keeps_cases_and_drops_notes_when_tight() {
        let case = test_decision_case("case-1", "Ship the importer before polish?", None, None);
        let big_note = (
            "note-1".to_string(),
            "Big note".to_string(),
            "evidence ".repeat(400),
        );

        let selection = apply_twin_context_budget(
            vec![case],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![big_note],
            60,
        );

        assert_eq!(selection.cases.len(), 1);
        assert!(selection.notes.is_empty());
    }

    #[test]
    fn sealed_prediction_prompt_is_immersed_with_identity_and_never_meta_framed() {
        let setup = test_twin_identity_setup();
        let options = vec!["Ship now".to_string(), "Wait a sprint".to_string()];
        let user_message = build_twin_prediction_user_message(
            &setup,
            "Ship the importer before polish?",
            &options,
            Some("Launch trust"),
        );
        let system_prompt = build_twin_context_prompt(
            &setup,
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            &TwinAnswerMode::Simulation,
            &PromptType::Decision,
            None,
        );

        assert!(user_message.contains("I am Alex Chen."));
        assert!(user_message.contains("Which option do I choose?"));
        assert!(user_message.contains("1. Ship now"));
        assert!(user_message.contains("2. Wait a sprint"));
        assert!(user_message.contains("predicted_option"));

        // The full model-facing prompt must never meta-frame the twin.
        let full_prompt = format!("{system_prompt}\n{user_message}").to_lowercase();
        for forbidden in [
            "simulate",
            "roleplay",
            "role-play",
            "predict what the user",
            "what would the user",
            "pretend to be",
        ] {
            assert!(
                !full_prompt.contains(forbidden),
                "model-facing prompt contains forbidden meta-framing: {forbidden}"
            );
        }
    }

    #[test]
    fn sealed_prediction_prompt_uses_advisor_framing_without_identity() {
        let options = vec!["Ship now".to_string(), "Wait a sprint".to_string()];
        let user_message = build_twin_prediction_user_message(
            &ConstitutionSetup::default(),
            "Ship the importer before polish?",
            &options,
            None,
        );

        assert!(!user_message.contains("I am "));
        assert!(user_message.contains("best fits this decision-maker's"));
        assert!(user_message.contains("predicted_option"));
    }

    #[test]
    fn test_build_selected_parent_chain_returns_root_to_leaf_order() {
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
                "Follow-up prompt",
                "openai/gpt-4",
                "Follow-up response",
                Some("tile-1"),
                Some("openai/gpt-4"),
            ),
            build_tile(
                "tile-3",
                "Deep prompt",
                "openai/gpt-4",
                "Deep response",
                Some("tile-2"),
                Some("openai/gpt-4"),
            ),
        ]);
        let request = build_request(
            "Newest prompt",
            "tile-3",
            "openai/gpt-4",
            ContextMode::FullHistory,
        );

        let turns = build_selected_parent_chain(&session, &request).unwrap();

        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].prompt, "Root prompt");
        assert_eq!(turns[1].prompt, "Follow-up prompt");
        assert_eq!(turns[2].prompt, "Deep prompt");
    }

    #[test]
    fn test_build_full_history_messages_interleaves_user_and_assistant_turns() {
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
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Root prompt");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "Root response");
        assert_eq!(messages[2].content, "Branch prompt");
        assert_eq!(messages[3].content, "Branch response");
        assert_eq!(messages[4].content, "Final prompt");
    }

    #[test]
    fn test_root_prompt_without_parent_ids_ignores_unrelated_canvas_tiles() {
        let session = build_session(vec![
            build_tile(
                "tile-1",
                "Unrelated root prompt",
                "openai/gpt-4",
                "Unrelated root response",
                None,
                None,
            ),
            build_tile(
                "tile-2",
                "Unrelated branch prompt",
                "openai/gpt-4",
                "Unrelated branch response",
                Some("tile-1"),
                Some("openai/gpt-4"),
            ),
        ]);
        let request = build_root_request("Fresh root prompt", ContextMode::None);

        let messages = build_canvas_messages(&session, &request).unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Fresh root prompt");
    }

    #[test]
    fn test_build_compact_history_messages_summarizes_older_turns() {
        let session = build_session(vec![
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
        ]);
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::Compact);

        let messages = build_compact_history_messages(&session, &request).unwrap();

        assert_eq!(messages.len(), 6);
        assert!(messages[0]
            .content
            .contains("Conversation summary before the most recent turns"));
        assert!(messages[0].content.contains("Prompt 1"));
        assert!(messages[1].content.contains("Prompt 3"));
        assert!(messages[2].content.contains("Response 3"));
        assert_eq!(messages[5].content, "Prompt 5");
    }

    #[test]
    fn test_build_selected_parent_chain_errors_when_parent_response_is_missing() {
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
                "anthropic/claude",
                "Branch response",
                Some("tile-1"),
                Some("openai/gpt-4"),
            ),
        ]);
        let request = build_request(
            "Next prompt",
            "tile-2",
            "openai/gpt-4",
            ContextMode::FullHistory,
        );

        let err = build_selected_parent_chain(&session, &request).unwrap_err();

        assert!(err.contains("Parent response not found"));
    }

    #[test]
    fn test_prompt_request_from_tile_preserves_web_search_settings() {
        let tile = PromptTile {
            web_search: true,
            web_search_max_results: 8,
            reasoning_effort: "high".to_string(),
            ..build_tile("tile-1", "Prompt", "openai/gpt-4", "Response", None, None)
        };

        let request = prompt_request_from_tile(&tile, vec!["openai/gpt-4".to_string()], 0.3);

        assert!(request.web_search);
        assert_eq!(request.web_search_max_results, 8);
        assert_eq!(request.temperature, 0.3);
        assert_eq!(request.max_tokens, None);
        assert_eq!(request.reasoning_effort, "high");
    }

    #[test]
    fn test_prompt_request_from_tile_preserves_twin_settings() {
        let tile = PromptTile {
            context_mode: ContextMode::Twin,
            twin_answer_mode: TwinAnswerMode::Simulation,
            twin_context_policy: Some("approved_plus_relevant_candidates".to_string()),
            twin_llm_provider: Some("ollama".to_string()),
            ..build_tile("tile-1", "Prompt", "openai/gpt-4", "Response", None, None)
        };

        let request = prompt_request_from_tile(&tile, vec!["openai/gpt-4".to_string()], 0.7);

        assert_eq!(request.context_mode, ContextMode::Twin);
        assert_eq!(request.twin_answer_mode, TwinAnswerMode::Simulation);
        assert_eq!(
            request.twin_context_policy.as_deref(),
            Some("approved_plus_relevant_candidates")
        );
        assert_eq!(request.twin_llm_provider.as_deref(), Some("ollama"));
    }

    #[test]
    fn test_classify_streamed_model_response_rejects_empty_content() {
        let update =
            classify_streamed_model_response("openai/gpt-4".to_string(), "   \n\t".to_string());

        assert_eq!(update.0, "openai/gpt-4");
        assert_eq!(update.1, "");
        assert_eq!(update.2, ResponseStatus::Error);
        assert_eq!(update.3.as_deref(), Some(EMPTY_MODEL_RESPONSE_ERROR));
    }

    #[test]
    fn test_classify_streamed_model_response_accepts_non_empty_content() {
        let update =
            classify_streamed_model_response("openai/gpt-4".to_string(), "Answer".to_string());

        assert_eq!(update.2, ResponseStatus::Completed);
        assert_eq!(update.3, None);
        assert_eq!(update.1, "Answer");
    }

    #[test]
    fn test_should_use_retrieved_notes_accepts_relevant_matches() {
        let results = vec![build_retrieval_result(
            "note-1",
            "Mirofish architecture ideas",
            "A note about robust social media posting architecture.",
            12.0,
            &["keyword match"],
        )];

        let decision = should_use_retrieved_notes(
            "How can I make the Mirofish social media architecture more robust?",
            &results,
        );

        assert_eq!(decision, RetrievalDecisionReason::UseRetrievedNotes);
    }

    #[test]
    fn test_should_use_retrieved_notes_rejects_off_topic_matches() {
        let results = vec![build_retrieval_result(
            "note-1",
            "Claude skills overview",
            "General AI skills and coding workflow tips.",
            72.0,
            &["keyword match", "hub (5 backlinks)"],
        )];

        let decision = should_use_retrieved_notes(
            "How can I make the Mirofish social media architecture more robust?",
            &results,
        );

        assert_eq!(decision, RetrievalDecisionReason::NoLexicalOverlap);
    }

    #[test]
    fn test_should_use_retrieved_notes_rejects_graph_only_results() {
        let results = vec![build_retrieval_result(
            "note-1",
            "Mirofish architecture",
            "A note about robust social media posting architecture.",
            20.0,
            &["graph neighbor (1 hop)", "hub (4 backlinks)"],
        )];

        let decision = should_use_retrieved_notes(
            "How can I make the Mirofish social media architecture more robust?",
            &results,
        );

        assert_eq!(decision, RetrievalDecisionReason::NoKeywordMatch);
    }
}
