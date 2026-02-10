"""MCP tools for distillation workflow and AI memory layer"""
from typing import List, Optional
from pydantic import BaseModel, Field

from app.models.distillation import (
    DistillMode,
    HubPolicy,
    CandidateAction,
)


class DistillNoteTool(BaseModel):
    """
    Distill a container note into atomic notes.
    
    This tool extracts key ideas from large notes (evidence, canvas exports, chat transcripts)
    and creates smaller, focused atomic notes linked back to the source.
    
    Use mode="suggest" first to get candidates, then mode="apply" with decisions.
    """
    
    note_id: str = Field(
        ...,
        description="ID of the container note to distill"
    )
    mode: DistillMode = Field(
        default=DistillMode.SUGGEST,
        description="'suggest' to extract candidates, 'apply' to create notes"
    )
    hub_policy: HubPolicy = Field(
        default=HubPolicy.AUTO,
        description="'auto' to create hubs automatically, 'manual' to only suggest"
    )
    min_score: float = Field(
        default=0.85,
        ge=0.0,
        le=1.0,
        description="Minimum similarity score for deduplication (0.85 recommended)"
    )


class NormalizeTagsTool(BaseModel):
    """
    Normalize tags for a note.
    
    - Lowercases all tags
    - Strips leading '#' symbols  
    - Converts spaces to hyphens
    - Merges inline #tags from body into YAML tags
    """
    
    note_id: str = Field(
        ...,
        description="ID of the note to normalize tags for"
    )


# ============================================================================
# Memory Layer MCP Tools
# ============================================================================


class MemoryRecallTool(BaseModel):
    """
    Recall relevant notes from the knowledge base using semantic search
    combined with graph context.

    Provide a natural-language query and optionally IDs of notes the user
    is currently viewing to boost graph-connected results.
    """

    query: str = Field(
        ...,
        description="Natural-language query to recall relevant notes"
    )
    context_note_ids: Optional[List[str]] = Field(
        default=None,
        description="IDs of notes the user is currently viewing (boosts graph neighbors)"
    )
    limit: int = Field(
        default=5,
        ge=1,
        le=50,
        description="Maximum number of results to return"
    )


class MemoryContradictionsTool(BaseModel):
    """
    Find potential contradictions for a given note.

    Detects notes with high semantic similarity but conflicting metadata
    (different status or disjoint tag sets).
    """

    note_id: str = Field(
        ...,
        description="ID of the note to check for contradictions"
    )


class MemoryExtractTool(BaseModel):
    """
    Extract note suggestions from a conversation.

    Parses assistant messages to identify substantive content and
    creates draft note suggestions with titles, tags, and content.
    """

    messages: List[dict] = Field(
        ...,
        description="List of chat messages with 'role' and 'content' keys"
    )
    source: Optional[str] = Field(
        default="conversation",
        description="Provenance label for extracted notes"
    )


# Tool descriptions for MCP discovery
MCP_TOOLS = {
    "distill_note": {
        "description": "Extract atomic notes from a container note. Use mode='suggest' first to preview, then mode='apply' to create.",
        "parameters": DistillNoteTool,
        "endpoint": "/api/notes/{note_id}/distill",
        "method": "POST"
    },
    "normalize_tags": {
        "description": "Normalize and merge inline #tags into YAML frontmatter tags.",
        "parameters": NormalizeTagsTool,
        "endpoint": "/api/notes/{note_id}/normalize-tags",
        "method": "POST"
    },
    "memory_recall": {
        "description": "Recall relevant notes using semantic search and graph context.",
        "parameters": MemoryRecallTool,
        "endpoint": "/api/memory/recall",
        "method": "POST"
    },
    "memory_contradictions": {
        "description": "Find potential contradictions for a given note (status/tag mismatches with similar notes).",
        "parameters": MemoryContradictionsTool,
        "endpoint": "/api/memory/contradictions/{note_id}",
        "method": "POST"
    },
    "memory_extract": {
        "description": "Extract draft note suggestions from a conversation's messages.",
        "parameters": MemoryExtractTool,
        "endpoint": "/api/memory/extract",
        "method": "POST"
    },
}
