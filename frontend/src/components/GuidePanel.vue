<template>
  <Teleport to="body">
    <Transition name="panel-slide">
      <div
        v-if="guide.panelOpen.value"
        class="guide-panel"
      >
        <div class="panel-header">
          <h2>Grafyn Guide</h2>
          <button
            class="panel-close-btn"
            @click="guide.closePanel()"
          >
            &times;
          </button>
        </div>

        <div class="panel-body">
          <div
            v-for="cat in guide.categories"
            :key="cat.id"
            class="guide-category"
          >
            <button
              class="category-header"
              @click="toggleCategory(cat.id)"
            >
              <span class="category-icon">{{ cat.icon }}</span>
              <span class="category-title">{{ cat.title }}</span>
              <span class="category-progress">
                {{ progress(cat.id).completed }}/{{ progress(cat.id).total }}
              </span>
              <span
                class="category-arrow"
                :class="{ expanded: expandedCategory === cat.id }"
              >&#9656;</span>
            </button>

            <Transition name="expand">
              <div
                v-if="expandedCategory === cat.id"
                class="category-steps"
              >
                <div
                  v-for="step in cat.steps"
                  :key="step.id"
                  class="step-card"
                  :class="{ completed: guide.isStepCompleted(step.id) }"
                >
                  <div class="step-header">
                    <span
                      v-if="guide.isStepCompleted(step.id)"
                      class="step-check"
                    >&#10003;</span>
                    <span class="step-title">{{ step.title }}</span>
                  </div>
                  <div class="step-content">
                    {{ step.content }}
                  </div>
                  <div class="step-actions">
                    <button
                      v-if="step.anchor"
                      class="step-btn"
                      @click="handleShowMe(step)"
                    >
                      Show me
                    </button>
                    <button
                      v-if="!guide.isStepCompleted(step.id)"
                      class="step-btn step-btn-ghost"
                      @click="guide.completeStep(step.id)"
                    >
                      Got it
                    </button>
                  </div>
                </div>
              </div>
            </Transition>
          </div>
        </div>

        <div class="panel-footer">
          <button
            class="reset-btn"
            @click="handleReset"
          >
            Reset progress
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup>
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { useGuide } from '@/composables/useGuide'

const guide = useGuide()
const router = useRouter()
const expandedCategory = ref(null)

function toggleCategory(catId) {
  expandedCategory.value = expandedCategory.value === catId ? null : catId
}

function progress(catId) {
  return guide.categoryProgress(catId)
}

function handleShowMe(step) {
  const cat = guide.categories.find(c => c.steps.some(s => s.id === step.id))
  if (cat && cat.route) {
    const currentPath = router.currentRoute.value.path
    const targetRoute = cat.route === '/' ? '/' : cat.route
    if (currentPath !== targetRoute && !currentPath.startsWith(targetRoute + '/')) {
      router.push(targetRoute)
    }
  }
  guide.closePanel()
  setTimeout(() => {
    guide.showTip(step.id)
  }, 400)
}

function handleReset() {
  guide.resetProgress()
  expandedCategory.value = null
}
</script>

<style scoped>
.guide-panel {
  position: fixed;
  right: 0;
  top: 0;
  bottom: 0;
  width: 380px;
  max-width: 100vw;
  z-index: 9000;
  background: var(--bg-secondary);
  border-left: 1px solid var(--bg-tertiary);
  display: flex;
  flex-direction: column;
  box-shadow: -4px 0 24px rgba(0, 0, 0, 0.3);
}

.panel-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--spacing-md, 16px) var(--spacing-lg, 24px);
  border-bottom: 1px solid var(--bg-tertiary);
}

.panel-header h2 {
  margin: 0;
  font-size: 1.125rem;
  color: var(--text-primary);
}

.panel-close-btn {
  background: none;
  border: none;
  font-size: 1.5rem;
  color: var(--text-muted);
  cursor: pointer;
  padding: 0 4px;
  line-height: 1;
  transition: color 0.15s;
}

