<template>
  <section
    class="twin-chat"
    aria-labelledby="twin-chat-title"
  >
    <header>
      <div>
        <span class="eyebrow">Conversation</span>
        <h2 id="twin-chat-title">
          Twin chat
        </h2>
      </div>
      <select
        v-model="answerMode"
        aria-label="Twin answer mode"
        :disabled="sending"
      >
        <option value="advisor">
          Advisor
        </option>
        <option
          value="simulation"
          :disabled="!identityReady"
        >
          Simulation
        </option>
      </select>
    </header>

    <p
      class="chat-context"
      data-chat-context
    >
      {{ chatContextLabel }}
    </p>

    <p
      v-if="!identityReady"
      class="identity-hint"
    >
      Add a Twin name and role in desktop setup before using Simulation.
    </p>
    <p
      v-if="answerMode === 'simulation'"
      class="simulation-disclosure"
      role="note"
    >
      This is a configured simulation built from reviewed evidence. It is not you and may be wrong.
    </p>

    <LinearCanvasThread
      :tiles="chatTiles"
      :streaming-models="canvasStore.streamingModels"
      :feedback-in-flight="currentFeedbackInFlight"
      @follow-up="replyTarget = $event"
      @regenerate="regenerate"
      @feedback="recordFeedback"
    />

    <form
      class="chat-composer"
      @submit.prevent="send"
    >
      <div
        v-if="replyTarget"
        class="reply-chip"
      >
        <span>Following {{ replyTarget.modelId }}</span>
        <button
          type="button"
          @click="replyTarget = null"
        >
          Use latest
        </button>
      </div>
      <textarea
        v-model="message"
        aria-label="Twin message"
        placeholder="Ask your Twin Advisor…"
        rows="3"
        @keydown.ctrl.enter="send"
      />
      <div class="chat-options">
        <select
          v-model="modelId"
          aria-label="Twin model"
          :disabled="sending"
        >
          <option
            v-for="model in canvasStore.availableModels"
            :key="model.id"
            :value="model.id"
          >
            {{ model.name || model.id }}
          </option>
        </select>
        <button
          type="submit"
          :disabled="sending || !message.trim() || !selectedModel"
        >
          {{ sending ? 'Sending…' : 'Send' }}
        </button>
      </div>
    </form>

    <p
      v-if="displayError"
      class="chat-error"
      role="alert"
    >
      {{ displayError }}
    </p>
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useCanvasStore } from '@/stores/canvas'
import { useTwinStore } from '@/stores/twin'
import {
  normalizeRelationshipVariant,
  relationshipVariantKey,
  relationshipVariantLabel,
} from '@/utils/twinFormat'
import LinearCanvasThread from './LinearCanvasThread.vue'

const CHAT_TAG = 'companion-twin-chat'

const props = defineProps({
  relationshipVariant: {
    type: Object,
    default: () => ({ relationships: [] }),
  },
})

const canvasStore = useCanvasStore()
const twinStore = useTwinStore()
const answerMode = ref('advisor')
const message = ref('')
const modelId = ref('')
const replyTarget = ref(null)
const sending = ref(false)
const error = ref('')
let componentActive = true
let loadingOwnedSession = false
let creatingOwnedSession = false
let messageRevision = 0
let initializationPromise = Promise.resolve()
let sessionDiscoveryComplete = false
let sessionDiscoveryError = ''

const identityReady = computed(() => Boolean(
  twinStore.setupDraft.twin_name.trim() && twinStore.setupDraft.twin_role.trim(),
))

const ownsCurrentSession = computed(() => (
  canvasStore.currentSession?.tags?.includes(CHAT_TAG) === true
))

const selectedRelationshipVariant = computed(() => normalizeRelationshipVariant(props.relationshipVariant))
const selectedRelationshipKey = computed(() => relationshipVariantKey(selectedRelationshipVariant.value))
const chatContextLabel = computed(() => {
  const relationships = selectedRelationshipVariant.value.relationships
  if (relationships.length === 0) {
    return 'Global context — relationship-specific memory excluded'
  }
  return `Relationship context — ${relationshipVariantLabel(selectedRelationshipVariant.value)}`
})
const chatTiles = computed(() => (
  ownsCurrentSession.value
    ? tilesForRelationshipVariant(selectedRelationshipVariant.value)
    : []
))
const currentFeedbackInFlight = computed(() => {
  const sessionId = ownsCurrentSession.value ? canvasStore.currentSession.id : null
  if (!sessionId) return new Set()
  const prefix = `${sessionId}:`
  return new Set([...canvasStore.feedbackInFlight]
    .filter(key => key.startsWith(prefix))
    .map(key => key.slice(prefix.length)))
})

