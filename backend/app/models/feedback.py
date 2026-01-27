"""Feedback models for bug reports and feature requests"""

from datetime import datetime
from enum import Enum
from typing import Optional, List
from pydantic import BaseModel, Field, field_validator


class FeedbackType(str, Enum):
    """Type of feedback being submitted"""

    BUG = "bug"
    FEATURE = "feature"
    GENERAL = "general"


class SystemInfo(BaseModel):
    """System information collected with feedback (opt-in)"""

    platform: str = Field(description="Operating system name and version")
    app_version: str = Field(description="Application version")
    runtime: str = Field(default="web-browser", description="Runtime environment")
    current_page: Optional[str] = Field(default=None, description="Current page/view in the app")


class FeedbackCreate(BaseModel):
    """Request to create new feedback"""

    title: str = Field(
        min_length=5,
        max_length=200,
        description="Short summary of the feedback (5-200 chars)",
    )
    description: str = Field(
        min_length=10,
        max_length=10000,
        description="Detailed description (10-10000 chars)",
    )
    feedback_type: FeedbackType = Field(
        default=FeedbackType.GENERAL, description="Type of feedback"
    )
    include_system_info: bool = Field(
        default=False, description="Whether to include system information"
    )
    system_info: Optional[SystemInfo] = Field(
        default=None, description="System information (populated if include_system_info is true)"
    )

    @field_validator("title", "description")
    @classmethod
    def strip_whitespace(cls, v: str) -> str:
        return v.strip()


class FeedbackResponse(BaseModel):
    """Response after submitting feedback"""

    success: bool = Field(description="Whether submission was successful")
    issue_number: Optional[int] = Field(
        default=None, description="GitHub issue number (if created)"
    )
    issue_url: Optional[str] = Field(
        default=None, description="GitHub issue URL (if created)"
    )
    message: str = Field(description="User-friendly message")
    queued: bool = Field(default=False, description="Whether feedback was queued for later")

    @classmethod
    def success_response(cls, issue_number: int, issue_url: str) -> "FeedbackResponse":
        """Create a success response with GitHub issue details"""
        return cls(
            success=True,
            issue_number=issue_number,
            issue_url=issue_url,
            message=f"Feedback submitted successfully as issue #{issue_number}",
            queued=False,
        )

    @classmethod
    def error_response(cls, message: str) -> "FeedbackResponse":
        """Create an error response"""
        return cls(success=False, message=message, queued=False)

    @classmethod
    def queued_response(cls) -> "FeedbackResponse":
        """Create a queued response for offline mode"""
        return cls(
            success=True,
            message="Feedback queued for later submission",
            queued=True,
        )


class FeedbackStatus(BaseModel):
    """Status of the feedback service"""

    configured: bool = Field(description="Whether the service is properly configured")
    pending_count: int = Field(
        default=0, description="Number of pending feedback items (offline queue)"
    )
    message: str = Field(description="User-friendly status message")
