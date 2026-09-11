<template>
  <main class="recall-view">
    <header class="recall-header">
      <p class="recall-kicker">
        RECALL
      </p>
      <h1>Find what matters now</h1>
      <label
        class="recall-search"
        for="companion-recall-query"
      >
        <span class="sr-only">Search your notes</span>
        <GIcon
          name="search"
          :size="18"
          aria-hidden="true"
        />
        <input
          id="companion-recall-query"
          v-model="query"
          type="search"
          placeholder="Search your notes…"
          autocomplete="off"
        >
      </label>
    </header>

    <section
      v-if="selectedNote || detailStatus === 'loading' || detailError"
      class="recall-detail"
      aria-live="polite"
    >
      <button
        type="button"
        class="btn btn-ghost recall-back"
        @click="closeDetail"
      >
        <span aria-hidden="true">←</span> Back to results
      </button>

      <p
        v-if="detailStatus === 'loading'"
        data-state="loading"
      >
        Loading note…
      </p>
      <p
        v-else-if="detailError"
        role="alert"
      >
        {{ detailError }}
      </p>
      <article v-else-if="selectedNote">
        <h2>{{ selectedNote.title || 'Untitled' }}</h2>
        <!-- Content is sanitized by renderMarkdown before binding. -->
        <!-- eslint-disable vue/no-v-html -->
        <div
          class="recall-detail-body"
          v-html="selectedNoteHtml"
        />
        <!-- eslint-enable vue/no-v-html -->
      </article>
    </section>

    <section
      v-else
      class="recall-results"
      aria-live="polite"
    >
      <div
        v-if="status === 'idle'"
        class="recall-state"
        data-state="idle"
      >
        <GIcon
          name="search"
          :size="28"
          aria-hidden="true"
        />
        <h2>Search your notes</h2>
        <p>Use a name, decision, idea, or situation you remember.</p>
      </div>
      <div
        v-else-if="status === 'loading'"
        class="recall-state"
        data-state="loading"
      >
        <GIcon
          name="loader"
          icon-class="spinning"
          :size="24"
          aria-hidden="true"
        />
        <p>Looking through your graph…</p>
      </div>
      <div
        v-else-if="status === 'empty'"
        class="recall-state"
        data-state="empty"
      >
        <h2>No notes found</h2>
        <p>Try a person, topic, or phrase with different wording.</p>
      </div>
      <p
        v-else-if="status === 'error'"
        class="recall-state recall-error"
        role="alert"
      >
        {{ error }}
      </p>
      <ol v-else-if="status === 'ready'">
        <li
          v-for="result in results"
          :key="result.note_id"
        >
          <button
            type="button"
            class="recall-result"
            @click="openNote(result.note_id)"
          >
            <span
              class="recall-result-heading"
              style="min-width: 0"
            >
              <strong style="min-width: 0; overflow-wrap: anywhere">{{ result.title }}</strong>
              <span
                v-if="result.graph_boost > 0"
                class="recall-linked"
              >linked</span>
            </span>
            <span
              v-if="result.snippet"
              class="recall-snippet"
            >{{ result.snippet }}</span>
            <span
              v-if="attentionExplanation(result.note_id)"
              class="recall-attention"
            >
              <GIcon
                name="orbit"
                :size="14"
                aria-hidden="true"
              />
              {{ attentionExplanation(result.note_id) }}
            </span>
          </button>
        </li>
      </ol>
    </section>
  </main>
</template>

<script setup>
import { computed, ref, watch } from 'vue'
import { notes, twin } from '@/api/client'
import { useRecallSearch } from '@/composables/useRecallSearch'
import { renderMarkdown } from '@/utils/markdown'
import GIcon from '@/components/ui/GIcon.vue'

const { query, results, status, error, completedQuery } = useRecallSearch()
const attentionById = ref(new Map())
const selectedNote = ref(null)
const detailStatus = ref('idle')
const detailError = ref('')
let attentionGeneration = 0
let detailGeneration = 0

const selectedNoteHtml = computed(() => renderMarkdown(selectedNote.value?.content || ''))

function attentionExplanation(noteId) {
  return attentionById.value.get(noteId) || ''
}

watch(query, () => {
  attentionGeneration += 1
  attentionById.value = new Map()
  closeDetail()
})

watch(completedQuery, value => {
  if (value) void loadAttention(value)
})

