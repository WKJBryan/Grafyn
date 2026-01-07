# Seedream Source Tree Analysis

> **Scan Level:** Exhaustive | **Total Files Scanned:** 30+

## Project Structure Overview

```
seedream/                        # Project root
├── .gitignore                   # Git ignore rules
├── README.md                    # Project documentation
├── pyproject.toml               # Python dependencies (uv)
├── uv.lock                      # Dependency lock file
│
├── backend/                     # FastAPI Python service
│   ├── .env.example             # Environment template
│   ├── Dockerfile               # Container build
│   ├── docker-compose.yml       # Docker setup
│   ├── requirements.txt         # Legacy pip requirements
│   └── app/                     # Application code
│       ├── __init__.py
│       ├── main.py              # ★ Entry point - FastAPI app setup
│       ├── config.py            # Settings from environment
│       │
│       ├── routers/             # API endpoints
│       │   ├── __init__.py
│       │   ├── notes.py         # Note CRUD (6 endpoints)
│       │   ├── search.py        # Semantic search (2 endpoints)
│       │   ├── graph.py         # Link graph (5 endpoints)
│       │   └── oauth.py         # OAuth authentication
│       │
│       ├── services/            # Business logic layer
│       │   ├── __init__.py
│       │   ├── knowledge_store.py  # Markdown note I/O
│       │   ├── vector_search.py    # LanceDB semantic search
│       │   ├── graph_index.py      # Wikilink parsing & backlinks
│       │   ├── embedding.py        # Text → vector encoding
│       │   └── token_store.py      # OAuth token management
│       │
│       ├── middleware/          # Request/response middleware
│       │   ├── __init__.py
│       │   ├── security.py      # Security headers & sanitization
│       │   ├── logging.py       # Request logging
│       │   └── rate_limit.py    # Rate limiting
│       │
│       ├── models/              # Pydantic schemas
│       │   ├── __init__.py
│       │   └── note.py          # Note, SearchResult, BacklinkInfo
│       │
│       └── mcp/                 # Model Context Protocol
│           ├── __init__.py
│           ├── server.py        # MCP server setup
│           └── tools.py         # 6 MCP tools for AI models
│
├── frontend/                    # Vue 3 SPA
│   ├── index.html               # HTML entry point
│   ├── package.json             # Node dependencies
│   ├── vite.config.js           # Vite configuration
│   ├── jsconfig.json            # JavaScript config
│   ├── .eslintrc.cjs            # ESLint config
│   ├── .prettierrc              # Prettier config
│   └── src/                     # Source code
│       ├── main.js              # ★ Entry point - Vue app with Pinia & Router
│       ├── App.vue              # Root component (router-view)
│       ├── style.css            # Design system (CSS variables)
│       │
│       ├── router/              # Vue Router configuration
│       │   └── index.js         # Route definitions
│       │
│       ├── stores/              # Pinia state management
│       │   ├── auth.js          # Authentication state
│       │   └── notes.js         # Notes state
│       │
│       ├── views/               # Page components
│       │   ├── HomeView.vue     # Main application view
│       │   ├── LoginView.vue    # Login page
│       │   ├── OAuthCallbackView.vue  # OAuth callback handler
│       │   └── NotFoundView.vue # 404 page
│       │
│       ├── api/                 # Backend communication
│       │   └── client.js        # REST API client
│       │
│       └── components/          # Vue components
│           ├── SearchBar.vue    # Semantic search input
│           ├── NoteList.vue     # Sidebar note listing
│           ├── NoteEditor.vue   # Markdown editor/preview
│           ├── BacklinksPanel.vue  # Backlinks display
│           └── GraphView.vue    # Graph visualization
│
├── vault/                       # Markdown notes storage
│   └── *.md                     # Note files
│
├── data/                        # LanceDB vector storage
│   └── .gitkeep                 # Placeholder
│
└── docs/                        # Generated documentation
    ├── index.md                 # ★ Master documentation index
    ├── getting-started.md       # Step-by-step setup guide
    ├── project-overview.md      # High-level overview
    ├── source-tree-analysis.md  # This file
    ├── integration-architecture.md  # Frontend ↔ Backend
    ├── architecture-backend.md  # Backend deep dive
    ├── architecture-frontend.md # Frontend deep dive
    ├── api-contracts-backend.md # API endpoint docs
    ├── data-models-backend.md   # Pydantic schemas
    ├── component-inventory-frontend.md  # Vue components
    ├── development-guide-backend.md  # Backend setup
    └── development-guide-frontend.md # Frontend setup
```

