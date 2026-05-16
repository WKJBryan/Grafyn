use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::models::twin::TwinContextRecord;

/// Canvas session containing prompts and model responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasSession {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub prompt_tiles: Vec<PromptTile>,
    #[serde(default)]
    pub debates: Vec<Debate>,
    #[serde(default)]
    pub viewport: CanvasViewport,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub pinned_note_ids: Vec<String>,
}

fn default_status() -> String {
    "draft".to_string()
}

impl Default for CanvasSession {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: "Untitled Session".to_string(),
            description: None,
            prompt_tiles: Vec::new(),
            debates: Vec::new(),
            viewport: CanvasViewport::default(),
            created_at: now,
            updated_at: now,
            tags: Vec::new(),
            status: "draft".to_string(),
            pinned_note_ids: Vec::new(),
        }
    }
}

/// Canvas viewport state for zoom/pan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasViewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl Default for CanvasViewport {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

/// A note used as context for a tile prompt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileContextNote {
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptType {
    #[default]
    Standard,
    Decision,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DecisionPromptMetadata {
    pub decision: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub stakes: Option<String>,
    #[serde(default)]
    pub initial_leaning: Option<String>,
    #[serde(default)]
    pub review_date: Option<String>,
}

/// A tile on the canvas containing a prompt and model responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTile {
    pub id: String,
    #[serde(default)]
    pub prompt_type: PromptType,
    pub prompt: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    pub models: Vec<String>,
    #[serde(default)]
    pub responses: HashMap<String, ModelResponse>,
    pub position: TilePosition,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub context_mode: ContextMode,
    #[serde(default)]
    pub parent_tile_id: Option<String>,
    #[serde(default)]
    pub parent_model_id: Option<String>,
    #[serde(default)]
    pub context_notes: Vec<TileContextNote>,
    #[serde(default)]
    pub approved_twin_records: Vec<TwinContextRecord>,
    #[serde(default)]
    pub candidate_twin_records: Vec<TwinContextRecord>,
    #[serde(default)]
    pub twin_answer_mode: TwinAnswerMode,
    #[serde(default)]
    pub twin_context_policy: Option<String>,
    #[serde(default)]
    pub twin_llm_provider: Option<String>,
    #[serde(default)]
    pub decision_metadata: Option<DecisionPromptMetadata>,
    #[serde(default)]
    pub decision_episode_id: Option<String>,
    #[serde(default)]
    pub web_search: bool,
    #[serde(default = "default_web_search_max_results")]
    pub web_search_max_results: u32,
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
}

impl Default for PromptTile {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            prompt_type: PromptType::default(),
            prompt: String::new(),
            system_prompt: None,
            models: Vec::new(),
            responses: HashMap::new(),
            position: TilePosition::default(),
            created_at: Utc::now(),
            context_mode: ContextMode::default(),
            parent_tile_id: None,
            parent_model_id: None,
            context_notes: Vec::new(),
            approved_twin_records: Vec::new(),
            candidate_twin_records: Vec::new(),
            twin_answer_mode: TwinAnswerMode::default(),
            twin_context_policy: None,
            twin_llm_provider: None,
            decision_metadata: None,
            decision_episode_id: None,
            web_search: false,
            web_search_max_results: default_web_search_max_results(),
            reasoning_effort: default_reasoning_effort(),
        }
    }
}

/// Position and size of a tile on the canvas
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TilePosition {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for TilePosition {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 300.0,
        }
    }
}

/// Context mode for branching conversations.
/// Values match what the frontend sends: none, full_history, compact, knowledge_search.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    #[default]
    None,
    FullHistory,
    Compact,
    KnowledgeSearch,
    Twin,
    /// Legacy alias for KnowledgeSearch — accept "semantic" from old saved sessions
    #[serde(rename = "semantic")]
    Semantic,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TwinAnswerMode {
    #[default]
    Advisor,
    Simulation,
}

/// Response from a single model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResponse {
    pub id: String,
    pub model_id: String,
    pub model_name: String,
    pub content: String,
    pub status: ResponseStatus,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub tokens_used: Option<u32>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub position: TilePosition,
}

