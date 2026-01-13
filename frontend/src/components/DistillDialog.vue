<template>
  <div v-if="isOpen" class="distill-dialog-overlay" @click.self="handleClose">
    <div class="distill-dialog">
      <div class="dialog-header">
        <h2>Distill Note</h2>
        <button class="btn-icon" @click="handleClose">×</button>
      </div>

      <!-- Loading State -->
      <div v-if="loading" class="dialog-loading">
        <div class="spinner"></div>
        <p>{{ loadingMessage }}</p>
      </div>

      <!-- Error State -->
      <div v-else-if="error" class="dialog-error">
        <p>{{ error }}</p>
        <button class="btn btn-secondary" @click="handleSuggest">Retry</button>
      </div>

      <!-- Candidates View -->
      <div v-else-if="candidates.length > 0" class="dialog-content">
        <p class="dialog-intro">
          Found {{ candidates.length }} potential atomic notes. Review and accept/skip each:
        </p>

        <div class="candidates-list">
          <div
            v-for="(candidate, index) in candidates"
            :key="candidate.id"
            :class="['candidate-card', { skipped: decisions[candidate.id] === 'skip' }]"
          >
            <div class="candidate-header">
              <span class="candidate-number">#{{ index + 1 }}</span>
              <input
                v-model="candidateTitles[candidate.id]"
                type="text"
                class="candidate-title-input"
                :placeholder="candidate.title"
              />
              <div class="candidate-actions">
                <button
                  :class="['action-btn', { active: decisions[candidate.id] !== 'skip' }]"
                  @click="setDecision(candidate.id, candidate.duplicate_match ? 'append' : 'create')"
                >
                  {{ candidate.duplicate_match ? 'Merge' : 'Create' }}
                </button>
                <button
                  :class="['action-btn skip-btn', { active: decisions[candidate.id] === 'skip' }]"
                  @click="setDecision(candidate.id, 'skip')"
                >
                  Skip
                </button>
              </div>
            </div>

            <!-- Duplicate Match Info -->
            <div v-if="candidate.duplicate_match" class="duplicate-info">
              <span class="duplicate-badge">
                🔗 Similar to: {{ candidate.duplicate_match.title }}
                ({{ Math.round(candidate.duplicate_match.score * 100) }}% match)
              </span>
              <p class="duplicate-snippet">{{ candidate.duplicate_match.snippet }}</p>
            </div>

            <!-- Summary Bullets -->
            <ul class="candidate-summary">
              <li v-for="(bullet, i) in candidate.summary" :key="i">{{ bullet }}</li>
            </ul>

            <!-- Tags -->
            <div class="candidate-tags">
              <span
                v-for="tag in candidate.recommended_tags"
                :key="tag"
                class="tag"
              >
                #{{ tag }}
              </span>
            </div>

            <!-- Hub Selection -->
            <div class="candidate-hub">
              <label>Hub:</label>
              <input
                v-model="candidateHubs[candidate.id]"
                type="text"
                class="hub-input"
                :placeholder="candidate.suggested_hub || 'No hub'"
              />
            </div>
          </div>
        </div>

        <div class="dialog-actions">
          <button class="btn btn-secondary" @click="handleClose">Cancel</button>
          <button
            class="btn btn-primary"
            :disabled="acceptedCount === 0"
            @click="handleApply"
          >
            Apply ({{ acceptedCount }} notes)
          </button>
        </div>
      </div>

      <!-- Empty State -->
      <div v-else class="dialog-empty">
        <p>No atomic notes could be extracted from this container.</p>
        <button class="btn btn-secondary" @click="handleClose">Close</button>
      </div>

      <!-- Success State -->
      <div v-if="success" class="dialog-success">
        <h3>✓ Distillation Complete</h3>
        <p>Created {{ result.created_note_ids.length }} new notes</p>
        <p>Updated {{ result.updated_note_ids.length }} existing notes</p>
        <p v-if="result.hub_updates.length">Updated {{ result.hub_updates.length }} hubs</p>
        <button class="btn btn-primary" @click="handleClose">Done</button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { notes } from '../api/client'

const props = defineProps({
  isOpen: { type: Boolean, default: false },
  noteId: { type: String, default: '' },
  noteTitle: { type: String, default: '' }
})

const emit = defineEmits(['close', 'success'])

// State
const loading = ref(false)
const loadingMessage = ref('')
const error = ref(null)
const candidates = ref([])
const decisions = ref({})
const candidateTitles = ref({})
const candidateHubs = ref({})
const success = ref(false)
const result = ref(null)

// Computed
const acceptedCount = computed(() => {
  return Object.values(decisions.value).filter(d => d !== 'skip').length
})

// Watch for dialog open
watch(() => props.isOpen, (open) => {
  if (open && props.noteId) {
    handleSuggest()
  }
})

// Methods
async function handleSuggest() {
  loading.value = true
  loadingMessage.value = 'Analyzing note content...'
  error.value = null
  candidates.value = []
  success.value = false
  
  try {
    const response = await notes.distill(props.noteId, {
      mode: 'suggest',
      min_score: 0.85
    })
    
    candidates.value = response.candidates || []
    
    // Initialize decisions (all accepted by default)
    for (const c of candidates.value) {
      decisions.value[c.id] = c.duplicate_match ? 'append' : 'create'
      candidateTitles.value[c.id] = c.title
      candidateHubs.value[c.id] = c.suggested_hub || ''
    }
  } catch (e) {
    error.value = e.response?.data?.detail || 'Failed to analyze note'
    console.error('Distill suggest failed:', e)
  } finally {
    loading.value = false
  }
}

