# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Quick Reference

| Component | Stack | Entry Point | Port |
|-----------|-------|-------------|------|
| **Backend** | FastAPI, LanceDB, sentence-transformers, OpenRouter | `backend/app/main.py` | 8080 |
| **Frontend** | Vue 3, Vite, Pinia, D3.js | `frontend/src/main.js` | 5173 |

## Development Commands

### Backend

```bash
# Start development server (from backend/)
cd backend
pip install -r requirements.txt
uvicorn app.main:app --reload --host 0.0.0.0 --port 8080

# Alternative: run directly
python -m app.main

# Docker setup
docker-compose up
```

### Frontend

```bash
# Start development server (from frontend/)
cd frontend
npm install
npm run dev

# Build for production
npm run build

# Preview production build
npm run preview

# Lint code
npm run lint

# Format code
npm run format
```

### Testing

```bash
# Backend tests (from backend/)
cd backend
pip install -r requirements-dev.txt

# Run all tests
pytest

# Run with coverage
pytest --cov=app --cov-report=html

# Run specific test categories
pytest -m unit              # Unit tests only
pytest -m integration       # Integration tests only
pytest -m security          # Security tests only
pytest -m "not slow"        # Skip slow tests

# Run specific test file
pytest tests/unit/services/test_knowledge_store.py
```

**Backend Test Coverage (Completed)**:
- ✅ **210+ unit tests** across 5 service files
- ✅ Security tests (path traversal, encryption, CSRF)
- ✅ Vector search and LanceDB integration
- ✅ Graph traversal and backlinks
- ✅ Wikilink parsing and validation
- ✅ Comprehensive fixtures and test data

**Test Files**:
- `tests/unit/services/test_knowledge_store.py` (50+ tests)
- `tests/unit/services/test_vector_search.py` (45+ tests)
- `tests/unit/services/test_graph_index.py` (35+ tests)
- `tests/unit/services/test_token_store.py` (40+ tests)
- `tests/unit/services/test_embedding.py` (40+ tests)

See `backend/tests/README.md` for complete testing documentation.

## Architecture Overview

### Service Layer Design

The backend uses a **singleton service pattern** initialized at startup via `lifespan`:

```python
# Services are attached to app.state and shared across all requests
app.state.knowledge_store    # Markdown CRUD + wikilink parsing
app.state.vector_search       # LanceDB semantic search
app.state.graph_index         # Backlinks + graph traversal
app.state.openrouter          # OpenRouter API client (Multi-LLM)
app.state.canvas_store        # Canvas session storage
```

**Access pattern in routers:**
```python
@router.get("/api/notes")
async def list_notes(request: Request):
    knowledge_store = request.app.state.knowledge_store
    notes = knowledge_store.list_notes()
```

### Core Services

| Service | File | Singleton | Purpose |
|---------|------|-----------|---------|
| `KnowledgeStore` | `services/knowledge_store.py` | Yes | Markdown file I/O, YAML frontmatter, wikilink extraction |
| `VectorSearchService` | `services/vector_search.py` | Yes | LanceDB indexing, semantic search (384-dim vectors) |
| `GraphIndexService` | `services/graph_index.py` | Yes | In-memory adjacency lists for backlinks/outgoing links |
| `EmbeddingService` | `services/embedding.py` | Yes | sentence-transformers wrapper (all-MiniLM-L6-v2) |
| `TokenStore` | `services/token_store.py` | No | OAuth token management (stateful per-instance) |
| `OpenRouterService` | `services/openrouter.py` | Yes | OpenRouter API client with streaming support |
| `CanvasSessionStore` | `services/canvas_store.py` | Yes | Canvas session persistence (JSON file storage) |

### Middleware Order (Applied Bottom-Up)

```python
# Last applied (outermost)
CORSMiddleware              # CORS headers
LoggingMiddleware           # Request/response logging
SecurityHeadersMiddleware   # X-Content-Type-Options, X-Frame-Options
RequestSanitizationMiddleware  # Input validation/sanitization
# First applied (innermost - closest to endpoint)
```

### Frontend State Management

**Pinia Stores:**
- `stores/auth.js` - OAuth state, user session, token management
- `stores/notes.js` - Note list, selected note, CRUD operations
- `stores/canvas.js` - Canvas sessions, tiles, SSE streaming, debates

**API Client Pattern:**
```javascript
// src/api/client.js provides typed API calls
import { notes, search, graph, auth, canvas } from '@/api/client'

// All API calls use Axios with JSON body
const result = await notes.get('note-id')
const models = await canvas.getModels()
```

## Key Concepts

### Wikilink Pattern

**In Markdown:**
```markdown
[[Note Title]]              → Links to note with exact title
[[Note Title|Display]]      → Custom display text
```

**Parsing:** `KnowledgeStore.extract_wikilinks()` uses regex: `\[\[(.*?)(?:\|(.*?))?\]\]`

