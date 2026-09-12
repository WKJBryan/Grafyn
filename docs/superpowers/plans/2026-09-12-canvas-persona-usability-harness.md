# Canvas Persona Usability Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a development-only harness in which Computer Use personas operate the real Grafyn Canvas surface while deterministic tooling records interaction evidence, renders state-specific heat maps, and produces evidence-linked usability reports.

**Architecture:** Run the real Vue Canvas against Grafyn's feature-gated production Rust system-test runtime and deterministic loopback OpenRouter fixture. Computer Use is the unprivileged visual driver; an opt-in frontend recorder sends bounded semantic interaction events to a loopback collector, and pure Node analysis converts those events into metrics, SVG heat maps, and report scaffolds. Playwright owns fixtures and regression assertions, not persona interpretation.

**Tech Stack:** Vue 3, Vite, Vitest, Node.js ESM and `node:test`, Playwright, Rust/Tauri feature-gated system runtime, Computer Use.

**Spec:** `docs/superpowers/specs/2026-09-12-canvas-persona-usability-harness-design.md`

## Global Constraints

- Execute from an isolated worktree created with `superpowers:using-git-worktrees`.
- Target a branch containing the system E2E runtime from `feat/evidence-workspace` (`frontend/src-tauri/src/test_runtime.rs`, `frontend/src/api/e2eTransport.js`, and `e2e/fixtures/openrouter-stub.js`). The current `main` snapshot does not contain it; if these files are absent, stop instead of duplicating that subsystem.
- Use the real Vue Canvas and production Rust commands/services. Only the paid OpenRouter boundary may be replaced with the deterministic loopback fixture.
- Treat the browser-served system-test surface as development Canvas evidence, not packaged-native lifecycle proof.
- Computer Use is the primary persona driver. A Playwright-only run cannot be labelled a valid persona run.
- Run personas serially with a fresh owned runtime root for each run.
- Store generated artifacts only under `e2e/test-results/persona-harness/`; keep that directory gitignored.
- Never read a real private vault or write simulated data to the real vault, Twin, Constitution, decisions, or production feedback paths.
- Never record prompt text, arbitrary typed text, secrets, API keys, or background application activity.
- Treat heat maps as click/navigation evidence, not eye tracking or population telemetry.
- Do not add a production analytics dependency or product-facing evaluation UI.
- Preserve observed facts, persona interpretation, and recommendations as separate report sections.
- Keep edited source files below the repository's 2,500-line hard limit and approximately 1,500 lines as the authoring target.

## Target File Structure

```text
e2e/persona-harness/
|-- README.md                         operator workflow and Computer Use contract
|-- contracts.mjs                    profile/scenario loading and hashing
|-- contracts.test.mjs
|-- recorder-server.mjs              bounded loopback event collector
|-- recorder-server.test.mjs
|-- analyze-run.mjs                  metrics, SVG heat maps, report scaffold
|-- analyze-run.test.mjs
|-- start-run.mjs                    runtime/stub/Vite/collector lifecycle
|-- start-run.test.mjs
|-- profiles/
|   |-- bryan.md
|   |-- mira.md
|   |-- arun.md
|   |-- elena.md
|   `-- theo.md
`-- scenarios/
    |-- core-ambiguous-idea.json
    |-- bryan-compression.json
    |-- mira-first-capture.json
    |-- arun-source-inspection.json
    |-- elena-decision.json
    `-- theo-resume-large-session.json

frontend/src/personaHarness/
|-- config.js                         strict dev-only configuration
|-- recorder.js                       DOM event capture and batching
`-- uiState.js                        stable state and semantic target resolution

frontend/src/__tests__/unit/personaHarness/
|-- config.spec.js
|-- recorder.spec.js
`-- uiState.spec.js

e2e/tests/persona-harness.spec.js      deterministic fixture/regression coverage
```

Existing files modified:

- `frontend/src/main.js` — install the recorder only when strict dev-only configuration is present.
- `frontend/src/views/CanvasView.vue` — stable state and action identifiers for session creation/resume.
- `frontend/src/components/canvas/CanvasContainer.vue` — stable identifiers for Canvas-level actions.
- `frontend/src/components/canvas/PromptDialog.vue` — stable identifiers for prompt options and submission.
- `frontend/src/components/canvas/LLMNode.vue` — stable identifiers for response actions.
- `frontend/src/components/canvas/DebateNode.vue` — stable identifiers for debate actions.
- `e2e/package.json` — harness scripts only; no new runtime dependency.
- `CLAUDE.md` — replace the design-only note with exact run and validation commands after implementation.

---

### Task 1: Versioned Persona and Scenario Contracts

**Files:**

- Create: `e2e/persona-harness/contracts.mjs`
- Create: `e2e/persona-harness/contracts.test.mjs`
- Create: `e2e/persona-harness/profiles/bryan.md`
- Create: `e2e/persona-harness/profiles/mira.md`
- Create: `e2e/persona-harness/profiles/arun.md`
- Create: `e2e/persona-harness/profiles/elena.md`
- Create: `e2e/persona-harness/profiles/theo.md`
- Create: `e2e/persona-harness/scenarios/core-ambiguous-idea.json`
- Create: `e2e/persona-harness/scenarios/bryan-compression.json`
- Create: `e2e/persona-harness/scenarios/mira-first-capture.json`
- Create: `e2e/persona-harness/scenarios/arun-source-inspection.json`
- Create: `e2e/persona-harness/scenarios/elena-decision.json`
- Create: `e2e/persona-harness/scenarios/theo-resume-large-session.json`

**Interfaces:**

- Produces: `loadProfile(filePath): Promise<PersonaProfile>` where `PersonaProfile` is `{ id, version, title, content, sections, sha256 }`.
- Produces: `loadScenario(filePath): Promise<PersonaScenario>` where `PersonaScenario` is `{ id, version, title, goal, fixture, requiredCapabilities, capabilityActions, successEvidence, blockerConditions, prohibitedData, sha256 }`.
- Produces: `loadPanel(rootDir): Promise<{ profiles, scenarios }>`.
- Consumes: Node built-ins `node:crypto`, `node:fs/promises`, and `node:path` only.

- [ ] **Step 1: Write failing contract tests**

```js
// e2e/persona-harness/contracts.test.mjs
import assert from 'node:assert/strict'
import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { test } from 'node:test'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { loadPanel, loadProfile, loadScenario } from './contracts.mjs'

const root = path.dirname(fileURLToPath(import.meta.url))

test('loads the five approved personas with stable hashes', async () => {
  const panel = await loadPanel(root)
  assert.deepEqual(panel.profiles.map(({ id }) => id), ['arun', 'bryan', 'elena', 'mira', 'theo'])
  for (const profile of panel.profiles) {
    assert.match(profile.sha256, /^[a-f0-9]{64}$/)
    assert.equal(profile.version, 1)
    assert.ok(profile.sections['Abandonment Conditions'])
    assert.ok(profile.sections['Useful Finished Result'])
  }
})

