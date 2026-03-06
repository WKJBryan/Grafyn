# Grafyn

A desktop knowledge graph platform with full-text search, Obsidian-style linking, Multi-LLM Canvas, and MCP (Model Context Protocol) integration for Claude Desktop.

## Features

### Core Knowledge Management
- **Obsidian-compatible notes** — Markdown files with YAML frontmatter and `[[wikilinks]]`
- **Full-text search** — Tantivy BM25 keyword search with graph-aware similarity ranking
- **Backlinks & graph** — Automatic bidirectional link tracking with force-directed visualization
- **Note workflows** — Draft → Evidence → Canonical status progression

### Multi-LLM Canvas
- **Compare AI models** — Send one prompt to multiple models simultaneously
- **Real-time streaming** — Parallel response streaming from 100+ models via OpenRouter
- **Semantic note context** — Automatically retrieves relevant notes as LLM context
- **Infinite canvas** — D3.js-powered zoom/pan interface with draggable tiles
- **Debate mode** — Models critique and respond to each other
- **Export to notes** — Convert canvas sessions to knowledge base notes

### AI Integration
- **MCP Server** — Native Rust binary for Claude Desktop integration (stdio transport)
- **Conversation Import** — Import chat history from ChatGPT, Claude, Grok, and Gemini
- **Distillation** — Transform container notes into atomic notes and topic hubs (configurable LLM model)
- **Link Discovery** — AI-powered suggestions for connecting related notes (Zettelkasten)

### Feedback & Bug Reporting
- **In-app feedback** — Submit bug reports, feature requests, and general feedback
- **GitHub Issues integration** — Submissions automatically create GitHub issues
- **Offline support** — Feedback queued when offline, auto-retries on reconnect

## Tech Stack

### Frontend (Vue 3)
- **Framework**: Vue 3.4+ with Composition API
- **State Management**: Pinia
- **Build Tool**: Vite 5.0+
- **Visualization**: D3.js v7+
- **Markdown**: marked 11.0+

### Desktop Backend (Tauri + Rust)
- **Framework**: Tauri 1.8
- **Search Engine**: Tantivy 0.22
- **Graph**: petgraph 0.6
- **LLM API**: OpenRouter via reqwest
- **MCP**: rmcp 0.15 (stdio transport)
- **Async Runtime**: tokio 1.0

## Quick Start

### Prerequisites

