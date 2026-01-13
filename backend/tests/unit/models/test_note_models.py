"""
Unit tests for Pydantic note models

Tests validation, defaults, and serialization for all note-related models
"""
import pytest
from datetime import datetime
from pydantic import ValidationError

from app.models.note import (
    NoteFrontmatter,
    Note,
    NoteCreate,
    NoteUpdate,
    NoteListItem,
    SearchResult,
    BacklinkInfo,
    TypedProperty,
    PropertyType,
)


# ============================================================================
# NoteFrontmatter Tests
# ============================================================================

class TestNoteFrontmatter:
    """Tests for NoteFrontmatter model"""

    def test_requires_title(self):
        """Title is required"""
        with pytest.raises(ValidationError):
            NoteFrontmatter()

    def test_valid_frontmatter(self):
        """Should create valid frontmatter"""
        fm = NoteFrontmatter(title="Test Note")

        assert fm.title == "Test Note"
        assert fm.status == "draft"
        assert fm.tags == []
        assert fm.aliases == []

    def test_defaults_created_to_none(self):
        """Created should default to None"""
        fm = NoteFrontmatter(title="Test")

        assert fm.created is None

    def test_defaults_modified_to_none(self):
        """Modified should default to None"""
        fm = NoteFrontmatter(title="Test")

        assert fm.modified is None

    def test_defaults_tags_to_empty_list(self):
        """Tags should default to empty list"""
        fm = NoteFrontmatter(title="Test")

        assert fm.tags == []
        assert isinstance(fm.tags, list)

    def test_defaults_status_to_draft(self):
        """Status should default to draft"""
        fm = NoteFrontmatter(title="Test")

        assert fm.status == "draft"

    def test_defaults_aliases_to_empty_list(self):
        """Aliases should default to empty list"""
        fm = NoteFrontmatter(title="Test")

        assert fm.aliases == []
        assert isinstance(fm.aliases, list)

    def test_accepts_datetime_created(self):
        """Should accept datetime for created"""
        now = datetime.now()
        fm = NoteFrontmatter(title="Test", created=now)

        assert fm.created == now

    def test_accepts_datetime_modified(self):
        """Should accept datetime for modified"""
        now = datetime.now()
        fm = NoteFrontmatter(title="Test", modified=now)

        assert fm.modified == now

    def test_accepts_tags_list(self):
        """Should accept list of tags"""
        fm = NoteFrontmatter(title="Test", tags=["tag1", "tag2"])

        assert fm.tags == ["tag1", "tag2"]

    def test_accepts_aliases_list(self):
        """Should accept list of aliases"""
        fm = NoteFrontmatter(title="Test", aliases=["alias1", "alias2"])

        assert fm.aliases == ["alias1", "alias2"]

    def test_accepts_custom_status(self):
        """Should accept custom status values"""
        fm = NoteFrontmatter(title="Test", status="canonical")

        assert fm.status == "canonical"


# ============================================================================
# Note Tests
# ============================================================================

class TestNote:
    """Tests for Note model"""

    def test_requires_id(self):
        """ID is required"""
        fm = NoteFrontmatter(title="Test")

        with pytest.raises(ValidationError):
            Note(title="Test", content="", frontmatter=fm)

    def test_requires_title(self):
        """Title is required"""
        fm = NoteFrontmatter(title="Test")

        with pytest.raises(ValidationError):
            Note(id="test", content="", frontmatter=fm)

    def test_requires_content(self):
        """Content is required"""
        fm = NoteFrontmatter(title="Test")

        with pytest.raises(ValidationError):
            Note(id="test", title="Test", frontmatter=fm)

    def test_requires_frontmatter(self):
        """Frontmatter is required"""
        with pytest.raises(ValidationError):
            Note(id="test", title="Test", content="")

    def test_valid_note(self):
        """Should create valid note"""
        fm = NoteFrontmatter(title="Test Note")
        note = Note(
            id="test-note",
            title="Test Note",
            content="Content here",
            frontmatter=fm
        )

        assert note.id == "test-note"
        assert note.title == "Test Note"
        assert note.content == "Content here"

    def test_defaults_outgoing_links_to_empty(self):
        """Outgoing links should default to empty list"""
        fm = NoteFrontmatter(title="Test")
        note = Note(id="test", title="Test", content="", frontmatter=fm)

        assert note.outgoing_links == []

    def test_defaults_backlinks_to_empty(self):
        """Backlinks should default to empty list"""
        fm = NoteFrontmatter(title="Test")
        note = Note(id="test", title="Test", content="", frontmatter=fm)

        assert note.backlinks == []

    def test_accepts_outgoing_links(self):
        """Should accept outgoing links"""
        fm = NoteFrontmatter(title="Test")
        note = Note(
            id="test",
            title="Test",
            content="",
            frontmatter=fm,
            outgoing_links=["link1", "link2"]
        )

        assert note.outgoing_links == ["link1", "link2"]

    def test_accepts_backlinks(self):
        """Should accept backlinks"""
        fm = NoteFrontmatter(title="Test")
        note = Note(
            id="test",
            title="Test",
            content="",
            frontmatter=fm,
            backlinks=["ref1", "ref2"]
        )

        assert note.backlinks == ["ref1", "ref2"]


