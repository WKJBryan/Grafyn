# Phase 1: Python Settings + Chat API

## Goal
Add server-side settings management and chat-with-notes endpoint to the Python backend. This phase is **non-breaking** — both backends continue working side by side.

## Why This Phase First
The Python backend currently uses `.env` for config, but the Tauri desktop app has a full settings system (`SettingsService` → `settings.json`). For the sidecar to work, Python needs to read/write the *same* `settings.json` and accept CLI args for `--vault-path`, `--data-path`, `--port`. The CLI arg parsing already exists in `main.py` (lines 209-280) — this phase extends it to work with the desktop settings file.

---

## Task 1: Settings Router + Service

### 1a. Create `backend/app/services/settings_service.py`

Mirrors the Rust `SettingsService` — reads/writes `settings.json` from the platform config dir.

```python
"""Desktop-compatible settings service.

Reads/writes the same settings.json as the Tauri desktop app
(AppData/Grafyn/settings.json on Windows, ~/.config/Grafyn/ on Linux/macOS).
"""

import json
import platform
from pathlib import Path
from typing import Optional
from pydantic import BaseModel


class UserSettings(BaseModel):
    """Mirrors Rust UserSettings struct in models/settings.rs"""
    vault_path: Optional[str] = None
    openrouter_api_key: Optional[str] = None
    setup_completed: bool = False
    theme: str = "system"
    mcp_enabled: bool = True

    def needs_setup(self) -> bool:
        return not self.setup_completed

    def has_openrouter_key(self) -> bool:
        return bool(self.openrouter_api_key)

    def effective_vault_path(self) -> Path:
        if self.vault_path:
            return Path(self.vault_path)
        return Path.home() / "Documents" / "Grafyn" / "vault"

    def effective_data_path(self) -> Path:
        """Data lives next to vault by default"""
        return self.effective_vault_path().parent / "data"


class SettingsStatus(BaseModel):
    needs_setup: bool
    has_vault_path: bool
    has_openrouter_key: bool


class SettingsUpdate(BaseModel):
    vault_path: Optional[str] = None
    openrouter_api_key: Optional[str] = None
    setup_completed: Optional[bool] = None
    theme: Optional[str] = None
    mcp_enabled: Optional[bool] = None


class SettingsService:
    """Manages settings.json — compatible with Tauri SettingsService."""

    def __init__(self):
        self._config_path = self._resolve_config_path()
        self._settings = self._load()

    @staticmethod
    def _resolve_config_path() -> Path:
        system = platform.system()
        if system == "Windows":
            base = Path.home() / "AppData" / "Roaming"
        elif system == "Darwin":
            base = Path.home() / "Library" / "Application Support"
        else:
            base = Path.home() / ".config"
        config_dir = base / "Grafyn"
        config_dir.mkdir(parents=True, exist_ok=True)
        return config_dir / "settings.json"

    def _load(self) -> UserSettings:
        if self._config_path.exists():
            try:
                data = json.loads(self._config_path.read_text())
                return UserSettings(**data)
            except Exception:
                return UserSettings()
        return UserSettings()

    def get(self) -> UserSettings:
        return self._settings

    def status(self) -> SettingsStatus:
        return SettingsStatus(
            needs_setup=self._settings.needs_setup(),
            has_vault_path=bool(self._settings.vault_path),
            has_openrouter_key=self._settings.has_openrouter_key(),
        )

    def update(self, update: SettingsUpdate) -> UserSettings:
        if update.vault_path is not None:
            path = Path(update.vault_path)
            path.mkdir(parents=True, exist_ok=True)
            self._settings.vault_path = update.vault_path
        if update.openrouter_api_key is not None:
            self._settings.openrouter_api_key = update.openrouter_api_key or None
        if update.setup_completed is not None:
            self._settings.setup_completed = update.setup_completed
        if update.theme is not None:
            self._settings.theme = update.theme
        if update.mcp_enabled is not None:
            self._settings.mcp_enabled = update.mcp_enabled
        self._save()
        return self._settings

    def complete_setup(self):
        self._settings.setup_completed = True
        self._save()

    def _save(self):
        self._config_path.write_text(
            json.dumps(self._settings.model_dump(), indent=2)
        )
```

### 1b. Create `backend/app/routers/settings.py`