- Node.js 18+
- Rust via [rustup](https://rustup.rs/)
- Platform-specific build tools:
  - **Windows**: Visual Studio Build Tools 2022 with C++ workload
  - **macOS**: Xcode Command Line Tools (`xcode-select --install`)
  - **Linux**: `sudo apt install build-essential libgtk-3-dev libwebkit2gtk-4.0-dev`

### Setup

```bash
cd frontend
npm install

# Generate app icons (required for first build)
node scripts/generate-icons.cjs

# Development mode with hot reload
npm run tauri:dev

# Production build
npm run tauri:build
```

**Build output:**
- Windows: `src-tauri/target/release/bundle/nsis/Grafyn_0.1.1_x64-setup.exe`
- macOS: `src-tauri/target/release/bundle/dmg/Grafyn_0.1.1_aarch64.dmg`
- Linux: `src-tauri/target/release/bundle/deb/grafyn_0.1.1_amd64.deb`

## Configuration

### Environment Variables

| Variable | Description |
|----------|-------------|
| `OPENROUTER_API_KEY` | Required for Multi-LLM Canvas, distillation, link discovery |
| `GITHUB_FEEDBACK_REPO` | Target repo for feedback issues (format: `owner/repo`) |
| `GITHUB_FEEDBACK_TOKEN` | GitHub PAT with `issues:write` scope |
| `RUST_LOG` | Logging level (default: `info`) |

### In-App Settings

The Settings UI (`Settings` button in the header) manages:
- **Vault path** — location of your markdown notes folder
- **OpenRouter API key** — for LLM features
- **LLM model** — model used for distillation and link discovery (default: `anthropic/claude-3.5-haiku`)
- **Theme** — light, dark, or system

## Usage

### Notes

1. Click **"+ New Note"** in the header
2. Write content in Markdown with `[[wikilinks]]` to other notes
3. Set status (draft/evidence/canonical) and add tags
4. Save — note is automatically indexed for search

### Search

- **Full-text search**: Type naturally in the search bar
- **Operators**: `tag:python`, `status:canonical`, `type:atomic`
- **Filters**: Include/exclude tags with `+tag` or `-tag`

### Multi-LLM Canvas

1. Navigate to the Canvas tab
2. Create a new session
3. Click **"+ Prompt"** to open the prompt dialog
4. Select models to compare (e.g., GPT-4, Claude, Gemini)
5. Enter your prompt — responses stream in real-time with relevant notes as context
6. Use **Debate Mode** to have models critique each other
7. Export insights to your knowledge base

### Conversation Import

1. Export conversations from ChatGPT, Claude, Grok, or Gemini
2. Navigate to the Import tab
3. Upload the JSON file
4. Review parsed conversations with quality scores
5. Select conversations to import as notes

### MCP Integration (Claude Desktop)

Connect Claude Desktop to your knowledge base using the bundled MCP server:

1. Open Grafyn Settings → copy the MCP config snippet
2. Add it to your `claude_desktop_config.json`:

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

The MCP server provides 10 tools: note CRUD, search, backlinks, outgoing links, recall, and conversation import.

### Feedback & Bug Reporting

1. Click the feedback button in the header
2. Select feedback type (Bug Report, Feature Request, or General)
3. Enter a title and description
4. Submit — creates a GitHub issue automatically

Feedback is queued when offline and automatically submitted when connectivity is restored.

## Architecture

```
┌────────────────────────────────────────────────┐
│           Tauri Desktop App (Single Binary)     │
│  ┌──────────────────────────────────────────┐  │
│  │         Vue 3 Frontend (WebView)          │  │
│  └──────────────────┬───────────────────────┘  │
│                     │ Tauri IPC (invoke)        │
│  ┌──────────────────▼───────────────────────┐  │
│  │            Rust Backend                   │  │
│  │  Commands → Services → Local Filesystem   │  │
│  │  (notes, search, graph, canvas, distill,   │  │
│  │   settings, feedback, mcp, memory,         │  │
│  │   import, priority, retrieval, zettelkasten)│  │
│  └──────────────────────────────────────────┘  │
│  ~/Documents/Grafyn/                          │
│  ├── vault/  (markdown notes)                   │
│  └── data/   (search index, canvas, settings)   │
└────────────────────────────────────────────────┘
```

### Project Structure

```
frontend/
├── src/
│   ├── views/             # Page components (4 views)
│   ├── components/        # UI components (32 total)
│   │   ├── canvas/        # Canvas-specific (13 components)
│   │   └── import/        # Import-specific (1 component)
│   ├── stores/            # Pinia state (3 stores)
│   └── api/               # Tauri IPC client
└── src-tauri/             # Rust backend
    ├── src/
    │   ├── commands/      # 65 IPC handlers across 13 modules
    │   ├── services/      # Business logic
    │   ├── models/        # Data structures
    │   └── mcp.rs         # MCP server binary entry point
    └── Cargo.toml
```

## Testing

```bash
# Rust tests (49 tests)
cd frontend/src-tauri
cargo test

# Frontend tests (230 tests)
cd frontend
npm run test:run
```

## Security

Desktop app — all data stored locally on disk. No server-side authentication needed.

- **Path traversal protection** — sanitized note IDs, resolved paths
- **Local-only storage** — notes and indexes stay on your machine
- **API key management** — OpenRouter key stored in local app data, never transmitted except to OpenRouter

## CI/CD

### Test Pipeline

`.github/workflows/test.yml` — runs on push to main and PRs: Rust tests, frontend tests, linting (ESLint + Clippy), security audit, and Vite production build.

### Release Pipeline

`.github/workflows/release.yml` — triggered by `v*` tags:

1. **Create release** — single draft GitHub release
2. **Build** — 4-platform matrix (Windows x64/ARM, macOS ARM, Linux x64) builds Tauri bundles with bundled MCP binary
3. **Publish release** — draft → published after all builds complete
4. **Upload to R2** — assets mirrored to Cloudflare R2 for auto-update distribution

Manual builds can be triggered via `workflow_dispatch` without creating a release.

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Write tests for new functionality
5. Run the test suite (`cargo test` and `npm run test:run`)
6. Submit a pull request

### Code Style

- **Frontend**: ESLint + Prettier (configured)
- **Rust**: `cargo fmt` and `cargo clippy`

## License

[Add your license here]

## Support

For issues and questions, please open an issue on GitHub.
