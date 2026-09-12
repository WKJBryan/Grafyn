# Canvas Persona Usability Harness Design

Date: 2026-09-12

Status: User-approved design, pending written-spec review

## Purpose

Build a development-only harness that lets simulated cognitive and product personas operate the real Grafyn Canvas and produce evidence-linked usability findings.

Bryan is the primary acceptance persona. The Canvas must support his non-linear exploration while helping him compress branches into structure, decisions, and action. Additional personas test whether a failure is specific to Bryan or reveals a broader interface problem.

The harness evaluates both task completion and interface simplicity. A feature does not pass merely because a persona eventually finds it. Frequently required functions must be visible, understandable, and reachable without unnecessary interaction depth.

## Design Decisions

1. Computer Use is the primary persona driver because it operates Grafyn through the visible Windows interface rather than through privileged DOM knowledge.
2. A development-only recorder captures interaction evidence from the app while Computer Use drives it.
3. Playwright prepares deterministic fixtures and verifies proposed improvements, but it does not substitute for an invalid Computer Use persona run.
4. Persona evaluation remains external to the product. It does not become a Twin record, user-facing evaluation dashboard, or production telemetry system.
5. Observations, interpretations, and recommendations remain separate in every report.
6. Heat maps represent interaction, not gaze. Agent latency and cursor position are not presented as human attention measurements.

## Non-Goals

- Predicting how all real users behave.
- Replacing usability tests with people.
- Adding production analytics, surveillance, or consent flows.
- Training or updating the Twin from simulated behavior.
- Evaluating base-model intelligence or Twin prediction accuracy.
- Redesigning Canvas before collecting a baseline.
- Running personas in parallel against shared state.

## System Boundary

```text
Versioned persona profile
        |
Versioned scenario and clean fixture
        |
Computer Use drives the real Grafyn window
        |
Development-only recorder captures interaction evidence
        |
Run artifact package
        |
Deterministic metrics and persona interpretation
        |
Cross-persona report and ranked UI changes
        |
Playwright regression of an approved change
```

The persona operates a real development or packaged Tauri application with the normal Vue Canvas and Rust services. The run uses a temporary owned vault and data root. Paid or variable model responses are replaced with a deterministic local fixture so personas receive equivalent content and failures can be attributed to the interface.

The harness must not read Bryan's private vault by default. A private-context test is a separate, explicitly authorized scenario.

## Components

### Persona profiles

Profiles live under a development-only harness directory as reviewable Markdown. The entire file is versioned and hashed for each run.

Every profile defines:

- identity and short role;
- cognitive pattern and working style;
- trigger for opening Canvas;
- expected mental model of the interface;
- natural exploration strategy;
- expectations of useful defaults;
- confusion and overload triggers;
- abandonment conditions;
- definition of a useful finished result;
- prior knowledge available at the start;
- persona-specific evaluation questions.

The profile is a test hypothesis, not a claim that it represents a demographic group. A persona must genuinely attempt the task and must not manufacture friction. It receives no source-code knowledge, selectors, hidden feature names, or instructions about where controls are located.

### Initial panel

#### Bryan - non-linear systems thinker

Tests whether Canvas preserves branching, relationships, and alternative frames while progressively exposing questions, constraints, synthesis, decisions, and next actions. This is the primary product-fit gate.

#### Mira - unfamiliar knowledge worker

Tests immediate capture, understandable defaults, terminology, onboarding burden, and whether the first useful action is obvious without prior product knowledge.

#### Arun - evidence-led researcher

Tests whether retrieved context, exact sources, provenance, model claims, and uncertainty can be inspected rather than merely trusted.

#### Elena - outcome-driven product lead

Tests whether exploration can become a decision, priorities, ownership, and concrete next actions without requiring the user to manually reconstruct the session.

#### Theo - returning high-volume user

Tests resuming old work, locating unresolved branches, navigating a large canvas, reusing prior structure, and completing frequent actions efficiently.

### Scenarios

Every persona runs the same core mission from the same clean fixture:

> Capture an ambiguous idea, obtain several perspectives, explore one promising branch, compare conflicting responses, produce a useful synthesis or decision, save it, leave, and later resume the work.

The mission states the goal but contains no UI directions. Each persona also receives one targeted mission that stresses its distinctive needs.

Each scenario defines:

- scenario ID and version;
- user-visible goal;
- frozen starting state;
- deterministic model-response fixture;
- capabilities that the task genuinely requires;
- success evidence;
- blocker and abandonment conditions;
- permitted test data;
- explicit exclusions and privacy boundary.

The required-capabilities list is hidden from the persona. It is used by the analyzer to compare feature demand with feature exposure.

### Computer Use driver

The persona subagent observes the current Grafyn window, decides one action at a time, performs it through Computer Use, and re-observes after any state-changing action. It follows Computer Use safety and freshness rules; stale accessibility indexes, screenshot identifiers, or coordinates are never reused.

The driver records a short intent before each action. Intent is evaluation context, not proof of human cognition. Tool and model latency are retained only for operational debugging and excluded from human-usability timing claims.

### Development-only interaction recorder

The recorder is enabled only for harness builds. It records interaction events generated inside the real Canvas without adding production analytics.

For each event it captures:

- run ID and monotonic sequence;
- timestamp;
- route and stable UI state;
- viewport dimensions;
- Canvas pan and zoom state where applicable;
- normalized and absolute click coordinates;
- semantic action identifier and accessible label when present;
- visibility and bounding box of the target;
- current menu or modal depth;
- attempted action and observed outcome;
- backtrack, dead-end, or failed-action classification;
- screenshot or observation receipt identifier.

The recorder samples only interactions needed for the evaluation. It does not record secrets, prompt contents beyond the synthetic fixture, arbitrary keyboard text, real vault content, or background application activity.

### Playwright fixture and regression layer

Playwright owns repeatable setup and post-change verification:

- launch the test build with temporary vault and data roots;
- load deterministic Canvas sessions and model responses;
- assert that the harness recorder is enabled only in test builds;
- capture canonical screenshots for stable UI states;
- replay deterministic regression paths after a UI change;
- verify visibility, labels, menu depth, and expected resulting state.

Playwright passing does not make a Computer Use persona run valid. It verifies mechanics and measurable improvement after the persona identifies a problem.

### Analyzer and reports

Deterministic analysis computes task and interaction metrics from the raw event stream. A separate evaluation pass interprets the evidence through the selected persona. The interpretation cannot alter the raw events.

Cross-persona analysis retains disagreements. It does not average them into a single universal usability score.

## Stable UI States

Heat maps and visibility analysis are grouped by stable interface state rather than flattened into one image:

1. empty Canvas;
2. new-session flow;
3. prompt dialog;
4. populated Canvas;
5. selected response;
6. branch or follow-up flow;
7. debate flow;
8. synthesis or decision state;
9. saved session;
10. resumed session.

Grouping by state prevents coordinates from different modals, pan positions, zoom levels, and viewport sizes from being combined misleadingly.

## Heat Maps and Simplicity Analysis

Every run produces:

- click-density heat maps;
- visible-control maps;
- hidden-action and menu-depth maps;
- dead-end and backtracking maps;
- required-function versus exposed-function maps;
- cross-persona aggregate maps with persona-specific layers retained.

Coordinates are normalized to the active viewport. Each point is also tied to a semantic action identifier when possible, so pan, zoom, and responsive layout changes do not destroy meaning.

Heat maps do not claim eye tracking. Cursor dwell is not treated as attention because simulated-agent timing contains model and tool latency.

### Interaction-depth classes

- **Direct:** visible and usable immediately or with one action.
- **Shallow:** reachable within two actions.
- **Buried:** requires three or more actions or an undisclosed prerequisite.
- **Undiscovered:** not found without assistance.

A frequently required function that is Buried or Undiscovered fails the simplicity gate for that scenario.

Button density is not automatically a failure. It becomes a failure when low-value controls compete with repeatedly required actions, create misclicks, obscure hierarchy, or cause the persona to search elsewhere.

## Metrics

The harness reports dimensions separately:

- task completion and final state;
- first-action discoverability;
- interaction depth for every required capability;
- number of unnecessary actions;
- dead ends and backtracks;
- important-function visibility;
- hidden prerequisites;
- control competition and misclicks;
- recovery after an incorrect action;
- resumability after leaving the session;
- exploration value;
- compression value;
- decision and next-action value;
- confidence and evidence completeness.

No composite score may hide a blocking failure. The Bryan verdict remains Strong Fit, Conditional Fit, Weak Fit, or Anti-Fit, followed by Why, Failure Point, Improvement, and Broader User Segment.

## Run Artifacts

Each run produces an immutable development artifact package:

```text
artifacts/<run-id>/
|-- manifest.json
|-- actions.jsonl
|-- observations.jsonl
|-- screenshots/
|-- heatmaps/
|-- metrics.json
|-- persona-report.md
`-- integrity.json
```

The manifest contains the app Git SHA/build, platform, viewport, persona ID and content hash, scenario ID and content hash, fixture hash, Computer Use capability result, and analyzer version.

The report links every conclusion to action-event and screenshot receipts. It separates:

1. **Observed:** what visibly happened.
2. **Interpreted:** what the evidence suggests for this persona.
3. **Recommended:** the smallest interface change likely to remove the friction.
4. **Validation:** the expected measurable difference on an identical rerun.

Generated artifacts are gitignored unless a specific baseline is intentionally promoted for review.

## Finding Priority

Findings are ranked by:

- frequency of the required capability within the scenarios;
- number of affected personas;
- completion impact;
- risk of work loss or destructive surprise;
- interaction-depth gap;
- evidence confidence.

Disagreement is preserved. If Theo benefits from a dense toolbar while Mira is overwhelmed, the system reports the difference and may recommend progressive disclosure rather than declaring the toolbar universally good or bad.

## Improvement Loop

1. Run the complete baseline on the unchanged Canvas.
2. Rank evidence-backed failures.
3. Select one focused interface change.
4. State which failure it addresses and what it replaces or simplifies.
5. Implement the change.
6. Run deterministic Playwright regression checks.
7. Rerun the identical Computer Use scenarios.
8. Compare paths, interaction depth, task outcome, and heat maps.
9. Keep, revise, or revert the change based on the evidence.

The harness does not automatically modify production UI. Product changes remain separately reviewed implementation work.

## Invalid and Blocked Runs

A run is invalid when:

- Computer Use cannot target exactly one Grafyn window;
- the expected development recorder is not active;
- the build, profile, scenario, or fixture identity is missing;
- required action or observation logs are incomplete;
- a click outcome is uncertain and not re-observed;
- the starting state differs from the scenario manifest;
- real private data appears without explicit authorization.

Infrastructure failures, Computer Use failures, model-fixture failures, and UI failures receive different classifications. Playwright cannot silently replace an invalid persona run.

When Computer Use is unavailable, the harness may still run fixture and regression diagnostics, but it reports that no persona usability conclusion was produced.

## Evidence and Privacy Safeguards

- Mark every result `simulated_persona`.
- Keep run artifacts outside the user vault and Twin store.
- Never call normal Canvas-to-Twin feedback, decision outcome, Constitution, or GitHub feedback paths from a persona run.
- Use synthetic fixtures and temporary owned data roots by default.
- Do not share memory or mutable state between persona runs.
- Freeze the profile, scenario, build, fixture, and analyzer identity.
- Treat generated decisions and outcomes as test artifacts, not personal facts.
- Do not present simulated frequencies as population usage statistics.
- Require real-user evidence before claiming broad external usability.

## Initial Success Criteria

The first implementation is successful when it can:

1. preflight Computer Use and target exactly one real Grafyn window;
2. run Bryan and the four comparison personas serially against the same clean core scenario;
3. preserve identical deterministic model content and starting state across runs;
4. capture semantic action events, coordinates, visible state, screenshots or observation receipts, and outcomes;
5. generate state-specific interaction heat maps and function-depth analysis;
6. produce evidence-linked persona reports and a disagreement-preserving cross-persona report;
7. mark missing Computer Use capability or incomplete evidence as invalid rather than substituting another test;
8. prove that no run writes simulated output into the real vault, Twin, or production feedback system;
9. preserve comparable run identities and metrics so a later approved UI change can be evaluated against the same baseline.

## Initial Baseline Questions

The first real run should test, without assuming the answer:

- Can a half-formed thought be captured with little setup?
- Are the most frequently required actions visible at the moment they are needed?
- Do defaults allow a useful first result?
- Can a persona understand and inspect what context was used?
- Can branches be compared and compressed into a synthesis or decision?
- Can the persona identify the next action?
- Can old work be resumed without reconstructing the session manually?
- Are destructive or irreversible actions described accurately and recoverable?
- Does the Canvas reduce cognitive work, or create another structure the user must manage?

These are hypotheses for measurement. Existing source-level concerns are not recorded as validated usability findings until a complete Computer Use baseline produces evidence.
