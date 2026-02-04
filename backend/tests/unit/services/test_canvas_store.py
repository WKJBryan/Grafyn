"""Unit tests for CanvasSessionStore service"""
import json
from pathlib import Path

import pytest

from app.models.canvas import (
    CanvasCreate,
    CanvasUpdate,
    CanvasViewport,
    DebateMode,
    EdgeType,
    TilePosition,
    TilePositionUpdate,
)
from app.services.canvas_store import CanvasSessionStore


# ============================================================================
# Session CRUD
# ============================================================================


@pytest.mark.unit
class TestSessionCRUD:
    """Tests for creating, reading, updating, and deleting sessions."""

    def test_create_session(self, canvas_store, canvas_session_data):
        """Creating a session returns a CanvasSession with correct fields."""
        data = CanvasCreate(**canvas_session_data)
        session = canvas_store.create_session(data)

        assert session.id is not None
        assert session.title == "Test Canvas Session"
        assert session.description == "A session for testing"
        assert session.tags == ["test", "canvas"]
        assert session.status == "draft"
        assert len(session.prompt_tiles) == 0
        assert len(session.debates) == 0
        assert session.created_at is not None
        assert session.updated_at is not None

    def test_get_session(self, canvas_store, canvas_session_data):
        """Getting a session by ID returns the correct session."""
        data = CanvasCreate(**canvas_session_data)
        created = canvas_store.create_session(data)

        fetched = canvas_store.get_session(created.id)
        assert fetched is not None
        assert fetched.id == created.id
        assert fetched.title == created.title

    def test_get_session_not_found(self, canvas_store):
        """Getting a non-existent session returns None."""
        result = canvas_store.get_session("non-existent-id")
        assert result is None

    def test_list_sessions(self, canvas_store):
        """Listing sessions returns summaries sorted by updated_at descending."""
        for i in range(3):
            canvas_store.create_session(
                CanvasCreate(title=f"Session {i}", tags=[f"tag{i}"])
            )

        items = canvas_store.list_sessions()
        assert len(items) == 3
        # Most recently updated first
        assert items[0].title == "Session 2"
        assert items[0].tile_count == 0
        assert items[0].debate_count == 0

    def test_update_session(self, canvas_store, canvas_session_data):
        """Updating a session modifies only the supplied fields."""
        session = canvas_store.create_session(CanvasCreate(**canvas_session_data))
        original_updated = session.updated_at

        updated = canvas_store.update_session(
            session.id,
            CanvasUpdate(title="Renamed Session", status="canonical"),
        )

        assert updated is not None
        assert updated.title == "Renamed Session"
        assert updated.status == "canonical"
        # Description should remain unchanged
        assert updated.description == "A session for testing"
        assert updated.updated_at >= original_updated

    def test_update_session_not_found(self, canvas_store):
        """Updating a non-existent session returns None."""
        result = canvas_store.update_session(
            "no-such-id", CanvasUpdate(title="X")
        )
        assert result is None

    def test_delete_session(self, canvas_store, canvas_session_data):
        """Deleting a session removes it from memory and disk."""
        session = canvas_store.create_session(CanvasCreate(**canvas_session_data))
        session_id = session.id

        assert canvas_store.delete_session(session_id) is True
        assert canvas_store.get_session(session_id) is None
        assert not (canvas_store.data_path / f"{session_id}.json").exists()

    def test_delete_session_not_found(self, canvas_store):
        """Deleting a non-existent session returns False."""
        assert canvas_store.delete_session("no-such-id") is False


# ============================================================================
# Prompt Tile Management
# ============================================================================


