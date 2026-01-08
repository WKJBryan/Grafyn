"""Canvas data models for Multi-LLM comparison feature"""
from pydantic import BaseModel, Field
from typing import List, Optional, Dict, Any
from datetime import datetime, timezone
from enum import Enum


class DebateMode(str, Enum):
    """Debate mode options"""
    AUTO = "auto"
    MEDIATED = "mediated"


class TilePosition(BaseModel):
    """Position and size of a tile on the canvas"""
    x: float = 0.0
    y: float = 0.0
    width: float = 400.0
    height: float = 300.0


class ModelResponse(BaseModel):
    """Response from a single LLM model"""
    id: str
    model_id: str
    model_name: str
    content: str = ""
    status: str = "pending"  # pending, streaming, completed, error
    error_message: Optional[str] = None
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    completed_at: Optional[datetime] = None
    tokens_used: Optional[int] = None
    position: TilePosition = Field(default_factory=TilePosition)


class PromptTile(BaseModel):
    """A prompt and its responses from multiple models"""
    id: str
    prompt: str
    system_prompt: Optional[str] = None
    models: List[str]
    responses: Dict[str, ModelResponse] = Field(default_factory=dict)
    position: TilePosition = Field(default_factory=TilePosition)
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))


class DebateRound(BaseModel):
    """A round of debate between models"""
    id: str
    participating_models: List[str]
    debate_mode: DebateMode = DebateMode.AUTO
    source_tile_ids: List[str]
    rounds: List[Dict[str, str]] = Field(default_factory=list)
    status: str = "active"  # active, paused, completed
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))
    position: TilePosition = Field(default_factory=TilePosition)


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
