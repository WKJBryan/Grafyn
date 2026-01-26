"""MCP write endpoints with OAuth authentication for ChatGPT integration"""
from fastapi import APIRouter, Depends, HTTPException, Request
from typing import Optional
from datetime import datetime, timezone
from enum import Enum

from app.models.note import (
    Note,
    NoteCreate,
    NoteUpdate,
    NoteFrontmatter,
    TypedProperty,
    ContentType,
    NoteType,
    PropertyType
)
from app.services.knowledge_store import KnowledgeStore
from app.services.vector_search import VectorSearchService
from app.mcp.oauth import verify_oauth
from app.mcp.write_tools import (
    CreateNoteRequest,
    UpdateNoteRequest,
    SetPropertyRequest,
    FindOrCreateNoteRequest
)
from app.config import get_settings

settings = get_settings()

router = APIRouter()

# In development mode, skip OAuth for easier testing
dev_mode = settings.environment == "development"


@router.get("/mcp/test")
async def mcp_test() -> dict:
    """Simple test endpoint to verify routing works"""
    return {"status": "ok", "message": "MCP write router is working"}


@router.post("/mcp/test/simple")
async def mcp_test_simple(title: str, content: str = "") -> dict:
    """Simple test endpoint without Pydantic models"""
    return {"status": "ok", "title": title, "content": content}


def get_services(request: Request):
    """Get required services from app state"""
    return {
        "knowledge_store": request.app.state.knowledge_store,
        "vector_search": request.app.state.vector_search,
    }


def add_mcp_provenance(properties: Optional[dict]) -> dict:
    """Add provenance tracking to note properties"""
    if properties is None:
        properties = {}
    # Mark that this note was created via MCP
    properties["source"] = {
        "type": "string",
        "value": "chatgpt-mcp",
        "label": "Source"
    }
    properties["created_via"] = {
        "type": "string",
        "value": "mcp",
        "label": "Created Via"
    }
    properties["mcp_created_at"] = {
        "type": "date",
        "value": datetime.now(timezone.utc).isoformat(),
        "label": "MCP Created At"
    }
    return properties


@router.post("/mcp/notes/create", dependencies=[] if dev_mode else [Depends(verify_oauth)])
async def mcp_create_note_simple(
    request: Request,
    title: str = None,
    content: str = "",
    tags: str = "",
    status: str = "draft"
) -> dict:
    """
    Create a new note via MCP (simple version - parameters in request body).

    Send JSON body: {"title": "Note Title", "content": "Note content"}
    """
    import sys
    import json

    # Try to get title from request body
    body = await request.body()
    if body:
        data = json.loads(body)
        title = data.get("title", title)
        content = data.get("content", content)
        tags = data.get("tags", tags)
        status = data.get("status", status)

    if not title:
        raise HTTPException(status_code=400, detail="title is required")

    print(f"DEBUG: mcp_create_note_simple called with title={title}", file=sys.stderr)

    services = get_services(request)
    knowledge_store: KnowledgeStore = services["knowledge_store"]
    vector_search: VectorSearchService = services["vector_search"]

    # Parse tags
    if isinstance(tags, list):
        tag_list = tags
    elif isinstance(tags, str):
        tag_list = [t.strip() for t in tags.split(",")] if tags else []
    else:
        tag_list = []

    # Create NoteCreate directly
    note_data = NoteCreate(
        title=title,
        content=content,
        tags=tag_list,
        status=status
    )

    try:
        # Create the note
        note = knowledge_store.create_note(note_data)

        # Index the new note
        vector_search.index_note(
            note_id=note.id,
            title=note.title,
            content=note.content,
            content_type=note.frontmatter.content_type.value,
            modified=note.frontmatter.modified.isoformat() if note.frontmatter.modified else None,
            tags=note.frontmatter.tags,
        )

        return {
            "id": note.id,
            "title": note.title,
            "status": "created",
            "message": f"Created note '{note.title}'"
        }

    except FileExistsError:
        raise HTTPException(
            status_code=409,
            detail=f"Note with title '{title}' already exists."
        )
    except Exception as e:
        import traceback
        print(f"ERROR: {e}", file=sys.stderr)
        print(f"TRACEBACK: {traceback.format_exc()}", file=sys.stderr)
        raise HTTPException(status_code=500, detail=str(e))


