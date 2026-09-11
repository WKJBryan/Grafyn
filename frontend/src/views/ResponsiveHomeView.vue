<template>
  <HomeView v-if="!isCompact" />
  <CaptureView v-else-if="captureAvailable" />
  <CapabilityUnavailable
    v-else
    capability="notesWrite"
  />
</template>

<script setup>
import { computed, defineAsyncComponent } from 'vue'
import { getRuntimeProfile } from '@/api/transport'
import { useCompanionLayout } from '@/composables/useCompanionLayout'
import { hasCapability } from '@/platform/capabilities'
import CapabilityUnavailable from '@/components/companion/CapabilityUnavailable.vue'

const CaptureView = defineAsyncComponent(() => (
  import('@/views/companion/CaptureView.vue').then(module => module.default)
))
const HomeView = defineAsyncComponent(() => (
  import('@/views/HomeView.vue').then(module => module.default)
))

const profile = getRuntimeProfile()
const { isCompact } = useCompanionLayout({ profile })
const captureAvailable = computed(() => hasCapability(profile, 'notesWrite'))
</script>
