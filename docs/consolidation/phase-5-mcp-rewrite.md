# Phase 5: MCP Binary Rewrite

## Goal
Rewrite `grafyn-mcp` from a self-contained Rust binary (with direct service calls) to a thin HTTP proxy that forwards requests to the Python sidecar. This eliminates the need for `grafyn-mcp` to bundle its own search index, graph, and note parsing — shrinking it from ~10MB to ~3-5MB.

## Why Proxy Instead of Delete
Claude Desktop launches `grafyn-mcp` via stdio transport. We can't replace it with a Python script (would require Python installed) or remove it entirely (MCP needs a binary). The cleanest approach: `grafyn-mcp` becomes a thin proxy that translates MCP tool calls into HTTP requests to the Python backend.

---

## Current Architecture

```
Claude Desktop → grafyn-mcp (stdio) → Direct Rust service calls
                                     ├── KnowledgeStore (file I/O)
                                     ├── SearchService (Tantivy)
                                     ├── GraphIndex (petgraph)
                                     └── MemoryService
```

### Current dependencies used by MCP binary:
- `rmcp` v0.15 (MCP protocol) — **keep**
- `schemars` (JSON Schema) — **keep**
- `clap` (CLI args) — **keep**
- `tantivy` (search) — **remove** (huge compile/binary cost)
- `petgraph` (graph) — **remove**
- `gray_matter` (YAML) — **remove**
- `walkdir` (file traversal) — **remove**
- `regex` (wikilinks) — **remove**
- `serde_yaml` — **remove**
- `chrono` — **remove**

## New Architecture

```
Claude Desktop → grafyn-mcp (stdio) → HTTP requests to Python backend
                    ↑                        ↓
              rmcp framework         http://127.0.0.1:{port}/api/*
```

---

## Task 1: Rewrite `mcp_tools.rs`

Replace all direct service calls with `reqwest` HTTP calls to the Python backend.

### Before (direct service call):
```rust
#[tool(description = "List all notes...")]
async fn list_notes(&self) -> Result<CallToolResult, McpError> {
    let ks = self.knowledge_store.read().await;
    match ks.list_notes() {
        Ok(notes) => json_result(&response),
        Err(e) => err_result(format!("Failed: {}", e)),
    }
}
```

### After (HTTP proxy):
```rust
#[tool(description = "List all notes...")]
async fn list_notes(&self) -> Result<CallToolResult, McpError> {
    let resp = self.client
        .get(format!("{}/api/notes", self.base_url))
        .send()
        .await
        .map_err(|e| McpError {
            code: rmcp::model::ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("HTTP error: {}", e)),
            data: None,
        })?;
    let body = resp.text().await.map_err(|e| McpError {
        code: rmcp::model::ErrorCode::INTERNAL_ERROR,
        message: Cow::from(format!("Read error: {}", e)),
        data: None,
    })?;
    text_result(body)
}
```

### New `GrafynMcpServer` struct:

```rust
#[derive(Clone)]
pub struct GrafynMcpServer {
    client: reqwest::Client,
    base_url: String,
    tool_router: ToolRouter<Self>,
}

impl GrafynMcpServer {
    pub fn new(base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            tool_router: Self::tool_router(),
        }
    }
}
```

### Tool → HTTP Endpoint Mapping:

| MCP Tool | HTTP Method | Endpoint |
|----------|------------|----------|
| `list_notes` | GET | `/api/notes` |
| `get_note` | GET | `/api/notes/{id}` |
| `create_note` | POST | `/api/notes` |
| `update_note` | PUT | `/api/notes/{id}` |
| `delete_note` | DELETE | `/api/notes/{id}` |
| `search_notes` | GET | `/api/search?q={query}&limit={limit}` |
| `get_backlinks` | GET | `/api/graph/backlinks/{note_id}` |
| `get_outgoing` | GET | `/api/graph/outgoing/{note_id}` |
| `recall_relevant` | POST | `/api/memory/recall` |

---

## Task 2: Rewrite `mcp.rs` Entry Point

### Before:
- Resolves paths from CLI/settings
- Initializes `KnowledgeStore`, `SearchService` (with read-only fallback), `GraphIndex`, `MemoryService`
- Builds graph index from all notes
- 165 lines

### After:
- Resolves backend URL from CLI/settings
- Checks that Python backend is reachable (health check)
- Starts the MCP server
- ~60 lines

```rust
//! Grafyn MCP Server Binary
//!
//! Thin proxy that forwards MCP tool calls to the Python backend via HTTP.
//!
//! Usage:
//!   grafyn-mcp [--url <backend-url>] [--vault <path>] [--data <path>]
//!
//! If --url is not specified, starts the Python backend as a subprocess
//! on a random port, or reads the URL from a lock file.

#![allow(dead_code)]

mod mcp_tools;

use crate::mcp_tools::GrafynMcpServer;
use clap::Parser;
use rmcp::ServiceExt;

#[derive(Parser, Debug)]
#[command(name = "grafyn-mcp", version, about)]
struct Args {
    /// URL of the Python backend (e.g., http://127.0.0.1:8765)
    #[arg(long, default_value = "http://127.0.0.1:8765")]
    url: String,

    /// Path to the vault directory (passed to Python backend if starting it)
    #[arg(long)]
    vault: Option<std::path::PathBuf>,

    /// Path to the data directory (passed to Python backend if starting it)
    #[arg(long)]
    data: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Stderr)
        .init();

    let args = Args::parse();

    // Health check the Python backend
    log::info!("Connecting to backend at {}", args.url);
    let client = reqwest::Client::new();
    match client.get(format!("{}/health", args.url)).send().await {
        Ok(resp) if resp.status().is_success() => {
            log::info!("Backend is healthy");
        }
        Ok(resp) => {
            log::error!("Backend returned {}", resp.status());
            return Err("Backend not healthy".into());
        }
        Err(e) => {
            log::error!("Cannot reach backend at {}: {}", args.url, e);
            log::error!("Make sure the Grafyn app or Python server is running.");
            return Err(e.into());
        }
    }

    let server = GrafynMcpServer::new(args.url);

    log::info!("Starting Grafyn MCP server on stdio...");
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .inspect_err(|e| log::error!("MCP server error: {}", e))?;

    service.waiting().await?;
    log::info!("MCP server shutting down.");
    Ok(())
}
```

