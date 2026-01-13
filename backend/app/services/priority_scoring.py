"""Priority scoring service for context retrieval prioritization"""
import logging
from typing import List, Dict, Optional, Any
from datetime import datetime, timezone, timedelta
from enum import Enum
from pydantic import BaseModel, Field

from backend.app.config import get_settings

logger = logging.getLogger(__name__)
settings = get_settings()


class ContentType(str, Enum):
    """Content type enum for priority classification"""
    CLAIM = "claim"
    DECISION = "decision"
    INSIGHT = "insight"
    QUESTION = "question"
    EVIDENCE = "evidence"
    GENERAL = "general"


class PriorityWeights(BaseModel):
    """User-configurable priority weights"""
    # Content type base scores
    claim_weight: float = Field(default=100.0, ge=0.0)
    decision_weight: float = Field(default=90.0, ge=0.0)
    insight_weight: float = Field(default=80.0, ge=0.0)
    question_weight: float = Field(default=70.0, ge=0.0)
    evidence_weight: float = Field(default=60.0, ge=0.0)
    general_weight: float = Field(default=50.0, ge=0.0)
    
    # Recency decay factor (percentage per month)
    recency_decay: float = Field(default=0.10, ge=0.0, le=1.0)
    
    # Link density boost
    link_density_boost: float = Field(default=5.0, ge=0.0)
    
    # Tag relevance boost
    tag_relevance_boost: float = Field(default=15.0, ge=0.0)
    
    # Semantic similarity weight (from vector search)
    semantic_weight: float = Field(default=1.0, ge=0.0)


class PriorityScore(BaseModel):
    """Priority score result with breakdown"""
    total_score: float
    base_score: float
    recency_score: float
    link_score: float
    tag_score: float
    semantic_score: float
    content_type: ContentType
    breakdown: Dict[str, float]


