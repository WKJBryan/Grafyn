# Seedream Data Models - Backend

> **Part:** Backend | **Models:** 8 | **Scan Level:** Exhaustive

## Overview

All data models are defined as Pydantic schemas in `backend/app/models/note.py`.

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
  "content": "# Welcome to Seedream\n\nThis is your knowledge base...",
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
