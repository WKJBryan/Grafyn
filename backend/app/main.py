"""Main FastAPI application for Seedream"""
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from pathlib import Path

from app.config import get_settings
from app.routers import notes, search, graph
from app.mcp.server import setup_mcp

# Get settings
settings = get_settings()

# Create FastAPI application
app = FastAPI(
    title="Seedream",
    description="Knowledge Graph Platform with Semantic Search and MCP",
    version="0.1.0",
    docs_url="/docs",
    redoc_url="/redoc"
)

# CORS middleware for frontend access
app.add_middleware(
    CORSMiddleware,
    allow_origins=settings.cors_origins,
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)


# Root endpoint
@app.get("/")
async def root():
    """API root endpoint with basic information"""
    vault_path = str(Path(settings.vault_path).resolve())
    return {
        "name": "Seedream Knowledge Graph",
        "version": "0.1.0",
        "vault_path": vault_path,
        "docs": "/docs",
        "mcp": "/sse",
        "oauth": "/auth"
    }


# Health check endpoint
@app.get("/health")
async def health_check():
    """Health check endpoint"""
    return {
        "status": "healthy",
        "service": "seedream"
    }


# Include routers
app.include_router(notes.router, prefix="/api/notes", tags=["notes"])
app.include_router(search.router, prefix="/api/search", tags=["search"])
app.include_router(graph.router, prefix="/api/graph", tags=["graph"])

# Setup MCP server
setup_mcp(app)


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(
        "app.main:app",
        host=settings.server_host,
        port=settings.server_port,
        reload=settings.environment == "development"
    )
