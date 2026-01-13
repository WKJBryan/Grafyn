# Seedream Knowledge Graph Platform - Project Documentation

> **Generated:** 2026-01-11 | **Scan Level:** Exhaustive | **Mode:** Full Rescan

## Project Overview

| Property | Value |
|----------|-------|
| **Type** | Multi-part (Backend + Frontend) |
| **Primary Language** | Python (Backend), JavaScript (Frontend) |
| **Architecture** | Service Layer + Middleware + Component-Based UI |
| **Repository Structure** | Monorepo with `backend/` and `frontend/` |

## Quick Reference

### Backend (FastAPI)
- **Framework:** FastAPI 0.128+
- **Database:** LanceDB (vector storage)
- **Embeddings:** sentence-transformers (all-MiniLM-L6-v2, 384 dimensions)
- **Entry Point:** `backend/app/main.py`
- **API Prefix:** `/api/`
- **Services:** 7 core services (KnowledgeStore, VectorSearch, GraphIndex, Embedding, TokenStore, CanvasSessionStore, OpenRouterService)
- **Middleware:** Security, Logging, Rate Limiting

### Frontend (Vue 3)
- **Framework:** Vue 3.4 with Composition API
- **State Management:** Pinia (3 stores: auth, notes, canvas)
- **Routing:** Vue Router
- **Build Tool:** Vite 5
- **HTTP Client:** Axios
- **Markdown:** marked 11.0+
- **Entry Point:** `frontend/src/main.js`
- **Components:** 14 Vue components + 5 Views
- **Visualization:** D3.js for canvas graphs

---

## Generated Documentation

### Getting Started
- **[Getting Started Guide](./getting-started.md)** ⭐ - Step-by-step setup instructions

### Core Documentation
- [Project Overview](./project-overview.md)
- [Source Tree Analysis](./source-tree-analysis.md)
- [Integration Architecture](./integration-architecture.md)

### Backend Documentation
- [Architecture - Backend](./architecture-backend.md)
- [API Contracts - Backend](./api-contracts-backend.md)
- [Data Models - Backend](./data-models-backend.md)
- [Development Guide - Backend](./development-guide-backend.md)

### Frontend Documentation
- [Architecture - Frontend](./architecture-frontend.md)
- [Component Inventory - Frontend](./component-inventory-frontend.md)
- [Development Guide - Frontend](./development-guide-frontend.md)

### Canvas/Multi-LLM Feature
- **[Canvas Architecture](./canvas-architecture.md)** ⭐ - Multi-LLM comparison canvas system

### AI Integration
- [Chat Ingestion Guide](./chat-ingestion-guide.md) - MCP setup, export scripts, ingestion workflows

---

## API Summary (30+ Endpoints)

| Category | Endpoints | Description |
|----------|-----------|-------------|
| **Notes** | 6 | CRUD operations + reindex |
| **Search** | 2 | Semantic/lexical search + similar notes |
| **Graph** | 5 | Backlinks, neighbors, unlinked mentions, rebuild |
| **OAuth** | 3+ | GitHub OAuth authentication |
| **Canvas** | 15+ | Multi-LLM canvas sessions, prompts, debates |
| **System** | 2 | Health check + API info |

## Service Summary (7 Services)

| Service | Purpose |
|---------|---------|
| **KnowledgeStore** | Markdown note CRUD with YAML frontmatter |
| **VectorSearchService** | LanceDB-backed semantic search |
| **GraphIndexService** | Wikilink parsing and backlink tracking |
| **EmbeddingService** | Text→vector via sentence-transformers |
| **TokenStore** | OAuth token management |
| **CanvasSessionStore** | Multi-LLM canvas session persistence |
| **OpenRouterService** | Multi-LLM API integration with streaming |

## Middleware

| Middleware | Purpose |
|------------|---------|
| **SecurityHeadersMiddleware** | Security headers injection |
| **RequestSanitizationMiddleware** | Input sanitization |
| **LoggingMiddleware** | Request/response logging |
| **Rate Limiting** | API rate limiting (slowapi) |

## MCP Tools (6 Tools)

| Tool | Description |
|------|-------------|
| `query_knowledge` | Semantic search the knowledge base |
| `get_note` | Retrieve full note content |
| `list_notes` | List notes with filtering |
| `get_backlinks` | Get notes linking to a note |
| `ingest_chat` | Store chat transcripts as evidence |
| `create_draft` | Create draft notes for human review |

---

## Existing Documentation

- [README.md](../README.md) - Project overview and quick start guide
- [IMPROVEMENTS.md](./IMPROVEMENTS.md) - Summary of codebase improvements

---

## Getting Started

### For Backend Development
```bash
# Using uv (recommended)
uv sync
uv run uvicorn backend.app.main:app --reload --host 0.0.0.0 --port 8080

# Or from backend directory
cd backend
cp .env.example .env
uv run uvicorn app.main:app --reload --host 0.0.0.0 --port 8080
```

### For Frontend Development
```bash
cd frontend
npm install
npm run dev
```

### Access Points
- **Frontend UI:** http://localhost:5173
- **Backend API:** http://localhost:8080
- **API Docs:** http://localhost:8080/docs
- **MCP Endpoint:** http://localhost:8080/sse
- **OAuth:** http://localhost:8080/api/oauth
- **Canvas View:** http://localhost:5173/canvas

---

## AI-Assisted Development

When working with AI coding assistants, point them to:
1. This index file for project overview
2. Specific architecture docs for component understanding
3. API contracts for endpoint details
4. Data models for schema information

### MCP Integration
The backend exposes 6 MCP tools for external AI models:
- `query_knowledge` - Semantic search
- `get_note` - Retrieve note content
- `list_notes` - List all notes
- `get_backlinks` - Get note connections
- `ingest_chat` - Store conversations
- `create_draft` - Create draft notes
