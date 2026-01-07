# Seedream Knowledge Graph Platform - Project Overview

## Executive Summary

Seedream is a **local knowledge graph platform** that enables semantic search, Obsidian-style linking, and MCP (Model Context Protocol) integration. It provides a self-hosted knowledge base with AI-ready capabilities for personal or organizational use.

## Key Features

| Feature | Description |
|---------|-------------|
| **Obsidian-compatible notes** | Markdown files with YAML frontmatter and `[[wikilinks]]` |
| **Semantic search** | Vector-based search using sentence-transformers (all-MiniLM-L6-v2) |
| **Backlinks** | Automatic bidirectional link tracking and discovery |
| **MCP Server** | Connects external AI models (Claude, ChatGPT, Gemini) to knowledge base |
| **OAuth Authentication** | GitHub OAuth for secure ChatGPT integration |
| **Web UI** | Vue 3 SPA with Pinia state management |

## Technology Stack Summary

| Layer | Technology | Version | Purpose |
|-------|------------|---------|---------|
| **Backend Framework** | FastAPI | 0.128+ | REST API server |
| **Vector Database** | LanceDB | 0.26+ | Embedding storage and similarity search |
| **Embeddings** | sentence-transformers | 5.2+ | Text to vector encoding (384 dimensions) |
| **MCP Integration** | fastapi-mcp | Latest | AI model protocol bridge |
| **Data Validation** | Pydantic | 2.12+ | Request/response schemas |
| **Rate Limiting** | slowapi | Latest | API protection |
| **Frontend Framework** | Vue 3 | 3.4+ | Reactive UI components |
| **State Management** | Pinia | Latest | Centralized state |
| **Routing** | Vue Router | 4.2+ | SPA navigation |
| **Build Tool** | Vite | 5.0+ | Fast development server |
| **Markdown** | marked | 11.0+ | Content rendering |

## Architecture Type

**Multi-Part Monorepo** with clear separation:

```
seedream/
├── backend/     → FastAPI Python service (data, search, MCP)
│   └── app/
│       ├── main.py           # Application entry point
│       ├── config.py         # Settings from .env
│       ├── routers/          # API endpoints (notes, search, graph, oauth)
│       ├── services/         # Business logic (5 services)
│       ├── models/           # Pydantic schemas
│       ├── middleware/       # Security, logging, rate limiting
│       └── mcp/              # MCP server integration
├── frontend/    → Vue 3 SPA (user interface)
│   └── src/
│       ├── main.js           # Vue app bootstrap
│       ├── App.vue           # Root component
│       ├── router/           # Vue Router configuration
│       ├── stores/           # Pinia stores (auth, notes)
│       ├── views/            # Page components
│       ├── components/       # UI components (5 total)
│       ├── api/              # Backend client
│       └── style.css         # Design system
├── vault/       → Markdown notes storage (Obsidian-compatible)
└── data/        → LanceDB vector storage
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

### Knowledge Graph
- Wikilinks create directed edges between notes
- Backlinks computed on-demand from outgoing links
- Neighbor traversal up to 3 hops for visualization
- Unlinked mention detection for link suggestions

### Authentication
- OAuth 2.0 with GitHub for ChatGPT integration
- Token-based session management
- Environment-based CORS configuration (stricter in production)

## Statistics

| Metric | Count |
|--------|-------|
| API Endpoints | 15+ |
| Backend Services | 5 |
| Middleware | 4 |
| MCP Tools | 6 |
| Vue Components | 5 |
| Vue Views | 4 |
| Pinia Stores | 2 |
| Pydantic Models | 8 |