---

## Critical Directories

### Backend (`backend/app/`)

| Directory | Purpose | Contents |
|-----------|---------|----------|
| `routers/` | API endpoint definitions | 4 router files, 15+ endpoints |
| `services/` | Business logic | 5 service classes |
| `middleware/` | Request processing | Security, logging, rate limiting |
| `models/` | Data schemas | 8 Pydantic models |
| `mcp/` | AI integration | MCP server + 6 tools |

### Frontend (`frontend/src/`)

| Directory | Purpose | Contents |
|-----------|---------|----------|
| `router/` | Vue Router setup | Route definitions |
| `stores/` | Pinia state management | auth.js, notes.js |
| `views/` | Page components | 4 view files |
| `components/` | UI components | 5 Vue SFCs |
| `api/` | Backend client | HTTP wrapper |

---

## Entry Points

### Backend

**Primary:** `backend/app/main.py`
- FastAPI application initialization
- Middleware registration (security, logging, rate limiting)
- Router registration (notes, search, graph, oauth)
- MCP server setup

**Run Command:**
```bash
# Using uv (from project root)
uv run uvicorn backend.app.main:app --reload --host 0.0.0.0 --port 8080

# Or from backend directory
cd backend
uv run uvicorn app.main:app --reload --host 0.0.0.0 --port 8080
```

### Frontend

**Primary:** `frontend/src/main.js`
- Vue 3 application bootstrap
- Pinia store initialization
- Vue Router installation
- Root component mounting

**Run Command:**
```bash
cd frontend
npm run dev
```

---

## Data Directories

### vault/ (Markdown Storage)
- **Format:** Markdown with YAML frontmatter
- **Link Format:** `[[wikilinks]]` (Obsidian-compatible)
- **Status Values:** `draft`, `evidence`, `canonical`

### data/ (Vector Storage)
- **Database:** LanceDB
- **Location:** `data/lancedb/`
- **Table:** `notes`
- **Schema:** `NoteEmbedding` (id, title, text, vector[384])

---

## Integration Points

### Frontend → Backend

```
frontend/src/api/client.js → GET/POST/PUT/DELETE → backend/app/routers/*.py
                                     ↓
                         http://localhost:8080/api/*
```

**Proxy Configuration (Vite):**
- `/api/*` → `http://localhost:8080`
- `/sse/*` → `http://localhost:8080`
- `/auth/*` → `http://localhost:8080`

### Backend → Storage

```
backend/app/services/
├── knowledge_store.py → vault/*.md (filesystem)
├── vector_search.py → data/lancedb/ (LanceDB)
├── graph_index.py → in-memory adjacency lists
└── token_store.py → in-memory token storage
```

---

## File Counts by Type

| Extension | Count | Location |
|-----------|-------|----------|
| `.py` | 16+ | `backend/app/` |
| `.vue` | 10 | `frontend/src/` (5 components + 4 views + App.vue) |
| `.js` | 5+ | `frontend/` |
| `.css` | 1 | `frontend/src/` |
| `.md` | 15+ | `docs/`, `vault/`, root |
| `.json` | 3 | `frontend/`, root |

---

## Gitignored Paths

| Pattern | Description |
|---------|-------------|
| `.venv/` | Python virtual environment |
| `node_modules/` | Node.js dependencies |
| `__pycache__/` | Python bytecode |
| `data/lancedb/` | Vector database |
| `.env` | Environment secrets |
| `dist/` | Build output |