# ============================================================================
# NoteCreate Tests
# ============================================================================

class TestNoteCreate:
    """Tests for NoteCreate model"""

    def test_requires_title(self):
        """Title is required"""
        with pytest.raises(ValidationError):
            NoteCreate()

    def test_valid_create(self):
        """Should create valid NoteCreate"""
        note = NoteCreate(title="New Note")

        assert note.title == "New Note"
        assert note.content == ""
        assert note.tags == []
        assert note.status == "draft"

    def test_title_min_length(self):
        """Title must have at least 1 character"""
        with pytest.raises(ValidationError):
            NoteCreate(title="")

    def test_title_max_length(self):
        """Title must be at most 255 characters"""
        with pytest.raises(ValidationError):
            NoteCreate(title="x" * 256)

    def test_title_255_chars_valid(self):
        """Title with exactly 255 characters should be valid"""
        note = NoteCreate(title="x" * 255)

        assert len(note.title) == 255

    def test_defaults_content_to_empty(self):
        """Content should default to empty string"""
        note = NoteCreate(title="Test")

        assert note.content == ""

    def test_defaults_tags_to_empty(self):
        """Tags should default to empty list"""
        note = NoteCreate(title="Test")

        assert note.tags == []

    def test_defaults_status_to_draft(self):
        """Status should default to draft"""
        note = NoteCreate(title="Test")

        assert note.status == "draft"

    def test_status_draft_valid(self):
        """Status 'draft' should be valid"""
        note = NoteCreate(title="Test", status="draft")

        assert note.status == "draft"

    def test_status_evidence_valid(self):
        """Status 'evidence' should be valid"""
        note = NoteCreate(title="Test", status="evidence")

        assert note.status == "evidence"

    def test_status_canonical_valid(self):
        """Status 'canonical' should be valid"""
        note = NoteCreate(title="Test", status="canonical")

        assert note.status == "canonical"

    def test_status_invalid_rejected(self):
        """Invalid status should be rejected"""
        with pytest.raises(ValidationError):
            NoteCreate(title="Test", status="invalid")

    def test_status_pending_rejected(self):
        """Status 'pending' should be rejected"""
        with pytest.raises(ValidationError):
            NoteCreate(title="Test", status="pending")

    def test_accepts_content(self):
        """Should accept content"""
        note = NoteCreate(title="Test", content="Some content")

        assert note.content == "Some content"

    def test_accepts_tags(self):
        """Should accept tags"""
        note = NoteCreate(title="Test", tags=["tag1", "tag2"])

        assert note.tags == ["tag1", "tag2"]


# ============================================================================
# NoteUpdate Tests
# ============================================================================

