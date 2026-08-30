use crate::models::note::{Note, NoteMeta, SearchResult};
use anyhow::{Context, Result};
use std::path::PathBuf;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy};

#[derive(Debug)]
pub enum SearchOpenError {
    WriterBusy(tantivy::TantivyError),
    CorruptOrIncompatible(tantivy::TantivyError),
    Other(anyhow::Error),
}

impl SearchOpenError {
    pub fn is_writer_busy(&self) -> bool {
        matches!(self, Self::WriterBusy(_))
    }

    pub fn is_corrupt_or_incompatible(&self) -> bool {
        matches!(self, Self::CorruptOrIncompatible(_))
    }
}

impl std::fmt::Display for SearchOpenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WriterBusy(error)
            | Self::CorruptOrIncompatible(error) => error.fmt(formatter),
            Self::Other(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SearchOpenError {}

fn classify_tantivy_open(error: tantivy::TantivyError) -> SearchOpenError {
    match error {
        error @ (tantivy::TantivyError::DataCorruption(_)
        | tantivy::TantivyError::IncompatibleIndex(_)
        | tantivy::TantivyError::SchemaError(_)) => {
            SearchOpenError::CorruptOrIncompatible(error)
        }
        error => SearchOpenError::Other(anyhow::Error::new(error)),
    }
}

/// Full-text search service using Tantivy
pub struct SearchService {
    index_path: PathBuf,
    index: Index,
    reader: IndexReader,
    writer: Option<IndexWriter>,
    // Field references
    id_field: Field,
    title_field: Field,
    content_field: Field,
    tags_field: Field,
    status_field: Field,
}

impl SearchService {
    pub fn new(data_path: PathBuf) -> std::result::Result<Self, SearchOpenError> {
        let index_path = data_path.join("search_index");
        std::fs::create_dir_all(&index_path)
            .map_err(|error| SearchOpenError::Other(error.into()))?;

        // Define schema
        let mut schema_builder = Schema::builder();

        let id_field = schema_builder.add_text_field("id", STRING | STORED);
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let content_field = schema_builder.add_text_field("content", TEXT | STORED);
        let tags_field = schema_builder.add_text_field("tags", TEXT | STORED);
        let status_field = schema_builder.add_text_field("status", STRING | STORED);

        let schema = schema_builder.build();

        // Open or create index
        let index = if index_path.join("meta.json").exists() {
            Index::open_in_dir(&index_path).map_err(classify_tantivy_open)?
        } else {
            Index::create_in_dir(&index_path, schema.clone())
                .map_err(classify_tantivy_open)?
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(classify_tantivy_open)?;

        let writer = index
            .writer(50_000_000)
            .map_err(|error| match error {
                error @ tantivy::TantivyError::LockFailure(_, _) => {
                    SearchOpenError::WriterBusy(error)
                }
                error => classify_tantivy_open(error),
            })?;

        Ok(Self {
            index_path,
            index,
            reader,
            writer: Some(writer),
            id_field,
            title_field,
            content_field,
            tags_field,
            status_field,
        })
    }

    /// Open the search index in read-only mode (no writer lock acquired).
    ///
    /// This allows the MCP binary to coexist with a running Tauri app, since
    /// Tantivy only permits one IndexWriter per directory. In read-only mode,
    /// search queries work normally but index_note/remove_note/reindex_all are
    /// no-ops (they require the writer).
    pub fn new_readonly(data_path: PathBuf) -> Result<Self> {
        let index_path = data_path.join("search_index");

        // In read-only mode, the index must already exist
        if !index_path.join("meta.json").exists() {
            anyhow::bail!(
                "Search index not found at {}. Run the Grafyn app first to create it.",
                index_path.display()
            );
        }

        let index = Index::open_in_dir(&index_path)
            .context("Failed to open existing index in read-only mode")?;

        let schema = index.schema();
        let id_field = schema.get_field("id").context("Missing id field")?;
        let title_field = schema.get_field("title").context("Missing title field")?;
        let content_field = schema
            .get_field("content")
            .context("Missing content field")?;
        let tags_field = schema.get_field("tags").context("Missing tags field")?;
        let status_field = schema.get_field("status").context("Missing status field")?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("Failed to create index reader")?;

        Ok(Self {
            index_path,
            index,
            reader,
            writer: None, // No writer — read-only mode
            id_field,
            title_field,
            content_field,
            tags_field,
            status_field,
        })
    }

    pub(crate) fn uses_data_path(&self, data_path: &std::path::Path) -> bool {
        self.index_path == data_path.join("search_index")
    }

    /// Whether this service has write capabilities (index updates).
    pub fn is_readonly(&self) -> bool {
        self.writer.is_none()
    }

    /// Index a note
    pub fn index_note(&mut self, note: &Note) -> Result<()> {
        let writer = self.writer.as_mut().context("Writer not available")?;

        // Delete existing document with this ID
        let term = tantivy::Term::from_field_text(self.id_field, &note.id);
        writer.delete_term(term);

        // Add new document
        writer.add_document(doc!(
            self.id_field => note.id.clone(),
            self.title_field => note.title.clone(),
            self.content_field => note.content.clone(),
            self.tags_field => note.tags.join(" "),
            self.status_field => note.status.to_string(),
        ))?;

        Ok(())
    }

    /// Remove a note from the index
    pub fn remove_note(&mut self, note_id: &str) -> Result<()> {
        let writer = self.writer.as_mut().context("Writer not available")?;
        let term = tantivy::Term::from_field_text(self.id_field, note_id);
        writer.delete_term(term);
        Ok(())
    }

    /// Commit pending changes and reload the reader.
    ///
    /// The reader uses `ReloadPolicy::OnCommitWithDelay`, which reloads
    /// asynchronously on a background thread after commit — there's no
    /// guarantee a `search()` immediately following `commit()` sees the new
    /// segment. Reloading synchronously here closes that race so a note is
    /// guaranteed searchable as soon as `commit()` returns (mirrors
    /// `ChunkIndex::commit()`, which already does this).
    pub fn commit(&mut self) -> Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer.commit()?;
            self.reader.reload()?;
        }
        Ok(())
    }

    /// Reindex all notes
    pub fn reindex_all(&mut self, notes: &[Note]) -> Result<()> {
        let writer = self.writer.as_mut().context("Writer not available")?;

        // Clear existing index
        writer.delete_all_documents()?;

        // Index all notes
        for note in notes {
            writer.add_document(doc!(
                self.id_field => note.id.clone(),
                self.title_field => note.title.clone(),
                self.content_field => note.content.clone(),
                self.tags_field => note.tags.join(" "),
                self.status_field => note.status.to_string(),
            ))?;
        }

        self.commit()
    }

    /// Search notes by query string
    pub fn search(&self, query_str: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let searcher = self.reader.searcher();

        // Parse query across title and content fields
        let query_parser =
            QueryParser::for_index(&self.index, vec![self.title_field, self.content_field]);

        let query = query_parser
            .parse_query(query_str)
            .context("Failed to parse query")?;

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .context("Search failed")?;

        let mut results = Vec::new();

        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;

            let id = doc
                .get_first(self.id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let title = doc
                .get_first(self.title_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let content = doc
                .get_first(self.content_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let tags: Vec<String> = doc
                .get_first(self.tags_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .split_whitespace()
                .map(String::from)
                .collect();

            let status = doc
                .get_first(self.status_field)
                .and_then(|v| v.as_str())
                .unwrap_or("draft")
                .parse()
                .unwrap_or_default();

            // Create snippet from content
            let snippet = create_snippet(&content, query_str, 150);

            results.push(SearchResult {
                note: NoteMeta {
                    id,
                    title,
                    relative_path: String::new(),
                    aliases: Vec::new(),
                    status,
                    tags,
                    created_at: chrono::Utc::now(), // Not stored in index
                    updated_at: chrono::Utc::now(),
                    schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
                    migration_source: None,
                    optimizer_managed: false,
                },
                score,
                snippet: Some(snippet),
            });
        }

        Ok(results)
    }

    /// Find notes similar to a given note (by content overlap)
    pub fn find_similar(
        &self,
        note_id: &str,
        content: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        // Use important words from the content as a query
        let query_words: Vec<&str> = content
            .split_whitespace()
            .filter(|w| w.len() > 4) // Skip short words
            .take(20) // Use first 20 significant words
            .collect();

        if query_words.is_empty() {
            return Ok(Vec::new());
        }

        let query_str = query_words.join(" ");
        let mut results = self.search(&query_str, limit + 1)?;

        // Filter out the source note
        results.retain(|r| r.note.id != note_id);
        results.truncate(limit);

        Ok(results)
    }
}

/// Create a text snippet around matching terms
fn create_snippet(content: &str, query: &str, max_len: usize) -> String {
    let content_lower = content.to_lowercase();
    let query_lower = query.to_lowercase();

    // Find first occurrence of any query term
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

    let mut best_pos = 0;
    for term in &query_terms {
        if let Some(pos) = content_lower.find(term) {
            best_pos = pos;
            break;
        }
    }

    // Extract snippet around the match
    let start = best_pos.saturating_sub(max_len / 2);
    let end = (start + max_len).min(content.len());

    let mut snippet: String = content.chars().skip(start).take(end - start).collect();

    // Add ellipsis if truncated
    if start > 0 {
        snippet = format!("...{}", snippet.trim_start());
    }
    if end < content.len() {
        snippet = format!("{}...", snippet.trim_end());
    }

    snippet
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{NoteStatus, CURRENT_NOTE_SCHEMA_VERSION};
    use chrono::Utc;

    fn note(id: &str, content: &str) -> Note {
        Note {
            id: id.to_string(),
            title: id.to_string(),
            content: content.to_string(),
            relative_path: format!("{id}.md"),
            aliases: Vec::new(),
            status: NoteStatus::Draft,
            tags: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            schema_version: CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            wikilinks: Vec::new(),
            parsed_links: Vec::new(),
            properties: Default::default(),
            frontmatter_raw_fallback: None,
        }
    }

    #[test]
    fn full_reindex_is_visible_to_the_existing_reader_before_return() {
        let root = tempfile::tempdir().unwrap();
        let mut search = SearchService::new(root.path().to_path_buf()).unwrap();

        search
            .reindex_all(&[note("fresh", "generation sentinel")])
            .unwrap();

        let results = search.search("generation", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].note.id, "fresh");
    }

    #[test]
    fn a_second_writer_is_typed_busy_and_never_destroys_the_healthy_index() {
        let root = tempfile::tempdir().unwrap();
        let first = SearchService::new(root.path().to_path_buf()).unwrap();
        let sentinel = root.path().join("search_index/sentinel");
        std::fs::write(&sentinel, b"healthy").unwrap();

        let error = match SearchService::new(root.path().to_path_buf()) {
            Ok(_) => panic!("a second Tantivy writer must not open"),
            Err(error) => error,
        };

        assert!(error.is_writer_busy());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"healthy");
        drop(first);
    }
}
