import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

export const useThemeStore = defineStore('theme', () => {
    // Initialize theme from localStorage or default to 'dark'
    const theme = ref(localStorage.getItem('theme') || 'dark')

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
