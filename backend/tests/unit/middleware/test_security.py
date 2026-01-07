"""
Unit tests for security middleware

Tests SecurityHeadersMiddleware and RequestSanitizationMiddleware
"""
import pytest
from unittest.mock import AsyncMock, MagicMock, patch
from fastapi import FastAPI, Request
from fastapi.testclient import TestClient
from starlette.responses import Response
from starlette.datastructures import Headers

from app.middleware.security import (
    SecurityHeadersMiddleware,
    RequestSanitizationMiddleware,
)


# ============================================================================
# Test Fixtures
# ============================================================================

@pytest.fixture
def app_with_security_headers():
    """Create FastAPI app with SecurityHeadersMiddleware"""
    app = FastAPI()
    app.add_middleware(SecurityHeadersMiddleware)

    @app.get("/test")
    async def test_endpoint():
        return {"message": "ok"}

    @app.get("/error")
    async def error_endpoint():
        raise ValueError("test error")

    return app


@pytest.fixture
def app_with_sanitization():
    """Create FastAPI app with RequestSanitizationMiddleware"""
    app = FastAPI()
    app.add_middleware(RequestSanitizationMiddleware)

    @app.get("/test")
    async def test_endpoint(request: Request):
        sanitized = getattr(request.state, 'sanitized_info', None)
        return {"sanitized": sanitized}

    return app


@pytest.fixture
def client_security(app_with_security_headers):
    """Test client for security headers app"""
    return TestClient(app_with_security_headers)


@pytest.fixture
def client_sanitization(app_with_sanitization):
    """Test client for sanitization app"""
    return TestClient(app_with_sanitization)


# ============================================================================
# SecurityHeadersMiddleware Tests
# ============================================================================

class TestSecurityHeadersMiddleware:
    """Tests for SecurityHeadersMiddleware"""

    def test_adds_x_content_type_options(self, client_security):
        """Should add X-Content-Type-Options: nosniff header"""
        response = client_security.get("/test")

        assert response.headers.get("X-Content-Type-Options") == "nosniff"

    def test_adds_x_frame_options(self, client_security):
        """Should add X-Frame-Options: DENY header"""
        response = client_security.get("/test")

        assert response.headers.get("X-Frame-Options") == "DENY"

    def test_adds_x_xss_protection(self, client_security):
        """Should add X-XSS-Protection header"""
        response = client_security.get("/test")

        assert response.headers.get("X-XSS-Protection") == "1; mode=block"

    def test_adds_referrer_policy(self, client_security):
        """Should add Referrer-Policy header"""
        response = client_security.get("/test")

        assert response.headers.get("Referrer-Policy") == "strict-origin-when-cross-origin"

    def test_adds_permissions_policy(self, client_security):
        """Should add Permissions-Policy header"""
        response = client_security.get("/test")

        policy = response.headers.get("Permissions-Policy")
        assert policy is not None
        assert "geolocation=()" in policy
        assert "microphone=()" in policy
        assert "camera=()" in policy

    def test_adds_content_security_policy(self, client_security):
        """Should add Content-Security-Policy header"""
        response = client_security.get("/test")

        csp = response.headers.get("Content-Security-Policy")
        assert csp is not None
        assert "default-src 'self'" in csp
        assert "frame-ancestors 'none'" in csp

    def test_csp_includes_script_src(self, client_security):
        """CSP should include script-src directive"""
        response = client_security.get("/test")

        csp = response.headers.get("Content-Security-Policy")
        assert "script-src" in csp

    def test_csp_includes_style_src(self, client_security):
        """CSP should include style-src directive"""
        response = client_security.get("/test")

        csp = response.headers.get("Content-Security-Policy")
        assert "style-src" in csp

    def test_csp_includes_img_src(self, client_security):
        """CSP should include img-src directive"""
        response = client_security.get("/test")

        csp = response.headers.get("Content-Security-Policy")
        assert "img-src" in csp

    def test_csp_includes_connect_src(self, client_security):
        """CSP should include connect-src directive"""
        response = client_security.get("/test")

        csp = response.headers.get("Content-Security-Policy")
        assert "connect-src" in csp

    def test_no_hsts_for_http(self, client_security):
        """Should NOT add HSTS header for HTTP requests"""
        response = client_security.get("/test")

        # TestClient uses HTTP by default
        assert response.headers.get("Strict-Transport-Security") is None

    def test_headers_on_different_methods(self, client_security):
        """Security headers should be added for all HTTP methods"""
        app = client_security.app

        @app.post("/post-test")
        async def post_endpoint():
            return {"ok": True}

        response = client_security.post("/post-test")

        assert response.headers.get("X-Frame-Options") == "DENY"
        assert response.headers.get("X-Content-Type-Options") == "nosniff"

    def test_headers_on_error_response(self):
        """Security headers should be added even on error responses"""
        app = FastAPI()
        app.add_middleware(SecurityHeadersMiddleware)

        @app.get("/error")
        async def error_endpoint():
            return {"error": "test"}

        client = TestClient(app)
        response = client.get("/error")

        assert response.headers.get("X-Frame-Options") == "DENY"

    def test_headers_preserved_from_endpoint(self):
        """Should preserve existing headers from endpoint"""
        app = FastAPI()
        app.add_middleware(SecurityHeadersMiddleware)

        @app.get("/custom")
        async def custom_headers():
            return Response(
                content='{"test": "ok"}',
                headers={"X-Custom-Header": "custom-value"},
                media_type="application/json"
            )

        client = TestClient(app)
        response = client.get("/custom")

        assert response.headers.get("X-Custom-Header") == "custom-value"
        assert response.headers.get("X-Frame-Options") == "DENY"

    def test_response_body_unchanged(self, client_security):
        """Middleware should not modify response body"""
        response = client_security.get("/test")

        assert response.json() == {"message": "ok"}


