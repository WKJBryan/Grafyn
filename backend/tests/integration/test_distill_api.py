"""
Integration tests for Distill API endpoints

Tests the full request/response cycle for distillation and tag normalization
endpoints mounted at /api/notes/{note_id}/distill and /api/notes/{note_id}/normalize-tags.
"""
import pytest
from fastapi.testclient import TestClient


# ============================================================================
# Helpers
# ============================================================================

def create_note_via_api(client: TestClient, title: str, content: str, **kwargs) -> dict:
    """Create a note through the API and return the response JSON."""
    payload = {
        "title": title,
        "content": content,
        "status": kwargs.get("status", "evidence"),
        "tags": kwargs.get("tags", []),
    }
    resp = client.post("/api/notes", json=payload)
    assert resp.status_code == 201, f"Failed to create note: {resp.text}"
    return resp.json()


CONTAINER_CONTENT = """\
# Research on Distributed Systems

This is a container note with substantial content about distributed systems.

## Consensus Algorithms

- Raft provides an understandable consensus algorithm for managing replicated logs
- Paxos is the foundational algorithm but is notoriously difficult to implement correctly
- Byzantine fault tolerance handles nodes that may act maliciously
- Leader election is a critical sub-problem in consensus

Key insight: choosing the right consensus algorithm depends on the trust model.

## CAP Theorem

- The CAP theorem states that a distributed system cannot simultaneously provide consistency, availability, and partition tolerance
- In practice, partition tolerance is non-negotiable, so the trade-off is between consistency and availability
- Modern systems like DynamoDB favor availability, while systems like Spanner favor consistency
- Eventually consistent systems require careful conflict resolution strategies

## Event Sourcing and CQRS

- Event sourcing stores every state change as an immutable event rather than overwriting current state
- CQRS separates the read model from the write model to optimize each independently
- Combining event sourcing with CQRS enables powerful audit trails and temporal queries
- Snapshotting mitigates the performance cost of replaying long event streams
"""

SIMPLE_CONTENT = """\
A short note without any H2 sections.

Just a single paragraph of content that is too short to extract anything from.
"""

TAGGED_CONTENT = """\
# Working with Tags

This note has some #python and #rust inline tags.

It also mentions #distributed-systems as a topic.

```python
# This is a comment, not a tag
x = 42
```

More text with a #testing tag here.
"""


# ============================================================================
# Distill Non-Existent Note
# ============================================================================

@pytest.mark.integration
class TestDistillNonExistentNote:
    """POST /api/notes/{note_id}/distill for a note that does not exist."""

    def test_returns_404(self, test_client: TestClient):
        """Should return 404 when the note does not exist."""
        resp = test_client.post(
            "/api/notes/does-not-exist-999/distill",
            json={"mode": "suggest"},
        )
        assert resp.status_code == 404

    def test_404_includes_detail(self, test_client: TestClient):
        """404 response should include a detail message."""
        resp = test_client.post(
            "/api/notes/does-not-exist-999/distill",
            json={"mode": "suggest"},
        )
        body = resp.json()
        assert "detail" in body
        assert "not found" in body["detail"].lower()


# ============================================================================
# Normalize Tags Non-Existent Note
# ============================================================================

@pytest.mark.integration
class TestNormalizeTagsNonExistentNote:
    """POST /api/notes/{note_id}/normalize-tags for a note that does not exist."""

    def test_returns_404(self, test_client: TestClient):
        """Should return 404 when the note does not exist."""
        resp = test_client.post("/api/notes/does-not-exist-999/normalize-tags")
        assert resp.status_code == 404

    def test_404_includes_detail(self, test_client: TestClient):
        """404 response should include a detail message."""
        resp = test_client.post("/api/notes/does-not-exist-999/normalize-tags")
        body = resp.json()
        assert "detail" in body


# ============================================================================
# Normalize Tags for Existing Note
# ============================================================================

