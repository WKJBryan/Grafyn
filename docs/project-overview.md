# Grafyn Knowledge Graph Platform - Project Overview

## Executive Summary

Grafyn is a **local knowledge graph platform** that enables semantic search, Obsidian-style linking, and MCP (Model Context Protocol) integration. It provides a self-hosted knowledge base with AI-ready capabilities for personal or organizational use.

## Key Features

| Feature | Description |
|---------|-------------|
| **Obsidian-compatible notes** | Markdown files with YAML frontmatter and `[[wikilinks]]` |
| **Semantic search** | Vector-based search using sentence-transformers (all-MiniLM-L6-v2) |
| **Backlinks** | Automatic bidirectional link tracking and discovery |
| **MCP Server** | Connects external AI models (Claude, ChatGPT, Gemini) to knowledge base |
| **OAuth Authentication** | GitHub OAuth for secure ChatGPT integration |
| **Multi-LLM Canvas** | Interactive canvas for comparing responses from multiple AI models with branching conversations |
| **Debate Mode** | AI models can debate and critique each other's responses |
| **Web UI** | Vue 3 SPA with Pinia state management and D3.js visualizations |

## Technology Stack Summary

| Layer | Technology | Version | Purpose |
|-------|------------|---------|---------|
| **Backend Framework** | FastAPI | 0.128+ | REST API server |
| **Vector Database** | LanceDB | 0.26+ | Embedding storage and similarity search |
| **Embeddings** | sentence-transformers | 5.2+ | Text to vector encoding (384 dimensions) |
| **MCP Integration** | fastapi-mcp | Latest | AI model protocol bridge |
| **Multi-LLM API** | OpenRouter | Latest | Unified API for 100+ AI models |
| **Data Validation** | Pydantic | 2.12+ | Request/response schemas |
| **Rate Limiting** | slowapi | Latest | API protection |
| **Frontend Framework** | Vue 3 | 3.4+ | Reactive UI components |
| **State Management** | Pinia | Latest | Centralized state |
| **Routing** | Vue Router | 4.2+ | SPA navigation |
| **Build Tool** | Vite | 5.0+ | Fast development server |
| **HTTP Client** | Axios | Latest | API communication |
| **Visualization** | D3.js | v7+ | Canvas graph visualization |
| **Markdown** | marked | 11.0+ | Content rendering |

## Architecture Type

**Multi-Part Monorepo** with clear separation:

```
grafyn/
├── backend/     → FastAPI Python service (data, search, MCP, Canvas)
│   └── app/
│       ├── main.py           # Application entry point
│       ├── config.py         # Settings from .env
│       ├── routers/          # API endpoints (notes, search, graph, oauth, canvas)
│       ├── services/         # Business logic (7 services)
│       ├── models/           # Pydantic schemas
│       ├── middleware/       # Security, logging, rate limiting
│       └── mcp/              # MCP server integration
├── frontend/    → Vue 3 SPA (user interface)
│   └── src/
│       ├── main.js           # Vue app bootstrap
│       ├── App.vue           # Root component
│       ├── router/           # Vue Router configuration
│       ├── stores/           # Pinia stores (auth, notes, canvas)
│       ├── views/            # Page components (5 total)
│       ├── components/       # UI components (14 total)
│       │   └── canvas/      # Canvas-specific components (9)
│       ├── api/              # Backend client
│       └── style.css         # Design system
├── vault/       → Markdown notes storage (Obsidian-compatible)
└── data/        → LanceDB vector storage + Canvas sessions
```

## Data Flow

```
User → Vue 3 Frontend → REST API → FastAPI Backend
                                         ↓
                        ┌────────────────┼────────────────┐
                        ↓                ↓                ↓
                  KnowledgeStore   VectorSearch    GraphIndex
                   (Markdown)       (LanceDB)      (Links)
                        ↓                ↓                ↓
                      vault/          data/         In-memory
```

## External AI Integration

```
External AI (Claude/ChatGPT) → MCP Protocol → /sse endpoint
                                                    ↓
                                            FastAPI routes
                                            exposed as tools

Multi-LLM Canvas:
OpenRouter API (100+ models) → Canvas API → Frontend Canvas
                                                    ↓
                                          SSE streaming
                                          (real-time responses)
```

## Core Concepts

### Notes
- Stored as `.md` files in `vault/` directory
- YAML frontmatter for metadata (title, tags, status, dates)
- Support `[[wikilinks]]` for inter-note linking
- Status workflow: `draft` → `evidence` → `canonical`

### Semantic Search
- Notes are embedded using `all-MiniLM-L6-v2` (384-dim vectors)
- Stored in LanceDB for fast similarity search
- Supports both semantic and lexical search modes
- Canvas tiles are also indexed for RAG-style context retrieval

### Knowledge Graph
- Wikilinks create directed edges between notes
- Backlinks computed on-demand from outgoing links
- Neighbor traversal up to 3 hops for visualization
- Unlinked mention detection for link suggestions

### Canvas (Multi-LLM Comparison)
- Visual canvas for comparing AI model responses side-by-side
- Supports branching conversations from any model response
- Three context modes: Full History, Compact, Semantic (RAG)
- Debate mode for AI models to critique each other
- Individual LLM nodes with draggable positioning
- Color-coded responses by model
- Export canvas content as markdown notes

### Authentication
- OAuth 2.0 with GitHub for ChatGPT integration
- Token-based session management
- Environment-based CORS configuration (stricter in production)

## Statistics

| Metric | Count |
|--------|-------|
| API Endpoints | 30+ |
| Backend Services | 7 |
| Middleware | 4 |
| MCP Tools | 6 |
| Vue Components | 14 |
| Vue Views | 5 |
| Pinia Stores | 3 |
| Pydantic Models | 15+ |
