"""Security middleware for HTTP headers and protections"""
from fastapi import Request, Response
from starlette.middleware.base import BaseHTTPMiddleware
import logging

logger = logging.getLogger(__name__)


class SecurityHeadersMiddleware(BaseHTTPMiddleware):
    """Middleware to add security headers to all responses"""
    
    async def dispatch(self, request: Request, call_next):
        response = await call_next(request)
        
        # Security headers
        response.headers["X-Content-Type-Options"] = "nosniff"
        response.headers["X-Frame-Options"] = "DENY"
        response.headers["X-XSS-Protection"] = "1; mode=block"
        response.headers["Referrer-Policy"] = "strict-origin-when-cross-origin"
        response.headers["Permissions-Policy"] = "geolocation=(), microphone=(), camera=()"
        
        # HSTS (only in production with HTTPS)
        if request.url.scheme == "https":
            response.headers["Strict-Transport-Security"] = "max-age=31536000; includeSubDomains"
        
        # Content Security Policy (basic)
        csp = (
            "default-src 'self'; "
            "script-src 'self' 'unsafe-inline' 'unsafe-eval'; "
            "style-src 'self' 'unsafe-inline'; "
            "img-src 'self' data: https:; "
            "font-src 'self' data:; "
            "connect-src 'self'; "
            "frame-ancestors 'none';"
        )
        response.headers["Content-Security-Policy"] = csp
        
        return response


class RequestSanitizationMiddleware(BaseHTTPMiddleware):
    """Middleware to sanitize request logs and prevent information leakage"""
    
    SENSITIVE_HEADERS = {'authorization', 'cookie', 'x-api-key', 'x-auth-token'}
    SENSITIVE_PARAMS = {'password', 'token', 'secret', 'api_key', 'access_token'}
    
    async def dispatch(self, request: Request, call_next):
        # Add sanitized request info to request state for logging
        request.state.sanitized_info = self._sanitize_request(request)
        
        response = await call_next(request)
        return response
    
    def _sanitize_request(self, request: Request) -> dict:
        """Sanitize request information for logging"""
        info = {
            "method": request.method,
            "url": str(request.url),
            "client": request.client.host if request.client else None,
            "headers": {}
        }
        
        # Redact sensitive headers
        for key, value in request.headers.items():
            if key.lower() in self.SENSITIVE_HEADERS:
                info["headers"][key] = "[REDACTED]"
            else:
                info["headers"][key] = value
        
        return info
