use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// A tile on the canvas containing a prompt and model responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTile {
    pub id: String,
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
}

impl Default for PromptTile {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
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
/// Values match what the frontend sends: none, full_history, compact, semantic.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    None,
    FullHistory,
    Compact,
    #[default]
    Semantic,
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
    pub system_prompt: Option<String>,
    pub models: Vec<String>,
    #[serde(default)]
    pub position: Option<TilePosition>,
    #[serde(default)]
    pub context_mode: ContextMode,
    #[serde(default)]
    pub parent_tile_id: Option<String>,
    #[serde(default)]
    pub parent_model_id: Option<String>,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_temperature() -> f64 {
    0.7
}

fn default_max_tokens() -> u32 {
    2048
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
