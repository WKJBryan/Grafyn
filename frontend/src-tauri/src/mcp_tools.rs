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
use crate::services::twin_events::{
    MutationCoordinator, MutationError,
};
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
    pub search_service: Option<Arc<RwLock<SearchService>>>,
    pub graph_index: Arc<RwLock<GraphIndex>>,
    pub memory_service: Arc<RwLock<MemoryService>>,
    pub chunk_index: Option<Arc<RwLock<ChunkIndex>>>,
    pub retrieval_service: Arc<RwLock<RetrievalService>>,
    pub priority_service: Arc<RwLock<PriorityScoringService>>,
    derived_ready: bool,
    derived_authority: DerivedAuthority,
    tool_router: ToolRouter<Self>,
}

#[derive(Clone)]
struct DerivedAuthority {
    coordinator: Arc<MutationCoordinator>,
    authoritative_token:
        Arc<std::sync::Mutex<crate::services::vault_namespace::VaultAuthorityTokenV1>>,
    derived_token: crate::services::vault_namespace::VaultAuthorityTokenV1,
}

struct McpAuthorityReadTicket {
    coordinator: Arc<MutationCoordinator>,
    expected: crate::services::vault_namespace::VaultAuthorityTokenV1,
    require_ready: bool,
}

impl McpAuthorityReadTicket {
    fn finish(self) -> Result<(), MutationError> {
        self.coordinator
            .validate_authority_token(&self.expected, self.require_ready)
    }
}

