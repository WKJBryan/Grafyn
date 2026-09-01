//! Tauri commands for the standalone local Twin Eval Lab.

use crate::models::twin::{ActionGap, ConstitutionItem};
use crate::services::openrouter::ChatMessage;
use crate::services::twin_eval::{
    build_base_completion_prompt, build_lab_prompt, default_model_matrix, export_results,
    parse_lab_question, score_lab_response, TwinEvalCaseResult, TwinEvalContextMode,
    TwinEvalContextPacket, TwinEvalContextSource, TwinEvalExport, TwinEvalLabQuestion,
    TwinEvalModelConfig, TwinEvalRunReport, TwinEvalRunRequest, TwinEvalRunSettings,
    TwinEvalSkippedModel,
};
use crate::AppState;
use futures::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Emitter, State};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LabStreamEvent {
    Start {
        question: TwinEvalLabQuestion,
        context_packet: TwinEvalContextPacket,
        model_keys: Vec<String>,
    },
    ModelStart {
        model_key: String,
    },
    ModelComplete {
        result: TwinEvalCaseResult,
    },
    Complete {
        skipped_models: Vec<TwinEvalSkippedModel>,
    },
    Error {
        message: String,
    },
}

#[tauri::command]
pub async fn get_twin_eval_model_matrix(
    state: State<'_, AppState>,
) -> Result<Vec<TwinEvalModelConfig>, String> {
    let installed = installed_ollama_model_ids(&state).await;
    Ok(default_model_matrix(&installed))
}

#[tauri::command]
pub async fn preview_twin_eval_input(
    raw_question: String,
    answer_key: Option<String>,
) -> Result<TwinEvalLabQuestion, String> {
    parse_lab_question(&raw_question, answer_key).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn preview_twin_eval_context(
    state: State<'_, AppState>,
    request: TwinEvalRunRequest,
) -> Result<TwinEvalContextPacket, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let question = parse_lab_question(&request.raw_question, request.answer_key.clone())
        .map_err(|error| error.to_string())?;
    let packet = build_context_packet(&state, &question, &request).await?;
    root_ticket.finish(state.inner()).await?;
    Ok(packet)
}

#[tauri::command]
pub async fn run_twin_eval_lab(
    state: State<'_, AppState>,
    request: TwinEvalRunRequest,
) -> Result<TwinEvalRunReport, String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let root_epoch = root_ticket.authority().clone();
    let question = parse_lab_question(&request.raw_question, request.answer_key.clone())
        .map_err(|error| error.to_string())?;
    let context_packet = build_context_packet(&state, &question, &request).await?;
    let settings = request.settings();
    root_ticket.finish(state.inner()).await?;

    let installed = installed_ollama_model_ids(&state).await;
    let matrix = default_model_matrix(&installed);
    let matrix_by_key = matrix
        .iter()
        .map(|model| (model.key.clone(), model.clone()))
        .collect::<HashMap<_, _>>();
    let selected_keys = if request.model_keys.is_empty() {
        matrix
            .iter()
            .filter(|model| model.runner_ready)
            .map(|model| model.key.clone())
            .collect::<Vec<_>>()
    } else {
        request.model_keys.clone()
    };

    let mut runnable = Vec::new();
    let mut skipped_models = Vec::new();
    for key in selected_keys {
        let Some(model) = matrix_by_key.get(&key) else {
            skipped_models.push(TwinEvalSkippedModel {
                key,
                reason: "unknown model key".to_string(),
            });
            continue;
        };

        if !model.runner_ready {
            skipped_models.push(TwinEvalSkippedModel {
                key: model.key.clone(),
                reason: format!(
                    "model is not installed as matched {} Ollama tag {}",
                    model.quantization, model.runner_model_id
                ),
            });
            continue;
        }

        runnable.push(model.clone());
    }

    if runnable.is_empty() {
        return Err("No runnable matched-quantization local Ollama models selected.".to_string());
    }

    let mut results = Vec::new();
    for model in &runnable {
        let is_base = model.training_stage.contains("base");
        let prompt = if is_base {
            build_base_completion_prompt(&question)
        } else {
            build_lab_prompt(&question, &context_packet, &settings, true)
        };
        match run_ollama_lab_case(&state, model, &prompt, &context_packet, &settings, is_base).await
        {
            Ok(response) => {
                results.push(score_lab_response(
                    &question,
                    &model.key,
                    &response,
                    settings.structured_output,
                    settings.show_reasoning_trace,
                ));
            }
            Err(error) => {
                results.push(TwinEvalCaseResult {
                    case_id: question.id.clone(),
                    model_key: model.key.clone(),
                    raw_response: String::new(),
                    final_answer: None,
                    selected_option: None,
                    outside_options_answer: None,
                    confidence: None,
                    rationale: None,
                    model_trace: None,
                    context_citations: Vec::new(),
                    correctness_score: None,
                    sycophancy_flag: false,
                    error: Some(error),
                });
            }
        }
    }

    let _root_guard =
        crate::commands::acquire_expected_root_epoch(state.inner(), &root_epoch).await?;
    Ok(TwinEvalRunReport {
        question,
        context_packet,
        results,
        skipped_models,
    })
}

