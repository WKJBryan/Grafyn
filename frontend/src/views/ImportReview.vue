<template>
  <div class="import-review">
    <nav class="page-nav">
      <router-link
        to="/"
        class="nav-back"
      >
        &larr; Back to Notes
      </router-link>
      <span class="nav-divider">/</span>
      <router-link
        to="/import"
        class="nav-link"
      >
        Import
      </router-link>
      <span class="nav-divider">/</span>
      <span class="nav-current">Review</span>
    </nav>

    <!-- Revert Confirmation Dialog -->
    <ConfirmDialog
      :visible="showRevertConfirm"
      title="Revert Import"
      message="Are you sure you want to revert this import? All imported notes will be deleted."
      confirm-label="Revert"
      cancel-label="Cancel"
      variant="danger"
      @confirm="confirmRevert"
      @cancel="showRevertConfirm = false"
    />

    <!-- Initial Loading State -->
    <div
      v-if="isLoading || isParsing"
      class="loading-overlay"
    >
      <div class="spinner" />
      <p>{{ isParsing ? 'Parsing file...' : 'Loading conversations...' }}</p>
    </div>

    <!-- Error State -->
    <div
      v-else-if="loadError"
      class="error-state"
    >
      <h3>⚠️ Error Loading Import</h3>
      <p>{{ loadError }}</p>
      <button
        class="btn btn-primary"
        @click="goBack"
      >
        ← Back to Upload
      </button>
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

      <div
        v-if="isApplying"
        class="loading-overlay"
      >
        <div class="spinner" />
        <p>Importing conversations...</p>
      </div>

      <div
        v-else
        class="content"
      >
        <!-- Toolbar -->
        <div class="toolbar">
          <button
            class="btn btn-sm btn-ai"
            :disabled="isAssessing"
            @click="assessQuality"
          >
            {{ isAssessing ? '🔄 Assessing...' : '🤖 AI Assess' }}
          </button>

          <!-- Zettelkasten Controls -->
          <div class="zettel-controls">
            <button
              class="btn btn-sm"
              :class="zettelEnabled ? 'btn-zettel' : 'btn-ghost'"
              :title="zettelEnabled ? 'Zettelkasten mode enabled' : 'Zettelkasten mode disabled'"
              @click="toggleZettel"
            >
              {{ zettelEnabled ? '🗃️ Zettelkasten ON' : '🗃️ Zettelkasten OFF' }}
            </button>
            <select
              v-model="linkModeValue"
              :disabled="!zettelEnabled"
              class="link-mode-select"
              title="How notes should be linked"
              @change="updateLinkMode"
            >
              <option value="automatic">
                Auto-Link
              </option>
              <option value="suggested">
                Suggest Links
              </option>
              <option value="manual">
                Manual Links
              </option>
            </select>
          </div>

          <div class="toolbar-divider" />
          <button
            class="btn btn-sm"
            @click="selectAll"
          >
            {{ allSelected ? 'Deselect All' : 'Select All' }}
          </button>
          <button
            class="btn btn-sm btn-success"
            :disabled="selectedCount === 0"
            @click="acceptSelected"
          >
            Accept ({{ selectedCount }})
          </button>
          <button
            class="btn btn-sm btn-secondary"
            :disabled="selectedCount === 0"
            @click="skipSelected"
          >
            Skip ({{ selectedCount }})
          </button>
          <button
            class="btn btn-sm btn-primary"
            @click="batchAcceptAll"
          >
            Accept All
          </button>
          <button
            class="btn btn-sm btn-warning"
            @click="batchSkipAll"
          >
            Skip All
          </button>
          <div class="spacer" />
          <button
            class="btn btn-sm"
            @click="showFilters = !showFilters"
          >
            {{ showFilters ? 'Hide Filters' : 'Show Filters' }}
          </button>
        </div>

        <!-- Filters -->
        <div
          v-if="showFilters"
          class="filters"
        >
          <label>
            <input
              v-model="filterDuplicates"
              type="checkbox"
            >
            Show only duplicates
          </label>
          <label>
            <input
              v-model="filterWithErrors"
              type="checkbox"
            >
            Show only with errors
          </label>
          <select v-model="filterByAction">
            <option value="all">
              All Actions
            </option>
            <option value="accept">
              Accept
            </option>
            <option value="skip">
              Skip
            </option>
            <option value="merge">
              Merge
            </option>
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
            <div
              class="conversation-header"
              @click="toggleSelect(conv.id)"
            >
              <input
                type="checkbox"
                :checked="selectedIds.includes(conv.id)"
                @click.stop="toggleSelect(conv.id)"
              >
              <div class="conversation-info">
                <h3 class="title">
                  {{ conv.title }}
                </h3>
                <div class="meta">
                  <span
                    class="badge"
                    :class="getDecisionBadgeClass(conv.id)"
                  >
                    {{ getDecisionLabel(conv.id) }}
                  </span>
                  <span
                    v-if="conv.duplicate_candidates.length > 0"
                    class="badge duplicate"
                  >
                    {{ conv.duplicate_candidates.length }} duplicate(s)
                  </span>
                  <!-- Quality badges -->
                  <span
                    v-if="conv.quality"
                    class="badge quality-badge"
                    :class="getQualityClass(conv.quality)"
                  >
                    {{ getQualitySuggestion(conv.quality) }}
                  </span>
                  <span
                    v-if="conv.quality"
                    class="score-badge"
                    :title="`Relevance: ${Math.round(conv.quality.relevance_score * 100)}%`"
                  >
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
              <button
                class="expand-btn"
                @click.stop="expandConversation(conv.id)"
              >
                {{ expandedIds.includes(conv.id) ? '▼' : '▶' }}
              </button>
            </div>

            <!-- Expanded Details -->
            <div
              v-if="expandedIds.includes(conv.id)"
              class="conversation-details"
            >
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
                  <div
                    v-if="conv.messages.length > 3"
                    class="more-messages"
                  >
                    ... and {{ conv.messages.length - 3 }} more messages
                  </div>
                </div>
              </div>

              <!-- Duplicate Candidates -->
              <div
                v-if="conv.duplicate_candidates.length > 0"
                class="duplicates-section"
              >
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
                      <span
                        v-if="dup.content_type"
                        class="content-type"
                      >{{ dup.content_type }}</span>
                    </div>
                    <div class="duplicate-tags">
                      <span
                        v-for="tag in dup.tags"
                        :key="tag"
                        class="tag"
                      >{{ tag }}</span>
                    </div>
                    <div class="duplicate-actions">
                      <button
                        class="btn btn-xs btn-link"
                        @click.stop="viewDuplicate(dup.note_id)"
                      >
                        View
                      </button>
                      <button
                        class="btn btn-xs btn-secondary"
                        @click.stop="mergeWith(conv.id, dup.note_id)"
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
                    >
                    <span>Container Only (No distillation)</span>
                  </label>
                  <label class="radio-option">
                    <input
                      type="radio"
                      :name="`distill-${conv.id}`"
                      value="auto_distill"
                      :checked="getDecision(conv.id).distill_option === 'auto_distill'"
                      @change="setDistillOption(conv.id, 'auto_distill')"
                    >
                    <span>Auto-Distill (Create atomic notes)</span>
                  </label>
                </div>

                <!-- Custom Atomics Toggle -->
                <div
                  v-if="getDecision(conv.id).distill_option === 'custom'"
                  class="custom-atomics"
                >
                  <button
                    class="btn btn-sm btn-secondary"
                    @click.stop="showCustomAtomics(conv.id)"
                  >
                    Configure Custom Atomics
                  </button>
                </div>
              </div>

              <!-- Tags Preview -->
              <div class="tags-section">
                <h4>Suggested Tags</h4>
                <div class="tag-list">
                  <span
                    v-for="tag in conv.suggested_tags"
                    :key="tag"
                    class="tag suggested"
                  >
                    {{ tag }}
                  </span>
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- Footer Actions -->
        <div class="footer">
          <button
            class="btn btn-secondary"
            @click="goBack"
          >
            ← Back to Upload
          </button>
          <div class="spacer" />
          <button
            class="btn btn-primary btn-lg"
            :disabled="!hasAcceptedConversations"
            @click="submitImport"
          >
            Import Selected ({{ acceptedCount }})
          </button>
        </div>
      </div>
    </template>

    <!-- Summary Modal -->
    <div
      v-if="showSummaryModal"
      class="modal-overlay"
    >
      <div class="modal">
        <div class="modal-header">
          <h3>Import Summary</h3>
          <button
            class="close-btn"
            @click="showSummaryModal = false"
          >
            ×
          </button>
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
          <button
            class="btn btn-danger"
            :disabled="isReverting"
            @click="revertImport"
          >
            {{ isReverting ? 'Reverting...' : '⟲ Revert Import' }}
          </button>
          <div class="spacer" />
          <button
            class="btn btn-primary"
            @click="viewNotes"
          >
            View Imported Notes
          </button>
        </div>
      </div>
    </div>

    <!-- Custom Atomics Modal -->
    <CustomAtomicsModal
      :visible="showCustomAtomicsModal"
      :conversation-id="customAtomicsConvId"
      @close="showCustomAtomicsModal = false"
      @apply="handleApplyCustomAtomics"
    />
  </div>
