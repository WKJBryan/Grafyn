"""Tests for PrioritySettingsService"""
import json
import pytest
from pathlib import Path

from app.services.priority_settings import PrioritySettingsService, PrioritySettings
from app.services.priority_scoring import PriorityWeights


@pytest.mark.unit
class TestPrioritySettingsService:
    """Tests for PrioritySettingsService CRUD operations"""

    def test_init_with_defaults(self, priority_settings_service):
        """Service should initialize with default weights"""
        weights = priority_settings_service.get_weights()
        assert weights.claim_weight == 100.0
        assert weights.decision_weight == 90.0
        assert weights.insight_weight == 80.0

    def test_get_weights_returns_priority_weights(self, priority_settings_service):
        """get_weights should return a PriorityWeights instance"""
        weights = priority_settings_service.get_weights()
        assert isinstance(weights, PriorityWeights)

    def test_update_weights(self, priority_settings_service):
        """update_weights should persist and return new weights"""
        new_weights = PriorityWeights(
            claim_weight=200.0,
            decision_weight=180.0,
            recency_decay=0.25,
        )
        result = priority_settings_service.update_weights(new_weights)
        assert result.claim_weight == 200.0
        assert result.decision_weight == 180.0
        assert result.recency_decay == 0.25

    def test_update_weights_persists_to_file(self, tmp_path):
        """Weights should persist to JSON on disk"""
        settings_file = tmp_path / "priority_settings.json"
        service = PrioritySettingsService(settings_file=settings_file)

        new_weights = PriorityWeights(claim_weight=150.0)
        service.update_weights(new_weights)

        assert settings_file.exists()
        data = json.loads(settings_file.read_text(encoding="utf-8"))
        assert data["weights"]["claim_weight"] == 150.0

    def test_load_persisted_weights(self, tmp_path):
        """A new service instance should load previously saved weights"""
        settings_file = tmp_path / "priority_settings.json"

        # Save weights
        service1 = PrioritySettingsService(settings_file=settings_file)
        service1.update_weights(PriorityWeights(claim_weight=999.0))

        # New instance should load them
        service2 = PrioritySettingsService(settings_file=settings_file)
        assert service2.get_weights().claim_weight == 999.0

    def test_reset_to_defaults(self, priority_settings_service):
        """reset_to_defaults should restore default PriorityWeights"""
        # Change weights first
        priority_settings_service.update_weights(
            PriorityWeights(claim_weight=999.0)
        )
        assert priority_settings_service.get_weights().claim_weight == 999.0

        # Reset
        result = priority_settings_service.reset_to_defaults()
        assert result.claim_weight == 100.0
        assert result.decision_weight == 90.0

    def test_get_content_type_scores(self, priority_settings_service):
        """get_content_type_scores should return a dict of all type scores"""
        scores = priority_settings_service.get_content_type_scores()
        assert "claim" in scores
        assert "decision" in scores
        assert "insight" in scores
        assert "question" in scores
        assert "evidence" in scores
        assert "general" in scores
        assert scores["claim"] == 100.0

    def test_get_recency_config(self, priority_settings_service):
        """get_recency_config should include decay_rate and description"""
        config = priority_settings_service.get_recency_config()
        assert "decay_rate" in config
        assert "description" in config
        assert config["decay_rate"] == 0.10

    def test_get_full_config(self, priority_settings_service):
        """get_full_config should include all config sections"""
        config = priority_settings_service.get_full_config()
        assert "content_type_scores" in config
        assert "recency" in config
        assert "link_density" in config
        assert "tag_relevance" in config
        assert "semantic" in config

    def test_missing_file_uses_defaults(self, tmp_path):
        """When settings file doesn't exist, defaults should be used"""
        settings_file = tmp_path / "nonexistent" / "settings.json"
        service = PrioritySettingsService(settings_file=settings_file)
        weights = service.get_weights()
        assert weights.claim_weight == 100.0

    def test_corrupted_file_uses_defaults(self, tmp_path):
        """When settings file is corrupted, defaults should be used"""
        settings_file = tmp_path / "priority_settings.json"
        settings_file.write_text("not valid json{{{", encoding="utf-8")
        service = PrioritySettingsService(settings_file=settings_file)
        weights = service.get_weights()
        assert weights.claim_weight == 100.0
