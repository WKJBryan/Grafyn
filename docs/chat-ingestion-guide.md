# OrgAI Chat Ingestion Guide

> **Purpose:** How to save conversations from ChatGPT, Claude, and other AI assistants into your OrgAI knowledge base

## Overview

OrgAI provides multiple pathways to ingest AI conversations:

```
┌─────────────────────────────────────────────────────────────────────┐
│                      Chat Ingestion Methods                          │
│                                                                      │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐               │
│  │   Method 1   │  │   Method 2   │  │   Method 3   │               │
│  │   MCP Live   │  │  REST API    │  │   Scripts    │               │
│  │  (Claude)    │  │  (Manual)    │  │  (Bulk)      │               │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘               │
│         │                 │                 │                        │
│         ▼                 ▼                 ▼                        │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                    OrgAI Backend                             │    │
│  │                   (/api/notes + /mcp)                        │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                              │                                       │
│                              ▼                                       │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │                      vault/*.md                              │    │
│  │                    (status: evidence)                        │    │
│  └─────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Method 1: MCP Live Integration (Claude Desktop)

### What is MCP?

The **Model Context Protocol (MCP)** allows AI assistants to interact with external tools. OrgAI exposes 6 MCP tools including `ingest_chat` for saving conversations.

### Setup Claude Desktop

1. **Locate Claude's config file:**
   - **Windows:** `%APPDATA%\Claude\claude_desktop_config.json`
   - **Mac:** `~/Library/Application Support/Claude/claude_desktop_config.json`

2. **Add OrgAI as an MCP server:**

```json
{
  "mcpServers": {
    "orgai": {
      "url": "http://localhost:8080/mcp",
      "transport": "sse"
    }
  }
}
```

3. **Restart Claude Desktop**

4. **Verify connection:**
   - Open Claude Desktop
   - You should see OrgAI tools available (hammer icon)
   - Ask Claude: "What tools do you have from OrgAI?"

### Using ingest_chat in Claude

Once connected, Claude can automatically save conversations:

**Example prompts:**

> "Save this conversation to my knowledge base titled 'Discussion about Python async'"

> "Ingest this chat as evidence with tags: project-planning, architecture"

**Behind the scenes, Claude calls:**
```json
{
  "tool": "ingest_chat",
  "arguments": {
    "content": "[full conversation transcript]",
    "title": "Discussion about Python async",
    "source": "claude",
    "tags": ["programming", "python"]
  }
}
```

### Available MCP Tools

| Tool | Purpose | When to Use |
|------|---------|-------------|
| `ingest_chat` | Save conversation | Save the current chat as evidence |
| `create_draft` | Create draft note | AI generates a summary/analysis |
| `query_knowledge` | Search notes | Find related information |
| `get_note` | Read a note | Reference existing knowledge |
| `list_notes` | List notes | Browse available notes |
| `get_backlinks` | Find connections | See what links to a topic |

---

## Method 2: REST API (Manual/Copy-Paste)

### Quick Save via cURL

```bash
curl -X POST http://localhost:8080/api/notes \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Chat with ChatGPT - Project Planning",
    "content": "# Chat Transcript\n\n**Date:** 2024-12-21\n**Source:** ChatGPT\n\n---\n\n## User\nHow should I structure my microservices?\n\n## Assistant\nHere are my recommendations...",
    "tags": ["chat", "chatgpt", "architecture"],
    "status": "evidence"
  }'
```

### Using the Web UI

1. Open OrgAI at http://localhost:5173
2. Click **+ New Note**
3. Title: "Chat with [AI] - [Topic]"
4. Paste the conversation
5. Set status to `evidence` (optional, via API)

### Browser Bookmarklet

Create a bookmarklet to quickly open OrgAI with clipboard content:

```javascript
javascript:(function(){
  const content = prompt('Paste chat content:');
  const title = prompt('Title for this chat:');
  if(content && title) {
    fetch('http://localhost:8080/api/notes', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({
        title: title,
        content: '# ' + title + '\n\n' + content,
        tags: ['chat', 'imported'],
        status: 'evidence'
      })
    }).then(r => r.json()).then(d => alert('Saved as: ' + d.id));
  }
})();
```

---

## Method 3: Export Scripts

### Python Script: Ingest from Clipboard

```python
#!/usr/bin/env python3
"""
ingest_clipboard.py - Save clipboard content to OrgAI
"""

import requests
import pyperclip
from datetime import datetime

ORGAI_URL = "http://localhost:8080"

def ingest_chat(title: str = None, source: str = "clipboard", tags: list = None):
    """Ingest clipboard content as a chat note."""
    content = pyperclip.paste()
    
    if not content:
        print("❌ Clipboard is empty")
        return
    
    if not title:
        title = f"Chat Import - {datetime.now().strftime('%Y-%m-%d %H:%M')}"
    
    payload = {
        "title": title,
        "content": f"# {title}\n\n*Ingested from {source} on {datetime.now().isoformat()}*\n\n---\n\n{content}",
        "tags": tags or ["chat", "imported", source],
        "status": "evidence",
    }
    
    response = requests.post(f"{ORGAI_URL}/api/notes", json=payload)
    
    if response.ok:
        note = response.json()
        print(f"✓ Saved as: {note['id']}")
        return note
    else:
        print(f"❌ Error: {response.text}")
        return None

if __name__ == "__main__":
    import sys
    title = sys.argv[1] if len(sys.argv) > 1 else None
    ingest_chat(title)
