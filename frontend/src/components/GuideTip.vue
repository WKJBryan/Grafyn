<template>
  <Teleport to="body">
    <div
      v-if="guide.activeTip.value"
      class="guide-tip-backdrop"
      @click="handleBackdropClick"
    />
    <Transition name="tip-fade">
      <div
        v-if="guide.activeTip.value && tipPosition"
        class="guide-tip-card"
        :style="cardStyle"
      >
        <div class="tip-header">
          <span class="tip-category">{{ tipCategoryTitle }}</span>
          <span
            v-if="tipCounter"
            class="tip-counter"
          >{{ tipCounter }}</span>
        </div>
        <div class="tip-title">
          {{ guide.activeTip.value.title }}
        </div>
        <div class="tip-content">
          {{ guide.activeTip.value.content }}
        </div>
        <div class="tip-actions">
          <button
            class="tip-btn tip-btn-primary"
            @click="handleGotIt"
          >
            Got it
          </button>
          <button
            class="tip-btn tip-btn-ghost"
            @click="handleSkipAll"
          >
            Skip all
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup>
import { ref, watch, onUnmounted, computed } from 'vue'
import { useGuide } from '@/composables/useGuide'
import { guideCategories } from '@/data/guideContent'

const guide = useGuide()
const tipPosition = ref(null)
let spotlightEl = null

const tipCategoryTitle = computed(() => {
  if (!guide.activeTip.value) return ''
  const cat = guideCategories.find(c =>
    c.steps.some(s => s.id === guide.activeTip.value.id)
  )
  return cat ? cat.title : ''
})

const tipCounter = computed(() => {
  if (!guide.activeTip.value) return ''
  const routeTips = guide.state.tipQueue
  const idx = routeTips.findIndex(t => t.id === guide.activeTip.value.id)
  if (idx === -1 || routeTips.length <= 1) return ''
  return `${idx + 1} of ${routeTips.length}`
})

const cardStyle = computed(() => {
  if (!tipPosition.value) return {}
  return {
    top: tipPosition.value.top + 'px',
    left: tipPosition.value.left + 'px',
  }
})

function positionTip(tip) {
  clearSpotlight()
  if (!tip || !tip.anchor) {
    tipPosition.value = null
    return
  }

  const el = document.querySelector(tip.anchor)
  if (!el) {
    tipPosition.value = null
    return
  }

  el.scrollIntoView({ behavior: 'smooth', block: 'nearest' })
  el.classList.add('guide-spotlight')
  spotlightEl = el

  requestAnimationFrame(() => {
    const rect = el.getBoundingClientRect()
    const cardWidth = 320
    const cardHeight = 200
    const margin = 12

    let top, left

    // Prefer placing below
    if (rect.bottom + cardHeight + margin < window.innerHeight) {
      top = rect.bottom + margin
      left = rect.left + rect.width / 2 - cardWidth / 2
    }
    // Try above
    else if (rect.top - cardHeight - margin > 0) {
      top = rect.top - cardHeight - margin
      left = rect.left + rect.width / 2 - cardWidth / 2
    }
    // Try right
    else if (rect.right + cardWidth + margin < window.innerWidth) {
      top = rect.top + rect.height / 2 - cardHeight / 2
      left = rect.right + margin
    }
    // Fallback: left
    else {
      top = rect.top + rect.height / 2 - cardHeight / 2
      left = rect.left - cardWidth - margin
    }

    // Clamp to viewport
    left = Math.max(margin, Math.min(left, window.innerWidth - cardWidth - margin))
    top = Math.max(margin, Math.min(top, window.innerHeight - cardHeight - margin))

    tipPosition.value = { top, left }
  })
}

function clearSpotlight() {
  if (spotlightEl) {
    spotlightEl.classList.remove('guide-spotlight')
    spotlightEl = null
  }
}

function handleGotIt() {
  if (guide.activeTip.value) {
    guide.completeTip(guide.activeTip.value.id)
  }
  clearSpotlight()
}

function handleSkipAll() {
  guide.dismissAllTips()
  clearSpotlight()
}

function handleBackdropClick() {
  handleGotIt()
}

function handleResize() {
  if (guide.activeTip.value) {
    positionTip(guide.activeTip.value)
  }
}

watch(() => guide.activeTip.value, (tip) => {
  if (tip) {
    positionTip(tip)
  } else {
    clearSpotlight()
    tipPosition.value = null
  }
})

// Recalculate on resize
if (typeof window !== 'undefined') {
  window.addEventListener('resize', handleResize)
}

onUnmounted(() => {
  clearSpotlight()
  if (typeof window !== 'undefined') {
    window.removeEventListener('resize', handleResize)
  }
})
</script>

<style>
/* Global spotlight class applied to target elements */
.guide-spotlight {
  position: relative;
  z-index: 9998 !important;
  box-shadow: 0 0 0 4px var(--accent-primary), 0 0 16px rgba(124, 92, 255, 0.3);
  border-radius: var(--radius-sm, 4px);
}
</style>

<style scoped>
.guide-tip-backdrop {
  position: fixed;
  inset: 0;
  z-index: 9997;
  background: rgba(0, 0, 0, 0.4);
}

.guide-tip-card {
  position: fixed;
  z-index: 9998;
  width: 320px;
  background: var(--bg-secondary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-lg, 12px);
  padding: var(--spacing-md, 16px);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
}

.tip-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--spacing-xs, 4px);
}

.tip-category {
  font-size: 0.7rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--accent-primary);
}

.tip-counter {
  font-size: 0.7rem;
  color: var(--text-muted);
}

.tip-title {
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
  margin-bottom: var(--spacing-xs, 4px);
}

.tip-content {
  font-size: 0.85rem;
  color: var(--text-secondary);
  line-height: 1.5;
  margin-bottom: var(--spacing-md, 16px);
}

.tip-actions {
  display: flex;
  gap: var(--spacing-sm, 8px);
}

.tip-btn {
  padding: 6px 14px;
  border-radius: var(--radius-sm, 4px);
  font-size: 0.8rem;
  font-weight: 500;
  cursor: pointer;
  border: none;
  transition: all 0.15s ease;
}

.tip-btn-primary {
  background: var(--accent-primary);
  color: white;
}

.tip-btn-primary:hover {
  filter: brightness(1.1);
}

.tip-btn-ghost {
  background: transparent;
  color: var(--text-muted);
}

.tip-btn-ghost:hover {
  color: var(--text-primary);
  background: var(--bg-hover);
}

/* Transition */
.tip-fade-enter-active {
  animation: tipIn 0.25s ease-out;
}

.tip-fade-leave-active {
  animation: tipOut 0.15s ease-in forwards;
}

@keyframes tipIn {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes tipOut {
  from {
    opacity: 1;
    transform: translateY(0);
  }
  to {
    opacity: 0;
    transform: translateY(8px);
  }
}
</style>
