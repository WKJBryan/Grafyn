<template>
  <div
    v-if="modelValue"
    class="modal-overlay"
    @click.self="$emit('update:modelValue', false)"
  >
    <div class="modal-content migration-modal">
      <div class="modal-header">
        <div>
          <h2>Migrate Markdown Vault</h2>
          <p class="subtle">
            Preview-first migration for nested Markdown vaults, sidecar overlays, and Grafyn topic hubs.
          </p>
        </div>
        <button
          class="close-btn"
          @click="$emit('update:modelValue', false)"
        >
          ×
        </button>
      </div>

      <div class="modal-body">
        <div class="config-grid">
          <label class="field">
            <span>Mode</span>
            <select v-model="mode">
              <option value="sidecar_first">Sidecar First</option>
              <option value="hybrid">Hybrid</option>
              <option value="full_rewrite">Full Rewrite</option>
            </select>
          </label>

          <label class="field">
            <span>Hub Folder</span>
            <input
              v-model="hubFolder"
              type="text"
              placeholder="_grafyn/hubs"
            >
          </label>

          <label class="field">
            <span>Program File</span>
            <input
              v-model="programPath"
              type="text"
              placeholder="_grafyn/program.md"
            >
          </label>
        </div>

        <div class="checkbox-grid">
          <label class="checkbox-row">
            <input
              v-model="startOptimizer"
              type="checkbox"
            >
            <span>Start the background vault optimizer after migration</span>
          </label>
          <label class="checkbox-row">
            <input
              v-model="enableLlm"
              type="checkbox"
              :disabled="!startOptimizer"
            >
            <span>Allow optimizer LLM refinement when budget is enabled</span>
          </label>
          <label class="checkbox-row">
            <input
              v-model="autoInsertLinks"
              type="checkbox"
              :disabled="mode === 'sidecar_first'"
            >
            <span>Auto-insert exact related-note links in write-enabled modes</span>
          </label>
        </div>

        <div class="preview-actions">
          <button
            class="btn btn-secondary"
            :disabled="isScanning || !vaultPath"
            @click="scanPreview"
          >
            {{ isScanning ? 'Scanning...' : 'Scan & Preview' }}
          </button>
          <button
            class="btn btn-primary"
            :disabled="isApplying || !preview"
            @click="applyMigration"
          >
            {{ isApplying ? 'Applying...' : 'Apply Migration' }}
          </button>
          <button
            v-if="status?.rollback_available && status?.run_id"
            class="btn btn-ghost"
            :disabled="isRollingBack"
            @click="rollbackRun"
          >
            {{ isRollingBack ? 'Rolling Back...' : 'Rollback Last Run' }}
          </button>
        </div>

        <div
          v-if="preview"
          class="preview-card"
        >
          <div class="card-header">
            <div>
              <strong>Preview</strong>
              <span class="subtle"> {{ preview.mode.replace('_', ' ') }}</span>
            </div>
            <span class="subtle">{{ preview.summary.total_scanned_notes }} notes scanned</span>
          </div>

          <div class="stats-grid">
            <div
              v-for="item in previewItems"
              :key="item.label"
              class="stat-card"
            >
              <span class="stat-value">{{ item.value }}</span>
              <span class="stat-label">{{ item.label }}</span>
            </div>
          </div>

          <div class="preview-lists">
            <div class="list-card">
              <div class="list-title">Topic Hubs</div>
              <div
                v-if="preview.topic_candidates.length === 0"
                class="subtle"
              >
                No topic-hub candidates found yet.
              </div>
              <ul v-else>
                <li
                  v-for="topic in preview.topic_candidates.slice(0, 8)"
                  :key="topic.topic_key"
                >
                  {{ topic.display_name }}
                  <span class="subtle">
                    {{ topic.reuse_existing_hub_id ? 'reuse existing hub' : `${topic.member_note_ids.length} member notes` }}
                  </span>
                </li>
              </ul>
            </div>

            <div class="list-card">
              <div class="list-title">Write Candidates</div>
              <div
                v-if="writeCandidates.length === 0"
                class="subtle"
              >
                This run is metadata-only unless you enable a write mode.
              </div>
              <ul v-else>
                <li
                  v-for="proposal in writeCandidates.slice(0, 8)"
                  :key="proposal.note_id"
                >
                  {{ proposal.title }}
                  <span class="subtle">confidence {{ proposal.confidence.toFixed(2) }}</span>
                </li>
              </ul>
            </div>
          </div>
        </div>

        <div
          v-if="status && !preview"
          class="status-card subtle"
        >
          Latest run: {{ status.status || 'idle' }}
          <span v-if="status.summary"> · {{ status.summary.total_scanned_notes }} scanned</span>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import { migration as migrationApi } from '@/api/client'
import { useToast } from '@/composables/useToast'

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false,
  },
  vaultPath: {
    type: String,
    default: '',
  },
})

const emit = defineEmits(['update:modelValue', 'applied'])

