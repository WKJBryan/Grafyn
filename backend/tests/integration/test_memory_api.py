"""Integration tests for the Memory API endpoints.

Tests the full request/response cycle for /api/memory/ routes:
recall, contradictions, and extract.
"""
import pytest
from fastapi.testclient import TestClient


# ============================================================================
# Recall Endpoint
# ============================================================================


@pytest.mark.integration
class TestRecallEndpoint:
    """Tests for POST /api/memory/recall"""

    def test_recall_returns_results(self, test_client: TestClient, create_sample_notes):
        resp = test_client.post(
            "/api/memory/recall",
            json={"query": "programming language", "limit": 3},
        )
        assert resp.status_code == 200
        body = resp.json()
        assert "results" in body
        assert isinstance(body["results"], list)

    def test_recall_limits_results(self, test_client: TestClient, create_sample_notes):
        resp = test_client.post(
            "/api/memory/recall",
            json={"query": "programming", "limit": 2},
        )
        assert resp.status_code == 200
        assert len(resp.json()["results"]) <= 2

    def test_recall_with_context_note_ids(self, test_client: TestClient, create_sample_notes):
        resp = test_client.post(
            "/api/memory/recall",
            json={
                "query": "programming",
                "context_note_ids": [create_sample_notes[0]],
                "limit": 5,
            },
        )
        assert resp.status_code == 200
        body = resp.json()
        assert "results" in body

    def test_recall_result_shape(self, test_client: TestClient, create_sample_notes):
        resp = test_client.post(
            "/api/memory/recall",
            json={"query": "Python", "limit": 1},
        )
        assert resp.status_code == 200
        results = resp.json()["results"]
        if results:
            r = results[0]
            assert "note_id" in r
            assert "title" in r
            assert "content" in r
            assert "relevance_score" in r
            assert "connection_type" in r
            assert r["connection_type"] in ("semantic", "graph", "both")

    def test_recall_empty_query_rejected(self, test_client: TestClient):
        resp = test_client.post(
            "/api/memory/recall",
            json={"query": "", "limit": 5},
        )
        assert resp.status_code == 422

    def test_recall_missing_query_rejected(self, test_client: TestClient):
        resp = test_client.post(
            "/api/memory/recall",
            json={"limit": 5},
        )
        assert resp.status_code == 422


# ============================================================================
# Contradictions Endpoint
# ============================================================================


@pytest.mark.integration
class TestContradictionsEndpoint:
    """Tests for POST /api/memory/contradictions/{note_id}"""

    def test_contradictions_returns_list(self, test_client: TestClient, create_sample_notes):
        note_id = create_sample_notes[0]
        resp = test_client.post(f"/api/memory/contradictions/{note_id}")
        assert resp.status_code == 200
        body = resp.json()
        assert "contradictions" in body
        assert isinstance(body["contradictions"], list)

    def test_contradictions_result_shape(self, test_client: TestClient, create_sample_notes):
        note_id = create_sample_notes[0]
        resp = test_client.post(f"/api/memory/contradictions/{note_id}")
        assert resp.status_code == 200
        for item in resp.json()["contradictions"]:
            assert "note_id" in item
            assert "title" in item
            assert "conflicting_field" in item
            assert "this_value" in item
            assert "other_value" in item
            assert "similarity_score" in item

    def test_contradictions_nonexistent_note(self, test_client: TestClient):
        resp = test_client.post("/api/memory/contradictions/does-not-exist")
        assert resp.status_code == 200
        assert resp.json()["contradictions"] == []


# ============================================================================
# Extract Endpoint
# ============================================================================


@pytest.mark.integration
class TestExtractEndpoint:
    """Tests for POST /api/memory/extract"""

    def test_extract_returns_suggestions(self, test_client: TestClient):
        resp = test_client.post(
            "/api/memory/extract",
            json={
                "messages": [
                    {"role": "user", "content": "Tell me about Python"},
                    {
                        "role": "assistant",
                        "content": (
                            "Python is a versatile programming language widely used "
                            "in data science, web development, and automation workflows. "
                            "It features a clean syntax and a large standard library."
                        ),
                    },
                ],
            },
        )
        assert resp.status_code == 200
        body = resp.json()
        assert "suggestions" in body
        assert len(body["suggestions"]) >= 1

    def test_extract_suggestion_shape(self, test_client: TestClient):
        resp = test_client.post(
            "/api/memory/extract",
            json={
                "messages": [
                    {
                        "role": "assistant",
                        "content": (
                            "Machine learning models require careful data preprocessing "
                            "before training. Feature engineering is a crucial step that "
                            "determines model performance."
                        ),
                    },
                ],
            },
        )
        assert resp.status_code == 200
        for s in resp.json()["suggestions"]:
            assert "title" in s
            assert "content" in s
            assert "tags" in s
            assert "status" in s
            assert s["status"] == "draft"
            assert "source" in s

    def test_extract_custom_source(self, test_client: TestClient):
        resp = test_client.post(
            "/api/memory/extract",
            json={
                "messages": [
                    {
                        "role": "assistant",
                        "content": "A detailed explanation about database indexing strategies and their performance implications in production systems.",
                    },
                ],
                "source": "chatgpt",
            },
        )
        assert resp.status_code == 200
        suggestions = resp.json()["suggestions"]
        if suggestions:
            assert suggestions[0]["source"] == "chatgpt"

    def test_extract_empty_messages_rejected(self, test_client: TestClient):
        resp = test_client.post(
            "/api/memory/extract",
            json={"messages": []},
        )
        assert resp.status_code == 422

    def test_extract_invalid_role_rejected(self, test_client: TestClient):
        resp = test_client.post(
            "/api/memory/extract",
            json={
                "messages": [
                    {"role": "system", "content": "You are helpful"},
                ],
            },
        )
        assert resp.status_code == 422

    def test_extract_skips_short_responses(self, test_client: TestClient):
        resp = test_client.post(
            "/api/memory/extract",
            json={
                "messages": [
                    {"role": "assistant", "content": "OK"},
                ],
            },
        )
        assert resp.status_code == 200
        assert resp.json()["suggestions"] == []
