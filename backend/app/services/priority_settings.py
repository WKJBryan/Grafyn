"""Priority settings service for user-configurable priority rules"""
import json
import logging
from pathlib import Path
from typing import Optional
from pydantic import BaseModel, Field

from backend.app.config import get_settings
from backend.app.services.priority_scoring import PriorityWeights

logger = logging.getLogger(__name__)
settings = get_settings()


class PrioritySettings(BaseModel):
    """User-configurable priority settings"""
    weights: PriorityWeights = Field(default_factory=PriorityWeights)
    
    def save(self, file_path: Optional[Path] = None):
        """Save settings to file"""
        if file_path is None:
            file_path = Path(settings.data_path) / "priority_settings.json"
        
        file_path.parent.mkdir(parents=True, exist_ok=True)
        
        with open(file_path, 'w', encoding='utf-8') as f:
            json.dump(self.model_dump(), f, indent=2, default=str)
        
        logger.info(f"Saved priority settings to {file_path}")
    
    @classmethod
    def load(cls, file_path: Optional[Path] = None) -> 'PrioritySettings':
        """Load settings from file"""
        if file_path is None:
            file_path = Path(settings.data_path) / "priority_settings.json"
        
        if not file_path.exists():
            logger.info(f"Priority settings file not found, using defaults")
            return cls()
        
        try:
            with open(file_path, 'r', encoding='utf-8') as f:
                data = json.load(f)
            
            # Load weights with validation
            weights_data = data.get('weights', {})
            weights = PriorityWeights(**weights_data)
            
            return cls(weights=weights)
        except Exception as e:
            logger.warning(f"Failed to load priority settings: {e}, using defaults")
            return cls()


class PrioritySettingsService:
    """Service for managing priority settings"""
    
    def __init__(self, settings_file: Optional[Path] = None):
        """Initialize priority settings service"""
        self.settings_file = settings_file
        self._settings: Optional[PrioritySettings] = None
    
    @property
    def settings(self) -> PrioritySettings:
        """Get current settings (lazy load)"""
        if self._settings is None:
            self._settings = PrioritySettings.load(self.settings_file)
        return self._settings
    
    def get_weights(self) -> PriorityWeights:
        """Get current priority weights"""
        return self.settings.weights
    
    def update_weights(self, weights: PriorityWeights) -> PriorityWeights:
        """
        Update priority weights.
        
        Args:
            weights: New weight configuration
            
        Returns:
            Updated weights
        """
        self.settings.weights = weights
        self.settings.save(self.settings_file)
        logger.info("Updated priority weights")
        return weights
    
    def reset_to_defaults(self) -> PriorityWeights:
        """Reset weights to default values"""
        default_weights = PriorityWeights()
        return self.update_weights(default_weights)
    
    def get_content_type_scores(self) -> dict:
        """Get a summary of content type base scores"""
        weights = self.get_weights()
        return {
            'claim': weights.claim_weight,
            'decision': weights.decision_weight,
            'insight': weights.insight_weight,
            'question': weights.question_weight,
            'evidence': weights.evidence_weight,
            'general': weights.general_weight,
        }
    
    def get_recency_config(self) -> dict:
        """Get recency decay configuration"""
        weights = self.get_weights()
        return {
            'decay_rate': weights.recency_decay,
            'description': 'Percentage score decay per month (0.0-1.0)'
        }
    
    def get_link_density_config(self) -> dict:
        """Get link density boost configuration"""
        weights = self.get_weights()
        return {
            'boost_per_link': weights.link_density_boost,
            'description': 'Score boost per link (capped at 10 links)'
        }
    
    def get_tag_relevance_config(self) -> dict:
        """Get tag relevance boost configuration"""
        weights = self.get_weights()
        return {
            'boost_per_match': weights.tag_relevance_boost,
            'description': 'Score boost per matching tag (capped at 3 matches)'
        }
    
    def get_semantic_config(self) -> dict:
        """Get semantic similarity weight configuration"""
        weights = self.get_weights()
        return {
            'weight': weights.semantic_weight,
            'description': 'Multiplier for semantic similarity score (0-1)'
        }
    
    def get_full_config(self) -> dict:
        """Get complete priority configuration"""
        return {
            'content_type_scores': self.get_content_type_scores(),
            'recency': self.get_recency_config(),
            'link_density': self.get_link_density_config(),
            'tag_relevance': self.get_tag_relevance_config(),
            'semantic': self.get_semantic_config(),
        }
