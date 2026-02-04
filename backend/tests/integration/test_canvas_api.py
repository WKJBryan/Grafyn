"""
Integration tests for Canvas API endpoints

Tests the full request/response cycle for canvas session CRUD,
tile position updates, tile deletion, and available models listing.
Uses the test_client fixture from conftest.py which provides a
FastAPI TestClient with real services backed by temporary directories.
"""
import pytest
from fastapi.testclient import TestClient


# ============================================================================
# Session CRUD Lifecycle
# ============================================================================


@pytest.mark.integration
class TestCanvasSessionLifecycle:
    """Tests for the full create-read-update-delete lifecycle of canvas sessions."""

    def test_create_session_returns_201(self, test_client: TestClient):
        """POST /api/canvas with valid data returns 201 and the new session."""
        payload = {
            "title": "My Test Canvas",
            "description": "Integration test session",
            "tags": ["test", "integration"],
        }
        response = test_client.post("/api/canvas", json=payload)

        assert response.status_code == 201
        data = response.json()
        assert data["title"] == "My Test Canvas"
        assert data["description"] == "Integration test session"
        assert data["tags"] == ["test", "integration"]
        assert "id" in data
        assert "created_at" in data
        assert "updated_at" in data
        assert data["status"] == "draft"
        assert data["prompt_tiles"] == []
        assert data["debates"] == []

    def test_create_session_with_defaults(self, test_client: TestClient):
        """POST /api/canvas with minimal data uses default title."""
        response = test_client.post("/api/canvas", json={})

        assert response.status_code == 201
        data = response.json()
        assert data["title"] == "Untitled Canvas"
        assert data["description"] is None
        assert data["tags"] == []

    def test_list_sessions_empty(self, test_client: TestClient):
        """GET /api/canvas returns empty list when no sessions exist."""
        response = test_client.get("/api/canvas")

        assert response.status_code == 200
        data = response.json()
        assert isinstance(data, list)
        assert len(data) == 0

    def test_list_sessions_after_create(self, test_client: TestClient):
        """GET /api/canvas returns created sessions as summary items."""
        # Create two sessions
        test_client.post(
            "/api/canvas",
            json={"title": "First Canvas", "tags": ["alpha"]},
        )
        test_client.post(
            "/api/canvas",
            json={"title": "Second Canvas", "tags": ["beta"]},
        )

        response = test_client.get("/api/canvas")
        assert response.status_code == 200
        data = response.json()
        assert len(data) == 2

        # List items should have summary fields
        titles = {item["title"] for item in data}
        assert "First Canvas" in titles
        assert "Second Canvas" in titles
        for item in data:
            assert "tile_count" in item
            assert "debate_count" in item
            assert "created_at" in item

    def test_get_session_by_id(self, test_client: TestClient):
        """GET /api/canvas/{id} returns the full session object."""
        create_resp = test_client.post(
            "/api/canvas",
            json={"title": "Retrievable Canvas", "description": "desc"},
        )
        session_id = create_resp.json()["id"]

        response = test_client.get(f"/api/canvas/{session_id}")
        assert response.status_code == 200
        data = response.json()
        assert data["id"] == session_id
        assert data["title"] == "Retrievable Canvas"
        assert data["description"] == "desc"

    def test_get_nonexistent_session_returns_404(self, test_client: TestClient):
        """GET /api/canvas/{id} with unknown id returns 404."""
        response = test_client.get("/api/canvas/nonexistent-id-12345")

        assert response.status_code == 404
        assert response.json()["detail"] == "Session not found"

    def test_update_session(self, test_client: TestClient):
        """PUT /api/canvas/{id} updates metadata and returns updated session."""
        create_resp = test_client.post(
            "/api/canvas",
            json={"title": "Original Title", "tags": ["old"]},
        )
        session_id = create_resp.json()["id"]

        update_payload = {
            "title": "Updated Title",
            "description": "Now with description",
            "tags": ["new", "updated"],
            "status": "canonical",
        }
        response = test_client.put(f"/api/canvas/{session_id}", json=update_payload)

        assert response.status_code == 200
        data = response.json()
        assert data["title"] == "Updated Title"
        assert data["description"] == "Now with description"
        assert data["tags"] == ["new", "updated"]
        assert data["status"] == "canonical"

    def test_update_session_partial(self, test_client: TestClient):
        """PUT /api/canvas/{id} with partial data updates only provided fields."""
        create_resp = test_client.post(
            "/api/canvas",
            json={"title": "Keep This Title", "description": "Original desc"},
        )
        session_id = create_resp.json()["id"]

        response = test_client.put(
            f"/api/canvas/{session_id}",
            json={"description": "New description only"},
        )

        assert response.status_code == 200
        data = response.json()
        assert data["title"] == "Keep This Title"
        assert data["description"] == "New description only"

    def test_update_nonexistent_session_returns_404(self, test_client: TestClient):
        """PUT /api/canvas/{id} with unknown id returns 404."""
        response = test_client.put(
            "/api/canvas/nonexistent-id",
            json={"title": "Won't Work"},
        )
        assert response.status_code == 404

    def test_delete_session(self, test_client: TestClient):
        """DELETE /api/canvas/{id} removes the session and returns 204."""
        create_resp = test_client.post(
            "/api/canvas",
            json={"title": "To Be Deleted"},
        )
        session_id = create_resp.json()["id"]

        response = test_client.delete(f"/api/canvas/{session_id}")
        assert response.status_code == 204

        # Confirm it is gone
        get_resp = test_client.get(f"/api/canvas/{session_id}")
        assert get_resp.status_code == 404

    def test_delete_nonexistent_session_returns_404(self, test_client: TestClient):
        """DELETE /api/canvas/{id} with unknown id returns 404."""
        response = test_client.delete("/api/canvas/nonexistent-id")
        assert response.status_code == 404

    def test_full_crud_lifecycle(self, test_client: TestClient):
        """Exercise the complete create -> read -> update -> delete cycle."""
        # Create
        create_resp = test_client.post(
            "/api/canvas",
            json={"title": "Lifecycle Test", "tags": ["lifecycle"]},
        )
        assert create_resp.status_code == 201
        session_id = create_resp.json()["id"]

        # Read
        get_resp = test_client.get(f"/api/canvas/{session_id}")
        assert get_resp.status_code == 200
        assert get_resp.json()["title"] == "Lifecycle Test"

        # Update
        put_resp = test_client.put(
            f"/api/canvas/{session_id}",
            json={"title": "Lifecycle Test Updated", "status": "evidence"},
        )
        assert put_resp.status_code == 200
        assert put_resp.json()["title"] == "Lifecycle Test Updated"
        assert put_resp.json()["status"] == "evidence"

        # List (should contain the session)
        list_resp = test_client.get("/api/canvas")
        assert list_resp.status_code == 200
        ids = [s["id"] for s in list_resp.json()]
        assert session_id in ids

        # Delete
        del_resp = test_client.delete(f"/api/canvas/{session_id}")
        assert del_resp.status_code == 204

        # Confirm gone
        gone_resp = test_client.get(f"/api/canvas/{session_id}")
        assert gone_resp.status_code == 404


