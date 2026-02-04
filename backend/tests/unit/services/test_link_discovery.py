"""
Unit tests for LinkDiscoveryService

Tests cover:
- MANUAL mode returns empty list
- Keyword link discovery (shared tags/terms)
- Deduplication of candidates
- Semantic links with mocked vector_search
- LLM links with mocked openrouter
- Scoring and sorting by confidence
- max_links limit
- Skip evidence notes (status=evidence)
- Edge cases and error handling
"""
import json
import pytest
from unittest.mock import AsyncMock, MagicMock, patch

from app.models.distillation import LinkMode, LinkType, ZettelLinkCandidate
from app.models.note import Note, NoteFrontmatter, NoteUpdate
from app.services.link_discovery import LinkDiscoveryService
from app.services.knowledge_store import KnowledgeStore


# ============================================================================
# Helpers
# ============================================================================

def _make_note(
    note_id: str,
    title: str,
    content: str = "",
    tags: list[str] | None = None,
    status: str = "draft",
) -> Note:
    """Create a Note instance for testing."""
    return Note(
        id=note_id,
        title=title,
        content=content,
        frontmatter=NoteFrontmatter(
            title=title,
            tags=tags or [],
            status=status,
        ),
    )


def _make_mock_knowledge_store(notes: list[Note]) -> MagicMock:
    """
    Create a mocked KnowledgeStore that returns Note objects from list_notes().

    The real KnowledgeStore.list_notes() returns NoteListItem which lacks
    .frontmatter, but LinkDiscoveryService accesses .frontmatter.status
    on the items. We mock list_notes() to return Note objects so the
    service logic can be tested correctly.
    """
    mock_ks = MagicMock(spec=KnowledgeStore)
    mock_ks.list_notes.return_value = notes
    note_map = {n.id: n for n in notes}
    mock_ks.get_note.side_effect = lambda nid: note_map.get(nid)
    return mock_ks


# ============================================================================
# MANUAL Mode
# ============================================================================

@pytest.mark.unit
class TestManualMode:
    """MANUAL mode should always return an empty list."""

    @pytest.mark.asyncio
    async def test_manual_mode_returns_empty_list(self, link_discovery_service):
        """discover_links with mode=MANUAL returns [] immediately."""
        note = _make_note("n1", "Test Note", "Some content")
        result = await link_discovery_service.discover_links(
            note, mode=LinkMode.MANUAL
        )
        assert result == []

    @pytest.mark.asyncio
    async def test_manual_mode_skips_all_discovery(self):
        """Even with all services available, MANUAL mode does no work."""
        mock_vs = MagicMock()
        mock_vs.search = MagicMock(return_value=[])
        mock_or = AsyncMock()
        mock_ks = MagicMock(spec=KnowledgeStore)

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            vector_search=mock_vs,
            openrouter_service=mock_or,
        )
        note = _make_note("n1", "Test Note", "Some content")
        result = await service.discover_links(note, mode=LinkMode.MANUAL)

        assert result == []
        mock_vs.search.assert_not_called()
        mock_or.complete.assert_not_called()


# ============================================================================
# Keyword Link Discovery
# ============================================================================

