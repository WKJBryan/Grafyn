# Testing Patterns

> **Purpose:** Document testing patterns and best practices for OrgAI project
> **Created:** 2025-12-31
> **Status:** Active

## Overview

This document describes testing patterns and conventions used in OrgAI project to ensure code quality and reliability.

## Test Organization

### Directory Structure

```
backend/tests/
├── conftest.py              # Shared fixtures
├── unit/                     # Unit tests
│   ├── test_knowledge_store.py
│   ├── test_embedding.py
│   ├── test_graph_index.py
│   └── test_vector_search.py
└── integration/               # Integration tests
    └── test_api_notes.py

frontend/tests/
├── unit/                     # Component tests
│   └── components/
│       ├── NoteEditor.spec.js
│       ├── SearchBar.spec.js
│       └── NoteList.spec.js
└── e2e/                      # End-to-end tests
    └── user-flows.spec.js
```

## Backend Unit Testing Patterns

### Pattern: Service Testing

Test services in isolation with mocked dependencies.

```python
# tests/unit/test_knowledge_store.py
import pytest
from pathlib import Path
from app.services.knowledge_store import KnowledgeStore
from app.models.note import NoteCreate

def test_create_note_success(tmp_path: Path):
    """Test creating a note successfully."""
    store = KnowledgeStore(str(tmp_path))
    
    note = store.create_note(NoteCreate(
        title="Test Note",
        content="Test content"
    ))
    
    assert note.id == "Test_Note"
    assert note.title == "Test Note"
    assert note.content == "Test content"
    assert note.frontmatter.status == "draft"
    assert len(note.outgoing_links) == 0

def test_create_note_with_wikilinks(tmp_path: Path):
    """Test creating a note with wikilinks."""
    store = KnowledgeStore(str(tmp_path))
    
    note = store.create_note(NoteCreate(
        title="Test Note",
        content="Link to [[Other Note]] and [[Another Note|Display]]"
    ))
    
    assert len(note.outgoing_links) == 2
    assert "Other Note" in note.outgoing_links
    assert "Another Note" in note.outgoing_links

def test_create_duplicate_note(tmp_path: Path):
    """Test creating a duplicate note raises error."""
    store = KnowledgeStore(str(tmp_path))
    
    store.create_note(NoteCreate(
        title="Test Note",
        content="Content"
    ))
    
    with pytest.raises(ValueError, match="already exists"):
        store.create_note(NoteCreate(
            title="Test Note",
            content="Different content"
        ))
```

### Pattern: Edge Case Testing

Test edge cases and boundary conditions.

```python
def test_get_note_nonexistent(tmp_path: Path):
    """Test retrieving a nonexistent note."""
    store = KnowledgeStore(str(tmp_path))
    
    with pytest.raises(FileNotFoundError):
        store.get_note("nonexistent")

def test_extract_wikilinks_edge_cases():
    """Test wikilink extraction with edge cases."""
    from app.services.knowledge_store import KnowledgeStore
    
    # Empty content
    assert KnowledgeStore.extract_wikilinks("") == []
    
    # No wikilinks
    assert KnowledgeStore.extract_wikilinks("Just plain text") == []
    
    # Multiple wikilinks
    content = "[[A]] and [[B]] and [[C]]"
    links = KnowledgeStore.extract_wikilinks(content)
    assert links == ["A", "B", "C"]
    
    # Wikilinks with aliases
    content = "[[Note|Display Text]]"
    links = KnowledgeStore.extract_wikilinks(content)
    assert links == ["Note"]
    
    # Duplicate wikilinks
    content = "[[A]] and [[A]]"
    links = KnowledgeStore.extract_wikilinks(content)
    assert links == ["A", "A"]  # Keep duplicates
```

### Pattern: Parameterized Testing

Test multiple inputs with same test logic.

```python
import pytest

@pytest.mark.parametrize("title,expected_id", [
    ("Simple", "Simple"),
    ("Two Words", "Two_Words"),
    ("With-Special!", "WithSpecial"),
    ("Multiple   Spaces", "Multiple_Spaces"),
])
def test_generate_id(title: str, expected_id: str):
    """Test ID generation from various titles."""
    from app.services.knowledge_store import KnowledgeStore
    
    assert KnowledgeStore._generate_id(title) == expected_id

@pytest.mark.parametrize("status", ["draft", "evidence", "canonical"])
def test_create_note_with_status(tmp_path: Path, status: str):
    """Test creating notes with different statuses."""
    store = KnowledgeStore(str(tmp_path))
    
    note = store.create_note(NoteCreate(
        title="Test",
        content="Content",
        status=status
    ))
    
    assert note.frontmatter.status == status
```

