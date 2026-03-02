# Phase 7: Chat with Notes Feature

## Goal
Implement the chat-with-notes feature — a conversational interface where users ask questions and get LLM responses augmented with context from their knowledge base. This is the **payoff feature** that justifies the consolidation: it leverages the Python backend's semantic search (LanceDB + sentence-transformers) and graph-boosted recall that the Rust backend never had.

---

## Architecture

```
User question
    ↓
recall_relevant(question, context_notes)
    ↓
Top N notes (semantic search + graph boost)
    ↓
Build system prompt with note context
    ↓
Stream from OpenRouter (user's chosen model)
    ↓
Display response with source citations
```

This is a **RAG (Retrieval-Augmented Generation)** pattern — the same approach used by ChatGPT with file attachments, but over the user's entire knowledge base.

---

## Task 1: Backend Chat Endpoint

### Update `backend/app/routers/chat.py`

Replace the Phase 1 stub with the full implementation:

```python
"""Chat API — conversational interface over your knowledge base."""

from fastapi import APIRouter, Request
from fastapi.responses import StreamingResponse
from pydantic import BaseModel, Field
from typing import List, Optional
import json

from app.utils.dependencies import get_memory_service, get_openrouter

router = APIRouter()


class ChatMessage(BaseModel):
    role: str  # "user", "assistant", or "system"
    content: str


class ChatRequest(BaseModel):
    messages: List[ChatMessage]
    model: str = "anthropic/claude-3-haiku"
    context_note_ids: Optional[List[str]] = None
    max_context_notes: int = 5
    temperature: float = 0.7
    max_tokens: int = 2048


class ContextNote(BaseModel):
    """A note used as context for the chat response."""
    id: str
    title: str
    snippet: str
    score: float


class ChatResponse(BaseModel):
    """Non-streaming response (for simple cases)."""
    content: str
    context_notes: List[ContextNote]
    model: str


@router.post("/completions")
async def chat_completions(request: ChatRequest, req: Request):
    """
    Chat with your notes — streams LLM response with knowledge base context.

    Pipeline:
    1. Extract the latest user message as the recall query
    2. Use recall_relevant() to find semantically related notes
    3. Build a system prompt with note excerpts
    4. Stream the LLM response from OpenRouter

    Returns: SSE stream with events:
    - context: { notes: [...] }  — retrieved context notes
    - chunk: { content: "..." }  — streaming text
    - done: { usage: {...} }     — completion signal
    - error: { message: "..." }  — error
    """
    memory_service = get_memory_service(req)
    openrouter = get_openrouter(req)

    # 1. Extract query from latest user message
    user_messages = [m for m in request.messages if m.role == "user"]
    if not user_messages:
        return StreamingResponse(
            _error_stream("No user message found"),
            media_type="text/event-stream"
        )
    query = user_messages[-1].content

    # 2. Retrieve relevant notes
    context_note_ids = request.context_note_ids or []
    try:
        from app.utils.dependencies import get_vector_search, get_graph_index
        vector_search = req.app.state.vector_search
        graph_index = req.app.state.graph_index

        recall_results = memory_service.recall_relevant(
            vector_search=vector_search,
            graph_index=graph_index,
            query=query,
            context_note_ids=context_note_ids,
            limit=request.max_context_notes,
        )
    except Exception as e:
        recall_results = []

    # 3. Build context-augmented messages
    context_notes = []
    context_text = ""
    for result in recall_results:
        context_notes.append(ContextNote(
            id=result.note_id,
            title=result.title,
            snippet=result.snippet[:200],
            score=result.total_score,
        ))
        context_text += f"\n\n---\n**{result.title}** (score: {result.total_score:.2f})\n{result.snippet}"

    system_prompt = _build_system_prompt(context_text, len(context_notes))

    # Build messages for OpenRouter
    llm_messages = [{"role": "system", "content": system_prompt}]
    for msg in request.messages:
        llm_messages.append({"role": msg.role, "content": msg.content})

    # 4. Stream from OpenRouter
    return StreamingResponse(
        _stream_chat(
            openrouter=openrouter,
            messages=llm_messages,
            model=request.model,
            temperature=request.temperature,
            max_tokens=request.max_tokens,
            context_notes=context_notes,
        ),
        media_type="text/event-stream",
        headers={
            "Cache-Control": "no-cache",
            "X-Accel-Buffering": "no",
        },
    )


def _build_system_prompt(context_text: str, num_notes: int) -> str:
    """Build a system prompt that includes retrieved note context."""
    if not context_text:
        return (
            "You are a helpful assistant with access to the user's knowledge base. "
            "No relevant notes were found for this query. Answer based on your general knowledge, "
            "but let the user know if their knowledge base might not have relevant information."
        )

    return f"""You are a helpful assistant with access to the user's knowledge base (Grafyn).
You have been provided with {num_notes} relevant notes from their knowledge base below.

When answering:
- Reference specific notes by title when they inform your answer
- Use [[Note Title]] wikilink syntax when citing notes
- If the notes don't fully answer the question, say so and supplement with general knowledge
- Be concise but thorough

## Relevant Notes from Knowledge Base
{context_text}

## Instructions
Answer the user's question using the above notes as context. Cite your sources using [[wikilinks]]."""


async def _stream_chat(openrouter, messages, model, temperature, max_tokens, context_notes):
    """Generator that yields SSE events for the chat stream."""
    # First, send context notes
    context_event = {
        "type": "context",
        "notes": [n.model_dump() for n in context_notes],
    }
    yield f"data: {json.dumps(context_event)}\n\n"

    # Stream from OpenRouter
    try:
        full_content = ""
        async for chunk in openrouter.stream_chat(
            messages=messages,
            model=model,
            temperature=temperature,
            max_tokens=max_tokens,
        ):
            if chunk.get("content"):
                full_content += chunk["content"]
                yield f"data: {json.dumps({'type': 'chunk', 'content': chunk['content']})}\n\n"

        yield f"data: {json.dumps({'type': 'done', 'content': full_content})}\n\n"
    except Exception as e:
        yield f"data: {json.dumps({'type': 'error', 'message': str(e)})}\n\n"

    yield "data: [DONE]\n\n"


async def _error_stream(message: str):
    """Yield an error event."""
    yield f"data: {json.dumps({'type': 'error', 'message': message})}\n\n"
    yield "data: [DONE]\n\n"
```