@pytest.mark.unit
class TestPromptTiles:
    """Tests for adding, deleting, and positioning prompt tiles."""

    def _make_session(self, canvas_store):
        return canvas_store.create_session(
            CanvasCreate(title="Tile Test Session")
        )

    def test_add_prompt_tile(self, canvas_store):
        """Adding a prompt tile creates responses for each model."""
        session = self._make_session(canvas_store)
        models = ["openai/gpt-4o", "anthropic/claude-3.5-sonnet"]

        tile = canvas_store.add_prompt_tile(
            session.id,
            prompt="What is AI?",
            models=models,
            system_prompt="You are helpful.",
        )

        assert tile is not None
        assert tile.prompt == "What is AI?"
        assert tile.system_prompt == "You are helpful."
        assert set(tile.models) == set(models)
        assert len(tile.responses) == 2
        for model_id, resp in tile.responses.items():
            assert resp.status == "pending"
            assert resp.model_id == model_id
            assert resp.content == ""

    def test_add_prompt_tile_auto_positioning(self, canvas_store):
        """Multiple root tiles are stacked vertically by default."""
        session = self._make_session(canvas_store)

        tile1 = canvas_store.add_prompt_tile(
            session.id, prompt="First", models=["m1"]
        )
        tile2 = canvas_store.add_prompt_tile(
            session.id, prompt="Second", models=["m1"]
        )

        # Second root tile should be offset vertically from the first
        assert tile2.position.y > tile1.position.y

    def test_add_prompt_tile_branching(self, canvas_store):
        """A branched tile records its parent tile and model."""
        session = self._make_session(canvas_store)
        parent = canvas_store.add_prompt_tile(
            session.id, prompt="Root", models=["model-a"]
        )
        # Simulate a completed response so branching position logic works
        canvas_store.update_response_content(
            session.id, parent.id, "model-a", "Root answer", status="completed"
        )

        child = canvas_store.add_prompt_tile(
            session.id,
            prompt="Follow-up",
            models=["model-a"],
            parent_tile_id=parent.id,
            parent_model_id="model-a",
        )

        assert child is not None
        assert child.parent_tile_id == parent.id
        assert child.parent_model_id == "model-a"

    def test_add_prompt_tile_invalid_session(self, canvas_store):
        """Adding a tile to a non-existent session returns None."""
        result = canvas_store.add_prompt_tile(
            "bad-id", prompt="Hello", models=["m1"]
        )
        assert result is None

    def test_delete_tile(self, canvas_store):
        """Deleting a tile removes it from the session."""
        session = self._make_session(canvas_store)
        tile = canvas_store.add_prompt_tile(
            session.id, prompt="Delete me", models=["m1"]
        )

        assert canvas_store.delete_tile(session.id, tile.id) is True

        refreshed = canvas_store.get_session(session.id)
        assert len(refreshed.prompt_tiles) == 0

    def test_delete_tile_cascades_children(self, canvas_store):
        """Deleting a parent tile also removes its child tiles."""
        session = self._make_session(canvas_store)
        parent = canvas_store.add_prompt_tile(
            session.id, prompt="Parent", models=["m1"]
        )
        canvas_store.update_response_content(
            session.id, parent.id, "m1", "Answer", status="completed"
        )
        canvas_store.add_prompt_tile(
            session.id,
            prompt="Child",
            models=["m1"],
            parent_tile_id=parent.id,
            parent_model_id="m1",
        )

        canvas_store.delete_tile(session.id, parent.id)
        refreshed = canvas_store.get_session(session.id)
        assert len(refreshed.prompt_tiles) == 0

    def test_update_tile_position(self, canvas_store):
        """Updating a tile position changes x/y and optionally width/height."""
        session = self._make_session(canvas_store)
        tile = canvas_store.add_prompt_tile(
            session.id, prompt="Move me", models=["m1"]
        )

        result = canvas_store.update_tile_position(
            session.id,
            tile.id,
            TilePositionUpdate(x=500, y=600, width=300, height=250),
        )

        assert result is True
        refreshed = canvas_store.get_session(session.id)
        moved = refreshed.prompt_tiles[0]
        assert moved.position.x == 500
        assert moved.position.y == 600
        assert moved.position.width == 300
        assert moved.position.height == 250


# ============================================================================
# Model Response Management
# ============================================================================


