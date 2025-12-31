# Backend Development Patterns

> **Purpose:** Document common patterns and best practices for OrgAI backend development
> **Created:** 2025-12-31
> **Status:** Active

## Overview

This document describes common patterns and conventions used in the OrgAI backend (FastAPI/Python) to ensure consistency and maintainability.

## Service Layer Pattern

### Pattern Description

Services encapsulate business logic and are used by routers to handle requests.

### Implementation

```python
# services/knowledge_store.py
from pathlib import Path
from typing import List, Dict, Set
import frontmatter
import re

class KnowledgeStore:
    """Service for managing Markdown notes."""
    
    WIKILINK_PATTERN = re.compile(r'\[\[([^\]|]+)(?:\|[^\]]+)?\]\]')
    
    def __init__(self, vault_path: str):
        self.vault_path = Path(vault_path)
        self.vault_path.mkdir(parents=True, exist_ok=True)
        self._cache: Dict[str, Note] = {}
    
    def list_notes(self) -> List[NoteListItem]:
        """List all notes in the vault."""
        notes = []
        for filepath in self.vault_path.glob("*.md"):
            note = self._load_note(filepath)
            notes.append(NoteListItem(
                id=note.id,
                title=note.title,
                status=note.frontmatter.status,
                tags=note.frontmatter.tags,
                created=note.frontmatter.created,
                modified=note.frontmatter.modified,
                link_count=len(note.outgoing_links)
            ))
        return notes
    
    def get_note(self, note_id: str) -> Note:
        """Retrieve a note by ID."""
        filepath = self._get_filepath(note_id)
        return self._load_note(filepath)
    
    def create_note(self, data: NoteCreate) -> Note:
        """Create a new note."""
        note_id = self._generate_id(data.title)
        filepath = self._get_filepath(note_id)
        
        if filepath.exists():
            raise ValueError(f"Note {note_id} already exists")
        
        note = Note(
            id=note_id,
            title=data.title,
            content=data.content,
            frontmatter=NoteFrontmatter(
                title=data.title,
                created=datetime.now(),
                modified=datetime.now(),
                tags=data.tags,
                status=data.status
            ),
            outgoing_links=self.extract_wikilinks(data.content),
            backlinks=[]
        )
        
        self._save_note(filepath, note)
        return note
    
    def update_note(self, note_id: str, data: NoteUpdate) -> Note:
        """Update an existing note."""
        filepath = self._get_filepath(note_id)
        existing = self._load_note(filepath)
        
        if data.title is not None:
            existing.title = data.title
        if data.content is not None:
            existing.content = data.content
        if data.tags is not None:
            existing.frontmatter.tags = data.tags
        if data.status is not None:
            existing.frontmatter.status = data.status
        
        existing.frontmatter.modified = datetime.now()
        existing.outgoing_links = self.extract_wikilinks(existing.content)
        
        self._save_note(filepath, existing)
        return existing
    
    def delete_note(self, note_id: str) -> None:
        """Delete a note."""
        filepath = self._get_filepath(note_id)
        if not filepath.exists():
            raise ValueError(f"Note {note_id} not found")
        filepath.unlink()
    
    @staticmethod
    def extract_wikilinks(content: str) -> List[str]:
        """Extract wikilinks from content."""
        matches = KnowledgeStore.WIKILINK_PATTERN.findall(content)
        return [m.strip() for m in matches]
    
    def _load_note(self, filepath: Path) -> Note:
        """Load note from file."""
        post = frontmatter.load(filepath)
        return Note(
            id=filepath.stem,
            title=post.get('title', filepath.stem),
            content=post.content,
            frontmatter=NoteFrontmatter(**post.metadata),
            outgoing_links=self.extract_wikilinks(post.content),
            backlinks=[]
        )
    
    def _save_note(self, filepath: Path, note: Note) -> None:
        """Save note to file."""
        post = frontmatter.Post(
            note.content,
            **note.frontmatter.dict(exclude_unset=True)
        )
        frontmatter.dump(post, filepath)
    
    def _get_filepath(self, note_id: str) -> Path:
        """Get filepath for note ID."""
        return self.vault_path / f"{note_id}.md"
    
    @staticmethod
    def _generate_id(title: str) -> str:
        """Generate note ID from title."""
        return title.replace(" ", "_")
```

### Usage in Router

```python
# routers/notes.py
from fastapi import APIRouter, HTTPException
from app.models.note import NoteCreate, NoteUpdate
from app.services.knowledge_store import KnowledgeStore

router = APIRouter()

@router.get("/")
async def list_notes(store: KnowledgeStore = Depends(get_store)):
    """List all notes."""
    return store.list_notes()

@router.get("/{note_id}")
async def get_note(note_id: str, store: KnowledgeStore = Depends(get_store)):
    """Get a specific note."""
    try:
        return store.get_note(note_id)
    except FileNotFoundError:
        raise HTTPException(status_code=404, detail="Note not found")

@router.post("/", status_code=201)
async def create_note(
    data: NoteCreate,
    store: KnowledgeStore = Depends(get_store)
):
    """Create a new note."""
    try:
        return store.create_note(data)
    except ValueError as e:
        raise HTTPException(status_code=409, detail=str(e))
```