class TestNoteUpdate:
    """Tests for NoteUpdate model"""

    def test_all_fields_optional(self):
        """All fields should be optional"""
        update = NoteUpdate()

        assert update.title is None
        assert update.content is None
        assert update.tags is None
        assert update.status is None

    def test_title_min_length(self):
        """Title must have at least 1 character if provided"""
        with pytest.raises(ValidationError):
            NoteUpdate(title="")

    def test_title_max_length(self):
        """Title must be at most 255 characters if provided"""
        with pytest.raises(ValidationError):
            NoteUpdate(title="x" * 256)

    def test_title_none_valid(self):
        """Title None should be valid (no update)"""
        update = NoteUpdate(title=None)

        assert update.title is None

    def test_accepts_title(self):
        """Should accept title"""
        update = NoteUpdate(title="Updated Title")

        assert update.title == "Updated Title"

    def test_accepts_content(self):
        """Should accept content"""
        update = NoteUpdate(content="Updated content")

        assert update.content == "Updated content"

    def test_accepts_tags(self):
        """Should accept tags"""
        update = NoteUpdate(tags=["new-tag"])

        assert update.tags == ["new-tag"]

    def test_status_draft_valid(self):
        """Status 'draft' should be valid"""
        update = NoteUpdate(status="draft")

        assert update.status == "draft"

    def test_status_evidence_valid(self):
        """Status 'evidence' should be valid"""
        update = NoteUpdate(status="evidence")

        assert update.status == "evidence"

    def test_status_canonical_valid(self):
        """Status 'canonical' should be valid"""
        update = NoteUpdate(status="canonical")

        assert update.status == "canonical"

    def test_status_invalid_rejected(self):
        """Invalid status should be rejected"""
        with pytest.raises(ValidationError):
            NoteUpdate(status="invalid")

    def test_partial_update(self):
        """Should allow partial updates"""
        update = NoteUpdate(title="New Title")

        assert update.title == "New Title"
        assert update.content is None
        assert update.tags is None
        assert update.status is None


# ============================================================================
# NoteListItem Tests
# ============================================================================

class TestNoteListItem:
    """Tests for NoteListItem model"""

    def test_requires_id(self):
        """ID is required"""
        with pytest.raises(ValidationError):
            NoteListItem(title="Test")

    def test_requires_title(self):
        """Title is required"""
        with pytest.raises(ValidationError):
            NoteListItem(id="test")

    def test_valid_list_item(self):
        """Should create valid list item"""
        item = NoteListItem(id="test", title="Test Note")

        assert item.id == "test"
        assert item.title == "Test Note"

    def test_defaults_status_to_draft(self):
        """Status should default to draft"""
        item = NoteListItem(id="test", title="Test")

        assert item.status == "draft"

    def test_defaults_tags_to_empty(self):
        """Tags should default to empty list"""
        item = NoteListItem(id="test", title="Test")

        assert item.tags == []

    def test_defaults_created_to_none(self):
        """Created should default to None"""
        item = NoteListItem(id="test", title="Test")

        assert item.created is None

    def test_defaults_modified_to_none(self):
        """Modified should default to None"""
        item = NoteListItem(id="test", title="Test")

        assert item.modified is None

    def test_defaults_link_count_to_zero(self):
        """Link count should default to 0"""
        item = NoteListItem(id="test", title="Test")

        assert item.link_count == 0

    def test_accepts_all_fields(self):
        """Should accept all fields"""
        now = datetime.now()
        item = NoteListItem(
            id="test",
            title="Test",
            status="canonical",
            tags=["tag1"],
            created=now,
            modified=now,
            link_count=5
        )

        assert item.status == "canonical"
        assert item.tags == ["tag1"]
        assert item.created == now
        assert item.link_count == 5


# ============================================================================
# SearchResult Tests
# ============================================================================

class TestSearchResult:
    """Tests for SearchResult model"""

    def test_requires_note_id(self):
        """Note ID is required"""
        with pytest.raises(ValidationError):
            SearchResult(title="Test", snippet="", score=0.5)

    def test_requires_title(self):
        """Title is required"""
        with pytest.raises(ValidationError):
            SearchResult(note_id="test", snippet="", score=0.5)

    def test_requires_snippet(self):
        """Snippet is required"""
        with pytest.raises(ValidationError):
            SearchResult(note_id="test", title="Test", score=0.5)

    def test_requires_score(self):
        """Score is required"""
        with pytest.raises(ValidationError):
            SearchResult(note_id="test", title="Test", snippet="")

    def test_valid_search_result(self):
        """Should create valid search result"""
        result = SearchResult(
            note_id="test",
            title="Test Note",
            snippet="...matching text...",
            score=0.85
        )

        assert result.note_id == "test"
        assert result.score == 0.85

    def test_score_min_zero(self):
        """Score must be >= 0"""
        with pytest.raises(ValidationError):
            SearchResult(
                note_id="test",
                title="Test",
                snippet="",
                score=-0.1
            )

    def test_score_max_one(self):
        """Score must be <= 1"""
        with pytest.raises(ValidationError):
            SearchResult(
                note_id="test",
                title="Test",
                snippet="",
                score=1.1
            )

    def test_score_zero_valid(self):
        """Score 0 should be valid"""
        result = SearchResult(
            note_id="test",
            title="Test",
            snippet="",
            score=0.0
        )

        assert result.score == 0.0

    def test_score_one_valid(self):
        """Score 1 should be valid"""
        result = SearchResult(
            note_id="test",
            title="Test",
            snippet="",
            score=1.0
        )

        assert result.score == 1.0

    def test_defaults_tags_to_empty(self):
        """Tags should default to empty list"""
        result = SearchResult(
            note_id="test",
            title="Test",
            snippet="",
            score=0.5
        )

        assert result.tags == []

    def test_accepts_tags(self):
        """Should accept tags"""
        result = SearchResult(
            note_id="test",
            title="Test",
            snippet="",
            score=0.5,
            tags=["tag1", "tag2"]
        )

        assert result.tags == ["tag1", "tag2"]


