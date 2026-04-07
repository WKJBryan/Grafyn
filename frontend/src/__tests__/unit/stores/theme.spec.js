import { describe, expect, it, vi } from 'vitest'
import { resolveThemePreference, useThemeStore } from '@/stores/theme'
import { createTestingPinia } from '@/__tests__/setup'

describe('theme store', () => {
  it('resolves system preference through matchMedia', () => {
    const mediaMatcher = vi.fn().mockReturnValue({ matches: true })

    expect(resolveThemePreference('system', mediaMatcher)).toBe('dark')
    expect(mediaMatcher).toHaveBeenCalledWith('(prefers-color-scheme: dark)')
  })

  it('falls back to the system preference when no saved theme exists', () => {
    localStorage.clear()
    window.matchMedia = vi.fn().mockReturnValue({ matches: false })

    const pinia = createTestingPinia()
    const themeStore = useThemeStore(pinia)

    expect(themeStore.theme).toBe('light')
    expect(document.documentElement.getAttribute('data-theme')).toBe('light')
  })
})
