<template>
  <div class="canvas-view">
    <!-- Sidebar -->
    <aside class="canvas-sidebar">
      <div class="sidebar-header">
        <h2>Canvas Sessions</h2>
        <div class="header-actions">
          <button
            class="btn btn-ghost btn-sm"
            title="Settings"
            data-guide="canvas-settings-btn"
            @click="showSettingsModal = true"
          >
            <GIcon name="settings" />
          </button>
          <button
            class="btn btn-ghost btn-sm"
            title="Toggle Theme"
            @click="handleThemeToggle"
          >
            <GIcon :name="themeStore.theme === 'dark' ? 'moon' : 'sun'" />
          </button>
          <button
            class="btn btn-ghost btn-sm"
            title="Guide"
            @click="guide.togglePanel()"
          >
            <GIcon name="help-circle" />
          </button>
          <button
            class="btn btn-primary btn-sm"
            data-guide="canvas-new-btn"
            @click="createNewSession"
          >
            + New
          </button>
        </div>
      </div>

      <div class="sessions-list">
        <div
          v-for="session in sessions"
          :key="session.id"
          class="session-item"
          :class="{ active: session.id === currentSessionId }"
          @click="selectSession(session.id)"
        >
          <div class="session-info">
            <span class="session-title">{{ session.title }}</span>
            <span class="session-meta">
              {{ session.tile_count }} tiles | {{ formatDate(session.updated_at) }}
            </span>
          </div>
          <button
            class="delete-btn"
            title="Delete session"
            @click.stop="deleteSession(session.id)"
          >
            <GIcon
              name="x"
              :size="12"
            />
          </button>
        </div>

        <div
          v-if="sessions.length === 0 && !loading"
          class="empty-state"
        >
          <p>No canvas sessions yet</p>
          <button
            class="btn btn-primary"
            @click="createNewSession"
          >
            Create your first canvas
          </button>
        </div>
      </div>

      <div class="sidebar-footer">
        <router-link
          to="/"
          class="back-link"
        >
          &#8592; Back to Notes
        </router-link>
      </div>
    </aside>

    <!-- Main Canvas Area -->
    <main class="canvas-main">
      <div
        v-if="!currentSessionId"
        class="no-session"
      >
        <div class="no-session-content">
          <h2>Multi-LLM Canvas</h2>
          <p>Compare responses from multiple AI models side by side</p>
          <button
            class="btn btn-primary btn-lg"
            @click="createNewSession"
          >
            Create New Canvas
          </button>
        </div>
      </div>

      <CanvasContainer
        v-else
        :session-id="currentSessionId"
        @session-loaded="onSessionLoaded"
      />
    </main>

    <!-- Create Session Dialog -->
    <div
      v-if="showCreateDialog"
      class="dialog-overlay"
      @click.self="showCreateDialog = false"
    >
      <div class="create-dialog">
        <div class="dialog-header">
          <h3>New Canvas Session</h3>
          <button
            class="close-btn"
            @click="showCreateDialog = false"
          >
            <GIcon
              name="x"
              :size="12"
            />
          </button>
        </div>
        <div class="dialog-body">
          <div class="form-group">
            <label for="sessionTitle">Title</label>
            <input
              id="sessionTitle"
              v-model="newSessionTitle"
              type="text"
              placeholder="Enter canvas title..."
              @keydown.enter="confirmCreateSession"
            >
          </div>
          <div class="form-group">
            <label for="sessionDescription">Description (optional)</label>
            <textarea
              id="sessionDescription"
              v-model="newSessionDescription"
              placeholder="Brief description..."
              rows="2"
            />
          </div>
        </div>
        <div class="dialog-footer">
          <button
            class="btn btn-secondary"
            @click="showCreateDialog = false"
          >
            Cancel
          </button>
          <button
            class="btn btn-primary"
            :disabled="!newSessionTitle.trim()"
            @click="confirmCreateSession"
          >
            Create
          </button>
        </div>
      </div>
    </div>

    <!-- Settings Modal (Desktop) -->
    <SettingsModal
      v-model="showSettingsModal"
      :is-setup="false"
      @saved="handleSettingsSaved"
    />

    <ConfirmDialog
      :visible="showDeleteConfirm"
      title="Delete Canvas Session"
      message="Are you sure you want to delete this canvas session? This cannot be undone."
      confirm-label="Delete"
      cancel-label="Cancel"
      variant="danger"
      @confirm="confirmDeleteSession"
      @cancel="showDeleteConfirm = false"
    />
  </div>
</template>

<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useCanvasStore } from '@/stores/canvas'
import { useThemeStore } from '@/stores/theme'
import { isDesktopApp, settings as settingsApi } from '@/api/client'
import CanvasContainer from '@/components/canvas/CanvasContainer.vue'
import SettingsModal from '@/components/SettingsModal.vue'
import ConfirmDialog from '@/components/ConfirmDialog.vue'
import GIcon from '@/components/ui/GIcon.vue'
import { useGuide } from '@/composables/useGuide'

const route = useRoute()
const router = useRouter()
const canvasStore = useCanvasStore()
const themeStore = useThemeStore()

// Local state
const showCreateDialog = ref(false)
const newSessionTitle = ref('')
const newSessionDescription = ref('')
const showSettingsModal = ref(false)
const showDeleteConfirm = ref(false)
const pendingDeleteSessionId = ref(null)
const _isDesktop = isDesktopApp()
const guide = useGuide()

// Function to toggle theme
async function handleThemeToggle() {
  const nextTheme = themeStore.theme === 'dark' ? 'light' : 'dark'
  themeStore.setTheme(nextTheme)

  try {
    await settingsApi.update({ theme: nextTheme })
  } catch (err) {
    console.error('Failed to persist theme preference:', err)
  }
}

