import { invoke } from '@tauri-apps/api/tauri'

// App boot API
export const boot = {
  status: () => invoke('get_boot_status', {}),
}

// Notes API
export const notes = {
  list: () => invoke('list_notes', {}),

  get: (id) => invoke('get_note', { id }),

  create: (data) => invoke('create_note', { note: data }),

  update: (id, data) => invoke('update_note', { id, update: data }),

  delete: (id) => invoke('delete_note', { id }),

  reindex: () => invoke('reindex', {}),

  distill: (id, request) => invoke('distill_note', { id, request }),

  normalizeTags: (id) => invoke('normalize_tags', { id }),
}

// Search API
export const search = {
  query: (q, { limit = 10 } = {}) => invoke('search_notes', { query: q, limit }),

  similar: (noteId, limit = 5) => invoke('find_similar', { noteId, limit }),
}

// Graph API
export const graph = {
  backlinks: (id) => invoke('get_backlinks', { noteId: id }),

  outgoing: (id) => invoke('get_outgoing', { noteId: id }),

  neighbors: (id) => invoke('get_neighbors', { noteId: id }),

  rebuild: () => invoke('rebuild_graph', {}),

  full: () => invoke('get_full_graph', {}),

  unlinked: () => invoke('get_unlinked', {}),
}

// Canvas API
export const canvas = {
  list: () => invoke('list_sessions', {}),

  get: (id) => invoke('get_session', { id }),

  create: (data) => invoke('create_session', { session: data }),

  update: (id, data) => invoke('update_session', { id, update: data }),

  delete: (id) => invoke('delete_session', { id }),

  getModels: () => invoke('get_available_models', {}),

  sendPrompt: (sessionId, request) => invoke('send_prompt', { sessionId, request }),

  updateTilePosition: (sessionId, tileId, position) =>
    invoke('update_tile_position', { sessionId, tileId, position }),

  updateLLMNodePosition: (sessionId, tileId, modelId, position) =>
    invoke('update_llm_node_position', { sessionId, tileId, modelId, position }),

  autoArrange: (sessionId, positions) => invoke('auto_arrange', { sessionId, positions }),

  deleteTile: (sessionId, tileId) => invoke('delete_tile', { sessionId, tileId }),

  deleteResponse: (sessionId, tileId, modelId) =>
    invoke('delete_response', { sessionId, tileId, modelId }),

  updateViewport: (sessionId, viewport) => invoke('update_viewport', { sessionId, viewport }),

  exportToNote: (sessionId) => invoke('export_to_note', { sessionId }),

  startDebate: (sessionId, request) => invoke('start_debate', { sessionId, request }),

  continueDebate: (sessionId, debateId, request) =>
    invoke('continue_debate', { sessionId, debateId, request }),

  addModelsToTile: (sessionId, tileId, request) =>
    invoke('add_models_to_tile', { sessionId, tileId, request }),

  regenerateResponse: (sessionId, tileId, modelId) =>
    invoke('regenerate_response', { sessionId, tileId, modelId }),
}

// Feedback API
export const feedback = {
  submit: (data) => invoke('submit_feedback', { feedback: data }),

  status: () => invoke('feedback_status', {}),

  getSystemInfo: (currentPage = null) => invoke('get_system_info', { currentPage }),

  getPending: () => invoke('get_pending_feedback', {}),

  retryPending: () => invoke('retry_pending_feedback', {}),
}

// Settings API
export const settings = {
  get: () => invoke('get_settings', {}),

  getStatus: () => invoke('get_settings_status', {}),

  update: (data) => invoke('update_settings', { update: data }),

  completeSetup: () => invoke('complete_setup', {}),

  pickVaultFolder: () => invoke('pick_vault_folder'),

  validateOpenRouterKey: (apiKey) => invoke('validate_openrouter_key', { apiKey }),

  getOpenRouterStatus: () => invoke('get_openrouter_status', {}),
}

// MCP API
export const mcp = {
  getStatus: () => invoke('get_mcp_status', {}),

  getConfigSnippet: () => invoke('get_mcp_config_snippet', {}),
}

// Priority Scoring API
export const priority = {
  get: () => invoke('get_priority_settings', {}),

  update: (update) => invoke('update_priority_settings', { update }),

  reset: () => invoke('reset_priority_settings', {}),
}

// Memory API
export const memory = {
  recall: (query, contextNoteIds = [], limit = 5) =>
    invoke('recall_relevant', { request: { query, context_note_ids: contextNoteIds, limit } }),

  contradictions: (noteId) => invoke('find_contradictions', { noteId }),

  extract: (messages) => invoke('extract_claims', { request: { messages } }),
}

// Zettelkasten Link Discovery API
export const zettelkasten = {
  discoverLinks: (noteId, mode = 'suggested', maxLinks = 10) =>
    invoke('discover_links', { noteId, mode, maxLinks }),

  applyLinks: (noteId, candidates) =>
    invoke('apply_links', {
      noteId,
      request: {
        link_ids: candidates.map(candidate => candidate.target_id),
        candidates,
      },
    }),

  createLink: (sourceId, targetId, linkType = 'related') =>
    invoke('create_link', { sourceId, targetId, linkType }),

  getLinkTypes: () => invoke('get_link_types', {}),
}

// Retrieval API (temporal + graph-aware)
export const retrieval = {
  retrieve: (query, limit = 10, contextNoteIds = []) =>
    invoke('retrieve_relevant', { query, limit, contextNoteIds }),

  getConfig: () => invoke('get_retrieval_config', {}),

  updateConfig: (update) => invoke('update_retrieval_config', { update }),
}

// Import API
export const importApi = {
  preview: (filePath) => invoke('preview_import', { filePath }),

  apply: (filePath, conversationIds = []) =>
    invoke('apply_import', { filePath, conversationIds }),

  getSupportedFormats: () => invoke('get_supported_formats', {}),
}

// Always true — this is a desktop-only app now
export const isDesktopApp = () => true
