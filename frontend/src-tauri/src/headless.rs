//! Shared vault core for headless Grafyn processes.
//!
//! [`Headless::open`] creates the vault and data directories and opens the
//! same services `grafyn-mcp` opens today: knowledge store, search (writable,
//! or read-only when the writer lock is held), an in-memory graph built from
//! the notes on disk, memory, retrieval, priority, and a read-only chunk index
//! when one is present. Path resolution stays with the caller. This opener
//! does not walk the working directory, does not invent a `.grafyn` data
//! directory, and does not call `reindex_all`.
//!
//! Operations return domain types. MCP JSON shaping stays in `mcp_tools`.

use crate::models::memory::RecallResult;
use crate::models::note::{
    ChunkResult, Note, NoteCreate, NoteMeta, NoteStatus, NoteUpdate, RelationType, SearchResult,
    CURRENT_NOTE_SCHEMA_VERSION,
};
use crate::services::chunk_index::ChunkIndex;
use crate::services::graph_index::GraphIndex;
use crate::services::import;
use crate::services::index_commit;
use crate::services::knowledge_store::KnowledgeStore;
use crate::services::memory::MemoryService;
use crate::services::priority::PriorityScoringService;
use crate::services::retrieval::{retain_resolvable_chunks, RetrievalService};
use crate::services::search::SearchService;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio::sync::RwLock;
use walkdir::WalkDir;
use zip::ZipArchive;

/// A note that reached disk, plus the search-index warning when the write
/// could not be committed.
#[derive(Debug, Clone)]
pub struct StoredNote {
    pub note: Note,
    pub warning: Option<String>,
}

/// A note removed from the vault, plus the search-index warning when the
/// delete could not be committed.
#[derive(Debug, Clone)]
pub struct DeletedNote {
    pub id: String,
    pub warning: Option<String>,
}

/// Notes created from one import file.
#[derive(Debug, Clone)]
pub struct ImportedNotes {
    pub platform: String,
    pub note_ids: Vec<String>,
    pub errors: Vec<String>,
    pub warning: Option<String>,
}

/// Paragraphs that fit a token budget.
#[derive(Debug, Clone)]
pub struct ChunkQueryResult {
    pub chunks: Vec<ChunkResult>,
    pub total_tokens: usize,
    pub token_budget: usize,
}

/// Graph-aware recall: paragraphs when a token budget and chunk index are
/// both available, otherwise whole notes.
#[derive(Debug, Clone)]
pub enum RecallOutcome {
    Chunks(Vec<ChunkResult>),
    Notes(Vec<RecallResult>),
}

struct ImportDraft {
    title: String,
    content: String,
    tags: Vec<String>,
    properties: HashMap<String, serde_json::Value>,
}

/// Open vault services and the operations `grafyn-mcp` exposes.
#[derive(Clone)]
pub struct Headless {
    knowledge_store: Arc<RwLock<KnowledgeStore>>,
    search_service: Arc<RwLock<SearchService>>,
    graph_index: Arc<RwLock<GraphIndex>>,
    memory_service: Arc<RwLock<MemoryService>>,
    chunk_index: Option<Arc<RwLock<ChunkIndex>>>,
    retrieval_service: Arc<RwLock<RetrievalService>>,
    priority_service: Arc<RwLock<PriorityScoringService>>,
    vault_path: PathBuf,
    /// Latest vault mtime this process has loaded; another process writing
    /// the vault moves it and triggers a cache + graph refresh.
    vault_stamp: Arc<Mutex<Option<SystemTime>>>,
    /// Monotonic time of the last vault probe, for throttling.
    last_probe: Arc<Mutex<Option<std::time::Instant>>>,
}

impl Headless {
    /// Open `vault_path` and `data_path`. Creates both directories.
    ///
    /// Search falls back to read-only when another process holds the writer
    /// lock. A missing chunk index disables chunk recall; it is not created
    /// here.
    pub fn open(
        vault_path: PathBuf,
        data_path: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        log::info!("Vault path: {}", vault_path.display());
        log::info!("Data path: {}", data_path.display());

        std::fs::create_dir_all(&vault_path)?;
        std::fs::create_dir_all(&data_path)?;

        let knowledge_store = KnowledgeStore::new(vault_path.clone(), data_path.clone());
        let search_service = open_search_service(&data_path)?;

        let mut graph_index = GraphIndex::new();
        let notes_for_graph: Vec<_> = knowledge_store
            .list_notes()
            .unwrap_or_default()
            .iter()
            .filter_map(|m| knowledge_store.get_note(&m.id).ok())
            .collect();
        graph_index.build_from_notes(&notes_for_graph);
        log::info!(
            "Graph index built: {} notes, {} links",
            graph_index.stats().total_notes,
            graph_index.stats().total_links
        );

        let memory_service = MemoryService::new();
        let priority_service = PriorityScoringService::new(data_path.clone());
        let retrieval_service = RetrievalService::new(data_path.clone());
        let chunk_index = open_chunk_index(&data_path);

        let vault_stamp = Arc::new(Mutex::new(latest_vault_mtime(&vault_path)));
        Ok(Self {
            knowledge_store: Arc::new(RwLock::new(knowledge_store)),
            search_service: Arc::new(RwLock::new(search_service)),
            graph_index: Arc::new(RwLock::new(graph_index)),
            memory_service: Arc::new(RwLock::new(memory_service)),
            chunk_index,
            retrieval_service: Arc::new(RwLock::new(retrieval_service)),
            priority_service: Arc::new(RwLock::new(priority_service)),
            vault_path,
            vault_stamp,
            last_probe: Arc::new(Mutex::new(None)),
        })
    }

