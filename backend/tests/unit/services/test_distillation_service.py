"""Comprehensive unit tests for distillation service utility functions and tag operations.

Tests cover module-level utility functions: normalize_tag, parse_inline_tags,
merge_tags, normalize_all_tags, update_protected_section, extract_protected_section,
atomic_write_file, and render_zettel_note.
"""
import os
from pathlib import Path
from unittest.mock import MagicMock

import pytest

from app.services.distillation import (
    CANVAS_END,
    CANVAS_START,
    atomic_write_file,
    extract_protected_section,
    merge_tags,
    normalize_all_tags,
    normalize_tag,
    parse_inline_tags,
    render_zettel_note,
    update_protected_section,
)
from app.models.distillation import (
    LinkType,
    ZettelLinkCandidate,
    ZettelNoteCandidate,
    ZettelType,
)
from app.models.note import Note, NoteFrontmatter


# ============================================================================
# normalize_tag
# ============================================================================


@pytest.mark.unit
class TestNormalizeTag:
    """Tests for normalize_tag utility function."""

    def test_strip_single_hash(self):
        """A single leading '#' should be stripped."""
        assert normalize_tag("#python") == "python"

    def test_strip_multiple_hashes(self):
        """Multiple leading '#' characters should all be stripped."""
        assert normalize_tag("##double") == "double"
        assert normalize_tag("###triple") == "triple"

    def test_lowercase_conversion(self):
        """Tags should be lowercased."""
        assert normalize_tag("Python") == "python"
        assert normalize_tag("TEXT-TO-3D") == "text-to-3d"
        assert normalize_tag("CamelCase") == "camelcase"

    def test_spaces_to_hyphens(self):
        """Spaces should be converted to hyphens."""
        assert normalize_tag("text to 3d") == "text-to-3d"
        assert normalize_tag("multi word tag") == "multi-word-tag"

    def test_combined_normalization(self):
        """All normalization rules should apply together."""
        assert normalize_tag("#Text To 3D") == "text-to-3d"
        assert normalize_tag("##My Tag Name") == "my-tag-name"

    def test_empty_string(self):
        """Empty string should return empty string."""
        assert normalize_tag("") == ""

    def test_hash_only(self):
        """A tag that is just '#' should return empty string."""
        assert normalize_tag("#") == ""

    def test_whitespace_stripping(self):
        """Leading/trailing whitespace should be stripped."""
        assert normalize_tag("  tag  ") == "tag"
        assert normalize_tag("# spaced ") == "spaced"

    def test_tag_with_hyphens_preserved(self):
        """Existing hyphens should be preserved."""
        assert normalize_tag("already-hyphenated") == "already-hyphenated"

    def test_tag_with_underscores(self):
        """Underscores should be preserved (only spaces become hyphens)."""
        assert normalize_tag("under_score") == "under_score"


# ============================================================================
# parse_inline_tags
# ============================================================================