@pytest.mark.unit
class TestModelResponses:
    """Tests for adding models, updating content, and setting errors."""

    def _session_with_tile(self, canvas_store):
        session = canvas_store.create_session(
            CanvasCreate(title="Response Test")
        )
        tile = canvas_store.add_prompt_tile(
            session.id, prompt="Hello", models=["model-a"]
        )
        return session, tile

    def test_add_models_to_tile(self, canvas_store):
        """Adding new models creates pending responses without duplicating existing ones."""
        session, tile = self._session_with_tile(canvas_store)

        result = canvas_store.add_models_to_tile(
            session.id, tile.id, ["model-b", "model-a"]  # model-a already exists
        )

        assert result is True
        refreshed = canvas_store.get_session(session.id)
        t = refreshed.prompt_tiles[0]
        assert "model-b" in t.responses
        assert len(t.responses) == 2  # model-a was not duplicated

    def test_add_models_invalid_session(self, canvas_store):
        """Adding models to a non-existent session returns False."""
        assert canvas_store.add_models_to_tile("nope", "nope", ["m"]) is False

    def test_update_response_content_streaming(self, canvas_store):
        """Streaming content updates the response without setting completed_at."""
        session, tile = self._session_with_tile(canvas_store)

        result = canvas_store.update_response_content(
            session.id, tile.id, "model-a", "Partial...", status="streaming"
        )

        assert result is True
        refreshed = canvas_store.get_session(session.id)
        resp = refreshed.prompt_tiles[0].responses["model-a"]
        assert resp.content == "Partial..."
        assert resp.status == "streaming"
        assert resp.completed_at is None

    def test_update_response_content_completed(self, canvas_store):
        """Completing a response sets completed_at timestamp."""
        session, tile = self._session_with_tile(canvas_store)

        canvas_store.update_response_content(
            session.id, tile.id, "model-a", "Done!", status="completed"
        )

        refreshed = canvas_store.get_session(session.id)
        resp = refreshed.prompt_tiles[0].responses["model-a"]
        assert resp.status == "completed"
        assert resp.completed_at is not None

    def test_set_response_error(self, canvas_store):
        """Setting an error marks the response as error with message."""
        session, tile = self._session_with_tile(canvas_store)

        result = canvas_store.set_response_error(
            session.id, tile.id, "model-a", "Rate limit exceeded"
        )

        assert result is True
        refreshed = canvas_store.get_session(session.id)
        resp = refreshed.prompt_tiles[0].responses["model-a"]
        assert resp.status == "error"
        assert resp.error_message == "Rate limit exceeded"

    def test_update_llm_node_position(self, canvas_store):
        """Updating an LLM node position changes x/y on the response."""
        session, tile = self._session_with_tile(canvas_store)

        result = canvas_store.update_llm_node_position(
            session.id,
            tile.id,
            "model-a",
            TilePositionUpdate(x=999, y=888),
        )

        assert result is True
        refreshed = canvas_store.get_session(session.id)
        pos = refreshed.prompt_tiles[0].responses["model-a"].position
        assert pos.x == 999
        assert pos.y == 888


# ============================================================================
# Debate Management
# ============================================================================


@pytest.mark.unit
class TestDebates:
    """Tests for debates: creation, rounds, and status updates."""

    def _session_with_tiles(self, canvas_store):
        session = canvas_store.create_session(
            CanvasCreate(title="Debate Test")
        )
        tile1 = canvas_store.add_prompt_tile(
            session.id, prompt="Topic A", models=["m1", "m2"]
        )
        canvas_store.update_response_content(
            session.id, tile1.id, "m1", "Answer from m1", status="completed"
        )
        canvas_store.update_response_content(
            session.id, tile1.id, "m2", "Answer from m2", status="completed"
        )
        return session, tile1

    def test_add_debate(self, canvas_store):
        """Adding a debate creates a DebateRound linked to source tiles."""
        session, tile = self._session_with_tiles(canvas_store)

        debate = canvas_store.add_debate(
            session.id,
            source_tile_ids=[tile.id],
            participating_models=["m1", "m2"],
            debate_mode=DebateMode.AUTO,
        )

        assert debate is not None
        assert debate.id is not None
        assert debate.status == "active"
        assert debate.debate_mode == DebateMode.AUTO
        assert debate.source_tile_ids == [tile.id]
        assert set(debate.participating_models) == {"m1", "m2"}

    def test_add_debate_invalid_session(self, canvas_store):
        """Adding a debate to a non-existent session returns None."""
        assert canvas_store.add_debate("bad", [], []) is None

    def test_add_debate_round(self, canvas_store):
        """Adding a round appends model responses to the debate."""
        session, tile = self._session_with_tiles(canvas_store)
        debate = canvas_store.add_debate(
            session.id,
            source_tile_ids=[tile.id],
            participating_models=["m1", "m2"],
        )

        round_data = {"m1": "I agree.", "m2": "I disagree."}
        result = canvas_store.add_debate_round(session.id, debate.id, round_data)

        assert result is True
        refreshed = canvas_store.get_session(session.id)
        d = refreshed.debates[0]
        assert len(d.rounds) == 1
        assert d.rounds[0] == round_data

    def test_update_debate_status(self, canvas_store):
        """Updating debate status changes the status field."""
        session, tile = self._session_with_tiles(canvas_store)
        debate = canvas_store.add_debate(
            session.id,
            source_tile_ids=[tile.id],
            participating_models=["m1", "m2"],
        )

        result = canvas_store.update_debate_status(
            session.id, debate.id, "completed"
        )

        assert result is True
        refreshed = canvas_store.get_session(session.id)
        assert refreshed.debates[0].status == "completed"

    def test_delete_debate_tile(self, canvas_store):
        """Deleting a debate tile removes it from the session."""
        session, tile = self._session_with_tiles(canvas_store)
        debate = canvas_store.add_debate(
            session.id,
            source_tile_ids=[tile.id],
            participating_models=["m1", "m2"],
        )

        result = canvas_store.delete_tile(session.id, debate.id)

        assert result is True
        refreshed = canvas_store.get_session(session.id)
        assert len(refreshed.debates) == 0

    def test_get_tile_responses(self, canvas_store):
        """get_tile_responses returns only completed responses from specified tiles."""
        session, tile = self._session_with_tiles(canvas_store)

        result = canvas_store.get_tile_responses(session.id, [tile.id])

        assert tile.id in result
        assert result[tile.id]["prompt"] == "Topic A"
        # Both m1 and m2 were completed
        assert "m1" in result[tile.id]["responses"]
        assert "m2" in result[tile.id]["responses"]


