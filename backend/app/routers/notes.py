"""Notes API router for CRUD operations"""
from fastapi import APIRouter, HTTPException, Request
from typing import List, Optional
from app.models.note import (
    Note, NoteCreate, NoteUpdate, NoteListItem, TypedProperty, PropertyType
)
from app.utils.dependencies import get_knowledge_store, get_vector_search, get_graph_index

router = APIRouter()


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
    vector_search = get_vector_search(request)
    
    try:
        note = knowledge_store.create_note(note_data)
        
        # Index the new note with content type for priority scoring
        vector_search.index_note(
            note_id=note.id,
            title=note.title,
            content=note.content,
            content_type=note.frontmatter.content_type.value,
            modified=note.frontmatter.modified.isoformat() if note.frontmatter.modified else None,
            tags=note.frontmatter.tags,
        )
        
        return note
    except FileExistsError:
        raise HTTPException(status_code=409, detail="Note already exists")


@router.put("/{note_id}", response_model=Note)
async def update_note(note_id: str, note_data: NoteUpdate, request: Request):
    """Update an existing note"""
    knowledge_store = get_knowledge_store(request)
    vector_search = get_vector_search(request)
    
    note = knowledge_store.update_note(note_id, note_data)
    if note is None:
        raise HTTPException(status_code=404, detail="Note not found")
    
    # Update vector index with new content type for priority scoring
    vector_search.index_note(
        note_id=note.id,
        title=note.title,
        content=note.content,
        content_type=note.frontmatter.content_type.value,
        modified=note.frontmatter.modified.isoformat() if note.frontmatter.modified else None,
        tags=note.frontmatter.tags,
    )
    
    return note


@router.delete("/{note_id}", status_code=204)
async def delete_note(note_id: str, request: Request):
    """Delete a note"""
    knowledge_store = get_knowledge_store(request)
    vector_search = get_vector_search(request)
    graph_index = get_graph_index(request)

    success = knowledge_store.delete_note(note_id)
    if not success:
        raise HTTPException(status_code=404, detail="Note not found")

    # Remove from vector search index
    vector_search.delete_note(note_id)

    # Rebuild graph to remove stale edges
    graph_index.build_index()


@router.post("/reindex", response_model=dict)
async def reindex_notes(request: Request):
    """Reindex all notes for search and graph"""
    knowledge_store = get_knowledge_store(request)
    vector_search = get_vector_search(request)
    graph_index = get_graph_index(request)

    notes = knowledge_store.get_all_content()

    # Reindex vector search with content type for priority scoring
    vector_search.index_all(notes)

    # Rebuild graph
    graph_index.build_index()

    return {
        "indexed": len(notes),
        "message": f"Reindexed {len(notes)} notes"
    }


# Property endpoints
@router.get("/{note_id}/properties", response_model=dict)
async def get_properties(note_id: str, request: Request):
    """Get all properties for a note"""
    knowledge_store = get_knowledge_store(request)
    note = knowledge_store.get_note(note_id)
    if note is None:
        raise HTTPException(status_code=404, detail="Note not found")
    
    return {
        "note_id": note_id,
        "properties": note.frontmatter.properties
    }


@router.get("/{note_id}/properties/{property_name}", response_model=TypedProperty)
async def get_property(note_id: str, property_name: str, request: Request):
    """Get a specific property from a note"""
    knowledge_store = get_knowledge_store(request)
    note = knowledge_store.get_note(note_id)
    if note is None:
        raise HTTPException(status_code=404, detail="Note not found")
    
    prop = note.frontmatter.get_property(property_name)
    if prop is None:
        raise HTTPException(status_code=404, detail="Property not found")
    
    return prop


@router.put("/{note_id}/properties/{property_name}", response_model=TypedProperty)
async def set_property(
    note_id: str,
    property_name: str,
    property_data: TypedProperty,
    request: Request
):
    """Set or update a property on a note"""
    knowledge_store = get_knowledge_store(request)
    note = knowledge_store.get_note(note_id)
    if note is None:
        raise HTTPException(status_code=404, detail="Note not found")
    
    # Update the property in the note's frontmatter
    note.frontmatter.set_property(property_name, property_data)
    
    # Update the note in storage
    from app.models.note import NoteUpdate
    update_data = NoteUpdate(properties=note.frontmatter.properties)
    updated_note = knowledge_store.update_note(note_id, update_data)
    
    if updated_note is None:
        raise HTTPException(status_code=500, detail="Failed to update note")
    
    return property_data


@router.delete("/{note_id}/properties/{property_name}", status_code=204)
async def delete_property(note_id: str, property_name: str, request: Request):
    """Delete a property from a note"""
    knowledge_store = get_knowledge_store(request)
    note = knowledge_store.get_note(note_id)
    if note is None:
        raise HTTPException(status_code=404, detail="Note not found")
    
    deleted = note.frontmatter.delete_property(property_name)
    if not deleted:
        raise HTTPException(status_code=404, detail="Property not found")
    
    # Update the note in storage
    from app.models.note import NoteUpdate
    update_data = NoteUpdate(properties=note.frontmatter.properties)
    updated_note = knowledge_store.update_note(note_id, update_data)
    
    if updated_note is None:
        raise HTTPException(status_code=500, detail="Failed to update note")
    
    return None
