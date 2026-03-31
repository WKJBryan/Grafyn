import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { boot as bootApi, isDesktopApp } from '@/api/client'

const BOOT_POLL_INTERVAL_MS = 2000
const BOOT_WATCHDOG_INTERVAL_MS = 1000
const BOOT_TOTAL_TIMEOUT_MS = 45000
const BOOT_NO_PROGRESS_TIMEOUT_MS = 20000

function defaultStatus() {
  return {
    phase: 'starting',
    message: 'Preparing workspace',
    ready: false,
    error: null,
  }
}

function isTerminalStatus(status) {
  return !!status?.ready || status?.phase === 'failed' || !!status?.error
}

function progressSignature(status) {
  return `${status?.phase || 'starting'}::${status?.message || 'Preparing workspace'}`
}

function describePhase(phase) {
  const labels = {
    starting: 'preparing the workspace',
    opening_store: 'loading notes from your vault',
    building_graph: 'building the graph',
    building_search_index: 'building the search index',
    building_chunk_index: 'building the chunk index',
    failed: 'starting Grafyn',
    ready: 'finishing startup'
  }

  return labels[phase] || `working through ${phase.replaceAll('_', ' ')}`
}

function createWatchdogError(phase) {
  return `Startup is taking longer than expected while ${describePhase(phase)}. You can continue to the app, but indexing may not be fully ready yet.`
}

export const useBootStore = defineStore('boot', () => {
  const phase = ref('starting')
  const message = ref('Preparing workspace')
  const ready = ref(false)
  const error = ref(null)
  const initialized = ref(false)
  const dismissed = ref(false)
  const listening = ref(false)
  const syntheticFailure = ref(false)
  const syntheticError = ref(null)
  const bootStartedAt = ref(null)
  const lastProgressAt = ref(null)
  const lastProgressSignature = ref(progressSignature(defaultStatus()))
  let unlisten = null
  let pollTimer = null
  let watchdogTimer = null
  let pollInFlight = false

  const failed = computed(() => syntheticFailure.value || phase.value === 'failed' || !!error.value)
  const status = computed(() => ({
    phase: phase.value,
    message: message.value,
    ready: ready.value,
    error: syntheticFailure.value ? syntheticError.value : error.value,
  }))
  const isVisible = computed(() => {
    if (ready.value) return false
    if (failed.value) return !dismissed.value
    return true
  })

  function clearSyntheticFailure() {
    syntheticFailure.value = false
    syntheticError.value = null
  }

  function stopMonitoring() {
    if (pollTimer) {
      clearInterval(pollTimer)
      pollTimer = null
    }

    if (watchdogTimer) {
      clearInterval(watchdogTimer)
      watchdogTimer = null
    }

    pollInFlight = false
  }

  function recordProgress(nextStatus) {
    const signature = progressSignature(nextStatus)
    const now = Date.now()

    if (bootStartedAt.value === null) {
      bootStartedAt.value = now
    }

    if (lastProgressSignature.value !== signature || lastProgressAt.value === null) {
      lastProgressSignature.value = signature
      lastProgressAt.value = now
    }
  }

  function ensureMonitoring() {
    if (pollTimer || watchdogTimer) return

    pollTimer = setInterval(() => {
      void pollBootStatus()
    }, BOOT_POLL_INTERVAL_MS)

    watchdogTimer = setInterval(() => {
      evaluateWatchdog()
    }, BOOT_WATCHDOG_INTERVAL_MS)
  }

  function applyNonTerminalStatus(nextStatus = defaultStatus()) {
    phase.value = nextStatus.phase || 'starting'
    message.value = nextStatus.message || 'Preparing workspace'
    ready.value = false
    error.value = null
    recordProgress(nextStatus)
    ensureMonitoring()

    if (!syntheticFailure.value) {
      dismissed.value = false
    }
  }

  function setStatus(nextStatus = defaultStatus()) {
    if (isTerminalStatus(nextStatus)) {
      clearSyntheticFailure()
      phase.value = nextStatus.phase || (nextStatus.ready ? 'ready' : 'failed')
      message.value = nextStatus.message || 'Preparing workspace'
      ready.value = !!nextStatus.ready
      error.value = nextStatus.error || null
      recordProgress(nextStatus)
      stopMonitoring()
      dismissed.value = false
      return
    }

    applyNonTerminalStatus(nextStatus)
  }

  function dismissSplash() {
    dismissed.value = true
  }

  async function pollBootStatus() {
    if (pollInFlight) return
    pollInFlight = true

    try {
      const current = await bootApi.status()

      if (isTerminalStatus(current)) {
        setStatus(current)
      } else {
        applyNonTerminalStatus(current)
      }
    } catch (err) {
      console.error('Failed to poll boot status:', err)
    } finally {
      pollInFlight = false
    }
  }

  function evaluateWatchdog() {
    if (ready.value || phase.value === 'failed' || error.value) {
      stopMonitoring()
      return
    }

    if (syntheticFailure.value) {
      return
    }

    const now = Date.now()
    const totalElapsed = bootStartedAt.value === null ? 0 : now - bootStartedAt.value
    const stalledElapsed = lastProgressAt.value === null ? 0 : now - lastProgressAt.value

    if (totalElapsed >= BOOT_TOTAL_TIMEOUT_MS || stalledElapsed >= BOOT_NO_PROGRESS_TIMEOUT_MS) {
      syntheticFailure.value = true
      syntheticError.value = createWatchdogError(phase.value)
      dismissed.value = false
    }
  }

  async function initialize() {
    if (initialized.value) return
    initialized.value = true

    try {
      if (isDesktopApp() && !listening.value) {
        const { listen } = await import('@tauri-apps/api/event')
        unlisten = await listen('boot-status', (event) => {
          setStatus(event.payload)
        })
        listening.value = true
      }

      const current = await bootApi.status()
      setStatus(current)
    } catch (err) {
      clearSyntheticFailure()
      stopMonitoring()
      setStatus({
        phase: 'failed',
        message: 'Startup failed',
        ready: false,
        error: err.message || 'Failed to get startup status',
      })
    }
  }

  function cleanup() {
    stopMonitoring()
    if (typeof unlisten === 'function') {
      unlisten()
    }
    unlisten = null
    listening.value = false
  }

  return {
    phase,
    message,
    ready,
    error,
    failed,
    status,
    isVisible,
    initialize,
    cleanup,
    setStatus,
    dismissSplash,
  }
})
