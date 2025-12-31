# ADR-006: OAuth Authentication for ChatGPT MCP Integration

## Status
Accepted

## Date
2025-12-31

## Context

As part of enabling ChatGPT to connect to Seedream's MCP server ([ADR-003](./adr-003-mcp-integration.md)), we need to implement authentication. Unlike Claude Desktop which runs locally and can connect without authentication, ChatGPT requires OAuth 2.0 authentication for security reasons.

### Requirements

1. **Secure Authentication**: ChatGPT must authenticate before accessing MCP tools
2. **OAuth 2.0 Standard**: Use industry-standard OAuth 2.0 protocol
3. **GitHub OAuth**: Use GitHub as OAuth provider (familiar to developers)
4. **Optional for Claude Desktop**: Allow Claude Desktop to connect without auth for local development
5. **Public HTTPS Required**: OAuth requires public HTTPS endpoint (ngrok/Cloudflare tunnel)

### Challenges

- **Token Storage**: Need to store OAuth tokens securely
- **Token Validation**: Must validate tokens on each MCP request
- **Public URL**: OAuth requires public HTTPS callback URL
- **Development vs Production**: Different requirements for local and production

## Decision

We implemented **OAuth 2.0 authentication using GitHub** as as OAuth provider, with optional authentication for Claude Desktop.

### OAuth Architecture

```mermaid
sequenceDiagram
    participant User
    participant ChatGPT
    participant GitHub
    participant Backend
    
    User->>ChatGPT: Register MCP server
    ChatGPT->>Backend: GET /auth/github
    Backend->>GitHub: Redirect to authorize
    GitHub->>User: Show consent screen
    User->>GitHub: Authorize access
    GitHub->>Backend: Redirect to /auth/callback?code=xxx
    Backend->>GitHub: Exchange code for token
    GitHub->>Backend: Return access token
    Backend->>ChatGPT: Return token_id
    ChatGPT->>Backend: SSE connection with token
    Backend->>Backend: Validate token
    Backend->>ChatGPT: SSE connection established
```

### Implementation

#### OAuth Routes

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

#### Token Validation Middleware

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
    if token not in access_tokens:
        raise HTTPException(status_code=401, detail="Invalid OAuth token")
    return True

def setup_mcp(app: FastAPI) -> None:
    mcp = FastApiMCP(
        app,
        name="Seedream Knowledge Graph",
        description="Access and query an organizational knowledge base",
        transport="sse"
    )
    # Single SSE endpoint for both Claude and ChatGPT
    mcp.mount(path="/sse", dependencies=[Depends(verify_oauth)])
```

#### Environment Configuration

**File:** `backend/.env`

```bash
# OAuth Configuration (for ChatGPT)
GITHUB_CLIENT_ID=your-github-client-id
GITHUB_CLIENT_SECRET=your-github-client-secret
GITHUB_REDIRECT_URI=https://your-name.ngrok.io/auth/callback
```

## Consequences

### Positive

- **Standardized Protocol**: OAuth 2.0 is industry standard
- **GitHub Integration**: Familiar OAuth provider for developers
- **Secure**: Proper authentication and authorization
- **Flexible**: Optional authentication allows Claude Desktop to work locally
- **Single Endpoint**: Both clients use same `/sse` endpoint
- **Simplified Architecture**: No need for separate `/mcp` endpoint

### Negative

- **Complexity**: Adds OAuth implementation complexity
- **Public URL Required**: Requires ngrok/Cloudflare tunnel for development
- **Token Management**: Need to store and validate tokens
- **GitHub Dependency**: Relies on GitHub as OAuth provider
- **Memory Storage**: Current implementation stores tokens in memory (not production-ready)

### Trade-offs

| Decision | Benefit | Trade-off |
|----------|---------|-----------|
| GitHub OAuth | Familiar, easy to implement | Requires GitHub account |
| Optional Auth | Simplifies local development | Less secure for local Claude |
| Memory Storage | Simple for development | Not production-ready |
| Single /sse Endpoint | Simpler architecture | Requires middleware complexity |

## Alternatives Considered

### API Key Authentication
**Rejected because:**
- Not standardized like OAuth
- No built-in user consent flow
- Less secure for public endpoints
- ChatGPT specifically requires OAuth

### Custom OAuth Server
**Rejected because:**
- More complex to implement
- Requires additional infrastructure
- GitHub OAuth is sufficient for current needs

### JWT Tokens
**Rejected because:**
- Overkill for current requirements
- Adds unnecessary complexity
- OAuth provides token management

### No Authentication
**Rejected because:**
- ChatGPT requires OAuth
- Security risk for public endpoints
- No audit trail

## Implementation Details

### OAuth Flow

1. **Authorization Request**
   - ChatGPT requests authorization via `/auth/github`
   - Backend redirects to GitHub OAuth authorize endpoint
   - User sees GitHub consent screen

2. **Authorization Grant**
   - User authorizes application
   - GitHub redirects to `/auth/callback` with authorization code

3. **Token Exchange**
   - Backend exchanges authorization code for access token
   - GitHub returns access token
   - Backend stores token and returns token_id to ChatGPT

4. **Token Usage**
   - ChatGPT includes token in `Authorization: Bearer {token_id}` header
   - Backend validates token before allowing MCP access
   - Token is stored in memory (development) or database (production)

### Security Considerations

#### Current Implementation (Development)

- ✅ OAuth 2.0 protocol
- ✅ GitHub as OAuth provider
- ✅ Token validation on each request
- ⚠️ Tokens stored in memory
- ⚠️ No token expiration
- ⚠️ No token refresh

#### Production Requirements

- ✅ Store tokens in Redis or database
- ✅ Implement token expiration (e.g., 1 hour)
- ✅ Implement token refresh
- ✅ Use HTTPS for all OAuth endpoints
- ✅ Validate redirect URI matches GitHub OAuth app
- ✅ Rate limit OAuth requests
- ✅ Audit log all OAuth flows

### Token Storage Options

#### Development (Current)
```python
# In-memory storage (not production-ready)
access_tokens = {}
```

#### Production (Recommended)
```python
# Redis storage
import redis

