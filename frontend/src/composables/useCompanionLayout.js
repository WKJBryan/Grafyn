import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { getRuntimeProfile } from '@/api/transport'
import { RUNTIME_PROFILES } from '@/platform/runtime'

const COMPACT_QUERY = '(max-width: 767px)'

export function useCompanionLayout({
  profile = getRuntimeProfile(),
  matchMedia = globalThis.window?.matchMedia?.bind(globalThis.window),
} = {}) {
  const mediaQuery = matchMedia?.(COMPACT_QUERY)
  const viewportIsCompact = ref(Boolean(mediaQuery?.matches))

  const handleChange = event => {
    viewportIsCompact.value = event.matches
  }

  onMounted(() => {
    mediaQuery?.addEventListener?.('change', handleChange)
  })

  onBeforeUnmount(() => {
    mediaQuery?.removeEventListener?.('change', handleChange)
  })

  const isCompact = computed(() => (
    profile.name === RUNTIME_PROFILES.ANDROID_COMPACT || viewportIsCompact.value
  ))

  return {
    isCompact,
    isWide: computed(() => !isCompact.value),
  }
}
