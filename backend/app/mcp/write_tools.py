"""MCP write tools for Seedream - define schemas for write operations"""
from pydantic import BaseModel, Field
from typing import Optional, List
from enum import Enum


class ContentType(str, Enum):
    """Content type for priority scoring"""
    claim = "claim"
    decision = "decision"
    insight = "insight"
    question = "question"
    evidence = "evidence"
    general = "general"


class NoteType(str, Enum):
    """Note type for knowledge hierarchy"""
    container = "container"
    atomic = "atomic"
    hub = "hub"
    general = "general"


class PropertyType(str, Enum):
    """Typed property types"""
    string = "string"
    number = "number"
    date = "date"
    boolean = "boolean"
    link = "link"


class CreateNoteRequest(BaseModel):
    """Request schema for creating a note via MCP"""
    title: str = Field(
        ...,
        min_length=1,
        max_length=255,
        description="Title of the note (will be slugified to create the ID)"
    )
    content: str = Field(
        "",
        description="Markdown content of the note body"
    )
    tags: List[str] = Field(
        default_factory=list,
        description="List of tags to associate with the note"
    )
    status: str = Field(
        "draft",
        pattern="^(draft|evidence|canonical)$",
        description="Note status: draft (in progress), evidence (source material), or canonical (refined)"
    )
    content_type: str = Field(
        "general",
        description="Content type for priority scoring (claim/decision/insight/question/evidence/general)"
    )
    note_type: str = Field(
        "general",
        description="Note type for knowledge hierarchy (container/atomic/hub/general)"
    )
    properties: Optional[dict] = Field(
        None,
        description="Additional typed properties as key-value pairs"
    )


class UpdateNoteRequest(BaseModel):
    """Request schema for updating a note via MCP"""
    note_id: str = Field(
        ...,
        description="ID of the note to update (filename without .md)"
    )
    title: Optional[str] = Field(
        None,
        min_length=1,
        max_length=255,
        description="New title for the note"
    )
    content: Optional[str] = Field(
        None,
        description="New content to append or replace. Use 'mode' to control behavior."
    )
    content_mode: str = Field(
        "replace",
        pattern="^(replace|append|prepend)$",
        description="How to apply content: replace (overwrite), append (add to end), prepend (add to start)"
    )
    tags: Optional[List[str]] = Field(
        None,
        description="Tags to set on the note (replaces existing tags)"
    )
    tags_mode: str = Field(
        "replace",
        pattern="^(replace|merge|remove)$",
        description="How to apply tags: replace (overwrite), merge (combine), remove (subtract)"
    )
    status: Optional[str] = Field(
        None,
        pattern="^(draft|evidence|canonical)$",
        description="Update note status"
    )


class SetPropertyRequest(BaseModel):
    """Request schema for setting a property on a note via MCP"""
    note_id: str = Field(
        ...,
        description="ID of the note to update"
    )
    property_name: str = Field(
        ...,
        description="Name of the property to set"
    )
    property_type: PropertyType = Field(
        ...,
        description="Type of the property value"
    )
    value: str = Field(
        ...,
        description="Property value (will be parsed according to type)"
    )
    label: Optional[str] = Field(
        None,
        description="Optional human-readable label for the property"
    )


class FindOrCreateNoteRequest(BaseModel):
    """Request schema for finding or creating a note via MCP"""
    search_query: str = Field(
        ...,
        description="Query to search for existing notes"
    )
    title: str = Field(
        ...,
        description="Title to use if creating a new note"
    )
    content: str = Field(
        "",
        description="Content to use if creating a new note"
    )
    threshold: float = Field(
        0.75,
        ge=0.0,
        le=1.0,
        description="Minimum similarity score to consider a note a match (0.75 = 75% similar)"
    )
    tags: List[str] = Field(
        default_factory=list,
        description="Tags to apply to new note if created"
    )


