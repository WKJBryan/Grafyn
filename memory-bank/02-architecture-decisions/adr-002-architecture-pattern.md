# ADR-002: Service Layer Architecture Pattern

## Status
Accepted

## Date
2024-11-05

## Context

Grafyn backend needs to handle multiple concerns:

1. **Note Management**: CRUD operations with Markdown/YAML parsing
2. **Semantic Search**: Vector embeddings and similarity search
3. **Knowledge Graph**: Wikilink parsing and relationship tracking
4. **Embedding Generation**: Text to vector encoding

We need an architecture that:

- Separates concerns clearly
- Makes code testable and maintainable
- Allows independent evolution of services
- Provides clear boundaries between layers
- Supports future feature additions

## Decision

We adopted a **Service Layer Architecture Pattern** with clear separation of concerns:

### Architecture Layers

```
┌──────────────────────────────────────────────────────────────┐
│                      API Layer (11 Routers)                 │
│  notes(12) search(2) graph(7) canvas(22) mcp_write(9)      │
│  distill(2) priority(7) import(7) zettel(6) feedback(2)     │
│  memory(3) oauth(4)                                         │
├──────────────────────────────────────────────────────────────┤
│                     Service Layer                            │
│  ┌────────────────┐ ┌─────────────────┐ ┌─────────────────┐  │
│  │ KnowledgeStore │ │ VectorSearch    │ │ GraphIndex      │  │
│  │ (Markdown I/O) │ │ (LanceDB)       │ │ (Link tracking) │  │
│  └────────────────┘ └─────────────────┘ └─────────────────┘  │
│            │                 │                               │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │              EmbeddingService                            │ │
│  │              (sentence-transformers)                     │ │
│  └─────────────────────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────────────┤
│                     Data Layer                              │
│  ┌────────────────┐ ┌─────────────────┐ ┌─────────────────┐  │
│  │ vault/*.md     │ │ data/lancedb/   │ │ In-memory graph │  │
│  │ (Markdown)     │ │ (Vectors)       │ │ (Adjacency)     │  │
│  └────────────────┘ └─────────────────┘ └─────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

### Service Responsibilities

#### 1. KnowledgeStore Service
**File:** `services/knowledge_store.py`

**Responsibilities:**
- Markdown file I/O operations
- YAML frontmatter parsing
- Wikilink extraction
- Note CRUD operations

**Key Methods:**
```python
class KnowledgeStore:
    def list_notes() -> List[NoteListItem]
    def get_note(id: str) -> Note
    def create_note(data: NoteCreate) -> Note
    def update_note(id: str, data: NoteUpdate) -> Note
    def delete_note(id: str) -> None
    def extract_wikilinks(content: str) -> List[str]
    def get_all_content() -> List[Tuple[str, str, str]]
```

#### 2. VectorSearch Service
**File:** `services/vector_search.py`

**Responsibilities:**
- LanceDB connection management
- Note indexing (single and batch)
- Semantic similarity search
- Vector storage and retrieval

**Key Methods:**
```python
class VectorSearchService:
    def index_note(id: str, title: str, content: str) -> None
    def index_all(notes: List[Tuple]) -> None
    def search(query: str, limit: int) -> List[SearchResult]
    def delete_note(id: str) -> None
```

#### 3. GraphIndex Service
**File:** `services/graph_index.py`

**Responsibilities:**
- Wikilink parsing and tracking
- Outgoing link management
- Backlink computation
- Graph traversal and neighbor discovery
- Unlinked mention detection

**Key Methods:**
```python
class GraphIndexService:
    def build_index() -> None
    def get_outgoing_links(id: str) -> List[str]
    def get_backlinks(id: str) -> List[str]
    def get_backlinks_with_context(id: str) -> List[BacklinkInfo]
    def get_neighbors(id: str, depth: int) -> Dict[str, List[str]]
    def find_unlinked_mentions(id: str) -> List[Dict]
    def update_note(id: str, old_content: str, new_content: str) -> None
```

#### 4. Embedding Service
**File:** `services/embedding.py`

**Responsibilities:**
- Model loading and caching
- Text to vector encoding
- Batch encoding
- Dimension management

**Key Methods:**
```python
class EmbeddingService:
    def encode(text: str) -> np.ndarray
    def encode_batch(texts: List[str]) -> List[np.ndarray]
    @property
    def dimension(self) -> int
```

### Router Layer

**Files:** `routers/notes.py`, `routers/search.py`, `routers/graph.py`, `routers/canvas.py`, `routers/mcp_write.py`, `routers/distill.py`, `routers/priority.py`, `routers/conversation_import.py`, `routers/zettelkasten.py`, `routers/feedback.py`, `routers/memory.py`, `routers/oauth.py`

**Responsibilities:**
- HTTP request/response handling
- Input validation via Pydantic
- Service orchestration
- Error handling
- Response formatting

**Example:**
```python
@router.get("/{note_id}")
async def get_note(note_id: str):
    note = knowledge_store.get_note(note_id)
    backlinks = graph_index.get_backlinks(note_id)
    note.backlinks = backlinks
    return note
