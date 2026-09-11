# Evidence import and repair

Build the headless tool from `frontend/src-tauri`:

```powershell
cargo build --no-default-features --features mcp --bin grafyn-evidence-maintenance
```

The resulting executable is `target/debug/grafyn-evidence-maintenance.exe` on Windows. The commands below assume that executable is on `PATH`; otherwise invoke it by its full path. These commands use the same import/evidence/repair services as the app. They do not create a separate implementation of repair logic.

The file-backed smoke needs an empty or absent directory. It creates a unique subdirectory and verifies draft resume, submissions in both interview domains, simultaneous goals, goal revision history, exact context receipts, idempotence, and restricted-source invalidation through the shared services. It prints a compact summary and retains the synthetic files for inspection.

```powershell
grafyn-evidence-maintenance smoke --data C:\repair\synthetic-smoke
```

Use explicit copied vault/twin roots and an isolated cache directory. Preview writes a manifest containing private evidence and exact original/replacement bytes; keep it with private copied artifacts, outside the repository. It never changes source notes or derived records.

```powershell
grafyn-evidence-maintenance preview --vault C:\repair\vault --twin-root C:\repair\twin --data C:\repair\cache --manifest C:\repair\manifest.json
grafyn-evidence-maintenance apply --vault C:\repair\vault --twin-root C:\repair\twin --data C:\repair\cache --manifest C:\repair\manifest.json
grafyn-evidence-maintenance rollback --vault C:\repair\vault --twin-root C:\repair\twin --data C:\repair\cache --manifest C:\repair\manifest.json
```

Apply snapshots the manifest inside `twin/repair_history` before rewriting unreviewed auto-derived personal claims. Repeating apply is a no-op. Rollback restores exact bytes only if the repaired version remains unchanged; conflicts preserve later edits. Active, edited, manually authored, guided-setup and research-fact records are preserved. Suspicious reviewed items appear in `review_only`. Decisions and original sources are untouched. Stop concurrent writes to the same Twin store while using the maintenance tool, then reload Grafyn's Twin state and rebuild derived indexes; the tool's lock only coordinates maintenance processes.

CSV/XLSX use the same parser in the desktop preview/import, MCP `import_conversation`, and this inspector. Default headers distinguish `Question`, `Option A`, `Option B`, `User Answer`, `User Rationale`, `Model Answer`, `Model Rationale`, and `Model`. Explicit column mapping supports headerless or custom tables. `sheet` optionally selects one worksheet; omit it to apply the mapping across all worksheets. Repeated header rows are skipped. Options on separate `A)`/`B)` lines in a question are preserved. Unknown answer mappings remain unanswered. Conflicting answers, rationales, reused rationales across different questions and explicit answer/rationale label mismatch remain review issues.

Example mapping file (adjust letters to the actual source; no person is inferred from this mapping):

```json
{"columns":{"question":"C","target_answer":"G","target_rationale":"H","model":"D","model_rationale":"E","model_answer":"F"},"first_data_row":1}
```

```powershell
grafyn-evidence-maintenance inspect-import --file C:\repair\decisions.xlsx --mapping-json C:\repair\mapping.json
```

The inspector prints counts and issue codes, never questions, answers or rationales. Imports retain full questions, target and model responses separately, exact file/sheet/row/cell receipts, and raw row cells. Cases are grouped by normalized full question and options, not row number; model runs never become additional target decisions. Imported historical cases begin as development data. Target attribution requires an explicit person ID and speaker mapping in desktop or MCP import. For tables, `target_speaker: "source"` denotes the explicitly mapped target-answer column.

The app's **Current vault** evidence scope uses that explicit target mapping. **Bryan pilot** creates isolated data under `data/pilots/bryan` with subject `bryan-pilot`; choosing it does not switch the app vault. Both use resumable interview drafts, concurrent goal revisions and the shared evidence selector. Grounded tentative personal statements and procedures can inform context automatically; review confirms or rejects them without pretending that model inference is human endorsement. Invalid receipts, unresolved attribution, conflicts, restrictions and held-out sources keep affected evidence out of context.

