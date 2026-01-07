"""
Integration tests for Graph API endpoints

Tests knowledge graph operations: backlinks, outgoing links, neighbors
"""
import pytest
from fastapi.testclient import TestClient
from unittest.mock import MagicMock

from app.main import create_app
from app.models.note import BacklinkInfo, NoteListItem


# ============================================================================
# Test Fixtures
# ============================================================================

@pytest.fixture
def mock_knowledge_store():
    """Create mock knowledge store"""
    store = MagicMock()
    store.list_notes.return_value = [
        NoteListItem(id="note-1", title="Note 1"),
        NoteListItem(id="note-2", title="Note 2"),
        NoteListItem(id="note-3", title="Note 3")
    ]
    return store


@pytest.fixture
def mock_graph_index():
    """Create mock graph index service"""
    graph = MagicMock()

    # Sample backlinks
    graph.get_backlinks_with_context.return_value = [
        BacklinkInfo(
            note_id="source-1",
            title="Source Note 1",
            context="...links to [[Target Note]]..."
        ),
        BacklinkInfo(
            note_id="source-2",
            title="Source Note 2",
            context="...also references [[Target Note]]..."
        )
    ]

    # Sample outgoing links
    graph.get_outgoing_links.return_value = ["linked-1", "linked-2", "linked-3"]

    # Sample neighbors
    graph.get_neighbors.return_value = {
        "nodes": ["center", "neighbor-1", "neighbor-2"],
        "edges": [["center", "neighbor-1"], ["center", "neighbor-2"]]
    }

    # Sample unlinked mentions
    graph.find_unlinked_mentions.return_value = [
        {"note_id": "mention-1", "title": "Mention Note", "context": "...mentions target..."}
    ]

    # Full graph
    graph.get_full_graph.return_value = {
        "nodes": ["note-1", "note-2", "note-3"],
        "edges": [["note-1", "note-2"], ["note-2", "note-3"]]
    }

    graph.build_index.return_value = None

    return graph


@pytest.fixture
def test_app(mock_knowledge_store, mock_graph_index):
    """Create test application with mocked services"""
    app = create_app()
    app.state.knowledge_store = mock_knowledge_store
    app.state.graph_index = mock_graph_index
    return app


@pytest.fixture
def client(test_app):
    """Create test client"""
    return TestClient(test_app)


# ============================================================================
# Get Backlinks Tests
# ============================================================================

class TestGetBacklinks:
    """Tests for GET /api/graph/backlinks/{note_id}"""

    def test_returns_200(self, client):
        """Should return 200 OK"""
        response = client.get("/api/graph/backlinks/test-note")
        assert response.status_code == 200

    def test_returns_list(self, client):
        """Should return a list of backlinks"""
        response = client.get("/api/graph/backlinks/test-note")
        assert isinstance(response.json(), list)

    def test_backlinks_have_required_fields(self, client):
        """Backlinks should have required fields"""
        response = client.get("/api/graph/backlinks/test-note")
        backlinks = response.json()

        if backlinks:
            backlink = backlinks[0]
            assert "note_id" in backlink
            assert "title" in backlink
            assert "context" in backlink

    def test_returns_empty_list_for_no_backlinks(self, client, mock_graph_index):
        """Should return empty list when no backlinks"""
        mock_graph_index.get_backlinks_with_context.return_value = []

        response = client.get("/api/graph/backlinks/isolated-note")
        assert response.json() == []

    def test_handles_url_encoded_note_id(self, client):
        """Should handle URL-encoded note IDs"""
        response = client.get("/api/graph/backlinks/note%20with%20spaces")
        assert response.status_code == 200


# ============================================================================
# Get Outgoing Links Tests
# ============================================================================

class TestGetOutgoingLinks:
    """Tests for GET /api/graph/outgoing/{note_id}"""

    def test_returns_200(self, client):
        """Should return 200 OK"""
        response = client.get("/api/graph/outgoing/test-note")
        assert response.status_code == 200

    def test_returns_list(self, client):
        """Should return a list of note IDs"""
        response = client.get("/api/graph/outgoing/test-note")
        assert isinstance(response.json(), list)

    def test_returns_string_ids(self, client):
        """Outgoing links should be string IDs"""
        response = client.get("/api/graph/outgoing/test-note")
        links = response.json()

        if links:
            assert all(isinstance(link, str) for link in links)

    def test_returns_empty_list_for_no_links(self, client, mock_graph_index):
        """Should return empty list when no outgoing links"""
        mock_graph_index.get_outgoing_links.return_value = []

        response = client.get("/api/graph/outgoing/isolated-note")
        assert response.json() == []


