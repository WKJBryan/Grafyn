use crate::services::openrouter::ChatMessage;
use crate::AppState;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::State;

/// Streaming events emitted to the frontend via window.emit("chat-stream", ...)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatStreamEvent {
    /// Context notes that were injected into the system prompt
    ContextNotes {
        message_id: String,
        notes: Vec<ChatContextNote>,
    },
    /// A chunk of the streaming response
    Chunk {
        message_id: String,
        chunk: String,
    },
    /// Streaming complete
    Complete {
        message_id: String,
    },
    /// An error occurred
    Error {
        message_id: String,
        error: String,
    },
}

/// A note used as context in the chat, sent to the frontend for display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatContextNote {
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
}

/// Request payload for chat_send
#[derive(Debug, Clone, Deserialize)]
pub struct ChatSendRequest {
    pub message: String,
    /// Previous messages in the conversation (for multi-turn)
    #[serde(default)]
    pub history: Vec<ChatHistoryMessage>,
    /// Explicit note IDs to include as context (in addition to retrieval)
    #[serde(default)]
    pub context_note_ids: Vec<String>,
    /// LLM model to use (defaults to a reasonable model)
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_model() -> String {
    "anthropic/claude-sonnet-4-20250514".to_string()
}

/// A message in the chat history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatHistoryMessage {
    pub role: String,
    pub content: String,
}

/// Send a chat message with note context — returns message_id immediately,
/// streams response via "chat-stream" Tauri events.
#[tauri::command]
pub async fn chat_send(
    window: tauri::Window,
    request: ChatSendRequest,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let message_id = uuid::Uuid::new_v4().to_string();

    // 1. Retrieve relevant notes for context
    let context_notes = {
        let search = state.search_service.read().await;
        let graph = state.graph_index.read().await;
        let priority = state.priority_service.read().await;
        let retrieval = state.retrieval_service.read().await;

        retrieval
            .retrieve(
                &search,
                &graph,
                &priority,
                &request.message,
                5,
                &request.context_note_ids,
            )
            .unwrap_or_default()
    };

    // 2. Get full note content for context injection
    let note_contexts: Vec<(String, String, String)> = {
        let store = state.knowledge_store.read().await;
        context_notes
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

    // 3. Build system prompt with note context
    let system_prompt = build_system_prompt(&note_contexts);

    // 4. Build conversation messages
    let mut messages: Vec<ChatMessage> = request
        .history
        .iter()
        .map(|m| ChatMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        })
        .collect();

    messages.push(ChatMessage {
        role: "user".to_string(),
        content: request.message,
    });

    // 5. Get the OpenRouter service (clone to move into spawned task)
    let openrouter = {
        let or = state.openrouter.read().await;
        or.clone()
    };

    let model = request.model;
    let msg_id = message_id.clone();

    // 6. Emit context notes to frontend
    let chat_context: Vec<ChatContextNote> = context_notes
        .iter()
        .map(|r| ChatContextNote {
            id: r.note.id.clone(),
            title: r.note.title.clone(),
            snippet: r.snippet.clone(),
            score: r.score,
        })
        .collect();

    let _ = window.emit(
        "chat-stream",
        ChatStreamEvent::ContextNotes {
            message_id: msg_id.clone(),
            notes: chat_context,
        },
    );

    // 7. Spawn async streaming task
    tauri::async_runtime::spawn(async move {
        match openrouter
            .chat_stream(&model, messages, Some(&system_prompt), Some(0.7), Some(4096))
            .await
        {
            Ok(stream) => {
                let mut stream = Box::pin(stream);

                loop {
                    match tokio::time::timeout(Duration::from_secs(60), stream.next()).await {
                        Ok(Some(Ok(chunk))) => {
                            let _ = window.emit(
                                "chat-stream",
                                ChatStreamEvent::Chunk {
                                    message_id: msg_id.clone(),
                                    chunk,
                                },
                            );
                        }
                        Ok(Some(Err(e))) => {
                            let _ = window.emit(
                                "chat-stream",
                                ChatStreamEvent::Error {
                                    message_id: msg_id.clone(),
                                    error: e.to_string(),
                                },
                            );
                            break;
                        }
                        Ok(None) => {
                            // Stream ended
                            let _ = window.emit(
                                "chat-stream",
                                ChatStreamEvent::Complete {
                                    message_id: msg_id.clone(),
                                },
                            );
                            break;
                        }
                        Err(_) => {
                            // Timeout
                            let _ = window.emit(
                                "chat-stream",
                                ChatStreamEvent::Error {
                                    message_id: msg_id.clone(),
                                    error: "Stream timeout (60s idle)".to_string(),
                                },
                            );
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                let _ = window.emit(
                    "chat-stream",
                    ChatStreamEvent::Error {
                        message_id: msg_id.clone(),
                        error: e.to_string(),
                    },
                );
            }
        }
    });

    Ok(message_id)
}

/// Build a system prompt that includes retrieved note context
fn build_system_prompt(notes: &[(String, String, String)]) -> String {
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
    fn test_build_system_prompt_with_notes() {
        let notes = vec![
            ("id1".into(), "Note A".into(), "Content of A".into()),
            ("id2".into(), "Note B".into(), "Content of B".into()),
        ];
        let prompt = build_system_prompt(&notes);

        assert!(prompt.contains("Note A"));
        assert!(prompt.contains("Content of A"));
        assert!(prompt.contains("Note B"));
        assert!(prompt.contains("id: id1"));
    }

    #[test]
    fn test_build_system_prompt_empty() {
        let notes: Vec<(String, String, String)> = vec![];
        let prompt = build_system_prompt(&notes);

        assert!(prompt.contains("No relevant notes were found"));
    }

    #[test]
    fn test_default_model() {
        assert!(default_model().contains("claude"));
    }
}
