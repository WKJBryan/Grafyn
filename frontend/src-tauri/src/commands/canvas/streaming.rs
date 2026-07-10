use super::context::{resolve_prompt_context, run_sealed_twin_prediction, TWIN_CONTEXT_VERSION};
use super::shared::{
    append_canvas_trace, effective_model_ids, resolve_model_route, ModelProviderRoute,
};
use crate::models::canvas::{
    AddModelsRequest, CanvasStreamEvent, ModelResponse, PromptRequest, PromptTile, PromptType,
    ResponseStatus, TilePosition,
};
use crate::models::twin::{
    DecisionEpisodeCreate, PrimitiveDecisionAssessment, ReflectionCardCreate, TraceEventType,
};
use crate::services::twin_store::TwinStore;
use crate::AppState;
use chrono::Utc;
use futures::StreamExt;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::State;
use tokio::sync::RwLock;

const EMPTY_MODEL_RESPONSE_ERROR: &str = "No response returned from model";

// LLM node layout constants
const LLM_NODE_WIDTH: f64 = 280.0;
const LLM_NODE_HEIGHT: f64 = 200.0;
const LLM_NODE_Y_STEP: f64 = 300.0; // height(200) + 100px gap for content overflow
const LLM_NODE_X_GAP: f64 = 80.0;

type StreamedResponseUpdate = (String, String, ResponseStatus, Option<String>);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::canvas::test_support::build_tile;
    use crate::models::canvas::{ContextMode, TwinAnswerMode};

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
}
