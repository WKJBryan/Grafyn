//! Parser for Claude conversation exports (.dms / JSON)
//!
//! Claude exports use a flat message array with "type" (prompt/response)
//! or "sender" (human/assistant) fields. Multiple message key names are
//! supported: chat_messages, messages, chat, conversation.

use crate::models::import::*;
use crate::services::import::parse_timestamp;
use anyhow::Result;

/// Check if JSON content matches Claude export format.
pub fn can_parse(content: &str) -> bool {
    let Ok(data) = serde_json::from_str::<serde_json::Value>(content) else {
        return false;
    };

    if let Some(obj) = data.as_object() {
        return obj.keys().any(|k| {
            matches!(
                k.as_str(),
                "uuid" | "chat" | "conversation" | "chat_log" | "conversation_log"
            )
        });
    }

    if let Some(arr) = data.as_array() {
        if arr.is_empty() {
            return false;
        }
        let first = &arr[0];
        if let Some(obj) = first.as_object() {
            return obj.keys().any(|k| {
                matches!(
                    k.as_str(),
                    "uuid" | "chat" | "conversation" | "message" | "chat_messages"
                )
            });
        }
    }

    false
}

/// Parse Claude conversations from JSON content.
pub fn parse(content: &str) -> Result<Vec<ParsedConversation>> {
    let data: serde_json::Value = serde_json::from_str(content)?;
    let mut conversations = Vec::new();

    if let Some(arr) = data.as_array() {
        for conv_data in arr {
            if let Some(conv) = parse_single(conv_data) {
                conversations.push(conv);
            }
        }
    } else if let Some(conv) = parse_single(&data) {
        conversations.push(conv);
    }

    log::info!("Parsed {} Claude conversations", conversations.len());
    Ok(conversations)
}

fn parse_single(data: &serde_json::Value) -> Option<ParsedConversation> {
    let conv_id = data
        .get("uuid")
        .or_else(|| data.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let title = data
        .get("name")
        .or_else(|| data.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled Conversation")
        .to_string();

    let messages_key = find_messages_key(data);
    let messages_list = data.get(&messages_key).and_then(|v| v.as_array())?;
    let messages = extract_messages(messages_list);

    if messages.is_empty() {
        return None;
    }

    let model = data
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let mut model_info = vec![];
    if model != "unknown" {
        model_info.push(model);
    }
    for msg in &messages {
        if let Some(ref m) = msg.model {
            if !model_info.contains(m) {
                model_info.push(m.clone());
            }
        }
    }

    let created_at = parse_timestamp(data.get("created_at").or_else(|| data.get("createdAt")));
    let updated_at =
        parse_timestamp(data.get("updated_at").or_else(|| data.get("updatedAt"))).or(created_at);

    Some(ParsedConversation {
        id: conv_id,
        title,
        platform: "claude".to_string(),
        metadata: ConversationMetadata {
            platform: "claude".to_string(),
            created_at,
            updated_at,
            message_count: messages.len(),
            model_info,
        },
        messages,
        suggested_tags: vec!["claude".to_string(), "import".to_string()],
    })
}

fn find_messages_key(data: &serde_json::Value) -> String {
    for key in &[
        "chat_messages",
        "messages",
        "chat",
        "conversation",
        "chat_log",
        "conversation_log",
    ] {
        if data.get(*key).and_then(|v| v.as_array()).is_some() {
            return key.to_string();
        }
    }
    "messages".to_string()
}

fn extract_messages(messages_list: &[serde_json::Value]) -> Vec<ParsedMessage> {
    let mut messages = Vec::new();

    for (i, msg_data) in messages_list.iter().enumerate() {
        let msg_type = msg_data.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let sender = msg_data
            .get("sender")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let role = if msg_type == "prompt" || sender == "human" {
            "user"
        } else if msg_type == "response" || sender == "assistant" {
            "assistant"
        } else if i % 2 == 0 {
            "user"
        } else {
            "assistant"
        };

        let content = extract_message_content(msg_data);
        if content.is_empty() {
            continue;
        }

        let timestamp = parse_timestamp(
            msg_data
                .get("timestamp")
                .or_else(|| msg_data.get("created_at"))
                .or_else(|| msg_data.get("updated_at")),
        );

        let model = if role == "assistant" {
            msg_data
                .get("model")
                .or_else(|| msg_data.get("model_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        } else {
            None
        };

        messages.push(ParsedMessage {
            index: i,
            role: role.to_string(),
            content,
            timestamp,
            model,
        });
    }

    messages
}

fn extract_message_content(msg_data: &serde_json::Value) -> String {
    // Direct text field (official Claude export)
    if let Some(text) = msg_data.get("text").and_then(|v| v.as_str()) {
        return text.to_string();
    }

    // Direct message field
    if let Some(msg) = msg_data.get("message") {
        if let Some(s) = msg.as_str() {
            return s.to_string();
        }
        if let Some(text) = msg.get("text").and_then(|v| v.as_str()) {
            return text.to_string();
        }
    }

    // Content field (various formats)
    if let Some(content) = msg_data.get("content") {
        if let Some(s) = content.as_str() {
            return s.to_string();
        }
        if let Some(obj) = content.as_object() {
            if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                return text.to_string();
            }
        }
        if let Some(arr) = content.as_array() {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|part| {
                    if let Some(s) = part.as_str() {
                        return Some(s.to_string());
                    }
                    part.get("text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            return parts.join("\n");
        }
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_parse_claude() {
        let content = r#"[{"uuid": "abc", "name": "Test", "chat_messages": [{"type": "prompt", "text": "hi"}]}]"#;
        assert!(can_parse(content));
    }

    #[test]
    fn test_parse_claude_conversation() {
        let content = r#"[{
            "uuid": "conv1",
            "name": "Claude Chat",
            "model": "claude-3-opus",
            "chat_messages": [
                {"type": "prompt", "text": "Hello Claude"},
                {"type": "response", "text": "Hello! How can I help?", "model": "claude-3-opus"}
            ]
        }]"#;

        let result = parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "Claude Chat");
        assert_eq!(result[0].messages.len(), 2);
        assert_eq!(result[0].messages[0].role, "user");
        assert_eq!(result[0].messages[0].content, "Hello Claude");
        assert_eq!(result[0].messages[1].role, "assistant");
    }

    #[test]
    fn test_parse_claude_sender_format() {
        let content = r#"{"uuid": "c1", "name": "Test", "messages": [
            {"sender": "human", "content": "question"},
            {"sender": "assistant", "content": "answer"}
        ]}"#;

        let result = parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].messages.len(), 2);
        assert_eq!(result[0].messages[0].role, "user");
        assert_eq!(result[0].messages[1].role, "assistant");
    }
}