const toast = useToast()
const mode = ref('sidecar_first')
const hubFolder = ref('_grafyn/hubs')
const programPath = ref('_grafyn/program.md')
const startOptimizer = ref(true)
const enableLlm = ref(false)
const autoInsertLinks = ref(false)
const isScanning = ref(false)
const isApplying = ref(false)
const isRollingBack = ref(false)
const preview = ref(null)
const status = ref(null)

const previewItems = computed(() => {
  if (!preview.value) return []
  const summary = preview.value.summary
  return [
    { label: 'No Frontmatter', value: summary.files_without_frontmatter },
    { label: 'Inferred Titles', value: summary.inferred_titles },
    { label: 'Inferred Aliases', value: summary.inferred_aliases },
    { label: 'Topic Seeds', value: summary.inferred_tags_or_topic_seeds },
    { label: 'Markdown Links', value: summary.markdown_links_resolved },
    { label: 'Wiki Links', value: summary.wikilinks_resolved },
    { label: 'Proposed Hubs', value: summary.proposed_hubs },
    { label: 'Files To Rewrite', value: summary.files_to_rewrite },
    { label: 'Files To Create', value: summary.files_to_create },
    { label: 'Grafyn Backfill', value: summary.old_grafyn_notes_eligible_for_backfill },
  ]
})

const writeCandidates = computed(() =>
  (preview.value?.note_proposals || []).filter(candidate => candidate.write_required)
)

watch(() => props.modelValue, async (open) => {
  if (open) {
    await loadStatus()
  }
})

async function loadStatus() {
  try {
    status.value = await migrationApi.status()
  } catch (error) {
    console.error('Failed to load migration status', error)
  }
}

function buildRequest() {
  return {
    mode: mode.value,
    hub_folder: hubFolder.value,
    start_optimizer: startOptimizer.value,
    enable_llm: enableLlm.value,
    auto_insert_links: autoInsertLinks.value,
    program_path: programPath.value,
  }
}

async function scanPreview() {
  if (!props.vaultPath) {
    toast.warning('Select a vault folder before scanning the migration preview.')
    return
  }

  isScanning.value = true
  try {
    preview.value = await migrationApi.preview(props.vaultPath, buildRequest())
    toast.success('Migration preview generated.')
  } catch (error) {
    console.error('Failed to preview migration', error)
    toast.error(`Migration preview failed: ${error.message}`)
  } finally {
    isScanning.value = false
  }
}

async function applyMigration() {
  if (!preview.value) return

  isApplying.value = true
  try {
    const result = await migrationApi.apply(preview.value.preview_id, buildRequest())
    toast.success(result.message || 'Migration applied.')
    await loadStatus()
    emit('applied', result)
    emit('update:modelValue', false)
  } catch (error) {
    console.error('Failed to apply migration', error)
    toast.error(`Migration apply failed: ${error.message}`)
  } finally {
    isApplying.value = false
  }
}

async function rollbackRun() {
  if (!status.value?.run_id) return

  isRollingBack.value = true
  try {
    await migrationApi.rollback(status.value.run_id)
    toast.success('Migration rolled back.')
    preview.value = null
    await loadStatus()
  } catch (error) {
    console.error('Failed to rollback migration', error)
    toast.error(`Rollback failed: ${error.message}`)
  } finally {
    isRollingBack.value = false
  }
}
</script>

<style scoped>
.migration-modal {
  width: min(960px, calc(100vw - 2rem));
  max-height: 88vh;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.subtle {
  color: var(--text-secondary);
  font-size: 0.92rem;
}

.config-grid,
.stats-grid,
.preview-lists {
  display: grid;
  gap: 0.9rem;
}

.config-grid {
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
}

.field {
  display: flex;
  flex-direction: column;
  gap: 0.45rem;
}

.field input,
.field select {
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-primary);
  color: var(--text-primary);
  padding: 0.75rem 0.85rem;
}

.checkbox-grid {
  display: grid;
  gap: 0.65rem;
  margin-top: 1rem;
}

.checkbox-row {
  display: flex;
  gap: 0.75rem;
  align-items: flex-start;
}

.preview-actions {
  display: flex;
  gap: 0.75rem;
  flex-wrap: wrap;
  margin-top: 1.2rem;
}

.preview-card,
.status-card,
.list-card {
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  padding: 1rem;
}

.card-header,
.list-title {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 0.85rem;
}

.stats-grid {
  grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
}

.stat-card {
  background: var(--bg-primary);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  padding: 0.75rem;
}

.stat-value {
  display: block;
  font-size: 1.2rem;
  font-weight: 700;
}

.stat-label {
  display: block;
  color: var(--text-secondary);
  font-size: 0.85rem;
}

.preview-lists {
  grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
  margin-top: 1rem;
}

.list-card ul {
  margin: 0;
  padding-left: 1rem;
}

@media (max-width: 720px) {
  .preview-actions {
    flex-direction: column;
  }
}
</style>
