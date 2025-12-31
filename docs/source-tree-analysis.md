# OrgAI Source Tree Analysis

> **Scan Level:** Exhaustive | **Total Files Scanned:** 25+

## Project Structure Overview

```
orgai/                           # Project root
├── .gitignore                   # Git ignore rules
├── README.md                    # Project documentation
│
├── backend/                     # FastAPI Python service
│   ├── .env.example             # Environment template
│   ├── requirements.txt         # Python dependencies
│   ├── venv/                    # Virtual environment (gitignored)
│   └── app/                     # Application code
│       ├── __init__.py
│       ├── main.py              # ★ Entry point - FastAPI app setup
│       ├── config.py            # Settings from environment
│       │
│       ├── routers/             # API endpoints
│       │   ├── __init__.py
│       │   ├── notes.py         # Note CRUD (6 endpoints)
│       │   ├── search.py        # Semantic search (2 endpoints)
│       │   └── graph.py         # Link graph (4 endpoints)
│       │
│       ├── services/            # Business logic layer
│       │   ├── __init__.py
│       │   ├── knowledge_store.py  # Markdown note I/O
│       │   ├── vector_search.py    # LanceDB semantic search
│       │   ├── graph_index.py      # Wikilink parsing & backlinks
│       │   └── embedding.py        # Text → vector encoding
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
│   ├── package-lock.json        # Lock file
│   ├── vite.config.js           # Vite configuration
│   ├── node_modules/            # Dependencies (gitignored)
│   └── src/                     # Source code
│       ├── main.js              # ★ Entry point - Vue app bootstrap
│       ├── App.vue              # Root component (layout)
│       ├── style.css            # Design system (CSS variables)
│       │
│       ├── api/                 # Backend communication
│       │   └── client.js        # REST API client
│       │
│       └── components/          # Vue components
│           ├── SearchBar.vue    # Semantic search input
│           ├── NoteList.vue     # Sidebar note listing
│           ├── NoteEditor.vue   # Markdown editor/preview
│           ├── BacklinksPanel.vue  # Backlinks display
│           └── GraphView.vue    # Graph viz (Phase 2)
│
├── vault/                       # Markdown notes storage
│   ├── Welcome.md               # Welcome note
│   └── Example Note.md          # Example/tutorial note
│
├── data/                        # LanceDB vector storage
│   └── .gitkeep                 # Placeholder
│
└── docs/                        # Generated documentation
    ├── index.md                 # ★ Master documentation index
    ├── project-overview.md      # High-level overview
    ├── source-tree-analysis.md  # This file
    ├── integration-architecture.md  # Frontend ↔ Backend
    ├── architecture-backend.md  # Backend deep dive
    ├── architecture-frontend.md # Frontend deep dive
    ├── api-contracts-backend.md # API endpoint docs
    ├── data-models-backend.md   # Pydantic schemas
    ├── component-inventory-frontend.md  # Vue components
    ├── development-guide-backend.md  # Backend setup
    ├── development-guide-frontend.md # Frontend setup
    └── project-scan-report.json # Scan state/metadata
```

---

## Critical Directories

### Backend (`backend/app/`)

| Directory | Purpose | Contents |
|-----------|---------|----------|
| `routers/` | API endpoint definitions | 3 router files, 14 endpoints |
| `services/` | Business logic | 4 service classes (singleton pattern) |
| `models/` | Data schemas | 8 Pydantic models |
| `mcp/` | AI integration | MCP server + 6 tools |

### Frontend (`frontend/src/`)

| Directory | Purpose | Contents |
|-----------|---------|----------|
| `components/` | UI components | 5 Vue SFCs |
| `api/` | Backend client | HTTP wrapper |

---

## Entry Points

### Backend

**Primary:** `backend/app/main.py`
- FastAPI application initialization
- Middleware configuration (CORS)
- Router registration
- MCP server setup

**Run Command:**
```bash
cd backend
uvicorn app.main:app --reload --host 0.0.0.0 --port 8080
```

### Frontend

**Primary:** `frontend/src/main.js`
- Vue 3 application bootstrap
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
- `/mcp/*` → `http://localhost:8080`

### Backend → Storage

```
backend/app/services/
├── knowledge_store.py → vault/*.md (filesystem)
├── vector_search.py → data/lancedb/ (LanceDB)
└── graph_index.py → in-memory adjacency lists
```

---

## File Counts by Type

| Extension | Count | Location |
|-----------|-------|----------|
| `.py` | 12 | `backend/app/` |
| `.vue` | 6 | `frontend/src/` |
| `.js` | 4 | `frontend/` |
| `.css` | 1 | `frontend/src/` |
| `.md` | 14 | `docs/`, `vault/`, root |
| `.json` | 3 | `frontend/`, `docs/` |

---

## Gitignored Paths

| Pattern | Description |
|---------|-------------|
| `venv/` | Python virtual environment |
| `node_modules/` | Node.js dependencies |
| `__pycache__/` | Python bytecode |
| `data/lancedb/` | Vector database |
| `.env` | Environment secrets |
| `dist/` | Build output |
