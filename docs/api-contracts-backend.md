# Seedream API Contracts - Backend

> **Part:** Backend | **Endpoints:** 30+ | **Scan Level:** Exhaustive

## Base URL

- **Development:** `http://localhost:8080`
- **API Prefix:** `/api/`
- **OpenAPI Docs:** `/docs`
- **MCP Endpoint:** `/sse`

---

## Notes API (`/api/notes`)

### GET /api/notes
List all notes in the vault.

**Response:** `200 OK`
```json
[
  {
    "id": "Welcome",
    "title": "Welcome",
    "status": "draft",
    "tags": ["welcome", "getting-started"],
    "created": "2024-12-17T00:00:00",
    "modified": "2024-12-17T00:00:00",
    "link_count": 2
  }
]
```

---

### GET /api/notes/{note_id}
Get a specific note by ID.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `note_id` | string | Note filename without `.md` extension |

**Response:** `200 OK`
```json
{
  "id": "Welcome",
  "title": "Welcome",
  "content": "# Welcome to Seedream\n\nThis is your knowledge base...",
  "frontmatter": {
    "title": "Welcome",
    "created": "2024-12-17T00:00:00",
    "modified": "2024-12-17T00:00:00",
    "tags": ["welcome", "getting-started"],
    "status": "draft",
    "aliases": []
  },
  "outgoing_links": ["Example Note", "Wikilinks"],
  "backlinks": ["Example Note"]
}
```

**Errors:**
- `404 Not Found`: Note doesn't exist

---

### POST /api/notes
Create a new note.

**Request Body:**
```json
{
  "title": "My New Note",
  "content": "# My New Note\n\nContent goes here...",
  "tags": ["example"],
  "status": "draft"
}
```

**Response:** `201 Created`
```json
{
  "id": "My_New_Note",
  "title": "My New Note",
  "content": "# My New Note\n\nContent goes here...",
  "frontmatter": {...},
  "outgoing_links": [],
  "backlinks": []
}
```

**Errors:**
- `409 Conflict`: Note with this title already exists

---

### PUT /api/notes/{note_id}
Update an existing note.

**Request Body:**
```json
{
  "title": "Updated Title",
  "content": "Updated content...",
  "tags": ["updated"],
  "status": "canonical"
}
```

**Response:** `200 OK` - Updated note object

**Errors:**
- `404 Not Found`: Note doesn't exist

---

### DELETE /api/notes/{note_id}
Delete a note.

**Response:** `204 No Content`

**Errors:**
- `404 Not Found`: Note doesn't exist

---

### POST /api/notes/reindex
Reindex all notes for search.

**Response:** `200 OK`
```json
{
  "indexed": 42,
  "message": "Reindexed 42 notes"
}
```

---

## Search API (`/api/search`)

### GET /api/search
Search notes using semantic or lexical matching.

**Query Parameters:**
| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `q` | string | **required** | Search query |
| `limit` | integer | 10 | Max results (1-50) |
| `semantic` | boolean | true | Use vector search |

**Response:** `200 OK`
```json
[
  {
    "note_id": "Welcome",
    "title": "Welcome",
    "snippet": "...your knowledge base. Create notes, link them together...",
    "score": 0.87,
    "tags": ["welcome", "getting-started"]
  }
]
```

---

### GET /api/search/similar/{note_id}
Find notes similar to a given note.

**Query Parameters:**
| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `limit` | integer | 5 | Max results (1-20) |

**Response:** `200 OK` - Array of SearchResult objects (excludes source note)

---

## Graph API (`/api/graph`)

### GET /api/graph/backlinks/{note_id}
Get all notes that link to the given note, with context.

**Response:** `200 OK`
```json
[
  {
    "source_id": "Example_Note",
    "source_title": "Example Note",
    "context": "...link to other notes using wikilinks like [[Welcome]]. The system will..."
  }
]
```

---

### GET /api/graph/outgoing/{note_id}
Get all notes that the given note links to.

**Response:** `200 OK`
```json
["Example_Note", "Wikilinks"]
```

---

### GET /api/graph/neighbors/{note_id}
Get neighboring notes up to a certain depth for visualization.

**Query Parameters:**
| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `depth` | integer | 1 | Traversal depth (1-3) |

**Response:** `200 OK`
```json
{
  "Welcome": ["Example_Note", "Wikilinks"],
  "Example_Note": ["Welcome"]
}
```

**Errors:**
- `404 Not Found`: Note doesn't exist