fn finish_mcp_read(
    ticket: McpAuthorityReadTicket,
    result: Result<CallToolResult, McpError>,
    unavailable: &str,
) -> Result<CallToolResult, McpError> {
    if ticket.finish().is_err() {
        return err_result(unavailable.to_string());
    }
    result
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
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_governed_derived_state(
        knowledge_store: Arc<RwLock<KnowledgeStore>>,
        search_service: Option<Arc<RwLock<SearchService>>>,
        graph_index: Arc<RwLock<GraphIndex>>,
        memory_service: Arc<RwLock<MemoryService>>,
        chunk_index: Option<Arc<RwLock<ChunkIndex>>>,
        retrieval_service: Arc<RwLock<RetrievalService>>,
        priority_service: Arc<RwLock<PriorityScoringService>>,
        coordinator: Arc<MutationCoordinator>,
        authoritative_token: crate::services::vault_namespace::VaultAuthorityTokenV1,
        derived_token: Option<crate::services::vault_namespace::VaultAuthorityTokenV1>,
    ) -> Result<Self, MutationError> {
        coordinator.validate_authority_token(&authoritative_token, false)?;
        if let Some(token) = &derived_token {
            coordinator.validate_authority_token(token, true)?;
            if token != &authoritative_token {
                return Err(MutationError::RecoveryConflict(
                    "MCP derived and authoritative startup tokens differ".into(),
                ));
            }
        }
        let derived_ready = derived_token.is_some();
        let admitted_derived_token = derived_token.unwrap_or_else(|| authoritative_token.clone());
        Ok(Self {
            knowledge_store,
            search_service,
            graph_index,
            memory_service,
            chunk_index,
            retrieval_service,
            priority_service,
            derived_ready,
            derived_authority: DerivedAuthority {
                coordinator,
                authoritative_token: Arc::new(std::sync::Mutex::new(authoritative_token)),
                derived_token: admitted_derived_token,
            },
            tool_router: Self::tool_router(),
        })
    }

    fn require_derived_ready(&self) -> Result<McpAuthorityReadTicket, CallToolResult> {
        if !self.derived_ready {
            return Err(err_result(
                "Vault-derived indexes are unavailable until Grafyn rebuilds this vault namespace."
                    .into(),
            )
            .expect("tool result construction is infallible"));
        }
        self.derived_authority
            .coordinator
            .validate_authority_token(&self.derived_authority.derived_token, true)
            .map_err(|_| {
                err_result(
                    "Vault-derived indexes are unavailable until Grafyn rebuilds this vault namespace."
                        .into(),
                )
                .expect("tool result construction is infallible")
            })?;
        Ok(McpAuthorityReadTicket {
            coordinator: self.derived_authority.coordinator.clone(),
            expected: self.derived_authority.derived_token.clone(),
            require_ready: true,
        })
    }

    fn begin_authoritative_read(&self) -> Result<McpAuthorityReadTicket, CallToolResult> {
        let expected = self
            .derived_authority
            .authoritative_token
            .lock()
            .map_err(|_| {
                err_result("Vault authority is unavailable until restart.".into())
                    .expect("tool result construction is infallible")
            })?
            .clone();
        self.derived_authority
            .coordinator
            .validate_authority_token(&expected, false)
            .map_err(|_| {
                err_result("Vault authority changed; restart this MCP server.".into())
                    .expect("tool result construction is infallible")
            })?;
        Ok(McpAuthorityReadTicket {
            coordinator: self.derived_authority.coordinator.clone(),
            expected,
            require_ready: false,
        })
    }

    fn refresh_authoritative_after_write(
        &self,
        store: &mut KnowledgeStore,
    ) -> Result<(), MutationError> {
        let token = self.derived_authority.coordinator.current_authority_token()?;
        store.reload_authoritative_state();
        self.derived_authority
            .coordinator
            .validate_authority_token(&token, false)?;
        *self
            .derived_authority
            .authoritative_token
            .lock()
            .map_err(|_| MutationError::Invalid("MCP authority token lock poisoned".into()))? =
            token;
        Ok(())
    }

    #[tool(
        description = "List all notes in the knowledge base with metadata (title, status, tags). Returns JSON array sorted by last updated."
    )]
    async fn list_notes(&self) -> Result<CallToolResult, McpError> {
        let ticket = match self.begin_authoritative_read() {
            Ok(ticket) => ticket,
            Err(result) => return Ok(result),
        };
        let result = {
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
        };
        finish_mcp_read(
            ticket,
            result,
            "Vault authority changed; restart this MCP server.",
        )
    }

    #[tool(
        description = "Get the full content of a note by ID. Returns title, status, tags, and markdown content including wikilinks."
    )]
    async fn get_note(
        &self,
        Parameters(params): Parameters<GetNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let ticket = match self.begin_authoritative_read() {
            Ok(ticket) => ticket,
            Err(result) => return Ok(result),
        };
        let result = {
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
        };
        finish_mcp_read(
            ticket,
            result,
            "Vault authority changed; restart this MCP server.",
        )
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
        match ks.create_note_from_source(create, "mcp") {
            Ok(note) => {
                if let Err(error) = self.refresh_authoritative_after_write(&mut ks) {
                    return err_result(format!(
                        "Note was committed, but MCP authority refresh failed: {error}"
                    ));
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
        match ks.update_note_from_source(&id, update, "mcp") {
            Ok(note) => {
                if let Err(error) = self.refresh_authoritative_after_write(&mut ks) {
                    return err_result(format!(
                        "Note was committed, but MCP authority refresh failed: {error}"
                    ));
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
        match ks.delete_note_from_source(&params.id, "mcp") {
            Ok(()) => {
                if let Err(error) = self.refresh_authoritative_after_write(&mut ks) {
                    return err_result(format!(
                        "Note was committed, but MCP authority refresh failed: {error}"
                    ));
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
        let ticket = match self.require_derived_ready() {
            Ok(ticket) => ticket,
            Err(result) => return Ok(result),
        };
        let Some(search_service) = &self.search_service else {
            return err_result("Vault-derived search index is unavailable.".into());
        };
        let result = {
            let search = search_service.read().await;
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
        };
        finish_mcp_read(
            ticket,
            result,
            "Vault-derived indexes changed while this search was running.",
        )
    }

    #[tool(
        description = "Get all notes that link TO a specific note (backlinks) with relationship types. Shows which notes reference this one via [[wikilinks]] and how they relate (supports, contradicts, expands, etc.)."
    )]
    async fn get_backlinks(
        &self,
        Parameters(params): Parameters<BacklinksParams>,
    ) -> Result<CallToolResult, McpError> {
        let ticket = match self.require_derived_ready() {
            Ok(ticket) => ticket,
            Err(result) => return Ok(result),
        };
        let result = {
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
        };
        finish_mcp_read(
            ticket,
            result,
            "Vault-derived indexes changed while backlinks were read.",
        )
    }

    #[tool(
        description = "Get all notes that a specific note links FROM (outgoing links) with relationship types. Shows what [[wikilinks]] exist in the note's content and the relationship type (supports, contradicts, expands, etc.)."
    )]
    async fn get_outgoing(
        &self,
        Parameters(params): Parameters<OutgoingParams>,
    ) -> Result<CallToolResult, McpError> {
        let ticket = match self.require_derived_ready() {
            Ok(ticket) => ticket,
            Err(result) => return Ok(result),
        };
        let result = {
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
        };
        finish_mcp_read(
            ticket,
            result,
            "Vault-derived indexes changed while outgoing links were read.",
        )
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

        let (platform, containers) = if let Some(platform) = import::detect_platform(&content) {
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
            let containers = to_import
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
                    let create = NoteCreate {
                        title: conv.title.clone(),
                        content: import::format_as_markdown(conv),
                        relative_path: None,
                        aliases: Vec::new(),
                        status: NoteStatus::Evidence,
                        tags,
                        schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                        migration_source: Some("mcp-import".into()),
                        optimizer_managed: false,
                        properties: props,
                    };
                    (conv.id.clone(), vec![create])
                })
                .collect::<Vec<_>>();
            (platform.to_string(), containers)
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
            let source_title = batch.source_title;
            let items = to_import
                .into_iter()
                .map(|item| {
                    let mut props = item.metadata;
                    props.insert(
                        "source_id".into(),
                        serde_json::Value::String(item.id.clone()),
                    );
                    NoteCreate {
                        title: item.title,
                        content: item.content,
                        relative_path: None,
                        aliases: Vec::new(),
                        status: NoteStatus::Evidence,
                        tags: item.suggested_tags,
                        schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                        migration_source: Some("mcp-import".into()),
                        optimizer_managed: false,
                        properties: props,
                    }
                })
                .collect::<Vec<_>>();
            ("document".to_string(), vec![(source_title, items)])
        };

        let mut created_notes = Vec::new();
        let mut errors = Vec::new();
        for (container_id, creates) in containers {
            if creates.is_empty() {
                continue;
            }
            let mut ks = self.knowledge_store.write().await;
            match ks.import_note_container(creates, &container_id, content.as_bytes()) {
                Ok(notes) => created_notes.extend(notes),
                Err(e) => {
                    errors.push(format!("Failed to import '{}': {}", container_id, e));
                }
            }
        }
        if !created_notes.is_empty() {
            let mut ks = self.knowledge_store.write().await;
            if let Err(error) = self.refresh_authoritative_after_write(&mut ks) {
                errors.push(format!(
                    "Notes were committed, but MCP authority refresh failed: {error}"
                ));
            }
        }
        let created_ids = created_notes
            .into_iter()
            .map(|note| note.id)
            .collect::<Vec<_>>();

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
        let ticket = match self.require_derived_ready() {
            Ok(ticket) => ticket,
            Err(result) => return Ok(result),
        };
        // If token_budget is set and chunk index is available, use chunk retrieval
        let result = if let (Some(budget), Some(chunk_index)) =
            (params.token_budget, &self.chunk_index)
        {
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
            let Some(search_service) = &self.search_service else {
                return err_result("Vault-derived search index is unavailable.".into());
            };
            let search = search_service.read().await;
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
        };
        finish_mcp_read(
            ticket,
            result,
            "Vault-derived indexes changed while recall was running.",
        )
    }

    #[tool(
        description = "Search for relevant paragraphs across all notes with token budgeting. Returns the best-matching text chunks that fit within the token budget, scored with graph-aware boosting. Ideal for retrieving precise context without exceeding token limits."
    )]
    async fn search_chunks(
        &self,
        Parameters(params): Parameters<SearchChunksParams>,
    ) -> Result<CallToolResult, McpError> {
        let ticket = match self.require_derived_ready() {
            Ok(ticket) => ticket,
            Err(result) => return Ok(result),
        };
        let Some(chunk_index) = &self.chunk_index else {
            return err_result(
                "Chunk index not available. Run the Grafyn app first to build it.".into(),
            );
        };

        let result = {
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
        };
        finish_mcp_read(
            ticket,
            result,
            "Vault-derived indexes changed while chunk search was running.",
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin_event::TwinEventPayload;
    use crate::services::twin_events::{
        MutationCoordinator, NoopMutationLifecycle, TwinEventStore,
    };
    use tempfile::tempdir;

    fn test_server(
        root: &Path,
    ) -> (
        GrafynMcpServer,
        Arc<TwinEventStore>,
        std::path::PathBuf,
        Arc<MutationCoordinator>,
    ) {
        let vault = root.join("vault");
        let data = root.join("data");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&data).unwrap();
        let events = Arc::new(TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = Arc::new(
            MutationCoordinator::new(
                &data,
                &vault,
                events.clone(),
                Arc::new(NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let namespace = coordinator.current_namespace_path().unwrap();
        let server = GrafynMcpServer::new_with_governed_derived_state(
            Arc::new(RwLock::new(KnowledgeStore::with_event_recorder(
                vault,
                namespace.clone(),
                coordinator.clone(),
            ))),
            Some(Arc::new(RwLock::new(
                SearchService::new(namespace).unwrap(),
            ))),
            Arc::new(RwLock::new(GraphIndex::new())),
            Arc::new(RwLock::new(MemoryService::new())),
            None,
            Arc::new(RwLock::new(RetrievalService::new(data.clone()))),
            Arc::new(RwLock::new(PriorityScoringService::new(data.clone()))),
            coordinator.clone(),
            coordinator.current_authority_token().unwrap(),
            Some(coordinator.current_authority_token().unwrap()),
        )
        .unwrap();
        (server, events, data, coordinator)
    }

    #[tokio::test]
    async fn mcp_crud_and_import_share_coordinated_groups_without_duplicates() {
        let root = tempdir().unwrap();
        let (server, events, data, _coordinator) = test_server(root.path());

        server
            .create_note(Parameters(CreateNoteParams {
                title: "MCP captured".into(),
                content: "first".into(),
                tags: Vec::new(),
                status: "draft".into(),
            }))
            .await
            .unwrap();
        server
            .update_note(Parameters(UpdateNoteParams {
                id: "mcp-captured".into(),
                title: None,
                content: Some("second".into()),
                tags: None,
                status: None,
            }))
            .await
            .unwrap();
        server
            .delete_note(Parameters(DeleteNoteParams {
                id: "mcp-captured".into(),
            }))
            .await
            .unwrap();

        let after_crud = events.ordered_events().unwrap();
        assert_eq!(after_crud.len(), 3);
        assert!(after_crud.iter().all(|event| {
            event.context.source_channel.as_str() == "mcp"
                && matches!(event.payload, TwinEventPayload::NoteChanged(_))
        }));

        let import_path = data.join("chatgpt.json");
        std::fs::write(
            &import_path,
            r#"[{"id":"conv1","title":"Imported Chat","create_time":1704067200,"mapping":{"msg1":{"message":{"content":{"parts":["Hello"]},"author":{"role":"user"},"create_time":1704067200}}}}]"#,
        )
        .unwrap();
        server
            .import_conversation(Parameters(ImportParams {
                file_path: import_path.to_string_lossy().into_owned(),
                conversation_ids: Vec::new(),
            }))
            .await
            .unwrap();

        let captured = events.ordered_events().unwrap();
        assert_eq!(captured.len(), 5);
        assert!(matches!(
            captured[3].payload,
            TwinEventPayload::NoteChanged(_)
        ));
        assert!(matches!(
            captured[4].payload,
            TwinEventPayload::ObservationRecorded(_)
        ));
        assert!(captured[3..]
            .iter()
            .all(|event| event.context.source_channel.as_str() == "import"));
        assert_eq!(captured[3].device_sequence + 1, captured[4].device_sequence);
        assert!(captured[4].causal_parents.contains(&captured[3].event_id));
    }

    #[tokio::test]
    async fn mismatched_namespace_allows_authoritative_notes_but_blocks_derived_tools() {
        let root = tempdir().unwrap();
        let (mut server, _events, _data, _coordinator) = test_server(root.path());
        server.derived_ready = false;
        server.search_service = None;

        server
            .create_note(Parameters(CreateNoteParams {
                title: "Still authoritative".into(),
                content: "durable bytes".into(),
                tags: Vec::new(),
                status: "draft".into(),
            }))
            .await
            .unwrap();
        let listed = server.list_notes().await.unwrap();
        assert!(format!("{listed:?}").contains("still-authoritative"));

        let search = server
            .search_notes(Parameters(SearchParams {
                query: "authoritative".into(),
                limit: 10,
            }))
            .await
            .unwrap();
        let backlinks = server
            .get_backlinks(Parameters(BacklinksParams {
                note_id: "still-authoritative".into(),
            }))
            .await
            .unwrap();
        let recall = server
            .recall_relevant(Parameters(RecallParams {
                query: "authoritative".into(),
                context_note_ids: Vec::new(),
                limit: 10,
                token_budget: None,
            }))
            .await
            .unwrap();
        for result in [search, backlinks, recall] {
            assert!(format!("{result:?}").contains("Vault-derived indexes are unavailable"));
        }
    }

    #[test]
    fn mcp_server_has_no_static_derived_readiness_constructor() {
        let source = include_str!("mcp_tools.rs");
        assert!(!source.contains("    pub fn new(\n"));
        assert!(source.contains("pub fn new_with_governed_derived_state("));
    }

    #[tokio::test]
    async fn running_mcp_rejects_every_derived_read_after_the_root_epoch_changes() {
        let root = tempdir().unwrap();
        let (server, _events, _data, coordinator) = test_server(root.path());
        server.require_derived_ready().unwrap();

        let next_vault = root.path().join("next-vault");
        std::fs::create_dir(&next_vault).unwrap();
        coordinator.retarget_markdown_root(&next_vault).unwrap();

        assert!(server.require_derived_ready().is_err());
        let results = [
            server
                .search_notes(Parameters(SearchParams {
                    query: "stale".into(),
                    limit: 10,
                }))
                .await
                .unwrap(),
            server
                .get_backlinks(Parameters(BacklinksParams {
                    note_id: "stale".into(),
                }))
                .await
                .unwrap(),
            server
                .get_outgoing(Parameters(OutgoingParams {
                    note_id: "stale".into(),
                }))
                .await
                .unwrap(),
            server
                .recall_relevant(Parameters(RecallParams {
                    query: "stale".into(),
                    context_note_ids: Vec::new(),
                    limit: 10,
                    token_budget: None,
                }))
                .await
                .unwrap(),
            server
                .search_chunks(Parameters(SearchChunksParams {
                    query: "stale".into(),
                    token_budget: 100,
                    context_note_ids: Vec::new(),
                }))
                .await
                .unwrap(),
        ];
        assert!(results
            .iter()
            .all(|result| format!("{result:?}").contains("Vault-derived indexes are unavailable")));
    }

    #[test]
    fn mcp_constructor_rejects_a_stale_preconstruction_ready_token() {
        let root = tempdir().unwrap();
        let vault = root.path().join("vault");
        let data = root.path().join("data");
        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(&data).unwrap();
        let events = Arc::new(TwinEventStore::new(&data));
        events.initialize().unwrap();
        let coordinator = Arc::new(
            MutationCoordinator::new(
                &data,
                &vault,
                events,
                Arc::new(NoopMutationLifecycle),
            )
            .unwrap(),
        );
        let namespace = coordinator.current_namespace_path().unwrap();
        let stale = coordinator.current_authority_token().unwrap();
        let mut knowledge = KnowledgeStore::with_event_recorder(
            vault.clone(),
            namespace.clone(),
            coordinator.clone(),
        );
        knowledge
            .create_note_from_source(
                NoteCreate {
                    title: "Peer write".into(),
                    content: "new generation".into(),
                    relative_path: None,
                    aliases: Vec::new(),
                    status: NoteStatus::Draft,
                    tags: Vec::new(),
                    schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                    migration_source: None,
                    optimizer_managed: false,
                    properties: Default::default(),
                },
                "mcp",
            )
            .unwrap();

        let result = GrafynMcpServer::new_with_governed_derived_state(
            Arc::new(RwLock::new(knowledge)),
            None,
            Arc::new(RwLock::new(GraphIndex::new())),
            Arc::new(RwLock::new(MemoryService::new())),
            None,
            Arc::new(RwLock::new(RetrievalService::new(data.clone()))),
            Arc::new(RwLock::new(PriorityScoringService::new(data))),
            coordinator,
            stale.clone(),
            Some(stale),
        );

        assert!(matches!(result, Err(MutationError::RecoveryConflict(_))));
    }

    #[tokio::test]
    async fn authoritative_list_is_optimistically_fenced_and_local_writes_refresh_its_token() {
        let root = tempdir().unwrap();
        let (server, _events, _data, coordinator) = test_server(root.path());

        server
            .create_note(Parameters(CreateNoteParams {
                title: "Local authority".into(),
                content: "durable".into(),
                tags: Vec::new(),
                status: "draft".into(),
            }))
            .await
            .unwrap();
        let listed = server.list_notes().await.unwrap();
        assert!(format!("{listed:?}").contains("local-authority"));

        coordinator
            .commit_local(
                crate::models::twin_event::CausalStream::LocalOnly,
                crate::models::twin_event::SourceChannel::parse("mcp").unwrap(),
                vec![crate::services::twin_events::TargetMutation::put(
                    crate::services::twin_events::TargetKind::Markdown,
                    "peer.md",
                    "peer",
                )],
                Vec::new(),
            )
            .unwrap();
        let stale = server.list_notes().await.unwrap();
        assert!(format!("{stale:?}").contains("Vault authority changed"));
    }
}
