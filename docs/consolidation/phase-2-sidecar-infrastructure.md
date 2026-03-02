# Phase 2: Sidecar Infrastructure

## Goal
Bundle the Python backend as a compiled PyInstaller binary (`grafyn-server`) and add Tauri sidecar management so the desktop app auto-launches it on startup. After this phase, the desktop app has **two backends** running simultaneously — Rust (IPC) and Python (HTTP on localhost).

## Why PyInstaller
- Single-file binary, no Python runtime needed on user's machine
- Already used in the project (see `backend/grafyn.spec` reference in MEMORY.md)
- Cross-platform: builds on Windows, macOS, Linux
- The `sentence-transformers` + `torch` dependencies make the binary large (~300-500MB), but it's a one-time download bundled in the installer

---

## Task 1: PyInstaller Build Configuration

### 1a. Create `backend/grafyn-server.spec`

```python
# PyInstaller spec for Grafyn sidecar backend
# Builds a single-directory distribution (not --onefile, since torch is large)
# Output: dist/grafyn-server/grafyn-server.exe (or grafyn-server on Unix)

import sys
from pathlib import Path

block_cipher = None
backend_dir = Path(SPECPATH)

a = Analysis(
    [str(backend_dir / 'app' / 'main.py')],
    pathex=[str(backend_dir)],
    binaries=[],
    datas=[
        # Include .env.example as fallback config
        (str(backend_dir / '.env.example'), '.'),
    ],
    hiddenimports=[
        'uvicorn.logging',
        'uvicorn.loops.auto',
        'uvicorn.protocols.http.auto',
        'uvicorn.protocols.websockets.auto',
        'uvicorn.lifespan.on',
        'app.routers.notes',
        'app.routers.search',
        'app.routers.graph',
        'app.routers.canvas',
        'app.routers.distill',
        'app.routers.feedback',
        'app.routers.memory',
        'app.routers.settings',
        'app.routers.chat',
        'app.routers.conversation_import',
        'app.routers.zettelkasten',
        'app.routers.priority',
        'sentence_transformers',
        'lancedb',
    ],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[
        'tkinter', 'matplotlib', 'PIL',  # Not needed, saves ~50MB
    ],
    noarchive=False,
)

pyz = PYZ(a.pure, a.zipped_data, cipher=block_cipher)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name='grafyn-server',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    console=True,  # No GUI — runs as background process
    target_arch=None,
)

coll = COLLECT(
    exe,
    a.binaries,
    a.zipfiles,
    a.datas,
    strip=False,
    upx=True,
    upx_exclude=[],
    name='grafyn-server',
)
```

### 1b. Build script: `backend/build-sidecar.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail

# Build grafyn-server sidecar binary using PyInstaller
# Usage: ./build-sidecar.sh [--target <triple>]
# Output: dist/grafyn-server/

cd "$(dirname "$0")"

echo "Installing PyInstaller..."
pip install pyinstaller

echo "Building grafyn-server..."
pyinstaller grafyn-server.spec --noconfirm

echo "Build complete: dist/grafyn-server/"
ls -lh dist/grafyn-server/grafyn-server*
```

### 1c. Test locally

```bash
cd backend
pip install pyinstaller
pyinstaller grafyn-server.spec --noconfirm
# Run the built binary:
dist/grafyn-server/grafyn-server --port 8765 --host 127.0.0.1 --no-reload
```

---

## Task 2: Tauri Sidecar Management

### 2a. Update `tauri.conf.json`

Add `grafyn-server` as a sidecar alongside `grafyn-mcp`:

```json
{
  "tauri": {
    "allowlist": {
      "shell": {
        "sidecar": true
      }
    },
    "bundle": {
      "externalBin": [
        "binaries/grafyn-mcp",
        "binaries/grafyn-server"
      ]
    }
  }
}
```

**Note:** Tauri v1 sidecar naming convention requires platform-specific suffixes. The binary in `binaries/` must be named `grafyn-server-x86_64-pc-windows-msvc.exe` (etc.) for Tauri to resolve it. The PyInstaller build step must copy the output with the correct suffix.

### 2b. Create `frontend/src-tauri/src/services/sidecar.rs`

```rust
//! Sidecar lifecycle management for grafyn-server (Python backend).
//!
//! Starts the Python backend as a child process on a random port,
//! waits for health check, provides the base URL to the frontend.

use std::sync::Arc;
use tokio::sync::RwLock;
use tauri::api::process::{Command, CommandChild};

pub struct SidecarManager {
    child: Option<CommandChild>,
    port: u16,
    base_url: String,
}

impl SidecarManager {
    pub fn new() -> Self {
        Self {
            child: None,
            port: 0,
            base_url: String::new(),
        }
    }

