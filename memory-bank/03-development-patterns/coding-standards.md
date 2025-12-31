# Coding Standards and Conventions

> **Purpose:** Define coding standards and conventions for OrgAI project
> **Created:** 2025-12-31
> **Status:** Active

## Overview

This document defines coding standards and conventions for the OrgAI project to ensure consistency, readability, and maintainability across the codebase.

## General Principles

1. **Readability First**: Code should be easy to read and understand
2. **Consistency**: Follow established patterns throughout the codebase
3. **Simplicity**: Avoid unnecessary complexity
4. **Documentation**: Document non-obvious code
5. **Testing**: Write tests for all new functionality
6. **Type Safety**: Use type hints where appropriate

## Python (Backend) Standards

### Code Style

**Follow PEP 8** with these specifics:

```python
# Good
class KnowledgeStore:
    def __init__(self, vault_path: str):
        self.vault_path = Path(vault_path)
        self._cache: Dict[str, Note] = {}

# Bad
class knowledgestore:
    def __init__(self, vaultPath):
        self.vaultPath=vaultPath
        self.cache={}
```

### Type Hints

**Always use type hints** for function signatures:

```python
# Good
def get_note(note_id: str) -> Note:
    """Retrieve a note by ID."""
    pass

def search_notes(query: str, limit: int = 10) -> List[SearchResult]:
    """Search notes by query."""
    pass

# Bad
def get_note(note_id):
    pass

def search_notes(query, limit=10):
    pass
```

### Docstrings

**Use Google-style docstrings**:

```python
def get_note(note_id: str) -> Note:
    """Retrieve a note by ID.
    
    Args:
        note_id: The note identifier (filename without .md)
    
    Returns:
        The complete note with content and metadata
    
    Raises:
        NoteNotFoundError: If note doesn't exist
    """
    pass
```

### Naming Conventions

| Type | Convention | Example |
|-------|------------|----------|
| Classes | PascalCase | `KnowledgeStore`, `VectorSearchService` |
| Functions | snake_case | `get_note()`, `search_notes()` |
| Variables | snake_case | `note_id`, `search_results` |
| Constants | UPPER_SNAKE_CASE | `MAX_RESULTS`, `DEFAULT_TIMEOUT` |
| Private members | _leading_underscore | `_cache`, `_load_note()` |
| Modules | snake_case | `knowledge_store.py`, `vector_search.py` |

### Imports

**Organize imports** in this order:

```python
# 1. Standard library
import os
import re
from pathlib import Path
from typing import List, Dict, Optional

# 2. Third-party
import frontmatter
import lancedb
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

# 3. Local imports
from app.config import get_settings
from app.models.note import Note, NoteCreate
from app.services.knowledge_store import KnowledgeStore
```

### Error Handling

**Use specific exceptions**:

```python
# Good
class NoteNotFoundError(Exception):
    """Raised when a note doesn't exist."""
    pass

def get_note(note_id: str) -> Note:
    if not self._note_exists(note_id):
        raise NoteNotFoundError(f"Note {note_id} not found")
    return self._load_note(note_id)

# Bad
def get_note(note_id: str) -> Note:
    if not self._note_exists(note_id):
        raise Exception("Not found")
    return self._load_note(note_id)
```

### Logging

**Use structured logging**:

```python
import logging

logger = logging.getLogger(__name__)

def get_note(note_id: str) -> Note:
    logger.info(f"Retrieving note: {note_id}")
    try:
        note = self._load_note(note_id)
        logger.debug(f"Note loaded: {note.title}")
        return note
    except FileNotFoundError as e:
        logger.error(f"Note not found: {note_id}", exc_info=True)
        raise NoteNotFoundError(f"Note {note_id} not found") from e
```

### Async/Await

**Use async for I/O operations**:

```python
# Good
@router.get("/{note_id}")
async def get_note(note_id: str):
    note = knowledge_store.get_note(note_id)
    return note

# Bad (unnecessary async)
@router.get("/{note_id}")
async def get_note(note_id: str):
    note = await knowledge_store.get_note(note_id)  # Not async
    return note
```

## JavaScript/Vue 3 (Frontend) Standards

### Code Style

**Follow Vue 3 Composition API patterns**:

```javascript
// Good
import { ref, computed, onMounted } from 'vue'

export default {
  name: 'NoteEditor',
  props: {
    noteId: String
  },
  setup(props) {
    const content = ref('')
    const isDirty = computed(() => content.value !== originalContent.value)
    
    onMounted(() => {
      loadNote(props.noteId)
    })
    
    return { content, isDirty }
  }
}

// Bad (Options API)
export default {
  name: 'NoteEditor',
  props: ['noteId'],
  data() {
    return {
      content: ''
    }
  },
  mounted() {
    this.loadNote(this.noteId)
  }
}
```

### Naming Conventions

| Type | Convention | Example |
|-------|------------|----------|
| Components | PascalCase | `NoteEditor.vue`, `SearchBar.vue` |
| Functions | camelCase | `loadNote()`, `searchNotes()` |
| Variables | camelCase | `noteId`, `searchResults` |
| Constants | UPPER_SNAKE_CASE | `MAX_RESULTS`, `API_URL` |
| CSS Classes | kebab-case | `.note-editor`, `.search-bar` |

### Component Structure

**Organize components** with this template:

