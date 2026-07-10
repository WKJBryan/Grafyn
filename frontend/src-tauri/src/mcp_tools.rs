//! MCP Tool Definitions for Grafyn
//!
//! Implements 12 MCP tools that expose the knowledge base to Claude Desktop
//! and other MCP clients via the rmcp crate.

use crate::models::note::{NoteCreate, NoteStatus, NoteUpdate, CURRENT_NOTE_SCHEMA_VERSION};
use crate::services::chunk_index::ChunkIndex;
use crate::services::graph_index::GraphIndex;
use crate::services::import;
use crate::services::index_commit;
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
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use zip::ZipArchive;

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
        description = "Absolute path to a content file. Supports ChatGPT, Claude, Grok, Gemini, Markdown, TXT, DOCX, PDF, and labeled transcripts."
    )]
    pub file_path: String,
    #[schemars(description = "IDs of specific content items to import. Empty array imports all.")]
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

fn read_mcp_import_content(file_path: &str) -> Result<String, String> {
    let extension = Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if extension == "docx" {
        let bytes = std::fs::read(file_path).map_err(|e| e.to_string())?;
        return extract_mcp_docx_text(&bytes);
    }

    if extension == "pdf" {
        let text = pdf_extract::extract_text(file_path).map_err(|e| e.to_string())?;
        let text = text.trim().to_string();
        return if text.is_empty() {
            Err("PDF did not contain readable text".to_string())
        } else {
            Ok(text)
        };
    }

    std::fs::read_to_string(file_path).map_err(|e| e.to_string())
}

fn extract_mcp_docx_text(bytes: &[u8]) -> Result<String, String> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        ZipArchive::new(cursor).map_err(|e| format!("Failed to open DOCX archive: {}", e))?;
    let mut document = archive
        .by_name("word/document.xml")
        .map_err(|e| format!("Failed to find DOCX document text: {}", e))?;
    let mut xml = String::new();
    document
        .read_to_string(&mut xml)
        .map_err(|e| format!("Failed to read DOCX document text: {}", e))?;

    let mut reader = quick_xml::Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut text = String::new();
    let mut in_text_run = false;

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(event)) => match event.name().as_ref() {
                b"w:t" => in_text_run = true,
                b"w:tab" => text.push('\t'),
                b"w:br" => text.push('\n'),
                _ => {}
            },
            Ok(quick_xml::events::Event::Empty(event)) => match event.name().as_ref() {
                b"w:tab" => text.push('\t'),
                b"w:br" => text.push('\n'),
                _ => {}
            },
            Ok(quick_xml::events::Event::Text(event)) if in_text_run => {
                let decoded = event
                    .xml_content()
                    .map_err(|e| format!("Failed to decode DOCX text: {}", e))?;
                text.push_str(&decoded);
            }
            Ok(quick_xml::events::Event::End(event)) => match event.name().as_ref() {
                b"w:t" => in_text_run = false,
                b"w:p" => {
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                }
                _ => {}
            },
            Ok(quick_xml::events::Event::Eof) => break,
            Err(e) => return Err(format!("Failed to parse DOCX document text: {}", e)),
            _ => {}
        }
    }

    let content = text.trim().to_string();
    if content.is_empty() {
        Err("DOCX did not contain readable text".to_string())
    } else {
        Ok(content)
    }
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
                    let _ = index_commit::index_note_for_search(&mut search, &note);
                    let _ = index_commit::commit_search(&mut search);
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
                    let _ = index_commit::index_note_for_search(&mut search, &note);
                    let _ = index_commit::commit_search(&mut search);
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
                    let _ = index_commit::remove_note_for_search(&mut search, &params.id);
                    let _ = index_commit::commit_search(&mut search);
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
        description = "Import conversations, documents, or transcripts as evidence notes. Auto-detects known chat exports and splits documents into linked section notes. Returns created note IDs."
    )]
    async fn import_conversation(
        &self,
        Parameters(params): Parameters<ImportParams>,
    ) -> Result<CallToolResult, McpError> {
        let content = match read_mcp_import_content(&params.file_path) {
            Ok(c) => c,
            Err(e) => return err_result(format!("Failed to read file: {}", e)),
        };

        let path = Path::new(&params.file_path);
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&params.file_path);
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();

        let (platform, items) = if let Some(platform) = import::detect_platform(&content) {
            let all_conversations = match import::parse_content(&content) {
                Ok(c) => c,
                Err(e) => return err_result(format!("Failed to parse: {}", e)),
            };
            let to_import = if params.conversation_ids.is_empty() {
                all_conversations
            } else {
                all_conversations
                    .into_iter()
                    .filter(|c| params.conversation_ids.contains(&c.id))
                    .collect()
            };
            let items = to_import
                .iter()
                .map(|conv| {
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
                    let mut tags = conv.suggested_tags.clone();
                    if !tags.contains(&"import".to_string()) {
                        tags.push("import".to_string());
                    }
                    tags.truncate(5);
                    (
                        conv.title.clone(),
                        import::format_as_markdown(conv),
                        tags,
                        props,
                    )
                })
                .collect::<Vec<_>>();
            (platform.to_string(), items)
        } else {
            let batch = match import::document::parse_document_text(file_name, extension, &content)
            {
                Ok(batch) => batch,
                Err(e) => return err_result(format!("Failed to parse content: {}", e)),
            };
            let to_import = if params.conversation_ids.is_empty() {
                batch.items
            } else {
                batch
                    .items
                    .into_iter()
                    .filter(|item| params.conversation_ids.contains(&item.id))
                    .collect()
            };
            let items = to_import
                .into_iter()
                .map(|item| {
                    let mut props = item.metadata;
                    props.insert(
                        "source_id".into(),
                        serde_json::Value::String(item.id.clone()),
                    );
                    (item.title, item.content, item.suggested_tags, props)
                })
                .collect::<Vec<_>>();
            ("document".to_string(), items)
        };

        let mut created_ids = Vec::new();
        let mut errors = Vec::new();

        for (title, content, tags, properties) in items {
            let note_create = NoteCreate {
                title: title.clone(),
                content,
                relative_path: None,
                aliases: Vec::new(),
                status: NoteStatus::Evidence,
                tags,
                schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: Some("mcp-import".into()),
                optimizer_managed: false,
                properties,
            };

            let mut ks = self.knowledge_store.write().await;
            match ks.create_note(note_create) {
                Ok(note) => {
                    // Index in search (if writable)
                    {
                        let mut search = self.search_service.write().await;
                        let _ = index_commit::index_note_for_search(&mut search, &note);
                        let _ = index_commit::commit_search(&mut search);
                    }
                    // Update graph
                    {
                        let mut graph = self.graph_index.write().await;
                        graph.update_note(&note);
                    }
                    created_ids.push(note.id);
                }
                Err(e) => {
                    errors.push(format!("Failed to create '{}': {}", title, e));
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
