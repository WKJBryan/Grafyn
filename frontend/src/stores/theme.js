import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

export function resolveThemePreference(themeValue, mediaMatcher = (query) => window.matchMedia(query)) {
  if (themeValue === 'light' || themeValue === 'dark') {
    return themeValue
  }

  try {
    return mediaMatcher('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
  } catch {
    return 'dark'
  }
}

export const useThemeStore = defineStore('theme', () => {
    // Initialize theme from localStorage or system preference fallback
    const theme = ref(localStorage.getItem('theme') || resolveThemePreference('system'))

    // Watch for theme changes and persist to localStorage
    watch(theme, (newTheme) => {
        localStorage.setItem('theme', newTheme)
        applyTheme(newTheme)
    })

    // Apply theme to document
    function applyTheme(themeValue) {
        document.documentElement.setAttribute('data-theme', themeValue)
    }

    // Toggle between light and dark themes
    function toggleTheme() {
        theme.value = theme.value === 'dark' ? 'light' : 'dark'
    }

    // Set specific theme
    function setTheme(themeValue) {
        if (themeValue === 'light' || themeValue === 'dark') {
            theme.value = themeValue
        }
    }

    // Initialize theme on store creation
    applyTheme(theme.value)

    return {
        theme,
        toggleTheme,
        setTheme
    }
})
