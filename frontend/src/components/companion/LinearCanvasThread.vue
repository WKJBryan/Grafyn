<template>
  <section
    class="linear-thread"
    aria-label="Canvas thread"
  >
    <p
      v-if="entries.length === 0"
      class="empty-thread"
    >
      Start with one question. Each answer stays in this chronological thread.
    </p>

    <article
      v-for="entry in entries"
      :key="entry.tile.id"
      class="thread-turn"
      :data-tile-id="entry.tile.id"
    >
      <div class="prompt-bubble">
        <span>You</span>
        <p>{{ entry.tile.prompt }}</p>
      </div>

      <ModelResponseCard
        v-if="entry.response"
        :response="entry.response"
        :is-streaming="isStreaming(entry.tile.id, entry.modelId)"
      />
      <p
        v-else
        class="missing-response"
      >
        The persisted response is unavailable.
      </p>

      <p
        v-if="hasRelationshipContext(entry.tile)"
        class="relationship-context"
        :data-relationship-context="entry.tile.id"
      >
        Context: {{ relationshipContextLabel(entry.tile) }}
      </p>

      <p
        v-if="isSimulationTurn(entry.tile)"
        class="simulation-disclosure"
        role="note"
        :data-simulation-disclosure="entry.tile.id"
      >
        This answer is a configured simulation built from reviewed evidence. It is not you and may be wrong.
      </p>

      <details
        v-if="entry.tile.twin_evidence_snapshot"
        class="evidence-snapshot"
      >
        <summary>Evidence snapshot</summary>
        <dl>
          <div>
            <dt>Projection</dt>
            <dd>{{ shortId(entry.tile.twin_evidence_snapshot.projection_snapshot_id) }}</dd>
          </div>
          <div>
            <dt>Captured</dt>
            <dd>{{ formatTime(entry.tile.twin_evidence_snapshot.reference_time) }}</dd>
          </div>
          <div>
            <dt>Evidence</dt>
            <dd>
              {{ entry.tile.twin_evidence_snapshot.evidence_event_ids?.length || 0 }} events ·
              {{ entry.tile.twin_evidence_snapshot.note_ids?.length || 0 }} note{{ entry.tile.twin_evidence_snapshot.note_ids?.length === 1 ? '' : 's' }}
            </dd>
          </div>
        </dl>
      </details>

      <div
        v-if="entry.response"
        class="turn-actions"
      >
        <button
          type="button"
          :aria-label="`Follow up on ${entry.tile.id} ${entry.modelId}`"
          :disabled="!canUseResponse(entry)"
          @click="$emit('follow-up', responseRef(entry))"
        >
          Follow up
        </button>
        <button
          type="button"
          :aria-label="`Regenerate ${entry.tile.id} ${entry.modelId}`"
          :disabled="!canRegenerate(entry)"
          @click="$emit('regenerate', responseRef(entry))"
        >
          Regenerate
        </button>
        <button
          type="button"
          :aria-label="`Accept ${entry.tile.id} ${entry.modelId}`"
          :disabled="!canUseResponse(entry) || feedbackIsPending(entry)"
          @click="$emit('feedback', { ...responseRef(entry), feedbackType: 'accept' })"
        >
          Useful
        </button>
        <button
          type="button"
          :aria-label="`Reject ${entry.tile.id} ${entry.modelId}`"
          :disabled="!canUseResponse(entry) || feedbackIsPending(entry)"
          @click="$emit('feedback', { ...responseRef(entry), feedbackType: 'reject' })"
        >
          Not useful
        </button>
        <button
          v-if="canCapturePreference(entry)"
          type="button"
          class="capture-preference-action"
          :aria-label="`Capture Twin preference from ${entry.tile.id} ${entry.modelId}`"
          :disabled="feedbackIsPending(entry) || captureIsPending(entry)"
          @click="$emit('capture-preference', preferenceResponseRef(entry))"
        >
          Capture Twin preference
        </button>
      </div>
    </article>
  </section>
</template>

<script setup>
import { computed } from 'vue'
import ModelResponseCard from '@/components/canvas/ModelResponseCard.vue'
import { relationshipVariantLabel } from '@/utils/twinFormat'

const props = defineProps({
  tiles: { type: Array, default: () => [] },
  streamingModels: { type: Object, default: () => new Set() },
  feedbackInFlight: { type: Object, default: () => new Set() },
  captureInFlight: { type: Object, default: () => new Set() },
  allowPreferenceCapture: { type: Boolean, default: false },
})

defineEmits(['follow-up', 'regenerate', 'feedback', 'capture-preference'])

const entries = computed(() => [...props.tiles]
  .sort((left, right) => {
    const time = String(left.created_at || '').localeCompare(String(right.created_at || ''))
    return time || String(left.id).localeCompare(String(right.id))
  })
  .map(tile => {
    const responseIds = Object.keys(tile.responses || {}).sort()
    const modelId = (tile.models || []).find(id => tile.responses?.[id]) || responseIds[0] || ''
    return { tile, modelId, response: tile.responses?.[modelId] || null }
  }))

function isStreaming(tileId, modelId) {
  return props.streamingModels?.has?.(`${tileId}:${modelId}`) || false
}

function responseRef(entry) {
  return { tileId: entry.tile.id, modelId: entry.modelId }
}