const selectedModel = computed(() => (
  canvasStore.availableModels.find(model => model.id === modelId.value) || null
))
const displayError = computed(() => error.value || canvasStore.error || '')

watch(selectedRelationshipKey, () => {
  replyTarget.value = null
})

watch(message, () => {
  messageRevision += 1
})

function latestParentFor(relationshipVariant) {
  const tiles = tilesForRelationshipVariant(relationshipVariant).sort((left, right) => {
    const time = String(left.created_at || '').localeCompare(String(right.created_at || ''))
    return time || String(left.id).localeCompare(String(right.id))
  })
  for (let index = tiles.length - 1; index >= 0; index -= 1) {
    const tile = tiles[index]
    const model = (tile.models || []).find(id => isUsableParentResponse(tile.responses?.[id]))
      || Object.keys(tile.responses || {}).sort()
        .find(id => isUsableParentResponse(tile.responses?.[id]))
    if (model) return { tileId: tile.id, modelId: model }
  }
  return null
}

function isUsableParentResponse(response) {
  return response?.status === 'completed' && Boolean(response.content?.trim())
}

watch(() => canvasStore.availableModels, models => {
  if (!models.some(model => model.id === modelId.value)) {
    modelId.value = models[0]?.id || ''
  }
}, { immediate: true, deep: true })

watch(identityReady, ready => {
  if (!ready) answerMode.value = 'advisor'
})

onMounted(() => {
  initializationPromise = initialize()
})

onBeforeUnmount(() => {
  componentActive = false
  if (loadingOwnedSession || creatingOwnedSession || ownsCurrentSession.value) canvasStore.clearSession()
})

async function ensureSession() {
  await initializationPromise
  if (!componentActive) throw new Error('Twin chat is no longer active.')
  if (!sessionDiscoveryComplete) {
    throw new Error(sessionDiscoveryError || 'Twin chat history could not be loaded.')
  }
  if (ownsCurrentSession.value) return canvasStore.currentSession

  const existing = latestChatSession()

  if (existing) {
    return loadExistingChatSession(existing.id)
  }

  creatingOwnedSession = true
  let created
  try {
    created = await canvasStore.createSession({
      title: 'Twin Advisor',
      description: 'Companion Twin conversation',
      tags: [CHAT_TAG],
    })
  } finally {
    creatingOwnedSession = false
  }
  if (!componentActive) throw new Error('Twin chat is no longer active.')
  if (created?.id !== canvasStore.currentSession?.id || !ownsCurrentSession.value) {
    throw new Error('Twin chat history could not be loaded.')
  }
  return created
}

async function initialize() {
  try {
    await canvasStore.loadSessions()
    if (canvasStore.error) throw new Error(canvasStore.error)
    sessionDiscoveryComplete = true
    await canvasStore.loadModels()
    if (!componentActive || ownsCurrentSession.value) return
    const existing = latestChatSession()
    if (existing) await loadExistingChatSession(existing.id)
  } catch (err) {
    sessionDiscoveryError = err?.message || String(err)
    if (componentActive) error.value = sessionDiscoveryError
  }
}

function latestChatSession() {
  return [...canvasStore.sessions]
    .filter(session => session.tags?.includes(CHAT_TAG))
    .sort((left, right) => {
      const time = String(right.updated_at || '').localeCompare(String(left.updated_at || ''))
      return time || String(left.id).localeCompare(String(right.id))
    })[0]
}

async function loadExistingChatSession(id) {
  loadingOwnedSession = true
  try {
    await canvasStore.loadSession(id)
  } finally {
    loadingOwnedSession = false
  }
  if (!componentActive) return null
  if (canvasStore.currentSession?.id !== id || !ownsCurrentSession.value) {
    throw new Error('Twin chat history could not be loaded.')
  }
  return canvasStore.currentSession
}