## Backend Integration Testing Patterns

### Pattern: API Endpoint Testing

Test API endpoints with TestClient.

```python
# tests/integration/test_api_notes.py
import pytest
from fastapi.testclient import TestClient
from app.main import app

client = TestClient(app)

def test_list_notes_empty():
    """Test listing notes when vault is empty."""
    response = client.get("/api/notes")
    
    assert response.status_code == 200
    assert response.json() == []

def test_list_notes_with_notes(tmp_vault):
    """Test listing notes with existing notes."""
    # Create test notes
    # ...
    
    response = client.get("/api/notes")
    
    assert response.status_code == 200
    notes = response.json()
    assert len(notes) == 2
    assert notes[0]["title"] == "First Note"

def test_get_note_success(tmp_vault):
    """Test getting an existing note."""
    response = client.get("/api/notes/Test_Note")
    
    assert response.status_code == 200
    note = response.json()
    assert note["id"] == "Test_Note"
    assert note["title"] == "Test Note"

def test_get_note_not_found():
    """Test getting a nonexistent note."""
    response = client.get("/api/notes/Nonexistent")
    
    assert response.status_code == 404
    assert "not found" in response.json()["detail"].lower()

def test_create_note_success():
    """Test creating a new note."""
    response = client.post(
        "/api/notes",
        json={
            "title": "New Note",
            "content": "New content",
            "tags": ["test"]
        }
    )
    
    assert response.status_code == 201
    note = response.json()
    assert note["id"] == "New_Note"
    assert note["title"] == "New Note"
```

### Pattern: Workflow Testing

Test complete user workflows.

```python
def test_note_lifecycle(tmp_vault):
    """Test complete note lifecycle: create, update, delete."""
    # Create
    create_response = client.post(
        "/api/notes",
        json={
            "title": "Lifecycle Test",
            "content": "Initial content"
        }
    )
    assert create_response.status_code == 201
    note_id = create_response.json()["id"]
    
    # Read
    get_response = client.get(f"/api/notes/{note_id}")
    assert get_response.status_code == 200
    note = get_response.json()
    assert note["content"] == "Initial content"
    
    # Update
    update_response = client.put(
        f"/api/notes/{note_id}",
        json={
            "content": "Updated content",
            "status": "canonical"
        }
    )
    assert update_response.status_code == 200
    updated_note = update_response.json()
    assert updated_note["content"] == "Updated content"
    assert updated_note["frontmatter"]["status"] == "canonical"
    
    # Delete
    delete_response = client.delete(f"/api/notes/{note_id}")
    assert delete_response.status_code == 204
    
    # Verify deleted
    get_response = client.get(f"/api/notes/{note_id}")
    assert get_response.status_code == 404
```

## Frontend Component Testing Patterns

### Pattern: Component Rendering

Test component renders correctly.

```javascript
// tests/unit/components/NoteEditor.spec.js
import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import NoteEditor from '@/components/NoteEditor.vue'

describe('NoteEditor', () => {
  it('renders note content', () => {
    const wrapper = mount(NoteEditor, {
      props: {
        noteId: 'test',
        note: {
          id: 'test',
          title: 'Test Note',
          content: 'Test content'
        }
      }
    })
    
    expect(wrapper.text()).toContain('Test content')
    expect(wrapper.find('input').element.value).toBe('Test Note')
  })
  
  it('shows save button when dirty', async () => {
    const wrapper = mount(NoteEditor, {
      props: {
        noteId: 'test',
        note: {
          id: 'test',
          title: 'Test',
          content: 'Content'
        }
      }
    })
    
    const saveButton = wrapper.find('button')
    expect(saveButton.attributes('disabled')).toBeDefined()
    
    // Make dirty
    await wrapper.find('textarea').setValue('New content')
    expect(saveButton.attributes('disabled')).toBeUndefined()
  })
})
```

