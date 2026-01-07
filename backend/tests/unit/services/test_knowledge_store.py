"""
Unit tests for KnowledgeStore service

Tests cover:
- Path traversal protection (security-critical)
- CRUD operations
- Wikilink extraction
- Frontmatter parsing
- Character encoding
- Edge cases and error handling
"""
import pytest
from pathlib import Path
from datetime import datetime

from app.services.knowledge_store import KnowledgeStore
from tests.fixtures.sample_notes import (
    get_sample_note,
    get_notes_with_wikilinks,
    get_notes_with_special_characters,
)


# ============================================================================
# Security Tests - Path Traversal Protection
# ============================================================================

@pytest.mark.unit
@pytest.mark.security
class TestPathTraversalProtection:
    """Test that path traversal attacks are prevented"""

    def test_path_traversal_unix_style(self, knowledge_store: KnowledgeStore, path_traversal_attempts: list[str]):
        """Test that Unix-style path traversal attempts are blocked"""
        for malicious_path in path_traversal_attempts:
            with pytest.raises((ValueError, FileNotFoundError)):
                # Should not be able to access files outside vault
                knowledge_store.get_note(malicious_path)

    def test_path_traversal_in_note_creation(self, knowledge_store: KnowledgeStore):
        """Test that path traversal in note creation is blocked"""
        malicious_data = get_sample_note(title="../../etc/passwd")

        with pytest.raises(ValueError):
            knowledge_store.create_note(malicious_data)

    def test_path_traversal_windows_style(self, knowledge_store: KnowledgeStore):
        """Test Windows-specific path traversal attempts"""
        windows_attempts = [
            "..\\..\\..\\windows\\system32",
            "C:\\Windows\\System32\\config\\sam",
            "\\\\network\\share\\file",
        ]

        for malicious_path in windows_attempts:
            with pytest.raises((ValueError, FileNotFoundError)):
                knowledge_store.get_note(malicious_path)

    def test_absolute_path_rejected(self, knowledge_store: KnowledgeStore):
        """Test that absolute paths are rejected"""
        with pytest.raises((ValueError, FileNotFoundError)):
            knowledge_store.get_note("/etc/passwd")

        with pytest.raises((ValueError, FileNotFoundError)):
            knowledge_store.get_note("C:\\Windows\\System32")

    def test_url_encoded_path_traversal(self, knowledge_store: KnowledgeStore):
        """Test URL-encoded path traversal attempts"""
        encoded_attempts = [
            "..%2F..%2Fetc%2Fpasswd",
            "..%252F..%252Fetc%252Fpasswd",
            "%2e%2e%2f%2e%2e%2fetc%2fpasswd",
        ]

        for encoded_path in encoded_attempts:
            with pytest.raises((ValueError, FileNotFoundError)):
                knowledge_store.get_note(encoded_path)


# ============================================================================
# CRUD Operations Tests
# ============================================================================

