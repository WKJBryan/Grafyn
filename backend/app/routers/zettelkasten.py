"""API router for Zettelkasten distillation and link discovery"""
from fastapi import APIRouter, HTTPException, Request, BackgroundTasks
from typing import Optional
import logging

from app.models.distillation import LinkMode, LinkType, DistillResponse
from app.utils.dependencies import get_knowledge_store, get_distillation, get_link_discovery

logger = logging.getLogger(__name__)

router = APIRouter()


@router.post("/notes/{note_id}/distill-zettel")
async def distill_zettelkasten(
    note_id: str,
    request: Request,
    link_mode: LinkMode = LinkMode.AUTOMATIC
):
    """
    Distill a container note into Zettelkasten-formatted atomic notes.

    This endpoint extracts atomic notes from a container note using LLM-based
    Zettelkasten principles, creating properly typed and formatted notes.

    Args:
        note_id: ID of the container note to distill
        link_mode: Link discovery mode
            - automatic: Create all discovered links immediately
            - suggested: Return link candidates for user approval
            - manual: No automatic linking

    Returns:
        DistillResponse with created note IDs and candidates
    """
    distillation = get_distillation(request)

    if not distillation:
        raise HTTPException(
            status_code=503,
            detail="Distillation service not available"
        )

    # Check if note exists
    note = get_knowledge_store(request).get_note(note_id)
    if not note:
        raise HTTPException(status_code=404, detail="Note not found")

    response = await distillation.distill_zettelkasten(
        note_id=note_id,
        link_mode=link_mode
    )

    return response


@router.get("/notes/{note_id}/discover-links")
async def discover_links(
    note_id: str,
    request: Request,
    mode: LinkMode = LinkMode.SUGGESTED,
    max_links: int = 10
):
    """
    Discover potential links for a note without applying them.

    This endpoint finds potential links based on:
    - Semantic similarity (vector search)
    - Keyword/tag overlap
    - LLM-based conceptual relationships

    Args:
        note_id: ID of the note to find links for
        mode: Link discovery mode (suggest or manual)
        max_links: Maximum number of links to return

    Returns:
        List of link candidates with confidence scores
    """
    link_service = get_link_discovery(request)

    if not link_service:
        raise HTTPException(
            status_code=503,
            detail="Link discovery service not available"
        )

    note = get_knowledge_store(request).get_note(note_id)
    if not note:
        raise HTTPException(status_code=404, detail="Note not found")

    links = await link_service.discover_links(
        note=note,
        mode=mode,
        max_links=max_links
    )

    return {
        "note_id": note_id,
        "links": [
            {
                "target_id": l.target_id,
                "target_title": l.target_title,
                "link_type": l.link_type.value,
                "confidence": l.confidence,
                "reason": l.reason
            }
            for l in links
        ]
    }


@router.post("/notes/{source_id}/link/{target_id}")
async def create_link(
    source_id: str,
    target_id: str,
    request: Request,
    link_type: str = "related"
):
    """
    Create a bidirectional link between two notes.

    Creates wikilinks in both notes and updates the graph index.

    Args:
        source_id: Source note ID
        target_id: Target note ID
        link_type: Type of relationship (related, supports, contradicts, expands, etc.)

    Returns:
        Status of the link creation
    """
    link_service = get_link_discovery(request)

    if not link_service:
        raise HTTPException(
            status_code=503,
            detail="Link discovery service not available"
        )

    # Validate notes exist
    knowledge_store = get_knowledge_store(request)
    source = knowledge_store.get_note(source_id)
    target = knowledge_store.get_note(target_id)

    if not source:
        raise HTTPException(status_code=404, detail=f"Source note {source_id} not found")
    if not target:
        raise HTTPException(status_code=404, detail=f"Target note {target_id} not found")

    # Parse link type
    try:
        link_type_enum = LinkType(link_type)
    except ValueError:
        link_type_enum = LinkType.RELATED

    success = await link_service.create_bidirectional_links(
        source_id=source_id,
        target_id=target_id,
        link_type=link_type_enum
    )

    if not success:
        raise HTTPException(
            status_code=400,
            detail="Failed to create link (may already exist)"
        )

    return {
        "status": "linked",
        "source": source_id,
        "target": target_id,
        "link_type": link_type_enum.value
    }


@router.get("/link-types")
async def get_link_types():
    """
    Get available link types for Zettelkasten linking.

    Returns a list of all supported link types with descriptions.
    """
    return {
        "link_types": [
            {
                "value": "related",
                "label": "Related",
                "description": "General conceptual relationship"
            },
            {
                "value": "supports",
                "label": "Supports",
                "description": "Evidence supports claim"
            },
            {
                "value": "contradicts",
                "label": "Contradicts",
                "description": "Notes contradict each other"
            },
            {
                "value": "expands",
                "label": "Expands",
                "description": "One note expands on another"
            },
            {
                "value": "questions",
                "label": "Questions",
                "description": "Note questions another"
            },
            {
                "value": "answers",
                "label": "Answers",
                "description": "Note answers another"
            },
            {
                "value": "example",
                "label": "Example",
                "description": "Note is example of concept"
            },
            {
                "value": "part_of",
                "label": "Part Of",
                "description": "Part-whole relationship"
            }
        ]
    }


@router.get("/zettel-types")
async def get_zettel_types():
    """
    Get available Zettelkasten note types.

    Returns a list of all supported note types with descriptions.
    """
    return {
        "zettel_types": [
            {
                "value": "concept",
                "label": "Concept",
                "icon": "💡",
                "description": "Definitions and explanations of ideas"
            },
            {
                "value": "claim",
                "label": "Claim",
                "icon": "📣",
                "description": "Assertions, hypotheses needing evidence"
            },
            {
                "value": "evidence",
                "label": "Evidence",
                "icon": "📊",
                "description": "Data, research, examples supporting claims"
            },
            {
                "value": "question",
                "label": "Question",
                "icon": "❓",
                "description": "Inquiries driving exploration"
            },
            {
                "value": "fleche",
                "label": "Structure",
                "icon": "🔗",
                "description": "Connections between multiple ideas"
            },
            {
                "value": "fleeting",
                "label": "Fleeting",
                "icon": "⚡",
                "description": "Quick temporary captures"
            }
        ]
    }


@router.post("/notes/{note_id}/discover-links/apply")
async def apply_discovered_links(
    note_id: str,
    request: Request,
    link_ids: list[str] = []
):
    """
    Apply selected discovered links to a note.

    Used after discover_links to apply user-selected links.

    Args:
        note_id: ID of the note to add links to
        link_ids: List of target note IDs to create links with

    Returns:
        Number of links created
    """
    link_service = get_link_discovery(request)

    if not link_service:
        raise HTTPException(
            status_code=503,
            detail="Link discovery service not available"
        )

    knowledge_store = get_knowledge_store(request)
    source = knowledge_store.get_note(note_id)
    if not source:
        raise HTTPException(status_code=404, detail="Note not found")

    created_count = 0
    for target_id in link_ids:
        target = knowledge_store.get_note(target_id)
        if not target:
            continue

        success = await link_service.create_bidirectional_links(
            source_id=note_id,
            target_id=target_id,
            link_type=LinkType.RELATED
        )
        if success:
            created_count += 1

    return {
        "note_id": note_id,
        "links_created": created_count,
        "links_attempted": len(link_ids)
    }
