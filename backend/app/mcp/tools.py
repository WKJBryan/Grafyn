"""MCP tools for distillation workflow"""
from typing import Optional
from pydantic import BaseModel, Field

from backend.app.models.distillation import (
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
    }
}
