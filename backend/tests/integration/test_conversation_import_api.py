"""
Integration tests for Conversation Import API endpoints.

Tests the full request/response cycle for the /api/import routes,
covering upload, parse, preview, apply, and revert workflows.
"""

import io
import json

import pytest
from fastapi.testclient import TestClient


# ============================================================================
# Sample Data
# ============================================================================

CHATGPT_CONVERSATION_DATA = [
    {
        "title": "Test Conversation",
        "create_time": 1704067200,
        "update_time": 1704153600,
        "mapping": {
            "msg-root": {
                "id": "msg-root",
                "message": None,
                "parent": None,
                "children": ["msg-1"],
            },
            "msg-1": {
                "id": "msg-1",
                "message": {
                    "id": "msg-1",
                    "author": {"role": "user"},
                    "content": {"parts": ["Hello, how are you?"]},
                    "create_time": 1704067200,
                },
                "parent": "msg-root",
                "children": ["msg-2"],
            },
            "msg-2": {
                "id": "msg-2",
                "message": {
                    "id": "msg-2",
                    "author": {"role": "assistant", "metadata": {}},
                    "content": {
                        "parts": ["I'm doing well! How can I help you today?"]
                    },
                    "create_time": 1704067260,
                },
                "parent": "msg-1",
                "children": [],
            },
        },
    }
]

MULTI_CONVERSATION_DATA = CHATGPT_CONVERSATION_DATA + [
    {
        "title": "Second Conversation",
        "create_time": 1704200000,
        "update_time": 1704250000,
        "mapping": {
            "msg-root-2": {
                "id": "msg-root-2",
                "message": None,
                "parent": None,
                "children": ["msg-3"],
            },
            "msg-3": {
                "id": "msg-3",
                "message": {
                    "id": "msg-3",
                    "author": {"role": "user"},
                    "content": {"parts": ["Tell me about Python programming."]},
                    "create_time": 1704200000,
                },
                "parent": "msg-root-2",
                "children": ["msg-4"],
            },
            "msg-4": {
                "id": "msg-4",
                "message": {
                    "id": "msg-4",
                    "author": {"role": "assistant", "metadata": {}},
                    "content": {
                        "parts": [
                            "Python is a high-level, interpreted programming language."
                        ]
                    },
                    "create_time": 1704200060,
                },
                "parent": "msg-3",
                "children": [],
            },
        },
    }
]


# ============================================================================
# Helpers
# ============================================================================


def _make_upload_file(data, filename="conversations.json"):
    """Create a file-like upload object from JSON-serializable data."""
    content = json.dumps(data).encode("utf-8")
    return {"file": (filename, io.BytesIO(content), "application/json")}


def _upload_and_parse(client: TestClient, data=None):
    """Upload a file and parse it, returning the parsed job dict."""
    if data is None:
        data = CHATGPT_CONVERSATION_DATA
    files = _make_upload_file(data)
    upload_resp = client.post("/api/import/upload", files=files)
    assert upload_resp.status_code == 200
    job = upload_resp.json()

    parse_resp = client.post(f"/api/import/{job['id']}/parse")
    assert parse_resp.status_code == 200
    return parse_resp.json()


# ============================================================================
# Upload Tests
# ============================================================================


@pytest.mark.integration
class TestUploadEndpoint:
    """Tests for POST /api/import/upload"""

    def test_upload_creates_job_with_uploaded_status(self, test_client: TestClient):
        files = _make_upload_file(CHATGPT_CONVERSATION_DATA)
        resp = test_client.post("/api/import/upload", files=files)

        assert resp.status_code == 200
        body = resp.json()
        assert body["status"] == "uploaded"
        assert body["id"]
        assert body["file_name"] == "conversations.json"

    def test_upload_preserves_filename(self, test_client: TestClient):
        files = _make_upload_file(
            CHATGPT_CONVERSATION_DATA, filename="my_export.json"
        )
        resp = test_client.post("/api/import/upload", files=files)

        assert resp.status_code == 200
        assert resp.json()["file_name"] == "my_export.json"

    def test_upload_returns_unique_job_ids(self, test_client: TestClient):
        files1 = _make_upload_file(CHATGPT_CONVERSATION_DATA)
        files2 = _make_upload_file(CHATGPT_CONVERSATION_DATA)

        resp1 = test_client.post("/api/import/upload", files=files1)
        resp2 = test_client.post("/api/import/upload", files=files2)

        assert resp1.status_code == 200
        assert resp2.status_code == 200
        assert resp1.json()["id"] != resp2.json()["id"]


# ============================================================================
# Get Job Tests
# ============================================================================


