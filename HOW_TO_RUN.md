# Grafyn — How to Run Everything

## 1. Desktop App (Tauri) — Recommended for Production Use

The full desktop experience: single binary, Rust backend, Vue frontend in a WebView.

```bash
cd frontend

# Windows (auto-sets up VS build environment)
./run-tauri-dev.bat        # Dev mode with hot reload
./build-tauri.bat          # Debug build

# macOS/Linux (or VS Developer Command Prompt on Windows)
npm run tauri:dev           # Dev mode with hot reload
npm run tauri:build         # Production build → src-tauri/target/release/bundle/
```

**Prerequisites:** Rust (rustup), Node.js, [Tauri v1 prerequisites](https://v1.tauri.app/v1/guides/getting-started/prerequisites)

**Environment:** `set OPENROUTER_API_KEY=your-key` for the Multi-LLM Canvas feature

**Data location:** `~/Documents/Grafyn/` (vault/ for notes, data/ for indexes)

---

## 2. Web Dev Mode (Python Backend + Vue Frontend)

Two separate processes — good for backend development and debugging.

### Terminal 1 — Python Backend (port 8080)
```bash
cd backend
pip install -r requirements.txt       # First time only
uvicorn app.main:app --reload --host 0.0.0.0 --port 8080
```

Or with Docker:
```bash
cd backend
docker-compose up
```

### Terminal 2 — Vue Frontend (port 5173)
```bash
cd frontend
npm install                            # First time only
npm run dev                            # Dev server on http://localhost:5173
```

The frontend auto-detects Tauri vs web: in web mode it uses Axios HTTP to `:8080`, in Tauri it uses IPC invoke().

**Environment file:**
```bash
cp backend/.env.example .env
# Edit .env — set OPENROUTER_API_KEY, GITHUB_FEEDBACK_TOKEN, etc.
```

---

## 3. Backend Tests (pytest)

```bash
cd backend
pip install -r requirements-dev.txt    # First time only

pytest                                 # All 795+ tests
pytest --cov=app --cov-report=html     # With coverage report
pytest -m unit                         # Unit tests only (~600)
pytest -m integration                  # Integration tests only (~200)
pytest -m security                     # Security tests
pytest tests/integration/test_graph_api.py  # Single file
pytest -k "TestGetNeighbors"           # Single test class
```

---

## 4. Frontend Only (Lint / Format / Build)

```bash
cd frontend
npm run dev              # Dev server on :5173
npm run build            # Production build (outputs to dist/)
npm run lint             # ESLint
npm run format           # Prettier
```

---

## 5. MCP Sidecar (Desktop + Claude/ChatGPT Integration)

Bundles a Python backend as a sidecar for MCP protocol support.

### Build the sidecar binary
```bash
cd backend
pip install pyinstaller
python build-exe.py      # Bundles to frontend/src-tauri/binaries/
```

### Enable at runtime
```bash
set MCP_ENABLED=1        # Windows
export MCP_ENABLED=1     # macOS/Linux
```

### Connect Claude Desktop
Add to `claude_desktop_config.json`:
```json
{ "mcpServers": { "grafyn-local": { "url": "http://localhost:8765/sse" } } }
```

---

## 6. Key Environment Variables

| Variable | Required For | Default |
|----------|-------------|---------|
| `OPENROUTER_API_KEY` | Multi-LLM Canvas | `""` (canvas disabled) |
| `GITHUB_FEEDBACK_REPO` | Feedback system | `""` |
| `GITHUB_FEEDBACK_TOKEN` | Feedback system | `""` |
| `TOKEN_ENCRYPTION_KEY` | OAuth tokens | — (generate with Fernet) |
| `ENVIRONMENT` | CORS policy | `development` |
| `MCP_ENABLED` | MCP sidecar | `0` (disabled) |
| `RUST_LOG` | Tauri logging | `info` |

---

## Quick Start (Most Common Workflow)

For day-to-day development, you likely want **one of**:

- **Frontend + Backend changes:** Run Web Dev Mode (#2) — two terminals
- **Desktop app testing:** Run Tauri Dev (#1) — single command
- **After code changes:** Run tests (#3) — `pytest --tb=short -q`
