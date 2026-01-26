<template>
  <div class="import-review">
    <!-- Initial Loading State -->
    <div v-if="isLoading || isParsing" class="loading-overlay">
      <div class="spinner"></div>
      <p>{{ isParsing ? 'Parsing file...' : 'Loading conversations...' }}</p>
    </div>

    <!-- Error State -->
    <div v-else-if="loadError" class="error-state">
      <h3>⚠️ Error Loading Import</h3>
      <p>{{ loadError }}</p>
      <button @click="goBack" class="btn btn-primary">← Back to Upload</button>
    </div>

    <!-- Main Content -->
    <template v-else>
      <div class="header">
        <h2>Review Import</h2>
        <div class="stats">
          <div class="stat-item">
            <span class="label">Platform:</span>
            <span class="value">{{ platform }}</span>
          </div>
          <div class="stat-item">
            <span class="label">Conversations:</span>
            <span class="value">{{ totalConversations }}</span>
          </div>
          <div class="stat-item">
            <span class="label">Duplicates:</span>
            <span class="value warning">{{ duplicatesFound }}</span>
          </div>
          <div class="stat-item">
            <span class="label">Est. Notes:</span>
            <span class="value success">{{ estimatedNotes }}</span>
          </div>
        </div>
      </div>

    <div v-if="isApplying" class="loading-overlay">
      <div class="spinner"></div>
      <p>Importing conversations...</p>
    </div>

    <div v-else class="content">
      <!-- Toolbar -->
      <div class="toolbar">
        <button @click="assessQuality" class="btn btn-sm btn-ai" :disabled="isAssessing">
          {{ isAssessing ? '🔄 Assessing...' : '🤖 AI Assess' }}
        </button>

        <!-- Zettelkasten Controls -->
        <div class="zettel-controls">
          <button
            @click="toggleZettel"
            class="btn btn-sm"
            :class="zettelEnabled ? 'btn-zettel' : 'btn-ghost'"
            :title="zettelEnabled ? 'Zettelkasten mode enabled' : 'Zettelkasten mode disabled'"
          >
            {{ zettelEnabled ? '🗃️ Zettelkasten ON' : '🗃️ Zettelkasten OFF' }}
          </button>
          <select
            v-model="linkModeValue"
            @change="updateLinkMode"
            :disabled="!zettelEnabled"
            class="link-mode-select"
            title="How notes should be linked"
          >
            <option value="automatic">Auto-Link</option>
            <option value="suggested">Suggest Links</option>
            <option value="manual">Manual Links</option>
          </select>
        </div>

        <div class="toolbar-divider"></div>
        <button @click="selectAll" class="btn btn-sm">
          {{ allSelected ? 'Deselect All' : 'Select All' }}
        </button>
        <button @click="acceptSelected" class="btn btn-sm btn-success" :disabled="selectedCount === 0">
          Accept ({{ selectedCount }})
        </button>
        <button @click="skipSelected" class="btn btn-sm btn-secondary" :disabled="selectedCount === 0">
          Skip ({{ selectedCount }})
        </button>
        <button @click="batchAcceptAll" class="btn btn-sm btn-primary">
          Accept All
        </button>
        <button @click="batchSkipAll" class="btn btn-sm btn-warning">
          Skip All
        </button>
        <div class="spacer"></div>
        <button @click="showFilters = !showFilters" class="btn btn-sm">
          {{ showFilters ? 'Hide Filters' : 'Show Filters' }}
        </button>
      </div>

      <!-- Filters -->
      <div v-if="showFilters" class="filters">
        <label>
          <input type="checkbox" v-model="filterDuplicates" />
          Show only duplicates
        </label>
        <label>
          <input type="checkbox" v-model="filterWithErrors" />
          Show only with errors
        </label>
        <select v-model="filterByAction">
          <option value="all">All Actions</option>
          <option value="accept">Accept</option>
          <option value="skip">Skip</option>
          <option value="merge">Merge</option>
        </select>
      </div>

      <!-- Conversations List -->
      <div class="conversations-list">
        <div
          v-for="conv in filteredConversations"
          :key="conv.id"
          class="conversation-item"
          :class="{
            'selected': selectedIds.includes(conv.id),
            'has-duplicates': conv.duplicate_candidates.length > 0
          }"
        >
          <div class="conversation-header" @click="toggleSelect(conv.id)">
            <input
              type="checkbox"
              :checked="selectedIds.includes(conv.id)"
              @click.stop="toggleSelect(conv.id)"
            />
            <div class="conversation-info">
              <h3 class="title">{{ conv.title }}</h3>
              <div class="meta">
                <span class="badge" :class="getDecisionBadgeClass(conv.id)">
                  {{ getDecisionLabel(conv.id) }}
                </span>
                <span v-if="conv.duplicate_candidates.length > 0" class="badge duplicate">
                  {{ conv.duplicate_candidates.length }} duplicate(s)
                </span>
                <!-- Quality badges -->
                <span v-if="conv.quality" class="badge quality-badge" :class="getQualityClass(conv.quality)">
                  {{ getQualitySuggestion(conv.quality) }}
                </span>
                <span v-if="conv.quality" class="score-badge" :title="`Relevance: ${Math.round(conv.quality.relevance_score * 100)}%`">
                  📊 {{ Math.round(conv.quality.relevance_score * 100) }}%
                </span>
                <span class="messages-count">
                  {{ conv.messages.length }} messages
                </span>
                <span class="date">
                  {{ formatDate(conv.metadata.created_at) }}
                </span>
              </div>
            </div>
            <button @click.stop="expandConversation(conv.id)" class="expand-btn">
              {{ expandedIds.includes(conv.id) ? '▼' : '▶' }}
            </button>
          </div>

          <!-- Expanded Details -->
          <div v-if="expandedIds.includes(conv.id)" class="conversation-details">
            <!-- Messages Preview -->
            <div class="messages-preview">
              <h4>Message Preview</h4>
              <div class="message-list">
                <div
                  v-for="(msg, idx) in conv.messages.slice(0, 3)"
                  :key="idx"
                  class="message-preview-item"
                  :class="msg.role"
                >
                  <strong>{{ msg.role }}:</strong>
                  <span class="message-text">{{ truncate(msg.content, 150) }}</span>
                </div>
                <div v-if="conv.messages.length > 3" class="more-messages">
                  ... and {{ conv.messages.length - 3 }} more messages
                </div>
              </div>
            </div>

            <!-- Duplicate Candidates -->
            <div v-if="conv.duplicate_candidates.length > 0" class="duplicates-section">
              <h4>Duplicates Found</h4>
              <div class="duplicate-list">
                <div
                  v-for="dup in conv.duplicate_candidates"
                  :key="dup.note_id"
                  class="duplicate-item"
                >
                  <div class="duplicate-info">
                    <strong>{{ dup.title }}</strong>
                    <span class="similarity">Similarity: {{ (dup.similarity_score * 100).toFixed(0) }}%</span>
                    <span v-if="dup.content_type" class="content-type">{{ dup.content_type }}</span>
                  </div>
                  <div class="duplicate-tags">
                    <span v-for="tag in dup.tags" :key="tag" class="tag">{{ tag }}</span>
                  </div>
                  <div class="duplicate-actions">
                    <button
                      @click.stop="viewDuplicate(dup.note_id)"
                      class="btn btn-xs btn-link"
                    >
                      View
                    </button>
                    <button
                      @click.stop="mergeWith(conv.id, dup.note_id)"
                      class="btn btn-xs btn-secondary"
                    >
                      Merge
                    </button>
                  </div>
                </div>
              </div>
            </div>

            <!-- Action Options -->
            <div class="action-options">
              <h4>Import Options</h4>
              <div class="distillation-options">
                <label class="radio-option">
                  <input
                    type="radio"
                    :name="`distill-${conv.id}`"
                    value="container_only"
                    :checked="getDecision(conv.id).distill_option === 'container_only'"
                    @change="setDistillOption(conv.id, 'container_only')"
                  />
                  <span>Container Only (No distillation)</span>
                </label>
                <label class="radio-option">
                  <input
                    type="radio"
                    :name="`distill-${conv.id}`"
                    value="auto_distill"
                    :checked="getDecision(conv.id).distill_option === 'auto_distill'"
                    @change="setDistillOption(conv.id, 'auto_distill')"
                  />
                  <span>Auto-Distill (Create atomic notes)</span>
                </label>
              </div>

              <!-- Custom Atomics Toggle -->
              <div v-if="getDecision(conv.id).distill_option === 'custom'" class="custom-atomics">
                <button @click.stop="showCustomAtomics(conv.id)" class="btn btn-sm btn-secondary">
                  Configure Custom Atomics
                </button>
              </div>
            </div>

            <!-- Tags Preview -->
            <div class="tags-section">
              <h4>Suggested Tags</h4>
              <div class="tag-list">
                <span v-for="tag in conv.suggested_tags" :key="tag" class="tag suggested">
                  {{ tag }}
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- Footer Actions -->
      <div class="footer">
        <button @click="goBack" class="btn btn-secondary">
          ← Back to Upload
        </button>
        <div class="spacer"></div>
        <button @click="submitImport" class="btn btn-primary btn-lg" :disabled="!hasAcceptedConversations">
          Import Selected ({{ acceptedCount }})
        </button>
      </div>
    </div>
    </template>

    <!-- Summary Modal -->
    <div v-if="showSummaryModal" class="modal-overlay">
      <div class="modal">
        <div class="modal-header">
          <h3>Import Summary</h3>
          <button @click="showSummaryModal = false" class="close-btn">×</button>
        </div>
        <div class="modal-body">
          <div class="summary-stats">
            <div class="summary-item success">
              <span class="count">{{ importSummary?.imported || 0 }}</span>
              <span class="label">Imported</span>
            </div>
            <div class="summary-item warning">
              <span class="count">{{ importSummary?.skipped || 0 }}</span>
              <span class="label">Skipped</span>
            </div>
            <div class="summary-item info">
              <span class="count">{{ importSummary?.merged || 0 }}</span>
              <span class="label">Merged</span>
            </div>
            <div class="summary-item error">
              <span class="count">{{ importSummary?.failed || 0 }}</span>
              <span class="label">Failed</span>
            </div>
          </div>
          <div class="notes-created">
            <p><strong>Container Notes:</strong> {{ importSummary?.container_notes || 0 }}</p>
            <p><strong>Atomic Notes:</strong> {{ importSummary?.atomic_notes || 0 }}</p>
            <p><strong>Total Notes Created:</strong> {{ importSummary?.notes_created || 0 }}</p>
          </div>
        </div>
        <div class="modal-footer">
          <button @click="revertImport" class="btn btn-danger" :disabled="isReverting">
            {{ isReverting ? 'Reverting...' : '⟲ Revert Import' }}
          </button>
          <div class="spacer"></div>
          <button @click="viewNotes" class="btn btn-primary">
            View Imported Notes
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script>
import { ref, computed, onMounted } from 'vue'
import { useImportStore } from '@/stores/import'
import { useRouter } from 'vue-router'