@pytest.mark.integration
class TestGetJobEndpoint:
    """Tests for GET /api/import/{job_id}"""

    def test_get_existing_job(self, test_client: TestClient):
        files = _make_upload_file(CHATGPT_CONVERSATION_DATA)
        upload_resp = test_client.post("/api/import/upload", files=files)
        job_id = upload_resp.json()["id"]

        resp = test_client.get(f"/api/import/{job_id}")
        assert resp.status_code == 200
        assert resp.json()["id"] == job_id
        assert resp.json()["status"] == "uploaded"

    def test_get_nonexistent_job_returns_404(self, test_client: TestClient):
        resp = test_client.get("/api/import/nonexistent-job-id-12345")
        assert resp.status_code == 404

    def test_get_job_reflects_status_changes(self, test_client: TestClient):
        """After parsing, the job status should be 'parsed' when retrieved."""
        job = _upload_and_parse(test_client)
        job_id = job["id"]

        resp = test_client.get(f"/api/import/{job_id}")
        assert resp.status_code == 200
        assert resp.json()["status"] == "parsed"


# ============================================================================
# Parse Tests
# ============================================================================


@pytest.mark.integration
class TestParseEndpoint:
    """Tests for POST /api/import/{job_id}/parse"""

    def test_parse_sets_status_to_parsed(self, test_client: TestClient):
        job = _upload_and_parse(test_client)
        assert job["status"] == "parsed"

    def test_parse_detects_chatgpt_platform(self, test_client: TestClient):
        job = _upload_and_parse(test_client)
        assert job["platform"] == "chatgpt"

    def test_parse_extracts_conversations(self, test_client: TestClient):
        job = _upload_and_parse(test_client)
        assert job["total_conversations"] == 1
        assert job["parsed_conversations"] is not None
        assert len(job["parsed_conversations"]) == 1

    def test_parse_extracts_conversation_title(self, test_client: TestClient):
        job = _upload_and_parse(test_client)
        conv = job["parsed_conversations"][0]
        assert conv["title"] == "Test Conversation"

    def test_parse_extracts_messages(self, test_client: TestClient):
        job = _upload_and_parse(test_client)
        conv = job["parsed_conversations"][0]
        messages = conv["messages"]
        assert len(messages) >= 1
        # At least the user message should be present
        roles = [m["role"] for m in messages]
        assert "user" in roles

    def test_parse_multiple_conversations(self, test_client: TestClient):
        job = _upload_and_parse(test_client, data=MULTI_CONVERSATION_DATA)
        assert job["total_conversations"] == 2
        titles = [c["title"] for c in job["parsed_conversations"]]
        assert "Test Conversation" in titles
        assert "Second Conversation" in titles

    def test_parse_invalid_format_sets_failed_status(self, test_client: TestClient):
        """Uploading non-JSON content should fail to parse."""
        content = b"This is not JSON at all"
        files = {"file": ("bad_file.txt", io.BytesIO(content), "text/plain")}
        upload_resp = test_client.post("/api/import/upload", files=files)
        assert upload_resp.status_code == 200
        job_id = upload_resp.json()["id"]

        parse_resp = test_client.post(f"/api/import/{job_id}/parse")
        # The parser should either return 400/500 or a job with "failed" status
        if parse_resp.status_code == 200:
            assert parse_resp.json()["status"] == "failed"
        else:
            assert parse_resp.status_code in (400, 500)


# ============================================================================
# Preview Tests
# ============================================================================


@pytest.mark.integration
class TestPreviewEndpoint:
    """Tests for GET /api/import/{job_id}/preview"""

    def test_preview_after_parsing(self, test_client: TestClient):
        job = _upload_and_parse(test_client)
        job_id = job["id"]

        resp = test_client.get(f"/api/import/{job_id}/preview")
        assert resp.status_code == 200
        preview = resp.json()
        assert preview["job_id"] == job_id
        assert preview["total_conversations"] == 1
        assert len(preview["conversations"]) == 1
        assert preview["estimated_notes_to_create"] > 0

    def test_preview_before_parsing_returns_error(self, test_client: TestClient):
        """Preview should fail if the job has not been parsed yet."""
        files = _make_upload_file(CHATGPT_CONVERSATION_DATA)
        upload_resp = test_client.post("/api/import/upload", files=files)
        job_id = upload_resp.json()["id"]

        resp = test_client.get(f"/api/import/{job_id}/preview")
        assert resp.status_code == 400

    def test_preview_contains_platform_info(self, test_client: TestClient):
        job = _upload_and_parse(test_client)
        resp = test_client.get(f"/api/import/{job['id']}/preview")
        assert resp.status_code == 200
        assert resp.json()["platform"] == "chatgpt"


# ============================================================================
# Apply Import Tests
# ============================================================================


