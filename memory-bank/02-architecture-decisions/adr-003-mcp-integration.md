# ADR-003: MCP Integration for AI Assistants

## Status
Accepted

## Date
2024-12-10
## Last Updated
2025-12-31

## Context

Grafyn aims to integrate with external AI assistants (Claude, ChatGPT, Gemini) to:

1. **Enable AI to query organizational knowledge**: AI assistants can search and retrieve information
2. **Allow AI to store conversations**: Save chat transcripts as evidence notes
3. **Support AI-generated drafts**: Create draft notes from AI analysis
4. **Provide bidirectional integration**: AI can both read and write to knowledge base

We need a standardized protocol for AI assistants to interact with Grafyn that is:

- **Standardized**: Works with multiple AI providers
- **Extensible**: Easy to add new tools and capabilities
- **Secure**: Proper authentication and authorization
- **Well-documented**: Clear API for AI model developers

## Decision

We adopted the **Model Context Protocol (MCP)** for AI integration using the `fastapi-mcp` library.

### MCP Integration Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    External AI Assistants                        │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐                 │
│  │  Claude  │  │ ChatGPT  │  │  Gemini  │                 │
│  │ Desktop  │  │   Web    │  │          │                 │
│  └─────┬────┘  └─────┬────┘  └─────┬────┘                 │
└────────┼───────────────┼───────────────┼─────────────────────┘
         │               │               │
         │               └───────┬───────┘
         │                       │
         │               ┌───────▼───────┐
         │               │   OAuth 2.0   │
         │               │  (ChatGPT)    │
         │               └───────┬───────┘
         │                       │
         └───────────────┬───────┴───────┐
                         │               │
         ┌───────────────┴───────────────┐
         │      MCP Protocol (SSE)        │
         └───────────────┬───────────────┘
                         │
         ┌───────────────┴───────────────┐
         │    Grafyn Backend /sse         │
         │  ┌─────────────────────────┐   │
         │  │   fastapi-mcp         │   │
         │  │   (MCP Server)        │   │
         │  │  + OAuth Middleware   │   │
         │  └───────────┬───────────┘   │
         └──────────────┼───────────────┘
                        │
         ┌──────────────┼───────────────┐
         │              │               │
    ┌────▼────┐  ┌────▼────┐  ┌────▼────┐
    │  Notes  │  │ Search  │  │  Graph  │
    │   API   │  │   API   │  │   API   │
    └─────────┘  └─────────┘  └─────────┘
```

### MCP Tools Exposed

We expose 6 MCP tools that map to backend API endpoints:

| MCP Tool | Backend Endpoint | Purpose |
|----------|------------------|---------|
| `query_knowledge` | `GET /api/search` | Semantic search knowledge base |
| `get_note` | `GET /api/notes/{id}` | Retrieve full note content |
| `list_notes` | `GET /api/notes` | List notes with filtering |
| `get_backlinks` | `GET /api/graph/backlinks/{id}` | Get notes linking to a note |
| `ingest_chat` | `POST /api/notes` | Store chat transcripts as evidence |
| `create_draft` | `POST /api/notes` | Create draft notes for review |

### Implementation

**File:** `backend/app/mcp/server.py`

```python
from fastapi import FastAPI, Depends, Header, HTTPException
from fastapi_mcp import FastApiMCP

# OAuth verification (optional for Claude Desktop)
async def verify_oauth(authorization: str = Header(None)):
    """Verify OAuth token for ChatGPT, allow Claude Desktop without auth"""
    if authorization is None:
        # Allow Claude Desktop without auth for local development
        return True
    
    token = authorization.replace("Bearer ", "")
    if not is_valid_token(token):
        raise HTTPException(status_code=401, detail="Invalid OAuth token")
    return True

def setup_mcp(app: FastAPI) -> None:
    mcp = FastApiMCP(
        app,
        name="Grafyn Knowledge Graph",
        description="Access and query an organizational knowledge base",
        transport="sse"
    )
    # Single SSE endpoint for both Claude and ChatGPT
    mcp.mount(path="/sse", dependencies=[Depends(verify_oauth)])
```

**File:** `backend/app/mcp/oauth.py`

```python
from fastapi import FastAPI, Request, HTTPException
from fastapi.responses import RedirectResponse
import httpx

GITHUB_CLIENT_ID = "your-github-client-id"
GITHUB_CLIENT_SECRET = "your-github-client-secret"
GITHUB_REDIRECT_URI = "https://your-name.ngrok.io/auth/callback"