</template>

<script>
import { ref, computed, onMounted } from 'vue'
import { useImportStore } from '@/stores/import'
import { useRouter } from 'vue-router'
import { useToast } from '@/composables/useToast'
import CustomAtomicsModal from '@/components/import/CustomAtomicsModal.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'

export default {
  name: 'ImportReview',
  components: { CustomAtomicsModal, ConfirmDialog },

  setup() {
    const router = useRouter()
    const importStore = useImportStore()
    
    const toast = useToast()

    // Local state
    const showRevertConfirm = ref(false)
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

    const customAtomicsConvId = ref(null)
    const showCustomAtomicsModal = ref(false)

    const showCustomAtomics = (id) => {
      customAtomicsConvId.value = id
      showCustomAtomicsModal.value = true
    }

    const handleApplyCustomAtomics = (config) => {
      if (!config.conversationId) return
      const atoms = config.customAtoms.map(a => ({
        title: a.title,
        content: a.content,
        tags: a.tags,
        content_type: a.content_type,
      }))
      importStore.updateDecision(config.conversationId, {
        distill_option: 'custom',
        custom_atoms: atoms.length > 0 ? atoms : null,
        summarization_settings: {
          model_id: 'anthropic/claude-3.5-sonnet',
          detail_level: config.detailLevel,
          max_tokens: 4096,
          temperature: 0.3,
        },
      })
      showCustomAtomicsModal.value = false
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
      showRevertConfirm.value = true
    }

    const confirmRevert = async () => {
      showRevertConfirm.value = false
      try {
        await importStore.revertLastImport()
        showSummaryModal.value = false
        toast.success('Import reverted. All imported notes have been deleted.')
        router.push('/import')
      } catch (error) {
        console.error('Revert failed:', error)
        toast.error('Failed to revert import: ' + error.message)
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
        toast.error('Quality assessment failed: ' + error.message)
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
      showCustomAtomicsModal,
      customAtomicsConvId,
      handleApplyCustomAtomics,
      submitImport,
      viewNotes,
      goBack,
      revertImport,
      confirmRevert,
      showRevertConfirm,
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
  padding: var(--spacing-md);
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.page-nav {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  margin-bottom: var(--spacing-lg);
}

.nav-back,
.nav-link {
  color: var(--text-secondary);
  text-decoration: none;
  font-size: 0.875rem;
  padding: var(--spacing-xs) var(--spacing-sm);
  border-radius: var(--radius-sm);
  transition: all var(--transition-fast);
}

.nav-back:hover,
.nav-link:hover {
  color: var(--accent-primary);
  background: var(--bg-hover);
}

.nav-divider {
  color: var(--text-muted);
  font-size: 0.75rem;
}

.nav-current {
  color: var(--text-muted);
  font-size: 0.875rem;
}

.header {
  margin-bottom: var(--spacing-lg);
}

.header h2 {
  font-size: 1.5rem;
  margin-bottom: var(--spacing-md);
  color: var(--text-primary);
}

.stats {
  display: flex;
  gap: var(--spacing-xl);
  flex-wrap: wrap;
}

.stat-item {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
}

.stat-item .label {
  font-size: 0.85rem;
  color: var(--text-muted);
}

.stat-item .value {
  font-size: 1.2rem;
  font-weight: 600;
  color: var(--text-primary);
}

.stat-item .value.warning {
  color: var(--accent-warning);
}

.stat-item .value.success {
  color: var(--accent-success);
}

.loading-overlay {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 4rem;
  color: var(--text-primary);
}

.spinner {
  width: 40px;
  height: 40px;
  border: 4px solid var(--bg-tertiary);
  border-top: 4px solid var(--accent-secondary);
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
  margin-bottom: var(--spacing-md);
  color: var(--accent-danger);
}

.error-state p {
  margin-bottom: var(--spacing-lg);
  color: var(--text-muted);
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: var(--spacing-lg);
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border-radius: var(--radius-md);
}

.spacer {
  flex: 1;
}

.btn {
  padding: 0.5rem 1rem;
  border: none;
  border-radius: var(--radius-sm);
  font-size: 0.9rem;
  cursor: pointer;
  transition: all var(--transition-fast);
  color: var(--text-primary);
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
  background: var(--accent-primary);
  color: white;
}

.btn-primary:hover:not(:disabled) {
  opacity: 0.9;
}

.btn-secondary {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

.btn-secondary:hover:not(:disabled) {
  background: var(--bg-hover);
}

.btn-danger {
  background: var(--accent-danger);
  color: white;
}

.btn-danger:hover:not(:disabled) {
  opacity: 0.9;
}

.btn-success {
  background: var(--accent-success);
  color: white;
}

.btn-success:hover:not(:disabled) {
  opacity: 0.9;
}

.btn-warning {
  background: var(--accent-warning);
  color: white;
}

.btn-link {
  background: transparent;
  color: var(--accent-secondary);
  border: 1px solid var(--accent-secondary);
}

.btn-link:hover {
  background: rgba(38, 139, 210, 0.1);
}

.filters {
  display: flex;
  gap: var(--spacing-lg);
  margin-bottom: var(--spacing-md);
  padding: 0.75rem;
  background: var(--bg-secondary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
}

.filters label {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  cursor: pointer;
}

.filters select {
  background: var(--bg-tertiary);
  color: var(--text-primary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  padding: 0.25rem 0.5rem;
}

.conversations-list {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-md);
  flex: 1;
  overflow-y: auto;
  padding-right: 0.5rem;
  max-height: calc(100vh - 320px);
}

.conversation-item {
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  background: var(--bg-secondary);
  overflow: hidden;
  transition: all var(--transition-fast);
  flex-shrink: 0;
}

.conversation-item.selected {
  border-color: var(--accent-secondary);
  box-shadow: 0 0 0 3px rgba(38, 139, 210, 0.2);
}

.conversation-item.has-duplicates {
  border-left: 4px solid var(--accent-warning);
}

.conversation-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-md);
  padding: 0.75rem var(--spacing-md);
  cursor: pointer;
  background: var(--bg-secondary);
  transition: background var(--transition-fast);
}

.conversation-header:hover {
  background: var(--bg-hover);
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
  color: var(--text-primary);
}

.meta {
  display: flex;
  gap: 0.5rem;
  flex-wrap: wrap;
  align-items: center;
}

.badge {
  padding: 0.25rem 0.5rem;
  border-radius: var(--radius-sm);
  font-size: 0.75rem;
  font-weight: 600;
}

.badge-success {
  background: rgba(133, 153, 0, 0.15);
  color: var(--accent-success);
}

.badge-secondary {
  background: var(--bg-tertiary);
  color: var(--text-secondary);
}

.badge-info {
  background: rgba(38, 139, 210, 0.15);
  color: var(--accent-blue, var(--accent-secondary));
}

.badge-warning {
  background: rgba(181, 137, 0, 0.15);
  color: var(--accent-warning);
}

.badge.duplicate {
  background: rgba(181, 137, 0, 0.15);
  color: var(--accent-warning);
}

.messages-count,
.date {
  font-size: 0.85rem;
  color: var(--text-muted);
}

.expand-btn {
  padding: 0.25rem 0.5rem;
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
  color: var(--text-secondary);
}

.expand-btn:hover {
  background: var(--bg-hover);
}

.conversation-details {
  padding: var(--spacing-md);
  border-top: 1px solid var(--bg-tertiary);
}

.messages-preview,
.duplicates-section,
.action-options,
.tags-section {
  margin-bottom: var(--spacing-md);
}

.messages-preview h4,
.duplicates-section h4,
.action-options h4,
.tags-section h4 {
  margin: 0 0 0.75rem 0;
  font-size: 0.95rem;
  color: var(--text-secondary);
}

.message-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.message-preview-item {
  padding: 0.5rem;
  border-radius: var(--radius-sm);
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

.message-preview-item.user {
  border-left: 3px solid var(--accent-violet, #6c71c4);
}

.message-preview-item.assistant {
  border-left: 3px solid var(--accent-success);
}

.message-text {
  color: var(--text-secondary);
}

.more-messages {
  color: var(--text-muted);
  font-style: italic;
}

.duplicate-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.duplicate-item {
  padding: 0.75rem;
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  background: rgba(181, 137, 0, 0.1);
}

.duplicate-info {
  display: flex;
  justify-content: space-between;
  margin-bottom: 0.5rem;
  color: var(--text-primary);
}

.similarity {
  color: var(--accent-warning);
  font-weight: 600;
}

.content-type {
  padding: 0.25rem 0.5rem;
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  font-size: 0.75rem;
  color: var(--text-secondary);
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
  color: var(--text-primary);
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
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  font-size: 0.85rem;
  color: var(--text-secondary);
}

.tag.suggested {
  background: rgba(38, 139, 210, 0.15);
  color: var(--accent-blue, var(--accent-secondary));
}

.footer {
  display: flex;
  align-items: center;
  gap: var(--spacing-md);
  margin-top: var(--spacing-lg);
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border-radius: var(--radius-md);
}

.modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.modal {
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-lg);
  max-width: 500px;
  width: 90%;
  box-shadow: 0 20px 25px rgba(0, 0, 0, 0.3);
}

.modal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--spacing-lg);
  border-bottom: 1px solid var(--bg-tertiary);
}

.modal-header h3 {
  margin: 0;
  color: var(--text-primary);
}

.close-btn {
  background: none;
  border: none;
  font-size: 1.5rem;
  cursor: pointer;
  color: var(--text-muted);
  transition: color var(--transition-fast);
}

.close-btn:hover {
  color: var(--text-primary);
}

.modal-body {
  padding: var(--spacing-lg);
}

.summary-stats {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(100px, 1fr));
  gap: var(--spacing-md);
  margin-bottom: var(--spacing-lg);
}

.summary-item {
  text-align: center;
  padding: var(--spacing-md);
  border-radius: var(--radius-sm);
}

.summary-item.success {
  background: rgba(133, 153, 0, 0.15);
}

.summary-item.warning {
  background: rgba(181, 137, 0, 0.15);
}

.summary-item.info {
  background: rgba(38, 139, 210, 0.15);
}

.summary-item.error {
  background: rgba(220, 50, 47, 0.15);
}

.summary-item .count {
  display: block;
  font-size: 2rem;
  font-weight: 700;
  margin-bottom: 0.25rem;
  color: var(--text-primary);
}

.summary-item .label {
  font-size: 0.9rem;
  color: var(--text-secondary);
}

.notes-created {
  text-align: center;
  padding: var(--spacing-md);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
}

.notes-created p {
  margin: 0.5rem 0;
}

.modal-footer {
  padding: var(--spacing-md) var(--spacing-lg);
  border-top: 1px solid var(--bg-tertiary);
  display: flex;
  align-items: center;
}

/* AI Assess Button */
.btn-ai {
  background: linear-gradient(135deg, var(--accent-violet, #6c71c4), var(--accent-blue, #268bd2));
  color: white;
  font-weight: 500;
}

.btn-ai:hover:not(:disabled) {
  opacity: 0.9;
}

.btn-ai:disabled {
  opacity: 0.7;
  cursor: wait;
}

.toolbar-divider {
  width: 1px;
  height: 24px;
  background: var(--bg-tertiary);
  margin: 0 0.5rem;
}

/* Quality Badges */
.quality-badge {
  font-size: 0.75rem;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
}

.quality-badge.quality-high {
  background: var(--accent-success);
  color: white;
}

.quality-badge.quality-medium {
  background: var(--accent-warning);
  color: white;
}

.quality-badge.quality-low {
  background: var(--accent-danger);
  color: white;
}

.score-badge {
  font-size: 0.75rem;
  padding: 2px 6px;
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
}

/* Zettelkasten Controls */
.zettel-controls {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.btn-zettel {
  background: var(--accent-success);
  color: white;
  font-weight: 500;
  border: none;
}

.btn-zettel:hover:not(:disabled) {
  opacity: 0.9;
}

.btn-ghost {
  background: var(--bg-tertiary);
  color: var(--text-muted);
  border: 1px solid var(--bg-tertiary);
}

.btn-ghost:hover:not(:disabled) {
  background: var(--bg-hover);
}

.link-mode-select {
  padding: 0.4rem 0.6rem;
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  font-size: 0.85rem;
  background: var(--bg-secondary);
  color: var(--text-primary);
  cursor: pointer;
}

.link-mode-select:disabled {
  opacity: 0.5;
  cursor: not-allowed;
  background: var(--bg-tertiary);
}

/* Zettel Type Badges */
.zettel-type {
  font-size: 0.75rem;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
}

.zettel-type.concept {
  background: rgba(38, 139, 210, 0.15);
  color: var(--accent-blue, var(--accent-secondary));
}

.zettel-type.claim {
  background: rgba(181, 137, 0, 0.15);
  color: var(--accent-warning);
}

.zettel-type.evidence {
  background: rgba(133, 153, 0, 0.15);
  color: var(--accent-success);
}

.zettel-type.question {
  background: rgba(211, 54, 130, 0.15);
  color: var(--accent-magenta, #d33682);
}

.zettel-type.fleche {
  background: rgba(108, 113, 196, 0.15);
  color: var(--accent-violet, #6c71c4);
}
</style>