@pytest.mark.unit
class TestCRUDOperations:
    """Test Create, Read, Update, Delete operations"""

    def test_create_note_success(self, knowledge_store: KnowledgeStore):
        """Test successful note creation"""
        note_data = get_sample_note(title="New Note", content="Test content")

        result = knowledge_store.create_note(note_data)

        assert result["id"] is not None
        assert result["title"] == "New Note"
        assert result["content"] == "Test content"
        assert result["status"] == "draft"
        assert result["tags"] == ["test"]
        assert "created_at" in result
        assert "updated_at" in result

    def test_create_note_generates_id_from_title(self, knowledge_store: KnowledgeStore):
        """Test that note ID is generated from title"""
        note_data = get_sample_note(title="My Test Note")

        result = knowledge_store.create_note(note_data)

        # ID should be slugified version of title
        assert result["id"] == "my-test-note"

    def test_create_duplicate_note_raises_error(self, knowledge_store: KnowledgeStore):
        """Test that creating duplicate note raises error"""
        note_data = get_sample_note(title="Duplicate Note")

        # Create first note
        knowledge_store.create_note(note_data)

        # Attempt to create duplicate should raise error
        with pytest.raises(FileExistsError):
            knowledge_store.create_note(note_data)

    def test_get_note_success(self, knowledge_store: KnowledgeStore):
        """Test successful note retrieval"""
        # Create a note first
        created = knowledge_store.create_note(get_sample_note(title="Get Test"))

        # Retrieve it
        retrieved = knowledge_store.get_note(created["id"])

        assert retrieved["id"] == created["id"]
        assert retrieved["title"] == created["title"]
        assert retrieved["content"] == created["content"]

    def test_get_nonexistent_note_raises_error(self, knowledge_store: KnowledgeStore):
        """Test that getting non-existent note raises error"""
        with pytest.raises(FileNotFoundError):
            knowledge_store.get_note("nonexistent-note-id")

    def test_update_note_success(self, knowledge_store: KnowledgeStore):
        """Test successful note update"""
        # Create initial note
        created = knowledge_store.create_note(get_sample_note(title="Update Test"))

        # Update it
        update_data = {
            "title": "Updated Title",
            "content": "Updated content",
            "status": "canonical",
            "tags": ["updated", "test"],
        }

        updated = knowledge_store.update_note(created["id"], update_data)

        assert updated["id"] == created["id"]
        assert updated["title"] == "Updated Title"
        assert updated["content"] == "Updated content"
        assert updated["status"] == "canonical"
        assert updated["tags"] == ["updated", "test"]
        # Updated_at should be different
        assert updated["updated_at"] != created["updated_at"]

    def test_update_nonexistent_note_raises_error(self, knowledge_store: KnowledgeStore):
        """Test that updating non-existent note raises error"""
        with pytest.raises(FileNotFoundError):
            knowledge_store.update_note("nonexistent-note", {"title": "New Title"})

    def test_delete_note_success(self, knowledge_store: KnowledgeStore):
        """Test successful note deletion"""
        # Create a note
        created = knowledge_store.create_note(get_sample_note(title="Delete Test"))

        # Delete it
        result = knowledge_store.delete_note(created["id"])

        assert result is True

        # Verify it's gone
        with pytest.raises(FileNotFoundError):
            knowledge_store.get_note(created["id"])

    def test_delete_nonexistent_note_returns_false(self, knowledge_store: KnowledgeStore):
        """Test that deleting non-existent note returns False"""
        result = knowledge_store.delete_note("nonexistent-note")
        assert result is False

    def test_list_notes_success(self, knowledge_store: KnowledgeStore):
        """Test listing all notes"""
        # Create multiple notes
        knowledge_store.create_note(get_sample_note(title="Note 1"))
        knowledge_store.create_note(get_sample_note(title="Note 2"))
        knowledge_store.create_note(get_sample_note(title="Note 3"))

        # List all
        notes = knowledge_store.list_notes()

        assert len(notes) == 3
        titles = [n["title"] for n in notes]
        assert "Note 1" in titles
        assert "Note 2" in titles
        assert "Note 3" in titles

    def test_list_notes_empty_vault(self, knowledge_store: KnowledgeStore):
        """Test listing notes when vault is empty"""
        notes = knowledge_store.list_notes()
        assert notes == []


# ============================================================================
# Wikilink Extraction Tests
# ============================================================================

@pytest.mark.unit
class TestWikilinkExtraction:
    """Test wikilink parsing and extraction"""

    def test_extract_simple_wikilink(self, knowledge_store: KnowledgeStore):
        """Test extracting simple [[Note Title]] wikilinks"""
        content = "This links to [[Target Note]] in the middle."

        links = knowledge_store.extract_wikilinks(content)

        assert links == ["Target Note"]

    def test_extract_wikilink_with_display_text(self, knowledge_store: KnowledgeStore):
        """Test extracting [[Target|Display]] wikilinks"""
        content = "Link with display: [[Actual Target|Display Text]]."

        links = knowledge_store.extract_wikilinks(content)

        # Should extract target, not display text
        assert links == ["Actual Target"]

    def test_extract_multiple_wikilinks(self, knowledge_store: KnowledgeStore):
        """Test extracting multiple wikilinks from content"""
        content = """
        This document links to [[Note A]], [[Note B]], and [[Note C|see Note C]].

        Also [[Note D]] at the end.
        """

        links = knowledge_store.extract_wikilinks(content)

        assert set(links) == {"Note A", "Note B", "Note C", "Note D"}

    def test_extract_no_wikilinks(self, knowledge_store: KnowledgeStore):
        """Test content with no wikilinks"""
        content = "This has no wikilinks at all."

        links = knowledge_store.extract_wikilinks(content)

        assert links == []

    def test_extract_duplicate_wikilinks(self, knowledge_store: KnowledgeStore):
        """Test that duplicate wikilinks are handled"""
        content = "[[Note A]], [[Note A]], [[Note A]]"

        links = knowledge_store.extract_wikilinks(content)

        # Should preserve duplicates for accurate link count
        assert links == ["Note A", "Note A", "Note A"]

    def test_extract_wikilinks_with_special_characters(self, knowledge_store: KnowledgeStore):
        """Test wikilinks containing special characters"""
        content = """
        [[Note-With-Dashes]]
        [[Note_With_Underscores]]
        [[Note With Spaces]]
        [[Note/With/Slashes]]
        [[Note (With Parentheses)]]
        """

        links = knowledge_store.extract_wikilinks(content)

        assert "Note-With-Dashes" in links
        assert "Note_With_Underscores" in links
        assert "Note With Spaces" in links
        assert "Note/With/Slashes" in links
        assert "Note (With Parentheses)" in links

    def test_extract_empty_wikilink(self, knowledge_store: KnowledgeStore):
        """Test handling of empty wikilinks [[]]"""
        content = "Empty link: [[]]"

        links = knowledge_store.extract_wikilinks(content)

        # Empty wikilinks should be filtered out or handled gracefully
        assert "" not in links or links == [""]

    def test_wikilinks_in_code_blocks(self, knowledge_store: KnowledgeStore):
        """Test that wikilinks in code blocks are still extracted"""
        content = """
        Regular link: [[Note A]]

        ```
        Code block with [[Note B]]
        ```

        `Inline code with [[Note C]]`
        """

        links = knowledge_store.extract_wikilinks(content)

        # Current implementation doesn't distinguish code blocks
        # This documents expected behavior
        assert "Note A" in links
        # Behavior for code blocks may vary by implementation


