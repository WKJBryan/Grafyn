"""Pydantic models for distillation workflow"""
from pydantic import BaseModel, Field
from typing import List, Optional
from enum import Enum


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