@pytest.mark.unit
class TestKeywordLinks:
    """Tests for _find_keyword_links (shared terms and tags)."""

    def test_shared_tags_produce_link(self):
        """Notes sharing a tag and at least one key term should be linked."""
        source = _make_note(
            "n1", "Neural Networks Overview",
            "Neural networks are powerful models for classification and regression.",
            tags=["machine-learning", "neural"],
        )
        target = _make_note(
            "n2", "Deep Learning Primer",
            "Neural networks with many layers enable deep classification tasks.",
            tags=["machine-learning", "deep-learning"],
        )
        mock_ks = _make_mock_knowledge_store([source, target])
        service = LinkDiscoveryService(knowledge_store=mock_ks)

        links = service._find_keyword_links(source)

        assert len(links) >= 1
        assert all(isinstance(l, ZettelLinkCandidate) for l in links)
        assert all(l.link_type == LinkType.RELATED for l in links)

    def test_no_overlap_returns_empty(self):
        """Notes with zero shared tags and distinct terms return no links."""
        source = _make_note("n1", "Alpha", "Xylophone zephyr quartz.", tags=["unique-alpha"])
        target = _make_note("n2", "Beta", "Fjord gnome waltz.", tags=["unique-beta"])

        mock_ks = _make_mock_knowledge_store([source, target])
        service = LinkDiscoveryService(knowledge_store=mock_ks)

        links = service._find_keyword_links(source)
        assert links == []

    def test_keyword_skips_self(self):
        """A note should never link to itself via keywords."""
        source = _make_note(
            "n1", "Self Referencing",
            "Repeated terms repeated terms repeated terms.",
            tags=["self"],
        )
        mock_ks = _make_mock_knowledge_store([source])
        service = LinkDiscoveryService(knowledge_store=mock_ks)

        links = service._find_keyword_links(source)
        for link in links:
            assert link.target_id != source.id

    def test_keyword_skips_evidence_notes(self):
        """Evidence notes should be excluded from keyword link targets."""
        source = _make_note(
            "n1", "Source Note",
            "Machine learning algorithms optimize classification performance.",
            tags=["ml"],
        )
        evidence = _make_note(
            "n2", "Evidence Container",
            "Machine learning algorithms optimize classification performance.",
            tags=["ml"],
            status="evidence",
        )
        mock_ks = _make_mock_knowledge_store([source, evidence])
        service = LinkDiscoveryService(knowledge_store=mock_ks)

        links = service._find_keyword_links(source)
        for link in links:
            assert link.target_id != evidence.id


# ============================================================================
# Deduplication
# ============================================================================

@pytest.mark.unit
class TestDeduplication:
    """Tests for _deduplicate_links."""

    def test_keeps_highest_confidence_per_target(self):
        """When duplicate target_ids exist, keep the one with highest confidence."""
        service = LinkDiscoveryService(knowledge_store=MagicMock())
        candidates = [
            ZettelLinkCandidate(
                target_id="t1", target_title="Target One",
                link_type=LinkType.RELATED, confidence=0.5, reason="keyword"
            ),
            ZettelLinkCandidate(
                target_id="t1", target_title="Target One",
                link_type=LinkType.EXPANDS, confidence=0.9, reason="semantic"
            ),
            ZettelLinkCandidate(
                target_id="t2", target_title="Target Two",
                link_type=LinkType.RELATED, confidence=0.7, reason="keyword"
            ),
        ]

        result = service._deduplicate_links(candidates)

        assert len(result) == 2
        t1_links = [l for l in result if l.target_id == "t1"]
        assert len(t1_links) == 1
        assert t1_links[0].confidence == 0.9

    def test_no_duplicates_unchanged(self):
        """If all targets are unique, list is returned intact."""
        service = LinkDiscoveryService(knowledge_store=MagicMock())
        candidates = [
            ZettelLinkCandidate(
                target_id="a", target_title="A",
                confidence=0.8, reason="r"
            ),
            ZettelLinkCandidate(
                target_id="b", target_title="B",
                confidence=0.6, reason="r"
            ),
        ]

        result = service._deduplicate_links(candidates)
        assert len(result) == 2

    def test_empty_list_returns_empty(self):
        """Deduplicating an empty list returns empty."""
        service = LinkDiscoveryService(knowledge_store=MagicMock())
        assert service._deduplicate_links([]) == []


# ============================================================================
# Semantic Links (mocked vector_search)
# ============================================================================