# Keep the original endpoint for now but with individual parameters
@router.post("/mcp/notes", dependencies=[] if dev_mode else [Depends(verify_oauth)])
async def mcp_create_note(
    note_request: CreateNoteRequest,
    request: Request
) -> dict:
    """
    Create a new note via MCP.

    This endpoint is called by ChatGPT to save information to the knowledge base.
    All notes created via MCP are tagged with provenance tracking.
    """
    # Debug logging
    import sys
    print(f"DEBUG: mcp_create_note called with title={note_request.title}", file=sys.stderr)

    services = get_services(request)
    knowledge_store: KnowledgeStore = services["knowledge_store"]
    vector_search: VectorSearchService = services["vector_search"]

    # Map string values to ContentType enum
    content_type_map = {
        "claim": ContentType.CLAIM,
        "decision": ContentType.DECISION,
        "insight": ContentType.INSIGHT,
        "question": ContentType.QUESTION,
        "evidence": ContentType.EVIDENCE,
        "general": ContentType.GENERAL,
    }

    note_type_map = {
        "container": NoteType.CONTAINER,
        "atomic": NoteType.ATOMIC,
        "hub": NoteType.HUB,
        "general": NoteType.GENERAL,
    }

    ct_value = note_request.content_type
    nt_value = note_request.note_type

    # Convert properties format for NoteCreate
    note_properties = {}
    if note_request.properties:
        for key, val in note_request.properties.items():
            if isinstance(val, dict):
                note_properties[key] = TypedProperty(
                    type=PropertyType(val.get("type", "string")),
                    value=val.get("value", ""),
                    label=val.get("label")
                )
            else:
                note_properties[key] = val

    # Add MCP provenance
    for key, val in add_mcp_provenance(None).items():
        note_properties[key] = TypedProperty(
            type=PropertyType(val["type"]),
            value=val["value"],
            label=val.get("label")
        )

    # Convert request to NoteCreate model
    note_data = NoteCreate(
        title=note_request.title,
        content=note_request.content,
        tags=note_request.tags,
        status=note_request.status,
        content_type=content_type_map.get(ct_value, ContentType.GENERAL),
        note_type=note_type_map.get(nt_value, NoteType.GENERAL),
        properties=note_properties
    )

    try:
        # Create the note
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

        # Return simplified response instead of full Note object
        return {
            "id": note.id,
            "title": note.title,
            "status": "created",
            "message": f"Created note '{note.title}'"
        }

    except FileExistsError:
        raise HTTPException(
            status_code=409,
            detail=f"Note with title '{note_request.title}' already exists. Use update_note instead."
        )
    except Exception as e:
        import traceback
        print(f"ERROR: {e}", file=sys.stderr)
        print(f"TRACEBACK: {traceback.format_exc()}", file=sys.stderr)
        raise HTTPException(status_code=500, detail=str(e))


@router.put("/mcp/notes/{note_id}", dependencies=[] if dev_mode else [Depends(verify_oauth)])
async def mcp_update_note(
    note_id: str,
    note_request: UpdateNoteRequest,
    request: Request
) -> Note:
    """
    Update an existing note via MCP.

    Supports different content modes:
    - replace: Overwrite existing content
    - append: Add new content to the end
    - prepend: Add new content to the beginning
    """
    services = get_services(request)
    knowledge_store: KnowledgeStore = services["knowledge_store"]
    vector_search: VectorSearchService = services["vector_search"]

    # Check if note exists
    existing_note = knowledge_store.get_note(note_id)
    if existing_note is None:
        raise HTTPException(
            status_code=404,
            detail=f"Note '{note_id}' not found. Use create_note to create it."
        )

    # Prepare update data
    update_data = NoteUpdate()

    # Handle title
    if note_request.title is not None:
        update_data.title = note_request.title

    # Handle content with mode
    if note_request.content is not None:
        if note_request.content_mode == "replace":
            update_data.content = note_request.content
        elif note_request.content_mode == "append":
            separator = "\n\n" if existing_note.content else ""
            update_data.content = existing_note.content + separator + note_request.content
        elif note_request.content_mode == "prepend":
            separator = "\n\n" if existing_note.content else ""
            update_data.content = note_request.content + separator + existing_note.content

    # Handle tags with mode
    if note_request.tags is not None:
        if note_request.tags_mode == "replace":
            update_data.tags = note_request.tags
        elif note_request.tags_mode == "merge":
            # Combine existing and new tags, deduplicating
            merged_tags = list(set(existing_note.frontmatter.tags + note_request.tags))
            update_data.tags = merged_tags
        elif note_request.tags_mode == "remove":
            # Remove specified tags
            filtered_tags = [t for t in existing_note.frontmatter.tags if t not in note_request.tags]
            update_data.tags = filtered_tags

    # Handle status
    if note_request.status is not None:
        update_data.status = note_request.status

    # Apply update
    updated_note = knowledge_store.update_note(note_id, update_data)

    if updated_note is None:
        raise HTTPException(status_code=500, detail="Failed to update note")

    # Update vector index
    vector_search.index_note(
        note_id=updated_note.id,
        title=updated_note.title,
        content=updated_note.content,
        content_type=updated_note.frontmatter.content_type.value,
        modified=updated_note.frontmatter.modified.isoformat() if updated_note.frontmatter.modified else None,
        tags=updated_note.frontmatter.tags,
    )

    # Add MCP provenance property if not present
    if not updated_note.frontmatter.get_property("modified_via"):
        from app.models.note import NoteUpdate
        properties = updated_note.frontmatter.properties.copy()
        properties["modified_via"] = {
            "type": "string",
            "value": "mcp",
            "label": "Modified Via"
        }
        properties["mcp_modified_at"] = {
            "type": "date",
            "value": datetime.now(timezone.utc).isoformat(),
            "label": "MCP Modified At"
        }
        knowledge_store.update_note(note_id, NoteUpdate(properties=properties))

    return updated_note


