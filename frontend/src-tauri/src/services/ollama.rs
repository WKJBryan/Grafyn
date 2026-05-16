use crate::models::canvas::AvailableModel;
use crate::services::openrouter::ChatMessage;
use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct OllamaService {
    client: Client,
    base_url: String,
}

impl OllamaService {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .build()
                .unwrap_or_else(|_| Client::new()),
            base_url: normalize_base_url(&base_url),
        }
    }

    pub fn set_base_url(&mut self, base_url: String) {
        self.base_url = normalize_base_url(&base_url);
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn list_models(&self) -> Result<Vec<AvailableModel>> {
        let response = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .context("Failed to reach Ollama. Is Ollama running locally?")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Ollama returned HTTP {} while listing models",
                response.status()
            ));
        }

        let tags: OllamaTagsResponse = response
            .json()
            .await
            .context("Failed to parse Ollama model list")?;

        Ok(tags
            .models
            .into_iter()
            .map(|model| AvailableModel {
                id: model.name.clone(),
                name: model.name,
                provider: "Ollama".to_string(),
                description: model.details.and_then(|details| details.display()),
                context_length: None,
                pricing: None,
            })
            .collect())
    }

    pub async fn status(&self, selected_model: Option<&str>) -> Result<OllamaStatus> {
        let models = self.list_models().await?;
        let selected_model_available = selected_model
            .filter(|model| !model.trim().is_empty())
            .map(|selected| models.iter().any(|model| model.id == selected))
            .unwrap_or(false);

        Ok(OllamaStatus {
            available: true,
            base_url: self.base_url.clone(),
            model_count: models.len(),
            selected_model_available,
        })
    }

    pub async fn chat_stream(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        system_prompt: Option<&str>,
        temperature: Option<f64>,
    ) -> Result<impl futures::Stream<Item = Result<String>>> {
        if model.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "Select an Ollama model for local vault/twin responses before sending vault context"
            ));
        }

        let mut all_messages = Vec::new();
        if let Some(system) = system_prompt {
            all_messages.push(ChatMessage {
                role: "system".to_string(),
                content: system.to_string(),
            });
        }
        all_messages.extend(messages);

        let request = OllamaChatRequest {
            model: model.to_string(),
            messages: all_messages,
            stream: true,
            options: temperature.map(|temperature| OllamaOptions { temperature }),
        };

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&request)
            .send()
            .await
            .context("Failed to reach Ollama. Is Ollama running locally?")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Ollama API error ({}): {}",
                status,
                error_text
            ));
        }

        let byte_stream = Box::pin(response.bytes_stream());
        let stream = futures::stream::unfold(
            (byte_stream, String::new()),
            |(mut inner, mut buffer)| async move {
                loop {
                    if let Some(pos) = buffer.find('\n') {
                        let line = buffer[..pos].to_string();
                        buffer = buffer[pos + 1..].to_string();
                        return Some((parse_ollama_line(&line), (inner, buffer)));
                    }

                    match inner.next().await {
                        Some(Ok(bytes)) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                        }
                        Some(Err(error)) => {
                            return Some((
                                Err(anyhow::anyhow!("Ollama stream error: {}", error)),
                                (inner, buffer),
                            ));
                        }
                        None => {
                            if !buffer.trim().is_empty() {
                                let remaining = std::mem::take(&mut buffer);
                                return Some((parse_ollama_line(&remaining), (inner, buffer)));
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

#[derive(Debug, Clone, Serialize)]
pub struct OllamaStatus {
    pub available: bool,
    pub base_url: String,
    pub model_count: usize,
    pub selected_model_available: bool,
}

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    temperature: f64,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModelInfo>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelInfo {
    name: String,
    details: Option<OllamaModelDetails>,
}

#[derive(Debug, Deserialize)]
struct OllamaModelDetails {
    family: Option<String>,
    parameter_size: Option<String>,
    quantization_level: Option<String>,
}

impl OllamaModelDetails {
    fn display(self) -> Option<String> {
        let parts = [self.family, self.parameter_size, self.quantization_level]
            .into_iter()
            .flatten()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>();

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        }
    }
}

#[derive(Debug, Deserialize)]
struct OllamaStreamLine {
    message: Option<ChatMessage>,
    #[serde(default)]
    done: bool,
    error: Option<String>,
}

fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        "http://localhost:11434".to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_ollama_line(line: &str) -> Result<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    let parsed: OllamaStreamLine =
        serde_json::from_str(trimmed).context("Failed to parse Ollama stream line")?;
    if let Some(error) = parsed.error {
        return Err(anyhow::anyhow!("Ollama stream error: {}", error));
    }
    if parsed.done {
        return Ok(String::new());
    }

    Ok(parsed
        .message
        .map(|message| message.content)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_serializes_existing_chat_messages_for_streaming() {
        let request = OllamaChatRequest {
            model: "llama3.1:8b".to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: "Use only local twin context.".to_string(),
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: "What should I do?".to_string(),
                },
            ],
            stream: true,
            options: None,
        };

        let value = serde_json::to_value(request).unwrap();

        assert_eq!(value["model"], "llama3.1:8b");
        assert_eq!(value["stream"], true);
        assert_eq!(value["messages"][0]["role"], "system");
        assert_eq!(value["messages"][1]["content"], "What should I do?");
    }
}