#[tauri::command]
pub async fn export_twin_eval_results(
    results: Vec<TwinEvalCaseResult>,
) -> Result<TwinEvalExport, String> {
    export_results(&results).map_err(|error| error.to_string())
}

/// Streaming version: returns immediately and emits `lab-stream` Tauri events as each
/// model completes. All selected models run in parallel via JoinSet so that small E2B
/// models finish and appear in the table while larger 27B models are still generating.
#[tauri::command]
pub async fn run_twin_eval_lab_stream(
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    request: TwinEvalRunRequest,
) -> Result<(), String> {
    let root_ticket = crate::commands::acquire_derived_root_epoch(state.inner()).await?;
    let root_epoch = root_ticket.authority().clone();
    let question = parse_lab_question(&request.raw_question, request.answer_key.clone())
        .map_err(|error| error.to_string())?;
    let context_packet = build_context_packet(&state, &question, &request).await?;
    let settings = request.settings();
    root_ticket.finish(state.inner()).await?;

    let installed = installed_ollama_model_ids(&state).await;
    let matrix = default_model_matrix(&installed);
    let matrix_by_key = matrix
        .iter()
        .map(|model| (model.key.clone(), model.clone()))
        .collect::<HashMap<_, _>>();
    let selected_keys = if request.model_keys.is_empty() {
        matrix
            .iter()
            .filter(|model| model.runner_ready)
            .map(|model| model.key.clone())
            .collect::<Vec<_>>()
    } else {
        request.model_keys.clone()
    };

    let mut runnable = Vec::new();
    let mut skipped_models = Vec::new();
    for key in selected_keys {
        let Some(model) = matrix_by_key.get(&key) else {
            skipped_models.push(TwinEvalSkippedModel {
                key,
                reason: "unknown model key".to_string(),
            });
            continue;
        };
        if !model.runner_ready {
            skipped_models.push(TwinEvalSkippedModel {
                key: model.key.clone(),
                reason: format!(
                    "model is not installed as matched {} Ollama tag {}",
                    model.quantization, model.runner_model_id
                ),
            });
            continue;
        }
        runnable.push(model.clone());
    }

    if runnable.is_empty() {
        let _ = window.emit(
            "lab-stream",
            LabStreamEvent::Error {
                message: "No runnable matched-quantization local Ollama models selected."
                    .to_string(),
            },
        );
        return Ok(());
    }

    let model_keys = runnable.iter().map(|m| m.key.clone()).collect::<Vec<_>>();

    let _ = window.emit(
        "lab-stream",
        LabStreamEvent::Start {
            question: question.clone(),
            context_packet: context_packet.clone(),
            model_keys,
        },
    );

    let ollama_arc = state.ollama_service()?;
    let question = Arc::new(question);
    let context_packet = Arc::new(context_packet);
    let settings = Arc::new(settings);
    let system_prompt = if context_packet.system_prompt.trim().is_empty() {
        None
    } else {
        Some(Arc::new(context_packet.system_prompt.clone()))
    };
    let root_state = state.inner().clone();

    tauri::async_runtime::spawn(async move {
        let mut join_set = tokio::task::JoinSet::new();

        for model in runnable {
            let ollama_arc = Arc::clone(&ollama_arc);
            let window = window.clone();
            let question = Arc::clone(&question);
            let context_packet = Arc::clone(&context_packet);
            let settings = Arc::clone(&settings);
            let system_prompt = system_prompt.clone();
            let root_state = root_state.clone();
            let root_epoch = root_epoch.clone();

            let is_base = model.training_stage.contains("base");
            let prompt = Arc::new(if is_base {
                build_base_completion_prompt(&question)
            } else {
                build_lab_prompt(&question, &context_packet, &settings, true)
            });

            let _ = window.emit(
                "lab-stream",
                LabStreamEvent::ModelStart {
                    model_key: model.key.clone(),
                },
            );

            join_set.spawn(async move {
                let result = run_ollama_lab_case_arc(
                    &ollama_arc,
                    &model,
                    &prompt,
                    system_prompt,
                    &settings,
                    is_base,
                )
                .await;
                let scored = match result {
                    Ok(response) => score_lab_response(
                        &question,
                        &model.key,
                        &response,
                        settings.structured_output,
                        settings.show_reasoning_trace,
                    ),
                    Err(error) => TwinEvalCaseResult {
                        case_id: question.id.clone(),
                        model_key: model.key.clone(),
                        raw_response: String::new(),
                        final_answer: None,
                        selected_option: None,
                        outside_options_answer: None,
                        confidence: None,
                        rationale: None,
                        model_trace: None,
                        context_citations: Vec::new(),
                        correctness_score: None,
                        sycophancy_flag: false,
                        error: Some(error),
                    },
                };
                let _root_guard =
                    match crate::commands::acquire_expected_root_epoch(&root_state, &root_epoch)
                        .await
                    {
                        Ok(guard) => guard,
                        Err(error) => {
                            let _ = window.emit(
                                "lab-stream",
                                LabStreamEvent::Error {
                                    message: format!(
                                        "Twin Eval result discarded after vault switch: {error}"
                                    ),
                                },
                            );
                            return;
                        }
                    };
                let _ = window.emit(
                    "lab-stream",
                    LabStreamEvent::ModelComplete { result: scored },
                );
            });
        }

        while join_set.join_next().await.is_some() {}

        let _ = window.emit("lab-stream", LabStreamEvent::Complete { skipped_models });
    });

    Ok(())
}

