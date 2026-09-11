use super::shared::{
    append_canvas_trace_expecting_authority, effective_model_ids, preserve_canvas_mutation_error,
    repair_canvas_trace_error, resolve_model_route, source_tile_context_provider,
    ModelProviderRoute,
};
use crate::models::canvas::{
    CanvasStreamEvent, ContextMode, Debate, DebateContinueRequest, DebateResponse, DebateRound,
    DebateStartRequest, PromptType, TilePosition,
};
use crate::models::twin::TraceEventType;
use crate::services::openrouter::{ChatMessage, StreamUpdate};
use crate::AppState;
use chrono::Utc;
use futures::StreamExt;
use serde_json::json;
use std::time::Duration;
use tauri::{Emitter, State};

fn format_previous_rounds(rounds: &[DebateRound]) -> String {
    let mut body = String::new();
    for round in rounds {
        body.push_str(&format!("\nAfter round {}:\n", round.round_number));
        for resp in &round.responses {
            body.push_str(&format!("\n{} said:\n{}\n", resp.model_name, resp.content));
        }
    }
    body
}

pub(super) fn build_debate_round_user_message(
    round_num: u32,
    source_content: &str,
    previous_rounds: &[DebateRound],
    steer: Option<&str>,
) -> String {
    let mut message = String::from(
        "You are in a room with other models. Talk like a person. \
Do not use headings, numbered sections, or labels like Understand, Think, or Position.\n\n",
    );
    message.push_str("Here is what started this:\n");
    message.push_str(source_content);
    message.push('\n');

    if previous_rounds.is_empty() {
        message.push_str(
            "\nThis is the first round. You have everyone's original answer, including your own. \
Hear them. Then say what you think. Take a stand if you have one.\n",
        );
    } else {
        message.push_str("\nYou've now heard the others talk:\n");
        message.push_str(&format_previous_rounds(previous_rounds));
        message.push_str(
            "\nThis is a later round. Answer them — agree, push back, or change your mind. \
Do not restart from the original prompt as if you hadn't heard anyone.\n",
        );
    }

    if let Some(steer) = steer.map(str::trim).filter(|value| !value.is_empty()) {
        message.push_str("\nThe human steering this round:\n");
        message.push_str(steer);
        message.push('\n');
    }

    let _ = round_num;
    message
}

pub(super) fn build_chair_synthesis_prompt(source_content: &str, rounds: &[DebateRound]) -> String {
    let mut message = String::from(
        "You were listening, not in the fight. Talk like a person who heard the room. \
Do not use headings or bins like Agreed, Split, or Moved. Do not declare a winner. \
Do not invent a position nobody took.\n\n",
    );
    message.push_str("What started this:\n");
    message.push_str(source_content);
    message.push_str(&format_previous_rounds(rounds));
    message.push_str("\nWhere did the room land?\n");
    message
}

fn no_cost_stream<S>(
    stream: S,
) -> std::pin::Pin<Box<dyn futures::Stream<Item = anyhow::Result<StreamUpdate>> + Send>>
where
    S: futures::Stream<Item = anyhow::Result<String>> + Send + 'static,
{
    Box::pin(stream.map(|result| {
        result.map(|content| StreamUpdate {
            content,
            cost_usd: None,
        })
    }))
}

