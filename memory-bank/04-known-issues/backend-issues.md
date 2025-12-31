# Backend Known Issues and Solutions

> **Purpose:** Document common backend issues and their solutions
> **Created:** 2025-12-31
> **Status:** Active

## Overview

This document records common issues encountered in the OrgAI backend and their solutions or workarounds.

## Installation and Setup Issues

### Issue: ModuleNotFoundError for sentence-transformers

**Symptom:**
```
ModuleNotFoundError: No module named 'sentence_transformers'
```

**Cause:**
sentence-transformers package not installed or virtual environment not activated.

**Solution:**
```bash
# Activate virtual environment
cd backend
venv\Scripts\activate  # Windows
source venv/bin/activate  # Linux/Mac

# Install dependencies
pip install -r requirements.txt

# Verify installation
python -c "import sentence_transformers; print('OK')"
```

**Prevention:**
- Always activate virtual environment before running commands
- Use `pip install -r requirements.txt` after pulling changes
- Keep requirements.txt up to date

---

### Issue: LanceDB Connection Error

**Symptom:**
```
RuntimeError: Failed to connect to LanceDB
```

**Cause:**
Data directory doesn't exist or permissions issue.

**Solution:**
```bash
# Create data directory
mkdir -p ../data

# Check permissions
ls -la ../data

# Verify path in .env
cat .env | grep DATA_PATH
```

**Prevention:**
- Create data directory during setup
- Check .env configuration
- Verify write permissions

---

### Issue: Port Already in Use

**Symptom:**
```
OSError: [Errno 48] Address already in use
```

**Cause:**
Another process is using port 8080.

**Solution:**
```bash
# Find process using port 8080
netstat -ano | findstr :8080  # Windows
lsof -i :8080  # Linux/Mac

# Kill the process
taskkill /PID <PID> /F  # Windows
kill -9 <PID>  # Linux/Mac

# Or use different port
uvicorn app.main:app --port 8081
```

**Prevention:**
- Stop server before restarting
- Use different ports for multiple instances
- Check for zombie processes

---

## Runtime Issues

### Issue: Wikilinks Not Found

**Symptom:**
Backlinks not appearing or wikilinks not resolving.

**Cause:**
- Graph index not rebuilt after note changes
- Case sensitivity issues
- Space/underscore mismatch

**Solution:**
```bash
# Rebuild graph index
curl -X POST http://localhost:8080/api/graph/rebuild

# Check backend logs
tail -f backend.log | grep graph

# Verify wikilink format
# Should be: [[Note Title]] or [[Note Title|Display]]
```

**Prevention:**
- Use consistent note titles
- Rebuild graph after bulk changes
- Check wikilink syntax

---

### Issue: Search Returns No Results

**Symptom:**
Semantic search returns empty results for valid queries.

**Cause:**
- Notes not indexed
- Embedding model not loaded
- Query too specific

**Solution:**
```bash
# Reindex all notes
curl -X POST http://localhost:8080/api/notes/reindex

# Check embedding model logs
tail -f backend.log | grep embedding

# Try broader query
curl "http://localhost:8080/api/search?q=broader&limit=10"
```

**Prevention:**
- Reindex after adding notes
- Check embedding service initialization
- Use broader search terms

---

### Issue: Note Update Doesn't Reflect Changes

**Symptom:**
Updating a note doesn't change the displayed content.

**Cause:**
- Frontend cache not cleared
- Note not saved to disk
- Frontend not re-fetching

**Solution:**
```javascript
// Force refresh
const refreshNote = async () => {
  await notesApi.get(noteId)
  // Clear cache
  noteCache.value = null
  // Reload
  loadNote(noteId)
}
```

**Prevention:**
- Implement cache invalidation
- Use reactive updates
- Verify save operation success

---

### Issue: Graph Index Inconsistent

**Symptom:**
Backlinks don't match outgoing links.

**Cause:**
- Graph index not rebuilt after link changes
- Concurrent modifications
- Cache inconsistency

**Solution:**
```bash
# Rebuild graph index
curl -X POST http://localhost:8080/api/graph/rebuild

# Check graph statistics
curl http://localhost:8080/api/graph/stats

# Verify consistency
# Outgoing links count should match backlinks count across all notes
```

**Prevention:**
- Rebuild graph after bulk changes
- Use transactions for updates
- Implement graph consistency checks