impl Default for ModelResponse {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            model_id: String::new(),
            model_name: String::new(),
            content: String::new(),
            status: ResponseStatus::Pending,
            error: None,
            tokens_used: None,
            created_at: Utc::now(),
            position: TilePosition {
                x: 0.0,
                y: 0.0,
                width: 280.0,
                height: 200.0,
            },
        }
    }
}

/// Status of a model response
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ResponseStatus {
    #[default]
    Pending,
    Streaming,
    Completed,
    Error,
}

/// A debate between models (restructured from DebateRound)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Debate {
    pub id: String,
    #[serde(default)]
    pub participating_models: Vec<String>,
    #[serde(default)]
    pub source_tile_ids: Vec<String>,
    #[serde(default)]
    pub rounds: Vec<DebateRound>,
    #[serde(default = "default_debate_status")]
    pub status: String,
    #[serde(default)]
    pub position: TilePosition,
    #[serde(default)]
    pub debate_mode: String,
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
    pub created_at: DateTime<Utc>,
}

fn default_debate_status() -> String {
    "active".to_string()
}

impl Default for Debate {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            participating_models: Vec::new(),
            source_tile_ids: Vec::new(),
            rounds: Vec::new(),
            status: "active".to_string(),
            position: TilePosition::default(),
            debate_mode: "auto".to_string(),
            reasoning_effort: default_reasoning_effort(),
            created_at: Utc::now(),
        }
    }
}

/// A single round in a debate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateRound {
    pub round_number: u32,
    pub topic: String,
    pub responses: Vec<DebateResponse>,
    pub created_at: DateTime<Utc>,
}

/// A model's response in a debate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateResponse {
    pub model_id: String,
    pub model_name: String,
    pub content: String,
    pub stance: Option<String>,
}

/// Request to create a new session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCreate {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Request to update a session
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
    pub viewport: Option<CanvasViewport>,
    pub pinned_note_ids: Option<Vec<String>>,
}

/// Minimal session metadata for list views
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub tile_count: usize,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
    pub status: String,
}

impl From<&CanvasSession> for SessionMeta {
    fn from(session: &CanvasSession) -> Self {
        Self {
            id: session.id.clone(),
            title: session.title.clone(),
            description: session.description.clone(),
            tile_count: session.prompt_tiles.len(),
            created_at: session.created_at,
            updated_at: session.updated_at,
            tags: session.tags.clone(),
            status: session.status.clone(),
        }
    }
}

/// Request to send a prompt to models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptRequest {
    pub prompt: String,
    #[serde(default)]
    pub prompt_type: PromptType,
    #[serde(default)]
    pub system_prompt: Option<String>,
    pub models: Vec<String>,
    #[serde(default)]
    pub position: Option<TilePosition>,
    #[serde(default)]
    pub context_mode: ContextMode,
    #[serde(default)]
    pub twin_answer_mode: TwinAnswerMode,
    #[serde(default)]
    pub twin_context_policy: Option<String>,
    #[serde(default)]
    pub twin_llm_provider: Option<String>,
    #[serde(default)]
    pub decision_metadata: Option<DecisionPromptMetadata>,
    #[serde(default)]
    pub parent_tile_id: Option<String>,
    #[serde(default)]
    pub parent_model_id: Option<String>,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub web_search: bool,
    #[serde(default = "default_web_search_max_results")]
    pub web_search_max_results: u32,
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
}

fn default_temperature() -> f64 {
    0.7
}

fn default_web_search_max_results() -> u32 {
    5
}

fn default_reasoning_effort() -> String {
    "none".to_string()
}

