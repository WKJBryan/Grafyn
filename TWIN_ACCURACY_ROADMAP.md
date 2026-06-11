# Twin Accuracy Roadmap: Path to ≥80% Decision Agreement

> **Rewritten 2026-06-11, superseding the 2026-06-10 draft.** The first draft organized the twin around a human-memory metaphor (core memory, consolidation, belief decay). That metaphor was dropped after a first-principles review: human memory architecture solves *biological* constraints (4-chunk working memory, lossy reconstructive storage, values stored in synapses) that an LLM does not share. The human metaphor survives only in the product UI ("my twin remembers…"); the machine underneath is specified below.
>
> **Amended same day after design review:** (1) two-axis data model — evidence *about the person* vs knowledge the person *reasons with*, mapped onto the two tier systems Grafyn already has; (2) constitution-with-receipts — constitution items are evidence-backed indexes rendered with their verbatim grounding excerpts, never evidence-replacing summaries; (3) the twin is **predictive-only** — an "aspirational twin" (who the user endorses being) was considered and rejected as unfalsifiable: it produces no ground-truth labels, so it cannot be benchmarked or improved. Advisor mode remains plain decision support, makes no fidelity claim, and is not benchmarked.

This document complements `TWIN_RAG_SPEC.md` (what the twin is) by defining **how accuracy is created, measured, and improved**.

## The Problem, Stated Precisely

The twin is a frozen LLM. Its weights cannot be changed (no model training — out of scope for this product, full stop). Therefore:

> **The entire twin problem is context construction: build the context string that maximizes the probability the model outputs what the user would output, given a new situation.**

Every mechanism in this roadmap is justified by its contribution to that objective — not by resemblance to human cognition.

## Success Criterion

**Decision agreement:** the twin predicts the user's choice *before* the user reveals it. Accuracy = % of decisions where the sealed prediction matches the actual choice, supplemented by holdout replay of past rankings and feedback. Target: **≥80%**, claimed only when the 95% confidence-interval lower bound clears 70% — and interpreted relative to the user's own measured self-consistency (see Statistical Honesty).

**Scoring is external by design (owner decision, 2026-06-10).** The app captures and exports; all scoring, replay, calibration, and dashboards live in the owner's external evaluation harness. No eval UX is imposed on public-repo users. In-app scope: sealed prediction capture, outcome capture, and export.

## The Error Budget

Every wrong prediction has exactly one of four root causes. Each phase attacks a specific row; the external harness attributes misses to rows so effort goes where the errors are.

| Error source | What it looks like | Lever |
|---|---|---|
| **Coverage** | The relevant case/value/fact was never captured | Phase 2 case mining, Phase 5 targeted acquisition |
| **Retrieval** | It was captured but not selected into context | Phase 3 situation-similarity search |
| **Conditioning** | It was in context but the model's own priors won anyway | Context-policy variants (catalog below), procedural scaffold, model selection |
| **Irreducible** | User's hidden state (mood, unrecorded info) and self-inconsistency | Nothing — this is the ceiling. Measure it, don't chase it |

## Current-State Assessment (verified against code)

| # | Finding | Where |
|---|---------|-------|
| 1 | **No accuracy measurement exists.** `DecisionEpisode` captures decision/options/chosen_option/outcome/regret, but nothing compares a twin prediction to the actual choice. `ReflectionScores` measure style traits via phrase matching, not correctness. Export splits are written but unconsumed. | `models/twin.rs:344-375`, `services/twin_store.rs:3063-3239`, export at `twin_store.rs:1736-1930` |
| 2 | **The richest asset is treated as a peripheral log.** `DecisionEpisode` — situation, options, choice, rationale, outcome — is exactly the structure of a behavioral-cloning exemplar, but it is never retrieved into twin context. The context is built from *abstracted records about* the user instead of *demonstrations by* the user. | `commands/canvas.rs:2827-2999` (`build_twin_context_prompt`) |
| 3 | **Retrieval is purely lexical.** Tantivy BM25 + keyword overlap; `SimilarityProvider` is a designed-but-unimplemented embedding path. "Should I take the Denver job?" cannot retrieve "I value proximity to family over salary" — nor the past case where the user turned down a relocation. | `services/search.rs`, `twin_store.rs:3662-3701` (`twin_record_relevance`), `services/similarity.rs:6-9` |
| 4 | **Twin inference learns app habits, not the user.** `local-signal-v1` = 13 hardcoded heuristics ("uses debate", "prefers structured answers"). Verbatim rationales, corrections, debate choices, and decision lessons sit unmined as 220-char excerpts. | `twin_store.rs:270-397`, signals at `twin_store.rs:3313-3498` |
| 5 | **Context dilution.** ALL endorsed/auto-promoted records enter every twin prompt regardless of relevance; no total token budget. Endorsement is a data-quality verification flag being misused as an inclusion privilege. | `commands/canvas.rs:2827-2999` |
| 6 | **Temporal validity is unmodeled.** `valid_from`/`valid_until` exist but are never populated; `RecordLink "supersedes"` is never created; recency decay conflates "old" with "no longer true". | `models/twin.rs` (RecordLink), `services/memory.rs:72-133` |
| 7 | **Capture is 100% active and high-burden, and the twin never makes a falsifiable prediction.** | `CanvasContainer.vue` feedback controls, `TwinReviewView.vue` |

