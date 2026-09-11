import { describe, expect, it } from 'vitest'
import {
  edgeDistance,
  isDirectional,
  focusRelationships,
  semanticNeighborhoodGroups,
  applicableGoalRevisions,
  relationshipAssessmentState,
  isEstablishedRelationship,
} from '@/utils/evidenceGraph'

describe('evidence graph semantics', () => {
  it('distinguishes obsolete discovery scores from queued contextual assessment', () => {
    const candidate = { provenance: 'local_semantic_similarity', review_status: 'tentative', similarity: null, assessment: null }
    expect(relationshipAssessmentState(candidate)).toBe('discovery_pending')
    expect(isEstablishedRelationship(candidate)).toBe(false)
    expect(isEstablishedRelationship({ ...candidate, assessment: { result: { verdict: 'related' } } })).toBe(true)
  })
  it('uses bounded monotonic lengths and a neutral length for unknown scores', () => {
    expect(edgeDistance(null)).toBe(100)
    expect(edgeDistance(undefined)).toBe(100)
    expect(edgeDistance(1)).toBeLessThan(edgeDistance(0.5))
    expect(edgeDistance(0.5)).toBeLessThan(edgeDistance(0))
    expect(edgeDistance(-3)).toBe(edgeDistance(0))
    expect(edgeDistance(3)).toBe(edgeDistance(1))
  })
  it('does not imply direction for similarity or contradiction', () => {
    expect(isDirectional('related')).toBe(false)
    expect(isDirectional('contradicts')).toBe(false)
    expect(isDirectional('conflicts')).toBe(false)
    expect(isDirectional('enables')).toBe(true)
  })
  it('gates local semantic candidates on contextual assessment without overriding review', () => {
    const candidate = (assessment, extra = {}) => ({
      provenance: 'local_semantic_similarity',
      review_status: 'tentative',
      assessment,
      ...extra,
    })
    expect(relationshipAssessmentState(candidate(null))).toBe('pending')
    expect(
      relationshipAssessmentState(
        candidate({ stale: true, error: null, result: { verdict: 'equivalent' } })
      )
    ).toBe('stale')
    expect(relationshipAssessmentState(candidate({ error: 'timeout', result: null }))).toBe('failed')
    expect(
      relationshipAssessmentState(
        candidate({ error: null, result: { verdict: 'insufficient' } })
      )
    ).toBe('withheld')
    expect(
      isEstablishedRelationship(candidate({ error: null, result: { verdict: 'equivalent' } }))
    ).toBe(true)
    expect(
      isEstablishedRelationship(candidate({ error: null, result: { verdict: 'unrelated' } }))
    ).toBe(false)
    expect(isEstablishedRelationship(candidate(null, { review_status: 'confirmed' }))).toBe(true)
    expect(
      isEstablishedRelationship(
        candidate(
          { stale: true, error: null, result: { verdict: 'unrelated' } },
          { review_status: 'confirmed' }
        )
      )
    ).toBe(true)
    expect(
      relationshipAssessmentState(
        candidate(
          { stale: false, error: null, result: { verdict: 'equivalent' } },
          { review_status: 'confirmed', review_stale: true }
        )
      )
    ).toBe('stale')
    expect(
      isEstablishedRelationship(
        candidate(
          { stale: true, error: null, result: { verdict: 'equivalent' } },
          { review_status: 'confirmed', review_stale: false }
        )
      )
    ).toBe(true)
    expect(isEstablishedRelationship({ provenance: 'imported', review_status: 'tentative' })).toBe(
      true
    )
  })
  it('retains access to weaker and unscored neighbors', () => {
    const edges = [
      { source: 'a', target: 'b', score: 0.9 },
      { source: 'a', target: 'c', score: 0.2 },
      { source: 'a', target: 'd', score: null },
    ]
    const focused = focusRelationships(edges, 'a', false, 1)
    expect(focused.visible).toEqual([edges[0]])
    expect(focused.hidden).toBe(2)
    expect(focusRelationships(edges, 'a', true, 1).visible).toEqual(edges)
  })
  it('builds overlapping anchor neighborhoods without transitive equivalence or unusable edges', () => {
    const edge = (source, target, extra = {}) => ({
      source,
      target,
      score: 0.8,
      relation: 'related',
      review_status: 'tentative',
      ...extra,
    })
    const groups = semanticNeighborhoodGroups([
      edge('alpha', 'shared'),
      edge('alpha', 'one'),
      edge('beta', 'shared'),
      edge('beta', 'two'),
      edge('alpha', 'rejected', { review_status: 'rejected' }),
      edge('alpha', 'stale', { invalidated: true }),
      edge('alpha', 'unscored', { score: null }),
      edge('alpha', 'effect', { relation: 'enables' }),
    ])
    const alpha = groups.find((group) => group.anchor === 'alpha')
    const beta = groups.find((group) => group.anchor === 'beta')
    expect(alpha.members).toContain('shared')
    expect(beta.members).toContain('shared')
    expect(alpha.members).not.toContain('beta')
    for (const unusable of ['rejected', 'stale', 'unscored', 'effect']) {
      expect(groups.flatMap((group) => group.members)).not.toContain(unusable)
    }
    expect(semanticNeighborhoodGroups([edge('a', 'b')])).toHaveLength(1)
  })
  it('does not revive rejected goals and keeps future revisions out of the current goal set', () => {
    const goal = (id, revision, extra = {}) => ({
      id,
      revision,
      recorded_at: '2000-01-01T00:00:00Z',
      effective_at: null,
      review_status: 'confirmed',
      ...extra,
    })
    const selected = applicableGoalRevisions(
      [
        goal('current', 1),
        goal('current', 2, { effective_at: '2999-01-01T00:00:00Z' }),
        goal('rejected', 1),
        goal('rejected', 2, { review_status: 'rejected' }),
        goal('future', 1, { effective_at: '2999-01-01T00:00:00Z' }),
      ],
      Date.parse('2026-09-05T00:00:00Z')
    )
    expect(selected.map((item) => [item.id, item.revision])).toEqual([['current', 1]])
  })
})