@pytest.mark.integration
class TestNormalizeTagsExistingNote:
    """POST /api/notes/{note_id}/normalize-tags for an existing note."""

    def test_returns_200(self, test_client: TestClient):
        """Should return 200 with the updated note."""
        note = create_note_via_api(
            test_client,
            title="Tag Norm Test",
            content=TAGGED_CONTENT,
            tags=["Python", "#Rust"],
        )
        resp = test_client.post(f"/api/notes/{note['id']}/normalize-tags")
        assert resp.status_code == 200

    def test_normalizes_yaml_tags(self, test_client: TestClient):
        """Tags should be lowercased and stripped of leading '#'."""
        note = create_note_via_api(
            test_client,
            title="Tag Norm Lowercase",
            content=TAGGED_CONTENT,
            tags=["Python", "#Rust"],
        )
        resp = test_client.post(f"/api/notes/{note['id']}/normalize-tags")
        body = resp.json()
        tags = body["frontmatter"]["tags"]

        # Original tags Python / #Rust should be normalized
        assert "python" in tags
        assert "rust" in tags
        # No '#' prefix should remain
        assert all(not t.startswith("#") for t in tags)

    def test_merges_inline_tags(self, test_client: TestClient):
        """Inline #tags from the body should be merged into YAML tags."""
        note = create_note_via_api(
            test_client,
            title="Tag Merge Inline",
            content=TAGGED_CONTENT,
            tags=["existing"],
        )
        resp = test_client.post(f"/api/notes/{note['id']}/normalize-tags")
        body = resp.json()
        tags = body["frontmatter"]["tags"]

        # Inline tags should have been picked up
        assert "python" in tags
        assert "distributed-systems" in tags
        assert "testing" in tags
        # Original tag should still be present
        assert "existing" in tags

    def test_returns_note_shape(self, test_client: TestClient):
        """Response should have the full Note model shape."""
        note = create_note_via_api(
            test_client,
            title="Tag Shape Check",
            content="No inline tags here.",
            tags=["alpha"],
        )
        resp = test_client.post(f"/api/notes/{note['id']}/normalize-tags")
        body = resp.json()

        assert "id" in body
        assert "title" in body
        assert "content" in body
        assert "frontmatter" in body


# ============================================================================
# Distill with mode="suggest"
# ============================================================================

@pytest.mark.integration
class TestDistillSuggest:
    """POST /api/notes/{note_id}/distill with mode=suggest."""

    def test_returns_200(self, test_client: TestClient):
        """Should return 200 with distill response."""
        note = create_note_via_api(
            test_client,
            title="Distill Suggest Container",
            content=CONTAINER_CONTENT,
            tags=["research"],
        )
        resp = test_client.post(
            f"/api/notes/{note['id']}/distill",
            json={"mode": "suggest"},
        )
        assert resp.status_code == 200

    def test_returns_candidates(self, test_client: TestClient):
        """Should return a list of atomic note candidates."""
        note = create_note_via_api(
            test_client,
            title="Distill Suggest Candidates",
            content=CONTAINER_CONTENT,
            tags=["distributed"],
        )
        resp = test_client.post(
            f"/api/notes/{note['id']}/distill",
            json={"mode": "suggest"},
        )
        body = resp.json()

        assert "candidates" in body
        assert len(body["candidates"]) > 0, "Expected at least one candidate from container content"

    def test_candidate_structure(self, test_client: TestClient):
        """Each candidate should have id, title, summary, and recommended_tags."""
        note = create_note_via_api(
            test_client,
            title="Distill Suggest Structure",
            content=CONTAINER_CONTENT,
            tags=["systems"],
        )
        resp = test_client.post(
            f"/api/notes/{note['id']}/distill",
            json={"mode": "suggest"},
        )
        body = resp.json()
        candidates = body["candidates"]
        assert len(candidates) > 0

        first = candidates[0]
        assert "id" in first
        assert "title" in first
        assert "summary" in first
        assert "recommended_tags" in first
        assert "confidence" in first

    def test_no_candidates_for_short_note(self, test_client: TestClient):
        """A very short note without H2 sections should yield no candidates."""
        note = create_note_via_api(
            test_client,
            title="Short Suggest Note",
            content=SIMPLE_CONTENT,
        )
        resp = test_client.post(
            f"/api/notes/{note['id']}/distill",
            json={"mode": "suggest"},
        )
        body = resp.json()
        assert body["candidates"] == []

    def test_message_field_present(self, test_client: TestClient):
        """Response should include a message summarizing the operation."""
        note = create_note_via_api(
            test_client,
            title="Distill Suggest Message",
            content=CONTAINER_CONTENT,
        )
        resp = test_client.post(
            f"/api/notes/{note['id']}/distill",
            json={"mode": "suggest"},
        )
        body = resp.json()
        assert "message" in body
        assert isinstance(body["message"], str)
        assert len(body["message"]) > 0