// Computed
const sessions = computed(() => canvasStore.sessions)
const loading = computed(() => canvasStore.loading)
const currentSessionId = computed(() => route.params.id || null)

// Lifecycle
onMounted(async () => {
  await canvasStore.loadSessions()
})

// Watch for route changes
watch(() => route.params.id, async (newId) => {
  if (newId) {
    await canvasStore.loadSession(newId)
  }
})

// Methods
function selectSession(sessionId) {
  router.push(`/canvas/${sessionId}`)
}

function createNewSession() {
  newSessionTitle.value = `Canvas ${new Date().toLocaleDateString()}`
  newSessionDescription.value = ''
  showCreateDialog.value = true
}

async function confirmCreateSession() {
  if (!newSessionTitle.value.trim()) return

  try {
    const session = await canvasStore.createSession({
      title: newSessionTitle.value.trim(),
      description: newSessionDescription.value.trim() || null
    })
    showCreateDialog.value = false
    router.push(`/canvas/${session.id}`)
  } catch (err) {
    console.error('Failed to create session:', err)
  }
}

function deleteSession(sessionId) {
  pendingDeleteSessionId.value = sessionId
  showDeleteConfirm.value = true
}

async function confirmDeleteSession() {
  const sessionId = pendingDeleteSessionId.value
  showDeleteConfirm.value = false
  pendingDeleteSessionId.value = null
  try {
    await canvasStore.deleteSession(sessionId)
    if (currentSessionId.value === sessionId) {
      router.push('/canvas')
    }
  } catch (err) {
    console.error('Failed to delete session:', err)
  }
}

function onSessionLoaded(_session) {
  // Could update page title, etc.
}

function handleSettingsSaved() {
  // Reload the canvas to pick up the new API key
  showSettingsModal.value = false
  // Reload models with the new API key
  canvasStore.loadModels()
}

function formatDate(dateStr) {
  if (!dateStr) return ''
  const date = new Date(dateStr)
  return date.toLocaleDateString()
}
</script>

<style scoped>
.canvas-view {
  display: flex;
  height: 100vh;
  background: var(--bg-primary);
}

.canvas-sidebar {
  width: 280px;
  background: var(--bg-secondary);
  border-right: 1px solid var(--border-default);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  box-shadow: var(--shadow-lg);
}

.sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md);
  border-bottom: 1px solid var(--border-subtle);
}

.sidebar-header h2 {
  margin: 0;
  font-size: 1rem;
  color: var(--text-primary);
}

.header-actions {
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
}

.sessions-list {
  flex: 1;
  overflow-y: auto;
  padding: var(--spacing-sm);
}

.session-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-sm) var(--spacing-md);
  margin-bottom: var(--spacing-xs);
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.session-item:hover {
  background: var(--bg-tertiary);
  border-color: var(--border-subtle);
}

.session-item.active {
  background: color-mix(in srgb, var(--accent-primary) 12%, transparent);
  border-color: var(--accent-primary);
  border-left: 3px solid var(--accent-primary);
}

.session-info {
  flex: 1;
  min-width: 0;
}

.session-title {
  display: block;
  font-size: 0.875rem;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.session-meta {
  display: block;
  font-size: 0.75rem;
  color: var(--text-muted);
  margin-top: 2px;
}

.delete-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-sm);
  opacity: 0;
  transition: all 0.15s;
}

.session-item:hover .delete-btn {
  opacity: 1;
}

.delete-btn:hover {
  background: rgba(248, 113, 113, 0.2);
  color: var(--accent-red);
}

.empty-state {
  text-align: center;
  padding: var(--spacing-lg);
  color: var(--text-muted);
}

.empty-state p {
  margin-bottom: var(--spacing-md);
}

.sidebar-footer {
  padding: var(--spacing-md);
  border-top: 1px solid var(--border-subtle);
}

.back-link {
  color: var(--text-secondary);
  text-decoration: none;
  font-size: 0.875rem;
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  padding: var(--spacing-xs) var(--spacing-sm);
  border-radius: var(--radius-sm);
  transition: all 150ms ease;
}

.back-link:hover {
  color: var(--accent-primary);
  background: var(--bg-hover);
}

.canvas-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.no-session {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}

.no-session-content {
  text-align: center;
  max-width: 400px;
}

.no-session-content h2 {
  font-size: 1.5rem;
  color: var(--text-primary);
  margin-bottom: var(--spacing-sm);
}

.no-session-content p {
  color: var(--text-secondary);
  margin-bottom: var(--spacing-lg);
}


/* Dialog styles */
.dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.7);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  backdrop-filter: blur(8px);
}

.create-dialog {
  background: var(--bg-secondary);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  width: 100%;
  max-width: 400px;
  box-shadow: var(--shadow-xl);
}

.dialog-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--border-subtle);
}

.dialog-header h3 {
  margin: 0;
  font-size: 1.125rem;
  color: var(--text-primary);
}

.close-btn {
  width: 32px;
  height: 32px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-sm);
}

.close-btn:hover {
  background: var(--bg-tertiary);
  color: var(--text-primary);
}

.dialog-body {
  padding: var(--spacing-lg);
  display: flex;
  flex-direction: column;
  gap: var(--spacing-md);
}

.form-group {
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
}

.form-group label {
  font-size: 0.875rem;
  color: var(--text-secondary);
}

.form-group input,
.form-group textarea {
  padding: var(--spacing-sm);
  background: var(--bg-tertiary);
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.875rem;
  font-family: inherit;
}

.form-group input:focus,
.form-group textarea:focus {
  border-color: var(--accent-primary);
  outline: none;
}

.dialog-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--spacing-sm);
  padding: var(--spacing-md) var(--spacing-lg);
  border-top: 1px solid var(--border-subtle);
}

</style>
