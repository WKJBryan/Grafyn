<template>
  <div
    v-if="headers.length > 0"
    class="on-this-page"
  >
    <div class="otp-header">
      On this page
    </div>
    <div class="otp-list">
      <div 
        v-for="(header, index) in headers" 
        :key="index"
        class="otp-item"
        :class="[`level-${header.level}`]"
        @click="scrollToHeader(header.id)"
      >
        {{ header.text }}
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  content: {
    type: String,
    default: ''
  }
})

const headers = computed(() => {
  if (!props.content) return []
  
  const regex = /^(#{1,6})\s+(.+)$/gm
  const matches = []
  let match
  
  while ((match = regex.exec(props.content)) !== null) {
    matches.push({
      level: match[1].length,
      text: match[2],
      id: match[2].toLowerCase().replace(/[^\w]+/g, '-')
    })
  }
  
  return matches
})

function scrollToHeader(id) {
  // Simple scroll to element logic would go here
  // For now, we just emit or log (functionality depends on how headers are rendered in NoteEditor)
  const element = document.getElementById(id)
  if (element) {
    element.scrollIntoView({ behavior: 'smooth' })
  }
}
</script>

<style scoped>
.on-this-page {
  padding: var(--spacing-md) 0;
}

.otp-header {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
  margin-bottom: var(--spacing-sm);
  padding-left: var(--spacing-md);
}

.otp-list {
  display: flex;
  flex-direction: column;
}

.otp-item {
  font-size: 0.85rem;
  color: var(--text-secondary);
  padding: 4px var(--spacing-md);
  cursor: pointer;
  border-left: 2px solid transparent;
  transition: all var(--transition-fast);
}

.otp-item:hover {
  color: var(--accent-primary);
  border-left-color: var(--accent-primary);
  background: var(--bg-hover);
}

.level-1 { padding-left: var(--spacing-md); font-weight: 500; }
.level-2 { padding-left: calc(var(--spacing-md) + 12px); }
.level-3 { padding-left: calc(var(--spacing-md) + 24px); }
.level-4, .level-5, .level-6 { padding-left: calc(var(--spacing-md) + 36px); }
</style>
