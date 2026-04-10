//! MCP Tool Definitions for Grafyn
//!
//! Implements 12 MCP tools that expose the knowledge base to Claude Desktop
//! and other MCP clients via the rmcp crate.

use crate::models::note::{NoteCreate, NoteStatus, NoteUpdate, CURRENT_NOTE_SCHEMA_VERSION};
use crate::services::chunk_index::ChunkIndex;
use crate::services::graph_index::GraphIndex;
use crate::services::import;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::memory::MemoryService;
use crate::services::priority::PriorityScoringService;
use crate::services::retrieval::RetrievalService;
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
    pub chunk_index: Option<Arc<RwLock<ChunkIndex>>>,
    pub retrieval_service: Arc<RwLock<RetrievalService>>,
    pub priority_service: Arc<RwLock<PriorityScoringService>>,
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
    #[schemars(
        description = "Token budget for chunk-level retrieval. When set, returns relevant paragraphs within this budget instead of whole notes."
    )]
    pub token_budget: Option<usize>,
}

fn default_recall_limit() -> usize {
    5
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchChunksParams {
    #[schemars(description = "Search query string")]
    pub query: String,
    #[schemars(
        description = "Token budget — returns best-matching paragraphs that fit within this limit (default: 4000)"
    )]
    #[serde(default = "default_token_budget")]
    pub token_budget: usize,
    #[schemars(description = "Note IDs to use as context for graph-boosted scoring")]
    #[serde(default)]
    pub context_note_ids: Vec<String>,
}

fn default_token_budget() -> usize {
    4000
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportParams {
    #[schemars(
        description = "Absolute path to a conversation export file (JSON or DMS). Supports ChatGPT, Claude, Grok, and Gemini formats."
    )]
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