@pytest.mark.unit
class TestParseInlineTags:
    """Tests for parse_inline_tags function."""

    def test_basic_tags(self):
        """Should find simple #tags in text."""
        content = "This is about #seedream and #canvas"
        tags = parse_inline_tags(content)
        assert "seedream" in tags
        assert "canvas" in tags

    def test_tags_with_hyphens(self):
        """Should capture tags containing hyphens."""
        content = "Using #text-to-3d models"
        tags = parse_inline_tags(content)
        assert "text-to-3d" in tags

    def test_tags_with_slashes(self):
        """Should capture namespace-style tags with slashes."""
        content = "Filed under #project/seedream"
        tags = parse_inline_tags(content)
        assert "project/seedream" in tags

    def test_tags_with_underscores(self):
        """Should capture tags with underscores."""
        content = "See #my_tag for details"
        tags = parse_inline_tags(content)
        assert "my_tag" in tags

    def test_ignore_headings(self):
        """Markdown headings (# Title) should not be parsed as tags."""
        content = "# Heading One\n## Heading Two\nSome text with #realtag"
        tags = parse_inline_tags(content)
        assert "realtag" in tags
        # Headings should not produce tags
        assert "heading" not in tags
        assert "Heading" not in tags

    def test_ignore_fenced_code_blocks(self):
        """Tags inside fenced code blocks (```) should be ignored."""
        content = """Some text #outside

```python
# This is a comment with #insidecode
x = "#alsocode"
```

More text #aftercode
"""
        tags = parse_inline_tags(content)
        assert "outside" in tags
        assert "aftercode" in tags
        assert "insidecode" not in tags
        assert "alsocode" not in tags

    def test_ignore_inline_code(self):
        """Tags inside inline code backticks should be ignored."""
        content = "Use `#notrealtag` syntax but #realtag works"
        tags = parse_inline_tags(content)
        assert "realtag" in tags
        assert "notrealtag" not in tags

    def test_deduplication(self):
        """Duplicate tags should appear only once."""
        content = "#seedream is great, love #seedream and also #Seedream"
        tags = parse_inline_tags(content)
        assert tags.count("seedream") == 1

    def test_empty_content(self):
        """Empty content should return an empty list."""
        assert parse_inline_tags("") == []

    def test_no_tags(self):
        """Content with no tags should return an empty list."""
        assert parse_inline_tags("Just some plain text here.") == []

    def test_tag_at_start_of_line(self):
        """A tag at the start of a line should be captured."""
        content = "#starttag is here"
        tags = parse_inline_tags(content)
        assert "starttag" in tags

    def test_tags_are_normalized(self):
        """Returned tags should be normalized (lowercase)."""
        content = "Check #UpperCase and #MixedCase"
        tags = parse_inline_tags(content)
        assert "uppercase" in tags
        assert "mixedcase" in tags


# ============================================================================
# merge_tags
# ============================================================================


@pytest.mark.unit
class TestMergeTags:
    """Tests for merge_tags function."""

    def test_merge_adds_new_tags(self):
        """New inline tags should be added to YAML tags."""
        result = merge_tags(["existing"], ["new"])
        assert "existing" in result
        assert "new" in result

    def test_merge_deduplicates(self):
        """Identical tags should appear only once."""
        result = merge_tags(["seedream"], ["seedream"])
        assert result.count("seedream") == 1

    def test_merge_normalizes_during_merge(self):
        """Tags should be normalized during the merge."""
        result = merge_tags(["#OldTag"], ["NewTag"])
        assert "oldtag" in result
        assert "newtag" in result

    def test_merge_preserves_yaml_when_inline_empty(self):
        """Deleting all inline tags should NOT remove YAML tags (additive-only)."""
        result = merge_tags(["important", "keep-this"], [])
        assert "important" in result
        assert "keep-this" in result

    def test_merge_returns_sorted(self):
        """Merged tags should be sorted alphabetically."""
        result = merge_tags(["zebra"], ["alpha"])
        assert result == ["alpha", "zebra"]

    def test_merge_both_empty(self):
        """Merging two empty lists should return an empty list."""
        assert merge_tags([], []) == []

    def test_merge_normalizes_duplicates_across_lists(self):
        """Tags that normalize to the same value should be deduplicated."""
        result = merge_tags(["#Python"], ["python"])
        assert result == ["python"]


# ============================================================================
# normalize_all_tags
# ============================================================================


@pytest.mark.unit
class TestNormalizeAllTags:
    """Tests for normalize_all_tags function."""

    def test_normalize_and_deduplicate(self):
        """Should normalize all tags and remove duplicates."""
        tags = ["#Seedream", "seedream", "Canvas Export"]
        result = normalize_all_tags(tags)
        assert len(result) == 2
        assert "seedream" in result
        assert "canvas-export" in result

    def test_sorted_output(self):
        """Should return tags in sorted order."""
        tags = ["zebra", "alpha", "middle"]
        result = normalize_all_tags(tags)
        assert result == ["alpha", "middle", "zebra"]

    def test_empty_list(self):
        """Empty input list should return empty list."""
        assert normalize_all_tags([]) == []

    def test_single_tag(self):
        """A single tag should be returned normalized."""
        assert normalize_all_tags(["#MyTag"]) == ["mytag"]


# ============================================================================
# update_protected_section
# ============================================================================


