export const conditionLabels = {
  no_evidence: 'Without personal evidence',
  personal_evidence: 'Personal evidence',
  goal_paths: 'Personal evidence + goal paths',
}

// Export only the same public projection that the UI is permitted to reveal.
export function publicPredictionLedger(ledger) {
  return {
    schema_version: 1,
    exported_at: new Date().toISOString(),
    batches: ledger.batches.map(({ id, model, model_digest, settings, created_at }) => ({
      id,
      model,
      model_digest,
      settings,
      created_at,
    })),
    records: ledger.records.map((record) => {
      if (!record.sealed) return record
      const {
        id,
        batch_id,
        request,
        validation,
        sealed,
        status,
        questions,
        human_choice,
        recorded_at,
      } = record
      return {
        id,
        batch_id,
        request,
        validation,
        sealed,
        status,
        questions: (questions || []).slice(0, 2),
        human_choice,
        recorded_at,
      }
    }),
  }
}

export function predictionCaptureSummary(records) {
  return ['product_project', 'everyday'].map((domain) => {
    const matching = records.filter((record) => record.request?.domain === domain)
    const comparisons = matching
      .filter((record) => !record.sealed)
      .flatMap((record) =>
        (record.comparisons || []).map((comparison) => ({
          ...comparison,
          judgement: record.adjudications?.[`${comparison.condition}:${comparison.stage}`],
        }))
      )
    return {
      domain,
      records: matching.length,
      choices: matching.filter((record) => record.human_choice != null).length,
      sealed: matching.filter((record) => record.sealed).length,
      comparisons: comparisons.length,
      judged: comparisons.filter((item) => item.judgement).length,
      agree: comparisons.filter((item) => item.judgement === 'agree').length,
      disagree: comparisons.filter((item) => item.judgement === 'disagree').length,
      ambiguous: comparisons.filter((item) => item.judgement === 'ambiguous').length,
      abstained: comparisons.filter((item) => item.status === 'abstained').length,
      failed: comparisons.filter((item) =>
        ['provider_failed', 'parse_failed', 'interrupted'].includes(item.status)
      ).length,
    }
  })
}