```

## Consequences

### Positive

- **Clear Separation**: Each service has a single, well-defined responsibility
- **Testability**: Services can be tested independently
- **Maintainability**: Easy to locate and modify specific functionality
- **Reusability**: Services can be used by multiple routers
- **Scalability**: Services can be optimized or replaced independently
- **Mockability**: Easy to mock services for testing
- **Evolution**: New services can be added without affecting existing ones

### Negative

- **Indirection**: Additional layer of abstraction adds complexity
- **Overhead**: More code than simple monolithic approach
- **Learning Curve**: Developers must understand service layer pattern
- **Boilerplate**: Some repetitive code in service methods

### Trade-offs

| Aspect | Benefit | Trade-off |
|--------|---------|-----------|
| Service Layer | Clear separation, testable | More code, indirection |
| In-Memory Graph | Fast, simple | Not persistent, rebuilds on restart |
| File-Based Notes | Simple, portable | Not ACID, manual consistency |

## Alternatives Considered

### Monolithic Architecture
**Rejected because:**
- Difficult to test individual components
- Tight coupling between concerns
- Hard to maintain as codebase grows
- No clear boundaries between features

### Microservices Architecture
**Rejected because:**
- Overkill for current scale
- Adds deployment complexity
- Requires service discovery and orchestration
- Network overhead between services
- Violates local-first principle

### Repository Pattern
**Rejected because:**
- More complex than needed
- Services already provide abstraction
- Additional layer of indirection
- Not using ORM (file-based storage)

### CQRS (Command Query Responsibility Segregation)
**Rejected because:**
- Overkill for current requirements
- Adds complexity without clear benefit
- Read and write paths are similar
- Not optimizing for high-throughput scenarios

## Implementation Guidelines

### Service Design Principles

1. **Single Responsibility**: Each service does one thing well
2. **Dependency Injection**: Services receive dependencies, don't create them
3. **Stateless**: Services should be stateless where possible
4. **Error Handling**: Services raise exceptions, routers handle HTTP responses
5. **Logging**: Services log operations for debugging

### Service Initialization

```python
# In main.py — services initialized via lifespan, attached to app.state
# 14+ services including:
# KnowledgeStore, VectorSearchService, GraphIndexService, EmbeddingService,
# OpenRouterService, CanvasSessionStore, DistillationService, ImportService,
# LinkDiscoveryService, PriorityScoringService, PrioritySettingsService,
# TokenStore, FeedbackService, MemoryService
# Access via dependency helpers in app/utils/dependencies.py
```

### Service Dependencies

```
KnowledgeStore (independent)
    ↓
EmbeddingService (independent)
    ↓
VectorSearchService (depends on EmbeddingService)
    ↓
GraphIndexService (depends on KnowledgeStore)
```

### Error Handling Pattern

```python
# Service raises exceptions
class NoteNotFoundError(Exception):
    pass

def get_note(id: str) -> Note:
    if not self._note_exists(id):
        raise NoteNotFoundError(f"Note {id} not found")
    return self._load_note(id)

# Router handles HTTP response
@router.get("/{note_id}")
async def get_note(note_id: str):
    try:
        note = knowledge_store.get_note(note_id)
        return note
    except NoteNotFoundError:
        raise HTTPException(status_code=404, detail="Note not found")
```

## Testing Strategy

### Unit Tests
- Test each service independently
- Mock dependencies
- Test edge cases and error conditions

### Integration Tests
- Test service interactions
- Test end-to-end workflows
- Use real file system and database

### Example Test Structure

```python
# tests/unit/test_knowledge_store.py
def test_get_note_existing():
    store = KnowledgeStore(temp_dir)
    note = store.create_note(NoteCreate(...))
    retrieved = store.get_note(note.id)
    assert retrieved.id == note.id

# tests/integration/test_api_notes.py
def test_get_note_endpoint():
    response = client.get(f"/api/notes/{note_id}")
    assert response.status_code == 200
    assert response.json()["id"] == note_id
```

## Future Considerations

### Potential Enhancements

1. **Service Interface**: Define abstract interfaces for services
2. **Dependency Injection**: Use a DI framework (e.g., dependency-injector)
3. **Service Discovery**: For distributed deployment (if needed)
4. **Service Monitoring**: Add metrics and health checks
5. **Service Versioning**: Version services for backward compatibility

### Scaling Considerations

- **Horizontal Scaling**: Stateless services can be scaled horizontally
- **Caching**: Add caching layer to services
- **Async Operations**: Convert services to async for better performance
- **Queue-Based**: Use message queues for long-running operations

## References

- [Service Layer Pattern](https://martinfowler.com/eaaCatalog/serviceLayer.html)
- [Clean Architecture](https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html)
- [Architecture - Backend](../../docs/architecture-backend.md)
- [ADR-001: Technology Stack](./adr-001-technology-stack.md)
- [Development Guide - Backend](../../docs/development-guide-backend.md)

## Related Decisions

- [ADR-001: Technology Stack](./adr-001-technology-stack.md) - Underlying technologies
- [ADR-003: MCP Integration](./adr-003-mcp-integration.md) - How MCP fits into architecture
- [ADR-004: Data Model](./adr-004-data-model.md) - Data structures used by services

---

**Status:** This decision is active and forms the core of Grafyn backend architecture.
