# OrgAI Backend Development Guide

> **Part:** Backend | **Language:** Python 3.10+ | **Framework:** FastAPI

## Prerequisites

| Requirement | Version | Check Command |
|-------------|---------|---------------|
| Python | 3.10+ | `python --version` |
| pip | Latest | `pip --version` |

---

## Quick Start

### 1. Setup Virtual Environment

```bash
cd backend
python -m venv venv

# Windows
venv\Scripts\activate

# Linux/Mac
source venv/bin/activate
```

### 2. Install Dependencies

```bash
pip install -r requirements.txt
```

### 3. Configure Environment

```bash
cp .env.example .env
# Edit .env if needed
```

### 4. Run Development Server

```bash
uvicorn app.main:app --reload --host 0.0.0.0 --port 8080
```

### 5. Verify

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
| `EMBEDDING_MODEL` | `all-MiniLM-L6-v2` | Sentence transformer model |

---

## Dependencies

```text
fastapi>=0.104.0          # Web framework
uvicorn[standard]>=0.24.0 # ASGI server
python-dotenv>=1.0.0      # Environment loading
pydantic>=2.5.0           # Data validation
pydantic-settings>=2.1.0  # Settings management
lancedb>=0.3.0            # Vector database
sentence-transformers>=2.2.0  # Embeddings
python-frontmatter>=1.0.0 # YAML frontmatter
fastapi-mcp>=0.1.0        # MCP integration
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
│   │   └── graph.py
│   ├── services/         # Business logic
│   │   ├── knowledge_store.py
│   │   ├── vector_search.py
│   │   ├── graph_index.py
│   │   └── embedding.py
│   ├── models/           # Pydantic schemas
│   │   └── note.py
│   └── mcp/              # MCP integration
│       ├── server.py
│       └── tools.py
├── requirements.txt
├── .env.example
└── venv/
```

---

## Development Tasks

### Run with Auto-Reload
```bash
uvicorn app.main:app --reload --host 0.0.0.0 --port 8080
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

### Singleton Pattern
All services use a lazy singleton pattern:

```python
_service: Optional[ServiceClass] = None

def get_service() -> ServiceClass:
    global _service
    if _service is None:
        _service = ServiceClass()
    return _service
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
   └── GraphIndexService (in-memory, uses KnowledgeStore)
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

### Add a New Pydantic Model

```python
# app/models/note.py
class NewModel(BaseModel):
    field1: str
    field2: int = 0
    optional_field: Optional[str] = None
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

## Common Issues

### "Module not found: app"
**Solution:** Ensure you're running from `backend/` directory with venv activated.

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
uvicorn app.main:app --port 8081
```