# ============================================================================
# Edge / Graph Queries
# ============================================================================


@pytest.mark.unit
class TestEdgesAndGraph:
    """Tests for tile edges, node edges, and connected-component discovery."""

    def test_get_tile_edges(self, canvas_store):
        """get_tile_edges returns parent-child tile relationships."""
        session = canvas_store.create_session(
            CanvasCreate(title="Edge Test")
        )
        parent = canvas_store.add_prompt_tile(
            session.id, prompt="Root", models=["m1"]
        )
        canvas_store.update_response_content(
            session.id, parent.id, "m1", "Answer", status="completed"
        )
        child = canvas_store.add_prompt_tile(
            session.id,
            prompt="Branch",
            models=["m1"],
            parent_tile_id=parent.id,
            parent_model_id="m1",
        )

        edges = canvas_store.get_tile_edges(session.id)

        assert len(edges) == 1
        assert edges[0].source_tile_id == parent.id
        assert edges[0].target_tile_id == child.id
        assert edges[0].source_model_id == "m1"

    def test_get_node_edges(self, canvas_store):
        """get_node_edges returns typed edges between prompt, LLM, and debate nodes."""
        session = canvas_store.create_session(
            CanvasCreate(title="Node Edge Test")
        )
        tile = canvas_store.add_prompt_tile(
            session.id, prompt="Q", models=["m1", "m2"]
        )
        canvas_store.update_response_content(
            session.id, tile.id, "m1", "A1", status="completed"
        )
        canvas_store.update_response_content(
            session.id, tile.id, "m2", "A2", status="completed"
        )

        edges = canvas_store.get_node_edges(session.id)

        # Should have 2 prompt->llm edges (one per model)
        prompt_to_llm = [e for e in edges if e.edge_type == EdgeType.PROMPT_TO_LLM]
        assert len(prompt_to_llm) == 2
        source_ids = {e.source_id for e in prompt_to_llm}
        assert source_ids == {f"prompt:{tile.id}"}

    def test_find_node_groups_single_group(self, canvas_store):
        """A single tile with responses forms one connected group."""
        session = canvas_store.create_session(
            CanvasCreate(title="Groups Test")
        )
        canvas_store.add_prompt_tile(
            session.id, prompt="Only tile", models=["m1"]
        )

        groups = canvas_store.find_node_groups(session.id)

        assert len(groups) == 1
        # Group should contain prompt node + llm node
        assert len(groups[0]) == 2

    def test_find_node_groups_disconnected(self, canvas_store):
        """Two independent tiles form two separate groups."""
        session = canvas_store.create_session(
            CanvasCreate(title="Disconnected Test")
        )
        canvas_store.add_prompt_tile(
            session.id, prompt="Tile A", models=["m1"]
        )
        canvas_store.add_prompt_tile(
            session.id, prompt="Tile B", models=["m2"]
        )

        groups = canvas_store.find_node_groups(session.id)

        assert len(groups) == 2


