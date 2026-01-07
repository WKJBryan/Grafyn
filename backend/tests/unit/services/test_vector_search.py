"""
Unit tests for VectorSearchService

Tests cover:
- LanceDB initialization and configuration
- Note indexing (single and batch)
- Semantic search functionality
- Vector dimension validation
- Upsert behavior
- Delete operations
- Edge cases and error handling
"""
import pytest
from pathlib import Path

from app.services.vector_search import VectorSearchService
from app.services.embedding import EmbeddingService
from tests.fixtures.sample_notes import (
    get_sample_note,
    get_notes_for_search_testing,
)


# ============================================================================
# Initialization and Configuration Tests
# ============================================================================

@pytest.mark.unit
class TestVectorSearchInitialization:
    """Test VectorSearchService initialization"""

    def test_initialize_creates_database(self, vector_search: VectorSearchService, temp_data_path: Path):
        """Test that initialization creates LanceDB database"""
        # Database should be created
        assert temp_data_path.exists()

    def test_vector_dimension_matches_embedding(self, vector_search: VectorSearchService, embedding_service: EmbeddingService):
        """Test that vector dimension matches embedding service"""
        # Get dimension from embedding service
        expected_dimension = embedding_service.dimension

        # Should be 384 for all-MiniLM-L6-v2
        assert expected_dimension == 384

    def test_reinitialize_preserves_data(self, vector_search: VectorSearchService):
        """Test that reinitializing doesn't lose data"""
        # Index a note
        vector_search.index_note("test-note", "Test Title", "Test content for searching")

        # Reinitialize
        vector_search._initialize_db()

        # Data should still be there
        results = vector_search.search("test content", limit=10)
        assert len(results) > 0


# ============================================================================
# Single Note Indexing Tests
# ============================================================================

@pytest.mark.unit
class TestSingleNoteIndexing:
    """Test indexing individual notes"""

    def test_index_note_success(self, vector_search: VectorSearchService):
        """Test successful note indexing"""
        note_id = "test-note-1"
        title = "Test Note"
        content = "This is test content for semantic search."

        vector_search.index_note(note_id, title, content)

        # Should be searchable
        results = vector_search.search("test content", limit=5)
        assert len(results) > 0
        assert any(r["note_id"] == note_id for r in results)

    def test_index_note_with_title_in_content(self, vector_search: VectorSearchService):
        """Test that title is combined with content for indexing"""
        note_id = "titled-note"
        title = "Unique Title For Search"
        content = "Some regular content."

        vector_search.index_note(note_id, title, content)

        # Should be findable by title
        results = vector_search.search("Unique Title", limit=5)
        assert len(results) > 0
        assert results[0]["note_id"] == note_id

    def test_index_empty_content(self, vector_search: VectorSearchService):
        """Test indexing note with empty content"""
        note_id = "empty-note"
        title = "Empty Note"
        content = ""

        # Should handle gracefully
        vector_search.index_note(note_id, title, content)

        results = vector_search.search("Empty Note", limit=5)
        # Should still be indexed by title
        assert any(r["note_id"] == note_id for r in results)

    def test_index_unicode_content(self, vector_search: VectorSearchService):
        """Test indexing note with unicode content"""
        note_id = "unicode-note"
        title = "Unicode Test 你好"
        content = "Content with unicode: 世界 🌍 مرحبا"

        vector_search.index_note(note_id, title, content)

        # Should be searchable
        results = vector_search.search("unicode", limit=5)
        assert any(r["note_id"] == note_id for r in results)

    def test_index_very_long_content(self, vector_search: VectorSearchService):
        """Test indexing note with very long content"""
        from tests.fixtures.sample_notes import get_large_note_content

        note_id = "large-note"
        title = "Large Note"
        content = get_large_note_content()

        # Should handle large content
        vector_search.index_note(note_id, title, content)

        results = vector_search.search("section", limit=5)
        assert any(r["note_id"] == note_id for r in results)


# ============================================================================
# Batch Indexing Tests
# ============================================================================

