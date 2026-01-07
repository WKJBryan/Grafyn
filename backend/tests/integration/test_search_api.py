"""
Integration tests for Search API endpoints

Tests semantic search and similar notes functionality
"""
import pytest
from fastapi.testclient import TestClient
from unittest.mock import MagicMock

from app.main import create_app
from app.models.note import Note, SearchResult, NoteFrontmatter


# ============================================================================
# Test Fixtures
# ============================================================================

@pytest.fixture
def mock_knowledge_store():
    """Create mock knowledge store"""
    store = MagicMock()

    fm = NoteFrontmatter(title="Test Note")
    sample_note = Note(
        id="test-note",
        title="Test Note",
        content="Test content about machine learning and AI",
        frontmatter=fm
    )

    store.get_note.return_value = sample_note
    store.list_notes.return_value = []

    return store


@pytest.fixture
def mock_vector_search():
    """Create mock vector search service"""
    search = MagicMock()

    sample_results = [
        SearchResult(
            note_id="note-1",
            title="Machine Learning Basics",
            snippet="...introduction to machine learning...",
            score=0.92,
            tags=["ml", "ai"]
        ),
        SearchResult(
            note_id="note-2",
            title="Deep Learning Guide",
            snippet="...neural networks and deep learning...",
            score=0.85,
            tags=["deep-learning"]
        )
    ]

    search.search.return_value = sample_results
    return search


@pytest.fixture
def test_app(mock_knowledge_store, mock_vector_search):
    """Create test application with mocked services"""
    app = create_app()
    app.state.knowledge_store = mock_knowledge_store
    app.state.vector_search = mock_vector_search
    return app


@pytest.fixture
def client(test_app):
    """Create test client"""
    return TestClient(test_app)


# ============================================================================
# Search Notes Tests
# ============================================================================

class TestSearchNotes:
    """Tests for GET /api/search"""

    def test_returns_200(self, client):
        """Should return 200 OK"""
        response = client.get("/api/search?q=machine+learning")
        assert response.status_code == 200

    def test_returns_list(self, client):
        """Should return a list of results"""
        response = client.get("/api/search?q=test")
        assert isinstance(response.json(), list)

    def test_requires_query(self, client):
        """Should require query parameter"""
        response = client.get("/api/search")
        assert response.status_code == 422

    def test_query_min_length(self, client):
        """Query must have at least 1 character"""
        response = client.get("/api/search?q=")
        assert response.status_code == 422

    def test_query_max_length(self, client):
        """Query must not exceed 500 characters"""
        long_query = "x" * 501
        response = client.get(f"/api/search?q={long_query}")
        assert response.status_code == 422

    def test_respects_limit(self, client, mock_vector_search):
        """Should pass limit to search service"""
        client.get("/api/search?q=test&limit=5")
        mock_vector_search.search.assert_called_once()
        call_args = mock_vector_search.search.call_args
        assert call_args[0][1] == 5  # limit argument

    def test_limit_default(self, client, mock_vector_search):
        """Default limit should be 10"""
        client.get("/api/search?q=test")
        call_args = mock_vector_search.search.call_args
        assert call_args[0][1] == 10

    def test_limit_min_value(self, client):
        """Limit must be at least 1"""
        response = client.get("/api/search?q=test&limit=0")
        assert response.status_code == 422

    def test_limit_max_value(self, client):
        """Limit must not exceed 50"""
        response = client.get("/api/search?q=test&limit=51")
        assert response.status_code == 422

    def test_semantic_search_by_default(self, client, mock_vector_search):
        """Should use semantic search by default"""
        client.get("/api/search?q=test")
        mock_vector_search.search.assert_called()

    def test_semantic_false_uses_lexical(self, client, mock_vector_search, mock_knowledge_store):
        """Should use lexical search when semantic=false"""
        mock_knowledge_store.list_notes.return_value = []

        response = client.get("/api/search?q=test&semantic=false")
        assert response.status_code == 200

    def test_results_have_required_fields(self, client):
        """Results should have required fields"""
        response = client.get("/api/search?q=test")
        results = response.json()

        if results:
            result = results[0]
            assert "note_id" in result
            assert "title" in result
            assert "snippet" in result
            assert "score" in result


# ============================================================================
# Find Similar Notes Tests
# ============================================================================

class TestFindSimilarNotes:
    """Tests for GET /api/search/similar/{note_id}"""

    def test_returns_200(self, client):
        """Should return 200 OK for existing note"""
        response = client.get("/api/search/similar/test-note")
        assert response.status_code == 200

    def test_returns_list(self, client):
        """Should return a list of similar notes"""
        response = client.get("/api/search/similar/test-note")
        assert isinstance(response.json(), list)

    def test_returns_404_for_missing_note(self, client, mock_knowledge_store):
        """Should return 404 for non-existent note"""
        mock_knowledge_store.get_note.return_value = None

        response = client.get("/api/search/similar/nonexistent")
        assert response.status_code == 404

    def test_respects_limit(self, client, mock_vector_search):
        """Should pass limit to search service"""
        client.get("/api/search/similar/test-note?limit=3")
        call_args = mock_vector_search.search.call_args
        assert call_args[0][1] == 3

    def test_limit_default(self, client, mock_vector_search):
        """Default limit should be 5"""
        client.get("/api/search/similar/test-note")
        call_args = mock_vector_search.search.call_args
        assert call_args[0][1] == 5

    def test_limit_min_value(self, client):
        """Limit must be at least 1"""
        response = client.get("/api/search/similar/test-note?limit=0")
        assert response.status_code == 422

    def test_limit_max_value(self, client):
        """Limit must not exceed 20"""
        response = client.get("/api/search/similar/test-note?limit=21")
        assert response.status_code == 422

    def test_uses_note_content_for_search(self, client, mock_vector_search, mock_knowledge_store):
        """Should use note content for similarity search"""
        client.get("/api/search/similar/test-note")

        call_args = mock_vector_search.search.call_args
        query = call_args[0][0]

        # Should include title and content
        assert "Test Note" in query or "machine learning" in query
