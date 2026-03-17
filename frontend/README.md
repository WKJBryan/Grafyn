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
npm run tauri:build     # Bundles installers under src-tauri/target/release/bundle/
```

## Scripts
- `npm run tauri:dev` – desktop dev
- `npm run tauri:build` – production bundle
- `npm run lint` – ESLint
- `npm run test:run` – Vitest unit suite
- `npm run build` – Vite production build (used by Tauri)

## API Integration
`src/api/client.js` calls `window.__TAURI__.invoke` commands; no Axios or HTTP proxy is used. Any new API surface should be added as a Tauri command and wired here.

## Environment
The only common variable is `OPENROUTER_API_KEY` (optional, for canvas LLM features). Data is stored under `~/Documents/Grafyn/` by default.

## Troubleshooting
- If commands fail, ensure Rust toolchain and Tauri prerequisites are installed.
- If the UI cannot reach backend commands, restart with `npm run tauri:dev` to ensure the Rust side is running.
