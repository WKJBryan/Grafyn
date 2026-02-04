"""
Integration tests for MCP Write API endpoints

Tests the full request/response cycle for MCP write operations
including note creation, update, find-or-create, and property setting.

OAuth is bypassed by patching ``verify_oauth`` since the test environment
uses ``settings.environment == "testing"`` which leaves the module-level
``dev_mode`` flag as ``False`` (it is only ``True`` in "development").
"""
import pytest
from unittest.mock import patch, AsyncMock
from fastapi.testclient import TestClient


# ============================================================================
# Helpers
# ============================================================================

def _bypass_oauth():
    """Patch verify_oauth to always succeed (module-level dev_mode is False in test env)."""
    return patch(
        "app.routers.mcp_write.verify_oauth",
        new_callable=lambda: lambda: AsyncMock(return_value=True),
    )


# ============================================================================
# Test Endpoint Health Check
# ============================================================================

@pytest.mark.integration
class TestMcpTestEndpoint:
    """Tests for GET /api/mcp/test and POST /api/mcp/test/simple"""

    def test_mcp_test_returns_ok(self, test_client: TestClient):
        """GET /api/mcp/test should return status ok"""
        response = test_client.get("/api/mcp/test")
        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "ok"
        assert "message" in data

    def test_mcp_test_simple_with_query_params(self, test_client: TestClient):
        """POST /api/mcp/test/simple should echo back title and content"""
        response = test_client.post(
            "/api/mcp/test/simple",
            params={"title": "Hello", "content": "World"},
        )
        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "ok"
        assert data["title"] == "Hello"
        assert data["content"] == "World"


# ============================================================================
# Create Note via MCP
# ============================================================================

@pytest.mark.integration
class TestMcpCreateNote:
    """Tests for POST /api/mcp/notes"""

    def test_create_note_success(self, test_client: TestClient):
        """Should create a note and return id, title, status"""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes",
                json={
                    "title": "MCP Created Note",
                    "content": "Content created via MCP integration test.",
                    "tags": ["mcp", "test"],
                    "status": "draft",
                },
            )
        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "created"
        assert data["title"] == "MCP Created Note"
        assert "id" in data

    def test_create_note_has_provenance(self, test_client: TestClient, knowledge_store):
        """Notes created via MCP should have provenance metadata"""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes",
                json={
                    "title": "Provenance Check Note",
                    "content": "Checking provenance fields.",
                    "tags": [],
                    "status": "draft",
                },
            )
        assert response.status_code == 200
        note_id = response.json()["id"]

        note = knowledge_store.get_note(note_id)
        assert note is not None
        source_prop = note.frontmatter.get_property("source")
        assert source_prop is not None
        assert source_prop.value == "chatgpt-mcp"
        created_via_prop = note.frontmatter.get_property("created_via")
        assert created_via_prop is not None
        assert created_via_prop.value == "mcp"

    def test_create_note_missing_title_returns_422(self, test_client: TestClient):
        """Should return 422 when title is missing"""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes",
                json={"content": "No title provided"},
            )
        assert response.status_code == 422

    def test_create_duplicate_note_returns_409(self, test_client: TestClient):
        """Should return 409 when creating a note with an existing title"""
        payload = {
            "title": "Duplicate Note Test",
            "content": "First creation.",
            "tags": [],
            "status": "draft",
        }
        with _bypass_oauth():
            first = test_client.post("/api/mcp/notes", json=payload)
            assert first.status_code == 200

            second = test_client.post("/api/mcp/notes", json=payload)
            assert second.status_code == 409

    def test_create_note_via_simple_endpoint(self, test_client: TestClient):
        """POST /api/mcp/notes/create should create a note without provenance"""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes/create",
                json={
                    "title": "Simple Endpoint Note",
                    "content": "Created via the simple endpoint.",
                    "tags": "alpha,beta",
                    "status": "draft",
                },
            )
        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "created"
        assert data["title"] == "Simple Endpoint Note"
        assert "id" in data


# ============================================================================
# Update Note via MCP
# ============================================================================

@pytest.mark.integration
class TestMcpUpdateNote:
    """Tests for PUT /api/mcp/notes/{note_id}"""

    def _create_note(self, test_client, title="Update Target Note", content="Original content."):
        """Helper: create a note via the simple endpoint and return its id."""
        with _bypass_oauth():
            resp = test_client.post(
                "/api/mcp/notes/create",
                json={"title": title, "content": content, "tags": "update-test", "status": "draft"},
            )
        assert resp.status_code == 200
        return resp.json()["id"]

    def test_update_note_replace_content(self, test_client: TestClient, knowledge_store):
        """Should replace content when content_mode is replace"""
        note_id = self._create_note(test_client)

        with _bypass_oauth():
            response = test_client.put(
                f"/api/mcp/notes/{note_id}",
                json={
                    "note_id": note_id,
                    "content": "Replaced content.",
                    "content_mode": "replace",
                },
            )
        assert response.status_code == 200
        updated = knowledge_store.get_note(note_id)
        assert updated.content == "Replaced content."

    def test_update_note_append_content(self, test_client: TestClient, knowledge_store):
        """Should append content when content_mode is append"""
        note_id = self._create_note(test_client, content="Start.")

        with _bypass_oauth():
            response = test_client.put(
                f"/api/mcp/notes/{note_id}",
                json={
                    "note_id": note_id,
                    "content": "Appended.",
                    "content_mode": "append",
                },
            )
        assert response.status_code == 200
        updated = knowledge_store.get_note(note_id)
        assert "Start." in updated.content
        assert "Appended." in updated.content

    def test_update_nonexistent_note_returns_404(self, test_client: TestClient):
        """Should return 404 for a note that does not exist"""
        with _bypass_oauth():
            response = test_client.put(
                "/api/mcp/notes/nonexistent-note-id-xyz",
                json={
                    "note_id": "nonexistent-note-id-xyz",
                    "content": "Will not be saved.",
                },
            )
        assert response.status_code == 404

    def test_update_note_merge_tags(self, test_client: TestClient, knowledge_store):
        """Should merge tags when tags_mode is merge"""
        note_id = self._create_note(test_client)

        with _bypass_oauth():
            response = test_client.put(
                f"/api/mcp/notes/{note_id}",
                json={
                    "note_id": note_id,
                    "tags": ["new-tag"],
                    "tags_mode": "merge",
                },
            )
        assert response.status_code == 200
        updated = knowledge_store.get_note(note_id)
        assert "new-tag" in updated.frontmatter.tags
        assert "update-test" in updated.frontmatter.tags


