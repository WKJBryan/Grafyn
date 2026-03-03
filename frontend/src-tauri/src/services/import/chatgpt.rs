//! Parser for ChatGPT conversation exports (conversations.json)
//!
//! ChatGPT exports use a tree-based "mapping" structure where each message
//! is a node with parent/children references. Messages are extracted by
//! sorting on create_time for chronological order.

use crate::models::import::*;
use crate::services::import::parse_timestamp;
use anyhow::Result;

/// Check if JSON content matches ChatGPT export format.
pub fn can_parse(content: &str) -> bool {
    let Ok(data) = serde_json::from_str::<serde_json::Value>(content) else {
        return false;
    };

    if let Some(arr) = data.as_array() {
        if arr.is_empty() {
            return false;
        }
        let first = &arr[0];
        // ChatGPT has 'mapping' key; exclude Claude which has 'chat_messages'
        first.get("mapping").is_some()
            || (first.get("title").is_some()
                && first.get("create_time").is_some()
                && first.get("chat_messages").is_none())
    } else {
        data.get("mapping").is_some() && data.get("chat_messages").is_none()
    }
}

/// Parse ChatGPT conversations from JSON content.
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

    log::info!("Parsed {} ChatGPT conversations", conversations.len());
    Ok(conversations)
}

fn parse_single(data: &serde_json::Value) -> Option<ParsedConversation> {
    let conv_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let title = data
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled Conversation")
        .to_string();

    let created_at = parse_timestamp(data.get("create_time"));
    let updated_at = parse_timestamp(data.get("update_time")).or(created_at);

    // Extract messages from mapping structure
    let mapping = data.get("mapping")?.as_object()?;
    let messages = extract_messages_from_mapping(mapping);

    if messages.is_empty() {
        return None;
    }

    let model_info: Vec<String> = {
        let mut set = std::collections::HashSet::new();
        for msg in &messages {
            if let Some(ref model) = msg.model {
                set.insert(model.clone());
            }
        }
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    };

    Some(ParsedConversation {
        id: conv_id,
        title,
        platform: "chatgpt".to_string(),
        metadata: ConversationMetadata {
            platform: "chatgpt".to_string(),
            created_at,
            updated_at,
            message_count: messages.len(),
            model_info,
        },
        messages,
        suggested_tags: vec!["chatgpt".to_string(), "import".to_string()],
    })
}

fn extract_messages_from_mapping(
    mapping: &serde_json::Map<String, serde_json::Value>,
) -> Vec<ParsedMessage> {
    // Collect messages that have actual content
    let mut entries: Vec<(&str, &serde_json::Value, f64)> = Vec::new();

    for (msg_id, node) in mapping {
        if let Some(message) = node.get("message") {
            if message.get("content").is_some() {
                let time = message
                    .get("create_time")
                    .and_then(|t| t.as_f64())
                    .unwrap_or(0.0);
                entries.push((msg_id.as_str(), message, time));
            }
        }
    }

    // Sort chronologically
    entries.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut messages = Vec::new();
    for (i, (_msg_id, message, _time)) in entries.iter().enumerate() {
        let author_role = message
            .get("author")
            .and_then(|a| a.get("role"))
            .and_then(|r| r.as_str())
            .unwrap_or("user");

        let role = match author_role {
            "system" => "system",
            "assistant" | "tool" => "assistant",
            _ => "user",
        };

        let content = extract_content(message);
        if content.is_empty() {
            continue;
        }

        let model = message
            .get("metadata")
            .and_then(|m| m.get("model_slug").or_else(|| m.get("modelSlug")))
            .and_then(|m| m.as_str())
            .map(|s| s.to_string());

        let timestamp = parse_timestamp(message.get("create_time"));

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

fn extract_content(message: &serde_json::Value) -> String {
    let Some(content_obj) = message.get("content") else {
        return String::new();
    };

    // Simple string
    if let Some(s) = content_obj.as_str() {
        return s.to_string();
    }

    // Dict with 'parts' array (common ChatGPT format)
    if let Some(parts) = content_obj.get("parts").and_then(|p| p.as_array()) {
        let text_parts: Vec<String> = parts
            .iter()
            .filter_map(|part| {
                if let Some(s) = part.as_str() {
                    Some(s.to_string())
                } else {
                    part.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())
                }
            })
            .collect();
        if !text_parts.is_empty() {
            return text_parts.join("\n");
        }
    }

    // Direct text field
    if let Some(text) = content_obj.get("text").and_then(|t| t.as_str()) {
        return text.to_string();
    }

    // Array of content parts
    if let Some(arr) = content_obj.as_array() {
        let text_parts: Vec<String> = arr
            .iter()
            .filter_map(|part| {
                if let Some(s) = part.as_str() {
                    return Some(s.to_string());
                }
                part.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())
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
    fn test_can_parse_chatgpt() {
        let content = r#"[{"id":"abc","title":"Test","create_time":1234567890,"mapping":{"n1":{"message":{"content":{"parts":["hello"]},"author":{"role":"user"},"create_time":1234567890}}}}]"#;
        assert!(can_parse(content));
    }

    #[test]
    fn test_rejects_claude_format() {
        let content = r#"[{"uuid":"abc","name":"Test","chat_messages":[]}]"#;
        assert!(!can_parse(content));
    }

    #[test]
    fn test_parse_chatgpt_conversation() {
        let content = r#"[{
            "id": "conv1",
            "title": "Test Chat",
            "create_time": 1704067200,
            "mapping": {
                "msg1": {
                    "message": {
                        "content": {"parts": ["Hello!"]},
                        "author": {"role": "user"},
                        "create_time": 1704067200
                    }
                },
                "msg2": {
                    "message": {
                        "content": {"parts": ["Hi there!"]},
                        "author": {"role": "assistant"},
                        "create_time": 1704067201,
                        "metadata": {"model_slug": "gpt-4"}
                    }
                }
            }
        }]"#;

        let result = parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "Test Chat");
        assert_eq!(result[0].messages.len(), 2);
        assert_eq!(result[0].messages[0].role, "user");
        assert_eq!(result[0].messages[0].content, "Hello!");
        assert_eq!(result[0].messages[1].role, "assistant");
        assert_eq!(result[0].messages[1].model, Some("gpt-4".to_string()));
    }
}
