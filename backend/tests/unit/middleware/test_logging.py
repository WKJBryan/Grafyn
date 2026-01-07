"""
Unit tests for logging middleware

Tests LoggingMiddleware for request/response logging
"""
import pytest
from unittest.mock import patch, MagicMock
import time
from fastapi import FastAPI, Request
from fastapi.testclient import TestClient

from app.middleware.logging import LoggingMiddleware


# ============================================================================
# Test Fixtures
# ============================================================================

@pytest.fixture
def app_with_logging():
    """Create FastAPI app with LoggingMiddleware"""
    app = FastAPI()
    app.add_middleware(LoggingMiddleware)

    @app.get("/test")
    async def test_endpoint():
        return {"message": "ok"}

    @app.get("/slow")
    async def slow_endpoint():
        time.sleep(0.1)
        return {"message": "slow"}

    @app.post("/post-test")
    async def post_endpoint():
        return {"posted": True}

    @app.get("/error")
    async def error_endpoint():
        raise ValueError("test error")

    return app


@pytest.fixture
def client(app_with_logging):
    """Test client for logging app"""
    return TestClient(app_with_logging, raise_server_exceptions=False)


# ============================================================================
# Request Logging Tests
# ============================================================================

class TestRequestLogging:
    """Tests for request logging"""

    def test_logs_request_method(self, client):
        """Should log HTTP method"""
        with patch('app.middleware.logging.logger') as mock_logger:
            client.get("/test")

            calls = [str(call) for call in mock_logger.info.call_args_list]
            request_log = [c for c in calls if "Request" in c or "GET" in c]
            assert len(request_log) > 0

    def test_logs_request_path(self, client):
        """Should log request path"""
        with patch('app.middleware.logging.logger') as mock_logger:
            client.get("/test")

            calls = [str(call) for call in mock_logger.info.call_args_list]
            assert any("/test" in c for c in calls)

    def test_logs_post_requests(self, client):
        """Should log POST requests"""
        with patch('app.middleware.logging.logger') as mock_logger:
            client.post("/post-test", json={})

            calls = [str(call) for call in mock_logger.info.call_args_list]
            assert any("POST" in c for c in calls)

    def test_uses_sanitized_info_when_available(self):
        """Should use sanitized_info from request state when available"""
        from app.middleware.security import RequestSanitizationMiddleware

        app = FastAPI()
        # Add sanitization first, then logging
        app.add_middleware(LoggingMiddleware)
        app.add_middleware(RequestSanitizationMiddleware)

        @app.get("/test")
        async def test_endpoint():
            return {"ok": True}

        client = TestClient(app)

        with patch('app.middleware.logging.logger') as mock_logger:
            client.get("/test")

            # Should have logged using sanitized info
            mock_logger.info.assert_called()


# ============================================================================
# Response Logging Tests
# ============================================================================

class TestResponseLogging:
    """Tests for response logging"""

    def test_logs_response_status(self, client):
        """Should log response status code"""
        with patch('app.middleware.logging.logger') as mock_logger:
            client.get("/test")

            calls = [str(call) for call in mock_logger.info.call_args_list]
            assert any("200" in c or "Status" in c for c in calls)

    def test_logs_404_status(self, client):
        """Should log 404 status for missing endpoints"""
        with patch('app.middleware.logging.logger') as mock_logger:
            client.get("/nonexistent")

            calls = [str(call) for call in mock_logger.info.call_args_list]
            assert any("404" in c for c in calls)

    def test_logs_process_time(self, client):
        """Should log process time"""
        with patch('app.middleware.logging.logger') as mock_logger:
            client.get("/test")

            calls = [str(call) for call in mock_logger.info.call_args_list]
            assert any("Time" in c or "ms" in c for c in calls)


# ============================================================================
# Timing Tests
# ============================================================================

class TestTimingHeader:
    """Tests for X-Process-Time header"""

    def test_adds_process_time_header(self, client):
        """Should add X-Process-Time header"""
        response = client.get("/test")

        assert "X-Process-Time" in response.headers

    def test_process_time_is_numeric(self, client):
        """X-Process-Time should be a numeric value"""
        response = client.get("/test")

        process_time = response.headers.get("X-Process-Time")
        assert float(process_time) >= 0

    def test_slow_request_has_higher_time(self, client):
        """Slow requests should have higher process time"""
        fast_response = client.get("/test")
        slow_response = client.get("/slow")

        fast_time = float(fast_response.headers.get("X-Process-Time"))
        slow_time = float(slow_response.headers.get("X-Process-Time"))

        # Slow endpoint should take longer (at least 50ms)
        assert slow_time > fast_time
        assert slow_time >= 50  # At least 50ms due to sleep(0.1)

    def test_process_time_in_milliseconds(self, client):
        """Process time should be in milliseconds"""
        response = client.get("/test")

        process_time = float(response.headers.get("X-Process-Time"))
        # Should be reasonable (less than 10 seconds = 10000ms for simple request)
        assert 0 < process_time < 10000


# ============================================================================
# Error Handling Tests
# ============================================================================

class TestErrorLogging:
    """Tests for error response logging"""

    def test_logs_error_responses(self, client):
        """Should log responses even when errors occur"""
        with patch('app.middleware.logging.logger') as mock_logger:
            client.get("/error")

            # Should still have logged something
            assert mock_logger.info.called

    def test_adds_header_on_error(self, client):
        """Should add X-Process-Time even on errors"""
        response = client.get("/error")

        # Even on error, header should be present
        assert "X-Process-Time" in response.headers


# ============================================================================
# Integration Tests
# ============================================================================

class TestLoggingIntegration:
    """Integration tests for logging middleware"""

    def test_does_not_modify_response_body(self, client):
        """Middleware should not modify response body"""
        response = client.get("/test")

        assert response.json() == {"message": "ok"}

    def test_does_not_affect_status_code(self, client):
        """Middleware should not affect status codes"""
        response = client.get("/test")
        assert response.status_code == 200

        response = client.get("/nonexistent")
        assert response.status_code == 404

    def test_multiple_requests_logged_separately(self, client):
        """Each request should be logged separately"""
        with patch('app.middleware.logging.logger') as mock_logger:
            client.get("/test")
            client.post("/post-test", json={})
            client.get("/test")

            # Should have at least 6 log calls (2 per request: request + response)
            assert mock_logger.info.call_count >= 6

    def test_concurrent_compatible(self, client):
        """Middleware should handle concurrent requests"""
        import concurrent.futures

        def make_request():
            return client.get("/test")

        with concurrent.futures.ThreadPoolExecutor(max_workers=5) as executor:
            futures = [executor.submit(make_request) for _ in range(10)]
            responses = [f.result() for f in futures]

        # All should succeed
        assert all(r.status_code == 200 for r in responses)
        assert all("X-Process-Time" in r.headers for r in responses)
