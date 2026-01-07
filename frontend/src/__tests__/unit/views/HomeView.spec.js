/**
 * Unit tests for HomeView
 *
 * Tests cover:
 * - 3-column layout rendering (TreeNav, Main, MiniGraph)
 * - Navigation interactions
 * - Graph toggling
 * - Search integration
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import HomeView from '@/views/HomeView.vue'
import * as apiClient from '@/api/client'

// Mock child components
vi.mock('@/components/SearchBar.vue', () => ({
  default: {
    name: 'SearchBar',
    template: '<div class="search-bar-stub" @click="$emit(\'select\', \'note-1\')"></div>',
    emits: ['select'],
  },
}))

vi.mock('@/components/TreeNav.vue', () => ({
  default: {
    name: 'TreeNav',
    template: '<div class="tree-nav-stub" @click="$emit(\'select\', \'note-2\')"></div>',
    props: ['notes', 'selectedId'],
    emits: ['select'],
  },
}))

vi.mock('@/components/NoteEditor.vue', () => ({
  default: {
    name: 'NoteEditor',
    template: `
      <div class="note-editor-stub">
        <button class="save-btn" @click="$emit('save', note.id, { title: 'Updated' })">Save</button>
        <button class="delete-btn" @click="$emit('delete', note.id)">Delete</button>
      </div>
    `,
    props: ['note'],
    emits: ['save', 'delete'],
  },
}))

vi.mock('@/components/BacklinksPanel.vue', () => ({
  default: {
    name: 'BacklinksPanel',
    template: '<div class="backlinks-panel-stub" @click="$emit(\'navigate\', \'note-3\')"></div>',
    props: ['noteId'],
    emits: ['navigate'],
  },
}))

vi.mock('@/components/MiniGraph.vue', () => ({
  default: {
    name: 'MiniGraph',
    template: '<div class="mini-graph-stub" @click="$emit(\'navigate\', \'note-4\')"></div>',
    emits: ['navigate'],
  },
}))

vi.mock('@/components/OnThisPage.vue', () => ({
  default: {
    name: 'OnThisPage',
    template: '<div class="on-this-page-stub"></div>',
    props: ['content'],
  },
}))

vi.mock('@/components/GraphView.vue', () => ({
  default: {
    name: 'GraphView',
    template: '<div class="full-graph-stub" @click="$emit(\'node-click\', \'note-5\')"></div>',
    emits: ['node-click'],
  },
}))

describe('HomeView', () => {
  let wrapper

  beforeEach(() => {
    vi.clearAllMocks()
    vi.spyOn(window, 'confirm').mockReturnValue(true) // Default to confirm
  })

  afterEach(() => {
    if (wrapper) {
      wrapper.unmount()
    }
  })

  // ============================================================================
  // Rendering Tests
  // ============================================================================

  describe('Rendering', () => {
    it('renders the 3-column layout', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])

      wrapper = mount(HomeView)
      await flushPromises()

      expect(wrapper.find('.sidebar-left').exists()).toBe(true)
      expect(wrapper.find('.main-content').exists()).toBe(true)
      expect(wrapper.find('.sidebar-right').exists()).toBe(true)
    })

    it('renders header with logo', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])

      wrapper = mount(HomeView)
      await flushPromises()

      expect(wrapper.find('.app-header').exists()).toBe(true)
      expect(wrapper.find('.logo').text()).toBe('Seedream')
    })

    it('renders TreeNav in left sidebar', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])

      wrapper = mount(HomeView)
      await flushPromises()

      expect(wrapper.find('.sidebar-left .tree-nav-stub').exists()).toBe(true)
    })

    it('renders MiniGraph in right sidebar', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])

      wrapper = mount(HomeView)
      await flushPromises()

      expect(wrapper.find('.sidebar-right .mini-graph-stub').exists()).toBe(true)
    })
  })

  // ============================================================================
  // Navigation & Data Loading
  // ============================================================================

  describe('Navigation', () => {
    it('loads note when TreeNav emits select', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])
      const getSpy = vi.spyOn(apiClient.notes, 'get').mockResolvedValue({
        id: 'note-2',
        title: 'Selected Note',
      })

      wrapper = mount(HomeView)
      await flushPromises()

      await wrapper.find('.tree-nav-stub').trigger('click')
      await flushPromises()

      expect(getSpy).toHaveBeenCalledWith('note-2')
      expect(wrapper.find('.note-editor-stub').exists()).toBe(true)
    })

    it('loads note when MiniGraph emits navigate', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])
      const getSpy = vi.spyOn(apiClient.notes, 'get').mockResolvedValue({
        id: 'note-4',
        title: 'Graph Note',
      })

      wrapper = mount(HomeView)
      await flushPromises()

      await wrapper.find('.mini-graph-stub').trigger('click')
      await flushPromises()

      expect(getSpy).toHaveBeenCalledWith('note-4')
    })

    it('loads note when SearchBar emits select', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])
      const getSpy = vi.spyOn(apiClient.notes, 'get').mockResolvedValue({
        id: 'note-1',
        title: 'Search Note',
      })

      wrapper = mount(HomeView)
      await flushPromises()

      await wrapper.find('.search-bar-stub').trigger('click')
      await flushPromises()

      expect(getSpy).toHaveBeenCalledWith('note-1')
    })
  })

  // ============================================================================
  // Full Graph View
  // ============================================================================

  describe('Full Graph View', () => {
    it('toggles full graph when logo is clicked', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])

      wrapper = mount(HomeView)
      await flushPromises()

      // Initially hidden
      expect(wrapper.find('.full-graph-container').exists()).toBe(false)

      // Click logo
      await wrapper.find('.logo-wrapper').trigger('click')
      await nextTick()

      // Should be visible
      expect(wrapper.find('.full-graph-container').exists()).toBe(true)
      expect(wrapper.find('.editor-container').exists()).toBe(false)
    })

    it('navigates to note from full graph', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])
      const getSpy = vi.spyOn(apiClient.notes, 'get').mockResolvedValue({
        id: 'note-5',
        title: 'Graph Note',
      })

      wrapper = mount(HomeView)
      await flushPromises()

      // Open graph
      await wrapper.find('.logo-wrapper').trigger('click')

      // Click node in graph
      await wrapper.find('.full-graph-stub').trigger('click')
      await flushPromises()

      // Should load note and close graph
      expect(getSpy).toHaveBeenCalledWith('note-5')
      expect(wrapper.find('.full-graph-container').exists()).toBe(false)
      expect(wrapper.find('.note-editor-stub').exists()).toBe(true)
    })
  })
})

function nextTick() {
  return new Promise(resolve => setTimeout(resolve, 0))
}
