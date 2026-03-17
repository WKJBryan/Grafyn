# Testing Status (Desktop-Only)

Current scope reflects the Tauri desktop app. The legacy Python backend and its pytest suite no longer ship with this repository.

| Area | Status | Notes |
|------|--------|-------|
| Frontend unit tests (Vitest) | Maintained | `npm run test:run` in `frontend` |
| Tauri/Rust tests | Maintained | `cargo test` in `frontend/src-tauri` |
| Playwright E2E | Maintained (UI-only) | Runs against Vite dev server on :5173 |
| Legacy backend tests | Removed | Not applicable; backend folder is absent |

## Quick Commands
- Frontend: `cd frontend && npm run test:run`
- Rust: `cd frontend/src-tauri && cargo test`
- E2E: `cd e2e && npm test` (after `npm run install-browsers`)

## Coverage Goals
- Keep frontend unit coverage healthy (>80%) and add specs alongside new IPC commands or components.
- Use E2E for cross-surface flows (navigation, note CRUD, markdown, settings, feedback).

## Historical Note
Earlier documentation referenced hundreds of Python backend tests and web-mode OAuth flows. Those paths are gone; new tests should target the desktop IPC architecture only.
