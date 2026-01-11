import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { canvas as canvasApi } from '@/api/client'

export const useCanvasStore = defineStore('canvas', () => {
  // State
  const sessions = ref([])
  const currentSession = ref(null)
  const availableModels = ref([])
  const loading = ref(false)
  const error = ref(null)
  const streamingModels = ref(new Set())

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
      currentSession.value = await canvasApi.get(sessionId)
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
      const session = await canvasApi.create(data)
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
      const updated = await canvasApi.update(sessionId, data)

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
      // Don't set error for models load - it's not critical
    }
  }

  async function sendPrompt(prompt, models, systemPrompt = null, temperature = 0.7, maxTokens = 2048, parentTileId = null, parentModelId = null, contextMode = 'full_history') {
    if (!currentSession.value) {
      throw new Error('No active session')
    }

    const sessionId = currentSession.value.id
    error.value = null

    // Mark models as streaming
    models.forEach(m => streamingModels.value.add(m))

    try {
      const response = await fetch(`/api/canvas/${sessionId}/prompt`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          prompt,
          models,
          system_prompt: systemPrompt,
          temperature,
          max_tokens: maxTokens,
          parent_tile_id: parentTileId,
          parent_model_id: parentModelId,
          context_mode: contextMode
        })
      })

      if (!response.ok) {
        throw new Error(`HTTP error: ${response.status}`)
      }

      const reader = response.body.getReader()
      const decoder = new TextDecoder()

      let tileId = null
      const modelContent = {}
      models.forEach(m => modelContent[m] = '')

      while (true) {
        const { done, value } = await reader.read()
        if (done) break

        const text = decoder.decode(value)
        const lines = text.split('\n')

        for (const line of lines) {
          if (!line.startsWith('data: ')) continue
          const data = line.slice(6)
          if (data === '[DONE]') break

          try {
            const event = JSON.parse(data)

            switch (event.type) {
              case 'tile_created':
                tileId = event.tile_id
                break

              case 'chunk':
                modelContent[event.model_id] += event.chunk
                updateTileResponseLocal(tileId, event.model_id, modelContent[event.model_id], 'streaming')
                break

              case 'complete':
                updateTileResponseLocal(tileId, event.model_id, modelContent[event.model_id], 'completed')
                streamingModels.value.delete(event.model_id)
                break

              case 'error':
                updateTileResponseLocal(tileId, event.model_id, event.error, 'error')
                streamingModels.value.delete(event.model_id)
                break

              case 'session_saved':
                // Refresh session data
                await loadSession(sessionId)
                break
            }
          } catch (e) {
            console.error('Failed to parse SSE event:', e)
          }
        }
      }

      return tileId
    } catch (err) {
      error.value = err.message || 'Failed to send prompt'
      console.error('Failed to send prompt:', err)
      throw err
    } finally {
      // Clear streaming state
      models.forEach(m => streamingModels.value.delete(m))
    }
  }

  function updateTileResponseLocal(tileId, modelId, content, status) {
    if (!currentSession.value) return

    const tile = currentSession.value.prompt_tiles.find(t => t.id === tileId)
    if (tile && tile.responses[modelId]) {
      tile.responses[modelId].content = content
      tile.responses[modelId].status = status
    }
  }

  async function updateTilePosition(tileId, position) {
    if (!currentSession.value) return

    // Optimistic update
    const tile = currentSession.value.prompt_tiles.find(t => t.id === tileId)
    if (tile) {
      tile.position = { ...tile.position, ...position }
    }

    // Also check debates
    const debate = currentSession.value.debates.find(d => d.id === tileId)
    if (debate) {
      debate.position = { ...debate.position, ...position }
    }

    // Persist to backend (debounced in component)
    try {
      await canvasApi.updateTilePosition(currentSession.value.id, tileId, position)
    } catch (err) {
      console.error('Failed to update tile position:', err)
    }
  }

  async function updateLLMNodePosition(tileId, modelId, position) {
    if (!currentSession.value) return

    // Optimistic update
    const tile = currentSession.value.prompt_tiles.find(t => t.id === tileId)
    if (tile && tile.responses[modelId]) {
      tile.responses[modelId].position = { ...tile.responses[modelId].position, ...position }
    }

    // Persist to backend
    try {
      await canvasApi.updateLLMNodePosition(currentSession.value.id, tileId, modelId, position)
    } catch (err) {
      console.error('Failed to update LLM node position:', err)
    }
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

    // Optimistic update - remove from prompt tiles
    const tileIndex = currentSession.value.prompt_tiles.findIndex(t => t.id === tileId)
    let removedTile = null
    if (tileIndex !== -1) {
      removedTile = currentSession.value.prompt_tiles[tileIndex]
      currentSession.value.prompt_tiles.splice(tileIndex, 1)
      // Also remove any child tiles
      currentSession.value.prompt_tiles = currentSession.value.prompt_tiles.filter(
        t => t.parent_tile_id !== tileId
      )
    }

    // Also try to remove from debates
    const debateIndex = currentSession.value.debates.findIndex(d => d.id === tileId)
    let removedDebate = null
    if (debateIndex !== -1) {
      removedDebate = currentSession.value.debates[debateIndex]
      currentSession.value.debates.splice(debateIndex, 1)
    }

    // Persist to backend
    try {
      await canvasApi.deleteTile(currentSession.value.id, tileId)
    } catch (err) {
      console.error('Failed to delete tile:', err)
      // Revert optimistic update on error
      if (removedTile) {
        currentSession.value.prompt_tiles.push(removedTile)
      }
      if (removedDebate) {
        currentSession.value.debates.push(removedDebate)
      }
      throw err
    }
  }

  async function updateViewport(viewport) {
    if (!currentSession.value) return

    currentSession.value.viewport = viewport

    try {
      await canvasApi.updateViewport(currentSession.value.id, viewport)
    } catch (err) {
      console.error('Failed to update viewport:', err)
    }
  }

  async function startDebate(tileIds, participatingModels, mode = 'auto', maxRounds = 3) {
    if (!currentSession.value) {
      throw new Error('No active session')
    }

    const sessionId = currentSession.value.id
    error.value = null

    // Mark models as streaming
    participatingModels.forEach(m => streamingModels.value.add(m))

    try {
      const response = await fetch(`/api/canvas/${sessionId}/debate`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          source_tile_ids: tileIds,
          participating_models: participatingModels,
          debate_mode: mode,
          max_rounds: maxRounds
        })
      })

      if (!response.ok) {
        throw new Error(`HTTP error: ${response.status}`)
      }

      const reader = response.body.getReader()
      const decoder = new TextDecoder()

      let debateId = null
      const roundContent = {}

      while (true) {
        const { done, value } = await reader.read()
        if (done) break

        const text = decoder.decode(value)
        const lines = text.split('\n')

        for (const line of lines) {
          if (!line.startsWith('data: ')) continue
          const data = line.slice(6)
          if (data === '[DONE]') break

          try {
            const event = JSON.parse(data)

            switch (event.type) {
              case 'debate_created':
                debateId = event.debate_id
                break

              case 'round_start':
                participatingModels.forEach(m => roundContent[m] = '')
                break

              case 'debate_chunk':
                if (!roundContent[event.model_id]) roundContent[event.model_id] = ''
                roundContent[event.model_id] += event.chunk
                // Could update UI here for streaming debate content
                break

              case 'model_complete':
                streamingModels.value.delete(event.model_id)
                break

              case 'debate_error':
                console.error(`Debate error for ${event.model_id}:`, event.error)
                streamingModels.value.delete(event.model_id)
                break

              case 'debate_complete':
                await loadSession(sessionId)
                break
            }
          } catch (e) {
            console.error('Failed to parse debate SSE event:', e)
          }
        }
      }

      return debateId
    } catch (err) {
      error.value = err.message || 'Failed to start debate'
      console.error('Failed to start debate:', err)
      throw err
    } finally {
      participatingModels.forEach(m => streamingModels.value.delete(m))
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
    participatingModels.forEach(m => streamingModels.value.add(m))

    try {
      const response = await fetch(`/api/canvas/${sessionId}/debate/${debateId}/continue`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ prompt })
      })

      if (!response.ok) {
        throw new Error(`HTTP error: ${response.status}`)
      }

      const reader = response.body.getReader()
      const decoder = new TextDecoder()

      while (true) {
        const { done, value } = await reader.read()
        if (done) break

        const text = decoder.decode(value)
        const lines = text.split('\n')

        for (const line of lines) {
          if (!line.startsWith('data: ')) continue
          const data = line.slice(6)
          if (data === '[DONE]') break

          try {
            const event = JSON.parse(data)

            if (event.type === 'round_complete') {
              await loadSession(sessionId)
            }
          } catch (e) {
            console.error('Failed to parse continue debate SSE event:', e)
          }
        }
      }
    } catch (err) {
      error.value = err.message || 'Failed to continue debate'
      throw err
    } finally {
      participatingModels.forEach(m => streamingModels.value.delete(m))
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
    streamingModels.value.clear()
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
  async function branchFromResponse(parentTileId, parentModelId, newPrompt, models, systemPrompt = null, temperature = 0.7, maxTokens = 2048, contextMode = 'full_history') {
    return sendPrompt(newPrompt, models, systemPrompt, temperature, maxTokens, parentTileId, parentModelId, contextMode)
  }

  return {
    // State
    sessions,
    currentSession,
    availableModels,
    loading,
    error,
    streamingModels,
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
    updateViewport,
    startDebate,
    continueDebate,
    saveAsNote,
    clearError,
    clearSession,
    reset,
    getParentResponseContent,
    branchFromResponse
  }
})