class PriorityScoringService:
    """Service for calculating priority scores for context retrieval"""
    
    def __init__(self, weights: Optional[PriorityWeights] = None):
        """Initialize priority scoring service"""
        self.weights = weights or PriorityWeights()
    
    def get_content_type_weight(self, content_type: ContentType) -> float:
        """Get base weight for a content type"""
        weight_map = {
            ContentType.CLAIM: self.weights.claim_weight,
            ContentType.DECISION: self.weights.decision_weight,
            ContentType.INSIGHT: self.weights.insight_weight,
            ContentType.QUESTION: self.weights.question_weight,
            ContentType.EVIDENCE: self.weights.evidence_weight,
            ContentType.GENERAL: self.weights.general_weight,
        }
        return weight_map.get(content_type, self.weights.general_weight)
    
    def calculate_recency_score(
        self, 
        modified_date: Optional[datetime], 
        base_score: float
    ) -> float:
        """
        Calculate recency score with time decay.
        
        Args:
            modified_date: When the note was last modified
            base_score: The base content type score to decay from
            
        Returns:
            Recency-adjusted score
        """
        if not modified_date:
            # If no date, assume recent (no penalty)
            return base_score
        
        now = datetime.now(timezone.utc)
        age = now - modified_date
        
        # Calculate months since modification
        months_old = age.total_seconds() / (30 * 24 * 60 * 60)
        
        # Apply exponential decay: score * (1 - decay_rate) ^ months
        decay_factor = (1.0 - self.weights.recency_decay) ** months_old
        recency_score = base_score * decay_factor
        
        # Ensure score doesn't go below 10% of base
        min_score = base_score * 0.1
        return max(recency_score, min_score)
    
    def calculate_link_score(
        self, 
        backlink_count: int,
        outgoing_link_count: int
    ) -> float:
        """
        Calculate link density score.
        
        Args:
            backlink_count: Number of notes linking to this note
            outgoing_link_count: Number of links from this note
            
        Returns:
            Link density bonus score
        """
        total_links = backlink_count + outgoing_link_count
        
        # Boost based on total link count (capped at 10 links for max boost)
        link_bonus = min(total_links, 10) * self.weights.link_density_boost
        
        # Extra boost for backlinks (more valuable than outgoing)
        backlink_bonus = min(backlink_count, 5) * (self.weights.link_density_boost * 0.5)
        
        return link_bonus + backlink_bonus
    
    def calculate_tag_score(
        self,
        note_tags: List[str],
        query_tags: List[str]
    ) -> float:
        """
        Calculate tag relevance score.
        
        Args:
            note_tags: Tags on the note
            query_tags: Tags from the query
            
        Returns:
            Tag relevance bonus score
        """
        if not query_tags or not note_tags:
            return 0.0
        
        # Normalize to lowercase for comparison
        note_tags_lower = [t.lower() for t in note_tags]
        query_tags_lower = [t.lower() for t in query_tags]
        
        # Count matching tags (including hierarchical matches)
        matches = 0
        for query_tag in query_tags_lower:
            for note_tag in note_tags_lower:
                if note_tag == query_tag or note_tag.startswith(query_tag + '/'):
                    matches += 1
                    break
        
        # Bonus per matching tag (capped at 3 matches)
        match_bonus = min(matches, 3) * self.weights.tag_relevance_boost
        
        return match_bonus
    
    def calculate_priority_score(
        self,
        content_type: ContentType,
        modified_date: Optional[datetime],
        backlink_count: int = 0,
        outgoing_link_count: int = 0,
        note_tags: Optional[List[str]] = None,
        query_tags: Optional[List[str]] = None,
        semantic_score: float = 0.0,
    ) -> PriorityScore:
        """
        Calculate comprehensive priority score for a note or tile.
        
        Args:
            content_type: Type of content (claim, decision, etc.)
            modified_date: When the content was last modified
            backlink_count: Number of backlinks
            outgoing_link_count: Number of outgoing links
            note_tags: Tags on the note
            query_tags: Tags from the query for relevance matching
            semantic_score: Semantic similarity score from vector search (0-1)
            
        Returns:
            PriorityScore with total and breakdown
        """
        note_tags = note_tags or []
        query_tags = query_tags or []
        
        # 1. Base score from content type
        base_score = self.get_content_type_weight(content_type)
        
        # 2. Recency score (decay over time)
        recency_score = self.calculate_recency_score(modified_date, base_score)
        
        # 3. Link density bonus
        link_score = self.calculate_link_score(backlink_count, outgoing_link_count)
        
        # 4. Tag relevance bonus
        tag_score = self.calculate_tag_score(note_tags, query_tags)
        
        # 5. Semantic similarity contribution
        semantic_contribution = semantic_score * self.weights.semantic_weight * 100
        
        # Total score
        total_score = recency_score + link_score + tag_score + semantic_contribution
        
        # Build breakdown
        breakdown = {
            "base_score": base_score,
            "recency_decay": recency_score,
            "link_density": link_score,
            "tag_relevance": tag_score,
            "semantic_similarity": semantic_contribution,
        }
        
        return PriorityScore(
            total_score=total_score,
            base_score=base_score,
            recency_score=recency_score,
            link_score=link_score,
            tag_score=tag_score,
            semantic_score=semantic_contribution,
            content_type=content_type,
            breakdown=breakdown,
        )
    
    def score_search_results(
        self,
        results: List[Dict[str, Any]],
        query_tags: List[str],
        knowledge_store=None,
    ) -> List[Dict[str, Any]]:
        """
        Apply priority scoring to search results.
        
        Args:
            results: List of search results from vector_search
            query_tags: Tags extracted from the query
            knowledge_store: Optional knowledge store for metadata
            
        Returns:
            Results with priority scores added, sorted by score
        """
        scored_results = []
        
        for result in results:
            # Determine content type (default to general)
            content_type = ContentType.GENERAL
            
            # Try to get content type from result metadata
            if 'content_type' in result:
                try:
                    content_type = ContentType(result['content_type'])
                except ValueError:
                    content_type = ContentType.GENERAL
            
            # Get metadata for scoring
            modified_date = result.get('modified')
            backlink_count = result.get('backlink_count', 0)
            outgoing_link_count = result.get('outgoing_link_count', 0)
            note_tags = result.get('tags', [])
            semantic_score = result.get('score', 0.0)
            
            # If we have knowledge_store, fetch additional metadata
            if knowledge_store and 'note_id' in result:
                note = knowledge_store.get_note(result['note_id'])
                if note:
                    backlink_count = len(note.backlinks)
                    outgoing_link_count = len(note.outgoing_links)
                    note_tags = note.frontmatter.tags if note.frontmatter else []
                    modified_date = note.frontmatter.modified if note.frontmatter else None
                    
                    # Try to infer content type from frontmatter properties
                    if note.frontmatter and note.frontmatter.properties:
                        if 'content_type' in note.frontmatter.properties:
                            try:
                                content_type = ContentType(
                                    note.frontmatter.get_property_value('content_type')
                                )
                            except (ValueError, TypeError):
                                pass
            
            # Calculate priority score
            priority = self.calculate_priority_score(
                content_type=content_type,
                modified_date=modified_date,
                backlink_count=backlink_count,
                outgoing_link_count=outgoing_link_count,
                note_tags=note_tags,
                query_tags=query_tags,
                semantic_score=semantic_score,
            )
            
            # Add priority to result
            result['priority_score'] = priority.total_score
            result['priority_breakdown'] = priority.breakdown
            result['content_type'] = content_type.value
            
            scored_results.append(result)
        
        # Sort by priority score descending
        scored_results.sort(key=lambda x: x.get('priority_score', 0), reverse=True)
        
        return scored_results
    
    def infer_content_type_from_content(self, content: str) -> ContentType:
        """
        Infer content type from text content using heuristics.
        
        Args:
            content: The text content to analyze
            
        Returns:
            Inferred content type
        """
        content_lower = content.lower()
        
        # Claim indicators
        claim_indicators = [
            'i claim', 'we claim', 'it is claimed', 'the claim is',
            'assertion', 'asserts', 'asserting', 'our position'
        ]
        if any(indicator in content_lower for indicator in claim_indicators):
            return ContentType.CLAIM
        
        # Decision indicators
        decision_indicators = [
            'decided to', 'we decided', 'decision made', 'the decision',
            'chose to', 'we chose', 'final decision', 'concluded that'
        ]
        if any(indicator in content_lower for indicator in decision_indicators):
            return ContentType.DECISION
        
        # Insight indicators
        insight_indicators = [
            'insight', 'realized that', 'discovered that', 'found that',
            'key insight', 'important realization', 'learned that'
        ]
        if any(indicator in content_lower for indicator in insight_indicators):
            return ContentType.INSIGHT
        
        # Question indicators
        if content.strip().endswith('?'):
            return ContentType.QUESTION
        if content_lower.startswith(('what', 'why', 'how', 'when', 'where', 'who', 'which')):
            return ContentType.QUESTION
        
        # Evidence indicators
        evidence_indicators = [
            'evidence shows', 'data indicates', 'research suggests',
            'studies show', 'according to', 'based on evidence'
        ]
        if any(indicator in content_lower for indicator in evidence_indicators):
            return ContentType.EVIDENCE
        
        return ContentType.GENERAL
    
    def update_weights(self, new_weights: PriorityWeights) -> None:
        """
        Update priority weights.
        
        Args:
            new_weights: New weight configuration
        """
        self.weights = new_weights
        logger.info("Updated priority weights")
    
    def get_weights(self) -> PriorityWeights:
        """Get current priority weights"""
        return self.weights
