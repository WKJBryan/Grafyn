# ADR-001: Technology Stack Selection

## Status
Accepted

## Date
2024-11-01

## Context

OrgAI needs a technology stack that supports:

1. **Local-first deployment**: Self-hosted without cloud dependencies
2. **Semantic search**: Vector embeddings and similarity search
3. **Knowledge graph**: Wikilink parsing and relationship tracking
4. **AI integration**: MCP protocol for external AI models
5. **Web interface**: Accessible from any device with a browser
6. **Developer experience**: Easy to understand, extend, and maintain

The project requires both backend (API, search, storage) and frontend (UI, visualization) components.

## Decision

We selected the following technology stack:

### Backend
- **Framework**: FastAPI 0.104+
- **Language**: Python 3.10+
- **Vector Database**: LanceDB 0.3+
- **Embeddings**: sentence-transformers 2.2+
- **Data Validation**: Pydantic 2.5+
- **MCP Integration**: fastapi-mcp 0.1+

### Frontend
- **Framework**: Vue 3.4+ (Composition API)
- **Build Tool**: Vite 5.0+
- **HTTP Client**: Axios 1.6+
- **Markdown**: marked 11.0+
- **Graph Visualization**: D3.js v7

### Storage
- **Notes**: Markdown files in `vault/` directory
- **Vectors**: LanceDB in `data/` directory
- **Graph Index**: In-memory adjacency lists

## Consequences

### Positive

- **Modern and Performant**: FastAPI and Vue 3 are modern frameworks with excellent performance
- **Type Safety**: Pydantic provides runtime validation, Vue 3 supports TypeScript
- **Developer Experience**: FastAPI auto-generates OpenAPI docs, Vue 3 has excellent tooling
- **Local-First**: No external database servers required, LanceDB is file-based
- **AI-Native**: sentence-transformers provides state-of-the-art embeddings
- **Standard Formats**: Markdown and YAML ensure interoperability
- **Async Support**: FastAPI's async capabilities improve scalability
- **Community**: Strong communities for all chosen technologies

### Negative

- **Python Dependency**: Backend requires Python environment setup
- **Model Size**: sentence-transformers model requires ~120MB disk space
- **Single Embedding Model**: Limited to one model (all-MiniLM-L6-v2)
- **No Database Server**: LanceDB is file-based, not suitable for distributed systems
- **Learning Curve**: Team must learn FastAPI and Vue 3 Composition API
- **No Built-in Auth**: Authentication must be implemented separately

### Trade-offs

| Decision | Benefit | Trade-off |
|----------|---------|-----------|
| FastAPI over Flask/Django | Modern, async, auto-docs | Smaller ecosystem than Django |
| Vue 3 over React | Simpler, better DX | Smaller ecosystem than React |
| LanceDB over PostgreSQL | No server setup, local-first | Not distributed, limited features |
| sentence-transformers over OpenAI | Free, local, privacy | Limited to available models |

## Alternatives Considered

### Backend Frameworks

#### Flask
**Rejected because:**
- Requires more boilerplate for validation
- No async support (without extensions)
- Manual API documentation
- Less modern developer experience

#### Django
**Rejected because:**
- Heavier than needed for this use case
- ORM not required (using file-based storage)
- Monolithic structure doesn't fit service layer pattern
- Steeper learning curve

#### Node.js (Express/Fastify)
**Rejected because:**
- sentence-transformers is Python-native
- Would require Python service for embeddings anyway
- Less experience with Node.js in team

### Frontend Frameworks

#### React
**Rejected because:**
- More complex than needed
- Requires additional state management libraries
- Smaller bundle size with Vue 3
- Composition API is more intuitive than React hooks

#### Svelte
**Rejected because:**
- Smaller ecosystem and community
- Less mature tooling
- Fewer learning resources
- Less team familiarity

#### Angular
**Rejected because:**
- Too heavyweight for this project
- Steep learning curve
- Opinionated structure not needed
- Slower development cycle

### Vector Databases

#### PostgreSQL + pgvector
**Rejected because:**
- Requires database server setup
- Overkill for local-first deployment
- Additional infrastructure complexity
- Not optimized for vector operations

#### ChromaDB
**Rejected because:**
- Less mature than LanceDB
- Fewer features
- Smaller community
- Less stable API

#### Pinecone
**Rejected because:**
- Cloud-only (violates local-first principle)
- Requires API keys and internet connection
- Monthly costs
- Data privacy concerns

#### Weaviate
**Rejected because:**
- Requires Docker deployment
- More complex than needed
- Heavier resource requirements
- Overkill for local use

### Embedding Models

#### OpenAI Embeddings
**Rejected because:**
- Requires API key and payment
- Internet connection required
- Data privacy concerns
- Monthly costs

#### HuggingFace Hub Models
**Rejected because:**
- Would require downloading multiple models
- Larger disk space requirements
- More complex model management
- All-MiniLM-L6-v2 is sufficient for current needs

#### Custom Model Training
**Rejected because:**
- Requires training data and infrastructure
- Time-consuming
- Overkill for initial release
- sentence-transformers models are excellent

## Implementation Notes

### Backend Setup
```python
# FastAPI application structure
app/
├── main.py              # Application entry point
├── config.py            # Settings from .env
├── routers/             # API endpoints
├── services/            # Business logic
├── models/              # Pydantic schemas
└── mcp/                # MCP integration
```

### Frontend Setup
```javascript
// Vue 3 application structure
src/
├── main.js             # Vue app bootstrap
├── App.vue             # Root component
├── components/          # UI components
├── api/                # Backend client
└── style.css           # Design system
```

### Dependencies

**Backend (requirements.txt):**
```
fastapi>=0.104.0
uvicorn>=0.24.0
pydantic>=2.5.0
pydantic-settings>=2.1.0
lancedb>=0.3.0
sentence-transformers>=2.2.0
python-frontmatter>=1.0.0
fastapi-mcp>=0.1.0
python-dotenv>=1.0.0
```

**Frontend (package.json):**
```json
{
  "dependencies": {
    "vue": "^3.4.0",
    "vue-router": "^4.2.0",
    "axios": "^1.6.0",
    "marked": "^11.0.0"
  },
  "devDependencies": {
    "@vitejs/plugin-vue": "^4.5.0",
    "vite": "^5.0.0"
  }
}
```

## References

- [FastAPI Documentation](https://fastapi.tiangolo.com/)
- [Vue 3 Documentation](https://vuejs.org/)
- [LanceDB Documentation](https://lancedb.github.io/lancedb/)
- [sentence-transformers](https://www.sbert.net/)
- [MCP Specification](https://modelcontextprotocol.io/)
- [Project History](../01-project-context/history.md)
- [Architecture - Backend](../../docs/architecture-backend.md)
- [Architecture - Frontend](../../docs/architecture-frontend.md)

## Related Decisions

- [ADR-002: Service Layer Architecture Pattern](./adr-002-architecture-pattern.md) - How we organize the backend
- [ADR-003: MCP Integration](./adr-003-mcp-integration.md) - How we integrate with AI
- [ADR-004: Data Model](./adr-004-data-model.md) - How we structure notes
- [ADR-005: Embedding Model](./adr-005-embedding-model.md) - Which embedding model we use

---

**Status:** This decision is active and forms the foundation of the OrgAI technology stack.
