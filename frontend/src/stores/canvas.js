import { defineStore } from 'pinia'
import { ref, shallowRef, computed, triggerRef, toRaw } from 'vue'
import { canvas as canvasApi, twin as twinApi } from '@/api/client'

export const DEFAULT_WEB_SEARCH_MAX_RESULTS = 5
export const THINK_HARDER_WEB_SEARCH_MAX_RESULTS = 8
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

  function normalizeResponse(response) {
    if (!response) return response

    return {
      ...response,
      error_message: response.error_message ?? response.error ?? null
    }
  }

  function normalizeSession(session) {
    if (!session?.prompt_tiles) return session

    return {
      ...session,
      prompt_tiles: session.prompt_tiles.map(tile => ({
        ...tile,
        responses: Object.fromEntries(
          Object.entries(tile.responses || {}).map(([modelId, response]) => [
            modelId,
            normalizeResponse(response)
          ])
        )
      }))
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
    loading.value = true
    error.value = null
    try {
      const session = normalizeSession(await canvasApi.create(data))
      sessions.value.unshift(session)
      currentSession.value = session
      return session
    } catch (err) {
      error.value = err.message || 'Failed to create session'
      console.error('Failed to create canvas session:', err)
      throw err
    } finally {
      loading.value = false
    }
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
    maxTokens = null,
    parentTileId = null,
    parentModelId = null,
    contextMode = 'knowledge_search',
    webSearch = false,
    webSearchMaxResults = DEFAULT_WEB_SEARCH_MAX_RESULTS
  ) {
    if (!currentSession.value) {
      throw new Error('No active session')
    }

    const sessionId = currentSession.value.id
    error.value = null

    // Mark models as streaming
    models.forEach(m => addStreaming(m))

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

    try {
      const modelContent = {}
      models.forEach(m => { modelContent[m] = '' })

      const request = {
        prompt,
        models,
        system_prompt: systemPrompt,
        temperature,
        parent_tile_id: parentTileId,
        parent_model_id: parentModelId,
        context_mode: contextMode,
        position,
        web_search: webSearch,
        web_search_max_results: webSearchMaxResults
      }
      if (maxTokens != null) {
        request.max_tokens = maxTokens
      }

      // Set up event listener BEFORE calling invoke
      const unlisten = await setupTauriStreamListener(sessionId, {
        tile_created: (data) => {
          if (currentSession.value && data.tile) {
            currentSession.value.prompt_tiles.push(data.tile)
          }
        },
        context_notes: (data) => {
          if (currentSession.value) {
            const tile = currentSession.value.prompt_tiles.find(t => t.id === data.tile_id)
            if (tile) tile.context_notes = data.notes || []
          }
        },
        chunk: (data) => {
          modelContent[data.model_id] = (modelContent[data.model_id] || '') + data.chunk
          updateTileResponseLocal(data.tile_id, data.model_id, modelContent[data.model_id], 'streaming')
        },
        complete: (data) => {
          updateTileResponseLocal(data.tile_id, data.model_id, modelContent[data.model_id], 'completed')
          removeStreaming(data.model_id)
        },
        error: (data) => {
          updateTileResponseLocal(data.tile_id, data.model_id, '', 'error', data.error)
          removeStreaming(data.model_id)
        },
        session_saved: async () => {
          await loadSession(sessionId)
        }
      })

      try {
        // invoke returns tile_id immediately; streaming happens via events
        const tileId = await canvasApi.sendPrompt(sessionId, request)
        await waitForModelsComplete(models, 120000)
        return tileId
      } finally {
        unlisten()
      }
    } catch (err) {
      error.value = err.message || 'Failed to send prompt'
      console.error('Failed to send prompt:', err)
      throw err
    } finally {
      models.forEach(m => removeStreaming(m))
    }
  }

  // Wait for all streaming models to complete (or timeout)
  function waitForModelsComplete(models, timeoutMs = 120000) {
    return new Promise((resolve) => {
      const start = Date.now()
      const check = () => {
        const stillStreaming = models.some(m => streamingModels.value.has(m))
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
      const request = {
        source_tile_ids: tileIds,
        participating_models: participatingModels,
        debate_mode: mode,
        max_rounds: maxRounds
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

    const sessionId = currentSession.value.id
    const debate = currentSession.value.debates.find(d => d.id === debateId)
    if (!debate) {
      throw new Error('Debate not found')
    }

    const participatingModels = debate.participating_models
    participatingModels.forEach(m => addStreaming(m))

    try {
      const request = { prompt }

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

    // Mark new models as streaming
    newModelIds.forEach(m => addStreaming(m))

    try {
      const modelContent = {}
      newModelIds.forEach(m => { modelContent[m] = '' })

      const request = { model_ids: newModelIds }

      const unlisten = await setupTauriStreamListener(sessionId, {
        models_added: (data) => {
          const tile = currentSession.value.prompt_tiles.find(t => t.id === data.tile_id)
          if (tile) {
            for (const [modelId, response] of Object.entries(data.responses)) {
              tile.responses[modelId] = response
            }
          }
        },
        chunk: (data) => {
          modelContent[data.model_id] = (modelContent[data.model_id] || '') + data.chunk
          updateTileResponseLocal(tileId, data.model_id, modelContent[data.model_id], 'streaming')
        },
        complete: (data) => {
          updateTileResponseLocal(tileId, data.model_id, modelContent[data.model_id], 'completed')
          removeStreaming(data.model_id)
        },
        error: (data) => {
          updateTileResponseLocal(tileId, data.model_id, '', 'error', data.error)
          removeStreaming(data.model_id)
        },
        session_saved: async () => {
          await loadSession(sessionId)
        }
      })

      try {
        await canvasApi.addModelsToTile(sessionId, tileId, request)
        await waitForModelsComplete(newModelIds, 120000)
      } finally {
        unlisten()
      }
    } catch (err) {
      error.value = err.message || 'Failed to add models'
      console.error('Failed to add models to tile:', err)
      throw err
    } finally {
      newModelIds.forEach(m => removeStreaming(m))
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

    // Mark model as streaming
    addStreaming(modelId)

    // Clear existing content
    updateTileResponseLocal(tileId, modelId, '', 'streaming')

    try {
      let content = ''

      const unlisten = await setupTauriStreamListener(sessionId, {
        chunk: (data) => {
          if (data.model_id === modelId) {
            content += data.chunk
            updateTileResponseLocal(tileId, modelId, content, 'streaming')
          }
        },
        complete: (data) => {
          if (data.model_id === modelId) {
            updateTileResponseLocal(tileId, modelId, content, 'completed')
            removeStreaming(modelId)
          }
        },
        error: (data) => {
          if (data.model_id === modelId) {
            updateTileResponseLocal(tileId, modelId, '', 'error', data.error)
            removeStreaming(modelId)
          }
        },
        session_saved: async () => {
          await loadSession(sessionId)
        }
      })

      try {
        await canvasApi.regenerateResponse(sessionId, tileId, modelId)
        await waitForModelsComplete([modelId], 120000)
      } finally {
        unlisten()
      }
    } catch (err) {
      error.value = err.message || 'Failed to regenerate response'
      console.error('Failed to regenerate response:', err)
      throw err
    } finally {
      removeStreaming(modelId)
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
    contextMode = 'knowledge_search',
    webSearch = false,
    webSearchMaxResults = DEFAULT_WEB_SEARCH_MAX_RESULTS
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
      webSearch,
      webSearchMaxResults
    )
  }

  async function thinkHarderFromResponse(parentTileId, parentModelId, options = {}) {
    const webSearch = options.webSearch ?? true
    const webSearchMaxResults = webSearch
      ? THINK_HARDER_WEB_SEARCH_MAX_RESULTS
      : DEFAULT_WEB_SEARCH_MAX_RESULTS

    return branchFromResponse(
      parentTileId,
      parentModelId,
      THINK_HARDER_PROMPT,
      [parentModelId],
      THINK_HARDER_SYSTEM_PROMPT,
      0.3,
      4096,
      'full_history',
      webSearch,
      webSearchMaxResults
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
    captureInsight,
    exportTwinData
  }
})
