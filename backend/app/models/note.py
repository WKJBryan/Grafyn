"""Pydantic models for notes and related entities"""
from pydantic import BaseModel, Field
from typing import List, Optional
from datetime import datetime


class NoteFrontmatter(BaseModel):
    """YAML frontmatter metadata for a note"""
    title: str
    created: Optional[datetime] = None
    modified: Optional[datetime] = None
    tags: List[str] = Field(default_factory=list)
    status: str = "draft"
    aliases: List[str] = Field(default_factory=list)


class Note(BaseModel):
    """Complete note with content and metadata"""
    id: str
    title: str
    content: str
    frontmatter: NoteFrontmatter
    outgoing_links: List[str] = Field(default_factory=list)
    backlinks: List[str] = Field(default_factory=list)


class NoteCreate(BaseModel):
    """Schema for creating a new note"""
    title: str = Field(..., min_length=1, max_length=255)
    content: str = ""
    tags: List[str] = Field(default_factory=list)
    status: str = Field(default="draft", pattern="^(draft|evidence|canonical)$")


class NoteUpdate(BaseModel):
    """Schema for updating an existing note"""
    title: Optional[str] = Field(None, min_length=1, max_length=255)
    content: Optional[str] = None
    tags: Optional[List[str]] = None
    status: Optional[str] = Field(None, pattern="^(draft|evidence|canonical)$")


class NoteListItem(BaseModel):
    """Summary of a note for list views"""
    id: str
    title: str
    status: str = "draft"
    tags: List[str] = Field(default_factory=list)
    created: Optional[datetime] = None
    modified: Optional[datetime] = None
    link_count: int = 0


class SearchResult(BaseModel):
    """Search result item"""
    note_id: str
    title: str
    snippet: str
    score: float = Field(..., ge=0.0, le=1.0)
    tags: List[str] = Field(default_factory=list)


class BacklinkInfo(BaseModel):
    """Information about a backlink to a note"""
    note_id: str
    title: str
    context: str = ""
