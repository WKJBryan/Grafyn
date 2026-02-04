<template>
  <Teleport to="body">
    <div
      v-if="visible"
      class="modal-overlay"
      @click.self="close"
    >
      <div class="modal-content">
        <div class="modal-header">
          <h3>Configure Custom Atomics</h3>
          <button
            class="close-btn"
            @click="close"
          >
            &times;
          </button>
        </div>

        <div class="modal-body">
          <p class="help-text">
            Define how this conversation should be split into atomic notes.
            Each atomic note captures one idea or concept.
          </p>

          <!-- Extraction Boundaries -->
          <div class="section">
            <h4>Extraction Boundaries</h4>
            <div class="radio-group">
              <label>
                <input
                  v-model="extractionMode"
                  type="radio"
                  value="auto"
                >
                <span>Automatic (Split by topic changes)</span>
              </label>
              <label>
                <input
                  v-model="extractionMode"
                  type="radio"
                  value="headings"
                >
                <span>By Headings (Each heading becomes a note)</span>
              </label>
              <label>
                <input
                  v-model="extractionMode"
                  type="radio"
                  value="messages"
                >
                <span>By Messages (Each exchange becomes a note)</span>
              </label>
            </div>
          </div>

          <!-- Custom Atomic Notes -->
          <div class="section">
            <h4>
              Atomic Notes
              <button
                class="btn btn-sm btn-secondary"
                @click="addAtomicNote"
              >
                + Add Note
              </button>
            </h4>

            <div
              v-if="atomicNotes.length === 0"
              class="empty-state"
            >
              <p>No custom atomics defined. Add notes manually or use automatic extraction.</p>
            </div>

            <div
              v-for="(note, index) in atomicNotes"
              :key="index"
              class="atomic-card"
            >
              <div class="atomic-header">
                <input
                  v-model="note.title"
                  placeholder="Note title"
                  class="title-input"
                >
                <button
                  class="btn-icon remove-btn"
                  @click="removeAtomicNote(index)"
                >
                  &times;
                </button>
              </div>

              <textarea
                v-model="note.content"
                placeholder="Note content (markdown supported)"
                class="content-input"
                rows="3"
              />

              <div class="atomic-meta">
                <input
                  v-model="note.tagsInput"
                  placeholder="Tags (comma-separated)"
                  class="tags-input"
                >
                <select
                  v-model="note.content_type"
                  class="type-select"
                >
                  <option value="">
                    Auto-detect type
                  </option>
                  <option value="concept">
                    Concept
                  </option>
                  <option value="claim">
                    Claim
                  </option>
                  <option value="evidence">
                    Evidence
                  </option>
                  <option value="question">
                    Question
                  </option>
                  <option value="fleche">
                    Structure
                  </option>
                </select>
              </div>
            </div>
          </div>

          <!-- LLM Settings -->
          <div class="section">
            <h4>Summarization Settings</h4>
            <div class="form-row">
              <label>Detail Level:</label>
              <select v-model="detailLevel">
                <option value="brief">
                  Brief
                </option>
                <option value="standard">
                  Standard
                </option>
                <option value="detailed">
                  Detailed
                </option>
              </select>
            </div>
          </div>
        </div>

        <div class="modal-footer">
          <button
            class="btn btn-secondary"
            @click="close"
          >
            Cancel
          </button>
          <button
            class="btn btn-primary"
            @click="apply"
          >
            Apply Configuration
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script>
import { ref, watch } from 'vue'

export default {
  name: 'CustomAtomicsModal',
  props: {
    visible: { type: Boolean, default: false },
    conversationId: { type: String, default: '' },
    existingAtomics: { type: Array, default: () => [] },
  },
  emits: ['close', 'apply'],
  setup(props, { emit }) {
    const extractionMode = ref('auto')
    const detailLevel = ref('detailed')
    const atomicNotes = ref([])

    // Initialize from existing atomics when modal opens
    watch(
      () => props.visible,
      (val) => {
        if (val && props.existingAtomics.length > 0) {
          atomicNotes.value = props.existingAtomics.map((a) => ({
            title: a.title || '',
            content: a.content || '',
            tagsInput: (a.tags || []).join(', '),
            content_type: a.content_type || '',
          }))
        }
      }
    )

    const addAtomicNote = () => {
      atomicNotes.value.push({
        title: '',
        content: '',
        tagsInput: '',
        content_type: '',
      })
    }

    const removeAtomicNote = (index) => {
      atomicNotes.value.splice(index, 1)
    }

    const close = () => emit('close')

    const apply = () => {
      const atoms = atomicNotes.value
        .filter((n) => n.title.trim())
        .map((n) => ({
          title: n.title.trim(),
          content: n.content.trim(),
          tags: n.tagsInput
            .split(',')
            .map((t) => t.trim())
            .filter(Boolean),
          content_type: n.content_type || null,
        }))

      emit('apply', {
        conversationId: props.conversationId,
        extractionMode: extractionMode.value,
        detailLevel: detailLevel.value,
        customAtoms: atoms,
      })
    }

    return {
      extractionMode,
      detailLevel,
      atomicNotes,
      addAtomicNote,
      removeAtomicNote,
      close,
      apply,
    }
  },
}
</script>