test('loads the shared mission without leaking feature locations', async () => {
  const scenario = await loadScenario(path.join(root, 'scenarios/core-ambiguous-idea.json'))
  assert.equal(scenario.id, 'core-ambiguous-idea')
  assert.equal(scenario.version, 1)
  assert.ok(scenario.requiredCapabilities.includes('capture'))
  assert.ok(scenario.requiredCapabilities.includes('resume'))
  assert.doesNotMatch(scenario.goal, /click|button|menu|selector/i)
  assert.deepEqual(scenario.prohibitedData, ['real-vault', 'twin', 'constitution', 'production-feedback'])
})

test('rejects an incomplete profile', async () => {
  const fixtureRoot = await mkdtemp(path.join(tmpdir(), 'grafyn-profile-'))
  const fixturePath = path.join(fixtureRoot, 'incomplete.md')
  await writeFile(fixturePath, '---\nid: incomplete\nversion: 1\ntitle: Incomplete\n---\n\n## Cognitive Pattern\nMissing the other required sections.\n')
  await assert.rejects(
    loadProfile(fixturePath),
    /missing required section: Abandonment Conditions/,
  )
})
```

- [ ] **Step 2: Run the tests and verify the contract module is missing**

Run: `node --test e2e/persona-harness/contracts.test.mjs`

Expected: FAIL with `ERR_MODULE_NOT_FOUND` for `contracts.mjs`.

- [ ] **Step 3: Implement strict Markdown and JSON loaders**

```js
// e2e/persona-harness/contracts.mjs
import { createHash } from 'node:crypto'
import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'

const PROFILE_SECTIONS = [
  'Cognitive Pattern',
  'Trigger',
  'Expected Mental Model',
  'Natural Strategy',
  'Useful Defaults',
  'Confusion Triggers',
  'Abandonment Conditions',
  'Useful Finished Result',
  'Prior Knowledge',
  'Evaluation Questions',
]

function hash(content) {
  return createHash('sha256').update(content, 'utf8').digest('hex')
}

function parseHeader(content) {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n/.exec(content)
  if (!match) throw new Error('profile must start with frontmatter')
  const values = Object.fromEntries(match[1].split(/\r?\n/).map(line => {
    const separator = line.indexOf(':')
    if (separator < 1) throw new Error('profile frontmatter is invalid')
    return [line.slice(0, separator).trim(), line.slice(separator + 1).trim()]
  }))
  if (!/^[a-z][a-z0-9-]*$/.test(values.id ?? '')) throw new Error('profile id is invalid')
  const version = Number(values.version)
  if (!Number.isSafeInteger(version) || version < 1) throw new Error('profile version is invalid')
  if (!values.title) throw new Error('profile title is required')
  return { id: values.id, version, title: values.title }
}

