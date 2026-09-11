# Contextual relationships and local acceleration implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development for independent implementation and review. Preserve the uncommitted twin-evidence branch and its private-data exclusions.

**Goal:** Separate candidate discovery from grounded contextual relationship assessment, establish a labeled evaluation, and determine a viable measured local acceleration path.

**Architecture:** Embeddings propose passage pairs. A bounded local scorer interprets the pair with attributable context and stores tentative typed relationships with exact receipts. Runtime identity remains separate from model identity; acceleration can change execution only after compatible model outputs and actual device placement are verified.

**Tech Stack:** Existing Rust/Tauri, Vue/Pinia, local Ollama; investigate Windows ML/ONNX Runtime compatibility before introducing a native runtime dependency.

**Spec:** User-approved September 5 conversation: automatic local acceleration, contextual relationship scoring first, fine-tuning only after measured retrieval/scoring failures; existing evidence requirements in `../specs/2026-09-05-evidence-ingestion-design.md` remain binding.

## Global constraints

- No cloud fallback, hidden model substitution, private training corpus publication, or automatic training.
- Preserve source revisions, target mapping, held-out restrictions, rejections and human corrections.
- An embedding score is relatedness, never certainty or causal strength. A model-assessed effect remains an inferred mechanism.
- No automatic model/provider downloads until explicit setup action. No claim of NPU operation without actual execution evidence.
- Keep qwen3.6:27b for the pilot scorer and capture exact model digest and prompt version.
- This build does not establish personal predictive accuracy or authorize adding private data to any shared training set.

## Task 1: Local runtime feasibility and embedding boundary

Files: `services/evidence/discovery.rs`, new `services/evidence/embedding.rs`; report `docs/local-embedding-runtime.md`.

- [x] Read existing embedding cache/discovery and hardware/runtime documentation. Inspect local hardware read-only.
- [x] Select the smallest currently executable integration. Separate backend-specific model lookup, embedding request, device observation and validation from candidate generation. Preserve `discover_relationships(snapshot, cache_dir, ollama_url)` for callers.
- [x] Test invalid vectors, incompatible dimensions, changed model identity, failed runtime, and absent/unknown device observations. Never label an unobserved device as NPU.
- [x] Verify actual supported local execution where installed. Record concrete blockers to native NPU validation; do not add a pretend provider or route unsupported models there.

## Task 2: Contextual pair assessment

Files: new `services/evidence/assessment.rs` with tests; edit `models.rs`, `mod.rs`, `discovery.rs`, `evidence_bridge.rs`, context filtering as needed.

- [x] Add failing tests for equivalent meanings versus different metrics/timeframes, contextual conflicts, direction reversal, insufficient evidence, exact receipts, source edits during inference, and preserved corrections.
- [x] Implement a bounded assessment envelope: verdict (`equivalent`, `related`, `conflicts`, `enables`, `inhibits`, `requires`, `unrelated`, `insufficient`), direction, explanation, conditions and exact evidence quotes. Validate types and evidence before applying tentative links.
- [x] Keep candidate cosine separately. Do not make unrelated/insufficient verdicts user rejections. Save completed assessment identities so repeated worker ticks do not repeat calls; interrupted calls stay retryable with bounded cooldown.
- [x] Assess a small number per tick outside store locks, check current revisions and manual review state on apply, and expose pending/error status. Never promote model-generated causality to observed fact.

## Task 3: Inspectable output and benchmark

Files: `components/twin/EvidenceRelationships.vue`, `RelationshipEvidencePanel.vue`, `utils/evidenceGraph.js`, focused frontend tests; synthetic fixtures and evaluator under `scripts/` or existing maintenance binary.

- [x] Make equivalent/conflicting links and assessment evidence inspectable; display actual runtime observation without claiming NPU selection.
- [x] Add a human-readable synthetic labeled pair suite covering paraphrases, number/deadline/beneficiary distinctions, directional mechanisms, generic-topic false positives and ambiguity. These labels are developer-authored fixtures, not user-adjudicated study data.
- [x] Run the same pairs through candidate discovery and contextual scoring separately where runtimes are installed. Record candidate recall, typed-label agreement, directional mistakes, abstentions and parse failures with explicit denominators; save private/local outputs outside the repo.
- [x] Run focused tests, then Rust/frontend suites, MCP compatibility and production build. Review changed paths and update CLAUDE.md with implemented behavior and limits.

## Acceptance boundary

Passing fixture tests establishes contracts, not semantic model quality. Live labeled-pair results establish only the small fixture baseline. NPU support requires a supported model/runtime/driver and real device test; if unavailable here, keep the verified local path and document the remaining hardware acceptance work precisely.

## Execution result

The executable runtime boundary, contextual assessment, inspection UI and synthetic evaluator are implemented. Final checks: 366 Rust tests, 478 frontend tests, production frontend/size checks, desktop/MCP builds and MCP import smoke passed. A real local scorer run matched 7/12 developer-authored labels; see the [maintenance guide](../../evidence-maintenance.md#contextual-relationship-baseline) for denominators, misses and limitations. This is a diagnostic baseline, not a quality acceptance pass. Native NPU execution and real embedding discovery remain unvalidated and are not claimed as delivered.
