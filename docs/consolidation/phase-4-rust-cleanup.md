# Phase 4: Rust Cleanup

## Goal
Delete all Rust business logic that's now handled by the Python sidecar. Strip `main.rs` to a thin shell that manages the sidecar, settings, MCP config, and native dialogs. Remove heavy Cargo dependencies.

## Why This Phase Is Satisfying
This is where you delete ~5000 lines of Rust and watch the compile time drop from minutes to seconds. The Tauri app becomes a thin launcher — it starts the sidecar and provides a webview.

---

## Task 1: Delete Command Modules

### Commands to DELETE (business logic now in Python):
| Module | File | Commands | Lines |
|--------|------|----------|-------|
| `notes` | `commands/notes.rs` | 5 | ~150 |
| `search` | `commands/search.rs` | 3 | ~100 |
| `graph` | `commands/graph.rs` | 6 | ~200 |
| `canvas` | `commands/canvas.rs` | 18 | ~900 |
| `distill` | `commands/distill.rs` | 2 | ~150 |
| `memory` | `commands/memory.rs` | 3 | ~100 |
| `feedback` | `commands/feedback.rs` | 6 | ~250 |

### Commands to KEEP (native OS / sidecar management):
| Module | File | Commands | Reason |
|--------|------|----------|--------|
| `settings` | `commands/settings.rs` | 7 | Native file dialog, sidecar sync |
| `mcp` | `commands/mcp.rs` | 2 | Reads local binary path |
| `sidecar` | `commands/sidecar.rs` | 2 | Sidecar status/URL (new in Phase 2) |

### Update `commands/mod.rs`:

```rust
// BEFORE:
pub mod canvas;
pub mod distill;
pub mod feedback;
pub mod graph;
pub mod mcp;
pub mod memory;
pub mod notes;
pub mod search;
pub mod settings;

// AFTER:
pub mod mcp;
pub mod settings;
pub mod sidecar;
```

---

## Task 2: Delete Service Modules

### Services to DELETE:
| Service | File | Lines |
|---------|------|-------|
| `KnowledgeStore` | `services/knowledge_store.rs` | ~400 |
| `SearchService` | `services/search.rs` | ~350 |
| `GraphIndex` | `services/graph_index.rs` | ~300 |
| `CanvasStore` | `services/canvas_store.rs` | ~400 |
| `OpenRouterService` | `services/openrouter.rs` | ~500 |
| `FeedbackService` | `services/feedback.rs` | ~300 |
| `MemoryService` | `services/memory.rs` | ~200 |

### Services to KEEP:
| Service | File | Reason |
|---------|------|--------|
| `SettingsService` | `services/settings.rs` | Reads settings.json for sidecar args |
| `SidecarManager` | `services/sidecar.rs` | Manages Python process (new in Phase 2) |

### Update `services/mod.rs`:

```rust
pub mod settings;
pub mod sidecar;
```

---

## Task 3: Delete Model Modules

### Models to DELETE:
| Model | File |
|-------|------|
| `note` | `models/note.rs` |
| `canvas` | `models/canvas.rs` |
| `feedback` | `models/feedback.rs` |
| `memory` | `models/memory.rs` |

### Models to KEEP:
| Model | File | Reason |
|-------|------|--------|
| `settings` | `models/settings.rs` | Used by SettingsService |

### Update `models/mod.rs`:

```rust
pub mod settings;
```

---

## Task 4: Simplify `main.rs`

### Before (242 lines):
- Initializes 8 services
- Loads all notes, builds indices in parallel
- Registers 52 commands
- Compile-time feedback credentials

### After (~80 lines):
```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod models;
mod services;

use services::settings::SettingsService;
use services::sidecar::SidecarManager;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;

pub struct AppState {
    pub settings_service: Arc<RwLock<SettingsService>>,
    pub sidecar: Arc<RwLock<SidecarManager>>,
}

fn main() {
    env_logger::init();

    tauri::Builder::default()
        .setup(|app| {
            let settings_service = SettingsService::load().unwrap_or_else(|e| {
                log::error!("Failed to load settings: {}. Using defaults.", e);
                SettingsService::load_defaults()
            });

            let vault_path = settings_service.vault_path();
            let data_path = settings_service.data_path();

            // Create app state
            let sidecar = SidecarManager::new();
            let state = AppState {
                settings_service: Arc::new(RwLock::new(settings_service)),
                sidecar: Arc::new(RwLock::new(sidecar)),
            };

            app.manage(state);

            // Start sidecar in background
            let state_handle = app.state::<AppState>().clone();
            let vault_str = vault_path.to_string_lossy().to_string();
            let data_str = data_path.to_string_lossy().to_string();
            tokio::spawn(async move {
                let mut s = state_handle.sidecar.write().await;
                if let Err(e) = s.start(&vault_str, &data_str).await {
                    log::error!("Failed to start sidecar: {}", e);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Settings (native dialogs + sync)
            commands::settings::get_settings,
            commands::settings::get_settings_status,
            commands::settings::update_settings,
            commands::settings::complete_setup,
            commands::settings::pick_vault_folder,
            commands::settings::validate_openrouter_key,
            commands::settings::get_openrouter_status,
            // MCP config
            commands::mcp::get_mcp_status,
            commands::mcp::get_mcp_config_snippet,
            // Sidecar management
            commands::sidecar::get_sidecar_status,
            commands::sidecar::get_sidecar_url,
        ])
        .on_window_event(|event| {
            if let tauri::WindowEvent::Destroyed = event.event() {
                let state = event.window().state::<AppState>();
                let sidecar = state.sidecar.clone();
                tauri::async_runtime::block_on(async {
                    let mut s = sidecar.write().await;
                    s.stop();
                });
            }
        })
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            log::error!("Error while running tauri application: {}", e);
            std::process::exit(1);
        });
}
```

