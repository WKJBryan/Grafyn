<template>
  <main class="canvas-companion">
    <header class="page-header">
      <div>
        <span class="eyebrow">Linear thread</span>
        <h1>{{ currentCanvasSession?.title || 'Canvas' }}</h1>
      </div>
      <button
        type="button"
        aria-label="Open Canvas sessions"
        @click="showSessions = !showSessions"
      >
        Threads
      </button>
    </header>

    <CanvasSessionSheet
      v-if="showSessions"
      :sessions="canvasSessions"
      :current-session-id="currentCanvasSession?.id || null"
      :loading="canvasStore.loading"
      @create="createSession"
      @select="selectSession"
      @rename="renameSession"
      @delete="requestDeleteSession"
    />

    <p
      v-if="displayError"
      class="canvas-error"
      role="alert"
    >
      {{ displayError }}
    </p>

    <LinearCanvasThread
      :tiles="currentCanvasSession ? canvasStore.promptTiles : []"
      :streaming-models="canvasStore.streamingModels"
      :feedback-in-flight="currentFeedbackInFlight"
      @follow-up="replyTarget = $event"
      @regenerate="regenerate"
      @feedback="recordFeedback"
    />

    <CanvasComposer
      :models="canvasStore.availableModels"
      :parent="replyTarget"
      :busy="currentSessionSending || loadingRouteSession"
      :draft="currentDraft"
      @submit="sendPrompt"
      @clear-parent="replyTarget = null"
      @update:draft="currentDraft = $event"
    />

    <ConfirmDialog
      :visible="Boolean(pendingDeleteSessionId)"
      title="Delete Canvas thread"
      message="Delete this Canvas thread? This cannot be undone."
      confirm-label="Delete"
      cancel-label="Cancel"
      variant="danger"
      @confirm="confirmDeleteSession"
      @cancel="pendingDeleteSessionId = null"
    />
  </main>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useCanvasStore } from '@/stores/canvas'
import CanvasComposer from '@/components/companion/CanvasComposer.vue'
import CanvasSessionSheet from '@/components/companion/CanvasSessionSheet.vue'
import LinearCanvasThread from '@/components/companion/LinearCanvasThread.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'

const TWIN_CHAT_TAG = 'companion-twin-chat'

const canvasStore = useCanvasStore()
const route = useRoute()
const router = useRouter()
const showSessions = ref(false)
const replyTarget = ref(null)
const loadingRouteSession = ref(false)
const pendingDeleteSessionId = ref(null)
const localError = ref('')
const draftsBySession = reactive(new Map())
const errorsBySession = reactive(new Map())
const sendingBySession = reactive(new Set())
let routeSessionLoad = null
let routeSessionTail = Promise.resolve()
let componentActive = true

const draftOwnerKey = computed(() => {
  const routedSessionId = typeof route.params.id === 'string' ? route.params.id : null
  return routedSessionId || currentCanvasSession.value?.id || 'new-canvas-session'
})
const currentDraft = computed({
  get: () => draftsBySession.get(draftOwnerKey.value) || '',
  set: value => draftsBySession.set(draftOwnerKey.value, value),
})
const currentSessionSending = computed(() => sendingBySession.has(draftOwnerKey.value))
const currentStoreError = computed(() => (
  !canvasStore.errorSessionId || canvasStore.errorSessionId === draftOwnerKey.value
    ? canvasStore.error
    : ''
))
const displayError = computed(() => (
  localError.value
  || errorsBySession.get(draftOwnerKey.value)
  || currentStoreError.value
  || ''
))
const canvasSessions = computed(() => canvasStore.sessions.filter(session => !isTwinChatSession(session)))
const currentFeedbackInFlight = computed(() => feedbackKeysForSession(currentCanvasSession.value?.id))
const currentCanvasSession = computed(() => {
  const session = canvasStore.currentSession
  if (!session || isTwinChatSession(session)) return null
  const routedSessionId = typeof route.params.id === 'string' ? route.params.id : null
  return !routedSessionId || session.id === routedSessionId ? session : null
})

