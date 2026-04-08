//! Parser for Grok (xAI) conversation exports
//!
//! Grok exports use an "Enhanced Grok Export" format with a `meta` object
//! and `chats` array. Messages have `type` (prompt/response) and optional
//! `mode` (think/fun/deepsearch).

use crate::models::import::*;
use crate::services::import::parse_timestamp;
use anyhow::Result;

/// Check if JSON content matches Grok export format.
pub fn can_parse(content: &str) -> bool {
    let Ok(data) = serde_json::from_str::<serde_json::Value>(content) else {
        return false;
    };

    if let Some(obj) = data.as_object() {
        return obj
            .keys()
            .any(|k| matches!(k.as_str(), "meta" | "speaker_stats" | "grok_mode"));
    }

    false
}

/// Parse Grok conversations from JSON content.
pub fn parse(content: &str) -> Result<Vec<ParsedConversation>> {
    let data: serde_json::Value = serde_json::from_str(content)?;
    let mut conversations = Vec::new();

    // Enhanced Grok Export format (meta + chats)
    if data.get("meta").is_some() {
        if let Some(conv) = parse_enhanced(&data) {
            conversations.push(conv);
        }
    }
    // Array of conversations
    else if let Some(arr) = data.as_array() {
        for conv_data in arr {
            if let Some(conv) = parse_generic(conv_data) {
                conversations.push(conv);
            }
        }
    }

    log::info!("Parsed {} Grok conversations", conversations.len());
    Ok(conversations)
}

fn parse_enhanced(data: &serde_json::Value) -> Option<ParsedConversation> {
    let meta = data.get("meta")?;
    let title = meta
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Grok Conversation")
        .to_string();
    let exported_at = meta.get("exported_at");

    let chats = data.get("chats").and_then(|v| v.as_array())?;
    let messages = extract_enhanced_messages(chats);

    if messages.is_empty() {
        return None;
    }

    // Collect mode info (think, fun, deepsearch)
    let mut modes: Vec<String> = Vec::new();

    let created_at = parse_timestamp(exported_at);

    let mut tags = vec!["grok".to_string(), "import".to_string()];
    // Detect modes from raw chat data
    for chat in chats {
        if let Some(mode) = chat.get("mode").and_then(|v| v.as_str()) {
            let mode_tag = format!("{}-mode", mode);
            if !modes.contains(&mode.to_string()) {
                modes.push(mode.to_string());
            }
            if !tags.contains(&mode_tag) {
                tags.push(mode_tag);
            }
        }
    }
    tags.truncate(5);

    Some(ParsedConversation {
        id: format!(
            "grok_{}",
            exported_at.and_then(|v| v.as_str()).unwrap_or("unknown")
        ),
        title,
        platform: "grok".to_string(),
        metadata: ConversationMetadata {
            platform: "grok".to_string(),
            created_at,
            updated_at: created_at,
            message_count: messages.len(),
            model_info: vec!["grok".to_string()],
        },
        messages,
        suggested_tags: tags,
    })
}

fn extract_enhanced_messages(chats: &[serde_json::Value]) -> Vec<ParsedMessage> {
    let mut messages = Vec::new();

    for (i, chat) in chats.iter().enumerate() {
        let msg_type = chat.get("type").and_then(|v| v.as_str()).unwrap_or("");

        let role = match msg_type {
            "prompt" => "user",
            "response" => "assistant",
            _ => {
                if i % 2 == 0 {
                    "user"
                } else {
                    "assistant"
                }
            }
        };

        let content = extract_enhanced_content(chat.get("message"));
        if content.is_empty() {
            continue;
        }

        let timestamp = parse_timestamp(chat.get("timestamp"));

        messages.push(ParsedMessage {
            index: i,
            role: role.to_string(),
            content,
            timestamp,
            model: Some("grok".to_string()),
        });
    }

    messages
}

fn extract_enhanced_content(message_obj: Option<&serde_json::Value>) -> String {
    let Some(msg) = message_obj else {
        return String::new();
    };

    // Direct string
    if let Some(s) = msg.as_str() {
        return s.to_string();
    }

    if let Some(obj) = msg.as_object() {
        // Object with "data" field
        if let Some(data) = obj.get("data") {
            if let Some(s) = data.as_str() {
                return s.to_string();
            }
            if let Some(arr) = data.as_array() {
                let parts: Vec<String> = arr
                    .iter()
                    .filter_map(|part| {
                        if let Some(s) = part.as_str() {
                            return Some(s.to_string());
                        }
                        part.get("data")
                            .and_then(|d| d.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect();
                return parts.join("\n");
            }
        }
        // Direct content field
        if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
            return content.to_string();
        }
    }

    String::new()
}

fn parse_generic(data: &serde_json::Value) -> Option<ParsedConversation> {
    let conv_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let title = data
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Grok Conversation")
        .to_string();

    let messages_list = data
        .get("messages")
        .or_else(|| data.get("chats"))
        .and_then(|v| v.as_array())?;

    let mut messages = Vec::new();
    for (i, msg_data) in messages_list.iter().enumerate() {
        let role = msg_data
            .get("role")
            .and_then(|v| v.as_str())
            .or_else(|| {
                msg_data
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|t| match t {
                        "prompt" => "user",
                        "response" => "assistant",
                        _ => "user",
                    })
            })
            .unwrap_or(if i % 2 == 0 { "user" } else { "assistant" });

        let content = msg_data
            .get("content")
            .or_else(|| msg_data.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if content.is_empty() {
            continue;
        }

        messages.push(ParsedMessage {
            index: i,
            role: role.to_string(),
            content,
            timestamp: parse_timestamp(msg_data.get("timestamp")),
            model: Some("grok".to_string()),
        });
    }

    if messages.is_empty() {
        return None;
    }

    Some(ParsedConversation {
        id: conv_id,
        title,
        platform: "grok".to_string(),
        metadata: ConversationMetadata {
            platform: "grok".to_string(),
            created_at: parse_timestamp(data.get("created_at")),
            updated_at: parse_timestamp(data.get("updated_at")),
            message_count: messages.len(),
            model_info: vec!["grok".to_string()],
        },
        messages,
        suggested_tags: vec!["grok".to_string(), "import".to_string()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_parse_grok() {
        let content =
            r#"{"meta": {"title": "Test"}, "chats": [{"type": "prompt", "message": "hi"}]}"#;
        assert!(can_parse(content));
    }

    #[test]
    fn test_parse_enhanced_grok() {
        let content = r#"{
            "meta": {"title": "Grok Chat", "exported_at": "2024-01-01 12:00:00"},
            "chats": [
                {"index": 0, "type": "prompt", "message": "Hello Grok"},
                {"index": 1, "type": "response", "message": "Hello!", "mode": "think"}
            ]
        }"#;

        let result = parse(content).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "Grok Chat");
        assert_eq!(result[0].messages.len(), 2);
        assert_eq!(result[0].messages[0].role, "user");
        assert_eq!(result[0].messages[1].role, "assistant");
    }
}
