import { getTransport } from './transport'

const invoke = (...args) => getTransport().invoke(...args)

export const evidence = {
  installEmbeddings: (scope = 'pilot') => invoke('install_evidence_embeddings', { scope }),
  listPredictions: (scope = 'pilot') => invoke('list_evidence_predictions', { scope }),
  snapshot: (scope = 'pilot') => invoke('evidence_snapshot', { scope }),
  createPilot: () => invoke('create_evidence_pilot', { scope: 'pilot' }),
  saveInterview: (request, scope = 'pilot') =>
    invoke('save_evidence_interview', { scope, request }),
  saveGoal: (request, scope = 'pilot') => invoke('save_evidence_goal', { scope, request }),
  reviewRelationship: (request, scope = 'pilot') =>
    invoke('review_evidence_relationship', { scope, request }),
  reviewStatement: (request, scope = 'pilot') =>
    invoke('review_evidence_statement', { scope, request }),
  processJobs: (scope = 'pilot') => invoke('process_evidence_jobs', { scope }),
  predict: (request, scope = 'pilot') => invoke('predict_evidence_decision', { scope, request }),
  recordChoice: (request, scope = 'pilot') => invoke('record_evidence_choice', { scope, request }),
}
