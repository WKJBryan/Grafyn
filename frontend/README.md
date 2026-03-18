# Grafyn Frontend (Tauri UI)

Vue 3 + Vite frontend packaged as a Tauri desktop app. All app interactions go through Tauri IPC; there is no HTTP web mode or external Python backend.

## Tech Stack
- Vue 3 (Composition API), Vite 5
- Pinia, Vue Router
- Tauri v1 with Rust backend
- Vitest for unit tests, Playwright for E2E

## Project Layout
```
frontend/
├── src/               # Vue app
│   ├── main.js        # bootstrap
│   ├── App.vue        # root shell
│   ├── api/client.js  # Tauri invoke() wrappers
│   ├── components/    # UI components
│   ├── views/         # screens
│   └── stores/        # Pinia stores
├── src-tauri/         # Rust backend + Tauri config
├── vite.config.js     # Vite build config (desktop only)
└── package.json
```

## Dev Workflow (Desktop-Only)
```bash
cd frontend
npm install
npm run tauri:dev       # Tauri + Vite with hot reload
```

## Build
```bash
cd frontend
npm run tauri:build     # Builds the grafyn-mcp sidecar, then bundles installers under src-tauri/target/release/bundle/
```
If `TAURI_PRIVATE_KEY` is unset, local builds skip the signed updater bundle automatically and still produce installer artifacts.

## Scripts
- `npm run tauri:dev` – desktop dev with automatic sidecar prep
- `npm run tauri:build` – production bundle with automatic sidecar prep
- `npm run lint` – ESLint
- `npm run test:run` – Vitest unit suite
- `npm run build` – Vite production build (used by Tauri)

## Key Features
- **Multi-LLM Canvas** — compare responses from multiple models in parallel on an infinite canvas
- **Semantic Note Context** — retrieves relevant vault notes as LLM context automatically
- **Smart Web Search** — auto-detects when prompts need web results (temporal queries, news, comparisons) via heuristic analysis; toggled in Settings (default: on), ~$0.02/query via OpenRouter plugin
- **Zettelkasten Link Discovery** — discovers potential wikilinks via semantic similarity + LLM analysis
- **Conversation Import** — import from ChatGPT, Claude, Grok, and Gemini
- **OS Keychain Storage** — API keys stored securely via `keyring` crate (Windows Credential Manager, macOS Keychain, Linux Secret Service)

## API Integration
`src/api/client.js` calls `window.__TAURI__.invoke` commands; no Axios or HTTP proxy is used. Any new API surface should be added as a Tauri command and wired here.

## Environment
The only common variable is `OPENROUTER_API_KEY` (optional, for canvas LLM features). Data is stored under `~/Documents/Grafyn/` by default.

## Troubleshooting
- If commands fail, ensure Rust toolchain and Tauri prerequisites are installed.
- If Tauri packaging fails, rerun `npm run tauri:build`; it now rebuilds the bundled `grafyn-mcp` sidecar automatically before packaging.
- If the UI cannot reach backend commands, restart with `npm run tauri:dev` to ensure the Rust side is running.
