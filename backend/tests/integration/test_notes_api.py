"""
Integration tests for Notes API endpoints

Tests the full request/response cycle for note CRUD operations
"""
import pytest
from fastapi.testclient import TestClient
from unittest.mock import MagicMock, patch
import json

from app.main import create_app
from app.models.note import Note, NoteCreate, NoteUpdate, NoteListItem, NoteFrontmatter


# ============================================================================
# Test Fixtures
# ============================================================================

@pytest.fixture
def mock_knowledge_store():
    """Create mock knowledge store"""
    store = MagicMock()

    # Sample notes
    fm = NoteFrontmatter(title="Test Note")
    sample_note = Note(
        id="test-note",
        title="Test Note",
        content="Test content with [[wikilink]]",
        frontmatter=fm,
        outgoing_links=["wikilink"],
        backlinks=[]
    )

    sample_list = [
        NoteListItem(id="note-1", title="Note 1", status="draft", tags=["test"]),
        NoteListItem(id="note-2", title="Note 2", status="canonical", tags=[])
    ]

    store.list_notes.return_value = sample_list
    store.get_note.return_value = sample_note
    store.create_note.return_value = sample_note
    store.update_note.return_value = sample_note
    store.delete_note.return_value = True
    store.get_all_content.return_value = []

    return store


@pytest.fixture
def mock_vector_search():
    """Create mock vector search service"""
    search = MagicMock()
    search.search.return_value = []
    search.index_all.return_value = None
    return search


@pytest.fixture
def mock_graph_index():
    """Create mock graph index service"""
    graph = MagicMock()
    graph.get_backlinks.return_value = []
    graph.get_outgoing_links.return_value = []
    graph.build_index.return_value = None
    return graph


@pytest.fixture
def test_app(mock_knowledge_store, mock_vector_search, mock_graph_index):
    """Create test application with mocked services"""
    app = create_app()
    app.state.knowledge_store = mock_knowledge_store
    app.state.vector_search = mock_vector_search
    app.state.graph_index = mock_graph_index
    return app


@pytest.fixture
def client(test_app):
    """Create test client"""
    return TestClient(test_app)


# ============================================================================
# List Notes Tests
# ============================================================================

class TestListNotes:
    """Tests for GET /api/notes"""

    def test_returns_200(self, client):
        """Should return 200 OK"""
        response = client.get("/api/notes")
        assert response.status_code == 200

    def test_returns_list(self, client):
        """Should return a list"""
        response = client.get("/api/notes")
        assert isinstance(response.json(), list)

    def test_returns_note_list_items(self, client):
        """Should return NoteListItem objects"""
        response = client.get("/api/notes")
        notes = response.json()

        if notes:
            assert "id" in notes[0]
            assert "title" in notes[0]
            assert "status" in notes[0]

    def test_content_type_json(self, client):
        """Should return JSON content type"""
        response = client.get("/api/notes")
        assert "application/json" in response.headers.get("content-type", "")


# ============================================================================
# Get Note Tests
# ============================================================================

class TestGetNote:
    """Tests for GET /api/notes/{note_id}"""

    def test_returns_200_for_existing_note(self, client):
        """Should return 200 for existing note"""
        response = client.get("/api/notes/test-note")
        assert response.status_code == 200

    def test_returns_note_object(self, client):
        """Should return Note object"""
        response = client.get("/api/notes/test-note")
        note = response.json()

        assert "id" in note
        assert "title" in note
        assert "content" in note
        assert "frontmatter" in note

    def test_returns_404_for_missing_note(self, client, mock_knowledge_store):
        """Should return 404 for non-existent note"""
        mock_knowledge_store.get_note.return_value = None

        response = client.get("/api/notes/nonexistent")
        assert response.status_code == 404

    def test_404_has_detail(self, client, mock_knowledge_store):
        """404 response should include detail"""
        mock_knowledge_store.get_note.return_value = None

        response = client.get("/api/notes/nonexistent")
        assert "detail" in response.json()

    def test_url_encoding_special_chars(self, client):
        """Should handle URL-encoded special characters"""
        response = client.get("/api/notes/note%20with%20spaces")
        # Should not error on URL encoding
        assert response.status_code in [200, 404]


# ============================================================================
# Create Note Tests
# ============================================================================

