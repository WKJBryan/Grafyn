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
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tauri::State;

const COMPACT_HISTORY_RECENT_TURNS: usize = 2;
const COMPACT_HISTORY_EXCERPT_CHARS: usize = 240;

// LLM node layout constants
const LLM_NODE_WIDTH: f64 = 280.0;
const LLM_NODE_HEIGHT: f64 = 200.0;
const LLM_NODE_Y_STEP: f64 = 300.0; // height(200) + 100px gap for content overflow
const LLM_NODE_X_GAP: f64 = 80.0;

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
    system_prompt: Option<String>,
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
    let session = {
        let mut store = state.canvas_store.write().await;
        store.get_session(&session_id).map_err(|e| e.to_string())?
    };
    let resolved_context = resolve_prompt_context(state.inner(), &session, &request).await?;

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
        web_search: request.web_search,
        web_search_max_results: request.web_search_max_results,
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
    let canvas_store_arc = state.canvas_store.clone();
    let models = request.models.clone();
    let messages = resolved_context.messages.clone();
    let system_prompt = resolved_context.system_prompt.clone();
    let temperature = request.temperature;
    let max_tokens = request.max_tokens;
    let web_search = request.web_search;
    let web_search_max_results = request.web_search_max_results;
    let tile_id_clone = tile_id.clone();
    let session_id_clone = session_id.clone();

    // Spawn async task for streaming (doesn't block the IPC response)
    tauri::async_runtime::spawn(async move {
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
                        max_tokens,
                        web_search,
                        web_search_max_results,
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
                        .chat_stream(&model_id, messages, None, Some(0.7), Some(1024), false, 5)
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
                    .chat_stream(&model_id, messages, None, Some(0.7), Some(1024), false, 5)
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
    let model_ids = request.model_ids.clone();
    let prompt_request = prompt_request_from_tile(&tile, model_ids.clone(), 0.7, None);
    let resolved_context = resolve_prompt_context(state.inner(), &session, &prompt_request).await?;

    let now = Utc::now();

    // Calculate positions for new models
    let existing_count = tile.responses.len();
    let prompt_pos = &tile.position;
    let llm_start_x = prompt_pos.x + prompt_pos.width + LLM_NODE_X_GAP;

    // Add initial pending responses
    {
        let mut store = state.canvas_store.write().await;
        let mut session = store.get_session(&session_id).map_err(|e| e.to_string())?;
        if let Some(t) = session.prompt_tiles.iter_mut().find(|t| t.id == tile_id) {
            for (i, model_id) in request.model_ids.iter().enumerate() {
                let model_name = model_id.split('/').last().unwrap_or(model_id).to_string();
                let llm_y = prompt_pos.y + ((existing_count + i) as f64) * LLM_NODE_Y_STEP;
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
                            width: LLM_NODE_WIDTH,
                            height: LLM_NODE_HEIGHT,
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
    let messages = resolved_context.messages.clone();
    let system_prompt = resolved_context.system_prompt.clone();
    let web_search = prompt_request.web_search;
    let web_search_max_results = prompt_request.web_search_max_results;

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

            join_set.spawn(async move {
                let openrouter = openrouter_arc.read().await;
                let stream_result = openrouter
                    .chat_stream(
                        &model_id,
                        messages,
                        system_prompt.as_deref(),
                        Some(0.7),
                        None,
                        web_search,
                        web_search_max_results,
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

    let request = prompt_request_from_tile(tile, vec![model_id.clone()], 0.7, None);
    let resolved_context = resolve_prompt_context(state.inner(), &session, &request).await?;

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
    let messages = resolved_context.messages.clone();
    let system_prompt = resolved_context.system_prompt.clone();
    let web_search = request.web_search;
    let web_search_max_results = request.web_search_max_results;

    tauri::async_runtime::spawn(async move {
        let openrouter = openrouter_arc.read().await;
        let stream_result = openrouter
            .chat_stream(
                &model_id,
                messages,
                system_prompt.as_deref(),
                Some(0.7),
                None,
                web_search,
                web_search_max_results,
            )
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

fn prompt_request_from_tile(
    tile: &PromptTile,
    models: Vec<String>,
    temperature: f64,
    max_tokens: Option<u32>,
) -> PromptRequest {
    PromptRequest {
        prompt: tile.prompt.clone(),
        system_prompt: tile.system_prompt.clone(),
        models,
        position: None,
        context_mode: tile.context_mode.clone(),
        parent_tile_id: tile.parent_tile_id.clone(),
        parent_model_id: tile.parent_model_id.clone(),
        temperature,
        max_tokens,
        web_search: tile.web_search,
        web_search_max_results: tile.web_search_max_results,
    }
}

async fn resolve_prompt_context(
    state: &AppState,
    session: &CanvasSession,
    request: &PromptRequest,
) -> Result<ResolvedPromptContext, String> {
    let messages = build_canvas_messages(session, request)?;

    if matches!(request.context_mode, ContextMode::KnowledgeSearch | ContextMode::Semantic) {
        let pinned_ids = session.pinned_note_ids.clone();
        let retrieval_results = {
            let search = state.search_service.read().await;
            let graph = state.graph_index.read().await;
            let priority = state.priority_service.read().await;
            let retrieval = state.retrieval_service.read().await;
            retrieval
                .retrieve(&search, &graph, &priority, &request.prompt, 5, &pinned_ids)
                .unwrap_or_default()
        };

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
        let system_prompt = match &request.system_prompt {
            Some(user_sp) if !user_sp.is_empty() => format!("{}\n\n{}", note_prompt, user_sp),
            _ => note_prompt,
        };

        Ok(ResolvedPromptContext {
            messages,
            context_notes,
            system_prompt: Some(system_prompt),
        })
    } else {
        Ok(ResolvedPromptContext {
            messages,
            context_notes: Vec::new(),
            system_prompt: request.system_prompt.clone(),
        })
    }
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
    let mut summary =
        String::from("Conversation summary before the most recent turns:\n");

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
            web_search: false,
            web_search_max_results: 5,
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

    fn build_request(prompt: &str, parent_tile_id: &str, parent_model_id: &str, context_mode: ContextMode) -> PromptRequest {
        PromptRequest {
            prompt: prompt.to_string(),
            system_prompt: None,
            models: vec!["openai/gpt-4".to_string()],
            position: None,
            context_mode,
            parent_tile_id: Some(parent_tile_id.to_string()),
            parent_model_id: Some(parent_model_id.to_string()),
            temperature: 0.7,
            max_tokens: None,
            web_search: false,
            web_search_max_results: 5,
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
    fn test_build_selected_parent_chain_returns_root_to_leaf_order() {
        let session = build_session(vec![
            build_tile("tile-1", "Root prompt", "openai/gpt-4", "Root response", None, None),
            build_tile("tile-2", "Follow-up prompt", "openai/gpt-4", "Follow-up response", Some("tile-1"), Some("openai/gpt-4")),
            build_tile("tile-3", "Deep prompt", "openai/gpt-4", "Deep response", Some("tile-2"), Some("openai/gpt-4")),
        ]);
        let request = build_request("Newest prompt", "tile-3", "openai/gpt-4", ContextMode::FullHistory);

        let turns = build_selected_parent_chain(&session, &request).unwrap();

        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].prompt, "Root prompt");
        assert_eq!(turns[1].prompt, "Follow-up prompt");
        assert_eq!(turns[2].prompt, "Deep prompt");
    }

    #[test]
    fn test_build_full_history_messages_interleaves_user_and_assistant_turns() {
        let session = build_session(vec![
            build_tile("tile-1", "Root prompt", "openai/gpt-4", "Root response", None, None),
            build_tile("tile-2", "Branch prompt", "openai/gpt-4", "Branch response", Some("tile-1"), Some("openai/gpt-4")),
        ]);
        let request = build_request("Final prompt", "tile-2", "openai/gpt-4", ContextMode::FullHistory);

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
    fn test_build_compact_history_messages_summarizes_older_turns() {
        let session = build_session(vec![
            build_tile("tile-1", "Prompt 1", "openai/gpt-4", "Response 1", None, None),
            build_tile("tile-2", "Prompt 2", "openai/gpt-4", "Response 2", Some("tile-1"), Some("openai/gpt-4")),
            build_tile("tile-3", "Prompt 3", "openai/gpt-4", "Response 3", Some("tile-2"), Some("openai/gpt-4")),
            build_tile("tile-4", "Prompt 4", "openai/gpt-4", "Response 4", Some("tile-3"), Some("openai/gpt-4")),
        ]);
        let request = build_request("Prompt 5", "tile-4", "openai/gpt-4", ContextMode::Compact);

        let messages = build_compact_history_messages(&session, &request).unwrap();

        assert_eq!(messages.len(), 6);
        assert!(messages[0].content.contains("Conversation summary before the most recent turns"));
        assert!(messages[0].content.contains("Prompt 1"));
        assert!(messages[1].content.contains("Prompt 3"));
        assert!(messages[2].content.contains("Response 3"));
        assert_eq!(messages[5].content, "Prompt 5");
    }

    #[test]
    fn test_build_selected_parent_chain_errors_when_parent_response_is_missing() {
        let session = build_session(vec![
            build_tile("tile-1", "Root prompt", "openai/gpt-4", "Root response", None, None),
            build_tile("tile-2", "Branch prompt", "anthropic/claude", "Branch response", Some("tile-1"), Some("openai/gpt-4")),
        ]);
        let request = build_request("Next prompt", "tile-2", "openai/gpt-4", ContextMode::FullHistory);

        let err = build_selected_parent_chain(&session, &request).unwrap_err();

        assert!(err.contains("Parent response not found"));
    }

    #[test]
    fn test_prompt_request_from_tile_preserves_web_search_settings() {
        let tile = PromptTile {
            web_search: true,
            web_search_max_results: 8,
            ..build_tile("tile-1", "Prompt", "openai/gpt-4", "Response", None, None)
        };

        let request = prompt_request_from_tile(&tile, vec!["openai/gpt-4".to_string()], 0.3, Some(4096));

        assert!(request.web_search);
        assert_eq!(request.web_search_max_results, 8);
        assert_eq!(request.temperature, 0.3);
        assert_eq!(request.max_tokens, Some(4096));
    }
}
