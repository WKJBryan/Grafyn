"""Main FastAPI application for Seedream"""

from contextlib import asynccontextmanager
from fastapi import FastAPI, Request
from fastapi.middleware.cors import CORSMiddleware
from pathlib import Path

from app.config import get_settings
from app.routers import notes, search, graph, oauth, canvas, distill, priority, feedback
from app.routers.conversation_import import router as import_router
from app.routers import mcp_write
from app.routers import zettelkasten
from app.mcp.server import setup_mcp
from app.services.knowledge_store import KnowledgeStore
from app.services.vector_search import VectorSearchService
from app.services.graph_index import GraphIndexService
from app.services.openrouter import OpenRouterService
from app.services.canvas_store import CanvasSessionStore
from app.services.priority_scoring import PriorityScoringService
from app.services.priority_settings import PrioritySettingsService
from app.services.distillation import DistillationService
from app.services.link_discovery import LinkDiscoveryService
from app.services.import_service import ImportService
from app.services.feedback import FeedbackService
from app.middleware.logging import LoggingMiddleware
from app.middleware.security import (
    SecurityHeadersMiddleware,
    RequestSanitizationMiddleware,
)
from app.middleware.rate_limit import limiter, init_limiter, rate_limit_handler
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
    openrouter = OpenRouterService()
    canvas_store = CanvasSessionStore()
    priority_settings = PrioritySettingsService()
    priority_scoring = PriorityScoringService(priority_settings.get_weights())

    # Distillation service (initialized with other services)
    distillation_service = DistillationService(
        knowledge_store=knowledge_store,
        vector_search=vector_search,
        graph_index=graph_index,
        openrouter_service=openrouter,
    )

    # Link discovery service for Zettelkasten
    link_discovery = LinkDiscoveryService(
        knowledge_store=knowledge_store,
        vector_search=vector_search,
        graph_index=graph_index,
        openrouter_service=openrouter,
    )

    import_service = ImportService(
        knowledge_store=knowledge_store,
        vector_search=vector_search,
        graph_index=graph_index,
        distillation_service=distillation_service,
        openrouter_service=openrouter,
    )

    # Feedback service for bug reports and feature requests
    feedback_service = FeedbackService()

    app.state.knowledge_store = knowledge_store
    app.state.vector_search = vector_search
    app.state.graph_index = graph_index
    app.state.openrouter = openrouter
    app.state.canvas_store = canvas_store
    app.state.priority_settings = priority_settings
    app.state.priority_scoring = priority_scoring
    app.state.distillation = distillation_service
    app.state.link_discovery = link_discovery
    app.state.import_service = import_service
    app.state.feedback_service = feedback_service

    yield

    # Shutdown
    await openrouter.close()
    await feedback_service.close()


# Create FastAPI application
app = FastAPI(
    title="Seedream",
    description="Knowledge Graph Platform with Semantic Search and MCP",
    version="0.1.0",
    docs_url="/docs",
    redoc_url="/redoc",
    lifespan=lifespan,
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
        max_age=3600,
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
        "oauth": "/auth",
    }


# Health check endpoint
@app.get("/health")
@limiter.limit("30 per minute")
async def health_check(request: Request):
    """Health check endpoint"""
    return {
        "status": "healthy",
        "service": "seedream",
        "environment": settings.environment,
    }


# Include routers
app.include_router(notes.router, prefix="/api/notes", tags=["notes"])
app.include_router(search.router, prefix="/api/search", tags=["search"])
app.include_router(graph.router, prefix="/api/graph", tags=["graph"])
app.include_router(oauth.router, prefix="/api/oauth", tags=["oauth"])
app.include_router(canvas.router, prefix="/api/canvas", tags=["canvas"])
app.include_router(distill.router, prefix="/api/notes", tags=["distillation"])
app.include_router(priority.router, prefix="/api/priority", tags=["priority"])
app.include_router(import_router, prefix="/api/import", tags=["import"])
app.include_router(mcp_write.router, prefix="/api", tags=["mcp-write"])
app.include_router(zettelkasten.router, prefix="/api/zettel", tags=["zettelkasten"])
app.include_router(feedback.router, prefix="/api/feedback", tags=["feedback"])

# Setup MCP server
setup_mcp(app)


def run_server(host: str = None, port: int = None, reload: bool = None):
    """Run the uvicorn server with the specified configuration."""
    import uvicorn

    uvicorn.run(
        "app.main:app",
        host=host or settings.server_host,
        port=port or settings.server_port,
        reload=reload if reload is not None else settings.environment == "development",
    )


if __name__ == "__main__":
    import argparse
    import os

    # Parse command-line arguments for sidecar mode
    parser = argparse.ArgumentParser(
        description="Seedream Knowledge Graph Backend",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Run with default settings (from .env)
  python -m app.main

  # Run as Tauri sidecar
  python -m app.main --port 8765 --host 127.0.0.1 --vault-path ~/Documents/Seedream/vault

  # Run in production mode
  python -m app.main --port 8080 --no-reload --environment production
        """
    )
    parser.add_argument(
        "--host",
        type=str,
        default=None,
        help="Host to bind to (default: from settings or 0.0.0.0)"
    )
    parser.add_argument(
        "--port",
        type=int,
        default=None,
        help="Port to bind to (default: from settings or 8080)"
    )
    parser.add_argument(
        "--vault-path",
        type=str,
        default=None,
        help="Path to the vault directory containing markdown notes"
    )
    parser.add_argument(
        "--data-path",
        type=str,
        default=None,
        help="Path to the data directory for LanceDB, canvas sessions, etc."
    )
    parser.add_argument(
        "--no-reload",
        action="store_true",
        help="Disable auto-reload (useful for production or sidecar mode)"
    )
    parser.add_argument(
        "--environment",
        type=str,
        choices=["development", "production"],
        default=None,
        help="Set the environment mode"
    )

    args = parser.parse_args()

    # Override environment variables based on CLI args
    # This allows the sidecar to configure paths dynamically
    if args.vault_path:
        os.environ["VAULT_PATH"] = args.vault_path
    if args.data_path:
        os.environ["DATA_PATH"] = args.data_path
    if args.environment:
        os.environ["ENVIRONMENT"] = args.environment

    # Determine reload setting
    reload = not args.no_reload if args.no_reload else None

    # Run the server
    run_server(
        host=args.host,
        port=args.port,
        reload=reload,
    )
