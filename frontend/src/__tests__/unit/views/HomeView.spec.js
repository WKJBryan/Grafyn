/**
 * Unit tests for HomeView
 *
 * Tests cover:
 * - 3-column layout rendering (TreeNav, Main, MiniGraph)
 * - Navigation interactions
 * - Graph (always visible) with editor overlay
 * - Search integration
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import HomeView from '@/views/HomeView.vue'
import * as apiClient from '@/api/client'

// Mock theme store
vi.mock('@/stores/theme', () => ({
  useThemeStore: vi.fn(() => ({
    theme: 'dark',
    toggleTheme: vi.fn(),
  })),
}))

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
    emits: ['save', 'delete', 'close'],
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

vi.mock('@/components/TagTree.vue', () => ({
  default: {
    name: 'TagTree',
    template: '<div class="tag-tree-stub"></div>',
    props: ['tags'],
    emits: ['filter'],
  },
}))

vi.mock('@/components/UnlinkedMentions.vue', () => ({
  default: {
    name: 'UnlinkedMentions',
    template: '<div class="unlinked-mentions-stub"></div>',
    props: ['noteId', 'noteTitle'],
    emits: ['navigate', 'link-created'],
  },
}))

vi.mock('@/components/TopicSelector.vue', () => ({
  default: {
    name: 'TopicSelector',
    template: '<div class="topic-selector-stub"></div>',
    props: ['existingTopics'],
    emits: ['create', 'close'],
  },
}))

vi.mock('@/components/FeedbackModal.vue', () => ({
  default: {
    name: 'FeedbackModal',
    template: '<div class="feedback-modal-stub"></div>',
    emits: ['close', 'submitted'],
  },
}))

vi.mock('@/components/SettingsModal.vue', () => ({
  default: {
    name: 'SettingsModal',
    template: '<div class="settings-modal-stub"></div>',
    props: ['modelValue', 'isSetup'],
    emits: ['update:modelValue', 'saved', 'setup-complete'],
  },
}))

describe('HomeView', () => {
  let wrapper

  beforeEach(() => {
    vi.clearAllMocks()
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    // Mock isDesktopApp
    vi.spyOn(apiClient, 'isDesktopApp').mockReturnValue(false)
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
      expect(wrapper.find('.logo').text()).toBe('Grafyn')
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

    it('keeps the newest selected note when earlier requests finish later', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])

      let resolveFirst
      let resolveSecond
      vi.spyOn(apiClient.notes, 'get').mockImplementation((id) => {
        if (id === 'note-2') {
          return new Promise((resolve) => { resolveFirst = resolve })
        }
        if (id === 'note-4') {
          return new Promise((resolve) => { resolveSecond = resolve })
        }
        return Promise.resolve({ id, title: id })
      })

      wrapper = mount(HomeView)
      await flushPromises()

      await wrapper.find('.tree-nav-stub').trigger('click')
      await wrapper.find('.mini-graph-stub').trigger('click')

      resolveSecond({ id: 'note-4', title: 'Newest Note' })
      await flushPromises()
      resolveFirst({ id: 'note-2', title: 'Older Note' })
      await flushPromises()

      expect(wrapper.find('.editor-panel-title').text()).toBe('Newest Note')
    })
  })

  // ============================================================================
  // Full Graph View (Always Visible)
  // ============================================================================

  describe('Full Graph View', () => {
    it('shows graph container by default', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])

      wrapper = mount(HomeView)
      await flushPromises()

      expect(wrapper.find('.full-graph-container').exists()).toBe(true)
      expect(wrapper.find('.full-graph-stub').exists()).toBe(true)
    })

    it('opens editor overlay when graph node is clicked', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])
      vi.spyOn(apiClient.notes, 'get').mockResolvedValue({
        id: 'note-5',
        title: 'Graph Note',
      })

      wrapper = mount(HomeView)
      await flushPromises()

      await wrapper.find('.full-graph-stub').trigger('click')
      await flushPromises()

      // Editor opens as overlay, graph stays visible
      expect(wrapper.find('.editor-panel-overlay').exists()).toBe(true)
      expect(wrapper.find('.note-editor-stub').exists()).toBe(true)
      expect(wrapper.find('.full-graph-container').exists()).toBe(true)
    })

    it('saves the title emitted by NoteEditor instead of forcing the stale header title', async () => {
      vi.spyOn(apiClient.notes, 'list').mockResolvedValue([])
      vi.spyOn(apiClient.notes, 'get')
        .mockResolvedValueOnce({
          id: 'note-5',
          title: 'Original Title',
          content: '',
          status: 'draft',
          tags: []
        })
        .mockResolvedValueOnce({
          id: 'note-5',
          title: 'Updated',
          content: '',
          status: 'draft',
          tags: []
        })
      const updateSpy = vi.spyOn(apiClient.notes, 'update').mockResolvedValue({})

      wrapper = mount(HomeView)
      await flushPromises()

      await wrapper.find('.full-graph-stub').trigger('click')
      await flushPromises()
      await wrapper.find('.save-btn').trigger('click')
      await flushPromises()

      expect(updateSpy).toHaveBeenCalledWith('note-5', expect.objectContaining({
        title: 'Updated'
      }))
    })
  })
})