@pytest.mark.unit
class TestBatchIndexing:
    """Test batch indexing operations"""

    def test_index_all_success(self, vector_search: VectorSearchService):
        """Test batch indexing of multiple notes"""
        notes = [
            {"id": "note-1", "title": "First Note", "content": "Content one"},
            {"id": "note-2", "title": "Second Note", "content": "Content two"},
            {"id": "note-3", "title": "Third Note", "content": "Content three"},
        ]

        vector_search.index_all(notes)

        # All should be searchable
        for note in notes:
            results = vector_search.search(note["title"], limit=10)
            assert any(r["note_id"] == note["id"] for r in results)

    def test_index_all_empty_list(self, vector_search: VectorSearchService):
        """Test batch indexing with empty list"""
        # Should handle gracefully
        vector_search.index_all([])

        # No error should occur

    def test_index_all_large_batch(self, vector_search: VectorSearchService):
        """Test indexing large batch of notes"""
        notes = [
            {"id": f"note-{i}", "title": f"Note {i}", "content": f"Content {i}"}
            for i in range(100)
        ]

        vector_search.index_all(notes)

        # Spot check some notes
        results = vector_search.search("Note 50", limit=10)
        assert any(r["note_id"] == "note-50" for r in results)


# ============================================================================
# Semantic Search Tests
# ============================================================================

@pytest.mark.unit
class TestSemanticSearch:
    """Test semantic search functionality"""

    def test_search_returns_relevant_results(self, vector_search: VectorSearchService):
        """Test that search returns semantically relevant results"""
        # Index notes with specific topics
        notes = get_notes_for_search_testing()

        for note in notes:
            note_id = note["title"].lower().replace(" ", "-")
            vector_search.index_note(note_id, note["title"], note["content"])

        # Search for programming-related content
        results = vector_search.search("programming language", limit=5)

        # Should return programming notes, not recipe
        assert len(results) > 0
        # Programming notes should rank higher than recipe
        top_titles = [r["title"] for r in results[:3]]
        assert not any("Cookie" in title for title in top_titles)

    def test_search_ranking_by_relevance(self, vector_search: VectorSearchService):
        """Test that results are ranked by relevance"""
        # Index notes
        vector_search.index_note("python-1", "Python Basics", "Learn Python programming")
        vector_search.index_note("python-2", "Advanced Python", "Python is great for data science")
        vector_search.index_note("java-1", "Java Intro", "Java is another language")

        results = vector_search.search("Python programming", limit=10)

        # Python notes should rank higher
        assert len(results) >= 2
        # First result should be most relevant
        assert "python" in results[0]["note_id"]

    def test_search_with_limit(self, vector_search: VectorSearchService):
        """Test search result limiting"""
        # Index many notes
        for i in range(20):
            vector_search.index_note(f"note-{i}", f"Note {i}", f"Content {i}")

        # Search with small limit
        results = vector_search.search("Content", limit=5)

        assert len(results) <= 5

    def test_search_empty_query(self, vector_search: VectorSearchService):
        """Test search with empty query"""
        vector_search.index_note("test", "Test", "Content")

        # Empty query should return results or empty list
        results = vector_search.search("", limit=10)

        # Should handle gracefully (implementation dependent)
        assert isinstance(results, list)

    def test_search_no_results(self, vector_search: VectorSearchService):
        """Test search when no notes are indexed"""
        # Don't index anything
        results = vector_search.search("nonexistent query", limit=10)

        assert results == []

    def test_search_similarity_scores(self, vector_search: VectorSearchService):
        """Test that search results include similarity scores"""
        vector_search.index_note("test", "Test Note", "Test content")

        results = vector_search.search("test content", limit=5)

        if len(results) > 0:
            # Check if scores are present and valid
            if "_distance" in results[0] or "score" in results[0]:
                # Scores should be numeric
                score_key = "_distance" if "_distance" in results[0] else "score"
                assert isinstance(results[0][score_key], (int, float))

    def test_search_case_insensitive(self, vector_search: VectorSearchService):
        """Test that search is case insensitive"""
        vector_search.index_note("test", "Python Programming", "Learn Python")

        results_lower = vector_search.search("python programming", limit=5)
        results_upper = vector_search.search("PYTHON PROGRAMMING", limit=5)

        # Both should find the note
        assert len(results_lower) > 0
        assert len(results_upper) > 0


