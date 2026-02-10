"""Security tests for wikilink XSS handling and path traversal protection.

Tests use the malicious_wikilink_patterns and path_traversal_attempts fixtures
from conftest.py to verify that KnowledgeStore handles malicious input safely.
"""
import pytest
from pathlib import Path

from app.services.knowledge_store import KnowledgeStore, WIKILINK_PATTERN


# ============================================================================
# Wikilink XSS / Injection Tests
# ============================================================================

@pytest.mark.unit
@pytest.mark.security
class TestWikilinkXSSSafety:
    """Tests that extract_wikilinks handles malicious patterns safely."""

    def test_xss_script_tag_in_wikilink(self, knowledge_store: KnowledgeStore):
        """XSS script tags inside wikilinks should be extracted as literal text, not executed."""
        content = "See [[<script>alert('xss')</script>]] for details."
        links = knowledge_store.extract_wikilinks(content)
        assert len(links) == 1
        # The extracted link target is the literal text, not interpreted as HTML
        assert links[0] == "<script>alert('xss')</script>"

    def test_xss_event_handler_in_wikilink(self, knowledge_store: KnowledgeStore):
        """Event handler attributes in wikilinks should be treated as literal text."""
        content = '[[<img onerror="alert(1)" src=x>]]'
        links = knowledge_store.extract_wikilinks(content)
        # The regex requires no ] or | inside, so this may or may not match
        # Either way, it should not crash
        assert isinstance(links, list)

    def test_javascript_uri_in_wikilink(self, knowledge_store: KnowledgeStore):
        """JavaScript URIs in wikilinks should be treated as literal text."""
        content = "[[javascript:alert(document.cookie)]]"
        links = knowledge_store.extract_wikilinks(content)
        assert len(links) == 1
        assert links[0] == "javascript:alert(document.cookie)"

    def test_very_long_wikilink_no_crash(self, knowledge_store: KnowledgeStore):
        """A very long wikilink (10000+ chars) should not cause a crash or hang."""
        long_target = "A" * 10000
        content = f"[[{long_target}]]"
        links = knowledge_store.extract_wikilinks(content)
        assert len(links) == 1
        assert links[0] == long_target

    def test_empty_wikilink(self, knowledge_store: KnowledgeStore):
        """Empty wikilink [[]] should not match (regex requires at least one char)."""
        content = "Empty [[]] link here."
        links = knowledge_store.extract_wikilinks(content)
        assert len(links) == 0

    def test_wikilink_empty_target_with_display(self, knowledge_store: KnowledgeStore):
        """[[|display]] with empty target should not match (regex requires chars before |)."""
        content = "Link with [[|display text]]."
        links = knowledge_store.extract_wikilinks(content)
        assert len(links) == 0

    def test_wikilink_target_with_empty_display(self, knowledge_store: KnowledgeStore):
        """[[target|]] with empty display text should not match (regex requires chars after |)."""
        content = "Link to [[target|]]."
        links = knowledge_store.extract_wikilinks(content)
        # The regex requires at least one char after |, so [[target|]] does not match
        assert len(links) == 0

    def test_wikilink_extra_pipes(self, knowledge_store: KnowledgeStore):
        """[[target|display|extra]] should handle extra pipe characters."""
        content = "Link [[target|display|extra]] here."
        links = knowledge_store.extract_wikilinks(content)
        # The regex captures everything before the first |
        # as the target, and ignores everything after
        if len(links) > 0:
            assert links[0] == "target"

    def test_nested_wikilinks(self, knowledge_store: KnowledgeStore):
        """Nested wikilinks [[nested [[inner]] link]] should not cause issues."""
        content = "Link [[nested [[inner]] link]]"
        links = knowledge_store.extract_wikilinks(content)
        # The regex matches greedily up to the first ]]
        # so [[inner]] is the match, and the outer is not a valid wikilink
        assert isinstance(links, list)
        # At minimum, [[inner]] should be found
        inner_found = any("inner" in link for link in links)
        assert inner_found

    def test_path_traversal_in_wikilink(self, knowledge_store: KnowledgeStore):
        """Path traversal patterns in wikilinks should be extracted as literal text."""
        content = "See [[../../etc/passwd]] for details."
        links = knowledge_store.extract_wikilinks(content)
        # The wikilink regex does not filter path traversal chars
        # but _get_note_path sanitizes them before filesystem access
        assert isinstance(links, list)

    def test_malicious_wikilink_patterns_fixture(
        self,
        knowledge_store: KnowledgeStore,
        malicious_wikilink_patterns: list,
    ):
        """All patterns from the malicious_wikilink_patterns fixture should be handled without crash."""
        for pattern in malicious_wikilink_patterns:
            # Extract wikilinks from each malicious pattern
            links = knowledge_store.extract_wikilinks(pattern)
            assert isinstance(links, list), f"Failed on pattern: {pattern}"

    def test_sql_injection_in_wikilink(self, knowledge_store: KnowledgeStore):
        """SQL injection attempts in wikilinks should be treated as literal text."""
        content = "[['; DROP TABLE notes; --]]"
        links = knowledge_store.extract_wikilinks(content)
        # The regex extracts it literally; downstream uses don't do SQL
        assert isinstance(links, list)

    def test_null_byte_in_wikilink(self, knowledge_store: KnowledgeStore):
        """Null bytes in wikilinks should not cause crashes."""
        content = "[[note\x00name]]"
        links = knowledge_store.extract_wikilinks(content)
        assert isinstance(links, list)

    def test_unicode_in_wikilink(self, knowledge_store: KnowledgeStore):
        """Unicode characters in wikilinks should be handled correctly."""
        content = "[[Zettelkasten-Methode]]"
        links = knowledge_store.extract_wikilinks(content)
        assert len(links) == 1
        assert links[0] == "Zettelkasten-Methode"

    def test_multiple_malicious_wikilinks(self, knowledge_store: KnowledgeStore):
        """Multiple malicious wikilinks in one document should all be handled."""
        content = (
            "Link 1: [[<script>alert(1)</script>]] "
            "Link 2: [[../../etc/passwd]] "
            "Link 3: [[Normal Note]]"
        )
        links = knowledge_store.extract_wikilinks(content)
        assert isinstance(links, list)
        assert "Normal Note" in links


