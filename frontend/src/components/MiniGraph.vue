<template>
  <div class="mini-graph">
    <GraphView
      :width="300"
      :height="250"
      :show-settings="false"
      @node-click="handleNodeClick"
    />
  </div>
</template>

<script setup>
import GraphView from './GraphView.vue'

const emit = defineEmits(['navigate'])

function handleNodeClick(noteId) {
  emit('navigate', noteId)
}
</script>

<style scoped>
.mini-graph {
  width: 100%;
  height: 250px;
  border-radius: var(--radius-md);
  overflow: hidden;
  border: 1px solid var(--bg-tertiary);
  background: var(--bg-primary);
}
</style>
