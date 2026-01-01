"""Graph API router for knowledge graph operations"""
from fastapi import APIRouter, HTTPException, Query
from typing import List, Dict
from app.models.note import BacklinkInfo
from app.services.graph_index import GraphIndexService

router = APIRouter()

# Initialize graph index service (will be properly initialized in app startup)
graph_index = None


@router.get("/backlinks/{note_id}", response_model=List[BacklinkInfo])
async def get_backlinks(note_id: str):
    """
    Get all notes that link to the specified note
    
    - **note_id**: ID of the note to get backlinks for
    """
    global graph_index
    if graph_index is None:
        graph_index = GraphIndexService()
    
    backlinks = graph_index.get_backlinks_with_context(note_id)
    return backlinks


@router.get("/outgoing/{note_id}", response_model=List[str])
async def get_outgoing_links(note_id: str):
    """
    Get all notes that the specified note links to
    
    - **note_id**: ID of the note to get outgoing links for
    """
    global graph_index
    if graph_index is None:
        graph_index = GraphIndexService()
    
    outgoing = graph_index.get_outgoing_links(note_id)
    return outgoing


@router.get("/neighbors/{note_id}", response_model=Dict[str, List[str]])
async def get_neighbors(
    note_id: str,
    depth: int = Query(1, ge=1, le=3, description="Traversal depth (1-3)")
):
    """
    Get neighboring notes within a specified depth
    
    - **note_id**: ID of the starting note
    - **depth**: Traversal depth (1-3, default: 1)
    """
    global graph_index
    if graph_index is None:
        graph_index = GraphIndexService()
    
    neighbors = graph_index.get_neighbors(note_id, depth)
    return neighbors


@router.get("/unlinked-mentions/{note_id}", response_model=List[dict])
async def find_unlinked_mentions(note_id: str):
    """
    Find notes that mention the note title but don't link to it
    
    - **note_id**: ID of the note to find unlinked mentions for
    """
    global graph_index
    if graph_index is None:
        graph_index = GraphIndexService()
    
    mentions = graph_index.find_unlinked_mentions(note_id)
    return mentions


@router.post("/rebuild", response_model=dict)
async def rebuild_graph():
    """
    Rebuild the entire knowledge graph from all notes
    """
    global graph_index
    if graph_index is None:
        graph_index = GraphIndexService()
    
    graph_index.build_index()
    
    # Get total number of notes
    from app.services.knowledge_store import KnowledgeStore
    knowledge_store = KnowledgeStore()
    notes = knowledge_store.list_notes()
    
    return {
        "processed": len(notes),
        "message": f"Rebuilt graph with {len(notes)} notes"
    }