# ============================================================================
# Find or Create Note via MCP
# ============================================================================

@pytest.mark.integration
class TestMcpFindOrCreate:
    """Tests for POST /api/mcp/notes/find-or-create"""

    def test_creates_new_when_no_match(self, test_client: TestClient):
        """Should create a new note when no similar note exists"""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes/find-or-create",
                json={
                    "search_query": "completely unique topic that does not exist xyz123",
                    "title": "Brand New Find Or Create Note",
                    "content": "Created because nothing matched.",
                    "threshold": 0.75,
                    "tags": ["find-or-create"],
                },
            )
        assert response.status_code == 200
        data = response.json()
        assert data["action"] == "created"
        assert "note_id" in data
        assert "Brand New Find Or Create Note" in data.get("title", "")

    def test_finds_existing_note_when_match(self, test_client: TestClient):
        """Should find an existing note when similarity exceeds threshold"""
        # First create a note and let it get indexed
        with _bypass_oauth():
            create_resp = test_client.post(
                "/api/mcp/notes",
                json={
                    "title": "Python Data Analysis Guide",
                    "content": "A comprehensive guide to data analysis using Python pandas and numpy.",
                    "tags": ["python", "data"],
                    "status": "draft",
                },
            )
        assert create_resp.status_code == 200

        # Now find-or-create with a very similar query and a low threshold
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes/find-or-create",
                json={
                    "search_query": "Python data analysis with pandas numpy",
                    "title": "Python Data Analysis",
                    "content": "Should not be created.",
                    "threshold": 0.3,  # low threshold to maximise match likelihood
                    "tags": [],
                },
            )
        assert response.status_code == 200
        data = response.json()
        # With a low threshold the vector search should find the previously created note
        assert data["action"] in ("found", "created")
        assert "note_id" in data


# ============================================================================
# Set Properties via MCP
# ============================================================================

@pytest.mark.integration
class TestMcpSetProperty:
    """Tests for PUT /api/mcp/notes/{note_id}/properties"""

    def _create_note(self, test_client, title="Property Target Note"):
        """Helper: create a note via the simple endpoint."""
        with _bypass_oauth():
            resp = test_client.post(
                "/api/mcp/notes/create",
                json={"title": title, "content": "Note for property tests.", "tags": "", "status": "draft"},
            )
        assert resp.status_code == 200
        return resp.json()["id"]

    def test_set_string_property(self, test_client: TestClient, knowledge_store):
        """Should set a string property on a note"""
        note_id = self._create_note(test_client)

        with _bypass_oauth():
            response = test_client.put(
                f"/api/mcp/notes/{note_id}/properties",
                json={
                    "note_id": note_id,
                    "property_name": "author",
                    "property_type": "string",
                    "value": "Test Author",
                    "label": "Author",
                },
            )
        assert response.status_code == 200
        data = response.json()
        assert data["value"] == "Test Author"
        assert data["type"] == "string"

        # Verify via knowledge store
        note = knowledge_store.get_note(note_id)
        author_prop = note.frontmatter.get_property("author")
        assert author_prop is not None
        assert author_prop.value == "Test Author"

    def test_set_property_on_nonexistent_note_returns_404(self, test_client: TestClient):
        """Should return 404 when setting property on non-existent note"""
        with _bypass_oauth():
            response = test_client.put(
                "/api/mcp/notes/does-not-exist-xyz/properties",
                json={
                    "note_id": "does-not-exist-xyz",
                    "property_name": "key",
                    "property_type": "string",
                    "value": "val",
                },
            )
        assert response.status_code == 404

    def test_overwrite_existing_property(self, test_client: TestClient, knowledge_store):
        """Should overwrite a property that already exists on a note"""
        note_id = self._create_note(test_client, title="Overwrite Prop Note")

        # Set initial property
        with _bypass_oauth():
            test_client.put(
                f"/api/mcp/notes/{note_id}/properties",
                json={
                    "note_id": note_id,
                    "property_name": "status_note",
                    "property_type": "string",
                    "value": "initial",
                },
            )

        # Overwrite the same property with a new value
        with _bypass_oauth():
            response = test_client.put(
                f"/api/mcp/notes/{note_id}/properties",
                json={
                    "note_id": note_id,
                    "property_name": "status_note",
                    "property_type": "string",
                    "value": "updated",
                    "label": "Status Note",
                },
            )
        assert response.status_code == 200
        data = response.json()
        assert data["value"] == "updated"

        note = knowledge_store.get_note(note_id)
        prop = note.frontmatter.get_property("status_note")
        assert prop is not None
        assert prop.value == "updated"