    /// If the vault changed on disk since we last loaded it (e.g. the Grafyn
    /// app wrote notes), reload the note cache and rebuild the local graph.
    async fn refresh_if_vault_changed(&self) {
        // The probe stats the whole vault; at most one probe per 2 s keeps bursts of tool calls cheap.
        {
            let mut last = self.last_probe.lock().unwrap_or_else(|e| e.into_inner());
            if last.is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(2)) {
                return;
            }
            *last = Some(std::time::Instant::now());
        }
        let current = latest_vault_mtime(&self.vault_path);
        {
            let mut seen = self.vault_stamp.lock().unwrap_or_else(|e| e.into_inner());
            if *seen == current {
                return;
            }
            *seen = current;
        }
        // Lock order matches the write tools: knowledge_store, then graph.
        let mut ks = self.knowledge_store.write().await;
        ks.set_vault_path(self.vault_path.clone()); // re-walks the vault cache (changed files only)
        let notes = ks.cached_full_notes();
        self.graph_index.write().await.sync_notes(&notes);
    }

    pub async fn list_notes(&self) -> Result<Vec<NoteMeta>, String> {
        self.refresh_if_vault_changed().await;
        let ks = self.knowledge_store.read().await;
        ks.list_notes()
            .map_err(|e| format!("Failed to list notes: {e}"))
    }

    pub async fn get_note(&self, id: &str) -> Result<Note, String> {
        let ks = self.knowledge_store.read().await;
        ks.get_note(id).map_err(|e| format!("Note not found: {e}"))
    }

    pub async fn create_note(
        &self,
        title: String,
        content: String,
        tags: Vec<String>,
        status: &str,
    ) -> Result<StoredNote, String> {
        let status = parse_status(status)?;
        let create = NoteCreate {
            title,
            content,
            relative_path: None,
            aliases: Vec::new(),
            status,
            tags,
            schema_version: CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            properties: Default::default(),
        };

        let mut ks = self.knowledge_store.write().await;
        match ks.create_note(create) {
            Ok(note) => {
                let warning = {
                    let mut search = self.search_service.write().await;
                    apply_search_change(&mut search, "written", |s| {
                        index_commit::index_note_for_search(s, &note)
                    })
                };
                {
                    let mut graph = self.graph_index.write().await;
                    graph.update_note(&note);
                }
                Ok(StoredNote { note, warning })
            }
            Err(e) => Err(format!("Failed to create note: {e}")),
        }
    }

    pub async fn update_note(
        &self,
        id: &str,
        title: Option<String>,
        content: Option<String>,
        tags: Option<Vec<String>>,
        status: Option<&str>,
    ) -> Result<StoredNote, String> {
        let status = status.map(parse_status).transpose()?;
        let update = NoteUpdate {
            title,
            content,
            relative_path: None,
            aliases: None,
            status,
            tags,
            schema_version: None,
            migration_source: None,
            optimizer_managed: None,
            properties: None,
        };

        let mut ks = self.knowledge_store.write().await;
        match ks.update_note(id, update) {
            Ok(note) => {
                let warning = {
                    let mut search = self.search_service.write().await;
                    apply_search_change(&mut search, "written", |s| {
                        index_commit::index_note_for_search(s, &note)
                    })
                };
                {
                    let mut graph = self.graph_index.write().await;
                    graph.update_note(&note);
                }
                Ok(StoredNote { note, warning })
            }
            Err(e) => Err(format!("Failed to update note: {e}")),
        }
    }

    pub async fn delete_note(&self, id: &str) -> Result<DeletedNote, String> {
        let mut ks = self.knowledge_store.write().await;
        match ks.delete_note(id) {
            Ok(()) => {
                let warning = {
                    let mut search = self.search_service.write().await;
                    apply_search_change(&mut search, "deleted", |s| {
                        index_commit::remove_note_for_search(s, id)
                    })
                };
                {
                    let mut graph = self.graph_index.write().await;
                    graph.remove_note(id);
                }
                Ok(DeletedNote {
                    id: id.to_string(),
                    warning,
                })
            }
            Err(e) => Err(format!("Failed to delete note: {e}")),
        }
    }

    pub async fn search_notes(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let search = self.search_service.read().await;
        search
            .search(query, limit.max(1))
            .map_err(|e| format!("Search failed: {e}"))
    }

    pub async fn get_backlinks(&self, note_id: &str) -> Vec<(NoteMeta, RelationType)> {
        self.refresh_if_vault_changed().await;
        self.graph_index.read().await.get_typed_backlinks(note_id)
    }

    pub async fn get_outgoing(&self, note_id: &str) -> Vec<(NoteMeta, RelationType)> {
        self.refresh_if_vault_changed().await;
        self.graph_index.read().await.get_typed_outgoing(note_id)
    }

    pub async fn import_conversation(
        &self,
        file_path: &str,
        conversation_ids: &[String],
    ) -> Result<ImportedNotes, String> {
        let content =
            read_import_content(file_path).map_err(|e| format!("Failed to read file: {e}"))?;
        let (platform, items) = plan_import(file_path, &content, conversation_ids)?;
        Ok(self.write_imported_notes(platform, items).await)
    }

    pub async fn recall_relevant(
        &self,
        query: &str,
        context_note_ids: &[String],
        limit: usize,
        token_budget: Option<usize>,
    ) -> Result<RecallOutcome, String> {
        // If token_budget is set and chunk index is available, use chunk retrieval.
        if let (Some(budget), Some(chunk_index)) = (token_budget, &self.chunk_index) {
            let chunk_index = chunk_index.read().await;
            let graph = self.graph_index.read().await;
            let priority = self.priority_service.read().await;
            let retrieval = self.retrieval_service.read().await;

            match retrieval.retrieve_chunks(
                &chunk_index,
                &graph,
                &priority,
                query,
                budget,
                context_note_ids,
            ) {
                Ok(mut chunks) => {
                    retain_resolvable_chunks(&mut chunks, &*self.knowledge_store.read().await);
                    Ok(RecallOutcome::Chunks(chunks))
                }
                Err(e) => Err(format!("Chunk recall failed: {e}")),
            }
        } else {
            // Note-level recall (original behavior).
            let search = self.search_service.read().await;
            let graph = self.graph_index.read().await;
            let memory = self.memory_service.read().await;

            memory
                .recall_relevant(&search, &graph, query, context_note_ids, limit.max(1))
                .map(RecallOutcome::Notes)
                .map_err(|e| format!("Recall failed: {e}"))
        }
    }

    pub async fn search_chunks(
        &self,
        query: &str,
        token_budget: usize,
        context_note_ids: &[String],
    ) -> Result<ChunkQueryResult, String> {
        let Some(chunk_index) = &self.chunk_index else {
            return Err(
                "Chunk index not available. Run the Grafyn app first to build it.".to_string(),
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
            query,
            token_budget,
            context_note_ids,
        ) {
            Ok(mut chunks) => {
                retain_resolvable_chunks(&mut chunks, &*self.knowledge_store.read().await);
                let total_tokens = chunks.iter().map(|c| c.token_estimate).sum();
                Ok(ChunkQueryResult {
                    chunks,
                    total_tokens,
                    token_budget,
                })
            }
            Err(e) => Err(format!("Chunk search failed: {e}")),
        }
    }

    async fn write_imported_notes(
        &self,
        platform: String,
        items: Vec<ImportDraft>,
    ) -> ImportedNotes {
        let mut note_ids = Vec::new();
        let mut created_notes = Vec::new();
        let mut errors = Vec::new();
        let mut warning = None;

        for item in items {
            let note_create = NoteCreate {
                title: item.title.clone(),
                content: item.content,
                relative_path: None,
                aliases: Vec::new(),
                status: NoteStatus::Evidence,
                tags: item.tags,
                schema_version: CURRENT_NOTE_SCHEMA_VERSION,
                migration_source: Some("mcp-import".into()),
                optimizer_managed: false,
                properties: item.properties,
            };

            let mut ks = self.knowledge_store.write().await;
            match ks.create_note(note_create) {
                Ok(note) => {
                    {
                        let mut search = self.search_service.write().await;
                        if let Some(w) = apply_search_change(&mut search, "written", |s| {
                            index_commit::index_note_for_search(s, &note)
                        }) {
                            warning = Some(w);
                        }
                    }
                    note_ids.push(note.id.clone());
                    created_notes.push(note);
                }
                Err(e) => {
                    errors.push(format!("Failed to create '{}': {e}", item.title));
                }
            }
        }
        // One graph rebuild for the whole import instead of one per note.
        self.graph_index.write().await.update_notes(&created_notes);

        ImportedNotes {
            platform,
            note_ids,
            errors,
            warning,
        }
    }
}

