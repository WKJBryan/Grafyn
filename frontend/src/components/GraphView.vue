<template>
  <div class="graph-view">
    <div class="placeholder-content">
      <h2>Graph Visualization</h2>
      <p class="text-muted">Coming in Phase 2</p>
      <p class="text-secondary">
        This feature will show the knowledge graph visualization with nodes and edges
        representing your notes and their relationships.
      </p>
    </div>
  </div>
</template>

<script setup>
// Placeholder component for Phase 2 graph visualization
</script>

<style scoped>
.graph-view {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-secondary);
  border-radius: var(--radius-md);
}

.placeholder-content {
  text-align: center;
  padding: var(--spacing-xl);
}

.placeholder-content h2 {
  margin-bottom: var(--spacing-md);
  color: var(--text-primary);
}

.placeholder-content p {
  margin-bottom: var(--spacing-sm);
  font-size: 0.875rem;
}
</style>
