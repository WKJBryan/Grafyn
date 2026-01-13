"""Priority management API router"""
from fastapi import APIRouter, Request, HTTPException
from typing import Dict, Any

from backend.app.services.priority_scoring import PriorityWeights, ContentType
from backend.app.services.priority_settings import PrioritySettingsService

router = APIRouter()


def get_priority_settings_service(request: Request) -> PrioritySettingsService:
    """Get priority settings service from app state"""
    return request.app.state.priority_settings


@router.get("/config", response_model=Dict[str, Any])
async def get_priority_config(
    request: Request
):
    """
    Get current priority configuration
    
    Returns complete priority scoring configuration including:
    - Content type base scores
    - Recency decay settings
    - Link density boost settings
    - Tag relevance boost settings
    - Semantic similarity weight
    """
    service = get_priority_settings_service(request)
    return service.get_full_config()


@router.get("/weights", response_model=PriorityWeights)
async def get_priority_weights(
    request: Request
):
    """
    Get current priority weights
    
    Returns the complete PriorityWeights configuration
    """
    service = get_priority_settings_service(request)
    return service.get_weights()


@router.put("/weights", response_model=PriorityWeights)
async def update_priority_weights(
    request: Request,
    weights: PriorityWeights
):
    """
    Update priority weights
    
    Allows customization of priority scoring behavior:
    - Content type base scores (claim, decision, insight, etc.)
    - Recency decay rate (0.0-1.0)
    - Link density boost per link
    - Tag relevance boost per match
    - Semantic similarity weight multiplier
    
    All weights must be non-negative. Decay rate must be between 0.0 and 1.0.
    """
    service = get_priority_settings_service(request)
    
    # Validate weights
    if weights.recency_decay < 0.0 or weights.recency_decay > 1.0:
        raise HTTPException(
            status_code=400,
            detail="Recency decay must be between 0.0 and 1.0"
        )
    
    if weights.semantic_weight < 0.0:
        raise HTTPException(
            status_code=400,
            detail="Semantic weight must be non-negative"
        )
    
    # Update weights
    updated_weights = service.update_weights(weights)
    
    # Update priority scoring service if available
    if hasattr(request.app.state, 'priority_scoring'):
        request.app.state.priority_scoring.update_weights(updated_weights)
    
    return updated_weights


@router.post("/reset", response_model=PriorityWeights)
async def reset_priority_weights(
    request: Request
):
    """
    Reset priority weights to default values
    
    Restores all priority weights to their default configuration:
    - claim: 100.0
    - decision: 90.0
    - insight: 80.0
    - question: 70.0
    - evidence: 60.0
    - general: 50.0
    - recency_decay: 0.10
    - link_density_boost: 5.0
    - tag_relevance_boost: 15.0
    - semantic_weight: 1.0
    """
    service = get_priority_settings_service(request)
    default_weights = service.reset_to_defaults()
    
    # Update priority scoring service if available
    if hasattr(request.app.state, 'priority_scoring'):
        request.app.state.priority_scoring.update_weights(default_weights)
    
    return default_weights


@router.get("/content-types", response_model=Dict[str, float])
async def get_content_type_scores(
    request: Request
):
    """
    Get content type base scores
    
    Returns the base priority score for each content type:
    - claim: Highest priority
    - decision: High priority
    - insight: Medium-high priority
    - question: Medium priority
    - evidence: Medium-low priority
    - general: Lowest priority
    """
    service = get_priority_settings_service(request)
    return service.get_content_type_scores()


@router.get("/recency", response_model=Dict[str, Any])
async def get_recency_config(
    request: Request
):
    """
    Get recency decay configuration
    
    Returns:
    - decay_rate: Percentage score decay per month (0.0-1.0)
    - description: Human-readable explanation
    """
    service = get_priority_settings_service(request)
    return service.get_recency_config()


@router.get("/link-density", response_model=Dict[str, Any])
async def get_link_density_config(
    request: Request
):
    """
    Get link density boost configuration
    
    Returns:
    - boost_per_link: Score boost per link
    - description: Human-readable explanation
    """
    service = get_priority_settings_service(request)
    return service.get_link_density_config()


@router.get("/tag-relevance", response_model=Dict[str, Any])
async def get_tag_relevance_config(
    request: Request
):
    """
    Get tag relevance boost configuration
    
    Returns:
    - boost_per_match: Score boost per matching tag
    - description: Human-readable explanation
    """
    service = get_priority_settings_service(request)
    return service.get_tag_relevance_config()


@router.get("/semantic", response_model=Dict[str, Any])
async def get_semantic_config(
    request: Request
):
    """
    Get semantic similarity weight configuration
    
    Returns:
    - weight: Multiplier for semantic similarity score
    - description: Human-readable explanation
    """
    service = get_priority_settings_service(request)
    return service.get_semantic_config()
