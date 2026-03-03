//! MCP Tool Definitions for Grafyn
//!
//! Implements 10 MCP tools that expose the knowledge base to Claude Desktop
//! and other MCP clients via the rmcp crate.

use crate::models::note::{NoteCreate, NoteStatus, NoteUpdate};
use crate::services::graph_index::GraphIndex;
use crate::services::import;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::memory::MemoryService;
use crate::services::search::SearchService;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared state for the MCP server, holding all services needed by tools.
#[derive(Clone)]
pub struct GrafynMcpServer {
    pub knowledge_store: Arc<RwLock<KnowledgeStore>>,
    pub search_service: Arc<RwLock<SearchService>>,
    pub graph_index: Arc<RwLock<GraphIndex>>,
    pub memory_service: Arc<RwLock<MemoryService>>,
    tool_router: ToolRouter<Self>,
}

// ── Tool parameter structs ───────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetNoteParams {
    #[schemars(description = "The note ID (slug format, e.g. 'my-note-title')")]
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateNoteParams {
    #[schemars(description = "Title for the new note")]
    pub title: String,
    #[schemars(description = "Markdown content (supports [[wikilinks]])")]
    pub content: String,
    #[schemars(description = "Tags for categorization")]
    #[serde(default)]
    pub tags: Vec<String>,
    #[schemars(description = "Note status: 'draft', 'evidence', or 'canonical'")]
    #[serde(default = "default_status")]
    pub status: String,
}

