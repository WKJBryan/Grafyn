# Grafyn - Getting Started Guide

> **Step-by-step setup guide for new developers**

## Prerequisites

| Requirement | Version | Installation |
|-------------|---------|--------------|
| **Python** | 3.11+ | [python.org](https://python.org) |
| **Node.js** | 18+ | [nodejs.org](https://nodejs.org) |
| **uv** | Latest | `pip install uv` or [docs.astral.sh/uv](https://docs.astral.sh/uv) |

Verify installations:
```bash
python --version   # Should show 3.11+
node --version     # Should show 18+
uv --version       # Should show uv version
```

---

## Step 1: Clone the Repository

```bash
git clone <repository-url>
cd Grafyn
```

---

## Step 2: Backend Setup

### 2.1 Install Dependencies

```bash
# From the project root
uv sync
```

This installs all Python dependencies from `pyproject.toml`.

### 2.2 Configure Environment

```bash
cd backend
cp .env.example .env
```

Edit `.env` to configure your settings:

```env
# Required paths
VAULT_PATH=../vault
DATA_PATH=../data

# Server settings
SERVER_HOST=0.0.0.0
SERVER_PORT=8080

# Environment (development or production)
ENVIRONMENT=development
```

### 2.3 Start the Backend Server

```bash
# From project root
uv run uvicorn backend.app.main:app --reload --host 0.0.0.0 --port 8080
```

Or from the backend directory:
```bash
cd backend
uv run uvicorn app.main:app --reload --host 0.0.0.0 --port 8080
```

### 2.4 Verify Backend is Running

- **API Docs:** http://localhost:8080/docs
- **Health Check:** http://localhost:8080/health
- **Root Info:** http://localhost:8080/

---

## Step 3: Frontend Setup

### 3.1 Install Dependencies

```bash
cd frontend
npm install
```

### 3.2 Start Development Server

```bash
npm run dev
```

### 3.3 Access the Application

Open http://localhost:5173 in your browser.

> **Note:** The frontend proxies API requests to `localhost:8080`, so ensure the backend is running.

---

## Step 4: First-Time Usage

### Creating Your First Note

1. Open the UI at http://localhost:5173
2. Click **"+ New Note"** in the header
3. Enter a title and content using Markdown
4. Use `[[wikilinks]]` to link to other notes
5. Click **Save**

### Testing Search

1. Create a few notes with different content
2. Use the search bar to search semantically
3. Click a result to open the note

### Viewing Backlinks

1. Select a note that has incoming links
2. The **Backlinks** panel on the right shows notes linking to it

---

## Step 5: Optional - GitHub OAuth Setup

For authentication features, configure GitHub OAuth:

### 5.1 Create a GitHub OAuth App

1. Go to [GitHub Developer Settings](https://github.com/settings/developers)
2. Click **"New OAuth App"**
3. Set **Authorization callback URL** to `http://localhost:5173/oauth/callback`
4. Copy the **Client ID** and generate a **Client Secret**

### 5.2 Configure Environment Variables

Add to your `.env`:

```env
GITHUB_CLIENT_ID=your_client_id
GITHUB_CLIENT_SECRET=your_client_secret
GITHUB_REDIRECT_URI=http://localhost:5173/oauth/callback
```

### 5.3 Restart the Backend

The OAuth login option will now appear on the frontend.

---

## Quick Reference

| Service | URL | Purpose |
|---------|-----|---------|
| Frontend UI | http://localhost:5173 | Main application |
| Backend API | http://localhost:8080 | REST API |
| API Docs | http://localhost:8080/docs | OpenAPI documentation |
| Health Check | http://localhost:8080/health | Service status |

---

## Next Steps

- [Project Overview](./project-overview.md) - Understand the architecture
- [Development Guide - Backend](./development-guide-backend.md) - Backend development
- [Development Guide - Frontend](./development-guide-frontend.md) - Frontend development
- [API Contracts](./api-contracts-backend.md) - Full API documentation
- [Chat Ingestion Guide](./chat-ingestion-guide.md) - MCP and AI integration

---

## Troubleshooting

### Backend won't start
- Ensure Python 3.11+ is installed
- Run `uv sync` to install dependencies
- Check `.env` file exists and has valid paths

### Frontend shows "Failed to fetch"
- Ensure backend is running on port 8080
- Check browser console for CORS errors

### First run is slow
- The embedding model (~90MB) downloads on first startup
- Subsequent starts are fast

### Can't find uv command
- Install with `pip install uv` or see [uv installation docs](https://docs.astral.sh/uv/getting-started/installation/)