@pytest.mark.unit
class TestUpdateProtectedSection:
    """Tests for update_protected_section function."""

    def test_replace_existing_section(self):
        """Should replace content between existing markers."""
        existing = (
            f"# Notes\n\nUser content.\n\n"
            f"{CANVAS_START}\nOld snapshot\n{CANVAS_END}\n\nMore content."
        )
        result = update_protected_section(existing, "New snapshot")
        assert "New snapshot" in result
        assert "Old snapshot" not in result
        assert "User content." in result
        assert "More content." in result

    def test_append_when_markers_missing(self):
        """Should safely append a new section when no markers exist."""
        existing = "# Notes\n\nManual content."
        result = update_protected_section(existing, "Canvas data")
        assert "Manual content." in result
        assert CANVAS_START in result
        assert CANVAS_END in result
        assert "Canvas data" in result
        assert "## Canvas Snapshot (auto)" in result

    def test_preserves_content_outside_markers(self):
        """User content outside the markers should remain intact."""
        user_notes = "IMPORTANT ANALYSIS"
        existing = (
            f"# Export\n\n{user_notes}\n\n"
            f"{CANVAS_START}\nOld\n{CANVAS_END}\n\n## My Thoughts\nThinking..."
        )
        result = update_protected_section(existing, "Updated")
        assert user_notes in result
        assert "My Thoughts" in result
        assert "Thinking..." in result
        assert "Updated" in result
        assert "Old" not in result

    def test_multiline_snapshot_replacement(self):
        """Should handle multiline content between markers."""
        existing = (
            f"{CANVAS_START}\nLine 1\nLine 2\nLine 3\n{CANVAS_END}"
        )
        result = update_protected_section(existing, "Single line")
        assert "Single line" in result
        assert "Line 1" not in result


# ============================================================================
# extract_protected_section
# ============================================================================


@pytest.mark.unit
class TestExtractProtectedSection:
    """Tests for extract_protected_section function."""

    def test_extract_content_between_markers(self):
        """Should extract and strip content between markers."""
        content = (
            f"Preamble\n\n"
            f"{CANVAS_START}\nProtected content here\n{CANVAS_END}\n\nPostamble"
        )
        extracted = extract_protected_section(content)
        assert extracted == "Protected content here"

    def test_returns_none_when_no_markers(self):
        """Should return None when no markers are present."""
        assert extract_protected_section("Regular content") is None

    def test_extract_multiline_content(self):
        """Should extract multiline protected content."""
        content = (
            f"{CANVAS_START}\n"
            f"Line A\nLine B\nLine C\n"
            f"{CANVAS_END}"
        )
        extracted = extract_protected_section(content)
        assert "Line A" in extracted
        assert "Line B" in extracted
        assert "Line C" in extracted

    def test_extract_empty_protected_section(self):
        """An empty protected section should return empty string."""
        content = f"{CANVAS_START}\n{CANVAS_END}"
        extracted = extract_protected_section(content)
        assert extracted == ""


# ============================================================================
# atomic_write_file
# ============================================================================


@pytest.mark.unit
class TestAtomicWriteFile:
    """Tests for atomic_write_file function."""

    def test_writes_content_to_file(self, tmp_path):
        """Should write content and produce a readable file."""
        file_path = tmp_path / "test.md"
        atomic_write_file(file_path, "Hello, world!")
        assert file_path.read_text(encoding="utf-8") == "Hello, world!"

    def test_overwrites_existing_file(self, tmp_path):
        """Should overwrite an existing file."""
        file_path = tmp_path / "existing.md"
        file_path.write_text("old content", encoding="utf-8")
        atomic_write_file(file_path, "new content")
        assert file_path.read_text(encoding="utf-8") == "new content"

    def test_creates_file_in_existing_directory(self, tmp_path):
        """Should create a new file when the parent directory already exists."""
        subdir = tmp_path / "subdir"
        subdir.mkdir()
        file_path = subdir / "note.md"
        atomic_write_file(file_path, "content in subdir")
        assert file_path.read_text(encoding="utf-8") == "content in subdir"

    def test_writes_unicode_content(self, tmp_path):
        """Should handle unicode content correctly."""
        file_path = tmp_path / "unicode.md"
        content = "Zettelkasten notes with umlauts: ae, oe, ue and emoji"
        atomic_write_file(file_path, content)
        assert file_path.read_text(encoding="utf-8") == content

    def test_accepts_path_as_string(self, tmp_path):
        """Should accept both Path objects and string paths."""
        file_path = str(tmp_path / "string_path.md")
        atomic_write_file(file_path, "string path content")
        assert Path(file_path).read_text(encoding="utf-8") == "string path content"


