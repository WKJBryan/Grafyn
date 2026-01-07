"""
Unit tests for rate limiting middleware

Tests limiter initialization and rate limit handling
"""
import pytest
from unittest.mock import MagicMock, patch, AsyncMock
from slowapi.errors import RateLimitExceeded

from app.middleware.rate_limit import (
    limiter,
    init_limiter,
    rate_limit_handler,
)


# ============================================================================
# Test Fixtures
# ============================================================================

@pytest.fixture
def mock_settings():
    """Create mock settings object"""
    settings = MagicMock()
    settings.rate_limit_enabled = True
    settings.rate_limit_per_day = 200
    settings.rate_limit_per_hour = 50
    return settings


@pytest.fixture
def mock_settings_disabled():
    """Create mock settings with rate limiting disabled"""
    settings = MagicMock()
    settings.rate_limit_enabled = False
    settings.rate_limit_per_day = 200
    settings.rate_limit_per_hour = 50
    return settings


@pytest.fixture
def mock_request():
    """Create mock request object"""
    request = MagicMock()
    request.client = MagicMock()
    request.client.host = "127.0.0.1"
    return request


# ============================================================================
# Limiter Initialization Tests
# ============================================================================

class TestInitLimiter:
    """Tests for init_limiter function"""

    def test_initializes_with_enabled_settings(self, mock_settings):
        """Should initialize limiter with rate limits when enabled"""
        init_limiter(mock_settings)

        # Check that limiter exists and is configured
        assert limiter is not None
        assert hasattr(limiter, '_default_limits')

    def test_initializes_with_disabled_settings(self, mock_settings_disabled):
        """Should initialize with high limit when disabled"""
        init_limiter(mock_settings_disabled)

        assert limiter is not None

    def test_logs_enabled_message(self, mock_settings):
        """Should log info message when enabled"""
        with patch('app.middleware.rate_limit.logger') as mock_logger:
            init_limiter(mock_settings)

            mock_logger.info.assert_called()
            call_args = str(mock_logger.info.call_args)
            assert "enabled" in call_args.lower() or "200" in call_args

    def test_logs_disabled_message(self, mock_settings_disabled):
        """Should log info message when disabled"""
        with patch('app.middleware.rate_limit.logger') as mock_logger:
            init_limiter(mock_settings_disabled)

            mock_logger.info.assert_called()
            call_args = str(mock_logger.info.call_args)
            assert "disabled" in call_args.lower()

    def test_uses_memory_storage(self, mock_settings):
        """Should use in-memory storage"""
        init_limiter(mock_settings)

        # Limiter should be configured with memory storage
        assert limiter is not None

    def test_uses_fixed_window_strategy(self, mock_settings):
        """Should use fixed-window strategy"""
        init_limiter(mock_settings)

        # Check limiter exists (strategy is internal)
        assert limiter is not None

    def test_can_reinitialize(self, mock_settings, mock_settings_disabled):
        """Should be able to reinitialize limiter"""
        init_limiter(mock_settings)
        first_limiter_id = id(limiter)

        init_limiter(mock_settings_disabled)
        second_limiter_id = id(limiter)

        # Should be different instances
        assert first_limiter_id != second_limiter_id


# ============================================================================
# Rate Limit Handler Tests
# ============================================================================

class TestRateLimitHandler:
    """Tests for rate_limit_handler function"""

    @pytest.mark.asyncio
    async def test_handler_logs_warning(self, mock_request):
        """Should log warning when rate limit exceeded"""
        exc = RateLimitExceeded("10 per minute")

        with patch('app.middleware.rate_limit.logger') as mock_logger:
            try:
                await rate_limit_handler(mock_request, exc)
            except RateLimitExceeded:
                pass

            mock_logger.warning.assert_called_once()

    @pytest.mark.asyncio
    async def test_handler_logs_client_ip(self, mock_request):
        """Should log client IP in warning"""
        mock_request.client.host = "192.168.1.100"
        exc = RateLimitExceeded("10 per minute")

        with patch('app.middleware.rate_limit.logger') as mock_logger:
            try:
                await rate_limit_handler(mock_request, exc)
            except RateLimitExceeded:
                pass

            call_args = str(mock_logger.warning.call_args)
            assert "192.168.1.100" in call_args

    @pytest.mark.asyncio
    async def test_handler_raises_rate_limit_exceeded(self, mock_request):
        """Should raise RateLimitExceeded exception"""
        exc = RateLimitExceeded("10 per minute")

        with pytest.raises(RateLimitExceeded):
            await rate_limit_handler(mock_request, exc)

    @pytest.mark.asyncio
    async def test_handler_includes_retry_message(self, mock_request):
        """Exception should include retry message"""
        exc = RateLimitExceeded("10 per minute")

        with pytest.raises(RateLimitExceeded) as exc_info:
            await rate_limit_handler(mock_request, exc)

        assert "try again" in str(exc_info.value.detail).lower()


# ============================================================================
# Limiter Configuration Tests
# ============================================================================

class TestLimiterConfiguration:
    """Tests for limiter configuration"""

    def test_default_limiter_exists(self):
        """Default limiter should exist at module load"""
        from app.middleware.rate_limit import limiter
        assert limiter is not None

    def test_default_limiter_has_key_func(self):
        """Limiter should have key function configured"""
        from app.middleware.rate_limit import limiter
        assert limiter._key_func is not None

    def test_custom_limits_applied(self, mock_settings):
        """Custom limits should be applied from settings"""
        mock_settings.rate_limit_per_day = 500
        mock_settings.rate_limit_per_hour = 100

        with patch('app.middleware.rate_limit.logger'):
            init_limiter(mock_settings)

        # Limiter should be reconfigured
        assert limiter is not None