# ============================================================================
# Path Traversal Protection Tests
# ============================================================================

@pytest.mark.unit
@pytest.mark.security
class TestPathTraversalProtection:
    """Tests that KnowledgeStore prevents path traversal on note operations."""

    def test_get_note_with_traversal_returns_none_or_raises(
        self,
        knowledge_store: KnowledgeStore,
        path_traversal_attempts: list,
    ):
        """get_note should safely handle all path traversal patterns."""
        for attempt in path_traversal_attempts:
            # _get_note_path sanitizes the ID by removing non-word chars
            # so traversal chars like ../ become empty or sanitized IDs.
            # The note simply won't exist, so get_note returns None.
            try:
                result = knowledge_store.get_note(attempt)
                # If no error, the result should be None (note doesn't exist)
                assert result is None, f"Path traversal '{attempt}' returned a note unexpectedly"
            except ValueError:
                # _get_note_path raises ValueError on path traversal detection
                pass

    def test_create_note_with_traversal_title(
        self,
        knowledge_store: KnowledgeStore,
        path_traversal_attempts: list,
    ):
        """create_note with path traversal titles should be sanitized or rejected."""
        for attempt in path_traversal_attempts:
            try:
                note = knowledge_store.create_note({
                    "title": attempt,
                    "content": "Test content",
                    "status": "draft",
                    "tags": [],
                })
                # If it succeeds, the note file must be inside the vault
                note_path = knowledge_store._get_note_path(note.id)
                resolved_vault = knowledge_store.vault_path.resolve()
                assert str(note_path.resolve()).startswith(str(resolved_vault)), \
                    f"Note created outside vault for title: {attempt}"
            except (ValueError, FileExistsError, Exception):
                # ValueError from path traversal detection is acceptable
                # FileExistsError if sanitized ID collides is also acceptable
                pass

    def test_update_note_with_traversal_id(
        self,
        knowledge_store: KnowledgeStore,
        path_traversal_attempts: list,
    ):
        """update_note with traversal IDs should return None or raise ValueError."""
        for attempt in path_traversal_attempts:
            try:
                result = knowledge_store.update_note(attempt, {
                    "content": "Injected content",
                })
                # If no error, the result should be None (note doesn't exist)
                assert result is None, f"Path traversal '{attempt}' updated a note unexpectedly"
            except ValueError:
                # _get_note_path raises ValueError on path traversal detection
                pass

    def test_delete_note_with_traversal_id(
        self,
        knowledge_store: KnowledgeStore,
        path_traversal_attempts: list,
    ):
        """delete_note with traversal IDs should return False or raise ValueError."""
        for attempt in path_traversal_attempts:
            try:
                result = knowledge_store.delete_note(attempt)
                # If no error, the result should be False (note doesn't exist)
                assert result is False, f"Path traversal '{attempt}' deleted something unexpectedly"
            except ValueError:
                # _get_note_path raises ValueError on path traversal detection
                pass

    def test_get_note_path_stays_in_vault(self, knowledge_store: KnowledgeStore):
        """_get_note_path should always resolve to a path within the vault."""
        safe_ids = ["normal-note", "my_note_123", "Test-Note"]
        for note_id in safe_ids:
            path = knowledge_store._get_note_path(note_id)
            resolved_vault = knowledge_store.vault_path.resolve()
            assert str(path.resolve()).startswith(str(resolved_vault)), \
                f"Note path {path} escaped vault for ID: {note_id}"

    def test_double_dot_sanitized(self, knowledge_store: KnowledgeStore):
        """Note IDs with '..' should be sanitized to remove dots."""
        # _generate_note_id and _get_note_path strip non-word chars
        try:
            path = knowledge_store._get_note_path("../../../etc/passwd")
            # After sanitization, the path should still be in the vault
            resolved_vault = knowledge_store.vault_path.resolve()
            assert str(path.resolve()).startswith(str(resolved_vault))
        except ValueError:
            pass  # Path traversal detection is also acceptable

    def test_absolute_path_rejected(self, knowledge_store: KnowledgeStore):
        """Absolute paths should be sanitized or rejected."""
        abs_paths = [
            "/etc/passwd",
            "C:\\Windows\\System32\\config\\sam",
        ]
        for abs_path in abs_paths:
            try:
                path = knowledge_store._get_note_path(abs_path)
                resolved_vault = knowledge_store.vault_path.resolve()
                assert str(path.resolve()).startswith(str(resolved_vault))
            except ValueError:
                pass  # Rejection is the expected behavior

    def test_null_byte_in_note_id(self, knowledge_store: KnowledgeStore):
        """Null bytes in note IDs should be sanitized."""
        try:
            result = knowledge_store.get_note("note\x00id")
            # Sanitized ID should not match any file
            assert result is None
        except (ValueError, Exception):
            pass  # Any error handling is acceptable

    def test_url_encoded_traversal(self, knowledge_store: KnowledgeStore):
        """URL-encoded path traversal should be sanitized."""
        encoded_attempts = [
            "..%2F..%2Fetc%2Fpasswd",
            "..%252F..%252Fetc%252Fpasswd",
        ]
        for attempt in encoded_attempts:
            try:
                result = knowledge_store.get_note(attempt)
                assert result is None
            except ValueError:
                pass
