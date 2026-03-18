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
npm run tauri:build  # builds the bundled MCP sidecar, then packages installers under src-tauri/target/release/bundle/
```
If `TAURI_PRIVATE_KEY` is unset, the local build skips the signed updater bundle automatically and still produces normal installer artifacts.

## Frontend-Only Tasks (no backend)
```bash
cd frontend
npm run lint
npm run test:run     # Vitest unit suite
npm run build        # Vite production build (used by Tauri)
```

## What "Desktop App" vs "Frontend-Only" Means

| Mode | What you have | What you do not have | Best for |
| --- | --- | --- | --- |
| Desktop app | Full Tauri app: Vue frontend plus Rust-powered desktop capabilities | No cloud/server features unless we build them separately | Real app behavior, local files, native menus, packaging, offline workflows |
| Frontend-only task | UI work in the Vue/Vite layer | No Rust/Tauri behavior at runtime, no native desktop APIs | Layouts, components, state flows, unit tests, visual polish |
| No backend | No server-side service or hosted database | No sync service, no server-owned auth/session logic, no safe place for private server keys | Local-first features and UI work that does not need cloud coordination |

### Practical impact
- Frontend-only work can build screens, forms, interactions, and local state, but it cannot verify native desktop features such as file dialogs, system tray behavior, window management, or other OS integrations.
- Desktop app work can use Tauri's native bridge, so we can open and save local files, integrate with the operating system, bundle the app, and test the real installed-app behavior.
- "No backend" does not mean "nothing works." It means features that depend on a shared server are missing, such as multi-device sync, server-managed accounts, background jobs, webhooks, and securely storing private API secrets on our own infrastructure.

### Rule of thumb
- Choose desktop app development when the feature needs local machine access or must behave like a packaged native app.
- Choose frontend-only tasks when the work is limited to UI behavior, styling, component logic, or tests that can run without the native layer.
- Add a backend only when the feature needs shared online data, centralized auth, cloud automation, or secure server-side key handling.

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
   `npm run tauri:build` now prepares the `grafyn-mcp` bundled sidecar automatically.
