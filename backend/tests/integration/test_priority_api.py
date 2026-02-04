"""Integration tests for Priority API endpoints"""
import pytest
from app.services.priority_scoring import PriorityWeights


@pytest.mark.integration
class TestGetPriorityConfig:
    """Tests for GET /api/priority/config"""

    def test_returns_200(self, test_client):
        response = test_client.get("/api/priority/config")
        assert response.status_code == 200

    def test_returns_full_config(self, test_client):
        data = test_client.get("/api/priority/config").json()
        assert "content_type_scores" in data
        assert "recency" in data
        assert "link_density" in data
        assert "tag_relevance" in data
        assert "semantic" in data

    def test_content_type_scores_has_all_types(self, test_client):
        data = test_client.get("/api/priority/config").json()
        scores = data["content_type_scores"]
        for key in ("claim", "decision", "insight", "question", "evidence", "general"):
            assert key in scores


@pytest.mark.integration
class TestGetPriorityWeights:
    """Tests for GET /api/priority/weights"""

    def test_returns_200(self, test_client):
        response = test_client.get("/api/priority/weights")
        assert response.status_code == 200

    def test_returns_default_weights(self, test_client):
        data = test_client.get("/api/priority/weights").json()
        assert data["claim_weight"] == 100.0
        assert data["decision_weight"] == 90.0
        assert data["recency_decay"] == 0.10


@pytest.mark.integration
class TestUpdatePriorityWeights:
    """Tests for PUT /api/priority/weights"""

    def test_update_weights(self, test_client):
        new_weights = PriorityWeights(claim_weight=200.0, decision_weight=180.0)
        response = test_client.put(
            "/api/priority/weights",
            json=new_weights.model_dump(),
        )
        assert response.status_code == 200
        data = response.json()
        assert data["claim_weight"] == 200.0
        assert data["decision_weight"] == 180.0

    def test_updated_weights_persist(self, test_client):
        """Updated weights should be returned by subsequent GET"""
        new_weights = PriorityWeights(claim_weight=300.0)
        test_client.put("/api/priority/weights", json=new_weights.model_dump())

        response = test_client.get("/api/priority/weights")
        assert response.json()["claim_weight"] == 300.0


@pytest.mark.integration
class TestResetPriorityWeights:
    """Tests for POST /api/priority/weights/reset"""

    def test_reset_returns_defaults(self, test_client):
        # First update to non-default
        custom = PriorityWeights(claim_weight=999.0)
        test_client.put("/api/priority/weights", json=custom.model_dump())

        # Reset
        response = test_client.post("/api/priority/reset")
        assert response.status_code == 200
        data = response.json()
        assert data["claim_weight"] == 100.0
