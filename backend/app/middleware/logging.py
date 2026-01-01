"""Logging middleware for request/response tracking with security"""
import logging
import time
from fastapi import Request
from starlette.middleware.base import BaseHTTPMiddleware

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
        
        # Process request
        response = await call_next(request)
        
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
