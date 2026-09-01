<template>
  <section
    class="session-sheet"
    aria-labelledby="canvas-sessions-title"
  >
    <header>
      <div>
        <span>Canvas</span>
        <h2 id="canvas-sessions-title">
          Threads
        </h2>
      </div>
      <button
        type="button"
        aria-label="Create Canvas session"
        @click="$emit('create')"
      >
        New
      </button>
    </header>

    <p
      v-if="loading"
      role="status"
    >
      Loading threads…
    </p>
    <p
      v-else-if="sortedSessions.length === 0"
      class="empty-copy"
    >
      No Canvas threads yet.
    </p>

    <div class="session-list">
      <article
        v-for="session in sortedSessions"
        :key="session.id"
        class="session-row"
        :class="{ active: session.id === currentSessionId }"
        :data-session-id="session.id"
      >
        <template v-if="editingId === session.id">
          <input
            v-model="editedTitle"
            :aria-label="`Session title ${session.id}`"
            @keydown.enter="saveRename(session.id)"
            @keydown.escape="cancelRename"
          >
          <button
            type="button"
            :aria-label="`Save Canvas session ${session.id}`"
            :disabled="!editedTitle.trim()"
            @click="saveRename(session.id)"
          >
            Save
          </button>
        </template>
        <template v-else>
          <button
            type="button"
            class="session-open"
            :aria-label="`Open Canvas session ${session.id}`"
            @click="$emit('select', session.id)"
          >
            <strong>{{ session.title }}</strong>
            <small>{{ formatTime(session.updated_at) }}</small>
          </button>
          <button
            type="button"
            :aria-label="`Rename Canvas session ${session.id}`"
            @click="startRename(session)"
          >
            Rename
          </button>
          <button
            type="button"
            class="danger"
            :aria-label="`Delete Canvas session ${session.id}`"
            @click="$emit('delete', session.id)"
          >
            Delete
          </button>
        </template>
      </article>
    </div>
  </section>
</template>

<script setup>
import { computed, ref } from 'vue'

const props = defineProps({
  sessions: { type: Array, default: () => [] },
  currentSessionId: { type: String, default: null },
  loading: { type: Boolean, default: false },
})

const emit = defineEmits(['create', 'select', 'rename', 'delete'])
const editingId = ref(null)
const editedTitle = ref('')

const sortedSessions = computed(() => [...props.sessions].sort((left, right) => {
  const time = String(right.updated_at || '').localeCompare(String(left.updated_at || ''))
  return time || String(left.id).localeCompare(String(right.id))
}))

function startRename(session) {
  editingId.value = session.id
  editedTitle.value = session.title || ''
}

function cancelRename() {
  editingId.value = null
  editedTitle.value = ''
}

function saveRename(id) {
  const title = editedTitle.value.trim()
  if (!title) return
  emit('rename', { id, title })
  cancelRename()
}

function formatTime(value) {
  if (!value) return ''
  return new Date(value).toLocaleDateString()
}
</script>

<style scoped>
.session-sheet,
.session-list {
  display: grid;
  gap: var(--spacing-sm);
}

header,
.session-row {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
}

header {
  justify-content: space-between;
}

header span {
  color: var(--text-muted);
  font-size: 0.7rem;
  text-transform: uppercase;
}

h2 {
  margin: 0;
  font-size: 1.15rem;
}

button,
input {
  min-height: 44px;
  box-sizing: border-box;
  border-radius: var(--radius-md);
}

button {
  padding: 0 var(--spacing-sm);
  color: var(--text-secondary);
  background: var(--bg-tertiary);
  border: 1px solid var(--border-default);
}

header button {
  color: var(--bg-primary);
  background: var(--accent-cyan);
  border-color: transparent;
  font-weight: 700;
}

.session-row {
  padding: var(--spacing-xs);
  background: var(--bg-secondary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
}

.session-row.active {
  border-color: var(--accent-cyan);
}

.session-open {
  flex: 1;
  min-width: 0;
  display: grid;
  justify-items: start;
  text-align: left;
  background: transparent;
  border-color: transparent;
}

.session-open strong,
.session-open small {
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.session-open small {
  color: var(--text-muted);
}

.danger {
  color: var(--accent-red);
}

input {
  flex: 1;
  min-width: 0;
  padding: 0 var(--spacing-sm);
  color: var(--text-primary);
  background: var(--bg-tertiary);
  border: 1px solid var(--accent-cyan);
}
</style>
