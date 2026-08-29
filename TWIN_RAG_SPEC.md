# Grafyn Native RAG Twin Spec

## Summary

Grafyn's first native twin is a Canvas context mode, not a newly trained model. It combines a configured Twin Identity, the existing vault retrieval pipeline, reviewed Constitution items, and reviewed twin records so a selected model can answer with the user's notes, preferences, reasoning patterns, and explicit negative boundaries in view.

This milestone ships the usable RAG twin path with first-person Simulation. It does not add personality tests, a local model runtime, or scratch training.

## Current Capture Layer

Stage 1 already captures explicit and passive evidence:

- Canvas feedback: accept, reject, correction, ranking, and captured insight.
- Canvas behavior: branching, model comparison, debate, regeneration, note export, and think-harder flows.
- Notes behavior: note creation/update and canonical promotion.
- Inferred user records: `fact`, `preference`, and `reasoning_pattern`.
- Review states: `candidate`, `endorsed`, `rejected`, `private`, and `no_train`. The legacy serialized `auto_promoted` value remains readable for audit only and is treated as pending/candidate rather than authority.

Only explicitly endorsed records enter approved export/training splits. Candidate and rejected records retain their separate legacy review/export surfaces, while legacy `auto_promoted` records are excluded from all export and training files. Rejected records are negative evidence for future pipelines, not live personalization context.

## Governed State Core

The append-only Twin event spine now has a pure deterministic read-model layer:

- exact repeated observations create pending proposal drafts after three supports;
- exact affirm/deny observations create two pending drafts and an unresolved contradiction cluster;
- global, single-relationship, and composite-relationship variants remain distinct;
- only an explicit accepted `MemoryReviewed` event creates reviewed memory;
- concurrent contradictory reviews remain pending until a causally later review resolves them;
- validity, expiry, reinforcement, and supersession are evaluated at an explicit reference time;
- projection snapshots use sorted vectors, integer values, and a content-derived v1 snapshot ID;
- Recall, Decision, Simulation, Reflection, and Capture Review use fixed integer attention profiles with hard governance gates before scoring.

This layer is implemented as pure Rust entry points. Capture hooks and projection-backed UI/API integration belong to later tasks; no current command is documented as emitting these events yet.

## Native RAG Twin Architecture

Twin Mode lives inside Canvas as `context_mode: "twin"` and reuses the existing model execution path.

Context assembly:

- Require Twin Identity name and role/context before Simulation mode can run.
- Inject Twin Identity before Constitution so the model has a first-person operating identity before it receives priors and evidence.
- Retrieve relevant vault notes/chunks through Grafyn's existing retrieval service.
- Include approved twin records only when explicitly `endorsed`.
- Include candidate records only when locally relevant to the prompt.
- Exclude `rejected`, `private`, and `no_train` records from live answer context.
- Store used note ids and twin record ids on the prompt tile and trace event.

The model receives a system prompt with separated sections:

- `Twin Identity`
- `Reviewed Constitution`
- `Action Gap Risks`
- `Relevant Evidence`
- `Approved User Records`
- `Tentative Candidate Records`
- `Answer Instructions`

Candidate records are labeled as unreviewed hypotheses and must not be treated as facts. Legacy `auto_promoted` records are not admitted to either prompt section.

## Answer Modes

Twin Mode supports two answer modes:

- `advisor`: decision-support mode. It uses reviewed memory to help the user reason, while separating grounded evidence from recommendation.
- `simulation`: first-person configured twin voice. It says `I am {twin_name}`, reasons from the configured role/context, and continues the documented reasoning pattern using selected evidence and reviewed Constitution.

Advisor stays a decision-support assistant. Simulation maximizes mimicry in the model-facing prompt; the app UI and docs disclose that it is a configured twin simulation.

## UI and Review

Twin Mode prompt context includes:

- Twin Identity
- reviewed Constitution
- action gaps
- notes
- approved twin records
- tentative candidate twin records

Twin Review remains the place to endorse, reject, private, or no-train records. Canvas feedback continues to feed the evidence loop after each answer.

## Decision Ledger Follow-Up

Better decisions require outcome feedback, not only personalization. The next milestone should add a structured decision ledger:

- decision
- options
- chosen option
- rejected options
- assumptions
- expected outcome
- confidence
- review date
- actual outcome
- lesson learned

That loop makes calibration and decision-quality analysis possible.

## Scratch-Trained Model Discussion

Training a language model from random initialization only on one person's data is not this milestone. One person's data is usually too small to train language ability or general reasoning from scratch.

More realistic later components:

- a personal retriever trained on Grafyn data
- a preference/ranking model from accept/reject/rank/correct feedback
- local embedding or clustering over notes and traces
- a decision-pattern classifier
- a small adapter on top of a general base model

The v1 twin should therefore be Twin Identity plus native RAG and reviewed behavioral records. Scratch-trained personal models remain a research path after the data contract and decision/outcome records are stable.
