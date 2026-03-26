use crate::models::note::{ChunkResult, Note};
use crate::services::texttiling::{self, TextTilingConfig};
use anyhow::{Context, Result};
use std::path::PathBuf;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy};

/// Chunk-level search index using Tantivy.
///
/// Parallel index to SearchService — indexes TextTiling segments of notes
/// for paragraph-granularity retrieval. Each note produces 1-N chunk documents.
pub struct ChunkIndex {
    #[allow(dead_code)]
    index: Index,
    reader: IndexReader,
    writer: Option<IndexWriter>,
    // Field references
    chunk_id_field: Field,
    parent_note_id_field: Field,
    parent_title_field: Field,
    text_field: Field,
    start_char_field: Field,
    end_char_field: Field,
    depth_score_field: Field,
}

impl ChunkIndex {
    pub fn new(data_path: PathBuf) -> Result<Self> {
        let index_path = data_path.join("chunk_index");
        std::fs::create_dir_all(&index_path)?;

        let mut schema_builder = Schema::builder();

        let chunk_id_field = schema_builder.add_text_field("chunk_id", STRING | STORED);
        let parent_note_id_field =
            schema_builder.add_text_field("parent_note_id", STRING | STORED);
        let parent_title_field = schema_builder.add_text_field("parent_title", TEXT | STORED);
        let text_field = schema_builder.add_text_field("text", TEXT | STORED);
        let start_char_field =
            schema_builder.add_u64_field("start_char", STORED);
        let end_char_field =
            schema_builder.add_u64_field("end_char", STORED);
        let depth_score_field =
            schema_builder.add_f64_field("depth_score", STORED);

        let schema = schema_builder.build();

        let index = if index_path.join("meta.json").exists() {
            Index::open_in_dir(&index_path).context("Failed to open chunk index")?
        } else {
            Index::create_in_dir(&index_path, schema.clone())
                .context("Failed to create chunk index")?
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("Failed to create chunk index reader")?;

        let writer = index
            .writer(30_000_000)
            .context("Failed to create chunk index writer")?;

        Ok(Self {
            index,
            reader,
            writer: Some(writer),
            chunk_id_field,
            parent_note_id_field,
            parent_title_field,
            text_field,
            start_char_field,
            end_char_field,
            depth_score_field,
        })
    }

    /// Open in read-only mode for MCP concurrent access.
    pub fn new_readonly(data_path: PathBuf) -> Result<Self> {
        let index_path = data_path.join("chunk_index");

        if !index_path.join("meta.json").exists() {
            anyhow::bail!(
                "Chunk index not found at {}. Run the Grafyn app first to create it.",
                index_path.display()
            );
        }

        let index = Index::open_in_dir(&index_path)
            .context("Failed to open chunk index in read-only mode")?;

        let schema = index.schema();
        let chunk_id_field = schema
            .get_field("chunk_id")
            .context("Missing chunk_id field")?;
        let parent_note_id_field = schema
            .get_field("parent_note_id")
            .context("Missing parent_note_id field")?;
        let parent_title_field = schema
            .get_field("parent_title")
            .context("Missing parent_title field")?;
        let text_field = schema.get_field("text").context("Missing text field")?;
        let start_char_field = schema
            .get_field("start_char")
            .context("Missing start_char field")?;
        let end_char_field = schema
            .get_field("end_char")
            .context("Missing end_char field")?;
        let depth_score_field = schema
            .get_field("depth_score")
            .context("Missing depth_score field")?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("Failed to create chunk index reader")?;

        Ok(Self {
            index,
            reader,
            writer: None,
            chunk_id_field,
            parent_note_id_field,
            parent_title_field,
            text_field,
            start_char_field,
            end_char_field,
            depth_score_field,
        })
    }

    pub fn is_readonly(&self) -> bool {
        self.writer.is_none()
    }

    /// Index all chunks for a single note. Deletes old chunks first, then adds new ones.
    pub fn index_note_chunks(&mut self, note: &Note) -> Result<()> {
        let writer = self.writer.as_mut().context("Writer not available")?;

        // Delete existing chunks for this note
        let term =
            tantivy::Term::from_field_text(self.parent_note_id_field, &note.id);
        writer.delete_term(term);

        // Segment note content using TextTiling
        let config = TextTilingConfig::default();
        let segments = texttiling::segment(&note.content, &config);

        // If TextTiling returns no segments (very short note), index the whole content as one chunk
        if segments.is_empty() && !note.content.trim().is_empty() {
            let chunk_id = format!("{}:0", note.id);
            writer.add_document(doc!(
                self.chunk_id_field => chunk_id,
                self.parent_note_id_field => note.id.clone(),
                self.parent_title_field => note.title.clone(),
                self.text_field => note.content.clone(),
                self.start_char_field => 0u64,
                self.end_char_field => note.content.len() as u64,
                self.depth_score_field => 0.0f64,
            ))?;
        } else {
            for segment in &segments {
                let chunk_id = format!("{}:{}", note.id, segment.start_char);
                writer.add_document(doc!(
                    self.chunk_id_field => chunk_id,
                    self.parent_note_id_field => note.id.clone(),
                    self.parent_title_field => note.title.clone(),
                    self.text_field => segment.content.clone(),
                    self.start_char_field => segment.start_char as u64,
                    self.end_char_field => segment.end_char as u64,
                    self.depth_score_field => segment.depth_score,
                ))?;
            }
        }

        Ok(())
    }

    /// Remove all chunks for a note.
    pub fn remove_note_chunks(&mut self, note_id: &str) -> Result<()> {
        let writer = self.writer.as_mut().context("Writer not available")?;
        let term = tantivy::Term::from_field_text(self.parent_note_id_field, note_id);
        writer.delete_term(term);
        Ok(())
    }

    /// Commit pending changes and reload the reader.
    pub fn commit(&mut self) -> Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer.commit()?;
            self.reader.reload()?;
        }
        Ok(())
    }

    /// Reindex all notes.
    pub fn reindex_all(&mut self, notes: &[Note]) -> Result<()> {
        let writer = self.writer.as_mut().context("Writer not available")?;
        writer.delete_all_documents()?;
        writer.commit()?;

        for note in notes {
            self.index_note_chunks(note)?;
        }

        self.commit()?;
        Ok(())
    }

    /// Search chunks by query, returning results ordered by BM25 score.
    pub fn search_chunks(&self, query: &str, limit: usize) -> Result<Vec<ChunkResult>> {
        let searcher = self.reader.searcher();
        let query_parser =
            QueryParser::for_index(searcher.index(), vec![self.text_field, self.parent_title_field]);

        let parsed_query = query_parser
            .parse_query(query)
            .context("Failed to parse chunk search query")?;

        let top_docs = searcher
            .search(&parsed_query, &TopDocs::with_limit(limit))
            .context("Chunk search failed")?;

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            let doc: tantivy::TantivyDocument = searcher
                .doc(doc_address)
                .context("Failed to retrieve chunk document")?;

            let chunk_id = doc
                .get_first(self.chunk_id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let parent_note_id = doc
                .get_first(self.parent_note_id_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let parent_title = doc
                .get_first(self.parent_title_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let text = doc
                .get_first(self.text_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let start_char = doc
                .get_first(self.start_char_field)
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;

            let end_char = doc
                .get_first(self.end_char_field)
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;

            let depth_score = doc
                .get_first(self.depth_score_field)
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            // Approximate token count: ~1.33 tokens per word for English text
            let token_estimate = text.split_whitespace().count() * 4 / 3;

            results.push(ChunkResult {
                chunk_id,
                parent_note_id,
                parent_title,
                text,
                start_char,
                end_char,
                depth_score,
                search_score: score,
                token_estimate,
            });
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::NoteStatus;
    use chrono::Utc;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn make_note(id: &str, title: &str, content: &str) -> Note {
        Note {
            id: id.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            status: NoteStatus::Draft,
            tags: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            wikilinks: vec![],
            properties: HashMap::new(),
        }
    }

    #[test]
    fn test_index_and_search_chunks() {
        let tmp = TempDir::new().unwrap();
        let mut index = ChunkIndex::new(tmp.path().to_path_buf()).unwrap();

        let note = make_note(
            "note1",
            "Machine Learning",
            "Machine learning is a subset of artificial intelligence. \
             It enables computers to learn from data without being explicitly programmed. \
             Deep learning uses neural networks with many layers to learn representations.",
        );

        index.index_note_chunks(&note).unwrap();
        index.commit().unwrap();

        let results = index.search_chunks("neural networks", 10).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].parent_note_id, "note1");
        assert_eq!(results[0].parent_title, "Machine Learning");
        assert!(results[0].token_estimate > 0);
    }

    #[test]
    fn test_remove_note_chunks() {
        let tmp = TempDir::new().unwrap();
        let mut index = ChunkIndex::new(tmp.path().to_path_buf()).unwrap();

        let note = make_note("note1", "Test", "Some content about testing.");
        index.index_note_chunks(&note).unwrap();
        index.commit().unwrap();

        assert!(!index.search_chunks("testing", 10).unwrap().is_empty());

        index.remove_note_chunks("note1").unwrap();
        index.commit().unwrap();

        assert!(index.search_chunks("testing", 10).unwrap().is_empty());
    }

    #[test]
    fn test_reindex_all() {
        let tmp = TempDir::new().unwrap();
        let mut index = ChunkIndex::new(tmp.path().to_path_buf()).unwrap();

        let notes = vec![
            make_note("n1", "Alpha", "Content about alpha topic."),
            make_note("n2", "Beta", "Content about beta topic."),
        ];

        index.reindex_all(&notes).unwrap();

        let alpha = index.search_chunks("alpha", 10).unwrap();
        let beta = index.search_chunks("beta", 10).unwrap();

        assert_eq!(alpha.len(), 1);
        assert_eq!(alpha[0].parent_note_id, "n1");
        assert_eq!(beta.len(), 1);
        assert_eq!(beta[0].parent_note_id, "n2");
    }
}