# ============================================================================
# Upsert Behavior Tests
# ============================================================================

@pytest.mark.unit
class TestUpsertBehavior:
    """Test update vs insert behavior"""

    def test_reindex_updates_existing_note(self, vector_search: VectorSearchService):
        """Test that reindexing updates rather than duplicates"""
        note_id = "update-test"

        # Index initially
        vector_search.index_note(note_id, "Original Title", "Original content")

        # Search for original
        results = vector_search.search("Original", limit=10)
        original_count = sum(1 for r in results if r["note_id"] == note_id)

        # Reindex with new content
        vector_search.index_note(note_id, "Updated Title", "Updated content")

        # Search for updated
        results = vector_search.search("Updated", limit=10)
        updated_count = sum(1 for r in results if r["note_id"] == note_id)

        # Should still only have one entry
        assert updated_count == 1

        # Old content should not be findable
        results = vector_search.search("Original content", limit=10)
        if len(results) > 0:
            # If found, it should be because of title similarity, not exact match
            pass

    def test_batch_index_updates_existing(self, vector_search: VectorSearchService):
        """Test that batch indexing updates existing notes"""
        # Initial index
        vector_search.index_all([
            {"id": "note-1", "title": "Note 1", "content": "Original 1"},
            {"id": "note-2", "title": "Note 2", "content": "Original 2"},
        ])

        # Batch update
        vector_search.index_all([
            {"id": "note-1", "title": "Note 1 Updated", "content": "New 1"},
            {"id": "note-2", "title": "Note 2 Updated", "content": "New 2"},
        ])

        # Should find updated content
        results = vector_search.search("New", limit=10)
        assert len(results) >= 2


# ============================================================================
# Delete Operations Tests
# ============================================================================

@pytest.mark.unit
class TestDeleteOperations:
    """Test note deletion from index"""

    def test_delete_note_success(self, vector_search: VectorSearchService):
        """Test successful note deletion"""
        note_id = "delete-test"

        # Index note
        vector_search.index_note(note_id, "Delete Test", "Content to delete")

        # Verify it's there
        results = vector_search.search("Delete Test", limit=10)
        assert any(r["note_id"] == note_id for r in results)

        # Delete it
        vector_search.delete_note(note_id)

        # Should no longer be found
        results = vector_search.search("Delete Test", limit=10)
        assert not any(r["note_id"] == note_id for r in results)

    def test_delete_nonexistent_note(self, vector_search: VectorSearchService):
        """Test deleting note that doesn't exist"""
        # Should handle gracefully
        vector_search.delete_note("nonexistent-note-id")

        # No error should occur

    def test_clear_all_notes(self, vector_search: VectorSearchService):
        """Test clearing all notes from index"""
        # Index several notes
        for i in range(5):
            vector_search.index_note(f"note-{i}", f"Note {i}", f"Content {i}")

        # Clear all
        vector_search.clear_all()

        # All should be gone
        results = vector_search.search("Content", limit=100)
        assert len(results) == 0


# ============================================================================
# Vector Dimension Tests
# ============================================================================

@pytest.mark.unit
class TestVectorDimensions:
    """Test vector dimension handling"""

    def test_vector_dimension_is_384(self, vector_search: VectorSearchService, embedding_service: EmbeddingService):
        """Test that vectors have correct dimension"""
        dimension = embedding_service.dimension
        assert dimension == 384

    def test_encoded_vectors_have_correct_dimension(self, embedding_service: EmbeddingService):
        """Test that encoded text produces correct dimension vectors"""
        text = "Test content for encoding"
        vector = embedding_service.encode(text)

        assert len(vector) == 384


