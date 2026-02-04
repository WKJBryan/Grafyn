"""Logging middleware for request/response tracking with security"""
import logging
import time
from fastapi import Request
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.responses import Response

logger = logging.getLogger(__name__)


class LoggingMiddleware(BaseHTTPMiddleware):
    """Middleware to log all requests and responses with security considerations"""

    async def dispatch(self, request: Request, call_next):
        start_time = time.time()

        # Get sanitized request info if available
        if hasattr(request.state, 'sanitized_info'):
            sanitized_info = request.state.sanitized_info
            logger.info(f"Request: {sanitized_info['method']} {sanitized_info['url']} from {sanitized_info['client']}")
        else:
            logger.info(f"Request: {request.method} {request.url.path}")

        # Process request with exception handling
        try:
            response = await call_next(request)
        except Exception as e:
            # Log the error and create error response
            process_time = (time.time() - start_time) * 1000
            logger.error(
                f"Exception: {request.method} {request.url.path} "
                f"Error: {type(e).__name__}: {str(e)} "
                f"Time: {process_time:.2f}ms"
            )
            # Create error response with timing header
            response = Response("Internal Server Error", status_code=500)
            response.headers["X-Process-Time"] = str(process_time)
            return response

        # Calculate duration
        process_time = (time.time() - start_time) * 1000

        # Log response (without sensitive data)
        logger.info(
            f"Response: {request.method} {request.url.path} "
            f"Status: {response.status_code} "
            f"Time: {process_time:.2f}ms"
        )

        # Add timing header
        response.headers["X-Process-Time"] = str(process_time)

        return response
