pub mod chatgpt;
pub mod claude;
pub mod document;
pub mod gemini;
pub mod grok;
pub mod semantic_links;
pub mod transcript;

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
    if transcript::can_parse(content) {
        return transcript::parse(content).context("Transcript parser failed");
    }

    Err(anyhow::anyhow!(
        "Could not detect conversation format. Supported: ChatGPT, Claude, Grok, Gemini, labeled transcript"
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
    if transcript::can_parse(content) {
        return Some("interview");
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
            "interviewee" => "Interviewee",
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
    fn test_detect_labeled_interview_transcript() {
        let content = "Interviewer: How do you decide what to trust?\nParticipant: I need to see a real demo first.";
        assert_eq!(detect_platform(content), Some("interview"));
        let parsed = parse_content(content).expect("transcript should parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].platform, "interview");
        assert_eq!(parsed[0].messages[0].role, "user");
        assert_eq!(parsed[0].messages[1].role, "interviewee");
    }

    #[test]
    fn test_detect_expert_labeled_interview_transcript() {
        let content =
            "Interviewer: How do you decide what to trust?\nExpert: I need to see a real demo first.";
        assert_eq!(detect_platform(content), Some("interview"));
        let parsed = parse_content(content).expect("expert transcript should parse");
        assert_eq!(parsed[0].messages[0].role, "user");
        assert_eq!(parsed[0].messages[1].role, "interviewee");
    }

    #[test]
    fn test_unlabeled_transcript_does_not_parse_for_extraction() {
        let content = "How do you decide what to trust?\nI need to see a real demo first.";
        assert_eq!(detect_platform(content), None);
        assert!(parse_content(content).is_err());
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

    #[test]
    fn document_import_splits_docx_source_pages_and_adds_structural_wikilinks() {
        let content = "\
https://www.sutd.edu.sg/media-releases-listing/sutd-pivots-50m-investment-worlds-first-design-ai-university/
SUTD Pivots Towards Artificial Intelligence With $50M Investment, Becoming World's First Design AI University
A total of $50 million worth of Design AI initiatives will be rolled out.
AI should be viewed as a partner rather than a tool.
https://www.sutd.edu.sg/about/design-ai/
SUTD is the world's first Design AI university.
Design AI expands human-machine collaboration across education and research.";

        let batch = document::parse_document_text("1. About SUTD Design AI.docx", "docx", content)
            .expect("document should parse");

        assert_eq!(batch.items.len(), 3);
        assert_eq!(batch.items[0].content_kind, "document_index");
        assert!(batch.items[0].content.contains("[[SUTD Pivots Towards Artificial Intelligence With $50M Investment, Becoming World's First Design AI University]]"));
        assert!(batch.items[0]
            .content
            .contains("[[SUTD is the world's first Design AI university.]]"));
        assert_eq!(batch.items[1].content_kind, "document_section");
        assert!(batch.items[1]
            .content
            .contains("Part of: [[1. About SUTD Design AI]]"));
        assert!(batch.items[1]
            .content
            .contains("Next: [[SUTD is the world's first Design AI university.]]"));
        assert_eq!(
            batch.items[1].metadata.get("source_url").and_then(|v| v.as_str()),
            Some("https://www.sutd.edu.sg/media-releases-listing/sutd-pivots-50m-investment-worlds-first-design-ai-university/")
        );
    }

    #[test]
    fn document_import_uses_pdf_headings_and_excludes_sources_tail() {
        let content = "\
Mr. Poon King Wang: A Decade of Professional Perspectives (2015-2025)
Decision-Making Style
Mr. Poon makes interdisciplinary and data-informed decisions.
Core Values and Priorities
He values human dignity and lifelong learning.
Evolution of Thinking (2015-2025)
His thinking evolved from smart cities to Design AI.
Perspective on Artificial Intelligence (AI)
AI should augment people and preserve trust.
Views on the Future of Work
Work will change through task redesign and reskilling.
Views on the Future of Education
Education should integrate AI as a co-creator.
Sources:
Helping Workers Weather Crisis and
Disruption: A Task Approach for Designing a New Future of Work";

        let batch = document::parse_document_text("Poon perspectives.pdf", "pdf", content)
            .expect("pdf text should parse");
        let titles = batch
            .items
            .iter()
            .map(|item| item.title.as_str())
            .collect::<Vec<_>>();

        assert!(titles.contains(&"Decision-Making Style"));
        assert!(titles.contains(&"Core Values and Priorities"));
        assert!(titles.contains(&"Evolution of Thinking (2015-2025)"));
        assert!(titles.contains(&"Perspective on Artificial Intelligence (AI)"));
        assert!(titles.contains(&"Views on the Future of Work"));
        assert!(titles.contains(&"Views on the Future of Education"));
        assert!(!titles.contains(&"Helping Workers Weather Crisis and"));
    }

    #[test]
    fn semantic_link_response_keeps_only_exact_known_titles() {
        let allowed = vec![
            "SUTD Pivots Towards Artificial Intelligence".to_string(),
            "Perspective on Artificial Intelligence (AI)".to_string(),
        ];
        let raw = r#"{
            "links": [
                {
                    "from_title": "SUTD Pivots Towards Artificial Intelligence",
                    "to_title": "Perspective on Artificial Intelligence (AI)",
                    "reason": "Both describe AI as a partner."
                },
                {
                    "from_title": "Made Up",
                    "to_title": "Perspective on Artificial Intelligence (AI)",
                    "reason": "Invalid endpoint."
                }
            ],
            "concerns": []
        }"#;

        let parsed = semantic_links::parse_semantic_link_response(raw, &allowed)
            .expect("valid JSON should parse");

        assert_eq!(parsed.links.len(), 1);
        assert_eq!(
            parsed.links[0].from_title,
            "SUTD Pivots Towards Artificial Intelligence"
        );
    }
}
