use crate::models::canvas::{
    AddModelsRequest, AvailableModel, CanvasSession, CanvasStreamEvent, ContextMode,
    CanvasViewport, Debate, DebateContinueRequest, DebateResponse, DebateRound,
    DebateStartRequest, LLMNodePositionUpdate, ModelResponse, PromptRequest, PromptTile,
    ResponseStatus, SessionCreate, SessionMeta, SessionUpdate, TileContextNote, TilePosition,
    TilePositionUpdate,
};
use crate::models::note::{NoteCreate, NoteStatus};
use crate::services::openrouter::ChatMessage;
use crate::AppState;
use chrono::Utc;
use futures::StreamExt;
use std::collections::HashMap;
use std::time::Duration;
use tauri::State;

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
    store.create_session(session).map_err(|e| e.to_string())
}

/// Update a canvas session
#[tauri::command]
pub async fn update_session(
    id: String,
    update: SessionUpdate,
    state: State<'_, AppState>,
) -> Result<CanvasSession, String> {
    let mut store = state.canvas_store.write().await;
    store.update_session(&id, update).map_err(|e| e.to_string())
}

/// Delete a canvas session
#[tauri::command]
pub async fn delete_session(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store.delete_session(&id).map_err(|e| e.to_string())
}

/// Get list of available LLM models
#[tauri::command]
pub async fn get_available_models(state: State<'_, AppState>) -> Result<Vec<AvailableModel>, String> {
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
    request: PromptRequest,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let tile_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();

    // --- Knowledge search context: retrieve relevant notes ---
    let use_knowledge_search = matches!(request.context_mode, ContextMode::KnowledgeSearch | ContextMode::Semantic);
    let (context_notes, computed_system_prompt) = if use_knowledge_search {
        // Get pinned note IDs from the session
        let pinned_ids = {
            let mut store = state.canvas_store.write().await;
            store
                .get_session(&session_id)
                .map(|s| s.pinned_note_ids.clone())
                .unwrap_or_default()
        };

        // Retrieve relevant notes
        let retrieval_results = {
            let search = state.search_service.read().await;
            let graph = state.graph_index.read().await;
            let priority = state.priority_service.read().await;
            let retrieval = state.retrieval_service.read().await;
            retrieval
                .retrieve(&search, &graph, &priority, &request.prompt, 5, &pinned_ids)
                .unwrap_or_default()
        };

        // Fetch full note content and build context
        let note_contexts: Vec<(String, String, String)> = {
            let store = state.knowledge_store.read().await;
            retrieval_results
                .iter()
                .filter_map(|r| {
                    store.get_note(&r.note.id).ok().map(|note| {
                        let truncated = if note.content.len() > 1500 {
                            format!("{}...", &note.content[..1500])
                        } else {
                            note.content.clone()
                        };
                        (note.id.clone(), note.title.clone(), truncated)
                    })
                })
                .collect()
        };

        // Build tile context notes for the frontend
        let tile_context: Vec<TileContextNote> = retrieval_results
            .iter()
            .map(|r| TileContextNote {
                id: r.note.id.clone(),
                title: r.note.title.clone(),
                snippet: r.snippet.clone(),
                score: r.score,
                pinned: pinned_ids.contains(&r.note.id),
            })
            .collect();

        // Build note context prompt, merge with user's system_prompt if provided
        let note_prompt = build_note_context_prompt(&note_contexts);
        let merged = match &request.system_prompt {
            Some(user_sp) if !user_sp.is_empty() => {
                format!("{}\n\n{}", note_prompt, user_sp)
            }
            _ => note_prompt,
        };

        (tile_context, Some(merged))
    } else {
        (Vec::new(), request.system_prompt.clone())
    };

    // Compute LLM node positions (offset from prompt tile)
    let prompt_pos = request.position.clone().unwrap_or_default();
    let llm_start_x = prompt_pos.x + prompt_pos.width + 80.0;

    // Create initial responses map with positions
    let mut responses: HashMap<String, ModelResponse> = HashMap::new();
    for (i, model_id) in request.models.iter().enumerate() {
        let model_name = model_id.split('/').last().unwrap_or(model_id).to_string();
        let llm_y = prompt_pos.y + (i as f64) * 280.0;
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
                    width: 280.0,
                    height: 200.0,
                },
            },
        );
    }

    // Create the tile (with context notes attached)
    let tile = PromptTile {
        id: tile_id.clone(),
        prompt: request.prompt.clone(),
        system_prompt: request.system_prompt.clone(),
        models: request.models.clone(),
        responses: responses.clone(),
        position: prompt_pos,
        created_at: now,
        context_mode: request.context_mode,
        parent_tile_id: request.parent_tile_id,
        parent_model_id: request.parent_model_id,
        context_notes: context_notes.clone(),
    };

    // Save tile to session
    {
        let mut store = state.canvas_store.write().await;
        store.add_tile(&session_id, tile.clone()).map_err(|e| e.to_string())?;
    }

    // Emit TileCreated event
    let _ = window.emit(
        "canvas-stream",
        CanvasStreamEvent::TileCreated {
            session_id: session_id.clone(),
            tile: tile.clone(),
        },
    );

    // Emit ContextNotes event (so frontend can display which notes were used)
    if !context_notes.is_empty() {
        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::ContextNotes {
                session_id: session_id.clone(),
                tile_id: tile_id.clone(),
                notes: context_notes,
            },
        );
    }

    // Clone what we need for the spawned task
    let openrouter_arc = state.openrouter.clone();
    let canvas_store_arc = state.canvas_store.clone();
    let models = request.models.clone();
    let prompt = request.prompt.clone();
    let system_prompt = computed_system_prompt;
    let temperature = request.temperature;
    let max_tokens = request.max_tokens;
    let tile_id_clone = tile_id.clone();
    let session_id_clone = session_id.clone();

    // Spawn async task for streaming (doesn't block the IPC response)
    tauri::async_runtime::spawn(async move {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }];

        // Stream all models concurrently using JoinSet
        let mut join_set = tokio::task::JoinSet::new();

        for model_id in models {
            let messages = messages.clone();
            let system_prompt = system_prompt.clone();
            let openrouter_arc = openrouter_arc.clone();
            let window = window.clone();
            let session_id = session_id_clone.clone();
            let tile_id = tile_id_clone.clone();

            join_set.spawn(async move {
                let openrouter = openrouter_arc.read().await;
                let stream_result = openrouter
                    .chat_stream(
                        &model_id,
                        messages,
                        system_prompt.as_deref(),
                        Some(temperature),
                        Some(max_tokens),
                    )
                    .await;
                drop(openrouter);

                match stream_result {
                    Ok(stream) => {
                        let mut stream = Box::pin(stream);
                        let mut full_content = String::new();

                        loop {
                            match tokio::time::timeout(
                                Duration::from_secs(60),
                                stream.next(),
                            )
                            .await
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
                                    let _ = window.emit(
                                        "canvas-stream",
                                        CanvasStreamEvent::Error {
                                            session_id: session_id.clone(),
                                            tile_id: tile_id.clone(),
                                            model_id: model_id.clone(),
                                            error: e.to_string(),
                                        },
                                    );
                                    return (model_id, e.to_string(), ResponseStatus::Error);
                                }
                                Ok(None) => break, // Stream ended naturally
                                Err(_) => {
                                    let _ = window.emit(
                                        "canvas-stream",
                                        CanvasStreamEvent::Error {
                                            session_id: session_id.clone(),
                                            tile_id: tile_id.clone(),
                                            model_id: model_id.clone(),
                                            error: "Stream idle timeout (60s)".to_string(),
                                        },
                                    );
                                    return (model_id, full_content, ResponseStatus::Error);
                                }
                            }
                        }

                        // Emit per-model completion immediately
                        let _ = window.emit(
                            "canvas-stream",
                            CanvasStreamEvent::Complete {
                                session_id: session_id.clone(),
                                tile_id: tile_id.clone(),
                                model_id: model_id.clone(),
                                tokens_used: None,
                            },
                        );

                        (model_id, full_content, ResponseStatus::Completed)
                    }
                    Err(e) => {
                        let _ = window.emit(
                            "canvas-stream",
                            CanvasStreamEvent::Error {
                                session_id: session_id.clone(),
                                tile_id: tile_id.clone(),
                                model_id: model_id.clone(),
                                error: e.to_string(),
                            },
                        );
                        (model_id, e.to_string(), ResponseStatus::Error)
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
        {
            let mut store = canvas_store_arc.write().await;
            let _ = store.batch_update_tile_responses(
                &session_id_clone,
                &tile_id_clone,
                &results,
            );
        }

        // Emit session saved after all models complete
        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::SessionSaved {
                session_id: session_id_clone,
            },
        );
    });

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
    store.delete_tile(&session_id, &tile_id).map_err(|e| e.to_string())
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
        .map_err(|e| e.to_string())
}