# ============================================================================
# Available Models Endpoint
# ============================================================================


@pytest.mark.integration
class TestAvailableModels:
    """Tests for GET /api/canvas/models/available."""

    def test_list_available_models(self, test_client: TestClient):
        """GET /api/canvas/models/available returns cached model list."""
        response = test_client.get("/api/canvas/models/available")

        assert response.status_code == 200
        data = response.json()
        assert isinstance(data, list)
        assert len(data) == 2  # mock_openrouter_client has 2 cached models

        model_ids = {m["id"] for m in data}
        assert "anthropic/claude-3.5-sonnet" in model_ids
        assert "openai/gpt-4o" in model_ids

        # Check model fields
        for model in data:
            assert "id" in model
            assert "name" in model
            assert "provider" in model
            assert "context_length" in model
            assert "supports_streaming" in model


# ============================================================================
# Tile Position Updates
# ============================================================================


@pytest.mark.integration
class TestTilePositionUpdates:
    """Tests for tile and LLM node position update endpoints."""

    def _create_session_with_tile(self, test_client: TestClient):
        """Helper: create a session, add a prompt tile via the store, return ids."""
        create_resp = test_client.post(
            "/api/canvas",
            json={"title": "Position Test Canvas"},
        )
        session_id = create_resp.json()["id"]

        # Add a tile directly through the canvas store (attached to app.state)
        from app.main import app

        store = app.state.canvas_store
        tile = store.add_prompt_tile(
            session_id,
            prompt="Test prompt",
            models=["openai/gpt-4o"],
        )
        return session_id, tile.id, tile

    def test_update_tile_position(self, test_client: TestClient):
        """PUT /api/canvas/{sid}/tiles/{tid}/position updates position."""
        session_id, tile_id, _ = self._create_session_with_tile(test_client)

        payload = {"x": 100.0, "y": 200.0, "width": 300.0, "height": 250.0}
        response = test_client.put(
            f"/api/canvas/{session_id}/tiles/{tile_id}/position",
            json=payload,
        )

        assert response.status_code == 200
        assert response.json()["status"] == "updated"

        # Verify position was persisted
        get_resp = test_client.get(f"/api/canvas/{session_id}")
        tile_data = get_resp.json()["prompt_tiles"][0]
        assert tile_data["position"]["x"] == 100.0
        assert tile_data["position"]["y"] == 200.0
        assert tile_data["position"]["width"] == 300.0
        assert tile_data["position"]["height"] == 250.0

    def test_update_tile_position_without_size(self, test_client: TestClient):
        """PUT position without width/height only updates x and y."""
        session_id, tile_id, _ = self._create_session_with_tile(test_client)

        payload = {"x": 50.0, "y": 75.0}
        response = test_client.put(
            f"/api/canvas/{session_id}/tiles/{tile_id}/position",
            json=payload,
        )

        assert response.status_code == 200

        get_resp = test_client.get(f"/api/canvas/{session_id}")
        tile_data = get_resp.json()["prompt_tiles"][0]
        assert tile_data["position"]["x"] == 50.0
        assert tile_data["position"]["y"] == 75.0

    def test_update_tile_position_nonexistent_tile(self, test_client: TestClient):
        """PUT position for nonexistent tile returns 404."""
        create_resp = test_client.post(
            "/api/canvas",
            json={"title": "Empty Canvas"},
        )
        session_id = create_resp.json()["id"]

        payload = {"x": 10.0, "y": 20.0}
        response = test_client.put(
            f"/api/canvas/{session_id}/tiles/fake-tile-id/position",
            json=payload,
        )
        assert response.status_code == 404

    def _create_session_with_simple_model_tile(self, test_client: TestClient):
        """Helper: create a session with a tile using a model ID without slashes.

        TestClient (httpx) normalizes %2F back to / in paths, which breaks
        path parameter matching for model IDs like 'openai/gpt-4o'. We use a
        simple model name here to test the position update logic in isolation.
        """
        create_resp = test_client.post(
            "/api/canvas",
            json={"title": "LLM Position Test"},
        )
        session_id = create_resp.json()["id"]

        from app.main import app

        store = app.state.canvas_store
        tile = store.add_prompt_tile(
            session_id,
            prompt="Test prompt for LLM node",
            models=["claude-sonnet"],
        )
        return session_id, tile.id, tile

    def test_update_llm_node_position(self, test_client: TestClient):
        """PUT /api/canvas/{sid}/tiles/{tid}/responses/{mid}/position updates LLM node."""
        session_id, tile_id, tile = self._create_session_with_simple_model_tile(
            test_client
        )
        model_id = "claude-sonnet"

        payload = {"x": 500.0, "y": 600.0, "width": 350.0, "height": 280.0}
        response = test_client.put(
            f"/api/canvas/{session_id}/tiles/{tile_id}/responses/{model_id}/position",
            json=payload,
        )

        assert response.status_code == 200
        assert response.json()["status"] == "updated"

        # Verify persisted position
        get_resp = test_client.get(f"/api/canvas/{session_id}")
        responses = get_resp.json()["prompt_tiles"][0]["responses"]
        assert model_id in responses
        assert responses[model_id]["position"]["x"] == 500.0
        assert responses[model_id]["position"]["y"] == 600.0
        assert responses[model_id]["position"]["width"] == 350.0
        assert responses[model_id]["position"]["height"] == 280.0

    def test_update_llm_node_position_nonexistent(self, test_client: TestClient):
        """PUT LLM node position for nonexistent node returns 404."""
        session_id, tile_id, _ = self._create_session_with_simple_model_tile(
            test_client
        )

        payload = {"x": 10.0, "y": 20.0}
        response = test_client.put(
            f"/api/canvas/{session_id}/tiles/{tile_id}/responses/fake-model/position",
            json=payload,
        )
        assert response.status_code == 404