fn open_search_service(data_path: &Path) -> Result<SearchService, Box<dyn std::error::Error>> {
    match SearchService::new(data_path.to_path_buf()) {
        Ok(service) => {
            log::info!("Search service initialized with write access");
            Ok(service)
        }
        Err(e) => {
            log::warn!(
                "Could not acquire search writer (Grafyn app may be running): {e}. \
                 Falling back to read-only search."
            );
            match SearchService::new_readonly(data_path.to_path_buf()) {
                Ok(service) => {
                    log::info!("Search service initialized in read-only mode");
                    Ok(service)
                }
                Err(e2) => {
                    // No no-search mode exists; fail with both causes instead of
                    // retrying the constructor that just failed.
                    Err(format!(
                        "Cannot open search index: writable open failed ({e}); \
                         read-only open failed ({e2})"
                    )
                    .into())
                }
            }
        }
    }
}

fn open_chunk_index(data_path: &Path) -> Option<Arc<RwLock<ChunkIndex>>> {
    // Read-only — the Tauri app may hold the writer lock.
    match ChunkIndex::new_readonly(data_path.to_path_buf()) {
        Ok(chunk_index) => {
            log::info!("Chunk index opened in read-only mode");
            Some(Arc::new(RwLock::new(chunk_index)))
        }
        Err(e) => {
            log::warn!(
                "Chunk index not available: {e}. search_chunks and chunk recall will be disabled."
            );
            None
        }
    }
}

