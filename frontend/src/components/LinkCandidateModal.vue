<template>
  <div
    class="link-modal-overlay"
    @click.self="$emit('close')"
  >
    <div class="link-modal">
      <div class="modal-header">
        <h3>Discover Links</h3>
        <button
          class="close-btn"
          @click="$emit('close')"
        >
          ×
        </button>
      </div>

      <div class="modal-body">
        <!-- Loading -->
        <div
          v-if="loading"
          class="loading-state"
        >
          <span class="loading-spinner" />
          <p>Discovering link candidates...</p>
        </div>

        <!-- Error -->
        <div
          v-else-if="error"
          class="error-state"
        >
          <p>{{ error }}</p>
          <button
            class="btn btn-secondary"
            @click="$emit('retry')"
          >
            Retry
          </button>
        </div>

        <!-- No results -->
        <div
          v-else-if="candidates.length === 0"
          class="empty-state"
        >
          <p>No link candidates found for this note.</p>
        </div>

        <!-- Candidate list -->
        <div
          v-else
          class="candidates-list"
        >
          <label class="select-all-row">
            <input
              v-model="allSelected"
              type="checkbox"
              @change="toggleAll"
            >
            <span>Select all ({{ candidates.length }} candidates)</span>
          </label>

          <div
            v-for="candidate in candidates"
            :key="candidate.target_id"
            class="candidate-item"
            :class="{ selected: selected.has(candidate.target_id) }"
          >
            <label class="candidate-check">
              <input
                type="checkbox"
                :checked="selected.has(candidate.target_id)"
                @change="toggleCandidate(candidate.target_id)"
              >
            </label>
            <div class="candidate-info">
              <div class="candidate-header">
                <span class="candidate-title">{{ candidate.target_title }}</span>
                <span
                  class="confidence-badge"
                  :class="confidenceClass(candidate.confidence)"
                >
                  {{ Math.round(candidate.confidence * 100) }}%
                </span>
              </div>
              <div
                v-if="candidate.reason"
                class="candidate-reason"
              >
                {{ candidate.reason }}
              </div>
              <span
                class="link-type-badge"
                :class="'type-' + candidate.link_type"
              >
                {{ candidate.link_type }}
              </span>
            </div>
          </div>
        </div>
      </div>

      <div
        v-if="candidates.length > 0"
        class="modal-footer"
      >
        <button
          class="btn btn-ghost"
          @click="$emit('close')"
        >
          Cancel
        </button>
        <button
          class="btn btn-primary"
          :disabled="selected.size === 0 || applying"
          @click="handleApply"
        >
          {{ applying ? 'Applying...' : `Apply ${selected.size} Link${selected.size !== 1 ? 's' : ''}` }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
import { zettelkasten } from '@/api/client'
import { useToast } from '@/composables/useToast'

const props = defineProps({
  noteId: {
    type: String,
    required: true,
  },
  candidates: {
    type: Array,
    default: () => [],
  },
  loading: {
    type: Boolean,
    default: false,
  },
  error: {
    type: String,
    default: null,
  },
})

const emit = defineEmits(['close', 'retry', 'applied'])
const toast = useToast()

const selected = ref(new Set())
const applying = ref(false)

const allSelected = computed({
  get: () => selected.value.size === props.candidates.length && props.candidates.length > 0,
  set: () => {},
})

function toggleAll() {
  if (selected.value.size === props.candidates.length) {
    selected.value = new Set()
  } else {
    selected.value = new Set(props.candidates.map(c => c.target_id))
  }
}

function toggleCandidate(id) {
  const next = new Set(selected.value)
  if (next.has(id)) {
    next.delete(id)
  } else {
    next.add(id)
  }
  selected.value = next
}

function confidenceClass(confidence) {
  if (confidence >= 0.8) return 'confidence-high'
  if (confidence >= 0.5) return 'confidence-medium'
  return 'confidence-low'
}

async function handleApply() {
  if (selected.value.size === 0 || applying.value) return

  applying.value = true
  try {
    const linkIds = Array.from(selected.value)
    const result = await zettelkasten.applyLinks(props.noteId, linkIds)
    toast.success(`Created ${result.links_created} link${result.links_created !== 1 ? 's' : ''}`)
    emit('applied', result)
  } catch (e) {
    console.error('Failed to apply links:', e)
    toast.error('Failed to apply links')
  } finally {
    applying.value = false
  }
}
</script>

<style scoped>
.link-modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  animation: fadeIn 0.2s ease;
}

@keyframes fadeIn {
  from { opacity: 0; }
  to { opacity: 1; }
}

