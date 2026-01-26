"""Canvas data models for Multi-LLM comparison feature"""
from pydantic import BaseModel, Field
from typing import List, Optional, Dict, Any
from datetime import datetime, timezone
from enum import Enum


class DebateMode(str, Enum):
    """Debate mode options"""
    AUTO = "auto"
    MEDIATED = "mediated"


class ContextMode(str, Enum):
    """Context mode for prompts"""
    NONE = "none"  # No additional context
    FULL_HISTORY = "full_history"  # Walk parent chain, include all turns
    COMPACT = "compact"  # Recent turns verbatim + summary of older context
    SEMANTIC = "semantic"  # RAG-style search across notes and tiles


class NodeType(str, Enum):
    """Node types in the canvas graph"""
    PROMPT = "prompt"
    LLM_RESPONSE = "llm_response"
    DEBATE = "debate"


class TilePosition(BaseModel):
    """Position and size of a tile/node on the canvas"""
    x: float = 0.0
    y: float = 0.0
    width: float = 280.0  # Default for LLM nodes
    height: float = 200.0  # Default for LLM nodes


class ModelResponse(BaseModel):
    """Response from a single LLM model - rendered as individual node in canvas"""
    id: str
    model_id: str
    model_name: str
    content: str = ""
    status: str = "pending"  # pending, streaming, completed, error
    error_message: Optional[str] = None
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    completed_at: Optional[datetime] = None
    tokens_used: Optional[int] = None
    # Position for individual LLM node on canvas
    position: TilePosition = Field(default_factory=lambda: TilePosition(width=280, height=200))
    # Color for this specific model (assigned at creation)
    color: str = "#7c5cff"  # Default violet, will be assigned per-model


class PromptTile(BaseModel):
    """A prompt node and its responses from multiple models"""
    id: str
    prompt: str
    system_prompt: Optional[str] = None
    models: List[str]
    responses: Dict[str, ModelResponse] = Field(default_factory=dict)
    # Position for the prompt node itself (compact size)
    position: TilePosition = Field(default_factory=lambda: TilePosition(width=200, height=120))
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    # Branching/mind-map support
    parent_tile_id: Optional[str] = None  # ID of the parent tile this branches from
    parent_model_id: Optional[str] = None  # Which model's response this branches from


class DebateRound(BaseModel):
    """A debate node and its rounds between models"""
    id: str
    participating_models: List[str]
    debate_mode: DebateMode = DebateMode.AUTO
    source_tile_ids: List[str]
    rounds: List[Dict[str, str]] = Field(default_factory=list)
    status: str = "active"  # active, paused, completed
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    # Position for debate node (compact size)
    position: TilePosition = Field(default_factory=lambda: TilePosition(width=200, height=150))


class CanvasViewport(BaseModel):
    """Canvas viewport state"""
    x: float = 0.0
    y: float = 0.0
    zoom: float = 1.0


class CanvasSession(BaseModel):
    """Complete canvas session state"""
    id: str
    title: str = "Untitled Canvas"
    description: Optional[str] = None
    prompt_tiles: List[PromptTile] = Field(default_factory=list)
    debates: List[DebateRound] = Field(default_factory=list)
    viewport: CanvasViewport = Field(default_factory=CanvasViewport)
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    updated_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    tags: List[str] = Field(default_factory=list)
    status: str = "draft"
    linked_note_id: Optional[str] = None


class CanvasSessionListItem(BaseModel):
    """Summary of canvas session for list views"""
    id: str
    title: str
    description: Optional[str] = None
    tile_count: int = 0
    debate_count: int = 0
    created_at: datetime
    updated_at: datetime
    tags: List[str] = Field(default_factory=list)
    status: str = "draft"


class CanvasCreate(BaseModel):
    """Schema for creating a new canvas"""
    title: str = Field(default="Untitled Canvas", max_length=255)
    description: Optional[str] = None
    tags: List[str] = Field(default_factory=list)


class CanvasUpdate(BaseModel):
    """Schema for updating canvas metadata"""
    title: Optional[str] = Field(None, max_length=255)
    description: Optional[str] = None
    viewport: Optional[CanvasViewport] = None
    tags: Optional[List[str]] = None
    status: Optional[str] = Field(None, pattern="^(draft|evidence|canonical)$")


class PromptRequest(BaseModel):
    """Request to send a prompt to multiple models"""
    prompt: str = Field(..., min_length=1)
    system_prompt: Optional[str] = None
    models: List[str] = Field(..., min_length=1)
    temperature: float = Field(default=0.7, ge=0.0, le=2.0)
    max_tokens: int = Field(default=2048, ge=1, le=32000)
    # Branching support
    parent_tile_id: Optional[str] = None
    parent_model_id: Optional[str] = None
    # Context mode for conversation history (default to semantic for note lookup)
    context_mode: ContextMode = Field(default=ContextMode.SEMANTIC)


class DebateStartRequest(BaseModel):
    """Request to start a debate between models"""
    source_tile_ids: List[str] = Field(..., min_length=1)
    participating_models: List[str] = Field(..., min_length=2)
    debate_mode: DebateMode = DebateMode.AUTO
    debate_prompt: Optional[str] = None
    max_rounds: int = Field(default=3, ge=1, le=10)


class DebateContinueRequest(BaseModel):
    """Request to continue a debate with custom prompt"""
    prompt: str = Field(..., min_length=1)


class AddModelsRequest(BaseModel):
    """Request to add new models to an existing tile"""
    model_ids: List[str] = Field(..., min_length=1)


class ModelInfo(BaseModel):
    """Information about an available model"""
    id: str
    name: str
    provider: str
    context_length: int = 4096
    pricing: Dict[str, float] = Field(default_factory=dict)
    supports_streaming: bool = True


class TilePositionUpdate(BaseModel):
    """Update for tile position"""
    x: float
    y: float
    width: Optional[float] = None
    height: Optional[float] = None


class EdgeType(str, Enum):
    """Types of edges in the canvas graph"""
    PROMPT_TO_LLM = "prompt_to_llm"  # Prompt node → LLM response node
    LLM_TO_PROMPT = "llm_to_prompt"  # LLM response → branched prompt (for branches)
    DEBATE_TO_LLM = "debate_to_llm"  # Debate node → LLM response node


class TileEdge(BaseModel):
    """Edge connecting nodes in the canvas graph (legacy compatibility)"""
    source_tile_id: str  # Parent tile ID
    target_tile_id: str  # Child tile ID
    source_model_id: Optional[str] = None  # Which model response the child branches from


class NodeEdge(BaseModel):
    """Edge connecting any two nodes in the canvas graph"""
    source_id: str  # Source node ID (format: "prompt:{id}" or "llm:{tile_id}:{model_id}" or "debate:{id}")
    target_id: str  # Target node ID
    edge_type: EdgeType = EdgeType.PROMPT_TO_LLM
    # Optional styling
    color: Optional[str] = None


class ArrangeRequest(BaseModel):
    """Request to batch update node positions after auto-arrange"""
    positions: Dict[str, TilePositionUpdate]  # node_id -> new position

