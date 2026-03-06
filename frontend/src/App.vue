<template>
  <div id="app">
    <router-view />
    <ToastNotification />
    <GuidePanel />
    <GuideTip />
  </div>
</template>

<script setup>
import { onMounted, watch } from 'vue'
import { useRoute } from 'vue-router'
import ToastNotification from '@/components/ToastNotification.vue'
import GuidePanel from '@/components/GuidePanel.vue'
import GuideTip from '@/components/GuideTip.vue'
import { useGuide } from '@/composables/useGuide'

const route = useRoute()
const guide = useGuide()

onMounted(() => {
  guide.checkNewFeatures()
})

let tipTimer = null
watch(() => route.path, (path) => {
  clearTimeout(tipTimer)
  tipTimer = setTimeout(() => {
    guide.showTipForRoute(path)
  }, 800)
})
</script>

<style>
/* Global styles are imported in main.js */
</style>
