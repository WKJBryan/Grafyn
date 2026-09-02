# Grafyn system E2E tests

This suite drives the real Vue UI through a feature-gated local Grafyn runtime. Notes, attachments, Twin events, projections, and encrypted sync use production Rust commands and services over owned temporary roots. Only the paid OpenRouter network boundary is replaced by a deterministic loopback fixture.

The harness is development/test-only. It does not ship in release binaries and it does not prove Android Keystore, WebView, FileProvider, installation, lifecycle, or physical-device behavior.

The same suite runs in the `system-e2e` job in `.github/workflows/test.yml`.

## Setup

```bash
cd frontend
npm ci
npm run prepare:sidecar
npm run build

cd ../e2e
npm ci
npx playwright install chromium
```

## Run

```bash
npm run test:fixtures   # strict runtime client and OpenRouter fixture
npm test                # desktop-wide plus Pixel companion journeys
npm run test:desktop
npm run test:companion
```

`npm run e2e` from `frontend/` runs the same complete Playwright suite after dependencies are installed.

## Journeys exercised

- Desktop-wide notes, search, graph, settings and read-only MCP/optimizer status, spatial Canvas, and import surfaces.
- Pixel-sized online image generation and governed save through the production core behind an injected in-memory test secret store.
- Offline contextual capture, runtime rebuild, then Recall with the saved image and an attention explanation.
- Candidate Twin memory exclusion before review, explicit acceptance, then history-aware Advisor chat.
- Reordered and duplicated ciphertext delivery to a second device, checking note, attachment, event projection, reviewed-memory convergence, and zero echo.
- All four compact destinations for horizontal overflow, padding, visible composers, 44px targets, and absence of desktop-only controls.

## Security boundary

The runtime binds only to `127.0.0.1`, requires a random 256-bit bearer token plus exact Origin/profile/device headers, accepts strict bounded JSON, and exposes a fixed command allowlist. It permits only read-only MCP and optimizer status calls; MCP mutation, optimizer administration, arbitrary paths, native dialogs/plugins, updater/process access, and migration remain rejected. The bridge is compiled only with `e2e-test-runtime`; the frontend transport is dev-only and is tree-shaken from production builds.

Because these journeys use a browser, they cannot execute installed-native picker or updater behavior. Those stay within runtime capability tests, native/unit contracts, and desktop bundle verification rather than being claimed as Playwright proof.

The Android E2E profile enables image generation only after the harness constructs production services over temporary roots with an injected `MemorySecretStore`. Production `RuntimeBootstrap` is unchanged and Android continues to advertise image generation unavailable; enabling it requires a separately verified installed-app, secret-backed receipt-producer path. No hosted relay, account, billing, pairing, or recovery service is represented by this suite.
