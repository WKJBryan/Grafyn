<template>
  <component :is="isCompact ? CompactTwinView : WideTwinView" />
</template>

<script setup>
import { defineAsyncComponent } from 'vue'
import { useCompanionLayout } from '@/composables/useCompanionLayout'

const WideTwinView = defineAsyncComponent(() => (
  import('./TwinReviewView.vue').then(module => module.default)
))
const CompactTwinView = defineAsyncComponent(() => (
  import('./companion/TwinCompanionView.vue').then(module => module.default)
))
const { isCompact } = useCompanionLayout()
</script>
