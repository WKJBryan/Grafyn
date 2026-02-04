"""MCP server setup for Grafyn backend"""
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
        name="Grafyn Knowledge Graph",
        description="Access and query an organizational knowledge base",
        # Exclude import endpoints to prevent recursion from self-referential models
        exclude_tags=["import"],
    )

    # Mount MCP SSE routes to app at /mcp
    # Note: fastapi-mcp auto-discovers all FastAPI routes as MCP tools
    mcp.mount_sse()

    # Register OAuth routes
    from app.mcp.oauth import setup_oauth_routes
    setup_oauth_routes(app)
