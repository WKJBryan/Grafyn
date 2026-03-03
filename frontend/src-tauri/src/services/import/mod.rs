pub mod chatgpt;
pub mod claude;
pub mod gemini;
pub mod grok;

use crate::models::import::ParsedConversation;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

/// Auto-detect format and parse conversations from JSON content.
pub fn parse_content(content: &str) -> Result<Vec<ParsedConversation>> {
    if chatgpt::can_parse(content) {
        return chatgpt::parse(content).context("ChatGPT parser failed");
    }
    if claude::can_parse(content) {
        return claude::parse(content).context("Claude parser failed");
    }
    if grok::can_parse(content) {
        return grok::parse(content).context("Grok parser failed");
    }
    if gemini::can_parse(content) {
        return gemini::parse(content).context("Gemini parser failed");
    }

    Err(anyhow::anyhow!(
        "Could not detect conversation format. Supported: ChatGPT, Claude, Grok, Gemini"
    ))
}

/// Detect the platform without fully parsing.
pub fn detect_platform(content: &str) -> Option<&'static str> {
    if chatgpt::can_parse(content) {
        return Some("chatgpt");
    }
    if claude::can_parse(content) {
        return Some("claude");
    }
    if grok::can_parse(content) {
        return Some("grok");
    }
    if gemini::can_parse(content) {
        return Some("gemini");
    }
    None
}

/// Format a parsed conversation as a markdown container note.
pub fn format_as_markdown(conv: &ParsedConversation) -> String {
    let mut md = format!("# {}\n\n", conv.title);
    md.push_str(&format!(
        "*Imported from {} on {}*\n\n",
        conv.platform,
        Utc::now().format("%Y-%m-%d %H:%M")
    ));

    if !conv.metadata.model_info.is_empty() {
        md.push_str(&format!(
            "**Models:** {}\n\n",
            conv.metadata.model_info.join(", ")
        ));
    }

    md.push_str("## Conversation History\n\n");

    for msg in &conv.messages {
        let role_label = match msg.role.as_str() {
            "user" => "User",
            "assistant" => "Assistant",
            "system" => "System",
            _ => "Unknown",
        };

        let model_suffix = msg
            .model
            .as_ref()
            .map(|m| format!(" ({})", m))
            .unwrap_or_default();

        md.push_str(&format!(
            "### Message {}: {}{}\n\n",
            msg.index + 1,
            role_label,
            model_suffix
        ));
        md.push_str(&msg.content);
        md.push_str("\n\n");
    }

    md
}

// ── Shared timestamp parsing ──────────────────────────────────────────────

/// Parse a serde_json Value as a timestamp (handles Unix float, ISO 8601, and date strings).
pub fn parse_timestamp(value: Option<&serde_json::Value>) -> Option<DateTime<Utc>> {
    let val = value?;

    // Unix timestamp (seconds, possibly fractional)
    if let Some(ts) = val.as_f64() {
        let secs = ts as i64;
        let nanos = ((ts.fract()) * 1e9) as u32;
        return DateTime::from_timestamp(secs, nanos);
    }

    // String timestamp
    if let Some(s) = val.as_str() {
        // Try RFC 3339 / ISO 8601
        let normalized = s.replace('Z', "+00:00");
        if let Ok(dt) = DateTime::parse_from_rfc3339(&normalized) {
            return Some(dt.with_timezone(&Utc));
        }
        // Try common date formats
        let cleaned = s.replace('T', " ").replace('Z', "");
        for fmt in &["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%d"] {
            if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&cleaned, fmt) {
                return Some(naive.and_utc());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_chatgpt() {
        let content = r#"[{"id": "abc", "title": "Test", "create_time": 1234567890, "mapping": {"node1": {"message": null}}}]"#;
        assert_eq!(detect_platform(content), Some("chatgpt"));
    }

    #[test]
    fn test_detect_claude() {
        let content = r#"[{"uuid": "abc", "name": "Test", "chat_messages": []}]"#;
        assert_eq!(detect_platform(content), Some("claude"));
    }

    #[test]
    fn test_detect_grok() {
        let content = r#"{"meta": {"title": "Test"}, "chats": []}"#;
        assert_eq!(detect_platform(content), Some("grok"));
    }

    #[test]
    fn test_detect_gemini() {
        let content = r#"{"project": "Test", "messages": [{"role": "user", "content": "hi"}]}"#;
        assert_eq!(detect_platform(content), Some("gemini"));
    }

    #[test]
    fn test_detect_unknown() {
        let content = r#"{"random": "data"}"#;
        assert_eq!(detect_platform(content), None);
    }

    #[test]
    fn test_parse_timestamp_unix() {
        let val = serde_json::json!(1704067200.0); // 2024-01-01 00:00:00 UTC
        let dt = parse_timestamp(Some(&val)).unwrap();
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2024-01-01");
    }

    #[test]
    fn test_parse_timestamp_iso() {
        let val = serde_json::json!("2024-01-01T12:00:00Z");
        let dt = parse_timestamp(Some(&val)).unwrap();
        assert_eq!(dt.format("%Y-%m-%d %H:%M").to_string(), "2024-01-01 12:00");
    }

    #[test]
    fn test_parse_timestamp_date_string() {
        let val = serde_json::json!("2024-01-01 14:30:00");
        let dt = parse_timestamp(Some(&val)).unwrap();
        assert_eq!(dt.format("%H:%M").to_string(), "14:30");
    }

    #[test]
    fn test_format_as_markdown() {
        use crate::models::import::*;
        let conv = ParsedConversation {
            id: "test".to_string(),
            title: "Test Conv".to_string(),
            platform: "chatgpt".to_string(),
            messages: vec![
                ParsedMessage {
                    index: 0,
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                    timestamp: None,
                    model: None,
                },
                ParsedMessage {
                    index: 1,
                    role: "assistant".to_string(),
                    content: "Hi there!".to_string(),
                    timestamp: None,
                    model: Some("gpt-4".to_string()),
                },
            ],
            metadata: ConversationMetadata {
                platform: "chatgpt".to_string(),
                created_at: None,
                updated_at: None,
                message_count: 2,
                model_info: vec!["gpt-4".to_string()],
            },
            suggested_tags: vec![],
        };
        let md = format_as_markdown(&conv);
        assert!(md.contains("# Test Conv"));
        assert!(md.contains("### Message 1: User"));
        assert!(md.contains("### Message 2: Assistant (gpt-4)"));
        assert!(md.contains("Hello"));
        assert!(md.contains("Hi there!"));
    }
}
