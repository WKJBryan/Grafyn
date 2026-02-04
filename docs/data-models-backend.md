# Grafyn Data Models - Backend

> **Part:** Backend | **Models:** 23 | **Scan Level:** Exhaustive

## Overview

All data models are defined as Pydantic schemas in `backend/app/models/note.py` and `backend/app/models/canvas.py`.

---

## Core Models

### Note
Full note representation returned by GET endpoints.

```python
class Note(BaseModel):
    id: str                              # Filename without .md extension
    title: str                           # Display title
    content: str                         # Markdown content (body only)
    frontmatter: NoteFrontmatter         # YAML metadata
    outgoing_links: List[str] = []       # [[wikilinks]] targets
    backlinks: List[str] = []            # Notes linking to this
```

**Example:**
```json
{
  "id": "Welcome",
  "title": "Welcome",
  "content": "# Welcome to Grafyn\n\nThis is your knowledge base...",
  "frontmatter": {
    "title": "Welcome",
    "created": "2024-12-17T00:00:00",
    "modified": "2024-12-17T00:00:00",
    "tags": ["welcome", "getting-started"],
    "status": "draft",
    "aliases": []
  },
  "outgoing_links": ["Example Note", "Wikilinks"],
  "backlinks": ["Example_Note"]
}
```

---

### NoteFrontmatter
YAML frontmatter metadata extracted from note files.

```python
class NoteFrontmatter(BaseModel):
    title: Optional[str] = None          # Note title
    created: Optional[datetime] = None   # Creation date
    modified: Optional[datetime] = None  # Last modified date
    tags: List[str] = []                 # Searchable tags
    status: str = "draft"                # draft | evidence | canonical
    aliases: List[str] = []              # Alternative titles for linking
```

**Status Workflow:**
```
draft → evidence → canonical
  ↑         ↑          ↑
  │         │          └── Verified, authoritative content
  │         └── Ingested content (e.g., chat transcripts)
  └── Proposed content awaiting review
```

---

### NoteCreate
Schema for creating new notes (POST /api/notes).

```python
class NoteCreate(BaseModel):
    title: str                           # Required - generates file ID
    content: str                         # Required - Markdown body
    tags: List[str] = []                 # Optional tags
    status: str = "draft"                # Initial status
```

**ID Generation:** `title.replace(" ", "_")` → filename

---

### NoteUpdate
Schema for updating notes (PUT /api/notes/{id}).

```python
class NoteUpdate(BaseModel):
    title: Optional[str] = None          # New title (optional)
    content: Optional[str] = None        # New content (optional)
    tags: Optional[List[str]] = None     # New tags (optional)
    status: Optional[str] = None         # New status (optional)
```

**Note:** All fields are optional - only provided fields are updated.

---

### NoteListItem
Lightweight note representation for list responses.

```python
class NoteListItem(BaseModel):
    id: str                              # Note ID
    title: str                           # Display title
    status: str                          # Current status
    tags: List[str]                      # All tags
    created: Optional[datetime] = None   # Creation date
    modified: Optional[datetime] = None  # Last modified
    link_count: int = 0                  # Number of outgoing [[wikilinks]]
```

---

### SearchResult
Search result with relevance score.

```python
class SearchResult(BaseModel):
    note_id: str                         # Note ID
    title: str                           # Note title
    snippet: str                         # Excerpt around match
    score: float                         # Similarity (0-1, higher = more similar)
    tags: List[str] = []                 # Note tags
```

**Score Interpretation:**
- `1.0` = Exact lexical match
- `0.8+` = Very similar semantically
- `0.5-0.8` = Related content
- `<0.5` = Weak match

---

### BacklinkInfo
Information about a backlink with context.

```python
class BacklinkInfo(BaseModel):
    source_id: str                       # ID of note containing the link
    source_title: str                    # Title of source note
    context: str                         # Text surrounding the [[link]]
```

**Context Extraction:** ±100 characters around the wikilink.

---

## Vector Storage Schema

### NoteEmbedding (LanceDB)
Schema for storing note embeddings in LanceDB.

```python
class NoteEmbedding(LanceModel):
    note_id: str                         # Note ID (primary key)
    title: str                           # Note title
    text: str                            # First 1000 chars for snippet
    vector: Vector(384)                  # all-MiniLM-L6-v2 embedding
```

