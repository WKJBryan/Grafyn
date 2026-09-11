import { describe, expect, it } from 'vitest'
import { publicPredictionLedger, predictionCaptureSummary } from '@/utils/evidencePredictions'

describe('prediction ledger projection', () => {
  it('never exports hidden forecast or frozen batch evidence before the choice', () => {
    const projected = publicPredictionLedger({
      batches: [{ id: 'batch', model: 'local-model', evidence: 'SECRET EVIDENCE' }],
      records: [
        {
          id: 'sealed',
          sealed: true,
          validation: true,
          request: { situation: 'Decision', domain: 'everyday' },
          comparisons: [{ raw_response: 'SECRET FORECAST' }],
          proposed_action: 'SECRET ACTION',
          assumptions: ['SECRET ASSUMPTION'],
        },
      ],
    })
    expect(JSON.stringify(projected)).not.toContain('SECRET')
    expect(projected.records[0].request.situation).toBe('Decision')
  })
  it('keeps missing, abstained, failed and human-judged counts distinct by domain', () => {
    const summary = predictionCaptureSummary([
      { request: { domain: 'everyday' }, sealed: true },
      {
        request: { domain: 'everyday' },
        sealed: false,
        human_choice: 'Both',
        adjudications: { 'goal_paths:before_clarification': 'ambiguous' },
        comparisons: [
          { condition: 'goal_paths', stage: 'before_clarification', status: 'completed' },
          { condition: 'no_evidence', stage: 'before_clarification', status: 'abstained' },
          {
            condition: 'personal_evidence',
            stage: 'before_clarification',
            status: 'provider_failed',
          },
        ],
      },
    ])
    expect(summary.find((row) => row.domain === 'everyday')).toMatchObject({
      records: 2,
      choices: 1,
      sealed: 1,
      comparisons: 3,
      judged: 1,
      ambiguous: 1,
      agree: 0,
      abstained: 1,
      failed: 1,
    })
    expect(summary.find((row) => row.domain === 'product_project')).toMatchObject({
      records: 0,
      choices: 0,
      comparisons: 0,
    })
  })
})
