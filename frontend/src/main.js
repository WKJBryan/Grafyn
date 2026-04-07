import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'
import { settings } from './api/client'
import { resolveThemePreference, useThemeStore } from './stores/theme'
import './style.css'

function removeBootstrapSplash() {
  const splash = document.getElementById('splash-screen')
  if (splash) {
    splash.style.transition = 'opacity 0.3s ease'
    splash.style.opacity = '0'
    setTimeout(() => splash.remove(), 300)
  }
}

let appShellShown = false
window.addEventListener('grafyn-app-mounted', () => {
  if (appShellShown) return
  appShellShown = true

  requestAnimationFrame(() => {
    if (window.__TAURI__) {
      import('@tauri-apps/api/window').then(({ appWindow }) => appWindow.show())
    }
    removeBootstrapSplash()
  })
}, { once: true })

const pinia = createPinia()
const themeStore = useThemeStore(pinia)

async function syncThemeFromSettings() {
  try {
    const currentSettings = await settings.get()
    themeStore.setTheme(resolveThemePreference(currentSettings?.theme))
  } catch (error) {
    console.error('Failed to load theme setting:', error)
  }
}

async function bootstrap() {
  await syncThemeFromSettings()

  const app = createApp(App)
  app.use(pinia)
  app.use(router)
  app.mount('#app')
}

void bootstrap()