function parseSections(content) {
  const sections = {}
  for (const part of content.split(/^## /m).slice(1)) {
    const newline = part.indexOf('\n')
    if (newline > 0) sections[part.slice(0, newline).trim()] = part.slice(newline + 1).trim()
  }
  return sections
}

export async function loadProfile(filePath) {
  const content = await readFile(filePath, 'utf8')
  const header = parseHeader(content)
  const sections = parseSections(content)
  for (const name of PROFILE_SECTIONS) {
    if (!sections[name]) throw new Error(`missing required section: ${name}`)
  }
  return { ...header, content, sections, sha256: hash(content) }
}

export async function loadScenario(filePath) {
  const content = await readFile(filePath, 'utf8')
  const value = JSON.parse(content)
  const required = ['id', 'version', 'title', 'goal', 'fixture', 'requiredCapabilities', 'capabilityActions', 'successEvidence', 'blockerConditions', 'prohibitedData']
  for (const key of required) if (!(key in value)) throw new Error(`scenario missing required field: ${key}`)
  if (!/^[a-z][a-z0-9-]*$/.test(value.id)) throw new Error('scenario id is invalid')
  if (!Number.isSafeInteger(value.version) || value.version < 1) throw new Error('scenario version is invalid')
  for (const key of ['requiredCapabilities', 'successEvidence', 'blockerConditions', 'prohibitedData']) {
    if (!Array.isArray(value[key]) || value[key].some(item => typeof item !== 'string')) {
      throw new Error(`scenario ${key} must be a string array`)
    }
  }
  if (value.capabilityActions === null || typeof value.capabilityActions !== 'object' || Array.isArray(value.capabilityActions)) {
    throw new Error('scenario capabilityActions must be an object')
  }
  for (const capability of value.requiredCapabilities) {
    const actions = value.capabilityActions[capability]
    if (!Array.isArray(actions) || actions.length === 0 || actions.some(action => typeof action !== 'string')) {
      throw new Error(`scenario capabilityActions missing: ${capability}`)
    }
  }
  return { ...value, sha256: hash(content) }
}

export async function loadPanel(rootDir) {
  const profileDir = path.join(rootDir, 'profiles')
  const scenarioDir = path.join(rootDir, 'scenarios')
  const profileFiles = (await readdir(profileDir)).filter(name => name.endsWith('.md')).sort()
  const scenarioFiles = (await readdir(scenarioDir)).filter(name => name.endsWith('.json')).sort()
  return {
    profiles: await Promise.all(profileFiles.map(name => loadProfile(path.join(profileDir, name)))),
    scenarios: await Promise.all(scenarioFiles.map(name => loadScenario(path.join(scenarioDir, name)))),
  }
}
```

- [ ] **Step 4: Add the five exact profiles**

Use the ten required headings from `PROFILE_SECTIONS`. Encode these approved distinctions without adding demographic claims:

| ID | Cognitive pattern | Trigger | Abandonment condition | Useful finished result |
|---|---|---|---|---|
| `bryan` | Non-linear systems thinker who externalizes cognition and keeps possibilities open | Ambiguous product, architecture, or research problem | Canvas grows without exposing structure, decision, or next action | Preserved branches compressed into a manipulable model, decision, and next action |
| `mira` | Unfamiliar knowledge worker who expects understandable defaults | A half-formed idea that must be captured quickly | Product requires unexplained configuration or hidden prerequisites before useful capture | Idea captured and developed without learning internal product vocabulary |
| `arun` | Evidence-led researcher who distinguishes source from inference | A question whose answer depends on notes and provenance | Claims cannot be traced to inspectable context | Synthesis with visible sources, uncertainty, and reproducible context |
| `elena` | Outcome-driven product lead who uses exploration to commit | A decision with competing options and constraints | Exploration cannot become priorities, ownership, or action | Decision record with rationale, unresolved risks, and next actions |
| `theo` | Returning high-volume user who values control and recovery | An old, branching session that must be resumed | Finding prior state costs as much as recreating it | Correct branch resumed quickly with reusable structure and no lost work |

Every profile must also say: genuinely attempt the task; do not invent confusion; use only visible UI information; never infer source code or selectors; distinguish observation from interpretation.

- [ ] **Step 5: Add the shared and targeted scenarios**

Set the shared scenario goal exactly to:

```json
{
  "id": "core-ambiguous-idea",
  "version": 1,
  "title": "Explore, compress, save, and resume an ambiguous idea",
  "goal": "Explore whether Grafyn should make its compiled session memory visible and editable. Obtain several perspectives, follow one promising branch, compare conflicting responses, produce a useful synthesis or decision, save the work, leave the Canvas, and later resume it.",
  "fixture": "canvas-core-v1",
  "requiredCapabilities": ["capture", "compare", "branch", "synthesize", "decide", "save", "leave", "resume"],
  "capabilityActions": {
    "capture": ["prompt-submit"],
    "compare": ["response-select", "canvas-start-debate"],
    "branch": ["response-branch"],
    "synthesize": ["debate-reply", "prompt-submit"],
    "decide": ["prompt-mode-decision", "prompt-submit"],
    "save": ["canvas-save-note"],
    "leave": ["canvas-back-to-notes"],
    "resume": ["canvas-open-session"]
  },
  "successEvidence": ["session-created", "multiple-responses-visible", "branch-created", "conflict-compared", "synthesis-or-decision-visible", "work-saved", "same-session-resumed"],
  "blockerConditions": ["required-action-undiscovered", "destructive-loss", "model-fixture-unavailable", "computer-use-unavailable"],
  "prohibitedData": ["real-vault", "twin", "constitution", "production-feedback"]
}
```

Create the five targeted scenarios with these exact capability-to-action mappings. Some expected action IDs intentionally do not exist in the baseline UI; their absence is measured as `undiscovered` rather than patched during harness construction.

| Scenario | Capability mappings |
|---|---|
| `bryan-compression` | `capture -> prompt-submit`; `branch -> response-branch`; `open-questions -> canvas-session-memory`; `constraints -> canvas-session-memory`; `synthesize -> debate-reply,prompt-submit`; `next-action -> canvas-next-action` |
| `mira-first-capture` | `new-session -> canvas-new-session`; `capture -> canvas-new-prompt,prompt-submit`; `default-model -> prompt-submit`; `submit -> prompt-submit`; `understand-result -> response-content` |
| `arun-source-inspection` | `knowledge-context -> prompt-context-mode`; `inspect-source -> context-source-chip`; `identify-used-context -> context-source-chip`; `separate-claim-from-source -> response-content,context-source-chip` |
| `elena-decision` | `decision-prompt -> prompt-mode-decision`; `compare -> response-select,canvas-start-debate`; `record-rationale -> prompt-decision-rationale`; `prioritize -> debate-reply`; `next-action -> canvas-next-action` |
| `theo-resume-large-session` | `find-session -> canvas-open-session`; `navigate-large-canvas -> canvas-arrange,canvas-minimap-node,canvas-reset-view`; `find-unresolved-branch -> canvas-session-memory`; `continue -> response-follow-up`; `preserve-layout -> canvas-open-session` |

Each targeted goal must describe the outcome without naming controls or locations.

- [ ] **Step 6: Run contract tests**

Run: `node --test e2e/persona-harness/contracts.test.mjs`

Expected: PASS, 3 tests.

- [ ] **Step 7: Commit the contracts**

```powershell
git add e2e/persona-harness/contracts.mjs e2e/persona-harness/contracts.test.mjs e2e/persona-harness/profiles e2e/persona-harness/scenarios
git commit -m "test: define canvas usability personas and scenarios"
```

---

### Task 2: Dev-Only Interaction Recorder

**Files:**

- Create: `frontend/src/personaHarness/config.js`
- Create: `frontend/src/personaHarness/uiState.js`
- Create: `frontend/src/personaHarness/recorder.js`
- Create: `frontend/src/__tests__/unit/personaHarness/config.spec.js`
- Create: `frontend/src/__tests__/unit/personaHarness/uiState.spec.js`
- Create: `frontend/src/__tests__/unit/personaHarness/recorder.spec.js`
- Modify: `frontend/src/main.js`

**Interfaces:**

- Produces: `resolvePersonaHarnessConfig(env): null | { endpoint, token, runId }`.
- Produces: `resolveUiState(documentRef, locationRef): string`.
- Produces: `describeTarget(element, documentRef): { actionId, label, visible, bounds, menuDepth }`.
- Produces: `readCanvasTransform(documentRef): null | { x, y, zoom }`.
- Produces: `installPersonaRecorder({ windowRef, documentRef, fetchImpl, config }): { flush, stop }`.
- Consumes: the loopback `POST /events` endpoint from Task 4.

- [ ] **Step 1: Write failing configuration tests**

```js
import { describe, expect, it } from 'vitest'
import { resolvePersonaHarnessConfig } from '@/personaHarness/config'

describe('resolvePersonaHarnessConfig', () => {
  it('stays disabled without all three explicit values', () => {
    expect(resolvePersonaHarnessConfig({ DEV: true })).toBeNull()
  })

  it('accepts only an exact loopback endpoint and bounded identifiers', () => {
    const config = resolvePersonaHarnessConfig({
      DEV: true,
      VITE_GRAFYN_PERSONA_RECORDER_URL: 'http://127.0.0.1:18892',
      VITE_GRAFYN_PERSONA_RECORDER_TOKEN: 'a'.repeat(64),
      VITE_GRAFYN_PERSONA_RUN_ID: 'bryan-core-001',
    })
    expect(config).toEqual({
      endpoint: 'http://127.0.0.1:18892',
      token: 'a'.repeat(64),
      runId: 'bryan-core-001',
    })
  })

  it('rejects production and non-loopback configuration', () => {
    expect(() => resolvePersonaHarnessConfig({
      DEV: false,
      VITE_GRAFYN_PERSONA_RECORDER_URL: 'http://127.0.0.1:18892',
      VITE_GRAFYN_PERSONA_RECORDER_TOKEN: 'a'.repeat(64),
      VITE_GRAFYN_PERSONA_RUN_ID: 'run-1',
    })).toThrow(/development builds/)
    expect(() => resolvePersonaHarnessConfig({
      DEV: true,
      VITE_GRAFYN_PERSONA_RECORDER_URL: 'https://example.com',
      VITE_GRAFYN_PERSONA_RECORDER_TOKEN: 'a'.repeat(64),
      VITE_GRAFYN_PERSONA_RUN_ID: 'run-1',
    })).toThrow(/loopback/)
  })
})
```

- [ ] **Step 2: Run the configuration test and verify failure**

Run: `cd frontend; npx vitest run src/__tests__/unit/personaHarness/config.spec.js`

Expected: FAIL because `@/personaHarness/config` does not exist.

- [ ] **Step 3: Implement strict dev-only configuration**

```js
const TOKEN = /^[a-f0-9]{64}$/
const RUN_ID = /^[a-z0-9][a-z0-9-]{0,79}$/
const LOOPBACK = /^http:\/\/127\.0\.0\.1:([1-9][0-9]{0,4})$/

export function resolvePersonaHarnessConfig(env = {}) {
  const values = [
    env.VITE_GRAFYN_PERSONA_RECORDER_URL,
    env.VITE_GRAFYN_PERSONA_RECORDER_TOKEN,
    env.VITE_GRAFYN_PERSONA_RUN_ID,
  ]
  if (values.every(value => value === undefined || value === '')) return null
  if (!env.DEV) throw new Error('Persona harness is available only in development builds')
  const [endpoint, token, runId] = values
  const match = typeof endpoint === 'string' ? LOOPBACK.exec(endpoint) : null
  if (!match || Number(match[1]) > 65535) throw new Error('Persona recorder must use exact loopback HTTP')
  if (!TOKEN.test(token ?? '')) throw new Error('Persona recorder token is invalid')
  if (!RUN_ID.test(runId ?? '')) throw new Error('Persona run id is invalid')
  return Object.freeze({ endpoint, token, runId })
}
```

- [ ] **Step 4: Write failing UI-state and target-description tests**

Test these exact cases:

```js
expect(resolveUiState(document, { pathname: '/canvas/session-1' })).toBe('populated-canvas')
document.body.innerHTML = '<div data-persona-state="prompt-dialog"></div>'
expect(resolveUiState(document, { pathname: '/canvas/session-1' })).toBe('prompt-dialog')

document.body.innerHTML = '<div class="dropdown-menu"><button data-persona-action="canvas-twin-capture" aria-label="Capture Insight"></button></div>'
const target = describeTarget(document.querySelector('button'), document)
expect(target).toMatchObject({ actionId: 'canvas-twin-capture', label: 'Capture Insight', menuDepth: 1 })
expect(target.bounds).toEqual(expect.objectContaining({ x: expect.any(Number), width: expect.any(Number) }))
```

- [ ] **Step 5: Implement semantic state and target resolution**

Use the nearest `[data-persona-action]`; fall back to the nearest button, link, input, select, textarea, or `[role]`. Derive labels in this order: `data-persona-label`, `aria-label`, associated `<label>`, `title`, then trimmed visible text capped at 120 characters. Determine visibility from `getBoundingClientRect()`, `hidden`, `aria-hidden`, and computed `display`, `visibility`, and `opacity`. Count visible ancestors matching `[data-persona-layer]`, `.dialog-overlay`, or `.dropdown-menu` for `menuDepth`.

Resolve stable state in this order: the topmost visible `[data-persona-state]`; `/canvas` with no session as `empty-canvas`; `/canvas/:id` with no prompt tiles as `empty-session`; `/canvas/:id` with tiles as `populated-canvas`; other routes as `outside-canvas`.

- [ ] **Step 6: Write failing recorder tests**

```js
it('records clicks without prompt text and flushes bounded events', async () => {
  document.body.innerHTML = '<button data-persona-action="canvas-new-prompt">New Prompt</button><textarea>private words</textarea>'
  const sent = []
  const recorder = installPersonaRecorder({
    windowRef: window,
    documentRef: document,
    config: { endpoint: 'http://127.0.0.1:18892', token: 'a'.repeat(64), runId: 'run-1' },
    fetchImpl: async (_url, request) => {
      sent.push(JSON.parse(request.body))
      return { ok: true }
    },
  })
  document.querySelector('button').click()
  await recorder.flush()
  expect(sent[0].events[0]).toMatchObject({ actionId: 'canvas-new-prompt', kind: 'click' })
  expect(JSON.stringify(sent)).not.toContain('private words')
  recorder.stop()
})
```

- [ ] **Step 7: Implement event capture and batching**

Capture `pointerdown`, `click`, `focusin`, and only the navigation keys `Enter`, `Escape`, `Tab`, and arrow keys. Never record printable key values or input values. Emit an initial `state` observation at installation, then use a debounced `MutationObserver` to emit another `state` observation only when `resolveUiState()` changes. A state observation includes the sorted IDs and bounds of currently visible `[data-persona-action]` elements, but no visible text. After every click, emit an `action-outcome` observation after two animation frames with `causedBySequence`, the resulting UI state, and whether the DOM or route changed; this records visible effect without copying content. Disconnect the observer in `stop()`. Each interaction event contains:

```js
{
  schemaVersion: 1,
  runId: config.runId,
  sequence,
  occurredAt: new Date().toISOString(),
  monotonicMs: windowRef.performance.now(),
  kind,
  uiState: resolveUiState(documentRef, windowRef.location),
  route: windowRef.location.pathname,
  viewport: { width: windowRef.innerWidth, height: windowRef.innerHeight },
  canvas: readCanvasTransform(documentRef),
  point: event.clientX === undefined ? null : {
    x: event.clientX,
    y: event.clientY,
    normalizedX: event.clientX / windowRef.innerWidth,
    normalizedY: event.clientY / windowRef.innerHeight,
  },
  target: describeTarget(event.target, documentRef),
}
```

Flush at 20 events, after 2 seconds, on `visibilitychange`, and from the returned `flush()` method. POST JSON with `Authorization: Bearer <token>`, `Content-Type: application/json`, `credentials: 'omit'`, and `redirect: 'error'`.

- [ ] **Step 8: Install the recorder before Vue mount**

In `frontend/src/main.js`, add a compile-time development branch with dynamic imports so the recorder module is absent from release bundles:

```js
async function installPersonaHarness() {
  if (!import.meta.env.DEV) return null
  const [{ resolvePersonaHarnessConfig }, { installPersonaRecorder }] = await Promise.all([
    import('./personaHarness/config'),
    import('./personaHarness/recorder'),
  ])
  const config = resolvePersonaHarnessConfig(import.meta.env)
  if (!config) return null
  const recorder = installPersonaRecorder({ config })
  window.addEventListener('beforeunload', () => recorder.flush(), { once: true })
  return recorder
}
```

Call `await installPersonaHarness()` as the first line of the existing `bootstrap()` function. Do not expose the token or recorder object on `window`.

- [ ] **Step 9: Run focused frontend tests**

Run: `cd frontend; npx vitest run src/__tests__/unit/personaHarness`

Expected: PASS.

- [ ] **Step 10: Verify the ordinary production bundle has no active endpoint**

Run: `cd frontend; npm run build`

Expected: PASS with no persona environment variables. Inspect `dist/` with `rg "VITE_GRAFYN_PERSONA|18892|installPersonaRecorder" dist`; expected: no persona configuration or recorder implementation in the release bundle.

- [ ] **Step 11: Commit the recorder**

```powershell
git add frontend/src/personaHarness frontend/src/__tests__/unit/personaHarness frontend/src/main.js
git commit -m "test: add development-only canvas interaction recorder"
```

---

### Task 3: Stable Canvas Action and State Identifiers

**Files:**

- Modify: `frontend/src/views/CanvasView.vue`
- Modify: `frontend/src/components/canvas/CanvasContainer.vue`
- Modify: `frontend/src/components/canvas/PromptDialog.vue`
- Modify: `frontend/src/components/canvas/LLMNode.vue`
- Modify: `frontend/src/components/canvas/DebateNode.vue`
- Modify: `frontend/src/__tests__/unit/views/CanvasView.spec.js`
- Modify: `frontend/src/__tests__/unit/components/CanvasContainer.spec.js`
- Modify: `frontend/src/__tests__/unit/components/PromptDialog.spec.js`
- Modify: `frontend/src/__tests__/unit/components/CanvasNodes.spec.js`

**Interfaces:**

- Produces: stable `data-persona-action` and `data-persona-state` attributes consumed by `describeTarget()` and `resolveUiState()`.
- Consumes: no runtime JavaScript API and changes no click behavior.

- [ ] **Step 1: Write failing selector-contract tests**

Add assertions for these identifiers:

```js
expect(wrapper.find('[data-persona-action="canvas-new-session"]').exists()).toBe(true)
expect(wrapper.find('[data-persona-action="canvas-new-prompt"]').exists()).toBe(true)
expect(wrapper.find('[data-persona-action="prompt-submit"]').exists()).toBe(true)
expect(wrapper.find('[data-persona-action="response-branch"]').exists()).toBe(true)
expect(wrapper.find('[data-persona-action="response-follow-up"]').exists()).toBe(true)
expect(wrapper.find('[data-persona-action="response-select"]').exists()).toBe(true)
expect(wrapper.find('[data-persona-action="debate-reply"]').exists()).toBe(true)
```

- [ ] **Step 2: Run focused tests and verify missing-attribute failures**

Run: `cd frontend; npx vitest run src/__tests__/unit/views/CanvasView.spec.js src/__tests__/unit/components/CanvasContainer.spec.js src/__tests__/unit/components/PromptDialog.spec.js src/__tests__/unit/components/CanvasNodes.spec.js`

Expected: FAIL on the new attribute assertions.

- [ ] **Step 3: Add Canvas-view identifiers**

Add `data-persona-state="canvas-view"` to the root. Add action IDs:

- `canvas-new-session`
- `canvas-open-session`
- `canvas-delete-session`
- `canvas-back-to-notes`

Use dynamic `data-persona-label` for session rows, but never include note contents.

- [ ] **Step 4: Add Canvas-container identifiers**

Add action IDs:

- `canvas-arrange`
- `canvas-open-pinned-notes`
- `canvas-open-twin-actions`
- `canvas-save-note`
- `canvas-reset-view`
- `canvas-zoom-in`
- `canvas-zoom-out`
- `canvas-new-prompt`
- `canvas-start-debate`
- `canvas-clear-selection`
- `canvas-twin-matches-me`
- `canvas-twin-not-me`
- `canvas-twin-correct`
- `canvas-twin-rank`
- `canvas-twin-capture`
- `canvas-twin-export`

Mark modal and dropdown roots with `data-persona-layer`. Do not move, rename, or restyle controls in this task.

- [ ] **Step 5: Add prompt-dialog identifiers**

Mark the dialog root `data-persona-state="prompt-dialog"` and `data-persona-layer="modal"`. Add:

- `prompt-mode-standard`
- `prompt-mode-decision`
- `prompt-text`
- `prompt-model-select`
- `prompt-context-mode`
- `prompt-twin-mode`
- `prompt-web-search`
- `prompt-advanced-settings`
- `prompt-cancel`
- `prompt-submit`

- [ ] **Step 6: Add response and debate identifiers**

Add to `LLMNode.vue`:

- `response-select`
- `response-branch`
- `response-follow-up`
- `response-think-harder`
- `response-regenerate`
- `response-delete`
- `response-decision-feedback`
- `response-content`

Add to `DebateNode.vue`:

- `debate-expand`
- `debate-collapse`
- `debate-reply`
- `debate-delete`

Also identify existing non-button surfaces that a persona may reasonably try to use:

- `canvas-session-memory`
- `canvas-minimap-node`
- `context-source-chip`
- `prompt-decision-rationale`

Do not add `canvas-next-action`; its deliberate absence lets the baseline measure whether the requested next-action surface exists.

- [ ] **Step 7: Run the selector-contract tests**

Run: `cd frontend; npx vitest run src/__tests__/unit/views/CanvasView.spec.js src/__tests__/unit/components/CanvasContainer.spec.js src/__tests__/unit/components/PromptDialog.spec.js src/__tests__/unit/components/CanvasNodes.spec.js`

Expected: PASS with existing behavior assertions unchanged.

- [ ] **Step 8: Commit semantic identifiers**

```powershell
git add frontend/src/views/CanvasView.vue frontend/src/components/canvas frontend/src/__tests__/unit/views/CanvasView.spec.js frontend/src/__tests__/unit/components
git commit -m "test: identify canvas actions for persona evidence"
```

---

### Task 4: Bounded Loopback Recorder and Immutable Run Manifest

**Files:**

- Create: `e2e/persona-harness/recorder-server.mjs`
- Create: `e2e/persona-harness/recorder-server.test.mjs`

**Interfaces:**

- Produces: `startRecorderServer({ artifactRoot, run, token, port }): Promise<{ url, runDir, close, finalize }>`.
- HTTP: `GET /health`, `POST /events`, `POST /finalize`.
- Writes: `manifest.json`, `actions.jsonl`, `observations.jsonl`, and `integrity.json` beneath an owned run directory.
- Consumes: event batches from Task 2 and profile/scenario hashes from Task 1.

- [ ] **Step 1: Write failing server tests**

```js
function validClickEvent() {
  return {
    schemaVersion: 1,
    runId: 'bryan-core-001',
    sequence: 1,
    occurredAt: '2026-09-12T00:00:00.000Z',
    monotonicMs: 1,
    kind: 'click',
    uiState: 'empty-session',
    route: '/canvas/session-1',
    viewport: { width: 1440, height: 900 },
    canvas: { x: 0, y: 0, zoom: 1 },
    point: { x: 720, y: 450, normalizedX: 0.5, normalizedY: 0.5 },
    target: { actionId: 'canvas-new-prompt', label: 'New Prompt', visible: true, bounds: { x: 680, y: 420, width: 120, height: 44 }, menuDepth: 0 },
  }
}

test('accepts bounded authenticated events and rejects other callers', async t => {
  const root = await mkdtemp(path.join(tmpdir(), 'grafyn-persona-'))
  const token = 'b'.repeat(64)
  const server = await startRecorderServer({
    artifactRoot: root,
    port: 0,
    token,
    run: {
      runId: 'bryan-core-001',
      persona: { id: 'bryan', version: 1, sha256: 'c'.repeat(64) },
      scenario: { id: 'core-ambiguous-idea', version: 1, sha256: 'd'.repeat(64) },
      app: { gitSha: '0123456789abcdef0123456789abcdef01234567' },
      viewport: { width: 1440, height: 900 },
    },
  })
  t.after(() => server.close())

  const denied = await fetch(`${server.url}/events`, { method: 'POST', body: '{}' })
  assert.equal(denied.status, 401)

  const accepted = await fetch(`${server.url}/events`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ events: [validClickEvent()] }),
  })
  assert.equal(accepted.status, 204)
  assert.equal((await readFile(path.join(server.runDir, 'actions.jsonl'), 'utf8')).trim().split('\n').length, 1)
})
```

Also test: host is always `127.0.0.1`; body limit is 256 KiB; maximum batch is 100 events; run IDs cannot escape `artifactRoot`; printable keys and any `value`, `text`, `prompt`, or `content` field are rejected; sequence numbers must be strictly increasing; finalization prevents further writes.

- [ ] **Step 2: Run server tests and verify failure**

Run: `node --test e2e/persona-harness/recorder-server.test.mjs`

Expected: FAIL because the server module does not exist.

- [ ] **Step 3: Implement owned run-directory creation**

Resolve `artifactRoot`, create it, resolve `runDir = path.join(artifactRoot, run.runId)`, and reject the run unless `path.relative(artifactRoot, runDir)` is a single non-parent segment. Use `open(..., 'wx')` for `manifest.json`; an existing run ID is an error. Write the manifest with `status: "recording"`, `schemaVersion: 1`, creation time, app/profile/scenario identity, viewport, and `simulatedPersona: true`.

- [ ] **Step 4: Implement strict endpoints and append-only events**

Bind only to `127.0.0.1`. Require the exact bearer token on `/events` and `/finalize`. Validate exact event keys and field bounds before appending one JSON object per line. Write `kind: "state"` entries to `observations.jsonl` and all interaction entries to `actions.jsonl`. Do not echo event bodies in errors. `/finalize` closes both append handles, computes SHA-256 for `manifest.json`, `actions.jsonl`, and `observations.jsonl`, writes `integrity.json`, and atomically replaces the manifest with `status: "recorded"`.

- [ ] **Step 5: Verify the existing artifact ignore boundary**

Run:

```powershell
git check-ignore e2e/test-results/persona-harness/example/manifest.json
```

Expected: the path is ignored by the existing `e2e/test-results/` rule. If it is not ignored, stop because the target branch no longer matches this plan.

- [ ] **Step 6: Run server tests**

Run: `node --test e2e/persona-harness/recorder-server.test.mjs`

Expected: PASS.

- [ ] **Step 7: Commit the collector**

```powershell
git add e2e/persona-harness/recorder-server.mjs e2e/persona-harness/recorder-server.test.mjs
git commit -m "test: collect bounded persona interaction evidence"
```

---

### Task 5: Deterministic Metrics, Heat Maps, and Report Scaffold

**Files:**

- Create: `e2e/persona-harness/analyze-run.mjs`
- Create: `e2e/persona-harness/analyze-run.test.mjs`

**Interfaces:**

- Produces: `analyzeEvents({ manifest, actions, observations, scenario }): PersonaRunAnalysis`.
- Produces: `analyzeRun(runDir, { scenario }): Promise<PersonaRunAnalysis>`.
- Produces: `classifyDepth(actionCount): 'direct' | 'shallow' | 'buried'`.
- Produces: `renderHeatmapSvg({ state, events, width, height }): string`.
- Writes: `metrics.json`, `heatmaps/<state>.svg`, and `persona-report.md`.
- Consumes: finalized artifacts from Task 4 and scenario requirements from Task 1.

- [ ] **Step 1: Write failing metric tests**

```js
test('classifies interaction depth at the approved boundaries', () => {
  assert.equal(classifyDepth(1), 'direct')
  assert.equal(classifyDepth(2), 'shallow')
  assert.equal(classifyDepth(3), 'buried')
})