class TestCreateNote:
    """Tests for POST /api/notes"""

    def test_returns_201_on_success(self, client):
        """Should return 201 Created"""
        response = client.post(
            "/api/notes",
            json={"title": "New Note", "content": "Content", "status": "draft", "tags": []}
        )
        assert response.status_code == 201

    def test_returns_created_note(self, client):
        """Should return the created note"""
        response = client.post(
            "/api/notes",
            json={"title": "New Note", "content": "Content", "status": "draft", "tags": []}
        )
        note = response.json()

        assert "id" in note
        assert "title" in note

    def test_requires_title(self, client):
        """Should require title field"""
        response = client.post(
            "/api/notes",
            json={"content": "Content only"}
        )
        assert response.status_code == 422

    def test_validates_status(self, client):
        """Should validate status field"""
        response = client.post(
            "/api/notes",
            json={"title": "Test", "status": "invalid-status"}
        )
        assert response.status_code == 422

    def test_returns_409_on_duplicate(self, client, mock_knowledge_store):
        """Should return 409 Conflict for duplicate note"""
        mock_knowledge_store.create_note.side_effect = FileExistsError()

        response = client.post(
            "/api/notes",
            json={"title": "Existing Note", "content": "", "status": "draft", "tags": []}
        )
        assert response.status_code == 409

    def test_accepts_json_body(self, client):
        """Should accept JSON body"""
        response = client.post(
            "/api/notes",
            content='{"title": "Test", "content": "", "status": "draft", "tags": []}',
            headers={"Content-Type": "application/json"}
        )
        assert response.status_code == 201


# ============================================================================
# Update Note Tests
# ============================================================================

class TestUpdateNote:
    """Tests for PUT /api/notes/{note_id}"""

    def test_returns_200_on_success(self, client):
        """Should return 200 OK on success"""
        response = client.put(
            "/api/notes/test-note",
            json={"title": "Updated Title"}
        )
        assert response.status_code == 200

    def test_returns_updated_note(self, client):
        """Should return the updated note"""
        response = client.put(
            "/api/notes/test-note",
            json={"title": "Updated Title"}
        )
        note = response.json()

        assert "id" in note
        assert "title" in note

    def test_returns_404_for_missing_note(self, client, mock_knowledge_store):
        """Should return 404 for non-existent note"""
        mock_knowledge_store.update_note.return_value = None

        response = client.put(
            "/api/notes/nonexistent",
            json={"title": "Updated"}
        )
        assert response.status_code == 404

    def test_validates_status(self, client):
        """Should validate status field in update"""
        response = client.put(
            "/api/notes/test-note",
            json={"status": "invalid"}
        )
        assert response.status_code == 422

    def test_partial_update(self, client):
        """Should allow partial updates"""
        response = client.put(
            "/api/notes/test-note",
            json={"content": "New content only"}
        )
        assert response.status_code == 200


# ============================================================================
# Delete Note Tests
# ============================================================================

class TestDeleteNote:
    """Tests for DELETE /api/notes/{note_id}"""

    def test_returns_204_on_success(self, client):
        """Should return 204 No Content"""
        response = client.delete("/api/notes/test-note")
        assert response.status_code == 204

    def test_returns_empty_body(self, client):
        """Should return empty body"""
        response = client.delete("/api/notes/test-note")
        assert response.content == b""

    def test_returns_404_for_missing_note(self, client, mock_knowledge_store):
        """Should return 404 for non-existent note"""
        mock_knowledge_store.delete_note.return_value = False

        response = client.delete("/api/notes/nonexistent")
        assert response.status_code == 404


# ============================================================================
# Reindex Tests
# ============================================================================

class TestReindexNotes:
    """Tests for POST /api/notes/reindex"""

    def test_returns_200(self, client):
        """Should return 200 OK"""
        response = client.post("/api/notes/reindex")
        assert response.status_code == 200

    def test_returns_index_count(self, client):
        """Should return indexed count"""
        response = client.post("/api/notes/reindex")
        data = response.json()

        assert "indexed" in data
        assert "message" in data

    def test_calls_vector_search_index(self, client, mock_vector_search):
        """Should call vector search index_all"""
        client.post("/api/notes/reindex")
        mock_vector_search.index_all.assert_called()

    def test_calls_graph_build_index(self, client, mock_graph_index):
        """Should call graph index build_index"""
        client.post("/api/notes/reindex")
        mock_graph_index.build_index.assert_called()
