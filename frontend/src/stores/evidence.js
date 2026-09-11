import { defineStore } from 'pinia'
import { ref, readonly } from 'vue'
import { evidence } from '@/api/evidence'

export const useEvidenceStore = defineStore('evidence', () => {
  const scope = ref('pilot')
  const snapshot = ref(null)
  const busy = ref(false)
  const error = ref('')
  const notice = ref('')
  const prediction = ref(null)
  const predictionRequest = ref(null)
  const predictionHistory = ref({ records: [], batches: [] })
  const batchId = ref('')
  const installingEmbeddings = ref(false)
  const predicting = ref(false)

  async function refreshHistory() {
    predictionHistory.value = await evidence.listPredictions(scope.value)
    if (!batchId.value) batchId.value = predictionHistory.value.batches.at(-1)?.id || ''
  }
  const loadPredictions = () => run(refreshHistory)
  function selectPrediction(record) {
    prediction.value = record
    predictionRequest.value = JSON.parse(JSON.stringify(record.request))
    batchId.value = record.batch_id
  }

  async function run(action) {
    if (busy.value) return null
    busy.value = true
    error.value = ''
    notice.value = ''
    try {
      return await action()
    } catch (cause) {
      const message = cause?.message || String(cause)
      error.value = message.includes('__TAURI_IPC__')
        ? 'Open the Grafyn desktop app to use evidence tools, then try again.'
        : message
      return null
    } finally {
      busy.value = false
    }
  }
  const load = () =>
    run(async () => {
      snapshot.value = await evidence.snapshot(scope.value)
      return snapshot.value
    })
  async function switchScope(nextScope) {
    if (busy.value || !['pilot', 'current'].includes(nextScope) || nextScope === scope.value)
      return false
    snapshot.value = null
    prediction.value = null
    predictionRequest.value = null
    predictionHistory.value = { records: [], batches: [] }
    batchId.value = ''
    error.value = ''
    notice.value = ''
    scope.value = nextScope
    await load()
    return true
  }
  const createPilot = () =>
    run(async () => {
      if (scope.value !== 'pilot') return null
      snapshot.value = await evidence.createPilot()
      notice.value = 'Bryan’s isolated pilot is ready.'
      return snapshot.value
    })
  async function mutate(method, request, message) {
    return run(async () => {
      const result = await evidence[method](request, scope.value)
      snapshot.value = await evidence.snapshot(scope.value)
      notice.value = message
      return result
    })
  }
  const saveInterview = (request) =>
    mutate(
      'saveInterview',
      request,
      request.submit
        ? 'Interview saved. Evidence is queued for processing.'
        : 'Progress saved. You can resume here.'
    )
  const saveGoal = (request) => mutate('saveGoal', request, 'Goal revision saved.')
  const reviewRelationship = (request) =>
    mutate('reviewRelationship', request, 'Relationship review saved.')
  const reviewStatement = (request) => mutate('reviewStatement', request, 'Statement review saved.')
  const processJobs = () =>
    run(async () => {
      await evidence.processJobs(scope.value)
      snapshot.value = await evidence.snapshot(scope.value)
      notice.value = 'Processing finished. Check job status for any unresolved items.'
    })
  const installEmbeddings = () =>
    run(async () => {
      installingEmbeddings.value = true
      try {
        await evidence.installEmbeddings(scope.value)
        snapshot.value = await evidence.snapshot(scope.value)
        notice.value = 'Local embedding model setup finished. Check discovery status below.'
      } finally {
        installingEmbeddings.value = false
      }
    })
  const predict = (request) =>
    run(async () => {
      predicting.value = true
      try {
        prediction.value = null
        predictionRequest.value = JSON.parse(JSON.stringify(request))
        prediction.value = await evidence.predict(request, scope.value)
        batchId.value = prediction.value.batch_id || batchId.value
        await refreshHistory()
        return prediction.value
      } finally {
        predicting.value = false
      }
    })
  const recordChoice = (request) =>
    run(async () => {
      prediction.value = await evidence.recordChoice(request, scope.value)
      await refreshHistory()
      notice.value = 'Your choice has been recorded.'
      return prediction.value
    })
  return {
    scope: readonly(scope),
    switchScope,
    snapshot,
    busy,
    error,
    notice,
    prediction,
    predictionRequest,
    predictionHistory,
    batchId,
    installingEmbeddings,
    predicting,
    installEmbeddings,
    loadPredictions,
    selectPrediction,
    load,
    createPilot,
    saveInterview,
    saveGoal,
    reviewRelationship,
    reviewStatement,
    processJobs,
    predict,
    recordChoice,
  }
})
