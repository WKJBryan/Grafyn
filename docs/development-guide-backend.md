# Seedream Backend Development Guide

> **Part:** Backend | **Language:** Python 3.11+ | **Framework:** FastAPI

## Prerequisites

| Requirement | Version | Check Command |
|-------------|---------|---------------|
| Python | 3.11+ | `python --version` |
| uv | Latest | `uv --version` |

---

## Quick Start

### 1. Install Dependencies

```bash
# From project root
uv sync
```

This installs all dependencies from `pyproject.toml`.

### 2. Configure Environment

```bash
cd backend
cp .env.example .env
# Edit .env if needed
```

### 3. Run Development Server

```bash
# From project root
uv run uvicorn backend.app.main:app --reload --host 0.0.0.0 --port 8080

# Or from backend directory
cd backend
uv run uvicorn app.main:app --reload --host 0.0.0.0 --port 8080
```

### 4. Verify

- **API Docs:** http://localhost:8080/docs
- **Health Check:** http://localhost:8080/health
- **Root Info:** http://localhost:8080/

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `VAULT_PATH` | `../vault` | Markdown notes directory |
| `DATA_PATH` | `../data` | LanceDB storage |
| `SERVER_HOST` | `0.0.0.0` | Bind address |
| `SERVER_PORT` | `8080` | HTTP port |
| `ENVIRONMENT` | `development` | Environment mode |
| `EMBEDDING_MODEL` | `all-MiniLM-L6-v2` | Sentence transformer model |
| `GITHUB_CLIENT_ID` | - | GitHub OAuth client ID |
| `GITHUB_CLIENT_SECRET` | - | GitHub OAuth client secret |
| `GITHUB_REDIRECT_URI` | - | OAuth redirect URI |
| `RATE_LIMIT_ENABLED` | `true` | Enable rate limiting |
| `RATE_LIMIT_PER_MINUTE` | `10` | Requests per minute |
| `RATE_LIMIT_PER_HOUR` | `50` | Requests per hour |
| `CORS_ORIGINS` | - | Comma-separated allowed origins |

---

## Dependencies

From `pyproject.toml`:

```toml
[project]
requires-python = ">=3.11"
dependencies = [
    "aiofiles>=25.1.0",
    "fastapi>=0.128.0",
    "httpx>=0.28.1",
    "lancedb>=0.26.0",
    "pydantic>=2.12.5",
    "pydantic-settings>=2.12.0",
    "python-dotenv>=1.2.1",
    "python-frontmatter>=1.1.0",
    "python-multipart>=0.0.21",
    "sentence-transformers>=5.2.0",
    "uvicorn[standard]>=0.40.0",
]
```

---

## Project Structure

```
backend/
├── app/
│   ├── __init__.py
│   ├── main.py           # Application entry
│   ├── config.py         # Settings
│   ├── routers/          # API endpoints
│   │   ├── notes.py
│   │   ├── search.py
│   │   ├── graph.py
│   │   └── oauth.py
│   ├── services/         # Business logic
│   │   ├── knowledge_store.py
│   │   ├── vector_search.py
│   │   ├── graph_index.py
│   │   ├── embedding.py
│   │   └── token_store.py
│   ├── middleware/       # Request processing
│   │   ├── security.py
│   │   ├── logging.py
│   │   └── rate_limit.py
│   ├── models/           # Pydantic schemas
│   │   └── note.py
│   └── mcp/              # MCP integration
│       ├── server.py
│       └── tools.py
├── Dockerfile
├── docker-compose.yml
├── requirements.txt
└── .env.example
```

---

## Development Tasks

### Run with Auto-Reload
```bash
uv run uvicorn backend.app.main:app --reload --host 0.0.0.0 --port 8080
```

### Reindex All Notes
```bash
curl -X POST http://localhost:8080/api/notes/reindex
```

### Rebuild Link Graph
```bash
curl -X POST http://localhost:8080/api/graph/rebuild
```

### Test API
```bash
# List notes
curl http://localhost:8080/api/notes

# Search
curl "http://localhost:8080/api/search?q=knowledge"

# Get specific note
curl http://localhost:8080/api/notes/Welcome
```

---

## Service Architecture

### Lifespan Pattern
Services are initialized during application startup:

```python
@asynccontextmanager
async def lifespan(app: FastAPI):
    # Initialize services
    app.state.knowledge_store = KnowledgeStore()
    app.state.vector_search = VectorSearchService()
    app.state.graph_index = GraphIndexService()
    yield
    # Cleanup on shutdown
```

### Service Dependencies

```
Config
   │
   ├── KnowledgeStore (vault filesystem)
   │
   ├── EmbeddingService (sentence-transformers)
   │       │
   │       └── VectorSearchService (LanceDB)
   │
   ├── GraphIndexService (in-memory, uses KnowledgeStore)
   │
   └── TokenStore (OAuth tokens)
```

---

## Adding New Features

### Add a New API Endpoint

1. **Create/Edit Router** (`app/routers/`)
```python
@router.get("/new-endpoint")
async def new_endpoint():
    service = get_service()
    return service.do_something()
```

2. **Register Router** (if new file, in `app/main.py`)
```python
from app.routers import new_router
app.include_router(new_router.router, prefix="/api/new", tags=["new"])
```

### Add a New Service

1. **Create Service** (`app/services/`)
```python
class NewService:
    def __init__(self):
        self._data = None
    
    def do_something(self):
        return "result"

_service = None
def get_new_service() -> NewService:
    global _service
    if _service is None:
        _service = NewService()
    return _service
```

2. **Use in Router**
```python
from app.services.new_service import get_new_service
```

### Add New Middleware

1. **Create Middleware** (`app/middleware/`)
```python
class NewMiddleware(BaseHTTPMiddleware):
    async def dispatch(self, request: Request, call_next):
        # Pre-processing
        response = await call_next(request)
        # Post-processing
        return response
```

2. **Register in main.py**
```python
app.add_middleware(NewMiddleware)
```

---

## Debugging

### Enable Debug Logging
```python
import logging
logging.basicConfig(level=logging.DEBUG)
```

### Check LanceDB Table
```python
import lancedb
db = lancedb.connect("../data/lancedb")
table = db.open_table("notes")
print(table.to_pandas())
```

### Inspect Embeddings
```python
from app.services.embedding import get_embedding_service
embed = get_embedding_service()
vector = embed.encode("test text")
print(f"Dimension: {len(vector)}")  # Should be 384
```

---

## Docker

### Build and Run
```bash
cd backend
docker-compose up --build
```

### Dockerfile
```dockerfile
FROM python:3.11-slim
WORKDIR /app
COPY requirements.txt .
RUN pip install -r requirements.txt
COPY . .
CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8080"]
```

---

## Common Issues

### "Module not found: app"
**Solution:** Ensure you're running from correct directory with uv.

### "LanceDB table doesn't exist"
**Solution:** Run reindex: `POST /api/notes/reindex`

### "Embedding model download slow"
**Cause:** First run downloads ~90MB model.
**Solution:** Wait for download or pre-download:
```python
from sentence_transformers import SentenceTransformer
model = SentenceTransformer("all-MiniLM-L6-v2")
```

### "Port already in use"
**Solution:** Kill existing process or use different port:
```bash
uv run uvicorn app.main:app --port 8081
```

### "Rate limit exceeded"
**Solution:** Wait or disable rate limiting:
```env
RATE_LIMIT_ENABLED=false
```
