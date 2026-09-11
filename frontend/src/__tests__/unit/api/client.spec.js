/**
 * Unit tests for Tauri-only API client
 *
 * Tests cover:
 * - Notes API methods (list, get, create, update, delete, reindex, distill, normalizeTags)
 * - Search API methods (query, similar)
 * - Graph API methods (backlinks, outgoing, neighbors, rebuild, full, unlinked)
 * - Canvas API methods (list, get, create, update, delete, getModels, sendPrompt, etc.)
 * - Twin collector API methods (records, traces, feedback, export)
 * - Feedback API methods (submit, status, getSystemInfo, getPending, retryPending)
 * - Settings API methods (get, getStatus, update, completeSetup, pickVaultFolder, etc.)
 * - MCP API methods (getStatus, getConfigSnippet)
 * - Memory API methods (recall, contradictions, extract)
 * - Zettelkasten API methods (discoverLinks, applyLinks, createLink, getLinkTypes)
 * - isDesktopApp detects a Tauri 2 desktop runtime without admitting mobile
 */

import { describe, it, expect, vi, beforeEach } from 'vitest'

// vi.hoisted ensures mockInvoke is declared before vi.mock's hoisted factory runs
const { mockInvoke, runtime } = vi.hoisted(() => ({
  mockInvoke: vi.fn(),
  runtime: {
    isTauri: false,
    platform: 'windows',
  },
}))
vi.mock('@tauri-apps/api/core', () => ({
  invoke: mockInvoke,
  isTauri: () => runtime.isTauri,
}))
vi.mock('@tauri-apps/plugin-os', () => ({
  platform: () => runtime.platform,
}))

import {
  boot,
  runtime as runtimeApi,
  notes,
  search,
  graph,
  canvas,
  images,
  twin,
  feedback,
  settings,
  sync,
  mcp,
  memory,
  zettelkasten,
  isDesktopApp,
} from '@/api/client'

