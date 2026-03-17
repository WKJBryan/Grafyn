/**
 * Unit tests for Tauri-only API client
 *
 * Tests cover:
 * - Notes API methods (list, get, create, update, delete, reindex, distill, normalizeTags)
 * - Search API methods (query, similar)
 * - Graph API methods (backlinks, outgoing, neighbors, rebuild, full, unlinked)
 * - Canvas API methods (list, get, create, update, delete, getModels, sendPrompt, etc.)
 * - Feedback API methods (submit, status, getSystemInfo, getPending, retryPending)
 * - Settings API methods (get, getStatus, update, completeSetup, pickVaultFolder, etc.)
 * - MCP API methods (getStatus, getConfigSnippet)
 * - Memory API methods (recall, contradictions, extract)
 * - Zettelkasten API methods (discoverLinks, applyLinks, createLink, getLinkTypes)
 * - isDesktopApp always returns true
 */

import { describe, it, expect, vi, beforeEach } from 'vitest'

// vi.hoisted ensures mockInvoke is declared before vi.mock's hoisted factory runs
const { mockInvoke } = vi.hoisted(() => ({ mockInvoke: vi.fn() }))
vi.mock('@tauri-apps/api/tauri', () => ({
  invoke: mockInvoke,
}))

import {
  boot,
  notes,
  search,
  graph,
  canvas,
  feedback,
  settings,
  mcp,
  memory,
  zettelkasten,
  isDesktopApp,
} from '@/api/client'