watch(() => route.params.id, async (id, previousId) => {
  replyTarget.value = null
  if (previousId !== undefined && id !== previousId) {
    localError.value = ''
    canvasStore.clearError()
  }
  if (id) {
    try {
      const session = await requireRouteSession(id)
      if (route.params.id === id && session?.id === id) localError.value = ''
    } catch (error) {
      if (route.params.id === id) localError.value = error?.message || String(error)
    }
  }
}, { immediate: true })

onMounted(async () => {
  await Promise.all([canvasStore.loadSessions(), canvasStore.loadModels()])
})

onBeforeUnmount(() => {
  componentActive = false
  canvasStore.clearSession()
})

async function createSession() {
  const ownerKey = draftOwnerKey.value
  localError.value = ''
  errorsBySession.delete(ownerKey)
  try {
    const created = await canvasStore.createSession({
      title: `Canvas ${new Date().toLocaleDateString()}`,
      description: null,
    })
    if (!componentActive || !created) return
    showSessions.value = false
    await router.push(`/canvas/${created.id}`)
  } catch (error) {
    errorsBySession.set(ownerKey, error?.message || String(error))
  }
}

async function selectSession(id) {
  showSessions.value = false
  await router.push(`/canvas/${id}`)
}

async function renameSession({ id, title }) {
  localError.value = ''
  errorsBySession.delete(id)
  try {
    await canvasStore.updateSession(id, { title })
  } catch (error) {
    errorsBySession.set(id, error?.message || String(error))
  }
}

function requestDeleteSession(id) {
  pendingDeleteSessionId.value = id
}

async function confirmDeleteSession() {
  const id = pendingDeleteSessionId.value
  if (!id) return
  pendingDeleteSessionId.value = null
  localError.value = ''
  errorsBySession.delete(id)
  try {
    await canvasStore.deleteSession(id)
    if (route.params.id === id) await router.push('/canvas')
  } catch (error) {
    errorsBySession.set(id, error?.message || String(error))
  }
}

async function ensureSession() {
  const routedSessionId = typeof route.params.id === 'string' ? route.params.id : null
  if (routedSessionId) {
    const routedSession = await requireRouteSession(routedSessionId)
    if (route.params.id !== routedSessionId || routedSession?.id !== routedSessionId) {
      throw new Error('The routed Canvas thread changed before it was ready.')
    }
    return routedSession
  }
  if (currentCanvasSession.value) {
    return currentCanvasSession.value
  }
  const created = await canvasStore.createSession({
    title: `Canvas ${new Date().toLocaleDateString()}`,
    description: null,
  })
  if (!componentActive || !created) {
    throw new Error('The Canvas thread changed before it was ready.')
  }
  moveSessionState(draftsBySession, draftOwnerKey.value, created.id)
  moveSessionState(errorsBySession, draftOwnerKey.value, created.id)
  await router.push(`/canvas/${created.id}`)
  return created
}

async function requireRouteSession(id) {
  if (canvasStore.currentSession?.id === id && !routeSessionLoad) {
    if (isTwinChatSession(canvasStore.currentSession)) {
      return rejectTwinChatRoute(id)
    }
    return canvasStore.currentSession
  }
  if (routeSessionLoad?.id === id) return routeSessionLoad.promise

  const priorLoad = routeSessionTail
  const request = { id, promise: null }
  loadingRouteSession.value = true
  request.promise = priorLoad.catch(() => null).then(async () => {
    if (!componentActive || route.params.id !== id) return null
    if (canvasStore.currentSession?.id !== id) await canvasStore.loadSession(id)
    if (!componentActive || route.params.id !== id) return null
    if (canvasStore.currentSession?.id !== id) {
      throw new Error(`Canvas thread ${id} could not be loaded.`)
    }
    if (isTwinChatSession(canvasStore.currentSession)) {
      return rejectTwinChatRoute(id)
    }
    return canvasStore.currentSession
  })
  routeSessionTail = request.promise
  routeSessionLoad = request

  try {
    return await request.promise
  } finally {
    if (routeSessionLoad === request) {
      routeSessionLoad = null
      loadingRouteSession.value = false
    }
  }
}

