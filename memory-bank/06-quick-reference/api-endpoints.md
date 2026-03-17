# API Quick Reference

> **Purpose:** Current command surface summary for the desktop app
> **Status:** Current

## Scope

Grafyn no longer exposes a local HTTP backend for the app UI. The current frontend talks to Rust through Tauri IPC commands defined in `frontend/src-tauri/src/main.rs` and wrapped in `frontend/src/api/client.js`.

If you find older `/api/*`, OAuth, or `localhost:8080` examples in the Memory Bank, treat them as historical context only.

## Main Command Groups

The frontend wrapper exports these command groups:

- `boot`
- `notes`
- `search`
- `graph`
- `canvas`
- `feedback`
- `settings`
- `mcp`
- `priority`
- `memory`
- `zettelkasten`
- `retrieval`
- `importApi`

## Notes Commands

Implemented through the `notes` client wrapper:

- `list_notes`
- `get_note`
- `create_note`
- `update_note`
- `delete_note`
- `reindex`
- `distill_note`
- `normalize_tags`

## Search And Graph Commands

- `search_notes`
- `find_similar`
- `get_backlinks`
- `get_outgoing`
- `get_neighbors`
- `get_unlinked`
- `get_full_graph`
- `rebuild_graph`

## Canvas And LLM Commands

- `list_sessions`
- `get_session`
- `create_session`
- `update_session`
- `delete_session`
- `get_available_models`
- `send_prompt`
- `update_tile_position`
- `update_llm_node_position`
- `auto_arrange`
- `delete_tile`
- `delete_response`
- `update_viewport`
- `export_to_note`
- `start_debate`
- `continue_debate`
- `add_models_to_tile`
- `regenerate_response`

## Settings And Feedback Commands

- `get_settings`
- `get_settings_status`
- `update_settings`
- `complete_setup`
- `pick_vault_folder`
- `validate_openrouter_key`
- `get_openrouter_status`
- `submit_feedback`
- `feedback_status`
- `get_system_info`
- `get_pending_feedback`
- `retry_pending_feedback`
- `clear_pending_feedback`

## MCP, Retrieval, Import, And Misc

- `get_mcp_status`
- `get_mcp_config_snippet`
- `get_priority_settings`
- `update_priority_settings`
- `reset_priority_settings`
- `recall_relevant`
- `find_contradictions`
- `extract_claims`
- `discover_links`
- `apply_links`
- `create_link`
- `get_link_types`
- `preview_import`
- `apply_import`
- `get_supported_formats`
- `retrieve_relevant`
- `get_retrieval_config`
- `update_retrieval_config`

## Source Of Truth

When updating this file, verify against:

- `frontend/src/api/client.js`
- `frontend/src-tauri/src/main.rs`
- `frontend/src-tauri/src/commands/`

For developer workflow and runtime commands, prefer:

- `HOW_TO_RUN.md`
- `frontend/README.md`
- `frontend/TESTING.md`
