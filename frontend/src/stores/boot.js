import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { boot as bootApi, isDesktopApp } from '@/api/client'

function defaultStatus() {
  return {
    phase: 'starting',
    message: 'Preparing workspace',
    ready: false,
    error: null,
  }
}

export const useBootStore = defineStore('boot', () => {
  const phase = ref('starting')
  const message = ref('Preparing workspace')
  const ready = ref(false)
  const error = ref(null)
  const initialized = ref(false)
  const dismissed = ref(false)
  const listening = ref(false)
  let unlisten = null

  const failed = computed(() => phase.value === 'failed' || !!error.value)
  const status = computed(() => ({
    phase: phase.value,
    message: message.value,
    ready: ready.value,
    error: error.value,
  }))
  const isVisible = computed(() => {
    if (ready.value) return false
    if (failed.value) return !dismissed.value
    return true
  })

  function setStatus(nextStatus = defaultStatus()) {
    phase.value = nextStatus.phase || 'starting'
    message.value = nextStatus.message || 'Preparing workspace'
    ready.value = !!nextStatus.ready
    error.value = nextStatus.error || null

    if (!failed.value) {
      dismissed.value = false
    }
  }

  function dismissSplash() {
    dismissed.value = true
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
      setStatus({
        phase: 'failed',
        message: 'Startup failed',
        ready: false,
        error: err.message || 'Failed to get startup status',
      })
    }
  }

  function cleanup() {
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