async function rejectTwinChatRoute(id) {
  canvasStore.clearSession()
  if (componentActive && route.params.id === id) await router.replace('/canvas')
  return null
}

function isTwinChatSession(session) {
  return session?.tags?.includes(TWIN_CHAT_TAG) === true
}

function moveSessionState(state, from, to) {
  if (from === to || !state.has(from)) return
  if (!state.has(to)) state.set(to, state.get(from))
  state.delete(from)
}

function feedbackKeysForSession(sessionId) {
  if (!sessionId) return new Set()
  const prefix = `${sessionId}:`
  return new Set([...canvasStore.feedbackInFlight]
    .filter(key => key.startsWith(prefix))
    .map(key => key.slice(prefix.length)))
}

async function sendPrompt(request) {
  const initialOwnerKey = draftOwnerKey.value
  if (sendingBySession.has(initialOwnerKey)) return
  const submittedDraft = draftsBySession.get(initialOwnerKey) || ''
  let submittedSession = null
  let submittedOwnerKey = initialOwnerKey
  sendingBySession.add(submittedOwnerKey)
  localError.value = ''
  errorsBySession.delete(initialOwnerKey)
  try {
    submittedSession = await ensureSession()
    if (submittedOwnerKey !== submittedSession.id) {
      sendingBySession.delete(submittedOwnerKey)
      submittedOwnerKey = submittedSession.id
      sendingBySession.add(submittedOwnerKey)
    }
    moveSessionState(draftsBySession, initialOwnerKey, submittedOwnerKey)
    moveSessionState(errorsBySession, initialOwnerKey, submittedOwnerKey)
    errorsBySession.delete(submittedOwnerKey)
    const tileId = await canvasStore.sendCompanionPrompt(request)
    if (draftsBySession.get(submittedOwnerKey) === submittedDraft) {
      draftsBySession.set(submittedOwnerKey, '')
    }
    if (!componentActive || currentCanvasSession.value?.id !== submittedSession.id) return
    replyTarget.value = { tileId, modelId: request.modelId }
  } catch (error) {
    const ownerKey = submittedSession?.id || initialOwnerKey
    errorsBySession.set(ownerKey, error?.message || String(error))
  } finally {
    sendingBySession.delete(submittedOwnerKey)
  }
}

async function regenerate({ tileId, modelId }) {
  const ownerKey = draftOwnerKey.value
  localError.value = ''
  errorsBySession.delete(ownerKey)
  try {
    await canvasStore.regenerateResponse(tileId, modelId)
  } catch (error) {
    errorsBySession.set(ownerKey, error?.message || String(error))
  }
}

async function recordFeedback({ tileId, modelId, feedbackType }) {
  const ownerKey = draftOwnerKey.value
  localError.value = ''
  errorsBySession.delete(ownerKey)
  try {
    await canvasStore.recordPreferenceFeedback(tileId, modelId, feedbackType)
  } catch (error) {
    errorsBySession.set(ownerKey, error?.message || String(error))
  }
}
</script>

<style scoped>
.canvas-companion {
  width: 100%;
  max-width: 50rem;
  min-width: 0;
  margin: 0 auto;
  padding: max(var(--spacing-md), env(safe-area-inset-top)) max(var(--spacing-md), env(safe-area-inset-right)) var(--spacing-xl) max(var(--spacing-md), env(safe-area-inset-left));
  box-sizing: border-box;
  display: grid;
  gap: var(--spacing-md);
  overflow-x: hidden;
}

.page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-md);
}

.eyebrow {
  color: var(--text-muted);
  font-size: 0.72rem;
  text-transform: uppercase;
}

h1 {
  max-width: min(70vw, 34rem);
  margin: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 1.5rem;
}

.page-header button {
  min-height: 44px;
  padding: 0 var(--spacing-md);
  color: var(--text-primary);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
}

.canvas-error {
  margin: 0;
  padding: var(--spacing-sm) var(--spacing-md);
  color: var(--accent-red);
  background: color-mix(in srgb, var(--accent-red) 10%, var(--bg-secondary));
  border-radius: var(--radius-md);
}
</style>
