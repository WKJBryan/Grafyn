"""Memory API router for contextual recall, contradiction detection, and extraction."""
from typing import List, Optional

from fastapi import APIRouter, Request, HTTPException
from pydantic import BaseModel, Field

from app.middleware.rate_limit import limiter
from app.utils.dependencies import get_memory_service

router = APIRouter()


# ============================================================================
# Request / Response Models
# ============================================================================


class RecallRequest(BaseModel):
    query: str = Field(..., min_length=1, max_length=1000)
    context_note_ids: Optional[List[str]] = None
    limit: int = Field(default=5, ge=1, le=50)


class RecallResult(BaseModel):
    note_id: str
    title: str
    content: str
    relevance_score: float
    connection_type: str  # "semantic" | "graph" | "both"


class RecallResponse(BaseModel):
    results: List[RecallResult]


class ContradictionItem(BaseModel):
    note_id: str
    title: str
    conflicting_field: str  # "status" | "tags" | "content"
    this_value: str
    other_value: str
    similarity_score: float


class ContradictionsResponse(BaseModel):
    contradictions: List[ContradictionItem]


class ChatMessage(BaseModel):
    role: str = Field(..., pattern="^(user|assistant)$")
    content: str


class ExtractRequest(BaseModel):
    messages: List[ChatMessage] = Field(..., min_length=1)
    source: Optional[str] = "conversation"


class NoteSuggestion(BaseModel):
    title: str
    content: str
    tags: List[str]
    status: str
    source: str


class ExtractResponse(BaseModel):
    suggestions: List[NoteSuggestion]


# ============================================================================
# Endpoints
# ============================================================================


@router.post("/recall", response_model=RecallResponse)
@limiter.limit("30 per minute")
async def recall(request: Request, body: RecallRequest):
    """Recall relevant notes using semantic search and graph context."""
    memory = get_memory_service(request)
    results = memory.recall_relevant(
        query=body.query,
        context_note_ids=body.context_note_ids,
        limit=body.limit,
    )
    return RecallResponse(results=results)


@router.post("/contradictions/{note_id}", response_model=ContradictionsResponse)
@limiter.limit("20 per minute")
async def contradictions(note_id: str, request: Request):
    """Find potential contradictions for a given note."""
    memory = get_memory_service(request)
    items = memory.find_contradictions(note_id)
    if items is None:
        raise HTTPException(status_code=404, detail="Note not found")
    return ContradictionsResponse(contradictions=items)


@router.post("/extract", response_model=ExtractResponse)
@limiter.limit("10 per minute")
async def extract(request: Request, body: ExtractRequest):
    """Extract note suggestions from a conversation."""
    memory = get_memory_service(request)
    messages = [{"role": m.role, "content": m.content} for m in body.messages]
    suggestions = memory.extract_from_conversation(
        messages=messages,
        source=body.source or "conversation",
    )
    return ExtractResponse(suggestions=suggestions)
