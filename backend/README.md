# Seedream Backend

FastAPI-based backend for the Seedream knowledge graph platform with semantic search and MCP integration.

## Features

- **Note Management**: CRUD operations for Markdown notes with YAML frontmatter
- **Semantic Search**: Vector-based search using LanceDB and sentence-transformers
- **Knowledge Graph**: Wikilink parsing and backlink tracking
- **MCP Integration**: Model Context Protocol for AI model integration (Claude Desktop, ChatGPT)
- **OAuth Authentication**: GitHub OAuth for ChatGPT MCP access

## Technology Stack

- **Framework**: FastAPI 0.104+
- **Language**: Python 3.10+
- **Vector Database**: LanceDB 0.3+
- **Embeddings**: sentence-transformers 2.2+ (all-MiniLM-L6-v2)
- **Data Validation**: Pydantic 2.5+
- **MCP Integration**: fastapi-mcp 0.1+

## Installation

### Prerequisites

- Python 3.10 or higher
- pip (Python package manager)

### Setup

1. **Clone the repository and navigate to the backend directory**

```bash
cd backend
```

2. **Create a virtual environment**

```bash
python -m venv venv
```

3. **Activate the virtual environment**

**Windows:**
```bash
venv\Scripts\activate
```

**macOS/Linux:**
```bash
source venv/bin/activate
```

4. **Install dependencies**

```bash
pip install -r requirements.txt
```

5. **Configure environment variables**

Copy the example environment file and customize it:

```bash
cp .env.example .env
```

Edit `.env` with your configuration:

```bash
# Server Configuration
SERVER_HOST=0.0.0.0
SERVER_PORT=8080

# Paths
VAULT_PATH=../vault
DATA_PATH=../data

# Embedding Model
EMBEDDING_MODEL=all-MiniLM-L6-v2

# OAuth Configuration (for ChatGPT MCP)
GITHUB_CLIENT_ID=your-github-client-id
GITHUB_CLIENT_SECRET=your-github-client-secret
GITHUB_REDIRECT_URI=https://your-name.ngrok.io/auth/callback

# CORS Configuration
CORS_ORIGINS=http://localhost:5173,http://localhost:3000

# Environment
ENVIRONMENT=development
```

6. **Create required directories**

```bash
mkdir -p vault data
```

## Running the Server

### Development Mode

Run the server with auto-reload:

```bash
python -m app.main
```

Or using uvicorn directly:

```bash
uvicorn app.main:app --host 0.0.0.0 --port 8080 --reload
```

The server will start at `http://localhost:8080`

### Production Mode

Run without auto-reload:

```bash
uvicorn app.main:app --host 0.0.0.0 --port 8080 --workers 4
```

## API Documentation

Once the server is running, access the interactive API documentation:

- **Swagger UI**: http://localhost:8080/docs
- **ReDoc**: http://localhost:8080/redoc

## API Endpoints

### Notes API

- `GET /api/notes` - List all notes
- `GET /api/notes/{note_id}` - Get a specific note
- `POST /api/notes` - Create a new note
- `PUT /api/notes/{note_id}` - Update a note
- `DELETE /api/notes/{note_id}` - Delete a note
- `POST /api/notes/reindex` - Reindex all notes

### Search API

- `GET /api/search` - Search notes (semantic or lexical)
- `GET /api/search/similar/{note_id}` - Find similar notes

### Graph API

- `GET /api/graph/backlinks/{note_id}` - Get backlinks
- `GET /api/graph/outgoing/{note_id}` - Get outgoing links
- `GET /api/graph/neighbors/{note_id}` - Get neighboring notes
- `GET /api/graph/unlinked-mentions/{note_id}` - Find unlinked mentions
- `POST /api/graph/rebuild` - Rebuild the graph

### System Endpoints

- `GET /` - API information
- `GET /health` - Health check

### MCP Endpoints

- `GET /sse` - MCP server endpoint (SSE transport)
- `GET /auth/github` - GitHub OAuth authorization
- `GET /auth/callback` - OAuth callback handler

## OAuth Setup for ChatGPT MCP

### 1. Register GitHub OAuth App

1. Go to https://github.com/settings/developers
2. Click "New OAuth App"
3. Fill in the form:
   - **Application name**: Seedream Knowledge Base
   - **Homepage URL**: https://your-name.ngrok.io
   - **Authorization callback URL**: https://your-name.ngrok.io/auth/callback
4. Click "Register application"
5. Copy the **Client ID** and **Client Secret**

### 2. Configure Backend

Add to `backend/.env`:

```bash
GITHUB_CLIENT_ID=your-github-client-id
GITHUB_CLIENT_SECRET=your-github-client-secret
GITHUB_REDIRECT_URI=https://your-name.ngrok.io/auth/callback
```

### 3. Expose Backend Publicly

For development, use ngrok:

```bash
# Install ngrok from https://ngrok.com/download

# Start ngrok tunnel
ngrok http 8080

# Copy HTTPS URL (e.g., https://abc123.ngrok.io)
# Update GITHUB_REDIRECT_URI to use this URL
```

### 4. Register MCP Server in ChatGPT

In ChatGPT settings, register the MCP server:

```
Server Name: Seedream Knowledge Base
SSE Endpoint: https://your-name.ngrok.io/sse
OAuth Provider: GitHub
Client ID: your-github-client-id
Client Secret: your-github-client-secret
Authorization URL: https://your-name.ngrok.io/auth/github
Callback URL: https://your-name.ngrok.io/auth/callback
```

## Project Structure

```
backend/
├── app/
│   ├── __init__.py
│   ├── main.py              # FastAPI application entry point
│   ├── config.py            # Configuration management
│   ├── routers/             # API route handlers
│   │   ├── __init__.py
│   │   ├── notes.py         # Note CRUD endpoints
│   │   ├── search.py        # Search endpoints
│   │   └── graph.py         # Graph endpoints
│   ├── services/            # Business logic
│   │   ├── __init__.py
│   │   ├── knowledge_store.py    # Markdown I/O operations
│   │   ├── vector_search.py     # LanceDB semantic search
│   │   ├── graph_index.py       # Wikilink and backlink tracking
│   │   └── embedding.py        # Text to vector encoding
│   ├── models/              # Pydantic schemas
│   │   ├── __init__.py
│   │   └── note.py         # Note models
│   └── mcp/                # MCP integration
│       ├── __init__.py
│       ├── server.py        # MCP server setup
│       └── oauth.py        # OAuth authentication
├── requirements.txt         # Python dependencies
├── .env.example            # Environment variables template
├── .gitignore             # Git ignore rules
└── README.md              # This file
```

## Development

### Running Tests

```bash
pytest
```

### Code Style

The project follows PEP 8 style guidelines. Use a linter like `flake8` or `black`:

```bash
pip install black flake8
black app/
flake8 app/
```

## Troubleshooting

### Port Already in Use

If port 8080 is already in use, change the port in `.env`:

```bash
SERVER_PORT=8081
```

### Embedding Model Download

The first time you run the server, it will download the sentence-transformers model (~120MB). This may take a few minutes depending on your internet connection.

### LanceDB Initialization

LanceDB will automatically create the necessary database files in the `data/` directory on first run.

## License

[Specify your license here]

## Contributing

[Specify your contribution guidelines here]