test('keeps blocking failures separate from aggregate counts', async () => {
  const analysis = analyzeEvents({
    manifest: { runId: 'run-1', status: 'recorded' },
    actions: [{ sequence: 1, kind: 'click', uiState: 'empty-session', target: { actionId: 'canvas-new-prompt' } }],
    observations: [{ sequence: 2, kind: 'state', uiState: 'prompt-dialog' }],
    scenario: {
      requiredCapabilities: ['save'],
      capabilityActions: { save: ['canvas-save-note'] },
    },
  })
  assert.equal(analysis.capabilities.save.depth, 'undiscovered')
  assert.equal(analysis.simplicityGate, 'fail')
  assert.deepEqual(analysis.blockers, ['required-action-undiscovered:save'])
})

test('renders state-specific normalized click points', () => {
  const svg = renderHeatmapSvg({
    state: 'prompt-dialog',
    width: 1000,
    height: 625,
    events: [{ point: { normalizedX: 0.5, normalizedY: 0.25 }, target: { actionId: 'prompt-submit' } }],
  })
  assert.match(svg, /cx="500"/)
  assert.match(svg, /cy="156\.25"/)
  assert.match(svg, /data-action="prompt-submit"/)
})
```

- [ ] **Step 2: Run analyzer tests and verify failure**

Run: `node --test e2e/persona-harness/analyze-run.test.mjs`

Expected: FAIL because `analyze-run.mjs` does not exist.

- [ ] **Step 3: Implement deterministic event metrics**

Compute, without an LLM:

```js
{
  schemaVersion: 1,
  runId,
  completion: 'completed' | 'blocked' | 'abandoned' | 'invalid',
  simplicityGate: 'pass' | 'fail' | 'invalid',
  eventCounts: { total, clicks, focusChanges, navigationKeys },
  uiStates: { [state]: { entered, clicks, uniqueActions, backtracks, deadEnds } },
  actions: { [actionId]: { count, firstSequence, states } },
  capabilities: { [capability]: { depth, evidenceSequences } },
  blockers: [],
  evidenceCompleteness: { manifest, actions, observations, finalized, integrity },
}
```

Capability evidence comes from `scenario.capabilityActions`, not text similarity. For the first successful mapped action, interaction depth is `target.menuDepth + 1`; use the minimum successful depth when multiple mapped actions exist. A required capability with no mapped successful action is `undiscovered`. A required capability at depth `buried` or `undiscovered` sets `simplicityGate: "fail"`. A repeated state path such as `A -> B -> A` is a deterministic backtrack. An action with no route, state, or DOM change is reported as `no-visible-effect`, not automatically called a dead end; the persona interpretation may classify it as a dead end with the cited observation.

- [ ] **Step 4: Implement state-specific SVG heat maps**

Render a fixed 1000 x 625 SVG for each state. Use normalized points, a translucent 44-pixel radial gradient, and a small semantic action label. Include state, run ID, click count, viewport source, and the warning `Interaction evidence; not eye tracking` in the SVG. Never combine points from different states.

- [ ] **Step 5: Implement the evidence-linked report scaffold**

Generate these exact headings:

```markdown
# Persona Usability Report: <persona> / <scenario>

