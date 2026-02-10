"""Unit tests for MemoryService.

Tests cover recall_relevant, find_contradictions, and extract_from_conversation
with mocked dependencies so no real embedding model or LanceDB is needed.
"""
from unittest.mock import MagicMock, patch

import pytest

from app.services.memory import MemoryService
from app.models.note import Note, NoteFrontmatter


# ============================================================================
# Helpers
# ============================================================================


def _make_note(note_id, title, content="", status="draft", tags=None):
    """Build a minimal Note for stubbing."""
    return Note(
        id=note_id,
        title=title,
        content=content,
        frontmatter=NoteFrontmatter(
            title=title,
            status=status,
            tags=tags or [],
        ),
    )


def _mock_services():
    """Return (knowledge_store, vector_search, graph_index) mocks."""
    ks = MagicMock()
    vs = MagicMock()
    gi = MagicMock()
    return ks, vs, gi


# ============================================================================
# recall_relevant
# ============================================================================


@pytest.mark.unit
class TestRecallRelevant:
    """Tests for MemoryService.recall_relevant."""

    def test_returns_semantic_results(self):
        ks, vs, gi = _mock_services()
        vs.search.return_value = [
            {"note_id": "a", "title": "Alpha", "snippet": "alpha content", "score": 0.9},
            {"note_id": "b", "title": "Beta", "snippet": "beta content", "score": 0.7},
        ]
        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)

        results = svc.recall_relevant("test query", limit=5)

        assert len(results) == 2
        assert results[0]["note_id"] == "a"
        assert results[0]["connection_type"] == "semantic"
        vs.search.assert_called_once_with("test query", 10)  # limit * 2

    def test_boosts_graph_neighbors(self):
        ks, vs, gi = _mock_services()
        vs.search.return_value = [
            {"note_id": "a", "title": "Alpha", "snippet": "", "score": 0.5},
            {"note_id": "b", "title": "Beta", "snippet": "", "score": 0.4},
        ]
        # Note "b" is a graph neighbor of the context note
        gi.get_neighbors.return_value = {"ctx": ["b"]}

        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)
        results = svc.recall_relevant("query", context_note_ids=["ctx"], limit=5)

        b_result = next(r for r in results if r["note_id"] == "b")
        assert b_result["connection_type"] == "both"
        # Score should be boosted above original 0.4
        assert b_result["relevance_score"] > 0.4

    def test_adds_graph_only_neighbors(self):
        ks, vs, gi = _mock_services()
        vs.search.return_value = [
            {"note_id": "a", "title": "Alpha", "snippet": "", "score": 0.9},
        ]
        gi.get_neighbors.return_value = {"ctx": ["c"]}
        ks.get_note.return_value = _make_note("c", "Charlie", "charlie content")

        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)
        results = svc.recall_relevant("query", context_note_ids=["ctx"], limit=5)

        ids = [r["note_id"] for r in results]
        assert "c" in ids
        c_result = next(r for r in results if r["note_id"] == "c")
        assert c_result["connection_type"] == "graph"

    def test_limits_results(self):
        ks, vs, gi = _mock_services()
        vs.search.return_value = [
            {"note_id": f"n{i}", "title": f"Note {i}", "snippet": "", "score": 1.0 - i * 0.1}
            for i in range(10)
        ]

        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)
        results = svc.recall_relevant("query", limit=3)

        assert len(results) == 3

    def test_empty_semantic_results(self):
        ks, vs, gi = _mock_services()
        vs.search.return_value = []

        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)
        results = svc.recall_relevant("query", limit=5)

        assert results == []

    def test_no_context_ids_skips_graph(self):
        ks, vs, gi = _mock_services()
        vs.search.return_value = [
            {"note_id": "a", "title": "Alpha", "snippet": "", "score": 0.9},
        ]

        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)
        results = svc.recall_relevant("query")

        gi.get_neighbors.assert_not_called()
        assert results[0]["connection_type"] == "semantic"


# ============================================================================
# find_contradictions
# ============================================================================