**Graph Index:**
- Parses all notes on `build_index()` to construct adjacency lists
- Backlinks computed as reverse edges: if A links to B, B has backlink from A

### Note Status Workflow

```
draft → evidence → canonical
```

Stored in YAML frontmatter `status` field. Frontend filters/displays based on status.

### Vector Embeddings

- **Model:** `all-MiniLM-L6-v2` (384 dimensions)
- **Storage:** LanceDB with schema `{note_id, title, text, vector}`
- **Index trigger:** Automatic on note create/update via router
- **Search:** Cosine similarity via LanceDB's `.search(query_vector).limit(n)`

### MCP Integration

- **Endpoint:** `/sse` (Server-Sent Events for MCP protocol)
- **Library:** `fastapi-mcp` wraps FastAPI routes as MCP tools
- **Setup:** `setup_mcp(app)` in `main.py` auto-exposes tagged endpoints
- **OAuth:** GitHub OAuth required for ChatGPT integration (tokens in `TokenStore`)

### Multi-LLM Canvas

The Canvas feature allows comparing responses from multiple LLM models simultaneously.

**Features:**
- Send one prompt to multiple models via OpenRouter
- Real-time SSE streaming of responses
- Infinite canvas with drag-and-drop tiles (D3.js zoom/pan)
- Model debate mode (auto or user-mediated)
- Session persistence (JSON files in `data/canvas/`)

**Architecture:**
```
User Prompt → CanvasStore (create tile) → OpenRouterService (parallel streams)
                                                    ↓
                                        SSE multiplexing via asyncio.Queue
                                                    ↓
                                        Frontend updates per-model content
```

**API Endpoints (`/api/canvas`):**
| Method | Endpoint | Purpose |
|--------|----------|---------|
| GET | `/` | List all sessions |
| POST | `/` | Create new session |
| GET | `/{id}` | Get session with tiles |
| PUT | `/{id}` | Update session metadata |
| DELETE | `/{id}` | Delete session |
| GET | `/models/available` | List OpenRouter models |
| POST | `/{id}/prompt` | Send prompt to models (SSE) |
| PUT | `/{id}/tiles/{tid}/position` | Update tile position |
| POST | `/{id}/debate` | Start model debate (SSE) |

**Frontend Route:** `/canvas` and `/canvas/:id`

## Configuration

### Environment Setup

```bash
# Backend (required) - run from project root
cp backend/.env.example .env
# Edit .env with:
# - VAULT_PATH (default: ../vault)
# - DATA_PATH (default: ../data)
# - GITHUB_CLIENT_ID/SECRET (for MCP OAuth)
# - TOKEN_ENCRYPTION_KEY (generate with: python -c "from cryptography.fernet import Fernet; print(Fernet.generate_key().decode())")
# - OPENROUTER_API_KEY (for Multi-LLM Canvas feature)

# Note: .env must be in project root when running from root
```

### Critical Settings

| Variable | Default | Notes |
|----------|---------|-------|
| `ENVIRONMENT` | `development` | Affects CORS policy (strict in production) |
| `CORS_ORIGINS` | `*` (dev) | Comma-separated in production |
| `RATE_LIMIT_ENABLED` | `true` | Slowapi rate limiting (10/min, 50/hr, 200/day) |
| `EMBEDDING_MODEL` | `all-MiniLM-L6-v2` | Must match LanceDB vector dimension (384) |
| `OPENROUTER_API_KEY` | `""` | Required for Multi-LLM Canvas feature |
| `APP_URL` | `http://localhost:8080` | Used in OpenRouter API headers |
| `CANVAS_DATA_PATH` | `../data/canvas` | JSON storage for canvas sessions |

## Common Patterns

### Adding a New Router Endpoint

```python
# In routers/example.py
from fastapi import APIRouter, Request
from app.middleware.rate_limit import limiter

router = APIRouter()

@router.get("/example")
@limiter.limit("10 per minute")
async def example(request: Request):
    service = request.app.state.knowledge_store
    return service.some_method()

# In main.py
app.include_router(example.router, prefix="/api/example", tags=["example"])
```

### Accessing Services in Custom Code

```python
# Services are singletons attached to app.state
def my_function(app: FastAPI):
    knowledge_store = app.state.knowledge_store
    vector_search = app.state.vector_search
    graph_index = app.state.graph_index
```

### Frontend Component Data Flow

```
User Action → Vue Component → Pinia Store Action → API Client → Backend Endpoint
                                        ↓
                                   Update State
                                        ↓
                                 Reactivity triggers re-render
```

## Project Structure Deep Dive

### Backend (`backend/app/`)