---

## Task 5: Simplify `Cargo.toml`

### Dependencies to REMOVE:
```toml
# No longer needed — business logic moved to Python
gray_matter = "0.2"       # Frontmatter parsing
walkdir = "2.4"           # File traversal
notify = "6.1"            # File watching
serde_yaml = "0.9"        # YAML parsing
tantivy = "0.22"          # Full-text search (BIG dependency)
petgraph = "0.6"          # Graph data structure
regex = "1.10"            # Wikilink parsing
lazy_static = "1.4"       # Static regex
chrono = "0.4"            # Date/time
uuid = "1.6"              # UUID generation
futures = "0.3"           # Async streams
```

### Dependencies to KEEP:
```toml
tauri = "1.8"             # Core framework
serde = "1.0"             # Serialization (for settings)
serde_json = "1.0"        # JSON (for settings)
tokio = "1.0"             # Async runtime (for sidecar)
reqwest = "0.12"          # HTTP client (health check, OpenRouter validation)
anyhow = "1.0"            # Error handling
log = "0.4"               # Logging
env_logger = "0.11"       # Log output
dirs = "5.0"              # System paths
```

### Estimated compile time improvement:
- `tantivy` alone is ~60s compile time — removing it is a massive win
- `sentence-transformers` was never in Rust, but `petgraph` + `regex` + `chrono` add up
- Expected: **3-5x faster** Rust compilation

---

## Task 6: Update `settings.rs` Commands

The `update_settings` command currently syncs `KnowledgeStore`, `SearchService`, `GraphIndex`, and `OpenRouterService`. After cleanup, it only needs to:
1. Write `settings.json`
2. Restart the sidecar with new paths (if vault path changed)

```rust
#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    update: SettingsUpdate,
) -> Result<UserSettings, String> {
    let vault_path_changed = update.vault_path.is_some();

    let mut settings = state.settings_service.write().await;
    let result = settings.update(update).map_err(|e| e.to_string())?;

    let new_vault = if vault_path_changed {
        Some(settings.vault_path())
    } else {
        None
    };
    let new_data = if vault_path_changed {
        Some(settings.data_path())
    } else {
        None
    };
    drop(settings);

    // Restart sidecar if vault path changed
    if let (Some(vault), Some(data)) = (new_vault, new_data) {
        let mut sidecar = state.sidecar.write().await;
        sidecar.stop();
        if let Err(e) = sidecar.start(
            &vault.to_string_lossy(),
            &data.to_string_lossy(),
        ).await {
            log::error!("Failed to restart sidecar: {}", e);
        }
    }

    Ok(result)
}
```

---

## Files Modified
| File | Action |
|------|--------|
| `src/commands/notes.rs` | **Delete** |
| `src/commands/search.rs` | **Delete** |
| `src/commands/graph.rs` | **Delete** |
| `src/commands/canvas.rs` | **Delete** |
| `src/commands/distill.rs` | **Delete** |
| `src/commands/memory.rs` | **Delete** |
| `src/commands/feedback.rs` | **Delete** |
| `src/commands/mod.rs` | **Rewrite** — keep mcp, settings, sidecar |
| `src/services/knowledge_store.rs` | **Delete** |
| `src/services/search.rs` | **Delete** |
| `src/services/graph_index.rs` | **Delete** |
| `src/services/canvas_store.rs` | **Delete** |
| `src/services/openrouter.rs` | **Delete** |
| `src/services/feedback.rs` | **Delete** |
| `src/services/memory.rs` | **Delete** |
| `src/services/mod.rs` | **Rewrite** — keep settings, sidecar |
| `src/models/note.rs` | **Delete** |
| `src/models/canvas.rs` | **Delete** |
| `src/models/feedback.rs` | **Delete** |
| `src/models/memory.rs` | **Delete** |
| `src/models/mod.rs` | **Rewrite** — keep settings |
| `src/main.rs` | **Rewrite** — thin shell |
| `Cargo.toml` | **Edit** — remove 12 dependencies |

## Estimated Deletions
- **~5000 lines of Rust** deleted
- **12 Cargo dependencies** removed
- **7 command modules**, **7 service modules**, **4 model modules** removed
- Compile time: 3-5x faster

## Validation
- `cargo build` succeeds with no errors
- `cargo clippy` clean
- `npm run tauri:dev` launches, sidecar starts, app works
- All 52 original commands verified to work via HTTP
- Settings changes restart sidecar correctly
- MCP status still works
