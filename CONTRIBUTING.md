# Contributing to Grafyn

Thanks for considering a contribution. Grafyn is a local-first desktop app with firm product rules — read them first; they're short and they're enforced in review.

## Product Rules

These are non-negotiable design constraints, not style preferences:

- **Desktop-first, local-first.** Do not add a hosted backend for core vault or twin storage.
- **Twin records are evidence-linked claims, not personality labels.** Every inferred record must trace back to the prompts, sessions, and excerpts that support it.
- **Never train on or use records marked `rejected`, `private`, or `no_train`.** Rejected records exist only as exportable negative evidence.
- **Candidate records** may influence live RAG answers only when relevant to the prompt, and must be disclosed as tentative.
- **Advisor mode is the default** for decision support. **Simulation mode requires a configured Twin Identity** and uses first-person model-facing instructions; disclosure that it's a configured simulation belongs in the app UI and docs, not inside the Simulation system prompt.
- **Twin accuracy evaluation is external by design.** The app captures and exports; it does not score, benchmark, or display accuracy results. Don't build eval UIs.
- Scratch-trained personal models are future research, not current product behavior.

## Build From Source

Prerequisites:

- Node.js 20+
- Rust via [rustup](https://rustup.rs/)
- [Tauri v1 dependencies](https://v1.tauri.app/v1/guides/getting-started/prerequisites)

```bash
cd frontend
npm install
node scripts/generate-icons.cjs

npm run tauri:dev        # dev mode with hot reload
npm run tauri:build      # production build → src-tauri/target/release/bundle/
```

## Tests & Verification

```bash
# Frontend unit tests (Vitest)
cd frontend
npm run test:run
npm run lint

# Rust test prerequisites (once per checkout):
npm run prepare:sidecar               # builds the grafyn-mcp sidecar binary
mkdir -p dist && echo '<html></html>' > dist/index.html   # stub for tauri::generate_context!

# Rust tests
cd src-tauri
cargo test

# Both binaries must build:
cargo build --no-default-features --features mcp   # the MCP binary's feature set

# Source-file size guardrail (also runs in CI's lint job):
cd .. && npm run check:file-sizes
```

### E2E (Playwright) — manual only

The `e2e/` suite is **not** wired into CI. Its specs call Tauri `invoke()`, which needs a live IPC backend; a plain Vite server in a headless browser has no IPC handler, so the boot sequence blocks and interactive specs time out. Run it manually:

```bash
npm run tauri:dev            # terminal 1 — leave running
cd frontend && npm run e2e   # terminal 2
```

See `e2e/README.md` for details.

### Known test noise

- Some HomeView unit tests emit `router-link` resolution warnings.
- A canvas store test intentionally logs a failed delete.
- Rust warns that `SimilarityProvider::encode_batch` is unused (a designed-but-unimplemented embedding seam).

## Code Conventions

- **Write paths are chokepoints.** All note writes go through `commit_note_write`/`commit_note_writes`, deletes through `commit_note_delete` (`commands/mod.rs`), and file persistence through `services/atomic_io.rs::write_atomic`. Don't hand-roll index updates or direct `fs::write` calls — drift between duplicated sequences is how past regressions happened.
- **Lock ordering is canonical:** `knowledge_store` before `vault_optimizer`, documented at the top of `commands/mod.rs`. Never invert it.
- **File size:** target ~1,500 lines per source file; CI fails the lint job at 2,500. When a file outgrows that, split it with the mod-facade pattern (`commands/canvas/` is the exemplar).
- Prefix intentionally unused variables with `_` (ESLint `argsIgnorePattern` is configured).

## Hub & Graph Rules

The topic-hub system is opinionated; changes to `services/topic_hub.rs` should preserve:

- Broad major hubs over many narrow hubs; minor recurring themes live in a hub's `Subtopics` section.
- Graph structure first, deterministic canonicalization as fallback.
- Model names, providers, transcript artifacts, and generic UI words must not become hubs.
- Auto-managed duplicate hubs can be merged or removed by sync; user-authored hubs are never silently deleted.

## Pull Requests

1. Fork and create a feature branch — never commit to `main`.
2. Keep changes scoped; add or update tests for behavior changes (this repo works test-first).
3. Run the verification commands above; CI runs the same suites plus multi-platform release smoke builds.
4. Open a PR with a clear summary. CI must be green to merge.

Deeper references: [CLAUDE.md](CLAUDE.md) (architecture and agent guidance), [WORKING_GUIDE.md](WORKING_GUIDE.md) (release workflow), [TWIN_RAG_SPEC.md](TWIN_RAG_SPEC.md) (twin design), [TWIN_ACCURACY_ROADMAP.md](TWIN_ACCURACY_ROADMAP.md) (twin roadmap).
