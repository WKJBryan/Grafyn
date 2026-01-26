from pydantic import BaseModel, Field
from typing import List, Optional, Dict, Any, Literal
from datetime import datetime


class ParsedMessage(BaseModel):
    index: int
    role: Literal["user", "assistant", "system"]
    content: str
    timestamp: Optional[datetime] = None
    model: Optional[str] = None
    metadata: Dict[str, Any] = Field(default_factory=dict)


class ConversationMetadata(BaseModel):
    platform: Literal["chatgpt", "claude", "grok", "gemini"]
    source_url: Optional[str] = None
    created_at: datetime
    updated_at: datetime
    message_count: int
    model_info: List[str] = Field(default_factory=list)
    platform_specific: Dict[str, Any] = Field(default_factory=dict)


class DuplicateCheck(BaseModel):
    note_id: str
    title: str
    similarity_score: float
    content_type: Optional[str] = None
    tags: List[str] = Field(default_factory=list)


class SummarizationSettings(BaseModel):
    """User preferences for LLM summarization during import"""
    model_id: str  # OpenRouter model ID selected by user
    detail_level: Literal["brief", "standard", "detailed"] = "detailed"
    max_tokens: int = 4096  # Higher for in-depth summaries
    temperature: float = Field(default=0.3, ge=0.0, le=1.0)  # Lower for consistent output


class ConversationQuality(BaseModel):
    """LLM-assessed quality metrics for import filtering"""
    relevance_score: float = Field(ge=0.0, le=1.0)  # How relevant to user's knowledge base
    informativeness: float = Field(ge=0.0, le=1.0)  # Contains extractable insights
    suggested_action: Literal["import_and_distill", "import_only", "skip"]
    skip_reason: Optional[str] = None
    key_topics: List[str] = Field(default_factory=list)  # For MOC matching
    detailed_summary: Optional[str] = None  # In-depth LLM summary


class ParsedConversation(BaseModel):
    id: str
    title: str
    platform: Literal["chatgpt", "claude", "grok", "gemini"]
    messages: List[ParsedMessage]
    metadata: ConversationMetadata
    suggested_tags: List[str] = Field(default_factory=list)
    duplicate_candidates: List[DuplicateCheck] = Field(default_factory=list)
    # NEW: LLM quality assessment
    quality: Optional[ConversationQuality] = None
    # NEW: Suggested notes to link to
    suggested_links: List[str] = Field(default_factory=list)


class AtomicNoteTemplate(BaseModel):
    title: str
    content: str
    tags: List[str] = Field(default_factory=list)
    content_type: Optional[str] = None
    parent_id: Optional[str] = None
    summary: List[str] = Field(default_factory=list)


class ImportDecision(BaseModel):
    conversation_id: str
    action: Literal["accept", "skip", "merge", "modify"]
    target_note_id: Optional[str] = None
    modifications: Optional[Dict[str, Any]] = None
    distill_option: Literal["container_only", "auto_distill", "custom"] = "auto_distill"
    custom_atoms: Optional[List[AtomicNoteTemplate]] = None
    # NEW: LLM model selection for summarization
    summarization_settings: Optional[SummarizationSettings] = None
    # NEW: Automatically add wikilinks to related notes
    auto_link: bool = True


class ImportErrorDetail(BaseModel):
    type: Literal["parse_error", "duplicate", "validation", "system"]
    message: str
    conversation_id: Optional[str] = None
    severity: Literal["error", "warning", "info"]
    context: Dict[str, Any] = Field(default_factory=dict)


class ImportSummary(BaseModel):
    total_conversations: int
    imported: int
    skipped: int
    merged: int
    failed: int
    notes_created: int
    container_notes: int
    atomic_notes: int
    errors: List[ImportErrorDetail] = Field(default_factory=list)
    warnings: List[ImportErrorDetail] = Field(default_factory=list)


class ImportJob(BaseModel):
    id: str
    status: Literal[
        "uploaded", "parsing", "parsed", "reviewing", "applying", "completed", "failed"
    ]
    file_path: str
    file_name: str
    platform: Optional[str] = None
    total_conversations: int = 0
    parsed_conversations: Optional[List[ParsedConversation]] = None
    decisions: Optional[List[ImportDecision]] = None
    created_at: datetime
    updated_at: datetime
    errors: List[ImportErrorDetail] = Field(default_factory=list)


class PreviewResult(BaseModel):
    job_id: str
    total_conversations: int
    conversations: List[ParsedConversation]
    platform: Optional[str] = None
    estimated_notes_to_create: int
    duplicates_found: int