---

### GET /api/graph/unlinked-mentions/{note_id}
Find notes that mention this note's title but don't link to it.

**Response:** `200 OK`
```json
[
  {
    "note_id": "Some_Other_Note",
    "title": "Some Other Note"
  }
]
```

---

### POST /api/graph/rebuild
Rebuild the entire link graph index.

**Response:** `200 OK`
```json
{
  "processed": 42,
  "message": "Rebuilt graph with 42 notes"
}
```

---

## OAuth API (`/api/oauth`)

### GET /api/oauth/github
Initiate GitHub OAuth flow.

**Response:** Redirect to GitHub authorization page

---

### GET /api/oauth/callback
Handle OAuth callback from GitHub.

**Query Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `code` | string | Authorization code from GitHub |
| `state` | string | CSRF protection state |

**Response:** Redirect with access token

---

## Canvas API (`/api/canvas`)

### GET /api/canvas
List all canvas sessions.

**Response:** `200 OK`
```json
[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "title": "My Canvas Session",
    "description": "Exploring AI model comparisons",
    "tile_count": 3,
    "debate_count": 1,
    "created_at": "2026-01-11T14:00:00Z",
    "updated_at": "2026-01-11T14:05:00Z",
    "tags": ["ai-comparison"],
    "status": "draft"
  }
]
```

---

### POST /api/canvas
Create a new canvas session.

**Request Body:**
```json
{
  "title": "My Canvas Session",
  "description": "Exploring AI model comparisons",
  "tags": ["ai-comparison"]
}
```

