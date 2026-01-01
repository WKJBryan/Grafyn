"""Rate limiting middleware"""
from slowapi import Limiter
from slowapi.util import get_remote_address
from slowapi.errors import RateLimitExceeded
from fastapi import Request
import logging

logger = logging.getLogger(__name__)

# Limiter will be initialized with config from settings
limiter = None

def init_limiter(settings):
    """Initialize limiter with settings from config"""
    global limiter
    if settings.rate_limit_enabled:
        limiter = Limiter(
            key_func=get_remote_address,
            default_limits=[
                f"{settings.rate_limit_per_day} per day",
                f"{settings.rate_limit_per_hour} per hour"
            ],
            storage_uri="memory://",
            strategy="fixed-window"
        )
        logger.info(f"Rate limiting enabled: {settings.rate_limit_per_day}/day, {settings.rate_limit_per_hour}/hour")
    else:
        limiter = Limiter(
            key_func=get_remote_address,
            default_limits=["10000 per day"],  # Very high limit when disabled
            storage_uri="memory://",
            strategy="fixed-window"
        )
        logger.info("Rate limiting disabled")

# Custom rate limit exceeded handler
async def rate_limit_handler(request: Request, exc: RateLimitExceeded):
    logger.warning(f"Rate limit exceeded for {request.client.host}")
    raise RateLimitExceeded(
        detail="Rate limit exceeded. Please try again later."
    )
