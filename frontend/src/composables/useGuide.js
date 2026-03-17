import { reactive, computed } from 'vue'
import { guideCategories, allSteps, getNewSteps } from '@/data/guideContent'

const STORAGE_KEY = 'grafyn_guide_state'
const VERSION_KEY = 'grafyn_last_seen_version'

function loadPersistedState() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw) {
      const parsed = JSON.parse(raw)
      return {
        completedSteps: new Set(parsed.completedSteps || []),
        dismissed: !!parsed.dismissed,
      }
    }
  } catch {
    // ignore corrupt data
  }
  return { completedSteps: new Set(), dismissed: false }
}

function savePersistedState() {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      completedSteps: Array.from(state.completedSteps),
      dismissed: state.dismissed,
    }))
  } catch {
    // localStorage may be full or unavailable
  }
}

const persisted = loadPersistedState()

const state = reactive({
  // Persisted
  completedSteps: persisted.completedSteps,
  dismissed: persisted.dismissed,
  // Transient
  panelOpen: false,
  panelCategory: null,
  activeTip: null,
  tipQueue: [],
  currentRoute: '/',
})

function checkNewFeatures() {
  if (state.dismissed) return

  const appVersion = typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : '0.0.0'
  const lastSeen = localStorage.getItem(VERSION_KEY)

  if (lastSeen !== appVersion) {
    const newSteps = getNewSteps(lastSeen)
    const unseen = newSteps.filter(s => !state.completedSteps.has(s.id))
    state.tipQueue = unseen.filter(s => s.anchor)
    localStorage.setItem(VERSION_KEY, appVersion)
  }
}

function getTipsForRoute(path) {
  return state.tipQueue.filter(tip => {
    if (!tip.route) return true
    if (tip.route === '/') return path === '/'
    return path.startsWith(tip.route)
  })
}

function setCurrentRoute(path) {
  state.currentRoute = path
}

function showNextTip() {
  if (state.dismissed || state.tipQueue.length === 0) {
    state.activeTip = null
    return
  }
  const routeTips = getTipsForRoute(state.currentRoute)
  state.activeTip = routeTips.length > 0 ? routeTips[0] : null
}

function skipTip() {
  state.activeTip = null
}

function showTipForRoute(path) {
  if (state.dismissed) return
  const routeTips = getTipsForRoute(path)
  if (routeTips.length > 0) {
    state.activeTip = routeTips[0]
  }
}

function completeTip(stepId) {
  state.completedSteps.add(stepId)
  state.tipQueue = state.tipQueue.filter(t => t.id !== stepId)
  savePersistedState()
  showNextTip()
}

function dismissAllTips() {
  state.dismissed = true
  state.tipQueue = []
  state.activeTip = null
  savePersistedState()
}

function togglePanel() {
  state.panelOpen = !state.panelOpen
}

function openPanel(categoryId) {
  state.panelOpen = true
  state.panelCategory = categoryId || null
}

function closePanel() {
  state.panelOpen = false
}

function showTip(stepId) {
  const step = allSteps.find(s => s.id === stepId)
  if (step) {
    state.activeTip = step
  }
}

function completeStep(stepId) {
  state.completedSteps.add(stepId)
  savePersistedState()
}

function resetProgress() {
  state.completedSteps.clear()
  state.dismissed = false
  state.tipQueue = []
  state.activeTip = null
  state.panelCategory = null
  localStorage.removeItem(STORAGE_KEY)
  localStorage.removeItem(VERSION_KEY)
}

function categoryProgress(categoryId) {
  const cat = guideCategories.find(c => c.id === categoryId)
  if (!cat) return { completed: 0, total: 0 }
  const completed = cat.steps.filter(s => state.completedSteps.has(s.id)).length
  return { completed, total: cat.steps.length }
}

export function useGuide() {
  return {
    state,
    categories: guideCategories,
    allSteps,
    panelOpen: computed(() => state.panelOpen),
    activeTip: computed(() => state.activeTip),
    checkNewFeatures,
    getTipsForRoute,
    setCurrentRoute,
    showNextTip,
    showTipForRoute,
    completeTip,
    skipTip,
    dismissAllTips,
    togglePanel,
    openPanel,
    closePanel,
    showTip,
    completeStep,
    resetProgress,
    categoryProgress,
    isStepCompleted: (stepId) => state.completedSteps.has(stepId),
  }
}