.panel-close-btn:hover {
  color: var(--text-primary);
}

.panel-body {
  flex: 1;
  overflow-y: auto;
  padding: var(--spacing-sm, 8px);
}

.guide-category {
  margin-bottom: var(--spacing-xs, 4px);
}

.category-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm, 8px);
  width: 100%;
  padding: var(--spacing-sm, 8px) var(--spacing-md, 16px);
  background: transparent;
  border: none;
  border-radius: var(--radius-sm, 4px);
  color: var(--text-primary);
  font-size: 0.9rem;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.15s;
}

.category-header:hover {
  background: var(--bg-hover);
}

.category-icon {
  font-size: 1rem;
  flex-shrink: 0;
}

.category-title {
  flex: 1;
  text-align: left;
}

.category-progress {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.category-arrow {
  font-size: 0.7rem;
  color: var(--text-muted);
  transition: transform 0.2s;
  flex-shrink: 0;
}

.category-arrow.expanded {
  transform: rotate(90deg);
}

.category-steps {
  padding: 0 var(--spacing-sm, 8px) var(--spacing-sm, 8px) var(--spacing-lg, 24px);
}

.step-card {
  padding: var(--spacing-sm, 8px) var(--spacing-md, 16px);
  margin-bottom: var(--spacing-xs, 4px);
  background: var(--bg-tertiary);
  border-radius: var(--radius-sm, 4px);
  border-left: 3px solid var(--accent-primary);
  transition: opacity 0.15s;
}

.step-card.completed {
  opacity: 0.6;
  border-left-color: var(--text-muted);
}

.step-header {
  display: flex;
  align-items: center;
  gap: var(--spacing-xs, 4px);
  margin-bottom: 2px;
}

.step-check {
  color: var(--accent-primary);
  font-size: 0.8rem;
  font-weight: 700;
}

.step-title {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--text-primary);
}

.step-content {
  font-size: 0.8rem;
  color: var(--text-secondary);
  line-height: 1.45;
  margin-bottom: var(--spacing-xs, 4px);
}

.step-actions {
  display: flex;
  gap: var(--spacing-xs, 4px);
}

.step-btn {
  padding: 3px 10px;
  font-size: 0.75rem;
  font-weight: 500;
  border: none;
  border-radius: var(--radius-sm, 4px);
  cursor: pointer;
  background: var(--accent-primary);
  color: white;
  transition: filter 0.15s;
}

.step-btn:hover {
  filter: brightness(1.1);
}

.step-btn-ghost {
  background: transparent;
  color: var(--text-muted);
}

.step-btn-ghost:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
  filter: none;
}

.panel-footer {
  padding: var(--spacing-md, 16px) var(--spacing-lg, 24px);
  border-top: 1px solid var(--bg-tertiary);
}

.reset-btn {
  background: none;
  border: none;
  color: var(--text-muted);
  font-size: 0.8rem;
  cursor: pointer;
  padding: 0;
  text-decoration: underline;
  transition: color 0.15s;
}

.reset-btn:hover {
  color: var(--text-primary);
}

/* Slide transition */
.panel-slide-enter-active {
  animation: slideIn 0.3s ease-out;
}

.panel-slide-leave-active {
  animation: slideOut 0.2s ease-in forwards;
}

@keyframes slideIn {
  from {
    transform: translateX(100%);
  }
  to {
    transform: translateX(0);
  }
}

@keyframes slideOut {
  from {
    transform: translateX(0);
  }
  to {
    transform: translateX(100%);
  }
}

/* Expand transition for category content */
.expand-enter-active {
  animation: expandIn 0.2s ease-out;
}

.expand-leave-active {
  animation: expandOut 0.15s ease-in forwards;
}

@keyframes expandIn {
  from {
    opacity: 0;
    max-height: 0;
  }
  to {
    opacity: 1;
    max-height: 500px;
  }
}

@keyframes expandOut {
  from {
    opacity: 1;
    max-height: 500px;
  }
  to {
    opacity: 0;
    max-height: 0;
  }
}
</style>
