"""Notes API router for CRUD operations"""
from fastapi import APIRouter, HTTPException, Request
from typing import List
from backend.app.models.note import Note, NoteCreate, NoteUpdate, NoteListItem
from backend.app.services.knowledge_store import KnowledgeStore
from backend.app.services.vector_search import VectorSearchService
from backend.app.services.graph_index import GraphIndexService

router = APIRouter()


def get_knowledge_store(request: Request) -> KnowledgeStore:
    """Get knowledge store from app state"""
    return request.app.state.knowledge_store


@router.get("", response_model=List[NoteListItem])
async def list_notes(request: Request):
    """List all notes with metadata"""
    knowledge_store = get_knowledge_store(request)
    return knowledge_store.list_notes()


@router.get("/{note_id}", response_model=Note)
async def get_note(note_id: str, request: Request):
    """Get a specific note by ID"""
    knowledge_store = get_knowledge_store(request)
    note = knowledge_store.get_note(note_id)
    if note is None:
        raise HTTPException(status_code=404, detail="Note not found")
    return note


@router.post("", response_model=Note, status_code=201)
async def create_note(note_data: NoteCreate, request: Request):
    """Create a new note"""
    knowledge_store = get_knowledge_store(request)
    try:
        note = knowledge_store.create_note(note_data)
        return note
    except FileExistsError:
        raise HTTPException(status_code=409, detail="Note already exists")


@router.put("/{note_id}", response_model=Note)
async def update_note(note_id: str, note_data: NoteUpdate, request: Request):
    """Update an existing note"""
    knowledge_store = get_knowledge_store(request)
    note = knowledge_store.update_note(note_id, note_data)
    if note is None:
        raise HTTPException(status_code=404, detail="Note not found")
    return note


@router.delete("/{note_id}", status_code=204)
async def delete_note(note_id: str, request: Request):
    """Delete a note"""
    knowledge_store = get_knowledge_store(request)
    success = knowledge_store.delete_note(note_id)
    if not success:
        raise HTTPException(status_code=404, detail="Note not found")


@router.post("/reindex", response_model=dict)
async def reindex_notes(request: Request):
    """Reindex all notes for search and graph"""
    knowledge_store = get_knowledge_store(request)
    vector_search: VectorSearchService = request.app.state.vector_search
    graph_index: GraphIndexService = request.app.state.graph_index

    notes = knowledge_store.get_all_content()

    # Reindex vector search
    vector_search.index_all(notes)

    # Rebuild graph
    graph_index.build_index()

    return {
        "indexed": len(notes),
        "message": f"Reindexed {len(notes)} notes"
    }
