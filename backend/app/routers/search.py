"""Search API router for semantic and lexical search"""
from fastapi import APIRouter, Query, Request, HTTPException
from typing import List, Optional
from backend.app.models.note import SearchResult
from backend.app.services.vector_search import VectorSearchService
from backend.app.services.knowledge_store import KnowledgeStore
from backend.app.services.priority_scoring import PriorityScoringService

router = APIRouter()


def get_vector_search(request: Request) -> VectorSearchService:
    """Get vector search service from app state"""
    return request.app.state.vector_search


def get_knowledge_store(request: Request) -> KnowledgeStore:
    """Get knowledge store from app state"""
    return request.app.state.knowledge_store


def get_priority_scoring(request: Request) -> Optional[PriorityScoringService]:
    """Get priority scoring service from app state (optional)"""
    return getattr(request.app.state, 'priority_scoring', None)


@router.get("", response_model=List[SearchResult])
async def search_notes(
    request: Request,
    q: str = Query(..., min_length=1, max_length=500, description="Search query"),
    limit: int = Query(10, ge=1, le=50, description="Maximum number of results"),
    semantic: bool = Query(True, description="Use semantic vector search"),
    use_priority: bool = Query(True, description="Apply priority scoring")
):
    """
    Search notes by query with optional priority scoring

    - **q**: Search query string (required)
    - **limit**: Maximum number of results (1-50, default: 10)
    - **semantic**: Use semantic vector search (default: true)
    - **use_priority**: Apply priority scoring for better relevance (default: true)
    """
    vector_search = get_vector_search(request)

    if semantic:
        results = vector_search.search(q, limit)
        
        # Apply priority scoring if enabled
        if use_priority:
            priority_scoring = get_priority_scoring(request)
            knowledge_store = get_knowledge_store(request)
            if priority_scoring:
                # Parse query for tags
                parsed = vector_search.parse_search_query(q)
                results = priority_scoring.score_search_results(
                    results, parsed.tags, knowledge_store
                )
        
        return results[:limit]
    else:
        # Lexical search fallback
        knowledge_store = get_knowledge_store(request)
        notes = knowledge_store.list_notes()

        # Simple text matching
        results = []
        query_lower = q.lower()
        for note in notes:
            if query_lower in note.title.lower():
                results.append(SearchResult(
                    note_id=note.id,
                    title=note.title,
                    snippet=note.title,
                    score=1.0,
                    tags=note.tags
                ))
        return results[:limit]


@router.get("/similar/{note_id}", response_model=List[SearchResult])
async def find_similar_notes(
    note_id: str,
    request: Request,
    limit: int = Query(5, ge=1, le=20, description="Maximum number of results")
):
    """
    Find notes similar to a given note

    - **note_id**: ID of the reference note
    - **limit**: Maximum number of results (1-20, default: 5)
    """
    vector_search = get_vector_search(request)
    knowledge_store = get_knowledge_store(request)

    note = knowledge_store.get_note(note_id)
    if note is None:
        raise HTTPException(status_code=404, detail="Note not found")

    # Search using note title and content
    query = f"{note.title}\n\n{note.content[:500]}"
    return vector_search.search(query, limit)