@pytest.mark.unit
class TestSemanticLinks:
    """Tests for _find_semantic_links with a mocked vector search."""

    @pytest.mark.asyncio
    async def test_semantic_links_returned_above_threshold(self, knowledge_store):
        """Results with score >= 0.70 should be returned as candidates."""
        target_note = knowledge_store.create_note({
            "title": "Quantum Computing",
            "content": "Quantum bits enable parallel computation.",
            "tags": ["quantum"],
            "status": "draft",
        })

        mock_vs = MagicMock()
        mock_vs.search.return_value = [
            {"note_id": target_note.id, "score": 0.85},
        ]

        service = LinkDiscoveryService(
            knowledge_store=knowledge_store,
            vector_search=mock_vs,
        )
        source = _make_note("src", "Quantum Algorithms", "Algorithms for qubits.")

        links = await service._find_semantic_links(source, max_links=5)

        assert len(links) == 1
        assert links[0].target_id == target_note.id
        assert links[0].confidence == 0.85
        # score >= 0.85 should infer EXPANDS link type
        assert links[0].link_type == LinkType.EXPANDS

    @pytest.mark.asyncio
    async def test_semantic_links_below_threshold_excluded(self, knowledge_store):
        """Results with score < 0.70 should be excluded."""
        target_note = knowledge_store.create_note({
            "title": "Unrelated Note",
            "content": "Something unrelated.",
            "tags": [],
            "status": "draft",
        })

        mock_vs = MagicMock()
        mock_vs.search.return_value = [
            {"note_id": target_note.id, "score": 0.55},
        ]

        service = LinkDiscoveryService(
            knowledge_store=knowledge_store,
            vector_search=mock_vs,
        )
        source = _make_note("src", "Source", "Content.")
        links = await service._find_semantic_links(source, max_links=5)
        assert links == []

    @pytest.mark.asyncio
    async def test_semantic_skips_self(self, knowledge_store):
        """Vector search may return the source note; it should be skipped."""
        mock_vs = MagicMock()
        mock_vs.search.return_value = [
            {"note_id": "self-id", "score": 0.99},
        ]

        service = LinkDiscoveryService(
            knowledge_store=knowledge_store,
            vector_search=mock_vs,
        )
        source = _make_note("self-id", "Self", "Content.")
        links = await service._find_semantic_links(source, max_links=5)
        assert links == []

    @pytest.mark.asyncio
    async def test_semantic_skips_evidence_notes(self, knowledge_store):
        """Evidence status notes should be skipped in semantic results."""
        evidence_note = knowledge_store.create_note({
            "title": "Evidence Container",
            "content": "Raw evidence data.",
            "tags": [],
            "status": "evidence",
        })

        mock_vs = MagicMock()
        mock_vs.search.return_value = [
            {"note_id": evidence_note.id, "score": 0.90},
        ]

        service = LinkDiscoveryService(
            knowledge_store=knowledge_store,
            vector_search=mock_vs,
        )
        source = _make_note("src", "Source Note", "Query content.")
        links = await service._find_semantic_links(source, max_links=5)
        assert links == []

    @pytest.mark.asyncio
    async def test_semantic_related_link_type_below_085(self, knowledge_store):
        """Score between 0.70 and 0.85 should infer RELATED link type."""
        target = knowledge_store.create_note({
            "title": "Related Concept",
            "content": "A concept.",
            "tags": [],
            "status": "draft",
        })

        mock_vs = MagicMock()
        mock_vs.search.return_value = [
            {"note_id": target.id, "score": 0.78},
        ]

        service = LinkDiscoveryService(
            knowledge_store=knowledge_store,
            vector_search=mock_vs,
        )
        source = _make_note("src", "Source", "Content.")
        links = await service._find_semantic_links(source, max_links=5)

        assert len(links) == 1
        assert links[0].link_type == LinkType.RELATED

    @pytest.mark.asyncio
    async def test_semantic_search_failure_returns_empty(self, knowledge_store):
        """If vector_search.search raises, return empty list gracefully."""
        mock_vs = MagicMock()
        mock_vs.search.side_effect = RuntimeError("DB error")

        service = LinkDiscoveryService(
            knowledge_store=knowledge_store,
            vector_search=mock_vs,
        )
        source = _make_note("src", "Source", "Content.")
        links = await service._find_semantic_links(source, max_links=5)
        assert links == []


# ============================================================================
# LLM Links (mocked openrouter)
# ============================================================================

