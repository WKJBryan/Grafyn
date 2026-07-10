use crate::models::canvas::{AvailableModel, ModelPricing};
use crate::services::utf8_chunk::Utf8ChunkBuffer;
use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const OPENROUTER_API_URL: &str = "https://openrouter.ai/api/v1";

/// Service for interacting with OpenRouter API
#[derive(Debug, Clone)]
pub struct OpenRouterService {
    client: Client,
    api_key: String,
}

impl OpenRouterService {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| Client::new()),
            api_key,
        }
    }

    /// Update the API key (called when user updates settings)
    pub fn set_api_key(&mut self, api_key: String) {
        self.api_key = api_key;
    }

    /// Get the current API key (masked for display)
    pub fn get_api_key_masked(&self) -> Option<String> {
        if self.api_key.is_empty() {
            None
        } else if self.api_key.len() <= 8 {
            Some("****".to_string())
        } else {
            Some(format!(
                "{}...{}",
                &self.api_key[..4],
                &self.api_key[self.api_key.len() - 4..]
            ))
        }
    }

    /// Check if the service is configured
    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    /// Get list of available models
    pub async fn get_available_models(&self) -> Result<Vec<AvailableModel>> {
        if !self.is_configured() {
            return Ok(get_default_models());
        }

        let response = self
            .client
            .get(format!("{}/models", OPENROUTER_API_URL))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .context("Failed to fetch models")?;

        if !response.status().is_success() {
            return Ok(get_default_models());
        }

        let models_response: ModelsResponse = response
            .json()
            .await
            .context("Failed to parse models response")?;

        let models = models_response
            .data
            .into_iter()
            .map(|m| AvailableModel {
                id: m.id.clone(),
                name: m.name.unwrap_or(m.id.clone()),
                provider: extract_provider(&m.id),
                description: m.description,
                context_length: m.context_length,
                pricing: m.pricing.map(|p| ModelPricing {
                    prompt: p.prompt.parse().unwrap_or(0.0),
                    completion: p.completion.parse().unwrap_or(0.0),
                }),
            })
            .collect();

        Ok(models)
    }

    /// Send a chat completion request (non-streaming)
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        system_prompt: Option<&str>,
        temperature: Option<f64>,
        max_tokens: Option<u32>,
        reasoning_effort: Option<&str>,
        web_search: bool,
        web_search_max_results: u32,
    ) -> Result<String> {
        if !self.is_configured() {
            return Err(anyhow::anyhow!("OpenRouter API key not configured"));
        }

        let mut all_messages = Vec::new();

        if let Some(system) = system_prompt {
            all_messages.push(ChatMessage {
                role: "system".to_string(),
                content: system.to_string(),
            });
        }

        all_messages.extend(messages);

        let plugins = build_web_plugins(web_search, web_search_max_results);
        let reasoning = build_reasoning(reasoning_effort);

        let request = ChatRequest {
            model: model.to_string(),
            messages: all_messages,
            stream: Some(false),
            temperature,
            max_tokens,
            reasoning,
            plugins,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", OPENROUTER_API_URL))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://grafyn.app")
            .header("X-Title", "Grafyn")
            .json(&request)
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .context("Failed to send chat request")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("OpenRouter API error: {}", error_text));
        }

        let chat_response: ChatResponse = response
            .json()
            .await
            .context("Failed to parse chat response")?;

        chat_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow::anyhow!("No response from model"))
    }

    /// Send a streaming chat completion request
    pub async fn chat_stream(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        system_prompt: Option<&str>,
        temperature: Option<f64>,
        max_tokens: Option<u32>,
        reasoning_effort: Option<&str>,
        web_search: bool,
        web_search_max_results: u32,
    ) -> Result<impl futures::Stream<Item = Result<String>>> {
        if !self.is_configured() {
            return Err(anyhow::anyhow!("OpenRouter API key not configured"));
        }

        let mut all_messages = Vec::new();

        if let Some(system) = system_prompt {
            all_messages.push(ChatMessage {
                role: "system".to_string(),
                content: system.to_string(),
            });
        }

        all_messages.extend(messages);

        let plugins = build_web_plugins(web_search, web_search_max_results);
        let reasoning = build_reasoning(reasoning_effort);

        let request = ChatRequest {
            model: model.to_string(),
            messages: all_messages,
            stream: Some(true),
            temperature,
            max_tokens,
            reasoning,
            plugins,
        };

        let response = self
            .client
            .post(format!("{}/chat/completions", OPENROUTER_API_URL))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("HTTP-Referer", "https://grafyn.app")
            .header("X-Title", "Grafyn")
            .json(&request)
            .send()
            .await
            .context("Failed to send streaming chat request")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("OpenRouter API error: {}", error_text));
        }

        // Buffer raw bytes and split on SSE event boundaries (\n\n) to prevent
        // content loss from TCP chunk splitting. Without this, partial JSON lines
        // silently fail at serde_json::from_str and tokens are dropped.
        //
        // Bytes are decoded via `Utf8ChunkBuffer` rather than
        // `String::from_utf8_lossy` per network chunk: a multibyte UTF-8
        // character split across two TCP chunks would otherwise be replaced
        // with U+FFFD before the continuation bytes ever arrive.
        let byte_stream = Box::pin(response.bytes_stream());

        let stream = futures::stream::unfold(
            (byte_stream, String::new(), Utf8ChunkBuffer::new()),
            |(mut inner, mut buffer, mut utf8_buffer)| async move {
                loop {
                    // Check for complete SSE event in buffer
                    if let Some(pos) = buffer.find("\n\n") {
                        let event = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();
                        return Some((parse_sse_chunk(&event), (inner, buffer, utf8_buffer)));
                    }

                    // Need more data from the byte stream
                    match inner.next().await {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&utf8_buffer.push(&bytes));
                        }
                        Some(Err(e)) => {
                            return Some((
                                Err(anyhow::anyhow!("Stream error: {}", e)),
                                (inner, buffer, utf8_buffer),
                            ));
                        }
                        None => {
                            // Stream ended — flush remaining buffer
                            buffer.push_str(&utf8_buffer.flush());
                            if !buffer.trim().is_empty() {
                                let remaining = std::mem::take(&mut buffer);
                                match parse_sse_chunk(&remaining) {
                                    Ok(content) if !content.is_empty() => {
                                        return Some((Ok(content), (inner, buffer, utf8_buffer)));
                                    }
                                    Err(error) => {
                                        return Some((Err(error), (inner, buffer, utf8_buffer)));
                                    }
                                    Ok(_) => {}
                                }
                            }
                            return None;
                        }
                    }
                }
            },
        );

        Ok(stream)
    }
}

