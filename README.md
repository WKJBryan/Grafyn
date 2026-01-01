# Seedream

A local knowledge graph platform with semantic search, Obsidian-style linking, and MCP (Model Context Protocol) integration.

## Features

- **Obsidian-compatible notes**: Markdown files with YAML frontmatter and `[[wikilinks]]`
- **Semantic search**: Vector-based search using sentence-transformers
- **Backlinks**: Automatic bidirectional link tracking and discovery
- **MCP Server**: Connects external AI models (Claude, ChatGPT) to knowledge base
- **Web UI**: Vue 3 SPA accessible from any device

## Tech Stack

### Backend
- **Framework**: FastAPI 0.104+
- **Vector Database**: LanceDB
- **Embeddings**: sentence-transformers (all-MiniLM-L6-v2)
- **Authentication**: OAuth (GitHub)
- **Python**: 3.11+

### Frontend
- **Framework**: Vue 3.4+
- **Build Tool**: Vite 5.0+
- **HTTP Client**: Axios 1.6+
- **Markdown**: marked 11.0+

## Quick Start

### Prerequisites

- Python 3.11 or higher
- Node.js 18 or higher
- Git

### Backend Setup

```bash
cd backend
pip install -r requirements.txt
cp .env.example .env
# Edit .env with your configuration
uvicorn app.main:app --reload
```

Backend will be available at `http://localhost:8080`

### Frontend Setup

```bash
cd frontend
npm install
npm run dev
```

Frontend will be available at `http://localhost:5173`

### Docker Setup

```bash
cd backend
docker-compose up
```

## Configuration

### Environment Variables

See `backend/.env.example` for all available options:

- `SERVER_HOST`: Server bind address (default: 0.0.0.0)
- `SERVER_PORT`: HTTP port (default: 8080)
- `VAULT_PATH`: Path to markdown notes (default: ../vault)
- `DATA_PATH`: Path to vector database (default: ../data)
- `EMBEDDING_MODEL`: Sentence transformer model (default: all-MiniLM-L6-v2)
- `GITHUB_CLIENT_ID`: GitHub OAuth client ID
- `GITHUB_CLIENT_SECRET`: GitHub OAuth client secret
- `GITHUB_REDIRECT_URI`: OAuth redirect URI
- `CORS_ORIGINS`: Comma-separated list of allowed origins
- `ENVIRONMENT`: development or production

## Usage

### Creating Notes

1. Click "+ New Note" in the header
2. Enter a title and write content in Markdown
3. Use `[[Note Title]]` to create wikilinks
4. Add tags (comma-separated) and set status
5. Click "Save"

### Searching

- Use the search bar for semantic search
- Results are ranked by similarity score
- Click a result to navigate to that note

### Backlinks

- When viewing a note, backlinks appear in the right panel
- Shows which notes link to the current note
- Includes context around the wikilink

## API Documentation

Interactive API documentation is available at:
- Swagger UI: `http://localhost:8080/docs`
- ReDoc: `http://localhost:8080/redoc`

### Main Endpoints

- `GET /api/notes` - List all notes
- `GET /api/notes/{id}` - Get a specific note
- `POST /api/notes` - Create a new note
- `PUT /api/notes/{id}` - Update a note
- `DELETE /api/notes/{id}` - Delete a note
- `GET /api/search?q={query}` - Search notes
- `GET /api/graph/backlinks/{id}` - Get backlinks for a note
- `GET /api/graph/outgoing/{id}` - Get outgoing links from a note

## Development

### Backend

```bash
cd backend
pip install -r requirements.txt
uvicorn app.main:app --reload
```

### Frontend

```bash
cd frontend
npm install
npm run dev
```

### Code Style

- Backend: Follow PEP 8, use type hints
- Frontend: Use ESLint and Prettier (configured)

## Architecture

The project follows a clean architecture with clear separation:

```
backend/
├── app/
│   ├── main.py          # FastAPI application
│   ├── config.py        # Settings management
│   ├── routers/         # API endpoints
│   ├── services/        # Business logic
│   ├── models/          # Pydantic schemas
│   └── mcp/            # MCP integration
└── requirements.txt

frontend/
├── src/
│   ├── main.js          # Vue app bootstrap
│   ├── App.vue          # Root component
│   ├── components/       # UI components
│   ├── stores/          # Pinia state management
│   └── api/            # API client
└── package.json
```

## Documentation

Comprehensive documentation is available in the `docs/` directory:
- [Project Overview](docs/project-overview.md)
- [Backend Architecture](docs/architecture-backend.md)
- [Frontend Architecture](docs/architecture-frontend.md)
- [API Contracts](docs/api-contracts-backend.md)
- [Development Guides](docs/development-guide-backend.md)

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Write tests (when applicable)
5. Submit a pull request

## License

[Add your license here]

## Support

For issues and questions, please open an issue on GitHub.