### Pattern: User Interaction

Test user interactions and events.

```javascript
describe('NoteEditor', () => {
  it('emits save event on save button click', async () => {
    const wrapper = mount(NoteEditor, {
      props: {
        noteId: 'test',
        note: {
          id: 'test',
          title: 'Test',
          content: 'Content'
        }
      }
    })
    
    // Make dirty
    await wrapper.find('textarea').setValue('New content')
    
    // Click save
    await wrapper.find('button').trigger('click')
    
    // Check emit
    expect(wrapper.emitted('save')).toBeTruthy()
    expect(wrapper.emitted('save')[0]).toEqual([
      'test',
      {
        title: 'Test',
        content: 'New content'
      }
    ])
  })
  
  it('emits cancel event on cancel button click', async () => {
    const wrapper = mount(NoteEditor, {
      props: {
        noteId: 'test',
        note: {
          id: 'test',
          title: 'Test',
          content: 'Content'
        }
      }
    })
    
    await wrapper.findAll('button')[1].trigger('click')
    
    expect(wrapper.emitted('cancel')).toBeTruthy()
  })
})
```

### Pattern: Computed Properties

Test computed properties.

```javascript
describe('NoteList', () => {
  it('filters notes by search query', async () => {
    const wrapper = mount(NoteList, {
      props: {
        notes: [
          { id: '1', title: 'Python Guide', tags: [] },
          { id: '2', title: 'JavaScript Guide', tags: [] },
          { id: '3', title: 'Python Tips', tags: [] }
        ],
        searchQuery: 'Python'
      }
    })
    
    const items = wrapper.findAll('.note-item')
    expect(items.length).toBe(2)
    expect(items[0].text()).toContain('Python Guide')
    expect(items[1].text()).toContain('Python Tips')
  })
  
  it('sorts notes by date', async () => {
    const wrapper = mount(NoteList, {
      props: {
        notes: [
          { id: '1', title: 'Old', modified: '2024-01-01' },
          { id: '2', title: 'New', modified: '2024-12-01' }
        ],
        sortBy: 'modified'
      }
    })
    
    const items = wrapper.findAll('.note-item')
    expect(items[0].text()).toContain('New')
    expect(items[1].text()).toContain('Old')
  })
})
```

## Fixture Patterns

### Pattern: Shared Fixtures

Use fixtures for common test setup.

```python
# tests/conftest.py
import pytest
from pathlib import Path
from app.services.knowledge_store import KnowledgeStore
from app.services.vector_search import VectorSearchService
from app.services.graph_index import GraphIndexService

@pytest.fixture
def tmp_path(tmp_path_factory):
    """Create temporary directory for tests."""
    return tmp_path_factory.mktemp()

@pytest.fixture
def temp_vault(tmp_path: Path) -> Path:
    """Create temporary vault directory."""
    vault = tmp_path / "vault"
    vault.mkdir()
    return vault

@pytest.fixture
def temp_data(tmp_path: Path) -> Path:
    """Create temporary data directory."""
    data = tmp_path / "data"
    data.mkdir()
    return data

@pytest.fixture
def knowledge_store(temp_vault: Path) -> KnowledgeStore:
    """Create KnowledgeStore with temporary vault."""
    return KnowledgeStore(str(temp_vault))

@pytest.fixture
def vector_search_service(temp_data: Path) -> VectorSearchService:
    """Create VectorSearchService with temporary data."""
    # Mock embedding service for tests
    from app.services.embedding import EmbeddingService
    embedding = EmbeddingService()
    return VectorSearchService(str(temp_data), embedding)

@pytest.fixture
def sample_note():
    """Provide sample note data."""
    return {
        "title": "Sample Note",
        "content": "Sample content with [[link]].",
        "tags": ["test", "sample"]
    }
```

## Mocking Patterns

### Pattern: Mock External Services

Mock external dependencies.

```python
from unittest.mock import Mock, patch

def test_search_with_mocked_embedding():
    """Test search with mocked embedding service."""
    with patch('app.services.vector_search.EmbeddingService') as mock_embedding:
        # Setup mock
        mock_service = Mock()
        mock_service.encode.return_value = [0.1, 0.2, 0.3]
        mock_embedding.return_value = mock_service
        
        # Test
        service = VectorSearchService("/tmp/data", mock_service)
        results = service.search("test query")
        
        # Verify
        mock_service.encode.assert_called_once_with("test query")
        assert len(results) >= 0
```