describe('API Client (Tauri)', () => {
  beforeEach(() => {
    mockInvoke.mockReset()
  })

  describe('isDesktopApp', () => {
    it('always returns true', () => {
      expect(isDesktopApp()).toBe(true)
    })
  })

  describe('Boot API', () => {
    it('status() invokes get_boot_status', async () => {
      mockInvoke.mockResolvedValue({ ready: false })
      await boot.status()
      expect(mockInvoke).toHaveBeenCalledWith('get_boot_status', {})
    })
  })

  // ============================================================================
  // Notes API
  // ============================================================================

  describe('Notes API', () => {
    it('list() invokes list_notes', async () => {
      mockInvoke.mockResolvedValue([{ id: '1', title: 'Test' }])
      const result = await notes.list()
      expect(mockInvoke).toHaveBeenCalledWith('list_notes', {})
      expect(result).toEqual([{ id: '1', title: 'Test' }])
    })

    it('get() invokes get_note with id', async () => {
      mockInvoke.mockResolvedValue({ id: 'n1', title: 'Note' })
      await notes.get('n1')
      expect(mockInvoke).toHaveBeenCalledWith('get_note', { id: 'n1' })
    })

    it('create() invokes create_note with note data', async () => {
      const data = { title: 'New', content: 'Body' }
      mockInvoke.mockResolvedValue({ id: 'new', ...data })
      await notes.create(data)
      expect(mockInvoke).toHaveBeenCalledWith('create_note', { note: data })
    })

    it('update() invokes update_note with id and update', async () => {
      const data = { title: 'Updated' }
      mockInvoke.mockResolvedValue({ id: 'n1', ...data })
      await notes.update('n1', data)
      expect(mockInvoke).toHaveBeenCalledWith('update_note', { id: 'n1', update: data })
    })

    it('delete() invokes delete_note with id', async () => {
      mockInvoke.mockResolvedValue(null)
      await notes.delete('n1')
      expect(mockInvoke).toHaveBeenCalledWith('delete_note', { id: 'n1' })
    })

    it('reindex() invokes reindex', async () => {
      mockInvoke.mockResolvedValue(null)
      await notes.reindex()
      expect(mockInvoke).toHaveBeenCalledWith('reindex', {})
    })

    it('distill() invokes distill_note with id and request', async () => {
      const request = { mode: 'rules' }
      mockInvoke.mockResolvedValue({})
      await notes.distill('n1', request)
      expect(mockInvoke).toHaveBeenCalledWith('distill_note', { id: 'n1', request })
    })

    it('normalizeTags() invokes normalize_tags with id', async () => {
      mockInvoke.mockResolvedValue({})
      await notes.normalizeTags('n1')
      expect(mockInvoke).toHaveBeenCalledWith('normalize_tags', { id: 'n1' })
    })
  })

  // ============================================================================
  // Search API
  // ============================================================================

  describe('Search API', () => {
    it('query() invokes search_notes with defaults', async () => {
      mockInvoke.mockResolvedValue([])
      await search.query('test')
      expect(mockInvoke).toHaveBeenCalledWith('search_notes', { query: 'test', limit: 10 })
    })

    it('query() uses custom limit', async () => {
      mockInvoke.mockResolvedValue([])
      await search.query('test', { limit: 5 })
      expect(mockInvoke).toHaveBeenCalledWith('search_notes', { query: 'test', limit: 5 })
    })

    it('similar() invokes find_similar with defaults', async () => {
      mockInvoke.mockResolvedValue([])
      await search.similar('n1')
      expect(mockInvoke).toHaveBeenCalledWith('find_similar', { noteId: 'n1', limit: 5 })
    })

    it('similar() uses custom limit', async () => {
      mockInvoke.mockResolvedValue([])
      await search.similar('n1', 3)
      expect(mockInvoke).toHaveBeenCalledWith('find_similar', { noteId: 'n1', limit: 3 })
    })
  })

  // ============================================================================
  // Graph API
  // ============================================================================

  describe('Graph API', () => {
    it('backlinks() invokes get_backlinks', async () => {
      mockInvoke.mockResolvedValue([])
      await graph.backlinks('n1')
      expect(mockInvoke).toHaveBeenCalledWith('get_backlinks', { noteId: 'n1' })
    })

    it('outgoing() invokes get_outgoing', async () => {
      mockInvoke.mockResolvedValue([])
      await graph.outgoing('n1')
      expect(mockInvoke).toHaveBeenCalledWith('get_outgoing', { noteId: 'n1' })
    })

    it('neighbors() invokes get_neighbors', async () => {
      mockInvoke.mockResolvedValue({ nodes: [], edges: [] })
      await graph.neighbors('n1', 2)
      expect(mockInvoke).toHaveBeenCalledWith('get_neighbors', { noteId: 'n1' })
    })

    it('rebuild() invokes rebuild_graph', async () => {
      mockInvoke.mockResolvedValue(null)
      await graph.rebuild()
      expect(mockInvoke).toHaveBeenCalledWith('rebuild_graph', {})
    })

    it('full() invokes get_full_graph', async () => {
      mockInvoke.mockResolvedValue({ nodes: [], edges: [] })
      await graph.full()
      expect(mockInvoke).toHaveBeenCalledWith('get_full_graph', {})
    })

    it('unlinked() invokes get_unlinked', async () => {
      mockInvoke.mockResolvedValue([])
      await graph.unlinked()
      expect(mockInvoke).toHaveBeenCalledWith('get_unlinked', {})
    })
  })

  // ============================================================================
  // Canvas API
  // ============================================================================

  describe('Canvas API', () => {
    it('list() invokes list_sessions', async () => {
      mockInvoke.mockResolvedValue([])
      await canvas.list()
      expect(mockInvoke).toHaveBeenCalledWith('list_sessions', {})
    })

    it('get() invokes get_session', async () => {
      mockInvoke.mockResolvedValue({})
      await canvas.get('s1')
      expect(mockInvoke).toHaveBeenCalledWith('get_session', { id: 's1' })
    })

    it('create() invokes create_session', async () => {
      const data = { name: 'Test' }
      mockInvoke.mockResolvedValue({ id: 's1', ...data })
      await canvas.create(data)
      expect(mockInvoke).toHaveBeenCalledWith('create_session', { session: data })
    })

    it('delete() invokes delete_session', async () => {
      mockInvoke.mockResolvedValue(null)
      await canvas.delete('s1')
      expect(mockInvoke).toHaveBeenCalledWith('delete_session', { id: 's1' })
    })

    it('getModels() invokes get_available_models', async () => {
      mockInvoke.mockResolvedValue([])
      await canvas.getModels()
      expect(mockInvoke).toHaveBeenCalledWith('get_available_models', {})
    })

    it('sendPrompt() invokes send_prompt', async () => {
      const request = { prompt: 'hello', models: ['gpt-4'] }
      mockInvoke.mockResolvedValue('tile-1')
      await canvas.sendPrompt('s1', request)
      expect(mockInvoke).toHaveBeenCalledWith('send_prompt', { sessionId: 's1', request })
    })

    it('exportToNote() invokes export_to_note', async () => {
      mockInvoke.mockResolvedValue({ note_id: 'n1' })
      await canvas.exportToNote('s1')
      expect(mockInvoke).toHaveBeenCalledWith('export_to_note', { sessionId: 's1' })
    })
  })

  // ============================================================================
  // Feedback API
  // ============================================================================

  describe('Feedback API', () => {
    it('submit() invokes submit_feedback', async () => {
      const data = { type: 'bug', description: 'broken' }
      mockInvoke.mockResolvedValue({})
      await feedback.submit(data)
      expect(mockInvoke).toHaveBeenCalledWith('submit_feedback', { feedback: data })
    })

    it('status() invokes feedback_status', async () => {
      mockInvoke.mockResolvedValue({})
      await feedback.status()
      expect(mockInvoke).toHaveBeenCalledWith('feedback_status', {})
    })

    it('getSystemInfo() invokes get_system_info', async () => {
      mockInvoke.mockResolvedValue({ platform: 'win32' })
      await feedback.getSystemInfo('/canvas')
      expect(mockInvoke).toHaveBeenCalledWith('get_system_info', { currentPage: '/canvas' })
    })
  })

  // ============================================================================
  // Settings API
  // ============================================================================

  describe('Settings API', () => {
    it('get() invokes get_settings', async () => {
      mockInvoke.mockResolvedValue({})
      await settings.get()
      expect(mockInvoke).toHaveBeenCalledWith('get_settings', {})
    })

    it('getStatus() invokes get_settings_status', async () => {
      mockInvoke.mockResolvedValue({ needs_setup: false })
      await settings.getStatus()
      expect(mockInvoke).toHaveBeenCalledWith('get_settings_status', {})
    })

    it('update() invokes update_settings', async () => {
      const data = { theme: 'dark' }
      mockInvoke.mockResolvedValue(data)
      await settings.update(data)
      expect(mockInvoke).toHaveBeenCalledWith('update_settings', { update: data })
    })

    it('pickVaultFolder() invokes pick_vault_folder', async () => {
      mockInvoke.mockResolvedValue('/path/to/vault')
      await settings.pickVaultFolder()
      expect(mockInvoke).toHaveBeenCalledWith('pick_vault_folder')
    })

    it('validateOpenRouterKey() invokes validate_openrouter_key', async () => {
      mockInvoke.mockResolvedValue(true)
      await settings.validateOpenRouterKey('sk-123')
      expect(mockInvoke).toHaveBeenCalledWith('validate_openrouter_key', { apiKey: 'sk-123' })
    })
  })

  // ============================================================================
  // MCP API
  // ============================================================================

  describe('MCP API', () => {
    it('getStatus() invokes get_mcp_status', async () => {
      mockInvoke.mockResolvedValue({ available: true })
      await mcp.getStatus()
      expect(mockInvoke).toHaveBeenCalledWith('get_mcp_status', {})
    })

    it('getConfigSnippet() invokes get_mcp_config_snippet', async () => {
      mockInvoke.mockResolvedValue('{}')
      await mcp.getConfigSnippet()
      expect(mockInvoke).toHaveBeenCalledWith('get_mcp_config_snippet', {})
    })
  })

  // ============================================================================
  // Memory API
  // ============================================================================

  describe('Memory API', () => {
    it('recall() invokes recall_relevant with request', async () => {
      mockInvoke.mockResolvedValue([])
      await memory.recall('test query', ['n1'], 3)
      expect(mockInvoke).toHaveBeenCalledWith('recall_relevant', {
        request: { query: 'test query', context_note_ids: ['n1'], limit: 3 },
      })
    })

    it('recall() uses defaults', async () => {
      mockInvoke.mockResolvedValue([])
      await memory.recall('query')
      expect(mockInvoke).toHaveBeenCalledWith('recall_relevant', {
        request: { query: 'query', context_note_ids: [], limit: 5 },
      })
    })

    it('contradictions() invokes find_contradictions', async () => {
      mockInvoke.mockResolvedValue([])
      await memory.contradictions('n1')
      expect(mockInvoke).toHaveBeenCalledWith('find_contradictions', { noteId: 'n1' })
    })

    it('extract() invokes extract_claims', async () => {
      const messages = [{ role: 'user', content: 'hello' }]
      mockInvoke.mockResolvedValue([])
      await memory.extract(messages)
      expect(mockInvoke).toHaveBeenCalledWith('extract_claims', { request: { messages } })
    })
  })

  // ============================================================================
  // Zettelkasten API
  // ============================================================================

  describe('Zettelkasten API', () => {
    it('discoverLinks() invokes discover_links with defaults', async () => {
      mockInvoke.mockResolvedValue([])
      await zettelkasten.discoverLinks('n1')
      expect(mockInvoke).toHaveBeenCalledWith('discover_links', {
        noteId: 'n1',
        mode: 'suggested',
        maxLinks: 10,
      })
    })

    it('discoverLinks() passes explicit algorithm mode', async () => {
      mockInvoke.mockResolvedValue([])
      await zettelkasten.discoverLinks('n1', 'algorithm', 5)
      expect(mockInvoke).toHaveBeenCalledWith('discover_links', {
        noteId: 'n1',
        mode: 'algorithm',
        maxLinks: 5,
      })
    })

    it('discoverLinks() passes explicit llm mode', async () => {
      mockInvoke.mockResolvedValue([])
      await zettelkasten.discoverLinks('n1', 'llm', 7)
      expect(mockInvoke).toHaveBeenCalledWith('discover_links', {
        noteId: 'n1',
        mode: 'llm',
        maxLinks: 7,
      })
    })

    it('applyLinks() invokes apply_links', async () => {
      mockInvoke.mockResolvedValue({})
      const candidates = [
        { target_id: 'l1', target_title: 'Note 1', link_type: 'related', confidence: 0.8, reason: 'A' },
        { target_id: 'l2', target_title: 'Note 2', link_type: 'supports', confidence: 0.7, reason: 'B' },
      ]
      await zettelkasten.applyLinks('n1', candidates)
      expect(mockInvoke).toHaveBeenCalledWith('apply_links', {
        noteId: 'n1',
        request: {
          link_ids: ['l1', 'l2'],
          candidates,
        },
      })
    })

    it('createLink() invokes create_link', async () => {
      mockInvoke.mockResolvedValue({})
      await zettelkasten.createLink('src', 'tgt', 'supports')
      expect(mockInvoke).toHaveBeenCalledWith('create_link', {
        sourceId: 'src',
        targetId: 'tgt',
        linkType: 'supports',
      })
    })

    it('getLinkTypes() invokes get_link_types', async () => {
      mockInvoke.mockResolvedValue([])
      await zettelkasten.getLinkTypes()
      expect(mockInvoke).toHaveBeenCalledWith('get_link_types', {})
    })
  })

  // ============================================================================
  // Error Handling
  // ============================================================================

  describe('Error Handling', () => {
    it('propagates invoke errors', async () => {
      mockInvoke.mockRejectedValue(new Error('Tauri error'))
      await expect(notes.list()).rejects.toThrow('Tauri error')
    })

    it('propagates string errors from Rust backend', async () => {
      mockInvoke.mockRejectedValue('Note not found')
      await expect(notes.get('nonexistent')).rejects.toBe('Note not found')
    })
  })
})