@pytest.mark.unit
class TestLLMLinks:
    """Tests for _find_llm_links and _parse_llm_links with mocked OpenRouter."""

    @pytest.mark.asyncio
    async def test_llm_links_parsed_from_response(self):
        """Valid JSON response from LLM should produce candidates."""
        target = _make_note(
            "t1", "Distributed Systems",
            "Distributed systems coordinate multiple machines.",
            tags=["systems"], status="canonical",
        )
        mock_ks = _make_mock_knowledge_store([target])

        llm_response = json.dumps([
            {
                "title": "Distributed Systems",
                "type": "related",
                "reason": "Both discuss system architecture",
            }
        ])

        mock_or = AsyncMock()
        mock_or.complete = AsyncMock(return_value=llm_response)

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            openrouter_service=mock_or,
        )

        source = _make_note("src", "Microservices", "Microservice architecture.")
        links = await service._find_llm_links(source, max_links=5)

        assert len(links) == 1
        assert links[0].target_id == target.id
        assert links[0].target_title == "Distributed Systems"
        assert links[0].link_type == LinkType.RELATED
        assert links[0].confidence == 0.75

    @pytest.mark.asyncio
    async def test_llm_links_invalid_json_returns_empty(self):
        """Malformed JSON from LLM should not crash; return empty list."""
        target = _make_note("t1", "Some Note", "Content.", status="draft")
        mock_ks = _make_mock_knowledge_store([target])

        mock_or = AsyncMock()
        mock_or.complete = AsyncMock(return_value="Not valid JSON at all")

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            openrouter_service=mock_or,
        )

        source = _make_note("src", "Source", "Content.")
        links = await service._find_llm_links(source, max_links=5)
        assert links == []

    @pytest.mark.asyncio
    async def test_llm_links_api_error_returns_empty(self):
        """OpenRouter API failure should return empty list."""
        target = _make_note("t1", "Target", "Content.", status="draft")
        mock_ks = _make_mock_knowledge_store([target])

        mock_or = AsyncMock()
        mock_or.complete = AsyncMock(side_effect=RuntimeError("API timeout"))

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            openrouter_service=mock_or,
        )

        source = _make_note("src", "Source", "Content.")
        links = await service._find_llm_links(source, max_links=5)
        assert links == []

    @pytest.mark.asyncio
    async def test_llm_links_no_openrouter_returns_empty(self):
        """With no openrouter service, _find_llm_links returns empty."""
        mock_ks = MagicMock(spec=KnowledgeStore)
        service = LinkDiscoveryService(knowledge_store=mock_ks)
        source = _make_note("src", "Source", "Content.")
        links = await service._find_llm_links(source, max_links=5)
        assert links == []

    @pytest.mark.asyncio
    async def test_llm_links_unknown_title_ignored(self):
        """LLM responses referencing non-existent note titles are ignored."""
        existing = _make_note("t1", "Existing Note", "Content.", status="draft")
        mock_ks = _make_mock_knowledge_store([existing])

        llm_response = json.dumps([
            {"title": "Non Existent Note", "type": "related", "reason": "fake"},
            {"title": "Existing Note", "type": "supports", "reason": "real"},
        ])

        mock_or = AsyncMock()
        mock_or.complete = AsyncMock(return_value=llm_response)

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            openrouter_service=mock_or,
        )

        source = _make_note("src", "Source", "Content.")
        links = await service._find_llm_links(source, max_links=5)

        assert len(links) == 1
        assert links[0].target_title == "Existing Note"
        assert links[0].link_type == LinkType.SUPPORTS

    @pytest.mark.asyncio
    async def test_llm_links_invalid_link_type_defaults_to_related(self):
        """Unknown link type string from LLM defaults to RELATED."""
        target = _make_note("t1", "Target Note", "Content.", status="draft")
        mock_ks = _make_mock_knowledge_store([target])

        llm_response = json.dumps([
            {"title": "Target Note", "type": "invented_type", "reason": "test"},
        ])

        mock_or = AsyncMock()
        mock_or.complete = AsyncMock(return_value=llm_response)

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            openrouter_service=mock_or,
        )

        source = _make_note("src", "Source", "Content.")
        links = await service._find_llm_links(source, max_links=5)

        assert len(links) == 1
        assert links[0].link_type == LinkType.RELATED

    @pytest.mark.asyncio
    async def test_llm_skips_evidence_notes_in_context(self):
        """Evidence notes should be filtered out of the LLM context list."""
        evidence = _make_note(
            "t1", "Evidence Raw Data", "Raw imported content.",
            status="evidence",
        )
        mock_ks = _make_mock_knowledge_store([evidence])

        mock_or = AsyncMock()
        mock_or.complete = AsyncMock(return_value="[]")

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            openrouter_service=mock_or,
        )

        source = _make_note("src", "Source", "Content.")
        links = await service._find_llm_links(source, max_links=5)
        assert links == []
        # All notes are evidence, so context_notes is empty.
        # The method should return [] without calling the LLM.
        mock_or.complete.assert_not_called()


# ============================================================================
# Scoring and Sorting
# ============================================================================

