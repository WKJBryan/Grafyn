use anyhow::{anyhow, Result};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;

pub const DEFAULT_IMPORT_LINK_MODEL: &str = "qwen3.6:35b-a3b";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticLinkSuggestion {
    pub from_title: String,
    pub to_title: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SemanticLinkResponse {
    #[serde(default)]
    pub links: Vec<SemanticLinkSuggestion>,
    #[serde(default)]
    pub concerns: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    content: String,
}

pub fn parse_semantic_link_response(
    raw: &str,
    allowed_titles: &[String],
) -> Result<SemanticLinkResponse> {
    let value = serde_json::from_str::<Value>(raw.trim())
        .map_err(|e| anyhow!("Semantic link response was not valid JSON: {}", e))?;
    let parsed = serde_json::from_value::<SemanticLinkResponse>(value)
        .map_err(|e| anyhow!("Semantic link response had an invalid schema: {}", e))?;
    let allowed = allowed_titles.iter().cloned().collect::<HashSet<_>>();

    let mut seen = HashSet::new();
    let links = parsed
        .links
        .into_iter()
        .filter(|link| {
            allowed.contains(&link.from_title)
                && allowed.contains(&link.to_title)
                && link.from_title != link.to_title
                && seen.insert((link.from_title.clone(), link.to_title.clone()))
        })
        .collect();

    Ok(SemanticLinkResponse {
        links,
        concerns: parsed.concerns,
    })
}

pub fn is_loopback_ollama_url(base_url: &str) -> bool {
    let Ok(url) = Url::parse(base_url) else {
        return false;
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        return false;
    }
    matches!(
        url.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("::1") | Some("[::1]")
    )
}

pub async fn suggest_semantic_links(
    base_url: &str,
    model: &str,
    titled_sections: &[(String, String)],
) -> Result<SemanticLinkResponse> {
    if titled_sections.len() < 2 {
        return Ok(SemanticLinkResponse::default());
    }
    if !is_loopback_ollama_url(base_url) {
        return Err(anyhow!(
            "Semantic import linking only permits loopback Ollama endpoints"
        ));
    }

    let base = base_url.trim_end_matches('/');
    let endpoint = format!("{}/api/chat", base);
    let titles = titled_sections
        .iter()
        .map(|(title, _)| title.clone())
        .collect::<Vec<_>>();

    let section_prompt = titled_sections
        .iter()
        .map(|(title, content)| {
            let excerpt = content.chars().take(900).collect::<String>();
            format!("TITLE: {}\nEXCERPT:\n{}", title, excerpt)
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let user_prompt = format!(
        "Suggest semantic Obsidian wikilinks between these imported evidence notes.\n\
         Return ONLY JSON with this exact shape: {{\"links\":[{{\"from_title\":\"exact title\",\"to_title\":\"exact title\",\"reason\":\"short reason\"}}],\"concerns\":[]}}.\n\
         Link endpoints must exactly match one of these note titles: {}.\n\
         Do not invent titles, aliases, markdown links, or prose.\n\n{}",
        titles.join(" | "),
        section_prompt
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()?;
    let response = client
        .post(endpoint)
        .json(&serde_json::json!({
            "model": model,
            "stream": false,
            "format": "json",
            "think": false,
            "options": {
                "temperature": 0.1
            },
            "messages": [
                {
                    "role": "system",
                    "content": "You are a strict JSON semantic linker. You only use exact provided titles."
                },
                {
                    "role": "user",
                    "content": user_prompt
                }
            ]
        }))
        .send()
        .await?
        .error_for_status()?
        .json::<OllamaChatResponse>()
        .await?;

    parse_semantic_link_response(&response.message.content, &titles)
}