fn default_status() -> String {
    "draft".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateNoteParams {
    #[schemars(description = "The note ID to update")]
    pub id: String,
    #[schemars(description = "New title (optional)")]
    pub title: Option<String>,
    #[schemars(description = "New markdown content (optional)")]
    pub content: Option<String>,
    #[schemars(description = "New tags (optional)")]
    pub tags: Option<Vec<String>>,
    #[schemars(description = "New status (optional)")]
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteNoteParams {
    #[schemars(description = "The note ID to delete")]
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    #[schemars(description = "Search query string")]
    pub query: String,
    #[schemars(description = "Maximum number of results (default: 10)")]
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    10
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BacklinksParams {
    #[schemars(description = "The note ID to get backlinks for")]
    pub note_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OutgoingParams {
    #[schemars(description = "The note ID to get outgoing links for")]
    pub note_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecallParams {
    #[schemars(description = "Natural language query for memory recall")]
    pub query: String,
    #[schemars(description = "Note IDs to use as context for graph-boosted recall")]
    #[serde(default)]
    pub context_note_ids: Vec<String>,
    #[schemars(description = "Maximum results (default: 5)")]
    #[serde(default = "default_recall_limit")]
    pub limit: usize,
}

fn default_recall_limit() -> usize {
    5
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportParams {
    #[schemars(description = "Absolute path to a conversation export file (JSON or DMS). Supports ChatGPT, Claude, Grok, and Gemini formats.")]
    pub file_path: String,
    #[schemars(description = "IDs of specific conversations to import. Empty array imports all.")]
    #[serde(default)]
    pub conversation_ids: Vec<String>,
}

// ── Tool response helpers ────────────────────────────────────────────────────

#[derive(Serialize)]
struct NoteResponse {
    id: String,
    title: String,
    status: String,
    tags: Vec<String>,
    content: String,
}

#[derive(Serialize)]
struct NoteMetaResponse {
    id: String,
    title: String,
    status: String,
    tags: Vec<String>,
}

fn text_result(text: String) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn json_result<T: Serialize>(data: &T) -> Result<CallToolResult, McpError> {
    let json = serde_json::to_string_pretty(data).map_err(|e| McpError {
        code: rmcp::model::ErrorCode::INTERNAL_ERROR,
        message: Cow::from(format!("JSON serialization failed: {}", e)),
        data: None,
    })?;
    text_result(json)
}

fn err_result(msg: String) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![Content::text(msg)]))
}

// ── Tool implementations ─────────────────────────────────────────────────────

#[tool_router]
impl GrafynMcpServer {
    pub fn new(
        knowledge_store: Arc<RwLock<KnowledgeStore>>,
        search_service: Arc<RwLock<SearchService>>,
        graph_index: Arc<RwLock<GraphIndex>>,
        memory_service: Arc<RwLock<MemoryService>>,
    ) -> Self {
        Self {
            knowledge_store,
            search_service,
            graph_index,
            memory_service,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List all notes in the knowledge base with metadata (title, status, tags). Returns JSON array sorted by last updated.")]
    async fn list_notes(&self) -> Result<CallToolResult, McpError> {
        let ks = self.knowledge_store.read().await;
        match ks.list_notes() {
            Ok(notes) => {
                let response: Vec<NoteMetaResponse> = notes
                    .into_iter()
                    .map(|n| NoteMetaResponse {
                        id: n.id,
                        title: n.title,
                        status: n.status.to_string(),
                        tags: n.tags,
                    })
                    .collect();
                json_result(&response)
            }
            Err(e) => err_result(format!("Failed to list notes: {}", e)),
        }
    }

    #[tool(description = "Get the full content of a note by ID. Returns title, status, tags, and markdown content including wikilinks.")]
    async fn get_note(
        &self,
        Parameters(params): Parameters<GetNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let ks = self.knowledge_store.read().await;
        match ks.get_note(&params.id) {
            Ok(note) => json_result(&NoteResponse {
                id: note.id,
                title: note.title,
                status: note.status.to_string(),
                tags: note.tags,
                content: note.content,
            }),
            Err(e) => err_result(format!("Note not found: {}", e)),
        }
    }

    #[tool(description = "Create a new note. ID is auto-generated from title. Content supports [[wikilinks]]. Returns the created note.")]
    async fn create_note(
        &self,
        Parameters(params): Parameters<CreateNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let status = params.status.parse().unwrap_or_default();
        let create = NoteCreate {
            title: params.title,
            content: params.content,
            status,
            tags: params.tags,
            properties: Default::default(),
        };

        let mut ks = self.knowledge_store.write().await;
        match ks.create_note(create) {
            Ok(note) => {
                // Update search index (if writable)
                {
                    let mut search = self.search_service.write().await;
                    if !search.is_readonly() {
                        let _ = search.index_note(&note);
                        let _ = search.commit();
                    }
                }
                // Update graph index
                {
                    let mut graph = self.graph_index.write().await;
                    graph.update_note(&note);
                }

                json_result(&NoteResponse {
                    id: note.id,
                    title: note.title,
                    status: note.status.to_string(),
                    tags: note.tags,
                    content: note.content,
                })
            }
            Err(e) => err_result(format!("Failed to create note: {}", e)),
        }
    }

    #[tool(description = "Update an existing note by ID. Only provided fields are changed (title, content, tags, status). Returns the updated note.")]
    async fn update_note(
        &self,
        Parameters(params): Parameters<UpdateNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = params.id.clone();
        let update = NoteUpdate {
            title: params.title,
            content: params.content,
            status: params.status.map(|s: String| s.parse().unwrap_or_default()),
            tags: params.tags,
            properties: None,
        };

        let mut ks = self.knowledge_store.write().await;
        match ks.update_note(&id, update) {
            Ok(note) => {
                // Update search index (if writable)
                {
                    let mut search = self.search_service.write().await;
                    if !search.is_readonly() {
                        let _ = search.index_note(&note);
                        let _ = search.commit();
                    }
                }
                // Update graph index
                {
                    let mut graph = self.graph_index.write().await;
                    graph.update_note(&note);
                }

                json_result(&NoteResponse {
                    id: note.id,
                    title: note.title,
                    status: note.status.to_string(),
                    tags: note.tags,
                    content: note.content,
                })
            }
            Err(e) => err_result(format!("Failed to update note: {}", e)),
        }
    }

    #[tool(description = "Delete a note by ID. This permanently removes the markdown file. Returns confirmation.")]
    async fn delete_note(
        &self,
        Parameters(params): Parameters<DeleteNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let mut ks = self.knowledge_store.write().await;
        match ks.delete_note(&params.id) {
            Ok(()) => {
                // Update search index (if writable)
                {
                    let mut search = self.search_service.write().await;
                    if !search.is_readonly() {
                        let _ = search.remove_note(&params.id);
                        let _ = search.commit();
                    }
                }
                // Update graph index
                {
                    let mut graph = self.graph_index.write().await;
                    graph.remove_note(&params.id);
                }

                text_result(format!("Note '{}' deleted successfully.", params.id))
            }
            Err(e) => err_result(format!("Failed to delete note: {}", e)),
        }
    }

    #[tool(description = "Full-text search across all notes. Searches titles and content. Returns matching notes with relevance scores and snippets.")]
    async fn search_notes(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let search = self.search_service.read().await;
        match search.search(&params.query, params.limit) {
            Ok(results) => {
                let response: Vec<serde_json::Value> = results
                    .into_iter()
                    .map(|r| {
                        serde_json::json!({
                            "id": r.note.id,
                            "title": r.note.title,
                            "score": r.score,
                            "snippet": r.snippet,
                            "status": r.note.status.to_string(),
                            "tags": r.note.tags,
                        })
                    })
                    .collect();
                json_result(&response)
            }
            Err(e) => err_result(format!("Search failed: {}", e)),
        }
    }

    #[tool(description = "Get all notes that link TO a specific note (backlinks). Shows which notes reference this one via [[wikilinks]].")]
    async fn get_backlinks(
        &self,
        Parameters(params): Parameters<BacklinksParams>,
    ) -> Result<CallToolResult, McpError> {
        let graph = self.graph_index.read().await;
        let backlinks = graph.get_backlinks(&params.note_id);
        let response: Vec<NoteMetaResponse> = backlinks
            .into_iter()
            .map(|n| NoteMetaResponse {
                id: n.id,
                title: n.title,
                status: n.status.to_string(),
                tags: n.tags,
            })
            .collect();
        json_result(&response)
    }

    #[tool(description = "Get all notes that a specific note links FROM (outgoing links). Shows what [[wikilinks]] exist in the note's content.")]
    async fn get_outgoing(
        &self,
        Parameters(params): Parameters<OutgoingParams>,
    ) -> Result<CallToolResult, McpError> {
        let graph = self.graph_index.read().await;
        let outgoing = graph.get_outgoing(&params.note_id);
        let response: Vec<NoteMetaResponse> = outgoing
            .into_iter()
            .map(|n| NoteMetaResponse {
                id: n.id,
                title: n.title,
                status: n.status.to_string(),
                tags: n.tags,
            })
            .collect();
        json_result(&response)
    }

    #[tool(description = "Import conversations from ChatGPT, Claude, Grok, or Gemini export files as evidence notes. Auto-detects format. Returns created note IDs.")]
    async fn import_conversation(
        &self,
        Parameters(params): Parameters<ImportParams>,
    ) -> Result<CallToolResult, McpError> {
        // Read the file
        let content = match std::fs::read_to_string(&params.file_path) {
            Ok(c) => c,
            Err(e) => return err_result(format!("Failed to read file: {}", e)),
        };

        // Auto-detect and parse
        let platform = import::detect_platform(&content)
            .unwrap_or("unknown");
        let all_conversations = match import::parse_content(&content) {
            Ok(c) => c,
            Err(e) => return err_result(format!("Failed to parse: {}", e)),
        };

        // Filter to selected conversations
        let to_import = if params.conversation_ids.is_empty() {
            all_conversations
        } else {
            all_conversations
                .into_iter()
                .filter(|c| params.conversation_ids.contains(&c.id))
                .collect()
        };

        let mut created_ids = Vec::new();
        let mut errors = Vec::new();

        for conv in &to_import {
            let markdown = import::format_as_markdown(conv);
            let mut tags = conv.suggested_tags.clone();
            if !tags.contains(&"import".to_string()) {
                tags.push("import".to_string());
            }
            tags.truncate(5);

            let note_create = NoteCreate {
                title: conv.title.clone(),
                content: markdown,
                status: NoteStatus::Evidence,
                tags,
                properties: {
                    let mut props = std::collections::HashMap::new();
                    props.insert("source".into(), serde_json::Value::String(conv.platform.clone()));
                    props.insert("source_id".into(), serde_json::Value::String(conv.id.clone()));
                    props.insert("created_via".into(), serde_json::Value::String("mcp-import".into()));
                    props
                },
            };

            let mut ks = self.knowledge_store.write().await;
            match ks.create_note(note_create) {
                Ok(note) => {
                    // Index in search (if writable)
                    {
                        let mut search = self.search_service.write().await;
                        if !search.is_readonly() {
                            let _ = search.index_note(&note);
                            let _ = search.commit();
                        }
                    }
                    // Update graph
                    {
                        let mut graph = self.graph_index.write().await;
                        graph.update_note(&note);
                    }
                    created_ids.push(note.id);
                }
                Err(e) => {
                    errors.push(format!("Failed to create '{}': {}", conv.title, e));
                }
            }
        }

        let result = serde_json::json!({
            "platform": platform,
            "imported": created_ids.len(),
            "note_ids": created_ids,
            "errors": errors,
        });
        json_result(&result)
    }

    #[tool(description = "Search with graph-aware boosting. Notes connected to context notes via wikilinks get a relevance boost. Best for finding related knowledge.")]
    async fn recall_relevant(
        &self,
        Parameters(params): Parameters<RecallParams>,
    ) -> Result<CallToolResult, McpError> {
        let search = self.search_service.read().await;
        let graph = self.graph_index.read().await;
        let memory = self.memory_service.read().await;

        match memory.recall_relevant(
            &search,
            &graph,
            &params.query,
            &params.context_note_ids,
            params.limit,
        ) {
            Ok(results) => {
                let response: Vec<serde_json::Value> = results
                    .into_iter()
                    .map(|r| {
                        serde_json::json!({
                            "note_id": r.note_id,
                            "title": r.title,
                            "snippet": r.snippet,
                            "score": r.score,
                            "graph_boost": r.graph_boost,
                            "total_score": r.total_score,
                            "tags": r.tags,
                        })
                    })
                    .collect();
                json_result(&response)
            }
            Err(e) => err_result(format!("Recall failed: {}", e)),
        }
    }
}

#[tool_handler]
impl ServerHandler for GrafynMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Grafyn knowledge base server. Use tools to search, browse, and manage \
                 markdown notes with [[wikilinks]], tags, and a graph of connections. \
                 Notes have statuses: draft, evidence, canonical."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
