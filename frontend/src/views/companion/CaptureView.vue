<template>
  <main class="companion-view capture-view">
    <QuickCaptureCard
      @captured="loadRecentCaptures"
      @capture-uncertain="loadRecentCaptures"
    />

    <section
      class="recent-captures"
      aria-labelledby="recent-captures-heading"
    >
      <div class="recent-captures-heading">
        <h2 id="recent-captures-heading">
          Recent captures
        </h2>
        <span v-if="loading">Loading…</span>
      </div>

      <p
        v-if="error"
        role="alert"
      >
        {{ error }}
      </p>
      <p
        v-else-if="!loading && recentCaptures.length === 0"
        class="recent-captures-empty"
      >
        Inbox captures will appear here.
      </p>
      <ol v-else>
        <li
          v-for="note in recentCaptures"
          :key="note.id"
        >
          <span class="recent-capture-title">{{ note.title || 'Untitled' }}</span>
          <time :datetime="note.created_at">
            {{ formatDate(note.created_at) }}
          </time>
        </li>
      </ol>
    </section>
  </main>
</template>

<script setup>
import { onMounted, ref } from 'vue'
import { notes } from '@/api/client'
import QuickCaptureCard from '@/components/companion/QuickCaptureCard.vue'

const recentCaptures = ref([])
const loading = ref(false)
const error = ref('')
let loadGeneration = 0

function formatDate(value) {
  if (!value) return ''
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(new Date(value))
}

async function loadRecentCaptures() {
  const generation = ++loadGeneration
  loading.value = true
  error.value = ''
  try {
    const response = await notes.list()
    if (generation !== loadGeneration) return
    const items = Array.isArray(response) ? response : response?.notes || []
    recentCaptures.value = items
      .filter(note => Array.isArray(note.tags) && note.tags.includes('inbox'))
      .sort((left, right) => {
        const chronology = new Date(right.created_at || 0) - new Date(left.created_at || 0)
        return chronology || String(left.id).localeCompare(String(right.id))
      })
      .slice(0, 5)
  } catch {
    if (generation === loadGeneration) {
      error.value = 'Recent captures are temporarily unavailable.'
    }
  } finally {
    if (generation === loadGeneration) {
      loading.value = false
    }
  }
}

onMounted(loadRecentCaptures)
</script>

<style scoped>
.companion-view {
  width: 100%;
  max-width: 720px;
  min-width: 0;
  margin: 0 auto;
  padding: max(var(--spacing-md), env(safe-area-inset-top)) max(var(--spacing-md), env(safe-area-inset-right)) var(--spacing-xl) max(var(--spacing-md), env(safe-area-inset-left));
}

.capture-view {
  display: grid;
  gap: var(--spacing-lg);
}

.recent-captures {
  min-width: 0;
}

.recent-captures-heading {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--spacing-sm);
}

.recent-captures-heading h2 {
  margin: 0 0 var(--spacing-sm);
  font-size: 1rem;
}

.recent-captures-heading span,
.recent-captures-empty,
.recent-captures [role='alert'] {
  color: var(--text-muted);
  font-size: 0.8rem;
}

.recent-captures ol {
  list-style: none;
  display: grid;
  gap: 1px;
  overflow: hidden;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  background: var(--border-subtle);
}

.recent-captures li {
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-md);
  padding: var(--spacing-sm) var(--spacing-md);
  background: var(--bg-secondary);
}

.recent-capture-title {
  overflow: hidden;
  color: var(--text-secondary);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.recent-captures time {
  flex: none;
  color: var(--text-muted);
  font-size: 0.7rem;
}
</style>