## Test Data Patterns

### Pattern: Test Factories

Create test data programmatically.

```python
# tests/factories.py
from app.models.note import NoteCreate, NoteFrontmatter
from datetime import datetime

def create_note_data(**overrides):
    """Create note data with defaults."""
    defaults = {
        "title": "Test Note",
        "content": "Test content",
        "tags": ["test"],
        "status": "draft"
    }
    defaults.update(overrides)
    return NoteCreate(**defaults)

def create_note_with_links(link_count: int = 3):
    """Create note with specified number of links."""
    links = " ".join([f"[[Link{i}]]" for i in range(link_count)])
    return create_note_data(
        title="Linked Note",
        content=f"Content with {links}"
    )

# Usage in tests
def test_note_with_many_links():
    note_data = create_note_with_links(5)
    assert len(note_data.tags) == 1
    assert "[[Link0]]" in note_data.content
```

## Test Organization Best Practices

### Naming Conventions

```python
# Good names
def test_create_note_success()
def test_create_note_with_wikilinks()
def test_create_duplicate_note_raises_error()
def test_search_returns_relevant_results()

# Bad names
def test1()
def test_create()
def check_search()
```

### Test Structure

```python
def test_feature_scenario_expected_result():
    """
    Test that feature does X when Y.
    
    Given: Precondition
    When: Action
    Then: Expected result
    """
    # Arrange
    setup_data()
    
    # Act
    result = perform_action()
    
    # Assert
    assert result == expected
```

## Running Tests

### Backend Tests

```bash
# Run all tests
pytest

# Run with coverage
pytest --cov=app --cov-report=html --cov-report=term-missing

# Run specific test file
pytest tests/unit/test_knowledge_store.py

# Run specific test
pytest tests/unit/test_knowledge_store.py::test_create_note_success

# Run with markers
pytest -m unit
pytest -m integration
pytest -m "not slow"

# Run with verbose output
pytest -v

# Run with detailed output
pytest -vv
```

### Frontend Tests

```bash
# Run all tests
npm test

# Run with coverage
npm run test:coverage

# Run specific test file
npm test NoteEditor.spec.js

# Run in watch mode
npm run test:watch

# Run with UI
npm run test:ui
```

## Test Coverage Goals

| Component | Target | Current |
|-----------|---------|----------|
| Backend Services | 80% | 70%+ |
| API Endpoints | 80% | 80%+ |
| Frontend Components | 70% | 0% |
| E2E Flows | 60% | 0% |

## Common Testing Pitfalls

### ❌ Testing Implementation Details

```python
# Bad - tests internal implementation
def test_create_note(tmp_path):
    store = KnowledgeStore(str(tmp_path))
    note = store.create_note(...)
    assert store._cache[note.id] is not None  # Tests internal

# Good - tests public API
def test_create_note(tmp_path):
    store = KnowledgeStore(str(tmp_path))
    note = store.create_note(...)
    retrieved = store.get_note(note.id)
    assert retrieved.id == note.id  # Tests behavior
```

### ❌ Brittle Tests

```javascript
// Bad - depends on exact HTML structure
expect(wrapper.find('div > div > span').text()).toBe('text')

// Good - uses semantic selectors
expect(wrapper.find('.note-title').text()).toBe('text')
```

### ❌ Ignoring Async

```python
# Bad - doesn't wait for async
response = client.post("/api/notes", json=data)
assert response.status_code == 201

# Good - handles async properly
@pytest.mark.asyncio
async def test_create_note():
    response = await client.post("/api/notes", json=data)
    assert response.status_code == 201
```

## Resources

- [Pytest Documentation](https://docs.pytest.org/)
- [Vitest Documentation](https://vitest.dev/)
- [@vue/test-utils](https://test-utils.vuejs.org/)
- [Testing Best Practices](https://testingjavascript.com/)

---

**See Also:**
- [Coding Standards](./coding-standards.md)
- [Backend Patterns](./backend-patterns.md)
- [Frontend Patterns](./frontend-patterns.md)
