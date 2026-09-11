<template>
  <main class="twin-companion">
    <header class="page-header">
      <div>
        <span class="eyebrow">Governed memory</span>
        <h1>Twin</h1>
      </div>
      <button
        type="button"
        aria-label="Refresh Twin companion"
        :disabled="controlsLocked"
        @click="refresh"
      >
        {{ refreshing ? 'Refreshing…' : 'Refresh' }}
      </button>
    </header>

    <label class="relationship-filter">
      <span>Relationship context</span>
      <select
        v-model="selectedRelationshipKey"
        aria-label="Twin relationship filter"
        :disabled="controlsLocked"
        @change="refresh"
      >
        <option value="">All relationships</option>
        <option
          v-for="option in relationshipOptions"
          :key="option.key"
          :value="option.key"
        >
          {{ option.label }}
        </option>
      </select>
    </label>

    <nav
      class="section-tabs"
      aria-label="Twin sections"
    >
      <button
        type="button"
        :aria-current="activeSection === 'review' ? 'page' : undefined"
        aria-label="Open Twin review"
        @click="openTwinReview"
      >
        Review <span>{{ twinStore.proposals.length + twinStore.memoryDigestItems.length }}</span>
      </button>
      <button
        v-if="twinChatAvailable"
        type="button"
        :aria-current="activeSection === 'chat' ? 'page' : undefined"
        aria-label="Open Twin chat"
        @click="openTwinChat"
      >
        Chat
      </button>
      <button
        type="button"
        :aria-current="activeSection === 'memory' ? 'page' : undefined"
        aria-label="Open Twin memory"
        @click="activeSection = 'memory'"
      >
        Memory
      </button>
    </nav>

    <p
      v-if="viewError"
      class="view-error"
      role="alert"
    >
      {{ viewError }}
    </p>
    <p
      v-else-if="viewNotice"
      class="view-notice"
      role="status"
    >
      {{ viewNotice }}
    </p>

    <TwinReviewQueue
      v-if="activeSection === 'review'"
      :proposals="twinStore.proposals"
      :digest-items="twinStore.memoryDigestItems"
      :attention-trace="refreshing ? null : twinStore.attention?.trace || null"
      :loading="!twinReviewAvailable || !reviewSnapshotReady || refreshing || twinStore.twinStateLoading.proposals || twinStore.twinStateLoading.review || twinStore.twinStateLoading.attention || digestReviewing"
      @review-proposal="reviewProposal"
      @review-digest="reviewDigest"
    />

    <TwinChat
      v-else-if="activeSection === 'chat' && twinChatAvailable"
      :relationship-variant="selectedRelationshipVariant"
    />

    <section
      v-else
      class="memory-state"
      aria-labelledby="reviewed-memory-title"
    >
      <header>
        <div>
          <span class="eyebrow">B-memory</span>
          <h2 id="reviewed-memory-title">
            Reviewed memory
          </h2>
        </div>
        <span>{{ reviewedMemories.length }}</span>
      </header>
      <p
        v-if="reviewedMemories.length === 0"
        class="empty-copy"
      >
        Accepted proposals will appear here as reviewed memory.
      </p>
      <article
        v-for="item in reviewedMemories"
        :key="item.item_id"
        class="memory-card"
      >
        <p>{{ claimLabel(item.claim) }}</p>
        <small
          v-if="relationshipVariantLabel(item)"
          class="memory-context"
        >
          Context: {{ relationshipVariantLabel(item) }}
        </small>
        <small>{{ item.support_count || 0 }} supporting observations</small>
      </article>

      <header class="timeline-heading">
        <div>
          <span class="eyebrow">Temporal field</span>
          <h2>State timeline</h2>
        </div>
      </header>
      <ol class="timeline-list">
        <li
          v-for="entry in twinStore.timeline"
          :key="timelineKey(entry)"
        >
          <strong>{{ stateLabel(entry.state) }}</strong>
          <span>{{ entry.item_id }}</span>
          <small class="timeline-context">
            Context: {{ relationshipVariantLabel(entry) || 'Global' }}
          </small>
          <time>{{ formatTime(entry.effective_at) }}</time>
        </li>
      </ol>
    </section>
  </main>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { getRuntimeProfile } from '@/api/transport'