/// Update viewport zoom/pan state
#[tauri::command]
pub async fn update_viewport(
    session_id: String,
    viewport: CanvasViewport,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut store = state.canvas_store.write().await;
    store.update_viewport(&session_id, viewport).map_err(|e| e.to_string())
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
            content.push_str(&format!("### Round {} - {}\n\n", round.round_number, round.topic));
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

    let note = ks.create_note(NoteCreate {
        title: session.title.clone(),
        content,
        status: NoteStatus::Evidence,
        tags: session.tags.clone(),
        properties: HashMap::new(),
    }).map_err(|e| e.to_string())?;

    drop(ks);

    // Update search index (mirrors create_note in notes.rs)
    {
        let mut search = state.search_service.write().await;
        if let Err(e) = search.index_note(&note) {
            log::error!("Failed to index exported note '{}': {}", note.id, e);
        }
        if let Err(e) = search.commit() {
            log::error!("Failed to commit search index after export '{}': {}", note.id, e);
        }
    }

    // Update graph index so backlinks/outgoing links are discoverable
    {
        let mut graph = state.graph_index.write().await;
        graph.update_note(&note);
    }

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
    request: DebateStartRequest,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let debate_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();

    // Collect source content from tiles
    let mut store = state.canvas_store.write().await;
    let session = store.get_session(&session_id).map_err(|e| e.to_string())?;
    drop(store);

    let mut source_content = String::new();
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
    let max_x = session.prompt_tiles.iter()
        .flat_map(|t| t.responses.values().map(|r| r.position.x + r.position.width))
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
        created_at: now,
    };

    // Save debate to session
    {
        let mut store = state.canvas_store.write().await;
        store.add_debate(&session_id, debate.clone()).map_err(|e| e.to_string())?;
    }

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
    let canvas_store_arc = state.canvas_store.clone();
    let models = request.participating_models.clone();
    let max_rounds = request.max_rounds;
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
                let window = window.clone();
                let session_id = session_id_clone.clone();
                let debate_id = debate_id_clone.clone();

                join_set.spawn(async move {
                    let openrouter = openrouter_arc.read().await;
                    let stream_result = openrouter
                        .chat_stream(&model_id, messages, None, Some(0.7), Some(1024))
                        .await;
                    drop(openrouter);

                    match stream_result {
                        Ok(stream) => {
                            let mut stream = Box::pin(stream);
                            let mut full_content = String::new();

                            loop {
                                match tokio::time::timeout(
                                    Duration::from_secs(60),
                                    stream.next(),
                                )
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

            // Persist after each round
            {
                let mut store = canvas_store_arc.write().await;
                let _ = store.update_debate(&session_id_clone, &debate_state);
            }
        }

        // Mark debate as complete
        debate_state.status = "completed".to_string();
        {
            let mut store = canvas_store_arc.write().await;
            let _ = store.update_debate(&session_id_clone, &debate_state);
        }

        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::DebateComplete {
                session_id: session_id_clone,
                debate_id: debate_id_clone,
            },
        );
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

    let openrouter_arc = state.openrouter.clone();
    let canvas_store_arc = state.canvas_store.clone();
    let models = debate.participating_models.clone();

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
        context.push_str(&format!("New prompt: {}\n\nRespond to this new direction.", request.prompt));

        // Stream all models concurrently using JoinSet
        let mut join_set = tokio::task::JoinSet::new();

        for model_id in models {
            let model_name = model_id.split('/').last().unwrap_or(&model_id).to_string();
            let messages = vec![ChatMessage {
                role: "user".to_string(),
                content: context.clone(),
            }];
            let openrouter_arc = openrouter_arc.clone();
            let window = window.clone();
            let session_id = session_id.clone();
            let debate_id = debate_id.clone();

            join_set.spawn(async move {
                let openrouter = openrouter_arc.read().await;
                let stream_result = openrouter
                    .chat_stream(&model_id, messages, None, Some(0.7), Some(1024))
                    .await;
                drop(openrouter);

                match stream_result {
                    Ok(stream) => {
                        let mut stream = Box::pin(stream);
                        let mut full_content = String::new();

                        loop {
                            match tokio::time::timeout(
                                Duration::from_secs(60),
                                stream.next(),
                            )
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
            topic: request.prompt,
            responses: round_responses,
            created_at: Utc::now(),
        };
        debate_state.rounds.push(round);

        {
            let mut store = canvas_store_arc.write().await;
            let _ = store.update_debate(&session_id, &debate_state);
        }

        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::DebateComplete {
                session_id,
                debate_id,
            },
        );
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

    let now = Utc::now();

    // Calculate positions for new models
    let existing_count = tile.responses.len();
    let prompt_pos = &tile.position;
    let llm_start_x = prompt_pos.x + prompt_pos.width + 80.0;

    // Add initial pending responses
    {
        let mut store = state.canvas_store.write().await;
        let mut session = store.get_session(&session_id).map_err(|e| e.to_string())?;
        if let Some(t) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
            for (i, model_id) in request.model_ids.iter().enumerate() {
                let model_name = model_id.split('/').last().unwrap_or(model_id).to_string();
                let llm_y = prompt_pos.y + ((existing_count + i) as f64) * 280.0;
                t.models.push(model_id.clone());
                t.responses.insert(
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
                            width: 280.0,
                            height: 200.0,
                        },
                    },
                );
            }
            session.updated_at = Utc::now();
            store.save_session(&session).map_err(|e| e.to_string())?;
        }
    }

    // Spawn streaming task
    let openrouter_arc = state.openrouter.clone();
    let canvas_store_arc = state.canvas_store.clone();
    let model_ids = request.model_ids;
    let prompt = tile.prompt.clone();
    let system_prompt = tile.system_prompt.clone();

    tauri::async_runtime::spawn(async move {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }];

        // Stream all new models concurrently using JoinSet
        let mut join_set = tokio::task::JoinSet::new();

        for model_id in model_ids {
            let messages = messages.clone();
            let system_prompt = system_prompt.clone();
            let openrouter_arc = openrouter_arc.clone();
            let window = window.clone();
            let session_id = session_id.clone();
            let tile_id = tile_id.clone();

            join_set.spawn(async move {
                let openrouter = openrouter_arc.read().await;
                let stream_result = openrouter
                    .chat_stream(&model_id, messages, system_prompt.as_deref(), Some(0.7), Some(2048))
                    .await;
                drop(openrouter);

                match stream_result {
                    Ok(stream) => {
                        let mut stream = Box::pin(stream);
                        let mut full_content = String::new();

                        loop {
                            match tokio::time::timeout(
                                Duration::from_secs(60),
                                stream.next(),
                            )
                            .await
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
                                    let _ = window.emit(
                                        "canvas-stream",
                                        CanvasStreamEvent::Error {
                                            session_id: session_id.clone(),
                                            tile_id: tile_id.clone(),
                                            model_id: model_id.clone(),
                                            error: e.to_string(),
                                        },
                                    );
                                    return (model_id, e.to_string(), ResponseStatus::Error);
                                }
                                Ok(None) => break,
                                Err(_) => {
                                    let _ = window.emit(
                                        "canvas-stream",
                                        CanvasStreamEvent::Error {
                                            session_id: session_id.clone(),
                                            tile_id: tile_id.clone(),
                                            model_id: model_id.clone(),
                                            error: "Stream idle timeout (60s)".to_string(),
                                        },
                                    );
                                    return (model_id, full_content, ResponseStatus::Error);
                                }
                            }
                        }

                        let _ = window.emit(
                            "canvas-stream",
                            CanvasStreamEvent::Complete {
                                session_id: session_id.clone(),
                                tile_id: tile_id.clone(),
                                model_id: model_id.clone(),
                                tokens_used: None,
                            },
                        );

                        (model_id, full_content, ResponseStatus::Completed)
                    }
                    Err(e) => {
                        let _ = window.emit(
                            "canvas-stream",
                            CanvasStreamEvent::Error {
                                session_id: session_id.clone(),
                                tile_id: tile_id.clone(),
                                model_id: model_id.clone(),
                                error: e.to_string(),
                            },
                        );
                        (model_id, e.to_string(), ResponseStatus::Error)
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
        {
            let mut store = canvas_store_arc.write().await;
            let _ = store.batch_update_tile_responses(
                &session_id,
                &tile_id,
                &results,
            );
        }

        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::SessionSaved {
                session_id,
            },
        );
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

    if !tile.responses.contains_key(&model_id) {
        return Err("Response not found".to_string());
    }

    let prompt = tile.prompt.clone();
    let system_prompt = tile.system_prompt.clone();

    // Reset response to streaming
    {
        let mut store = state.canvas_store.write().await;
        let _ = store.update_tile_response(
            &session_id, &tile_id, &model_id, "",
            ResponseStatus::Streaming,
        );
    }

    let openrouter_arc = state.openrouter.clone();
    let canvas_store_arc = state.canvas_store.clone();

    tauri::async_runtime::spawn(async move {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }];

        let openrouter = openrouter_arc.read().await;
        let stream_result = openrouter
            .chat_stream(&model_id, messages, system_prompt.as_deref(), Some(0.7), Some(2048))
            .await;
        drop(openrouter);

        match stream_result {
            Ok(stream) => {
                let mut stream = Box::pin(stream);
                let mut full_content = String::new();

                while let Some(chunk_result) = stream.next().await {
                    match chunk_result {
                        Ok(chunk) => {
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
                        Err(e) => {
                            let _ = window.emit(
                                "canvas-stream",
                                CanvasStreamEvent::Error {
                                    session_id: session_id.clone(),
                                    tile_id: tile_id.clone(),
                                    model_id: model_id.clone(),
                                    error: e.to_string(),
                                },
                            );
                            break;
                        }
                    }
                }

                {
                    let mut store = canvas_store_arc.write().await;
                    let _ = store.update_tile_response(
                        &session_id, &tile_id, &model_id, &full_content,
                        ResponseStatus::Completed,
                    );
                }

                let _ = window.emit(
                    "canvas-stream",
                    CanvasStreamEvent::Complete {
                        session_id: session_id.clone(),
                        tile_id: tile_id.clone(),
                        model_id: model_id.clone(),
                        tokens_used: None,
                    },
                );
            }
            Err(e) => {
                {
                    let mut store = canvas_store_arc.write().await;
                    let _ = store.update_tile_response(
                        &session_id, &tile_id, &model_id, &e.to_string(),
                        ResponseStatus::Error,
                    );
                }
                let _ = window.emit(
                    "canvas-stream",
                    CanvasStreamEvent::Error {
                        session_id: session_id.clone(),
                        tile_id: tile_id.clone(),
                        model_id: model_id.clone(),
                        error: e.to_string(),
                    },
                );
            }
        }

        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::SessionSaved {
                session_id,
            },
        );
    });

    Ok(())
}

/// Build a system prompt that includes retrieved note context.
/// Used by send_prompt when context_mode is Semantic.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
