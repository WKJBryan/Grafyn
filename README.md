# Grafyn

A local knowledge graph platform with semantic search, Obsidian-style linking, Multi-LLM Canvas, and MCP (Model Context Protocol) integration. Available as both a web app and a native desktop application.

## Features

### Core Knowledge Management
- **Obsidian-compatible notes** - Markdown files with YAML frontmatter and `[[wikilinks]]`
- **Semantic search** - Vector-based search using sentence-transformers (384-dim embeddings)
- **Backlinks & graph** - Automatic bidirectional link tracking with force-directed visualization
- **Note workflows** - Draft → Evidence → Canonical status progression
- **Typed properties** - Custom metadata fields with type validation

### Multi-LLM Canvas
- **Compare AI models** - Send one prompt to multiple models simultaneously
- **Real-time streaming** - SSE-based response streaming from 100+ models via OpenRouter
- **Infinite canvas** - D3.js-powered zoom/pan interface with draggable tiles
- **Debate mode** - Models critique and respond to each other
- **Branching conversations** - Create conversation trees with context inheritance
- **Export to notes** - Convert canvas sessions to knowledge base notes

### AI Integration
- **MCP Server** - Connect Claude, ChatGPT, and other AI models to your knowledge base
- **MCP Write Operations** - AI models can create and update notes directly
- **Conversation Import** - Import chat history from ChatGPT, Claude, Grok, and Gemini
- **Distillation Workflow** - Transform container notes into atomic notes and topic hubs
- **Link Discovery** - AI-powered suggestions for connecting related notes

### Feedback & Bug Reporting
- **In-app feedback** - Submit bug reports, feature requests, and general feedback
- **GitHub Issues integration** - Submissions automatically create GitHub issues
- **Offline support (Desktop)** - Feedback queued when offline, auto-retries on reconnect

### Deployment Options
| Mode | Backend | Use Case |
|------|---------|----------|
| **Web** | Python/FastAPI | Development, MCP integration, multi-user |
| **Desktop** | Rust/Tauri | Single-user, portable, offline-capable, live vault switching |

## Tech Stack

### Backend (Python)
- **Framework**: FastAPI 0.128+
- **Vector Database**: LanceDB 0.26+
- **Embeddings**: sentence-transformers 5.2+ (all-MiniLM-L6-v2)
- **Multi-LLM API**: OpenRouter
- **Authentication**: OAuth 2.0 (GitHub)
- **Testing**: pytest (210+ tests)

### Frontend (Vue 3)
- **Framework**: Vue 3.4+ with Composition API
- **State Management**: Pinia
- **Build Tool**: Vite 5.0+
- **Visualization**: D3.js v7+
- **Markdown**: marked 11.0+

### Desktop (Tauri)
- **Framework**: Tauri 1.8
- **Search Engine**: Tantivy 0.22
- **Graph**: petgraph 0.6
- **Async Runtime**: tokio 1.0

## Quick Start

### Prerequisites

- Python 3.11+ (for web backend)
- Node.js 18+
- Rust (for desktop app only)

### Web App Setup

```bash
# Backend
cd backend
pip install -r requirements.txt
cp .env.example .env
# Edit .env with your configuration
uvicorn app.main:app --reload --port 8080

# Frontend (new terminal)
cd frontend
npm install
npm run dev
```

- Backend: http://localhost:8080
- Frontend: http://localhost:5173

### Desktop App Setup

