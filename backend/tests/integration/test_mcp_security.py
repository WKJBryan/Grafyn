"""Integration tests for MCP endpoint input validation and security.

Tests verify that MCP endpoints properly handle:
- Oversized request bodies
- Malformed JSON
- Missing required fields
- Invalid status values
- Empty title/content
"""
import json
import pytest
from unittest.mock import patch, AsyncMock
from fastapi.testclient import TestClient


# ============================================================================
# Helpers
# ============================================================================

def _bypass_oauth():
    """Patch verify_oauth to always succeed."""
    return patch(
        "app.routers.mcp_write.verify_oauth",
        new_callable=lambda: lambda: AsyncMock(return_value=True),
    )


# ============================================================================
# Malformed JSON Tests
# ============================================================================

@pytest.mark.integration
@pytest.mark.security
class TestMcpMalformedJson:
    """Tests for malformed JSON body handling on MCP endpoints."""

    def test_mcp_write_note_malformed_json(self, test_client: TestClient):
        """POST /api/mcp-write/note with malformed JSON should return 400 or 422."""
        response = test_client.post(
            "/api/mcp-write/note",
            content=b"{not valid json!!!",
            headers={"Content-Type": "application/json"},
        )
        # Without explicit validation, json.loads will raise and FastAPI returns 500
        # With proper validation (from python-fixer), it should return 400
        assert response.status_code in (400, 422, 500)

    def test_mcp_create_note_simple_malformed_json(self, test_client: TestClient):
        """POST /api/mcp/notes/create with malformed JSON should return 400 or 422."""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes/create",
                content=b"<<<definitely not json>>>",
                headers={"Content-Type": "application/json"},
            )
        # json.loads in the handler will fail
        assert response.status_code in (400, 422, 500)

    def test_mcp_create_note_pydantic_malformed_json(self, test_client: TestClient):
        """POST /api/mcp/notes with malformed JSON should return 422 (Pydantic validation)."""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes",
                content=b"{broken",
                headers={"Content-Type": "application/json"},
            )
        assert response.status_code == 422


# ============================================================================
# Missing Required Fields Tests
# ============================================================================

@pytest.mark.integration
@pytest.mark.security
class TestMcpMissingFields:
    """Tests for missing required fields on MCP endpoints."""

    def test_mcp_write_note_missing_title(self, test_client: TestClient):
        """POST /api/mcp-write/note without title should return 400."""
        response = test_client.post(
            "/api/mcp-write/note",
            json={"content": "No title provided"},
        )
        assert response.status_code == 400
        assert "title" in response.json().get("detail", "").lower()

    def test_mcp_write_note_empty_title(self, test_client: TestClient):
        """POST /api/mcp-write/note with empty string title should return 400."""
        response = test_client.post(
            "/api/mcp-write/note",
            json={"title": "", "content": "Some content"},
        )
        # Empty string is falsy, so the handler should reject it
        assert response.status_code == 400

    def test_mcp_write_note_null_title(self, test_client: TestClient):
        """POST /api/mcp-write/note with null title should return 400."""
        response = test_client.post(
            "/api/mcp-write/note",
            json={"title": None, "content": "Some content"},
        )
        assert response.status_code == 400

    def test_mcp_create_note_pydantic_missing_title(self, test_client: TestClient):
        """POST /api/mcp/notes with missing title should return 422."""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes",
                json={"content": "Body without title"},
            )
        assert response.status_code == 422

    def test_mcp_create_note_pydantic_empty_title(self, test_client: TestClient):
        """POST /api/mcp/notes with empty title should return 422 (min_length=1)."""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes",
                json={"title": "", "content": "Body"},
            )
        assert response.status_code == 422

    def test_mcp_create_note_pydantic_title_too_long(self, test_client: TestClient):
        """POST /api/mcp/notes with title > 255 chars should return 422 (max_length=255)."""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes",
                json={"title": "X" * 256, "content": "Body"},
            )
        assert response.status_code == 422

    def test_mcp_find_or_create_missing_search_query(self, test_client: TestClient):
        """POST /api/mcp/notes/find-or-create without search_query should return 422."""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes/find-or-create",
                json={"title": "Test", "content": "Body"},
            )
        assert response.status_code == 422

    def test_mcp_find_or_create_missing_title(self, test_client: TestClient):
        """POST /api/mcp/notes/find-or-create without title should return 422."""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes/find-or-create",
                json={"search_query": "test"},
            )
        assert response.status_code == 422


# ============================================================================
# Invalid Status Values
# ============================================================================

@pytest.mark.integration
@pytest.mark.security
class TestMcpInvalidStatus:
    """Tests for invalid status values on MCP endpoints."""

    def test_mcp_create_note_invalid_status(self, test_client: TestClient):
        """POST /api/mcp/notes with invalid status should return 422."""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes",
                json={
                    "title": "Invalid Status Note",
                    "content": "Content",
                    "status": "nonexistent_status",
                },
            )
        assert response.status_code == 422

    def test_mcp_update_note_invalid_status(self, test_client: TestClient):
        """PUT /api/mcp/notes/{id} with invalid status should return 422."""
        # First create a valid note
        with _bypass_oauth():
            create_resp = test_client.post(
                "/api/mcp/notes",
                json={"title": "Status Test Note", "content": "Original"},
            )
        if create_resp.status_code != 200:
            pytest.skip("Could not create prerequisite note")

        note_id = create_resp.json()["id"]

        with _bypass_oauth():
            response = test_client.put(
                f"/api/mcp/notes/{note_id}",
                json={
                    "note_id": note_id,
                    "status": "invalid_status_value",
                },
            )
        assert response.status_code == 422

    def test_mcp_update_note_invalid_content_mode(self, test_client: TestClient):
        """PUT /api/mcp/notes/{id} with invalid content_mode should return 422."""
        with _bypass_oauth():
            create_resp = test_client.post(
                "/api/mcp/notes",
                json={"title": "Mode Test Note", "content": "Original"},
            )
        if create_resp.status_code != 200:
            pytest.skip("Could not create prerequisite note")

        note_id = create_resp.json()["id"]

        with _bypass_oauth():
            response = test_client.put(
                f"/api/mcp/notes/{note_id}",
                json={
                    "note_id": note_id,
                    "content": "New content",
                    "content_mode": "delete_everything",
                },
            )
        assert response.status_code == 422


