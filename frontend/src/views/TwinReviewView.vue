<template>
  <div class="twin-review-view">
    <header class="review-header">
      <div class="header-left">
        <router-link
          to="/"
          class="back-link"
        >
          Notes
        </router-link>
        <router-link
          to="/canvas"
          class="back-link"
        >
          Canvas
        </router-link>
      </div>
      <div class="header-title">
        <h1>Twin Review</h1>
        <span>{{ filteredRecords.length }} / {{ reviewRecords.length }} records</span>
      </div>
      <button
        class="btn btn-primary"
        :disabled="running"
        @click="runInference"
      >
        {{ running ? 'Running...' : 'Run Inference' }}
      </button>
    </header>

    <main class="review-main">
      <aside class="state-sidebar">
        <button
          v-for="state in states"
          :key="state"
          class="state-filter"
          :class="{ active: selectedState === state }"
          @click="selectedState = state"
        >
          <span>{{ stateLabel(state) }}</span>
          <strong>{{ stateCounts[state] || 0 }}</strong>
        </button>
      </aside>

      <section class="record-list">
        <div
          v-if="loading"
          class="empty-panel"
        >
          Loading records...
        </div>
        <div
          v-else-if="filteredRecords.length === 0"
          class="empty-panel"
        >
          No records in this state.
        </div>
        <article
          v-for="item in filteredRecords"
          :key="item.record.id"
          class="record-card"
        >
          <div class="record-card-header">
            <div>
              <span class="record-kind">{{ kindLabel(item.record.kind) }}</span>
              <h2>{{ item.record.content }}</h2>
            </div>
            <span class="state-pill">{{ stateLabel(item.record.promotion_state) }}</span>
          </div>

          <div class="record-meta">
            <span>Confidence {{ formatPercent(item.record.confidence) }}</span>
            <span>{{ item.evidence_count }} evidence events</span>
            <span>{{ item.record.metadata?.signal_family || item.record.origin }}</span>
          </div>

          <div
            v-if="item.latest_evidence"
            class="latest-evidence"
          >
            {{ item.latest_evidence.summary }}
          </div>

          <div class="record-actions">
            <button
              class="btn btn-secondary btn-sm"
              @click="openEvidence(item.record.id)"
            >
              Evidence
            </button>
            <button
              class="btn btn-secondary btn-sm"
              @click="setPromotion(item.record.id, 'endorsed')"
            >
              Endorse
            </button>
            <button
              class="btn btn-secondary btn-sm"
              @click="setPromotion(item.record.id, 'candidate')"
            >
              Candidate
            </button>
            <button
              class="btn btn-secondary btn-sm"
              @click="setPromotion(item.record.id, 'private')"
            >
              Private
            </button>
            <button
              class="btn btn-secondary btn-sm"
              @click="setPromotion(item.record.id, 'no_train')"
            >
              No Train
            </button>
            <button
              class="btn btn-danger btn-sm"
              @click="rejectRecord(item.record.id)"
            >
              Reject
            </button>
          </div>
        </article>
      </section>

      <aside
        v-if="selectedRecordId"
        class="evidence-drawer"
      >
        <div class="drawer-header">
          <h2>Evidence</h2>
          <button
            class="close-btn"
            @click="selectedRecordId = null"
          >
            x
          </button>
        </div>

        <div
          v-if="evidenceLoading"
          class="empty-panel"
        >
          Loading evidence...
        </div>
        <div
          v-else-if="selectedEvidence.length === 0"
          class="empty-panel"
        >
          No evidence events found.
        </div>
        <div
          v-for="item in selectedEvidence"
          :key="item.event_id"
          class="evidence-item"
        >
          <div class="evidence-topline">
            <strong>{{ eventLabel(item.event_type) }}</strong>
            <span>{{ formatDate(item.created_at) }}</span>
          </div>
          <p v-if="item.prompt_excerpt">
            {{ item.prompt_excerpt }}
          </p>
          <p v-if="item.response_excerpt">
            {{ item.response_excerpt }}
          </p>
          <div class="evidence-ids">
            <span>{{ item.session_id }}</span>
            <span v-if="item.model_id">{{ item.model_id }}</span>
          </div>
        </div>
      </aside>
    </main>

    <div
      v-if="message"
      class="save-toast"
      :class="message.type"
    >
      {{ message.text }}
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { twin } from '@/api/client'

