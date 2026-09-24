# Semantic and temporal map

## Goal

Answer two different questions: *which ideas are related now?* and *how did a
person's perspective change?* The first is spatial; the second is a sourced
history. The map must never infer a historical belief from a note's `updated_at`.

## First usable slice (implemented)

- Overview zoom emphasizes named topic hubs and their bridges. Ordinary notes
  remain as clickable circles; zoom, search, and focus expose their labels and
  connections. Membership links are faint at overview scale.
- Existing graph edge kinds give bounded preferred lengths: membership 0.7×,
  note links 1×, related hubs 1.6× the configurable link distance. This is
  structural spacing, not a claim of calibrated semantic similarity.
- A separate append-only perspective ledger records a statement, title,
  change type, reason, source note IDs, effective time, recorded time, and the
  ID of its prior state. Users explicitly author these records.
- The Time panel selects an as-of date and a perspective, follows its backward
  lineage, and highlights its source notes on the **current** graph. A note
  hidden by filters is reported. Historical graph layouts are not reconstructed.
- History lives in app data under `perspectives/states`, one immutable JSON
  record per state. The graph view reads it through dedicated Tauri commands.

## Next increments

1. **Validate on a dense real vault.** Measure time to first paint, pan/zoom
   frame times, label collisions, source-finding success, and the stability of
   positions after a note is added. Retune zoom and link density against those
   observations, not a tiny fixture.
2. **Semantic candidates.** Index note-level representations, retrieve only
   top-k neighbours per note, and store score provenance and model/version.
   Evaluate candidate pairs on manually judged same-topic, analogous,
   contrasting, and unrelated examples. An embedding score indicates content
   proximity, not support or contradiction.
3. **Map layout.** Keep topic region anchors stable across app restarts; use
   scored neighbours for bounded local attraction, repel overlapping notes,
   and preserve explicit relationship kinds. Compare before/after layouts on
   known bridges and unrelated notes. Preserve a search path for orphans.
4. **Historical slices.** Persist versioned semantic assignments and layout
   anchors when they change. Only then can an as-of slice reconstruct what the
   map looked like at that time. Imported historical states must retain both
   effective and recorded times; unknown earlier layouts remain unknown.
5. **Perspective proposals.** Suggest candidate changes from source material
   for human review. Never silently write a belief or lineage. Permit a state
   to branch from an earlier state; expose branches and competing evidence.
6. **Side view.** Once historical slices exist, project the time axis into a
   side-on stack, with selectable lineage strings. Keep the 2D map and dated
   trail as the accessible fallback; test whether free 3D navigation helps
   users trace a real change before adopting it.

## Acceptance checks

- Low zoom exposes regions without concealing notes from click or search.
- Zooming restores all ordinary labels and link types; selecting one note
  exposes its local connections at overview zoom.
- A perspective state cannot be recorded without an existing source note;
  a successor must reference a state of the same perspective at an earlier or
  equal effective time. Earlier records remain unchanged.
- Moving the time slider changes which authored state is visible and its
  evidence highlight; the UI explicitly says the underlying note map is current.
- Deleted source notes remain identifiable in history by ID, with a missing
  evidence indicator rather than a fabricated replacement.

## Boundaries

The SVG still allocates a circle, label, and line for every filtered node and
edge. Opacity and visibility reduce clutter but do not make very large graphs
cheap to simulate or paint. If dense-vault measurements fail, use viewport
culling, rendering batches, or a canvas/WebGL layer as a separate performance
change. Avoid turning visibility rules into data deletion.