# ============================================================================
# render_zettel_note
# ============================================================================


@pytest.mark.unit
class TestRenderZettelNote:
    """Tests for render_zettel_note function."""

    @pytest.fixture
    def container_note(self):
        """Create a minimal container Note for testing."""
        return Note(
            id="container-001",
            title="Source Conversation",
            content="Full conversation content here.",
            frontmatter=NoteFrontmatter(
                title="Source Conversation",
                tags=["chat", "evidence"],
                status="evidence",
            ),
        )

    def _make_candidate(self, zettel_type, **overrides):
        """Helper to build a ZettelNoteCandidate with defaults."""
        defaults = dict(
            id="cand-001",
            title="Test Candidate Title",
            zettel_type=zettel_type,
            content="Some content",
            summary=["Point A", "Point B"],
            key_claims=["Claim one"],
            open_questions=["Question one"],
            recommended_tags=["ai", "testing"],
            confidence=0.8,
            suggested_links=[],
        )
        defaults.update(overrides)
        return ZettelNoteCandidate(**defaults)

    def test_concept_note_structure(self, container_note):
        """Concept notes should include Definition, Related Concepts, Sources sections."""
        candidate = self._make_candidate(ZettelType.CONCEPT)
        result = render_zettel_note(candidate, container_note)
        assert f"# {candidate.title}" in result
        assert "## Definition" in result
        assert "## Related Concepts" in result
        assert "## Sources" in result
        assert f"[[{container_note.title}]]" in result
        assert "Point A" in result
        assert "Point B" in result

    def test_concept_note_includes_questions(self, container_note):
        """Concept notes should render open questions."""
        candidate = self._make_candidate(
            ZettelType.CONCEPT,
            open_questions=["Why does this matter?"],
        )
        result = render_zettel_note(candidate, container_note)
        assert "Why does this matter?" in result

    def test_concept_note_includes_suggested_links(self, container_note):
        """Concept notes should render suggested links as wikilinks."""
        links = [
            ZettelLinkCandidate(
                target_title="Related Concept",
                link_type=LinkType.RELATED,
                reason="They overlap",
            )
        ]
        candidate = self._make_candidate(
            ZettelType.CONCEPT, suggested_links=links,
        )
        result = render_zettel_note(candidate, container_note)
        assert "[[Related Concept]]" in result
        assert "(related)" in result

    def test_question_note_structure(self, container_note):
        """Question notes should include Question, Context, Approaches sections."""
        candidate = self._make_candidate(ZettelType.QUESTION)
        result = render_zettel_note(candidate, container_note)
        assert "## Question" in result
        assert "## Context" in result
        assert "## Approaches" in result
        assert f"[[{container_note.title}]]" in result

    def test_evidence_note_structure(self, container_note):
        """Evidence notes should include Evidence, Source, Supports sections."""
        candidate = self._make_candidate(ZettelType.EVIDENCE)
        result = render_zettel_note(candidate, container_note)
        assert "## Evidence" in result
        assert "## Source" in result
        assert "## Supports" in result
        assert f"[[{container_note.title}]]" in result

    def test_fleche_note_structure(self, container_note):
        """Fleche (structure) notes should include Argument Structure section."""
        candidate = self._make_candidate(ZettelType.FLECHE)
        result = render_zettel_note(candidate, container_note)
        assert "## Argument Structure" in result
        assert "## Connected Concepts" in result
        assert f"[[{container_note.title}]]" in result

    def test_fleeting_falls_back_to_generic(self, container_note):
        """Fleeting type should use the generic renderer with TL;DR section."""
        candidate = self._make_candidate(ZettelType.FLEETING)
        result = render_zettel_note(candidate, container_note)
        assert "## TL;DR" in result
        assert "## Details" in result
        assert f"[[{container_note.title}]]" in result

    def test_zettel_id_included(self, container_note):
        """Rendered notes should include a Zettel ID timestamp."""
        candidate = self._make_candidate(ZettelType.CONCEPT)
        result = render_zettel_note(candidate, container_note)
        assert "*Zettel ID:" in result

    def test_type_label_included(self, container_note):
        """Rendered notes should include the Zettel type label."""
        candidate = self._make_candidate(ZettelType.EVIDENCE)
        result = render_zettel_note(candidate, container_note)
        assert "*Type: evidence*" in result