**Vector Details:**
- **Dimension:** 384 (fixed by model)
- **Model:** `sentence-transformers/all-MiniLM-L6-v2`
- **Input:** Concatenated `{title}\n\n{content}`

---

## Entity Relationships

```
┌─────────────────┐         ┌─────────────────┐
│     Note        │◄────────│  BacklinkInfo   │
│                 │         │                 │
│  - id           │         │  - source_id    │
│  - title        │         │  - source_title │
│  - content      │         │  - context      │
│  - frontmatter  │         └─────────────────┘
│  - outgoing     │
│  - backlinks    │         ┌─────────────────┐
├─────────────────┤         │  SearchResult   │
│ NoteFrontmatter │         │                 │
│                 │         │  - note_id      │
│  - title        │         │  - title        │
│  - created      │         │  - snippet      │
│  - modified     │         │  - score        │
│  - tags         │         │  - tags         │
│  - status       │         └─────────────────┘
│  - aliases      │
└─────────────────┘         ┌─────────────────┐
                            │  NoteEmbedding  │
┌─────────────────┐         │  (LanceDB)      │
│   NoteCreate    │         │                 │
│                 │         │  - note_id      │
│  - title        │         │  - title        │
│  - content      │         │  - text         │
│  - tags         │         │  - vector[384]  │
│  - status       │         └─────────────────┘
└─────────────────┘
```

---

## File Storage Format

Notes are stored as Markdown files with YAML frontmatter:

```markdown
---
title: Example Note
created: 2024-12-17
modified: 2024-12-17
tags:
  - example
  - documentation
status: canonical
aliases:
  - Sample Note
---

# Example Note

This is the content of the note. You can use [[wikilinks]] to link
to other notes like [[Welcome]].
```

**Parsing:** `python-frontmatter` library for reading/writing.

---

## Canvas Models

### CanvasSession
Complete canvas session state.

```python
class CanvasSession(BaseModel):
    id: str                              # Session UUID
    title: str                            # Session title
    description: Optional[str] = None         # Optional description
    prompt_tiles: List[PromptTile] = []    # All prompt tiles
    debates: List[DebateRound] = []       # All debate rounds
    viewport: CanvasViewport                 # Pan/zoom state
    created_at: datetime                   # Creation timestamp
    updated_at: datetime                   # Last modified
    tags: List[str] = []                  # Session tags
    status: str = "draft"                 # draft | evidence | canonical
    linked_note_id: Optional[str] = None     # Linked markdown note
```

### CanvasSessionListItem
Lightweight session representation for list views.

```python
class CanvasSessionListItem(BaseModel):
    id: str                              # Session UUID
    title: str                            # Session title
    description: Optional[str] = None         # Optional description
    tile_count: int = 0                   # Number of prompt tiles
    debate_count: int = 0                  # Number of debates
    created_at: datetime                   # Creation timestamp
    updated_at: datetime                   # Last modified
    tags: List[str] = []                  # Session tags
    status: str = "draft"                 # Session status
```

### PromptTile
A prompt node and its responses from multiple models.

```python
class PromptTile(BaseModel):
    id: str                              # Tile UUID
    prompt: str                           # User's input text
    system_prompt: Optional[str] = None      # Optional system instructions
    models: List[str]                      # List of model IDs
    responses: Dict[str, ModelResponse]      # model_id -> response
    position: TilePosition                   # Canvas coordinates
    created_at: datetime                   # Creation timestamp
    parent_tile_id: Optional[str] = None     # For branching
    parent_model_id: Optional[str] = None    # Which model's response
```

### ModelResponse
Response from a single LLM model.

```python
class ModelResponse(BaseModel):
    id: str                              # Response UUID
    model_id: str                         # OpenRouter model ID
    model_name: str                        # Display name
    content: str = ""                      # Response text
    status: str = "pending"                # pending | streaming | completed | error
    error_message: Optional[str] = None       # Error details if failed
    created_at: datetime                   # Creation timestamp
    completed_at: Optional[datetime] = None   # Completion timestamp
    tokens_used: Optional[int] = None         # Token count
    position: TilePosition                   # Canvas coordinates
    color: str = "#7c5cff"               # Visual color
```

### DebateRound
A debate node and its rounds between models.

