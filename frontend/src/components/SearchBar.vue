<template>
  <div class="search-bar">
    <div class="search-input-wrapper">
      <input
        v-model="query"
        type="text"
        placeholder="Search notes..."
        role="combobox"
        :aria-expanded="showResults"
        aria-autocomplete="list"
        :aria-activedescendant="activeIndex >= 0 ? `search-result-${activeIndex}` : undefined"
        @input="handleInput"
        @keydown.enter="handleEnter"
        @keydown.escape="handleEscape"
        @keydown.down.prevent="handleArrowDown"
        @keydown.up.prevent="handleArrowUp"
      >
      <span
        v-if="query"
        class="clear-btn"
        @click="handleClear"
      >×</span>
    </div>

    <div
      v-if="showResults"
      class="search-results"
      role="listbox"
    >
      <template v-if="results.length > 0">
        <div
          v-for="(result, index) in results"
          :id="`search-result-${index}`"
          :key="result.note_id"
          class="search-result-item"
          :class="{ 'active': index === activeIndex }"
          role="option"
          :aria-selected="index === activeIndex"
          @click="handleSelect(result.note_id)"
          @mouseenter="activeIndex = index"
        >
          <div class="result-title">
            {{ result.title }}
            <span
              v-if="result.graph_boost > 0"
              class="graph-badge"
            >linked</span>
          </div>
          <div
            v-if="result.snippet"
            class="result-snippet"
          >
            {{ result.snippet }}
          </div>
          <div class="result-score">
            <div
              class="score-bar"
              :style="{ width: `${(result.total_score || result.score || 0) * 100}%` }"
            />
          </div>
        </div>
      </template>
      <div
        v-else
        class="no-results"
      >
        No results found for "{{ query }}"
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, onMounted, onBeforeUnmount } from 'vue'
import { memory } from '../api/client'

const emit = defineEmits(['select'])

const query = ref('')
const results = ref([])
const showResults = ref(false)
const activeIndex = ref(-1)
let debounceTimer = null

function handleInput() {
  clearTimeout(debounceTimer)
  
  if (!query.value.trim()) {
    results.value = []
    showResults.value = false
    return
  }

  debounceTimer = setTimeout(async () => {
    try {
      const data = await memory.recall(query.value, [], 5)
      const items = data.results || data // unwrap Pydantic wrapper (web) or flat array (Tauri)
      results.value = (Array.isArray(items) ? items : []).map(r => ({
        note_id: r.note_id,
        title: r.title,
        snippet: r.snippet || r.content?.substring(0, 200) || '',
        score: r.score ?? r.relevance_score ?? 0,
        total_score: r.total_score ?? r.relevance_score ?? 0,
        graph_boost: r.graph_boost ?? (r.connection_type === 'both' || r.connection_type === 'graph' ? 1 : 0),
      }))
      showResults.value = true
      activeIndex.value = -1
    } catch (error) {
      console.error('Search failed:', error)
    }
  }, 300)
}

function handleSelect(noteId) {
  emit('select', noteId)
  query.value = ''
  results.value = []
  showResults.value = false
  activeIndex.value = -1
}

function handleEnter() {
  if (activeIndex.value >= 0 && activeIndex.value < results.value.length) {
    handleSelect(results.value[activeIndex.value].note_id)
  } else if (results.value.length > 0) {
    handleSelect(results.value[0].note_id)
  }
}

function handleEscape() {
  query.value = ''
  results.value = []
  showResults.value = false
  activeIndex.value = -1
}

function handleClear() {
  query.value = ''
  results.value = []
  showResults.value = false
  activeIndex.value = -1
}

function handleArrowDown() {
  if (results.value.length > 0) {
    activeIndex.value = Math.min(activeIndex.value + 1, results.value.length - 1)
  }
}

function handleArrowUp() {
  if (results.value.length > 0) {
    activeIndex.value = Math.max(activeIndex.value - 1, 0)
  }
}

// Store event handler reference
const handleClickOutside = (e) => {
  if (!e.target.closest('.search-bar')) {
    showResults.value = false
  }
}

onMounted(() => {
  document.addEventListener('click', handleClickOutside)
})

onBeforeUnmount(() => {
  document.removeEventListener('click', handleClickOutside)
  clearTimeout(debounceTimer)
})
</script>

<style scoped>
.search-bar {
  position: relative;
  width: 100%;
}

.search-input-wrapper {
  position: relative;
}

.search-input-wrapper input {
  width: 100%;
  padding: var(--spacing-sm) var(--spacing-md);
  padding-right: 32px;
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  font-size: 0.875rem;
  transition: border-color var(--transition-fast);
}

.search-input-wrapper input:focus {
  outline: none;
  border-color: var(--accent-primary);
}

.clear-btn {
  position: absolute;
  right: var(--spacing-sm);
  top: 50%;
  transform: translateY(-50%);
  cursor: pointer;
  color: var(--text-muted);
  font-size: 1.25rem;
  line-height: 1;
  padding: 0 4px;
}

.clear-btn:hover {
  color: var(--text-primary);
}

.search-results {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
  max-height: 300px;
  overflow-y: auto;
  z-index: 1000;
}

.search-result-item {
  padding: var(--spacing-sm) var(--spacing-md);
  cursor: pointer;
  transition: background-color var(--transition-fast);
}

.search-result-item:hover,
.search-result-item.active {
  background: var(--bg-hover);
}

.result-title {
  font-size: 0.875rem;
  color: var(--text-primary);
  margin-bottom: 4px;
  display: flex;
  align-items: center;
  gap: 6px;
}

.graph-badge {
  font-size: 0.65rem;
  color: var(--accent-primary);
  background: rgba(99, 102, 241, 0.1);
  padding: 1px 6px;
  border-radius: 4px;
  flex-shrink: 0;
}

.result-snippet {
  font-size: 0.75rem;
  color: var(--text-secondary);
  margin-bottom: 4px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.result-score {
  height: 2px;
  background: var(--bg-tertiary);
  border-radius: 1px;
  overflow: hidden;
}

.score-bar {
  height: 100%;
  background: var(--accent-primary);
  transition: width var(--transition-normal);
}

.no-results {
  padding: var(--spacing-md);
  text-align: center;
  color: var(--text-muted);
  font-size: 0.875rem;
}
</style>
