"""OAuth authentication for ChatGPT MCP integration"""
from fastapi import FastAPI, Request, HTTPException
from fastapi.responses import RedirectResponse
from fastapi.security import HTTPBearer, HTTPAuthorizationCredentials
import httpx
import secrets
from datetime import datetime, timezone, timedelta

from app.config import get_settings
from app.services.token_store import TokenStore

settings = get_settings()

# Initialize token store
token_store = TokenStore()

# Security scheme for authentication
security = HTTPBearer(auto_error=False)

# Token expiration time (1 hour for access tokens)
TOKEN_EXPIRATION_HOURS = 1


def setup_oauth_routes(app: FastAPI):
    """Setup OAuth routes for GitHub authentication"""
    
    @app.get("/auth/github")
    async def github_auth():
        """Redirect user to GitHub for authorization"""
        if not settings.github_client_id or not settings.github_redirect_uri:
            raise HTTPException(
                status_code=500,
                detail="OAuth not configured. Set GITHUB_CLIENT_ID and GITHUB_REDIRECT_URI in .env"
            )
        
        # Generate state parameter for CSRF protection
        state = secrets.token_urlsafe(32)
        token_store.store_token(f"state_{state}", "valid", expires_at=datetime.now(timezone.utc) + timedelta(minutes=10))
        
        auth_url = (
            f"https://github.com/login/oauth/authorize?"
            f"client_id={settings.github_client_id}&"
            f"redirect_uri={settings.github_redirect_uri}&"
            f"scope=read:user&"
            f"state={state}"
        )
        return RedirectResponse(auth_url)
    
    @app.get("/auth/callback")
    async def github_callback(code: str, state: str = None):
        """Handle GitHub callback and exchange code for token"""
        # Validate state parameter for CSRF protection
        if state:
            stored_state = token_store.get_token(f"state_{state}")
            if not stored_state:
                raise HTTPException(status_code=400, detail="Invalid state parameter")
            token_store.delete_token(f"state_{state}")
        
        if not settings.github_client_id or not settings.github_client_secret:
            raise HTTPException(
                status_code=500,
                detail="OAuth not configured. Set GITHUB_CLIENT_ID and GITHUB_CLIENT_SECRET in .env"
            )
        
        async with httpx.AsyncClient(timeout=30.0) as client:
            response = await client.post(
                "https://github.com/login/oauth/access_token",
                data={
                    "client_id": settings.github_client_id,
                    "client_secret": settings.github_client_secret,
                    "code": code,
                    "redirect_uri": settings.github_redirect_uri,
                },
                headers={"Accept": "application/json"}
            )
            token_data = response.json()
            access_token = token_data.get("access_token")
            
            if not access_token:
                raise HTTPException(status_code=400, detail="Failed to get access token")
            
            # Generate cryptographically secure token ID
            token_id = secrets.token_urlsafe(32)
            
            # Store token with expiration
            expires_at = datetime.now(timezone.utc) + timedelta(hours=TOKEN_EXPIRATION_HOURS)
            token_store.store_token(token_id, access_token, expires_at=expires_at)
            
            # Return token to ChatGPT
            return {"access_token": token_id, "expires_in": TOKEN_EXPIRATION_HOURS * 3600}


async def verify_oauth(credentials: HTTPAuthorizationCredentials = None):
    """
    Verify OAuth token for ChatGPT and MCP clients
    
    This function is used as a dependency in FastAPI routes.
    Authentication is required in production environments.
    """
    # Allow unauthenticated access only in development mode
    if settings.environment == "development" and credentials is None:
        return True
    
    # Require authentication in production
    if credentials is None:
        raise HTTPException(
            status_code=401,
            detail="Authentication required",
            headers={"WWW-Authenticate": "Bearer"}
        )
    
    # Extract token from credentials
    token = credentials.credentials
    
    # Validate token and check expiration
    token_data = token_store.get_token(token)
    if not token_data:
        raise HTTPException(status_code=401, detail="Invalid or expired OAuth token")
    
    # Check if token has expired
    if isinstance(token_data, dict) and 'expires_at' in token_data:
        expires_at = token_data['expires_at']
        if datetime.now(timezone.utc) > expires_at:
            token_store.delete_token(token)
            raise HTTPException(status_code=401, detail="OAuth token has expired")
    
    return True
