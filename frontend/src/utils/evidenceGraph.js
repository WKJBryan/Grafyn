// Scores are layer-specific relatedness, never probability or causal magnitude.
export function edgeDistance(score, neutral = 100) {
  if (typeof score !== 'number' || !Number.isFinite(score)) return neutral
  return neutral * (1.8 - 1.2 * Math.max(0, Math.min(1, score)))
}

export function isDirectional(relation) {
  return [
    'supports',
    'expands',
    'questions',
    'answers',
    'example',
    'part_of',
    'enables',
    'inhibits',
    'requires',
    'contributes_to',
  ].includes(relation)
}

export function relationshipAssessmentState(edge) {
  if (edge.review_stale) return 'stale'
  if (edge.provenance !== 'local_semantic_similarity' || edge.review_status === 'confirmed')
    return 'established'
  if (edge.similarity === null && !edge.assessment?.result) return 'discovery_pending'
  if (!edge.assessment) return 'pending'
  if (edge.assessment.stale) return 'stale'
  if (edge.assessment.error) return 'failed'
  const verdict = edge.assessment.result?.verdict
  if (!verdict) return 'pending'
  return ['unrelated', 'insufficient'].includes(verdict) ? 'withheld' : 'established'
}

export function isEstablishedRelationship(edge) {
  return relationshipAssessmentState(edge) === 'established'
}

export function focusRelationships(edges, focus, expanded, limit = 8) {
  const neighbors = edges.filter((edge) => !focus || edge.source === focus || edge.target === focus)
  const ranked = [...neighbors].sort((a, b) => (b.score ?? -1) - (a.score ?? -1))
  return {
    visible: expanded || !focus ? ranked : ranked.slice(0, limit),
    hidden: expanded || !focus ? 0 : Math.max(0, ranked.length - limit),
  }
}

// Anchor neighborhoods can overlap. They do not assert transitive equivalence.
export function semanticNeighborhoodGroups(edges, limit = 8) {
  const semantic = edges.filter(
    (edge) =>
      !edge.invalidated &&
      edge.review_status !== 'rejected' &&
      typeof edge.score === 'number' &&
      Number.isFinite(edge.score) &&
      [
        'related',
        'equivalent',
        'conflicts',
        'supports',
        'contradicts',
        'expands',
        'questions',
        'answers',
        'example',
        'part_of',
      ].includes(edge.relation)
  )
  const seen = new Set()
  return [...new Set(semantic.flatMap((edge) => [edge.source, edge.target]))]
    .sort()
    .flatMap((anchor) => {
      const neighbors = semantic
        .filter((edge) => edge.source === anchor || edge.target === anchor)
        .sort((a, b) => b.score - a.score)
      const rankedIds = [
        ...new Set(neighbors.map((edge) => (edge.source === anchor ? edge.target : edge.source))),
      ]
      const members = [anchor, ...rankedIds.slice(0, limit)]
      const signature = JSON.stringify([...members].sort())
      if (members.length < 2 || seen.has(signature)) return []
      seen.add(signature)
      return [
        { id: anchor, anchor, members, hiddenNeighbors: Math.max(0, rankedIds.length - limit) },
      ]
    })
}

export function applicableGoalRevisions(revisions, asOf = Date.now()) {
  const latest = new Map()
  for (const goal of [...revisions].sort((a, b) => a.revision - b.revision)) {
    if (
      !(Date.parse(goal.recorded_at) <= asOf) ||
      (goal.effective_at && !(Date.parse(goal.effective_at) <= asOf))
    )
      continue
    latest.set(goal.id, goal)
  }
  return [...latest.values()].filter(
    (goal) => !goal.invalidated && goal.review_status !== 'rejected'
  )
}