```

**Usage:**
```bash
# Copy chat to clipboard, then:
python ingest_clipboard.py "Discussion about API design"
```

---

### Python Script: Bulk Import from Files

```python
#!/usr/bin/env python3
"""
bulk_import.py - Import multiple chat exports
"""

import os
import requests
from pathlib import Path
from datetime import datetime

ORGAI_URL = "http://localhost:8080"
IMPORT_DIR = "./chat_exports"  # Directory with .txt or .md files

def import_file(filepath: Path):
    """Import a single file as a chat note."""
    content = filepath.read_text(encoding="utf-8")
    title = filepath.stem.replace("_", " ").replace("-", " ").title()
    
    # Detect source from filename
    source = "unknown"
    for s in ["claude", "chatgpt", "gemini", "copilot"]:
        if s in filepath.stem.lower():
            source = s
            break
    
    payload = {
        "title": title,
        "content": f"# {title}\n\n*Imported from {filepath.name}*\n\n---\n\n{content}",
        "tags": ["chat", "imported", source],
        "status": "evidence",
    }
    
    response = requests.post(f"{ORGAI_URL}/api/notes", json=payload)
    return response.ok, title

def bulk_import():
    """Import all files from IMPORT_DIR."""
    import_path = Path(IMPORT_DIR)
    
    if not import_path.exists():
        print(f"❌ Directory not found: {IMPORT_DIR}")
        return
    
    files = list(import_path.glob("*.txt")) + list(import_path.glob("*.md"))
    
    if not files:
        print(f"❌ No .txt or .md files found in {IMPORT_DIR}")
        return
    
    print(f"📂 Found {len(files)} files to import")
    
    success = 0
    for f in files:
        ok, title = import_file(f)
        if ok:
            print(f"  ✓ {title}")
            success += 1
        else:
            print(f"  ❌ Failed: {f.name}")
    
    print(f"\n✓ Imported {success}/{len(files)} files")
    
    # Reindex for search
    requests.post(f"{ORGAI_URL}/api/notes/reindex")
    print("✓ Reindexed for search")

if __name__ == "__main__":
    bulk_import()
```

**Usage:**
```bash
# Place chat exports in ./chat_exports/
python bulk_import.py
```

---

### JavaScript Script: Browser Console

For ChatGPT, you can run this in the browser console to export the current conversation:

```javascript
// Run in ChatGPT browser console
(async function exportToOrgAI() {
    // Extract messages
    const messages = document.querySelectorAll('[data-message-author-role]');
    let content = '# ChatGPT Export\n\n';
    
    messages.forEach(msg => {
        const role = msg.dataset.messageAuthorRole === 'user' ? '## User' : '## Assistant';
        const text = msg.querySelector('.markdown')?.innerText || msg.innerText;
        content += `${role}\n\n${text}\n\n---\n\n`;
    });
    
    const title = prompt('Title for this chat:') || `ChatGPT Export - ${new Date().toISOString().slice(0,10)}`;
    
    const response = await fetch('http://localhost:8080/api/notes', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
            title: title,
            content: content,
            tags: ['chat', 'chatgpt', 'exported'],
            status: 'evidence'
        })
    });
    
    if (response.ok) {
        const note = await response.json();
        console.log('✓ Saved as:', note.id);
        alert(`Saved as: ${note.id}`);
    } else {
        console.error('Failed:', await response.text());
    }
})();
```

**Note:** This requires CORS to be configured (already enabled in OrgAI).

---

## Chat Note Format

When chats are ingested, they're stored with this structure:

```markdown
---
title: Chat with Claude - API Design
created: 2024-12-21T10:30:00
modified: 2024-12-21T10:30:00
tags:
  - chat
  - claude
  - evidence
  - api-design
status: evidence
---

# Chat with Claude - API Design

*Ingested from claude on 2024-12-21T10:30:00*

---

## User

How should I design my REST API?

## Assistant

Here are my recommendations for REST API design...

## User

What about error handling?

## Assistant

For error handling, consider these patterns...
```

---

## Status Workflow for Chats

```
evidence → draft → canonical
    │         │         │
    │         │         └── Reviewed and verified
    │         └── Extracted key insights
    └── Raw chat transcript (AI-generated)
```

**Recommended workflow:**
1. Save chats as `evidence`
2. Review and extract key insights into `draft` notes
3. Verify and promote to `canonical` when confirmed

---

## Searching Chat Content

Once ingested, chats are searchable:

```bash
# Semantic search
curl "http://localhost:8080/api/search?q=REST%20API%20design"

# Filter by tag
# (via UI or custom query)
```

**Via MCP (ask Claude):**
> "Search my knowledge base for discussions about API design"

---

## Limitations & Future Enhancements

| Feature | Current Status | Notes |
|---------|----------------|-------|
| Claude MCP | ✅ Works | Requires Claude Desktop config |
| ChatGPT MCP | ❌ Not supported | ChatGPT doesn't support custom MCP |
| Gemini MCP | ⏳ Possible | Similar config to Claude |
| Auto-export | ❌ Not built | Would need browser extensions |
| Conversation threading | ❌ Flat | No follow-up linking |
| Duplicate detection | ❌ Not built | Manual check needed |

---

## Troubleshooting

### "Connection refused" from scripts
**Solution:** Ensure OrgAI backend is running on port 8080

### MCP tools not appearing in Claude
**Solution:** Check config path, restart Claude Desktop

### CORS errors in browser console
**Solution:** OrgAI allows all origins by default; check backend logs

### Note not searchable
**Solution:** Run reindex: `POST /api/notes/reindex`
