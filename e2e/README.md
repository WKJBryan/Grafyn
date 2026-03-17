# Grafyn E2E Tests (Playwright)

The E2E suite drives the desktop UI (via Vite dev server) without a separate backend or Python service. Web mode is removed.

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