# ============================================================================
# BacklinkInfo Tests
# ============================================================================

class TestBacklinkInfo:
    """Tests for BacklinkInfo model"""

    def test_requires_note_id(self):
        """Note ID is required"""
        with pytest.raises(ValidationError):
            BacklinkInfo(title="Test")

    def test_requires_title(self):
        """Title is required"""
        with pytest.raises(ValidationError):
            BacklinkInfo(note_id="test")

    def test_valid_backlink_info(self):
        """Should create valid backlink info"""
        info = BacklinkInfo(note_id="test", title="Test Note")

        assert info.note_id == "test"
        assert info.title == "Test Note"

    def test_defaults_context_to_empty(self):
        """Context should default to empty string"""
        info = BacklinkInfo(note_id="test", title="Test")

        assert info.context == ""

    def test_accepts_context(self):
        """Should accept context"""
        info = BacklinkInfo(
            note_id="test",
            title="Test",
            context="...text containing [[Test]]..."
        )

        assert "[[Test]]" in info.context


# ============================================================================
# Serialization Tests
# ============================================================================

class TestModelSerialization:
    """Tests for model serialization"""

    def test_note_to_dict(self):
        """Note should serialize to dict"""
        fm = NoteFrontmatter(title="Test")
        note = Note(
            id="test",
            title="Test",
            content="Content",
            frontmatter=fm
        )

        data = note.model_dump()

        assert isinstance(data, dict)
        assert data["id"] == "test"
        assert "frontmatter" in data

    def test_note_create_to_dict(self):
        """NoteCreate should serialize to dict"""
        create = NoteCreate(title="Test", content="Content")

        data = create.model_dump()

        assert isinstance(data, dict)
        assert data["title"] == "Test"

    def test_note_update_excludes_none(self):
        """NoteUpdate should be able to exclude None values"""
        update = NoteUpdate(title="New Title")

        data = update.model_dump(exclude_none=True)

        assert "title" in data
        assert "content" not in data
        assert "tags" not in data

    def test_search_result_to_json(self):
        """SearchResult should serialize to JSON"""
        result = SearchResult(
            note_id="test",
            title="Test",
            snippet="snippet",
            score=0.5
        )

        json_str = result.model_dump_json()

        assert '"note_id"' in json_str
        assert '"score"' in json_str


# ============================================================================
# TypedProperty Tests
# ============================================================================