## Run Validity
## Outcome
## Simplicity Gate
## Observed Evidence
## Persona Interpretation
## Recommended Changes
## Validation Contract
## Limits
```

Populate Run Validity, Outcome, Simplicity Gate, Observed Evidence, and Limits deterministically. Leave the interpretation and recommendation sections with the explicit final sentence `No persona interpretation has been recorded.` Each observed line cites event sequences such as `[actions:12,18]` and a heat-map path.

- [ ] **Step 6: Run analyzer tests**

Run: `node --test e2e/persona-harness/analyze-run.test.mjs`

Expected: PASS.

- [ ] **Step 7: Commit analysis**

```powershell
git add e2e/persona-harness/analyze-run.mjs e2e/persona-harness/analyze-run.test.mjs
git commit -m "test: render persona usability metrics and heat maps"
```

---

### Task 6: Persona Runtime Stack and Computer Use Protocol

**Files:**

- Create: `e2e/persona-harness/start-run.mjs`
- Create: `e2e/persona-harness/start-run.test.mjs`
- Create: `e2e/persona-harness/README.md`
- Modify: `e2e/package.json`

**Interfaces:**

- Produces CLI: `npm run persona:start -- --persona <id> --scenario <id> --run-id <id>`.
- Produces CLI: `npm run persona:analyze -- --run-dir <absolute-run-dir>`.
- Produces stdout handoff JSON: `{ runId, canvasUrl, recorderUrl, runDir, personaPath, scenarioPath }`.
- Produces: `resolveRunRequest(argv): Promise<ResolvedRunRequest>` and `publicHandoff(run): PublicRunHandoff` for argument and secret-redaction tests.
- Consumes: `startOpenRouterStub()` from `e2e/fixtures/openrouter-stub.js`, `grafyn-test-runtime`, Vite, Task 4 collector, and Task 1 contracts.

- [ ] **Step 1: Write failing argument and capability tests**

```js
test('requires known persona scenario and bounded run id', async () => {
  await assert.rejects(
    resolveRunRequest(['--persona', 'unknown', '--scenario', 'core-ambiguous-idea', '--run-id', '../bad']),
    /unknown persona|run id/,
  )
})