---

## Task 3: Update Cargo.toml MCP Feature

Since the MCP binary no longer needs services or models:

```toml
[[bin]]
name = "grafyn-mcp"
path = "src/mcp.rs"
required-features = ["mcp"]

[features]
mcp = ["dep:rmcp", "dep:schemars", "dep:clap"]
# No longer needs: tantivy, petgraph, gray_matter, walkdir, etc.
```

After Phase 4, the shared `services/` and `models/` modules are already stripped down. The MCP binary no longer imports them at all — it only uses `mcp_tools.rs` which only uses `reqwest` and `rmcp`.

### Remove from `mcp.rs`:
```rust
// DELETE these:
mod models;
mod services;

use crate::services::graph_index::GraphIndex;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::memory::MemoryService;
use crate::services::search::SearchService;
use crate::services::settings::SettingsService;
```

---

## Task 4: Update MCP Config in Settings UI

The MCP config snippet in `SettingsModal.vue` and `commands/mcp.rs` needs to pass the backend URL:

### Before:
```json
{
  "mcpServers": {
    "grafyn": {
      "command": "path/to/grafyn-mcp",
      "args": ["--vault", "path/to/vault", "--data", "path/to/data"]
    }
  }
}
```

### After:
```json
{
  "mcpServers": {
    "grafyn": {
      "command": "path/to/grafyn-mcp",
      "args": ["--url", "http://127.0.0.1:8765"]
    }
  }
}
```

**Note:** When the Grafyn desktop app is running, the sidecar port is dynamic. Two options:
1. Write the port to a file (`~/.config/Grafyn/sidecar.port`) that `grafyn-mcp` reads
2. Use a fixed port (e.g., 8765) for the sidecar

Recommend option 1 for flexibility, with option 2 as fallback.

### Update `commands/mcp.rs`:

```rust
#[tauri::command]
pub async fn get_mcp_config_snippet(state: State<'_, AppState>) -> Result<String, String> {
    let sidecar = state.sidecar.read().await;
    let mcp_binary = get_mcp_binary_path();
    let url = sidecar.base_url();

    Ok(format!(
        r#"{{
  "mcpServers": {{
    "grafyn": {{
      "command": "{}",
      "args": ["--url", "{}"]
    }}
  }}
}}"#,
        mcp_binary.display(),
        url
    ))
}
```

---

## Task 5: Backend Discovery for Standalone MCP

When `grafyn-mcp` runs without the desktop app (e.g., user manually configured Claude Desktop), it needs to find the Python backend. Strategy:

1. Check `--url` CLI arg (explicit)
2. Check `~/.config/Grafyn/sidecar.json` for `{ "port": 8765, "pid": 12345 }` (written by sidecar manager)
3. Try `http://127.0.0.1:8765` (default port)
4. If nothing works, print error with instructions

```rust
async fn resolve_backend_url(args: &Args) -> Result<String, String> {
    // 1. Explicit CLI arg
    if !args.url.is_empty() {
        return Ok(args.url.clone());
    }

    // 2. Read from sidecar lock file
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("Grafyn");
    let lock_file = config_dir.join("sidecar.json");
    if lock_file.exists() {
        if let Ok(content) = std::fs::read_to_string(&lock_file) {
            if let Ok(info) = serde_json::from_str::<SidecarInfo>(&content) {
                let url = format!("http://127.0.0.1:{}", info.port);
                // Verify it's alive
                let client = reqwest::Client::new();
                if let Ok(resp) = client.get(format!("{}/health", url)).send().await {
                    if resp.status().is_success() {
                        return Ok(url);
                    }
                }
            }
        }
    }

    // 3. Default
    Ok("http://127.0.0.1:8765".to_string())
}
```

---

## Files Modified
| File | Action |
|------|--------|
| `src/mcp.rs` | **Rewrite** — thin proxy entry point (~60 lines) |
| `src/mcp_tools.rs` | **Rewrite** — HTTP proxy calls instead of direct service calls |
| `src/commands/mcp.rs` | **Edit** — update config snippet to use `--url` |
| `src/services/sidecar.rs` | **Edit** — write port to `sidecar.json` for MCP discovery |
| `Cargo.toml` | **Edit** — MCP feature no longer pulls in service dependencies |

## Estimated Impact
- **Binary size**: ~10MB → ~3-5MB (no Tantivy, petgraph, etc.)
- **Compile time**: Much faster (MCP binary only needs rmcp + reqwest)
- **Code**: ~450 lines → ~200 lines
- **Concurrent access**: No more read-only fallback concern — all goes through HTTP

## Validation
- `cargo build --release --bin grafyn-mcp --no-default-features --features mcp` succeeds
- `grafyn-mcp --url http://127.0.0.1:8765` connects to running Python backend
- All 9 MCP tools work in Claude Desktop
- MCP config snippet in Settings UI shows correct `--url` syntax
- Standalone MCP (without desktop app) reads `sidecar.json` correctly