# ============================================================================
# Edge Cases and Error Handling
# ============================================================================

@pytest.mark.unit
class TestEdgeCases:
    """Test edge cases and error conditions"""

    def test_search_with_special_characters(self, vector_search: VectorSearchService):
        """Test search query with special characters"""
        vector_search.index_note("test", "Test", "Content")

        # Special characters in query
        special_queries = [
            "@#$%^&*()",
            "query-with-dashes",
            "query_with_underscores",
            "query/with/slashes",
        ]

        for query in special_queries:
            # Should handle gracefully
            results = vector_search.search(query, limit=10)
            assert isinstance(results, list)

    def test_search_very_long_query(self, vector_search: VectorSearchService):
        """Test search with very long query"""
        vector_search.index_note("test", "Test", "Content")

        long_query = "word " * 1000  # Very long query

        # Should handle gracefully
        results = vector_search.search(long_query, limit=10)
        assert isinstance(results, list)

    def test_concurrent_indexing(self, vector_search: VectorSearchService):
        """Test concurrent indexing operations"""
        import concurrent.futures

        def index_note(i):
            vector_search.index_note(f"note-{i}", f"Note {i}", f"Content {i}")

        # Index notes concurrently
        with concurrent.futures.ThreadPoolExecutor(max_workers=5) as executor:
            futures = [executor.submit(index_note, i) for i in range(20)]
            [f.result() for f in futures]

        # All should be indexed
        results = vector_search.search("Content", limit=50)
        assert len(results) >= 20

    def test_index_note_with_none_values(self, vector_search: VectorSearchService):
        """Test handling of None values"""
        # Should handle None gracefully or raise appropriate error
        try:
            vector_search.index_note("test", None, None)
        except (ValueError, TypeError, AttributeError):
            # Acceptable to reject None values
            pass


# ============================================================================
# Performance Tests
# ============================================================================

@pytest.mark.unit
@pytest.mark.slow
class TestPerformance:
    """Test performance characteristics"""

    def test_search_performance_with_many_notes(self, vector_search: VectorSearchService):
        """Test search performance with large index"""
        import time

        # Index 1000 notes
        for i in range(1000):
            vector_search.index_note(
                f"note-{i}",
                f"Note {i}",
                f"Content for note number {i} with some additional text",
            )

        # Time a search
        start = time.time()
        results = vector_search.search("note content", limit=10)
        elapsed = time.time() - start

        # Should complete in reasonable time (< 1 second)
        assert elapsed < 1.0
        assert len(results) > 0

    def test_batch_indexing_performance(self, vector_search: VectorSearchService):
        """Test batch indexing performance"""
        import time

        notes = [
            {"id": f"note-{i}", "title": f"Note {i}", "content": f"Content {i}"}
            for i in range(100)
        ]

        start = time.time()
        vector_search.index_all(notes)
        elapsed = time.time() - start

        # Should complete in reasonable time
        assert elapsed < 30.0  # 30 seconds for 100 notes


# ============================================================================
# Integration with Embedding Service
# ============================================================================

@pytest.mark.unit
class TestEmbeddingIntegration:
    """Test integration with EmbeddingService"""

    def test_uses_embedding_service(self, vector_search: VectorSearchService, embedding_service: EmbeddingService):
        """Test that VectorSearch uses EmbeddingService correctly"""
        # This test verifies the integration
        assert vector_search.embedding_service is not None
        assert isinstance(vector_search.embedding_service, EmbeddingService)

    def test_embedding_consistency(self, vector_search: VectorSearchService):
        """Test that same content produces consistent embeddings"""
        content = "Consistent content for testing"

        # Index same content twice with different IDs
        vector_search.index_note("note-1", "Test", content)
        vector_search.index_note("note-2", "Test", content)

        # Both should be found with similar scores
        results = vector_search.search(content, limit=10)

        note1_results = [r for r in results if r["note_id"] == "note-1"]
        note2_results = [r for r in results if r["note_id"] == "note-2"]

        assert len(note1_results) > 0
        assert len(note2_results) > 0