# ============================================================================
# Tile Deletion
# ============================================================================


@pytest.mark.integration
class TestTileDeletion:
    """Tests for DELETE /api/canvas/{session_id}/tiles/{tile_id}."""

    def test_delete_tile(self, test_client: TestClient):
        """DELETE tile returns 204 and removes it from the session."""
        create_resp = test_client.post(
            "/api/canvas",
            json={"title": "Tile Delete Test"},
        )
        session_id = create_resp.json()["id"]

        from app.main import app

        store = app.state.canvas_store
        tile = store.add_prompt_tile(
            session_id,
            prompt="Delete me",
            models=["openai/gpt-4o"],
        )

        response = test_client.delete(
            f"/api/canvas/{session_id}/tiles/{tile.id}"
        )
        assert response.status_code == 204

        # Verify the tile is removed
        get_resp = test_client.get(f"/api/canvas/{session_id}")
        assert len(get_resp.json()["prompt_tiles"]) == 0

    def test_delete_tile_nonexistent_returns_404(self, test_client: TestClient):
        """DELETE nonexistent tile returns 404."""
        create_resp = test_client.post(
            "/api/canvas",
            json={"title": "No Tiles Here"},
        )
        session_id = create_resp.json()["id"]

        response = test_client.delete(
            f"/api/canvas/{session_id}/tiles/nonexistent-tile"
        )
        assert response.status_code == 404

    def test_delete_tile_nonexistent_session(self, test_client: TestClient):
        """DELETE tile on nonexistent session returns 404."""
        response = test_client.delete(
            "/api/canvas/fake-session/tiles/fake-tile"
        )
        assert response.status_code == 404