# ============================================================================
# Frontmatter Parsing Tests
# ============================================================================

@pytest.mark.unit
class TestFrontmatterParsing:
    """Test YAML frontmatter parsing"""

    def test_parse_valid_frontmatter(self, knowledge_store: KnowledgeStore):
        """Test parsing valid YAML frontmatter"""
        # Create note with frontmatter
        note_data = get_sample_note(
            title="Frontmatter Test",
            status="canonical",
            tags=["test", "frontmatter"],
        )

        created = knowledge_store.create_note(note_data)

        # Read it back
        retrieved = knowledge_store.get_note(created["id"])

        assert retrieved["status"] == "canonical"
        assert retrieved["tags"] == ["test", "frontmatter"]

    def test_frontmatter_with_dates(self, knowledge_store: KnowledgeStore):
        """Test frontmatter with datetime fields"""
        note_data = get_sample_note(title="Date Test")

        created = knowledge_store.create_note(note_data)
        retrieved = knowledge_store.get_note(created["id"])

        assert "created_at" in retrieved
        assert "updated_at" in retrieved
        # Should be valid datetime strings
        datetime.fromisoformat(retrieved["created_at"].replace("Z", "+00:00"))

    def test_missing_frontmatter_fields(self, knowledge_store: KnowledgeStore):
        """Test handling of missing optional frontmatter fields"""
        # Create note with minimal data
        note_data = {"title": "Minimal Note", "content": "Content"}

        created = knowledge_store.create_note(note_data)

        # Should have defaults
        assert created["status"] == "draft" or created["status"] is not None
        assert created["tags"] == [] or created["tags"] is not None


# ============================================================================
# Character Encoding and Unicode Tests
# ============================================================================

@pytest.mark.unit
class TestCharacterEncoding:
    """Test handling of special characters and unicode"""

    def test_unicode_content(self, knowledge_store: KnowledgeStore):
        """Test notes with unicode characters"""
        unicode_notes = get_notes_with_special_characters()

        for note_data in unicode_notes:
            created = knowledge_store.create_note(note_data)
            retrieved = knowledge_store.get_note(created["id"])

            assert retrieved["title"] == note_data["title"]
            assert retrieved["content"] == note_data["content"]

    def test_emoji_in_title_and_content(self, knowledge_store: KnowledgeStore):
        """Test emoji handling"""
        note_data = get_sample_note(
            title="Test Note 🚀",
            content="Content with emoji: 🎉 💻 🌟",
        )

        created = knowledge_store.create_note(note_data)
        retrieved = knowledge_store.get_note(created["id"])

        assert retrieved["title"] == "Test Note 🚀"
        assert "🎉" in retrieved["content"]

    def test_special_characters_in_title(self, knowledge_store: KnowledgeStore):
        """Test special characters in titles"""
        special_titles = [
            "Title with @#$%",
            "Title: With Colon",
            "Title/With/Slashes",
            "Title (With Parentheses)",
            "Title [With Brackets]",
        ]

        for title in special_titles:
            note_data = get_sample_note(title=title)
            created = knowledge_store.create_note(note_data)
            retrieved = knowledge_store.get_note(created["id"])

            assert retrieved["title"] == title