@pytest.mark.integration
class TestApplyImportEndpoint:
    """Tests for POST /api/import/{job_id}/apply"""

    def test_apply_accept_creates_notes(self, test_client: TestClient):
        job = _upload_and_parse(test_client)
        job_id = job["id"]
        conv_id = job["parsed_conversations"][0]["id"]

        decisions = [
            {
                "conversation_id": conv_id,
                "action": "accept",
                "distill_option": "container_only",
            }
        ]

        resp = test_client.post(
            f"/api/import/{job_id}/apply", json=decisions
        )
        assert resp.status_code == 200
        summary = resp.json()
        assert summary["imported"] == 1
        assert summary["skipped"] == 0
        assert summary["notes_created"] >= 1
        assert summary["container_notes"] == 1

    def test_apply_skip_creates_no_notes(self, test_client: TestClient):
        job = _upload_and_parse(test_client)
        job_id = job["id"]
        conv_id = job["parsed_conversations"][0]["id"]

        decisions = [
            {
                "conversation_id": conv_id,
                "action": "skip",
            }
        ]

        resp = test_client.post(
            f"/api/import/{job_id}/apply", json=decisions
        )
        assert resp.status_code == 200
        summary = resp.json()
        assert summary["imported"] == 0
        assert summary["skipped"] == 1
        assert summary["notes_created"] == 0

    def test_apply_mixed_decisions(self, test_client: TestClient):
        """With two conversations, accept one and skip the other."""
        job = _upload_and_parse(test_client, data=MULTI_CONVERSATION_DATA)
        job_id = job["id"]
        convs = job["parsed_conversations"]
        assert len(convs) == 2

        decisions = [
            {
                "conversation_id": convs[0]["id"],
                "action": "accept",
                "distill_option": "container_only",
            },
            {
                "conversation_id": convs[1]["id"],
                "action": "skip",
            },
        ]

        resp = test_client.post(
            f"/api/import/{job_id}/apply", json=decisions
        )
        assert resp.status_code == 200
        summary = resp.json()
        assert summary["imported"] == 1
        assert summary["skipped"] == 1
        assert summary["total_conversations"] == 2

    def test_apply_sets_job_status_to_completed(self, test_client: TestClient):
        job = _upload_and_parse(test_client)
        job_id = job["id"]
        conv_id = job["parsed_conversations"][0]["id"]

        decisions = [
            {
                "conversation_id": conv_id,
                "action": "accept",
                "distill_option": "container_only",
            }
        ]

        test_client.post(f"/api/import/{job_id}/apply", json=decisions)

        get_resp = test_client.get(f"/api/import/{job_id}")
        assert get_resp.status_code == 200
        assert get_resp.json()["status"] == "completed"

    def test_apply_with_empty_decisions(self, test_client: TestClient):
        job = _upload_and_parse(test_client)
        job_id = job["id"]

        resp = test_client.post(
            f"/api/import/{job_id}/apply", json=[]
        )
        assert resp.status_code == 200
        summary = resp.json()
        assert summary["imported"] == 0
        assert summary["skipped"] == 0
        assert summary["notes_created"] == 0


# ============================================================================
# Revert Import Tests
# ============================================================================


@pytest.mark.integration
class TestRevertImportEndpoint:
    """Tests for POST /api/import/{job_id}/revert"""

    def test_revert_deletes_created_notes(self, test_client: TestClient):
        # First, import a conversation
        job = _upload_and_parse(test_client)
        job_id = job["id"]
        conv_id = job["parsed_conversations"][0]["id"]

        decisions = [
            {
                "conversation_id": conv_id,
                "action": "accept",
                "distill_option": "container_only",
            }
        ]

        apply_resp = test_client.post(
            f"/api/import/{job_id}/apply", json=decisions
        )
        assert apply_resp.status_code == 200
        notes_created = apply_resp.json()["notes_created"]
        assert notes_created >= 1

        # Now revert
        revert_resp = test_client.post(f"/api/import/{job_id}/revert")
        assert revert_resp.status_code == 200
        result = revert_resp.json()
        assert result["deleted"] == notes_created
        assert result["job_id"] == job_id

    def test_revert_without_prior_import_returns_error(
        self, test_client: TestClient
    ):
        """Reverting a job that was never applied should fail."""
        job = _upload_and_parse(test_client)
        job_id = job["id"]

        resp = test_client.post(f"/api/import/{job_id}/revert")
        assert resp.status_code == 400

    def test_revert_notes_no_longer_exist(self, test_client: TestClient):
        """After revert, the notes should not be retrievable from the vault."""
        job = _upload_and_parse(test_client)
        job_id = job["id"]
        conv_id = job["parsed_conversations"][0]["id"]

        decisions = [
            {
                "conversation_id": conv_id,
                "action": "accept",
                "distill_option": "container_only",
            }
        ]

        test_client.post(f"/api/import/{job_id}/apply", json=decisions)

        # Get notes list before revert
        notes_before = test_client.get("/api/notes").json()
        assert len(notes_before) >= 1

        # Revert
        test_client.post(f"/api/import/{job_id}/revert")

        # The imported notes should be gone
        notes_after = test_client.get("/api/notes").json()
        assert len(notes_after) < len(notes_before)


# ============================================================================
# Cancel Job Tests
# ============================================================================


@pytest.mark.integration
class TestCancelJobEndpoint:
    """Tests for DELETE /api/import/{job_id}"""

    def test_cancel_removes_job(self, test_client: TestClient):
        files = _make_upload_file(CHATGPT_CONVERSATION_DATA)
        upload_resp = test_client.post("/api/import/upload", files=files)
        job_id = upload_resp.json()["id"]

        del_resp = test_client.delete(f"/api/import/{job_id}")
        assert del_resp.status_code == 200

        get_resp = test_client.get(f"/api/import/{job_id}")
        assert get_resp.status_code == 404

    def test_cancel_nonexistent_job_returns_404(self, test_client: TestClient):
        resp = test_client.delete("/api/import/does-not-exist")
        assert resp.status_code == 404