const states = ['auto_promoted', 'candidate', 'endorsed', 'rejected', 'private', 'no_train']

const reviewRecords = ref([])
const selectedState = ref('candidate')
const selectedRecordId = ref(null)
const selectedEvidence = ref([])
const loading = ref(false)
const running = ref(false)
const evidenceLoading = ref(false)
const message = ref(null)

const stateCounts = computed(() => {
  return reviewRecords.value.reduce((counts, item) => {
    const state = item.record.promotion_state
    counts[state] = (counts[state] || 0) + 1
    return counts
  }, {})
})

const filteredRecords = computed(() => {
  return reviewRecords.value.filter(item => item.record.promotion_state === selectedState.value)
})

onMounted(loadReview)

async function loadReview() {
  loading.value = true
  try {
    reviewRecords.value = await twin.getReview()
  } catch (err) {
    showMessage('error', err.message || 'Failed to load twin review')
  } finally {
    loading.value = false
  }
}

async function runInference() {
  running.value = true
  try {
    const summary = await twin.runInference()
    await loadReview()
    showMessage(
      'success',
      `Inference complete: ${summary.created_records} created, ${summary.updated_records} updated`,
      4000
    )
  } catch (err) {
    showMessage('error', err.message || 'Failed to run inference')
  } finally {
    running.value = false
  }
}

async function openEvidence(recordId) {
  selectedRecordId.value = recordId
  selectedEvidence.value = []
  evidenceLoading.value = true
  try {
    selectedEvidence.value = await twin.resolveEvidence(recordId)
  } catch (err) {
    showMessage('error', err.message || 'Failed to load evidence')
  } finally {
    evidenceLoading.value = false
  }
}

async function setPromotion(recordId, promotionState, rationale = null) {
  try {
    await twin.setPromotion(recordId, promotionState, rationale)
    await loadReview()
    showMessage('success', `Set record to ${stateLabel(promotionState)}`, 2500)
  } catch (err) {
    showMessage('error', err.message || 'Failed to update record')
  }
}

async function rejectRecord(recordId) {
  const reason = globalThis.prompt?.('Why should this inferred record be rejected?', '')
  if (typeof reason !== 'string') return
  await setPromotion(recordId, 'rejected', reason.trim() || null)
}

function stateLabel(state) {
  return String(state || '').replace(/_/g, ' ').replace(/\b\w/g, char => char.toUpperCase())
}

function kindLabel(kind) {
  return stateLabel(kind)
}

function eventLabel(eventType) {
  return stateLabel(eventType)
}

function formatPercent(value) {
  return `${Math.round((value || 0) * 100)}%`
}

function formatDate(value) {
  if (!value) return ''
  return new Date(value).toLocaleString()
}

function showMessage(type, text, duration = 5000) {
  message.value = { type, text }
  setTimeout(() => {
    if (message.value?.text === text) {
      message.value = null
    }
  }, duration)
}
</script>

<style scoped>
.twin-review-view {
  min-height: 100vh;
  background: var(--bg-primary);
  color: var(--text-primary);
  display: flex;
  flex-direction: column;
}

.review-header {
  height: 56px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-md);
  padding: 0 var(--spacing-lg);
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--border-subtle);
}

.header-left,
.record-actions,
.record-meta,
.evidence-ids {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
}

.header-title {
  display: flex;
  align-items: baseline;
  gap: var(--spacing-md);
}

.header-title h1 {
  margin: 0;
  font-size: 1rem;
}