# ============================================================================
# Context Building (Conversation History)
# ============================================================================


@pytest.mark.unit
class TestContextBuilding:
    """Tests for build_full_history and build_compact_history."""

    def _chain_of_three(self, canvas_store):
        """Create a 3-tile parent chain: root -> mid -> leaf."""
        session = canvas_store.create_session(
            CanvasCreate(title="History Test")
        )
        model = "openai/gpt-4o"

        root = canvas_store.add_prompt_tile(
            session.id, prompt="Question 1", models=[model]
        )
        canvas_store.update_response_content(
            session.id, root.id, model, "Answer 1", status="completed"
        )

        mid = canvas_store.add_prompt_tile(
            session.id,
            prompt="Question 2",
            models=[model],
            parent_tile_id=root.id,
            parent_model_id=model,
        )
        canvas_store.update_response_content(
            session.id, mid.id, model, "Answer 2", status="completed"
        )

        leaf = canvas_store.add_prompt_tile(
            session.id,
            prompt="Question 3",
            models=[model],
            parent_tile_id=mid.id,
            parent_model_id=model,
        )
        canvas_store.update_response_content(
            session.id, leaf.id, model, "Answer 3", status="completed"
        )
        # Save to persist the completed responses
        canvas_store.save_session(session.id)

        return session, root, mid, leaf, model

    def test_build_full_history(self, canvas_store):
        """Full history walks the parent chain and returns messages oldest-first."""
        session, root, mid, leaf, model = self._chain_of_three(canvas_store)

        messages = canvas_store.build_full_history(
            session.id, leaf.id, model
        )

        # 3 turns * 2 messages each = 6 messages
        assert len(messages) == 6
        assert messages[0] == {"role": "user", "content": "Question 1"}
        assert messages[1] == {"role": "assistant", "content": "Answer 1"}
        assert messages[4] == {"role": "user", "content": "Question 3"}
        assert messages[5] == {"role": "assistant", "content": "Answer 3"}

    def test_build_full_history_no_parent(self, canvas_store):
        """Full history with no parent tile/model returns empty list."""
        session = canvas_store.create_session(
            CanvasCreate(title="No parent")
        )
        result = canvas_store.build_full_history(session.id, None, None)
        assert result == []

    def test_build_compact_history_short(self, canvas_store):
        """Compact history returns full messages when history is short."""
        session = canvas_store.create_session(
            CanvasCreate(title="Short History")
        )
        model = "m1"
        tile = canvas_store.add_prompt_tile(
            session.id, prompt="Only one", models=[model]
        )
        canvas_store.update_response_content(
            session.id, tile.id, model, "Single reply", status="completed"
        )
        canvas_store.save_session(session.id)

        messages = canvas_store.build_compact_history(
            session.id, tile.id, model, max_recent=2
        )

        # Only 1 turn (2 messages), which is within max_recent
        assert len(messages) == 2
        assert messages[0]["role"] == "user"
        assert messages[1]["role"] == "assistant"

    def test_build_compact_history_long(self, canvas_store):
        """Compact history summarizes older turns when history exceeds max_recent."""
        session, root, mid, leaf, model = self._chain_of_three(canvas_store)

        messages = canvas_store.build_compact_history(
            session.id, leaf.id, model, max_recent=1
        )

        # Should have: 1 system summary + 1 recent turn (2 messages) = 3 messages
        assert len(messages) == 3
        assert messages[0]["role"] == "system"
        assert "Previous discussion" in messages[0]["content"]
        assert messages[1]["role"] == "user"
        assert messages[1]["content"] == "Question 3"


# ============================================================================
# Viewport Management
# ============================================================================


@pytest.mark.unit
class TestViewport:
    """Tests for viewport updates."""

    def test_update_viewport(self, canvas_store, canvas_session_data):
        """Updating viewport stores new x, y, zoom values."""
        session = canvas_store.create_session(CanvasCreate(**canvas_session_data))

        viewport = CanvasViewport(x=100, y=200, zoom=1.5)
        result = canvas_store.update_viewport(session.id, viewport)

        assert result is True
        refreshed = canvas_store.get_session(session.id)
        assert refreshed.viewport.x == 100
        assert refreshed.viewport.y == 200
        assert refreshed.viewport.zoom == 1.5

    def test_update_viewport_invalid_session(self, canvas_store):
        """Updating viewport for a non-existent session returns False."""
        viewport = CanvasViewport(x=0, y=0, zoom=1)
        assert canvas_store.update_viewport("bad-id", viewport) is False