---

## Performance Issues

### Issue: Slow Search Performance

**Symptom:**
Search takes > 2 seconds for small note sets.

**Cause:**
- Embedding model not cached
- LanceDB not optimized
- Too many notes indexed

**Solution:**
```python
# Cache embedding model
embedding_service = EmbeddingService()
# First call loads and caches model
vector = embedding_service.encode("text")

# Optimize LanceDB
table.create_index(
    metric="cosine",
    num_partitions=256,
    num_sub_vectors=16
)
```

**Prevention:**
- Cache embedding model
- Create LanceDB indexes
- Implement pagination for large datasets

---

### Issue: High Memory Usage

**Symptom:**
Backend process uses > 1GB memory.

**Cause:**
- All notes loaded into memory
- Large embedding vectors
- Memory leak in services

**Solution:**
```python
# Lazy load notes
def get_all_notes() -> List[Note]:
    for filepath in vault_path.glob("*.md"):
        yield load_note(filepath)  # Generator

# Clear caches
def clear_cache():
    knowledge_store._cache.clear()
    graph_index._outgoing.clear()
    graph_index._incoming.clear()
```

**Prevention:**
- Use generators for large datasets
- Implement cache limits
- Monitor memory usage

---

## Security Issues

### Issue: Path Traversal Vulnerability

**Symptom:**
Potential security vulnerability in note ID handling.

**Cause:**
Note IDs not sanitized before file access.

**Solution:**
```python
# Sanitize note IDs
def _get_filepath(self, note_id: str) -> Path:
    # Remove path traversal attempts
    note_id = note_id.replace("..", "").replace("/", "").replace("\\", "")
    # Remove special characters
    note_id = re.sub(r'[^\w\s-]', '', note_id)
    return self.vault_path / f"{note_id}.md"
```

**Prevention:**
- Always sanitize user input
- Use pathlib.Path.resolve()
- Write security tests

---

### Issue: CORS Too Permissive

**Symptom:**
Any origin can access API.

**Cause:**
CORS configured to allow all origins.

**Solution:**
```python
# In production, restrict origins
app.add_middleware(
    CORSMiddleware,
    allow_origins=["https://yourdomain.com"],  # Specific domains
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)
```

**Prevention:**
- Configure specific origins in production
- Use environment variables for allowed origins
- Document CORS configuration

---

## Data Issues

### Issue: Duplicate Note IDs

**Symptom:**
Creating a note with same title as existing note.

**Cause:**
ID generation doesn't check for existing notes.

**Solution:**
```python
def create_note(self, data: NoteCreate) -> Note:
    note_id = self._generate_id(data.title)
    filepath = self._get_filepath(note_id)
    
    if filepath.exists():
        # Append number to make unique
        counter = 1
        while filepath.exists():
            note_id = f"{self._generate_id(data.title)}_{counter}"
            filepath = self._get_filepath(note_id)
            counter += 1
    
    # Create note
    # ...
```

**Prevention:**
- Check for existing notes before creating
- Use unique IDs with timestamps
- Implement conflict resolution

---

### Issue: YAML Frontmatter Parsing Error

**Symptom:**
Error reading note files with frontmatter.

**Cause:**
Invalid YAML syntax in frontmatter.

**Solution:**
```python
# Handle parsing errors gracefully
def _load_note(self, filepath: Path) -> Note:
    try:
        post = frontmatter.load(filepath)
    except Exception as e:
        logger.error(f"Failed to parse frontmatter: {filepath}", exc_info=True)
        # Try to load without frontmatter
        with open(filepath, 'r', encoding='utf-8') as f:
            content = f.read()
        post = frontmatter.Post(content)
    
    return Note(...)
```

**Prevention:**
- Validate YAML syntax
- Use YAML linter
- Provide examples in documentation

---

## Testing Issues

### Issue: Tests Fail with FileNotFoundError

**Symptom:**
Tests fail because files don't exist in test directory.

**Cause:**
Test fixtures not creating temporary directories.

**Solution:**
```python
@pytest.fixture
def tmp_path(tmp_path_factory):
    """Create temporary directory for tests."""
    tmp = tmp_path_factory.mktemp()
    # Create subdirectories
    (tmp / "vault").mkdir()
    (tmp / "data").mkdir()
    return tmp

@pytest.fixture
def temp_vault(tmp_path):
    """Create temporary vault directory."""
    vault = tmp_path / "vault"
    vault.mkdir(exist_ok=True)
    return vault
```