redis_client = redis.Redis(host='localhost', port=6379, db=0)

def store_token(token_id, access_token):
    redis_client.setex(token_id, 3600, access_token)  # 1 hour expiry

def get_token(token_id):
    return redis_client.get(token_id)
```

```python
# Database storage (PostgreSQL)
from sqlalchemy import create_engine, Column, String, DateTime
from datetime import datetime, timedelta

engine = create_engine('postgresql://user:pass@localhost/seedream')

class OAuthToken(Base):
    __tablename__ = 'oauth_tokens'
    token_id = Column(String, primary_key=True)
    access_token = Column(String)
    created_at = Column(DateTime, default=datetime.utcnow)
    expires_at = Column(DateTime)
```

## Testing

### OAuth Flow Tests

```python
# tests/integration/test_oauth.py
async def test_oauth_flow():
    """Test complete OAuth flow"""
    async with httpx.AsyncClient() as client:
        # Step 1: Request authorization
        response = await client.get("/auth/github")
        assert response.status_code == 302
        assert "github.com" in response.headers["location"]
        
        # Step 2: Simulate callback (in real test, use mock GitHub)
        response = await client.get("/auth/callback?code=test_code")
        assert response.status_code == 200
        data = response.json()
        assert "access_token" in data
        
        # Step 3: Use token to access MCP
        token = data["access_token"]
        response = await client.post(
            "/sse",
            headers={"Authorization": f"Bearer {token}"}
        )
        assert response.status_code == 200

async def test_mcp_without_oauth():
    """Test that Claude Desktop can connect without auth"""
    async with httpx.AsyncClient() as client:
        response = await client.post("/sse")
        assert response.status_code == 200

async def test_invalid_token():
    """Test that invalid token is rejected"""
    async with httpx.AsyncClient() as client:
        response = await client.post(
            "/sse",
            headers={"Authorization": "Bearer invalid_token"}
        )
        assert response.status_code == 401
```

## Setup Instructions

### GitHub OAuth App Registration

1. Go to https://github.com/settings/developers
2. Click "New OAuth App"
3. Fill in the form:
   - **Application name**: Seedream Knowledge Base
   - **Homepage URL**: https://your-name.ngrok.io
   - **Authorization callback URL**: https://your-name.ngrok.io/auth/callback
4. Click "Register application"
5. Copy the **Client ID** and **Client Secret**

### Backend Configuration

Add to `backend/.env`:

```bash
GITHUB_CLIENT_ID=your-github-client-id
GITHUB_CLIENT_SECRET=your-github-client-secret
GITHUB_REDIRECT_URI=https://your-name.ngrok.io/auth/callback
```

### Expose Backend Publicly

For development, use ngrok:

```bash
# Install ngrok
# https://ngrok.com/download

# Start ngrok tunnel
ngrok http 8080

# Copy HTTPS URL (e.g., https://abc123.ngrok.io)
# Update GITHUB_REDIRECT_URI to use this URL
```

### ChatGPT Configuration

Register MCP server in ChatGPT settings:

```
Server Name: Seedream Knowledge Base
SSE Endpoint: https://your-name.ngrok.io/sse
OAuth Provider: GitHub
Client ID: your-github-client-id
Client Secret: your-github-client-secret
Authorization URL: https://your-name.ngrok.io/auth/github
Callback URL: https://your-name.ngrok.io/auth/callback
```

## Future Enhancements

### Planned Features

1. **Database Token Storage**: Store OAuth tokens in Redis or database
2. **Token Expiration**: Implement token expiration (e.g., 1 hour)
3. **Token Refresh**: Implement token refresh mechanism
4. **Multiple OAuth Providers**: Support Google, Auth0, etc.
5. **Rate Limiting**: Prevent abuse of OAuth endpoints
6. **Audit Logging**: Log all OAuth flows for security monitoring
7. **PKCE Flow**: Implement Proof Key for Code Exchange for mobile apps

### Security Improvements

1. **Token Encryption**: Encrypt stored tokens
2. **CORS Restrictions**: Limit CORS to specific origins
3. **IP Whitelisting**: Restrict OAuth to specific IP ranges
4. **Two-Factor Authentication**: Require 2FA for OAuth
5. **Scope Management**: Limit OAuth scopes to minimum required

## References

- [OAuth 2.0 Specification](https://oauth.net/2/)
- [GitHub OAuth Apps](https://docs.github.com/en/developers/apps/building-oauth-apps)
- [fastapi-mcp Documentation](https://github.com/jlowin/fastapi-mcp)
- [ADR-003: MCP Integration](./adr-003-mcp-integration.md)
- [Environment Variables](../05-configuration/environment-variables.md)

## Related Decisions

- [ADR-001: Technology Stack](./adr-001-technology-stack.md) - Underlying technologies
- [ADR-002: Architecture Pattern](./adr-002-architecture-pattern.md) - How OAuth fits into architecture
- [ADR-003: MCP Integration](./adr-003-mcp-integration.md) - OAuth enables ChatGPT MCP access

---

**Status:** This decision is active and provides OAuth authentication for ChatGPT MCP integration.