async function handleApply() {
  loading.value = true
  loadingMessage.value = 'Creating atomic notes...'
  
  // Build decisions array
  const decisionsList = candidates.value.map(c => ({
    candidate_id: c.id,
    action: decisions.value[c.id],
    hub_title: candidateHubs.value[c.id] || null,
    custom_title: candidateTitles.value[c.id] !== c.title 
      ? candidateTitles.value[c.id] 
      : null
  }))
  
  try {
    const response = await notes.distill(props.noteId, {
      mode: 'apply',
      decisions: decisionsList,
      candidates: candidates.value,
      hub_policy: 'auto',
      min_score: 0.85
    })
    
    result.value = response
    success.value = true
    emit('success', response)
  } catch (e) {
    error.value = e.response?.data?.detail || 'Failed to apply distillation'
    console.error('Distill apply failed:', e)
  } finally {
    loading.value = false
  }
}

function setDecision(candidateId, action) {
  decisions.value[candidateId] = action
}

function handleClose() {
  // Reset state
  loading.value = false
  error.value = null
  candidates.value = []
  decisions.value = {}
  candidateTitles.value = {}
  candidateHubs.value = {}
  success.value = false
  result.value = null
  emit('close')
}
</script>

<style scoped>
.distill-dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  backdrop-filter: blur(4px);
}

.distill-dialog {
  background: var(--bg-secondary);
  border-radius: var(--radius-lg);
  width: 90%;
  max-width: 700px;
  max-height: 85vh;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  box-shadow: 0 20px 50px rgba(0, 0, 0, 0.4);
}

.dialog-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--bg-tertiary);
}

.dialog-header h2 {
  margin: 0;
  font-size: 1.25rem;
  color: var(--text-primary);
}

.btn-icon {
  background: none;
  border: none;
  font-size: 1.5rem;
  color: var(--text-secondary);
  cursor: pointer;
  padding: 4px 8px;
  border-radius: var(--radius-sm);
}

.btn-icon:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
}

.dialog-loading,
.dialog-empty,
.dialog-error,
.dialog-success {
  padding: var(--spacing-xl);
  text-align: center;
  color: var(--text-secondary);
}

.spinner {
  width: 40px;
  height: 40px;
  border: 3px solid var(--bg-tertiary);
  border-top-color: var(--accent-primary);
  border-radius: 50%;
  animation: spin 1s linear infinite;
  margin: 0 auto var(--spacing-md);
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.dialog-content {
  flex: 1;
  overflow-y: auto;
  padding: var(--spacing-lg);
}

.dialog-intro {
  margin-bottom: var(--spacing-md);
  color: var(--text-secondary);
}

.candidates-list {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-md);
}

.candidate-card {
  background: var(--bg-primary);
  border-radius: var(--radius-md);
  padding: var(--spacing-md);
  border: 1px solid var(--bg-tertiary);
  transition: opacity 0.2s, border-color 0.2s;
}

.candidate-card.skipped {
  opacity: 0.5;
  border-color: transparent;
}

.candidate-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  margin-bottom: var(--spacing-sm);
}

.candidate-number {
  color: var(--accent-primary);
  font-weight: 600;
  font-size: 0.875rem;
}

.candidate-title-input {
  flex: 1;
  font-size: 1rem;
  font-weight: 600;
  background: transparent;
  border: none;
  color: var(--text-primary);
  padding: 4px 0;
}

.candidate-title-input:focus {
  outline: none;
  border-bottom: 1px solid var(--accent-primary);
}

.candidate-actions {
  display: flex;
  gap: var(--spacing-xs);
}

.action-btn {
  padding: 4px 12px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--bg-tertiary);
  background: transparent;
  color: var(--text-secondary);
  font-size: 0.75rem;
  cursor: pointer;
  transition: all 0.2s;
}

.action-btn:hover {
  border-color: var(--accent-primary);
  color: var(--accent-primary);
}

.action-btn.active {
  background: var(--accent-primary);
  border-color: var(--accent-primary);
  color: white;
}

.action-btn.skip-btn.active {
  background: var(--text-muted);
  border-color: var(--text-muted);
}

.duplicate-info {
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  padding: var(--spacing-sm);
  margin-bottom: var(--spacing-sm);
}

.duplicate-badge {
  font-size: 0.75rem;
  color: var(--accent-secondary);
}

.duplicate-snippet {
  font-size: 0.75rem;
  color: var(--text-muted);
  margin-top: var(--spacing-xs);
  font-style: italic;
}

.candidate-summary {
  margin: var(--spacing-sm) 0;
  padding-left: var(--spacing-lg);
  color: var(--text-secondary);
  font-size: 0.875rem;
}

.candidate-summary li {
  margin-bottom: 4px;
}

.candidate-tags {
  display: flex;
  gap: var(--spacing-xs);
  flex-wrap: wrap;
  margin-bottom: var(--spacing-sm);
}

.tag {
  background: var(--bg-tertiary);
  color: var(--accent-primary);
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  font-size: 0.75rem;
}

.candidate-hub {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  font-size: 0.875rem;
}

.candidate-hub label {
  color: var(--text-secondary);
}

.hub-input {
  flex: 1;
  background: var(--bg-tertiary);
  border: none;
  border-radius: var(--radius-sm);
  padding: 4px 8px;
  color: var(--text-primary);
  font-size: 0.875rem;
}

.hub-input:focus {
  outline: 1px solid var(--accent-primary);
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-md);
  padding: var(--spacing-md) var(--spacing-lg);
  border-top: 1px solid var(--bg-tertiary);
}

.dialog-success {
  color: var(--accent-success);
}

.dialog-success h3 {
  color: var(--accent-success);
  margin-bottom: var(--spacing-md);
}

.dialog-error {
  color: var(--accent-danger);
}
</style>
