import { defineStore } from 'pinia'
import { ref, shallowRef, computed, triggerRef, toRaw } from 'vue'
import { canvas as canvasApi, twin as twinApi } from '@/api/client'
import { useAsyncOperation } from '@/composables/useAsyncOperation'

export const DEFAULT_WEB_SEARCH_MAX_RESULTS = 5
export const THINK_HARDER_WEB_SEARCH_MAX_RESULTS = 8
export const REASONING_EFFORTS = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh']
export const THINK_HARDER_PROMPT = 'Think harder and improve your previous answer.'
export const THINK_HARDER_SYSTEM_PROMPT = [
  'You are revisiting your immediately previous answer.',
  'Think more carefully before responding.',
  'Verify factual claims when possible, correct any mistakes, consider edge cases, and improve clarity.',
  'If web results are available, use them to increase accuracy.',
  'Return the improved answer directly and call out any remaining uncertainty briefly.'
].join(' ')

export const useCanvasStore = defineStore('canvas', () => {
  // State
  const sessions = ref([])
  const currentSession = ref(null)
  const availableModels = ref([])
  const loading = ref(false)
  const error = ref(null)
  const { run } = useAsyncOperation(loading, error)
  // shallowRef avoids deep reactivity tracking — these update on every streaming chunk,
  // so deep proxying wastes cycles. Use triggerRef() after mutations to notify watchers.
  const streamingModels = shallowRef(new Set())
  const streamingModelCounts = new Map()
  // Debate streaming state: { [debateId]: { currentRound, models: { [modelId]: text }, completedRounds: [] } }
  // Kept as ref() (not shallowRef) because deeply nested mutations need automatic reactivity for streaming display
  const debateStreamingContent = ref({})

  // shallowRef mutation helpers — wrap mutation + triggerRef
  function addStreaming(modelId) {
    const count = streamingModelCounts.get(modelId) || 0
    streamingModelCounts.set(modelId, count + 1)
    streamingModels.value.add(modelId)
    triggerRef(streamingModels)
  }

  function removeStreaming(modelId) {
    const count = streamingModelCounts.get(modelId) || 0
    if (count <= 1) {
      streamingModelCounts.delete(modelId)
      streamingModels.value.delete(modelId)
    } else {
      streamingModelCounts.set(modelId, count - 1)
    }
    triggerRef(streamingModels)
  }

  function clearStreaming() {
    streamingModelCounts.clear()
    streamingModels.value.clear()
    triggerRef(streamingModels)
  }

  // Tracks streaming keys owned by a single in-flight operation (sendPrompt /
  // addModelToTile / regenerateResponse). Single-ownership rule: the operation's own
  // complete/error handler clears a key the moment that model finishes, and the
  // operation's outer `finally` only clears whatever keys are still pending (crash/
  // timeout safety) — so a given addStreaming() call is decremented exactly once,
  // never both by the handler AND by the outer finally.
  function createStreamingTracker() {
    const pending = new Set()
    return {
      mark(key) {
        if (!pending.has(key)) {
          pending.add(key)
          addStreaming(key)
        }
      },
      clear(key) {
        if (pending.delete(key)) removeStreaming(key)
      },
      finalize() {
        pending.forEach(key => removeStreaming(key))
        pending.clear()
      }
    }
  }

  function normalizeResponse(response) {
    if (!response) return response

    return {
      ...response,
      error_message: response.error_message ?? response.error ?? null
    }
  }

  function normalizeReasoningEffort(reasoningEffort) {
    return REASONING_EFFORTS.includes(reasoningEffort) ? reasoningEffort : 'none'
  }

  function compareReasoningEffort(left, right) {
    return REASONING_EFFORTS.indexOf(normalizeReasoningEffort(left)) -
      REASONING_EFFORTS.indexOf(normalizeReasoningEffort(right))
  }

  function maxReasoningEffort(efforts) {
    return efforts.reduce((max, effort) => (
      compareReasoningEffort(effort, max) > 0 ? normalizeReasoningEffort(effort) : max
    ), 'none')
  }

  function normalizeSession(session) {
    if (!session?.prompt_tiles) return session

    return {
      ...session,
      prompt_tiles: session.prompt_tiles.map(tile => ({
        ...tile,
        prompt_type: tile.prompt_type || 'standard',
        reasoning_effort: normalizeReasoningEffort(tile.reasoning_effort),
        decision_metadata: tile.decision_metadata || null,
        decision_episode_id: tile.decision_episode_id || null,
        responses: Object.fromEntries(
          Object.entries(tile.responses || {}).map(([modelId, response]) => [
            modelId,
            normalizeResponse(response)
          ])
        )
      })),
      debates: (session.debates || []).map(debate => ({
        ...debate,
        reasoning_effort: normalizeReasoningEffort(debate.reasoning_effort)
      }))
    }
  }

  // Silent reconciliation for `session_saved` events fired mid-stream by sendPrompt /
  // addModelToTile / regenerateResponse. Unlike loadSession(), this must NOT touch the
  // global `loading` flag (that flag drives CanvasContainer's full-screen overlay —
  // toggling it on every background disk save produced a loading flash on every prompt
  // completion) and must NOT replace currentSession wholesale, which would (a) wipe
  // content for models still mid-stream when the save landed, and (b) snap tile/node
  // positions back to their pre-drag values if the user dragged something inside the
  // ~150ms position-debounce window (_debouncedPersist).
  //
  // Merge rule: for any tile/debate/response that exists locally, keep the LOCAL
  // position (the server can never have a newer position than the client that set it —
  // positions only ever originate from local drags) and keep LOCAL response
  // content/status for any model still marked streaming (the server's disk copy
  // predates the in-flight chunks). Everything else — prompt text, tokens_used,
  // normalized status fields, tiles/debates the server knows about that we haven't seen
  // a broadcast event for yet — is taken from the fresh server fetch. Tiles/debates that
  // exist locally but are NOT yet in the server's fetch (e.g. a concurrent operation's
  // tile_created that hasn't been persisted yet) are kept as-is, never dropped.
  function mergeById(localItems, fetchedItems, mergeOne) {
    const fetchedById = new Map(fetchedItems.map(item => [item.id, item]))
    const merged = localItems.map(localItem => {
      const fetchedItem = fetchedById.get(localItem.id)
      if (!fetchedItem) return localItem
      fetchedById.delete(localItem.id)
      return mergeOne(localItem, fetchedItem)
    })
    merged.push(...fetchedById.values())
    return merged
  }

  function mergeStreamingSafeResponse(localResponse, fetchedResponse, streamingKey) {
    if (!localResponse) return fetchedResponse
    if (streamingModels.value.has(streamingKey)) return localResponse
    return { ...fetchedResponse, position: localResponse.position || fetchedResponse.position }
  }

  function mergeSavedSession(fetched) {
    if (!currentSession.value || currentSession.value.id !== fetched.id) {
      currentSession.value = fetched
      return
    }

    const mergedTiles = mergeById(currentSession.value.prompt_tiles, fetched.prompt_tiles, (localTile, fetchedTile) => {
      const responses = {}
      for (const [modelId, fetchedResponse] of Object.entries(fetchedTile.responses || {})) {
        responses[modelId] = mergeStreamingSafeResponse(
          localTile.responses?.[modelId],
          fetchedResponse,
          `${localTile.id}:${modelId}`
        )
      }
      return {
        ...fetchedTile,
        position: localTile.position || fetchedTile.position,
        responses
      }
    })

    const mergedDebates = mergeById(currentSession.value.debates || [], fetched.debates || [], (localDebate, fetchedDebate) => ({
      ...fetchedDebate,
      position: localDebate.position || fetchedDebate.position
    }))

    currentSession.value = {
      ...fetched,
      viewport: currentSession.value.viewport ?? fetched.viewport,
      prompt_tiles: mergedTiles,
      debates: mergedDebates
    }
  }

  // Silent refetch used by `session_saved` handlers — does NOT set `loading`, does NOT
  // replace currentSession wholesale. See mergeSavedSession() for the merge rule.
  async function reconcileSessionSaved(sessionId) {
    try {
      const fetched = normalizeSession(await canvasApi.get(sessionId))
      mergeSavedSession(fetched)
    } catch (err) {
      console.error('Failed to reconcile canvas session after save:', err)
    }
  }

  // Getters
  const promptTiles = computed(() => currentSession.value?.prompt_tiles || [])
  const debates = computed(() => currentSession.value?.debates || [])
  const hasSession = computed(() => currentSession.value !== null)
  const isStreaming = computed(() => streamingModels.value.size > 0)

  const modelsByProvider = computed(() => {
    const groups = {}
    for (const model of availableModels.value) {
      const provider = model.provider || 'Other'
      if (!groups[provider]) groups[provider] = []
      groups[provider].push(model)
    }
    return groups
  })

  // Computed edges for mind-map visualization
  const tileEdges = computed(() => {
    if (!currentSession.value) return []
    return currentSession.value.prompt_tiles
      .filter(t => t.parent_tile_id)
      .map(t => ({
        source_tile_id: t.parent_tile_id,
        target_tile_id: t.id,
        source_model_id: t.parent_model_id,
        type: 'prompt'
      }))
  })

  // Computed edges for debates (from source tiles to debate)
  const debateEdges = computed(() => {
    if (!currentSession.value) return []
    const edges = []
    for (const debate of currentSession.value.debates || []) {
      for (const sourceTileId of debate.source_tile_ids || []) {
        edges.push({
          source_tile_id: sourceTileId,
          target_id: debate.id,
          target_type: 'debate',
          type: 'debate'
        })
      }
    }
    return edges
  })

  // Actions
  async function loadSessions() {
    loading.value = true
    error.value = null
    try {
      sessions.value = await canvasApi.list()
    } catch (err) {
      error.value = err.message || 'Failed to load sessions'
      console.error('Failed to load canvas sessions:', err)
    } finally {
      loading.value = false
    }
  }

  async function loadSession(sessionId) {
    loading.value = true
    error.value = null
    try {
      currentSession.value = normalizeSession(await canvasApi.get(sessionId))
    } catch (err) {
      error.value = err.message || 'Failed to load session'
      console.error('Failed to load canvas session:', err)
    } finally {
      loading.value = false
    }
  }

  async function createSession(data = {}) {
    return run(async () => {
      const session = normalizeSession(await canvasApi.create(data))
      sessions.value.unshift(session)
      currentSession.value = session
      return session
    })
  }

  async function updateSession(sessionId, data) {
    try {
      const updated = normalizeSession(await canvasApi.update(sessionId, data))

      // Update in sessions list
      const idx = sessions.value.findIndex(s => s.id === sessionId)
      if (idx !== -1) {
        sessions.value[idx] = { ...sessions.value[idx], ...updated }
      }

      // Update current session if it's the same
      if (currentSession.value?.id === sessionId) {
        currentSession.value = updated
      }

      return updated
    } catch (err) {
      error.value = err.message || 'Failed to update session'
      console.error('Failed to update canvas session:', err)
      throw err
    }
  }

  async function deleteSession(sessionId) {
    try {
      await canvasApi.delete(sessionId)
      sessions.value = sessions.value.filter(s => s.id !== sessionId)

      if (currentSession.value?.id === sessionId) {
        currentSession.value = null
      }
    } catch (err) {
      error.value = err.message || 'Failed to delete session'
      console.error('Failed to delete canvas session:', err)
      throw err
    }
  }

  async function loadModels() {
    try {
      availableModels.value = await canvasApi.getModels()
    } catch (err) {
      console.error('Failed to load models:', err)
    }
  }

  // Helper to set up Tauri event listener for canvas-stream events
  // Returns unlisten cleanup handle
  async function setupTauriStreamListener(sessionId, handlers) {
    const { listen } = await import('@tauri-apps/api/event')
    const unlisten = await listen('canvas-stream', (event) => {
      const data = event.payload
      // Filter events for this session
      if (data.session_id !== sessionId) return

      const handler = handlers[data.type]
      if (handler) handler(data)
    })
    return unlisten
  }

  async function sendPrompt(
    prompt,
    models,
    systemPrompt = null,
    temperature = 0.7,
    _maxTokens = null,
    parentTileId = null,
    parentModelId = null,
    contextMode = 'none',
    twinAnswerMode = 'simulation',
    webSearch = false,
    webSearchMaxResults = DEFAULT_WEB_SEARCH_MAX_RESULTS,
    promptType = 'standard',
    decisionMetadata = null,
    reasoningEffort = 'none',
    twinLlmProvider = null
  ) {
    if (!currentSession.value) {
      throw new Error('No active session')
    }

    const sessionId = currentSession.value.id
    error.value = null

    // Calculate position for new tile when branching from a parent response
    let position = undefined
    if (parentTileId && parentModelId) {
      const parentTile = currentSession.value.prompt_tiles.find(t => t.id === parentTileId)
      if (parentTile && parentTile.responses[parentModelId]) {
        const parentPos = parentTile.responses[parentModelId].position
        position = {
          x: parentPos.x,
          y: parentPos.y + (parentPos.height || 200) + 80,
          width: 400,
          height: 300
        }
      }
    }

    // This operation's own tile id — the ONLY reliable source is the invoke() return
    // value below (a direct 1:1 RPC response), because tile_created is a broadcast
    // event: every concurrent sendPrompt/addModelToTile/regenerateResponse call shares
    // the same listener stream, so a second operation's tile_created can arrive on
    // this operation's listener too. Until we know our own tile id, any tile-scoped
    // event (chunk/complete/error/context_notes) is buffered rather than guessed at,
    // then replayed and filtered once the real id is known — this covers events that
    // arrive (as they do in production, since invoke() and events are separate
    // channels) before the invoke() promise itself resolves.
    //
    // NOTE: buffering scopes events across concurrent operations within THIS window
    // only — Tauri's per-window `window.emit()` isolation is load-bearing here, since
    // events emitted for another window's operations never reach this listener.
    let ownTileId = null
    const pendingEvents = []
    const tracker = createStreamingTracker()

    try {
      const modelContent = {}
      models.forEach(m => { modelContent[m] = '' })

      const request = {
        prompt,
        prompt_type: promptType,
        models,
        system_prompt: systemPrompt,
        temperature,
        parent_tile_id: parentTileId,
        parent_model_id: parentModelId,
        context_mode: contextMode,
        twin_answer_mode: twinAnswerMode,
        twin_context_policy: contextMode === 'twin' ? 'approved_plus_relevant_candidates' : null,
        twin_llm_provider: twinLlmProvider,
        decision_metadata: decisionMetadata,
        reasoning_effort: normalizeReasoningEffort(reasoningEffort),
        position,
        web_search: webSearch,
        web_search_max_results: webSearchMaxResults
      }

      const applyContextNotes = (data) => {
        if (currentSession.value) {
          const tile = currentSession.value.prompt_tiles.find(t => t.id === data.tile_id)
          if (tile) tile.context_notes = data.notes || []
        }
      }
      const applyChunk = (data) => {
        modelContent[data.model_id] = (modelContent[data.model_id] || '') + data.chunk
        updateTileResponseLocal(data.tile_id, data.model_id, modelContent[data.model_id], 'streaming')
      }
      const applyComplete = (data) => {
        updateTileResponseLocal(data.tile_id, data.model_id, modelContent[data.model_id], 'completed')
        tracker.clear(`${ownTileId}:${data.model_id}`)
      }
      const applyError = (data) => {
        updateTileResponseLocal(data.tile_id, data.model_id, '', 'error', data.error)
        tracker.clear(`${ownTileId}:${data.model_id}`)
      }
      const scopedAppliers = { context_notes: applyContextNotes, chunk: applyChunk, complete: applyComplete, error: applyError }

      // Set up event listener BEFORE calling invoke
      const unlisten = await setupTauriStreamListener(sessionId, {
        tile_created: (data) => {
          // Safe for every concurrent listener: push-if-absent dedupes the tile so N
          // concurrent operations broadcasting to each other's listeners still result
          // in exactly one push per tile, regardless of which operation it belongs to.
          if (currentSession.value && data.tile) {
            const exists = currentSession.value.prompt_tiles.some(t => t.id === data.tile.id)
            if (!exists) currentSession.value.prompt_tiles.push(data.tile)
          }
        },
        context_notes: (data) => {
          if (ownTileId === null) { pendingEvents.push(['context_notes', data]); return }
          if (data.tile_id !== ownTileId) return
          applyContextNotes(data)
        },
        chunk: (data) => {
          if (ownTileId === null) { pendingEvents.push(['chunk', data]); return }
          if (data.tile_id !== ownTileId) return
          applyChunk(data)
        },
        complete: (data) => {
          if (ownTileId === null) { pendingEvents.push(['complete', data]); return }
          if (data.tile_id !== ownTileId) return
          applyComplete(data)
        },
        error: (data) => {
          if (ownTileId === null) { pendingEvents.push(['error', data]); return }
          if (data.tile_id !== ownTileId) return
          applyError(data)
        },
        session_saved: async () => {
          await reconcileSessionSaved(sessionId)
        }
      })

      try {
        // invoke returns tile_id immediately; streaming happens via events
        const tileId = await canvasApi.sendPrompt(sessionId, request)
        ownTileId = tileId
        models.forEach(m => tracker.mark(`${tileId}:${m}`))

        // Replay anything buffered while we didn't yet know our own tile id, dropping
        // events that turned out to belong to a different concurrent operation.
        pendingEvents.splice(0).forEach(([type, data]) => {
          if (data.tile_id === ownTileId) scopedAppliers[type](data)
        })

        await waitForModelsComplete(models.map(m => `${tileId}:${m}`), 120000)
        return tileId
      } finally {
        unlisten()
      }
    } catch (err) {
      error.value = err.message || 'Failed to send prompt'
      console.error('Failed to send prompt:', err)
      throw err
    } finally {
      tracker.finalize()
    }
  }

  // Wait for all streaming keys to complete (or timeout). `keys` are the composite
  // `${tileId}:${modelId}` streaming-state keys owned by the calling operation.
  function waitForModelsComplete(keys, timeoutMs = 120000) {
    return new Promise((resolve) => {
      const start = Date.now()
      const check = () => {
        const stillStreaming = keys.some(k => streamingModels.value.has(k))
        if (!stillStreaming || Date.now() - start > timeoutMs) {
          resolve()
        } else {
          setTimeout(check, 100)
        }
      }
      check()
    })
  }

  function updateTileResponseLocal(tileId, modelId, content, status, errorMessage = null) {
    if (!currentSession.value) return

    const tile = currentSession.value.prompt_tiles.find(t => t.id === tileId)
    if (tile && tile.responses[modelId]) {
      tile.responses[modelId].content = content
      tile.responses[modelId].status = status
      tile.responses[modelId].error = status === 'error'
        ? (errorMessage || content || 'Error occurred')
        : null
      tile.responses[modelId].error_message = status === 'error'
        ? (errorMessage || content || 'Error occurred')
        : null
    }
  }

  function collectDescendantTileIds(tileId, parentModelId = null) {
    if (!currentSession.value) return new Set()

    const descendants = new Set()
    const queue = currentSession.value.prompt_tiles
      .filter(tile =>
        tile.parent_tile_id === tileId &&
        (parentModelId === null || tile.parent_model_id === parentModelId)
      )
      .map(tile => tile.id)

    while (queue.length > 0) {
      const currentId = queue.shift()
      if (!currentId || descendants.has(currentId)) continue

      descendants.add(currentId)

      const directChildren = currentSession.value.prompt_tiles
        .filter(tile => tile.parent_tile_id === currentId)
        .map(tile => tile.id)

      queue.push(...directChildren)
    }

    return descendants
  }

  function pruneDependentDebates(removedTileIds, deletedResponse = null) {
    if (!currentSession.value) return []

    return currentSession.value.debates.filter(debate => {
      const sourceTileIds = debate.source_tile_ids || []

      if (removedTileIds.size > 0 && sourceTileIds.some(sourceTileId => removedTileIds.has(sourceTileId))) {
        return false
      }

      if (deletedResponse) {
        const usesDeletedResponse = sourceTileIds.includes(deletedResponse.tileId) &&
          (debate.participating_models || []).includes(deletedResponse.modelId)

        if (usesDeletedResponse) {
          return false
        }
      }

      return true
    })
  }

  // Debounce timers for position persistence — optimistic updates are instant,
  // but backend writes (JSON serialize + disk I/O) are debounced to avoid
  // hammering the backend at 60fps during drag operations.
  const _positionTimers = {}

  function _debouncedPersist(key, fn, delay = 150) {
    clearTimeout(_positionTimers[key])
    _positionTimers[key] = setTimeout(fn, delay)
  }

  async function updateTilePosition(tileId, position) {
    if (!currentSession.value) return

    // Optimistic update (instant — keeps UI smooth during drag)
    const tile = currentSession.value.prompt_tiles.find(t => t.id === tileId)
    if (tile) {
      tile.position = { ...tile.position, ...position }
    }

    // Also check debates
    const debate = currentSession.value.debates.find(d => d.id === tileId)
    if (debate) {
      debate.position = { ...debate.position, ...position }
    }

    // Persist to backend (debounced — only writes after drag pauses)
    const sessionId = currentSession.value.id
    _debouncedPersist(`tile:${tileId}`, async () => {
      try {
        await canvasApi.updateTilePosition(sessionId, tileId, position)
      } catch (err) {
        console.error('Failed to update tile position:', err)
      }
    })
  }

  async function updateLLMNodePosition(tileId, modelId, position) {
    if (!currentSession.value) return

    // Optimistic update (instant)
    const tile = currentSession.value.prompt_tiles.find(t => t.id === tileId)
    if (tile && tile.responses[modelId]) {
      tile.responses[modelId].position = { ...tile.responses[modelId].position, ...position }
    }

    // Persist to backend (debounced)
    const sessionId = currentSession.value.id
    _debouncedPersist(`llm:${tileId}:${modelId}`, async () => {
      try {
        await canvasApi.updateLLMNodePosition(sessionId, tileId, modelId, position)
      } catch (err) {
        console.error('Failed to update LLM node position:', err)
      }
    })
  }

  async function autoArrange(positions) {
    if (!currentSession.value) return

    // Optimistic update: apply all positions locally
    for (const [nodeId, position] of Object.entries(positions)) {
      const parts = nodeId.split(':')

      if (parts[0] === 'prompt' && parts.length >= 2) {
        const tileId = parts[1]
        const tile = currentSession.value.prompt_tiles.find(t => t.id === tileId)
        if (tile) {
          tile.position = { ...tile.position, ...position }
        }
      } else if (parts[0] === 'llm' && parts.length >= 3) {
        const tileId = parts[1]
        const modelId = parts.slice(2).join(':')
        const tile = currentSession.value.prompt_tiles.find(t => t.id === tileId)
        if (tile && tile.responses[modelId]) {
          tile.responses[modelId].position = { ...tile.responses[modelId].position, ...position }
        }
      } else if (parts[0] === 'debate' && parts.length >= 2) {
        const debateId = parts[1]
        const debate = currentSession.value.debates.find(d => d.id === debateId)
        if (debate) {
          debate.position = { ...debate.position, ...position }
        }
      }
    }

    // Persist to backend
    try {
      await canvasApi.autoArrange(currentSession.value.id, positions)
    } catch (err) {
      console.error('Failed to auto-arrange nodes:', err)
    }
  }

  async function deleteTile(tileId) {
    if (!currentSession.value) return

    const previousSession = structuredClone(toRaw(currentSession.value))
    const tileIdsToRemove = collectDescendantTileIds(tileId)

    if (currentSession.value.prompt_tiles.some(tile => tile.id === tileId)) {
      tileIdsToRemove.add(tileId)
    }

    currentSession.value.prompt_tiles = currentSession.value.prompt_tiles.filter(
      tile => !tileIdsToRemove.has(tile.id)
    )
    currentSession.value.debates = currentSession.value.debates.filter(
      debate => debate.id !== tileId
    )
    currentSession.value.debates = pruneDependentDebates(tileIdsToRemove).filter(
      debate => debate.id !== tileId
    )

    // Persist to backend
    try {
      await canvasApi.deleteTile(currentSession.value.id, tileId)
    } catch (err) {
      console.error('Failed to delete tile:', err)
      currentSession.value = previousSession
      throw err
    }
  }

  async function deleteResponse(tileId, modelId) {
    if (!currentSession.value) return

    // Find the tile
    const tile = currentSession.value.prompt_tiles.find(t => t.id === tileId)
    if (!tile) return

    const previousSession = structuredClone(toRaw(currentSession.value))
    const tileIdsToRemove = collectDescendantTileIds(tileId, modelId)

    delete tile.responses[modelId]
    tile.models = tile.models.filter(model => model !== modelId)
    currentSession.value.prompt_tiles = currentSession.value.prompt_tiles.filter(
      promptTile => !tileIdsToRemove.has(promptTile.id)
    )
    currentSession.value.debates = pruneDependentDebates(tileIdsToRemove, { tileId, modelId })

    try {
      await canvasApi.deleteResponse(currentSession.value.id, tileId, modelId)
    } catch (err) {
      console.error('Failed to delete response:', err)
      currentSession.value = previousSession
      throw err
    }
  }

  async function updateViewport(viewport) {
    if (!currentSession.value) return

    currentSession.value.viewport = viewport

    // Debounce viewport persistence (fires on every zoom/pan frame)
    const sessionId = currentSession.value.id
    _debouncedPersist('viewport', async () => {
      try {
        await canvasApi.updateViewport(sessionId, viewport)
      } catch (err) {
        console.error('Failed to update viewport:', err)
      }
    })
  }

  async function startDebate(tileIds, participatingModels, mode = 'auto', maxRounds = 3) {
    if (!currentSession.value) {
      throw new Error('No active session')
    }

    const sessionId = currentSession.value.id
    error.value = null

    // Mark models as streaming
    participatingModels.forEach(m => addStreaming(m))

    try {
      const reasoningEffort = maxReasoningEffort(
        tileIds.map(tileId => currentSession.value.prompt_tiles.find(tile => tile.id === tileId)?.reasoning_effort)
      )
      const request = {
        source_tile_ids: tileIds,
        participating_models: participatingModels,
        debate_mode: mode,
        max_rounds: maxRounds,
        reasoning_effort: reasoningEffort
      }

      let debateCompleteResolve
      const debateCompletePromise = new Promise(resolve => { debateCompleteResolve = resolve })

      const unlisten = await setupTauriStreamListener(sessionId, {
        debate_created: (data) => {
          if (currentSession.value && data.debate) {
            currentSession.value.debates.push(data.debate)
          }
        },
        round_start: (data) => {
          const debateId = data.debate_id
          const existing = debateStreamingContent.value[debateId]
          if (existing && Object.keys(existing.models).length > 0) {
            // Snapshot current round into completedRounds
            existing.completedRounds.push({
              round_number: existing.currentRound,
              models: { ...existing.models }
            })
          }
          debateStreamingContent.value[debateId] = {
            ...(existing || {}),
            currentRound: data.round_number,
            models: {},
            completedRounds: existing?.completedRounds || []
          }
        },
        debate_chunk: (data) => {
          const state = debateStreamingContent.value[data.debate_id]
          if (state && state.currentRound === data.round_number) {
            state.models[data.model_id] = (state.models[data.model_id] || '') + data.chunk
          }
        },
        model_complete: (data) => {
          removeStreaming(data.model_id)
        },
        debate_error: (data) => {
          console.error(`Debate error for ${data.model_id}:`, data.error)
          removeStreaming(data.model_id)
        },
        debate_complete: async (data) => {
          // Snapshot final round if it has content
          const state = debateStreamingContent.value[data.debate_id]
          if (state && Object.keys(state.models).length > 0) {
            state.completedRounds.push({
              round_number: state.currentRound,
              models: { ...state.models }
            })
          }
          delete debateStreamingContent.value[data.debate_id]
          await loadSession(sessionId)
          debateCompleteResolve()
        }
      })

      try {
        const debateId = await canvasApi.startDebate(sessionId, request)
        // Wait for debate_complete (the real end signal), not model_complete
        await Promise.race([
          debateCompletePromise,
          new Promise(resolve => setTimeout(resolve, 180000))
        ])
        return debateId
      } finally {
        unlisten()
      }
    } catch (err) {
      error.value = err.message || 'Failed to start debate'
      console.error('Failed to start debate:', err)
      throw err
    } finally {
      participatingModels.forEach(m => removeStreaming(m))
    }
  }

  async function continueDebate(debateId, prompt) {
    if (!currentSession.value) {
      throw new Error('No active session')
    }

    if (!prompt || !prompt.trim()) {
      error.value = 'Enter a prompt to continue the debate.'
      throw new Error(error.value)
    }

    const sessionId = currentSession.value.id
    const debate = currentSession.value.debates.find(d => d.id === debateId)
    if (!debate) {
      throw new Error('Debate not found')
    }

    const participatingModels = debate.participating_models
    participatingModels.forEach(m => addStreaming(m))

    try {
      const request = {
        prompt,
        reasoning_effort: normalizeReasoningEffort(debate.reasoning_effort)
      }

      let debateCompleteResolve
      const debateCompletePromise = new Promise(resolve => { debateCompleteResolve = resolve })

      const unlisten = await setupTauriStreamListener(sessionId, {
        round_start: (data) => {
          debateStreamingContent.value[debateId] = {
            currentRound: data.round_number,
            models: {},
            completedRounds: []
          }
        },
        debate_chunk: (data) => {
          const state = debateStreamingContent.value[data.debate_id]
          if (state && state.currentRound === data.round_number) {
            state.models[data.model_id] = (state.models[data.model_id] || '') + data.chunk
          }
        },
        model_complete: (data) => {
          removeStreaming(data.model_id)
        },
        debate_error: (data) => {
          removeStreaming(data.model_id)
        },
        debate_complete: async (data) => {
          const state = debateStreamingContent.value[data.debate_id]
          if (state && Object.keys(state.models).length > 0) {
            state.completedRounds.push({
              round_number: state.currentRound,
              models: { ...state.models }
            })
          }
          delete debateStreamingContent.value[data.debate_id]
          await loadSession(sessionId)
          debateCompleteResolve()
        }
      })

      try {
        await canvasApi.continueDebate(sessionId, debateId, request)
        await Promise.race([
          debateCompletePromise,
          new Promise(resolve => setTimeout(resolve, 180000))
        ])
      } finally {
        unlisten()
      }
    } catch (err) {
      error.value = err.message || 'Failed to continue debate'
      throw err
    } finally {
      participatingModels.forEach(m => removeStreaming(m))
    }
  }

  async function saveAsNote() {
    if (!currentSession.value) {
      throw new Error('No active session')
    }

    const sessionId = currentSession.value.id
    error.value = null

    try {
      const result = await canvasApi.exportToNote(sessionId)

      // Update current session with linked note
      if (currentSession.value) {
        currentSession.value.linked_note_id = result.note_id
      }

      return result
    } catch (err) {
      error.value = err.message || 'Failed to export to note'
      console.error('Failed to export canvas to note:', err)
      throw err
    }
  }

  // Add new models to an existing tile (same prompt, different models)
  async function addModelToTile(tileId, newModelIds) {
    if (!currentSession.value) {
      throw new Error('No active session')
    }

    const tile = currentSession.value.prompt_tiles.find(t => t.id === tileId)
    if (!tile) {
      throw new Error('Tile not found')
    }

    const sessionId = currentSession.value.id
    error.value = null

    // tileId is known synchronously (an existing tile), so streaming keys can be
    // marked immediately — no need to learn ownership asynchronously like sendPrompt.
    const tracker = createStreamingTracker()
    newModelIds.forEach(m => tracker.mark(`${tileId}:${m}`))

    try {
      const modelContent = {}
      newModelIds.forEach(m => { modelContent[m] = '' })

      const request = { model_ids: newModelIds }

      const unlisten = await setupTauriStreamListener(sessionId, {
        models_added: (data) => {
          if (data.tile_id !== tileId) return
          const targetTile = currentSession.value.prompt_tiles.find(t => t.id === data.tile_id)
          if (targetTile) {
            for (const [modelId, response] of Object.entries(data.responses)) {
              targetTile.responses[modelId] = response
            }
          }
        },
        chunk: (data) => {
          if (data.tile_id !== tileId || !newModelIds.includes(data.model_id)) return
          modelContent[data.model_id] = (modelContent[data.model_id] || '') + data.chunk
          updateTileResponseLocal(tileId, data.model_id, modelContent[data.model_id], 'streaming')
        },
        complete: (data) => {
          if (data.tile_id !== tileId || !newModelIds.includes(data.model_id)) return
          updateTileResponseLocal(tileId, data.model_id, modelContent[data.model_id], 'completed')
          tracker.clear(`${tileId}:${data.model_id}`)
        },
        error: (data) => {
          if (data.tile_id !== tileId || !newModelIds.includes(data.model_id)) return
          updateTileResponseLocal(tileId, data.model_id, '', 'error', data.error)
          tracker.clear(`${tileId}:${data.model_id}`)
        },
        session_saved: async () => {
          await reconcileSessionSaved(sessionId)
        }
      })

      try {
        await canvasApi.addModelsToTile(sessionId, tileId, request)
        await waitForModelsComplete(newModelIds.map(m => `${tileId}:${m}`), 120000)
      } finally {
        unlisten()
      }
    } catch (err) {
      error.value = err.message || 'Failed to add models'
      console.error('Failed to add models to tile:', err)
      throw err
    } finally {
      tracker.finalize()
    }
  }

  // Regenerate response for a specific model
  async function regenerateResponse(tileId, modelId) {
    if (!currentSession.value) {
      throw new Error('No active session')
    }

    const tile = currentSession.value.prompt_tiles.find(t => t.id === tileId)
    if (!tile || !tile.responses[modelId]) {
      throw new Error('Response not found')
    }

    const sessionId = currentSession.value.id
    error.value = null

    // tileId is known synchronously (an existing tile), so the streaming key can be
    // marked immediately.
    const streamingKey = `${tileId}:${modelId}`
    const tracker = createStreamingTracker()
    tracker.mark(streamingKey)

    // Clear existing content
    updateTileResponseLocal(tileId, modelId, '', 'streaming')

    try {
      let content = ''

      const unlisten = await setupTauriStreamListener(sessionId, {
        chunk: (data) => {
          if (data.tile_id !== tileId || data.model_id !== modelId) return
          content += data.chunk
          updateTileResponseLocal(tileId, modelId, content, 'streaming')
        },
        complete: (data) => {
          if (data.tile_id !== tileId || data.model_id !== modelId) return
          updateTileResponseLocal(tileId, modelId, content, 'completed')
          tracker.clear(streamingKey)
        },
        error: (data) => {
          if (data.tile_id !== tileId || data.model_id !== modelId) return
          updateTileResponseLocal(tileId, modelId, '', 'error', data.error)
          tracker.clear(streamingKey)
        },
        session_saved: async () => {
          await reconcileSessionSaved(sessionId)
        }
      })

      try {
        await canvasApi.regenerateResponse(sessionId, tileId, modelId)
        await waitForModelsComplete([streamingKey], 120000)
      } finally {
        unlisten()
      }
    } catch (err) {
      error.value = err.message || 'Failed to regenerate response'
      console.error('Failed to regenerate response:', err)
      throw err
    } finally {
      tracker.finalize()
    }
  }

  function clearError() {
    error.value = null
  }

  function clearSession() {
    currentSession.value = null
  }

  function reset() {
    sessions.value = []
    currentSession.value = null
    availableModels.value = []
    loading.value = false
    error.value = null
    clearStreaming()
    debateStreamingContent.value = {}
  }

  // Update pinned note IDs for the current session
  async function updatePinnedNotes(noteIds) {
    if (!currentSession.value) return
    const sessionId = currentSession.value.id
    currentSession.value.pinned_note_ids = noteIds
    try {
      await canvasApi.update(sessionId, { pinned_note_ids: noteIds })
    } catch (err) {
      console.error('Failed to update pinned notes:', err)
    }
  }

  // Branching helper - get parent response content for context
  function getParentResponseContent(parentTileId, parentModelId) {
    if (!currentSession.value || !parentTileId) return null
    const tile = currentSession.value.prompt_tiles.find(t => t.id === parentTileId)
    if (!tile) return null

    const response = tile.responses[parentModelId]
    return {
      prompt: tile.prompt,
      model: parentModelId,
      content: response?.content || ''
    }
  }

  // Branch from a specific model response
  async function branchFromResponse(
    parentTileId,
    parentModelId,
    newPrompt,
    models,
    systemPrompt = null,
    temperature = 0.7,
    maxTokens = null,
    contextMode = 'none',
    twinAnswerMode = 'simulation',
    webSearch = false,
    webSearchMaxResults = DEFAULT_WEB_SEARCH_MAX_RESULTS,
    promptType = 'standard',
    decisionMetadata = null,
    reasoningEffort = 'none',
    twinLlmProvider = null
  ) {
    return sendPrompt(
      newPrompt,
      models,
      systemPrompt,
      temperature,
      maxTokens,
      parentTileId,
      parentModelId,
      contextMode,
      twinAnswerMode,
      webSearch,
      webSearchMaxResults,
      promptType,
      decisionMetadata,
      reasoningEffort,
      twinLlmProvider
    )
  }

  async function thinkHarderFromResponse(parentTileId, parentModelId, options = {}) {
    const webSearch = options.webSearch ?? true
    const webSearchMaxResults = webSearch
      ? THINK_HARDER_WEB_SEARCH_MAX_RESULTS
      : DEFAULT_WEB_SEARCH_MAX_RESULTS

    const parentTile = currentSession.value?.prompt_tiles.find(tile => tile.id === parentTileId)
    const reasoningEffort = maxReasoningEffort([parentTile?.reasoning_effort, 'high'])

    return branchFromResponse(
      parentTileId,
      parentModelId,
      THINK_HARDER_PROMPT,
      [parentModelId],
      THINK_HARDER_SYSTEM_PROMPT,
      0.3,
      null,
      'full_history',
      'advisor',
      webSearch,
      webSearchMaxResults,
      'standard',
      null,
      reasoningEffort
    )
  }

  async function recordCanvasFeedback(request) {
    if (!currentSession.value) {
      throw new Error('No active session')
    }

    return twinApi.recordCanvasFeedback(currentSession.value.id, request)
  }

  async function recordPreferenceFeedback(tileId, modelId, feedbackType, rationale = null, content = null) {
    return recordCanvasFeedback({
      feedback_type: feedbackType,
      response: {
        tile_id: tileId,
        model_id: modelId
      },
      rationale,
      content
    })
  }

  async function recordSelectionRanking(responseRefs, rationale = null, content = null) {
    return recordCanvasFeedback({
      feedback_type: 'ranking',
      ranked_responses: responseRefs,
      rationale,
      content
    })
  }

  async function recordCorrectionFeedback(tileId, modelId, content, rationale = null) {
    return recordCanvasFeedback({
      feedback_type: 'correction',
      response: {
        tile_id: tileId,
        model_id: modelId
      },
      kind: 'fact',
      content,
      rationale,
      confidence: 0.85
    })
  }

  async function captureInsight(kind, content, options = {}) {
    return recordCanvasFeedback({
      feedback_type: 'insight',
      kind,
      content,
      rationale: options.rationale ?? null,
      response: options.response ?? null,
      confidence: options.confidence ?? 0.8
    })
  }

  async function exportTwinData(request = {}) {
    return twinApi.exportData(request)
  }

  async function listMemoryDigest() {
    return twinApi.listMemoryDigest()
  }

  async function reviewMemoryDigestItem(id, action, rationale = null) {
    return twinApi.reviewMemoryDigestItem(id, { action, rationale })
  }

  async function listDecisionEpisodes() {
    return twinApi.listDecisionEpisodes()
  }

  async function updateDecisionOutcome(id, update) {
    return twinApi.updateDecisionOutcome(id, update)
  }

  async function getDecisionMirrorConfig() {
    return twinApi.getDecisionMirrorConfig()
  }

  async function updateDecisionMirrorConfig(update) {
    return twinApi.updateDecisionMirrorConfig(update)
  }

  async function resetDecisionMirrorConfig() {
    return twinApi.resetDecisionMirrorConfig()
  }

  function isTwinWorkspaceTutorialDismissed() {
    return localStorage.getItem('grafyn.twinWorkspaceTutorial.dismissed') === 'true'
  }

  function setTwinWorkspaceTutorialDismissed(dismissed) {
    localStorage.setItem(
      'grafyn.twinWorkspaceTutorial.dismissed',
      dismissed ? 'true' : 'false'
    )
  }

  return {
    // State
    sessions,
    currentSession,
    availableModels,
    loading,
    error,
    streamingModels,
    debateStreamingContent,
    // Getters
    promptTiles,
    debates,
    hasSession,
    isStreaming,
    modelsByProvider,
    tileEdges,
    debateEdges,
    // Actions
    loadSessions,
    loadSession,
    createSession,
    updateSession,
    deleteSession,
    loadModels,
    sendPrompt,
    updateTilePosition,
    updateLLMNodePosition,
    autoArrange,
    deleteTile,
    deleteResponse,
    updateViewport,
    startDebate,
    continueDebate,
    saveAsNote,
    clearError,
    clearSession,
    reset,
    getParentResponseContent,
    branchFromResponse,
    thinkHarderFromResponse,
    addModelToTile,
    regenerateResponse,
    updatePinnedNotes,
    recordCanvasFeedback,
    recordPreferenceFeedback,
    recordSelectionRanking,
    recordCorrectionFeedback,
    captureInsight,
    exportTwinData,
    listMemoryDigest,
    reviewMemoryDigestItem,
    listDecisionEpisodes,
    updateDecisionOutcome,
    getDecisionMirrorConfig,
    updateDecisionMirrorConfig,
    resetDecisionMirrorConfig,
    isTwinWorkspaceTutorialDismissed,
    setTwinWorkspaceTutorialDismissed
  }
})
