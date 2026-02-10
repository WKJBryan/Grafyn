use crate::models::note::{Note, NoteMeta, SearchResult};
use anyhow::{Context, Result};
use std::path::PathBuf;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy};

/// Full-text search service using Tantivy
pub struct SearchService {
    index: Index,
    reader: IndexReader,
    writer: Option<IndexWriter>,
    schema: Schema,
    // Field references
    id_field: Field,
    title_field: Field,
    content_field: Field,
    tags_field: Field,
    status_field: Field,
}

impl SearchService {
    pub fn new(data_path: PathBuf) -> Result<Self> {
        let index_path = data_path.join("search_index");
        std::fs::create_dir_all(&index_path)?;

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
            Index::open_in_dir(&index_path).context("Failed to open existing index")?
        } else {
            Index::create_in_dir(&index_path, schema.clone())
                .context("Failed to create new index")?
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("Failed to create index reader")?;

        let writer = index
            .writer(50_000_000)
            .context("Failed to create index writer")?;

        Ok(Self {
            index,
            reader,
            writer: Some(writer),
            schema,
            id_field,
            title_field,
            content_field,
            tags_field,
            status_field,
        })
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

    /// Commit pending changes
    pub fn commit(&mut self) -> Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer.commit()?;
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

        writer.commit()?;
        Ok(())
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
                    status,
                    tags,
                    created_at: chrono::Utc::now(), // Not stored in index
                    updated_at: chrono::Utc::now(),
                },
                score,
                snippet: Some(snippet),
            });
        }

        Ok(results)
    }

    /// Find notes similar to a given note (by content overlap)
    pub fn find_similar(&self, note_id: &str, content: &str, limit: usize) -> Result<Vec<SearchResult>> {
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

    let mut snippet: String = content
        .chars()
        .skip(start)
        .take(end - start)
        .collect();

    // Add ellipsis if truncated
    if start > 0 {
        snippet = format!("...{}", snippet.trim_start());
    }
    if end < content.len() {
        snippet = format!("{}...", snippet.trim_end());
    }

    snippet
}