**Prerequisites:**
- Windows: Visual Studio Build Tools 2022 with C++ workload
- macOS: Xcode Command Line Tools (`xcode-select --install`)
- Linux: `sudo apt install build-essential libgtk-3-dev libwebkit2gtk-4.0-dev`
- All platforms: Rust via [rustup](https://rustup.rs/)

```bash
cd frontend
npm install

# Generate app icons (required for first build)
node scripts/generate-icons.cjs

# Development mode
npm run tauri:dev       # macOS/Linux
./run-tauri-dev.bat     # Windows

# Production build
npm run tauri:build     # macOS/Linux
./build-tauri.bat       # Windows
```

**Output locations:**
- Windows: `src-tauri/target/release/bundle/msi/Grafyn_1.0.0_x64.msi`
- macOS: `src-tauri/target/release/bundle/dmg/Grafyn_1.0.0_x64.dmg`
- Linux: `src-tauri/target/release/bundle/deb/grafyn_1.0.0_amd64.deb`

### Docker Setup

```bash
cd backend
docker-compose up
```

## Configuration

### Environment Variables

Create `.env` in the project root (see `backend/.env.example`):

| Variable | Default | Description |
|----------|---------|-------------|
| `VAULT_PATH` | `../vault` | Path to markdown notes |
| `DATA_PATH` | `../data` | Path to vector database and indices |
| `OPENROUTER_API_KEY` | - | Required for Multi-LLM Canvas |
| `GITHUB_CLIENT_ID` | - | GitHub OAuth (for MCP integration) |
| `GITHUB_CLIENT_SECRET` | - | GitHub OAuth secret |
| `TOKEN_ENCRYPTION_KEY` | - | Fernet key for token encryption |
| `ENVIRONMENT` | `development` | `development` or `production` |
| `CORS_ORIGINS` | `*` | Comma-separated allowed origins |
| `GITHUB_FEEDBACK_REPO` | - | Target repo for feedback issues (`owner/repo`) |
| `GITHUB_FEEDBACK_TOKEN` | - | GitHub PAT with `public_repo` scope |

Generate encryption key:
```bash
python -c "from cryptography.fernet import Fernet; print(Fernet.generate_key().decode())"
```

## Usage

### Notes

1. Click **"+ New Note"** in the header
2. Write content in Markdown with `[[wikilinks]]` to other notes
3. Set status (draft/evidence/canonical) and add tags
4. Save - note is automatically indexed for semantic search

### Search

- **Semantic search**: Type naturally in the search bar
- **Operators**: `tag:python`, `status:canonical`, `type:atomic`
- **Filters**: Include/exclude tags with `+tag` or `-tag`

### Multi-LLM Canvas

1. Navigate to `/canvas` or click the Canvas tab
2. Create a new session
3. Click **"+ Prompt"** to open the prompt dialog
4. Select models to compare (e.g., GPT-4, Claude, Gemini)
5. Enter your prompt - responses stream in real-time
6. Use **Debate Mode** to have models critique each other
7. Export insights to your knowledge base

### Conversation Import

1. Export conversations from ChatGPT, Claude, Grok, or Gemini
2. Navigate to `/import`
3. Upload the JSON file
4. Review parsed conversations with quality scores
5. Select conversations to import as notes

### MCP Integration

Connect external AI models to your knowledge base:

1. Create a GitHub OAuth app at https://github.com/settings/developers
2. Configure `.env` with OAuth credentials
3. Start ngrok tunnel: `ngrok http 8080`
4. Add MCP server in your AI client: `https://your-url.ngrok-free.dev/sse`

See [CHATGPT_MCP_SETUP_GUIDE.md](CHATGPT_MCP_SETUP_GUIDE.md) for detailed instructions.

### Feedback & Bug Reporting

Submit feedback directly from the app:

1. Click the **💬** button in the header
2. Select feedback type (Bug Report, Feature Request, or General)
3. Enter a title and description
4. Optionally include system information
5. Submit - creates a GitHub issue automatically

**Desktop offline support:** Feedback is queued when offline and automatically submitted when connectivity is restored.

## API Documentation

Interactive docs available at:
- **Swagger UI**: http://localhost:8080/docs
- **ReDoc**: http://localhost:8080/redoc

### Main Endpoints

| Category | Endpoint | Description |
|----------|----------|-------------|
| **Notes** | `GET /api/notes` | List all notes |
| | `POST /api/notes` | Create note |
| | `GET /api/notes/{id}` | Get note |
| | `PUT /api/notes/{id}` | Update note |
| | `DELETE /api/notes/{id}` | Delete note |
| **Search** | `GET /api/search?q={query}` | Semantic search |
| | `GET /api/search/similar/{id}` | Find similar notes |
| **Graph** | `GET /api/graph/backlinks/{id}` | Get backlinks |
| | `GET /api/graph/neighbors/{id}` | Get graph neighbors |
| **Canvas** | `GET /api/canvas` | List sessions |
| | `POST /api/canvas/{id}/prompt` | Send prompt (SSE) |
| | `POST /api/canvas/{id}/debate` | Start debate (SSE) |
| **Import** | `POST /api/import/upload` | Upload conversations |
| | `POST /api/import/jobs/{id}/import` | Execute import |
| **Zettelkasten** | `GET /api/zettel/suggestions/{id}` | Get link suggestions |
| | `GET /api/zettel/orphans` | Find unlinked notes |
| **Feedback** | `POST /api/feedback` | Submit feedback |
| | `GET /api/feedback/status` | Check service status |

## Architecture

```
grafyn/
├── backend/                    # Python FastAPI backend
│   ├── app/
│   │   ├── main.py            # Application entry point
│   │   ├── routers/           # API endpoints (10 routers)
│   │   ├── services/          # Business logic (12 services)
│   │   ├── models/            # Pydantic schemas
│   │   ├── middleware/        # Security, logging, rate limiting
│   │   └── mcp/               # MCP protocol integration
│   └── tests/                 # pytest test suite (210+ tests)
│
├── frontend/                   # Vue 3 SPA + Tauri desktop
│   ├── src/
│   │   ├── views/             # Page components (7 views)
│   │   ├── components/        # UI components (33 total)
│   │   │   └── canvas/        # Canvas-specific (9 components)
│   │   ├── stores/            # Pinia state (5 stores)
│   │   └── api/               # API client (auto-detects Tauri/web)
│   └── src-tauri/             # Rust desktop backend
│       ├── src/
│       │   ├── commands/      # IPC handlers
│       │   ├── services/      # Rust business logic
│       │   └── models/        # Data structures
│       └── Cargo.toml
│
├── vault/                      # Markdown notes (Obsidian-compatible)
├── data/                       # LanceDB, search indices, canvas data
└── docs/                       # Project documentation (14 guides)
```

### Service Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Vue 3 Frontend                            │
│  Notes │ Canvas │ Graph │ Search │ Import                   │
└────────────────────────┬────────────────────────────────────┘
                         │
        ┌────────────────┴────────────────┐
        │                                 │
        ▼                                 ▼
┌───────────────────┐           ┌───────────────────┐
│  Python Backend   │           │   Rust Backend    │
│    (Web Mode)     │           │  (Desktop Mode)   │
├───────────────────┤           ├───────────────────┤
│ • KnowledgeStore  │           │ • KnowledgeStore  │
│ • VectorSearch    │           │ • SearchService   │
│ • GraphIndex      │           │ • GraphIndex      │
│ • OpenRouter      │           │ • OpenRouter      │
│ • CanvasStore     │           │ • CanvasStore     │
│ • Distillation    │           └───────────────────┘
│ • ImportService   │
│ • LinkDiscovery   │
└───────────────────┘
```

## Testing

```bash
cd backend

# Install dev dependencies
pip install -r requirements-dev.txt

# Run all tests
pytest

# Run with coverage
pytest --cov=app --cov-report=html

# Run by category
pytest -m unit           # Unit tests only
pytest -m integration    # Integration tests
pytest -m security       # Security tests
pytest -m "not slow"     # Skip slow tests
```

**Test coverage:**
- `test_knowledge_store.py` - 50+ tests (CRUD, path traversal, wikilinks)
- `test_vector_search.py` - 45+ tests (LanceDB, indexing, similarity)
- `test_graph_index.py` - 35+ tests (backlinks, traversal, cycles)
- `test_token_store.py` - 40+ tests (encryption, TTL, cleanup)
- `test_embedding.py` - 40+ tests (vector encoding, dimensions)

## Security

- **Path traversal protection** - Sanitized note IDs, resolved paths
- **OAuth 2.0** - GitHub authentication for MCP
- **Token encryption** - Fernet encryption for stored tokens
- **Rate limiting** - 10/min, 50/hr, 200/day per IP
- **Security headers** - X-Content-Type-Options, X-Frame-Options
- **Input sanitization** - Request validation middleware
- **CORS** - Strict origin checking in production

## Documentation

Comprehensive documentation in the `docs/` directory:

- [Project Overview](docs/project-overview.md)
- [Backend Architecture](docs/architecture-backend.md)
- [Frontend Architecture](docs/architecture-frontend.md)
- [Canvas Architecture](docs/canvas-architecture.md)
- [API Contracts](docs/api-contracts-backend.md)
- [Data Models](docs/data-models-backend.md)
- [Backend Development Guide](docs/development-guide-backend.md)
- [Frontend Development Guide](docs/development-guide-frontend.md)
- [Chat Ingestion Guide](docs/chat-ingestion-guide.md)

## CI/CD

Releases are built automatically when a `v*` tag is pushed. The workflow in `.github/workflows/release.yml` runs in four phases:

1. **Create release** — a single draft GitHub release is created
2. **Build** — 4 parallel matrix jobs (Windows x64/ARM, macOS ARM, Linux x64) build Tauri bundles and upload artifacts to the draft release
3. **Publish release** — the draft is marked as published once all builds complete
4. **Upload to R2** — assets are mirrored to Cloudflare R2 for auto-update distribution

Manual builds can be triggered via `workflow_dispatch` without creating a release.

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Write tests for new functionality
5. Run the test suite (`pytest`)
6. Submit a pull request

### Code Style

- **Backend**: PEP 8, type hints required
- **Frontend**: ESLint + Prettier (configured)
- **Rust**: `cargo fmt` and `cargo clippy`

## License

[Add your license here]

## Support

For issues and questions, please open an issue on GitHub.
