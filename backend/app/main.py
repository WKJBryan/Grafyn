"""Main FastAPI application for Seedream"""
from contextlib import asynccontextmanager
from fastapi import FastAPI, Request
from fastapi.middleware.cors import CORSMiddleware
from pathlib import Path

from backend.app.config import get_settings
from backend.app.routers import notes, search, graph, oauth
from backend.app.mcp.server import setup_mcp
from backend.app.services.knowledge_store import KnowledgeStore
from backend.app.services.vector_search import VectorSearchService
from backend.app.services.graph_index import GraphIndexService
from backend.app.middleware.logging import LoggingMiddleware
from backend.app.middleware.security import SecurityHeadersMiddleware, RequestSanitizationMiddleware
from backend.app.middleware.rate_limit import limiter, init_limiter, rate_limit_handler
from slowapi.errors import RateLimitExceeded

# Get settings
settings = get_settings()

# Initialize limiter with settings
init_limiter(settings)


@asynccontextmanager
async def lifespan(app: FastAPI):
    # Startup
    knowledge_store = KnowledgeStore()
    vector_search = VectorSearchService()
    graph_index = GraphIndexService()
    
    app.state.knowledge_store = knowledge_store
    app.state.vector_search = vector_search
    app.state.graph_index = graph_index
    
    yield
    
    # Shutdown
    # Cleanup if needed


# Create FastAPI application
app = FastAPI(
    title="Seedream",
    description="Knowledge Graph Platform with Semantic Search and MCP",
    version="0.1.0",
    docs_url="/docs",
    redoc_url="/redoc",
    lifespan=lifespan
)

# Security middleware (order matters - request sanitization first)
app.add_middleware(RequestSanitizationMiddleware)
app.add_middleware(SecurityHeadersMiddleware)
app.add_middleware(LoggingMiddleware)

# CORS middleware for frontend access - more restrictive in production
if settings.environment == "production":
    # In production, only allow specific origins
    app.add_middleware(
        CORSMiddleware,
        allow_origins=settings.cors_origins,
        allow_credentials=False,  # Disable credentials in production
        allow_methods=["GET", "POST", "PUT", "DELETE"],
        allow_headers=["Content-Type", "Authorization"],
        max_age=3600
    )
else:
    # Development mode - more permissive
    app.add_middleware(
        CORSMiddleware,
        allow_origins=["*"],  # Allow all origins in development
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )

# Add rate limiting to app
app.state.limiter = limiter
app.add_exception_handler(RateLimitExceeded, rate_limit_handler)


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
@limiter.limit("30 per minute")
async def health_check(request: Request):
    """Health check endpoint"""
    return {
        "status": "healthy",
        "service": "seedream",
        "environment": settings.environment
    }


# Include routers
app.include_router(notes.router, prefix="/api/notes", tags=["notes"])
app.include_router(search.router, prefix="/api/search", tags=["search"])
app.include_router(graph.router, prefix="/api/graph", tags=["graph"])
app.include_router(oauth.router, prefix="/api/oauth", tags=["oauth"])

# Setup MCP server
setup_mcp(app)


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(
        "backend.app.main:app",
        host=settings.server_host,
        port=settings.server_port,
        reload=settings.environment == "development"
    )