### Add `stream_chat` to OpenRouterService

The existing `OpenRouterService` has streaming support for canvas prompts. Add a simplified version for chat:

```python
# In backend/app/services/openrouter.py

async def stream_chat(self, messages, model, temperature=0.7, max_tokens=2048):
    """Stream a chat completion from OpenRouter. Yields content chunks."""
    async with self.client.stream(
        "POST",
        "https://openrouter.ai/api/v1/chat/completions",
        headers={
            "Authorization": f"Bearer {self.api_key}",
            "HTTP-Referer": self.app_url,
        },
        json={
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "stream": True,
        },
    ) as response:
        response.raise_for_status()
        async for line in response.aiter_lines():
            if not line.startswith("data: "):
                continue
            data = line[6:]
            if data == "[DONE]":
                break
            try:
                chunk = json.loads(data)
                delta = chunk.get("choices", [{}])[0].get("delta", {})
                content = delta.get("content")
                if content:
                    yield {"content": content}
            except json.JSONDecodeError:
                continue
```

---

## Task 2: Frontend Chat Component

### Create `frontend/src/views/ChatView.vue`

```vue
<template>
  <div class="chat-view">
    <div class="chat-header">
      <h2>Chat with Notes</h2>
      <ModelSelector v-model="selectedModel" />
    </div>

    <div class="chat-messages" ref="messagesContainer">
      <!-- Context notes banner (shown when notes are retrieved) -->
      <div v-if="contextNotes.length" class="context-banner">
        <span>Using {{ contextNotes.length }} notes as context:</span>
        <div class="context-chips">
          <router-link
            v-for="note in contextNotes"
            :key="note.id"
            :to="{ path: '/', query: { note: note.id } }"
            class="context-chip"
          >
            {{ note.title }}
          </router-link>
        </div>
      </div>

      <!-- Messages -->
      <div
        v-for="(msg, i) in messages"
        :key="i"
        :class="['message', msg.role]"
      >
        <div class="message-content" v-html="renderMarkdown(msg.content)" />
      </div>

      <!-- Streaming indicator -->
      <div v-if="isStreaming" class="message assistant streaming">
        <div class="message-content" v-html="renderMarkdown(streamingContent)" />
        <span class="cursor" />
      </div>
    </div>

    <div class="chat-input">
      <textarea
        v-model="input"
        @keydown.enter.exact="sendMessage"
        placeholder="Ask about your notes..."
        :disabled="isStreaming"
        rows="2"
      />
      <button @click="sendMessage" :disabled="isStreaming || !input.trim()">
        Send
      </button>
    </div>
  </div>
</template>
```

### Create `frontend/src/stores/chat.js`

