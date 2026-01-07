# Seedream Backend Architecture

> **Part:** Backend | **Type:** FastAPI Python Service | **Scan Level:** Exhaustive

## Overview

The backend is a FastAPI-based REST API providing:
- Note CRUD operations with Markdown/YAML frontmatter support
- Semantic vector search using LanceDB + sentence-transformers
- Knowledge graph with wikilink parsing and backlinks
- MCP server for external AI model integration
- OAuth authentication for ChatGPT
- Security middleware (rate limiting, input sanitization, security headers)

## Entry Point

**File:** `backend/app/main.py`

```python
from contextlib import asynccontextmanager
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware

from app.config import get_settings
from app.routers import notes, search, graph, oauth
from app.mcp.server import setup_mcp
from app.middleware.logging import LoggingMiddleware
from app.middleware.security import SecurityHeadersMiddleware, RequestSanitizationMiddleware
from app.middleware.rate_limit import limiter, init_limiter, rate_limit_handler

settings = get_settings()

@asynccontextmanager
async def lifespan(app: FastAPI):
    # Initialize services on startup
    knowledge_store = KnowledgeStore()
    vector_search = VectorSearchService()
    graph_index = GraphIndexService()
    
    app.state.knowledge_store = knowledge_store
    app.state.vector_search = vector_search
    app.state.graph_index = graph_index
    yield

app = FastAPI(
    title="Seedream",
    description="Knowledge Graph Platform with Semantic Search and MCP",
    version="0.1.0",
    lifespan=lifespan
)

# Middleware (order matters)
app.add_middleware(RequestSanitizationMiddleware)
app.add_middleware(SecurityHeadersMiddleware)
app.add_middleware(LoggingMiddleware)
app.add_middleware(CORSMiddleware, ...)

# Routers
app.include_router(notes.router, prefix="/api/notes", tags=["notes"])
app.include_router(search.router, prefix="/api/search", tags=["search"])
app.include_router(graph.router, prefix="/api/graph", tags=["graph"])
app.include_router(oauth.router, prefix="/api/oauth", tags=["oauth"])

# MCP server
setup_mcp(app)
```

## Architecture Layers

```
┌──────────────────────────────────────────────────────────────┐
│                      Middleware Layer                         │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐   │
│  │ RateLimit   │  │ Security    │  │ Logging             │   │
│  │ (slowapi)   │  │ Headers     │  │ Middleware          │   │
│  └─────────────┘  └─────────────┘  └─────────────────────┘   │
├──────────────────────────────────────────────────────────────┤
│                      API Layer (Routers)                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌──────┐ │
│  │ notes.py    │  │ search.py   │  │ graph.py    │  │oauth │ │
│  │ 6 endpoints │  │ 2 endpoints │  │ 5 endpoints │  │.py   │ │
│  └─────────────┘  └─────────────┘  └─────────────┘  └──────┘ │
├──────────────────────────────────────────────────────────────┤
│                     Service Layer                             │
│  ┌────────────────┐ ┌─────────────────┐ ┌─────────────────┐  │
│  │ KnowledgeStore │ │ VectorSearch    │ │ GraphIndex      │  │
│  │ (Markdown I/O) │ │ (LanceDB)       │ │ (Link tracking) │  │
│  └────────────────┘ └─────────────────┘ └─────────────────┘  │
│            │                 │                               │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │              EmbeddingService + TokenStore              │ │
│  │         (sentence-transformers) (OAuth tokens)          │ │
│  └─────────────────────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────────────┤
│                     Data Layer                                │
│  ┌────────────────┐ ┌─────────────────┐ ┌─────────────────┐  │
│  │ vault/*.md     │ │ data/lancedb/   │ │ In-memory graph │  │
│  │ (Markdown)     │ │ (Vectors)       │ │ (Adjacency)     │  │
│  └────────────────┘ └─────────────────┘ └─────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

## Configuration

**File:** `backend/app/config.py`

```python
class Settings(BaseSettings):
    # Server
    server_host: str = "0.0.0.0"
    server_port: int = 8080
    environment: str = "development"
    
    # Paths
    vault_path: str = "../vault"
    data_path: str = "../data"
    
    # Embedding
    embedding_model: str = "all-MiniLM-L6-v2"
    
    # OAuth
    github_client_id: str = ""
    github_client_secret: str = ""
    github_redirect_uri: str = ""
    
    # CORS
    cors_origins: Optional[str] = None
    
    # Rate Limiting
    rate_limit_enabled: bool = True
    rate_limit_per_day: int = 200
    rate_limit_per_hour: int = 50
    rate_limit_per_minute: int = 10
    
    class Config:
        env_file = ".env"
