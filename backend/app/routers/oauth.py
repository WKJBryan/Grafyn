"""OAuth API router for frontend authentication"""
from fastapi import APIRouter, HTTPException, Depends
from fastapi.security import HTTPBearer, HTTPAuthorizationCredentials
from app.mcp.oauth import token_store
from app.config import get_settings
import httpx
import secrets
from datetime import datetime, timezone, timedelta

router = APIRouter()
settings = get_settings()

# Security scheme
security = HTTPBearer(auto_error=False)

# Token expiration time
TOKEN_EXPIRATION_HOURS = 1


@router.get("/authorize/{provider}")
async def get_authorization_url(provider: str):
    """Get OAuth authorization URL for a provider"""
    if provider == "github":
        if not settings.github_client_id or not settings.github_redirect_uri:
            raise HTTPException(
                status_code=500,
                detail="GitHub OAuth not configured"
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
        return {"authorization_url": auth_url, "state": state}
    
    elif provider == "google":
        # Placeholder for Google OAuth
        raise HTTPException(
            status_code=501,
            detail="Google OAuth not yet implemented"
        )
    
    else:
        raise HTTPException(
            status_code=400,
            detail=f"Unsupported provider: {provider}"
        )


@router.post("/callback/{provider}")
async def exchange_code(provider: str, code: str, state: str = None):
    """Exchange OAuth code for access token"""
    # Validate state parameter for CSRF protection
    if state:
        stored_state = token_store.get_token(f"state_{state}")
        if not stored_state:
            raise HTTPException(status_code=400, detail="Invalid state parameter")
        token_store.delete_token(f"state_{state}")
    
    if provider == "github":
        if not settings.github_client_id or not settings.github_client_secret:
            raise HTTPException(
                status_code=500,
                detail="GitHub OAuth not configured"
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
                raise HTTPException(
                    status_code=400,
                    detail="Failed to get access token"
                )
            
            # Generate cryptographically secure token ID
            token_id = secrets.token_urlsafe(32)
            
            # Store token with expiration
            expires_at = datetime.now(timezone.utc) + timedelta(hours=TOKEN_EXPIRATION_HOURS)
            token_store.store_token(token_id, access_token, expires_at=expires_at)
            
            return {"access_token": token_id, "expires_in": TOKEN_EXPIRATION_HOURS * 3600}
    
    else:
        raise HTTPException(
            status_code=501,
            detail=f"{provider} OAuth not yet implemented"
        )


@router.get("/user")
async def get_user(credentials: HTTPAuthorizationCredentials = Depends(security)):
    """Get current authenticated user info"""
    # Validate token
    if credentials:
        token = credentials.credentials
        token_data = token_store.get_token(token)
        if not token_data:
            raise HTTPException(status_code=401, detail="Invalid or expired token")
    
    # Placeholder - would need to fetch from GitHub API
    return {
        "id": "user_123",
        "name": "User",
        "email": "user@example.com"
    }


@router.post("/logout")
async def logout(credentials: HTTPAuthorizationCredentials = Depends(security)):
    """Logout current user"""
    if credentials:
        token = credentials.credentials
        token_store.delete_token(token)
    
    return {"message": "Logged out successfully"}