.header-title span,
.record-meta,
.evidence-ids {
  color: var(--text-muted);
  font-size: 0.75rem;
}

.back-link {
  color: var(--text-secondary);
  text-decoration: none;
  font-size: 0.875rem;
}

.back-link:hover {
  color: var(--accent-primary);
}

.review-main {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: 220px minmax(0, 1fr) 360px;
}

.state-sidebar,
.evidence-drawer {
  background: var(--bg-secondary);
  border-right: 1px solid var(--border-subtle);
  padding: var(--spacing-md);
  overflow-y: auto;
}

.evidence-drawer {
  border-right: none;
  border-left: 1px solid var(--border-subtle);
}

.state-filter {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-sm);
  margin-bottom: var(--spacing-xs);
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.state-filter:hover,
.state-filter.active {
  background: var(--bg-tertiary);
  color: var(--text-primary);
  border-color: var(--border-subtle);
}

.record-list {
  padding: var(--spacing-lg);
  overflow-y: auto;
}

.record-card,
.evidence-item,
.empty-panel {
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  background: var(--bg-secondary);
}

.record-card {
  padding: var(--spacing-md);
  margin-bottom: var(--spacing-md);
}

.record-card-header,
.drawer-header,
.evidence-topline {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--spacing-md);
}

.record-card h2,
.drawer-header h2 {
  margin: 0;
  font-size: 0.95rem;
  line-height: 1.4;
}

.record-kind,
.state-pill {
  display: inline-flex;
  align-items: center;
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  font-size: 0.6875rem;
  font-weight: 700;
  color: var(--accent-cyan);
  border: 1px solid color-mix(in srgb, var(--accent-cyan) 35%, transparent);
}

.state-pill {
  white-space: nowrap;
}

.record-meta {
  margin-top: var(--spacing-sm);
  flex-wrap: wrap;
}

.latest-evidence {
  margin-top: var(--spacing-sm);
  color: var(--text-secondary);
  font-size: 0.8125rem;
}

.record-actions {
  flex-wrap: wrap;
  margin-top: var(--spacing-md);
}

.btn-danger {
  border: 1px solid color-mix(in srgb, var(--accent-red) 40%, transparent);
  background: color-mix(in srgb, var(--accent-red) 12%, transparent);
  color: var(--accent-red);
}

.empty-panel {
  padding: var(--spacing-lg);
  color: var(--text-muted);
}

.close-btn {
  background: transparent;
  border: none;
  color: var(--text-muted);
  cursor: pointer;
}

.evidence-item {
  padding: var(--spacing-sm);
  margin-bottom: var(--spacing-sm);
}

.evidence-topline strong {
  font-size: 0.8125rem;
}

.evidence-topline span {
  color: var(--text-muted);
  font-size: 0.75rem;
}

.evidence-item p {
  margin: var(--spacing-sm) 0 0;
  color: var(--text-secondary);
  font-size: 0.8125rem;
  line-height: 1.5;
}

.evidence-ids {
  margin-top: var(--spacing-sm);
  flex-wrap: wrap;
}

.save-toast {
  position: fixed;
  right: var(--spacing-md);
  top: 70px;
  padding: var(--spacing-sm) var(--spacing-md);
  border-radius: var(--radius-sm);
  z-index: 200;
}

.save-toast.success {
  background: rgba(74, 222, 128, 0.2);
  color: var(--accent-green, #4ade80);
  border: 1px solid rgba(74, 222, 128, 0.3);
}

.save-toast.error {
  background: rgba(248, 113, 113, 0.2);
  color: var(--accent-red);
  border: 1px solid rgba(248, 113, 113, 0.3);
}

@media (max-width: 1100px) {
  .review-main {
    grid-template-columns: 180px minmax(0, 1fr);
  }

  .evidence-drawer {
    position: fixed;
    top: 56px;
    right: 0;
    bottom: 0;
    width: min(420px, 100vw);
    z-index: 100;
  }
}
</style>
