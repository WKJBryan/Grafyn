"""Shared dependency injection utilities for accessing app.state services.

All services are initialized as singletons in main.py lifespan and attached to app.state.
These helper functions provide typed access to services from route handlers.

Usage:
    from app.utils import get_knowledge_store, get_vector_search

    @router.get("/notes")
    async def list_notes(request: Request):
        knowledge_store = get_knowledge_store(request)
        return knowledge_store.list_notes()
"""

from typing import Optional
from fastapi import Request

from app.services.knowledge_store import KnowledgeStore
from app.services.vector_search import VectorSearchService
from app.services.graph_index import GraphIndexService
from app.services.openrouter import OpenRouterService
from app.services.canvas_store import CanvasSessionStore
from app.services.priority_scoring import PriorityScoringService
from app.services.priority_settings import PrioritySettingsService
from app.services.distillation import DistillationService
from app.services.link_discovery import LinkDiscoveryService
from app.services.import_service import ImportService
from app.services.feedback import FeedbackService
from app.services.memory import MemoryService


def get_knowledge_store(request: Request) -> KnowledgeStore:
    """Get knowledge store singleton from app state."""
    return request.app.state.knowledge_store


def get_vector_search(request: Request) -> VectorSearchService:
    """Get vector search service singleton from app state."""
    return request.app.state.vector_search


def get_graph_index(request: Request) -> GraphIndexService:
    """Get graph index service singleton from app state."""
    return request.app.state.graph_index


def get_openrouter(request: Request) -> OpenRouterService:
    """Get OpenRouter service singleton from app state."""
    return request.app.state.openrouter


def get_canvas_store(request: Request) -> CanvasSessionStore:
    """Get canvas session store singleton from app state."""
    return request.app.state.canvas_store


def get_priority_scoring(request: Request) -> Optional[PriorityScoringService]:
    """Get priority scoring service singleton from app state (optional)."""
    return getattr(request.app.state, 'priority_scoring', None)


def get_priority_settings(request: Request) -> PrioritySettingsService:
    """Get priority settings service singleton from app state."""
    return request.app.state.priority_settings


def get_distillation(request: Request) -> DistillationService:
    """Get distillation service singleton from app state."""
    return request.app.state.distillation


def get_link_discovery(request: Request) -> LinkDiscoveryService:
    """Get link discovery service singleton from app state."""
    return request.app.state.link_discovery


def get_import_service(request: Request) -> ImportService:
    """Get import service singleton from app state."""
    return request.app.state.import_service


def get_feedback_service(request: Request) -> FeedbackService:
    """Get feedback service singleton from app state."""
    return request.app.state.feedback_service


def get_memory_service(request: Request) -> MemoryService:
    """Get memory service singleton from app state."""
    return request.app.state.memory_service