async function send() {
  const prompt = message.value.trim()
  const model = selectedModel.value
  const submittedParent = replyTarget.value ? { ...replyTarget.value } : null
  const submittedAnswerMode = answerMode.value
  const submittedRelationshipVariant = normalizeRelationshipVariant(props.relationshipVariant)
  const submittedRelationshipKey = relationshipVariantKey(submittedRelationshipVariant)
  const submittedMessageRevision = messageRevision
  if (!prompt || !model || sending.value) return
  if (submittedAnswerMode === 'simulation' && !identityReady.value) return

  sending.value = true
  error.value = ''
  try {
    const submittedSession = await ensureSession()
    const parent = submittedParent || latestParentFor(submittedRelationshipVariant)
    const tileId = await canvasStore.sendCompanionPrompt({
      prompt,
      modelId: model.id,
      provider: runtimeProvider(model),
      mode: 'twin',
      answerMode: submittedAnswerMode,
      relationshipVariant: submittedRelationshipVariant,
      parentTileId: parent?.tileId || null,
      parentModelId: parent?.modelId || null,
    })
    const stillOwnsSubmittedSession = componentActive
      && ownsCurrentSession.value
      && canvasStore.currentSession?.id === submittedSession.id
    if (stillOwnsSubmittedSession && selectedRelationshipKey.value === submittedRelationshipKey) {
      replyTarget.value = { tileId, modelId: model.id }
    }
    if (stillOwnsSubmittedSession && messageRevision === submittedMessageRevision) message.value = ''
  } catch (err) {
    error.value = err?.message || String(err)
  } finally {
    sending.value = false
  }
}

function tilesForRelationshipVariant(relationshipVariant) {
  if (!ownsCurrentSession.value) return []
  const expectedKey = relationshipVariantKey(relationshipVariant)
  return (canvasStore.currentSession.prompt_tiles || [])
    .filter(tile => {
      const turnVariant = turnRelationshipVariant(tile)
      return turnVariant && relationshipVariantKey(turnVariant) === expectedKey
    })
}

function turnRelationshipVariant(tile) {
  const tileVariant = tile.twin_relationship_variant
  const snapshotVariant = tile.twin_evidence_snapshot?.twin_relationship_variant
  if (
    tileVariant
    && snapshotVariant
    && relationshipVariantKey(tileVariant) !== relationshipVariantKey(snapshotVariant)
  ) {
    return null
  }
  return tileVariant || snapshotVariant || { relationships: [] }
}

function runtimeProvider(model) {
  return model.provider?.toLowerCase() === 'ollama' ? 'ollama' : 'openrouter'
}

async function regenerate({ tileId, modelId: responseModelId }) {
  error.value = ''
  try {
    await canvasStore.regenerateResponse(tileId, responseModelId)
  } catch (err) {
    error.value = err?.message || String(err)
  }
}

async function recordFeedback({ tileId, modelId: responseModelId, feedbackType }) {
  error.value = ''
  try {
    await canvasStore.recordPreferenceFeedback(tileId, responseModelId, feedbackType)
  } catch (err) {
    error.value = err?.message || String(err)
  }
}
</script>

<style scoped>
.twin-chat {
  display: grid;
  gap: var(--spacing-md);
}

header,
.chat-options,
.reply-chip {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
}

header,
.reply-chip {
  justify-content: space-between;
}

.eyebrow {
  color: var(--text-muted);
  font-size: 0.72rem;
}

h2 {
  margin: 0;
  font-size: 1.15rem;
}

select,
button,
textarea {
  min-height: 44px;
  box-sizing: border-box;
  color: var(--text-primary);
  border-radius: var(--radius-md);
}

select,
textarea {
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-default);
}

textarea {
  width: 100%;
  resize: vertical;
}

.identity-hint,
.chat-context,
.simulation-disclosure,
.chat-error {
  margin: 0;
  padding: var(--spacing-sm) var(--spacing-md);
  border-radius: var(--radius-md);
  font-size: 0.78rem;
}

.identity-hint {
  color: var(--text-muted);
  background: var(--bg-secondary);
}

.chat-context {
  color: var(--accent-cyan);
  background: color-mix(in srgb, var(--accent-cyan) 8%, var(--bg-secondary));
}

.simulation-disclosure {
  color: var(--accent-yellow);
  background: color-mix(in srgb, var(--accent-yellow) 10%, var(--bg-secondary));
  border: 1px solid color-mix(in srgb, var(--accent-yellow) 35%, var(--border-default));
}

.chat-composer {
  position: sticky;
  bottom: 0;
  display: grid;
  gap: var(--spacing-sm);
  padding: var(--spacing-md);
  background: color-mix(in srgb, var(--bg-secondary) 94%, transparent);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  backdrop-filter: blur(12px);
}

.chat-options select {
  flex: 1;
  min-width: 0;
}

.chat-options button {
  padding: 0 var(--spacing-lg);
  color: var(--bg-primary);
  background: var(--accent-cyan);
  border: 1px solid transparent;
  font-weight: 800;
}

.reply-chip {
  color: var(--text-secondary);
  font-size: 0.75rem;
}

.reply-chip button {
  padding: 0 var(--spacing-sm);
  color: var(--text-secondary);
  background: transparent;
  border: 1px solid var(--border-default);
}

.chat-error {
  color: var(--accent-red);
  background: color-mix(in srgb, var(--accent-red) 10%, var(--bg-secondary));
}
</style>
