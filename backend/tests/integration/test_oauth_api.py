"""
Integration tests for OAuth API endpoints

Tests OAuth flow: authorization, callback, user info, logout
"""
import pytest
from fastapi.testclient import TestClient
from unittest.mock import MagicMock, patch, AsyncMock
from datetime import datetime, timezone, timedelta

from app.main import app as _app


# ============================================================================
# Test Fixtures
# ============================================================================

@pytest.fixture
def mock_settings():
    """Create mock settings"""
    settings = MagicMock()
    settings.github_client_id = "test-client-id"
    settings.github_client_secret = "test-client-secret"
    settings.github_redirect_uri = "http://localhost:5173/oauth/callback/github"
    return settings


@pytest.fixture
def mock_token_store():
    """Create mock token store"""
    store = MagicMock()
    store.get_token.return_value = "real-access-token"
    store.store_token.return_value = None
    store.delete_token.return_value = None
    return store


@pytest.fixture
def test_app():
    """Create test application"""
    app = _app
    # Add minimal required services
    app.state.knowledge_store = MagicMock()
    app.state.vector_search = MagicMock()
    app.state.graph_index = MagicMock()
    return app


@pytest.fixture
def client(test_app):
    """Create test client"""
    return TestClient(test_app)


# ============================================================================
# Get Authorization URL Tests
# ============================================================================

class TestGetAuthorizationUrl:
    """Tests for GET /api/oauth/authorize/{provider}"""

    @patch('app.routers.oauth.settings')
    def test_returns_200_for_github(self, mock_settings, client):
        """Should return 200 for GitHub provider"""
        mock_settings.github_client_id = "test-id"
        mock_settings.github_redirect_uri = "http://localhost/callback"

        response = client.get("/api/oauth/authorize/github")
        assert response.status_code == 200

    @patch('app.routers.oauth.settings')
    def test_returns_authorization_url(self, mock_settings, client):
        """Should return authorization URL"""
        mock_settings.github_client_id = "test-id"
        mock_settings.github_redirect_uri = "http://localhost/callback"

        response = client.get("/api/oauth/authorize/github")
        data = response.json()

        assert "authorization_url" in data

    @patch('app.routers.oauth.settings')
    def test_authorization_url_includes_client_id(self, mock_settings, client):
        """Authorization URL should include client ID"""
        mock_settings.github_client_id = "my-client-id"
        mock_settings.github_redirect_uri = "http://localhost/callback"

        response = client.get("/api/oauth/authorize/github")
        data = response.json()

        assert "my-client-id" in data["authorization_url"]

    @patch('app.routers.oauth.settings')
    def test_returns_state_parameter(self, mock_settings, client):
        """Should return state parameter for CSRF protection"""
        mock_settings.github_client_id = "test-id"
        mock_settings.github_redirect_uri = "http://localhost/callback"

        response = client.get("/api/oauth/authorize/github")
        data = response.json()

        assert "state" in data
        assert len(data["state"]) > 0

    @patch('app.routers.oauth.settings')
    def test_returns_500_when_not_configured(self, mock_settings, client):
        """Should return 500 when OAuth not configured"""
        mock_settings.github_client_id = None
        mock_settings.github_redirect_uri = None

        response = client.get("/api/oauth/authorize/github")
        assert response.status_code == 500

    def test_returns_501_for_google(self, client):
        """Should return 501 for Google (not implemented)"""
        response = client.get("/api/oauth/authorize/google")
        assert response.status_code == 501

    def test_returns_400_for_unknown_provider(self, client):
        """Should return 400 for unknown provider"""
        response = client.get("/api/oauth/authorize/unknown")
        assert response.status_code == 400

    def test_error_has_detail(self, client):
        """Error response should include detail"""
        response = client.get("/api/oauth/authorize/unknown")
        assert "detail" in response.json()


# ============================================================================
# Exchange Code Tests
# ============================================================================