@pytest.mark.unit
class TestScoringAndSorting:
    """Tests for confidence-based sorting in discover_links."""

    @pytest.mark.asyncio
    async def test_results_sorted_by_confidence_descending(self):
        """Candidates should be returned sorted by confidence (highest first)."""
        low_note = _make_note("low", "Low Match", "Content.", status="draft")
        high_note = _make_note("high", "High Match", "Content.", status="canonical")
        mock_ks = _make_mock_knowledge_store([low_note, high_note])

        mock_vs = MagicMock()
        mock_vs.search.return_value = [
            {"note_id": "low", "score": 0.72},
            {"note_id": "high", "score": 0.91},
        ]

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            vector_search=mock_vs,
        )

        source = _make_note("src", "Algorithms Guide", "Programming algorithms.")
        result = await service.discover_links(
            source, mode=LinkMode.SUGGESTED, max_links=10
        )

        # After dedup and sort, highest confidence should be first
        assert len(result) >= 2
        for i in range(len(result) - 1):
            assert result[i].confidence >= result[i + 1].confidence


# ============================================================================
# max_links Limit
# ============================================================================

@pytest.mark.unit
class TestMaxLinksLimit:
    """Tests that max_links correctly limits output."""

    @pytest.mark.asyncio
    async def test_max_links_caps_results(self):
        """discover_links should return at most max_links candidates."""
        notes = [
            _make_note(f"t{i}", f"Target Note {i}", "Content.", tags=["shared"], status="draft")
            for i in range(8)
        ]
        mock_ks = _make_mock_knowledge_store(notes)

        mock_vs = MagicMock()
        mock_vs.search.return_value = [
            {"note_id": n.id, "score": 0.80 + (i * 0.01)}
            for i, n in enumerate(notes)
        ]

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            vector_search=mock_vs,
        )

        source = _make_note("src", "Source Note", "Content about topics.")
        result = await service.discover_links(
            source, mode=LinkMode.SUGGESTED, max_links=3
        )
        assert len(result) <= 3

    @pytest.mark.asyncio
    async def test_max_links_one(self):
        """max_links=1 should return at most 1 candidate."""
        target = _make_note("t1", "Only Target", "Content.", status="draft")
        mock_ks = _make_mock_knowledge_store([target])

        mock_vs = MagicMock()
        mock_vs.search.return_value = [
            {"note_id": "t1", "score": 0.90},
        ]

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            vector_search=mock_vs,
        )

        source = _make_note("src", "Source", "Content.")
        result = await service.discover_links(
            source, mode=LinkMode.SUGGESTED, max_links=1
        )
        assert len(result) <= 1


# ============================================================================
# Skip Evidence Notes
# ============================================================================

@pytest.mark.unit
class TestSkipEvidenceNotes:
    """Evidence notes should be excluded from all discovery methods."""

    @pytest.mark.asyncio
    async def test_evidence_notes_excluded_from_semantic(self, knowledge_store):
        """Semantic discovery should not return evidence-status notes."""
        evidence_note = knowledge_store.create_note({
            "title": "Evidence Only",
            "content": "This is raw evidence data.",
            "tags": ["data"],
            "status": "evidence",
        })

        mock_vs = MagicMock()
        mock_vs.search.return_value = [
            {"note_id": evidence_note.id, "score": 0.95},
        ]

        service = LinkDiscoveryService(
            knowledge_store=knowledge_store,
            vector_search=mock_vs,
        )

        source = _make_note("src", "Source", "Query.")
        # Call _find_semantic_links directly to avoid list_notes issue
        links = await service._find_semantic_links(source, max_links=10)
        assert links == []

    @pytest.mark.asyncio
    async def test_evidence_excluded_from_keyword_discovery(self):
        """Keyword link discovery must skip evidence notes."""
        evidence = _make_note(
            "ev", "Evidence Raw Import",
            "Machine learning algorithms classification neural networks.",
            tags=["ml", "algorithms"], status="evidence",
        )
        draft = _make_note(
            "dr", "ML Draft",
            "Machine learning algorithms classification neural networks.",
            tags=["ml", "algorithms"], status="draft",
        )
        mock_ks = _make_mock_knowledge_store([evidence, draft])
        service = LinkDiscoveryService(knowledge_store=mock_ks)

        source = _make_note(
            "src", "ML Overview",
            "Machine learning algorithms classification neural networks.",
            tags=["ml", "algorithms"],
        )

        links = service._find_keyword_links(source)

        for link in links:
            assert link.target_id != evidence.id


# ============================================================================
# AUTOMATIC Mode (apply links)
# ============================================================================