fn build_web_plugins(web_search: bool, web_search_max_results: u32) -> Option<Vec<WebPlugin>> {
    if !web_search {
        return None;
    }

    Some(vec![WebPlugin {
        id: "web".to_string(),
        max_results: Some(web_search_max_results),
    }])
}

fn build_reasoning(reasoning_effort: Option<&str>) -> Option<ReasoningRequest> {
    match reasoning_effort {
        Some(effort @ ("minimal" | "low" | "medium" | "high" | "xhigh")) => {
            Some(ReasoningRequest {
                effort: effort.to_string(),
            })
        }
        _ => None,
    }
}

/// Parse SSE chunk to extract content.
///
/// OpenRouter can emit a mid-stream error as a `data:` payload shaped like
/// `{"error": {"message": ..., "code": ...}}` instead of the normal
/// `StreamChunk` shape. That payload previously failed to parse as
/// `StreamChunk` and was silently ignored, so a truncated response was
/// classified as a normal `Completed` stream with no indication anything
/// went wrong. Such payloads are now detected and surfaced as an `Err` so
/// the caller can mark the response as errored instead.
fn parse_sse_chunk(chunk: &str) -> Result<String> {
    let mut content = String::new();

    for line in chunk.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if data == "[DONE]" {
                continue;
            }

            if let Ok(parsed) = serde_json::from_str::<StreamChunk>(data) {
                if let Some(choice) = parsed.choices.first() {
                    if let Some(delta_content) = &choice.delta.content {
                        content.push_str(delta_content);
                    }
                }
                continue;
            }

            if let Ok(error_payload) = serde_json::from_str::<StreamErrorPayload>(data) {
                let message = error_payload
                    .error
                    .message
                    .unwrap_or_else(|| "Unknown streaming error".to_string());
                return Err(anyhow::anyhow!("OpenRouter stream error: {}", message));
            }
        }
    }

    Ok(content)
}

