# SDD ledger — plan: docs/superpowers/plans/2026-09-05-contextual-relationships.md

User approved the runtime/scoring recommendation with "sounds good" after the implementation discussion. Changes remain in the existing isolated worktree.

Ruling: implement assessment and measured-runtime boundaries before custom training; do not install another model on implicit setup permission. An available runtime may be probed read-only.

| Interface | Preflight finding |
|---|---|
| Task 1 / Task 2 discovery.rs | Runtime worker owns embedding extraction only; root owns assessment call and discovery application. Coordinate before editing shared file. |
| Task 2 / Task 3 Relationship | Root publishes verdict/metadata fields before UI integration. |
| Task 1 self | No NPU claim without observed execution; feasibility report can record unsupported hardware rather than fabricate success. |
| Task 2 self | Suppressed machine verdicts are not human rejection; review overrides persist. |
| Task 3 self | Synthetic labels measure semantic behavior separately from personal prediction benefit. |

Task 1: complete executable Ollama boundary and truthful hardware feasibility report. Native NPU runtime remains explicitly unimplemented; no compatible NPU/model artifact acceptance on this machine and no installation approval received.
Task 2: implemented, focused and full tests pass. Review fixed omitted nearby source context, stale confirmed goals, historical cutoff, correction metadata and raw scorer envelope leakage into prediction prompts.
Task 3: UI and benchmark implemented; 478 frontend tests pass. Actual local 12-pair scorer benchmark complete; embedding installation remains pending so retrieval recall is unavailable.

Verification: 366 Rust tests passed, three external-runtime/data tests ignored this run; 478 frontend tests passed across 39 files; production frontend and file-size checks passed; final MCP binaries and desktop executable rebuilt successfully after the discovery lifecycle fix; final MCP stdio repeated import smoke passed. Executable: `C:\Users\bryan\Seedream\frontend\src-tauri\target\debug\grafyn.exe`.

Ruling: a confirmed interpretation survives as a human review record, but a changed goal context requires explicit reaffirmation before reuse. Raw scorer envelopes stay in audit storage and are excluded from prediction context. A frozen batch validates against its own goal cutoff, not wall-clock time.

Scoped review: all four assessment/backend findings fixed and re-reviewed without blockers. Final integration review identified reversed inspector direction wording and stale embedding score metadata across model versions; both are fixed. Three lifecycle regression tests pass. Fixture corrections supplied required support scope and compared consistently projected snapshots; no production assertion was weakened.

Live baseline complete: `.grafyn-private/relationship-baseline-01/report.json`,12 valid judgments,7/12exact label agreement,6/9accepted-only,3/4expected-direction disagreements (includes alternative relation/symmetric classification),0abstentions,0provider/parsingfailures,unchangedqwendigest. Discovery unavailable/modelabsent; relevant-pair denominator10 retained. No prompt tuning or modeltraining based on these results. This is developer-fixture diagnostic evidence, not human-adjudicated or personal accuracy evidence.

Handoff: implementation and bounded feasibility investigation complete; all changes remain uncommitted in `codex/twin-evidence`. Native NPU integration, embedding installation/retrieval-quality validation, scorer-quality improvement and any eventual custom training remain separate follow-up work. No private interview/workbook or original KW state was changed in this follow-up.