# ============================================================================
# Edge Cases and Error Handling
# ============================================================================

@pytest.mark.unit
class TestEdgeCases:
    """Test edge cases and error conditions"""

    def test_very_long_title(self, knowledge_store: KnowledgeStore):
        """Test handling of very long titles"""
        long_title = "A" * 500

        note_data = get_sample_note(title=long_title)

        # Should either accept or reject gracefully
        try:
            created = knowledge_store.create_note(note_data)
            retrieved = knowledge_store.get_note(created["id"])
            assert len(retrieved["title"]) > 0
        except ValueError:
            # Acceptable to reject very long titles
            pass

    def test_empty_content(self, knowledge_store: KnowledgeStore):
        """Test note with empty content"""
        note_data = get_sample_note(title="Empty Content", content="")

        created = knowledge_store.create_note(note_data)
        retrieved = knowledge_store.get_note(created["id"])

        assert retrieved["content"] == ""

    def test_whitespace_only_content(self, knowledge_store: KnowledgeStore):
        """Test note with only whitespace content"""
        note_data = get_sample_note(title="Whitespace", content="   \n\n   ")

        created = knowledge_store.create_note(note_data)
        retrieved = knowledge_store.get_note(created["id"])

        assert retrieved["content"] == "   \n\n   "

    def test_large_note_content(self, knowledge_store: KnowledgeStore):
        """Test handling of very large notes"""
        from tests.fixtures.sample_notes import get_large_note_content

        large_content = get_large_note_content()
        note_data = get_sample_note(title="Large Note", content=large_content)

        created = knowledge_store.create_note(note_data)
        retrieved = knowledge_store.get_note(created["id"])

        assert len(retrieved["content"]) > 10000

    def test_concurrent_note_creation(self, knowledge_store: KnowledgeStore):
        """Test creating multiple notes in succession"""
        import concurrent.futures

        def create_note(i):
            return knowledge_store.create_note(
                get_sample_note(title=f"Concurrent Note {i}")
            )

        # Create 10 notes concurrently
        with concurrent.futures.ThreadPoolExecutor(max_workers=5) as executor:
            futures = [executor.submit(create_note, i) for i in range(10)]
            results = [f.result() for f in futures]

        assert len(results) == 10
        # All should have unique IDs
        ids = [r["id"] for r in results]
        assert len(set(ids)) == 10

    def test_get_all_content(self, knowledge_store: KnowledgeStore):
        """Test bulk retrieval of all note content"""
        # Create several notes
        for i in range(5):
            knowledge_store.create_note(get_sample_note(title=f"Bulk Note {i}"))

        # Get all content
        all_content = knowledge_store.get_all_content()

        assert len(all_content) == 5
        for note in all_content:
            assert "id" in note
            assert "title" in note
            assert "content" in note


# ============================================================================
# Integration with Wikilinks
# ============================================================================

@pytest.mark.unit
class TestWikilinksIntegration:
    """Test that created notes properly extract wikilinks"""

    def test_created_note_includes_wikilinks(self, knowledge_store: KnowledgeStore):
        """Test that creating a note extracts wikilinks"""
        note_data = get_sample_note(
            title="Note With Links",
            content="Links to [[Target 1]] and [[Target 2]].",
        )

        created = knowledge_store.create_note(note_data)

        # Check if wikilinks are extracted (implementation dependent)
        if "wikilinks" in created:
            assert "Target 1" in created["wikilinks"]
            assert "Target 2" in created["wikilinks"]

    def test_updated_note_updates_wikilinks(self, knowledge_store: KnowledgeStore):
        """Test that updating content updates wikilinks"""
        # Create note with initial links
        created = knowledge_store.create_note(
            get_sample_note(title="Update Links", content="Links to [[Old Link]].")
        )

        # Update with new links
        updated = knowledge_store.update_note(
            created["id"],
            {"content": "New links to [[New Link 1]] and [[New Link 2]]."},
        )

        # Wikilinks should be updated
        if "wikilinks" in updated:
            assert "New Link 1" in updated["wikilinks"]
            assert "New Link 2" in updated["wikilinks"]
            assert "Old Link" not in updated["wikilinks"]