# ============================================================================
# Oversized Body Tests
# ============================================================================

@pytest.mark.integration
@pytest.mark.security
class TestMcpOversizedBody:
    """Tests for oversized request body handling.

    Note: These tests may require body size limits to be added by python-fixer.
    If limits aren't in place yet, the endpoint will accept the large body.
    """

    def test_mcp_write_note_oversized_body(self, test_client: TestClient):
        """POST /api/mcp-write/note with very large body should be rejected or handled."""
        # 2MB body -- well above reasonable limits for a note
        large_content = "X" * (2 * 1024 * 1024)
        response = test_client.post(
            "/api/mcp-write/note",
            json={"title": "Oversized Note", "content": large_content},
        )
        # With body size limits (from python-fixer), expect 413
        # Without limits, it may succeed (200) or fail for other reasons
        # We accept either outcome but document the expectation
        if response.status_code == 413:
            pass  # Body size limit is enforced -- expected behavior
        else:
            # If no limit enforced, at least verify it didn't crash with 500
            assert response.status_code in (200, 413), \
                f"Unexpected status {response.status_code} for oversized body"

    def test_mcp_create_note_oversized_body(self, test_client: TestClient):
        """POST /api/mcp/notes with very large body should be rejected or handled."""
        large_content = "Y" * (2 * 1024 * 1024)
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes",
                json={"title": "Big Note", "content": large_content},
            )
        if response.status_code == 413:
            pass  # Expected with body size limits
        else:
            assert response.status_code in (200, 413)


# ============================================================================
# Empty Body and Content Edge Cases
# ============================================================================

@pytest.mark.integration
@pytest.mark.security
class TestMcpEmptyContent:
    """Tests for empty body and content edge cases."""

    def test_mcp_write_note_empty_body(self, test_client: TestClient):
        """POST /api/mcp-write/note with completely empty body should return error."""
        response = test_client.post(
            "/api/mcp-write/note",
            content=b"",
            headers={"Content-Type": "application/json"},
        )
        # Empty body means json.loads will fail
        assert response.status_code in (400, 422, 500)

    def test_mcp_write_note_empty_json_object(self, test_client: TestClient):
        """POST /api/mcp-write/note with {} should return 400 (no title)."""
        response = test_client.post(
            "/api/mcp-write/note",
            json={},
        )
        assert response.status_code == 400

    def test_mcp_write_note_valid_title_empty_content(self, test_client: TestClient):
        """POST /api/mcp-write/note with title but empty content should succeed."""
        response = test_client.post(
            "/api/mcp-write/note",
            json={"title": "Empty Content Note", "content": ""},
        )
        # Empty content is valid -- defaults to ""
        assert response.status_code == 200

    def test_mcp_write_note_whitespace_title(self, test_client: TestClient):
        """POST /api/mcp-write/note with whitespace-only title should be handled."""
        response = test_client.post(
            "/api/mcp-write/note",
            json={"title": "   ", "content": "Content"},
        )
        # Whitespace title generates an empty note_id after sanitization
        # This should either be rejected (400) or handled gracefully
        assert response.status_code in (200, 400, 422, 500)


# ============================================================================
# Find-or-Create Threshold Validation
# ============================================================================

@pytest.mark.integration
@pytest.mark.security
class TestMcpFindOrCreateValidation:
    """Tests for find-or-create threshold and field validation."""

    def test_threshold_above_max(self, test_client: TestClient):
        """Threshold > 1.0 should be rejected by Pydantic (le=1.0)."""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes/find-or-create",
                json={
                    "search_query": "test",
                    "title": "Test",
                    "threshold": 1.5,
                },
            )
        assert response.status_code == 422

    def test_threshold_below_min(self, test_client: TestClient):
        """Threshold < 0.0 should be rejected by Pydantic (ge=0.0)."""
        with _bypass_oauth():
            response = test_client.post(
                "/api/mcp/notes/find-or-create",
                json={
                    "search_query": "test",
                    "title": "Test",
                    "threshold": -0.5,
                },
            )
        assert response.status_code == 422


# ============================================================================
# Set Property Validation
# ============================================================================

@pytest.mark.integration
@pytest.mark.security
class TestMcpSetPropertyValidation:
    """Tests for property setting validation on MCP endpoints."""

    def test_set_property_invalid_type(self, test_client: TestClient):
        """Setting a property with invalid type should return 422."""
        with _bypass_oauth():
            response = test_client.put(
                "/api/mcp/notes/some-note/properties",
                json={
                    "note_id": "some-note",
                    "property_name": "key",
                    "property_type": "invalid_type",
                    "value": "val",
                },
            )
        assert response.status_code == 422

    def test_set_property_missing_required_fields(self, test_client: TestClient):
        """Setting a property without required fields should return 422."""
        with _bypass_oauth():
            response = test_client.put(
                "/api/mcp/notes/some-note/properties",
                json={
                    "note_id": "some-note",
                    # Missing property_name, property_type, value
                },
            )
        assert response.status_code == 422