import { hasCapability } from '@/platform/capabilities'
import { useTwinStore } from '@/stores/twin'
import {
  normalizeRelationshipVariant,
  relationshipVariantKey,
  relationshipVariantLabel,
  relationshipVariantRequest,
} from '@/utils/twinFormat'
import TwinReviewQueue from '@/components/companion/TwinReviewQueue.vue'
import TwinChat from '@/components/companion/TwinChat.vue'

const twinStore = useTwinStore()
const activeSection = ref('review')
const selectedRelationshipKey = ref('')
const refreshing = ref(false)
const reviewSnapshotReady = ref(false)
const digestReviewing = ref(false)
const localError = ref('')
const runtimeProfile = getRuntimeProfile()
const twinReviewAvailable = computed(() => hasCapability(runtimeProfile, 'twinReview'))
const twinChatAvailable = computed(() => hasCapability(runtimeProfile, 'twinChat'))
const controlsLocked = computed(() => !twinReviewAvailable.value
  || refreshing.value
  || digestReviewing.value
  || twinStore.twinStateLoading.review
  || twinStore.twinStateLoading.attention)

watch(twinChatAvailable, available => {
  if (!available && activeSection.value === 'chat') activeSection.value = 'review'
})

const relationshipOptions = computed(() => {
  const options = new Map()
  for (const state of twinStore.projection?.relationship_variants || []) {
    const persistedVariant = normalizeRelationshipVariant(state.relationship_variant)
    if (persistedVariant.relationships.length === 0) continue
    const key = relationshipVariantKey(persistedVariant)
    options.set(key, {
      key,
      requestRelationships: relationshipVariantRequest(persistedVariant),
      persistedVariant,
      label: relationshipVariantLabel(persistedVariant),
    })
  }
  return [...options.values()].sort((left, right) => left.key.localeCompare(right.key))
})

const selectedRelationshipOption = computed(() => (
  relationshipOptions.value.find(option => option.key === selectedRelationshipKey.value) || null
))
const selectedRelationshipVariant = computed(() => (
  selectedRelationshipOption.value?.persistedVariant || { relationships: [] }
))

const currentFilter = computed(() => ({
  relationships: selectedRelationshipOption.value?.requestRelationships || [],
  goals: [],
  tags: [],
}))

const reviewedMemories = computed(() => {
  const items = twinStore.projection?.reviewed_memories || []
  if (!selectedRelationshipKey.value) return items
  return items.filter(item => relationshipVariantKey(item.relationship_variant) === selectedRelationshipKey.value)
})

const viewError = computed(() => localError.value
  || twinStore.twinStateError.review
  || twinStore.twinStateError.proposals
  || twinStore.twinStateError.projection
  || twinStore.twinStateError.timeline
  || twinStore.twinStateError.attention
  || (twinStore.message?.type === 'error' ? twinStore.message.text : '')
  || '')

const viewNotice = computed(() => (
  twinStore.message?.type !== 'error' ? twinStore.message?.text || '' : ''
))

onMounted(refresh)

onBeforeUnmount(() => {
  twinStore.invalidateTwinStateRequests()
})

function openTwinChat() {
  if (twinChatAvailable.value) activeSection.value = 'chat'
}

async function openTwinReview() {
  if (!twinReviewAvailable.value || controlsLocked.value) return
  if (await refresh()) activeSection.value = 'review'
}

async function refresh() {
  if (!twinReviewAvailable.value || controlsLocked.value) return false
  refreshing.value = true
  reviewSnapshotReady.value = false
  localError.value = ''
  twinStore.attention = null
  const referenceTime = new Date().toISOString()
  const filter = currentFilter.value
  const pageRequest = { referenceTime, filter, cursor: null, limit: 50 }
  try {
    const results = await Promise.all([
      twinStore.loadWorkspace(),
      twinStore.loadProjection({ referenceTime }),
      twinStore.loadProposals(pageRequest),
      twinStore.loadTimeline(pageRequest),
      twinStore.rankAttention({
        referenceTime,
        profile: 'capture_review',
        query: 'Review pending Twin proposals',
        relationshipVariant: { relationships: filter.relationships },
        goals: [],
        destination: 'local',
        filter,
        limit: 50,
      }),
    ])
    reviewSnapshotReady.value = results.every(result => result !== null)
    return reviewSnapshotReady.value
  } catch (error) {
    localError.value = error?.message || String(error)
    return false
  } finally {
    refreshing.value = false
  }
}