# Store tokens in memory (for development)
# In production, use a database
access_tokens = {}

def setup_oauth_routes(app: FastAPI):
    
    @app.get("/auth/github")
    async def github_auth():
        """Redirect user to GitHub for authorization"""
        auth_url = (
            f"https://github.com/login/oauth/authorize?"
            f"client_id={GITHUB_CLIENT_ID}&"
            f"redirect_uri={GITHUB_REDIRECT_URI}&"
            f"scope=read:user"
        )
        return RedirectResponse(auth_url)
    
    @app.get("/auth/callback")
    async def github_callback(code: str):
        """Handle GitHub callback and exchange code for token"""
        async with httpx.AsyncClient() as client:
            response = await client.post(
                "https://github.com/login/oauth/access_token",
                data={
                    "client_id": GITHUB_CLIENT_ID,
                    "client_secret": GITHUB_CLIENT_SECRET,
                    "code": code,
                    "redirect_uri": GITHUB_REDIRECT_URI,
                },
                headers={"Accept": "application/json"}
            )
            token_data = response.json()
            access_token = token_data.get("access_token")
            
            if not access_token:
                raise HTTPException(status_code=400, detail="Failed to get access token")
            
            # Store token (in production, store in database)
            token_id = f"token_{len(access_tokens)}"
            access_tokens[token_id] = access_token
            
            # Return token to ChatGPT
            return {"access_token": token_id}
```

### Client Configurations

#### Claude Desktop Configuration

Users configure Claude Desktop to connect to Grafyn:

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

#### ChatGPT Configuration

Users register the MCP server in ChatGPT settings:

```
Server Name: Grafyn Knowledge Base
SSE Endpoint: https://your-name.ngrok.io/sse
OAuth Provider: GitHub
Client ID: your-github-client-id
Client Secret: your-github-client-secret
Authorization URL: https://your-name.ngrok.io/auth/github
Callback URL: https://your-name.ngrok.io/auth/callback
```

**Note:** ChatGPT requires a public HTTPS URL (via ngrok or Cloudflare tunnel) for OAuth to work.

### Usage Examples

**Semantic Search:**
```
User: "Search my knowledge base for information about REST API design"
Claude: [Calls query_knowledge tool] "Found 3 notes about REST API design..."
```

**Ingest Chat:**
```
User: "Save this conversation to my knowledge base"
Claude: [Calls ingest_chat tool] "Saved as note 'Discussion about Python async'"
```

**Create Draft:**
```
User: "Create a draft note summarizing our discussion"
Claude: [Calls create_draft tool] "Created draft note 'API Design Summary'"
```

## Consequences

### Positive

- **Standardized Protocol**: MCP is emerging standard for AI tool integration
- **Multi-Provider Support**: Works with Claude and ChatGPT
- **Automatic Tool Generation**: No manual tool definition needed
- **Type Safety**: FastAPI types automatically propagate to MCP
- **Extensible**: Easy to add new tools by adding API endpoints
- **Well-Documented**: MCP specification and examples available
- **Future-Proof**: Growing ecosystem of MCP-compatible tools
- **Simplified Architecture**: Single `/sse` endpoint for all clients

### Negative

- **Dependency on fastapi-mcp**: Relies on third-party library
- **SSE Transport**: Requires persistent connections
- **OAuth Complexity**: Requires OAuth provider setup and public HTTPS endpoint
- **Public Exposure Required**: ChatGPT requires public URL (ngrok/Cloudflare tunnel)
- **Single Server**: Currently only one MCP server per AI assistant
- **Documentation Overhead**: Need to document MCP setup for users
- **Token Management**: Need to store and validate OAuth tokens

### Trade-offs

| Decision | Benefit | Trade-off |
|----------|---------|-----------|
| MCP vs Custom API | Standardized, extensible | OAuth complexity for ChatGPT |
| fastapi-mcp vs Manual | Automatic tool generation | Dependency on library |
| SSE vs HTTP | Real-time, bi-directional | More complex than HTTP |
| Single /sse endpoint | Simpler architecture | Requires OAuth middleware |
| GitHub OAuth | Easy to implement | Requires public HTTPS URL |

## Alternatives Considered

### Custom REST API for AI
**Rejected because:**
- No standardization across AI providers
- Each AI provider requires different integration
- More development effort
- No ecosystem of tools

### OpenAI Function Calling
**Rejected because:**
- OpenAI-specific (not portable)
- Different API per provider
- Less flexible than MCP
- No standard tool format
- **Now obsolete**: ChatGPT supports MCP with OAuth, making this unnecessary

### LangChain Tools
**Rejected because:**
- Overkill for our use case
- Heavy dependency
- More complex than needed
- Not designed for external AI integration

### Webhooks
**Rejected because:**
- AI assistants don't support webhooks
- One-way communication only
- No real-time interaction
- Not designed for tool calling

### GraphQL
**Rejected because:**
- Not designed for tool calling
- Overkill for simple queries
- AI assistants prefer REST/MCP
- No standardization benefit

## Implementation Details

### MCP Tool Specifications

#### query_knowledge
```python
def query_knowledge(query: str, limit: int = 10) -> List[SearchResult]:
    """
    Semantic search the knowledge base.
    
    Args:
        query: Search query text
        limit: Maximum number of results (default: 10)
    
    Returns:
        List of search results with relevance scores
    """
