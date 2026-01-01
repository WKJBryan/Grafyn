"""Notes API router for CRUD operations"""
from fastapi import APIRouter, HTTPException
from typing import List
from app.models.note import Note, NoteCreate, NoteUpdate, NoteListItem
from app.services.knowledge_store import KnowledgeStore

router = APIRouter()

# Initialize knowledge store (will be properly initialized in app startup)
knowledge_store = None


@router.get("", response_model=List[NoteListItem])
async def list_notes():
    """List all notes with metadata"""
    global knowledge_store
    if knowledge_store is None:
        knowledge_store = KnowledgeStore()
    return knowledge_store.list_notes()


@router.get("/{note_id}", response_model=Note)
async def get_note(note_id: str):
    """Get a specific note by ID"""
    global knowledge_store
    if knowledge_store is None:
        knowledge_store = KnowledgeStore()
    
    note = knowledge_store.get_note(note_id)
    if note is None:
        raise HTTPException(status_code=404, detail="Note not found")
    return note


@router.post("", response_model=Note, status_code=201)
async def create_note(note_data: NoteCreate):
    """Create a new note"""
    global knowledge_store
    if knowledge_store is None:
        knowledge_store = KnowledgeStore()
    
    try:
        note = knowledge_store.create_note(note_data)
        return note
    except FileExistsError:
        raise HTTPException(status_code=409, detail="Note already exists")


@router.put("/{note_id}", response_model=Note)
async def update_note(note_id: str, note_data: NoteUpdate):
    """Update an existing note"""
    global knowledge_store
    if knowledge_store is None:
        knowledge_store = KnowledgeStore()
    
    note = knowledge_store.update_note(note_id, note_data)
    if note is None:
        raise HTTPException(status_code=404, detail="Note not found")
    return note


@router.delete("/{note_id}", status_code=204)
async def delete_note(note_id: str):
    """Delete a note"""
    global knowledge_store
    if knowledge_store is None:
        knowledge_store = KnowledgeStore()
    
    success = knowledge_store.delete_note(note_id)
    if not success:
        raise HTTPException(status_code=404, detail="Note not found")


@router.post("/reindex", response_model=dict)
async def reindex_notes():
    """Reindex all notes for search and graph"""
    global knowledge_store
    if knowledge_store is None:
        knowledge_store = KnowledgeStore()
    
    # Import here to avoid circular dependency
    from app.services.vector_search import VectorSearchService
    from app.services.graph_index import GraphIndexService
    
    notes = knowledge_store.get_all_content()
    
    # Reindex vector search
    vector_search = VectorSearchService()
    vector_search.index_all(notes)
    
    # Rebuild graph
    graph_index = GraphIndexService()
    graph_index.build_index()
    
    return {
        "indexed": len(notes),
        "message": f"Reindexed {len(notes)} notes"
    }