export default {
  name: 'ImportReview',
  
  setup() {
    const router = useRouter()
    const importStore = useImportStore()
    
    // Local state
    const showFilters = ref(false)
    const filterDuplicates = ref(false)
    const filterWithErrors = ref(false)
    const filterByAction = ref('all')
    const selectedIds = ref([])
    const expandedIds = ref([])
    const showSummaryModal = ref(false)
    const importSummary = ref(null)
    const isLoading = ref(true)
    const loadError = ref(null)

    // Initialize on mount
    onMounted(async () => {
      try {
        // Check if we have a current job
        if (!importStore.currentJobId) {
          // No job, redirect back to upload
          router.push('/import')
          return
        }

        // Parse the file if not already parsed
        if (importStore.currentJob?.status === 'uploaded') {
          await importStore.parseJob(importStore.currentJobId)
        }

        // Load preview data
        if (importStore.currentJob?.status === 'parsed') {
          await importStore.loadPreview(importStore.currentJobId)
        }
      } catch (error) {
        console.error('Failed to load import preview:', error)
        loadError.value = error.message
      } finally {
        isLoading.value = false
      }
    })

    // Computed
    const isParsing = computed(() => importStore.isParsing)
    const isApplying = computed(() => importStore.isApplying)
    
    const allSelected = computed(() => {
      return filteredConversations.value.length > 0 && 
        selectedIds.value.length === filteredConversations.value.length
    })
    
    const selectedCount = computed(() => selectedIds.value.length)
    
    const acceptedCount = computed(() => {
      return filteredConversations.value.filter(conv => {
        const decision = importStore.decisions[conv.id]
        return decision && decision.action === 'accept'
      }).length
    })
    
    const hasAcceptedConversations = computed(() => acceptedCount.value > 0)
    
    const filteredConversations = computed(() => {
      let conversations = [...importStore.parsedConversations]
      
      if (filterDuplicates.value) {
        conversations = conversations.filter(conv => conv.duplicate_candidates.length > 0)
      }
      
      if (filterByAction.value !== 'all') {
        conversations = conversations.filter(conv => {
          const decision = importStore.decisions[conv.id]
          return decision && decision.action === filterByAction.value
        })
      }
      
      return conversations
    })

    // Methods
    const toggleSelect = (id) => {
      const idx = selectedIds.value.indexOf(id)
      if (idx === -1) {
        selectedIds.value.push(id)
      } else {
        selectedIds.value.splice(idx, 1)
      }
    }

    const selectAll = () => {
      if (allSelected.value) {
        selectedIds.value = []
      } else {
        selectedIds.value = filteredConversations.value.map(c => c.id)
      }
    }

    const acceptSelected = () => {
      selectedIds.value.forEach(id => {
        importStore.updateDecision(id, { action: 'accept' })
      })
    }

    const skipSelected = () => {
      selectedIds.value.forEach(id => {
        importStore.updateDecision(id, { action: 'skip' })
      })
    }

    const batchAcceptAll = () => {
      importStore.batchUpdateDecisions(
        filteredConversations.value.map(c => c.id),
        'accept'
      )
    }

    const batchSkipAll = () => {
      importStore.batchUpdateDecisions(
        filteredConversations.value.map(c => c.id),
        'skip'
      )
    }

    const expandConversation = (id) => {
      const idx = expandedIds.value.indexOf(id)
      if (idx === -1) {
        expandedIds.value.push(id)
      } else {
        expandedIds.value.splice(idx, 1)
      }
    }

    const getDecision = (id) => {
      return importStore.decisions[id] || {
        conversation_id: id,
        action: 'skip',
        distill_option: 'auto_distill'
      }
    }

    const getDecisionLabel = (id) => {
      const action = getDecision(id).action
      const labels = {
        accept: 'Accept',
        skip: 'Skip',
        merge: 'Merge',
        modify: 'Modify'
      }
      return labels[action] || action
    }

    const getDecisionBadgeClass = (id) => {
      const action = getDecision(id).action
      const classes = {
        accept: 'badge-success',
        skip: 'badge-secondary',
        merge: 'badge-info',
        modify: 'badge-warning'
      }
      return classes[action] || 'badge-secondary'
    }

    const setDistillOption = (id, option) => {
      importStore.updateDecision(id, { distill_option: option })
    }

    const viewDuplicate = (noteId) => {
      router.push(`/notes/${noteId}`)
    }

    const mergeWith = (convId, targetNoteId) => {
      importStore.updateDecision(convId, {
        action: 'merge',
        target_note_id: targetNoteId
      })
    }

    const showCustomAtomics = (id) => {
      // TODO: Show custom atomics configuration modal
      console.log('Show custom atomics for', id)
    }

    const submitImport = async () => {
      try {
        const decisions = Object.values(importStore.decisions)
        const summary = await importStore.submitDecisions(decisions)
        importSummary.value = summary
        showSummaryModal.value = true
      } catch (error) {
        console.error('Import failed:', error)
      }
    }

    const viewNotes = () => {
      router.push('/')
    }

    const goBack = () => {
      router.push('/import')
    }

    const revertImport = async () => {
      if (!confirm('Are you sure you want to revert this import? All imported notes will be deleted.')) {
        return
      }
      
      try {
        await importStore.revertLastImport()
        showSummaryModal.value = false
        alert('Import reverted successfully. All imported notes have been deleted.')
        router.push('/import')
      } catch (error) {
        console.error('Revert failed:', error)
        alert('Failed to revert import: ' + error.message)
      }
    }

    const truncate = (text, maxLength) => {
      if (text.length <= maxLength) return text
      return text.substring(0, maxLength) + '...'
    }

    const formatDate = (dateStr) => {
      if (!dateStr) return 'N/A'
      const date = new Date(dateStr)
      return date.toLocaleDateString()
    }

    // Quality assessment helpers
    const getQualityClass = (quality) => {
      if (!quality) return ''
      if (quality.suggested_action === 'import_and_distill') return 'quality-high'
      if (quality.suggested_action === 'import_only') return 'quality-medium'
      return 'quality-low'
    }

    const getQualitySuggestion = (quality) => {
      if (!quality) return ''
      const labels = {
        'import_and_distill': '✨ Distill',
        'import_only': '📥 Import',
        'skip': '⏭️ Skip'
      }
      return labels[quality.suggested_action] || quality.suggested_action
    }

    const assessQuality = async () => {
      try {
        await importStore.assessQuality()
      } catch (error) {
        console.error('Quality assessment failed:', error)
        alert('Quality assessment failed: ' + error.message)
      }
    }

    // Zettelkasten methods
    const zettelEnabled = computed(() => importStore.zettelEnabled)
    const linkModeValue = computed({
      get: () => importStore.linkMode,
      set: (val) => importStore.setLinkMode(val)
    })

    const toggleZettel = () => {
      importStore.toggleZettelEnabled()
    }

    const updateLinkMode = () => {
      // Link mode is already updated via v-model
    }

    const getZettelTypeLabel = (type) => {
      return importStore.getZettelTypeLabel(type)
    }

    return {
      isLoading,
      loadError,
      isParsing,
      isApplying,
      isAssessing: computed(() => importStore.isAssessing),
      showFilters,
      filterDuplicates,
      filterWithErrors,
      filterByAction,
      selectedIds,
      expandedIds,
      showSummaryModal,
      importSummary,
      allSelected,
      selectedCount,
      acceptedCount,
      hasAcceptedConversations,
      filteredConversations,
      platform: computed(() => importStore.platform),
      totalConversations: computed(() => importStore.totalConversations),
      duplicatesFound: computed(() => importStore.duplicatesFound),
      estimatedNotes: computed(() => importStore.estimatedNotes),
      toggleSelect,
      selectAll,
      acceptSelected,
      skipSelected,
      batchAcceptAll,
      batchSkipAll,
      expandConversation,
      getDecision,
      getDecisionLabel,
      getDecisionBadgeClass,
      setDistillOption,
      viewDuplicate,
      mergeWith,
      showCustomAtomics,
      submitImport,
      viewNotes,
      goBack,
      revertImport,
      isReverting: computed(() => importStore.isReverting),
      truncate,
      formatDate,
      getQualityClass,
      getQualitySuggestion,
      assessQuality,
      // Zettelkasten
      zettelEnabled,
      linkModeValue,
      toggleZettel,
      updateLinkMode,
      getZettelTypeLabel
    }
  }
}
</script>

