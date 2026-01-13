"""Unit tests for priority scoring service"""
import pytest
from datetime import datetime, timezone, timedelta
from app.services.priority_scoring import (
    PriorityScoringService,
    PriorityWeights,
    ContentType,
    PriorityScore
)


class TestPriorityScoringService:
    """Test cases for PriorityScoringService"""
    
    def test_default_weights(self):
        """Test that default weights are set correctly"""
        service = PriorityScoringService()
        weights = service.get_weights()
        
        assert weights.claim_weight == 100.0
        assert weights.decision_weight == 90.0
        assert weights.insight_weight == 80.0
        assert weights.question_weight == 70.0
        assert weights.evidence_weight == 60.0
        assert weights.general_weight == 50.0
        assert weights.recency_decay == 0.10
        assert weights.link_density_boost == 5.0
        assert weights.tag_relevance_boost == 15.0
        assert weights.semantic_weight == 1.0
    
    def test_get_content_type_weight(self):
        """Test getting weights for different content types"""
        service = PriorityScoringService()
        
        assert service.get_content_type_weight(ContentType.CLAIM) == 100.0
        assert service.get_content_type_weight(ContentType.DECISION) == 90.0
        assert service.get_content_type_weight(ContentType.INSIGHT) == 80.0
        assert service.get_content_type_weight(ContentType.QUESTION) == 70.0
        assert service.get_content_type_weight(ContentType.EVIDENCE) == 60.0
        assert service.get_content_type_weight(ContentType.GENERAL) == 50.0
    
    def test_recency_score_recent(self):
        """Test recency score for recent notes"""
        service = PriorityScoringService()
        now = datetime.now(timezone.utc)
        
        # Recent note (1 day old)
        score = service.calculate_recency_score(now - timedelta(days=1), 100.0)
        assert score > 95.0  # Should be close to base score
    
    def test_recency_score_old(self):
        """Test recency score for old notes"""
        service = PriorityScoringService()
        now = datetime.now(timezone.utc)
        
        # Old note (1 year old)
        score = service.calculate_recency_score(now - timedelta(days=365), 100.0)
        assert score < 50.0  # Should be significantly reduced
    
    def test_recency_score_no_date(self):
        """Test recency score when no date is provided"""
        service = PriorityScoringService()
        
        # No date provided
        score = service.calculate_recency_score(None, 100.0)
        assert score == 100.0  # Should return base score unchanged
    
    def test_link_score(self):
        """Test link density score calculation"""
        service = PriorityScoringService()
        
        # No links
        score = service.calculate_link_score(0, 0)
        assert score == 0.0
        
        # Some links
        score = service.calculate_link_score(2, 3)
        assert score > 0.0
        
        # Many links (capped)
        score = service.calculate_link_score(20, 20)
        assert score < 100.0  # Should be capped
    
    def test_tag_score_no_matches(self):
        """Test tag score with no matches"""
        service = PriorityScoringService()
        
        score = service.calculate_tag_score(['research', 'draft'], ['other', 'test'])
        assert score == 0.0
    
    def test_tag_score_with_matches(self):
        """Test tag score with matches"""
        service = PriorityScoringService()
        
        # One match
        score = service.calculate_tag_score(['research', 'draft'], ['research'])
        assert score > 0.0
        
        # Multiple matches
        score = service.calculate_tag_score(['research', 'draft', 'important'], ['research', 'draft'])
        assert score > 15.0  # Should be higher than single match
    
    def test_tag_score_hierarchical(self):
        """Test tag score with hierarchical matching"""
        service = PriorityScoringService()
        
        # Hierarchical match (research/ai should match research)
        score = service.calculate_tag_score(['research/ai', 'research/ml'], ['research'])
        assert score > 0.0
    
    def test_calculate_priority_score_claim(self):
        """Test priority score for claim content type"""
        service = PriorityScoringService()
        now = datetime.now(timezone.utc)
        
        result = service.calculate_priority_score(
            content_type=ContentType.CLAIM,
            modified_date=now,
            backlink_count=5,
            outgoing_link_count=3,
            note_tags=['important'],
            query_tags=['important'],
            semantic_score=0.8,
        )
        
        assert isinstance(result, PriorityScore)
        assert result.content_type == ContentType.CLAIM
        assert result.base_score == 100.0
        assert result.total_score > 100.0  # Should have bonuses
    
    def test_calculate_priority_score_general(self):
        """Test priority score for general content type"""
        service = PriorityScoringService()
        now = datetime.now(timezone.utc)
        
        result = service.calculate_priority_score(
            content_type=ContentType.GENERAL,
            modified_date=now,
            backlink_count=0,
            outgoing_link_count=0,
            note_tags=[],
            query_tags=[],
            semantic_score=0.5,
        )
        
        assert isinstance(result, PriorityScore)
        assert result.content_type == ContentType.GENERAL
        assert result.base_score == 50.0
        # Should have some semantic contribution
        assert result.semantic_score > 0.0
    
    def test_score_search_results(self):
        """Test scoring search results"""
        service = PriorityScoringService()
        
        results = [
            {
                'note_id': '1',
                'title': 'Test Note 1',
                'snippet': 'Content 1',
                'score': 0.8,
                'tags': ['important'],
                'modified': datetime.now(timezone.utc).isoformat(),
            },
            {
                'note_id': '2',
                'title': 'Test Note 2',
                'snippet': 'Content 2',
                'score': 0.9,
                'tags': [],
                'modified': datetime.now(timezone.utc).isoformat(),
            },
        ]
        
        scored = service.score_search_results(results, ['important'], None)
        
        # First result should have higher priority due to tag match
        assert scored[0]['note_id'] == '1'
        assert 'priority_score' in scored[0]
        assert 'priority_breakdown' in scored[0]
        assert 'content_type' in scored[0]
    
    def test_update_weights(self):
        """Test updating priority weights"""
        service = PriorityScoringService()
        
        new_weights = PriorityWeights(
            claim_weight=150.0,
            decision_weight=140.0,
        )
        
        service.update_weights(new_weights)
        
        assert service.get_weights().claim_weight == 150.0
        assert service.get_weights().decision_weight == 140.0
    
    def test_infer_content_type_claim(self):
        """Test inferring claim content type"""
        service = PriorityScoringService()
        
        content = "I claim that this approach is optimal for our use case."
        inferred = service.infer_content_type_from_content(content)
        
        assert inferred == ContentType.CLAIM
    
    def test_infer_content_type_decision(self):
        """Test inferring decision content type"""
        service = PriorityScoringService()
        
        content = "We decided to use Python for the backend implementation."
        inferred = service.infer_content_type_from_content(content)
        
        assert inferred == ContentType.DECISION
    
    def test_infer_content_type_insight(self):
        """Test inferring insight content type"""
        service = PriorityScoringService()
        
        content = "Key insight: The performance bottleneck is in the database layer."
        inferred = service.infer_content_type_from_content(content)
        
        assert inferred == ContentType.INSIGHT
    
    def test_infer_content_type_question(self):
        """Test inferring question content type"""
        service = PriorityScoringService()
        
        content = "What is the best approach for handling this error?"
        inferred = service.infer_content_type_from_content(content)
        
        assert inferred == ContentType.QUESTION
    
    def test_infer_content_type_evidence(self):
        """Test inferring evidence content type"""
        service = PriorityScoringService()
        
        content = "Evidence shows that the new algorithm is 2x faster."
        inferred = service.infer_content_type_from_content(content)
        
        assert inferred == ContentType.EVIDENCE
    
    def test_infer_content_type_general(self):
        """Test inferring general content type"""
        service = PriorityScoringService()
        
        content = "This is just some general information about the project."
        inferred = service.infer_content_type_from_content(content)
        
        assert inferred == ContentType.GENERAL