```javascript
import { defineStore } from 'pinia'
import { ref } from 'vue'
import { getApiBaseUrl } from '@/api/client'

export const useChatStore = defineStore('chat', () => {
  const messages = ref([])
  const contextNotes = ref([])
  const isStreaming = ref(false)
  const streamingContent = ref('')
  const selectedModel = ref('anthropic/claude-3-haiku')

  async function sendMessage(content) {
    // Add user message
    messages.value.push({ role: 'user', content })
    isStreaming.value = true
    streamingContent.value = ''
    contextNotes.value = []

    try {
      const apiBase = await getApiBaseUrl()
      const response = await fetch(`${apiBase}/chat/completions`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          messages: messages.value.map(m => ({
            role: m.role,
            content: m.content,
          })),
          model: selectedModel.value,
        }),
      })

      if (!response.ok) throw new Error(`HTTP ${response.status}`)

      const reader = response.body.getReader()
      const decoder = new TextDecoder()

      while (true) {
        const { done, value } = await reader.read()
        if (done) break

        const text = decoder.decode(value)
        for (const line of text.split('\n')) {
          if (!line.startsWith('data: ')) continue
          const data = line.slice(6)
          if (data === '[DONE]') break

          try {
            const event = JSON.parse(data)
            switch (event.type) {
              case 'context':
                contextNotes.value = event.notes
                break
              case 'chunk':
                streamingContent.value += event.content
                break
              case 'done':
                messages.value.push({
                  role: 'assistant',
                  content: event.content,
                })
                streamingContent.value = ''
                break
              case 'error':
                messages.value.push({
                  role: 'assistant',
                  content: `Error: ${event.message}`,
                })
                break
            }
          } catch (e) {
            console.error('Parse error:', e)
          }
        }
      }
    } catch (err) {
      messages.value.push({
        role: 'assistant',
        content: `Error: ${err.message}`,
      })
    } finally {
      isStreaming.value = false
    }
  }

  function clearChat() {
    messages.value = []
    contextNotes.value = []
    streamingContent.value = ''
  }

  return {
    messages,
    contextNotes,
    isStreaming,
    streamingContent,
    selectedModel,
    sendMessage,
    clearChat,
  }
})
```

### Add route

```javascript
// In frontend/src/router/index.js
{
  path: '/chat',
  name: 'chat',
  component: () => import('@/views/ChatView.vue'),
}
```

### Add navigation

Add a "Chat" link to the sidebar/navigation alongside Notes, Canvas, and Import.

---

## Task 3: Recall Service Enhancements

The current `MemoryService.recall_relevant()` returns search results with graph boost. For chat, we may want:

### Enhanced recall with content extraction

```python
# In backend/app/services/memory.py

def recall_for_chat(self, query, context_note_ids=None, limit=5):
    """Recall relevant notes and return full content excerpts for LLM context."""
    results = self.recall_relevant(
        vector_search=self.vector_search,
        graph_index=self.graph_index,
        query=query,
        context_note_ids=context_note_ids or [],
        limit=limit,
    )

    # Enrich with full content (truncated to avoid token overflow)
    enriched = []
    for r in results:
        note = self.knowledge_store.get_note(r.note_id)
        if note:
            content = note.content[:2000]  # Cap at ~500 tokens
            enriched.append({
                **r.__dict__,
                "full_content": content,
            })

    return enriched
```

---

## Task 4: Wikilink Rendering in Chat Responses

When the LLM cites `[[Note Title]]` in responses, render them as clickable links:

```javascript
// In ChatView.vue or a composable
function renderMarkdown(content) {
  // Standard markdown rendering (use marked or similar)
  let html = marked.parse(content)

  // Convert [[wikilinks]] to router links
  html = html.replace(
    /\[\[([^\]|]+?)(?:\|([^\]]+?))?\]\]/g,
    (match, title, display) => {
      const slug = title.toLowerCase().replace(/\s+/g, '-')
      const text = display || title
      return `<a href="/?note=${encodeURIComponent(slug)}" class="wikilink">${text}</a>`
    }
  )

  return html
}
```

---

## Files Modified
| File | Action |
|------|--------|
| `backend/app/routers/chat.py` | **Rewrite** — full chat endpoint (replace Phase 1 stub) |
| `backend/app/services/openrouter.py` | **Edit** — add `stream_chat()` method |
| `frontend/src/views/ChatView.vue` | **Create** — chat UI component |
| `frontend/src/stores/chat.js` | **Create** — chat state management |
| `frontend/src/router/index.js` | **Edit** — add `/chat` route |
| Navigation component | **Edit** — add Chat link |

## Validation
- `POST /api/chat/completions` streams SSE response
- Context notes appear in the response stream before the first chunk
- Chat UI shows retrieved notes as chips
- `[[wikilinks]]` in responses are clickable
- Conversation history is maintained across messages
- Empty knowledge base gracefully falls back to general LLM response
- Model selector works (multiple OpenRouter models)

## Future Enhancements
- **Note pinning**: Pin specific notes as persistent context
- **Chat history**: Persist conversations to disk
- **Multi-note context**: Click notes in the sidebar to add them as context
- **Follow-up recall**: Re-run recall on assistant responses to discover more notes
- **Inline note creation**: "Save this answer as a note" button