# ============================================================================
# Persistence
# ============================================================================


@pytest.mark.unit
class TestPersistence:
    """Tests for JSON file persistence and reload on init."""

    def test_session_saved_as_json(self, canvas_store, canvas_session_data):
        """Creating a session writes a valid JSON file to disk."""
        session = canvas_store.create_session(CanvasCreate(**canvas_session_data))
        json_path = canvas_store.data_path / f"{session.id}.json"

        assert json_path.exists()
        with open(json_path, "r", encoding="utf-8") as f:
            data = json.load(f)
        assert data["id"] == session.id
        assert data["title"] == "Test Canvas Session"

    def test_sessions_loaded_on_init(self, tmp_path, canvas_session_data):
        """A new CanvasSessionStore instance loads existing JSON files."""
        canvas_dir = tmp_path / "reload_canvas"
        canvas_dir.mkdir()

        # Create a session with the first store instance
        store1 = CanvasSessionStore(data_path=str(canvas_dir))
        session = store1.create_session(CanvasCreate(**canvas_session_data))
        session_id = session.id

        # Create a second store instance pointing to the same directory
        store2 = CanvasSessionStore(data_path=str(canvas_dir))
        loaded = store2.get_session(session_id)

        assert loaded is not None
        assert loaded.title == "Test Canvas Session"

    def test_delete_removes_json_file(self, canvas_store, canvas_session_data):
        """Deleting a session removes its JSON file from disk."""
        session = canvas_store.create_session(CanvasCreate(**canvas_session_data))
        json_path = canvas_store.data_path / f"{session.id}.json"
        assert json_path.exists()

        canvas_store.delete_session(session.id)
        assert not json_path.exists()

    def test_save_session_explicit(self, canvas_store, canvas_session_data):
        """Explicitly calling save_session persists in-memory changes."""
        session = canvas_store.create_session(CanvasCreate(**canvas_session_data))
        tile = canvas_store.add_prompt_tile(
            session.id, prompt="Test", models=["m1"]
        )
        # update_response_content does NOT save to disk by design
        canvas_store.update_response_content(
            session.id, tile.id, "m1", "Streamed content", status="completed"
        )
        canvas_store.save_session(session.id)

        # Reload from disk
        store2 = CanvasSessionStore(data_path=str(canvas_store.data_path))
        loaded = store2.get_session(session.id)
        resp = loaded.prompt_tiles[0].responses["m1"]
        assert resp.content == "Streamed content"
        assert resp.status == "completed"

    def test_link_note(self, canvas_store, canvas_session_data):
        """Linking a note stores the note ID and persists it."""
        session = canvas_store.create_session(CanvasCreate(**canvas_session_data))

        result = canvas_store.link_note(session.id, "my-note-id")
        assert result is True

        refreshed = canvas_store.get_session(session.id)
        assert refreshed.linked_note_id == "my-note-id"


# ============================================================================
# Batch Position Updates
# ============================================================================


@pytest.mark.unit
class TestBatchPositions:
    """Tests for batch_update_positions used by auto-arrange."""

    def test_batch_update_prompt_and_llm(self, canvas_store):
        """batch_update_positions moves both prompt nodes and LLM nodes."""
        session = canvas_store.create_session(
            CanvasCreate(title="Batch Test")
        )
        tile = canvas_store.add_prompt_tile(
            session.id, prompt="Q", models=["model-x"]
        )

        positions = {
            f"prompt:{tile.id}": TilePositionUpdate(x=10, y=20),
            f"llm:{tile.id}:model-x": TilePositionUpdate(x=300, y=20),
        }
        result = canvas_store.batch_update_positions(session.id, positions)

        assert result is True
        refreshed = canvas_store.get_session(session.id)
        t = refreshed.prompt_tiles[0]
        assert t.position.x == 10
        assert t.position.y == 20
        assert t.responses["model-x"].position.x == 300
        assert t.responses["model-x"].position.y == 20
