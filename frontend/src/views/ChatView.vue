<template>
  <div class="chat-view">
    <header class="chat-header">
      <router-link
        to="/"
        class="btn btn-ghost back-link"
      >
        &larr; Back to Notes
      </router-link>
      <h1>Chat with Notes</h1>
      <button
        class="btn btn-ghost"
        title="Clear conversation"
        :disabled="messages.length === 0"
        @click="clearChat"
      >
        Clear
      </button>
    </header>

    <div
      ref="messagesContainer"
      class="messages-area"
    >
      <div
        v-if="messages.length === 0"
        class="empty-state"
      >
        <p class="empty-title">
          Ask questions about your notes
        </p>
        <p class="empty-hint">
          Your knowledge base is searched automatically for relevant context.
        </p>
      </div>

      <div
        v-for="msg in messages"
        :key="msg.id"
        class="message"
        :class="msg.role"
      >
        <div class="message-role">
          {{ msg.role === 'user' ? 'You' : 'Assistant' }}
        </div>
        <div
          v-if="msg.role === 'assistant'"
          class="message-content"
          v-html="renderMarkdown(msg.content)"
        />
        <div
          v-else
          class="message-content"
        >
          {{ msg.content }}
        </div>

        <!-- Context notes (shown for assistant messages) -->
        <div
          v-if="msg.contextNotes && msg.contextNotes.length > 0"
          class="context-notes"
        >
          <span class="context-label">Context:</span>
          <router-link
            v-for="note in msg.contextNotes"
            :key="note.id"
            :to="`/?note=${note.id}`"
            class="context-note-link"
            :title="note.snippet || note.title"
          >
            {{ note.title }}
          </router-link>
        </div>

        <!-- Streaming indicator -->
        <div
          v-if="msg.streaming"
          class="streaming-indicator"
        >
          <span class="dot" />
          <span class="dot" />
          <span class="dot" />
        </div>
      </div>
    </div>

    <div class="input-area">
      <textarea
        ref="inputEl"
        v-model="inputText"
        placeholder="Ask a question about your notes..."
        :disabled="isStreaming"
        rows="1"
        @keydown.enter.exact.prevent="sendMessage"
        @input="autoResize"
      />
      <button
        class="btn btn-primary send-btn"
        :disabled="!inputText.trim() || isStreaming"
        @click="sendMessage"
      >
        {{ isStreaming ? '...' : 'Send' }}
      </button>
    </div>
  </div>
</template>

<script setup>
import { ref, nextTick, onBeforeUnmount } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { chat } from '@/api/client'

const messages = ref([])
const inputText = ref('')
const isStreaming = ref(false)
const messagesContainer = ref(null)
const inputEl = ref(null)

let unlisten = null
let currentMessageId = null

// Simple markdown rendering (bold, italic, code, links, headers)
function renderMarkdown(text) {
  if (!text) return ''
  return text
    // Code blocks
    .replace(/```(\w*)\n([\s\S]*?)```/g, '<pre><code>$2</code></pre>')
    // Inline code
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    // Bold
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    // Italic
    .replace(/\*(.+?)\*/g, '<em>$1</em>')
    // Headers
    .replace(/^### (.+)$/gm, '<h4>$1</h4>')
    .replace(/^## (.+)$/gm, '<h3>$1</h3>')
    .replace(/^# (.+)$/gm, '<h2>$1</h2>')
    // Line breaks
    .replace(/\n/g, '<br>')
}

function autoResize() {
  if (!inputEl.value) return
  inputEl.value.style.height = 'auto'
  inputEl.value.style.height = Math.min(inputEl.value.scrollHeight, 150) + 'px'
}

function scrollToBottom() {
  nextTick(() => {
    if (messagesContainer.value) {
      messagesContainer.value.scrollTop = messagesContainer.value.scrollHeight
    }
  })
}

async function sendMessage() {
  const text = inputText.value.trim()
  if (!text || isStreaming.value) return

  // Add user message
  const userMsg = {
    id: Date.now().toString(),
    role: 'user',
    content: text,
    contextNotes: [],
    streaming: false,
  }
  messages.value.push(userMsg)

  // Add placeholder assistant message
  const assistantMsg = {
    id: null,
    role: 'assistant',
    content: '',
    contextNotes: [],
    streaming: true,
  }
  messages.value.push(assistantMsg)

  inputText.value = ''
  if (inputEl.value) inputEl.value.style.height = 'auto'
  isStreaming.value = true
  scrollToBottom()

  // Build history from previous messages (exclude the current pair)
  const history = messages.value
    .slice(0, -2)
    .filter(m => m.content)
    .map(m => ({ role: m.role, content: m.content }))

  try {
    // Set up event listener before invoke
    unlisten = await listen('chat-stream', (event) => {
      const data = event.payload
      if (currentMessageId && data.message_id !== currentMessageId) return

      switch (data.type) {
        case 'context_notes':
          currentMessageId = data.message_id
          assistantMsg.id = data.message_id
          assistantMsg.contextNotes = data.notes || []
          break
        case 'chunk':
          assistantMsg.content += data.chunk
          scrollToBottom()
          break
        case 'complete':
          assistantMsg.streaming = false
          isStreaming.value = false
          currentMessageId = null
          break
        case 'error':
          assistantMsg.content += `\n\n**Error:** ${data.error}`
          assistantMsg.streaming = false
          isStreaming.value = false
          currentMessageId = null
          break
      }
    })

    // Invoke the command — returns message_id immediately
    const messageId = await chat.send({
      message: text,
      history,
      context_note_ids: [],
    })

    // Track the message ID for filtering events
    if (!currentMessageId) {
      currentMessageId = messageId
      assistantMsg.id = messageId
    }
  } catch (e) {
    assistantMsg.content = `**Error:** ${e.message || e}`
    assistantMsg.streaming = false
    isStreaming.value = false
    currentMessageId = null
  }
}

function clearChat() {
  messages.value = []
  currentMessageId = null
}

onBeforeUnmount(() => {
  if (unlisten) {
    unlisten()
    unlisten = null
  }
})
</script>

<style scoped>
.chat-view {
  display: flex;
  flex-direction: column;
  height: 100vh;
  max-width: 900px;
  margin: 0 auto;
  background: var(--bg-primary);
}

.chat-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--bg-tertiary);
  flex-shrink: 0;
}

.chat-header h1 {
  font-size: 1.1rem;
  font-weight: 600;
  color: var(--text-primary);
}

.back-link {
  text-decoration: none;
  font-size: 0.85rem;
}

.messages-area {
  flex: 1;
  overflow-y: auto;
  padding: var(--spacing-lg);
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
}

.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: var(--spacing-sm);
  color: var(--text-muted);
}

