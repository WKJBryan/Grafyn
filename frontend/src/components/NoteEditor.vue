<template>
  <div class="note-editor">
    <div class="editor-header">
      <div class="editor-actions">
        <button
          v-if="canDistill"
          class="btn btn-accent"
          :disabled="isDistilling"
          @click="handleDistill"
          title="Extract atomic notes from this container"
        >
          {{ isDistilling ? '⏳ Distilling...' : '⚗️ Distill' }}
        </button>
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

    <!-- Distill Status Message -->
    <div v-if="distillMessage" class="distill-message" :class="{ error: distillMessage.includes('failed') }">
      {{ distillMessage }}
    </div>
  </div>
</template>

<script setup>
import { ref, computed, watch } from 'vue'
import { marked } from 'marked'
import { notes } from '@/api/client'

const props = defineProps({
  note: {
    type: Object,
    required: true
  }
})

const emit = defineEmits(['save', 'delete', 'distill-success', 'close'])

const localNote = ref({ ...props.note })
const mode = ref('preview')
const isDirty = ref(false)
const tagsInput = ref(props.note.tags ? props.note.tags.join(', ') : '')
const isDistilling = ref(false)
const distillMessage = ref('')

// Computed: can distill if status is evidence, has canvas-export tag, or source is mcp
const canDistill = computed(() => {
  const status = localNote.value.frontmatter?.status || localNote.value.status
  const tags = localNote.value.frontmatter?.tags || localNote.value.tags || []
  const source = localNote.value.frontmatter?.source
  return status === 'evidence' || tags.includes('canvas-export') || source === 'mcp'
})

// Watch for prop changes
watch(() => props.note, (newNote) => {
  localNote.value = { ...newNote }
  tagsInput.value = newNote.tags ? newNote.tags.join(', ') : ''
  isDirty.value = false
}, { deep: true })