.link-modal {
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-lg);
  width: 100%;
  max-width: 560px;
  max-height: 80vh;
  display: flex;
  flex-direction: column;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.4);
  animation: slideUp 0.3s ease;
}

@keyframes slideUp {
  from { transform: translateY(20px); opacity: 0; }
  to { transform: translateY(0); opacity: 1; }
}

.modal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--bg-tertiary);
}

.modal-header h3 {
  margin: 0;
  font-size: 1.1rem;
  color: var(--text-primary);
}

.close-btn {
  background: none;
  border: none;
  font-size: 1.5rem;
  color: var(--text-muted);
  cursor: pointer;
  padding: 0;
  line-height: 1;
  transition: color var(--transition-fast);
}

.close-btn:hover {
  color: var(--text-primary);
}

.modal-body {
  padding: var(--spacing-lg);
  overflow-y: auto;
  flex: 1;
}

.loading-state,
.empty-state,
.error-state {
  text-align: center;
  padding: var(--spacing-xl);
  color: var(--text-muted);
}

.loading-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--spacing-md);
}

.loading-spinner {
  width: 24px;
  height: 24px;
  border: 3px solid var(--bg-tertiary);
  border-top-color: var(--accent-primary);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

.select-all-row {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm) 0;
  margin-bottom: var(--spacing-sm);
  font-size: 0.85rem;
  color: var(--text-secondary);
  cursor: pointer;
  border-bottom: 1px solid var(--bg-tertiary);
}

.select-all-row input {
  accent-color: var(--accent-primary);
}

.candidates-list {
  display: flex;
  flex-direction: column;
}

.candidate-item {
  display: flex;
  gap: var(--spacing-sm);
  padding: var(--spacing-sm) var(--spacing-xs);
  border-radius: var(--radius-md);
  transition: background var(--transition-fast);
}

.candidate-item:hover {
  background: var(--bg-hover);
}

.candidate-item.selected {
  background: rgba(99, 102, 241, 0.05);
}

.candidate-check {
  padding-top: 2px;
  cursor: pointer;
}

.candidate-check input {
  accent-color: var(--accent-primary);
}

.candidate-info {
  flex: 1;
  min-width: 0;
}

.candidate-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--spacing-sm);
}

.candidate-title {
  font-size: 0.9rem;
  font-weight: 500;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.confidence-badge {
  font-size: 0.7rem;
  font-weight: 600;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  flex-shrink: 0;
}

.confidence-high {
  color: #22c55e;
  background: rgba(34, 197, 94, 0.1);
}

.confidence-medium {
  color: #f59e0b;
  background: rgba(245, 158, 11, 0.1);
}

.confidence-low {
  color: var(--text-muted);
  background: var(--bg-tertiary);
}

.candidate-reason {
  font-size: 0.78rem;
  color: var(--text-secondary);
  margin: 2px 0 4px;
  line-height: 1.4;
}

.link-type-badge {
  display: inline-block;
  font-size: 0.65rem;
  text-transform: uppercase;
  font-weight: 600;
  letter-spacing: 0.03em;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  color: var(--text-muted);
  background: var(--bg-tertiary);
}

.type-supports { color: #22c55e; background: rgba(34, 197, 94, 0.1); }
.type-contradicts { color: #ef4444; background: rgba(239, 68, 68, 0.1); }
.type-expands { color: #6366f1; background: rgba(99, 102, 241, 0.1); }
.type-related { color: #3b82f6; background: rgba(59, 130, 246, 0.1); }
.type-questions { color: #f59e0b; background: rgba(245, 158, 11, 0.1); }
.type-answers { color: #22c55e; background: rgba(34, 197, 94, 0.1); }
.type-example { color: #8b5cf6; background: rgba(139, 92, 246, 0.1); }
.type-part_of { color: #06b6d4; background: rgba(6, 182, 212, 0.1); }

.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-sm);
  padding: var(--spacing-md) var(--spacing-lg);
  border-top: 1px solid var(--bg-tertiary);
}

.btn {
  padding: var(--spacing-sm) var(--spacing-lg);
  border-radius: var(--radius-md);
  font-weight: 500;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.btn-ghost {
  background: transparent;
  border: 1px solid var(--bg-tertiary);
  color: var(--text-secondary);
}

.btn-ghost:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.btn-primary {
  background: var(--accent-primary);
  border: none;
  color: white;
}

.btn-primary:hover:not(:disabled) {
  filter: brightness(1.1);
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.error-state .btn {
  margin-top: var(--spacing-md);
}
</style>
