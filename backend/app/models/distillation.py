"""Pydantic models for distillation workflow"""
from pydantic import BaseModel, Field
from typing import List, Optional, TYPE_CHECKING
from enum import Enum

if TYPE_CHECKING:
    pass  # Avoid circular import


# ============================================================================
# ZETTELKASTEN MODELS
# ============================================================================

class ZettelType(str, Enum):
    """Zettelkasten note types following atomic principles"""
    CONCEPT = "concept"      # Definitions, explanations of ideas
    CLAIM = "claim"          # Assertions, hypotheses needing evidence
    EVIDENCE = "evidence"    # Data, research, examples supporting claims
    QUESTION = "question"    # Inquiries driving exploration
    FLECHE = "fleche"        # Structure/argument chain notes (connecting ideas)
    FLEETING = "fleeting"    # Quick temporary captures


class LinkMode(str, Enum):
    """Link discovery modes for Zettelkasten"""
    AUTOMATIC = "automatic"      # Create all links without review
    SUGGESTED = "suggested"      # Suggest links for user approval
    MANUAL = "manual"            # User triggers on-demand


class LinkType(str, Enum):
    """Types of relationships between Zettelkasten notes"""
    RELATED = "related"          # General conceptual relationship
    SUPPORTS = "supports"        # Evidence supports claim
    CONTRADICTS = "contradicts"  # Notes contradict each other
    EXPANDS = "expands"          # One note expands on another
    QUESTIONS = "questions"      # Note questions another
    ANSWERS = "answers"          # Note answers another
    EXAMPLE = "example"          # Note is example of concept
    PART_OF = "part_of"         # Part-whole relationship


class ZettelLinkCandidate(BaseModel):
    """A candidate link between notes for Zettelkasten"""
    target_id: str = ""  # Will be resolved when creating links
    target_title: str
    link_type: LinkType = LinkType.RELATED
    confidence: float = Field(default=0.5, ge=0.0, le=1.0)
    reason: str = ""  # Why this link is suggested


class ZettelNoteCandidate(BaseModel):
    """Enhanced atomic note candidate with Zettelkasten metadata"""
    id: str
    title: str
    zettel_type: ZettelType = ZettelType.CONCEPT
    content: str = ""  # Full note content with proper structure
    summary: List[str] = Field(default_factory=list)
    key_claims: List[str] = Field(default_factory=list)
    open_questions: List[str] = Field(default_factory=list)
    recommended_tags: List[str] = Field(default_factory=list)
    confidence: float = Field(default=0.5, ge=0.0, le=1.0)
    suggested_links: List[ZettelLinkCandidate] = Field(default_factory=list)
    source_section: Optional[str] = None


# ============================================================================
# ORIGINAL DISTILLATION MODELS
# ============================================================================

class DistillMode(str, Enum):
    """Mode for distillation operation"""
    SUGGEST = "suggest"  # Return candidates without applying
    APPLY = "apply"      # Create/update notes based on decisions
    AUTO = "auto"        # Summarize + auto-create drafts (no review)


class ExtractionMethod(str, Enum):
    """Method for extracting atomic notes"""
    LLM = "llm"          # Use LLM summarization
    RULES = "rules"      # Use rule-based extraction
    AUTO = "auto"        # Auto-select (prefer LLM, fallback to rules)


class HubPolicy(str, Enum):
    """Policy for hub creation"""
    AUTO = "auto"        # Create hubs automatically
    MANUAL = "manual"    # Only suggest, don't create


class CandidateAction(str, Enum):
    """User's decision for a candidate"""
    CREATE = "create"   # Create new atomic note
    APPEND = "append"   # Merge into existing note
    SKIP = "skip"       # User rejected this candidate


class DuplicateMatch(BaseModel):
    """Details about a potential duplicate for user review"""
    note_id: str
    title: str
    score: float = Field(ge=0.0, le=1.0)
    title_similarity: float = Field(ge=0.0, le=1.0)
    snippet: str = ""  # Show user why we think it matches


class AtomicNoteCandidate(BaseModel):
    """Candidate extracted from container note"""
    id: str  # Unique ID for frontend reference
    title: str
    summary: List[str] = Field(default_factory=list, max_length=6)  # 3-6 bullets
    key_claims: List[str] = Field(default_factory=list)
    open_questions: List[str] = Field(default_factory=list)
    recommended_tags: List[str] = Field(default_factory=list, max_length=5)
    confidence: float = Field(default=0.5, ge=0.0, le=1.0)
    suggested_hub: Optional[str] = None
    duplicate_match: Optional[DuplicateMatch] = None
    # Source info for provenance
    source_section: Optional[str] = None  # Section header in container


class CandidateDecision(BaseModel):
    """User's decision for a single candidate (sent with APPLY)"""
    candidate_id: str
    action: CandidateAction
    hub_title: Optional[str] = None  # Override suggested hub
    custom_title: Optional[str] = None  # Override extracted title
    custom_tags: Optional[List[str]] = None  # Override suggested tags


class DistillRequest(BaseModel):
    """Request for distillation endpoint"""
    mode: DistillMode = DistillMode.SUGGEST
    hub_policy: HubPolicy = HubPolicy.AUTO
    extraction_method: ExtractionMethod = ExtractionMethod.AUTO
    min_score: float = Field(default=0.85, ge=0.0, le=1.0)
    # For APPLY mode: user decisions from dialog
    decisions: List[CandidateDecision] = Field(default_factory=list)
    # Original candidates (for APPLY mode context)
    candidates: List[AtomicNoteCandidate] = Field(default_factory=list)


class HubUpdate(BaseModel):
    """Typed hub update info"""
    hub_id: str
    hub_title: str
    action: str  # "created" | "updated"
    atomic_ids_added: List[str] = Field(default_factory=list)


class DistillResponse(BaseModel):
    """Response from distillation endpoint"""
    summary: Optional[str] = None  # LLM-generated summary (AUTO mode)
    candidates: List[AtomicNoteCandidate] = Field(default_factory=list)
    created_note_ids: List[str] = Field(default_factory=list)
    updated_note_ids: List[str] = Field(default_factory=list)
    hub_updates: List[HubUpdate] = Field(default_factory=list)
    container_updated: bool = False
    message: str = ""
    extraction_method_used: Optional[str] = None  # "llm" or "rules"
    status: Optional[str] = None  # Progress status for async operations

