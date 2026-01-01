<template>
  <div class="search-bar">
    <div class="search-input-wrapper">
      <input
        v-model="query"
        type="text"
        placeholder="Search notes..."
        @input="handleInput"
        @keydown.enter="handleEnter"
        @keydown.escape="handleEscape"
      />
      <span v-if="query" class="clear-btn" @click="handleClear">×</span>
    </div>

    <div v-if="showResults && results.length > 0" class="search-results">
      <div
        v-for="result in results"
        :key="result.id"
        class="search-result-item"
        @click="handleSelect(result.id)"
      >
        <div class="result-title">{{ result.title }}</div>
        <div class="result-score">
          <div class="score-bar" :style="{ width: `${result.score * 100}%` }"></div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, watch, onBeforeUnmount } from 'vue'
import { search as searchApi } from '../api/client'

const emit = defineEmits(['select'])

const query = ref('')
const results = ref([])
const showResults = ref(false)
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
      const data = await searchApi.query(query.value, { limit: 5 })
      results.value = data
      showResults.value = true
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
}

function handleEnter() {
  if (results.value.length > 0) {
    handleSelect(results.value[0].id)
  }
}

function handleEscape() {
  query.value = ''
  results.value = []
  showResults.value = false
}

function handleClear() {
  query.value = ''
  results.value = []
  showResults.value = false
}

// Store event handler reference
const handleClickOutside = (e) => {
  if (!e.target.closest('.search-bar')) {
    showResults.value = false
  }
}

// Add event listener on mount
document.addEventListener('click', handleClickOutside)

// Clean up on unmount
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

.search-result-item:hover {
  background: var(--bg-hover);
}

.result-title {
  font-size: 0.875rem;
  color: var(--text-primary);
  margin-bottom: 4px;
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
</style>
