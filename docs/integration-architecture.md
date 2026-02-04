# Grafyn Integration Architecture

> **Type:** Multi-Part Monorepo | **Parts:** Backend + Frontend | **Scan Level:** Exhaustive

## Overview

Grafyn consists of two main parts that communicate via REST API:

```
┌─────────────────────────────────────────────────────────────────────┐
│                           Grafyn Platform                             │
│                                                                      │
│  ┌──────────────────────┐         ┌──────────────────────────────┐  │
│  │     Frontend         │  HTTP   │          Backend              │  │
│  │     (Vue 3)          │◄───────►│         (FastAPI)             │  │
│  │                      │  :5173  │                               │  │
│  │  ┌────────────────┐  │   ↓     │  ┌─────────────────────────┐  │  │
│  │  │   App.vue      │  │ proxy   │  │      API Routers        │  │  │
│  │  │   SearchBar    │  │   ↓     │  │  /api/notes             │  │  │
│  │  │   NoteEditor   │──┼────────►│  │  /api/search            │  │  │
│  │  │   NoteList     │  │         │  │  /api/graph             │  │  │
│  │  │   Backlinks    │  │         │  └─────────────────────────┘  │  │
│  │  └────────────────┘  │         │             │                 │  │
│  └──────────────────────┘         │             ▼                 │  │
│                                   │  ┌─────────────────────────┐  │  │
│  ┌──────────────────────┐         │  │      Services           │  │  │
│  │   External AI        │  MCP    │  │  KnowledgeStore         │  │  │
│  │   (Claude/ChatGPT)   │◄───────►│  │  VectorSearch           │  │  │
│  │                      │  :8080  │  │  GraphIndex             │  │  │
│  └──────────────────────┘  /sse   │  │  Embedding              │  │  │
│                                   │  └─────────────────────────┘  │  │
│                                   │             │                 │  │
│                                   │             ▼                 │  │
│                                   │  ┌─────────────────────────┐  │  │
│                                   │  │      Storage            │  │  │
│                                   │  │  vault/ (Markdown)      │  │  │
│                                   │  │  data/  (LanceDB)       │  │  │
│                                   │  └─────────────────────────┘  │  │
│                                   └──────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Communication Patterns

### Frontend → Backend (REST API)

**Protocol:** HTTP/JSON over REST  
**Development Proxy:** Vite dev server proxies `/api/*` to `localhost:8080`

| Frontend Action | API Endpoint | Response |
|-----------------|--------------|----------|
| Load notes list | `GET /api/notes` | `NoteListItem[]` |
| Open note | `GET /api/notes/{id}` | `Note` |
| Create note | `POST /api/notes` | `Note` |
| Save note | `PUT /api/notes/{id}` | `Note` |
| Delete note | `DELETE /api/notes/{id}` | `204` |
| Search | `GET /api/search?q=...` | `SearchResult[]` |
| Get backlinks | `GET /api/graph/backlinks/{id}` | `BacklinkInfo[]` |
| Reindex | `POST /api/notes/reindex` | `{indexed: n}` |

**API Client Pattern:**
```javascript
// frontend/src/api/client.js
async function request(endpoint, options = {}) {
    const url = `${API_BASE}${endpoint}`
    const response = await fetch(url, {
        headers: { 'Content-Type': 'application/json' },
        ...options,
        body: options.body ? JSON.stringify(options.body) : undefined,
    })
    if (!response.ok) throw new Error(...)
    return response.json()
}
```

---

### External AI → Backend (MCP)

**Protocol:** Model Context Protocol (SSE)  
**Endpoint:** `GET /sse`  
**Library:** `fastapi-mcp`  
**Authentication:** OAuth 2.0 (ChatGPT), Optional (Claude Desktop)

| MCP Tool | Backend Service | Purpose |
|----------|-----------------|---------|
| `query_knowledge` | VectorSearchService | Semantic search |
| `get_note` | KnowledgeStore | Full content |
| `list_notes` | KnowledgeStore | Filtered listing |
| `get_backlinks` | GraphIndexService | Link context |
| `ingest_chat` | KnowledgeStore + VectorSearch | Store transcripts |
| `create_draft` | KnowledgeStore + VectorSearch | Create drafts |

**MCP Flow:**
```
Claude Desktop → SSE → FastAPI /sse → Auto-discovered tools
                    ↓
            Execute tool (e.g., query_knowledge)
                    ↓
            Return results to AI

ChatGPT → OAuth → GitHub → FastAPI /auth/callback → Token
          ↓
ChatGPT → SSE + Token → FastAPI /sse → Auto-discovered tools
                    ↓
            Execute tool (e.g., query_knowledge)
                    ↓
            Return results to AI
```

**OAuth Flow (ChatGPT Only):**
```
1. ChatGPT → Redirect to /auth/github
2. User → Authorize via GitHub
3. GitHub → Redirect to /auth/callback with code
4. Backend → Exchange code for token
5. Backend → Return token to ChatGPT
6. ChatGPT → Include token in Authorization header
7. Backend → Validate token and allow MCP access
```

---

## Data Flow Diagrams

### Note Creation Flow

```
User (Frontend)
      │
      ▼
┌─────────────┐     POST /api/notes     ┌──────────────────┐
│  NoteEditor │ ───────────────────────►│    notes.py      │
│             │                          │    (router)      │
└─────────────┘                          └────────┬─────────┘
                                                   │
                                ┌──────────────────┼──────────────────┐
                                ▼                  ▼                  ▼
                     ┌──────────────────┐  ┌──────────────┐  ┌──────────────┐
                     │  KnowledgeStore  │  │ VectorSearch │  │  GraphIndex  │
                     │  (create note)   │  │ (index note) │  │  (rebuild)    │
                     └────────┬─────────┘  └──────────────┘  └──────────────┘
                              │
                              ▼
                     ┌──────────────────┐
                     │   vault/Note.md  │
                     │   (filesystem)   │
                     └──────────────────┘
```

### Search Flow

```
User (Frontend)
      │
      ▼
┌─────────────┐     GET /api/search?q=...   ┌──────────────────┐
│  SearchBar  │ ───────────────────────────►│    search.py     │
│  (debounce) │                              │    (router)      │
└─────────────┘                              └────────┬─────────┘
                                                       │
                                                       ▼
                                           ┌────────────────────┐
                                           │  VectorSearchService│
                                           │   search(query)     │
                                           └────────┬────────────┘
                                                    │
                           ┌────────────────────────┼────────────────────────┐
                           ▼                        ▼                        ▼
                  ┌────────────────┐     ┌────────────────┐      ┌────────────────┐
                  │ EmbeddingService│    │    LanceDB     │      │  Format        │
                  │ encode(query)  │     │ .search()      │      │  SearchResult  │
                  └────────────────┘     └────────────────┘      └────────────────┘
```

### Backlinks Flow

```
User (Frontend)
      │
      ▼
┌─────────────────┐   GET /api/graph/backlinks/{id}   ┌──────────────────┐
│  BacklinksPanel │ ─────────────────────────────────►│    graph.py      │
│                 │                                    │    (router)      │
└─────────────────┘                                    └────────┬─────────┘
                                                                 │
                                                                 ▼
                                                    ┌──────────────────────┐
                                                    │  GraphIndexService   │
                                                    │  get_backlinks_      │
                                                    │  with_context(id)    │
                                                    └────────┬─────────────┘
                                                             │
                                   ┌─────────────────────────┼─────────────────────────┐
                                   ▼                         ▼                         ▼
                        ┌──────────────────┐      ┌──────────────────┐      ┌──────────────────┐
                        │  _incoming map   │      │  KnowledgeStore  │      │  _extract_link_  │
                        │  (adjacency)     │      │  get_note()      │      │  context()       │
                        └──────────────────┘      └──────────────────┘      └──────────────────┘
```

---

## Port Configuration

| Service | Port | Purpose |
|---------|------|---------|
| **Backend (FastAPI)** | 8080 | REST API + MCP server |
| **Frontend (Vite dev)** | 5173 | Development server |
| **OpenAPI Docs** | 8080/docs | Swagger UI |
| **MCP Endpoint (SSE)** | 8080/sse | AI model integration |
| **OAuth Endpoints** | 8080/auth/* | OAuth authentication (ChatGPT) |

---

## Environment Configuration

### Backend (`.env`)
```env
VAULT_PATH=../vault
DATA_PATH=../data
SERVER_HOST=0.0.0.0
SERVER_PORT=8080
EMBEDDING_MODEL=all-MiniLM-L6-v2

# OAuth Configuration (for ChatGPT)
GITHUB_CLIENT_ID=your-github-client-id
GITHUB_CLIENT_SECRET=your-github-client-secret
GITHUB_REDIRECT_URI=https://your-name.ngrok.io/auth/callback
```

### Frontend (`vite.config.js`)
```javascript
server: {
    port: 5173,
    proxy: {
        '/api': { target: 'http://localhost:8080', changeOrigin: true },
        '/sse': { target: 'http://localhost:8080', changeOrigin: true },
        '/auth': { target: 'http://localhost:8080', changeOrigin: true },
    },
}
```

---

## Shared Concepts

### Note ID Convention
- **Storage:** Filename without `.md` extension
- **Generation:** `title.replace(" ", "_")`
- **URL Encoding:** Required in API paths (`%20` → `_`)

### Wikilink Format
- **Source:** `[[Note Title]]` or `[[Note Title|Display Text]]`
- **Parsing:** Regex in `KnowledgeStore.extract_wikilinks()`
- **Storage:** Target title only (spaces preserved)
- **Lookup:** Normalized to underscore form

### Status Workflow
```
draft → evidence → canonical
  │         │          │
  │         │          └── Verified, authoritative
  │         └── AI-ingested content (chat transcripts)
  └── Proposed/new content
```

---

## Error Handling

### Frontend
```javascript
try {
    await notesApi.create(data)
} catch (err) {
    alert(`Failed to create note: ${err.message}`)
}
```

### Backend
```python
if note is None:
    raise HTTPException(
        status_code=status.HTTP_404_NOT_FOUND,
        detail=f"Note '{note_id}' not found",
    )
```

### Error Response Format
```json
{
    "detail": "Note 'NonExistent' not found"
}
```

---

## Client Configuration

### Claude Desktop (Local Development)

**File:** `~/.config/Claude/claude_desktop_config.json`

```json
{
  "mcpServers": {
    "grafyn": {
      "url": "http://localhost:8080/sse",
      "transport": "sse"
    }
  }
}
```

### ChatGPT (Production)

ChatGPT requires OAuth authentication and a public HTTPS endpoint:

**Setup Steps:**
1. Expose backend via ngrok: `ngrok http 8080`
2. Register GitHub OAuth app at https://github.com/settings/developers
3. Configure ChatGPT with:
   - **Server Name**: Grafyn Knowledge Base
   - **SSE Endpoint**: `https://your-name.ngrok.io/sse`
   - **OAuth Provider**: GitHub
   - **Client ID**: Your GitHub OAuth app client ID
   - **Client Secret**: Your GitHub OAuth app client secret
   - **Authorization URL**: `https://your-name.ngrok.io/auth/github`
   - **Callback URL**: `https://your-name.ngrok.io/auth/callback`

**Note:** Claude Desktop can connect without authentication (local development), while ChatGPT requires OAuth authentication.