```python
class DebateRound(BaseModel):
    id: str                              # Debate UUID
    participating_models: List[str]          # Models in debate
    debate_mode: DebateMode = DebateMode.AUTO # auto | mediated
    source_tile_ids: List[str]             # Which prompt responses
    rounds: List[Dict[str, str]] = []    # Round data
    status: str = "active"                # active | paused | completed
    created_at: datetime                   # Creation timestamp
    position: TilePosition                   # Canvas coordinates
```

### TilePosition
Position and size of a tile/node on the canvas.

```python
class TilePosition(BaseModel):
    x: float = 0.0                        # X coordinate
    y: float = 0.0                        # Y coordinate
    width: float = 280.0                    # Width in pixels
    height: float = 200.0                    # Height in pixels
```

### CanvasViewport
Canvas viewport state for persistence.

```python
class CanvasViewport(BaseModel):
    x: float = 0.0                        # Pan X offset
    y: float = 0.0                        # Pan Y offset
    zoom: float = 1.0                       # Zoom level
```

### DebateMode
Debate mode options.

```python
class DebateMode(str, Enum):
    AUTO = "auto"              # Automatic multi-round debate
    MEDIATED = "mediated"      # User-controlled single rounds
```

### ContextMode
Context mode for branched prompts.

```python
class ContextMode(str, Enum):
    FULL_HISTORY = "full_history"      # Walk parent chain, all turns
    COMPACT = "compact"                # Recent + summary
    SEMANTIC = "semantic"              # RAG-style search
```

### NodeType
Node types in the canvas graph.

```python
class NodeType(str, Enum):
    PROMPT = "prompt"                # Prompt node
    LLM_RESPONSE = "llm_response"    # LLM response node
    DEBATE = "debate"                # Debate node
```

### EdgeType
Types of edges in the canvas graph.

```python
class EdgeType(str, Enum):
    PROMPT_TO_LLM = "prompt_to_llm"      # Prompt → LLM response
    LLM_TO_PROMPT = "llm_to_prompt"      # LLM → Branch prompt
    DEBATE_TO_LLM = "debate_to_llm"      # Debate → LLMs
```

### NodeEdge
Edge connecting any two nodes in the canvas graph.

```python
class NodeEdge(BaseModel):
    source_id: str                       # Format: "prompt:{id}" or "llm:{tile_id}:{model_id}" or "debate:{id}"
    target_id: str                       # Target node ID
    edge_type: EdgeType = EdgeType.PROMPT_TO_LLM
    color: Optional[str] = None           # Optional styling
```

### CanvasCreate
Schema for creating a new canvas session.

```python
class CanvasCreate(BaseModel):
    title: str = "Untitled Canvas"      # Session title
    description: Optional[str] = None     # Optional description
    tags: List[str] = []                # Session tags
```

### CanvasUpdate
Schema for updating canvas metadata.

```python
class CanvasUpdate(BaseModel):
    title: Optional[str] = None         # New title
    description: Optional[str] = None     # New description
    viewport: Optional[CanvasViewport] = None  # New viewport
    tags: Optional[List[str]] = None     # New tags
    status: Optional[str] = None         # New status (draft|evidence|canonical)
```

### PromptRequest
Request to send a prompt to multiple models.

```python
class PromptRequest(BaseModel):
    prompt: str                          # Required: user's input
    system_prompt: Optional[str] = None     # Optional: system instructions
    models: List[str]                     # Required: model IDs
    temperature: float = 0.7              # 0.0 - 2.0
    max_tokens: int = 2048                # 1 - 32000
    parent_tile_id: Optional[str] = None     # For branching
    parent_model_id: Optional[str] = None    # Which model's response
    context_mode: ContextMode = ContextMode.FULL_HISTORY
```

### DebateStartRequest
Request to start a debate between models.

```python
class DebateStartRequest(BaseModel):
    source_tile_ids: List[str]             # Required: prompt tiles to debate
    participating_models: List[str]        # Required: 2+ models
    debate_mode: DebateMode = DebateMode.AUTO
    debate_prompt: Optional[str] = None      # Custom debate prompt
    max_rounds: int = 3                   # 1 - 10 rounds
```

### DebateContinueRequest
Request to continue a debate with custom prompt.

```python
class DebateContinueRequest(BaseModel):
    prompt: str                          # Required: user's instruction
```

### ModelInfo
Information about an available model from OpenRouter.

