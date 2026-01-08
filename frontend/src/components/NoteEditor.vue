<template>
  <div class="note-editor">
    <div class="editor-header">
      <input
        v-model="localNote.title"
        type="text"
        class="title-input"
        placeholder="Note title..."
        @input="handleDirty"
      />
      <div class="editor-actions">
        <button
          class="btn btn-secondary"
          :disabled="!isDirty"
          @click="handleSave"
        >
          Save
        </button>
        <button
          v-if="note.id"
          class="btn btn-ghost"
          @click="handleDelete"
        >
          Delete
        </button>
      </div>
    </div>

    <div class="editor-tabs">
      <button
        :class="['tab-btn', { active: mode === 'edit' }]"
        @click="mode = 'edit'"
      >
        Edit
      </button>
      <button
        :class="['tab-btn', { active: mode === 'preview' }]"
        @click="mode = 'preview'"
      >
        Preview
      </button>
    </div>

    <div class="editor-content">
      <textarea
        v-if="mode === 'edit'"
        v-model="localNote.content"
        class="editor-textarea"
        placeholder="Write your note in Markdown..."
        @input="handleDirty"
      />
      <div v-else class="editor-preview" v-html="renderedContent"></div>
    </div>

    <div class="editor-footer">
      <select v-model="localNote.status" class="status-select" @change="handleDirty">
        <option value="draft">Draft</option>
        <option value="canonical">Canonical</option>
        <option value="evidence">Evidence</option>
      </select>
      <input
        v-model="tagsInput"
        type="text"
        class="tags-input"
        placeholder="Tags (comma-separated)"
        @input="handleTagsInput"
      />
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { marked } from 'marked'

const props = defineProps({
  note: {
    type: Object,
    required: true
  }
})

const emit = defineEmits(['save', 'delete'])

const localNote = ref({ ...props.note })
const mode = ref('edit')
const isDirty = ref(false)
const tagsInput = ref(props.note.tags ? props.note.tags.join(', ') : '')

// Watch for prop changes
watch(() => props.note, (newNote) => {
  localNote.value = { ...newNote }
  tagsInput.value = newNote.tags ? newNote.tags.join(', ') : ''
  isDirty.value = false
}, { deep: true })

const renderedContent = computed(() => {
  if (!localNote.value.content) return ''
  
  let html = marked(localNote.value.content)
  
  // Render wikilinks
  html = html.replace(
    /\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g,
    (match, target, display) => {
      const text = display || target
      return `<span class="wikilink" data-target="${target}">${text}</span>`
    }
  )
  
  return html
})

function handleDirty() {
  isDirty.value = true
}

function handleTagsInput() {
  const tags = tagsInput.value
    .split(',')
    .map(tag => tag.trim())
    .filter(tag => tag.length > 0)
  localNote.value.tags = tags
  isDirty.value = true
}

function handleSave() {
  if (!localNote.value.title.trim()) {
    alert('Please enter a title')
    return
  }
  
  emit('save', props.note.id, {
    title: localNote.value.title,
    content: localNote.value.content,
    status: localNote.value.status,
    tags: localNote.value.tags
  })
  isDirty.value = false
}

function handleDelete() {
  if (confirm('Are you sure you want to delete this note?')) {
    emit('delete', props.note.id)
  }
}
</script>

<style scoped>
.note-editor {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--bg-secondary);
  border-radius: var(--radius-md);
  overflow: hidden;
}

.editor-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md);
  border-bottom: 1px solid var(--bg-tertiary);
  gap: var(--spacing-md);
}

.title-input {
  flex: 1;
  font-size: 1.25rem;
  font-weight: 600;
  background: transparent;
  border: none;
  color: var(--text-primary);
  padding: 0;
}

.title-input:focus {
  outline: none;
}

.title-input::placeholder {
  color: var(--text-muted);
}

.editor-actions {
  display: flex;
  gap: var(--spacing-sm);
  flex-shrink: 0;
}

.editor-tabs {
  display: flex;
  border-bottom: 1px solid var(--bg-tertiary);
}

.tab-btn {
  flex: 1;
  padding: var(--spacing-sm) var(--spacing-md);
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: var(--text-secondary);
  font-size: 0.875rem;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.tab-btn:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
}

.tab-btn.active {
  color: var(--accent-primary);
  border-bottom-color: var(--accent-primary);
}

.editor-content {
  flex: 1;
  overflow: auto;
  padding: var(--spacing-md);
}

.editor-textarea {
  width: 100%;
  height: 100%;
  background: transparent;
  border: none;
  color: var(--text-primary);
  font-family: 'Fira Code', monospace;
  font-size: 0.875rem;
  line-height: 1.6;
  resize: none;
  padding: 0;
}

.editor-textarea:focus {
  outline: none;
}

.editor-preview {
  color: var(--text-primary);
  line-height: 1.6;
}

.editor-preview :deep(*) {
  color: inherit;
}

.editor-preview :deep(h1),
.editor-preview :deep(h2),
.editor-preview :deep(h3),
.editor-preview :deep(h4),
.editor-preview :deep(h5),
.editor-preview :deep(h6) {
  margin-top: var(--spacing-lg);
  margin-bottom: var(--spacing-md);
  font-weight: 600;
  color: var(--text-primary);
}

.editor-preview :deep(h1) { font-size: 2rem; }
.editor-preview :deep(h2) { font-size: 1.5rem; }
.editor-preview :deep(h3) { font-size: 1.25rem; }
.editor-preview :deep(h4) { font-size: 1.125rem; }

.editor-preview :deep(p) {
  margin-bottom: var(--spacing-md);
  color: var(--text-primary);
}

.editor-preview :deep(ul),
.editor-preview :deep(ol) {
  margin-bottom: var(--spacing-md);
  padding-left: var(--spacing-lg);
  color: var(--text-primary);
}

.editor-preview :deep(li) {
  margin-bottom: var(--spacing-xs);
  color: var(--text-primary);
}

.editor-preview :deep(code) {
  background: var(--bg-tertiary);
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  font-size: 0.9em;
  color: var(--text-primary);
}

.editor-preview :deep(pre) {
  background: var(--bg-tertiary);
  padding: var(--spacing-md);
  border-radius: var(--radius-md);
  overflow-x: auto;
  margin-bottom: var(--spacing-md);
  color: var(--text-primary);
}

.editor-preview :deep(pre code) {
  background: none;
  padding: 0;
  color: var(--text-primary);
}

.editor-preview :deep(blockquote) {
  border-left: 3px solid var(--accent-primary);
  padding-left: var(--spacing-md);
  margin-left: 0;
  color: var(--text-secondary);
  margin-bottom: var(--spacing-md);
}

.editor-preview :deep(a) {
  color: var(--accent-primary);
  text-decoration: none;
}

.editor-preview :deep(a:hover) {
  text-decoration: underline;
}

.editor-footer {
  display: flex;
  gap: var(--spacing-md);
  padding: var(--spacing-md);
  border-top: 1px solid var(--bg-tertiary);
}

.status-select {
  width: auto;
  min-width: 120px;
}

.tags-input {
  flex: 1;
}
</style>