**Prevention:**
- Use pytest tmp_path fixture
- Create all required directories
- Clean up after tests

---

### Issue: Async Tests Hang

**Symptom:**
Tests timeout or hang indefinitely.

**Cause:**
Async/await mismatch in test code.

**Solution:**
```python
# Use pytest-asyncio
import pytest

@pytest.mark.asyncio
async def test_async_function():
    result = await async_function()
    assert result is not None

# Or use sync wrapper
def test_async_function_sync():
    import asyncio
    result = asyncio.run(async_function())
    assert result is not None
```

**Prevention:**
- Use pytest.mark.asyncio for async tests
- Match async/await in test and code
- Set appropriate timeouts

---

## Logging Issues

### Issue: No Logs Generated

**Symptom:**
Backend runs but no log files created.

**Cause:**
Logging not configured or file path invalid.

**Solution:**
```python
# In logging_config.py
import logging
from logging.handlers import RotatingFileHandler

def setup_logging():
    logger = logging.getLogger()
    logger.setLevel(logging.INFO)
    
    # File handler
    file_handler = RotatingFileHandler(
        'backend.log',
        maxBytes=10*1024*1024,  # 10MB
        backupCount=5
    )
    file_handler.setLevel(logging.INFO)
    
    # Console handler
    console_handler = logging.StreamHandler()
    console_handler.setLevel(logging.INFO)
    
    # Formatter
    formatter = logging.Formatter(
        '%(asctime)s - %(name)s - %(levelname)s - %(message)s'
    )
    file_handler.setFormatter(formatter)
    console_handler.setFormatter(formatter)
    
    # Add handlers
    logger.addHandler(file_handler)
    logger.addHandler(console_handler)

# In main.py
from app.logging_config import setup_logging
setup_logging()
```

**Prevention:**
- Configure logging on startup
- Use RotatingFileHandler for log rotation
- Set appropriate log levels

---

## MCP Integration Issues

### Issue: MCP Tools Not Appearing in Claude

**Symptom:**
Claude Desktop doesn't show OrgAI MCP tools.

**Cause:**
- MCP server not running
- Configuration file incorrect
- Network connection issue

**Solution:**
```json
// Check Claude Desktop config
{
  "mcpServers": {
    "orgai": {
      "url": "http://localhost:8080/mcp",
      "transport": "sse"
    }
  }
}

// Verify backend is running
curl http://localhost:8080/mcp

// Check Claude Desktop logs
// Location: ~/Library/Logs/Claude/
```

**Prevention:**
- Verify backend is running before using MCP
- Test MCP endpoint with curl
- Check Claude Desktop logs

---

### Issue: MCP Tool Calls Fail

**Symptom:**
Claude calls MCP tool but gets error.

**Cause:**
- Tool arguments invalid
- Service error not handled
- MCP server not responding

**Solution:**
```python
# Add proper error handling in MCP tools
@router.post("/api/notes")
async def create_note(data: NoteCreate):
    try:
        note = knowledge_store.create_note(data)
        return note
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    except Exception as e:
        logger.error(f"Error creating note: {e}", exc_info=True)
        raise HTTPException(status_code=500, detail="Internal error")
```

**Prevention:**
- Add error handling to all endpoints
- Log MCP tool calls
- Validate tool arguments

---

## Troubleshooting Checklist

When encountering backend issues:

1. **Check logs**: `tail -f backend.log`
2. **Verify configuration**: Check `.env` file
3. **Restart services**: Stop and restart backend
4. **Test endpoints**: Use curl or Postman
5. **Check dependencies**: Verify all packages installed
6. **Clear caches**: Delete `data/` and rebuild
7. **Review recent changes**: Check git diff for issues

## Related Documentation

- [Frontend Issues](./frontend-issues.md)
- [Solutions](./solutions.md)
- [Configuration Reference](../05-configuration/)
- [Development Guide - Backend](../../docs/development-guide-backend.md)

---

**See Also:**
- [Architecture - Backend](../../docs/architecture-backend.md)
- [API Contracts](../../docs/api-contracts-backend.md)
- [IMPROVEMENTS.md](../../docs/IMPROVEMENTS.md)
