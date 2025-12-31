# Setup Guide

> **Purpose:** Guide for setting up OrgAI development environment
> **Created:** 2025-12-31
> **Status:** Active

## Overview

This guide provides step-by-step instructions for setting up OrgAI development environment on your local machine.

## Prerequisites

### Required Software

| Software | Version | Purpose |
|----------|---------|---------|
| Python | 3.10+ | Backend development |
| Node.js | 18+ | Frontend development |
| Git | Latest | Version control |
| VS Code | Latest | Recommended IDE |

### Optional Software

| Software | Purpose |
|----------|---------|
| Docker | Containerization |
| Postman | API testing |
| Chrome DevTools | Browser debugging |

## Backend Setup

### 1. Clone Repository

```bash
git clone https://github.com/your-org/orgai.git
cd orgai
```

### 2. Create Virtual Environment

```bash
cd backend

# Windows
python -m venv venv
venv\Scripts\activate

# Linux/Mac
python -m venv venv
source venv/bin/activate
```

### 3. Install Dependencies

```bash
# Install production dependencies
pip install -r requirements.txt

# Install development dependencies
pip install -r requirements-dev.txt
```

### 4. Configure Environment

```bash
# Copy example environment file
cp .env.example .env

# Edit .env with your settings
# Windows: notepad .env
# Linux/Mac: nano .env
```

**Minimum .env configuration:**
```bash
VAULT_PATH=../vault
DATA_PATH=../data
SERVER_HOST=0.0.0.0
SERVER_PORT=8080
EMBEDDING_MODEL=all-MiniLM-L6-v2
```

### 5. Create Directories

```bash
# Create vault directory
mkdir -p ../vault

# Create data directory
mkdir -p ../data
```

### 6. Verify Installation

```bash
# Test Python imports
python -c "import fastapi; print('FastAPI OK')"
python -c "import lancedb; print('LanceDB OK')"
python -c "import sentence_transformers; print('Sentence Transformers OK')"

# Run tests
pytest tests/unit/ -v
```

### 7. Start Backend Server

```bash
# Development server with auto-reload
uvicorn app.main:app --reload --host 0.0.0.0 --port 8080

# Production server
uvicorn app.main:app --host 0.0.0.0 --port 8080 --workers 4
```

### 8. Verify Backend

```bash
# Check health endpoint
curl http://localhost:8080/health

# Check API docs
# Open browser to: http://localhost:8080/docs
```

## Frontend Setup

### 1. Navigate to Frontend Directory

```bash
cd frontend
```

### 2. Install Dependencies

```bash
npm install
```

### 3. Configure Environment (Optional)

```bash
# Create .env file if needed
cat > .env << EOF
VITE_API_BASE_URL=/api
VITE_MCP_BASE_URL=/mcp
EOF
```

### 4. Verify Installation

```bash
# Check Node.js version
node --version  # Should be 18+

# Check npm version
npm --version

# Verify dependencies
npm list
```

### 5. Start Development Server

```bash
# Development server with hot reload
npm run dev

# Build for production
npm run build

# Preview production build
npm run preview
```

### 6. Verify Frontend

```bash
# Open browser to: http://localhost:5173
# Should see OrgAI interface
```

## Development Workflow

### Starting Development Environment

```bash
# Terminal 1: Backend
cd backend
venv\Scripts\activate  # Windows
source venv/bin/activate  # Linux/Mac
uvicorn app.main:app --reload

# Terminal 2: Frontend
cd frontend
npm run dev
```

### Access Points

| Service | URL | Description |
|----------|-----|-------------|
| Frontend UI | http://localhost:5173 | Web interface |
| Backend API | http://localhost:8080 | REST API |
| API Docs | http://localhost:8080/docs | OpenAPI documentation |
| MCP Endpoint | http://localhost:8080/mcp | MCP server |

## IDE Setup

### VS Code Extensions

Recommended extensions for OrgAI development:

| Extension | Purpose |
|-----------|---------|
| Python | Python language support |
| Pylance | Python IntelliSense |
| ESLint | JavaScript linting |
| Prettier | Code formatting |
| Volar | Vue 3 language support |
| GitLens | Git integration |

### VS Code Settings

**File:** `.vscode/settings.json`

```json
{
  "python.linting.enabled": true,
  "python.formatting.provider": "black",
  "editor.formatOnSave": true,
  "editor.codeActionsOnSave": {
    "source.fixAll.eslint": true
  },
  "volar.takeOverMode.enabled": true
}
```

### VS Code Launch Configuration

**File:** `.vscode/launch.json`

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "name": "Python: FastAPI",
      "type": "debugpy",
      "request": "launch",
      "module": "uvicorn",
      "args": [
        "app.main:app",
        "--reload",
        "--host",
        "0.0.0.0",
        "--port",
        "8080"
      ],
      "cwd": "${workspaceFolder}/backend",
      "console": "integratedTerminal"
    },
    {
      "name": "JavaScript: Vite",
      "type": "node",
      "request": "launch",
      "runtimeExecutable": "npm",
      "runtimeArgs": ["run", "dev"],
      "cwd": "${workspaceFolder}/frontend",
      "console": "integratedTerminal"
    }
  ]
}
```

## Docker Setup (Optional)

### Backend Dockerfile

**File:** `backend/Dockerfile`

```dockerfile
FROM python:3.10-slim