async function loadAttention(searchQuery) {
  const generation = ++attentionGeneration
  const candidateNoteIds = results.value.map(result => result.note_id)
  if (candidateNoteIds.length === 0) return

  const referenceTime = new Date().toISOString()
  const request = {
    referenceTime,
    profile: 'recall',
    query: searchQuery,
    candidateNoteIds,
    relationshipVariant: { relationships: [] },
    goals: [],
    destination: 'local',
    filter: { relationships: [], goals: [], tags: [] },
    limit: 10,
  }

  let response
  try {
    response = await twin.rankAttention(request)
  } catch {
    return
  }
  if (generation !== attentionGeneration) return

  const selectedByItemId = new Map()
  const selected = response?.trace?.selected
  if (Array.isArray(selected)) {
    for (const item of selected) {
      if (item?.item_id && item.attention?.explanation) {
        selectedByItemId.set(item.item_id, item.attention.explanation)
      }
    }
  }

  const candidateIds = new Set(candidateNoteIds)
  const explanations = new Map()
  const bindings = response?.noteBindings
  if (Array.isArray(bindings)) {
    for (const binding of bindings) {
      const explanation = selectedByItemId.get(binding?.itemId)
      if (explanation && candidateIds.has(binding.noteId)) {
        explanations.set(binding.noteId, explanation)
      }
    }
  }
  attentionById.value = explanations
}

async function openNote(noteId) {
  const generation = ++detailGeneration
  detailStatus.value = 'loading'
  detailError.value = ''
  selectedNote.value = null

  try {
    const note = await notes.get(noteId)
    if (generation !== detailGeneration) return
    selectedNote.value = note
    detailStatus.value = 'ready'
  } catch {
    if (generation !== detailGeneration) return
    detailStatus.value = 'error'
    detailError.value = 'This note is temporarily unavailable.'
  }
}

function closeDetail() {
  detailGeneration += 1
  selectedNote.value = null
  detailStatus.value = 'idle'
  detailError.value = ''
}
</script>

<style scoped>
.recall-view {
  width: 100%;
  max-width: 720px;
  min-width: 0;
  min-height: 100%;
  margin: 0 auto;
  padding: max(var(--spacing-md), env(safe-area-inset-top)) max(var(--spacing-md), env(safe-area-inset-right)) var(--spacing-xl) max(var(--spacing-md), env(safe-area-inset-left));
}

.recall-header h1 {
  margin: 0 0 var(--spacing-md);
  color: var(--text-secondary);
  font-size: clamp(1.35rem, 6vw, 1.75rem);
}

.recall-kicker {
  margin: 0 0 var(--spacing-xs);
  color: var(--accent-cyan);
  font-family: 'Fira Code', monospace;
  font-size: 0.7rem;
  font-weight: 600;
  letter-spacing: 0.12em;
}

.recall-search {
  min-height: 48px;
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  padding: 0 var(--spacing-md);
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
}

.recall-search:focus-within {
  border-color: var(--accent-secondary);
  outline: 2px solid color-mix(in srgb, var(--accent-secondary) 30%, transparent);
}

.recall-search input {
  min-width: 0;
  padding: var(--spacing-sm) 0;
  background: transparent;
  border: 0;
}

.recall-search input:focus {
  border: 0;
  outline: 0;
}

.recall-results,
.recall-detail {
  margin-top: var(--spacing-lg);
}

.recall-state {
  display: grid;
  justify-items: center;
  gap: var(--spacing-sm);
  padding: var(--spacing-xl) var(--spacing-md);
  color: var(--text-muted);
  text-align: center;
}

.recall-state h2,
.recall-state p {
  margin: 0;
}

.recall-state h2 {
  color: var(--text-secondary);
  font-size: 1rem;
}

.recall-error {
  color: var(--accent-danger);
}

.recall-results ol {
  list-style: none;
  display: grid;
  gap: var(--spacing-sm);
}

.recall-result {
  width: 100%;
  min-height: 48px;
  display: grid;
  gap: var(--spacing-xs);
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.recall-result:hover,
.recall-result:focus-visible {
  background: var(--bg-tertiary);
  border-color: var(--border-strong);
}

.recall-result:focus-visible,
.recall-back:focus-visible {
  outline: 2px solid var(--accent-secondary);
  outline-offset: 2px;
}

.recall-result-heading {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  color: var(--text-secondary);
}

.recall-linked {
  flex: none;
  padding: 1px 6px;
  color: var(--accent-primary);
  border: 1px solid color-mix(in srgb, var(--accent-primary) 45%, transparent);
  border-radius: var(--radius-sm);
  font-size: 0.65rem;
}

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

.recall-snippet {
  overflow: hidden;
  color: var(--text-muted);
  font-size: 0.8rem;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.recall-attention {
  min-width: 0;
  display: flex;
  align-items: flex-start;
  gap: var(--spacing-xs);
  margin-top: var(--spacing-xs);
  color: var(--accent-cyan);
  font-size: 0.75rem;
}

.recall-attention .g-icon {
  margin-top: 2px;
}

.recall-back {
  min-height: 44px;
  margin-bottom: var(--spacing-md);
}

.recall-detail article {
  min-width: 0;
  padding: var(--spacing-md);
  background: var(--bg-secondary);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
}

.recall-detail h2 {
  overflow-wrap: anywhere;
}

.recall-detail-body {
  min-width: 0;
  overflow-wrap: anywhere;
}

.recall-detail-body :deep(img),
.recall-detail-body :deep(pre) {
  max-width: 100%;
}
</style>