/// Extract provider name from model ID (e.g., "openai/gpt-4" -> "OpenAI")
fn extract_provider(model_id: &str) -> String {
    model_id
        .split('/')
        .next()
        .map(|p| {
            // Capitalize provider name
            let mut chars = p.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => p.to_string(),
            }
        })
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Get default model list when API is not available
fn get_default_models() -> Vec<AvailableModel> {
    vec![
        AvailableModel {
            id: "openai/gpt-4o".to_string(),
            name: "GPT-4o".to_string(),
            provider: "OpenAI".to_string(),
            description: Some("Most capable GPT-4 model".to_string()),
            context_length: Some(128000),
            pricing: None,
        },
        AvailableModel {
            id: "openai/gpt-4o-mini".to_string(),
            name: "GPT-4o Mini".to_string(),
            provider: "OpenAI".to_string(),
            description: Some("Fast and affordable GPT-4".to_string()),
            context_length: Some(128000),
            pricing: None,
        },
        AvailableModel {
            id: "anthropic/claude-3.5-sonnet".to_string(),
            name: "Claude 3.5 Sonnet".to_string(),
            provider: "Anthropic".to_string(),
            description: Some("Balanced performance and speed".to_string()),
            context_length: Some(200000),
            pricing: None,
        },
        AvailableModel {
            id: "anthropic/claude-3-opus".to_string(),
            name: "Claude 3 Opus".to_string(),
            provider: "Anthropic".to_string(),
            description: Some("Most capable Claude model".to_string()),
            context_length: Some(200000),
            pricing: None,
        },
        AvailableModel {
            id: "google/gemini-pro-1.5".to_string(),
            name: "Gemini Pro 1.5".to_string(),
            provider: "Google".to_string(),
            description: Some("Google's latest model".to_string()),
            context_length: Some(1000000),
            pricing: None,
        },
        AvailableModel {
            id: "meta-llama/llama-3.1-70b-instruct".to_string(),
            name: "Llama 3.1 70B".to_string(),
            provider: "Meta".to_string(),
            description: Some("Open source large model".to_string()),
            context_length: Some(131072),
            pricing: None,
        },
    ]
}

// API request/response types

#[derive(Debug, Serialize)]
struct WebPlugin {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_results: Option<u32>,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugins: Option<Vec<WebPlugin>>,
}

#[derive(Debug, Serialize)]
struct ReasoningRequest {
    effort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

/// Shape of a mid-stream SSE error payload, e.g.
/// `{"error": {"message": "rate limited", "code": 429}}`.
#[derive(Debug, Deserialize)]
struct StreamErrorPayload {
    error: StreamErrorDetail,
}

#[derive(Debug, Deserialize)]
struct StreamErrorDetail {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    code: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Debug, Deserialize)]
struct ModelInfo {
    id: String,
    name: Option<String>,
    description: Option<String>,
    context_length: Option<u32>,
    pricing: Option<PricingInfo>,
}

#[derive(Debug, Deserialize)]
struct PricingInfo {
    prompt: String,
    completion: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_serializes_reasoning_effort_without_max_tokens() {
        let request = ChatRequest {
            model: "openai/gpt-5".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "Think carefully".to_string(),
            }],
            stream: Some(true),
            temperature: Some(0.7),
            max_tokens: None,
            reasoning: Some(ReasoningRequest {
                effort: "high".to_string(),
            }),
            plugins: None,
        };

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(value["reasoning"]["effort"], "high");
        assert!(value.get("max_tokens").is_none());
    }

    #[test]
    fn parse_sse_chunk_extracts_content_from_well_formed_delta() {
        let chunk = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        let result = parse_sse_chunk(chunk).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn parse_sse_chunk_ignores_done_marker() {
        let chunk = "data: [DONE]";
        let result = parse_sse_chunk(chunk).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn parse_sse_chunk_surfaces_mid_stream_error_instead_of_dropping_it() {
        let chunk = r#"data: {"error":{"message":"rate limited","code":429}}"#;
        let result = parse_sse_chunk(chunk);
        assert!(
            result.is_err(),
            "mid-stream error payload must surface as Err, not be silently dropped"
        );
        assert!(result.unwrap_err().to_string().contains("rate limited"));
    }

    #[test]
    fn parse_sse_chunk_surfaces_error_with_missing_message() {
        let chunk = r#"data: {"error":{"code":500}}"#;
        let result = parse_sse_chunk(chunk);
        assert!(result.is_err());
    }
}