    /// Start the Python sidecar on a random available port.
    /// Passes vault-path and data-path from settings.
    pub async fn start(
        &mut self,
        vault_path: &str,
        data_path: &str,
    ) -> Result<String, String> {
        // Find available port
        let port = find_available_port().map_err(|e| e.to_string())?;
        self.port = port;
        self.base_url = format!("http://127.0.0.1:{}", port);

        let (mut rx, child) = Command::new_sidecar("grafyn-server")
            .map_err(|e| format!("Failed to create sidecar command: {}", e))?
            .args([
                "--port", &port.to_string(),
                "--host", "127.0.0.1",
                "--vault-path", vault_path,
                "--data-path", data_path,
                "--no-reload",
                "--environment", "production",
            ])
            .spawn()
            .map_err(|e| format!("Failed to spawn sidecar: {}", e))?;

        self.child = Some(child);

        // Log sidecar output in background
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                match event {
                    tauri::api::process::CommandEvent::Stdout(line) => {
                        log::info!("[sidecar] {}", line);
                    }
                    tauri::api::process::CommandEvent::Stderr(line) => {
                        log::warn!("[sidecar] {}", line);
                    }
                    _ => {}
                }
            }
        });

        // Wait for health check (poll /health every 200ms, timeout 15s)
        self.wait_for_health(15000).await?;

        log::info!("Sidecar started on {}", self.base_url);
        Ok(self.base_url.clone())
    }

    /// Poll the health endpoint until the server is ready.
    async fn wait_for_health(&self, timeout_ms: u64) -> Result<(), String> {
        let client = reqwest::Client::new();
        let start = std::time::Instant::now();

        loop {
            if start.elapsed().as_millis() as u64 > timeout_ms {
                return Err("Sidecar health check timed out".to_string());
            }

            match client.get(format!("{}/health", self.base_url)).send().await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                _ => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
            }
        }
    }

    /// Stop the sidecar process.
    pub fn stop(&mut self) {
        if let Some(child) = self.child.take() {
            let _ = child.kill();
            log::info!("Sidecar stopped");
        }
    }

    /// Get the base URL of the running sidecar.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Check if the sidecar is running.
    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for SidecarManager {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Find an available TCP port.
fn find_available_port() -> Result<u16, std::io::Error> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}
```

### 2c. Add `SidecarManager` to `AppState`

In `main.rs`, add the sidecar to state and start it during `.setup()`:

```rust
pub struct AppState {
    // ... existing fields ...
    pub sidecar: Arc<RwLock<SidecarManager>>,
}

// In setup:
let mut sidecar = SidecarManager::new();
let vault_path_str = vault_path.to_string_lossy().to_string();
let data_path_str = data_path.to_string_lossy().to_string();

// Start sidecar (non-blocking — app shows UI immediately, sidecar boots in background)
let sidecar_arc = Arc::new(RwLock::new(sidecar));
let sidecar_clone = sidecar_arc.clone();
tokio::spawn(async move {
    let mut s = sidecar_clone.write().await;
    match s.start(&vault_path_str, &data_path_str).await {
        Ok(url) => log::info!("Python sidecar ready at {}", url),
        Err(e) => log::error!("Failed to start sidecar: {}", e),
    }
});
```

### 2d. Sidecar Tauri commands

```rust
// commands/sidecar.rs

#[tauri::command]
pub async fn get_sidecar_status(state: State<'_, AppState>) -> Result<SidecarStatus, String> {
    let sidecar = state.sidecar.read().await;
    Ok(SidecarStatus {
        running: sidecar.is_running(),
        base_url: sidecar.base_url().to_string(),
    })
}

#[tauri::command]
pub async fn get_sidecar_url(state: State<'_, AppState>) -> Result<String, String> {
    let sidecar = state.sidecar.read().await;
    if sidecar.is_running() {
        Ok(sidecar.base_url().to_string())
    } else {
        Err("Sidecar not running".to_string())
    }
}
```

### 2e. Frontend: Expose sidecar URL

Add to `client.js`:

```javascript
// Sidecar API (Desktop only)
export const sidecar = {
  getStatus: () => invokeOrHttp('get_sidecar_status', {}, () => Promise.resolve({ running: false })),
  getUrl: () => invokeOrHttp('get_sidecar_url', {}, () => Promise.resolve(window.location.origin)),
}
```

---

## Task 3: Graceful Shutdown

Ensure the sidecar is stopped when the Tauri app closes:

```rust
// In main.rs, add on_exit handler:
.on_window_event(|event| {
    if let tauri::WindowEvent::Destroyed = event.event() {
        // Stop sidecar when last window closes
        let state = event.window().state::<AppState>();
        let sidecar = state.sidecar.clone();
        tauri::async_runtime::block_on(async {
            let mut s = sidecar.write().await;
            s.stop();
        });
    }
})
```

---

## Files Modified
| File | Action |
|------|--------|
| `backend/grafyn-server.spec` | **Create** — PyInstaller build config |
| `backend/build-sidecar.sh` | **Create** — build script |
| `frontend/src-tauri/tauri.conf.json` | **Edit** — enable sidecar, add grafyn-server to externalBin |
| `frontend/src-tauri/src/services/sidecar.rs` | **Create** — sidecar lifecycle management |
| `frontend/src-tauri/src/services/mod.rs` | **Edit** — add `pub mod sidecar;` |
| `frontend/src-tauri/src/main.rs` | **Edit** — add SidecarManager to AppState, start in setup, stop on exit |
| `frontend/src-tauri/src/commands/sidecar.rs` | **Create** — status/URL commands |
| `frontend/src-tauri/src/commands/mod.rs` | **Edit** — add `pub mod sidecar;` |
| `frontend/src/api/client.js` | **Edit** — add sidecar API |

## Validation
- `pyinstaller grafyn-server.spec` succeeds on all platforms
- Built binary starts: `dist/grafyn-server/grafyn-server --port 8765` → `GET /health` returns 200
- Tauri dev mode starts sidecar automatically, logs show "Python sidecar ready at http://127.0.0.1:XXXXX"
- Closing the app stops the sidecar process (no orphan)
- `get_sidecar_url` returns correct URL from frontend

## Size Considerations
- PyInstaller binary with sentence-transformers + torch: ~300-500MB
- This is the biggest concern — consider options:
  1. **Accept it** — users download once, auto-updates are delta
  2. **Strip torch** — use ONNX runtime instead (~50MB), requires model conversion
  3. **Lazy download** — ship without embedding model, download on first use
- Recommend option 1 for v1, revisit if bundle size is a real user complaint