.empty-title {
  font-size: 1.1rem;
  font-weight: 600;
  color: var(--text-secondary);
}

.empty-hint {
  font-size: 0.85rem;
}

.message {
  padding: var(--spacing-md);
  border-radius: var(--radius-md);
}

.message.user {
  background: var(--bg-secondary);
  align-self: flex-end;
  max-width: 85%;
}

.message.assistant {
  background: transparent;
  border: 1px solid var(--bg-tertiary);
  max-width: 95%;
}

.message-role {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
  margin-bottom: var(--spacing-xs);
}

.message-content {
  color: var(--text-primary);
  line-height: 1.6;
  word-wrap: break-word;
}

.message-content :deep(pre) {
  background: var(--bg-secondary);
  padding: var(--spacing-sm);
  border-radius: var(--radius-sm);
  overflow-x: auto;
  margin: var(--spacing-sm) 0;
}

.message-content :deep(code) {
  font-family: 'JetBrains Mono', monospace;
  font-size: 0.85em;
}

.message-content :deep(h2),
.message-content :deep(h3),
.message-content :deep(h4) {
  margin-top: var(--spacing-md);
  margin-bottom: var(--spacing-xs);
  color: var(--text-primary);
}

.context-notes {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--spacing-xs);
  margin-top: var(--spacing-sm);
  padding-top: var(--spacing-sm);
  border-top: 1px solid var(--bg-tertiary);
}

.context-label {
  font-size: 0.7rem;
  font-weight: 600;
  text-transform: uppercase;
  color: var(--text-muted);
}

.context-note-link {
  font-size: 0.75rem;
  padding: 1px 8px;
  background: var(--accent-primary);
  color: white;
  border-radius: var(--radius-sm);
  text-decoration: none;
  transition: opacity var(--transition-fast);
}

.context-note-link:hover {
  opacity: 0.8;
}

.streaming-indicator {
  display: flex;
  gap: 4px;
  margin-top: var(--spacing-xs);
}

.streaming-indicator .dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--text-muted);
  animation: bounce 1.2s infinite ease-in-out;
}

.streaming-indicator .dot:nth-child(2) {
  animation-delay: 0.2s;
}

.streaming-indicator .dot:nth-child(3) {
  animation-delay: 0.4s;
}

@keyframes bounce {
  0%, 80%, 100% { opacity: 0.3; }
  40% { opacity: 1; }
}

.input-area {
  display: flex;
  gap: var(--spacing-sm);
  padding: var(--spacing-md) var(--spacing-lg);
  border-top: 1px solid var(--bg-tertiary);
  flex-shrink: 0;
}

.input-area textarea {
  flex: 1;
  resize: none;
  border: 1px solid var(--bg-tertiary);
  border-radius: var(--radius-md);
  padding: var(--spacing-sm) var(--spacing-md);
  font-size: 0.95rem;
  font-family: inherit;
  background: var(--bg-secondary);
  color: var(--text-primary);
  line-height: 1.5;
  min-height: 40px;
  max-height: 150px;
}

.input-area textarea:focus {
  outline: none;
  border-color: var(--accent-primary);
}

.input-area textarea:disabled {
  opacity: 0.6;
}

.send-btn {
  align-self: flex-end;
  padding: var(--spacing-sm) var(--spacing-lg);
  min-width: 70px;
}
</style>
