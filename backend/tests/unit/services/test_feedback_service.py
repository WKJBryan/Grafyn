"""Tests for FeedbackService"""
import pytest
from unittest.mock import AsyncMock, MagicMock, patch
import httpx

from app.services.feedback import FeedbackService
from app.models.feedback import (
    FeedbackCreate,
    FeedbackResponse,
    FeedbackStatus,
    FeedbackType,
    SystemInfo,
)


@pytest.mark.unit
class TestFeedbackServiceConfiguration:
    """Tests for service configuration and status"""

    def test_is_configured_true(self, feedback_service):
        """Service should be configured when repo and token are set"""
        assert feedback_service.is_configured() is True

    def test_is_configured_false_no_repo(self):
        """Service should not be configured without repo"""
        service = FeedbackService()
        service.repo = ""
        service.token = "some-token"
        assert service.is_configured() is False

    def test_is_configured_false_no_token(self):
        """Service should not be configured without token"""
        service = FeedbackService()
        service.repo = "owner/repo"
        service.token = ""
        assert service.is_configured() is False

    def test_get_status_configured(self, feedback_service):
        """get_status should report configured when properly set up"""
        status = feedback_service.get_status()
        assert isinstance(status, FeedbackStatus)
        assert status.configured is True
        assert "ready" in status.message.lower()

    def test_get_status_not_configured(self):
        """get_status should report not configured when missing config"""
        service = FeedbackService()
        service.repo = ""
        service.token = ""
        status = service.get_status()
        assert status.configured is False
        assert "not configured" in status.message.lower()

    def test_get_system_info(self, feedback_service):
        """get_system_info should return SystemInfo with platform and version"""
        info = feedback_service.get_system_info(current_page="/canvas")
        assert isinstance(info, SystemInfo)
        assert info.current_page == "/canvas"
        assert info.runtime == "python-fastapi"


@pytest.mark.unit
class TestFeedbackSubmission:
    """Tests for feedback submission to GitHub"""

    @pytest.mark.asyncio
    async def test_submit_not_configured(self):
        """submit should return error when not configured"""
        service = FeedbackService()
        service.repo = ""
        service.token = ""
        feedback = FeedbackCreate(
            title="Test Bug Report",
            description="Something is broken and needs fixing",
            feedback_type=FeedbackType.BUG,
        )
        result = await service.submit(feedback)
        assert result.success is False
        assert "not configured" in result.message.lower()

    @pytest.mark.asyncio
    async def test_submit_success(self, feedback_service, mock_github_api):
        """submit should create GitHub issue and return success"""
        mock_response = MagicMock()
        mock_response.status_code = 201
        mock_response.json.return_value = mock_github_api["success"]

        mock_client = AsyncMock()
        mock_client.post.return_value = mock_response
        mock_client.is_closed = False
        feedback_service._client = mock_client

        feedback = FeedbackCreate(
            title="Test Bug Report",
            description="Something is broken and needs fixing",
            feedback_type=FeedbackType.BUG,
        )
        result = await feedback_service.submit(feedback)
        assert result.success is True
        assert result.issue_number == 42

    @pytest.mark.asyncio
    async def test_submit_github_error(self, feedback_service):
        """submit should handle GitHub API errors gracefully"""
        mock_response = MagicMock()
        mock_response.status_code = 401
        mock_response.text = '{"message": "Bad credentials"}'

        mock_client = AsyncMock()
        mock_client.post.return_value = mock_response
        mock_client.is_closed = False
        feedback_service._client = mock_client

        feedback = FeedbackCreate(
            title="Test Feature Request",
            description="Add a really cool new feature please",
            feedback_type=FeedbackType.FEATURE,
        )
        result = await feedback_service.submit(feedback)
        assert result.success is False
        assert "401" in result.message

    @pytest.mark.asyncio
    async def test_submit_connection_error(self, feedback_service):
        """submit should handle connection errors gracefully"""
        mock_client = AsyncMock()
        mock_client.post.side_effect = httpx.ConnectError("Connection refused")
        mock_client.is_closed = False
        feedback_service._client = mock_client

        feedback = FeedbackCreate(
            title="Test feedback item",
            description="Some detailed description for testing",
            feedback_type=FeedbackType.GENERAL,
        )
        result = await feedback_service.submit(feedback)
        assert result.success is False
        assert "connect" in result.message.lower()


@pytest.mark.unit
class TestFeedbackFormatting:
    """Tests for issue body and label formatting"""

    def test_format_issue_body_bug(self, feedback_service):
        """Bug reports should include bug emoji and system info"""
        feedback = FeedbackCreate(
            title="Test Bug",
            description="Something broke",
            feedback_type=FeedbackType.BUG,
            include_system_info=True,
            system_info=SystemInfo(
                platform="Windows 11",
                app_version="1.0.0",
                runtime="python-fastapi",
                current_page="/notes",
            ),
        )
        body = feedback_service._format_issue_body(feedback)
        assert "🐛" in body
        assert "Bug Report" in body
        assert "Windows 11" in body
        assert "/notes" in body

    def test_get_labels_bug(self, feedback_service):
        """Bug type should get bug label"""
        labels = feedback_service._get_labels(FeedbackType.BUG)
        assert "bug" in labels
        assert "user-feedback" in labels

    def test_get_labels_feature(self, feedback_service):
        """Feature type should get enhancement label"""
        labels = feedback_service._get_labels(FeedbackType.FEATURE)
        assert "enhancement" in labels

    def test_get_labels_general(self, feedback_service):
        """General type should get feedback label"""
        labels = feedback_service._get_labels(FeedbackType.GENERAL)
        assert "feedback" in labels

    def test_format_issue_body_feature(self, feedback_service):
        """Feature requests should include lightbulb emoji"""
        feedback = FeedbackCreate(
            title="New Feature Idea",
            description="Add dark mode to the application",
            feedback_type=FeedbackType.FEATURE,
        )
        body = feedback_service._format_issue_body(feedback)
        assert "💡" in body
        assert "Feature Request" in body