async fn run_ollama_lab_case_arc(
    ollama_arc: &Arc<tokio::sync::RwLock<crate::services::ollama::OllamaService>>,
    model: &TwinEvalModelConfig,
    prompt: &str,
    system_prompt: Option<Arc<String>>,
    settings: &TwinEvalRunSettings,
    is_base: bool,
) -> Result<String, String> {
    let ollama = ollama_arc.read().await;
    let mut raw = String::new();

    if is_base {
        let stream = ollama
            .generate_completion_stream(
                &model.runner_model_id,
                prompt,
                settings.temperature,
                settings.top_p,
                Some(lab_generation_token_limit(settings, is_base)),
            )
            .await
            .map_err(|error| error.to_string())?;
        drop(ollama);
        futures::pin_mut!(stream);
        while let Some(chunk) = stream.next().await {
            raw.push_str(&chunk.map_err(|error| error.to_string())?);
        }
    } else {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }];
        let system_prompt_str = system_prompt.as_deref().map(|s| s.as_str());
        let stream = ollama
            .chat_stream_with_options(
                &model.runner_model_id,
                messages,
                system_prompt_str,
                settings.temperature,
                settings.top_p,
                Some(settings.max_tokens),
                None,
            )
            .await
            .map_err(|error| error.to_string())?;
        drop(ollama);
        futures::pin_mut!(stream);
        while let Some(chunk) = stream.next().await {
            raw.push_str(&chunk.map_err(|error| error.to_string())?);
        }
    }

    Ok(raw)
}