**Constraints honored throughout:** desktop-only, local-first (no hosted backend), context-construction only (no model training), records marked `rejected`/`private`/`no_train` are never used, OpenRouter for cloud + Ollama for local, scoring external.

---

## Target Architecture

### The two axes

Every decision depends on two distinct inputs, and Grafyn already has a separate, mature trust ladder for each. They were built for different features and must now be deliberately composed:

| Input to a decision | What it is | Existing pipeline | Trust ladder |
|---|---|---|---|
| **Person-model** — values, procedure, past choices | Evidence *about* the user | Twin records / cases / constitution | `Candidate → AutoPromoted → Endorsed` (PromotionState) |
| **Knowledge base** — domain expertise, settled facts about the world | Knowledge the user reasons *with* | Vault notes | `draft → evidence → canonical` (note status) |

This classification matters because misrouting destroys signal: a frequently-quoted article is not weak *identity* evidence — it is strong *domain knowledge* with a salience signal attached. A CV is not curated self-presentation to be distrusted — it is a fact source (roles, skills, domains) for the knowledge axis. A person with deep domain expertise decides differently *because of what they know*; a twin with perfect values but missing the user's knowledge fails exactly where the user's expertise is the deciding factor. Both axes are benchmarked by the same single metric — decision agreement — via `context_version` A/B in the external harness.

### Components

Six data structures plus one policy, replacing the memory-tier model:

### A. Decision case library — the center of gravity
Structured episodes: situation, options, the user's choice, **verbatim rationale**, outcome, regret. `DecisionEpisode` already is this struct; it becomes the primary retrieval target instead of a log. Rationale for centrality: a verbatim demonstration conditions the LLM on the user's values *and* reasoning procedure simultaneously (in-context learning is the model's strongest native mechanism), and **two episodes can never contradict each other** — only generalizations extracted from them can. Cases natively express context-dependent preferences that abstracted claims force into false conflicts.

### B. Constitution — evidence-backed index, rendered with receipts
Explicit value rules, ranked, **induced from behavioral evidence** (transcripts, prompts, decision rationales) as well as guided setup — not free-form self-report. Job: tie-breaking and out-of-distribution coverage when no case is near. Always in context **because it is small and globally applicable** — not because it is "core memory." Hard cap (~10-15 items in twin context).

**The receipts rule:** evidence-backing fixes *bias* (rules grounded in what the user did, not what they wish), but not *underdetermination* — the induction step still discards the weights, thresholds, and exceptions, which live only in the raw moments. So the constitution is an **index over its evidence, never a replacement for it**: every item keeps mandatory evidence links, and at context-assembly time each included item is rendered together with 1-2 of its verbatim grounding excerpts (the transcript moment, the actual prompt, the decision rationale). Rule + receipts. An LLM holding only the rule reinvents the weights with *its* priors; the excerpts carry the user's.

### C. Procedural scaffold — how the user decides, as instructions
A one-time short interview capturing the user's decision *procedure*: speed (gut-first vs deliberate), risk posture, satisficer vs maximizer, who they consult, instant-rejection criteria. Rendered as explicit reasoning instructions in the twin prompt ("First reject any option violating X. Decide quickly; don't enumerate exhaustively. Weight downside 2:1."). This directly counters the LLM's assistant-default reasoning style — the largest *conditioning*-row error source. Lives alongside `ConstitutionSetup`; small build, outsized effect.

### D. Bitemporal fact store
Facts about the user's world (job, family, location, constraints) with validity intervals — wire the dormant `valid_from`/`valid_until`. A changed fact is a *versioning* event (end-date the old row), not a "belief conflict." Add `last_confirmed_at`: re-confirmation resets the staleness clock. Recency decay is demoted to tiebreaker only.

### E. Knowledge base — the vault, status-weighted
The user's domain expertise is a first-class twin input, served by the existing vault and its `draft → evidence → canonical` ladder. **Canonical = settled knowledge the user has verified and stands behind** — the twin reasons from it the way the user would, so it gets the strongest retrieval boost into twin context (the priority-scoring status boosts already exist; twin context assembly must lean on them hard). Evidence-status notes surface with lower weight; drafts barely at all. Every note the user promotes to canonical is them telling the twin "this is part of how I see the world" — the existing curation workflow becomes direct accuracy leverage. **Reference-frequency salience:** sources the user repeatedly quotes or cites in prompts/transcripts get a retrieval boost and a promote-to-canonical suggestion — what someone quotes constantly is functionally part of their reasoning apparatus.

### F. Voice exemplar bank — Simulation mode only
Verbatim snippets of the user's writing/speech, tagged by register (meeting, chat, written). Decision accuracy and stylistic mimicry are different objectives with different payloads; Advisor/decision context spends zero tokens on voice, Simulation retrieves register-matched exemplars. Distilled style *descriptions* are not a substitute — distillation destroys voice.

### G. Selection policy — budgeted, relevance-ranked context assembly
Given a decision, fill a hard token budget by expected information value: **nearest cases first** (they dominate), then applicable constitution rules with their receipts, then procedural scaffold, then relevant facts, then status-weighted knowledge, then voice exemplars if simulating. Every assembled context is tagged `context_version`. No tier has unconditional inclusion rights except the (capped) constitution and scaffold.

**The eval loop is the learning algorithm.** With frozen weights, the system cannot improve by accumulating data alone — it improves by hill-climbing the selection policy against sealed-prediction outcomes. Phase 1 is not "measurement infrastructure"; it is the twin's optimizer. Every technique below is a policy variant the external harness can A/B on identical replayed episodes.

---

## Conditioning Techniques Beyond Cases (catalog)

Brainstormed alternatives/complements to plain case retrieval. Each is a context-policy or inference-policy variant — cheap to prototype, measurable via replay, adoptable only if it moves agreement. Ordered by expected value:

| # | Technique | Mechanism | Why it might work | Cost / risk |
|---|---|---|---|---|
| 1 | **Contrastive pairs (negative exemplars)** | Show chosen-vs-rejected, Matches-Me vs Not-Me side by side: "In situation S, I chose A **not** B because…" | Preference pairs carry far more signal than positive examples alone; they sharpen the decision boundary. **The data already exists** — ranking events, debate winners, Not-Me labels are captured today and currently unused in context. | Near zero — context formatting only |
| 2 | **Correction bank (error-driven exemplars)** | Store every twin miss with the user's actual choice + explanation; retrieve as "lessons" when similar situations recur: "Last time you predicted X; I chose Y because…" | Highest-information exemplars in existence — they mark exactly where the model's priors override the user. Generated automatically by the Phase 1 loop as a free byproduct. | Near zero once Phase 1 ships |
| 3 | **Frame-then-decide (two-step inference)** | Step 1: "Rewrite this decision as {user} would frame it to themselves — what they'd consider salient, what they'd ignore." Step 2: decide from the reframed problem. | People differ more in how they *frame* decisions than in how they solve framed ones; framing is where assistant-default reasoning does the most damage. | 2× inference cost on decision tiles |
| 4 | **Generate-then-rank (best-of-N selection)** | Generate N candidate decisions/answers; score each by similarity to the user's past choices and voice (embeddings + nearest cases); emit the best. | Converts the problem from "generate like the user" (hard) to "recognize which output is most like the user" (easier — discrimination beats generation). | N× inference cost; needs Phase 3 embeddings |
| 5 | **Critic pass (twin self-check)** | After drafting, a second call checks the draft against the constitution and nearest cases: "Would {user} actually say this? What would they object to?" Revise once. | Catches constitution violations and prior bleed-through cheaply; mirrors how rubric-checking improves LLM outputs generally. | 2× inference cost; can overcorrect toward rules |
| 6 | **Ensemble vote across context samples** | Run the sealed prediction k times with different retrieved-case subsets; majority vote, disagreement = low confidence. | Averages out retrieval noise; the disagreement signal is a free, well-calibrated confidence estimate. | k× inference cost; reserve for sealed predictions only |
| 7 | **Persona narrative (user-authored)** | A first-person "how I am" page the user writes/edits directly — narrative, not trait list — included in Simulation context. | User-authored narrative beats machine-extracted trait lists for persona fidelity; also the cheapest user-facing trust lever ("the twin runs on what I wrote about myself"). | One editor view; staleness — pair with `last_confirmed_at` |
| 8 | **Model selection as a fitted parameter** | The external harness scores agreement per base model (already multi-model via Canvas/Ollama) and the user adopts the model whose priors fight them least. | Different base models have measurably different reasoning priors; picking the closest one is free accuracy with zero in-app work. | None in-app — export already carries `model_id` |
| 9 | **Trait-vector calibration** | Forced-choice psychometric items → structured scores (risk tolerance, deliberation speed) as compact prompt parameters. | Weak alone (labels invite stereotyped role-play) but a cheap complement to the procedural scaffold; doubles as Phase 5 interview content. | Low; least promising standalone |

**Adoption rule:** none of these ships on faith. Each enters as a `context_version` variant, gets replayed on identical holdout episodes externally, and is kept only if the agreement delta clears noise. Items 1-2 are near-free and should be in the first wave; 3-6 trade inference cost for accuracy and are worth testing once a baseline exists; 7-9 are cheap complements.

---

## Phase 0 — Context Assembly Hygiene (2-3 days, alongside Phase 1)

**Objective:** raise signal-to-noise in twin prompts so the Phase 1 baseline isn't artificially depressed. (Justification is attention dilution and lost-in-the-middle — not a "core memory" tier.)

- **Remove unconditional inclusion of endorsed/auto-promoted records.** Endorsement is verification (label QA), not an inclusion privilege. Gate all records through `twin_record_relevance`; only the capped constitution + procedural scaffold are always-on.
- Hard total token budget on the assembled twin context (mirror the existing 4000-token chunk-budget pattern), filled in selection-policy order: nearest decision cases > constitution > scaffold > relevant records > relevant facts.
- **Start retrieving decision cases into context now**, even with lexical matching — `DecisionEpisode` data exists today and currently never reaches the prompt. Lexical kNN is a weak version of the end-state, but it establishes the case-first prompt shape immediately.
- Place critical content at the start and end of the assembled context (serial-position effects are real in LLMs); tag every assembly with `context_version` in trace events.

**Measured by:** external harness, segmented by `context_version`.

## Phase 1 — Sealed Predictions + Export (1.5-2 wk) — the optimizer loop

**Objective:** every Decision tile produces a sealed, falsifiable twin prediction stored alongside the user's actual choice; exports make all policy variants externally comparable. This phase is the twin's improvement engine, not just its scoreboard: with frozen weights, the system learns *only* by varying the context policy and selecting what wins.

### Sealed prediction in the Decision Mirror lifecycle

Current flow: tile marked Decision → `record_decision_episode` → models answer → user later records outcome via `DecisionOutcomeUpdate`. Insert:

1. When a Decision tile runs, fire one hidden LLM call (twin context + decision + `options[]`) returning strict JSON: `{"predicted_option", "option_index", "confidence", "rationale"}`.
2. **Do not display it.** Store immediately; UI shows a locked badge ("Twin has sealed a prediction").
3. When the user records `chosen_option` (existing path), the backend computes agreement and reveals prediction + rationale. The reveal moment also feeds the **correction bank** (technique #2): on a miss, one optional free-text line — "why was the twin wrong?" — becomes a retrievable lesson exemplar.

**Storage** — extend `DecisionEpisode` with an optional, `#[serde(default)]` struct (existing JSON store migrates for free):

```rust
twin_prediction: Option<TwinPrediction> {
    predicted_option, matched_option_index, confidence,
    rationale, model_id, context_version, sealed_at,
}
agreement: Option<bool>   // computed when chosen_option is set
```

**Predicted-option parsing fallback chain:** strict JSON → normalized string match against `options[]` → (after Phase 3) embedding similarity → one-tap manual adjudication, counted but tagged.

**Integrity rules:**
- `sealed_at` must precede the `chosen_option` write timestamp, or the episode is excluded from accuracy.
- Capture `initial_leaning` before responses render (field exists) and export the **influence rate** — % of episodes where the final choice converged toward the twin's prediction away from the leaning — as the leakage control.

### Export obligations (scoring happens externally)

The app must ensure exports include: sealed predictions with `context_version` + `model_id`, decision outcomes, full ranking/feedback trace payloads (the contrastive-pair raw material), correction-bank entries, and the existing train/eval/holdout splits — all under the `rejected`/`private`/`no_train` filter. The external harness owns: top-1 agreement, Kendall tau on ranking replay, Matches-Me label agreement, Wilson CIs, calibration curves, per-model and per-`context_version` breakdowns, and A/B replay of every conditioning technique in the catalog.

### Statistical honesty

- Binomial 95% CI half-widths near 80% true accuracy: N=25 → ±16pp, N=50 → ±11pp, N=100 → ±8pp. **80% is indistinguishable from 70% until N≈100-130 scored decisions.** Replay inflates effective N; Phase 5 micro-decisions count as episodes.
- **The ceiling problem:** agreement is scored against a person who is only ~70-90% consistent with themselves on repeated preference elicitation. The external harness should occasionally replay disguised repeat items to estimate the user's own test-retest consistency — that number, not 100%, is the ceiling the twin is chasing. If self-consistency is 85%, sustained 80% agreement is near-optimal performance.

## Phase 2 — Case Mining + Procedural Scaffold (2-3 wk) — largest expected accuracy lift

**Objective:** grow the case library and capture the decision procedure. **This phase mines *cases*, not claims.** The prior draft centered on extracting abstracted Fact/Preference/ReasoningPattern claims; that inherited human memory's compression habit on a substrate whose greatest strength is learning from verbatim examples. Abstractions also manufacture contradictions the underlying episodes don't have — keeping claims minimal shrinks Phase 4's workload at the source.

- New `services/twin_miner.rs`: batched background LLM extraction over imports (chat exports, **meeting transcripts** — strong sources of real decisions), trace events, and conversations → structured **decision cases** `{situation, options[], choice, verbatim_rationale, date, source_refs[]}` stored as `DecisionEpisode`-compatible records. The verbatim rationale is preserved, never paraphrased.
- **Two-axis router:** every extracted piece is classified onto one of the two axes and sent down the corresponding existing pipeline. *About-you* → twin pipeline (cases, constitution candidates with evidence links, facts), enters as `Candidate` behind the review gate. *Knowledge* → vault pipeline as normal notes (`draft → evidence → canonical`), wikilinked and hub-clustered like any other note. One transcript yields both: "chose vendor A over B because of lead-time risk" → case; "vendor B's lead times run 6-8 weeks" → knowledge note. CVs, articles the user appears in, and quoted/recurring reading material route primarily to the **knowledge** axis (fact sources + salience), not the identity axis.
- **Evidence-class weighting** for about-you claims — miner confidence scales by how *revealed* (vs performed) the source is: revealed choices (decisions with outcomes, debate winners, user edits) > spontaneous transcript utterances > Canvas prompts (strong for deliberative criteria and values, zero for interpersonal style — people perform less impression management toward an AI, but prompting register carries no social behavior) > everything else. Self-descriptions and quoted material are weak *identity* evidence on their own; their proper home is the knowledge axis or as receipts under a constitution item.
- **Secondary output only:** durable facts (→ bitemporal fact store) and explicit self-statements in the user's own words (→ constitution candidates). No machine-paraphrased preference claims as a primary product.
- All mined items enter as `Candidate` behind the existing review gate with mandatory evidence links; provenance tag (`case-miner-v1` vs `local-signal-v1`) so the harness attributes accuracy by source. Ollama-only mining mode for privacy. Dedupe before insert.
- **Procedural scaffold interview:** one-time, ~10 forced-choice + short-answer items on decision style (speed, risk posture, rejection criteria, who they consult); rendered into the always-on scaffold section of the twin prompt. Re-confirmable (resets `last_confirmed_at`).
- **Voice exemplar bank:** during mining, flag high-signal verbatim snippets and tag by register (meeting/chat/written); Simulation mode retrieves register-matched exemplars instead of style descriptions.
- **Bulk review** in `TwinReviewView.vue`: multi-select, accept/reject-all, keyboard shortcuts, grouping — mining multiplies candidate volume and per-item review will not survive it.

**Measured by:** external replay delta before/after the mining backlog; acceptance rate of mined cases (target >60%).

## Phase 3 — Situation-Similarity Retrieval (1.5-2 wk)

**Objective:** the right cases surface for new situations. The primary index is the **case library** — "what did this person choose in the most similar past situations?" is the single highest-value retrieval question in the system. Notes/chunks are the secondary index.

- Implement the existing `SimilarityProvider` path: `OllamaEmbeddingProvider` with `encode_batch`; extend `services/ollama.rs` with `/api/embeddings` (default `nomic-embed-text`). Local-only embedding keeps privacy trivial.
- Embed: **decision cases (situation + options text)** first; then chunks, records, constitution items, correction-bank entries, voice exemplars.
- Vector store: flat sidecar table `(id, kind, model, content_hash, dim, vec_blob)`; brute-force cosine is fine at desktop scale; no ANN dependency.
- Hybrid scoring `score = α·norm(BM25) + (1-α)·cosine` (α ≈ 0.4) for case selection and chunk retrieval; replaces keyword-overlap in twin context assembly.
- **Graph-neighbor expansion:** when a case or note is selected, optionally pull 1-hop linked neighbors (typed-link weighted, budget-capped) — the mechanism already exists for notes via `graph_hop_depth`; extend neighbor *content* inclusion to twin context and traverse `RecordLink` edges the same way.
- **Status-weighted knowledge retrieval into twin context:** the priority-scoring status boosts already exist — twin context assembly applies them so canonical notes dominate the knowledge slots, evidence-status notes surface lower, drafts barely at all (Architecture E).
- **Reference-frequency salience:** count how often a note/source is referenced in prompts, transcripts, and conversations; high counts add a retrieval boost term to the existing priority scoring and trigger a promote-to-canonical suggestion. A counter plus a boost term — not a new system.
- **Constitution receipts at assembly time:** each included constitution item is rendered with 1-2 of its linked verbatim evidence excerpts (Architecture B); the excerpt lookup rides the same retrieval infrastructure.
- Lazy + background indexing keyed by content hash; if Ollama is absent, degrade to lexical and tag the mode into `context_version`.

**Measured by:** external A/B — identical replayed episodes, lexical vs hybrid case retrieval.

## Phase 4 — Temporal Validity (1 wk, shrunk from the prior draft)

**Objective:** the twin reasons from the user's *current* world. The prior draft planned semantic belief-revision machinery; most of that workload was self-inflicted by claim over-abstraction and disappears with case-first Phase 2. What remains:

- **Bitemporal facts:** populate `valid_from`/`valid_until`; a changed fact end-dates the old row (versioning, not conflict). Context assembly uses only currently-valid facts; add `last_confirmed_at`, reset on re-confirmation.
- **Rare genuine conflicts** (value drift between explicit self-statements): flag via embedding similarity + cheap LLM verifier; resolve via review cards — **newer supersedes** (creates the `supersedes` `RecordLink`), **both true / context-dependent**, **older still correct**. Time is a *prior* favoring the newer statement, never an auto-verdict; no silent supersession.
- Context assembly: exclude superseded records; annotate unresolved conflicts explicitly rather than silently picking one.
- Rewire `find_contradictions` (`services/memory.rs`) to the same path.

**Measured by:** external replay delta; miss-tagging at reveal ("twin used outdated info").

## Phase 5 — Targeted Case Acquisition (2 wk)

**Objective:** fill empty regions of case-space — the precise meaning of "information gain" here is *ask where the case library has no neighbors*. Each answer is simultaneously a context exemplar and an eval point (sealed prediction first → grows N).

Anti-fatigue mechanics are first-class requirements, not polish:

- **Single capped inbox:** all elicitation (interview items, conflict cards, mined-case review) flows through one queue, hard-capped (~5 items/day surfaced). The system may *generate* more; it may not *show* more.
- **Forced-choice only** for elicitation — no free-text essays; optional one-line rationale.
- **Earn-the-right-to-ask:** an item enters the queue only if its expected information gain clears a bar (no nearby cases, or the harness shows a weak domain). No generic "tell me about yourself" items, ever.
- **Auto-expiry:** stale candidates and unanswered items expire silently; an old queue is a guilt list, not a dataset.
- **Warm-moment piggybacking:** post-decision reveal is the one moment users tolerate one extra question ("would you decide the same way for a smaller amount?") — one item max, skippable.
- Decision UX: structured option chips on Decision tiles (the main parse-failure source); pending-reveal reminders.
- Passive signals (low-creep, already in-app): debate-winner choice, copy/export of one response over another, **user edits to twin output** (an edit is a free correction-bank entry). Skip dwell-time/keystroke tracking.

**Measured by:** external coverage maps (case density per domain) and agreement trend in previously-weak domains; queue answer-rate (>70% target — below that, the gain bar is too low).

---

## Risk Register

| Risk | Mitigation |
|------|------------|
| **Small-N noise** | Wilson CI always reported externally; replay multiplies effective N; micro-decisions count; success claimed only when CI lower bound > 70% |
| **Ceiling misread** — chasing 80% when user self-consistency is lower | External harness estimates test-retest consistency via disguised repeats; target interpreted relative to it |
| **Prediction leakage** — twin's visible answer influences the choice | Sealed predictions; `initial_leaning` captured pre-answer; influence rate exported as honesty metric |
| **Prior bleed-through** — model reasons like an assistant, not the user | Procedural scaffold; demonstrations-over-declarations; frame-then-decide and critic-pass variants; per-model agreement comparison externally |
| **Structured-output failures** (esp. local models) | Parsing fallback chain → one-tap adjudication; option chips |
| **Mined-case hallucination** | Candidate-gated review, mandatory evidence links, verbatim-rationale requirement (paraphrase is a rejection criterion), acceptance-rate monitoring |
| **Privacy regressions** | `rejected`/`private`/`no_train` enforced at four choke points: miner input, embedding store, context assembly, export |
| **Ollama absent** | Every path degrades to current lexical/heuristic behavior; mode tagged into `context_version` |
| **Elicitation fatigue** | Capped inbox, auto-expiry, forced-choice, information-gain bar, warm-moment placement |
| **Distribution shift** — replay scores history, not future | Replay treated as regression detector; live sealed predictions remain the headline metric |

## Sequencing Summary

| Phase | Theme | Effort | Primary impact (error-budget row) |
|-------|-------|--------|------------------------------------|
| 0 | Context hygiene, case-first prompt shape, versioning | 2-3 days | Conditioning (S/N) + baseline integrity |
| 1 | Sealed predictions, correction bank, export | 1.5-2 wk | Creates the optimizer loop |
| 2 | Case mining, two-axis router, procedural scaffold, voice bank, bulk review | 2-3 wk | Coverage — largest expected lift |
| 3 | Case-similarity embeddings, graph-neighbor expansion, status-weighted knowledge + receipts | 1.5-2 wk | Retrieval |
| 4 | Bitemporal facts, rare-conflict resolution | 1 wk | Coverage (staleness) |
| 5 | Targeted acquisition, anti-fatigue elicitation | 2 wk | Coverage in empty case-space + raises N |

Conditioning-catalog variants (contrastive pairs, frame-then-decide, generate-then-rank, critic pass, ensemble) slot in *after Phase 1* as `context_version` experiments at any point — they are policy changes, not infrastructure. Total ≈ 8-11 person-weeks. The 80% target becomes statistically defensible once ~100+ sealed episodes accumulate; external replay gives a directional read within weeks of Phase 1.

## Critical Files

- `frontend/src-tauri/src/models/twin.rs` — `DecisionEpisode` extension (`TwinPrediction`), `RecordLink`, `valid_from`/`valid_until`, export models
- `frontend/src-tauri/src/services/twin_store.rs` — relevance, export splits, miner integration, correction bank
- `frontend/src-tauri/src/services/twin_miner.rs` — (new) case mining from imports/traces
- `frontend/src-tauri/src/commands/canvas.rs` — Decision tile lifecycle, `build_twin_context_prompt` (selection policy lives here)
- `frontend/src-tauri/src/services/similarity.rs` — `SimilarityProvider` embedding path
- `frontend/src-tauri/src/services/ollama.rs` — embeddings endpoint
- `frontend/src-tauri/src/services/retrieval.rs` — graph-neighbor expansion pattern to extend; status boosts for knowledge weighting
- `frontend/src-tauri/src/services/priority.rs` — status/recency boosts; reference-frequency salience term lands here
- `frontend/src/views/TwinReviewView.vue` — capped inbox, bulk review, conflict cards