#[derive(Serialize)]
struct TypedNoteMetaResponse {
    id: String,
    title: String,
    status: String,
    tags: Vec<String>,
    relation: String,
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
        chunk_index: Option<Arc<RwLock<ChunkIndex>>>,
        retrieval_service: Arc<RwLock<RetrievalService>>,
        priority_service: Arc<RwLock<PriorityScoringService>>,
    ) -> Self {
        Self {
            knowledge_store,
            search_service,
            graph_index,
            memory_service,
            chunk_index,
            retrieval_service,
            priority_service,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "List all notes in the knowledge base with metadata (title, status, tags). Returns JSON array sorted by last updated."
    )]
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

    #[tool(
        description = "Get the full content of a note by ID. Returns title, status, tags, and markdown content including wikilinks."
    )]
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

    #[tool(
        description = "Create a new note. ID is auto-generated from title. Content supports [[wikilinks]]. Returns the created note."
    )]
    async fn create_note(
        &self,
        Parameters(params): Parameters<CreateNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let status = params.status.parse().unwrap_or_default();
        let create = NoteCreate {
            title: params.title,
            content: params.content,
            relative_path: None,
            aliases: Vec::new(),
            status,
            tags: params.tags,
            schema_version: CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
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

    #[tool(
        description = "Update an existing note by ID. Only provided fields are changed (title, content, tags, status). Returns the updated note."
    )]
    async fn update_note(
        &self,
        Parameters(params): Parameters<UpdateNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = params.id.clone();
        let update = NoteUpdate {
            title: params.title,
            content: params.content,
            relative_path: None,
            aliases: None,
            status: params.status.map(|s: String| s.parse().unwrap_or_default()),
            tags: params.tags,
            schema_version: None,
            migration_source: None,
            optimizer_managed: None,
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

    #[tool(
        description = "Delete a note by ID. This permanently removes the markdown file. Returns confirmation."
    )]
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

    #[tool(
        description = "Full-text search across all notes. Searches titles and content. Returns matching notes with relevance scores and snippets."
    )]
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

    #[tool(
        description = "Get all notes that link TO a specific note (backlinks) with relationship types. Shows which notes reference this one via [[wikilinks]] and how they relate (supports, contradicts, expands, etc.)."
    )]
    async fn get_backlinks(
        &self,
        Parameters(params): Parameters<BacklinksParams>,
    ) -> Result<CallToolResult, McpError> {
        let graph = self.graph_index.read().await;
        let backlinks = graph.get_typed_backlinks(&params.note_id);
        let response: Vec<TypedNoteMetaResponse> = backlinks
            .into_iter()
            .map(|(n, relation)| TypedNoteMetaResponse {
                id: n.id,
                title: n.title,
                status: n.status.to_string(),
                tags: n.tags,
                relation: relation.to_string(),
            })
            .collect();
        json_result(&response)
    }

    #[tool(
        description = "Get all notes that a specific note links FROM (outgoing links) with relationship types. Shows what [[wikilinks]] exist in the note's content and the relationship type (supports, contradicts, expands, etc.)."
    )]
    async fn get_outgoing(
        &self,
        Parameters(params): Parameters<OutgoingParams>,
    ) -> Result<CallToolResult, McpError> {
        let graph = self.graph_index.read().await;
        let outgoing = graph.get_typed_outgoing(&params.note_id);
        let response: Vec<TypedNoteMetaResponse> = outgoing
            .into_iter()
            .map(|(n, relation)| TypedNoteMetaResponse {
                id: n.id,
                title: n.title,
                status: n.status.to_string(),
                tags: n.tags,
                relation: relation.to_string(),
            })
            .collect();
        json_result(&response)
    }

    #[tool(
        description = "Import conversations from ChatGPT, Claude, Grok, or Gemini export files as evidence notes. Auto-detects format. Returns created note IDs."
    )]
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
        let platform = import::detect_platform(&content).unwrap_or("unknown");
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
                relative_path: None,
                aliases: Vec::new(),
                status: NoteStatus::Evidence,
                tags,
                schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: Some("mcp-import".into()),
                optimizer_managed: false,
                properties: {
                    let mut props = std::collections::HashMap::new();
                    props.insert(
                        "source".into(),
                        serde_json::Value::String(conv.platform.clone()),
                    );
                    props.insert(
                        "source_id".into(),
                        serde_json::Value::String(conv.id.clone()),
                    );
                    props.insert(
                        "created_via".into(),
                        serde_json::Value::String("mcp-import".into()),
                    );
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

    #[tool(
        description = "Search with graph-aware boosting. When token_budget is set, returns relevant paragraphs (chunks) within that budget using the full retrieval pipeline. Without token_budget, returns note-level results with graph boosting."
    )]
    async fn recall_relevant(
        &self,
        Parameters(params): Parameters<RecallParams>,
    ) -> Result<CallToolResult, McpError> {
        // If token_budget is set and chunk index is available, use chunk retrieval
        if let (Some(budget), Some(chunk_index)) = (params.token_budget, &self.chunk_index) {
            let chunk_index = chunk_index.read().await;
            let graph = self.graph_index.read().await;
            let priority = self.priority_service.read().await;
            let retrieval = self.retrieval_service.read().await;

            match retrieval.retrieve_chunks(
                &chunk_index,
                &graph,
                &priority,
                &params.query,
                budget,
                &params.context_note_ids,
            ) {
                Ok(chunks) => {
                    let response: Vec<serde_json::Value> = chunks
                        .into_iter()
                        .map(|c| {
                            serde_json::json!({
                                "parent_note_id": c.parent_note_id,
                                "parent_title": c.parent_title,
                                "text": c.text,
                                "score": c.search_score,
                                "token_estimate": c.token_estimate,
                            })
                        })
                        .collect();
                    json_result(&response)
                }
                Err(e) => err_result(format!("Chunk recall failed: {}", e)),
            }
        } else {
            // Note-level recall (original behavior)
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

    #[tool(
        description = "Search for relevant paragraphs across all notes with token budgeting. Returns the best-matching text chunks that fit within the token budget, scored with graph-aware boosting. Ideal for retrieving precise context without exceeding token limits."
    )]
    async fn search_chunks(
        &self,
        Parameters(params): Parameters<SearchChunksParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(chunk_index) = &self.chunk_index else {
            return err_result(
                "Chunk index not available. Run the Grafyn app first to build it.".into(),
            );
        };

        let chunk_index = chunk_index.read().await;
        let graph = self.graph_index.read().await;
        let priority = self.priority_service.read().await;
        let retrieval = self.retrieval_service.read().await;

        match retrieval.retrieve_chunks(
            &chunk_index,
            &graph,
            &priority,
            &params.query,
            params.token_budget,
            &params.context_note_ids,
        ) {
            Ok(chunks) => {
                let total_tokens: usize = chunks.iter().map(|c| c.token_estimate).sum();
                let response = serde_json::json!({
                    "chunks": chunks.iter().map(|c| {
                        serde_json::json!({
                            "parent_note_id": c.parent_note_id,
                            "parent_title": c.parent_title,
                            "text": c.text,
                            "score": c.search_score,
                            "token_estimate": c.token_estimate,
                        })
                    }).collect::<Vec<_>>(),
                    "total_tokens": total_tokens,
                    "token_budget": params.token_budget,
                });
                json_result(&response)
            }
            Err(e) => err_result(format!("Chunk search failed: {}", e)),
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
                 Notes have statuses: draft, evidence, canonical. \
                 Use search_chunks for token-budgeted paragraph-level retrieval."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