/// Strict status parse: a typo must not silently demote a note to draft.
fn parse_status(raw: &str) -> Result<NoteStatus, String> {
    raw.parse().map_err(|_| {
        format!("Invalid status '{raw}': expected 'draft', 'evidence', or 'canonical'")
    })
}

/// Applies a search-index change and commits it. Returns a warning for the
/// tool result when the change could not be applied (instead of silently
/// leaving the note unsearchable).
fn apply_search_change(
    search: &mut SearchService,
    verb: &str,
    change: impl FnOnce(&mut SearchService) -> Result<(), String>,
) -> Option<String> {
    if search.is_readonly() {
        return Some(format!(
            "note {verb}; search index held by the app — results may be stale until reindex"
        ));
    }
    change(search)
        .and_then(|_| index_commit::commit_search(search))
        .err()
        .map(|e| format!("note {verb}; search index update failed: {e}"))
}

/// Latest modification time of anything under the vault (files and dirs, so
/// adds, edits, deletes and renames all move it). Stat-only, no file reads.
fn latest_vault_mtime(vault: &Path) -> Option<SystemTime> {
    WalkDir::new(vault)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.metadata().ok()?.modified().ok())
        .max()
}

fn plan_import(
    file_path: &str,
    content: &str,
    conversation_ids: &[String],
) -> Result<(String, Vec<ImportDraft>), String> {
    if let Some(platform) = import::detect_platform(content) {
        let items = chat_drafts(content, conversation_ids)?;
        Ok((platform.to_string(), items))
    } else {
        let items = document_drafts(file_path, content, conversation_ids)?;
        Ok(("document".to_string(), items))
    }
}

fn chat_drafts(content: &str, conversation_ids: &[String]) -> Result<Vec<ImportDraft>, String> {
    let all_conversations =
        import::parse_content(content).map_err(|e| format!("Failed to parse: {e}"))?;
    let to_import = if conversation_ids.is_empty() {
        all_conversations
    } else {
        all_conversations
            .into_iter()
            .filter(|conversation| conversation_ids.contains(&conversation.id))
            .collect::<Vec<_>>()
    };
    Ok(to_import
        .iter()
        .map(|conv| {
            let mut props = HashMap::new();
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
            ImportDraft {
                title: conv.title.clone(),
                content: import::format_as_markdown(conv),
                tags,
                properties: props,
            }
        })
        .collect())
}

