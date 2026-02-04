# Grafyn Backend Architecture

> **Part:** Backend | **Type:** FastAPI Python Service | **Scan Level:** Exhaustive

## Overview

The backend is a FastAPI-based REST API providing:
- Note CRUD operations with Markdown/YAML frontmatter support
- Semantic vector search using LanceDB + sentence-transformers
- Knowledge graph with wikilink parsing and backlinks
- **Distillation service** for Container → Atomic → Hub knowledge workflow
- MCP server for external AI model integration
- Multi-LLM Canvas for comparing AI model responses
- OAuth authentication for ChatGPT
- OpenRouter integration for 100+ AI models
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
    title="Grafyn",
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
app.include_router(canvas.router, prefix="/api/canvas", tags=["canvas"])

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
│  │ notes.py    │  │ search.py   │  │ graph.py    │  │oauth │ │canvas│ │
│  │ 6 endpoints │  │ 2 endpoints │  │ 5 endpoints │  │.py   │ │.py  │ │
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
│  │ vault/*.md     │ │ data/lancedb/   │ │ In-memory graph │ │
│  │ (Markdown)     │ │ (Vectors)       │ │ (Adjacency)     │ │
│  └────────────────┘ └─────────────────┘ └─────────────────┘ │
│            │                 │                               │
│  ┌────────────────┐ ┌─────────────────┐ ┌─────────────────┐  │
│  │data/canvas/    │ │ OpenRouter API  │ │                 │  │
│  │(Sessions JSON) │ │ (External)      │ │                 │  │
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
    
    # OpenRouter
    openrouter_api_key: str = ""
    app_url: str = "http://localhost:8080"
    canvas_data_path: str = "../data/canvas"
    
    class Config:
        env_file = ".env"
```

| Setting | Default | Purpose |
|---------|---------|---------|
| `VAULT_PATH` | `../vault` | Markdown notes directory |
| `DATA_PATH` | `../data` | LanceDB storage |
| `CANVAS_DATA_PATH` | `../data/canvas` | Canvas session storage |
| `SERVER_HOST` | `0.0.0.0` | Bind address |
| `SERVER_PORT` | `8080` | HTTP port |
| `ENVIRONMENT` | `development` | Environment mode |
| `EMBEDDING_MODEL` | `all-MiniLM-L6-v2` | Sentence transformer model |
| `GITHUB_CLIENT_ID` | - | OAuth client ID |
| `GITHUB_CLIENT_SECRET` | - | OAuth client secret |
| `OPENROUTER_API_KEY` | - | OpenRouter API key |
| `APP_URL` | `http://localhost:8080` | App URL for OpenRouter |
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

### 6. CanvasSessionStore (`services/canvas_store.py`)

**Purpose:** Multi-LLM canvas session persistence and management

**Key Methods:**
| Method | Description |
|--------|-------------|
| `list_sessions()` | List all canvas sessions |
| `get_session(id)` | Get specific session |
| `create_session(data)` | Create new session |
| `update_session(id, data)` | Update session metadata |
| `delete_session(id)` | Delete session |
| `add_prompt_tile()` | Add prompt with model responses |
| `delete_tile(id)` | Delete prompt or debate tile |
| `update_tile_position()` | Update tile canvas position |
| `update_llm_node_position()` | Update individual LLM node position |
| `update_response_content()` | Update streaming response |
| `add_debate()` | Create debate between models |
| `add_debate_round()` | Add round to debate |
| `update_debate_status()` | Update debate state |
| `get_tile_responses()` | Get responses for debate context |
| `update_viewport()` | Save viewport state |
| `link_note()` | Link canvas to note |
| `get_tile_edges()` | Get parent-child edges |
| `get_node_edges()` | Get all graph edges |
| `find_node_groups()` | Find connected components |
| `batch_update_positions()` | Auto-arrange positions |
| `build_full_history()` | Build conversation context (full) |
| `build_compact_history()` | Build conversation context (compact) |
| `build_semantic_context()` | Build context from vector search |

**Data Structures:**
- Sessions stored as JSON files in `data/canvas/`
- In-memory cache for active sessions
- Node graph with adjacency lists for edges

### 7. OpenRouterService (`services/openrouter.py`)

**Purpose:** Multi-LLM API integration with streaming support

**Key Methods:**
| Method | Description |
|--------|-------------|
| `list_models()` | Get available models (cached 5min) |
| `stream_completion()` | Stream chat completion |
| `complete()` | Non-streaming completion |
| `validate_api_key()` | Test API key |
| `close()` | Close HTTP client |

**Features:**
- Unified API for 100+ AI models (OpenAI, Anthropic, Google, etc.)
- Server-Sent Events (SSE) streaming
- Model metadata (pricing, context length, provider)
- Connection pooling with httpx
- Automatic retry and timeout handling

### 8. DistillationService (`services/distillation.py`)

**Purpose:** Container → Atomic → Hub knowledge workflow

**Key Methods:**
| Method | Description |
|--------|-------------|
| `distill(note_id, request)` | Main entry point for distillation |
| `normalize_tag(tag)` | Normalize tag (lowercase, strip #, space→hyphen) |
| `parse_inline_tags(content)` | Extract #tags from markdown (ignores code/headings) |
| `merge_tags(yaml_tags, inline_tags)` | Merge inline tags into YAML (additive only) |
| `update_protected_section(content, snapshot)` | Safe canvas export update |
| `find_duplicate(candidate, min_score)` | Semantic search for existing atomic notes |

**Features:**
- Tag normalization and inline `#tag` parsing (heading-safe)
- Protected section markers for canvas export preservation
- Dedup filtering (only matches draft/canonical notes)
- Atomic file writes with filelock
- Hub note creation and linking

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
        name="Grafyn Knowledge Graph",
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
| `distill_note` | `note_id`, `mode`, `hub_policy`, `min_score` | Container → Atomic distillation |
| `normalize_tags` | `note_id` | Normalize and merge inline #tags |

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
| httpx | ≥0.28.0 | HTTP client (OAuth, OpenRouter) |
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
│   │   ├── oauth.py         # OAuth endpoints
│   │   └── canvas.py        # Canvas API endpoints (15+)
│   ├── services/
│   │   ├── __init__.py
│   │   ├── knowledge_store.py
│   │   ├── vector_search.py
│   │   ├── graph_index.py
│   │   ├── embedding.py
│   │   ├── token_store.py
│   │   ├── canvas_store.py   # Canvas session management
│   │   ├── openrouter.py     # Multi-LLM API client
│   │   └── distillation.py   # Container → Atomic → Hub workflow
│   ├── middleware/
│   │   ├── __init__.py
│   │   ├── security.py
│   │   ├── logging.py
│   │   └── rate_limit.py
│   ├── models/
│   │   ├── __init__.py
│   │   ├── note.py
│   │   ├── canvas.py        # Canvas data models
│   │   └── distillation.py  # Distillation workflow models
│   └── mcp/
│       ├── __init__.py
│       ├── server.py
│       └── tools.py
├── Dockerfile
├── docker-compose.yml
├── requirements.txt
└── .env.example
```