Meaning groups and goal paths are available in the evidence workspace with exact receipt panels and statement/relationship review. Local semantic discovery requires embeddinggemma. Use the explicit **Install embeddinggemma locally** action after deciding to install it; imports never silently download embeddings or fall back to a cloud provider. The model is not installed on the current validation machine, so real semantic quality is unvalidated.

Prediction comparisons use local `qwen3.6:27b` and exactly three conditions: `no_evidence`, `personal_evidence`, and `goal_paths`. A batch freezes its model digest, evidence snapshot and settings (`temperature: 0`, `top_p: 1`, `seed: 42`, `num_predict: 2048`). Validation mode hides forecasts and context until the actual choice is recorded; up to two neutral questions from a fixed topic whitelist are available before reveal. Arbitrary model questions are not exposed while sealed. Keep prediction ledgers and Canvas `prediction_contexts` replay envelopes private alongside Twin data.

## Verified scope on 2026-09-05

- Rust: 341 passed, zero failed, three ignored; all three ignored integration tests also passed when explicitly run. Frontend: 471 passed across 39 files; production build and size checks passed.
- The synthetic file-backed smoke covered two interview domains, three goal revisions and four goal links. Actual local narrative extraction returned one case, one statement and two exact receipts. Synthetic local three-condition comparisons passed sealed/revealed checks.
- Actual workbook parsing retained 42 answered scenarios, 336 row receipts and 294 model outputs. The complete copied-data pipeline produced 42 notes, 31 accepted cases and 11 flagged cases, with 31 cases and 31 exact receipts in shared context. Historical data and flags do not establish predictive accuracy.
- Actual MCP stdio initialization, tool discovery and duplicate import produced one mapped evidence note with a durable queued job.
- The five-item repair passed apply, repeated no-op and exact rollback on an identical copy. After preflight, the same narrow repair was applied to the original derived state; the next preview reported zero automatic changes and one review-only item. Original private source files were untouched. Legacy unanswered decision episodes were not joined or given retroactive predictions.

A fully populated native desktop workspace was not visually inspected; browser empty/error states and component graph checks are narrower evidence. No real embedding quality result, empirical 20-decision study, predictive lift or quantitative causal simulation is claimed. The [approved design](superpowers/specs/2026-09-05-evidence-ingestion-design.md) separates delivered behavior from remaining acceptance work.

## Contextual relationship baseline

The follow-up assessment build passes 366 Rust tests (three external tests remain opt-in), 478 frontend tests, frontend production/size checks, desktop compilation, and a repeated-import MCP stdio smoke. The earlier explicit interview/workbook/prediction checks above describe the initial ingestion build, not fresh repetitions of those external tests.

Run the developer-authored synthetic pair suite with the configured local Qwen model. `--data` must be a new directory with an existing parent; use an isolated location outside your evidence vault. The command makes no model downloads and refuses to overwrite previous results.

```powershell
grafyn-evidence-maintenance relationship-benchmark --data C:\repair\relationship-baseline-01
```

The 12 labeled pairs cover concrete paraphrases, metric/beneficiary/deadline distinctions, shared-vocabulary false positives, resource conflicts, directional mechanisms and insufficient context. Gold labels and descriptive fixture IDs are excluded from scorer inputs. Candidate discovery sees the combined 24-source corpus, while the scorer evaluates all 12 intended pairs independently of discovery. Without installed embeddings, candidate recall is unavailable rather than zero. The report retains exact inputs/outputs, model and prompt identities, label agreement with both all-pair and accepted-only denominators, direction errors, abstentions and failure categories. Changed/missing model identities make the batch non-comparable. These small development fixtures do not establish broad semantic quality or personal predictive benefit.

The first actual local run on September 5 returned 12 valid judgments with a single unchanged model digest and no provider/parsing failures. Exact label agreement was 7/12 overall and 6/9 among accepted relationship judgments; there were three unrelated verdicts and zero abstentions. Direction differed from the fixture expectation on 3/4 gold-directional pairs. This includes a requires/enables alternative and a symmetric conflict response, not three proven arrow reversals. The explicit shared-budget conflict was missed. These results are below a basis for trusting automatic relationship judgments; label definitions and direction handling need further development and human adjudication before broader quality claims. Candidate recall remains unavailable (10 relevant-pair denominator) because embeddinggemma was not installed. No prompt tuning or training was performed on these results.
