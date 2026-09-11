# Automatic evidence ingestion and clustering

Status: approved design; core ingestion, evidence workspace, isolated pilot and narrow derived-state repair implemented as of 2026-09-05. The coverage and validation notes below distinguish delivered behavior from remaining acceptance work. Detailed requirements in later sections are the approved target, not a claim that every proposed integration or experiment is complete.

## Implementation coverage

September 5 follow-up: candidate discovery is now separate from local contextual pair assessment. Equivalent meanings, conflicts and directional hypotheses require assessment; unassessed/withheld candidates remain inspectable. Source/goal context changes invalidate applicability, including human-reviewed links until reaffirmed. The runtime is still Ollama; native NPU routing awaits a compatible model/runtime and actual hardware validation. See the [follow-up plan](../plans/2026-09-05-contextual-relationships.md), [runtime boundary](../../local-embedding-runtime.md), and [benchmark command](../../evidence-maintenance.md#contextual-relationship-baseline).

| Area | Delivered behavior | Remaining boundary |
|---|---|---|
| Source and import | Shared desktop/MCP CSV/XLSX, conversation and document path; explicit person/speaker and headerless column mapping; provenance-based body boundaries; exact locators; repeated-import/scenario deduplication | Preserves extracted text/raw cells, not a byte-for-byte archive of every external input file; old transcript compatibility still uses `ParsedMessage.model` for speaker labels |
| Evidence processing | Durable revision-aware jobs, constrained local extraction, source restrictions/invalidation, structured cases and personal statements/procedures, shared Canvas/Lab context selector | Unresolved mappings and conflicting cases remain exceptions; no inferred answer or blanket endorsement |
| Evidence workspace | Current-vault target and separate Bryan pilot; resumable two-domain interviews; concurrent goal revisions; statement/relationship review; meaning groups and bounded goal paths with receipt panels | Delivered in `components/twin/EvidenceWorkspace.vue` and its components, not a replacement of every legacy `GraphView`/`MiniGraph` surface |
| Local semantic discovery | Embedding adapter/cache, scored passage neighbors and explicit embeddinggemma installation action | embeddinggemma is not installed on the validation machine; real embedding quality, labeled-pair recall/precision and full legacy graph integration are not validated |
| Predictions | Three fixed conditions, frozen batch model digest/settings/evidence, safe sealed clarification and later human choice/adjudication capture | No completed 20-decision study, withheld-case benefit, predictive lift or quantitative causal simulation |
| Repair | Exact before/after snapshot manifests, idempotent apply and conflict-aware rollback; five narrow auto-derived legacy claims repaired | Reviewed/manual items and sources preserved; one suspicious reviewed item remains review-only; no automatic join of the 148 unanswered legacy episodes |

Implementation entry points are `frontend/src-tauri/src/services/{source_content.rs,import/,evidence/,evidence_bridge.rs,evidence_prediction.rs,evidence_repair.rs}`, `commands/evidence.rs`, and `frontend/src/components/twin/EvidenceWorkspace.vue`. See [maintenance commands](../../evidence-maintenance.md) and [project architecture](../../../CLAUDE.md).

### Validation snapshot

- Desktop compilation passes. The final Rust suite passed 341 tests with zero failures and three ignored integration tests; all three ignored tests were also run explicitly and passed. The full frontend suite passed 471 tests across 39 files. The production frontend build and size checks passed.
- An actual local Qwen narrative extraction produced one case, one personal statement and two valid receipts. The file-backed synthetic smoke passed both interview domains, three goal revisions and four goal links, including receipt/restriction/idempotence checks.
- The actual workbook parser produced 42 answered scenario records, 336 row receipts and 294 separately stored model outputs. The real workbook pipeline produced 42 notes, 31 accepted cases and 11 flagged cases; shared context selected 31 cases with 31 exact receipts. These are pipeline counts, not an accuracy result.
- Actual MCP stdio initialization, tool discovery and two identical imports passed: one evidence note persisted with explicit mapping and a durable queued job. Synthetic local three-condition prediction execution passed sealed/revealed checks. Canvas uses the same personal evidence service and saves private exact replay envelopes under the Twin root's `prediction_contexts` before dispatch.
- A copy of the existing Twin state passed five-item apply, repeated no-op and exact rollback. The same narrow five-item repair was then applied to the original derived state after preflight; live preview reports zero remaining automatic changes and one review-only item. No private source files were changed or committed.
- Browser empty/error states and component graph behavior were checked; a fully populated native desktop workspace was not visually inspected. Real embedding semantic quality, a 20-decision study and predictive improvement remain unvalidated.

## Outcome

Import an interview, document, or decision workbook once. Grafyn preserves the source, identifies whose statements it contains, extracts usable evidence and decision cases, and groups related content automatically. The user resolves attribution and alignment exceptions instead of manually copying answers between Excel and the app.

The first release establishes trustworthy ingestion. Better decision accuracy must subsequently be demonstrated on withheld and new decisions; it is not an acceptance claim for this release.

## Historical diagnosis before implementation

- The configured KingWang store contains 148 decision JSON files with no chosen options and no sealed predictions. `select_decision_cases` excludes these entries. Historical answers exist separately in the workbook.
- Auto-derived Constitution claims included generated `Previous:`, `Next:` or `Part of:` wrappers and disputed speaker attribution. The repair audit identified five unreviewed candidates safe for narrow repair and one suspicious reviewed item to preserve for review; historical totals are not a current-state count.
- `services/twin/constitution.rs::distill_constitution_setup_from_notes` derives setup from interviewer questions. `constitution_claim_from_note` searches the whole note for a preference cue but excerpts the beginning, where imported navigation lives.
- Transcript import recognizes a fixed list of role labels. It puts the original speaker label in `ParsedMessage.model`; no stable person attribution is carried through the evidence pipeline.
- Document import generates section/index notes with Previous/Next/Part-of links. `topic_hub.rs::build_graph_cluster_adjacency` gives all parsed links weight 5, including structural navigation. `document` and `interview` are absent from `is_structural_tag`.
- Desktop import writes notes and synchronizes indexes but does not populate a verified decision-case library. MCP has a separate import orchestration path.
- Canvas and Eval Lab have separate context builders. Lab exports discard the complete question/context/settings envelope.
- The June workbook has 42 distinct scenarios repeated across model blocks. Eleven target answers are Neither; eighteen are Neither or combinations. Scenario 9's rationale appears to describe a different question. The two experiment sheets have identical stored question text, despite a name implying modified options; actual prompt history cannot be reconstructed from this workbook alone.

## Assumptions and choices

1. A twin has one configured target person. The app user, interviewer, and target can be different people. A source can contain several speakers.
2. Existing PDF, DOCX, Markdown, JSON conversation, and labeled transcript imports remain supported. Add CSV and XLSX decision-table import so the existing experiment workbook can enter the same workflow.
3. Automatically process attributable, source-grounded evidence. Grounded narrative interpretations and personal procedures can be usable as tentative evidence without prior manual approval. Unresolved attribution, invalid receipts and conflicting answer mappings are quarantined for correction. Tentative usability is distinct from human endorsement; a model's confidence cannot supply endorsement.
4. Keep raw sources and human edits. Derived items can be invalidated and rebuilt with an audit trail.
5. Continue desktop Rust/Vue and the existing stores, review UI, search, and hub mechanisms. Add local semantic similarity for relationship discovery, with versioned embeddings cached on disk. No hosted backend, model training, or new vector database is required for this release.
6. Use bounded local Ollama extraction and honor source restrictions, with no silent cloud fallback. The implemented comparison pilot fixes `qwen3.6:27b` and records its digest. Import preview currently exposes source/speaker/column mapping; a general per-import provider chooser is not implemented.

### Approaches considered

| Approach | Benefit | Limitation |
|---|---|---|
| Patch the current heuristics only | Small and useful for navigation defects | Cannot robustly extract decisions and speaker attribution from varied interviews |
| Let an LLM rewrite, cluster, and approve everything | Flexible with messy inputs | Can invent attribution and case boundaries; difficult to audit or replay |
| Recommended: deterministic source processing plus constrained extraction | Automates routine work while retaining exact receipts and reviewable exceptions | Requires shared evidence metadata and a durable processing job |

## Processing flow

`Import -> preserve source -> segment and attribute -> extract and validate -> store cases/evidence -> cluster and index -> review exceptions`

### 1. Preserve and segment the source

- Assign a source ID, content hash, source revision, and parser version. Preserve extracted text and exact locators; tables also retain raw row cells. The external source remains untouched. A complete local archive of original input bytes is a remaining extension. Never run spreadsheet macros or external workbook links.
- Each segment retains an exact source locator: message/turn index, document section and offsets (page only when available), or workbook sheet/row/cell coordinates.
- Store actual body text separately from app-generated headings, frontmatter, navigation, and hub text. One shared content-extraction helper serves inference, keyword extraction, clustering, and retrieved evidence.
- Preserve paragraph and speaker boundaries. Keep a question with its answering turns; don't make sentence fragments into independent person-model evidence.
- Existing imported notes use their format metadata to locate the source-body region. If its boundary cannot be established, keep it as searchable knowledge and flag it for reprocessing rather than guessing an identity claim.

### 2. Attribute statements to people

- Carry explicit source-level person/speaker mapping into evidence attribution. Existing transcript parsing retains its compatibility speaker label in `ParsedMessage.model`; replacing that legacy field is not required for safe explicit mapping.
- In preview, show the detected target speaker and other speakers. Recognized labels can be mapped once for a source. Ambiguous names need one source-level mapping, not approval on every paragraph.
- Separate `target_statement`, `other_person_statement`, `model_output`, `domain_knowledge`, and `unknown` evidence roles. A quoted author's first-person statement remains the author's statement.
- The target may be interviewer or interviewee. Select by identity mapping, never by assuming the `user` role is the target.
- Imported articles stay knowledge unless a target-authored or target-endorsed passage supports a personal claim. Speaker labels alone do not establish that a quoted passage is an endorsement.

### 3. Extract evidence and decisions

Use deterministic table/turn parsing first. For narrative material, request structured proposals with source segment IDs and exact quotations. Reject invalid IDs, absent quotes, crossed speaker spans, or model-generated answers offered as human choices. Exact quotation establishes provenance, not semantic truth; uncertain interpretations remain candidates.

Store three kinds of output:

- **Knowledge evidence:** material the target may reason with; carries source and attribution without asserting personal beliefs.
- **Personal evidence:** explicit statements, preferences, constraints, and decision procedures. Grounded tentative statements and interpretations are usable with their status and receipts visible; confirmation/rejection remains available in the statement review panel. They are not automatically promoted to human-endorsed Constitution rules. Repeated imports do not count as independent supporting evidence.
- **Decision cases:** situation, offered options, target's actual response, optional rejected options, verbatim rationale, constraints, source receipts, and case kind (`observed_action` or `hypothetical_response`). Predictions and model answers have their own provenance and never supply missing human responses.

Decision responses support a single option, a combination, a reframed action, and an unresolved response. Preserve the actual response text. A response of Neither without an explanation is not enough to infer a new action.

For CSV/XLSX:

- Detect question, target answer, target rationale, model answer, and model-name columns from headers; preview an explicit column mapping when ambiguous. Sheet1 in the existing workbook has no header, so it needs a mapping.
- Deduplicate person-cases across repeated model blocks using target ID plus normalized full question/options and source provenance. Model predictions remain separate observations. Different target answers to the same question remain conflicting/versioned evidence, not last-write-wins.
- A future legacy-episode reconciliation may match exact normalized situations/options; ambiguous matches require review. The current importer stores new structured cases without joining the 148 legacy unanswered entries. Never join answers by row number across sheets.
- Validate rationale/answer/question alignment. A semantic mismatch can be flagged, but the system must not silently move an answer to another question.
- Mark imports of known historical answers as historical. Do not invent a sealed prediction or retroactive prediction accuracy.

### 4. Automatic processing without repeated manual work

- Maintain a small durable queue under the vault-scoped twin data directory. States: queued, processing, completed, needs_review, failed. Persist changes through `write_atomic`.
- Job identity includes source revision, target mapping revision, and extractor version. Retrying the same revision is idempotent. A changed source or attribution invalidates its derived output and schedules reprocessing.
- One extraction worker per vault, one request in flight, bounded source segments per request. Run outside store locks. A failed provider call retains raw import and a visible retry action; no endless retries or hidden cloud fallback.
- Enqueue after source import or content edits, including MCP imports. A startup reconciliation scan finds unprocessed revisions. Hub rewrites and derived metadata updates must not enqueue themselves indefinitely.
- Before committing results, recheck source revision, target mapping, and vault identity. Discard stale results. Vault switches cannot send a completed job into the newly selected vault.
- Automatically make structured and narrative evidence usable when attribution and receipts validate and no conflict/alignment issue blocks it. Inferred interpretations retain tentative status and remain reviewable; usable evidence is not labeled as human-endorsed. Review is an exception/correction workflow rather than a gate on every interpretation.
- Source privacy/no-train restrictions propagate into extraction, retrieval, and exports. Removing or restricting a source also removes its derived items from usable context.

### 5. Cluster content by meaning and decision context

Retain the existing hub and graph machinery, but repair its inputs:

- Exclude generated Previous/Next/Part-of edges from topical clustering. Keep them available for document navigation. Preserve genuine authored links; identify generated wrappers by provenance rather than blanket string deletion.
- Exclude transport/format tags such as document, interview, import, source names, speaker-role labels, and generic chapter headings from topic seeds.
- Build candidate neighbors from cleaned passage content using both local semantic embeddings and the existing keyword/TF-IDF services. Include authored semantic links. Structural adjacency alone cannot establish a topic cluster. Embedding similarity proposes related passages; it does not establish support, contradiction, or personal belief.
- Keep subject identity and evidence role as filters. A shared topic cannot promote knowledge into a personality trait or mix statements across people.
- Permit multiple topic memberships and unclustered items. Do not force every section into a major community.
- For decision retrieval, use situation, constraints, alternatives, and decision-procedure cues. Merely sharing a document or vocabulary is insufficient evidence of the same decision pattern. An LLM may propose related-case links with cited receipts; unverified links do not become behavioral facts.
- Cluster labels are navigation aids. Re-clustering does not rewrite source content, synthesize a Constitution item, or change human review decisions.

### 5a. Discover relationships and show why they exist

Added after the owner's 2026-09-05 clarification: relatedness must drive both clustering and visible links. The graph is an inspectable view of the same relationship records used by retrieval, not a separate visualization-generated network.

**Granularity:** compare coherent passages and decision cases, retaining their parent notes. The overview renders notes and cluster summaries. Expanding a note reveals the passages responsible for its relationships. Do not connect entire long documents merely because one passage overlaps, or create thousands of visible passage nodes by default.

**Two independent relationship dimensions:**

- Topical/decision-context similarity: whether passages concern similar ideas, constraints, or decisions. Used for candidate discovery and weighted topical communities.
- Evidence relationship: related, supports, contradicts, expands, questions/answers, example, or structural part-of. Preserve the existing typed-link vocabulary and direction; a similar decision procedure is a described related link, not another speculative ontology.

Different choices in different situations are not automatically contradictory. Support/contradiction proposals must cite the relevant statements and identify compatible scope, subject, and time; otherwise label them related or leave the interpretation unresolved. A shared cluster never supplies independent corroboration.

**Small relationship record:** stable endpoint IDs and source revisions, relation/direction, provenance (authored, similarity, or semantic extraction), similarity score and model/version where applicable, proposed/reviewed/rejected status, short explanation, and source receipts at both endpoints. Scores are ranking measures, not probabilities that a claim is true. Keep the similarity score distinct from review status; do not display an invented confidence percentage.

**Bounded discovery:**

1. Encode cleaned passages with an available configured local embedding model. Cache by content hash, model version, and preprocessing version; compare only compatible vectors. Model changes rebuild the cache. Do not silently download a model or send documents to a cloud embedding service.
2. Form a bounded union of lexical and semantic nearest neighbors. Exclude self-matches, navigation-only content, restricted material, and repeated copies of the same source passage. Repeated copies do not increase relationship strength or evidence count.
3. Run the constrained relationship classifier only on candidate pairs, using the configured extraction provider and exact passages. Cache per pair and source revision. Preserve proposed status for inferred evidence claims; users need not approve every generic similar-content edge to browse it.
4. Build weighted topical communities from similarity and genuine content relations. Structural edges, generated cluster memberships, and inferred contradiction labels alone do not pull nodes into a community. Avoid circular feedback: a generated hub cannot reinforce the very relationship that created it.
5. Update affected relationships on import/edit/delete. Keep unaffected reviewed links and stable cluster identity where membership remains substantially the same. A rejected inferred pair stays rejected until its source meaning changes or the user explicitly reconsiders it.

For a small vault, exact similarity against cached vectors is sufficient. Start with at most ten candidates per passage from each retrieval method and at most five visible inferred neighbors per node; these are working resource/display limits, not claims of optimal accuracy. Tune discovery thresholds on labeled related/unrelated passage pairs before enabling automatic links. If the local embedding runtime is unavailable, show lexical-only discovery explicitly and retain a retry action; do not label lexical results as semantic verification.

**Graph behavior, extending the existing D3 view:**

- Overview: named topic groups and their strongest cross-group links. A node can belong to several groups; show secondary membership on selection. Isolated notes remain visible as unclustered.
- Focus: selecting a note shows its strongest related content across documents and its group memberships. Expanding a group reveals its members without opening every other group.
- Edges: target length represents relational closeness within the selected relationship layer: stronger relatedness yields a shorter link. Use a bounded monotonic mapping from the layer's score to target length. Keep width neutral initially so it does not compete with length. Arrows appear only on directional relations. Proposed relationships use dashed lines, reviewed/authored relationships solid lines. Relation labels and a legend supplement color. Unscored edges use a neutral length and expose their unscored status.
- Clicking or keyboard-selecting an edge opens an evidence panel with both exact passages, the relation explanation, source location/time, and provenance/review status. Allow accept, reject, and relation correction without editing source text. Provide the same neighbors and actions in a keyboard-accessible list.
- Filters: relation type, proposed/reviewed, source, target person, and time. Document-navigation links are available as a separate structural layer, off by default in the meaning view. Clearly identify hidden weak links and offer expansion instead of silently dropping them.
- Layout: replace the current single global D3 link distance with per-edge target distances, and keep stable positioning on refresh. A force layout cannot satisfy every pairwise distance simultaneously; focused neighborhoods emphasize the selected node's distances. Explain that near means more closely related in the selected layer, not more agreeable or more certain. Do not treat the final screen coordinates as numeric data or equate semantic similarity with causal influence.
- Node size represents visible connectivity or group membership count, labeled in the legend; it never implies that a statement is truer or a person is more certain.

**Example to validate:** two differently worded passages about preserving a project's objective through negotiation should be discoverable across files. Two chapters connected only by Previous/Next should not receive a semantic edge. Two passages about AI with different constraints should not automatically become the same decision procedure. These are test fixtures, not claims of confirmed relationships in KW's current data.

### 5b. Person-specific goals and consequence paths

Added after the owner's further clarification: the useful relationship may be a chain from action through first- and second-order consequences to an overarching goal, including what that goal means to the target person. Topic clustering alone does not express this. Person-scoped concurrent goals, revisions and bounded receipt-backed paths are implemented in the evidence workspace. The richer criteria, extraction and visualization requirements below remain the design target where not covered by the implementation table; an empirically validated causal simulator is not implemented.

**Keep two selectable views over shared evidence:**

- Meaning view: which passages and cases express related concepts, using the closeness layout above.
- Goal-path view: how this person believes an action enables or obstructs an outcome and a goal. Show actions, intermediate outcomes, constraints, and goals as distinct node kinds, each grounded in source passages. Preserve topic navigation but do not combine all edge scores into one similarity/causality number.

**Goal identity is richer than a label.** Store target-person ID, goal ID, their operational definition in their own words, beneficiary, scope, time horizon when stated, success criteria, examples/counterexamples, competing goals and unacceptable costs, source receipts, and validity/revision. Unknown fields remain unknown. Importance and thresholds are hypotheses unless stated or supported by reviewed choices. Separate stated aspirations from observed priorities. Similar wording across people creates an optional related-concept link; it never merges their goals. Within one person, identical wording can still refer to different contexts or times.

**Concurrent and changing goals:** a person can pursue several goals at once, across different roles and timescales. Do not require one overarching goal, one total ranking, or numeric weights summing to one. An action can advance several goals and impede others; retain these relationships together.

- Track each goal independently with active, paused, achieved, or abandoned status when supported. A new goal does not replace an existing goal unless the source explicitly establishes replacement. Silence about a goal is not abandonment.
- Record context-specific priorities and non-negotiable constraints with receipts. For example, reach may matter most for one program while quality dominates another. An inferred tradeoff remains a hypothesis; do not invent a fixed priority from a single choice.
- Changes to meaning, targets, deadlines, priority, or status create revisions with both effective time (when the change applied) and recorded time (when Grafyn learned it), plus the reason/source when known. Unknown effective dates remain unknown. Do not treat import time as the historical date of a goal change.
- Store the relevant goal IDs and revisions with each decision/prediction. Historical evaluation uses only information available at that decision time, preventing later priorities from rewriting earlier predictions. Retrospective reconstructions remain separately labeled.
- The goal-path view can show several selected goals, shared supporting actions, conflicts, and paused/historical goals on demand. Switching time/context reveals the applicable goal set. A goal's urgency, importance, and graph proximity remain distinct attributes.

**Measurable goal criteria:** vague phrases such as reach many people quickly remain incomplete until the target's intended quantity and timeframe are grounded. Each measurable criterion records:

- Metric and counting rule: what counts, unit, population, deduplication, and denominator for rates. Views, unique attendees, completed training, and demonstrated capability are different metrics.
- Baseline value and observation date when known, separately from the desired change or final target.
- Comparator and target: at least, at most, or a range; distinguish minimum acceptable from preferred results when stated.
- Start event/date, deadline or duration, and any sustained-success period. A 90-day goal needs an anchor such as program launch, not just a duration.
- Measurement source and observation schedule; separate observed progress from forecasts and missing measurements.
- Constraints and tradeoffs such as budget, staff time, quality, or exclusion criteria. Meeting a reach metric does not silently satisfy the person's broader definition of impact.
- Per-field evidence/provenance: explicitly stated, inferred candidate, confirmed, or unknown. Never convert many into an invented number or quickly into an invented deadline. Preserve qualitative goals when quantification would misrepresent the target; mark an agreed proxy as a proxy.

Illustrative definition, not a KW claim: at least 1,000 unique public-sector staff complete a two-hour workshop within 90 days of program launch, within an SGD 50,000 delivery budget. Completion and attendance must have explicit counting rules. Any further capability or adoption objective requires its own criterion; workshop completion alone cannot prove it.

The goal node displays a compact target/deadline and observed progress only when known, with unresolved fields visible on expansion. Import extracts concrete criteria from sources automatically. If missing criteria materially affect a prediction, the app asks a focused question about the quantity, deadline, or tradeoff; it does not block unrelated imports. Criterion changes create a new goal revision, preserving which definition was available when a prediction was made. Every effect path must identify the goal criterion it contributes to; an effect of uncertain size or timing remains insufficient to claim the target will be achieved.

**Effect links:** use a separate relation family for `enables`, `inhibits`, `requires`, and `contributes_to`, with direction, source receipts, stated conditions, and delay/magnitude only when supported. Every link distinguishes a target-stated causal belief, an extracted hypothesis, and an empirical claim with outcome evidence. A sequence of events or a plausible model explanation does not prove a causal effect. Source validation does not upgrade causal evidence automatically.

First-order and second-order effects are relative to the selected action: its outgoing effects and the next step along a specified path. They are not permanent node labels. A distant effect can be larger than a direct one, and hop count is not elapsed time. Keep feedback loops visible as cycles; traversal has a depth bound and must not turn a cycle into repeated independent support.

**Extraction and retrieval:**

1. Extract explicit goal definitions and statements about mechanisms, retaining exact attribution and scope. Use the full question/answer context where needed.
2. Propose cross-source connections when outcomes and goals match in meaning, beneficiary, scope, and time. Embeddings generate candidates, while receipts and reviewed interpretations establish the relationship type. Missing connecting mechanisms remain visible gaps; never fill them with asserted facts to complete a path.
3. Keep alternative pathways, opposing consequences, and goal conflicts. Retrieve an action's relevant goal paths together with analogous actual choices and corrections, so the twin does not automatically behave like an ideal goal optimizer.
4. When an ambiguous goal changes the interpretation of a real decision, surface one targeted clarification with concrete contrasting examples. Do not impose a generic goal-definition questionnaire on every import.

**Goal-path visualization:** select a goal to trace supporting and obstructing paths back to actions, or select an action to expand consequences one level at a time. Use directional labels and separate visual lanes for goal progress and costs/conflicts. Preserve a shared node ID if it appears in several paths. Length encodes scored relational closeness within this view only where a defensible score exists; otherwise use neutral link lengths and indicate unknown strength. The number of visible steps communicates direct versus indirect consequences. Time delay, effect magnitude, and uncertainty appear as separate labeled attributes when known. A short path is not automatically better, more probable, or more beneficial.

**Minimum goal-path acceptance fixtures:**

- Two people use the word impact, one meaning people reached and another meaning durable capability: retain two goal definitions and divergent paths.
- Two simultaneous goals survive importing a third; one action can support one and hinder the other. A new mention does not silently replace or reprioritize either goal.
- A priority change learned later preserves both effective and recorded times; replay of an earlier prediction cannot access the later revision. Paused goals can resume without losing their history, and unmentioned goals are not automatically abandoned.
- Many people quickly yields unknown target and deadline; an explicit 1,000 unique completions within 90 days of launch preserves count, completion definition, and time anchor. Repeated attendance cannot inflate unique completions. Missing observations never render as zero or completed.
- A changed target/deadline keeps prior predictions attached to the original goal revision. A link suggesting increased attendance does not assert that a completion or capability target was met.
- One person's apparent contradiction resolves under different constraints; retain both scoped statements without merging or deleting either.
- An explicit action -> outcome -> goal chain renders its first and second steps, with exact receipts per edge; an unsupported bridge renders as a proposed gap.
- A harmful side effect and a competing goal remain visible even when the main path supports the chosen goal.
- Same topic with no causal statement does not create an effect edge; a target-stated belief remains labeled as that belief even after review.

Deliver the inspectable, evidence-grounded map first. Quantitative counterfactual simulation requires separately specified causal assumptions and suitable outcome/intervention data. Do not multiply language-model confidence scores along paths or promise predictive accuracy from adding this visualization.

### 6. Consume and evaluate the same evidence

- Move the reusable context-packet assembly out of the Canvas command-private path into a service usable by Canvas and the separate Lab. Preserve Advisor/Simulation behavior and intentional Lab ablations.
- Include usable decision cases, personal statements and target evidence with receipts, preserving tentative/confirmed status. Quarantined, unresolved-attribution, superseded, restricted, and held-out items are excluded by the shared selector.
- Export the complete run envelope: full question/options, actual submitted prompt/context, source IDs and revisions, model/runtime identity, settings, parse status, and later target response. A base-model control that intentionally lacks context must be labeled accordingly.
- Assign evaluation exclusion by source/scenario group before case retrieval or rule extraction. Duplicates, derived rules, and quotes containing held-out answers inherit exclusion. Historical workbook cases start as development data unless explicitly assigned otherwise; no accuracy claim comes from replaying their answers back into context.
- Keep scoring and accuracy dashboards in the external Lab, following the existing product boundary.

## App experience

The Import screen shows recognized source content and explicit target-person/speaker mapping. CSV/XLSX can be previewed again with worksheet, column and first-row mappings; issues remain attached to cases. Identity fields are optional for knowledge import and required for personal attribution. A recognized `user` role does not select the target person.

The Evidence workspace offers **Current vault** and **Bryan pilot** scopes without changing the app's selected vault. The pilot is isolated under `data/pilots/bryan`, uses subject `bryan-pilot`, and starts without seeded private examples. The current scope resolves the explicitly mapped target; it does not automatically treat every document as that person's statement. Four component views cover interview drafts/submissions, concurrent goals, relationships, and predictions. Personal statements/procedures and edges expose exact receipts with confirm/reject or relation-correction actions. Generalized per-case editing/batch exception workflows remain future work where absent from those views.

Meaning groups expose scored neighbors, overlapping membership and expandable groups; source/status/time filters and goal selection keep the view inspectable. The same view displays bounded action/consequence/constraint/goal paths. Dashed tentative edges differ from reviewed edges. Relatedness, human review and causal strength remain separate. **Process evidence** processes pending material; missing local embeddings offer **Install embeddinggemma locally**, never an automatic download.

### Fixed comparison and sealed questions

Prediction batches fix `qwen3.6:27b`, its installed digest, an evidence snapshot, and settings `temperature: 0`, `top_p: 1`, `seed: 42`, `num_predict: 2048`. Exactly three conditions run: `no_evidence`, `personal_evidence`, `goal_paths`. Payloads and outcomes are persisted for external analysis. The digest/settings constrain comparisons but do not guarantee bitwise determinism; changed restrictions or invalid evidence require a new batch.

Validation mode seals predictions, raw responses and evidence context until the actual human choice is recorded. Up to two pre-reveal clarifications use neutral app-authored questions selected from the allowed topics: timeframe, budget, success metric, competing goals, constraints and alternatives. Arbitrary model questions cannot leak the sealed answer. Later adjudication records agree/disagree/ambiguous per comparison; it is capture for external evaluation, not an in-app accuracy dashboard.

## Existing-data repair

The implemented maintenance path is narrower than the complete migration sequence below: it quarantines only proven, unedited auto-derived candidate claims using exact manifests, preserves active/manual/guided/research records and source files, and refuses conflicting rollback writes. Copy validation and the authorized five-item live repair are complete. The reviewed suspicious item remains `review_only`. Workbook ingestion does not retroactively populate or seal the 148 legacy episodes. See the [maintenance guide](../../evidence-maintenance.md) for commands and the scope of each mutation.

1. Produce a read-only repair manifest and snapshot the affected derived state before changes.
2. Mark proven generated navigation claims unusable; preserve originals and receipts in the repair history. Source this classification from known import structure. Send uncertain personal claims or speaker mappings to review.
3. Reprocess the KingWang sources with an explicit target mapping. Import the workbook through the new app importer, deduplicate repeated scenarios, and flag the scenario-9 alignment issue.
4. Populate accepted historical cases only from verified answers. Retain the 148 existing episode IDs and join only unambiguous matches; unmatched episodes remain unanswered.
5. Rebuild derived hubs/indexes. Retain manual notes, authored links, manual Constitution items, review decisions, and source files.
6. A second identical repair must make no new cases or changes. Rollback restores the recorded previous derived state and rebuilds indexes. Refuse to overwrite a subsequently edited item during rollback; surface that conflict.

## Implementation slices and acceptance checks

This table retains the approved acceptance target and original integration suggestions. The implementation coverage table above is authoritative for delivered locations and completed verification; these rows are not a checklist of already-passed experiments.

| Slice | Files / responsibility | Required verification |
|---|---|---|
| 1. Prevent bad evidence and structural clusters | New shared source-content helper; `services/import/{document,transcript}.rs`, `models/import.rs`, `services/twin/constitution.rs`, `services/topic_hub.rs` | Preference in body yields the body quote; navigation yields zero claims; target interviewee and target interviewer fixtures both attribute correctly; chapter navigation and format tags cannot create a semantic cluster |
| 2. Populate cases through import | New `services/import/decision_table.rs`; evidence/case metadata in `models/twin.rs`; focused extraction/validation module; `services/twin/decisions.rs` | Multi-sheet repeated model rows create one target case; Neither/combinations survive; swapped rationale is flagged; missing answers remain missing; all accepted cases have valid source locators |
| 3. Make processing automatic | Small shared ingestion job service, desktop/MCP import integration, source-edit invalidation, Import and Twin review components | Import completes despite extractor outage; restart resumes pending job once; source edit/vault switch invalidates stale output; no duplicate jobs from hub writes; one source-level mapping resolves affected items |
| 4. Discover and visualize content relationships | Local embedding adapter/cache and bounded pair discovery; shared relationship records; `services/graph_index.rs`, `commands/graph.rs`, `GraphView.vue`, `MiniGraph.vue`, `GraphSettings.vue`, and an edge evidence panel | Paraphrased related passages are found; merely adjacent chapters stay unlinked semantically; source edits invalidate stale edges; proposed/reviewed links differ visibly; selecting an edge exposes both exact receipts; layout and display limits preserve access to weaker links |
| 5. Map goals and consequence paths | Person-scoped goal/effect records beside existing twin records; bounded extraction and traversal; goal-path mode in the existing graph and evidence panel | Same goal label across people is not merged; explicit first-/second-order paths retain scope and receipts; missing mechanisms remain gaps; conflicting goals and harmful effects are visible; target beliefs are not presented as measured causality |
| 6. Repair and verify end to end | Repair manifest/apply/rollback path; shared context-packet service; Lab run export | Snapshot repair preserves sources and manual edits; repeat repair is a no-op; rollback works; accepted case appears in actual model context; restricted/held-out case and derived rule do not; replay envelope contains the actual prompt |

Use redacted synthetic regression fixtures mirroring observed failures. Do not commit private interview/workbook contents to this public repository. Run focused Rust tests, frontend import/review/graph tests, the production frontend build, and sidecar compatibility checks. Run actual import/reprocess/context inspection against a copied vault before applying the reviewed repair to the live vault. Check graph discovery against manually labeled passage pairs: candidate recall and displayed-edge precision have different denominators and must both be reported. Include paraphrases, generic-topic false positives, contradictions, navigation, and duplicates. Inspect the real graph interactions and evidence panel; unit tests establish pipeline correctness, while separate withheld-case experiments establish predictive benefit.

## Deliberately deferred

Fine-tuning, a vector database or large-scale approximate-neighbor infrastructure, broad UI redesign beyond the relationship graph, automatic merging of contradictory beliefs, and a new accuracy dashboard. These can be evaluated after this loop works.

## Documentation

Keep CLAUDE.md and the implementation/validation tables here aligned with delivered behavior. Broader TWIN_RAG_SPEC.md and TWIN_ACCURACY_ROADMAP.md updates remain separate where their legacy descriptions have not been reconciled. Existing uncommitted Canvas changes belong to the user and must be preserved.