describe('API Client (Tauri)', () => {
  beforeEach(() => {
    mockInvoke.mockReset()
    runtime.isTauri = false
    runtime.platform = 'windows'
    delete window.__TAURI_IPC__
  })

  describe('isDesktopApp', () => {
    it('uses the Tauri 2 runtime API instead of the removed Tauri 1 IPC global', () => {
      window.__TAURI_IPC__ = vi.fn()
      expect(isDesktopApp()).toBe(false)

      runtime.isTauri = true
      expect(isDesktopApp()).toBe(true)
    })

    it.each(['windows', 'macos', 'linux'])('admits the %s desktop runtime', (platform) => {
      runtime.isTauri = true
      runtime.platform = platform

      expect(isDesktopApp()).toBe(true)
    })

    it.each(['android', 'ios'])('rejects the %s mobile runtime', (platform) => {
      runtime.isTauri = true
      runtime.platform = platform

      expect(isDesktopApp()).toBe(false)
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
      const request = { prompt: 'hello', models: ['gpt-4'], web_search_max_results: 8 }
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

  describe('Twin API', () => {
    it('listRecords() invokes list_user_records', async () => {
      mockInvoke.mockResolvedValue([])
      await twin.listRecords()
      expect(mockInvoke).toHaveBeenCalledWith('list_user_records', {})
    })

    it('recordCanvasFeedback() invokes record_canvas_feedback', async () => {
      const request = {
        feedback_type: 'accept',
        response: { tile_id: 'tile-1', model_id: 'model-a' }
      }

      mockInvoke.mockResolvedValue({ trace_event_id: 'evt-1', created_record_ids: ['rec-1'] })
      await twin.recordCanvasFeedback('session-1', request)
      expect(mockInvoke).toHaveBeenCalledWith('record_canvas_feedback', {
        sessionId: 'session-1',
        request
      })
    })

    it('runInference() invokes run_twin_inference', async () => {
      mockInvoke.mockResolvedValue({ created_records: 1 })
      await twin.runInference()
      expect(mockInvoke).toHaveBeenCalledWith('run_twin_inference', {})
    })

    it('getReview() invokes get_twin_review', async () => {
      mockInvoke.mockResolvedValue([])
      await twin.getReview()
      expect(mockInvoke).toHaveBeenCalledWith('get_twin_review', {})
    })

    it('createCompanionCapture() invokes the exact command and normalizes its response', async () => {
      const request = {
        content: 'A field note',
        captureKind: 'text',
        context: {
          person: '',
          role: '',
          relationship: '',
          environment: 'train',
          activity: 'reading',
          goal: '',
        },
        attachmentDigests: [],
        grafynSync: 'inherit',
      }
      mockInvoke.mockResolvedValue({
        note: { id: 'note-1', title: 'A field note' },
        observation_event_id: 'event-1',
      })

      await expect(twin.createCompanionCapture(request)).resolves.toEqual({
        note: { id: 'note-1', title: 'A field note' },
        observationEventId: 'event-1',
      })
      expect(mockInvoke).toHaveBeenCalledWith('create_companion_capture', { request })
      expect(request).not.toHaveProperty('title')
    })

    it('listObservations() invokes list_twin_observations with its exact request', async () => {
      const request = {
        referenceTime: '2026-08-31T10:00:00Z',
        filter: {
          relationships: [{
            subjectId: 'owner',
            predicate: 'works_with',
            objectId: 'person-a',
            direction: 'directed'
          }],
          goals: ['ship'],
          tags: ['work']
        },
        cursor: 'cursor-observation',
        limit: 25
      }

      mockInvoke.mockResolvedValue({ items: [] })
      await twin.listObservations(request)

      expect(mockInvoke).toHaveBeenCalledWith('list_twin_observations', { request })
    })

    it('listProposals() invokes list_twin_proposals with its exact request', async () => {
      const request = {
        referenceTime: '2026-08-31T10:00:00Z',
        filter: { relationships: [], goals: [], tags: ['planning'] },
        cursor: null,
        limit: 20
      }

      mockInvoke.mockResolvedValue({ items: [] })
      await twin.listProposals(request)

      expect(mockInvoke).toHaveBeenCalledWith('list_twin_proposals', { request })
    })

    it('reviewProposal() invokes review_twin_proposal with optimistic snapshot fields', async () => {
      const request = {
        memoryId: 'memory-1',
        decision: 'accept',
        reviewedClaim: {
          subject_id: 'owner',
          predicate: 'prefers',
          object: 'evidence first',
          polarity: 'affirmed'
        },
        rationale: 'This matches my intent.',
        expectedSnapshotId: 'a'.repeat(64),
        snapshotReferenceTime: '2026-08-31T10:00:00Z'
      }

      mockInvoke.mockResolvedValue({ reviewEventId: 'event-1' })
      await twin.reviewProposal(request)

      expect(mockInvoke).toHaveBeenCalledWith('review_twin_proposal', { request })
    })

    it('getStateProjection() invokes get_twin_state_projection with its exact request', async () => {
      const request = { referenceTime: '2026-08-31T10:00:00Z' }

      mockInvoke.mockResolvedValue({ snapshot_id: 'a'.repeat(64) })
      await twin.getStateProjection(request)

      expect(mockInvoke).toHaveBeenCalledWith('get_twin_state_projection', { request })
    })

    it('rankAttention() invokes rank_twin_attention with its exact request', async () => {
      const request = {
        referenceTime: '2026-08-31T10:00:00Z',
        profile: 'decision',
        query: 'What should I prioritize?',
        relationshipVariant: {
          relationships: [{
            subject_id: 'owner',
            predicate: 'works_with',
            object_id: 'person-a',
            direction: 'directed'
          }]
        },
        goals: ['ship'],
        destination: 'local',
        filter: { relationships: [], goals: ['ship'], tags: [] },
        limit: 10
      }

      mockInvoke.mockResolvedValue({ trace: { selected: [], excluded: [] } })
      await twin.rankAttention(request)

      expect(mockInvoke).toHaveBeenCalledWith('rank_twin_attention', { request })
    })

    it('getEventTimeline() invokes get_twin_event_timeline with its exact request', async () => {
      const request = {
        referenceTime: '2026-08-31T10:00:00Z',
        filter: { relationships: [], goals: ['ship'], tags: ['work'] },
        cursor: 'cursor-timeline',
        limit: 30
      }

      mockInvoke.mockResolvedValue({ items: [] })
      await twin.getEventTimeline(request)

      expect(mockInvoke).toHaveBeenCalledWith('get_twin_event_timeline', { request })
    })

    it('resolveEvidence() invokes resolve_user_record_evidence', async () => {
      mockInvoke.mockResolvedValue([])
      await twin.resolveEvidence('record-1')
      expect(mockInvoke).toHaveBeenCalledWith('resolve_user_record_evidence', { id: 'record-1' })
    })

    it('setPromotion() invokes set_user_record_promotion', async () => {
      mockInvoke.mockResolvedValue({ id: 'record-1', promotion_state: 'rejected' })
      await twin.setPromotion('record-1', 'rejected', 'wrong inference')
      expect(mockInvoke).toHaveBeenCalledWith('set_user_record_promotion', {
        id: 'record-1',
        promotionState: 'rejected',
        rationale: 'wrong inference'
      })
    })

    it('exportData() invokes export_twin_data', async () => {
      const request = { eval_percentage: 20 }
      mockInvoke.mockResolvedValue({ train: { count: 1 }, eval: { count: 1 }, holdout: { count: 0 } })
      await twin.exportData(request)
      expect(mockInvoke).toHaveBeenCalledWith('export_twin_data', { request })
    })

    it('Decision Mirror APIs invoke their twin commands', async () => {
      mockInvoke.mockResolvedValue([])
      await twin.listDecisionEpisodes()
      expect(mockInvoke).toHaveBeenLastCalledWith('list_decision_episodes', {})

      await twin.updateDecisionOutcome('decision-1', { outcome: 'shipped' })
      expect(mockInvoke).toHaveBeenLastCalledWith('update_decision_outcome', {
        id: 'decision-1',
        update: { outcome: 'shipped' }
      })

      await twin.getDecisionMirrorConfig()
      expect(mockInvoke).toHaveBeenLastCalledWith('get_decision_mirror_config', {})

      await twin.updateDecisionMirrorConfig({ preset: 'evidence_strict' })
      expect(mockInvoke).toHaveBeenLastCalledWith('update_decision_mirror_config', {
        update: { preset: 'evidence_strict' }
      })

      await twin.resetDecisionMirrorConfig()
      expect(mockInvoke).toHaveBeenLastCalledWith('reset_decision_mirror_config', {})

      await twin.listMemoryDigest()
      expect(mockInvoke).toHaveBeenLastCalledWith('list_memory_digest', {})

      await twin.reviewMemoryDigestItem('digest-1', { action: 'keep' })
      expect(mockInvoke).toHaveBeenLastCalledWith('review_memory_digest_item', {
        id: 'digest-1',
        request: { action: 'keep' }
      })

      await twin.listConstitutionItems()
      expect(mockInvoke).toHaveBeenLastCalledWith('list_constitution_items', {})

      await twin.createConstitutionItem({ claim: 'Validate first', dimension: 'values' })
      expect(mockInvoke).toHaveBeenLastCalledWith('create_constitution_item', {
        item: { claim: 'Validate first', dimension: 'values' }
      })

      await twin.updateConstitutionItem('constitution-1', { confidence: 0.9 })
      expect(mockInvoke).toHaveBeenLastCalledWith('update_constitution_item', {
        id: 'constitution-1',
        update: { confidence: 0.9 }
      })

      await twin.reviewConstitutionItem('constitution-1', { action: 'soften' })
      expect(mockInvoke).toHaveBeenLastCalledWith('review_constitution_item', {
        id: 'constitution-1',
        request: { action: 'soften' }
      })

      await twin.listActionGaps()
      expect(mockInvoke).toHaveBeenLastCalledWith('list_action_gaps', {})

      await twin.reviewActionGap('gap-1', { action: 'not_me' })
      expect(mockInvoke).toHaveBeenLastCalledWith('review_action_gap', {
        id: 'gap-1',
        request: { action: 'not_me' }
      })

      await twin.getConstitutionSetup()
      expect(mockInvoke).toHaveBeenLastCalledWith('get_constitution_setup', {})

      await twin.saveConstitutionSetup({ values: ['proof'] })
      expect(mockInvoke).toHaveBeenLastCalledWith('save_constitution_setup', {
        setup: { values: ['proof'] }
      })

      await twin.runConstitutionInference()
      expect(mockInvoke).toHaveBeenLastCalledWith('run_constitution_inference', {})
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

    it('getOllamaStatus() invokes get_ollama_status', async () => {
      mockInvoke.mockResolvedValue({ available: true })
      await settings.getOllamaStatus()
      expect(mockInvoke).toHaveBeenCalledWith('get_ollama_status', {})
    })

    it('listOllamaModels() invokes list_ollama_models', async () => {
      mockInvoke.mockResolvedValue([])
      await settings.listOllamaModels()
      expect(mockInvoke).toHaveBeenCalledWith('list_ollama_models', {})
    })
  })

  describe('Runtime API', () => {
    it('gets the typed backend runtime status', async () => {
      mockInvoke.mockResolvedValue({ schemaVersion: 1, runtime: 'android' })

      await runtimeApi.getStatus()

      expect(mockInvoke).toHaveBeenCalledWith('get_runtime_status', {})
    })
  })

  describe('Generated image API', () => {
    it('maps discovery, capability, generation, save, export, share, and load to strict request DTOs', async () => {
      mockInvoke.mockResolvedValue({})

      await images.discoverModels()
      expect(mockInvoke).toHaveBeenLastCalledWith('discover_image_models', { request: {} })

      await images.getModelCapability('author/image-model')
      expect(mockInvoke).toHaveBeenLastCalledWith('get_image_model_capability', {
        request: { modelId: 'author/image-model' },
      })

      await images.generate({
        prompt: 'A quiet workspace',
        modelId: 'author/image-model',
        resolution: '1024x1024',
        aspectRatio: '1:1',
      })
      expect(mockInvoke).toHaveBeenLastCalledWith('generate_image', {
        request: {
          prompt: 'A quiet workspace',
          modelId: 'author/image-model',
          resolution: '1024x1024',
          aspectRatio: '1:1',
        },
      })

      await images.save({
        receiptId: '018f0ca8-2e42-7c1e-ae13-7b35f09b4501',
        annotation: 'Concept sketch',
        retentionPolicy: 'strip_metadata',
        grafynSync: 'local_only',
      })
      expect(mockInvoke).toHaveBeenLastCalledWith('save_generated_image', {
        request: {
          receiptId: '018f0ca8-2e42-7c1e-ae13-7b35f09b4501',
          annotation: 'Concept sketch',
          retentionPolicy: 'strip_metadata',
          grafynSync: 'local_only',
        },
      })

      await images.saveAs('018f0ca8-2e42-7c1e-ae13-7b35f09b4501', 'strip_metadata')
      expect(mockInvoke).toHaveBeenLastCalledWith('export_generated_image', {
        request: {
          receiptId: '018f0ca8-2e42-7c1e-ae13-7b35f09b4501',
          retentionPolicy: 'strip_metadata',
        },
      })

      await images.shareGeneratedImage(
        '018f0ca8-2e42-7c1e-ae13-7b35f09b4501',
        'retain_original',
      )
      expect(mockInvoke).toHaveBeenLastCalledWith('share_generated_image', {
        request: {
          receiptId: '018f0ca8-2e42-7c1e-ae13-7b35f09b4501',
          retentionPolicy: 'retain_original',
        },
      })

      await images.load('a'.repeat(64))
      expect(mockInvoke).toHaveBeenLastCalledWith('load_generated_image', {
        request: { attachmentDigest: 'a'.repeat(64) },
      })

      await images.discard('018f0ca8-2e42-7c1e-ae13-7b35f09b4501')
      expect(mockInvoke).toHaveBeenLastCalledWith('discard_generated_image_receipt', {
        request: { receiptId: '018f0ca8-2e42-7c1e-ae13-7b35f09b4501' },
      })
    })
  })

  // ============================================================================
  // Sync foundation API
  // ============================================================================

  describe('Sync foundation API', () => {
    it('uses only the local status, conflict, ciphertext, import, and rebuild commands', async () => {
      const bundle = { schemaVersion: 1, envelopes: ['{"ciphertext":"opaque"}'] }
      mockInvoke.mockResolvedValue({})

      await sync.getStatus()
      expect(mockInvoke).toHaveBeenLastCalledWith('get_sync_status', {})
      await sync.listConflicts()
      expect(mockInvoke).toHaveBeenLastCalledWith('list_sync_conflicts', {})
      await sync.exportOutbox()
      expect(mockInvoke).toHaveBeenLastCalledWith('export_sync_outbox', {})
      await sync.importEnvelopes(bundle)
      expect(mockInvoke).toHaveBeenLastCalledWith('import_sync_envelopes', { bundle })
      await sync.rebuildState()
      expect(mockInvoke).toHaveBeenLastCalledWith('rebuild_sync_state', {})
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

    it('listSuggestionQueue() invokes list_link_suggestion_queue', async () => {
      mockInvoke.mockResolvedValue([])
      await zettelkasten.listSuggestionQueue('pending', 12)
      expect(mockInvoke).toHaveBeenCalledWith('list_link_suggestion_queue', {
        status: 'pending',
        limit: 12,
      })
    })

    it('dismissSuggestion() invokes dismiss_link_suggestion', async () => {
      mockInvoke.mockResolvedValue({})
      await zettelkasten.dismissSuggestion('n1', 'n2')
      expect(mockInvoke).toHaveBeenCalledWith('dismiss_link_suggestion', {
        noteId: 'n1',
        targetId: 'n2',
      })
    })

    it('getDiscoveryStatus() invokes get_link_discovery_status', async () => {
      mockInvoke.mockResolvedValue({})
      await zettelkasten.getDiscoveryStatus()
      expect(mockInvoke).toHaveBeenCalledWith('get_link_discovery_status', {})
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
