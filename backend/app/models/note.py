"""Pydantic models for notes and related entities"""
from pydantic import BaseModel, Field, field_validator
from typing import List, Optional, Dict, Union, Any
from datetime import datetime
from enum import Enum


class PropertyType(str, Enum):
    """Enum for property types"""
    STRING = "string"
    NUMBER = "number"
    DATE = "date"
    BOOLEAN = "boolean"
    LINK = "link"


class ContentType(str, Enum):
    """Content type enum for priority classification"""
    CLAIM = "claim"
    DECISION = "decision"
    INSIGHT = "insight"
    QUESTION = "question"
    EVIDENCE = "evidence"
    GENERAL = "general"


class NoteType(str, Enum):
    """Note type enum for knowledge hierarchy"""
    CONTAINER = "container"  # Source material, canvas exports, evidence
    ATOMIC = "atomic"        # Distilled single-concept notes
    HUB = "hub"              # Index/map-of-content notes
    GENERAL = "general"      # Default, unclassified notes


class TypedProperty(BaseModel):
    """A typed property with validation"""
    type: PropertyType
    value: Union[str, int, float, bool, datetime]
    label: Optional[str] = None
    
    @field_validator('value')
    @classmethod
    def validate_value_matches_type(cls, v: Any, info) -> Union[str, int, float, bool, datetime]:
        """Validate that value matches the declared type"""
        if not hasattr(info, 'data'):
            return v
        
        prop_type = info.data.get('type')
        
        if prop_type == PropertyType.STRING:
            if not isinstance(v, str):
                raise ValueError(f"String property requires string value, got {type(v).__name__}")
            return v
        elif prop_type == PropertyType.NUMBER:
            if not isinstance(v, (int, float)):
                raise ValueError(f"Number property requires numeric value, got {type(v).__name__}")
            return v
        elif prop_type == PropertyType.DATE:
            if not isinstance(v, datetime):
                raise ValueError(f"Date property requires datetime value, got {type(v).__name__}")
            return v
        elif prop_type == PropertyType.BOOLEAN:
            if not isinstance(v, bool):
                raise ValueError(f"Boolean property requires bool value, got {type(v).__name__}")
            return v
        elif prop_type == PropertyType.LINK:
            if not isinstance(v, str):
                raise ValueError(f"Link property requires string value, got {type(v).__name__}")
            return v
        
        return v
    
    def get_value(self) -> Union[str, int, float, bool, datetime]:
        """Get the property value"""
        return self.value
    
    def set_value(self, new_value: Union[str, int, float, bool, datetime]) -> None:
        """Set a new value with type validation"""
        # Temporarily store type for validation
        temp_data = {'type': self.type, 'value': new_value, 'label': self.label}
        TypedProperty(**temp_data)
        self.value = new_value


class NoteFrontmatter(BaseModel):
    """YAML frontmatter metadata for a note"""
    title: str
    created: Optional[datetime] = None
    modified: Optional[datetime] = None
    tags: List[str] = Field(default_factory=list)
    status: str = "draft"
    aliases: List[str] = Field(default_factory=list)
    # Provenance fields
    source: Optional[str] = None        # "mcp" | "canvas" | None
    source_id: Optional[str] = None     # conversation_id or canvas_session_id
    container_of: List[str] = Field(default_factory=list)  # atomic note IDs
    # Priority scoring fields
    content_type: ContentType = ContentType.GENERAL  # For priority scoring
    # Note type classification
    note_type: NoteType = NoteType.GENERAL  # container/atomic/hub/general
    # Typed properties
    properties: Dict[str, TypedProperty] = Field(default_factory=dict)
    
    def get_property(self, name: str) -> Optional[TypedProperty]:
        """Get a property by name"""
        return self.properties.get(name)
    
    def set_property(self, name: str, prop: TypedProperty) -> None:
        """Set a property"""
        self.properties[name] = prop
    
    def delete_property(self, name: str) -> bool:
        """Delete a property by name"""
        if name in self.properties:
            del self.properties[name]
            return True
        return False
    
    def get_property_value(self, name: str) -> Optional[Union[str, int, float, bool, datetime]]:
        """Get a property value by name"""
        prop = self.properties.get(name)
        return prop.get_value() if prop else None


class Note(BaseModel):
    """Complete note with content and metadata"""
    id: str
    title: str
    content: str
    frontmatter: NoteFrontmatter
    outgoing_links: List[str] = Field(default_factory=list)
    backlinks: List[str] = Field(default_factory=list)


class NoteCreate(BaseModel):
    """Schema for creating a new note"""
    title: str = Field(..., min_length=1, max_length=255)
    content: str = ""
    tags: List[str] = Field(default_factory=list)
    status: str = Field(default="draft", pattern="^(draft|evidence|canonical)$")
    content_type: ContentType = ContentType.GENERAL  # For priority scoring
    note_type: NoteType = NoteType.GENERAL  # container/atomic/hub/general
    properties: Dict[str, TypedProperty] = Field(default_factory=dict)


class NoteUpdate(BaseModel):
    """Schema for updating an existing note"""
    title: Optional[str] = Field(None, min_length=1, max_length=255)
    content: Optional[str] = None
    tags: Optional[List[str]] = None
    status: Optional[str] = Field(None, pattern="^(draft|evidence|canonical)$")
    content_type: Optional[ContentType] = None  # For priority scoring
    note_type: Optional[NoteType] = None  # container/atomic/hub/general
    properties: Optional[Dict[str, TypedProperty]] = None


class NoteListItem(BaseModel):
    """Summary of a note for list views"""
    id: str
    title: str
    status: str = "draft"
    tags: List[str] = Field(default_factory=list)
    created: Optional[datetime] = None
    modified: Optional[datetime] = None
    link_count: int = 0
    note_type: NoteType = NoteType.GENERAL


class SearchResult(BaseModel):
    """Search result item"""
    note_id: str
    title: str
    snippet: str
    score: float = Field(..., ge=0.0, le=1.0)
    tags: List[str] = Field(default_factory=list)


class BacklinkInfo(BaseModel):
    """Information about a backlink to a note"""
    note_id: str
    title: str
    context: str = ""
