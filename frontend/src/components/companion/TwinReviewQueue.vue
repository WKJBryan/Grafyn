<template>
  <section
    class="review-queue"
    aria-labelledby="review-queue-title"
  >
    <header class="section-heading">
      <div>
        <span class="eyebrow">Dendritic review</span>
        <h2 id="review-queue-title">
          Review before memory
        </h2>
      </div>
      <span class="count">{{ proposals.length + digestItems.length }}</span>
    </header>

    <p
      v-if="loading"
      class="status-copy"
      role="status"
    >
      Refreshing review queue…
    </p>
    <p
      v-else-if="proposals.length === 0 && digestItems.length === 0"
      class="status-copy"
    >
      Nothing needs review right now.
    </p>

    <div class="card-list">
      <article
        v-for="item in proposals"
        :key="item.item_id"
        class="review-card"
        :data-proposal-id="item.item_id"
      >
        <div class="card-meta">
          <span>Proposal</span>
          <span>{{ item.support_count || 0 }} support</span>
          <span v-if="item.opposition_count">{{ item.opposition_count }} opposed</span>
        </div>
        <p class="claim">
          {{ claimLabel(item.claim) }}
        </p>
        <p
          v-if="attentionFor(item.item_id)"
          class="attention-reason"
        >
          Why now: {{ attentionFor(item.item_id) }}
        </p>
        <p
          v-if="relationshipLabel(item)"
          class="relationship-label"
        >
          {{ relationshipLabel(item) }}
        </p>

        <div
          v-if="editingId === item.item_id"
          class="edit-panel"
        >
          <label :for="`edit-${item.item_id}`">Edit the claim object</label>
          <textarea
            :id="`edit-${item.item_id}`"
            v-model="editedObject"
            :aria-label="`Edited claim for ${item.item_id}`"
            rows="2"
          />
          <div class="actions">
            <button
              type="button"
              class="secondary"
              :disabled="loading"
              @click="cancelEdit"
            >
              Cancel
            </button>
            <button
              type="button"
              :aria-label="`Accept edited proposal ${item.item_id}`"
              :disabled="loading || !editedObject.trim()"
              @click="acceptEdited(item)"
            >
              Accept edit
            </button>
          </div>
        </div>

        <div
          v-else
          class="actions"
        >
          <button
            type="button"
            :aria-label="`Accept proposal ${item.item_id}`"
            :disabled="loading"
            @click="review(item.item_id, 'accept')"
          >
            Accept
          </button>
          <button
            type="button"
            class="secondary"
            :aria-label="`Edit proposal ${item.item_id}`"
            :disabled="loading"
            @click="startEdit(item)"
          >
            Edit
          </button>
          <button
            type="button"
            class="danger"
            :aria-label="`Reject proposal ${item.item_id}`"
            :disabled="loading"
            @click="review(item.item_id, 'reject')"
          >
            Reject
          </button>
        </div>
      </article>

      <article
        v-for="item in digestItems"
        :key="item.id"
        class="review-card digest-card"
        :data-digest-id="item.id"
      >
        <div class="card-meta">
          <span>Adaptive digest</span>
          <span>{{ item.evidence_count || 0 }} evidence</span>
        </div>
        <p class="claim">
          {{ item.pattern }}
        </p>
        <p
          v-if="item.latest_evidence?.summary"
          class="attention-reason"
        >
          {{ item.latest_evidence.summary }}
        </p>
        <div class="actions">
          <button
            type="button"
            :aria-label="`Keep digest ${item.id}`"
            :disabled="loading"
            @click="$emit('review-digest', { id: item.id, action: 'keep' })"
          >
            Keep
          </button>
          <button
            type="button"
            class="danger"
            :aria-label="`Reject digest ${item.id}`"
            :disabled="loading"
            @click="$emit('review-digest', { id: item.id, action: 'reject' })"
          >
            Reject
          </button>
        </div>
      </article>
    </div>
  </section>
