"""Graph API router for knowledge graph operations"""
from fastapi import APIRouter, HTTPException, Query, Request
from typing import List, Dict
from app.models.note import BacklinkInfo
from app.utils.dependencies import get_graph_index, get_knowledge_store

router = APIRouter()


@router.get("/backlinks/{note_id}", response_model=List[BacklinkInfo])
async def get_backlinks(note_id: str, request: Request):
    """
    Get all notes that link to the specified note

    - **note_id**: ID of the note to get backlinks for
    """
    graph_index = get_graph_index(request)
    backlinks = graph_index.get_backlinks_with_context(note_id)
    return backlinks


@router.get("/outgoing/{note_id}", response_model=List[str])
async def get_outgoing_links(note_id: str, request: Request):
    """
    Get all notes that the specified note links to

    - **note_id**: ID of the note to get outgoing links for
    """
    graph_index = get_graph_index(request)
    outgoing = graph_index.get_outgoing_links(note_id)
    return outgoing


@router.get("/neighbors/{note_id}", response_model=Dict[str, List[str]])
async def get_neighbors(
    note_id: str,
    request: Request,
    depth: int = Query(1, ge=1, le=3, description="Traversal depth (1-3)")
):
    """
    Get neighboring notes within a specified depth

    - **note_id**: ID of the starting note
    - **depth**: Traversal depth (1-3, default: 1)
    """
    graph_index = get_graph_index(request)
    neighbors = graph_index.get_neighbors(note_id, depth)
    return neighbors


@router.get("/unlinked", response_model=List[dict])
async def get_unlinked_notes(request: Request):
    """
    Get all notes that have no incoming or outgoing links (orphan notes)

    Returns notes that are completely disconnected from the knowledge graph.
    """
    graph_index = get_graph_index(request)
    return graph_index.get_unlinked_notes()


@router.get("/unlinked-mentions/{note_id}", response_model=List[dict])
async def find_unlinked_mentions(note_id: str, request: Request):
    """
    Find notes that mention the note title but don't link to it

    - **note_id**: ID of the note to find unlinked mentions for
    """
    graph_index = get_graph_index(request)
    mentions = graph_index.find_unlinked_mentions(note_id)
    return mentions


@router.post("/rebuild", response_model=dict)
async def rebuild_graph(request: Request):
    """
    Rebuild the entire knowledge graph from all notes
    """
    graph_index = get_graph_index(request)
    knowledge_store = get_knowledge_store(request)

    graph_index.build_index()
    notes = knowledge_store.list_notes()

    return {
        "processed": len(notes),
        "message": f"Rebuilt graph with {len(notes)} notes"
    }


@router.get("/full", response_model=dict)
async def get_full_graph(request: Request):
    """
    Get the complete knowledge graph structure
    """
    graph_index = get_graph_index(request)
    return graph_index.get_full_graph()
