# Grafyn feature map

Grafyn is a local-first desktop app. A person writes markdown notes, searches and links them, compares models on a canvas, imports chats and documents, and reviews twin evidence. The same vault is also exposed to MCP clients through the `grafyn-mcp` binary. This map is the user-facing surface and the invariants a change has to keep.

Data lives in two directories the user picks (defaults `~/Documents/Grafyn/vault` for notes and a `data/` directory for indexes, canvas sessions, and twin files). Verification must use a fresh temp pair and leave the real directories untouched.

## Notes (`/`)

The home screen is the daily vault.

- Header: search (`Search notes...`), **Canvas**, **Twin Review**, Settings, the guide, and **+ New Note**.
- The left sidebar lists notes. Choosing one opens the editor overlay. Save and Delete live there. Delete asks for confirmation.
- A new note goes through the topic selector, then the editor. Status is `draft`, `evidence`, or `canonical`.
- The graph view draws `[[wikilinks]]`. A typed link is written `[[Target Title]] (supports)`. The nine relations are `related`, `supports`, `contradicts`, `expands`, `questions`, `answers`, `example`, `part_of`, and `untyped` (a bare wikilink).
- Settings is where the vault path, OpenRouter key, Ollama, and the Claude Desktop MCP snippet are changed. **Import Content** in Settings opens `/import`.

Invariants:

- A note is a markdown file with YAML frontmatter (`note_id`, `title`, `status`, `tags`, timestamps). The body is the part the user typed.
- Writes land through an atomic temp-file rename. A finished save leaves no `*.tmp-*` file beside the note.
- Status strings outside `draft`, `evidence`, and `canonical` are rejected and do not change the file.
- Wikilink targets resolve by note title. `[[Harbor Note]] (supports)` is an outgoing `supports` edge. The backlink stored on the target uses the reverse relation: `supports` reverses to `related`, `questions` reverses to `answers`, `contradicts` stays `contradicts`.
- A wikilink inside a fenced or inline code span does not create an edge, even when a note with that title exists.
- Full-text search (Tantivy) and graph-aware recall see a note after it is saved, and stop returning it after it is deleted. The markdown file is gone too.
- One process owns the search writer. A second opener (the desktop app, or another `grafyn-mcp`) searches read-only and reports that results may be stale until reindex.

## Import (`/import`)

Settings → **Import Content**. The same path exists as the MCP tool `import_conversation`.

- ChatGPT, Claude, Grok, and Gemini exports are detected from their JSON shape. A labeled transcript (`Interviewer:` / `Participant:`) is detected from speaker labels. DOCX and PDF are split into section notes.
- Imported notes are status `evidence`. Provenance is written on the file: `source`, `source_id`, `created_via`, and `migration_source`.

The live check uses a tiny ChatGPT `conversations.json`. It does not need a network call.

## Canvas (`/canvas`, `/canvas/:id`)

- Empty state: **Create your first canvas**. A session has a title, a prompt, and one response node per model.
- The user can branch, add models, regenerate, pin notes, and start a debate. A chair model writes the debate recap. Follow-ups hang off that recap.
- Vault context is retrieval over the notes above. Pinned notes stay in the prompt. `context_mode=none` skips retrieval.
- Sessions are stored under the data directory, and a session can be exported back into the vault as a note.
- Web search is a per-prompt flag. Smart web search is a local hint about the prompt text.

Live model calls spend an OpenRouter or Ollama account. The verification run does not send one. Canvas behavior is covered by frontend unit tests and, on a machine that can compile Tauri, `cargo test`.

## Twin (`/twin`)

The workspace title is **Twin Workspace**. Tabs:

| Group | Tabs |
| --- | --- |
| Home | Overview |
| Work | Decisions |
| Review | Memory Review, Constitution, Action Gaps |
| Configure | Setup, Config |
| Help | Guide |

Header actions: **Run Records**, **Run Constitution**. Notes and Canvas link back to the other surfaces.

Invariants (enforced in the Rust twin store, not by the MCP note tools):

- Records marked `rejected`, `private`, or `no_train` stay out of live answers. Rejected records remain exportable negative evidence.
- Candidate records may be used only when they are relevant, and the prompt discloses them as tentative.
- Simulation mode requires a Twin Identity (name and role). Advisor mode is the decision-support default.
- The app records and exports twin evidence. It does not score accuracy or show an eval dashboard.
- An export bundle is JSONL the user trains on elsewhere.

A change to those rules needs the Rust tests. The MCP live check does not open `/twin`.

## MCP (`grafyn-mcp`)

Claude Desktop launches this binary over stdio. Settings generates the snippet: `grafyn-mcp --vault <path> --data <path>`.

Tools the server advertises:

`list_notes`, `get_note`, `create_note`, `update_note`, `delete_note`, `search_notes`, `get_backlinks`, `get_outgoing`, `recall_relevant`, `search_chunks`, `import_conversation`.

`search_chunks` and chunk-level `recall_relevant` need the chunk index the desktop app builds. A vault that has only been touched by `grafyn-mcp` answers `search_chunks` with an error that the chunk index is not available. Note-level search, recall, and the link graph still work.

The protocol is newline-delimited JSON-RPC. The server's current protocol version is `2025-03-26`. Logs go to stderr. Stdout is the protocol. `get_info()` leaves `serverInfo` at the rmcp default (`rmcp 0.15.0`). The instructions text is what says this process is the Grafyn knowledge base.

## What the live check covers

The script in this skill drives a real `grafyn-mcp` process through the note, search, graph, import, status, delete, and read-only-index flows above. Canvas streaming, the Twin Workspace, and the Vue shell need the extra suites named in `SKILL.md`.
