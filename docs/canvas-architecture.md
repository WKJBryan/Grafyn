# Canvas Architecture - Multi-LLM Comparison System

> **Part:** Canvas Feature | **Type:** Multi-LLM Canvas | **Scan Level:** Comprehensive

## Overview

The Canvas is a visual, interactive system for comparing responses from multiple AI models side-by-side. It enables users to:

- Send prompts to multiple AI models simultaneously
- Compare model responses in real-time with streaming
- Branch conversations from any model response
- Conduct debates between AI models
- Export canvas content as markdown notes
- Organize responses with drag-and-drop positioning

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         CanvasView.vue                               │
│  ┌──────────────────────┐  ┌──────────────────────────────────┐   │
│  │   Session Sidebar    │  │      CanvasContainer.vue        │   │
│  │   (List sessions)    │  │  ┌──────────────────────────┐   │   │
│  └──────────────────────┘  │  │   Canvas Surface (D3)   │   │   │
│                           │  │  ┌────────────────────┐   │   │   │
│                           │  │  │  PromptNodes      │   │   │   │
│                           │  │  └────────────────────┘   │   │   │
│                           │  │  ┌────────────────────┐   │   │   │
│                           │  │  │  LLMNodes        │   │   │   │
│                           │  │  │  (Individual)    │   │   │   │
│                           │  │  └────────────────────┘   │   │   │
│                           │  │  ┌────────────────────┐   │   │   │
│                           │  │  │  DebateNodes     │   │   │   │
│                           │  │  └────────────────────┘   │   │   │
│                           │  │  ┌────────────────────┐   │   │   │
│                           │  │  │  SVG Edges      │   │   │   │
│                           │  │  └────────────────────┘   │   │   │
│                           │  └──────────────────────────┘   │   │   │
│                           └──────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                      Pinia Store (canvas.js)                           │
│  - State: sessions, currentSession, availableModels, streamingModels  │
│  - Actions: sendPrompt, startDebate, branchFromResponse              │
│  - Getters: promptTiles, debates, tileEdges, debateEdges            │
└─────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                      Axios API Client                                │
│  - canvas.list()                                                   │
│  - canvas.get(sessionId)                                            │
│  - canvas.create(data)                                               │
│  - canvas.update(sessionId, data)                                    │
│  - canvas.delete(sessionId)                                          │
│  - canvas.getModels()                                               │
│  - canvas.updateTilePosition()                                       │
│  - canvas.updateLLMNodePosition()                                    │
│  - canvas.autoArrange()                                              │
│  - canvas.deleteTile()                                              │
│  - canvas.updateViewport()                                            │
│  - canvas.startDebate()                                             │
│  - canvas.continueDebate()                                           │
│  - canvas.exportToNote()                                            │
└─────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                    Backend FastAPI Router (canvas.py)                   │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │ Session Management                                           │   │
│  │  GET    /api/canvas              - List all sessions          │   │
│  │  POST   /api/canvas              - Create new session         │   │
│  │  GET    /api/canvas/{id}         - Get session details       │   │
│  │  PUT    /api/canvas/{id}         - Update session metadata   │   │
│  │  DELETE /api/canvas/{id}         - Delete session            │   │
│  │  GET    /api/canvas/models/available - List available models │   │
│  └──────────────────────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │ Prompt & Streaming (SSE)                                   │   │
│  │  POST   /api/canvas/{id}/prompt    - Send to multiple models│   │
│  └──────────────────────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │ Debate Mode (SSE)                                          │   │
│  │  POST   /api/canvas/{id}/debate    - Start debate           │   │
│  │  POST   /api/canvas/{id}/debate/{did}/continue - Continue    │   │
│  │  PUT    /api/canvas/{id}/debate/{did}/status - Update status│   │
│  └──────────────────────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │ Position & Layout                                           │   │
│  │  PUT    /api/canvas/{id}/viewport   - Update viewport       │   │
│  │  PUT    /api/canvas/{id}/tiles/{tid}/position              │   │
│  │  PUT    /api/canvas/{id}/tiles/{tid}/responses/{mid}/pos   │   │
│  │  POST   /api/canvas/{id}/arrange   - Auto-arrange nodes   │   │
│  └──────────────────────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │ Export & Utilities                                         │   │
│  │  POST   /api/canvas/{id}/export-note - Export to markdown   │   │
│  │  DELETE /api/canvas/{id}/tiles/{tid} - Delete tile          │   │
│  │  GET    /api/canvas/{id}/node-edges - Get graph edges       │   │
│  │  GET    /api/canvas/{id}/node-groups - Find components     │   │
│  └──────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                     Backend Services                                 │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ CanvasSessionStore (canvas_store.py)                        │    │
│  │  - Session persistence (JSON files)                         │    │
│  │  - Tile management (prompt, debate)                         │    │
│  │  - Position tracking                                       │    │
│  │  - Conversation context building (3 modes)                    │    │
│  │  - Node graph operations (edges, groups)                     │    │
│  └──────────────────────────────────────────────────────────────┘    │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ OpenRouterService (openrouter.py)                         │    │
│  │  - Multi-LLM API client (100+ models)                     │    │
│  │  - Streaming completion support                            │    │
│  │  - Model list caching                                     │    │
│  └──────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                     External Services                               │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ OpenRouter API (https://openrouter.ai/api/v1)            │    │
│  │  - Unified API for 100+ AI models                        │    │
│  │  - Streaming chat completions                              │    │
│  │  - Model metadata (pricing, context length)                │    │
│  └──────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────┘
```

## Core Concepts

### Canvas Session

A canvas session represents a complete multi-LLM exploration:

- **Metadata**: Title, description, tags, status, timestamps
- **Prompt Tiles**: Individual prompts with multiple model responses
- **Debate Rounds**: Structured debates between models
- **Viewport State**: Pan/zoom position for persistence
- **Linked Note**: Optional connection to a markdown note

### Prompt Tiles

Each prompt tile contains:

- **Prompt**: The user's input text
- **System Prompt**: Optional system-level instructions
- **Models**: List of model IDs to query
- **Responses**: Individual model responses with streaming status
- **Position**: Canvas coordinates (x, y, width, height)
- **Parent References**: For branching from other tiles

### LLM Response Nodes

Each model response is rendered as an individual node:

- **Model ID**: OpenRouter model identifier (e.g., `openai/gpt-4`)
- **Model Name**: Display name for UI
- **Content**: Streaming response text
- **Status**: `pending`, `streaming`, `completed`, `error`
- **Position**: Independent canvas coordinates
- **Color**: Assigned from palette for visual distinction

### Debate Rounds

Debates enable models to critique each other:

- **Participating Models**: 2+ models in the debate
- **Source Tiles**: Which prompt responses to debate
- **Mode**: `auto` (automatic rounds) or `mediated` (user-controlled)
- **Rounds**: Array of model responses per round
- **Status**: `active`, `paused`, `completed`

## Conversation Context Modes

The canvas supports three context modes for branching conversations:

### Full History Mode

Walks the entire parent chain, including all conversation turns:

```python
# Example: 3-turn conversation
Tile 1 → GPT-4 → Tile 2 → Claude → Tile 3 (current)
         ↓            ↓           ↓
      Response 1   Response 2   (new prompt)
```

Messages sent to LLM:
```json
[
  {"role": "user", "content": "First question"},
  {"role": "assistant", "content": "Response 1"},
  {"role": "user", "content": "Follow-up question"},
  {"role": "assistant", "content": "Response 2"},
  {"role": "user", "content": "New question"}
]
```

### Compact Mode

Recent turns included verbatim, older turns summarized:

```json
[
  {"role": "system", "content": "Previous discussion covered: First question; Follow-up question"},
  {"role": "user", "content": "Follow-up question"},
  {"role": "assistant", "content": "Response 2"},
  {"role": "user", "content": "New question"}
]
```

### Semantic Mode (RAG)

Uses vector search to find relevant context from notes and tiles:

```python
# Search across knowledge base
results = vector_search.search_all(prompt, limit=3)

# Context includes:
# - Previous canvas tiles with similar content
# - Notes with semantic similarity
```

## Node Graph Structure

### Node ID Format

- **Prompt Node**: `prompt:{tile_id}`
- **LLM Node**: `llm:{tile_id}:{model_id}`
- **Debate Node**: `debate:{debate_id}`

### Edge Types

| Edge Type | Source → Target | Visual Style |
|-----------|----------------|--------------|
| `PROMPT_TO_LLM` | Prompt → LLM Response | Solid line, model color |
| `LLM_TO_PROMPT` | LLM Response → Branch Prompt | Dashed line |
| `DEBATE_TO_LLM` | Debate → Source LLMs | Cyan solid line |

### Connected Components

The canvas can identify isolated node groups using BFS traversal:

```python
# Example: Two separate conversation trees
Group 1: [prompt:1, llm:1:gpt-4, llm:1:claude, prompt:2]
Group 2: [prompt:3, llm:3:gpt-4]
```

## Streaming Architecture

### Server-Sent Events (SSE)

The canvas uses SSE for real-time streaming:

```javascript
// Frontend SSE handling
const response = await fetch(`/api/canvas/${sessionId}/prompt`, {
  method: 'POST',
  body: JSON.stringify({ prompt, models })
})

const reader = response.body.getReader()
const decoder = new TextDecoder()

while (true) {
  const { done, value } = await reader.read()
  if (done) break
  
  const lines = decoder.decode(value).split('\n')
  for (const line of lines) {
    if (line.startsWith('data: ')) {
      const event = JSON.parse(line.slice(6))
      
      switch (event.type) {
        case 'tile_created':
          // New prompt tile created
          break
        case 'chunk':
          // Streaming content from event.model_id
          break
        case 'complete':
          // Model finished streaming
          break
        case 'session_saved':
          // Session persisted to disk
          break
      }
    }
  }
}
```

### SSE Event Types

| Event Type | Payload | Description |
|------------|----------|-------------|
| `tile_created` | `{tile_id}` | New prompt tile created |
| `chunk` | `{model_id, chunk}` | Streaming text chunk |
| `complete` | `{model_id}` | Model finished |
| `error` | `{model_id, error}` | Model error |
| `session_saved` | - | Session persisted |
| `debate_created` | `{debate_id}` | New debate started |
| `round_start` | `{round}` | Debate round started |
| `debate_chunk` | `{round, model_id, chunk}` | Debate streaming |
| `model_complete` | `{round, model_id}` | Model finished round |
| `debate_complete` | `{debate_id}` | Debate finished |
| `[DONE]` | - | Stream complete |

## Auto-Arrange Algorithm

The canvas includes an automatic layout algorithm:

```python
def layout_prompt_tree(tile, startX, startY):
    # Position prompt node
    positions[f"prompt:{tile.id}"] = {x: startX, y: startY}
    
    # Position LLM nodes to the right (stacked vertically)
    llmX = startX + PROMPT_WIDTH + GAP
    for idx, (model_id, response) in enumerate(tile.responses):
        llmY = startY + (idx * LLM_HEIGHT + GAP)
        positions[f"llm:{tile.id}:{model_id}"] = {x: llmX, y: llmY}
        
        # Recursively layout branches from this LLM
        branches = find_branches(tile.id, model_id)
        for branch in branches:
            branchX = llmX + LLM_WIDTH + GAP
            layout_prompt_tree(branch, branchX, llmY)
```

**Layout Rules:**
1. Root prompts arranged vertically on the left
2. LLM responses positioned to the right of prompts
3. Branches extend further to the right
4. Debate nodes in a separate column on the far right
5. No overlap between trees

## Color Palette

Each model is assigned a color from a predefined palette:

| Index | Color | Hex |
|--------|-------|-----|
| 0 | Violet | `#7c5cff` |
| 1 | Cyan | `#22d3ee` |
| 2 | Amber | `#f59e0b` |
| 3 | Emerald | `#10b981` |
| 4 | Rose | `#f43f5e` |
| 5 | Purple | `#8b5cf6` |
| 6 | Teal | `#06b6d4` |
| 7 | Pink | `#ec4899` |
| 8 | Lime | `#84cc16` |
| 9 | Blue | `#3b82f6` |

Colors cycle for models beyond the first 10.

## Minimap Navigation

The canvas includes a minimap for navigation:

- **Scale**: 2% of actual canvas size
- **Nodes**: Rendered as colored rectangles
- **Viewport**: Shows current view area
- **Click**: Pans to clicked node location

## Export to Note

Canvas sessions can be exported as markdown notes:

```markdown
# Canvas: My Session

*Canvas ID: `abc123`*
*Created: 2026-01-11 14:00*

---

## Prompt 1
> What is the meaning of life?

### GPT-4
The meaning of life is a philosophical question...

### Claude
This is one of humanity's oldest questions...

## Debates

### Debate 1 (auto mode)
*Participants: GPT-4, Claude*

#### Round 1
**GPT-4:**
I agree with Claude's assessment, but would add...

**Claude:**
GPT-4 makes a good point about...
```

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|----------|-------------|
| `OPENROUTER_API_KEY` | - | OpenRouter API key (required) |
| `CANVAS_DATA_PATH` | `../data/canvas` | Canvas session storage |
| `APP_URL` | `http://localhost:8080` | App URL for OpenRouter |

### Rate Limits

| Endpoint | Limit | Description |
|----------|--------|-------------|
| `POST /api/canvas/{id}/prompt` | 20/min | Send prompts |
| `POST /api/canvas/{id}/debate` | 10/min | Start debates |
| `GET /api/canvas/models/available` | 30/min | List models |

## File Structure

```
backend/app/
├── models/
│   └── canvas.py              # Canvas Pydantic models
├── routers/
│   └── canvas.py              # Canvas API endpoints (842 lines)
├── services/
│   ├── canvas_store.py         # Session persistence (838 lines)
│   └── openrouter.py         # OpenRouter API client (193 lines)
└── config.py                 # Canvas configuration

frontend/src/
├── views/
│   └── CanvasView.vue         # Session list and canvas container
├── stores/
│   └── canvas.js             # Pinia store (633 lines)
├── components/canvas/
│   ├── CanvasContainer.vue    # Main canvas component (1146 lines)
│   ├── PromptNode.vue        # Prompt tile node
│   ├── LLMNode.vue          # Individual LLM response node
│   ├── DebateNode.vue        # Debate tile node
│   ├── PromptDialog.vue      # New prompt dialog
│   ├── PromptTile.vue       # Legacy tile component
│   ├── ModelResponseCard.vue
│   ├── DebateTile.vue
│   ├── DebateControls.vue
│   └── ModelSelector.vue
└── api/
    └── client.js            # Canvas API client methods
```

## Usage Flow

### Creating a Canvas

1. Navigate to `/canvas`
2. Click "+ New" in sidebar
3. Enter title and optional description
4. Canvas is created and ready for prompts

### Sending a Prompt

1. Click "+ New Prompt" button
2. Select models from dropdown (grouped by provider)
3. Enter prompt text
4. Optionally add system prompt
5. Choose context mode for branching
6. Click "Send"
7. Responses stream in real-time

### Branching a Conversation

1. Click "Branch" button on any LLM response node
2. Enter new prompt
3. Optionally change models
4. Context is automatically included based on mode

### Starting a Debate

1. Select 2+ LLM response nodes (click to select)
2. Click "Debate (N)" button
3. Choose debate mode (auto/mediated)
4. Models critique each other's responses
5. Results displayed in debate node

### Exporting to Note

1. Click "Save as Note" button
2. Canvas content formatted as markdown
3. Note created in vault with `canvas-export` tag
4. Linked to canvas session for future updates

## Performance Considerations

### Streaming Optimization

- Content updates use optimistic UI updates
- Backend saves session only after all streams complete
- Debounced position updates during drag

### Memory Management

- Canvas sessions stored as JSON files (not in memory)
- Model list cached for 5 minutes
- HTTP client connection pooling

### Scalability

- Each canvas session is independent file
- No in-memory session limit
- Vector search indexed for semantic context retrieval