<style scoped>
.import-review {
  max-width: 1200px;
  margin: 0 auto;
  padding: 1rem;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.header {
  margin-bottom: 1.5rem;
}

.header h2 {
  font-size: 1.5rem;
  margin-bottom: 1rem;
}

.stats {
  display: flex;
  gap: 2rem;
  flex-wrap: wrap;
}

.stat-item {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.stat-item .label {
  font-size: 0.85rem;
  color: #666;
}

.stat-item .value {
  font-size: 1.2rem;
  font-weight: 600;
}

.stat-item .value.warning {
  color: #f59e0b;
}

.stat-item .value.success {
  color: #10b981;
}

.loading-overlay {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 4rem;
}

.spinner {
  width: 40px;
  height: 40px;
  border: 4px solid #f3f3f3;
  border-top: 4px solid #4a90e2;
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  0% { transform: rotate(0deg); }
  100% { transform: rotate(360deg); }
}

.error-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 4rem;
  text-align: center;
}

.error-state h3 {
  margin-bottom: 1rem;
  color: #dc2626;
}

.error-state p {
  margin-bottom: 1.5rem;
  color: #6b7280;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: 1.5rem;
  padding: 1rem;
  background: #f9fafb;
  border-radius: 8px;
}

.spacer {
  flex: 1;
}

.btn {
  padding: 0.5rem 1rem;
  border: none;
  border-radius: 4px;
  font-size: 0.9rem;
  cursor: pointer;
  transition: all 0.2s;
}

.btn-sm {
  padding: 0.4rem 0.8rem;
  font-size: 0.85rem;
}

.btn-xs {
  padding: 0.25rem 0.5rem;
  font-size: 0.75rem;
}

.btn-lg {
  padding: 0.75rem 1.5rem;
  font-size: 1rem;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-primary {
  background: #4a90e2;
  color: white;
}

.btn-primary:hover:not(:disabled) {
  background: #357abd;
}

.btn-secondary {
  background: #64748b;
  color: white;
}

.btn-secondary:hover:not(:disabled) {
  background: #4b5563;
}

.btn-danger {
  background: #dc2626;
  color: white;
}

.btn-danger:hover:not(:disabled) {
  background: #b91c1c;
}

.btn-success {
  background: #10b981;
  color: white;
}

.btn-success:hover:not(:disabled) {
  background: #059669;
}

.btn-warning {
  background: #f59e0b;
  color: white;
}

.btn-link {
  background: transparent;
  color: #4a90e2;
  border: 1px solid #4a90e2;
}

.btn-link:hover {
  background: rgba(74, 144, 226, 0.1);
}

.filters {
  display: flex;
  gap: 1.5rem;
  margin-bottom: 1rem;
  padding: 0.75rem;
  background: white;
  border-radius: 4px;
}

.filters label {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  cursor: pointer;
}

.conversations-list {
  display: flex;
  flex-direction: column;
  gap: 1rem;
  flex: 1;
  overflow-y: auto;
  padding-right: 0.5rem;
  max-height: calc(100vh - 320px);
}

.conversation-item {
  border: 1px solid #e5e7eb;
  border-radius: 8px;
  background: white;
  overflow: hidden;
  transition: all 0.2s;
  flex-shrink: 0;
}

.conversation-item.selected {
  border-color: #4a90e2;
  box-shadow: 0 0 0 3px rgba(74, 144, 226, 0.2);
}

.conversation-item.has-duplicates {
  border-left: 4px solid #f59e0b;
}

.conversation-header {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 0.75rem 1rem;
  cursor: pointer;
  background: #f9fafb;
  transition: background 0.2s;
}

.conversation-header:hover {
  background: #f3f4f6;
}

.conversation-header input[type="checkbox"] {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
}

.conversation-info {
  flex: 1;
}

.title {
  margin: 0 0 0.5rem 0;
  font-size: 1.1rem;
}

.meta {
  display: flex;
  gap: 0.5rem;
  flex-wrap: wrap;
  align-items: center;
}

.badge {
  padding: 0.25rem 0.5rem;
  border-radius: 3px;
  font-size: 0.75rem;
  font-weight: 600;
}

.badge-success {
  background: #d1fae5;
  color: #065f46;
}

.badge-secondary {
  background: #e5e7eb;
  color: #374151;
}

.badge-info {
  background: #dbeafe;
  color: #1e40af;
}

.badge-warning {
  background: #fef3c7;
  color: #92400e;
}

.badge.duplicate {
  background: #fef3c7;
  color: #92400e;
}

.messages-count,
.date {
  font-size: 0.85rem;
  color: #6b7280;
}

.expand-btn {
  padding: 0.25rem 0.5rem;
  background: white;
  border: 1px solid #d1d5db;
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.2s;
}

.expand-btn:hover {
  background: #f3f4f6;
}

.conversation-details {
  padding: 1rem;
  border-top: 1px solid #e5e7eb;
}

.messages-preview,
.duplicates-section,
.action-options,
.tags-section {
  margin-bottom: 1rem;
}

.messages-preview h4,
.duplicates-section h4,
.action-options h4,
.tags-section h4 {
  margin: 0 0 0.75rem 0;
  font-size: 0.95rem;
  color: #374151;
}

.message-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.message-preview-item {
  padding: 0.5rem;
  border-radius: 4px;
  background: #f9fafb;
}

.message-preview-item.user {
  border-left: 3px solid #6366f1;
}

.message-preview-item.assistant {
  border-left: 3px solid #10b981;
}

.message-text {
  color: #4b5563;
}

.more-messages {
  color: #6b7280;
  font-style: italic;
}

.duplicate-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.duplicate-item {
  padding: 0.75rem;
  border: 1px solid #e5e7eb;
  border-radius: 4px;
  background: #fef3c7;
}

.duplicate-info {
  display: flex;
  justify-content: space-between;
  margin-bottom: 0.5rem;
}

.similarity {
  color: #d97706;
  font-weight: 600;
}

.content-type {
  padding: 0.25rem 0.5rem;
  background: white;
  border-radius: 3px;
  font-size: 0.75rem;
}

.duplicate-tags {
  display: flex;
  gap: 0.5rem;
  flex-wrap: wrap;
  margin-bottom: 0.5rem;
}

.duplicate-actions {
  display: flex;
  gap: 0.5rem;
}

.distillation-options {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.radio-option {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  cursor: pointer;
}

.custom-atomics {
  margin-top: 0.75rem;
}

.tag-list {
  display: flex;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.tag {
  padding: 0.25rem 0.5rem;
  background: #e5e7eb;
  border-radius: 3px;
  font-size: 0.85rem;
}

.tag.suggested {
  background: #dbeafe;
  color: #1e40af;
}

.footer {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-top: 1.5rem;
  padding: 1rem;
  background: #f9fafb;
  border-radius: 8px;
}

.modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.modal {
  background: white;
  border-radius: 8px;
  max-width: 500px;
  width: 90%;
  box-shadow: 0 20px 25px rgba(0, 0, 0, 0.1);
}

.modal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 1.5rem;
  border-bottom: 1px solid #e5e7eb;
}

.modal-header h3 {
  margin: 0;
}

.close-btn {
  background: none;
  border: none;
  font-size: 1.5rem;
  cursor: pointer;
  color: #6b7280;
}

.modal-body {
  padding: 1.5rem;
}

.summary-stats {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(100px, 1fr));
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.summary-item {
  text-align: center;
  padding: 1rem;
  border-radius: 4px;
}

.summary-item.success {
  background: #d1fae5;
}

.summary-item.warning {
  background: #fef3c7;
}

.summary-item.info {
  background: #dbeafe;
}

.summary-item.error {
  background: #fee2e2;
}

.summary-item .count {
  display: block;
  font-size: 2rem;
  font-weight: 700;
  margin-bottom: 0.25rem;
}

.summary-item .label {
  font-size: 0.9rem;
  color: #4b5563;
}

.notes-created {
  text-align: center;
  padding: 1rem;
  background: #f9fafb;
  border-radius: 4px;
}

.notes-created p {
  margin: 0.5rem 0;
}

.modal-footer {
  padding: 1rem 1.5rem;
  border-top: 1px solid #e5e7eb;
  text-align: center;
}

/* AI Assess Button */
.btn-ai {
  background: linear-gradient(135deg, #8b5cf6, #6366f1);
  color: white;
  font-weight: 500;
}

.btn-ai:hover:not(:disabled) {
  background: linear-gradient(135deg, #7c3aed, #4f46e5);
}

.btn-ai:disabled {
  opacity: 0.7;
  cursor: wait;
}

.toolbar-divider {
  width: 1px;
  height: 24px;
  background: #d1d5db;
  margin: 0 0.5rem;
}

/* Quality Badges */
.quality-badge {
  font-size: 0.75rem;
  padding: 2px 8px;
  border-radius: 4px;
}

.quality-badge.quality-high {
  background: linear-gradient(135deg, #34d399, #10b981);
  color: white;
}

.quality-badge.quality-medium {
  background: linear-gradient(135deg, #fbbf24, #f59e0b);
  color: white;
}

.quality-badge.quality-low {
  background: linear-gradient(135deg, #f87171, #ef4444);
  color: white;
}

.score-badge {
  font-size: 0.75rem;
  padding: 2px 6px;
  background: #e5e7eb;
  border-radius: 4px;
  color: #4b5563;
}

/* Zettelkasten Controls */
.zettel-controls {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.btn-zettel {
  background: linear-gradient(135deg, #10b981, #059669);
  color: white;
  font-weight: 500;
  border: none;
}

.btn-zettel:hover:not(:disabled) {
  background: linear-gradient(135deg, #059669, #047857);
}

.btn-ghost {
  background: #f3f4f6;
  color: #6b7280;
  border: 1px solid #e5e7eb;
}

.btn-ghost:hover:not(:disabled) {
  background: #e5e7eb;
}

.link-mode-select {
  padding: 0.4rem 0.6rem;
  border: 1px solid #d1d5db;
  border-radius: 4px;
  font-size: 0.85rem;
  background: white;
  cursor: pointer;
}

.link-mode-select:disabled {
  opacity: 0.5;
  cursor: not-allowed;
  background: #f9fafb;
}

/* Zettel Type Badges */
.zettel-type {
  font-size: 0.75rem;
  padding: 2px 8px;
  border-radius: 4px;
}

.zettel-type.concept {
  background: #dbeafe;
  color: #1e40af;
}

.zettel-type.claim {
  background: #fef3c7;
  color: #92400e;
}

.zettel-type.evidence {
  background: #d1fae5;
  color: #065f46;
}

.zettel-type.question {
  background: #fce7f3;
  color: #9f1239;
}

.zettel-type.fleche {
  background: #e0e7ff;
  color: #3730a3;
}
</style>