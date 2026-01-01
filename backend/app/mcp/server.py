"""MCP server setup for Seedream backend"""
from fastapi import FastAPI, Depends, Header, HTTPException
from fastapi_mcp import FastApiMCP

from app.config import get_settings
from app.mcp.oauth import verify_oauth

settings = get_settings()

# Store tokens in memory (for development)
# In production, use a database
access_tokens = {}


def setup_mcp(app: FastAPI) -> None:
    """Setup MCP server with FastAPI"""
    
    mcp = FastApiMCP(
        app,
        name="Seedream Knowledge Graph",
        description="Access and query an organizational knowledge base",
        transport="sse"
    )
    
    # Single SSE endpoint for both Claude and ChatGPT
    # OAuth verification is optional for Claude Desktop
    mcp.mount(path="/sse", dependencies=[Depends(verify_oauth)])
    
    # Register OAuth routes
    from app.mcp.oauth import setup_oauth_routes
    setup_oauth_routes(app)