# ============================================================================
# Batch Arrange
# ============================================================================


@pytest.mark.integration
class TestBatchArrange:
    """Tests for POST /api/canvas/{session_id}/arrange."""

    def test_batch_arrange_nodes(self, test_client: TestClient):
        """POST arrange with valid positions returns success with node count."""
        create_resp = test_client.post(
            "/api/canvas",
            json={"title": "Arrange Test"},
        )
        session_id = create_resp.json()["id"]

        from app.main import app

        store = app.state.canvas_store
        tile = store.add_prompt_tile(
            session_id,
            prompt="Arrange me",
            models=["openai/gpt-4o"],
        )

        prompt_node_id = f"prompt:{tile.id}"
        llm_node_id = f"llm:{tile.id}:openai/gpt-4o"

        payload = {
            "positions": {
                prompt_node_id: {"x": 0.0, "y": 0.0},
                llm_node_id: {"x": 400.0, "y": 0.0},
            }
        }

        response = test_client.post(
            f"/api/canvas/{session_id}/arrange",
            json=payload,
        )

        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "arranged"
        assert data["node_count"] == 2

    def test_batch_arrange_nonexistent_session(self, test_client: TestClient):
        """POST arrange on nonexistent session returns 404."""
        payload = {"positions": {"prompt:fake": {"x": 0, "y": 0}}}
        response = test_client.post(
            "/api/canvas/nonexistent-session/arrange",
            json=payload,
        )
        assert response.status_code == 404
