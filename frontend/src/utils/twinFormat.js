// Pure formatting/labeling helpers shared across Twin Workspace components and
// the twin Pinia store. Extracted verbatim from TwinReviewView.vue (Task 4.3) —
// no behavior changes, just relocated so multiple components can share them.

export const decisionMirrorPresets = [
  { value: 'balanced', label: 'Balanced' },
  { value: 'evidence_strict', label: 'Stricter Evidence' },
  { value: 'insight_search', label: 'Find Blind Spots' },
  { value: 'action_bias', label: 'Push Next Action' }
]

export const configWeightRows = [
  { key: 'notes_weight', label: 'Vault Evidence' },
  { key: 'approved_records_weight', label: 'Trusted Self-Model' },
  { key: 'candidate_records_weight', label: 'Tentative Patterns' },
  { key: 'constitution_weight', label: 'Values Fit' },
  { key: 'action_gaps_weight', label: 'Follow-Through Risk' },
  { key: 'recency_weight', label: 'Current Self' },
  { key: 'evidence_count_weight', label: 'Repeated Evidence' },
  { key: 'outcome_history_weight', label: 'Past Outcomes' },
  { key: 'contradiction_weight', label: 'Tensions' },
  { key: 'breadth_weight', label: 'Reflection Breadth' },
  { key: 'depth_weight', label: 'Reflection Depth' },
  { key: 'evidence_grounding_weight', label: 'Grounded Claims' },
  { key: 'blind_spot_weight', label: 'Blind Spots' },
  { key: 'counter_position_weight', label: 'Counterargument' },
  { key: 'actionability_weight', label: 'Next Step Clarity' },
  { key: 'uncertainty_weight', label: 'Honest Uncertainty' },
  { key: 'privacy_weight', label: 'Privacy Safety' },
  { key: 'unsupported_penalty_weight', label: 'Unsupported Claim Penalty' }
]

export function defaultDecisionMirrorWeights() {
  return Object.fromEntries(configWeightRows.map(row => [row.key, 1]))
}

export function splitLines(value) {
  return value
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean)
}

export function decisionState(item) {
  if (item.prediction_sealed) return 'is-sealed'
  if (item.episode.agreement === true) return 'is-match'
  if (item.episode.agreement === false) return 'is-miss'
  if (!item.episode.outcome) return 'is-pending'
  return ''
}

export function decisionChips(item) {
  const chips = []
  if (item.prediction_sealed) chips.push({ id: 'sealed', label: 'Sealed', cls: 'chip-sealed' })
  if (item.episode.agreement === true) chips.push({ id: 'match', label: 'Matched', cls: 'chip-match' })
  if (item.episode.agreement === false) chips.push({ id: 'miss', label: 'Missed', cls: 'chip-miss' })
  if (!item.episode.outcome) chips.push({ id: 'pending', label: 'Outcome pending', cls: 'chip-pending' })
  return chips
}

export function actionLabel(action) {
  return {
    keep: 'Keep',
    soften: 'Soften',
    not_me: 'Not Me',
    private: 'Private',
    no_train: 'No Train',
    reject: 'Reject'
  }[action] || action
}

export function statusLabel(value) {
  return String(value || 'unknown')
    .split('_')
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ')
}

export function kindLabel(value) {
  return statusLabel(value)
}

export function dimensionLabel(value) {
  return statusLabel(value || 'general')
}

export function formatPercent(value) {
  return `${Math.round((value || 0) * 100)}%`
}

export function formatWeight(value) {
  return Number(value || 0).toFixed(2)
}

export function presetLabel(value) {
  const preset = decisionMirrorPresets.find(item => item.value === value)
  return preset?.label || statusLabel(value || 'balanced')
}

export function sourceTypeLabel(value) {
  const labels = {
    note: 'Note',
    behavior: 'Behavior',
    'interview-question': 'Interview Question',
    'interview-answer': 'Interview Answer',
    decision: 'Decision',
    setup: 'Setup',
    approved_record: 'Approved Record',
    candidate_record: 'Candidate Record',
    constitution_item: 'Constitution',
    action_gap: 'Action Gap'
  }
  return labels[value] || statusLabel(value)
}

export function constitutionSourceLabel(item) {
  if (item.source) return sourceTypeLabel(item.source)
  const first = item.evidence_refs?.[0]
  return sourceTypeLabel(first?.source_type || 'evidence')
}

export function constitutionEvidenceLabels(item) {
  const labels = new Set()
  for (const ref of item.evidence_refs || []) {
    if (ref.source_type) labels.add(sourceTypeLabel(ref.source_type))
    if (ref.source_label) labels.add(ref.source_label)
  }
  return [...labels].slice(0, 4)
}

export function constitutionRunSummary(summary = {}) {
  const parts = [
    `${summary.created_constitution_items || 0} items`,
    `${summary.created_action_gaps || 0} gaps`
  ]
  if (summary.auto_active_items) parts.push(`${summary.auto_active_items} active`)
  if (summary.review_candidate_items) parts.push(`${summary.review_candidate_items} review`)
  if (summary.scanned_behavior_events) parts.push(`${summary.scanned_behavior_events} behavior events`)
  if (summary.scanned_notes) parts.push(`${summary.scanned_notes} notes`)
  if (summary.scanned_interviews) parts.push(`${summary.scanned_interviews} interviews`)
  if (summary.extracted_research_findings) parts.push(`${summary.extracted_research_findings} findings`)
  if (summary.pruned_stale_constitution_items) parts.push(`${summary.pruned_stale_constitution_items} stale items removed`)
  if (summary.pruned_stale_records) parts.push(`${summary.pruned_stale_records} stale records removed`)
  if (summary.updated_setup_entries) parts.push(`${summary.updated_setup_entries} setup entries`)
  if (summary.skipped_domain_claims) parts.push(`${summary.skipped_domain_claims} skipped`)
  return `Constitution: ${parts.join(' / ')}`
}

export function scoreRows(scores = {}) {
  return [
    ['Breadth', scores.breadth_score],
    ['Depth', scores.depth_score],
    ['Grounding', scores.evidence_grounding_score],
    ['Blind Spot', scores.blind_spot_score],
    ['Action', scores.actionability_score],
    ['Counter', scores.counterargument_score],
    ['Uncertainty', scores.uncertainty_score],
    ['Privacy', scores.privacy_score],
    ['Overall', scores.overall_score]
  ].map(([label, value]) => ({
    label,
    value: value == null ? 'n/a' : formatPercent(value)
  }))
}

export function feedbackEventLabel(event = {}) {
  return statusLabel(event.payload?.feedback_type || event.event_type || 'feedback')
}

export function feedbackEventNote(event = {}) {
  return event.payload?.rationale
    || event.payload?.content
    || event.payload?.response?.response_excerpt
    || event.payload?.response?.response_content
    || 'Feedback recorded'
}

export function formatDate(value) {
  return value ? new Date(value).toLocaleString() : ''
}

export function eventLabel(value) {
  return statusLabel(value)
}