async function reviewProposal({ itemId, decision, reviewedClaim }) {
  if (!twinReviewAvailable.value) return
  const response = await twinStore.reviewProposal(itemId, decision, reviewedClaim)
  if (!response?.referenceTime) return
  const filter = currentFilter.value
  twinStore.attention = null
  await twinStore.rankAttention({
    referenceTime: response.referenceTime,
    profile: 'capture_review',
    query: 'Review pending Twin proposals',
    relationshipVariant: { relationships: filter.relationships },
    goals: [],
    destination: 'local',
    filter,
    limit: 50,
  })
}

async function reviewDigest({ id, action }) {
  if (!twinReviewAvailable.value || digestReviewing.value) return
  digestReviewing.value = true
  try {
    await twinStore.reviewMemoryDigestItem(id, action)
  } finally {
    digestReviewing.value = false
  }
}

function claimLabel(claim = {}) {
  return [claim.subject_id, claim.predicate, claim.object, claim.polarity === 'denied' ? '(denied)' : '']
    .filter(Boolean)
    .join(' ')
}

function stateLabel(state = '') {
  return state.charAt(0).toUpperCase() + state.slice(1).replaceAll('_', ' ')
}

function timelineKey(entry) {
  return `${entry.item_id}:${entry.source_event_id || ''}:${entry.state}:${entry.effective_at}`
}

function formatTime(value) {
  return value ? new Date(value).toLocaleString() : ''
}
</script>

<style scoped>
.twin-companion {
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

.page-header,
.memory-state > header,
.section-tabs {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-sm);
}

.eyebrow {
  color: var(--text-muted);
  font-size: 0.72rem;
  text-transform: uppercase;
}

h1,
h2 {
  margin: 0;
}

h1 {
  font-size: 1.7rem;
}

h2 {
  font-size: 1.1rem;
}

button,
select {
  min-height: 44px;
  box-sizing: border-box;
  color: var(--text-primary);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
}

.page-header button {
  padding: 0 var(--spacing-md);
}

.relationship-filter {
  display: grid;
  gap: 0.25rem;
}

.relationship-filter span {
  color: var(--text-muted);
  font-size: 0.72rem;
}

.relationship-filter select {
  width: 100%;
  padding: 0 var(--spacing-sm);
}

.section-tabs {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.section-tabs button {
  padding: 0 var(--spacing-xs);
}

.section-tabs button[aria-current='page'] {
  color: var(--accent-cyan);
  border-color: var(--accent-cyan);
  background: color-mix(in srgb, var(--accent-cyan) 10%, var(--bg-secondary));
}

.view-error {
  margin: 0;
  padding: var(--spacing-sm);
  color: var(--accent-red);
  background: color-mix(in srgb, var(--accent-red) 10%, var(--bg-secondary));
  border-radius: var(--radius-md);
}

.view-notice {
  margin: 0;
  padding: var(--spacing-sm);
  color: var(--text-secondary);
  background: var(--bg-secondary);
  border-radius: var(--radius-md);
}

.memory-state,
.timeline-list {
  display: grid;
  gap: var(--spacing-sm);
}

.memory-card,
.timeline-list li,
.empty-copy {
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
}

.memory-card p,
.empty-copy {
  margin: 0;
}

.memory-card small,
.timeline-list time,
.timeline-list span,
.timeline-context {
  color: var(--text-muted);
}

.memory-card .memory-context {
  display: block;
  color: var(--accent-cyan);
}

.timeline-heading {
  margin-top: var(--spacing-md);
}

.timeline-list {
  margin: 0;
  padding: 0;
  list-style: none;
}

.timeline-list li {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 0.25rem var(--spacing-sm);
}

.timeline-list span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.timeline-list time {
  grid-column: 1 / -1;
  font-size: 0.72rem;
}

.timeline-context {
  grid-column: 1 / -1;
  font-size: 0.72rem;
}
</style>