```python
class ModelInfo(BaseModel):
    id: str                              # Model ID (e.g., "openai/gpt-4")
    name: str                             # Display name
    provider: str                          # Provider name
    context_length: int = 4096              # Max token context
    pricing: Dict[str, float] = {}          # Pricing info
    supports_streaming: bool = True        # Streaming support
```

### TilePositionUpdate
Update for tile position.

```python
class TilePositionUpdate(BaseModel):
    x: float                              # New X coordinate
    y: float                              # New Y coordinate
    width: Optional[float] = None           # Optional new width
    height: Optional[float] = None          # Optional new height
```

### ArrangeRequest
Request to batch update node positions after auto-arrange.

```python
class ArrangeRequest(BaseModel):
    positions: Dict[str, TilePositionUpdate]  # node_id -> new position
```

---

## Canvas Entity Relationships

```
┌─────────────────┐         ┌─────────────────┐         ┌─────────────────┐
│ CanvasSession │◄────────│  PromptTile    │◄────────│ ModelResponse  │
│               │         │                 │         │                 │
│  - id         │         │  - id           │         │  - id           │
│  - title       │         │  - prompt       │         │  - model_id     │
│  - viewport    │         │  - models       │         │  - content      │
│  - tags        │         │  - responses    │         │  - status       │
│  - status      │         │  - position     │         │  - position     │
│  - linked_note │         │  - parent_*     │         │  - color        │
└─────────────────┘         └─────────────────┘         └─────────────────┘
       │                          │                          │
       │                          └──────────────────────────┘
       │
       ▼
┌─────────────────┐         ┌─────────────────┐
│  DebateRound  │◄────────│  NodeEdge       │
│               │         │                 │
│  - id         │         │  - source_id    │
│  - models     │         │  - target_id    │
│  - mode       │         │  - edge_type    │
│  - rounds     │         │  - color        │
│  - status     │         └─────────────────┘
└─────────────────┘
```

---

## Canvas Storage Format

Canvas sessions are stored as JSON files in `data/canvas/`:

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "title": "My Canvas Session",
  "description": "Exploring AI model comparisons",
  "prompt_tiles": [
    {
      "id": "tile-1",
      "prompt": "What is the meaning of life?",
      "models": ["openai/gpt-4", "anthropic/claude-3-opus"],
      "responses": {
        "openai/gpt-4": {
          "id": "resp-1",
          "model_id": "openai/gpt-4",
          "model_name": "GPT-4",
          "content": "The meaning of life...",
          "status": "completed",
          "created_at": "2026-01-11T14:00:00Z",
          "completed_at": "2026-01-11T14:00:05Z",
          "position": {"x": 300, "y": 50, "width": 280, "height": 200},
          "color": "#7c5cff"
        },
        "anthropic/claude-3-opus": {
          "id": "resp-2",
          "model_id": "anthropic/claude-3-opus",
          "model_name": "Claude 3 Opus",
          "content": "This is one of humanity's...",
          "status": "completed",
          "created_at": "2026-01-11T14:00:00Z",
          "completed_at": "2026-01-11T14:00:06Z",
          "position": {"x": 300, "y": 280, "width": 280, "height": 200},
          "color": "#22d3ee"
        }
      },
      "position": {"x": 50, "y": 50, "width": 200, "height": 120},
      "created_at": "2026-01-11T14:00:00Z"
    }
  ],
  "debates": [
    {
      "id": "debate-1",
      "participating_models": ["openai/gpt-4", "anthropic/claude-3-opus"],
      "debate_mode": "auto",
      "source_tile_ids": ["tile-1"],
      "rounds": [
        {
          "openai/gpt-4": "I agree with Claude's assessment...",
          "anthropic/claude-3-opus": "GPT-4 makes a good point..."
        }
      ],
      "status": "completed",
      "created_at": "2026-01-11T14:05:00Z",
      "position": {"x": 700, "y": 50, "width": 600, "height": 400}
    }
  ],
  "viewport": {"x": 0, "y": 0, "zoom": 1},
  "created_at": "2026-01-11T14:00:00Z",
  "updated_at": "2026-01-11T14:05:00Z",
  "tags": ["ai-comparison", "philosophy"],
  "status": "draft"
}
```

**Parsing:** JSON files loaded by [`CanvasSessionStore`](./architecture-backend.md#6-canvassessionstore-servicescanvas_storepy).