@router.post("/mcp/notes/find-or-create", dependencies=[] if dev_mode else [Depends(verify_oauth)])
async def mcp_find_or_create_note(
    note_request: FindOrCreateNoteRequest,
    request: Request
) -> dict:
    """
    Search for an existing note, create new only if no good match found.

    This prevents duplicate notes on the same topic. Searches using semantic
    search and returns existing note if similarity >= threshold.
    """
    services = get_services(request)
    knowledge_store: KnowledgeStore = services["knowledge_store"]
    vector_search: VectorSearchService = services["vector_search"]

    # Search for existing notes
    results = vector_search.search(
        query=note_request.search_query,
        limit=5
    )

    # Check if any result is similar enough
    for result in results:
        if result["score"] >= note_request.threshold:
            existing_note = knowledge_store.get_note(result["note_id"])
            if existing_note:
                return {
                    "action": "found",
                    "note_id": existing_note.id,
                    "title": existing_note.title,
                    "similarity": result["score"],
                    "message": f"Found existing note with {result["score"]:.1%} similarity"
                }

    # No good match found, create new note
    create_request = CreateNoteRequest(
        title=note_request.title,
        content=note_request.content,
        tags=note_request.tags,
        status="draft"
    )

    created_note = await mcp_create_note(create_request, request)

    return {
        "action": "created",
        "note_id": created_note.id,
        "title": created_note.title,
        "message": f"Created new note (no existing note met the {note_request.threshold:.1%} similarity threshold)"
    }


@router.put("/mcp/notes/{note_id}/properties", dependencies=[] if dev_mode else [Depends(verify_oauth)])
async def mcp_set_property(
    note_id: str,
    prop_request: SetPropertyRequest,
    request: Request
) -> TypedProperty:
    """
    Set or update a typed property on a note via MCP.

    Properties are typed key-value pairs stored in note frontmatter.
    """
    services = get_services(request)
    knowledge_store: KnowledgeStore = services["knowledge_store"]

    # Check if note exists
    note = knowledge_store.get_note(note_id)
    if note is None:
        raise HTTPException(
            status_code=404,
            detail=f"Note '{note_id}' not found"
        )

    # Create TypedProperty
    typed_property = TypedProperty(
        type=prop_request.property_type.value,
        value=prop_request.value,
        label=prop_request.label
    )

    # Set the property on the note
    note.frontmatter.set_property(prop_request.property_name, typed_property)

    # Update the note
    from app.models.note import NoteUpdate
    update_data = NoteUpdate(properties=note.frontmatter.properties)
    updated_note = knowledge_store.update_note(note_id, update_data)

    if updated_note is None:
        raise HTTPException(status_code=500, detail="Failed to update note property")

    return typed_property


@router.get("/mcp/notes/search")
async def mcp_search_notes(
    q: str,
    limit: int = 10,
    request: Request = None
) -> list:
    """
    Search notes via MCP (used by find_or_create_note and ChatGPT).
    No auth required for search in development mode.
    """
    # FastAPI will inject Request automatically
    if request is None:
        return []

    vector_search: VectorSearchService = request.app.state.vector_search

    results = vector_search.search(
        query=q,
        limit=min(limit, 20)
    )

    return [
        {
            "note_id": r["note_id"],
            "title": r["title"],
            "snippet": r.get("snippet", ""),
            "score": r["score"],
            "tags": r.get("tags", [])
        }
        for r in results
    ]

@router.post("/mcp-write/note")
async def mcp_write_note(request: Request) -> dict:
    """Create note via MCP - simplified endpoint"""
    import sys
    import json
    
    body_bytes = await request.body()
    data = json.loads(body_bytes)
    
    title = data.get("title")
    if not title:
        raise HTTPException(status_code=400, detail="title is required")
    
    content = data.get("content", "")
    tags = data.get("tags", [])
    status = data.get("status", "draft")
    
    services = get_services(request)
    knowledge_store: KnowledgeStore = services["knowledge_store"]
    vector_search: VectorSearchService = services["vector_search"]
    
    # Create NoteCreate
    note_data = NoteCreate(
        title=title,
        content=content,
        tags=tags if isinstance(tags, list) else [],
        status=status
    )
    
    try:
        note = knowledge_store.create_note(note_data)
        
        # Index for vector search
        vector_search.index_note(
            note_id=note.id,
            title=note.title,
            content=note.content,
            content_type=note.frontmatter.content_type.value,
            modified=note.frontmatter.modified.isoformat() if note.frontmatter.modified else None,
            tags=note.frontmatter.tags,
        )
        
        print(f"SUCCESS: Created note {note.id}", file=sys.stderr)
        
        return {
            "id": note.id,
            "title": note.title,
            "status": "created",
            "message": f"Created note '{note.title}'"
        }
        
    except FileExistsError:
        raise HTTPException(status_code=409, detail=f"Note '{title}' already exists")
    except Exception as e:
        import traceback
        print(f"ERROR: {e}\n{traceback.format_exc()}", file=sys.stderr)
        raise HTTPException(status_code=500, detail=str(e))