```

#### get_note
```python
def get_note(note_id: str) -> Note:
    """
    Retrieve full note content by ID.
    
    Args:
        note_id: Note identifier
    
    Returns:
        Complete note with content and metadata
    """
```

#### list_notes
```python
def list_notes(tag: str = None, status: str = None) -> List[NoteListItem]:
    """
    List notes with optional filtering.
    
    Args:
        tag: Filter by tag (optional)
        status: Filter by status (optional)
    
    Returns:
        List of note summaries
    """
```

#### get_backlinks
```python
def get_backlinks(note_id: str) -> List[BacklinkInfo]:
    """
    Get notes that link to the specified note.
    
    Args:
        note_id: Note identifier
    
    Returns:
        List of backlinks with context
    """
```

#### ingest_chat
```python
def ingest_chat(
    content: str,
    title: str,
    source: str = "unknown",
    tags: List[str] = None
) -> Note:
    """
    Store chat transcript as evidence note.
    
    Args:
        content: Chat transcript content
        title: Note title
        source: Source AI assistant (e.g., "claude", "chatgpt")
        tags: Optional tags
    
    Returns:
        Created note
    """
```

#### create_draft
```python
def create_draft(
    title: str,
    content: str,
    based_on: str = None,
    tags: List[str] = None
) -> Note:
    """
    Create draft note for human review.
    
    Args:
        title: Note title
        content: Note content
        based_on: Source note or conversation (optional)
        tags: Optional tags
    
    Returns:
        Created draft note
    """
```

### Error Handling

MCP tools return errors in standard format:

```python
# Service raises exception
raise ValueError("Invalid note ID")

# fastapi-mcp converts to MCP error
{
  "error": {
    "code": -32602,
    "message": "Invalid params: Invalid note ID"
  }
}
```

### Logging

All MCP tool calls are logged:

```python
logger.info(f"MCP tool called: {tool_name}", extra={
    "tool": tool_name,
    "args": args,
    "result_count": len(results)
})
```

## Security Considerations

### Current Status
- ✅ OAuth 2.0 authentication via GitHub
- ✅ Optional authentication (Claude Desktop can connect without auth)
- ⚠️ Tokens stored in memory (production should use database)
- ⚠️ CORS allows all origins (should restrict for production)

### OAuth Implementation Details

**GitHub OAuth Flow:**
1. ChatGPT redirects user to `/auth/github`
2. User authorizes via GitHub
3. GitHub redirects to `/auth/callback` with authorization code
4. Backend exchanges code for access token
5. Backend stores token and returns token ID to ChatGPT
6. ChatGPT includes token in `Authorization: Bearer {token_id}` header
7. Backend validates token on each MCP request

**Security Considerations:**
- OAuth tokens are stored in memory (development only)
- Production should use Redis or database for token storage
- Claude Desktop can connect without auth (local development)
- ChatGPT requires valid OAuth token
- Public HTTPS endpoint required for OAuth (ngrok/Cloudflare tunnel)

### Future Improvements
1. **Database Token Storage**: Store OAuth tokens in Redis or database
2. **Rate Limiting**: Prevent abuse of MCP tools
3. **Audit Logging**: Log all MCP tool calls with user identity
4. **CORS Restrictions**: Limit to specific origins
5. **Tool-Level Permissions**: Restrict certain tools to authorized users
6. **Token Expiration**: Implement token refresh and expiration
7. **Multiple OAuth Providers**: Support Google, Auth0, etc.

## Testing

### MCP Integration Tests

```python
# tests/integration/test_mcp.py
async def test_mcp_query_knowledge():
    async with httpx.AsyncClient() as client:
        response = await client.post(
            "http://localhost:8080/sse",
            json={
                "jsonrpc": "2.0",
                "method": "tools/call",
                "params": {
                    "name": "query_knowledge",
                    "arguments": {"query": "test", "limit": 5}
                },
                "id": 1
            }
        )
        assert response.status_code == 200
        assert "result" in response.json()