test('emits a Computer Use handoff without secrets', async () => {
  const handoff = publicHandoff({
    runId: 'bryan-core-001',
    canvasUrl: 'http://127.0.0.1:5173/canvas',
    recorderUrl: 'http://127.0.0.1:18892',
    token: 'a'.repeat(64),
    runDir: 'C:\\tmp\\run',
    personaPath: 'profiles/bryan.md',
    scenarioPath: 'scenarios/core-ambiguous-idea.json',
  })
  assert.equal(handoff.token, undefined)
  assert.equal(handoff.canvasUrl, 'http://127.0.0.1:5173/canvas')
})
```

- [ ] **Step 2: Run launcher tests and verify failure**

Run: `node --test e2e/persona-harness/start-run.test.mjs`

Expected: FAIL because `start-run.mjs` does not exist.

- [ ] **Step 3: Implement strict request resolution**

Load the panel from Task 1. Accept only exact known IDs and a run ID matching `/^[a-z0-9][a-z0-9-]{0,79}$/`. Resolve Git SHA with `git rev-parse HEAD` and require 40 lowercase hex characters. Generate independent random 256-bit tokens for the Rust runtime and recorder.

- [ ] **Step 4: Start the four local processes with owned roots**

Start in this order:

1. Task 4 recorder on `127.0.0.1:18892`.
2. Existing OpenRouter stub on `127.0.0.1:18891`.
3. Existing `grafyn-test-runtime` on `127.0.0.1:18890` with a new empty owned root under `e2e/test-results/persona-harness/<run-id>/runtime`.
4. Vite on `127.0.0.1:5173` with both E2E transport and persona-recorder environment variables.

Pass these exact frontend variables:

```js
{
  VITE_GRAFYN_E2E_RUNTIME_URL: 'http://127.0.0.1:18890',
  VITE_GRAFYN_E2E_RUNTIME_TOKEN: runtimeToken,
  VITE_GRAFYN_PERSONA_RECORDER_URL: 'http://127.0.0.1:18892',
  VITE_GRAFYN_PERSONA_RECORDER_TOKEN: recorderToken,
  VITE_GRAFYN_PERSONA_RUN_ID: request.runId,
}
```

Use `spawn()` argument arrays with `shell: false`; on Windows resolve `npm.cmd` and `cargo.exe` explicitly through `where.exe` once. Never build command strings. Poll only each loopback `/health` endpoint with a 60-second bound for Node/Vite and 300-second bound for Rust. On failure, terminate only child processes started by this run and finalize the manifest as invalid.

- [ ] **Step 5: Implement clean shutdown and finalization**

Handle `SIGINT` and `SIGTERM` once. Ask the collector to finalize, terminate owned child processes, await exit, then run `analyzeRun()`. Do not recursively delete the run directory. Preserve invalid and failed-run evidence.

- [ ] **Step 6: Add package scripts**

```json
{
  "persona:test": "node --test persona-harness/*.test.mjs",
  "persona:start": "node persona-harness/start-run.mjs",
  "persona:analyze": "node persona-harness/analyze-run.mjs"
}
```

Do not regenerate `e2e/package-lock.json`; script-only changes do not alter the dependency lock.

- [ ] **Step 7: Write the Computer Use operator contract**

The README must instruct the orchestrator to:

1. start one run and wait for healthy handoff JSON;
2. load exactly one profile and one scenario;
3. create a fresh persona subagent with no prior run history;
4. give it only the profile, goal, permitted data, Canvas URL, and Computer Use safety contract;
5. have it target exactly one visible Grafyn browser tab and stop if selection is ambiguous;
6. re-observe after every state-changing action;
7. never provide selectors, feature locations, or rescue hints;
8. record `completed`, `blocked`, or `abandoned` and the evidence sequence;
9. stop and finalize the run;
10. run personas serially and start a fresh owned runtime for the next persona.

State explicitly that Computer Use screenshots remain observation receipts in the Codex task history unless an approved export mechanism provides local files. Canonical Playwright screenshots may illustrate stable states but must not be mislabelled as exact persona screenshots.

- [ ] **Step 8: Run launcher and harness tests**

Run: `cd e2e; npm run persona:test`

Expected: PASS for contract, collector, analyzer, and launcher tests.

- [ ] **Step 9: Commit runtime orchestration**

```powershell
git add e2e/persona-harness e2e/package.json
git commit -m "test: orchestrate computer-use persona runs"
```

---

### Task 7: Deterministic Canvas Fixture and Regression Journey

**Files:**

- Modify: `e2e/fixtures/openrouter-stub.js`
- Modify: `e2e/fixtures/openrouter-stub.test.js`
- Create: `e2e/tests/persona-harness.spec.js`
- Modify: `e2e/playwright.config.js`

**Interfaces:**

- Produces: deterministic fixture ID `canvas-core-v1` with stable multi-model responses and a resumable session.
- Produces: Playwright project `persona-regression` at 1440 x 900.
- Consumes: the same Rust runtime and frontend recorder used by Computer Use runs.

- [ ] **Step 1: Write failing OpenRouter fixture tests**

Add a test that submits the core question to three model IDs and asserts these stable response markers:

```js
assert.match(responses['grafyn/e2e-diverge'], /keep the compiled memory invisible/i)
assert.match(responses['grafyn/e2e-expose'], /editable session compass/i)
assert.match(responses['grafyn/e2e-simplify'], /reduce setup before capture/i)
```

Also assert the fixture returns streaming usage and never contacts a non-loopback host.

- [ ] **Step 2: Run the fixture test and verify failure**

Run: `cd e2e; npm run test:fixtures`

Expected: FAIL because the three Canvas model fixture IDs are absent.

- [ ] **Step 3: Implement deterministic Canvas model responses**

Extend `/api/v1/models` with exactly:

- `grafyn/e2e-diverge`
- `grafyn/e2e-expose`
- `grafyn/e2e-simplify`

Return fixed streamed text keyed by `body.model`. The responses must disagree meaningfully, include no source-code knowledge, and remain under 600 words combined. Preserve existing image and Twin fixture behavior.

- [ ] **Step 4: Add the persona regression project**

Add:

```js
{
  name: 'persona-regression',
  testMatch: 'persona-harness.spec.js',
  use: {
    ...devices['Desktop Chrome'],
    viewport: { width: 1440, height: 900 },
    trace: 'on',
    screenshot: 'on',
  },
}
```

Keep `fullyParallel: false` and `workers: 1`.

- [ ] **Step 5: Write the deterministic core journey**

The Playwright test must:

1. open `/canvas` and wait for boot;
2. create a fresh session;
3. open New Prompt;
4. select all three fixture models;
5. submit the approved shared question;
6. wait for three completed response nodes;
7. branch from `grafyn/e2e-expose`;
8. select two conflicting responses and start debate;
9. save as note or record why the current UI cannot preserve the required structure;
10. navigate away, return, and reopen the same session;
11. assert the semantic action identifiers used by the persona recorder;
12. capture named screenshots for every stable UI state.

Use role/name or `data-persona-action` selectors only. Do not use coordinate clicks in Playwright.

- [ ] **Step 6: Run the persona regression journey**

Run: `cd e2e; npx playwright test --project=persona-regression`

Expected: PASS for mechanics. Any product limitation is asserted as an observed baseline state, not worked around or called a usability pass.

- [ ] **Step 7: Commit deterministic regression coverage**

```powershell
git add e2e/fixtures/openrouter-stub.js e2e/fixtures/openrouter-stub.test.js e2e/tests/persona-harness.spec.js e2e/playwright.config.js
git commit -m "test: cover deterministic canvas persona journey"
```

---

### Task 8: End-to-End Pilot, Full Panel Baseline, and Documentation

**Files:**

- Modify: `e2e/persona-harness/README.md`
- Modify: `CLAUDE.md`
- Generated only: `e2e/test-results/persona-harness/<run-id>/...`

**Interfaces:**

- Consumes: all prior tasks.
- Produces: one valid Bryan pilot, then five valid core-scenario runs and five targeted runs.
- Produces: a cross-persona report that retains persona-specific findings and disagreements.

- [ ] **Step 1: Run all deterministic automated checks**

Run:

```powershell
cd frontend
npx vitest run src/__tests__/unit/personaHarness src/__tests__/unit/views/CanvasView.spec.js src/__tests__/unit/components/CanvasContainer.spec.js src/__tests__/unit/components/PromptDialog.spec.js src/__tests__/unit/components/CanvasNodes.spec.js
npm run build

