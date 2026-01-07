# Seedream API Contracts - Backend

> **Part:** Backend | **Endpoints:** 15+ | **Scan Level:** Exhaustive

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