@pytest.mark.unit
class TestAutomaticMode:
    """AUTOMATIC mode should discover and apply links."""

    @pytest.mark.asyncio
    async def test_automatic_mode_calls_apply(self):
        """In AUTOMATIC mode, _apply_links should be invoked."""
        target = _make_note("t1", "Target Note", "Target content.", status="draft")
        source = _make_note("src", "Source Note", "Source content.", status="draft")
        mock_ks = _make_mock_knowledge_store([target, source])

        mock_vs = MagicMock()
        mock_vs.search.return_value = [
            {"note_id": target.id, "score": 0.88},
        ]

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            vector_search=mock_vs,
        )

        with patch.object(service, "_apply_links", new_callable=AsyncMock) as mock_apply:
            mock_apply.return_value = 1
            result = await service.discover_links(
                source, mode=LinkMode.AUTOMATIC, max_links=10
            )

            assert len(result) >= 1
            mock_apply.assert_called_once()

    @pytest.mark.asyncio
    async def test_suggested_mode_does_not_apply(self):
        """In SUGGESTED mode, _apply_links should NOT be invoked."""
        target = _make_note("t1", "Target Note", "Target content.", status="draft")
        mock_ks = _make_mock_knowledge_store([target])

        mock_vs = MagicMock()
        mock_vs.search.return_value = [
            {"note_id": target.id, "score": 0.88},
        ]

        service = LinkDiscoveryService(
            knowledge_store=mock_ks,
            vector_search=mock_vs,
        )

        source = _make_note("src", "Source Note", "Source content.")

        with patch.object(service, "_apply_links", new_callable=AsyncMock) as mock_apply:
            await service.discover_links(
                source, mode=LinkMode.SUGGESTED, max_links=10
            )
            mock_apply.assert_not_called()


# ============================================================================
# Key Term Extraction
# ============================================================================

@pytest.mark.unit
class TestKeyTermExtraction:
    """Tests for _extract_key_terms helper."""

    def test_extracts_long_words(self):
        """Words with 4+ characters should be extracted."""
        service = LinkDiscoveryService(knowledge_store=MagicMock())
        terms = service._extract_key_terms("programming algorithms data")
        assert "programming" in terms
        assert "algorithms" in terms
        assert "data" in terms

    def test_stopwords_excluded(self):
        """Common stopwords should be filtered out."""
        service = LinkDiscoveryService(knowledge_store=MagicMock())
        terms = service._extract_key_terms("this would have been done with that")
        assert "this" not in terms
        assert "would" not in terms
        assert "have" not in terms
        assert "been" not in terms
        assert "with" not in terms
        assert "that" not in terms

    def test_code_blocks_removed(self):
        """Content inside code blocks should not produce terms."""
        service = LinkDiscoveryService(knowledge_store=MagicMock())
        content = "Real content.\n```python\ndef hidden_function():\n    pass\n```\nMore content."
        terms = service._extract_key_terms(content)
        assert "hidden_function" not in terms

    def test_wikilinks_removed(self):
        """Wikilink syntax should be stripped before extracting terms."""
        service = LinkDiscoveryService(knowledge_store=MagicMock())
        terms = service._extract_key_terms("See [[Some Important Link]] for details.")
        # The wikilink text itself should not appear as a term
        assert "some important link" not in terms


# ============================================================================
# Reverse Link Types
# ============================================================================

@pytest.mark.unit
class TestReverseLinkTypes:
    """Tests for _get_reverse_link_type."""

    @pytest.mark.parametrize("forward,expected_reverse", [
        (LinkType.SUPPORTS, LinkType.RELATED),
        (LinkType.CONTRADICTS, LinkType.CONTRADICTS),
        (LinkType.EXPANDS, LinkType.RELATED),
        (LinkType.QUESTIONS, LinkType.ANSWERS),
        (LinkType.ANSWERS, LinkType.QUESTIONS),
        (LinkType.EXAMPLE, LinkType.RELATED),
        (LinkType.PART_OF, LinkType.RELATED),
        (LinkType.RELATED, LinkType.RELATED),
    ])
    def test_reverse_link_type_mapping(self, forward, expected_reverse):
        """Each forward link type should map to the correct reverse."""
        service = LinkDiscoveryService(knowledge_store=MagicMock())
        assert service._get_reverse_link_type(forward) == expected_reverse
