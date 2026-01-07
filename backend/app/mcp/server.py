"""MCP server setup for Seedream backend"""
from fastapi import FastAPI, Depends, Header, HTTPException
from fastapi_mcp import FastApiMCP

from backend.app.config import get_settings
from backend.app.mcp.oauth import verify_oauth

settings = get_settings()

# Store tokens in memory (for development)
# In production, use a database
access_tokens = {}


def setup_mcp(app: FastAPI) -> None:
    """Setup MCP server with FastAPI"""
    
    mcp = FastApiMCP(
        app,
        name="Seedream Knowledge Graph",
        description="Access and query an organizational knowledge base"
    )
    
    # Mount MCP SSE routes to app at /mcp
    mcp.mount_sse()
    
    # Register OAuth routes
    from backend.app.mcp.oauth import setup_oauth_routes
    setup_oauth_routes(app)
