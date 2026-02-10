"""Feedback API router for bug reports and feature requests"""

from fastapi import APIRouter, Request
from app.models.feedback import FeedbackCreate, FeedbackResponse, FeedbackStatus
from app.middleware.rate_limit import limiter
from app.utils.dependencies import get_feedback_service

router = APIRouter()


@router.post("", response_model=FeedbackResponse, status_code=201)
@limiter.limit("5 per hour")
async def submit_feedback(feedback: FeedbackCreate, request: Request):
    """
    Submit feedback (bug report, feature request, or general feedback).

    This creates a GitHub issue in the configured repository.
    Rate limited to 5 submissions per hour per IP.
    """
    service = get_feedback_service(request)
    return await service.submit(feedback)


@router.get("/status", response_model=FeedbackStatus)
async def get_feedback_status(request: Request):
    """
    Get the status of the feedback service.

    Returns whether the service is configured and ready to accept feedback.
    """
    service = get_feedback_service(request)
    return service.get_status()
