# OrgAI Backend Architecture

> **Part:** Backend | **Type:** FastAPI Python Service | **Scan Level:** Exhaustive

## Overview

The backend is a FastAPI-based REST API providing:
- Note CRUD operations with Markdown/YAML frontmatter support
- Semantic vector search using LanceDB + sentence-transformers
- Knowledge graph with wikilink parsing and backlinks
- MCP server for external AI model integration

## Entry Point

**File:** `backend/app/main.py`

```python
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware

from app.config import get_settings
from app.routers import notes, search, graph
from app.mcp.server import setup_mcp

app = FastAPI(
    title="OrgAI",
    description="Knowledge Graph Platform with Semantic Search and MCP",
    version="0.1.0",
)

# CORS middleware for frontend access
app.add_middleware(CORSMiddleware, allow_origins=["*"], ...)

# Include routers
app.include_router(notes.router, prefix="/api/notes", tags=["notes"])
app.include_router(search.router, prefix="/api/search", tags=["search"])
app.include_router(graph.router, prefix="/api/graph", tags=["graph"])

# Setup MCP server
setup_mcp(app)
```

## Architecture Layers

```
┌──────────────────────────────────────────────────────────────┐
│                      API Layer (Routers)                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐   │
│  │ notes.py    │  │ search.py   │  │ graph.py            │   │
│  │ 6 endpoints │  │ 2 endpoints │  │ 4 endpoints         │   │
│  └─────────────┘  └─────────────┘  └─────────────────────┘   │
├──────────────────────────────────────────────────────────────┤
│                     Service Layer                             │
│  ┌────────────────┐ ┌─────────────────┐ ┌─────────────────┐  │
│  │ KnowledgeStore │ │ VectorSearch    │ │ GraphIndex      │  │
│  │ (Markdown I/O) │ │ (LanceDB)       │ │ (Link tracking) │  │
│  └────────────────┘ └─────────────────┘ └─────────────────┘  │
│            │                 │                               │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │              EmbeddingService                            │ │
│  │              (sentence-transformers)                     │ │
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
    vault_path: str = "../vault"
    data_path: str = "../data"
    server_host: str = "0.0.0.0"
    server_port: int = 8080
    embedding_model: str = "all-MiniLM-L6-v2"
    
    class Config:
        env_file = ".env"
```

| Setting | Default | Purpose |
|---------|---------|---------|
| `VAULT_PATH` | `../vault` | Markdown notes directory |
| `DATA_PATH` | `../data` | LanceDB storage |
| `SERVER_HOST` | `0.0.0.0` | Bind address |
| `SERVER_PORT` | `8080` | HTTP port |
| `EMBEDDING_MODEL` | `all-MiniLM-L6-v2` | Sentence transformer model |

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

**Wikilink Pattern:**
```python
WIKILINK_PATTERN = re.compile(r'\[\[([^\]|]+)(?:\|[^\]]+)?\]\]')
```

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
| `update_note(id, old, new)` | Incremental graph update |

### 4. EmbeddingService (`services/embedding.py`)

**Purpose:** Text to vector encoding

**Key Methods:**
| Method | Description |
|--------|-------------|
| `encode(text)` | Single text → vector |
| `encode_batch(texts)` | Batch encoding |
| `dimension` | Returns 384 (model output size) |

## MCP Integration

**Files:** `mcp/server.py`, `mcp/tools.py`

The backend exposes FastAPI routes as MCP tools via `fastapi-mcp`:

```python
def setup_mcp(app: FastAPI) -> None:
    from fastapi_mcp import FastApiMCP
    
    mcp = FastApiMCP(
        app,
        name="OrgAI Knowledge Graph",
        description="Access and query an organizational knowledge base",
    )
    mcp.mount()  # Mounts at /mcp
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
| fastapi | ≥0.104.0 | Web framework |
| uvicorn | ≥0.24.0 | ASGI server |
| pydantic | ≥2.5.0 | Data validation |
| pydantic-settings | ≥2.1.0 | Settings management |
| lancedb | ≥0.3.0 | Vector database |
| sentence-transformers | ≥2.2.0 | Embeddings |
| python-frontmatter | ≥1.0.0 | YAML frontmatter parsing |
| fastapi-mcp | ≥0.1.0 | MCP integration |
| python-dotenv | ≥1.0.0 | Environment loading |

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
│   │   └── graph.py         # Graph endpoints
│   ├── services/
│   │   ├── __init__.py
│   │   ├── knowledge_store.py  # Markdown I/O
│   │   ├── vector_search.py    # LanceDB search
│   │   ├── graph_index.py      # Link tracking
│   │   └── embedding.py        # Text encoding
│   ├── models/
│   │   ├── __init__.py
│   │   └── note.py          # Pydantic schemas
│   └── mcp/
│       ├── __init__.py
│       ├── server.py        # MCP setup
│       └── tools.py         # MCP tool definitions
├── requirements.txt
├── .env.example
└── venv/                    # Virtual environment
```