cd ../e2e
npm run persona:test
npm run test:fixtures
npx playwright test --project=persona-regression
```

Expected: every command passes.

- [ ] **Step 2: Start a Bryan core pilot**

Run: `cd e2e; npm run persona:start -- --persona bryan --scenario core-ambiguous-idea --run-id bryan-core-001`

Expected: healthy handoff JSON with a Canvas URL and artifact directory, but no token.

- [ ] **Step 3: Perform the Computer Use preflight**

Use Computer Use to list available browsers/tabs, bind exactly one visible tab to the emitted Canvas URL, and capture its current state. If no target is available, finalize `bryan-core-001` as `invalid` with blocker `computer-use-unavailable`; do not continue to the full panel and do not substitute Playwright.

- [ ] **Step 4: Run the Bryan pilot without rescue hints**

Give the Bryan subagent the complete `bryan.md` profile and only the core scenario's visible goal. Let it operate until completed, blocked, or abandoned. Finalize and analyze the run.

Expected: `manifest.json`, `actions.jsonl`, `integrity.json`, `metrics.json`, at least one state-specific SVG heat map, and `persona-report.md`. The report must cite event sequences and label the run `simulated_persona`.

- [ ] **Step 5: Review pilot evidence integrity before scaling**

Verify:

- the app Git SHA and content hashes are present;
- no recorded event contains prompt text, API keys, or private data;
- required clicks have semantic action IDs;
- no production vault/Twin/feedback path was used;
- Computer Use observation receipts exist in the task history;
- the report separates observation from interpretation;
- tool latency is not described as human hesitation.

If any item fails, fix the responsible earlier task and repeat `bryan-core-001` with a new run ID.

- [ ] **Step 6: Run the full panel serially**

Run five fresh core runs and five fresh targeted runs. Use run IDs `<persona>-core-001` and `<persona>-targeted-001`. Start and stop a fresh owned runtime for every run. Never reuse a persona subagent's conversation or mutable state.

- [ ] **Step 7: Produce the disagreement-preserving cross-persona report**

Create `e2e/test-results/persona-harness/baseline-001/cross-persona-report.md` with:

```markdown
# Canvas Persona Baseline 001

