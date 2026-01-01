"""OAuth authentication for ChatGPT MCP integration"""
from fastapi import FastAPI, Request, HTTPException
from fastapi.responses import RedirectResponse
import httpx

from app.config import get_settings

settings = get_settings()

# Store tokens in memory (for development)
# In production, use a database
access_tokens = {}


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
        
        auth_url = (
            f"https://github.com/login/oauth/authorize?"
            f"client_id={settings.github_client_id}&"
            f"redirect_uri={settings.github_redirect_uri}&"
            f"scope=read:user"
        )
        return RedirectResponse(auth_url)
    
    @app.get("/auth/callback")
    async def github_callback(code: str):
        """Handle GitHub callback and exchange code for token"""
        if not settings.github_client_id or not settings.github_client_secret:
            raise HTTPException(
                status_code=500,
                detail="OAuth not configured. Set GITHUB_CLIENT_ID and GITHUB_CLIENT_SECRET in .env"
            )
        
        async with httpx.AsyncClient() as client:
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
            
            # Store token (in production, store in database)
            token_id = f"token_{len(access_tokens)}"
            access_tokens[token_id] = access_token
            
            # Return token to ChatGPT
            return {"access_token": token_id}


async def verify_oauth(authorization: str = None):
    """
    Verify OAuth token for ChatGPT, allow Claude Desktop without auth
    
    This function is used as a dependency in FastAPI routes.
    """
    if authorization is None:
        # Allow Claude Desktop without auth for local development
        return True
    
    # Extract token from "Bearer {token}" format
    token = authorization.replace("Bearer ", "")
    
    if token not in access_tokens:
        raise HTTPException(status_code=401, detail="Invalid OAuth token")
    
    return True