@pytest.mark.unit
class TestFindContradictions:
    """Tests for MemoryService.find_contradictions."""

    def test_detects_status_contradiction(self):
        ks, vs, gi = _mock_services()
        ks.get_note.side_effect = lambda nid: {
            "note-a": _make_note("note-a", "Topic A", status="draft", tags=["ai"]),
            "note-b": _make_note("note-b", "Topic B", status="canonical", tags=["ai"]),
        }.get(nid)

        vs.search.return_value = [
            {"note_id": "note-b", "title": "Topic B", "score": 0.95},
        ]

        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)
        contradictions = svc.find_contradictions("note-a")

        assert len(contradictions) >= 1
        status_c = [c for c in contradictions if c["conflicting_field"] == "status"]
        assert len(status_c) == 1
        assert status_c[0]["this_value"] == "draft"
        assert status_c[0]["other_value"] == "canonical"

    def test_detects_tag_contradiction(self):
        ks, vs, gi = _mock_services()
        ks.get_note.side_effect = lambda nid: {
            "note-a": _make_note("note-a", "Topic A", status="draft", tags=["python"]),
            "note-b": _make_note("note-b", "Topic B", status="draft", tags=["javascript"]),
        }.get(nid)

        vs.search.return_value = [
            {"note_id": "note-b", "title": "Topic B", "score": 0.85},
        ]

        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)
        contradictions = svc.find_contradictions("note-a")

        tag_c = [c for c in contradictions if c["conflicting_field"] == "tags"]
        assert len(tag_c) == 1

    def test_no_contradiction_for_same_status(self):
        ks, vs, gi = _mock_services()
        ks.get_note.side_effect = lambda nid: {
            "note-a": _make_note("note-a", "Topic A", status="draft", tags=["ai"]),
            "note-b": _make_note("note-b", "Topic B", status="draft", tags=["ai"]),
        }.get(nid)

        vs.search.return_value = [
            {"note_id": "note-b", "title": "Topic B", "score": 0.9},
        ]

        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)
        contradictions = svc.find_contradictions("note-a")

        # Same status + overlapping tags => no contradiction
        assert len(contradictions) == 0

    def test_skips_self_in_results(self):
        ks, vs, gi = _mock_services()
        ks.get_note.return_value = _make_note("note-a", "Topic A", status="draft")

        vs.search.return_value = [
            {"note_id": "note-a", "title": "Topic A", "score": 1.0},
        ]

        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)
        contradictions = svc.find_contradictions("note-a")

        assert len(contradictions) == 0

    def test_returns_empty_for_missing_note(self):
        ks, vs, gi = _mock_services()
        ks.get_note.return_value = None

        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)
        contradictions = svc.find_contradictions("nonexistent")

        assert contradictions == []

    def test_low_similarity_ignored(self):
        ks, vs, gi = _mock_services()
        ks.get_note.side_effect = lambda nid: {
            "note-a": _make_note("note-a", "Topic A", status="draft", tags=["python"]),
            "note-b": _make_note("note-b", "Different", status="canonical", tags=["javascript"]),
        }.get(nid)

        vs.search.return_value = [
            {"note_id": "note-b", "title": "Different", "score": 0.5},
        ]

        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)
        contradictions = svc.find_contradictions("note-a")

        # Similarity 0.5 < 0.8 threshold => no contradiction
        assert len(contradictions) == 0


# ============================================================================
# extract_from_conversation
# ============================================================================


@pytest.mark.unit
class TestExtractFromConversation:
    """Tests for MemoryService.extract_from_conversation."""

    def test_extracts_assistant_messages(self):
        ks, vs, gi = _mock_services()
        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)

        messages = [
            {"role": "user", "content": "Tell me about Python"},
            {"role": "assistant", "content": "Python is a versatile programming language used widely in data science, web development, and automation. " * 3},
        ]
        suggestions = svc.extract_from_conversation(messages)

        assert len(suggestions) == 1
        assert suggestions[0]["status"] == "draft"
        assert suggestions[0]["source"] == "conversation"

    def test_skips_user_messages(self):
        ks, vs, gi = _mock_services()
        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)

        messages = [
            {"role": "user", "content": "This is a long user message that should be skipped regardless of length. " * 10},
        ]
        suggestions = svc.extract_from_conversation(messages)

        assert len(suggestions) == 0

    def test_skips_short_assistant_messages(self):
        ks, vs, gi = _mock_services()
        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)

        messages = [
            {"role": "assistant", "content": "OK"},
        ]
        suggestions = svc.extract_from_conversation(messages)

        assert len(suggestions) == 0

    def test_custom_source_label(self):
        ks, vs, gi = _mock_services()
        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)

        messages = [
            {"role": "assistant", "content": "A substantive response about databases and their importance in modern applications. " * 3},
        ]
        suggestions = svc.extract_from_conversation(messages, source="chatgpt-import")

        assert suggestions[0]["source"] == "chatgpt-import"

    def test_extracts_tags_from_content(self):
        ks, vs, gi = _mock_services()
        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)

        messages = [
            {"role": "assistant", "content": "This is about #python and #data-science which are very important for modern analysis workflows and techniques"},
        ]
        suggestions = svc.extract_from_conversation(messages)

        assert "python" in suggestions[0]["tags"]
        assert "data-science" in suggestions[0]["tags"]

    def test_title_extracted_from_first_line(self):
        ks, vs, gi = _mock_services()
        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)

        messages = [
            {"role": "assistant", "content": "Understanding Neural Networks\n\nNeural networks are computational models inspired by the brain. " * 3},
        ]
        suggestions = svc.extract_from_conversation(messages)

        assert suggestions[0]["title"] == "Understanding Neural Networks"

    def test_multiple_assistant_messages(self):
        ks, vs, gi = _mock_services()
        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)

        messages = [
            {"role": "user", "content": "Q1"},
            {"role": "assistant", "content": "First substantive answer about topic one that has enough content to be meaningful. " * 3},
            {"role": "user", "content": "Q2"},
            {"role": "assistant", "content": "Second substantive answer about topic two that also has sufficient content for extraction. " * 3},
        ]
        suggestions = svc.extract_from_conversation(messages)

        assert len(suggestions) == 2

    def test_empty_messages_list(self):
        ks, vs, gi = _mock_services()
        svc = MemoryService(knowledge_store=ks, vector_search=vs, graph_index=gi)

        suggestions = svc.extract_from_conversation([])

        assert suggestions == []


# ============================================================================
# _extract_title
# ============================================================================


@pytest.mark.unit
class TestExtractTitle:
    """Tests for MemoryService._extract_title static method."""

    def test_uses_first_line(self):
        assert MemoryService._extract_title("First Line\nSecond") == "First Line"

    def test_strips_markdown_heading(self):
        assert MemoryService._extract_title("# Heading\nBody") == "Heading"

    def test_truncates_long_titles(self):
        long_line = "A" * 120
        title = MemoryService._extract_title(long_line)
        assert len(title) == 80

    def test_skips_blank_lines(self):
        assert MemoryService._extract_title("\n\nActual Title\nBody") == "Actual Title"

    def test_fallback_for_empty_content(self):
        assert MemoryService._extract_title("") == "Untitled Note"
        assert MemoryService._extract_title("   \n  \n  ") == "Untitled Note"
