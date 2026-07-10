use super::shared::{
    append_canvas_trace, effective_model_ids, resolve_model_route, source_tile_context_provider,
    ModelProviderRoute,
};
use crate::models::canvas::{
    CanvasStreamEvent, ContextMode, Debate, DebateContinueRequest, DebateResponse, DebateRound,
    DebateStartRequest, PromptType, TilePosition,
};
use crate::models::twin::TraceEventType;
use crate::services::openrouter::ChatMessage;
use crate::AppState;
use chrono::Utc;
use futures::StreamExt;
use serde_json::json;
use std::time::Duration;
use tauri::State;

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
