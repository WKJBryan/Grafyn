//! MCP tool definitions for Grafyn.
//!
//! Parameter schemas and `CallToolResult` payloads live here. Vault
//! operations are [`crate::headless::Headless`] and return domain types;
//! these handlers only map those results onto the existing MCP JSON.

use crate::headless::{
    ChunkQueryResult, DeletedNote, Headless, ImportedNotes, RecallOutcome, StoredNote,
};
use crate::models::memory::RecallResult;
use crate::models::note::{ChunkResult, Note, NoteMeta, RelationType, SearchResult};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// MCP server: a headless vault plus the rmcp tool router.
#[derive(Clone)]
pub struct GrafynMcpServer {
    core: Headless,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<String>,
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
        message: Cow::from(format!("JSON serialization failed: {e}")),
        data: None,
    })?;
    text_result(json)
}

fn err_result(msg: String) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::error(vec![Content::text(msg)]))
}

fn note_response(stored: StoredNote) -> NoteResponse {
    NoteResponse {
        id: stored.note.id,
        title: stored.note.title,
        status: stored.note.status.to_string(),
        tags: stored.note.tags,
        content: stored.note.content,
        warning: stored.warning,
    }
}

fn note_body_response(note: Note) -> NoteResponse {
    NoteResponse {
        id: note.id,
        title: note.title,
        status: note.status.to_string(),
        tags: note.tags,
        content: note.content,
        warning: None,
    }
}

fn meta_response(note: NoteMeta) -> NoteMetaResponse {
    NoteMetaResponse {
        id: note.id,
        title: note.title,
        status: note.status.to_string(),
        tags: note.tags,
    }
}

fn typed_meta_response(note: NoteMeta, relation: RelationType) -> TypedNoteMetaResponse {
    TypedNoteMetaResponse {
        id: note.id,
        title: note.title,
        status: note.status.to_string(),
        tags: note.tags,
        relation: relation.to_string(),
    }
}

fn search_payload(result: &SearchResult) -> serde_json::Value {
    serde_json::json!({
        "id": result.note.id,
        "title": result.note.title,
        "score": result.score,
        "snippet": result.snippet,
        "status": result.note.status.to_string(),
        "tags": result.note.tags,
    })
}

fn recall_payload(result: &RecallResult) -> serde_json::Value {
    serde_json::json!({
        "note_id": result.note_id,
        "title": result.title,
        "snippet": result.snippet,
        "score": result.score,
        "graph_boost": result.graph_boost,
        "total_score": result.total_score,
        "tags": result.tags,
    })
}

fn chunk_payload(chunk: &ChunkResult) -> serde_json::Value {
    serde_json::json!({
        "parent_note_id": chunk.parent_note_id,
        "parent_title": chunk.parent_title,
        "text": chunk.text,
        "score": chunk.search_score,
        "token_estimate": chunk.token_estimate,
    })
}

fn delete_message(deleted: &DeletedNote) -> String {
    let mut msg = format!("Note '{}' deleted successfully.", deleted.id);
    if let Some(warning) = &deleted.warning {
        msg.push_str(&format!(" Warning: {warning}"));
    }
    msg
}

fn import_payload(imported: &ImportedNotes) -> serde_json::Value {
    serde_json::json!({
        "platform": imported.platform,
        "imported": imported.note_ids.len(),
        "note_ids": imported.note_ids,
        "errors": imported.errors,
        "warning": imported.warning,
    })
}

fn chunk_query_payload(found: &ChunkQueryResult) -> serde_json::Value {
    serde_json::json!({
        "chunks": found.chunks.iter().map(chunk_payload).collect::<Vec<_>>(),
        "total_tokens": found.total_tokens,
        "token_budget": found.token_budget,
    })
}

// ── Tool implementations ─────────────────────────────────────────────────────

#[tool_router]
impl GrafynMcpServer {
    pub fn new(core: Headless) -> Self {
        Self {
            core,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "List all notes in the knowledge base with metadata (title, status, tags). Returns JSON array sorted by last updated."
    )]
    async fn list_notes(&self) -> Result<CallToolResult, McpError> {
        match self.core.list_notes().await {
            Ok(notes) => json_result(&notes.into_iter().map(meta_response).collect::<Vec<_>>()),
            Err(e) => err_result(e),
        }
    }

