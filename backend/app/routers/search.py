"""Search API router for semantic and lexical search"""
from fastapi import APIRouter, Query
from typing import List
from app.models.note import SearchResult
from app.services.vector_search import VectorSearchService

router = APIRouter()

# Initialize vector search service (will be properly initialized in app startup)
vector_search = None


@router.get("", response_model=List[SearchResult])
async def search_notes(
    q: str = Query(..., min_length=1, max_length=500, description="Search query"),
    limit: int = Query(10, ge=1, le=50, description="Maximum number of results"),
    semantic: bool = Query(True, description="Use semantic vector search")
):
    """
    Search notes by query
    
    - **q**: Search query string (required)
    - **limit**: Maximum number of results (1-50, default: 10)
    - **semantic**: Use semantic vector search (default: true)
    """
    global vector_search
    if vector_search is None:
        vector_search = VectorSearchService()
    
    if semantic:
        return vector_search.search(q, limit)
    else:
        # Lexical search fallback
        from app.services.knowledge_store import KnowledgeStore
        knowledge_store = KnowledgeStore()
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
    limit: int = Query(5, ge=1, le=20, description="Maximum number of results")
):
    """
    Find notes similar to a given note
    
    - **note_id**: ID of the reference note
    - **limit**: Maximum number of results (1-20, default: 5)
    """
    global vector_search
    if vector_search is None:
        vector_search = VectorSearchService()
    
    from app.services.knowledge_store import KnowledgeStore
    knowledge_store = KnowledgeStore()
    note = knowledge_store.get_note(note_id)
    
    if note is None:
        from fastapi import HTTPException
        raise HTTPException(status_code=404, detail="Note not found")
    
    # Search using note title and content
    query = f"{note.title}\n\n{note.content[:500]}"
    return vector_search.search(query, limit)