async def test_mcp_with_oauth():
    """Test MCP with OAuth authentication"""
    async with httpx.AsyncClient() as client:
        # First, get OAuth token
        auth_response = await client.get("/auth/callback?code=test_code")
        token = auth_response.json()["access_token"]
        
        # Then use token to call MCP
        response = await client.post(
            "http://localhost:8080/sse",
            json={
                "jsonrpc": "2.0",
                "method": "tools/call",
                "params": {
                    "name": "query_knowledge",
                    "arguments": {"query": "test", "limit": 5}
                },
                "id": 1
            },
            headers={"Authorization": f"Bearer {token}"}
        )
        assert response.status_code == 200
```

## Documentation

### User Documentation

- [Chat Ingestion Guide](../../docs/chat-ingestion-guide.md) - How to use MCP with Claude
- MCP setup instructions for each AI provider
- Example prompts and workflows

### Developer Documentation

- MCP tool specifications
- API endpoint mappings
- Error handling patterns
- Testing procedures

## Future Enhancements

### Planned Features

1. ✅ **ChatGPT Integration**: Now supported via OAuth
2. **Gemini Integration**: When Gemini supports MCP
3. **Custom Tools**: Add more specialized tools
4. **Streaming**: Support streaming responses
5. **Batch Operations**: Batch note creation and updates
6. **Multiple OAuth Providers**: Support Google, Auth0, etc.

### Potential New Tools

- `update_note`: Update existing notes
- `delete_note`: Delete notes
- `get_neighbors`: Get graph neighbors
- `find_similar`: Find similar notes
- `export_notes`: Export notes in various formats

## References

- [MCP Specification](https://modelcontextprotocol.io/)
- [fastapi-mcp Documentation](https://github.com/jlowin/fastapi-mcp)
- [Claude Desktop MCP Setup](https://claude.ai/mcp)
- [Chat Ingestion Guide](../../docs/chat-ingestion-guide.md)
- [API Contracts](../../docs/api-contracts-backend.md)

## Related Decisions

- [ADR-001: Technology Stack](./adr-001-technology-stack.md) - Underlying technologies
- [ADR-002: Architecture Pattern](./adr-002-architecture-pattern.md) - How MCP fits into architecture
- [ADR-004: Data Model](./adr-004-data-model.md) - Data structures used by MCP tools
- [ADR-006: OAuth Authentication](./adr-006-oauth-authentication.md) - OAuth implementation details

---

**Status:** This decision is active and provides AI integration capabilities for both Claude Desktop and ChatGPT via a single SSE endpoint with optional OAuth authentication.

## Migration Notes (2025-12-31)

### Changes from Original Implementation

1. **Endpoint Changed**: `/mcp` → `/sse`
   - Claude Desktop config must update URL to `http://localhost:8080/sse`

2. **OAuth Authentication Added**: 
   - ChatGPT now requires OAuth 2.0 authentication
   - Claude Desktop can connect without auth (local development)
   - GitHub OAuth implementation provided

3. **Public URL Required for ChatGPT**:
   - Must expose backend via ngrok or Cloudflare tunnel
   - OAuth requires HTTPS callback URL

4. **Simplified Architecture**:
   - Single `/sse` endpoint for both clients
   - Removed `/mcp` endpoint to reduce complexity
   - Consistent SSE transport across all clients

### Setup Instructions

#### For Claude Desktop (Local Development)
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

#### For ChatGPT (Production)
1. Expose backend via ngrok: `ngrok http 8080`
2. Register GitHub OAuth app at https://github.com/settings/developers
3. Configure ChatGPT with:
   - SSE Endpoint: `https://your-name.ngrok.io/sse`
   - OAuth Provider: GitHub
   - Client ID/Secret from GitHub OAuth app
   - Callback URL: `https://your-name.ngrok.io/auth/callback`