# ============================================================================
# Get Neighbors Tests
# ============================================================================

class TestGetNeighbors:
    """Tests for GET /api/graph/neighbors/{note_id}"""

    def test_returns_200(self, client):
        """Should return 200 OK"""
        response = client.get("/api/graph/neighbors/test-note")
        assert response.status_code == 200

    def test_returns_dict(self, client):
        """Should return a dictionary"""
        response = client.get("/api/graph/neighbors/test-note")
        assert isinstance(response.json(), dict)

    def test_respects_depth_parameter(self, client, mock_graph_index):
        """Should pass depth to graph service"""
        client.get("/api/graph/neighbors/test-note?depth=2")
        mock_graph_index.get_neighbors.assert_called_with("test-note", 2)

    def test_depth_default(self, client, mock_graph_index):
        """Default depth should be 1"""
        client.get("/api/graph/neighbors/test-note")
        mock_graph_index.get_neighbors.assert_called_with("test-note", 1)

    def test_depth_min_value(self, client):
        """Depth must be at least 1"""
        response = client.get("/api/graph/neighbors/test-note?depth=0")
        assert response.status_code == 422

    def test_depth_max_value(self, client):
        """Depth must not exceed 3"""
        response = client.get("/api/graph/neighbors/test-note?depth=4")
        assert response.status_code == 422

    def test_depth_1_valid(self, client):
        """Depth 1 should be valid"""
        response = client.get("/api/graph/neighbors/test-note?depth=1")
        assert response.status_code == 200

    def test_depth_3_valid(self, client):
        """Depth 3 should be valid"""
        response = client.get("/api/graph/neighbors/test-note?depth=3")
        assert response.status_code == 200


# ============================================================================
# Find Unlinked Mentions Tests
# ============================================================================

class TestFindUnlinkedMentions:
    """Tests for GET /api/graph/unlinked-mentions/{note_id}"""

    def test_returns_200(self, client):
        """Should return 200 OK"""
        response = client.get("/api/graph/unlinked-mentions/test-note")
        assert response.status_code == 200

    def test_returns_list(self, client):
        """Should return a list"""
        response = client.get("/api/graph/unlinked-mentions/test-note")
        assert isinstance(response.json(), list)

    def test_mentions_have_required_fields(self, client):
        """Mentions should have required fields"""
        response = client.get("/api/graph/unlinked-mentions/test-note")
        mentions = response.json()

        if mentions:
            mention = mentions[0]
            assert "note_id" in mention
            assert "title" in mention

    def test_returns_empty_for_no_mentions(self, client, mock_graph_index):
        """Should return empty list when no unlinked mentions"""
        mock_graph_index.find_unlinked_mentions.return_value = []

        response = client.get("/api/graph/unlinked-mentions/obscure-note")
        assert response.json() == []


# ============================================================================
# Rebuild Graph Tests
# ============================================================================

class TestRebuildGraph:
    """Tests for POST /api/graph/rebuild"""

    def test_returns_200(self, client):
        """Should return 200 OK"""
        response = client.post("/api/graph/rebuild")
        assert response.status_code == 200

    def test_returns_processed_count(self, client):
        """Should return processed count"""
        response = client.post("/api/graph/rebuild")
        data = response.json()

        assert "processed" in data
        assert "message" in data

    def test_calls_build_index(self, client, mock_graph_index):
        """Should call graph build_index"""
        client.post("/api/graph/rebuild")
        mock_graph_index.build_index.assert_called_once()


# ============================================================================
# Get Full Graph Tests
# ============================================================================

class TestGetFullGraph:
    """Tests for GET /api/graph/full"""

    def test_returns_200(self, client):
        """Should return 200 OK"""
        response = client.get("/api/graph/full")
        assert response.status_code == 200

    def test_returns_dict(self, client):
        """Should return a dictionary"""
        response = client.get("/api/graph/full")
        assert isinstance(response.json(), dict)

    def test_has_nodes_and_edges(self, client):
        """Response should have nodes and edges"""
        response = client.get("/api/graph/full")
        data = response.json()

        assert "nodes" in data
        assert "edges" in data

    def test_nodes_is_list(self, client):
        """Nodes should be a list"""
        response = client.get("/api/graph/full")
        data = response.json()

        assert isinstance(data["nodes"], list)

    def test_edges_is_list(self, client):
        """Edges should be a list"""
        response = client.get("/api/graph/full")
        data = response.json()

        assert isinstance(data["edges"], list)
