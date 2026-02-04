"""Integration tests for Feedback API endpoints"""
import pytest
from unittest.mock import AsyncMock, MagicMock

from app.models.feedback import FeedbackType


@pytest.mark.integration
class TestFeedbackStatus:
    """Tests for GET /api/feedback/status"""

    def test_returns_200(self, test_client):
        response = test_client.get("/api/feedback/status")
        assert response.status_code == 200

    def test_returns_configured_status(self, test_client):
        data = test_client.get("/api/feedback/status").json()
        assert "configured" in data
        assert "message" in data
        assert data["configured"] is True


@pytest.mark.integration
class TestSubmitFeedback:
    """Tests for POST /api/feedback"""

    def test_submit_bug_report(self, test_client):
        """Submit should create a GitHub issue (mocked)"""
        # Mock the GitHub API call inside the feedback service
        mock_response = MagicMock()
        mock_response.status_code = 201
        mock_response.json.return_value = {
            "number": 99,
            "html_url": "https://github.com/test/repo/issues/99",
        }

        mock_client = AsyncMock()
        mock_client.post.return_value = mock_response
        mock_client.is_closed = False

        # Inject mock client
        test_client.app.state.feedback_service._client = mock_client

        response = test_client.post(
            "/api/feedback",
            json={
                "title": "Test Bug Report Title",
                "description": "This is a detailed bug description for testing",
                "feedback_type": "bug",
            },
        )
        assert response.status_code == 201
        data = response.json()
        assert data["success"] is True
        assert data["issue_number"] == 99

    def test_submit_feature_request(self, test_client):
        """Feature request should also succeed"""
        mock_response = MagicMock()
        mock_response.status_code = 201
        mock_response.json.return_value = {
            "number": 100,
            "html_url": "https://github.com/test/repo/issues/100",
        }

        mock_client = AsyncMock()
        mock_client.post.return_value = mock_response
        mock_client.is_closed = False
        test_client.app.state.feedback_service._client = mock_client

        response = test_client.post(
            "/api/feedback",
            json={
                "title": "Feature: Dark mode support",
                "description": "Please add dark mode to the application for better UX",
                "feedback_type": "feature",
            },
        )
        assert response.status_code == 201
        assert response.json()["success"] is True

    def test_submit_validation_error_short_title(self, test_client):
        """Title too short should fail validation"""
        response = test_client.post(
            "/api/feedback",
            json={
                "title": "Hi",
                "description": "This is a detailed description that is long enough",
                "feedback_type": "general",
            },
        )
        assert response.status_code == 422

    def test_submit_validation_error_short_description(self, test_client):
        """Description too short should fail validation"""
        response = test_client.post(
            "/api/feedback",
            json={
                "title": "Valid Title Here",
                "description": "Short",
                "feedback_type": "general",
            },
        )
        assert response.status_code == 422

    def test_submit_with_system_info(self, test_client):
        """Feedback with system info should include it in the issue"""
        mock_response = MagicMock()
        mock_response.status_code = 201
        mock_response.json.return_value = {
            "number": 101,
            "html_url": "https://github.com/test/repo/issues/101",
        }

        mock_client = AsyncMock()
        mock_client.post.return_value = mock_response
        mock_client.is_closed = False
        test_client.app.state.feedback_service._client = mock_client

        response = test_client.post(
            "/api/feedback",
            json={
                "title": "Bug with system info included",
                "description": "This bug happens on Windows when using canvas feature",
                "feedback_type": "bug",
                "include_system_info": True,
                "system_info": {
                    "platform": "Windows 11",
                    "app_version": "1.0.0",
                    "runtime": "python-fastapi",
                    "current_page": "/canvas",
                },
            },
        )
        assert response.status_code == 201
        assert response.json()["success"] is True