<style scoped>
.modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.modal-content {
  background: var(--bg-primary, #fff);
  border-radius: 12px;
  width: 680px;
  max-width: 90vw;
  max-height: 80vh;
  display: flex;
  flex-direction: column;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.3);
}

.modal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 16px 20px;
  border-bottom: 1px solid var(--border-color, #e0e0e0);
}

.modal-header h3 {
  margin: 0;
  font-size: 1.1rem;
}

.close-btn {
  background: none;
  border: none;
  font-size: 1.5rem;
  cursor: pointer;
  color: var(--text-secondary, #666);
  padding: 0 4px;
}

.modal-body {
  padding: 20px;
  overflow-y: auto;
  flex: 1;
}

.help-text {
  color: var(--text-secondary, #666);
  font-size: 0.9rem;
  margin-bottom: 16px;
}

.section {
  margin-bottom: 20px;
}

.section h4 {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
  font-size: 0.95rem;
}

.radio-group {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.radio-group label {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  font-size: 0.9rem;
}

.empty-state {
  padding: 16px;
  text-align: center;
  color: var(--text-secondary, #888);
  background: var(--bg-secondary, #f8f8f8);
  border-radius: 8px;
  font-size: 0.85rem;
}

.atomic-card {
  border: 1px solid var(--border-color, #e0e0e0);
  border-radius: 8px;
  padding: 12px;
  margin-bottom: 10px;
}

.atomic-header {
  display: flex;
  gap: 8px;
  margin-bottom: 8px;
}

.title-input {
  flex: 1;
  padding: 6px 10px;
  border: 1px solid var(--border-color, #ddd);
  border-radius: 6px;
  font-size: 0.9rem;
  background: var(--bg-primary, #fff);
  color: var(--text-primary, #333);
}

.content-input {
  width: 100%;
  padding: 8px 10px;
  border: 1px solid var(--border-color, #ddd);
  border-radius: 6px;
  font-size: 0.85rem;
  resize: vertical;
  font-family: inherit;
  background: var(--bg-primary, #fff);
  color: var(--text-primary, #333);
}

.atomic-meta {
  display: flex;
  gap: 8px;
  margin-top: 8px;
}

.tags-input {
  flex: 1;
  padding: 6px 10px;
  border: 1px solid var(--border-color, #ddd);
  border-radius: 6px;
  font-size: 0.85rem;
  background: var(--bg-primary, #fff);
  color: var(--text-primary, #333);
}

.type-select {
  padding: 6px 10px;
  border: 1px solid var(--border-color, #ddd);
  border-radius: 6px;
  font-size: 0.85rem;
  background: var(--bg-primary, #fff);
  color: var(--text-primary, #333);
}

.btn-icon {
  background: none;
  border: none;
  cursor: pointer;
  font-size: 1.2rem;
  color: var(--text-secondary, #888);
  padding: 2px 6px;
}

.btn-icon:hover {
  color: #e74c3c;
}

.form-row {
  display: flex;
  align-items: center;
  gap: 12px;
}

.form-row label {
  font-size: 0.9rem;
  min-width: 100px;
}

.form-row select {
  padding: 6px 10px;
  border: 1px solid var(--border-color, #ddd);
  border-radius: 6px;
  background: var(--bg-primary, #fff);
  color: var(--text-primary, #333);
}

.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 14px 20px;
  border-top: 1px solid var(--border-color, #e0e0e0);
}

.btn {
  padding: 8px 16px;
  border-radius: 6px;
  font-size: 0.9rem;
  cursor: pointer;
  border: none;
}

.btn-primary {
  background: var(--accent-color, #4f46e5);
  color: white;
}

.btn-primary:hover {
  opacity: 0.9;
}

.btn-secondary {
  background: var(--bg-secondary, #f0f0f0);
  color: var(--text-primary, #333);
  border: 1px solid var(--border-color, #ddd);
}

.btn-sm {
  padding: 4px 10px;
  font-size: 0.8rem;
}

.remove-btn {
  color: #e74c3c;
}
</style>
