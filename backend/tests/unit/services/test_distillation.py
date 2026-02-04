"""Unit tests for distillation service"""
import pytest
from backend.app.services.distillation import (
    normalize_tag,
    parse_inline_tags,
    merge_tags,
    normalize_all_tags,
    update_protected_section,
    extract_protected_section,
    CANVAS_START,
    CANVAS_END,
)


class TestTagNormalization:
    """Tests for tag normalization utilities"""
    
    def test_normalize_tag_lowercase(self):
        """Tags should be lowercased"""
        assert normalize_tag("Grafyn") == "grafyn"
        assert normalize_tag("TEXT-TO-3D") == "text-to-3d"
    
    def test_normalize_tag_strip_hash(self):
        """Leading # should be stripped"""
        assert normalize_tag("#topic") == "topic"
        assert normalize_tag("##double") == "double"
    
    def test_normalize_tag_spaces_to_hyphens(self):
        """Spaces should become hyphens"""
        assert normalize_tag("text to 3d") == "text-to-3d"
        assert normalize_tag("multi word tag") == "multi-word-tag"
    
    def test_normalize_tag_combined(self):
        """All normalizations should apply together"""
        assert normalize_tag("#Text To 3D") == "text-to-3d"


class TestInlineTagParsing:
    """Tests for inline #tag parsing"""
    
    def test_parse_basic_tags(self):
        """Should find basic #tags"""
        content = "This is about #grafyn and #canvas"
        tags = parse_inline_tags(content)
        assert "grafyn" in tags
        assert "canvas" in tags
    
    def test_parse_tags_with_hyphens(self):
        """Should capture tags with hyphens"""
        content = "Using #text-to-3d models"
        tags = parse_inline_tags(content)
        assert "text-to-3d" in tags
    
    def test_parse_tags_with_slashes(self):
        """Should capture namespace tags"""
        content = "This is #project/grafyn"
        tags = parse_inline_tags(content)
        assert "project/grafyn" in tags
    
    def test_ignores_headings(self):
        """# Title should NOT be parsed as a tag"""
        content = """# This is a heading
        
## Another heading

Some text with #realtag here.
"""
        tags = parse_inline_tags(content)
        assert "realtag" in tags
        # Headings should not be captured
        assert "This" not in tags
        assert "this" not in tags
        assert "Another" not in tags
    
    def test_ignores_code_blocks(self):
        """Tags inside fenced code blocks should be ignored"""
        content = """Some text #outside

```python
# This is a comment #insidecode
print("#alsocode")
```

More text #aftercode
"""
        tags = parse_inline_tags(content)
        assert "outside" in tags
        assert "aftercode" in tags
        assert "insidecode" not in tags
        assert "alsocode" not in tags
    
    def test_ignores_inline_code(self):
        """Tags inside inline code should be ignored"""
        content = "Use the `#notrealtag` syntax but #realtag works"
        tags = parse_inline_tags(content)
        assert "realtag" in tags
        assert "notrealtag" not in tags
    
    def test_deduplicates_tags(self):
        """Same tag appearing multiple times should only appear once"""
        content = "#grafyn is great, love #grafyn"
        tags = parse_inline_tags(content)
        assert tags.count("grafyn") == 1


class TestTagMerging:
    """Tests for tag merging logic"""
    
    def test_merge_adds_new_tags(self):
        """New inline tags should be added to YAML tags"""
        yaml_tags = ["existing"]
        inline_tags = ["new"]
        merged = merge_tags(yaml_tags, inline_tags)
        assert "existing" in merged
        assert "new" in merged
    
    def test_merge_deduplicates(self):
        """Same tag in both should only appear once"""
        yaml_tags = ["grafyn"]
        inline_tags = ["grafyn"]
        merged = merge_tags(yaml_tags, inline_tags)
        assert merged.count("grafyn") == 1
    
    def test_merge_normalizes(self):
        """Tags should be normalized during merge"""
        yaml_tags = ["#OldTag"]
        inline_tags = ["NewTag"]
        merged = merge_tags(yaml_tags, inline_tags)
        assert "oldtag" in merged
        assert "newtag" in merged
    
    def test_merge_preserves_yaml_tags(self):
        """
        CRITICAL: Deleting inline tag should NOT remove YAML tag.
        Merge is additive-only.
        """
        yaml_tags = ["important", "keep-this"]
        inline_tags = []  # User deleted all inline tags
        merged = merge_tags(yaml_tags, inline_tags)
        assert "important" in merged
        assert "keep-this" in merged


class TestProtectedSections:
    """Tests for canvas export protected section handling"""
    
    def test_replace_existing_section(self):
        """Should replace content between markers"""
        existing = f"""# My Notes

Some user content here.

{CANVAS_START}
Old snapshot content
{CANVAS_END}

More user content.
"""
        new_snapshot = "New snapshot content"
        result = update_protected_section(existing, new_snapshot)
        
        assert "New snapshot content" in result
        assert "Old snapshot content" not in result
        assert "Some user content here." in result
        assert "More user content." in result
    
    def test_append_if_markers_missing(self):
        """Should append section if no markers exist (safe migration)"""
        existing = """# My Notes

This is my manually written content.
I don't want to lose this!
"""
        new_snapshot = "Auto-generated canvas content"
        result = update_protected_section(existing, new_snapshot)
        
        # Original content should be preserved
        assert "This is my manually written content." in result
        assert "I don't want to lose this!" in result
        
        # New section should be appended
        assert CANVAS_START in result
        assert CANVAS_END in result
        assert "Auto-generated canvas content" in result
        assert "## Canvas Snapshot (auto)" in result
    
    def test_preserves_user_text_outside_markers(self):
        """User edits outside markers should remain intact"""
        user_notes = "MY IMPORTANT NOTES - DO NOT DELETE"
        existing = f"""# Canvas Export

{user_notes}

{CANVAS_START}
Old content
{CANVAS_END}

## My Analysis
This is my own analysis of the canvas.
"""
        result = update_protected_section(existing, "Updated content")
        
        assert user_notes in result
        assert "My Analysis" in result
        assert "This is my own analysis" in result
        assert "Updated content" in result
        assert "Old content" not in result
    
    def test_extract_protected_section(self):
        """Should extract content between markers"""
        content = f"""Some text

{CANVAS_START}
Protected content here
{CANVAS_END}

More text
"""
        extracted = extract_protected_section(content)
        assert extracted == "Protected content here"
    
    def test_extract_returns_none_if_no_markers(self):
        """Should return None if no markers exist"""
        content = "Just regular content"
        assert extract_protected_section(content) is None


class TestNormalizeAllTags:
    """Tests for normalize_all_tags utility"""
    
    def test_normalizes_and_dedupes(self):
        """Should normalize and remove duplicates"""
        tags = ["#Grafyn", "grafyn", "Canvas Export"]
        result = normalize_all_tags(tags)
        assert len(result) == 2
        assert "grafyn" in result
        assert "canvas-export" in result
    
    def test_sorts_alphabetically(self):
        """Should return sorted tags"""
        tags = ["zebra", "alpha", "middle"]
        result = normalize_all_tags(tags)
        assert result == ["alpha", "middle", "zebra"]
