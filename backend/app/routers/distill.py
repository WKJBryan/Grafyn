"""Distillation API router for Container → Atomic → Hub workflow"""
from fastapi import APIRouter, HTTPException, Request
from app.models.distillation import (
    DistillRequest,
    DistillResponse,
    DistillMode,
    ExtractionMethod,
)
from app.models.note import Note
from app.utils.dependencies import get_distillation as get_distillation_service, get_knowledge_store

router = APIRouter()


@router.post("/{note_id}/distill", response_model=DistillResponse)
async def distill_note(
    note_id: str,
    request_body: DistillRequest,
    request: Request
):
    """
    Distill container note into atomic notes.
    
    - mode="suggest": Extract candidates, find duplicates, return for review
    - mode="apply": Process user decisions, create/update atomic notes and hubs
    - mode="auto": Use LLM to summarize and auto-create draft atomic notes
    
    Extraction methods:
    - extraction_method="llm": Use LLM summarization (requires OpenRouter)
    - extraction_method="rules": Use rule-based extraction
    - extraction_method="auto": Prefer LLM, fallback to rules (default)
    
    Typical flow:
    1. Call with mode="suggest" to get candidates
    2. User reviews in UI, makes accept/skip/modify decisions
    3. Call with mode="apply" + decisions array to execute
    
    Or for automatic processing:
    1. Call with mode="auto" to auto-create drafts using LLM or rules
    """
    service = get_distillation_service(request)
    
    # Verify note exists
    note = get_knowledge_store(request).get_note(note_id)
    if not note:
        raise HTTPException(status_code=404, detail="Note not found")
    
    # Await the async distill method
    response = await service.distill(note_id, request_body)
    return response


@router.post("/{note_id}/normalize-tags", response_model=Note)
async def normalize_note_tags(note_id: str, request: Request):
    """
    Normalize YAML tags + merge inline #tags for a note.
    
    - Lowercases all tags
    - Strips leading '#' symbols
    - Converts spaces to hyphens
    - Merges inline #tags from body into YAML tags (merge-only)
    """
    service = get_distillation_service(request)
    
    note = service.normalize_note_tags(note_id)
    if not note:
        raise HTTPException(status_code=404, detail="Note not found")
    
    return note
