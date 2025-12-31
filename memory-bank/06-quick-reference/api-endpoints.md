# API Endpoints Quick Reference

> **Purpose:** Concise reference for all Seedream API endpoints
> **Created:** 2025-12-31
> **Last Updated:** 2025-12-31
> **Status:** Active

## Overview

Quick reference for all 14 API endpoints in Seedream backend.

## Base URL

```
Development: http://localhost:8080
API Prefix: /api/
API Docs: /docs
MCP Endpoint: /sse (SSE transport)
OAuth Endpoints: /auth/*
Health Check: /health
```

## Notes API

### List All Notes

```
GET /api/notes
```

**Response:** 200 OK
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

### Get Note

```
GET /api/notes/{note_id}
```

**Response:** 200 OK
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
- 404: Note not found

---

### Create Note

```
POST /api/notes
Content-Type: application/json
```

**Request:**
```json
{
  "title": "My New Note",
  "content": "# My New Note\n\nContent goes here...",
  "tags": ["example"],
  "status": "draft"
}
```

**Response:** 201 Created
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
- 409: Note already exists

---

### Update Note

```
PUT /api/notes/{note_id}
Content-Type: application/json
```

**Request:**
```json
{
  "title": "Updated Title",
  "content": "Updated content...",
  "tags": ["updated"],
  "status": "canonical"
}
```

**Response:** 200 OK
```json
{
  "id": "My_New_Note",
  "title": "Updated Title",
  "content": "Updated content...",
  "frontmatter": {...},
  "outgoing_links": [],
  "backlinks": []
}
```

**Errors:**
- 404: Note not found

---

### Delete Note

```
DELETE /api/notes/{note_id}
```

**Response:** 204 No Content

**Errors:**
- 404: Note not found

---

### Reindex Notes

```
POST /api/notes/reindex
```

**Response:** 200 OK
```json
{
  "indexed": 42,
  "message": "Reindexed 42 notes"
}
```

## Search API

### Search Notes

```
GET /api/search?q={query}&limit={limit}&semantic={boolean}
```

**Parameters:**
| Parameter | Type | Default | Description |
|-----------|------|----------|-------------|
| `q` | string | required | Search query |
| `limit` | integer | 10 | Max results (1-50) |
| `semantic` | boolean | true | Use vector search |

**Response:** 200 OK
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

**Score Interpretation:**
- 1.0: Exact lexical match
- 0.8+: Very similar semantically
- 0.5-0.8: Related content
- <0.5: Weak match

---

### Find Similar Notes

```
GET /api/search/similar/{note_id}?limit={limit}
```

**Parameters:**
| Parameter | Type | Default | Description |
|-----------|------|----------|-------------|
| `limit` | integer | 5 | Max results (1-20) |

**Response:** 200 OK
```json
[
  {
    "note_id": "Related_Note",
    "title": "Related Note",
    "snippet": "...related content...",
    "score": 0.75,
    "tags": ["related"]
  }
]
```

## Graph API

### Get Backlinks

```
GET /api/graph/backlinks/{note_id}
```

**Response:** 200 OK
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

### Get Outgoing Links

```
GET /api/graph/outgoing/{note_id}
```

**Response:** 200 OK
```json
["Example_Note", "Wikilinks"]
```

---

### Get Neighbors

```
GET /api/graph/neighbors/{note_id}?depth={depth}
```

**Parameters:**
| Parameter | Type | Default | Description |
|-----------|------|----------|-------------|
| `depth` | integer |1 | Traversal depth (1-3) |

**Response:** 200 OK
```json
{
  "Welcome": ["Example_Note", "Wikilinks"],
  "Example_Note": ["Welcome"]
}
```

---

### Find Unlinked Mentions

```
GET /api/graph/unlinked-mentions/{note_id}
```

**Response:** 200 OK
```json
[
  {
    "note_id": "Some_Other_Note",
    "title": "Some Other Note"
  }
]
```

---

### Rebuild Graph

```
POST /api/graph/rebuild
```

**Response:** 200 OK
```json
{
  "processed": 42,
  "message": "Rebuilt graph with 42 notes"
}
```

## System Endpoints

### Health Check

```
GET /health
```

**Response:** 200 OK
```json
{
  "status": "healthy",
  "service": "seedream"
}
```

---

### Root Endpoint

```
GET /
```

**Response:** 200 OK
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

## Error Responses

All endpoints may return these error formats:

```json
{
  "detail": "Error message describing issue"
}
```

| Status Code | Meaning |
|-------------|---------|
| 400 | Bad Request - Invalid parameters |
| 404 | Not Found - Resource doesn't exist |
| 409 | Conflict - Resource already exists |
| 500 | Internal Server Error |

## MCP Tools

The backend exposes these 6 MCP tools at `/sse` (SSE transport):

| Tool | Arguments | Purpose |
|------|-----------|---------|
| `query_knowledge` | `query`, `limit` | Semantic search |
| `get_note` | `note_id` | Retrieve note content |
| `list_notes` | `tag`, `status` | List notes with filtering |
| `get_backlinks` | `note_id` | Get note connections |
| `ingest_chat` | `content`, `title`, `source`, `tags` | Store conversations |
| `create_draft` | `title`, `content`, `based_on`, `tags` | Create draft notes |

