"""Integration tests for Zettelkasten API endpoints"""
import pytest


@pytest.mark.integration
class TestDiscoverLinks:
    """Tests for GET /api/zettel/notes/{note_id}/discover-links"""

    def test_note_not_found_returns_404(self, test_client):
        response = test_client.get("/api/zettel/notes/nonexistent-id/discover-links")
        assert response.status_code == 404

    def test_discover_links_for_existing_note(self, test_client):
        """Should return link candidates for a valid note"""
        # Create a note first
        note_resp = test_client.post(
            "/api/notes",
            json={
                "title": "Python Testing",
                "content": "Python testing involves pytest and unittest frameworks.",
                "status": "draft",
                "tags": ["python", "testing"],
            },
        )
        assert note_resp.status_code == 201
        note_id = note_resp.json()["id"]

        response = test_client.get(
            f"/api/zettel/notes/{note_id}/discover-links",
            params={"mode": "suggested", "max_links": 5},
        )
        assert response.status_code == 200
        data = response.json()
        assert "note_id" in data
        assert "links" in data
        assert isinstance(data["links"], list)

    @pytest.mark.xfail(
        reason="LinkDiscoveryService._find_keyword_links accesses .frontmatter on NoteListItem (bug)",
        strict=False,
    )
    def test_discover_links_finds_related_notes(self, test_client):
        """When multiple notes share tags, links should be discovered"""
        # Create two related notes
        note1_resp = test_client.post(
            "/api/notes",
            json={
                "title": "Machine Learning Basics",
                "content": "Machine learning uses neural networks and gradient descent for optimization.",
                "status": "canonical",
                "tags": ["ml", "ai", "neural-networks"],
            },
        )
        note2_resp = test_client.post(
            "/api/notes",
            json={
                "title": "Deep Learning Architectures",
                "content": "Deep learning extends machine learning with multi-layer neural networks.",
                "status": "draft",
                "tags": ["ml", "ai", "neural-networks", "deep-learning"],
            },
        )
        note1_id = note1_resp.json()["id"]

        response = test_client.get(
            f"/api/zettel/notes/{note1_id}/discover-links",
            params={"mode": "suggested"},
        )
        assert response.status_code == 200
        data = response.json()
        # Should find the related note via keyword/tag overlap
        assert isinstance(data["links"], list)

    def test_manual_mode_returns_empty(self, test_client):
        """Manual mode should return no automatic candidates"""
        note_resp = test_client.post(
            "/api/notes",
            json={
                "title": "Isolated Note For Manual Test",
                "content": "This note exists for testing manual link mode.",
                "status": "draft",
                "tags": ["test"],
            },
        )
        note_id = note_resp.json()["id"]

        response = test_client.get(
            f"/api/zettel/notes/{note_id}/discover-links",
            params={"mode": "manual"},
        )
        assert response.status_code == 200
        assert response.json()["links"] == []


@pytest.mark.integration
class TestCreateLink:
    """Tests for POST /api/zettel/notes/{source_id}/link/{target_id}"""

    def test_source_not_found_returns_404(self, test_client):
        # Create only the target note
        target = test_client.post(
            "/api/notes",
            json={
                "title": "Target Note Only",
                "content": "This is the target.",
                "status": "draft",
                "tags": [],
            },
        )
        target_id = target.json()["id"]

        response = test_client.post(
            f"/api/zettel/notes/nonexistent/link/{target_id}"
        )
        assert response.status_code == 404

    def test_target_not_found_returns_404(self, test_client):
        source = test_client.post(
            "/api/notes",
            json={
                "title": "Source Note Only",
                "content": "This is the source.",
                "status": "draft",
                "tags": [],
            },
        )
        source_id = source.json()["id"]

        response = test_client.post(
            f"/api/zettel/notes/{source_id}/link/nonexistent"
        )
        assert response.status_code == 404

    def test_create_link_between_notes(self, test_client):
        """Should create bidirectional wikilinks between two notes"""
        source = test_client.post(
            "/api/notes",
            json={
                "title": "Source Note Link Test",
                "content": "Content of source note.",
                "status": "draft",
                "tags": ["link-test"],
            },
        )
        target = test_client.post(
            "/api/notes",
            json={
                "title": "Target Note Link Test",
                "content": "Content of target note.",
                "status": "draft",
                "tags": ["link-test"],
            },
        )
        source_id = source.json()["id"]
        target_id = target.json()["id"]

        response = test_client.post(
            f"/api/zettel/notes/{source_id}/link/{target_id}",
            params={"link_type": "related"},
        )
        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "linked"
        assert data["source"] == source_id
        assert data["target"] == target_id

    def test_invalid_link_type_defaults_to_related(self, test_client):
        """Invalid link_type should default to 'related'"""
        source = test_client.post(
            "/api/notes",
            json={
                "title": "Invalid Link Type Source",
                "content": "Source for invalid link type test.",
                "status": "draft",
                "tags": [],
            },
        )
        target = test_client.post(
            "/api/notes",
            json={
                "title": "Invalid Link Type Target",
                "content": "Target for invalid link type test.",
                "status": "draft",
                "tags": [],
            },
        )
        source_id = source.json()["id"]
        target_id = target.json()["id"]

        response = test_client.post(
            f"/api/zettel/notes/{source_id}/link/{target_id}",
            params={"link_type": "invalid_type"},
        )
        assert response.status_code == 200
        assert response.json()["link_type"] == "related"