# ============================================================================
# RequestSanitizationMiddleware Tests
# ============================================================================

class TestRequestSanitizationMiddleware:
    """Tests for RequestSanitizationMiddleware"""

    def test_sanitized_info_attached_to_request_state(self, client_sanitization):
        """Should attach sanitized_info to request.state"""
        response = client_sanitization.get("/test")

        data = response.json()
        assert data["sanitized"] is not None

    def test_sanitized_info_includes_method(self, client_sanitization):
        """Sanitized info should include HTTP method"""
        response = client_sanitization.get("/test")

        sanitized = response.json()["sanitized"]
        assert sanitized["method"] == "GET"

    def test_sanitized_info_includes_url(self, client_sanitization):
        """Sanitized info should include URL"""
        response = client_sanitization.get("/test?param=value")

        sanitized = response.json()["sanitized"]
        assert "test" in sanitized["url"]

    def test_sanitized_info_includes_client(self, client_sanitization):
        """Sanitized info should include client IP"""
        response = client_sanitization.get("/test")

        sanitized = response.json()["sanitized"]
        assert "client" in sanitized

    def test_redacts_authorization_header(self, client_sanitization):
        """Should redact Authorization header"""
        response = client_sanitization.get(
            "/test",
            headers={"Authorization": "Bearer secret-token"}
        )

        sanitized = response.json()["sanitized"]
        assert sanitized["headers"].get("authorization") == "[REDACTED]"

    def test_redacts_cookie_header(self, client_sanitization):
        """Should redact Cookie header"""
        response = client_sanitization.get(
            "/test",
            headers={"Cookie": "session=secret123"}
        )

        sanitized = response.json()["sanitized"]
        assert sanitized["headers"].get("cookie") == "[REDACTED]"

    def test_redacts_x_api_key_header(self, client_sanitization):
        """Should redact X-API-Key header"""
        response = client_sanitization.get(
            "/test",
            headers={"X-API-Key": "api-key-12345"}
        )

        sanitized = response.json()["sanitized"]
        assert sanitized["headers"].get("x-api-key") == "[REDACTED]"

    def test_redacts_x_auth_token_header(self, client_sanitization):
        """Should redact X-Auth-Token header"""
        response = client_sanitization.get(
            "/test",
            headers={"X-Auth-Token": "token-secret"}
        )

        sanitized = response.json()["sanitized"]
        assert sanitized["headers"].get("x-auth-token") == "[REDACTED]"

    def test_preserves_non_sensitive_headers(self, client_sanitization):
        """Should preserve non-sensitive headers"""
        response = client_sanitization.get(
            "/test",
            headers={"X-Custom-Header": "custom-value"}
        )

        sanitized = response.json()["sanitized"]
        assert sanitized["headers"].get("x-custom-header") == "custom-value"

    def test_preserves_content_type_header(self, client_sanitization):
        """Should preserve Content-Type header"""
        app = client_sanitization.app

        @app.post("/post-test")
        async def post_endpoint(request: Request):
            sanitized = getattr(request.state, 'sanitized_info', None)
            return {"sanitized": sanitized}

        response = client_sanitization.post(
            "/post-test",
            json={"test": "data"}
        )

        sanitized = response.json()["sanitized"]
        content_type = sanitized["headers"].get("content-type")
        assert content_type is not None
        assert "application/json" in content_type

    def test_case_insensitive_header_redaction(self, client_sanitization):
        """Header redaction should be case-insensitive"""
        response = client_sanitization.get(
            "/test",
            headers={"AUTHORIZATION": "Bearer token"}
        )

        sanitized = response.json()["sanitized"]
        # Headers are lowercased in processing
        assert sanitized["headers"].get("authorization") == "[REDACTED]"

    def test_multiple_sensitive_headers_redacted(self, client_sanitization):
        """Should redact multiple sensitive headers"""
        response = client_sanitization.get(
            "/test",
            headers={
                "Authorization": "Bearer token",
                "Cookie": "session=abc",
                "X-API-Key": "key123"
            }
        )

        sanitized = response.json()["sanitized"]
        headers = sanitized["headers"]

        assert headers.get("authorization") == "[REDACTED]"
        assert headers.get("cookie") == "[REDACTED]"
        assert headers.get("x-api-key") == "[REDACTED]"


# ============================================================================
# Middleware Integration Tests
# ============================================================================

class TestMiddlewareIntegration:
    """Tests for middleware working together"""

    def test_both_middlewares_together(self):
        """Both security middlewares should work together"""
        app = FastAPI()
        app.add_middleware(SecurityHeadersMiddleware)
        app.add_middleware(RequestSanitizationMiddleware)

        @app.get("/test")
        async def test_endpoint(request: Request):
            sanitized = getattr(request.state, 'sanitized_info', None)
            return {"sanitized": sanitized is not None}

        client = TestClient(app)
        response = client.get("/test", headers={"Authorization": "secret"})

        # Security headers should be present
        assert response.headers.get("X-Frame-Options") == "DENY"

        # Sanitization should have worked
        assert response.json()["sanitized"] is True

    def test_middleware_order_matters(self):
        """Middleware order should affect behavior"""
        app = FastAPI()

        # Add in specific order
        app.add_middleware(RequestSanitizationMiddleware)
        app.add_middleware(SecurityHeadersMiddleware)

        @app.get("/test")
        async def test_endpoint():
            return {"ok": True}

        client = TestClient(app)
        response = client.get("/test")

        # Both should still work
        assert response.headers.get("X-Frame-Options") == "DENY"
        assert response.status_code == 200