fn lab_generation_token_limit(settings: &TwinEvalRunSettings, _is_base: bool) -> u32 {
    settings.max_tokens
}

async fn build_context_packet(
    state: &State<'_, AppState>,
    question: &TwinEvalLabQuestion,
    request: &TwinEvalRunRequest,
) -> Result<TwinEvalContextPacket, String> {
    let mode = request.context_mode.clone();
    let mut packet = TwinEvalContextPacket::empty(mode.clone());
    if mode == TwinEvalContextMode::SystemOnly {
        packet.system_prompt = request
            .system_prompt
            .clone()
            .filter(|prompt| !prompt.trim().is_empty())
            .unwrap_or_default();
    }

    if mode.uses_retrieval() {
        let retrieval_query = build_retrieval_query(question);
        packet.retrieval_items = retrieve_note_context(state, &retrieval_query).await?;
    }

    if mode.uses_constitution() {
        let store = state.twin_store.read().await;
        let (items, gaps) = store
            .select_constitution_context(&question.raw_input)
            .map_err(|error| error.to_string())?;
        packet.constitution_items = items.into_iter().map(constitution_source).collect();
        packet.action_gaps = gaps.into_iter().map(action_gap_source).collect();
    }

    packet.private_store_accessed = mode.uses_retrieval() || mode.uses_constitution();
    Ok(packet)
}

async fn retrieve_note_context(
    state: &State<'_, AppState>,
    query: &str,
) -> Result<Vec<TwinEvalContextSource>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }

    let search = state.search_service.read().await;
    let graph = state.graph_index.read().await;
    let priority = state.priority_service.read().await;
    let retrieval = state.retrieval_service.read().await;
    let results = retrieval.retrieve(&search, &graph, &priority, query, 5, &[])?;

    Ok(results
        .into_iter()
        .map(|result| TwinEvalContextSource {
            source_type: "retrieval_note".to_string(),
            id: result.note.id,
            label: result.note.title,
            snippet: result.snippet,
            weight: Some(result.score),
            reason: Some(result.relevance_reasons.join("; ")),
        })
        .collect())
}

fn build_retrieval_query(question: &TwinEvalLabQuestion) -> String {
    let mut text = question.question.clone();
    for option in &question.options {
        text.push(' ');
        text.push_str(&option.text);
    }

    sanitize_retrieval_query(&text)
}

fn sanitize_retrieval_query(input: &str) -> String {
    let normalized = input
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character.is_whitespace() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>();

    normalized
        .split_whitespace()
        .filter(|token| {
            token.len() > 1
                && !matches!(
                    *token,
                    "an" | "and"
                        | "are"
                        | "as"
                        | "at"
                        | "be"
                        | "by"
                        | "do"
                        | "for"
                        | "from"
                        | "in"
                        | "is"
                        | "it"
                        | "of"
                        | "on"
                        | "or"
                        | "the"
                        | "to"
                        | "what"
                        | "which"
                        | "with"
                        | "you"
                )
        })
        .take(80)
        .collect::<Vec<_>>()
        .join(" ")
}

fn constitution_source(item: ConstitutionItem) -> TwinEvalContextSource {
    TwinEvalContextSource {
        source_type: "constitution_item".to_string(),
        id: item.id,
        label: item.dimension,
        snippet: item.claim,
        weight: Some(item.confidence),
        reason: Some(format!(
            "status {:?}; priority {:.2}",
            item.status, item.priority
        )),
    }
}