## Valid Runs
## Invalid Runs
## Required-Function Exposure
## Shared Friction
## Persona-Specific Friction
## Preserved Disagreements
## Good Existing Behavior
## Ranked Improvements
## Smallest Next UI Experiment
## Evidence Limits
```

Rank changes by required-function frequency, affected personas, completion impact, destructive risk, interaction depth, and evidence confidence. Do not use a mean score to erase a blocking failure.

- [ ] **Step 8: Update live documentation with verified commands and boundaries**

In `CLAUDE.md`, keep the development-only boundary and add the exact `persona:test`, `persona:start`, and `persona:analyze` commands. State whether the Computer Use pilot was valid. Do not copy baseline findings into the architecture document as permanent product truth.

- [ ] **Step 9: Run final repository validation**

Run:

```powershell
cd frontend
npm run test:run
npm run build
npm run check:file-sizes

cd src-tauri
cargo test --locked

cd ../../e2e
npm run persona:test
npm run test:fixtures
npx playwright test --project=persona-regression
```

Expected: all commands pass. Record ignored or environment-dependent tests separately; do not describe them as passed.

- [ ] **Step 10: Commit the verified harness documentation**

```powershell
git add CLAUDE.md e2e/persona-harness/README.md
git commit -m "docs: document canvas persona usability workflow"
```

## Completion Gate

Do not claim the harness complete unless:

- a valid Computer Use Bryan pilot exists;
- the full five-person panel has valid core runs or each invalid run is explicitly reported;
- state-specific heat maps and function-depth metrics are generated;
- reports link conclusions to event evidence;
- the real vault, Twin, Constitution, decisions, and production feedback remain untouched;
- the automated focused and full validation commands pass;
- packaged-native behavior is not claimed unless separately exercised through a targetable native Grafyn window.