function preferenceResponseRef(entry) {
  return {
    ...responseRef(entry),
    responseId: entry.response.id,
    responseContent: entry.response.content,
  }
}

function feedbackIsPending(entry) {
  return props.feedbackInFlight?.has?.(`${entry.tile.id}:${entry.modelId}`) || false
}

function captureIsPending(entry) {
  return props.captureInFlight?.has?.(`${entry.tile.id}:${entry.modelId}`) || false
}

function responseIsStreaming(entry) {
  return isStreaming(entry.tile.id, entry.modelId)
    || ['pending', 'streaming'].includes(entry.response?.status)
}

function canUseResponse(entry) {
  return !responseIsStreaming(entry) && entry.response?.status === 'completed'
}

function canCapturePreference(entry) {
  const relationships = entry.tile?.twin_relationship_variant?.relationships
  return props.allowPreferenceCapture
    && canUseResponse(entry)
    && typeof entry.response?.id === 'string'
    && entry.response.id.trim().length > 0
    && typeof entry.response?.content === 'string'
    && entry.response.content.trim().length > 0
    && Array.isArray(relationships)
    && relationships.length === 0
}

function canRegenerate(entry) {
  return !responseIsStreaming(entry)
    && !captureIsPending(entry)
    && ['completed', 'error'].includes(entry.response?.status)
}

function isSimulationTurn(tile) {
  return ['twin', 'twin_history'].includes(tile.context_mode)
    && tile.twin_answer_mode === 'simulation'
}

function hasRelationshipContext(tile) {
  return Boolean(tile.twin_relationship_variant
    || tile.twin_evidence_snapshot?.twin_relationship_variant)
}

function relationshipContextLabel(tile) {
  const variant = tile.twin_relationship_variant
    || tile.twin_evidence_snapshot?.twin_relationship_variant
  return relationshipVariantLabel(variant, 'Global')
}

function shortId(value = '') {
  return value ? `${value.slice(0, 10)}…` : 'Unavailable'
}

function formatTime(value) {
  if (!value) return 'Unavailable'
  return new Date(value).toLocaleString()
}
</script>

<style scoped>
.linear-thread {
  display: grid;
  gap: var(--spacing-lg);
  min-width: 0;
}

.empty-thread,
.missing-response {
  margin: 0;
  padding: var(--spacing-lg);
  color: var(--text-muted);
  text-align: center;
  background: var(--bg-secondary);
  border: 1px dashed var(--border-default);
  border-radius: var(--radius-lg);
}

.thread-turn {
  display: grid;
  gap: var(--spacing-sm);
  min-width: 0;
}

.prompt-bubble {
  justify-self: end;
  max-width: min(88%, 42rem);
  padding: var(--spacing-sm) var(--spacing-md);
  color: var(--text-primary);
  background: color-mix(in srgb, var(--accent-primary) 18%, var(--bg-secondary));
  border: 1px solid color-mix(in srgb, var(--accent-primary) 40%, var(--border-default));
  border-radius: var(--radius-lg) var(--radius-lg) var(--radius-sm) var(--radius-lg);
}

.prompt-bubble span {
  color: var(--text-muted);
  font-size: 0.68rem;
  font-weight: 700;
  text-transform: uppercase;
}

.prompt-bubble p {
  margin: 0.25rem 0 0;
  white-space: pre-wrap;
}

.evidence-snapshot {
  padding: var(--spacing-sm) var(--spacing-md);
  color: var(--text-secondary);
  background: var(--bg-secondary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  font-size: 0.75rem;
}

.simulation-disclosure {
  margin: 0;
  padding: var(--spacing-sm) var(--spacing-md);
  color: var(--accent-yellow);
  background: color-mix(in srgb, var(--accent-yellow) 10%, var(--bg-secondary));
  border: 1px solid color-mix(in srgb, var(--accent-yellow) 35%, var(--border-default));
  border-radius: var(--radius-md);
  font-size: 0.78rem;
}

.relationship-context {
  margin: 0;
  color: var(--text-muted);
  font-size: 0.75rem;
}

.evidence-snapshot summary {
  min-height: 32px;
  display: flex;
  align-items: center;
  cursor: pointer;
  color: var(--accent-cyan);
  font-weight: 700;
}

.evidence-snapshot dl,
.evidence-snapshot div {
  display: grid;
  gap: 0.2rem;
}

.evidence-snapshot dl {
  grid-template-columns: repeat(3, minmax(0, 1fr));
  margin: var(--spacing-sm) 0 0;
}

.evidence-snapshot dt {
  color: var(--text-muted);
}

.evidence-snapshot dd {
  margin: 0;
  overflow-wrap: anywhere;
}

.turn-actions {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--spacing-xs);
}

.turn-actions button {
  min-height: 44px;
  padding: var(--spacing-xs);
  color: var(--text-secondary);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  font-size: 0.72rem;
}

.turn-actions button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.turn-actions .capture-preference-action {
  grid-column: 1 / -1;
  color: var(--accent-cyan);
  border-color: color-mix(in srgb, var(--accent-cyan) 45%, var(--border-default));
}

@media (max-width: 420px) {
  .evidence-snapshot dl {
    grid-template-columns: 1fr;
  }
}
</style>