fn action_gap_source(gap: ActionGap) -> TwinEvalContextSource {
    TwinEvalContextSource {
        source_type: "action_gap".to_string(),
        id: gap.id,
        label: gap.decision_risk.clone(),
        snippet: format!(
            "Stated: {} | Revealed: {} | Risk: {}",
            gap.stated_value, gap.revealed_behavior, gap.decision_risk
        ),
        weight: Some(gap.confidence),
        reason: gap.driver_hypothesis,
    }
}

async fn installed_ollama_model_ids(state: &State<'_, AppState>) -> Vec<String> {
    let Ok(service) = state.ollama_service() else {
        return Vec::new();
    };
    let ollama = service.read().await;
    ollama
        .list_models()
        .await
        .map(|models| models.into_iter().map(|model| model.id).collect())
        .unwrap_or_default()
}

async fn run_ollama_lab_case(
    state: &State<'_, AppState>,
    model: &TwinEvalModelConfig,
    prompt: &str,
    context_packet: &TwinEvalContextPacket,
    settings: &TwinEvalRunSettings,
    is_base: bool,
) -> Result<String, String> {
    let service = state.ollama_service()?;
    let ollama = service.read().await;
    let mut raw = String::new();

    if is_base {
        let stream = ollama
            .generate_completion_stream(
                &model.runner_model_id,
                prompt,
                settings.temperature,
                settings.top_p,
                Some(lab_generation_token_limit(settings, is_base)),
            )
            .await
            .map_err(|error| error.to_string())?;
        drop(ollama);
        futures::pin_mut!(stream);
        while let Some(chunk) = stream.next().await {
            raw.push_str(&chunk.map_err(|error| error.to_string())?);
        }
    } else {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt.to_string(),
        }];
        let system_prompt = if context_packet.system_prompt.trim().is_empty() {
            None
        } else {
            Some(context_packet.system_prompt.as_str())
        };
        let stream = ollama
            .chat_stream_with_options(
                &model.runner_model_id,
                messages,
                system_prompt,
                settings.temperature,
                settings.top_p,
                Some(settings.max_tokens),
                None,
            )
            .await
            .map_err(|error| error.to_string())?;
        drop(ollama);
        futures::pin_mut!(stream);
        while let Some(chunk) = stream.next().await {
            raw.push_str(&chunk.map_err(|error| error.to_string())?);
        }
    }

    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::twin_eval::TwinEvalOption;

    #[test]
    fn derives_search_safe_retrieval_query_from_parsed_question() {
        let question = TwinEvalLabQuestion {
            id: "case-1".to_string(),
            raw_input: "\"Prediction Scenario 1: Strategic Alignment vs. Opportunity\"\n\nA) Accept\nB) Negotiate\nC) Decline".to_string(),
            question: "What is the best choice for SUTD's Design AI transformation?".to_string(),
            options: vec![
                TwinEvalOption {
                    key: "A".to_string(),
                    text: "Accept the grant to secure funding.".to_string(),
                },
                TwinEvalOption {
                    key: "B".to_string(),
                    text: "Negotiate to make the project more design-centric.".to_string(),
                },
                TwinEvalOption {
                    key: "C".to_string(),
                    text: "Decline the grant to protect faculty time.".to_string(),
                },
            ],
            answer_key: None,
        };

        let query = build_retrieval_query(&question);

        assert!(query.contains("sutd"));
        assert!(query.contains("design"));
        assert!(query.contains("grant"));
        assert!(!query.contains('"'));
        assert!(!query.contains(')'));
        assert!(!query.contains(':'));
    }

    #[test]
    fn base_lab_generation_uses_configured_max_tokens() {
        let settings = TwinEvalRunSettings {
            max_tokens: 1024,
            ..TwinEvalRunSettings::default()
        };

        assert_eq!(lab_generation_token_limit(&settings, true), 1024);
        assert_eq!(lab_generation_token_limit(&settings, false), 1024);
    }
}