    #[tool(
        description = "Get the full content of a note by ID. Returns title, status, tags, and markdown content including wikilinks."
    )]
    async fn get_note(
        &self,
        Parameters(params): Parameters<GetNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        match self.core.get_note(&params.id).await {
            Ok(note) => json_result(&note_body_response(note)),
            Err(e) => err_result(e),
        }
    }

    #[tool(
        description = "Create a new note. ID is auto-generated from title. Content supports [[wikilinks]]. Returns the created note."
    )]
    async fn create_note(
        &self,
        Parameters(params): Parameters<CreateNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        match self
            .core
            .create_note(params.title, params.content, params.tags, &params.status)
            .await
        {
            Ok(stored) => json_result(&note_response(stored)),
            Err(e) => err_result(e),
        }
    }

    #[tool(
        description = "Update an existing note by ID. Only provided fields are changed (title, content, tags, status). Returns the updated note."
    )]
    async fn update_note(
        &self,
        Parameters(params): Parameters<UpdateNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        match self
            .core
            .update_note(
                &params.id,
                params.title,
                params.content,
                params.tags,
                params.status.as_deref(),
            )
            .await
        {
            Ok(stored) => json_result(&note_response(stored)),
            Err(e) => err_result(e),
        }
    }

    #[tool(
        description = "Delete a note by ID. This permanently removes the markdown file. Returns confirmation."
    )]
    async fn delete_note(
        &self,
        Parameters(params): Parameters<DeleteNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        match self.core.delete_note(&params.id).await {
            Ok(deleted) => text_result(delete_message(&deleted)),
            Err(e) => err_result(e),
        }
    }

    #[tool(
        description = "Full-text search across all notes. Searches titles and content. Returns matching notes with relevance scores and snippets."
    )]
    async fn search_notes(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        match self.core.search_notes(&params.query, params.limit).await {
            Ok(results) => json_result(&results.iter().map(search_payload).collect::<Vec<_>>()),
            Err(e) => err_result(e),
        }
    }

    #[tool(
        description = "Get all notes that link TO a specific note (backlinks) with relationship types. Shows which notes reference this one via [[wikilinks]] and how they relate (supports, contradicts, expands, etc.)."
    )]
    async fn get_backlinks(
        &self,
        Parameters(params): Parameters<BacklinksParams>,
    ) -> Result<CallToolResult, McpError> {
        let backlinks = self.core.get_backlinks(&params.note_id).await;
        json_result(
            &backlinks
                .into_iter()
                .map(|(note, relation)| typed_meta_response(note, relation))
                .collect::<Vec<_>>(),
        )
    }

    #[tool(
        description = "Get all notes that a specific note links FROM (outgoing links) with relationship types. Shows what [[wikilinks]] exist in the note's content and the relationship type (supports, contradicts, expands, etc.)."
    )]
    async fn get_outgoing(
        &self,
        Parameters(params): Parameters<OutgoingParams>,
    ) -> Result<CallToolResult, McpError> {
        let outgoing = self.core.get_outgoing(&params.note_id).await;
        json_result(
            &outgoing
                .into_iter()
                .map(|(note, relation)| typed_meta_response(note, relation))
                .collect::<Vec<_>>(),
        )
    }

    #[tool(
        description = "Import conversations, documents, or transcripts as evidence notes. Auto-detects known chat exports and splits documents into linked section notes. Returns created note IDs."
    )]
    async fn import_conversation(
        &self,
        Parameters(params): Parameters<ImportParams>,
    ) -> Result<CallToolResult, McpError> {
        match self
            .core
            .import_conversation(&params.file_path, &params.conversation_ids)
            .await
        {
            Ok(imported) => json_result(&import_payload(&imported)),
            Err(e) => err_result(e),
        }
    }

    #[tool(
        description = "Search with graph-aware boosting. When token_budget is set, returns relevant paragraphs (chunks) within that budget using the full retrieval pipeline. Without token_budget, returns note-level results with graph boosting."
    )]
    async fn recall_relevant(
        &self,
        Parameters(params): Parameters<RecallParams>,
    ) -> Result<CallToolResult, McpError> {
        match self
            .core
            .recall_relevant(
                &params.query,
                &params.context_note_ids,
                params.limit,
                params.token_budget,
            )
            .await
        {
            Ok(RecallOutcome::Chunks(chunks)) => {
                json_result(&chunks.iter().map(chunk_payload).collect::<Vec<_>>())
            }
            Ok(RecallOutcome::Notes(results)) => {
                json_result(&results.iter().map(recall_payload).collect::<Vec<_>>())
            }
            Err(e) => err_result(e),
        }
    }

    #[tool(
        description = "Search for relevant paragraphs across all notes with token budgeting. Returns the best-matching text chunks that fit within the token budget, scored with graph-aware boosting. Ideal for retrieving precise context without exceeding token limits."
    )]
    async fn search_chunks(
        &self,
        Parameters(params): Parameters<SearchChunksParams>,
    ) -> Result<CallToolResult, McpError> {
        match self
            .core
            .search_chunks(&params.query, params.token_budget, &params.context_note_ids)
            .await
        {
            Ok(found) => json_result(&chunk_query_payload(&found)),
            Err(e) => err_result(e),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_message_appends_the_index_warning() {
        assert_eq!(
            delete_message(&DeletedNote {
                id: "abc".into(),
                warning: None,
            }),
            "Note 'abc' deleted successfully."
        );
        assert_eq!(
            delete_message(&DeletedNote {
                id: "abc".into(),
                warning: Some(
                    "note deleted; search index held by the app — results may be stale until reindex"
                        .into(),
                ),
            }),
            "Note 'abc' deleted successfully. Warning: note deleted; search index held by the app — results may be stale until reindex"
        );
    }

    #[test]
    fn import_payload_keeps_null_warning_and_counts_ids() {
        let payload = import_payload(&ImportedNotes {
            platform: "chatgpt".into(),
            note_ids: vec!["a".into()],
            errors: Vec::new(),
            warning: None,
        });
        assert_eq!(payload["platform"], "chatgpt");
        assert_eq!(payload["imported"], 1);
        assert_eq!(payload["note_ids"][0], "a");
        assert!(payload["errors"].as_array().unwrap().is_empty());
        assert!(payload["warning"].is_null());
    }
}