```python
"""Settings API router — mirrors Tauri settings commands."""

from fastapi import APIRouter, Request, HTTPException
from app.services.settings_service import SettingsUpdate, SettingsStatus, UserSettings
from app.utils.dependencies import get_settings_service

router = APIRouter()

@router.get("", response_model=UserSettings)
async def get_settings(request: Request):
    service = get_settings_service(request)
    return service.get()

@router.get("/status", response_model=SettingsStatus)
async def get_settings_status(request: Request):
    service = get_settings_service(request)
    return service.status()

@router.put("", response_model=UserSettings)
async def update_settings(update: SettingsUpdate, request: Request):
    service = get_settings_service(request)
    result = service.update(update)
    # Sync dependent services if paths/keys changed
    if update.openrouter_api_key is not None:
        openrouter = request.app.state.openrouter
        openrouter.api_key = update.openrouter_api_key
    return result

@router.post("/complete-setup")
async def complete_setup(request: Request):
    service = get_settings_service(request)
    service.complete_setup()
    return {"status": "ok"}

@router.post("/validate-openrouter-key")
async def validate_openrouter_key(request: Request):
    body = await request.json()
    api_key = body.get("api_key", "")
    if not api_key:
        return {"valid": False}
    import httpx
    async with httpx.AsyncClient() as client:
        resp = await client.get(
            "https://openrouter.ai/api/v1/models",
            headers={"Authorization": f"Bearer {api_key}"},
        )
        return {"valid": resp.is_success}
```

### 1c. Wire into `main.py`

- Import `SettingsService` and attach to `app.state.settings_service` in lifespan
- Add `get_settings_service` to `app/utils/dependencies.py`
- Include router: `app.include_router(settings.router, prefix="/api/settings", tags=["settings"])`
- When `--vault-path` / `--data-path` are passed as CLI args, override `app.state.settings_service` paths

### 1d. Sync existing Python config with settings

The `config.py` `Settings` class reads `.env` vars. When running as a sidecar, these should be overridden by `settings.json` values. Add logic to `main.py` CLI entrypoint:

```python
# After parsing CLI args, apply settings.json overrides
settings_service = SettingsService()
if args.vault_path:
    os.environ["VAULT_PATH"] = args.vault_path
elif settings_service.get().vault_path:
    os.environ["VAULT_PATH"] = settings_service.get().vault_path

if settings_service.get().openrouter_api_key:
    os.environ["OPENROUTER_API_KEY"] = settings_service.get().openrouter_api_key
```

---

## Task 2: OpenRouter Status Endpoint

The frontend's `settings.js` calls `get_openrouter_status` — add a matching HTTP endpoint.

### Add to settings router:

```python
@router.get("/openrouter-status")
async def get_openrouter_status(request: Request):
    service = get_settings_service(request)
    settings = service.get()
    return {
        "has_key": settings.has_openrouter_key(),
        "is_configured": settings.has_openrouter_key(),
    }
```

---

## Task 3: Chat-with-Notes Endpoint (Stub)

This is the **new feature** — a chat endpoint that uses `recall_relevant` for context injection. Phase 7 implements the full feature; this phase just creates the router stub so the API surface is ready.

### Create `backend/app/routers/chat.py`

```python
"""Chat API router — chat with your notes using LLM + semantic recall."""

from fastapi import APIRouter, Request
from fastapi.responses import StreamingResponse
from pydantic import BaseModel
from typing import List, Optional

router = APIRouter()

class ChatMessage(BaseModel):
    role: str  # "user" or "assistant"
    content: str

class ChatRequest(BaseModel):
    messages: List[ChatMessage]
    model: str = "anthropic/claude-3-haiku"
    context_note_ids: Optional[List[str]] = None
    max_context_notes: int = 5
    temperature: float = 0.7

@router.post("/completions")
async def chat_completions(request: ChatRequest, req: Request):
    """Chat with your notes — returns SSE stream."""
    # Phase 7 implements the full pipeline:
    # 1. Extract latest user message
    # 2. recall_relevant() to find context notes
    # 3. Build system prompt with note context
    # 4. Stream from OpenRouter
    raise NotImplementedError("Chat endpoint will be implemented in Phase 7")
```

Wire into `main.py`: `app.include_router(chat.router, prefix="/api/chat", tags=["chat"])`

---

## Files Modified
| File | Action |
|------|--------|
| `backend/app/services/settings_service.py` | **Create** — desktop-compatible settings |
| `backend/app/routers/settings.py` | **Create** — settings HTTP endpoints |
| `backend/app/routers/chat.py` | **Create** — chat endpoint stub |
| `backend/app/utils/dependencies.py` | **Edit** — add `get_settings_service` |
| `backend/app/main.py` | **Edit** — wire settings + chat routers, lifespan init |

## Validation
- `pytest` passes (no breaking changes)
- `GET /api/settings/status` returns correct JSON
- `PUT /api/settings` writes `settings.json` to correct platform path
- Settings are compatible with Tauri's `settings.json` format (round-trip test)
- `npm run dev` + `npm run tauri:dev` both still work (no regressions)
