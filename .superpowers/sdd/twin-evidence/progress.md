# Grafyn evidence implementation

Approved implementation: `docs/superpowers/specs/2026-09-05-evidence-ingestion-design.md`, reconciled with the user's September 5 implementation request.

Worktree: `.worktrees/twin-evidence`, branch `codex/twin-evidence`, base `a3c9243`.
Original user Canvas context/debate changes were copied and must be preserved.

## Ownership and status
- evidence_core: services/evidence typed storage, jobs, exact receipts, goals, context, embedding adapter (implemented and reviewed).
- ingestion: source boundaries, attribution, topic clustering, CSV/XLSX, desktop/MCP imports (implemented and verified).
- frontend: scoped interview, goals, graph, prediction UI (implemented and verified).
- root: shared integration, sealed comparison capture, app/MCP workers, repair, executable and documentation handoff verified.

## Binding decisions
- Pilot is separate from the current KW vault and does not switch global settings.
- Explicit target mapping is required; app user, interviewer and model outputs imply no target identity.
- Source-grounded interpretations may be used automatically as tentative, never human endorsement.
- Local embeddinggemma requires an explicit setup action; no provider substitution.
- Prediction comparisons freeze qwen3.6:27b identity/settings and development context; validation hides predictions until actual choice.
- The real interview and 20 fresh decisions must be supplied by the user; fixtures are not empirical predictive validation.

## Baseline evidence
- Twin frontend store: 11 tests passed.
- Eval Lab frontend: 6 existing tests failed against the pre-change UI.
- Rust MCP test compilation initially failed in existing OllamaOptions constructors (Option fields added without updating two constructors). Root repairs those before focused validation.
- Live KW repair: five unedited automatic candidates quarantined with exact rollback snapshots; original source files unchanged; one potentially reviewed item preserved. A second preview proposes zero changes.

## Final validation
- Frontend: 471 tests passed across 39 files; production build and file-size checks pass.
- Runnable desktop development executable built successfully with the production frontend embedded: `C:\Users\bryan\Seedream\frontend\src-tauri\target\debug\grafyn.exe`. Work remains uncommitted on `codex/twin-evidence`.
- Desktop Rust: 341 tests passed, zero failed; three external-runtime/data tests ignored by default and each run separately successfully.
- Actual local Qwen extraction: one synthetic case, one personal statement, two exact receipts.
- Actual local prediction: all three conditions returned valid forecasts; sealed projection and choice reveal verified with exact request envelopes.
- Actual workbook pipeline in temporary notes: 42 deduplicated scenarios, 31 usable cases, 11 flagged jobs, 31 selected context cases with exact receipts.
- MCP stdio initialize/schema/import/repeated-import smoke passed; one evidence note after two identical imports; durable queue visible.
- Copied KW repair: five writes, repeat no-op, byte-exact rollback; original applied only after identical-change preflight.
- Private fixtures and repair manifests stay under ignored `.grafyn-private/`.

## Validation boundaries
- Browser inspection covered empty/error UI and scoped controls; filled interview/graph behavior is component-tested, with real backend service flows tested separately.
- embeddinggemma is not installed yet. Explicit install control exists; an asynchronous permission request was sent for real semantic-quality validation. No download without that setup action.
- No real personal interview or 20 fresh decisions supplied; predictive benefit is unproven and cannot be inferred from synthetic tests.