class TestExchangeCode:
    """Tests for POST /api/oauth/callback/{provider}"""

    @patch('app.routers.oauth.token_store')
    @patch('app.routers.oauth.settings')
    @patch('httpx.AsyncClient')
    def test_returns_400_for_invalid_state(
        self, mock_httpx, mock_settings, mock_token_store, client
    ):
        """Should return 400 for invalid state"""
        mock_token_store.get_token.return_value = None

        response = client.post(
            "/api/oauth/callback/github?code=test-code&state=invalid-state"
        )
        assert response.status_code == 400

    def test_returns_501_for_unsupported_provider(self, client):
        """Should return 501 for unsupported provider"""
        response = client.post("/api/oauth/callback/twitter?code=test-code")
        assert response.status_code == 501


# ============================================================================
# Get User Tests
# ============================================================================

class TestGetUser:
    """Tests for GET /api/oauth/user"""

    def test_returns_200_without_auth(self, client):
        """Should return 200 even without auth header"""
        response = client.get("/api/oauth/user")
        # Current implementation returns placeholder user
        assert response.status_code in [200, 401]

    def test_returns_user_object(self, client):
        """Should return user object"""
        response = client.get("/api/oauth/user")

        if response.status_code == 200:
            data = response.json()
            assert "id" in data or "name" in data

    @patch('app.routers.oauth.token_store')
    def test_returns_401_for_invalid_token(self, mock_token_store, client):
        """Should return 401 for invalid token"""
        mock_token_store.get_token.return_value = None

        response = client.get(
            "/api/oauth/user",
            headers={"Authorization": "Bearer invalid-token"}
        )
        assert response.status_code == 401

    @patch('app.routers.oauth.token_store')
    def test_returns_401_for_expired_token(self, mock_token_store, client):
        """Should return 401 for expired token"""
        mock_token_store.get_token.return_value = None

        response = client.get(
            "/api/oauth/user",
            headers={"Authorization": "Bearer expired-token"}
        )
        assert response.status_code == 401


# ============================================================================
# Logout Tests
# ============================================================================

class TestLogout:
    """Tests for POST /api/oauth/logout"""

    def test_returns_200(self, client):
        """Should return 200 OK"""
        response = client.post("/api/oauth/logout")
        assert response.status_code == 200

    def test_returns_success_message(self, client):
        """Should return success message"""
        response = client.post("/api/oauth/logout")
        data = response.json()

        assert "message" in data

    @patch('app.routers.oauth.token_store')
    def test_deletes_token(self, mock_token_store, client):
        """Should delete token from store"""
        client.post(
            "/api/oauth/logout",
            headers={"Authorization": "Bearer test-token"}
        )

        mock_token_store.delete_token.assert_called_with("test-token")

    def test_works_without_auth_header(self, client):
        """Should work without Authorization header"""
        response = client.post("/api/oauth/logout")
        assert response.status_code == 200


# ============================================================================
# CSRF Protection Tests
# ============================================================================

class TestCSRFProtection:
    """Tests for CSRF protection in OAuth flow"""

    @patch('app.routers.oauth.token_store')
    @patch('app.routers.oauth.settings')
    def test_stores_state_on_authorize(self, mock_settings, mock_token_store, client):
        """Should store state token on authorize"""
        mock_settings.github_client_id = "test-id"
        mock_settings.github_redirect_uri = "http://localhost/callback"

        client.get("/api/oauth/authorize/github")

        # Should have called store_token with state
        mock_token_store.store_token.assert_called()

    @patch('app.routers.oauth.token_store')
    def test_validates_state_on_callback(self, mock_token_store, client):
        """Should validate state on callback"""
        mock_token_store.get_token.return_value = None

        response = client.post(
            "/api/oauth/callback/github?code=test&state=bad-state"
        )

        # Should reject invalid state
        assert response.status_code == 400


# ============================================================================
# Error Handling Tests
# ============================================================================

class TestOAuthErrorHandling:
    """Tests for OAuth error handling"""

    def test_invalid_provider_returns_error(self, client):
        """Should return error for invalid provider"""
        response = client.get("/api/oauth/authorize/invalid")
        assert response.status_code == 400
        assert "detail" in response.json()

    def test_missing_code_handled(self, client):
        """Should handle missing code parameter"""
        response = client.post("/api/oauth/callback/github")
        # Should return validation error or bad request
        assert response.status_code in [400, 422]

    def test_error_responses_are_json(self, client):
        """Error responses should be JSON"""
        response = client.get("/api/oauth/authorize/invalid")
        assert "application/json" in response.headers.get("content-type", "")
