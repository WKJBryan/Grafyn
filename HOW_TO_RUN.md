# Grafyn — Run & Test (Desktop-Only)

The product ships as a Tauri desktop app: Vue 3 + Vite in a WebView with a Rust backend. Web mode and the old Python backend are removed.

## Prerequisites
- Rust toolchain (`rustup`)
- Node.js 18+ and npm
- Tauri v1 prerequisites for your OS (see tauri.app)

## Develop the Desktop App
```bash
cd frontend
npm install          # first time
npm run tauri:dev    # Tauri + Vite with hot reload
```

## Build the Desktop App
```bash
cd frontend
npm run tauri:build  # bundles to src-tauri/target/release/bundle/
```

## Frontend-Only Tasks (no backend)
```bash
cd frontend
npm run lint
npm run test:run     # Vitest unit suite
npm run build        # Vite production build (used by Tauri)
```

## Rust Backend Tests
```bash
cd frontend/src-tauri
cargo test
```

## Playwright E2E (against the Vite UI, no backend)
```bash
cd e2e
npm install
npm run install-browsers
npm test             # or npm run test:ui for the UI runner
```

## Environment Notes
- `OPENROUTER_API_KEY` (optional) enables the multi-LLM canvas.
- App data lives under `~/Documents/Grafyn/` by default (vault, data, cache).

## Quick Start
1) Install deps: `npm install` in `frontend` and `e2e`, `rustup` for Tauri.
2) Run live app: `npm run tauri:dev`.
3) Before shipping: `npm run lint && npm run test:run && cargo test && npm run tauri:build`.