**Authentication:**
- Claude Desktop: No authentication required (local development)
- ChatGPT: OAuth 2.0 token required (Authorization header)

## OAuth Endpoints

### GitHub OAuth Authorization

```
GET /auth/github
```

Redirects user to GitHub for authorization.

**Response:** 302 Redirect to GitHub

---

### OAuth Callback

```
GET /auth/callback?code={authorization_code}
```

Handles GitHub callback and exchanges authorization code for access token.

**Parameters:**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `code` | string | Yes | Authorization code from GitHub |

**Response:** 200 OK
```json
{
  "access_token": "token_123"
}
```

**Authentication:**
- ChatGPT includes token in `Authorization: Bearer {token}` header
- Backend validates token before allowing MCP access
- Tokens are stored in memory (development) or database (production)

## Quick Reference Table

| Endpoint | Method | Purpose | Auth |
|----------|--------|---------|-------|
| `/api/notes` | GET | List all notes | No |
| `/api/notes/{id}` | GET | Get note | No |
| `/api/notes` | POST | Create note | No |
| `/api/notes/{id}` | PUT | Update note | No |
| `/api/notes/{id}` | DELETE | Delete note | No |
| `/api/notes/reindex` | POST | Reindex search | No |
| `/api/search` | GET | Search notes | No |
| `/api/search/similar/{id}` | GET | Find similar | No |
| `/api/graph/backlinks/{id}` | GET | Get backlinks | No |
| `/api/graph/outgoing/{id}` | GET | Get outgoing | No |
| `/api/graph/neighbors/{id}` | GET | Get neighbors | No |
| `/api/graph/unlinked-mentions/{id}` | GET | Find mentions | No |
| `/api/graph/rebuild` | POST | Rebuild graph | No |
| `/health` | GET | Health check | No |
| `/` | GET | API info | No |
| `/sse` | SSE | MCP server (Claude + ChatGPT) | Optional (OAuth for ChatGPT) |
| `/auth/github` | GET | OAuth authorization (ChatGPT) | No |
| `/auth/callback` | GET | OAuth callback (ChatGPT) | No |

## cURL Examples

### Notes

```bash
# List notes
curl http://localhost:8080/api/notes

# Get note
curl http://localhost:8080/api/notes/Welcome

# Create note
curl -X POST http://localhost:8080/api/notes \
  -H "Content-Type: application/json" \
  -d '{"title":"Test","content":"Test content","tags":["test"]}'

# Update note
curl -X PUT http://localhost:8080/api/notes/Welcome \
  -H "Content-Type: application/json" \
  -d '{"content":"Updated content"}'

# Delete note
curl -X DELETE http://localhost:8080/api/notes/Welcome

# Reindex
curl -X POST http://localhost:8080/api/notes/reindex
```

### Search

```bash
# Semantic search
curl "http://localhost:8080/api/search?q=REST%20API&limit=10&semantic=true"

# Lexical search
curl "http://localhost:8080/api/search?q=REST%20API&semantic=false"

# Find similar
curl "http://localhost:8080/api/search/similar/Welcome?limit=5"
```

### Graph

```bash
# Get backlinks
curl http://localhost:8080/api/graph/backlinks/Welcome

# Get outgoing
curl http://localhost:8080/api/graph/outgoing/Welcome

# Get neighbors
curl "http://localhost:8080/api/graph/neighbors/Welcome?depth=2"

# Find unlinked mentions
curl http://localhost:8080/api/graph/unlinked-mentions/Welcome

# Rebuild graph
curl -X POST http://localhost:8080/api/graph/rebuild
```

### System

```bash
# Health check
curl http://localhost:8080/health

# API info
curl http://localhost:8080/
```

### MCP (SSE)

```bash
# Connect to MCP server (Claude Desktop - no auth)
curl -N http://localhost:8080/sse

# Connect with OAuth token (ChatGPT)
curl -N http://localhost:8080/sse \
  -H "Authorization: Bearer token_123"
```

### OAuth

```bash
# Initiate OAuth flow
curl http://localhost:8080/auth/github

# Handle callback (automated by OAuth provider)
curl "http://localhost:8080/auth/callback?code=abc123"
```

## Related Documentation

- [API Contracts - Backend](../../docs/api-contracts-backend.md)
- [Data Models](./data-models.md)
- [Services](./services.md)
- [Development Guide - Backend](../../docs/development-guide-backend.md)
- [ADR-003: MCP Integration](../02-architecture-decisions/adr-003-mcp-integration.md)
- [ADR-006: OAuth Authentication](../02-architecture-decisions/adr-006-oauth-authentication.md)

---

**See Also:**
- [Architecture - Backend](../../docs/architecture-backend.md)
- [Integration Architecture](../../docs/integration-architecture.md)
- [MCP Integration](../../docs/chat-ingestion-guide.md)
- [Frontend API Client](../../docs/architecture-frontend.md#api-client)