## Dependency Injection Pattern

### Pattern Description

Use FastAPI's dependency injection to provide services to routers.

### Implementation

```python
# main.py
from fastapi import FastAPI
from fastapi import Depends
from app.services import KnowledgeStore, VectorSearchService, GraphIndexService
from app.config import get_settings

# Initialize services
settings = get_settings()
knowledge_store = KnowledgeStore(settings.vault_path)
vector_search = VectorSearchService(settings.data_path)
graph_index = GraphIndexService(knowledge_store)

# Dependency providers
def get_store() -> KnowledgeStore:
    return knowledge_store

def get_vector_search() -> VectorSearchService:
    return vector_search

def get_graph_index() -> GraphIndexService:
    return graph_index

app = FastAPI()

# Register dependencies
app.dependency_overrides[KnowledgeStore] = get_store
app.dependency_overrides[VectorSearchService] = get_vector_search
app.dependency_overrides[GraphIndexService] = get_graph_index
```

## Error Handling Pattern

### Pattern Description

Define custom exceptions and handle them in routers.

### Implementation

```python
# exceptions.py
class NoteNotFoundError(Exception):
    """Raised when a note doesn't exist."""
    pass

class EmbeddingError(Exception):
    """Raised when embedding generation fails."""
    pass

class GraphError(Exception):
    """Raised when graph operations fail."""
    pass

# routers/notes.py
from app.exceptions import NoteNotFoundError

@router.get("/{note_id}")
async def get_note(note_id: str, store: KnowledgeStore = Depends(get_store)):
    try:
        return store.get_note(note_id)
    except NoteNotFoundError as e:
        raise HTTPException(status_code=404, detail=str(e))
    except Exception as e:
        logger.error(f"Error getting note {note_id}: {e}", exc_info=True)
        raise HTTPException(status_code=500, detail="Internal server error")
```

## Validation Pattern

### Pattern Description

Use Pydantic for request/response validation.

### Implementation

```python
# models/note.py
from pydantic import BaseModel, Field, validator
from datetime import datetime
from typing import List, Optional

class NoteCreate(BaseModel):
    title: str = Field(..., min_length=1, max_length=200)
    content: str = Field(..., min_length=0)
    tags: List[str] = Field(default_factory=list, max_items=20)
    status: str = Field(default="draft", regex="^(draft|evidence|canonical)$")
    
    @validator('tags')
    def validate_tags(cls, v):
        for tag in v:
            if len(tag) > 50:
                raise ValueError(f"Tag too long: {tag}")
        return v

class NoteUpdate(BaseModel):
    title: Optional[str] = Field(None, min_length=1, max_length=200)
    content: Optional[str] = Field(None, min_length=0)
    tags: Optional[List[str]] = Field(None, max_items=20)
    status: Optional[str] = Field(None, regex="^(draft|evidence|canonical)$")
```

## Async Pattern

### Pattern Description

Use async for I/O operations, sync for CPU-bound operations.

### Implementation

```python
# routers/notes.py
@router.get("/")
async def list_notes(store: KnowledgeStore = Depends(get_store)):
    # File I/O - can be async, but sync is fine for local files
    return store.list_notes()

@router.post("/reindex")
async def reindex_notes(
    vector_search: VectorSearchService = Depends(get_vector_search),
    store: KnowledgeStore = Depends(get_store)
):
    # Long-running operation - use async to not block
    notes = store.get_all_content()
    await vector_search.index_all(notes)
    return {"indexed": len(notes), "message": f"Reindexed {len(notes)} notes"}
```

## Logging Pattern

### Pattern Description

Use structured logging with module-level loggers.

### Implementation

```python
# services/knowledge_store.py
import logging

logger = logging.getLogger(__name__)

class KnowledgeStore:
    def create_note(self, data: NoteCreate) -> Note:
        logger.info(f"Creating note: {data.title}")
        try:
            note = self._create_note_impl(data)
            logger.debug(f"Note created: {note.id}")
            return note
        except Exception as e:
            logger.error(f"Failed to create note: {e}", exc_info=True)
            raise
```

## Configuration Pattern

### Pattern Description

Use Pydantic Settings for configuration.

### Implementation

```python
# config.py
from pydantic_settings import BaseSettings

class Settings(BaseSettings):
    vault_path: str = "../vault"
    data_path: str = "../data"
    server_host: str = "0.0.0.0"
    server_port: int = 8080
    embedding_model: str = "all-MiniLM-L6-v2"
    
    class Config:
        env_file = ".env"

def get_settings() -> Settings:
    return Settings()

# Usage
settings = get_settings()
store = KnowledgeStore(settings.vault_path)
```

## Testing Pattern