class TestTypedProperty:
    """Tests for TypedProperty model"""

    def test_requires_type(self):
        """Type is required"""
        with pytest.raises(ValidationError):
            TypedProperty(value="test")

    def test_requires_value(self):
        """Value is required"""
        with pytest.raises(ValidationError):
            TypedProperty(type=PropertyType.STRING)

    def test_label_optional(self):
        """Label should be optional"""
        prop = TypedProperty(type=PropertyType.STRING, value="test")

        assert prop.label is None

    def test_valid_string_property(self):
        """Should create valid string property"""
        prop = TypedProperty(type=PropertyType.STRING, value="Hello World")

        assert prop.type == PropertyType.STRING
        assert prop.value == "Hello World"

    def test_valid_number_property_int(self):
        """Should create valid number property with int"""
        prop = TypedProperty(type=PropertyType.NUMBER, value=42)

        assert prop.type == PropertyType.NUMBER
        assert prop.value == 42

    def test_valid_number_property_float(self):
        """Should create valid number property with float"""
        prop = TypedProperty(type=PropertyType.NUMBER, value=3.14)

        assert prop.type == PropertyType.NUMBER
        assert prop.value == 3.14

    def test_valid_date_property(self):
        """Should create valid date property"""
        now = datetime.now()
        prop = TypedProperty(type=PropertyType.DATE, value=now)

        assert prop.type == PropertyType.DATE
        assert prop.value == now

    def test_valid_boolean_property(self):
        """Should create valid boolean property"""
        prop = TypedProperty(type=PropertyType.BOOLEAN, value=True)

        assert prop.type == PropertyType.BOOLEAN
        assert prop.value is True

    def test_valid_link_property(self):
        """Should create valid link property"""
        prop = TypedProperty(type=PropertyType.LINK, value="[[Some Note]]")

        assert prop.type == PropertyType.LINK
        assert prop.value == "[[Some Note]]"

    def test_string_with_label(self):
        """Should accept label for string property"""
        prop = TypedProperty(
            type=PropertyType.STRING,
            value="test",
            label="Test Label"
        )

        assert prop.label == "Test Label"

    def test_string_rejects_non_string(self):
        """String property should reject non-string values"""
        with pytest.raises(ValidationError):
            TypedProperty(type=PropertyType.STRING, value=42)

    def test_number_rejects_non_number(self):
        """Number property should reject non-number values"""
        with pytest.raises(ValidationError):
            TypedProperty(type=PropertyType.NUMBER, value="not a number")

    def test_date_rejects_non_date(self):
        """Date property should reject non-datetime values"""
        with pytest.raises(ValidationError):
            TypedProperty(type=PropertyType.DATE, value="2024-01-01")

    def test_boolean_rejects_non_boolean(self):
        """Boolean property should reject non-boolean values"""
        with pytest.raises(ValidationError):
            TypedProperty(type=PropertyType.BOOLEAN, value="true")

    def test_link_rejects_non_string(self):
        """Link property should reject non-string values"""
        with pytest.raises(ValidationError):
            TypedProperty(type=PropertyType.LINK, value=123)

    def test_get_value(self):
        """get_value should return the property value"""
        prop = TypedProperty(type=PropertyType.STRING, value="test")

        assert prop.get_value() == "test"

    def test_set_value_valid(self):
        """set_value should update value with valid type"""
        prop = TypedProperty(type=PropertyType.STRING, value="old")
        prop.set_value("new")

        assert prop.value == "new"

    def test_set_value_invalid_type(self):
        """set_value should reject invalid type"""
        prop = TypedProperty(type=PropertyType.STRING, value="old")

        with pytest.raises(ValidationError):
            prop.set_value(42)

    def test_serialization(self):
        """TypedProperty should serialize correctly"""
        prop = TypedProperty(
            type=PropertyType.STRING,
            value="test",
            label="Label"
        )

        data = prop.model_dump()

        assert data["type"] == "string"
        assert data["value"] == "test"
        assert data["label"] == "Label"

    def test_deserialization(self):
        """TypedProperty should deserialize from dict"""
        data = {
            "type": "string",
            "value": "test",
            "label": "Label"
        }

        prop = TypedProperty(**data)

        assert prop.type == PropertyType.STRING
        assert prop.value == "test"
        assert prop.label == "Label"


# ============================================================================
# NoteFrontmatter with Properties Tests
# ============================================================================