```vue
<template>
  <!-- Template content -->
</template>

<script>
import { ref, computed, onMounted } from 'vue'

export default {
  name: 'ComponentName',
  props: {
    /* props */
  },
  emits: ['event-name'],
  setup(props, { emit }) {
    // State
    const state = ref(null)
    
    // Computed
    const computedValue = computed(() => state.value)
    
    // Methods
    const method = () => {
      // implementation
    }
    
    // Lifecycle
    onMounted(() => {
      // initialization
    })
    
    return {
      state,
      computedValue,
      method
    }
  }
}
</script>

<style scoped>
/* Component styles */
</style>
```

### API Calls

**Use the centralized API client**:

```javascript
// Good
import { notes } from '@/api/client'

export default {
  setup() {
    const loadNote = async (id) => {
      try {
        const note = await notes.get(id)
        return note
      } catch (error) {
        console.error('Failed to load note:', error)
        throw error
      }
    }
    
    return { loadNote }
  }
}

// Bad (direct fetch)
export default {
  setup() {
    const loadNote = async (id) => {
      const response = await fetch(`/api/notes/${id}`)
      const note = await response.json()
      return note
    }
    
    return { loadNote }
  }
}
```

### Reactivity

**Use ref and computed** appropriately:

```javascript
// Good
import { ref, computed } from 'vue'

const count = ref(0)
const doubled = computed(() => count.value * 2)

// Bad (reactive object when ref is sufficient)
const state = reactive({ count: 0 })
const doubled = computed(() => state.count * 2)
```

## File Organization

### Backend Structure

```
backend/app/
├── __init__.py
├── main.py              # Application entry point
├── config.py            # Configuration
├── logging_config.py    # Logging setup
├── routers/             # API endpoints
│   ├── __init__.py
│   ├── notes.py
│   ├── search.py
│   └── graph.py
├── services/            # Business logic
│   ├── __init__.py
│   ├── knowledge_store.py
│   ├── vector_search.py
│   ├── graph_index.py
│   └── embedding.py
├── models/              # Pydantic schemas
│   ├── __init__.py
│   └── note.py
└── mcp/                # MCP integration
    ├── __init__.py
    ├── server.py
    └── tools.py
```

### Frontend Structure

```
frontend/src/
├── main.js             # Application entry
├── App.vue             # Root component
├── style.css           # Global styles
├── api/                # API client
│   └── client.js
└── components/          # Vue components
    ├── SearchBar.vue
    ├── NoteList.vue
    ├── NoteEditor.vue
    ├── BacklinksPanel.vue
    └── GraphView.vue
```

## Testing Standards

### Python Tests

**Use pytest** with these conventions:

```python
# tests/unit/test_knowledge_store.py
import pytest
from app.services.knowledge_store import KnowledgeStore

def test_get_note_existing(tmp_path):
    """Test retrieving an existing note."""
    store = KnowledgeStore(tmp_path)
    note = store.create_note(NoteCreate(
        title="Test",
        content="Content"
    ))
    
    retrieved = store.get_note(note.id)
    assert retrieved.id == note.id
    assert retrieved.title == "Test"

def test_get_note_nonexistent(tmp_path):
    """Test retrieving a nonexistent note."""
    store = KnowledgeStore(tmp_path)
    with pytest.raises(NoteNotFoundError):
        store.get_note("nonexistent")
```

### Vue Component Tests

**Use Vitest** with @vue/test-utils:

```javascript
// tests/components/NoteEditor.spec.js
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
  })
})
```

## Git Commit Messages

**Follow Conventional Commits**:

```
<type>(<scope>): <subject>

<body>

<footer>
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting)
- `refactor`: Code refactoring
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

**Examples:**

```
feat(notes): add note deletion endpoint

Implement DELETE /api/notes/{id} endpoint with proper
error handling and validation.

Closes #42
```

```
fix(search): handle empty query gracefully

Return empty array instead of error when query is empty.

Fixes #15
```

## Code Review Checklist

Before submitting code for review:

- [ ] Code follows style guidelines
- [ ] Type hints are present (Python)
- [ ] Docstrings are present for public functions
- [ ] Tests are added/updated
- [ ] Tests pass locally
- [ ] No console.log or print statements
- [ ] Error handling is appropriate
- [ ] Logging is added where needed
- [ ] Commit message follows conventions
- [ ] Documentation is updated if needed

## Resources

### Python
- [PEP 8 Style Guide](https://peps.python.org/pep-0008/)
- [Google Python Style Guide](https://google.github.io/styleguide/pyguide.html)
- [FastAPI Best Practices](https://fastapi.tiangolo.com/tutorial/)

### Vue 3
- [Vue 3 Style Guide](https://vuejs.org/style-guide/)
- [Composition API RFC](https://github.com/vuejs/rfcs/blob/master/active-rfcs/0013-composition-api.md)
- [Vite Guide](https://vitejs.dev/guide/)

### Testing
- [Pytest Documentation](https://docs.pytest.org/)
- [Vitest Documentation](https://vitest.dev/)
- [@vue/test-utils](https://test-utils.vuejs.org/)

---

**See Also:**
- [Backend Patterns](./backend-patterns.md)
- [Frontend Patterns](./frontend-patterns.md)
- [Testing Patterns](./testing-patterns.md)