WORKDIR /app

# Install system dependencies
RUN apt-get update && apt-get install -y \
    gcc \
    && rm -rf /var/lib/apt/lists/*

# Copy requirements
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy application
COPY . .

# Create directories
RUN mkdir -p /data /vault

# Expose port
EXPOSE 8080

# Run application
CMD ["uvicorn", "app.main:app", "--host", "0.0.0.0", "--port", "8080"]
```

### Frontend Dockerfile

**File:** `frontend/Dockerfile`

```dockerfile
FROM node:18-alpine

WORKDIR /app

# Copy package files
COPY package*.json ./

# Install dependencies
RUN npm ci

# Copy source
COPY . .

# Build application
RUN npm run build

# Expose port
EXPOSE 5173

# Run application
CMD ["npm", "run", "preview", "--", "--host", "0.0.0.0"]
```

### Docker Compose

**File:** `docker-compose.yml`

```yaml
version: '3.8'

services:
  backend:
    build: ./backend
    ports:
      - "8080:8080"
    volumes:
      - ./vault:/data/vault
      - ./data:/data/lancedb
    environment:
      - VAULT_PATH=/data/vault
      - DATA_PATH=/data/lancedb
      - SERVER_HOST=0.0.0.0
      - SERVER_PORT=8080

  frontend:
    build: ./frontend
    ports:
      - "5173:5173"
    depends_on:
      - backend
    environment:
      - VITE_API_BASE_URL=http://localhost:8080/api
```

### Running with Docker

```bash
# Build and start services
docker-compose up --build

# Start in background
docker-compose up -d

# View logs
docker-compose logs -f

# Stop services
docker-compose down
```

## Troubleshooting

### Backend Issues

#### Issue: Module Not Found

```bash
# Solution: Activate virtual environment
cd backend
source venv/bin/activate  # Linux/Mac
venv\Scripts\activate  # Windows

# Reinstall dependencies
pip install -r requirements.txt
```

#### Issue: Port Already in Use

```bash
# Solution: Use different port
export SERVER_PORT=8081
uvicorn app.main:app --reload --port 8081
```

#### Issue: Permission Denied

```bash
# Solution: Check directory permissions
ls -la ../vault ../data

# Fix permissions
chmod 755 ../vault ../data
```

### Frontend Issues

#### Issue: npm Install Fails

```bash
# Solution: Clear cache and reinstall
npm cache clean --force
rm -rf node_modules package-lock.json
npm install
```

#### Issue: Vite Dev Server Error

```bash
# Solution: Check Node.js version
node --version  # Should be 18+

# Update Node.js using nvm
nvm install 20
nvm use 20
```

#### Issue: Proxy Not Working

```bash
# Solution: Verify Vite configuration
cat vite.config.js

# Check proxy settings
# Should have: /api proxy to http://localhost:8080
```

## Verification Checklist

After setup, verify:

### Backend

- [ ] Virtual environment activated
- [ ] Dependencies installed successfully
- [ ] .env file configured
- [ ] Directories created (vault, data)
- [ ] Server starts without errors
- [ ] Health endpoint returns 200
- [ ] API docs accessible at /docs
- [ ] Tests pass: `pytest`

### Frontend

- [ ] Node.js version 18+
- [ ] Dependencies installed successfully
- [ ] Dev server starts without errors
- [ ] Frontend accessible at localhost:5173
- [ ] Can create notes through UI
- [ ] Can search notes
- [ ] No console errors

### Integration

- [ ] Frontend can call backend API
- [ ] CORS configured correctly
- [ ] MCP endpoint accessible
- [ ] End-to-end flow works

## Next Steps

After setup:

1. **Read Documentation**: Start with [`docs/index.md`](../../docs/index.md)
2. **Explore Code**: Review [`architecture-backend.md`](../../docs/architecture-backend.md)
3. **Run Tests**: Execute `pytest` and `npm test`
4. **Make Changes**: Follow [`Coding Standards`](../03-development-patterns/coding-standards.md)
5. **Contribute**: Follow [`Development Workflow`](../07-workflows/development-workflow.md)

## Related Documentation

- [Environment Variables](./environment-variables.md)
- [Troubleshooting](./troubleshooting.md)
- [Development Guide - Backend](../../docs/development-guide-backend.md)
- [Development Guide - Frontend](../../docs/development-guide-frontend.md)

---

**See Also:**
- [Project Overview](../../docs/project-overview.md)
- [Architecture - Backend](../../docs/architecture-backend.md)
- [Architecture - Frontend](../../docs/architecture-frontend.md)
