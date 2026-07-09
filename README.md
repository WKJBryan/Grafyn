<p align="center">
  <img src="frontend/src-tauri/icons/icon.png" alt="Grafyn" width="128" height="128">
</p>

<h1 align="center">Grafyn</h1>

<p align="center">
  A desktop knowledge graph and Canvas for capturing how you think, what you know, and how a future digital twin should reason with your evidence.
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
  <a href="#what-grafyn-is">What It Is</a> &middot;
  <a href="#twin-capture-pipeline">Twin Pipeline</a> &middot;
  <a href="#quick-start">Quick Start</a> &middot;
  <a href="#developer-guidelines">Guidelines</a>
</p>

> **Early development** - expect rough edges. Grafyn is currently focused on local evidence capture, knowledge organization, Twin Identity setup, and the first native RAG twin workflow. It is not yet a scratch-trained personal model.

---

## What Grafyn Is

Grafyn is a desktop-only app for building a local knowledge vault and using it inside a multi-model Canvas. The long-term goal is to become the capture layer for a personal digital twin pipeline: users work inside Grafyn, Grafyn records explicit and passive evidence about their knowledge and reasoning patterns, and later twin systems can use that evidence.

The current app builds an evidence-grounded twin prompt from your configured **Twin Identity**, reviewed Constitution, notes, and user records. Simulation mode is intentionally first-person so the selected model can mimic the configured person's documented reasoning pattern more closely.

The first usable twin mode is a **native RAG twin**:

1. Retrieve relevant notes from your vault.
2. Retrieve reviewed user records about your thinking and preferences.
3. Assemble that context into Canvas prompts.
4. Let the chosen model answer in either advisor mode or first-person Simulation mode.
5. Feed your accept/reject/correct/rank feedback back into the evidence loop.

## Core Features

### Knowledge Vault

- Markdown notes with `[[wikilinks]]` and YAML frontmatter.
- Draft, evidence, and canonical note status workflow.
- Full-text search powered by Tantivy.
- Backlinks, outgoing links, and graph-aware retrieval.
- Conversation import from ChatGPT, Claude, Grok, Gemini, and Codex-style exports.
- Structured vault migration with preview, apply, and rollback.

### Knowledge Graph And Hub Clustering

- D3 force-directed graph view for notes and topic hubs.
- Auto-managed topic hubs for broad knowledge areas.
- Deterministic graph-based clustering over explicit links and shared topic/title signals.
- Label-propagation communities so linked note groups become broader hubs instead of many tiny hubs.
- Noise filtering so model/provider names such as `Claude` do not become hubs just because they appear in a prompt.
- Major hubs keep minor recurring tags under a `Subtopics` section instead of exploding the sidebar with one folder per tiny topic.

### Multi-LLM Canvas

- Compare multiple OpenRouter models side by side.
- Stream responses in parallel.
- Branch from model responses.
- Debate mode for model critique and synthesis.
- Graph-aware note context from the vault (Tantivy keyword search + link-graph boosts, not embedding-based retrieval).
- Twin context mode using reviewed user records.
- Smart web search detection for prompts that need current information.
- Save Canvas sessions as notes.
- **Local model support** via Ollama — run Canvas and twin answers on local models alongside OpenRouter cloud models.

### Vault Automation

- **Background link discovery** — scans the vault continuously for potential `[[wikilinks]]` using keyword extraction and similarity, surfaces suggestions in the Link Suggestion inbox.
- **Vault Optimizer** — background service that queues and applies structural improvements to notes (tag cleanup, sidecar metadata, full rewrites). Budget caps and per-change rollback keep it safe to enable.

### Twin Capture And Review

- Canvas feedback controls: `Matches Me`, `Not Me`, `Correct`, `Rank Selection`, `Capture Insight`, and `Export Twin Data`.
- Local evidence capture from feedback, branching, note exports, canonical promotion, debate choices, and related passive signals.
- Local signal inference for `Fact`, `Preference`, and `ReasoningPattern` records.
- Review dashboard at `/twin` for candidate, auto-promoted, endorsed, rejected, private, and no-train records.
- Evidence resolution so records can be traced back to the prompts, sessions, models, and excerpts that supported them.
- Revert/reject support that prevents rejected inference keys from being silently auto-promoted again.

### Native RAG Twin

Canvas supports a `Twin` context mode with two answer modes:

- **Advisor** - decision-support assistant using your reviewed notes and user records.
- **Simulation** - first-person configured twin voice grounded in Twin Identity, reviewed Constitution, notes, and user records.

Twin Simulation requires a **Twin Identity** in Twin Workspace setup:

- `Name` - who the twin speaks as.
- `Role / context` - the role or decision context the twin reasons from.
- `Source boundaries` - optional guidance for which materials define the twin.

Twin context uses:

- Configured Twin Identity.
- Relevant vault notes and chunks.
- Reviewed Constitution items: values, taste, constraints, somatic cues, and action tendencies.
- Approved user records: `endorsed` and `auto_promoted`.
- Relevant candidate records only when they match the prompt, disclosed separately as tentative.

Twin context excludes:

- `rejected`
- `private`
- `no_train`

Rejected records are preserved for export as negative evidence, not used as live answer context.

## Twin Capture Pipeline

Grafyn currently learns in the **evidence and retrieval sense**, not by changing model weights.

```text
User work in Canvas/Notes
        |
        v
Trace events + feedback + note actions
        |
        v
Local signal inference
        |
        v
Evidence-linked user records
        |
        v
Twin Review: endorse / reject / private / no-train
        |
        v
Native RAG twin context + export bundles
```

### Current Stage: Local Evidence And RAG

Grafyn stores what happened and infers specific records such as:

- "Prefers evidence-backed implementation detail."
- "Rejects vague strategic answers."
- "Often asks for blunt tradeoff analysis."

These records are linked to evidence. They are not broad personality labels.

Twin Identity is setup metadata, not an inferred personality label. It defines who the Simulation speaks as; the Constitution defines the operating priors used to reason in that voice.

### Export Contract

Twin exports separate reviewed records into different JSONL files:

- `approved_user_records.jsonl` - endorsed and auto-promoted records.
- `candidate_user_records.jsonl` - tentative records for later review or weak-signal use.
- `rejected_user_records.jsonl` - negative evidence for future pipelines.

The export manifest includes matching counts and paths.

### Future Training Paths

Grafyn's data can later support stronger personal models, but those are not v1:

- **RAG twin with Twin Identity** - implemented first; no model weights change.
- **Preference/ranking model** - learns what answer shape or decision style you choose.
- **Local adapters or fine-tuning** - adjusts a capable base model using reviewed examples.
- **Scratch-trained personal model** - research path only. Prompts alone are not enough; it would require large volumes of personal writing, decisions, outcomes, corrections, and domain evidence.

## Project Status

| Area | Status |
|------|--------|
| Knowledge vault (notes, wikilinks, full-text search) | ✅ Stable |
| Knowledge graph + topic hub clustering | ✅ Stable |
| Multi-LLM Canvas with graph-aware note context | ✅ Stable |
| Conversation import (ChatGPT, Claude, Grok, Gemini) | ✅ Stable |
| Native RAG twin (Advisor + Simulation modes) | 🧪 Experimental |
| Twin Identity, Constitution, Decision Mirror | 🧪 Experimental |
| Twin evidence capture and review dashboard | 🧪 Experimental |
| Local model support via Ollama | ✅ Stable |
| Background link discovery | ✅ Stable |
| Vault Optimizer (background vault improvements) | ✅ Stable |
| Structured vault migration (preview/apply/rollback) | ✅ Stable |
| MCP server (`grafyn-mcp`) for Claude Desktop / Codex Desktop | ✅ Stable |
| Preference / ranking model from export bundles | 🔲 Not started |
| Local adapters or fine-tuning from reviewed evidence | 🔲 Not started |

Twin data capture and export are dependable today; the accuracy machinery that would make twin answers trustworthy — semantic (embedding-based) retrieval, temporal validity, and calibrated confidence — is roadmap work tracked in [TWIN_ACCURACY_ROADMAP.md](TWIN_ACCURACY_ROADMAP.md).

