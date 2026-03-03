//! Parser for Gemini (Google) conversation exports
//!
//! Gemini exports typically have a `project` name, `conversation_id`,
//! and a `messages` array where roles are "user" or "model".

use crate::models::import::*;
use crate::services::import::parse_timestamp;
use anyhow::Result;

/// Check if JSON content matches Gemini export format.
pub fn can_parse(content: &str) -> bool {
    let Ok(data) = serde_json::from_str::<serde_json::Value>(content) else {
        return false;
    };

    if let Some(obj) = data.as_object() {
        return obj.keys().any(|k| {
            matches!(
                k.as_str(),
                "project" | "conversation_id" | "gemini_version" | "canvas_content"
            )
        });
    }

    false
}

/// Parse Gemini conversations from JSON content.
pub fn parse(content: &str) -> Result<Vec<ParsedConversation>> {
    let data: serde_json::Value = serde_json::from_str(content)?;
    let mut conversations = Vec::new();

    if data.get("messages").is_some() || data.get("chat_messages").is_some() {
        if let Some(conv) = parse_single(&data) {
            conversations.push(conv);
        }
    } else if let Some(arr) = data.as_array() {
        for conv_data in arr {
            if let Some(conv) = parse_single(conv_data) {
                conversations.push(conv);
            }
        }
    }

    log::info!("Parsed {} Gemini conversations", conversations.len());
    Ok(conversations)
}

fn parse_single(data: &serde_json::Value) -> Option<ParsedConversation> {
    let conv_id = data
        .get("conversation_id")
        .or_else(|| data.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let project_name = data
        .get("project")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled Project");

    let title = format!("Gemini: {}", project_name);

    let messages_key = find_messages_key(data);
    let messages_list = data.get(&messages_key).and_then(|v| v.as_array())?;
    let messages = extract_messages(messages_list);

    if messages.is_empty() {
        return None;
    }

    let mut model_info: Vec<String> = Vec::new();
    for msg in &messages {
        if let Some(ref m) = msg.model {
            if !model_info.contains(m) {
                model_info.push(m.clone());
            }
        }
    }
    if model_info.is_empty() {
        model_info.push("gemini".to_string());
    }

    let created_at = parse_timestamp(data.get("export_time").or_else(|| data.get("timestamp")));

    Some(ParsedConversation {
        id: conv_id,
        title,
        platform: "gemini".to_string(),
        metadata: ConversationMetadata {
            platform: "gemini".to_string(),
            created_at,
            updated_at: created_at,
            message_count: messages.len(),
            model_info,
        },
        messages,
        suggested_tags: vec!["gemini".to_string(), "import".to_string()],
    })
}

fn find_messages_key(data: &serde_json::Value) -> String {
    for key in &["messages", "chat_messages", "conversation", "chat"] {
        if data.get(*key).and_then(|v| v.as_array()).is_some() {
            return key.to_string();
        }
    }
    "messages".to_string()
}

fn extract_messages(messages_list: &[serde_json::Value]) -> Vec<ParsedMessage> {
    let mut messages = Vec::new();

    for (i, msg_data) in messages_list.iter().enumerate() {
        let role_str = msg_data
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let role = match role_str {
            "user" => "user",
            "model" => "assistant",
            _ => {
                if i % 2 == 0 {
                    "user"
                } else {
                    "assistant"
                }
            }
        };

        let content = extract_message_content(msg_data);
        if content.is_empty() {
            continue;
        }

        let timestamp = parse_timestamp(msg_data.get("timestamp"));

        let model = if role == "assistant" {
            msg_data
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| Some("gemini".to_string()))
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
    let Some(content_obj) = msg_data.get("content") else {
        return String::new();
    };

    // Simple string
    if let Some(s) = content_obj.as_str() {
        return s.to_string();
    }

    // Object with parts (multimodal content)
    if let Some(parts) = content_obj.get("parts").and_then(|v| v.as_array()) {
        let text_parts: Vec<String> = parts
            .iter()
            .filter_map(|part| {
                if let Some(s) = part.as_str() {
                    return Some(s.to_string());
                }
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    return Some(text.to_string());
                }
                if let Some(code) = part.get("code").and_then(|v| v.as_str()) {
                    let lang = part
                        .get("language")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    return Some(format!("```{}\n{}\n```", lang, code));
                }
                None
            })
            .collect();
        return text_parts.join("\n");
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_parse_gemini() {
        let content = r#"{"project": "Test", "messages": [{"role": "user", "content": "hi"}]}"#;
        assert!(can_parse(content));
    }

    #[test]
    fn test_parse_gemini_conversation() {
        let content = r#"{
            "project": "My Project",
            "conversation_id": "gem1",
            "messages": [
                {"role": "user", "content": "What is Rust?"},
                {"role": "model", "content": "Rust is a systems programming language.", "model": "gemini-pro"}
            ]
        }"#;

        let result = parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "Gemini: My Project");
        assert_eq!(result[0].messages.len(), 2);
        assert_eq!(result[0].messages[0].role, "user");
        assert_eq!(result[0].messages[1].role, "assistant");
        assert_eq!(result[0].messages[1].model, Some("gemini-pro".to_string()));
    }
}
