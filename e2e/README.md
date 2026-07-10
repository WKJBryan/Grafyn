# Grafyn E2E Tests (Playwright)

The E2E suite drives the desktop UI (via Vite dev server) without a separate backend or Python service. Web mode is removed.

**Manual-only, not wired into CI.** All API calls go through `invoke()` from `@tauri-apps/api/tauri`, which requires a live Tauri IPC backend. Against a plain `npm run dev` Vite server in an ordinary browser (which is all a hosted CI runner can cheaply provide), there is no IPC handler, so `get_boot_status`/`settings.get()` calls never resolve and the app's boot sequence gets stuck in its `failed` phase — a `.startup-splash[data-phase="failed"]` overlay blocks all pointer interaction. Confirmed by running `npx playwright test --project=chromium tests/navigation.spec.js`: the 10 tests that only check static layout pass, but the 11 that need working IPC (note creation, TreeNav selection, canvas navigation, refresh persistence) fail/time out waiting on elements blocked by the failed-boot overlay. Running the suite for real requires the built/dev Tauri desktop app (`npm run tauri:dev`) as the IPC backend, which CI cannot provide cheaply (Tauri v1 GTK/webkit2gtk build + a display). Run it locally instead — see `npm run e2e` in `frontend/package.json`.

## Setup
```bash
cd e2e
npm install
npm run install-browsers
```

## Run
```bash
npm test            # headless default
npm run test:ui     # Playwright UI runner
npm run test:headed # headed debug
```

## What the Suite Covers
- Navigation/layout basics
- Note CRUD, search, markdown editor flows
- Settings and feedback UI paths (mocked)
- Canvas UI smoke (no external LLM calls unless `OPENROUTER_API_KEY` is set)

## How It Connects
- `playwright.config.js` starts the Vite dev server in `../frontend` and targets `http://localhost:5173`.
- All API calls are routed through Tauri IPC; there is no `localhost:8080` backend.

## Tips
- If startup times out, ensure no other process is using port 5173 and rerun `npm run dev` inside `frontend` to warm dependencies.
- Use `DEBUG=pw:api` for verbose Playwright logs.
- Record a trace on failures: `npm test -- --trace on --project=chromium`.