fn document_drafts(
    file_path: &str,
    content: &str,
    conversation_ids: &[String],
) -> Result<Vec<ImportDraft>, String> {
    let path = Path::new(file_path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(file_path);
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let batch = import::document::parse_document_text(file_name, extension, content)
        .map_err(|e| format!("Failed to parse content: {e}"))?;
    let to_import = if conversation_ids.is_empty() {
        batch.items
    } else {
        batch
            .items
            .into_iter()
            .filter(|item| conversation_ids.contains(&item.id))
            .collect()
    };
    Ok(to_import
        .into_iter()
        .map(|item| {
            let mut props = item.metadata;
            props.insert(
                "source_id".into(),
                serde_json::Value::String(item.id.clone()),
            );
            ImportDraft {
                title: item.title,
                content: item.content,
                tags: item.suggested_tags,
                properties: props,
            }
        })
        .collect())
}

fn read_import_content(file_path: &str) -> Result<String, String> {
    let extension = Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if extension == "docx" {
        let bytes = std::fs::read(file_path).map_err(|e| e.to_string())?;
        return extract_docx_text(&bytes);
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

fn extract_docx_text(bytes: &[u8]) -> Result<String, String> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        ZipArchive::new(cursor).map_err(|e| format!("Failed to open DOCX archive: {e}"))?;
    let mut document = archive
        .by_name("word/document.xml")
        .map_err(|e| format!("Failed to find DOCX document text: {e}"))?;
    let mut xml = String::new();
    document
        .read_to_string(&mut xml)
        .map_err(|e| format!("Failed to read DOCX document text: {e}"))?;

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
                    .map_err(|e| format!("Failed to decode DOCX text: {e}"))?;
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
            Err(e) => return Err(format!("Failed to parse DOCX document text: {e}")),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp() -> (tempfile::TempDir, tempfile::TempDir, Headless) {
        let vault = tempfile::TempDir::new().unwrap();
        let data = tempfile::TempDir::new().unwrap();
        let core = Headless::open(vault.path().to_path_buf(), data.path().to_path_buf())
            .unwrap_or_else(|err| panic!("{err}"));
        (vault, data, core)
    }

    #[test]
    fn parse_status_rejects_unknown_strings() {
        assert_eq!(parse_status("canonical").unwrap(), NoteStatus::Canonical);
        assert!(parse_status("canonicl").is_err());
    }

    #[test]
    fn readonly_search_yields_stale_index_warning() {
        let tmp = tempfile::TempDir::new().unwrap();
        drop(SearchService::new(tmp.path().to_path_buf()).unwrap());
        let mut search = SearchService::new_readonly(tmp.path().to_path_buf()).unwrap();
        let warning = apply_search_change(&mut search, "written", |_| Ok(())).unwrap();
        assert!(warning.contains("stale until reindex"));
    }

    #[test]
    fn vault_mtime_moves_when_a_note_is_added() {
        let tmp = tempfile::TempDir::new().unwrap();
        let before = latest_vault_mtime(tmp.path());
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(tmp.path().join("new.md"), "# new").unwrap();
        assert_ne!(latest_vault_mtime(tmp.path()), before);
    }

    #[tokio::test]
    async fn open_create_get_list_search_and_delete_round_trip() {
        let (_vault, _data, core) = open_temp();

        let err = core
            .create_note("Nope".into(), "body".into(), Vec::new(), "published")
            .await
            .unwrap_err();
        assert!(err.contains("Invalid status 'published'"));
        assert!(core.list_notes().await.unwrap().is_empty());

        let written = core
            .create_note(
                "Harbor Note".into(),
                "Unique token kelp-anchor".into(),
                vec!["tide".into()],
                "evidence",
            )
            .await
            .unwrap();
        assert_eq!(written.note.status, NoteStatus::Evidence);
        assert!(written.warning.is_none());

        let fetched = core.get_note(&written.note.id).await.unwrap();
        assert_eq!(fetched.content, "Unique token kelp-anchor");

        let listed = core.list_notes().await.unwrap();
        assert!(listed.iter().any(|note| note.id == written.note.id));

        let hits = core.search_notes("kelp-anchor", 10).await.unwrap();
        assert!(hits.iter().any(|hit| hit.note.id == written.note.id));

        let deleted = core.delete_note(&written.note.id).await.unwrap();
        assert_eq!(deleted.id, written.note.id);
        assert!(core.get_note(&written.note.id).await.is_err());
    }

    #[tokio::test]
    async fn typed_wikilink_is_an_outgoing_edge_and_a_reversed_backlink() {
        let (_vault, _data, core) = open_temp();
        let harbor = core
            .create_note("Harbor Note".into(), "dock".into(), Vec::new(), "draft")
            .await
            .unwrap();
        let source = core
            .create_note(
                "Source Note".into(),
                "See [[Harbor Note]] (supports).\n".into(),
                Vec::new(),
                "draft",
            )
            .await
            .unwrap();

        let outgoing = core.get_outgoing(&source.note.id).await;
        assert!(
            outgoing
                .iter()
                .any(|(note, relation)| note.id == harbor.note.id
                    && *relation == RelationType::Supports),
            "outgoing={outgoing:?}"
        );

        let backlinks = core.get_backlinks(&harbor.note.id).await;
        assert!(
            backlinks
                .iter()
                .any(|(note, relation)| note.id == source.note.id
                    && *relation == RelationType::Related),
            "backlinks={backlinks:?}"
        );
    }

    #[tokio::test]
    async fn search_chunks_without_an_index_returns_the_existing_error() {
        let (_vault, _data, core) = open_temp();
        let err = core.search_chunks("kelp", 4000, &[]).await.unwrap_err();
        assert_eq!(
            err,
            "Chunk index not available. Run the Grafyn app first to build it."
        );
    }

    #[tokio::test]
    async fn search_chunks_reads_a_populated_index_and_obeys_the_token_budget() {
        let (_vault, _data, core, alpha, beta) = vault_with_indexed_chunks().await;

        let wide = core
            .search_chunks("zephyrquartz", 4_000, &[])
            .await
            .unwrap();
        assert!(
            wide.chunks.len() >= 2,
            "expected both passages, got {:?}",
            wide.chunks
                .iter()
                .map(|chunk| chunk.parent_title.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(wide.token_budget, 4_000);
        assert_eq!(
            wide.total_tokens,
            wide.chunks
                .iter()
                .map(|chunk| chunk.token_estimate)
                .sum::<usize>()
        );
        for chunk in &wide.chunks {
            assert!(chunk.search_score > 0.0, "score={}", chunk.search_score);
            assert!(chunk.token_estimate > 1, "tokens={}", chunk.token_estimate);
            assert!(chunk.text.contains("zephyrquartz"));
            assert!(
                (chunk.parent_note_id == alpha.id && chunk.parent_title == alpha.title)
                    || (chunk.parent_note_id == beta.id && chunk.parent_title == beta.title)
            );
        }

        let smallest = wide
            .chunks
            .iter()
            .map(|chunk| chunk.token_estimate)
            .min()
            .unwrap();
        let one = core
            .search_chunks("zephyrquartz", smallest, &[])
            .await
            .unwrap();
        assert!(!one.chunks.is_empty());
        assert!(one.total_tokens <= smallest);
        assert!(one.chunks.len() < wide.chunks.len());
        assert!(one
            .chunks
            .iter()
            .all(|chunk| chunk.text.contains("zephyrquartz")));

        let none = core
            .search_chunks("zephyrquartz", smallest - 1, &[])
            .await
            .unwrap();
        assert!(none.chunks.is_empty());
        assert_eq!(none.total_tokens, 0);
        assert_eq!(none.token_budget, smallest - 1);

        let missing = core
            .search_chunks("quuxnotpresent", 4_000, &[])
            .await
            .unwrap();
        assert!(missing.chunks.is_empty());
        assert_eq!(missing.total_tokens, 0);
    }

    #[tokio::test]
    async fn recall_boosts_a_linked_note_and_token_budget_bounds_chunks() {
        let (_vault, _data, core) = open_temp();
        let linked = core
            .create_note(
                "Linked Note".into(),
                "zephyrquartz sits on the linked dock.".into(),
                Vec::new(),
                "draft",
            )
            .await
            .unwrap();
        let unlinked = core
            .create_note(
                "Unlinked Note".into(),
                "zephyrquartz sits on the far dock.".into(),
                Vec::new(),
                "draft",
            )
            .await
            .unwrap();
        let context = core
            .create_note(
                "Context Note".into(),
                "See [[Linked Note]] (supports).\n".into(),
                Vec::new(),
                "draft",
            )
            .await
            .unwrap();

        let recalled = core
            .recall_relevant("zephyrquartz", &[context.note.id], 5, None)
            .await
            .unwrap();
        let RecallOutcome::Notes(notes) = recalled else {
            panic!("note-level recall should not return chunks");
        };
        let linked_hit = notes
            .iter()
            .find(|hit| hit.note_id == linked.note.id)
            .expect("linked note");
        let unlinked_hit = notes
            .iter()
            .find(|hit| hit.note_id == unlinked.note.id)
            .expect("unlinked note");
        assert_eq!(linked_hit.graph_boost, 0.15);
        assert_eq!(unlinked_hit.graph_boost, 0.0);
        assert!(linked_hit.total_score > linked_hit.score);
        assert_eq!(unlinked_hit.total_score, unlinked_hit.score);
        assert!(linked_hit.total_score > unlinked_hit.total_score);

        let (_chunk_vault, _chunk_data, chunked, _, _) = vault_with_indexed_chunks().await;
        let wide = chunked
            .search_chunks("zephyrquartz", 4_000, &[])
            .await
            .unwrap();
        let smallest = wide
            .chunks
            .iter()
            .map(|chunk| chunk.token_estimate)
            .min()
            .unwrap();
        let bounded = chunked
            .recall_relevant("zephyrquartz", &[], 10, Some(smallest))
            .await
            .unwrap();
        let RecallOutcome::Chunks(chunks) = bounded else {
            panic!("a token budget should return chunks");
        };
        let used: usize = chunks.iter().map(|chunk| chunk.token_estimate).sum();
        assert!(!chunks.is_empty());
        assert!(used <= smallest);
        assert!(chunks.len() < wide.chunks.len());
        assert!(chunks
            .iter()
            .all(|chunk| chunk.text.contains("zephyrquartz")));

        let too_small = chunked
            .recall_relevant("zephyrquartz", &[], 10, Some(smallest - 1))
            .await
            .unwrap();
        let RecallOutcome::Chunks(empty) = too_small else {
            panic!("a token budget should return chunks");
        };
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn content_edit_keeps_raw_frontmatter_and_metadata_edit_rewrites_it() {
        let vault = tempfile::TempDir::new().unwrap();
        let data = tempfile::TempDir::new().unwrap();
        let raw = "\
---
title: Custom Note
status: draft
tags:
\t- alpha
custom_key: keep-me
---

Original body about the dock.
";
        std::fs::write(vault.path().join("custom-note.md"), raw).unwrap();
        let core = Headless::open(vault.path().to_path_buf(), data.path().to_path_buf())
            .unwrap_or_else(|err| panic!("{err}"));
        let listed = core.list_notes().await.unwrap();
        let mut loaded = None;
        for meta in &listed {
            let note = core.get_note(&meta.id).await.unwrap();
            if note.content.contains("Original body about the dock") {
                loaded = Some(note);
                break;
            }
        }
        let loaded = loaded.expect("custom note missing from the vault");
        let id = loaded.id.clone();
        let fallback = loaded
            .frontmatter_raw_fallback
            .expect("unparsable frontmatter is kept");
        assert!(fallback.contains("custom_key: keep-me"));
        assert!(fallback.contains("\t- alpha"));

        let edited = core
            .update_note(
                &id,
                None,
                Some("Edited body about the dock.\n".into()),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            edited.note.content.trim_end(),
            "Edited body about the dock."
        );
        let preserved = std::fs::read_to_string(vault.path().join("custom-note.md")).unwrap();
        let block = frontmatter_block(&preserved);
        assert!(block.contains("custom_key: keep-me"));
        assert!(block.contains("\t- alpha"));
        assert!(preserved.contains("Edited body about the dock."));
        assert!(!preserved.contains("Original body about the dock."));
        assert!(edited
            .note
            .frontmatter_raw_fallback
            .as_ref()
            .is_some_and(|raw| raw.contains("custom_key: keep-me")));

        let rewritten = core
            .update_note(&id, None, None, None, Some("canonical"))
            .await
            .unwrap();
        assert_eq!(rewritten.note.status, NoteStatus::Canonical);
        assert_eq!(
            rewritten.note.content.trim_end(),
            "Edited body about the dock."
        );
        assert!(rewritten.note.frontmatter_raw_fallback.is_none());
        let file =
            std::fs::read_to_string(vault.path().join(&rewritten.note.relative_path)).unwrap();
        assert!(file.contains("status: canonical") || file.contains("status: \"canonical\""));
        assert!(file.contains("Edited body about the dock."));
        assert!(!file.contains("\t- alpha"));
        assert!(!file.contains("custom_key: keep-me"));
    }

    #[tokio::test]
    async fn imports_claude_markdown_and_plain_text() {
        let (_vault, _data, core) = open_temp();
        let files = tempfile::TempDir::new().unwrap();

        let claude = files.path().join("claude.json");
        std::fs::write(
            &claude,
            r#"[{
                "uuid": "claude-1",
                "name": "Harbor Chat",
                "chat_messages": [
                    {"sender": "human", "text": "Where is the dock?"},
                    {"sender": "assistant", "text": "Beside the kelp."}
                ]
            }]"#,
        )
        .unwrap();
        let imported = core
            .import_conversation(claude.to_str().unwrap(), &[])
            .await
            .unwrap();
        assert_eq!(imported.platform, "claude");
        assert_eq!(imported.note_ids.len(), 1);
        assert!(imported.errors.is_empty());
        let chat = core.get_note(&imported.note_ids[0]).await.unwrap();
        assert_eq!(chat.status, NoteStatus::Evidence);
        assert_eq!(chat.migration_source.as_deref(), Some("mcp-import"));
        assert_eq!(
            chat.properties
                .get("source")
                .and_then(|value| value.as_str()),
            Some("claude")
        );
        assert_eq!(
            chat.properties
                .get("created_via")
                .and_then(|value| value.as_str()),
            Some("mcp-import")
        );
        assert!(chat.content.contains("Beside the kelp."));

        let skipped = core
            .import_conversation(claude.to_str().unwrap(), &["missing-id".into()])
            .await
            .unwrap();
        assert_eq!(skipped.platform, "claude");
        assert!(skipped.note_ids.is_empty());
        assert!(skipped.errors.is_empty());

        let markdown = files.path().join("field-guide.md");
        std::fs::write(
            &markdown,
            "# Field Guide\n\n## Dock\n\nThe harbor dock is wooden.\n\n## Kelp\n\nThe kelp bed is green.\n",
        )
        .unwrap();
        let sections = core
            .import_conversation(markdown.to_str().unwrap(), &[])
            .await
            .unwrap();
        assert_eq!(sections.platform, "document");
        assert!(sections.note_ids.len() >= 3, "ids={:?}", sections.note_ids);
        let bodies = note_bodies(&core, &sections.note_ids).await;
        assert!(bodies.iter().any(|body| body.contains("wooden")));
        assert!(bodies.iter().any(|body| body.contains("green")));
        assert!(bodies.iter().all(|body| !body.is_empty()));
        for id in &sections.note_ids {
            assert_eq!(
                core.get_note(id).await.unwrap().status,
                NoteStatus::Evidence
            );
        }

        let plain = files.path().join("plain-note.txt");
        std::fs::write(&plain, "A plain sentence about quartz pebbles.\n").unwrap();
        let text = core
            .import_conversation(plain.to_str().unwrap(), &[])
            .await
            .unwrap();
        assert_eq!(text.platform, "document");
        assert!(text.note_ids.len() >= 2, "ids={:?}", text.note_ids);
        let bodies = note_bodies(&core, &text.note_ids).await;
        assert!(bodies.iter().any(|body| body.contains("quartz pebbles")));
    }

    #[tokio::test]
    async fn import_and_lookup_errors_do_not_write_a_note() {
        let (vault, _data, core) = open_temp();
        let missing = core
            .import_conversation("/tmp/grafyn-no-such-import-file.json", &[])
            .await
            .unwrap_err();
        assert!(missing.starts_with("Failed to read file:"), "{missing}");

        let empty = vault.path().join("blank.txt");
        std::fs::write(&empty, " \n\t\n").unwrap();
        let parsed = core
            .import_conversation(empty.to_str().unwrap(), &[])
            .await
            .unwrap_err();
        assert!(parsed.starts_with("Failed to parse content:"), "{parsed}");

        let err = core.get_note("missing-note").await.unwrap_err();
        assert!(err.starts_with("Note not found:"), "{err}");

        let created = core
            .create_note("Keep".into(), "original body".into(), Vec::new(), "draft")
            .await
            .unwrap();
        let rejected = core
            .update_note(&created.note.id, None, None, None, Some("published"))
            .await
            .unwrap_err();
        assert!(rejected.contains("Invalid status 'published'"));
        let still = core.get_note(&created.note.id).await.unwrap();
        assert_eq!(still.content, "original body");
        assert_eq!(still.status, NoteStatus::Draft);
        assert_eq!(core.list_notes().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn readonly_opener_warns_that_search_stays_stale() {
        let (vault, data, primary) = open_temp();
        primary
            .create_note("Seed".into(), "seed body".into(), Vec::new(), "draft")
            .await
            .unwrap();
        let secondary = Headless::open(vault.path().to_path_buf(), data.path().to_path_buf())
            .unwrap_or_else(|err| panic!("{err}"));

        let written = secondary
            .create_note(
                "Locked Note".into(),
                "readonly-token body".into(),
                Vec::new(),
                "draft",
            )
            .await
            .unwrap();
        let warning = written.warning.expect("read-only search warns");
        assert!(warning.contains("stale until reindex"), "{warning}");
        assert_eq!(
            secondary.get_note(&written.note.id).await.unwrap().content,
            "readonly-token body"
        );
        let hits = secondary.search_notes("readonly-token", 10).await.unwrap();
        assert!(hits.iter().all(|hit| hit.note.id != written.note.id));
    }

    fn frontmatter_block(file: &str) -> &str {
        let rest = file
            .strip_prefix("---\n")
            .expect("file starts with frontmatter");
        let end = rest.find("\n---\n").expect("frontmatter closes");
        &rest[..end]
    }

    async fn note_bodies(core: &Headless, ids: &[String]) -> Vec<String> {
        let mut bodies = Vec::new();
        for id in ids {
            bodies.push(core.get_note(id).await.unwrap().content);
        }
        bodies
    }

    async fn vault_with_indexed_chunks(
    ) -> (tempfile::TempDir, tempfile::TempDir, Headless, Note, Note) {
        let (vault, data, core) = open_temp();
        let passage = |label: &str| {
            let words = (0..48)
                .map(|index| format!("{label}{index}"))
                .collect::<Vec<_>>()
                .join(" ");
            format!("{words} zephyrquartz")
        };
        let alpha = core
            .create_note(
                "Alpha Passage".into(),
                passage("alpha"),
                Vec::new(),
                "draft",
            )
            .await
            .unwrap()
            .note;
        let beta = core
            .create_note("Beta Passage".into(), passage("beta"), Vec::new(), "draft")
            .await
            .unwrap()
            .note;
        drop(core);

        {
            let mut chunks = ChunkIndex::new(data.path().to_path_buf()).unwrap();
            chunks.index_note_chunks(&alpha).unwrap();
            chunks.index_note_chunks(&beta).unwrap();
            chunks.commit().unwrap();
        }

        let reopened = Headless::open(vault.path().to_path_buf(), data.path().to_path_buf())
            .unwrap_or_else(|err| panic!("{err}"));
        (vault, data, reopened, alpha, beta)
    }
}
