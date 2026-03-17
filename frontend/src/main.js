import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'
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

const app = createApp(App)
app.use(createPinia())
app.use(router)
app.mount('#app')
