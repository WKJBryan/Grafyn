<template>
  <div id="app">
    <router-view />
    <ToastNotification />
    <GuidePanel />
    <GuideTip />
    <Transition name="startup-splash">
      <StartupSplash
        v-if="boot.isVisible"
        :status="boot.status"
        @dismiss="boot.dismissSplash()"
      />
    </Transition>
  </div>
</template>

<script setup>
import { onBeforeUnmount, onMounted, watch } from 'vue'
import { useRoute } from 'vue-router'
import ToastNotification from '@/components/ToastNotification.vue'
import GuidePanel from '@/components/GuidePanel.vue'
import GuideTip from '@/components/GuideTip.vue'
import StartupSplash from '@/components/StartupSplash.vue'
import { useBootStore } from '@/stores/boot'
import { useGuide } from '@/composables/useGuide'

const route = useRoute()
const guide = useGuide()
const boot = useBootStore()

function handleExternalLinkClick(event) {
  let el = event.target
  while (el && el.tagName !== 'A') {
    el = el.parentElement
  }
  if (!el) return

  const href = el.getAttribute('href')
  if (href && (href.startsWith('http://') || href.startsWith('https://'))) {
    event.preventDefault()
    import('@tauri-apps/api/shell').then(({ open }) => open(href))
  }
}

onMounted(() => {
  window.dispatchEvent(new Event('grafyn-app-mounted'))
  boot.initialize()
  guide.setCurrentRoute(route.path)
  guide.checkNewFeatures()

  if (window.__TAURI__) {
    document.addEventListener('click', handleExternalLinkClick)
  }
})

let tipTimer = null
watch(() => route.path, (path) => {
  guide.setCurrentRoute(path)
  clearTimeout(tipTimer)
  tipTimer = setTimeout(() => {
    guide.showTipForRoute(path)
  }, 800)
})

onBeforeUnmount(() => {
  boot.cleanup()
  document.removeEventListener('click', handleExternalLinkClick)
})
</script>

<style>
/* Global styles are imported in main.js */
.startup-splash-enter-active,
.startup-splash-leave-active {
  transition: opacity 0.3s ease;
}

.startup-splash-enter-from,
.startup-splash-leave-to {
  opacity: 0;
}
</style>
