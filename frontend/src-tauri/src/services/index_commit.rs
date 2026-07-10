//! Shared search-index-commit primitives used by BOTH the Tauri desktop app
//! (`main.rs` / `commands/mod.rs`) and the standalone MCP binary
//! (`mcp.rs` / `mcp_tools.rs`).
//!
//! Both binaries share the `services/` module tree verbatim (each declares
//! its own `mod services;` pointing at the same directory — see `main.rs`
//! and `mcp.rs`), so a plain function here compiles into both without any
//! feature-gating, as long as it depends only on other `services`/`models`
//! types and never on `AppState`, `tauri::State`, or anything Tauri-specific.
//!
//! This module intentionally covers ONLY the search-index half of "commit a
//! note write" — not chunk indexing, not graph updates, not topic hubs. Chunk
//! indexing in particular is NOT genuinely shared: the Tauri app always
//! syncs the chunk index on every note write (`commands::sync_chunk_index_for_note`),
//! but the MCP binary's `create_note`/`update_note`/`delete_note` tools have
//! never written to the chunk index at all (only `search_chunks` reads it,
//! and only when a chunk index was supplied at startup) — extracting a
//! shared "index commit" that included chunk sync would either change MCP
//! behavior (out of scope) or need a parallel MCP-only chunk path that
//! doesn't exist today. See the Part B write-up in the follow-up report for
//! the full reasoning.
use crate::models::note::Note;
use crate::services::search::SearchService;

/// Indexes a single note into `search`.
///
/// A no-op (returns `Ok(())` without touching the index) when `search` is in
/// read-only mode — the MCP binary's fallback when it can't acquire the
/// Tantivy writer lock because the Tauri app already holds it
/// (`SearchService::new_readonly`). The Tauri app's own `SearchService` is
/// never constructed read-only, so this check is unconditionally safe there
/// too (always `false`).
///
/// Does NOT call `search.commit()` — batched call sites (e.g. Tauri's
/// `commit_note_writes`, which indexes several notes before committing once)
/// need to control exactly when the commit happens. Call [`commit_search`]
/// when the caller is done indexing for this batch.
pub fn index_note_for_search(search: &mut SearchService, note: &Note) -> Result<(), String> {
    if search.is_readonly() {
        return Ok(());
    }
    search.index_note(note).map_err(|error| error.to_string())
}

/// Removes a single note from `search` by id. Same read-only no-op and
/// commit-is-separate contract as [`index_note_for_search`].
pub fn remove_note_for_search(search: &mut SearchService, note_id: &str) -> Result<(), String> {
    if search.is_readonly() {
        return Ok(());
    }
    search
        .remove_note(note_id)
        .map_err(|error| error.to_string())
}

/// Commits pending index writes and reloads the reader so they're
/// immediately searchable. A no-op when `search` is in read-only mode (there
/// is no writer to commit).
pub fn commit_search(search: &mut SearchService) -> Result<(), String> {
    if search.is_readonly() {
        return Ok(());
    }
    search.commit().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::{NoteStatus, CURRENT_NOTE_SCHEMA_VERSION};
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn make_note(id: &str, title: &str) -> Note {
        let now = chrono::Utc::now();
        Note {
            id: id.to_string(),
            title: title.to_string(),
            content: format!("Content of {}", title),
            relative_path: format!("{}.md", id),
            aliases: Vec::new(),
            status: NoteStatus::Draft,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            schema_version: CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: None,
            optimizer_managed: false,
            wikilinks: Vec::new(),
            parsed_links: Vec::new(),
            properties: HashMap::new(),
            ..Default::default()
        }
    }

    #[test]
    fn index_note_for_search_makes_note_searchable_after_commit() {
        let data_dir = tempdir().expect("temp dir should be created");
        let mut search =
            SearchService::new(data_dir.path().to_path_buf()).expect("search service should init");

        let note = make_note("note-1", "Shared Helper Topic");
        index_note_for_search(&mut search, &note).expect("index should succeed");
        commit_search(&mut search).expect("commit should succeed");

        let results = search
            .search("Shared Helper Topic", 10)
            .expect("search should not error");
        assert!(results.iter().any(|r| r.note.id == "note-1"));
    }

    #[test]
    fn remove_note_for_search_drops_note_from_results() {
        let data_dir = tempdir().expect("temp dir should be created");
        let mut search =
            SearchService::new(data_dir.path().to_path_buf()).expect("search service should init");

        let note = make_note("note-2", "Removable Helper Topic");
        index_note_for_search(&mut search, &note).expect("index should succeed");
        commit_search(&mut search).expect("commit should succeed");

        remove_note_for_search(&mut search, "note-2").expect("remove should succeed");
        commit_search(&mut search).expect("commit should succeed");

        let results = search
            .search("Removable Helper Topic", 10)
            .expect("search should not error");
        assert!(!results.iter().any(|r| r.note.id == "note-2"));
    }

    #[test]
    fn readonly_search_service_is_a_no_op_instead_of_erroring() {
        let data_dir = tempdir().expect("temp dir should be created");
        // A readonly SearchService requires an existing index — create one
        // first via a writable instance, then reopen readonly.
        {
            let mut writer = SearchService::new(data_dir.path().to_path_buf())
                .expect("search service should init");
            writer.commit().expect("initial commit should succeed");
        }

        let mut readonly = SearchService::new_readonly(data_dir.path().to_path_buf())
            .expect("readonly search service should open");

        let note = make_note("note-3", "Readonly Helper Topic");
        assert!(index_note_for_search(&mut readonly, &note).is_ok());
        assert!(remove_note_for_search(&mut readonly, "note-3").is_ok());
        assert!(commit_search(&mut readonly).is_ok());
    }
}