const renderedContent = computed(() => {
  if (!localNote.value.content) return ''
  
  // Configure marked to add IDs to headings for On This Page navigation
  marked.use({
    renderer: {
      heading(text, level) {
        const id = text.toLowerCase().replace(/[^\w]+/g, '-')
        return `<h${level} id="${id}">${text}</h${level}>`
      }
    }
  })
  
  let html = marked(localNote.value.content)
  
  // Render embed syntax ![[Note]] - these become embedded content placeholders
  html = html.replace(
    /!\[\[([^\]|#]+)(?:#([^\]|]+))?(?:\|([^\]]+))?\]\]/g,
    (match, target, anchor, display) => {
      const anchorAttr = anchor ? ` data-anchor="${anchor}"` : ''
      return `<div class="embed-placeholder" data-target="${target}"${anchorAttr}>
        <span class="embed-icon">📄</span>
        <span class="embed-title">${display || target}${anchor ? '#' + anchor : ''}</span>
        <span class="embed-hint">Click to view embedded content</span>
      </div>`
    }
  )
  
  // Render wikilinks with optional heading anchors
  // [[Note#Heading]] or [[Note#^block-id]] or [[Note|Display]]
  html = html.replace(
    /\[\[([^\]|#]+)(?:#([^\]|]+))?(?:\|([^\]]+))?\]\]/g,
    (match, target, anchor, display) => {
      const text = display || (anchor ? `${target}#${anchor}` : target)
      const anchorAttr = anchor ? ` data-anchor="${anchor}"` : ''
      return `<span class="wikilink" data-target="${target}"${anchorAttr}>${text}</span>`
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
    tags: localNote.value.tags,
    note_type: localNote.value.note_type
  })
  isDirty.value = false
}

function handleDelete() {
  if (confirm('Are you sure you want to delete this note?')) {
    emit('delete', props.note.id)
  }
}

async function handleDistill() {
  if (isDistilling.value) return
  
  isDistilling.value = true
  distillMessage.value = ''
  
  try {
    const result = await notes.distill(props.note.id, { mode: 'auto' })
    
    if (result.created_note_ids?.length > 0) {
      distillMessage.value = `✓ Created ${result.created_note_ids.length} draft notes`
      emit('distill-success', result)
    } else {
      distillMessage.value = result.message || 'No atomic notes found to extract'
    }
    
    // Show success briefly
    setTimeout(() => {
      distillMessage.value = ''
    }, 3000)
  } catch (error) {
    console.error('Distill failed:', error)
    distillMessage.value = error.response?.data?.detail || 'Distillation failed'
  } finally {
    isDistilling.value = false
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
  justify-content: flex-end;
  padding: var(--spacing-md);
  border-bottom: 1px solid var(--bg-tertiary);
  gap: var(--spacing-sm);
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

/* Table styles */
.editor-preview :deep(table) {
  width: 100%;
  border-collapse: collapse;
  margin-bottom: var(--spacing-md);
  overflow-x: auto;
  display: block;
}

.editor-preview :deep(table thead) {
  border-bottom: 2px solid var(--bg-tertiary);
}

.editor-preview :deep(table th) {
  padding: var(--spacing-sm) var(--spacing-md);
  text-align: left;
  font-weight: 600;
  background: var(--bg-tertiary);
  color: var(--text-primary);
  border-bottom: 1px solid var(--bg-tertiary);
}

.editor-preview :deep(table th[align="center"]) {
  text-align: center;
}

.editor-preview :deep(table th[align="right"]) {
  text-align: right;
}

.editor-preview :deep(table tbody tr) {
  border-bottom: 1px solid var(--bg-tertiary);
  transition: background var(--transition-fast);
}

.editor-preview :deep(table tbody tr:last-child) {
  border-bottom: none;
}

.editor-preview :deep(table tbody tr:hover) {
  background: var(--bg-hover);
}

.editor-preview :deep(table td) {
  padding: var(--spacing-sm) var(--spacing-md);
  color: var(--text-primary);
  border-bottom: 1px solid var(--bg-tertiary);
}

.editor-preview :deep(table td[align="center"]) {
  text-align: center;
}

.editor-preview :deep(table td[align="right"]) {
  text-align: right;
}

.editor-preview :deep(table code) {
  background: var(--bg-tertiary);
  padding: 2px 6px;
  border-radius: var(--radius-sm);
  font-size: 0.9em;
  color: var(--text-primary);
}

.editor-preview :deep(table pre) {
  background: var(--bg-tertiary);
  padding: var(--spacing-sm);
  border-radius: var(--radius-sm);
  margin: 0;
  overflow-x: auto;
}

/* Responsive table container for mobile */
.editor-preview :deep(.table-wrapper) {
  overflow-x: auto;
  -webkit-overflow-scrolling: touch;
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

.distill-message {
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--accent-primary);
  color: white;
  font-size: 0.875rem;
  text-align: center;
  animation: fadeIn 0.3s ease;
}

.distill-message.error {
  background: var(--error-bg, #dc2626);
}

@keyframes fadeIn {
  from { opacity: 0; transform: translateY(-10px); }
  to { opacity: 1; transform: translateY(0); }
}

/* Wikilink styles */
.editor-preview :deep(.wikilink) {
  color: var(--accent-primary);
  cursor: pointer;
  text-decoration: none;
  border-bottom: 1px dashed var(--accent-primary);
  transition: all var(--transition-fast);
}

.editor-preview :deep(.wikilink:hover) {
  background: var(--accent-primary);
  color: var(--bg-primary);
  border-radius: 2px;
  border-bottom-color: transparent;
}

/* Wikilink with anchor indicator */
.editor-preview :deep(.wikilink[data-anchor]) {
  position: relative;
}

.editor-preview :deep(.wikilink[data-anchor])::after {
  content: '§';
  font-size: 0.75em;
  margin-left: 2px;
  opacity: 0.6;
}

/* Embed placeholder styles */
.editor-preview :deep(.embed-placeholder) {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--spacing-xs);
  padding: var(--spacing-md);
  margin: var(--spacing-md) 0;
  background: var(--bg-tertiary);
  border: 1px dashed var(--accent-primary);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.editor-preview :deep(.embed-placeholder:hover) {
  background: var(--accent-primary);
  border-style: solid;
}

.editor-preview :deep(.embed-placeholder:hover) * {
  color: white;
}

.editor-preview :deep(.embed-icon) {
  font-size: 1.5rem;
}

.editor-preview :deep(.embed-title) {
  font-weight: 600;
  color: var(--accent-primary);
}

.editor-preview :deep(.embed-hint) {
  font-size: 0.75rem;
  color: var(--text-muted);
}
</style>