**Response:** `201 Created`
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "title": "My Canvas Session",
  "description": "Exploring AI model comparisons",
  "prompt_tiles": [],
  "debates": [],
  "viewport": {"x": 0, "y": 0, "zoom": 1},
  "created_at": "2026-01-11T14:00:00Z",
  "updated_at": "2026-01-11T14:00:00Z",
  "tags": ["ai-comparison"],
  "status": "draft"
}
```

---

### GET /api/canvas/{session_id}
Get a specific canvas session.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |

**Response:** `200 OK` - Full CanvasSession object

**Errors:**
- `404 Not Found`: Session doesn't exist

---

### PUT /api/canvas/{session_id}
Update canvas session metadata.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |

**Request Body:**
```json
{
  "title": "Updated Title",
  "description": "Updated description",
  "viewport": {"x": 100, "y": 50, "zoom": 1.2},
  "tags": ["new-tag"],
  "status": "evidence"
}
```

**Response:** `200 OK` - Updated CanvasSession object

**Errors:**
- `404 Not Found`: Session doesn't exist

---

### DELETE /api/canvas/{session_id}
Delete a canvas session.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |

**Response:** `204 No Content`

**Errors:**
- `404 Not Found`: Session doesn't exist

---

### GET /api/canvas/models/available
Get list of available models from OpenRouter. Rate limited to 30/minute.

**Response:** `200 OK`
```json
[
  {
    "id": "openai/gpt-4",
    "name": "GPT-4",
    "provider": "openai",
    "context_length": 8192,
    "pricing": {"prompt": 0.03, "completion": 0.06},
    "supports_streaming": true
  },
  {
    "id": "anthropic/claude-3-opus",
    "name": "Claude 3 Opus",
    "provider": "anthropic",
    "context_length": 200000,
    "pricing": {"prompt": 0.015, "completion": 0.075},
    "supports_streaming": true
  }
]
```

**Errors:**
- `503 Service Unavailable`: OpenRouter API key not configured

---

### PUT /api/canvas/{session_id}/viewport
Update canvas viewport state.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |

**Request Body:**
```json
{
  "x": 100,
  "y": 50,
  "zoom": 1.2
}
```

**Response:** `200 OK`
```json
{
  "status": "updated"
}
```

**Errors:**
- `404 Not Found`: Session doesn't exist

---

### PUT /api/canvas/{session_id}/tiles/{tile_id}/position
Update a tile's position on the canvas.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |
| `tile_id` | string | Tile UUID |

**Request Body:**
```json
{
  "x": 300,
  "y": 100,
  "width": 280,
  "height": 200
}
```

**Response:** `200 OK`
```json
{
  "status": "updated"
}
```

**Errors:**
- `404 Not Found`: Tile doesn't exist

---

### PUT /api/canvas/{session_id}/tiles/{tile_id}/responses/{model_id}/position
Update an individual LLM response node's position on the canvas.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |
| `tile_id` | string | Tile UUID |
| `model_id` | string | Model ID (URL encoded, may contain `/`) |

**Request Body:**
```json
{
  "x": 600,
  "y": 150,
  "width": 280,
  "height": 200
}
```

**Response:** `200 OK`
```json
{
  "status": "updated"
}
```

**Errors:**
- `404 Not Found`: LLM node doesn't exist

---

### DELETE /api/canvas/{session_id}/tiles/{tile_id}
Delete a tile (prompt or debate) from the canvas.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |
| `tile_id` | string | Tile UUID |

**Response:** `204 No Content`

**Errors:**
- `404 Not Found`: Tile doesn't exist

---

### GET /api/canvas/{session_id}/edges
Get all parent-child tile edges for mind-map visualization (legacy).

**Response:** `200 OK`
```json
[
  {
    "source_tile_id": "tile-1",
    "target_tile_id": "tile-2",
    "source_model_id": "openai/gpt-4"
  }
]
```

**Errors:**
- `404 Not Found`: Session doesn't exist

---

### GET /api/canvas/{session_id}/node-edges
Get all edges in the canvas graph for node-graph visualization.

**Response:** `200 OK`
```json
[
  {
    "source_id": "prompt:tile-1",
    "target_id": "llm:tile-1:openai/gpt-4",
    "edge_type": "prompt_to_llm",
    "color": "#7c5cff"
  },
  {
    "source_id": "llm:tile-1:openai/gpt-4",
    "target_id": "prompt:tile-2",
    "edge_type": "llm_to_prompt"
  }
]
```

**Errors:**
- `404 Not Found`: Session doesn't exist

---

### POST /api/canvas/{session_id}/arrange
Batch update node positions after auto-arrange.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |

**Request Body:**
```json
{
  "positions": {
    "prompt:tile-1": {"x": 50, "y": 50, "width": 200, "height": 120},
    "llm:tile-1:openai/gpt-4": {"x": 300, "y": 50, "width": 280, "height": 200},
    "debate:debate-1": {"x": 700, "y": 50, "width": 600, "height": 400}
  }
}
```

**Response:** `200 OK`
```json
{
  "status": "arranged",
  "node_count": 3
}
```

**Errors:**
- `400 Bad Request`: Failed to update positions
- `404 Not Found`: Session doesn't exist

---

### GET /api/canvas/{session_id}/node-groups
Get isolated node groups for multi-note export.

**Response:** `200 OK`
```json
{
  "groups": [
    ["prompt:tile-1", "llm:tile-1:openai/gpt-4", "llm:tile-1:claude-3-opus"],
    ["prompt:tile-2", "llm:tile-2:gpt-4"]
  ],
  "count": 2
}
```

**Errors:**
- `404 Not Found`: Session doesn't exist

---

### POST /api/canvas/{session_id}/prompt
Send a prompt to multiple models with SSE streaming. Rate limited to 20/minute.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |

**Request Body:**
```json
{
  "prompt": "What is the meaning of life?",
  "system_prompt": "You are a helpful assistant.",
  "models": ["openai/gpt-4", "anthropic/claude-3-opus"],
  "temperature": 0.7,
  "max_tokens": 2048,
  "parent_tile_id": null,
  "parent_model_id": null,
  "context_mode": "full_history"
}
```

**Response:** `200 OK` - Server-Sent Events (SSE) stream

**SSE Events:**
| Event Type | Payload | Description |
|-----------|----------|-------------|
| `tile_created` | `{tile_id}` | New prompt tile created |
| `chunk` | `{model_id, chunk}` | Streaming text chunk |
| `complete` | `{model_id}` | Model finished streaming |
| `error` | `{model_id, error}` | Model error occurred |
| `session_saved` | - | Session persisted to disk |
| `[DONE]` | - | Stream complete |

**Errors:**
- `404 Not Found`: Session doesn't exist
- `503 Service Unavailable`: OpenRouter API key not configured

---

### POST /api/canvas/{session_id}/debate
Start a debate between models with SSE streaming. Rate limited to 10/minute.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |

**Request Body:**
```json
{
  "source_tile_ids": ["tile-1", "tile-2"],
  "participating_models": ["openai/gpt-4", "anthropic/claude-3-opus"],
  "debate_mode": "auto",
  "debate_prompt": null,
  "max_rounds": 3
}
```

**Response:** `200 OK` - Server-Sent Events (SSE) stream

**SSE Events:**
| Event Type | Payload | Description |
|-----------|----------|-------------|
| `debate_created` | `{debate_id}` | New debate started |
| `round_start` | `{round}` | New round started |
| `debate_chunk` | `{round, model_id, chunk}` | Streaming debate content |
| `model_complete` | `{round, model_id}` | Model finished round |
| `debate_error` | `{round, model_id, error}` | Model error |
| `round_complete` | `{round}` | Round finished |
| `debate_complete` | `{debate_id}` | Debate finished |
| `[DONE]` | - | Stream complete |

**Errors:**
- `404 Not Found`: Session doesn't exist
- `400 Bad Request`: No valid source tiles found
- `503 Service Unavailable`: OpenRouter API key not configured

---

### POST /api/canvas/{session_id}/debate/{debate_id}/continue
Continue a debate with a custom prompt (user-mediated mode). Rate limited to 10/minute.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |
| `debate_id` | string | Debate UUID |

**Request Body:**
```json
{
  "prompt": "Please elaborate on your previous points."
}
```

**Response:** `200 OK` - Server-Sent Events (SSE) stream

**SSE Events:**
| Event Type | Payload | Description |
|-----------|----------|-------------|
| `round_start` | `{round}` | New round started |
| `debate_chunk` | `{round, model_id, chunk}` | Streaming debate content |
| `model_complete` | `{round, model_id}` | Model finished round |
| `debate_error` | `{round, model_id, error}` | Model error |
| `round_complete` | `{round}` | Round finished |
| `[DONE]` | - | Stream complete |

**Errors:**
- `404 Not Found`: Session or debate doesn't exist

---

### PUT /api/canvas/{session_id}/debate/{debate_id}/status
Update debate status (pause/resume/complete).

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |
| `debate_id` | string | Debate UUID |

**Query Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `status` | string | New status: `active`, `paused`, or `completed` |

**Response:** `200 OK`
```json
{
  "status": "updated"
}
```

**Errors:**
- `400 Bad Request`: Invalid status value
- `404 Not Found`: Debate doesn't exist

---

### POST /api/canvas/{session_id}/export-note
Export canvas session as a markdown note.

**Path Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `session_id` | string | Canvas session UUID |

**Response:** `200 OK`
```json
{
  "note_id": "Canvas_My_Canvas_Session",
  "title": "Canvas: My Canvas Session",
  "message": "Canvas exported to note: Canvas: My Canvas Session",
  "updated": false
}
```

**Errors:**
- `404 Not Found`: Session doesn't exist
- `500 Internal Server Error`: Failed to export

---

## System Endpoints

### GET /health
Health check endpoint. Rate limited to 30/minute.

**Response:** `200 OK`
```json
{
  "status": "healthy",
  "service": "seedream",
  "environment": "development"
}
```

---

### GET /
Root endpoint with API info.

**Response:** `200 OK`
```json
{
  "name": "Seedream Knowledge Graph",
  "version": "0.1.0",
  "vault_path": "/path/to/vault",
  "docs": "/docs",
  "mcp": "/sse",
  "oauth": "/auth"
}
```

---

## Error Responses

All endpoints may return these error formats:

**4xx/5xx Error:**
```json
{
  "detail": "Error message describing the issue"
}
```

| Status Code | Meaning |
|-------------|---------|
| 400 | Bad Request - Invalid parameters |
| 401 | Unauthorized - Authentication required |
| 404 | Not Found - Resource doesn't exist |
| 409 | Conflict - Resource already exists |
| 429 | Too Many Requests - Rate limit exceeded |
| 500 | Internal Server Error |

---

## Rate Limiting

Rate limiting is enforced via `slowapi`:

**Default Limits:**
- 10 requests per minute
- 50 requests per hour
- 200 requests per day

**Rate Limit Headers:**
```
X-RateLimit-Limit: 10
X-RateLimit-Remaining: 9
X-RateLimit-Reset: 1704067200
```

**Rate Limit Exceeded Response:**
```json
{
  "detail": "Rate limit exceeded. Please try again later."
}
```

---

## CORS Configuration

```python
# Development
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Production
app.add_middleware(
    CORSMiddleware,
    allow_origins=settings.cors_origins,  # Specific origins only
    allow_credentials=False,
    allow_methods=["GET", "POST", "PUT", "DELETE"],
    allow_headers=["Content-Type", "Authorization"],
    max_age=3600
)
```

**Note:** CORS configuration differs between development and production environments.