```

| Setting | Default | Purpose |
|---------|---------|---------|
| `VAULT_PATH` | `../vault` | Markdown notes directory |
| `DATA_PATH` | `../data` | LanceDB storage |
| `SERVER_HOST` | `0.0.0.0` | Bind address |
| `SERVER_PORT` | `8080` | HTTP port |
| `ENVIRONMENT` | `development` | Environment mode |
| `EMBEDDING_MODEL` | `all-MiniLM-L6-v2` | Sentence transformer model |
| `GITHUB_CLIENT_ID` | - | OAuth client ID |
| `GITHUB_CLIENT_SECRET` | - | OAuth client secret |
| `RATE_LIMIT_ENABLED` | `true` | Enable rate limiting |

## Service Details

### 1. KnowledgeStore (`services/knowledge_store.py`)

**Purpose:** CRUD operations for Markdown notes with YAML frontmatter

**Key Methods:**
| Method | Description |
|--------|-------------|
| `list_notes()` | List all notes with metadata |
| `get_note(id)` | Get full note content |
| `create_note(data)` | Create new note file |
| `update_note(id, data)` | Update existing note |
| `delete_note(id)` | Delete note file |
| `extract_wikilinks(content)` | Parse `[[wikilinks]]` from content |
| `get_all_content()` | Get all notes for bulk indexing |

### 2. VectorSearchService (`services/vector_search.py`)

**Purpose:** LanceDB-backed semantic search

**Key Methods:**
| Method | Description |
|--------|-------------|
| `index_note(id, title, content)` | Index single note |
| `index_all(notes)` | Batch index all notes |
| `search(query, limit)` | Semantic similarity search |
| `delete_note(id)` | Remove from index |

**LanceDB Schema:**
```python
class NoteEmbedding(LanceModel):
    note_id: str
    title: str
    text: str
    vector: Vector(384)  # all-MiniLM-L6-v2 dimension
```

### 3. GraphIndexService (`services/graph_index.py`)

**Purpose:** Wikilink parsing and backlink tracking

**Data Structures:**
```python
self._outgoing: Dict[str, Set[str]]  # note_id → linked note IDs
self._incoming: Dict[str, Set[str]]  # note_id → notes linking to it
```

**Key Methods:**
| Method | Description |
|--------|-------------|
| `build_index()` | Rebuild full graph from all notes |
| `get_outgoing_links(id)` | Get notes this note links to |
| `get_backlinks(id)` | Get notes that link to this note |
| `get_backlinks_with_context(id)` | Backlinks with surrounding text |
| `get_neighbors(id, depth)` | BFS traversal for visualization |
| `find_unlinked_mentions(id)` | Find potential links |

### 4. EmbeddingService (`services/embedding.py`)

**Purpose:** Text to vector encoding

**Key Methods:**
| Method | Description |
|--------|-------------|
| `encode(text)` | Single text → vector |
| `encode_batch(texts)` | Batch encoding |
| `dimension` | Returns 384 (model output size) |

### 5. TokenStore (`services/token_store.py`)

**Purpose:** OAuth token management

**Key Methods:**
| Method | Description |
|--------|-------------|
| `store_token(token, user_data)` | Store OAuth token |
| `validate_token(token)` | Validate and return user data |
| `revoke_token(token)` | Revoke a token |

## Middleware

### SecurityHeadersMiddleware
Adds security headers to all responses:
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY`
- `X-XSS-Protection: 1; mode=block`

### RequestSanitizationMiddleware
Sanitizes incoming request data to prevent injection attacks.

### LoggingMiddleware
Logs request/response details for debugging and monitoring.

### Rate Limiting (slowapi)
Configurable rate limits per endpoint:
- Default: 10/minute, 50/hour, 200/day
- Health check: 30/minute

## MCP Integration

**Files:** `mcp/server.py`, `mcp/tools.py`

The backend exposes FastAPI routes as MCP tools via `fastapi-mcp`:

```python
def setup_mcp(app: FastAPI) -> None:
    from fastapi_mcp import FastApiMCP
    
    mcp = FastApiMCP(
        app,
        name="Seedream Knowledge Graph",
        description="Access and query an organizational knowledge base",
    )
    mcp.mount()  # Mounts at /sse
```

**Available MCP Tools:**

| Tool | Arguments | Purpose |
|------|-----------|---------|
| `query_knowledge` | `query`, `limit` | Semantic search |
| `get_note` | `note_id` | Full note content |
| `list_notes` | `tag`, `status` | Filtered listing |
| `get_backlinks` | `note_id` | Backlinks with context |
| `ingest_chat` | `content`, `title`, `source`, `tags` | Store transcripts |
| `create_draft` | `title`, `content`, `based_on`, `tags` | Create drafts |

## Dependencies

| Package | Version | Purpose |
|---------|---------|---------|
| fastapi | ≥0.128.0 | Web framework |
| uvicorn | ≥0.40.0 | ASGI server |
| pydantic | ≥2.12.0 | Data validation |
| pydantic-settings | ≥2.12.0 | Settings management |
| lancedb | ≥0.26.0 | Vector database |
| sentence-transformers | ≥5.2.0 | Embeddings |
| python-frontmatter | ≥1.1.0 | YAML frontmatter parsing |
| slowapi | Latest | Rate limiting |
| httpx | ≥0.28.0 | HTTP client (OAuth) |
| python-dotenv | ≥1.2.0 | Environment loading |

## File Structure

```
backend/
├── app/
│   ├── __init__.py
│   ├── main.py              # FastAPI application
│   ├── config.py            # Settings from .env
│   ├── routers/
│   │   ├── __init__.py
│   │   ├── notes.py         # Note CRUD endpoints
│   │   ├── search.py        # Search endpoints
│   │   ├── graph.py         # Graph endpoints
│   │   └── oauth.py         # OAuth endpoints
│   ├── services/
│   │   ├── __init__.py
│   │   ├── knowledge_store.py
│   │   ├── vector_search.py
│   │   ├── graph_index.py
│   │   ├── embedding.py
│   │   └── token_store.py
│   ├── middleware/
│   │   ├── __init__.py
│   │   ├── security.py
│   │   ├── logging.py
│   │   └── rate_limit.py
│   ├── models/
│   │   ├── __init__.py
│   │   └── note.py
│   └── mcp/
│       ├── __init__.py
│       ├── server.py
│       └── tools.py
├── Dockerfile
├── docker-compose.yml
├── requirements.txt
└── .env.example
```
