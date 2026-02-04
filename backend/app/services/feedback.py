"""Feedback service for submitting bug reports and feature requests to GitHub"""

import httpx
import logging
from typing import Optional
from app.config import get_settings
from app.models.feedback import (
    FeedbackCreate,
    FeedbackResponse,
    FeedbackStatus,
    FeedbackType,
    SystemInfo,
)

logger = logging.getLogger(__name__)

GITHUB_API_URL = "https://api.github.com"


class FeedbackService:
    """Service for handling feedback submission to GitHub Issues"""

    def __init__(self):
        settings = get_settings()
        self.repo = settings.github_feedback_repo
        self.token = settings.github_feedback_token
        self._client: Optional[httpx.AsyncClient] = None

    async def _get_client(self) -> httpx.AsyncClient:
        """Get or create HTTP client"""
        if self._client is None or self._client.is_closed:
            self._client = httpx.AsyncClient(
                timeout=httpx.Timeout(30.0),
                headers={
                    "User-Agent": "Grafyn-Backend",
                    "Accept": "application/vnd.github+json",
                    "X-GitHub-Api-Version": "2022-11-28",
                },
            )
        return self._client

    def is_configured(self) -> bool:
        """Check if the service is properly configured"""
        return bool(self.repo and self.token)

    def get_status(self) -> FeedbackStatus:
        """Get the current status of the feedback service"""
        if not self.is_configured():
            return FeedbackStatus(
                configured=False,
                pending_count=0,
                message="Feedback service not configured. Set GITHUB_FEEDBACK_REPO and GITHUB_FEEDBACK_TOKEN.",
            )

        return FeedbackStatus(
            configured=True,
            pending_count=0,  # Web backend doesn't support offline queue
            message="Feedback service ready",
        )

    def get_system_info(self, current_page: Optional[str] = None) -> SystemInfo:
        """Get system information for the feedback form"""
        return SystemInfo(
            platform="Web Browser",
            app_version="1.0.0",
            runtime="python-fastapi",
            current_page=current_page,
        )

    async def submit(self, feedback: FeedbackCreate) -> FeedbackResponse:
        """Submit feedback to GitHub Issues"""
        if not self.is_configured():
            return FeedbackResponse.error_response(
                "Feedback service not configured. Please set GITHUB_FEEDBACK_REPO and GITHUB_FEEDBACK_TOKEN."
            )

        try:
            # Add system info if requested but not provided
            if feedback.include_system_info and feedback.system_info is None:
                feedback.system_info = self.get_system_info()

            return await self._submit_to_github(feedback)

        except httpx.ConnectError:
            logger.warning("Failed to connect to GitHub API")
            return FeedbackResponse.error_response(
                "Unable to connect to GitHub. Please check your internet connection."
            )
        except httpx.TimeoutException:
            logger.warning("GitHub API request timed out")
            return FeedbackResponse.error_response(
                "Request to GitHub timed out. Please try again."
            )
        except Exception as e:
            logger.exception("Unexpected error submitting feedback")
            return FeedbackResponse.error_response(f"An unexpected error occurred: {str(e)}")

    async def _submit_to_github(self, feedback: FeedbackCreate) -> FeedbackResponse:
        """Submit feedback directly to GitHub Issues API"""
        if "/" not in self.repo:
            return FeedbackResponse.error_response(
                "Invalid repo format, expected 'owner/repo'"
            )

        owner, repo = self.repo.split("/", 1)
        client = await self._get_client()

        # Build issue body
        body = self._format_issue_body(feedback)

        # Determine labels based on feedback type
        labels = self._get_labels(feedback.feedback_type)

        request_data = {
            "title": feedback.title,
            "body": body,
            "labels": labels,
        }

        response = await client.post(
            f"{GITHUB_API_URL}/repos/{owner}/{repo}/issues",
            json=request_data,
            headers={"Authorization": f"Bearer {self.token}"},
        )

        if response.status_code == 201:
            data = response.json()
            return FeedbackResponse.success_response(
                issue_number=data["number"],
                issue_url=data["html_url"],
            )
        else:
            error_text = response.text
            logger.error(f"GitHub API error: {response.status_code} - {error_text}")
            return FeedbackResponse.error_response(
                f"Failed to create GitHub issue: {response.status_code}"
            )

    def _format_issue_body(self, feedback: FeedbackCreate) -> str:
        """Format the GitHub issue body with feedback details"""
        type_emoji = {
            FeedbackType.BUG: "🐛",
            FeedbackType.FEATURE: "💡",
            FeedbackType.GENERAL: "💬",
        }

        type_label = {
            FeedbackType.BUG: "Bug Report",
            FeedbackType.FEATURE: "Feature Request",
            FeedbackType.GENERAL: "General Feedback",
        }

        emoji = type_emoji.get(feedback.feedback_type, "💬")
        label = type_label.get(feedback.feedback_type, "General Feedback")

        body = f"## {emoji} {label}\n\n{feedback.description}\n\n"

        if feedback.system_info:
            body += "---\n\n"
            body += "### System Information\n\n"
            body += f"- **Platform:** {feedback.system_info.platform}\n"
            body += f"- **App Version:** {feedback.system_info.app_version}\n"
            body += f"- **Runtime:** {feedback.system_info.runtime}\n"
            if feedback.system_info.current_page:
                body += f"- **Current Page:** {feedback.system_info.current_page}\n"

        body += "\n---\n*Submitted via Grafyn Web App*"

        return body

    def _get_labels(self, feedback_type: FeedbackType) -> list[str]:
        """Get GitHub labels based on feedback type"""
        label_map = {
            FeedbackType.BUG: ["bug", "user-feedback"],
            FeedbackType.FEATURE: ["enhancement", "user-feedback"],
            FeedbackType.GENERAL: ["feedback", "user-feedback"],
        }
        return label_map.get(feedback_type, ["feedback", "user-feedback"])

    async def close(self):
        """Close the HTTP client"""
        if self._client and not self._client.is_closed:
            await self._client.aclose()
            self._client = None