/// Start a debate between models with streaming via Tauri events
#[tauri::command]
pub async fn start_debate(
    window: tauri::WebviewWindow,
    session_id: String,
    mut request: DebateStartRequest,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let input_root_epoch = root_ticket.authority().clone();
    let debate_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();

    // Collect source content from tiles
    let mut store = state.canvas_store.write().await;
    store.reload_authoritative_state();
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
    root_ticket.validate(state.inner()).await?;

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
        recap: None,
        recap_updated_at: None,
        created_at: now,
    };

    // Save debate to session
    {
        let mut store = state.canvas_store.write().await;
        let commit = store
            .add_debate_expecting_authority(&session_id, debate.clone(), input_root_epoch.clone())
            .map_err(|e| e.to_string())?;
        crate::services::canvas_store::require_canvas_only_commit(&commit)
            .map_err(|error| error.to_string())?;
    }

    let trace_commit = match append_canvas_trace_expecting_authority(
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
        input_root_epoch,
    )
    .await
    {
        Ok(commit) => commit,
        Err(error) => {
            drop(root_ticket);
            crate::commands::acknowledge_reported_repair(
                repair_canvas_trace_error(state.inner(), &error, "Canvas debate start").await,
            );
            log::error!("Canvas debate was saved, but its audit trace failed: {error}");
            let _ = window.emit(
                "canvas-stream",
                CanvasStreamEvent::DebateCreated {
                    session_id: session_id.clone(),
                    debate: debate.clone(),
                },
            );
            let _ = window.emit(
                "canvas-stream",
                CanvasStreamEvent::DebateError {
                    session_id,
                    debate_id: debate.id,
                    model_id: "system".to_string(),
                    error: format!(
                        "Canvas debate was saved, but its required trace failed: {error}"
                    ),
                    round_number: 0,
                },
            );
            return Ok(debate_id);
        }
    };
    let root_epoch = match crate::commands::repair_after_authority_mutation(
        state.inner(),
        &trace_commit,
        "Canvas debate start",
    )
    .await
    {
        crate::commands::PostAuthorityRepair::Ready(epoch) => epoch,
        crate::commands::PostAuthorityRepair::Unavailable(_) => {
            drop(root_ticket);
            let _ = window.emit(
                "canvas-stream",
                CanvasStreamEvent::DebateCreated { session_id, debate },
            );
            return Ok(debate_id);
        }
        crate::commands::PostAuthorityRepair::NotRequired => {
            return Err("Canvas debate trace did not mutate content authority".into());
        }
    };
    drop(root_ticket);

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
    let twin_store_arc = state.twin_store.clone();
    let settings_arc = state.settings_service.clone();
    let models = request.participating_models.clone();
    let max_rounds = request.max_rounds;
    let reasoning_effort = request.reasoning_effort.clone();
    let provider_route = model_route.provider.clone();
    let debate_id_clone = debate_id.clone();
    let session_id_clone = session_id.clone();
    let stream_root_state = state.inner().clone();

    tauri::async_runtime::spawn(async move {
        let mut debate_state = debate;
        let mut root_epoch = root_epoch;

        for round_num in 1..=max_rounds {
            let _ = window.emit(
                "canvas-stream",
                CanvasStreamEvent::RoundStart {
                    session_id: session_id_clone.clone(),
                    debate_id: debate_id_clone.clone(),
                    round_number: round_num,
                },
            );

            let context = build_debate_round_user_message(
                round_num,
                &source_content,
                &debate_state.rounds,
                None,
            );

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
                    let response_provider = provider_route.provider_label().to_string();
                    let response_provenance = provider_route.provenance_label(true).to_string();
                    let stream_result = match provider_route {
                        ModelProviderRoute::Ollama => {
                            let ollama_handle = ollama_arc
                                .as_ref()
                                .expect("Ollama debate route was validated before spawning");
                            let ollama = ollama_handle.read().await;
                            let result = ollama
                                .chat_stream(&model_id, messages, None, Some(0.7))
                                .await;
                            drop(ollama);
                            result.map(no_cost_stream)
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
                                    Some(&format!("canvas:{session_id}:{model_id}")),
                                )
                                .await;
                            drop(openrouter);
                            result.map(|stream| {
                                Box::pin(stream)
                                    as std::pin::Pin<
                                        Box<
                                            dyn futures::Stream<Item = anyhow::Result<StreamUpdate>>
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
                            let mut cost_usd = None;

                            loop {
                                match tokio::time::timeout(Duration::from_secs(60), stream.next())
                                    .await
                                {
                                    Ok(Some(Ok(update))) => {
                                        if update.cost_usd.is_some() {
                                            cost_usd = update.cost_usd;
                                        }
                                        let chunk = update.content;
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
                                            cost_usd: None,
                                            provider: Some(response_provider.clone()),
                                            provenance: Some(response_provenance.clone()),
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
                                            cost_usd: None,
                                            provider: Some(response_provider.clone()),
                                            provenance: Some(response_provenance.clone()),
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
                                    cost_usd,
                                },
                            );

                            DebateResponse {
                                model_id,
                                model_name,
                                content: full_content,
                                stance: None,
                                cost_usd,
                                provider: Some(response_provider.clone()),
                                provenance: Some(response_provenance.clone()),
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
                                    cost_usd: None,
                                },
                            );
                            DebateResponse {
                                model_id,
                                model_name,
                                content: e.to_string(),
                                stance: None,
                                cost_usd: None,
                                provider: Some(response_provider),
                                provenance: Some(response_provenance),
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

            let round_root_guard =
                match crate::commands::acquire_expected_root_epoch(&stream_root_state, &root_epoch)
                    .await
                {
                    Ok(guard) => guard,
                    Err(error) => {
                        emit_debate_persist_error(
                            &window,
                            &session_id_clone,
                            &debate_id_clone,
                            round_num,
                            &debate_state,
                            &anyhow::anyhow!(error),
                        );
                        return;
                    }
                };

            // Persist after each round. If this fails, surface it instead of
            // silently continuing to stream rounds that will never survive a
            // session reopen.
            let round_commit = {
                let mut store = canvas_store_arc.write().await;
                match store.update_debate_expecting_authority(
                    &session_id_clone,
                    &debate_state,
                    root_epoch.clone(),
                ) {
                    Ok(commit) => commit,
                    Err(error) => {
                        emit_debate_persist_error(
                            &window,
                            &session_id_clone,
                            &debate_id_clone,
                            round_num,
                            &debate_state,
                            &error,
                        );
                        return;
                    }
                }
            };
            root_epoch = round_commit
                .authority_token
                .unwrap_or_else(|| root_epoch.clone());
            drop(round_root_guard);
        }

        // Mark debate as complete
        debate_state.status = "completed".to_string();
        let final_round_number = debate_state
            .rounds
            .last()
            .map(|round| round.round_number)
            .unwrap_or(max_rounds);
        let root_guard =
            match crate::commands::acquire_expected_root_epoch(&stream_root_state, &root_epoch)
                .await
            {
                Ok(guard) => guard,
                Err(error) => {
                    emit_debate_persist_error(
                        &window,
                        &session_id_clone,
                        &debate_id_clone,
                        final_round_number,
                        &debate_state,
                        &anyhow::anyhow!(error),
                    );
                    return;
                }
            };
        let completion_commit = {
            let mut store = canvas_store_arc.write().await;
            match store.update_debate_expecting_authority(
                &session_id_clone,
                &debate_state,
                root_epoch.clone(),
            ) {
                Ok(commit) => commit,
                Err(error) => {
                    emit_debate_persist_error(
                        &window,
                        &session_id_clone,
                        &debate_id_clone,
                        final_round_number,
                        &debate_state,
                        &error,
                    );
                    return;
                }
            }
        };
        root_epoch = completion_commit
            .authority_token
            .unwrap_or_else(|| root_epoch.clone());
        drop(root_guard);
        if !matches!(
            crate::commands::repair_after_authority_token(
                &stream_root_state,
                &root_epoch,
                "Canvas debate",
            )
            .await,
            crate::commands::PostAuthorityRepair::Ready(_)
        ) {
            return;
        }
        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::DebateComplete {
                session_id: session_id_clone.clone(),
                debate_id: debate_id_clone.clone(),
            },
        );
        spawn_debate_chair_synthesis(
            window,
            session_id_clone,
            debate_id_clone,
            source_content,
            canvas_store_arc,
            twin_store_arc,
            openrouter_arc,
            ollama_arc,
            settings_arc,
        );
    });

    Ok(debate_id)
}

/// Continue a debate with a new round
#[tauri::command]
pub async fn continue_debate(
    window: tauri::WebviewWindow,
    session_id: String,
    debate_id: String,
    request: DebateContinueRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let input_root_epoch = root_ticket.authority().clone();
    let mut store = state.canvas_store.write().await;
    store.reload_authoritative_state();
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
    root_ticket.validate(state.inner()).await?;

    let trace_commit = match append_canvas_trace_expecting_authority(
        state.twin_store.clone(),
        &session_id,
        TraceEventType::DebateContinued,
        json!({
            "debate_id": debate_id.clone(),
            "prompt": request.prompt.clone(),
            "participating_models": debate.participating_models.clone(),
        }),
        input_root_epoch,
    )
    .await
    {
        Ok(commit) => commit,
        Err(error) => {
            drop(root_ticket);
            crate::commands::acknowledge_reported_repair(
                repair_canvas_trace_error(state.inner(), &error, "Canvas debate continue").await,
            );
            return Err(error.to_string());
        }
    };
    let root_epoch = match crate::commands::repair_after_authority_mutation(
        state.inner(),
        &trace_commit,
        "Canvas debate continue",
    )
    .await
    {
        crate::commands::PostAuthorityRepair::Ready(epoch) => epoch,
        crate::commands::PostAuthorityRepair::Unavailable(_) => return Ok(()),
        crate::commands::PostAuthorityRepair::NotRequired => {
            return Err("Canvas debate trace did not mutate content authority".into());
        }
    };
    drop(root_ticket);

    let openrouter_arc = state.openrouter.clone();
    let ollama_arc = state.ollama.clone();
    let canvas_store_arc = state.canvas_store.clone();
    let twin_store_arc = state.twin_store.clone();
    let settings_arc = state.settings_service.clone();
    let models = effective_model_ids(&model_route, &debate.participating_models);
    let reasoning_effort = request.reasoning_effort.clone();
    let provider_route = model_route.provider.clone();
    let stream_root_state = state.inner().clone();

    tauri::async_runtime::spawn(async move {
        let mut debate_state = debate;
        let mut root_epoch = root_epoch;
        let round_num = debate_state.rounds.len() as u32 + 1;

        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::RoundStart {
                session_id: session_id.clone(),
                debate_id: debate_id.clone(),
                round_number: round_num,
            },
        );

        let context = build_debate_round_user_message(
            round_num,
            "",
            &debate_state.rounds,
            Some(&request.prompt),
        );

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
                let response_provider = provider_route.provider_label().to_string();
                let response_provenance = provider_route.provenance_label(true).to_string();
                let stream_result = match provider_route {
                    ModelProviderRoute::Ollama => {
                        let ollama_handle = ollama_arc
                            .as_ref()
                            .expect("Ollama debate route was validated before spawning");
                        let ollama = ollama_handle.read().await;
                        let result = ollama
                            .chat_stream(&model_id, messages, None, Some(0.7))
                            .await;
                        drop(ollama);
                        result.map(no_cost_stream)
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
                                Some(&format!("canvas:{session_id}:{model_id}")),
                            )
                            .await;
                        drop(openrouter);
                        result.map(|stream| {
                            Box::pin(stream)
                                as std::pin::Pin<
                                    Box<
                                        dyn futures::Stream<Item = anyhow::Result<StreamUpdate>>
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
                        let mut cost_usd = None;

                        loop {
                            match tokio::time::timeout(Duration::from_secs(60), stream.next()).await
                            {
                                Ok(Some(Ok(update))) => {
                                    if update.cost_usd.is_some() {
                                        cost_usd = update.cost_usd;
                                    }
                                    let chunk = update.content;
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
                                        cost_usd: None,
                                        provider: Some(response_provider.clone()),
                                        provenance: Some(response_provenance.clone()),
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
                                        cost_usd: None,
                                        provider: Some(response_provider.clone()),
                                        provenance: Some(response_provenance.clone()),
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
                                cost_usd,
                            },
                        );

                        DebateResponse {
                            model_id,
                            model_name,
                            content: full_content,
                            stance: None,
                            cost_usd,
                            provider: Some(response_provider.clone()),
                            provenance: Some(response_provenance.clone()),
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
                                cost_usd: None,
                            },
                        );
                        DebateResponse {
                            model_id,
                            model_name,
                            content: e.to_string(),
                            cost_usd: None,
                            stance: None,
                            provider: Some(response_provider),
                            provenance: Some(response_provenance),
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
            topic: request.prompt.clone(),
            responses: round_responses,
            created_at: Utc::now(),
        };
        debate_state.rounds.push(round);

        let root_guard =
            match crate::commands::acquire_expected_root_epoch(&stream_root_state, &root_epoch)
                .await
            {
                Ok(guard) => guard,
                Err(error) => {
                    emit_debate_persist_error(
                        &window,
                        &session_id,
                        &debate_id,
                        round_num,
                        &debate_state,
                        &anyhow::anyhow!(error),
                    );
                    return;
                }
            };

        let commit = {
            let mut store = canvas_store_arc.write().await;
            match store.update_debate_expecting_authority(
                &session_id,
                &debate_state,
                root_epoch.clone(),
            ) {
                Ok(commit) => commit,
                Err(error) => {
                    let error = preserve_canvas_mutation_error(error);
                    drop(store);
                    drop(root_guard);
                    crate::commands::acknowledge_reported_repair(
                        repair_canvas_trace_error(
                            &stream_root_state,
                            &error,
                            "Canvas debate continuation persistence",
                        )
                        .await,
                    );
                    emit_debate_persist_error(
                        &window,
                        &session_id,
                        &debate_id,
                        round_num,
                        &debate_state,
                        &error,
                    );
                    return;
                }
            }
        };
        root_epoch = commit.authority_token.unwrap_or_else(|| root_epoch.clone());
        drop(root_guard);
        if !matches!(
            crate::commands::repair_after_authority_token(
                &stream_root_state,
                &root_epoch,
                "Canvas debate continuation",
            )
            .await,
            crate::commands::PostAuthorityRepair::Ready(_)
        ) {
            return;
        }
        let _ = window.emit(
            "canvas-stream",
            CanvasStreamEvent::DebateComplete {
                session_id: session_id.clone(),
                debate_id: debate_id.clone(),
            },
        );
        spawn_debate_chair_synthesis(
            window,
            session_id,
            debate_id,
            request.prompt.clone(),
            canvas_store_arc,
            twin_store_arc,
            openrouter_arc,
            ollama_arc,
            settings_arc,
        );
    });

    Ok(())
}

/// Same contract as `emit_persistence_error` but for debate rounds, which
/// don't have a single tile/model — a round can involve several models at
/// once. Emits a `DebateError` per participating model in the round that
/// failed to persist so the frontend has something concrete to render.
fn emit_debate_persist_error(
    window: &tauri::WebviewWindow,
    session_id: &str,
    debate_id: &str,
    round_number: u32,
    debate_state: &Debate,
    error: &(impl std::fmt::Display + ?Sized),
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

fn spawn_debate_chair_synthesis(
    window: tauri::WebviewWindow,
    session_id: String,
    debate_id: String,
    source_content: String,
    canvas_store: std::sync::Arc<tokio::sync::RwLock<crate::services::canvas_store::CanvasStore>>,
    twin_store: std::sync::Arc<tokio::sync::RwLock<crate::services::twin::TwinStore>>,
    openrouter: std::sync::Arc<tokio::sync::RwLock<crate::services::openrouter::OpenRouterService>>,
    ollama: Option<std::sync::Arc<tokio::sync::RwLock<crate::services::ollama::OllamaService>>>,
    settings: std::sync::Arc<tokio::sync::RwLock<crate::services::settings::SettingsService>>,
) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run_debate_chair_synthesis(
            window,
            session_id,
            debate_id,
            source_content,
            canvas_store,
            twin_store,
            openrouter,
            ollama,
            settings,
        )
        .await
        {
            log::warn!("Debate chair synthesis failed: {error}");
        }
    });
}

async fn run_debate_chair_synthesis(
    window: tauri::WebviewWindow,
    session_id: String,
    debate_id: String,
    source_content: String,
    canvas_store: std::sync::Arc<tokio::sync::RwLock<crate::services::canvas_store::CanvasStore>>,
    twin_store: std::sync::Arc<tokio::sync::RwLock<crate::services::twin::TwinStore>>,
    openrouter: std::sync::Arc<tokio::sync::RwLock<crate::services::openrouter::OpenRouterService>>,
    ollama: Option<std::sync::Arc<tokio::sync::RwLock<crate::services::ollama::OllamaService>>>,
    settings: std::sync::Arc<tokio::sync::RwLock<crate::services::settings::SettingsService>>,
) -> Result<(), String> {
    let debate = {
        let mut store = canvas_store.write().await;
        let session = store.get_session(&session_id).map_err(|e| e.to_string())?;
        session
            .debates
            .iter()
            .find(|debate| debate.id == debate_id)
            .cloned()
            .ok_or_else(|| "Debate not found for chair synthesis".to_string())?
    };

    let prompt = build_chair_synthesis_prompt(&source_content, &debate.rounds);
    let settings_snapshot = settings.read().await.get().clone();
    let use_ollama = settings_snapshot
        .twin_llm_provider
        .eq_ignore_ascii_case("ollama");
    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let raw = if use_ollama {
        let model = settings_snapshot.ollama_model.trim();
        if model.is_empty() {
            return Err("Select an Ollama model before synthesizing a debate".to_string());
        }
        let ollama =
            ollama.ok_or_else(|| "Local Ollama is unavailable on this runtime".to_string())?;
        let ollama = ollama.read().await;
        ollama
            .chat(model, messages, None, Some(0.4))
            .await
            .map_err(|e| e.to_string())?
    } else {
        let model = settings_snapshot.llm_model.trim();
        if model.is_empty() {
            return Err("No synthesis model configured".to_string());
        }
        let openrouter = openrouter.read().await;
        openrouter
            .chat(
                model,
                messages,
                None,
                Some(0.4),
                Some(700),
                Some("none"),
                false,
                0,
            )
            .await
            .map_err(|e| e.to_string())?
    };

    let recap = raw.trim().to_string();
    if recap.is_empty() {
        return Ok(());
    }

    {
        let mut store = canvas_store.write().await;
        let session = store.get_session(&session_id).map_err(|e| e.to_string())?;
        let Some(mut debate) = session
            .debates
            .iter()
            .find(|debate| debate.id == debate_id)
            .cloned()
        else {
            return Err("Debate not found for chair synthesis".to_string());
        };
        debate.recap = Some(recap.clone());
        debate.recap_updated_at = Some(Utc::now());
        store
            .update_debate(&session_id, &debate)
            .map_err(|e| e.to_string())?;
    }

    let _ = window.emit(
        "canvas-stream",
        CanvasStreamEvent::DebateRecapUpdated {
            session_id: session_id.clone(),
            debate_id: debate_id.clone(),
            recap: recap.clone(),
        },
    );

    let mut twin = twin_store.write().await;
    let _ = twin.append_trace_event(
        &session_id,
        TraceEventType::WorkingMemoryCompiled,
        json!({
            "debate_id": debate_id,
            "kind": "debate_chair",
            "recap": recap.chars().take(240).collect::<String>(),
        }),
    );

    Ok(())
}
