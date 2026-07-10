import { defineStore } from 'pinia'
import { computed, reactive, ref } from 'vue'
import { twin as twinApi } from '@/api/client'
import { defaultDecisionMirrorWeights, splitLines, constitutionRunSummary, statusLabel } from '@/utils/twinFormat'

const TUTORIAL_STORAGE_KEY = 'grafyn.twinWorkspaceTutorial.dismissed'

/**
 * Owns all Twin Workspace UI state and API calls (`TwinReviewView.vue` and its
 * per-tab components under `components/twin/`). Extracted from the view in
 * Task 4.3 — behavior-identical, state names preserved from the original refs.
 */
export const useTwinStore = defineStore('twin', () => {
  // State
  const activeTab = ref('overview')
  const reviewRecords = ref([])
  const memoryDigestItems = ref([])
  const constitutionItems = ref([])
  const actionGaps = ref([])
  const decisions = ref([])
  const selectedRecordState = ref('candidate')
  const selectedRecordId = ref(null)
  const selectedEvidence = ref([])
  const evidenceLoading = ref(false)
  const runningTwinInference = ref(false)
  const runningConstitutionInference = ref(false)
  const savingSetup = ref(false)
  const savingConfig = ref(false)
  const exportingBenchmark = ref(false)
  const message = ref(null)
  const showTutorialIntro = ref(localStorage.getItem(TUTORIAL_STORAGE_KEY) !== 'true')
  const setupDraft = reactive({
    twin_name: '',
    twin_role: '',
    source_boundaries: '',
    values: '',
    tastes: '',
    constraints: '',
    somatic_cues: '',
    action_tendencies: ''
  })
  const configDraft = reactive({
    preset: 'balanced',
    advanced_enabled: false,
    weights: defaultDecisionMirrorWeights()
  })

  // Getters
  const activeConstitutionCount = computed(() =>
    constitutionItems.value.filter(item => ['active', 'candidate', 'softened'].includes(item.status)).length
  )
  const activeActionGapCount = computed(() =>
    actionGaps.value.filter(gap => ['active', 'candidate', 'softened'].includes(gap.status)).length
  )
  const pendingReviewCount = computed(() =>
    memoryDigestItems.value.length + reviewRecords.value.filter(item => item.record.promotion_state === 'candidate').length
  )
  const pendingOutcomeCount = computed(() =>
    decisions.value.filter(item => !item.episode.outcome).length
  )
  const healthSummary = computed(() =>
    `${constitutionItems.value.length} principles / ${actionGaps.value.length} gaps / ${decisions.value.length} decisions`
  )
  const recentDecisions = computed(() => decisions.value.slice(0, 4))
  const topActionGaps = computed(() => actionGaps.value.slice(0, 4))
  const filteredReviewRecords = computed(() =>
    reviewRecords.value.filter(item => item.record.promotion_state === selectedRecordState.value)
  )
  const groupedConstitution = computed(() => {
    const groups = new Map()
    for (const item of constitutionItems.value) {
      const key = item.dimension || 'general'
      if (!groups.has(key)) groups.set(key, [])
      groups.get(key).push(item)
    }
    return [...groups.entries()]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([dimension, items]) => ({ dimension, items }))
  })

  // Actions
  async function loadWorkspace() {
    try {
      const [
        review,
        digest,
        constitution,
        gaps,
        decisionRows,
        setup,
        mirrorConfig
      ] = await Promise.all([
        twinApi.getReview(),
        twinApi.listMemoryDigest(),
        twinApi.listConstitutionItems(),
        twinApi.listActionGaps(),
        twinApi.listDecisionEpisodes(),
        twinApi.getConstitutionSetup(),
        twinApi.getDecisionMirrorConfig()
      ])
      reviewRecords.value = review
      memoryDigestItems.value = digest
      constitutionItems.value = constitution
      actionGaps.value = gaps
      decisions.value = decisionRows
      loadSetupDraft(setup)
      loadConfigDraft(mirrorConfig)
    } catch (err) {
      showMessage('error', err.message || 'Failed to load twin workspace')
    }
  }

  async function runTwinInference() {
    runningTwinInference.value = true
    try {
      const summary = await twinApi.runInference()
      await loadWorkspace()
      showMessage('success', `Records: ${summary.created_records} created, ${summary.updated_records} updated`, 3500)
    } catch (err) {
      showMessage('error', err.message || 'Failed to run record inference')
    } finally {
      runningTwinInference.value = false
    }
  }

  async function runConstitutionInference() {
    runningConstitutionInference.value = true
    try {
      const summary = await twinApi.runConstitutionInference()
      await loadWorkspace()
      showMessage('success', constitutionRunSummary(summary), 4500)
    } catch (err) {
      showMessage('error', err.message || 'Failed to run constitution inference')
    } finally {
      runningConstitutionInference.value = false
    }
  }

  async function reviewConstitutionItem(id, action) {
    try {
      await twinApi.reviewConstitutionItem(id, { action })
      await loadWorkspace()
      showMessage('success', 'Updated constitution item', 1800)
    } catch (err) {
      showMessage('error', err.message || 'Failed to update constitution item')
    }
  }

  async function reviewActionGap(id, action) {
    try {
      await twinApi.reviewActionGap(id, { action })
      await loadWorkspace()
      showMessage('success', 'Updated action gap', 1800)
    } catch (err) {
      showMessage('error', err.message || 'Failed to update action gap')
    }
  }

  async function reviewMemoryDigestItem(id, action) {
    try {
      await twinApi.reviewMemoryDigestItem(id, { action })
      await loadWorkspace()
      showMessage('success', 'Updated digest item', 1800)
    } catch (err) {
      showMessage('error', err.message || 'Failed to update digest item')
    }
  }

  async function setPromotion(recordId, promotionState) {
    try {
      await twinApi.setPromotion(recordId, promotionState, null)
      await loadWorkspace()
      showMessage('success', `Set record to ${statusLabel(promotionState)}`, 1800)
    } catch (err) {
      showMessage('error', err.message || 'Failed to update record')
    }
  }

  async function openEvidence(recordId) {
    selectedRecordId.value = recordId
    selectedEvidence.value = []
    evidenceLoading.value = true
    try {
      selectedEvidence.value = await twinApi.resolveEvidence(recordId)
    } catch (err) {
      showMessage('error', err.message || 'Failed to load evidence')
    } finally {
      evidenceLoading.value = false
    }
  }

  async function updateDecisionOutcome(id, update) {
    try {
      const payload = { ...update }
      if (payload.regret_score == null) delete payload.regret_score
      await twinApi.updateDecisionOutcome(id, payload)
      await loadWorkspace()
      showMessage('success', 'Updated decision outcome', 1800)
    } catch (err) {
      showMessage('error', err.message || 'Failed to update decision')
    }
  }

  async function saveSetup() {
    savingSetup.value = true
    try {
      await twinApi.saveConstitutionSetup({
        twin_name: setupDraft.twin_name.trim(),
        twin_role: setupDraft.twin_role.trim(),
        source_boundaries: splitLines(setupDraft.source_boundaries),
        values: splitLines(setupDraft.values),
        tastes: splitLines(setupDraft.tastes),
        constraints: splitLines(setupDraft.constraints),
        somatic_cues: splitLines(setupDraft.somatic_cues),
        action_tendencies: splitLines(setupDraft.action_tendencies)
      })
      await loadWorkspace()
      showMessage('success', 'Saved setup', 2000)
    } catch (err) {
      showMessage('error', err.message || 'Failed to save setup')
    } finally {
      savingSetup.value = false
    }
  }

  async function saveDecisionMirrorConfig() {
    savingConfig.value = true
    try {
      const update = {
        preset: configDraft.preset,
        advanced_enabled: configDraft.advanced_enabled
      }
      if (configDraft.advanced_enabled) {
        update.weights = { ...configDraft.weights }
      }
      const config = await twinApi.updateDecisionMirrorConfig(update)
      loadConfigDraft(config)
      showMessage('success', 'Saved Decision Mirror config', 2000)
    } catch (err) {
      showMessage('error', err.message || 'Failed to save Decision Mirror config')
    } finally {
      savingConfig.value = false
    }
  }

  async function resetDecisionMirrorConfig() {
    savingConfig.value = true
    try {
      const config = await twinApi.resetDecisionMirrorConfig()
      loadConfigDraft(config)
      showMessage('success', 'Reset Decision Mirror config', 2000)
    } catch (err) {
      showMessage('error', err.message || 'Failed to reset Decision Mirror config')
    } finally {
      savingConfig.value = false
    }
  }

  async function exportDecisionBenchmark() {
    exportingBenchmark.value = true
    try {
      const bundle = await twinApi.exportData({ bundle_name: 'decision-mirror-benchmark' })
      showMessage(
        'success',
        `Exported benchmark: ${bundle.decision_mirror_benchmark?.count || 0} decisions`,
        3500
      )
    } catch (err) {
      showMessage('error', err.message || 'Failed to export benchmark')
    } finally {
      exportingBenchmark.value = false
    }
  }

  function dismissTutorial() {
    localStorage.setItem(TUTORIAL_STORAGE_KEY, 'true')
    showTutorialIntro.value = false
  }

  function loadSetupDraft(setup) {
    setupDraft.twin_name = setup?.twin_name || ''
    setupDraft.twin_role = setup?.twin_role || ''
    setupDraft.source_boundaries = (setup?.source_boundaries || []).join('\n')
    setupDraft.values = (setup?.values || []).join('\n')
    setupDraft.tastes = (setup?.tastes || []).join('\n')
    setupDraft.constraints = (setup?.constraints || []).join('\n')
    setupDraft.somatic_cues = (setup?.somatic_cues || []).join('\n')
    setupDraft.action_tendencies = (setup?.action_tendencies || []).join('\n')
  }

  function loadConfigDraft(config) {
    configDraft.preset = config?.preset || 'balanced'
    configDraft.advanced_enabled = Boolean(config?.advanced_enabled)
    configDraft.weights = {
      ...defaultDecisionMirrorWeights(),
      ...(config?.weights || {})
    }
  }

  function showMessage(type, text, duration = 3500) {
    message.value = { type, text }
    setTimeout(() => {
      if (message.value?.text === text) message.value = null
    }, duration)
  }

  return {
    // State
    activeTab,
    reviewRecords,
    memoryDigestItems,
    constitutionItems,
    actionGaps,
    decisions,
    selectedRecordState,
    selectedRecordId,
    selectedEvidence,
    evidenceLoading,
    runningTwinInference,
    runningConstitutionInference,
    savingSetup,
    savingConfig,
    exportingBenchmark,
    message,
    showTutorialIntro,
    setupDraft,
    configDraft,
    // Getters
    activeConstitutionCount,
    activeActionGapCount,
    pendingReviewCount,
    pendingOutcomeCount,
    healthSummary,
    recentDecisions,
    topActionGaps,
    filteredReviewRecords,
    groupedConstitution,
    // Actions
    loadWorkspace,
    runTwinInference,
    runConstitutionInference,
    reviewConstitutionItem,
    reviewActionGap,
    reviewMemoryDigestItem,
    setPromotion,
    openEvidence,
    updateDecisionOutcome,
    saveSetup,
    saveDecisionMirrorConfig,
    resetDecisionMirrorConfig,
    exportDecisionBenchmark,
    dismissTutorial,
    showMessage
  }
})
