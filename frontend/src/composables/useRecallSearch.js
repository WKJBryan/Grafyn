import { onBeforeUnmount, ref, watch } from 'vue'
import { memory } from '@/api/client'

function graphBoost(result) {
  if (result.graph_boost != null) return result.graph_boost
  if (result.graphBoost != null) return result.graphBoost
  return ['graph', 'both'].includes(result.connection_type) ? 1 : 0
}

export function normalizeRecallResults(response) {
  const results = Array.isArray(response) ? response : response?.results
  if (!Array.isArray(results)) return []

  return results
    .map(result => ({
      note_id: result.note_id || result.noteId || result.id || '',
      title: result.title || 'Untitled',
      snippet: result.snippet || result.content?.slice(0, 200) || '',
      score: result.score ?? result.relevance_score ?? result.total_score ?? result.totalScore ?? 0,
      total_score: result.total_score ?? result.totalScore ?? result.relevance_score ?? result.score ?? 0,
      graph_boost: graphBoost(result),
      tags: Array.isArray(result.tags) ? result.tags : [],
    }))
    .filter(result => result.note_id)
}

export function useRecallSearch({
  search = (query, limit) => memory.recall(query, [], limit),
  debounceMs = 300,
  limit = 5,
  onError = null,
} = {}) {
  const query = ref('')
  const results = ref([])
  const status = ref('idle')
  const error = ref('')
  const completedQuery = ref('')
  let timer = null
  let generation = 0

  watch(query, value => {
    const currentGeneration = ++generation
    clearTimeout(timer)
    const normalizedQuery = value.trim()

    if (!normalizedQuery) {
      results.value = []
      status.value = 'idle'
      error.value = ''
      completedQuery.value = ''
      return
    }

    results.value = []
    status.value = 'loading'
    error.value = ''
    completedQuery.value = ''

    timer = setTimeout(async () => {
      try {
        const response = await search(normalizedQuery, limit)
        if (currentGeneration !== generation) return

        results.value = normalizeRecallResults(response)
        completedQuery.value = normalizedQuery
        status.value = results.value.length ? 'ready' : 'empty'
      } catch {
        if (currentGeneration !== generation) return

        results.value = []
        completedQuery.value = normalizedQuery
        status.value = 'error'
        error.value = 'Recall is temporarily unavailable.'
        onError?.()
      }
    }, debounceMs)
  }, { flush: 'sync' })

  onBeforeUnmount(() => {
    clearTimeout(timer)
    generation += 1
  })

  return {
    query,
    results,
    status,
    error,
    completedQuery,
  }
}
