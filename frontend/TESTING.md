# Frontend Testing

Tests use Vitest with jsdom and Vue Test Utils. All network calls are mocked; there is no HTTP backend or Axios layer.

## Commands
```bash
cd frontend
npm run test:run   # CI-friendly one-shot
npm test           # watch mode
npm run lint       # ESLint
```

## Structure
- `src/__tests__/setup.js` – global test setup/mocks
- `src/__tests__/unit/` – component, store, and API client specs

## Guidelines
- Prefer mocking Tauri `invoke` calls exposed in `src/api/client.js`.
- Keep tests UI-focused; avoid reintroducing backend/web-mode assumptions.
- When adding new IPC commands, add a small unit spec that verifies the correct `invoke` payload.
