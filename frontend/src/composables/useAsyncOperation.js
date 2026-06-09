import { ref } from 'vue'

/**
 * Returns a `run(fn)` helper that manages a loading/error state pair.
 * Accepts an existing loading+error pair (for Pinia stores) or creates
 * a fresh pair when called with no arguments (for component-local use).
 */
export function useAsyncOperation(loading = ref(false), error = ref(null)) {
  async function run(fn) {
    loading.value = true
    error.value = null
    try {
      return await fn()
    } catch (err) {
      error.value = err.message || 'Operation failed'
      throw err
    } finally {
      loading.value = false
    }
  }
  return { run, loading, error }
}