# ============================================================================
# Distill with mode="auto" and extraction_method="rules"
# ============================================================================

@pytest.mark.integration
class TestDistillAutoRules:
    """POST /api/notes/{note_id}/distill with mode=auto, extraction_method=rules.

    Uses rule-based extraction so no LLM / OpenRouter key is required.
    """

    def test_returns_200(self, test_client: TestClient):
        """Should return 200."""
        note = create_note_via_api(
            test_client,
            title="Auto Rules Container",
            content=CONTAINER_CONTENT,
            tags=["research"],
        )
        resp = test_client.post(
            f"/api/notes/{note['id']}/distill",
            json={"mode": "auto", "extraction_method": "rules"},
        )
        assert resp.status_code == 200

    def test_creates_draft_notes(self, test_client: TestClient):
        """Auto mode should auto-create draft atomic notes."""
        note = create_note_via_api(
            test_client,
            title="Auto Rules Drafts",
            content=CONTAINER_CONTENT,
            tags=["distributed"],
        )
        resp = test_client.post(
            f"/api/notes/{note['id']}/distill",
            json={"mode": "auto", "extraction_method": "rules"},
        )
        body = resp.json()

        assert "created_note_ids" in body
        assert len(body["created_note_ids"]) > 0, "Expected at least one created note"

    def test_extraction_method_used_is_rules(self, test_client: TestClient):
        """extraction_method_used should be 'rules'."""
        note = create_note_via_api(
            test_client,
            title="Auto Rules Method",
            content=CONTAINER_CONTENT,
        )
        resp = test_client.post(
            f"/api/notes/{note['id']}/distill",
            json={"mode": "auto", "extraction_method": "rules"},
        )
        body = resp.json()
        assert body.get("extraction_method_used") == "rules"

    def test_container_updated_flag(self, test_client: TestClient):
        """container_updated should be True when notes were created."""
        note = create_note_via_api(
            test_client,
            title="Auto Rules Container Flag",
            content=CONTAINER_CONTENT,
        )
        resp = test_client.post(
            f"/api/notes/{note['id']}/distill",
            json={"mode": "auto", "extraction_method": "rules"},
        )
        body = resp.json()

        if body["created_note_ids"]:
            assert body["container_updated"] is True

    def test_no_candidates_for_short_note(self, test_client: TestClient):
        """Auto-distill of a note with no H2 sections creates nothing."""
        note = create_note_via_api(
            test_client,
            title="Auto Rules Short",
            content=SIMPLE_CONTENT,
        )
        resp = test_client.post(
            f"/api/notes/{note['id']}/distill",
            json={"mode": "auto", "extraction_method": "rules"},
        )
        body = resp.json()
        assert body["created_note_ids"] == []
        assert "no atomic note candidates" in body["message"].lower() or "0" in body["message"]

    def test_created_notes_are_retrievable(self, test_client: TestClient):
        """Notes created by auto-distill should be fetchable via GET /api/notes/{id}."""
        note = create_note_via_api(
            test_client,
            title="Auto Rules Retrieve",
            content=CONTAINER_CONTENT,
            tags=["systems"],
        )
        resp = test_client.post(
            f"/api/notes/{note['id']}/distill",
            json={"mode": "auto", "extraction_method": "rules"},
        )
        body = resp.json()
        created_ids = body["created_note_ids"]
        assert len(created_ids) > 0

        # Each created note should be retrievable
        for note_id in created_ids:
            get_resp = test_client.get(f"/api/notes/{note_id}")
            assert get_resp.status_code == 200, f"Could not retrieve created note {note_id}"
            fetched = get_resp.json()
            assert fetched["frontmatter"]["status"] == "draft"