/// Available LLM model information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableModel {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub context_length: Option<u32>,
    #[serde(default)]
    pub pricing: Option<ModelPricing>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prompt_request_defaults_web_search_max_results_when_omitted() {
        let request: PromptRequest = serde_json::from_value(json!({
            "prompt": "hello",
            "models": ["openai/gpt-4"]
        }))
        .unwrap();

        assert_eq!(request.web_search_max_results, 5);
        assert_eq!(request.max_tokens, None);
        assert_eq!(request.prompt_type, PromptType::Standard);
        assert_eq!(request.context_mode, ContextMode::None);
        assert_eq!(request.reasoning_effort, "none");
    }

    #[test]
    fn prompt_tile_defaults_web_search_max_results_when_omitted() {
        let tile: PromptTile = serde_json::from_value(json!({
            "id": "tile-1",
            "prompt": "hello",
            "models": ["openai/gpt-4"],
            "responses": {},
            "position": {
                "x": 0.0,
                "y": 0.0,
                "width": 400.0,
                "height": 300.0
            },
            "created_at": "2026-03-17T00:00:00Z"
        }))
        .unwrap();

        assert_eq!(tile.web_search_max_results, 5);
        assert_eq!(tile.twin_answer_mode, TwinAnswerMode::Advisor);
        assert_eq!(tile.prompt_type, PromptType::Standard);
        assert_eq!(tile.reasoning_effort, "none");
        assert!(tile.approved_twin_records.is_empty());
    }

    #[test]
    fn decision_prompt_request_deserializes_metadata() {
        let request: PromptRequest = serde_json::from_value(json!({
            "prompt": "Should I build Decision Mirror first?",
            "prompt_type": "decision",
            "models": ["openai/gpt-4"],
            "decision_metadata": {
                "decision": "Should I build Decision Mirror first?",
                "options": ["Decision Mirror", "Topology"],
                "stakes": "Product direction",
                "initial_leaning": "Decision Mirror",
                "review_date": "2026-05-15"
            }
        }))
        .unwrap();

        assert_eq!(request.prompt_type, PromptType::Decision);
        let metadata = request
            .decision_metadata
            .expect("decision metadata should deserialize");
        assert_eq!(metadata.options.len(), 2);
        assert_eq!(metadata.review_date.as_deref(), Some("2026-05-15"));
    }

    #[test]
    fn prompt_request_accepts_twin_context_mode_and_answer_mode() {
        let request: PromptRequest = serde_json::from_value(json!({
            "prompt": "What would my twin do?",
            "models": ["openai/gpt-4"],
            "context_mode": "twin",
            "twin_answer_mode": "simulation"
        }))
        .unwrap();

        assert_eq!(request.context_mode, ContextMode::Twin);
        assert_eq!(request.twin_answer_mode, TwinAnswerMode::Simulation);
    }
}

/// Model pricing information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub prompt: f64,
    pub completion: f64,
}

/// Update tile position request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TilePositionUpdate {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
}

/// Streaming events emitted to the Tauri frontend via window.emit()
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CanvasStreamEvent {
    TileCreated {
        session_id: String,
        tile: PromptTile,
    },
    Chunk {
        session_id: String,
        tile_id: String,
        model_id: String,
        chunk: String,
    },
    Complete {
        session_id: String,
        tile_id: String,
        model_id: String,
        tokens_used: Option<u32>,
    },
    Error {
        session_id: String,
        tile_id: String,
        model_id: String,
        error: String,
    },
    ModelsAdded {
        session_id: String,
        tile_id: String,
        responses: HashMap<String, ModelResponse>,
    },
    ContextNotes {
        session_id: String,
        tile_id: String,
        notes: Vec<TileContextNote>,
    },
    SessionSaved {
        session_id: String,
    },
    DebateCreated {
        session_id: String,
        debate: Debate,
    },
    RoundStart {
        session_id: String,
        debate_id: String,
        round_number: u32,
    },
    DebateChunk {
        session_id: String,
        debate_id: String,
        model_id: String,
        chunk: String,
        round_number: u32,
    },
    ModelComplete {
        session_id: String,
        debate_id: String,
        model_id: String,
        round_number: u32,
    },
    DebateError {
        session_id: String,
        debate_id: String,
        model_id: String,
        error: String,
        round_number: u32,
    },
    DebateComplete {
        session_id: String,
        debate_id: String,
    },
}

/// Request to start a debate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateStartRequest {
    pub source_tile_ids: Vec<String>,
    pub participating_models: Vec<String>,
    #[serde(default = "default_debate_mode")]
    pub debate_mode: String,
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u32,
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
}

fn default_debate_mode() -> String {
    "auto".to_string()
}

fn default_max_rounds() -> u32 {
    3
}

/// Request to continue a debate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateContinueRequest {
    pub prompt: String,
    #[serde(default = "default_reasoning_effort")]
    pub reasoning_effort: String,
}

/// Request to add models to an existing tile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddModelsRequest {
    pub model_ids: Vec<String>,
}

/// LLM node position update request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMNodePositionUpdate {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
}
