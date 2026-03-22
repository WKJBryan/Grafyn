<p align="center">
  <img src="frontend/src-tauri/icons/icon.png" alt="Grafyn" width="128" height="128">
</p>

<h1 align="center">Grafyn</h1>

<p align="center">
  A desktop knowledge graph with full-text search, multi-LLM canvas, and MCP integration.
  <br>
  <strong>Windows</strong> &middot; <strong>macOS</strong> &middot; <strong>Linux</strong>
</p>

<p align="center">
  <a href="https://github.com/WKJBryan/Grafyn/releases/latest"><img src="https://img.shields.io/github/v/release/WKJBryan/Grafyn?style=flat-square&color=blue" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-green?style=flat-square" alt="License: GPL-3.0"></a>
  <a href="https://github.com/WKJBryan/Grafyn/actions/workflows/test.yml"><img src="https://img.shields.io/github/actions/workflow/status/WKJBryan/Grafyn/test.yml?branch=main&style=flat-square&label=tests" alt="Tests"></a>
  <a href="https://github.com/WKJBryan/Grafyn/releases"><img src="https://img.shields.io/github/downloads/WKJBryan/Grafyn/total?style=flat-square&color=orange" alt="Downloads"></a>
</p>

<p align="center">
  <a href="https://github.com/WKJBryan/Grafyn/releases/latest">Download</a> &middot;
  <a href="#features">Features</a> &middot;
  <a href="#quick-start">Quick Start</a> &middot;
  <a href="#mcp-integration">MCP for Claude</a>
</p>

> **Early development** — expect rough edges. Bug reports welcome via [Issues](https://github.com/WKJBryan/Grafyn/issues).

---

## Features

### Knowledge Management
- **Markdown notes** with `[[wikilinks]]` and YAML frontmatter (Obsidian-compatible)
- **Full-text search** powered by Tantivy with graph-aware ranking
- **Backlink graph** with force-directed D3.js visualization
- **Note workflows** — Draft, Evidence, Canonical status progression

### Multi-LLM Canvas
- **Compare models side-by-side** — send one prompt to GPT-4, Claude, Gemini, and 100+ others simultaneously
- **Real-time streaming** — parallel responses via OpenRouter
- **Semantic note context** — automatically retrieves relevant notes as LLM context
- **Debate mode** — models critique and build on each other's responses
- **Infinite canvas** — drag, zoom, and pan with D3.js
- **Smart web search** — auto-detects queries that benefit from live web results

### AI-Powered Tools
- **Distillation** — split large notes into atomic notes and topic hubs using LLM or rules-based extraction
- **Link Discovery** — AI suggests connections between notes (Zettelkasten-style)
- **Conversation Import** — bring in chat history from ChatGPT, Claude, Grok, and Gemini

### MCP Integration
- **Native Rust MCP server** bundled with the app for Claude Desktop
- 10 tools: note CRUD, search, backlinks, outgoing links, recall, and conversation import
- Zero-config — copy the config snippet from Grafyn Settings into Claude Desktop

## Quick Start

### Download

Grab the latest installer from [Releases](https://github.com/WKJBryan/Grafyn/releases/latest):

| Platform | File |
|----------|------|
| **Windows (64-bit)** | `Grafyn_*_x64-setup.exe` |
| **Windows (ARM)** | `Grafyn_*_arm64-setup.exe` |
| **macOS (Apple Silicon)** | `Grafyn_*_aarch64.dmg` |
| **Linux (Debian/Ubuntu)** | `grafyn_*_amd64.deb` |
| **Linux (Universal)** | `grafyn_*_amd64.AppImage` |

Grafyn auto-updates after installation.

### Build from Source

**Prerequisites:** Node.js 20+, Rust via [rustup](https://rustup.rs/), and [Tauri v1 dependencies](https://v1.tauri.app/v1/guides/getting-started/prerequisites).

```bash
cd frontend
npm install
node scripts/generate-icons.cjs    # first build only

npm run tauri:dev                   # dev mode with hot reload
npm run tauri:build                 # production build
```

### Configuration

On first launch, Grafyn walks you through setup:
1. **Vault path** — where your markdown notes are stored (default: `~/Documents/Grafyn/vault/`)
2. **OpenRouter API key** — required for Canvas, distillation, and link discovery ([get one here](https://openrouter.ai/keys))

All data stays on your machine. No account needed.

## MCP Integration

Connect Claude Desktop to your knowledge base:

1. Open Grafyn **Settings** and copy the MCP config snippet
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

The MCP server is a standalone Rust binary (`grafyn-mcp`) bundled with the installer. It shares the same vault and search index as the desktop app.

## Architecture

Single binary, no server — Tauri wraps a Vue 3 frontend with a Rust backend.

```
Tauri Desktop App
├── Vue 3 Frontend (WebView)
│   └── Tauri IPC (invoke)
├── Rust Backend
│   ├── 65 IPC commands across 13 modules
│   ├── Tantivy full-text search
│   ├── petgraph link graph
│   └── OpenRouter LLM integration
├── grafyn-mcp (bundled MCP server)
└── ~/Documents/Grafyn/
    ├── vault/  (markdown notes)
    └── data/   (search index, canvas sessions, settings)
```

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Frontend | Vue 3, Vite, Pinia, D3.js |
| Desktop | Tauri 1.8 |
| Search | Tantivy 0.22 |
| Graph | petgraph 0.6 |
| LLM | OpenRouter via reqwest |
| MCP | rmcp (stdio transport) |
| Updates | Cloudflare R2 + Workers |

## Contributing

1. Fork and create a feature branch
2. Make changes and add tests
3. Run `cargo test` and `npm run test:run`
4. Submit a pull request

See [CLAUDE.md](CLAUDE.md) for detailed architecture docs, IPC command reference, and CI pitfalls.

## License

[GPL-3.0](LICENSE)
