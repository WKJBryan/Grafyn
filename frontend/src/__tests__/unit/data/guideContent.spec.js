import { describe, it, expect } from 'vitest'
import { guideCategories, allSteps, getNewSteps } from '@/data/guideContent'

describe('guideContent', () => {
  it('all steps have required fields', () => {
    for (const step of allSteps) {
      expect(step.id).toBeTruthy()
      expect(step.title).toBeTruthy()
      expect(step.content).toBeTruthy()
      expect(step.sinceVersion).toMatch(/^\d+\.\d+\.\d+$/)
    }
  })

  it('has no duplicate step IDs', () => {
    const ids = allSteps.map(s => s.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('all anchors use data-guide selector format or are null', () => {
    for (const step of allSteps) {
      if (step.anchor !== null) {
        expect(step.anchor).toMatch(/^\[data-guide="[a-z0-9-]+"\]$/)
      }
    }
  })

  it('every step belongs to a category', () => {
    for (const step of allSteps) {
      expect(step.categoryId).toBeTruthy()
      const cat = guideCategories.find(c => c.id === step.categoryId)
      expect(cat).toBeDefined()
    }
  })

  it('getNewSteps returns all steps when sinceVersion is null', () => {
    const result = getNewSteps(null)
    expect(result.length).toBe(allSteps.length)
  })

  it('getNewSteps filters by semver correctly', () => {
    const result = getNewSteps('0.1.0')
    // Only steps with sinceVersion > 0.1.0 should be included
    for (const step of result) {
      expect(step.sinceVersion).not.toBe('0.1.0')
    }
    // Steps at 0.1.0 should not be included
    const at010 = allSteps.filter(s => s.sinceVersion === '0.1.0')
    for (const step of at010) {
      expect(result.find(r => r.id === step.id)).toBeUndefined()
    }
  })

  it('getNewSteps returns steps after a given version', () => {
    // Steps with sinceVersion 0.1.1 exist (canvas-context)
    const result = getNewSteps('0.1.0')
    const contextStep = result.find(s => s.id === 'canvas-context')
    expect(contextStep).toBeDefined()
    expect(contextStep.sinceVersion).toBe('0.1.1')
  })

  it('getNewSteps returns empty for future version', () => {
    const result = getNewSteps('99.99.99')
    expect(result.length).toBe(0)
  })

  it('categories have required fields', () => {
    for (const cat of guideCategories) {
      expect(cat.id).toBeTruthy()
      expect(cat.title).toBeTruthy()
      expect(cat.icon).toBeTruthy()
      expect(cat.route).toBeTruthy()
      expect(Array.isArray(cat.steps)).toBe(true)
      expect(cat.steps.length).toBeGreaterThan(0)
    }
  })

  it('anchors the canvas prompt tip to the new prompt button', () => {
    const promptStep = allSteps.find(step => step.id === 'canvas-prompt')
    expect(promptStep).toBeDefined()
    expect(promptStep.anchor).toBe('[data-guide="canvas-prompt-btn"]')
    expect(promptStep.content).toContain('+ New Prompt')
  })
})