```
routers/
  notes.py     - 6 endpoints (CRUD, list, reindex)
  search.py    - 2 endpoints (query, similar)
  graph.py     - 5 endpoints (backlinks, outgoing, neighbors, unlinked, rebuild)
  oauth.py     - 2 endpoints (GitHub OAuth flow)
  canvas.py    - 9 endpoints (sessions, prompts, debates, SSE streaming)

services/
  knowledge_store.py  - Markdown I/O, frontmatter parsing
  vector_search.py    - LanceDB wrapper with NoteEmbedding schema
  graph_index.py      - Adjacency lists (Dict[str, Set[str]])
  embedding.py        - sentence-transformers SentenceTransformer wrapper
  token_store.py      - In-memory OAuth token storage with TTL
  openrouter.py       - OpenRouter API client with streaming
  canvas_store.py     - Canvas session CRUD (JSON file storage)

models/
  note.py      - Note, NoteCreate, NoteUpdate schemas
  canvas.py    - CanvasSession, PromptTile, ModelResponse, DebateRound schemas

middleware/
  security.py   - SecurityHeadersMiddleware, RequestSanitizationMiddleware
  logging.py    - Request/response logging
  rate_limit.py - Slowapi limiter configuration

mcp/
  server.py  - setup_mcp() function to mount MCP endpoint
  tools.py   - MCP tool definitions (query_knowledge, get_note, etc.)
```

### Frontend (`frontend/src/`)

```
views/
  HomeView.vue          - Main app (header, sidebar, editor, backlinks panel)
  CanvasView.vue        - Multi-LLM Canvas with session sidebar
  LoginView.vue         - GitHub OAuth login page
  OAuthCallbackView.vue - OAuth code→token exchange
  NotFoundView.vue      - 404 page

components/
  SearchBar.vue      - Debounced semantic search dropdown
  NoteList.vue       - Sidebar list with status badges
  NoteEditor.vue     - Markdown editor with edit/preview toggle
  BacklinksPanel.vue - Right panel showing incoming links
  GraphView.vue      - Graph visualization (neighbors endpoint)

components/canvas/
  CanvasContainer.vue   - Infinite canvas with D3 zoom/pan
  PromptTile.vue        - Draggable tile with prompt + model responses
  ModelResponseCard.vue - Single model response with streaming
  ModelSelector.vue     - Model picker grouped by provider
  PromptDialog.vue      - New prompt modal with settings
  DebateTile.vue        - Debate visualization with rounds
  DebateControls.vue    - Auto/mediated debate toggle

stores/
  auth.js   - { user, token, isAuthenticated } + login/logout/checkAuth
  notes.js  - { notes[], selectedNote } + loadNotes/createNote/updateNote/deleteNote
  canvas.js - { sessions[], currentSession, availableModels } + SSE streaming
```

## Data Models

### Note Object (Pydantic)

```python
class Note(BaseModel):
    id: str                  # Filename without .md
    title: str
    content: str             # Markdown body
    status: str              # draft|evidence|canonical
    tags: List[str]
    created_at: datetime
    updated_at: datetime
    wikilinks: List[str]     # Extracted [[links]]
```

### YAML Frontmatter Format

```markdown
---
title: Note Title
status: draft
tags: [tag1, tag2]
created_at: 2025-01-07T12:00:00Z
updated_at: 2025-01-07T12:00:00Z
---

Markdown content here with [[wikilinks]].
```

### Canvas Session Object (Pydantic)

```python
class CanvasSession(BaseModel):
    id: str
    title: str
    description: Optional[str]
    prompt_tiles: List[PromptTile]      # Tiles with prompts + responses
    debates: List[DebateRound]          # Model debate rounds
    viewport: CanvasViewport            # { x, y, zoom }
    created_at: datetime
    updated_at: datetime
    tags: List[str]
    status: str                         # draft|evidence|canonical

class PromptTile(BaseModel):
    id: str
    prompt: str
    system_prompt: Optional[str]
    models: List[str]                   # Model IDs sent to
    responses: Dict[str, ModelResponse] # model_id -> response
    position: TilePosition              # { x, y, width, height }

class ModelResponse(BaseModel):
    id: str
    model_id: str                       # e.g., "openai/gpt-4o"
    model_name: str                     # Display name
    content: str                        # Streamed response
    status: str                         # pending|streaming|completed|error
```

## Testing

No test framework is currently configured. When adding tests:

**Backend:** Use `pytest` with `pytest-asyncio` for async tests
**Frontend:** Use Vitest (already compatible with Vite) or Jest

## Deployment Notes

- **CORS:** Set `CORS_ORIGINS` to specific domains in production (comma-separated)
- **Rate Limiting:** Adjust `RATE_LIMIT_*` settings based on expected load
- **OAuth:** Configure GitHub OAuth app with production redirect URI
- **Encryption:** Generate production `TOKEN_ENCRYPTION_KEY` with: `python -c "from cryptography.fernet import Fernet; print(Fernet.generate_key().decode())"`
- **Docker:** Use `docker-compose.yml` in `backend/` for containerized deployment