</template>

<script setup>
import { computed, ref } from 'vue'
import { relationshipVariantLabel } from '@/utils/twinFormat'

const props = defineProps({
  proposals: { type: Array, default: () => [] },
  digestItems: { type: Array, default: () => [] },
  attentionTrace: { type: Object, default: null },
  loading: { type: Boolean, default: false },
})

const emit = defineEmits(['review-proposal', 'review-digest'])
const editingId = ref(null)
const editedObject = ref('')

const attentionById = computed(() => new Map(
  (props.attentionTrace?.selected || [])
    .filter(item => item?.item_id && item.attention?.explanation)
    .map(item => [item.item_id, item.attention.explanation]),
))

function attentionFor(itemId) {
  return attentionById.value.get(itemId) || ''
}

function claimLabel(claim = {}) {
  return [claim.subject_id, claim.predicate, claim.object, claim.polarity === 'denied' ? '(denied)' : '']
    .filter(Boolean)
    .join(' ')
}

function relationshipLabel(item) {
  return relationshipVariantLabel(item)
}

function review(itemId, decision, reviewedClaim = null) {
  emit('review-proposal', { itemId, decision, reviewedClaim })
}

function startEdit(item) {
  editingId.value = item.item_id
  editedObject.value = item.claim?.object || ''
}

function cancelEdit() {
  editingId.value = null
  editedObject.value = ''
}

function acceptEdited(item) {
  const object = editedObject.value.trim()
  if (!object) return
  review(item.item_id, 'accept', { ...item.claim, object })
  cancelEdit()
}
</script>

<style scoped>
.review-queue,
.card-list {
  display: grid;
  gap: var(--spacing-md);
}

.section-heading,
.card-meta,
.actions {
  display: flex;
  align-items: center;
}

.section-heading {
  justify-content: space-between;
  gap: var(--spacing-md);
}

.eyebrow,
.card-meta,
.relationship-label {
  color: var(--text-muted);
  font-size: 0.72rem;
}

h2,
.claim,
.status-copy {
  margin: 0;
}

h2 {
  font-size: 1.15rem;
}

.count {
  min-width: 2rem;
  padding: 0.2rem 0.5rem;
  text-align: center;
  color: var(--accent-cyan);
  border: 1px solid var(--border-default);
  border-radius: 999px;
}

.review-card {
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
}

.card-meta {
  flex-wrap: wrap;
  gap: var(--spacing-sm);
}

.claim {
  margin-top: var(--spacing-sm);
  color: var(--text-primary);
  line-height: 1.5;
}

.attention-reason {
  margin: var(--spacing-sm) 0 0;
  padding-left: var(--spacing-sm);
  color: var(--text-secondary);
  border-left: 2px solid var(--accent-cyan);
  font-size: 0.82rem;
}

.relationship-label {
  margin: var(--spacing-sm) 0 0;
}

.actions {
  gap: var(--spacing-sm);
  margin-top: var(--spacing-md);
}

.actions button {
  min-height: 44px;
  padding: 0 var(--spacing-md);
  color: var(--bg-primary);
  background: var(--accent-cyan);
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  font-weight: 700;
}

.actions .secondary {
  color: var(--text-primary);
  background: transparent;
  border-color: var(--border-default);
}

.actions .danger {
  color: var(--accent-red);
  background: transparent;
  border-color: color-mix(in srgb, var(--accent-red) 50%, var(--border-default));
}

.actions button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.edit-panel {
  display: grid;
  gap: var(--spacing-xs);
  margin-top: var(--spacing-md);
}

.edit-panel label {
  color: var(--text-secondary);
  font-size: 0.78rem;
}

.edit-panel textarea {
  width: 100%;
  box-sizing: border-box;
  padding: var(--spacing-sm);
  color: var(--text-primary);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  resize: vertical;
}
</style>