### Pattern Description

Use pytest with fixtures for testing.

### Implementation

```python
# tests/conftest.py
import pytest
from pathlib import Path
from app.services.knowledge_store import KnowledgeStore

@pytest.fixture
def temp_vault(tmp_path: Path) -> Path:
    """Create temporary vault directory."""
    vault_path = tmp_path / "vault"
    vault_path.mkdir()
    return vault_path

@pytest.fixture
def knowledge_store(temp_vault: Path) -> KnowledgeStore:
    """Create KnowledgeStore with temporary vault."""
    return KnowledgeStore(str(temp_vault))

# tests/unit/test_knowledge_store.py
def test_create_note(knowledge_store: KnowledgeStore):
    """Test creating a note."""
    note = knowledge_store.create_note(NoteCreate(
        title="Test Note",
        content="Test content"
    ))
    
    assert note.id == "Test_Note"
    assert note.title == "Test Note"
    assert note.content == "Test content"
```

## Path Sanitization Pattern

### Pattern Description

Sanitize user input to prevent path traversal attacks.

### Implementation

```python
# services/knowledge_store.py
import re

def _get_filepath(self, note_id: str) -> Path:
    """Get filepath for note ID with sanitization."""
    # Remove path traversal attempts
    note_id = note_id.replace("..", "").replace("/", "").replace("\\", "")
    # Remove special characters
    note_id = re.sub(r'[^\w\s-]', '', note_id)
    return self.vault_path / f"{note_id}.md"
```

## Batch Processing Pattern

### Pattern Description

Process items in batches for efficiency.

### Implementation

```python
# services/vector_search.py
def index_all(self, notes: List[Tuple[str, str, str]]) -> None:
    """Index all notes in batches."""
    BATCH_SIZE = 100
    
    for i in range(0, len(notes), BATCH_SIZE):
        batch = notes[i:i + BATCH_SIZE]
        logger.info(f"Indexing batch {i // BATCH_SIZE + 1}/{(len(notes) + BATCH_SIZE - 1) // BATCH_SIZE}")
        
        vectors = self.embedding_service.encode_batch([
            f"{title}\n\n{content}" for title, content, _ in batch
        ])
        
        for (note_id, title, content), vector in zip(batch, vectors):
            self.table.add({
                "note_id": note_id,
                "title": title,
                "text": content[:1000],
                "vector": vector
            })
```

## Common Patterns Summary

| Pattern | Use Case | Example |
|---------|-----------|----------|
| Service Layer | Business logic encapsulation | `KnowledgeStore` |
| Dependency Injection | Provide services to routers | `Depends(get_store)` |
| Error Handling | Custom exceptions | `NoteNotFoundError` |
| Validation | Request/response validation | Pydantic models |
| Async | I/O operations | `async def` |
| Logging | Structured logging | `logger.info()` |
| Configuration | Environment-based config | `Settings` class |
| Testing | Isolated test fixtures | `@pytest.fixture` |
| Path Sanitization | Security | `note_id.replace("..", "")` |
| Batch Processing | Efficiency | Process in chunks of 100 |

## Anti-Patterns to Avoid

### ❌ Don't Use Global State

```python
# Bad
store = KnowledgeStore()  # Global

def get_note(note_id: str):
    return store.get_note(note_id)

# Good - use dependency injection
def get_note(note_id: str, store: KnowledgeStore = Depends(get_store)):
    return store.get_note(note_id)
```

### ❌ Don't Ignore Exceptions

```python
# Bad
def get_note(note_id: str):
    try:
        return store.get_note(note_id)
    except:
        return None

# Good
def get_note(note_id: str):
    try:
        return store.get_note(note_id)
    except FileNotFoundError:
        raise NoteNotFoundError(f"Note {note_id} not found")
```

### ❌ Don't Use Print Statements

```python
# Bad
print("Creating note...")

# Good
logger.info("Creating note...")
```

### ❌ Don't Repeat Code

```python
# Bad
def get_note(note_id: str):
    filepath = vault_path / f"{note_id}.md"
    post = frontmatter.load(filepath)
    return Note(...)

def update_note(note_id: str, data: NoteUpdate):
    filepath = vault_path / f"{note_id}.md"
    post = frontmatter.load(filepath)
    # ...

# Good - extract common logic
def _get_filepath(note_id: str) -> Path:
    return vault_path / f"{note_id}.md"

def _load_note(filepath: Path) -> Note:
    post = frontmatter.load(filepath)
    return Note(...)
```

## Resources

- [FastAPI Best Practices](https://fastapi.tiangolo.com/tutorial/)
- [Pydantic Documentation](https://docs.pydantic.dev/)
- [Python Logging](https://docs.python.org/3/library/logging.html)
- [Pytest Documentation](https://docs.pytest.org/)

---

**See Also:**
- [Coding Standards](./coding-standards.md)
- [Frontend Patterns](./frontend-patterns.md)
- [Testing Patterns](./testing-patterns.md)
