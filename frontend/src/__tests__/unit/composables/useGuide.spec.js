import { describe, it, expect, beforeEach, vi } from 'vitest'

// Mock localStorage
const localStorageMock = (() => {
  let store = {}
  return {
    getItem: vi.fn((key) => store[key] ?? null),
    setItem: vi.fn((key, value) => { store[key] = value }),
    removeItem: vi.fn((key) => { delete store[key] }),
    clear: vi.fn(() => { store = {} }),
    _store: () => store,
  }
})()

vi.stubGlobal('localStorage', localStorageMock)
vi.stubGlobal('__APP_VERSION__', '0.1.1')

// Must import after mocks are set up
let useGuide
beforeEach(async () => {
  localStorageMock.clear()
  // Re-import to reset module-level state
  vi.resetModules()
  const mod = await import('@/composables/useGuide')
  useGuide = mod.useGuide
})

describe('useGuide', () => {
  it('initializes with empty completed steps', () => {
    const guide = useGuide()
    expect(guide.state.completedSteps.size).toBe(0)
    expect(guide.state.dismissed).toBe(false)
  })

  it('persists completed steps to localStorage', () => {
    const guide = useGuide()
    guide.completeStep('notes-create')
    expect(guide.isStepCompleted('notes-create')).toBe(true)

    const saved = JSON.parse(localStorageMock.setItem.mock.calls.at(-1)[1])
    expect(saved.completedSteps).toContain('notes-create')
  })

  it('completeTip removes from queue and persists', () => {
    const guide = useGuide()
    // Manually add a tip to the queue
    guide.state.tipQueue = [{ id: 'test-tip', anchor: '[data-guide="test"]' }]
    guide.completeTip('test-tip')
    expect(guide.state.tipQueue.length).toBe(0)
    expect(guide.isStepCompleted('test-tip')).toBe(true)
  })

  it('dismissAllTips clears queue and sets flag', () => {
    const guide = useGuide()
    guide.state.tipQueue = [{ id: 'a' }, { id: 'b' }]
    guide.dismissAllTips()
    expect(guide.state.tipQueue.length).toBe(0)
    expect(guide.state.dismissed).toBe(true)
    expect(guide.state.activeTip).toBeNull()
  })

  it('resetProgress clears all state', () => {
    const guide = useGuide()
    guide.completeStep('notes-create')
    guide.state.dismissed = true
    guide.resetProgress()
    expect(guide.state.completedSteps.size).toBe(0)
    expect(guide.state.dismissed).toBe(false)
    expect(localStorageMock.removeItem).toHaveBeenCalledWith('grafyn_guide_state')
    expect(localStorageMock.removeItem).toHaveBeenCalledWith('grafyn_last_seen_version')
  })

  it('togglePanel toggles panel state', () => {
    const guide = useGuide()
    expect(guide.panelOpen.value).toBe(false)
    guide.togglePanel()
    expect(guide.panelOpen.value).toBe(true)
    guide.togglePanel()
    expect(guide.panelOpen.value).toBe(false)
  })

  it('openPanel sets panel open and category', () => {
    const guide = useGuide()
    guide.openPanel('canvas')
    expect(guide.panelOpen.value).toBe(true)
    expect(guide.state.panelCategory).toBe('canvas')
  })

  it('categoryProgress returns correct counts', () => {
    const guide = useGuide()
    const prog = guide.categoryProgress('notes')
    expect(prog.total).toBeGreaterThan(0)
    expect(prog.completed).toBe(0)

    // Complete a step in notes category
    guide.completeStep('notes-create')
    const prog2 = guide.categoryProgress('notes')
    expect(prog2.completed).toBe(1)
  })

  it('getTipsForRoute filters by route', () => {
    const guide = useGuide()
    guide.state.tipQueue = [
      { id: 'a', route: '/', anchor: '[data-guide="x"]' },
      { id: 'b', route: '/canvas', anchor: '[data-guide="y"]' },
      { id: 'c', route: '/import', anchor: '[data-guide="z"]' },
    ]
    const homeTips = guide.getTipsForRoute('/')
    expect(homeTips.length).toBe(1)
    expect(homeTips[0].id).toBe('a')

    const canvasTips = guide.getTipsForRoute('/canvas')
    expect(canvasTips.length).toBe(1)
    expect(canvasTips[0].id).toBe('b')

    const canvasIdTips = guide.getTipsForRoute('/canvas/some-id')
    expect(canvasIdTips.length).toBe(1)
    expect(canvasIdTips[0].id).toBe('b')
  })

  it('checkNewFeatures queues tips on version bump', () => {
    const guide = useGuide()
    // Simulate a previous version
    localStorageMock.setItem('grafyn_last_seen_version', '0.1.0')
    guide.checkNewFeatures()
    // Should have queued the tips that are new since 0.1.0
    // canvas-context (0.1.1) has an anchor so it should be in the queue
    const contextTip = guide.state.tipQueue.find(t => t.id === 'canvas-context')
    expect(contextTip).toBeDefined()
  })

  it('checkNewFeatures does nothing when dismissed', () => {
    const guide = useGuide()
    guide.state.dismissed = true
    localStorageMock.setItem('grafyn_last_seen_version', '0.0.1')
    guide.checkNewFeatures()
    expect(guide.state.tipQueue.length).toBe(0)
  })

  it('showTip sets activeTip from allSteps', () => {
    const guide = useGuide()
    guide.showTip('notes-create')
    expect(guide.activeTip.value).toBeDefined()
    expect(guide.activeTip.value.id).toBe('notes-create')
  })

  it('showTip does nothing for unknown step', () => {
    const guide = useGuide()
    guide.showTip('nonexistent-step')
    expect(guide.activeTip.value).toBeNull()
  })
})
