import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'
import './style.css'

// Show Tauri window after splash is painted (eliminates white flash)
if (window.__TAURI__) {
  requestAnimationFrame(() => {
    import('@tauri-apps/api/window').then(({ appWindow }) => appWindow.show())
  })
}

const app = createApp(App)
app.use(createPinia())
app.use(router)
app.mount('#app')

// Remove splash when initial route is fully loaded
router.isReady().then(() => {
  const splash = document.getElementById('splash-screen')
  if (splash) {
    splash.style.transition = 'opacity 0.3s ease'
    splash.style.opacity = '0'
    setTimeout(() => splash.remove(), 300)
  }
})