Current version: see [Releases](https://github.com/WKJBryan/Grafyn/releases/latest).

## Quick Start

### Download

Grab the latest installer from [Releases](https://github.com/WKJBryan/Grafyn/releases/latest):

| Platform | File |
|----------|------|
| Windows x64 | `Grafyn_*_x64-setup.exe` |
| Windows ARM64 | `Grafyn_*_arm64-setup.exe` |
| macOS Apple Silicon | `Grafyn_*_aarch64.dmg` |
| Linux Debian/Ubuntu | `grafyn_*_amd64.deb` |
| Linux Universal | `grafyn_*_amd64.AppImage` |

Grafyn auto-updates after installation.

### Build From Source

Prerequisites:

- Node.js 20+
- Rust via [rustup](https://rustup.rs/)
- [Tauri v1 dependencies](https://v1.tauri.app/v1/guides/getting-started/prerequisites)

```bash
cd frontend
npm install
node scripts/generate-icons.cjs

npm run tauri:dev
npm run tauri:build
```

### Configuration

On first launch, Grafyn walks through setup:

1. Vault path - where markdown notes are stored. Default: `~/Documents/Grafyn/vault/`.
2. OpenRouter API key - required for Canvas model execution, distillation, link discovery, and native RAG twin answers.

Local vault data stays on your machine. Canvas model calls send the selected prompt context to the configured model runtime.

## MCP Integration

Grafyn bundles a native Rust MCP server, `grafyn-mcp`, for desktop agents such as Claude Desktop or Codex Desktop.

Use Grafyn Settings to copy the generated MCP config snippet, or configure it manually:

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

The MCP binary shares the same vault and index paths as the desktop app. If the desktop app is holding the search writer lock, MCP falls back to read-only search.

## Architecture

Grafyn is a single desktop app: Vue 3 frontend, Rust/Tauri backend, local filesystem storage.

```text
Tauri Desktop App
├── Vue 3 Frontend
│   ├── Notes
│   ├── Knowledge Graph
│   ├── Canvas
│   └── Twin Review
├── Rust Backend
│   ├── Tauri IPC commands
│   ├── Knowledge store
│   ├── Tantivy search and chunk retrieval
│   ├── Graph index and topic clustering
│   ├── Canvas session store
│   ├── Twin evidence store
│   └── OpenRouter integration
├── grafyn-mcp
└── ~/Documents/Grafyn/
    ├── vault/
    └── data/
```

## Tech Stack

| Layer | Technology |
|-------|------------|
| Frontend | Vue 3, Vite, Pinia, D3.js |
| Desktop | Tauri 1.8 |
| Backend | Rust |
| Search | Tantivy |
| Graph | petgraph + local graph algorithms |
| LLM Runtime | OpenRouter via reqwest; Ollama for local models |
| MCP | rmcp over stdio |
| Storage | Local markdown vault + JSON data files |
| Updates | Cloudflare R2 + Workers |

## Developer Guidelines

### Product Rules

- Grafyn is desktop-first and local-first. Do not add a hosted backend for core vault or twin storage.
- Treat user records as evidence-linked claims, not personality labels.
- Do not silently train on or use records marked `rejected`, `private`, or `no_train`.
- Candidate records may influence live RAG answers only when relevant and must be disclosed as tentative.
- Advisor mode is the default for decision support.
- Simulation mode requires configured Twin Identity and uses first-person model-facing instructions; disclosure belongs in the app UI and docs, not inside the Simulation system prompt.
- Scratch-trained personal models are future research, not current product behavior.

### Hub And Graph Rules

- Prefer broad major hubs over many narrow hubs.
- Use graph structure first, then deterministic canonicalization as fallback.
- Model names, providers, transcript artifacts, and generic UI words should not become hubs.
- Auto-managed duplicate hubs can be merged or removed by sync.
- User-authored hubs should not be silently deleted.
- Minor recurring themes belong in a hub's `Subtopics` section unless they become large enough to justify their own major hub.

### Development Commands

```bash
# Frontend tests
cd frontend
npm run test:run
npm run build

# Prepare Rust/Tauri test prerequisites
cd frontend
npm run prepare:sidecar

# Rust tests
cd frontend/src-tauri
cargo test

# E2E (Playwright) — manual only, not run in CI
npm run tauri:dev        # in one terminal, leave running (provides the Tauri IPC backend)
cd frontend && npm run e2e   # in another terminal
```

The `e2e/` Playwright suite is **not** wired into CI. Its specs call `invoke()` for note CRUD, canvas, etc., which needs a live Tauri IPC backend; a plain Vite dev server in a headless CI browser has no IPC handler, so the app's boot sequence gets stuck in a `failed` phase and most interactive specs time out (confirmed by a manual run — only the static-layout tests pass). CI cannot cheaply provide the built/dev Tauri desktop app, so the suite is run manually via `npm run tauri:dev` + `npm run e2e`. See `e2e/README.md` for details.

Known test noise:

- Some HomeView unit tests emit `router-link` resolution warnings.
- A Canvas store test intentionally logs a failed delete.
- Rust currently warns that `SimilarityProvider::encode_batch` is unused.

## Contributing

1. Fork the repository and create a feature branch.
2. Keep changes scoped and evidence-backed.
3. Add or update tests for behavior changes.
4. Run the frontend and Rust verification commands.
5. Submit a pull request.

See [CLAUDE.md](CLAUDE.md), [WORKING_GUIDE.md](WORKING_GUIDE.md), and [TWIN_RAG_SPEC.md](TWIN_RAG_SPEC.md) for deeper architecture and workflow notes.