class TestNoteFrontmatterProperties:
    """Tests for NoteFrontmatter with typed properties"""

    def test_defaults_properties_to_empty(self):
        """Properties should default to empty dict"""
        fm = NoteFrontmatter(title="Test")

        assert fm.properties == {}

    def test_accepts_properties(self):
        """Should accept properties dict"""
        prop = TypedProperty(type=PropertyType.STRING, value="test")
        fm = NoteFrontmatter(title="Test", properties={"priority": prop})

        assert "priority" in fm.properties
        assert fm.properties["priority"].value == "test"

    def test_accepts_multiple_properties(self):
        """Should accept multiple properties"""
        props = {
            "priority": TypedProperty(type=PropertyType.STRING, value="high"),
            "count": TypedProperty(type=PropertyType.NUMBER, value=5),
            "done": TypedProperty(type=PropertyType.BOOLEAN, value=False)
        }
        fm = NoteFrontmatter(title="Test", properties=props)

        assert len(fm.properties) == 3
        assert fm.properties["priority"].value == "high"
        assert fm.properties["count"].value == 5
        assert fm.properties["done"].value is False

    def test_get_property(self):
        """get_property should return property by name"""
        prop = TypedProperty(type=PropertyType.STRING, value="test")
        fm = NoteFrontmatter(title="Test", properties={"name": prop})

        result = fm.get_property("name")

        assert result is not None
        assert result.value == "test"

    def test_get_property_not_found(self):
        """get_property should return None for missing property"""
        fm = NoteFrontmatter(title="Test")

        result = fm.get_property("nonexistent")

        assert result is None

    def test_set_property(self):
        """set_property should add or update property"""
        fm = NoteFrontmatter(title="Test")
        prop = TypedProperty(type=PropertyType.STRING, value="test")

        fm.set_property("name", prop)

        assert "name" in fm.properties
        assert fm.properties["name"].value == "test"

    def test_set_property_update(self):
        """set_property should update existing property"""
        prop1 = TypedProperty(type=PropertyType.STRING, value="old")
        fm = NoteFrontmatter(title="Test", properties={"name": prop1})

        prop2 = TypedProperty(type=PropertyType.STRING, value="new")
        fm.set_property("name", prop2)

        assert fm.properties["name"].value == "new"

    def test_delete_property(self):
        """delete_property should remove property"""
        prop = TypedProperty(type=PropertyType.STRING, value="test")
        fm = NoteFrontmatter(title="Test", properties={"name": prop})

        result = fm.delete_property("name")

        assert result is True
        assert "name" not in fm.properties

    def test_delete_property_not_found(self):
        """delete_property should return False for missing property"""
        fm = NoteFrontmatter(title="Test")

        result = fm.delete_property("nonexistent")

        assert result is False

    def test_get_property_value(self):
        """get_property_value should return property value"""
        prop = TypedProperty(type=PropertyType.NUMBER, value=42)
        fm = NoteFrontmatter(title="Test", properties={"count": prop})

        result = fm.get_property_value("count")

        assert result == 42

    def test_get_property_value_not_found(self):
        """get_property_value should return None for missing property"""
        fm = NoteFrontmatter(title="Test")

        result = fm.get_property_value("nonexistent")

        assert result is None

    def test_serialization_with_properties(self):
        """NoteFrontmatter should serialize with properties"""
        prop = TypedProperty(type=PropertyType.STRING, value="test")
        fm = NoteFrontmatter(title="Test", properties={"name": prop})

        data = fm.model_dump()

        assert "properties" in data
        assert "name" in data["properties"]
        assert data["properties"]["name"]["value"] == "test"


# ============================================================================
# NoteCreate with Properties Tests
# ============================================================================

class TestNoteCreateProperties:
    """Tests for NoteCreate with typed properties"""

    def test_defaults_properties_to_empty(self):
        """Properties should default to empty dict"""
        create = NoteCreate(title="Test")

        assert create.properties == {}

    def test_accepts_properties(self):
        """Should accept properties"""
        prop = TypedProperty(type=PropertyType.STRING, value="test")
        create = NoteCreate(title="Test", properties={"name": prop})

        assert "name" in create.properties

    def test_serialization_with_properties(self):
        """NoteCreate should serialize with properties"""
        prop = TypedProperty(type=PropertyType.STRING, value="test")
        create = NoteCreate(title="Test", properties={"name": prop})

        data = create.model_dump()

        assert "properties" in data
        assert "name" in data["properties"]


# ============================================================================
# NoteUpdate with Properties Tests
# ============================================================================

class TestNoteUpdateProperties:
    """Tests for NoteUpdate with typed properties"""

    def test_properties_optional(self):
        """Properties should be optional"""
        update = NoteUpdate()

        assert update.properties is None

    def test_accepts_properties(self):
        """Should accept properties"""
        prop = TypedProperty(type=PropertyType.STRING, value="test")
        update = NoteUpdate(properties={"name": prop})

        assert "name" in update.properties

    def test_partial_update_with_properties(self):
        """Should allow partial updates with properties"""
        prop = TypedProperty(type=PropertyType.STRING, value="test")
        update = NoteUpdate(properties={"name": prop})

        assert update.properties is not None
        assert update.title is None
        assert update.content is None
