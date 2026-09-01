<template>
  <component :is="isCompact ? CompactCanvasView : WideCanvasView" />
</template>

<script setup>
import { defineAsyncComponent } from 'vue'
import { useCompanionLayout } from '@/composables/useCompanionLayout'

const WideCanvasView = defineAsyncComponent(() => (
  import('./CanvasView.vue').then(module => module.default)
))
const CompactCanvasView = defineAsyncComponent(() => (
  import('./companion/CanvasCompanionView.vue').then(module => module.default)
))
const { isCompact } = useCompanionLayout()
</script>
