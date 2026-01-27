/**
 * Unit tests for SearchBar component
 *
 * Tests cover:
 * - Component rendering
 * - Search input and debouncing (300ms)
 * - Search API calls
 * - Results display with scores
 * - Result selection
 * - Keyboard shortcuts (Enter, Escape)
 * - Clear button functionality
 * - Click-outside behavior
 * - Empty query handling
 * - Error handling
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import SearchBar from '@/components/SearchBar.vue'
import * as apiClient from '@/api/client'

describe('SearchBar', () => {
  let wrapper

  beforeEach(() => {
    vi.useFakeTimers()
  })

  afterEach(() => {
    if (wrapper) {
      wrapper.unmount()
    }
    vi.restoreAllMocks()
    vi.useRealTimers()
  })

  // ============================================================================
  // Rendering Tests
  // ============================================================================

  describe('Rendering', () => {
    it('renders the search input field', () => {
      wrapper = mount(SearchBar)

      const input = wrapper.find('input[type="text"]')
      expect(input.exists()).toBe(true)
      expect(input.attributes('placeholder')).toBe('Search notes...')
    })

    it('does not show clear button when query is empty', () => {
      wrapper = mount(SearchBar)

      expect(wrapper.find('.clear-btn').exists()).toBe(false)
    })

    it('shows clear button when query has value', async () => {
      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test query')

      expect(wrapper.find('.clear-btn').exists()).toBe(true)
    })

    it('does not show results dropdown initially', () => {
      wrapper = mount(SearchBar)

      expect(wrapper.find('.search-results').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Debouncing Tests
  // ============================================================================

  describe('Search Debouncing', () => {
    it('debounces search input with 300ms delay', async () => {
      const mockSearch = vi.spyOn(apiClient.search, 'query').mockResolvedValue([])

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')

      // Should not call immediately
      expect(mockSearch).not.toHaveBeenCalled()

      // Fast forward 299ms - should still not call
      vi.advanceTimersByTime(299)
      expect(mockSearch).not.toHaveBeenCalled()

      // Fast forward to 300ms - should call now
      vi.advanceTimersByTime(1)
      await nextTick()

      expect(mockSearch).toHaveBeenCalledTimes(1)
      expect(mockSearch).toHaveBeenCalledWith('test', { limit: 5 })
    })

    it('resets debounce timer on each keystroke', async () => {
      const mockSearch = vi.spyOn(apiClient.search, 'query').mockResolvedValue([])

      wrapper = mount(SearchBar)

      // First keystroke
      await wrapper.find('input').setValue('t')
      vi.advanceTimersByTime(200)

      // Second keystroke before 300ms
      await wrapper.find('input').setValue('te')
      vi.advanceTimersByTime(200)

      // Third keystroke
      await wrapper.find('input').setValue('tes')
      vi.advanceTimersByTime(200)

      // Only called once after user stops typing
      expect(mockSearch).not.toHaveBeenCalled()

      vi.advanceTimersByTime(100) // Complete the 300ms
      await nextTick()

      expect(mockSearch).toHaveBeenCalledTimes(1)
      expect(mockSearch).toHaveBeenCalledWith('tes', { limit: 5 })
    })

    it('does not search if query is empty', async () => {
      const mockSearch = vi.spyOn(apiClient.search, 'query')

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('')
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(mockSearch).not.toHaveBeenCalled()
    })

    it('does not search if query is only whitespace', async () => {
      const mockSearch = vi.spyOn(apiClient.search, 'query')

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('   ')
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(mockSearch).not.toHaveBeenCalled()
    })
  })

  // ============================================================================
  // Search API Tests
  // ============================================================================

  describe('Search API Calls', () => {
    it('calls search API with correct parameters', async () => {
      const mockSearch = vi
        .spyOn(apiClient.search, 'query')
        .mockResolvedValue([])

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test query')
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(mockSearch).toHaveBeenCalledWith('test query', { limit: 5 })
    })

    it('limits results to 5', async () => {
      const mockSearch = vi.spyOn(apiClient.search, 'query').mockResolvedValue([])

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(mockSearch).toHaveBeenCalledWith('test', { limit: 5 })
    })

    it('handles search errors gracefully', async () => {
      const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
      const mockSearch = vi
        .spyOn(apiClient.search, 'query')
        .mockRejectedValue(new Error('Search failed'))

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(consoleError).toHaveBeenCalledWith(
        'Search failed:',
        expect.any(Error)
      )
    })
  })

  // ============================================================================
  // Results Display Tests
  // ============================================================================

  describe('Results Display', () => {
    it('displays search results', async () => {
      const mockResults = [
        { note_id: 'note-1', title: 'Test Note 1', score: 0.95 },
        { note_id: 'note-2', title: 'Test Note 2', score: 0.85 },
      ]

      vi.spyOn(apiClient.search, 'query').mockResolvedValue(mockResults)

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      const results = wrapper.findAll('.search-result-item')
      expect(results).toHaveLength(2)
      expect(results[0].text()).toContain('Test Note 1')
      expect(results[1].text()).toContain('Test Note 2')
    })

    it('shows score bars for results', async () => {
      const mockResults = [{ note_id: 'note-1', title: 'Test Note', score: 0.75 }]

      vi.spyOn(apiClient.search, 'query').mockResolvedValue(mockResults)

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      const scoreBar = wrapper.find('.score-bar')
      expect(scoreBar.attributes('style')).toContain('width: 75%')
    })

    it('does not show results when query is cleared', async () => {
      const mockResults = [{ note_id: 'note-1', title: 'Test Note', score: 0.95 }]

      vi.spyOn(apiClient.search, 'query').mockResolvedValue(mockResults)

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(wrapper.find('.search-results').exists()).toBe(true)

      await wrapper.find('input').setValue('')
      await nextTick()

      expect(wrapper.find('.search-results').exists()).toBe(false)
    })

    it('does not show dropdown when no results', async () => {
      vi.spyOn(apiClient.search, 'query').mockResolvedValue([])

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('nonexistent')
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(wrapper.find('.search-results').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Result Selection Tests
  // ============================================================================

  describe('Result Selection', () => {
    it('emits select event when result is clicked', async () => {
      const mockResults = [
        { note_id: 'note-1', title: 'Test Note 1', score: 0.95 },
        { note_id: 'note-2', title: 'Test Note 2', score: 0.85 },
      ]

      vi.spyOn(apiClient.search, 'query').mockResolvedValue(mockResults)

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      const results = wrapper.findAll('.search-result-item')
      await results[0].trigger('click')

      expect(wrapper.emitted('select')).toBeTruthy()
      expect(wrapper.emitted('select')[0]).toEqual(['note-1'])
    })

    it('clears input and results after selection', async () => {
      const mockResults = [{ note_id: 'note-1', title: 'Test Note', score: 0.95 }]

      vi.spyOn(apiClient.search, 'query').mockResolvedValue(mockResults)

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      await wrapper.find('.search-result-item').trigger('click')

      expect(wrapper.find('input').element.value).toBe('')
      expect(wrapper.find('.search-results').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Keyboard Shortcuts Tests
  // ============================================================================

  describe('Keyboard Shortcuts', () => {
    it('selects first result on Enter key', async () => {
      const mockResults = [
        { note_id: 'note-1', title: 'First Note', score: 0.95 },
        { note_id: 'note-2', title: 'Second Note', score: 0.85 },
      ]

      vi.spyOn(apiClient.search, 'query').mockResolvedValue(mockResults)

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      await wrapper.find('input').trigger('keydown.enter')

      expect(wrapper.emitted('select')).toBeTruthy()
      expect(wrapper.emitted('select')[0]).toEqual(['note-1'])
    })

    it('does not emit select on Enter if no results', async () => {
      vi.spyOn(apiClient.search, 'query').mockResolvedValue([])

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      await wrapper.find('input').trigger('keydown.enter')

      expect(wrapper.emitted('select')).toBeFalsy()
    })

    it('clears query and results on Escape key', async () => {
      const mockResults = [{ note_id: 'note-1', title: 'Test Note', score: 0.95 }]

      vi.spyOn(apiClient.search, 'query').mockResolvedValue(mockResults)

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(wrapper.find('.search-results').exists()).toBe(true)

      await wrapper.find('input').trigger('keydown.escape')

      expect(wrapper.find('input').element.value).toBe('')
      expect(wrapper.find('.search-results').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Clear Button Tests
  // ============================================================================

  describe('Clear Button', () => {
    it('clears query when clear button is clicked', async () => {
      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test query')
      expect(wrapper.find('input').element.value).toBe('test query')

      await wrapper.find('.clear-btn').trigger('click')

      expect(wrapper.find('input').element.value).toBe('')
    })

    it('clears results when clear button is clicked', async () => {
      const mockResults = [{ note_id: 'note-1', title: 'Test Note', score: 0.95 }]

      vi.spyOn(apiClient.search, 'query').mockResolvedValue(mockResults)

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(wrapper.find('.search-results').exists()).toBe(true)

      await wrapper.find('.clear-btn').trigger('click')

      expect(wrapper.find('.search-results').exists()).toBe(false)
    })

    it('hides clear button after clicking it', async () => {
      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue('test')
      expect(wrapper.find('.clear-btn').exists()).toBe(true)

      await wrapper.find('.clear-btn').trigger('click')

      expect(wrapper.find('.clear-btn').exists()).toBe(false)
    })
  })

  // ============================================================================
  // Click Outside Tests
  // ============================================================================

  describe('Click Outside Behavior', () => {
    it('hides results when clicking outside', async () => {
      const mockResults = [{ note_id: 'note-1', title: 'Test Note', score: 0.95 }]

      vi.spyOn(apiClient.search, 'query').mockResolvedValue(mockResults)

      wrapper = mount(SearchBar, { attachTo: document.body })

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(wrapper.find('.search-results').exists()).toBe(true)

      // Click outside
      document.body.click()
      await nextTick()

      expect(wrapper.find('.search-results').exists()).toBe(false)

      wrapper.unmount()
    })

    it('keeps results visible when clicking inside search bar', async () => {
      const mockResults = [{ note_id: 'note-1', title: 'Test Note', score: 0.95 }]

      vi.spyOn(apiClient.search, 'query').mockResolvedValue(mockResults)

      wrapper = mount(SearchBar, { attachTo: document.body })

      await wrapper.find('input').setValue('test')
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(wrapper.find('.search-results').exists()).toBe(true)

      // Click inside search bar
      wrapper.find('.search-bar').element.click()
      await nextTick()

      expect(wrapper.find('.search-results').exists()).toBe(true)

      wrapper.unmount()
    })
  })

  // ============================================================================
  // Component Lifecycle Tests
  // ============================================================================

  describe('Component Lifecycle', () => {
    it('cleans up event listeners on unmount', () => {
      const removeEventListener = vi.spyOn(document, 'removeEventListener')

      wrapper = mount(SearchBar)
      wrapper.unmount()

      expect(removeEventListener).toHaveBeenCalledWith(
        'click',
        expect.any(Function)
      )
    })

    it('clears debounce timer on unmount', () => {
      const clearTimeout = vi.spyOn(global, 'clearTimeout')

      wrapper = mount(SearchBar)

      // Start a search
      wrapper.find('input').setValue('test')

      wrapper.unmount()

      expect(clearTimeout).toHaveBeenCalled()
    })
  })

  // ============================================================================
  // Edge Cases
  // ============================================================================

  describe('Edge Cases', () => {
    it('handles very long search queries', async () => {
      const longQuery = 'a'.repeat(1000)
      const mockSearch = vi.spyOn(apiClient.search, 'query').mockResolvedValue([])

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue(longQuery)
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(mockSearch).toHaveBeenCalledWith(longQuery, { limit: 5 })
    })

    it('handles special characters in search query', async () => {
      const specialQuery = '@#$%^&*()[]{}|;:\'",.<>?/~`'
      const mockSearch = vi.spyOn(apiClient.search, 'query').mockResolvedValue([])

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue(specialQuery)
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(mockSearch).toHaveBeenCalledWith(specialQuery, { limit: 5 })
    })

    it('handles unicode characters in search', async () => {
      const unicodeQuery = '你好世界 🌍'
      const mockSearch = vi.spyOn(apiClient.search, 'query').mockResolvedValue([])

      wrapper = mount(SearchBar)

      await wrapper.find('input').setValue(unicodeQuery)
      vi.advanceTimersByTime(300)
      await nextTick()

      expect(mockSearch).toHaveBeenCalledWith(unicodeQuery, { limit: 5 })
    })

    it('handles rapid consecutive searches', async () => {
      const mockSearch = vi.spyOn(apiClient.search, 'query').mockResolvedValue([])

      wrapper = mount(SearchBar)

      // Rapid typing
      for (let i = 1; i <= 10; i++) {
        await wrapper.find('input').setValue('a'.repeat(i))
        vi.advanceTimersByTime(50)
      }

      // Wait for debounce
      vi.advanceTimersByTime(300)
      await nextTick()

      // Should only search once with final query
      expect(mockSearch).toHaveBeenCalledTimes(1)
      expect(mockSearch).toHaveBeenCalledWith('aaaaaaaaaa', { limit: 5 })
    })
  })
})
