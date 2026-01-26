import { defineStore } from 'pinia'

export const useImportStore = defineStore('import', {
  state: () => ({
    jobs: [],
    currentJob: null,
    previewData: null,
    decisions: {},
    errors: [],
    isUploading: false,
    isParsing: false,
    isApplying: false,
    isReverting: false,
    isAssessing: false,
    lastImportSummary: null,
    lastImportJobId: null,
    // Zettelkasten configuration
    zettelConfig: {
      enabled: true,
      linkMode: 'automatic', // automatic | suggested | manual
      noteTypes: ['concept', 'claim', 'evidence', 'question', 'fleche'],
      autoCreateHubs: true
    },
    // Zettelkasten types reference
    zettelTypes: [
      { value: 'concept', label: '💡 Concept', description: 'Definitions and explanations' },
      { value: 'claim', label: '📣 Claim', description: 'Assertions needing evidence' },
      { value: 'evidence', label: '📊 Evidence', description: 'Data and examples' },
      { value: 'question', label: '❓ Question', description: 'Inquiries and exploration' },
      { value: 'fleche', label: '🔗 Structure', description: 'Connections between ideas' }
    ],
    linkModes: [
      { value: 'automatic', label: 'Auto-Link', description: 'Create all links automatically' },
      { value: 'suggested', label: 'Suggest Links', description: 'Show for approval' },
      { value: 'manual', label: 'Manual Links', description: 'User-triggered only' }
    ]
  }),

  getters: {
    currentJobId: (state) => state.currentJob?.id || null,
    parsedConversations: (state) => state.previewData?.conversations || [],
    totalConversations: (state) => state.previewData?.total_conversations || 0,
    duplicatesFound: (state) => state.previewData?.duplicates_found || 0,
    estimatedNotes: (state) => state.previewData?.estimated_notes_to_create || 0,
    platform: (state) => state.previewData?.platform || null,
    hasErrors: (state) => state.errors.length > 0,
    canRevert: (state) => state.lastImportSummary?.notes_created > 0,
    qualityAssessed: (state) => {
      const convs = state.previewData?.conversations || []
      return convs.some(c => c.quality != null)
    },
    zettelEnabled: (state) => state.zettelConfig.enabled,
    linkMode: (state) => state.zettelConfig.linkMode
  },

  actions: {
    async uploadFile(file) {
      this.isUploading = true
      this.errors = []

      try {
        const formData = new FormData()
        formData.append('file', file)

        const response = await fetch('/api/import/upload', {
          method: 'POST',
          body: formData
        })

        if (!response.ok) {
          throw new Error(`Upload failed: ${response.statusText}`)
        }

        const job = await response.json()
        this.currentJob = job
        this.jobs.push(job)
        return job
      } catch (error) {
        this.errors.push({
          type: 'upload',
          message: error.message,
          severity: 'error'
        })
        throw error
      } finally {
        this.isUploading = false
      }
    },

    async parseJob(jobId) {
      this.isParsing = true
      this.errors = []

      try {
        const response = await fetch(`/api/import/${jobId}/parse`, {
          method: 'POST'
        })

        if (!response.ok) {
          throw new Error(`Parse failed: ${response.statusText}`)
        }

        const job = await response.json()
        this.currentJob = job

        const jobIndex = this.jobs.findIndex(j => j.id === jobId)
        if (jobIndex !== -1) {
          this.jobs[jobIndex] = job
        }

        return job
      } catch (error) {
        this.errors.push({
          type: 'parse',
          message: error.message,
          severity: 'error'
        })
        throw error
      } finally {
        this.isParsing = false
      }
    },

    async loadPreview(jobId) {
      try {
        const response = await fetch(`/api/import/${jobId}/preview`)

        if (!response.ok) {
          throw new Error(`Preview failed: ${response.statusText}`)
        }

        const preview = await response.json()
        this.previewData = preview

        // Initialize decisions for all conversations
        preview.conversations.forEach(conv => {
          if (!(conv.id in this.decisions)) {
            this.decisions[conv.id] = {
              conversation_id: conv.id,
              action: conv.duplicate_candidates.length > 0 ? 'skip' : 'accept',
              distill_option: 'auto_distill'
            }
          }
        })

        return preview
      } catch (error) {
        this.errors.push({
          type: 'preview',
          message: error.message,
          severity: 'error'
        })
        throw error
      }
    },

    async assessQuality(modelId = null) {
      this.isAssessing = true
      this.errors = []

      try {
        const body = modelId ? {
          summarization_settings: {
            model_id: modelId,
            detail_level: 'detailed',
            max_tokens: 4096
          }
        } : null

        const response = await fetch(`/api/import/${this.currentJobId}/assess`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: body ? JSON.stringify(body) : null
        })

        if (!response.ok) {
          throw new Error(`Assessment failed: ${response.statusText}`)
        }

        const job = await response.json()
        this.currentJob = job

        // Update preview data with quality info
        if (job.parsed_conversations) {
          this.previewData.conversations = job.parsed_conversations

          // Update decisions based on quality suggestions
          job.parsed_conversations.forEach(conv => {
            if (conv.quality) {
              const action = conv.quality.suggested_action === 'skip' ? 'skip' :
                conv.quality.suggested_action === 'import_and_distill' ? 'accept' : 'accept'
              const distill = conv.quality.suggested_action === 'import_and_distill' ? 'auto_distill' : 'container_only'

              this.decisions[conv.id] = {
                ...this.decisions[conv.id],
                action,
                distill_option: distill
              }
            }
          })
        }

        return job
      } catch (error) {
        this.errors.push({
          type: 'assess',
          message: error.message,
          severity: 'error'
        })
        throw error
      } finally {
        this.isAssessing = false
      }
    },

    async submitDecisions(decisions) {
      this.isApplying = true
      this.errors = []

      try {
        const response = await fetch(`/api/import/${this.currentJobId}/apply`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(decisions)
        })

        if (!response.ok) {
          throw new Error(`Import failed: ${response.statusText}`)
        }

        const summary = await response.json()

        // Update job status
        if (this.currentJob) {
          this.currentJob.status = 'completed'
        }

        // Store the created note IDs for potential revert
        this.lastImportSummary = summary
        this.lastImportJobId = this.currentJobId

        return summary
      } catch (error) {
        this.errors.push({
          type: 'apply',
          message: error.message,
          severity: 'error'
        })
        throw error
      } finally {
        this.isApplying = false
      }
    },

    async revertLastImport() {
      if (!this.lastImportSummary?.notes_created || this.lastImportSummary.notes_created === 0) {
        throw new Error('No import to revert')
      }

      this.isReverting = true
      this.errors = []

      try {
        // Call the revert endpoint
        const response = await fetch(`/api/import/${this.lastImportJobId}/revert`, {
          method: 'POST'
        })

        if (!response.ok) {
          const errorData = await response.json().catch(() => ({}))
          throw new Error(errorData.detail || `Revert failed: ${response.statusText}`)
        }

        const result = await response.json()

        // Clear last import tracking
        this.lastImportSummary = null
        this.lastImportJobId = null

        return result
      } catch (error) {
        this.errors.push({
          type: 'revert',
          message: error.message,
          severity: 'error'
        })
        throw error
      } finally {
        this.isReverting = false
      }
    },

    async cancelJob(jobId) {
      try {
        const response = await fetch(`/api/import/${jobId}`, {
          method: 'DELETE'
        })

        if (!response.ok) {
          throw new Error(`Cancel failed: ${response.statusText}`)
        }

        // Remove from jobs list
        this.jobs = this.jobs.filter(j => j.id !== jobId)

        if (this.currentJob?.id === jobId) {
          this.currentJob = null
          this.previewData = null
          this.decisions = {}
        }
      } catch (error) {
        this.errors.push({
          type: 'cancel',
          message: error.message,
          severity: 'error'
        })
        throw error
      }
    },

    async loadJobs() {
      try {
        const response = await fetch('/api/import')

        if (!response.ok) {
          throw new Error(`Load jobs failed: ${response.statusText}`)
        }

        this.jobs = await response.json()
      } catch (error) {
        this.errors.push({
          type: 'load',
          message: error.message,
          severity: 'error'
        })
        throw error
      }
    },

    updateDecision(conversationId, decision) {
      this.decisions[conversationId] = {
        ...this.decisions[conversationId],
        ...decision
      }
    },

    batchUpdateDecisions(conversationIds, action) {
      conversationIds.forEach(id => {
        this.updateDecision(id, { action })
      })
    },

    clearErrors() {
      this.errors = []
    },

    clearCurrentJob() {
      this.currentJob = null
      this.previewData = null
      this.decisions = {}
      this.errors = []
    },

    // Zettelkasten configuration actions
    setLinkMode(mode) {
      this.zettelConfig.linkMode = mode
    },

    toggleZettelEnabled() {
      this.zettelConfig.enabled = !this.zettelConfig.enabled
    },

    setZettelConfig(config) {
      this.zettelConfig = { ...this.zettelConfig, ...config }
    },

    getZettelTypeLabel(type) {
      const t = this.zettelTypes.find(z => z.value === type)
      return t ? t.label : type
    },

    getLinkModeLabel(mode) {
      const m = this.linkModes.find(l => l.value === mode)
      return m ? m.label : mode
    },

    async distillZettelkasten(noteId, linkMode = null) {
      if (!this.zettelConfig.enabled) {
        throw new Error('Zettelkasten mode is not enabled')
      }

      const mode = linkMode || this.zettelConfig.linkMode

      try {
        const response = await fetch(`/api/zettel/notes/${noteId}/distill-zettel?link_mode=${mode}`, {
          method: 'POST'
        })

        if (!response.ok) {
          throw new Error(`Zettelkasten distillation failed: ${response.statusText}`)
        }

        return await response.json()
      } catch (error) {
        this.errors.push({
          type: 'zettelkasten',
          message: error.message,
          severity: 'error'
        })
        throw error
      }
    },

    async discoverLinks(noteId, mode = 'suggested') {
      try {
        const response = await fetch(`/api/zettel/notes/${noteId}/discover-links?mode=${mode}`)

        if (!response.ok) {
          throw new Error(`Link discovery failed: ${response.statusText}`)
        }

        return await response.json()
      } catch (error) {
        this.errors.push({
          type: 'link_discovery',
          message: error.message,
          severity: 'error'
        })
        throw error
      }
    },

    async createLink(sourceId, targetId, linkType = 'related') {
      try {
        const response = await fetch(`/api/zettel/notes/${sourceId}/link/${targetId}`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ link_type: linkType })
        })

        if (!response.ok) {
          throw new Error(`Link creation failed: ${response.statusText}`)
        }

        return await response.json()
      } catch (error) {
        this.errors.push({
          type: 'link_creation',
          message: error.message,
          severity: 'error'
        })
        throw error
      }
    }
  }
})