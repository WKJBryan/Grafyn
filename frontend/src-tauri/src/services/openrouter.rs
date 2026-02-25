use crate::models::canvas::{AvailableModel, ModelPricing};
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
            Some(format!("{}...{}", &self.api_key[..4], &self.api_key[self.api_key.len()-4..]))
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

        let request = ChatRequest {
            model: model.to_string(),
            messages: all_messages,
            stream: Some(false),
            temperature,
            max_tokens,
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

        let request = ChatRequest {
            model: model.to_string(),
            messages: all_messages,
            stream: Some(true),
            temperature,
            max_tokens,
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
        let byte_stream = Box::pin(response.bytes_stream());

        let stream = futures::stream::unfold(
            (byte_stream, String::new()),
            |(mut inner, mut buffer)| async move {
                loop {
                    // Check for complete SSE event in buffer
                    if let Some(pos) = buffer.find("\n\n") {
                        let event = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();
                        return Some((parse_sse_chunk(&event), (inner, buffer)));
                    }

                    // Need more data from the byte stream
                    match inner.next().await {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        Some(Err(e)) => {
                            return Some((
                                Err(anyhow::anyhow!("Stream error: {}", e)),
                                (inner, buffer),
                            ));
                        }
                        None => {
                            // Stream ended — flush remaining buffer
                            if !buffer.trim().is_empty() {
                                let remaining = std::mem::take(&mut buffer);
                                if let Ok(content) = parse_sse_chunk(&remaining) {
                                    if !content.is_empty() {
                                        return Some((Ok(content), (inner, buffer)));
                                    }
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

/// Parse SSE chunk to extract content
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
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
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
