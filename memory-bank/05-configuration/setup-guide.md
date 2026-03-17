# Setup Guide

> **Purpose:** Current setup notes for the Tauri desktop app
> **Status:** Current

## Scope

Grafyn now runs as a desktop-first Tauri application. The old Python backend and HTTP web mode are gone. Treat [HOW_TO_RUN.md](../../HOW_TO_RUN.md) as the main source of truth and use this page as a short companion note inside the Memory Bank.

## Prerequisites

You need:

- Node.js 18+
- Rust toolchain
- Tauri system prerequisites for your platform
- Git

Optional runtime configuration:

- `OPENROUTER_API_KEY` if you want Canvas LLM features immediately
- `GITHUB_FEEDBACK_REPO` and `GITHUB_FEEDBACK_TOKEN` only if you intentionally enable runtime feedback submission for local testing

## Install

```bash
git clone https://github.com/WKJBryan/Grafyn.git
cd Grafyn

cd frontend
npm install

cd ../e2e
npm install
```

## Run The App

```bash
cd frontend
npm run tauri:dev
```

Useful adjacent commands:

```bash
cd frontend
npm run lint
npm run test:run
npm run build

cd src-tauri
cargo test
```

## Testing

- Frontend unit tests live under `frontend/src/__tests__`.
- Rust tests live under `frontend/src-tauri`.
- Playwright tests live under `e2e/tests` and target the frontend/Vite surface, not a `localhost:8080` backend.

```bash
cd e2e
npm test
```

## Notes For Memory Bank Readers

- Older Memory Bank pages may still describe the removed Python backend, OAuth flows, or HTTP endpoints.
- When a Memory Bank page conflicts with the root docs, prefer the root docs and the current code.
- If you update a historical page, mark whether it is current guidance or archival context.
