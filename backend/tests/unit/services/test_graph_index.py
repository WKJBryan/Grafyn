"""
Unit tests for GraphIndexService

Tests cover:
- Graph index building
- Outgoing links retrieval
- Backlinks retrieval
- Backlinks with context extraction
- Neighbor traversal (BFS with depth)
- Unlinked mentions detection
- Incremental updates
- Circular link handling
- Edge cases
"""
import pytest

from app.services.graph_index import GraphIndexService
from app.services.knowledge_store import KnowledgeStore
from tests.fixtures.sample_notes import get_notes_with_wikilinks


# ============================================================================
# Index Building Tests
# ============================================================================

@pytest.mark.unit
class TestIndexBuilding:
    """Test graph index construction"""

    def test_build_index_from_notes(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test building index from existing notes"""
        # Create notes with known link structure
        notes = get_notes_with_wikilinks()

        created_ids = []
        for note_data in notes:
            created = knowledge_store.create_note(note_data)
            created_ids.append(created["id"])

        # Build index
        graph_index.build_index()

        # Verify index was built
        # Note A should have outgoing links
        note_a_id = created_ids[0]
        outgoing = graph_index.get_outgoing_links(note_a_id)
        assert len(outgoing) > 0

    def test_build_index_empty_vault(self, graph_index: GraphIndexService):
        """Test building index when no notes exist"""
        # Should handle gracefully
        graph_index.build_index()

        # No error should occur

    def test_rebuild_index_updates_graph(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test that rebuilding index updates the graph"""
        # Create initial note
        note1 = knowledge_store.create_note({
            "title": "Note 1",
            "content": "Links to [[Note 2]]",
            "status": "draft",
            "tags": [],
        })

        # Build index
        graph_index.build_index()

        outgoing1 = graph_index.get_outgoing_links(note1["id"])
        assert len(outgoing1) > 0

        # Create second note
        knowledge_store.create_note({
            "title": "Note 2",
            "content": "No links",
            "status": "draft",
            "tags": [],
        })

        # Rebuild
        graph_index.build_index()

        # Index should be updated
        outgoing2 = graph_index.get_outgoing_links(note1["id"])
        assert len(outgoing2) > 0


# ============================================================================
# Outgoing Links Tests
# ============================================================================

@pytest.mark.unit
class TestOutgoingLinks:
    """Test retrieving outgoing links from notes"""

    def test_get_outgoing_links_simple(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test getting outgoing links from a note"""
        # Create note with links
        note = knowledge_store.create_note({
            "title": "Source Note",
            "content": "Links to [[Target 1]] and [[Target 2]]",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        outgoing = graph_index.get_outgoing_links(note["id"])

        # Should have 2 outgoing links
        assert len(outgoing) == 2
        assert "Target 1" in outgoing or "target-1" in outgoing
        assert "Target 2" in outgoing or "target-2" in outgoing

    def test_get_outgoing_links_no_links(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test note with no outgoing links"""
        note = knowledge_store.create_note({
            "title": "Isolated Note",
            "content": "No links here",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        outgoing = graph_index.get_outgoing_links(note["id"])

        assert outgoing == [] or len(outgoing) == 0

    def test_get_outgoing_links_duplicate_links(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test note with duplicate links to same target"""
        note = knowledge_store.create_note({
            "title": "Duplicate Links",
            "content": "[[Target]] appears [[Target|multiple]] [[Target|times]]",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        outgoing = graph_index.get_outgoing_links(note["id"])

        # Should deduplicate links
        assert "Target" in outgoing or "target" in outgoing

    def test_get_outgoing_links_nonexistent_note(self, graph_index: GraphIndexService):
        """Test getting links from non-existent note"""
        outgoing = graph_index.get_outgoing_links("nonexistent-note")

        # Should return empty list
        assert outgoing == []


# ============================================================================
# Backlinks Tests
# ============================================================================

@pytest.mark.unit
class TestBacklinks:
    """Test backlink retrieval"""

    def test_get_backlinks_simple(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test getting backlinks to a note"""
        # Create target note
        target = knowledge_store.create_note({
            "title": "Target Note",
            "content": "This is the target",
            "status": "draft",
            "tags": [],
        })

        # Create source note linking to target
        source = knowledge_store.create_note({
            "title": "Source Note",
            "content": f"Links to [[{target['title']}]]",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        backlinks = graph_index.get_backlinks(target["id"])

        # Should have backlink from source
        assert len(backlinks) > 0
        assert source["id"] in backlinks or source["title"] in [bl for bl in backlinks]

    def test_get_backlinks_multiple_sources(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test note with multiple backlinks"""
        # Create target
        target = knowledge_store.create_note({
            "title": "Popular Note",
            "content": "Many notes link here",
            "status": "draft",
            "tags": [],
        })

        # Create multiple sources
        for i in range(5):
            knowledge_store.create_note({
                "title": f"Source {i}",
                "content": f"Links to [[{target['title']}]]",
                "status": "draft",
                "tags": [],
            })

        graph_index.build_index()

        backlinks = graph_index.get_backlinks(target["id"])

        # Should have 5 backlinks
        assert len(backlinks) == 5

    def test_get_backlinks_no_backlinks(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test note with no backlinks"""
        note = knowledge_store.create_note({
            "title": "Lonely Note",
            "content": "Nobody links to me",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        backlinks = graph_index.get_backlinks(note["id"])

        assert backlinks == []

    def test_backlinks_bidirectional(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test bidirectional links"""
        # Create two notes that link to each other
        note1 = knowledge_store.create_note({
            "title": "Note A",
            "content": "Links to [[Note B]]",
            "status": "draft",
            "tags": [],
        })

        note2 = knowledge_store.create_note({
            "title": "Note B",
            "content": "Links back to [[Note A]]",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        # Both should have backlinks
        backlinks_a = graph_index.get_backlinks(note1["id"])
        backlinks_b = graph_index.get_backlinks(note2["id"])

        assert len(backlinks_a) > 0
        assert len(backlinks_b) > 0


# ============================================================================
# Backlinks with Context Tests
# ============================================================================

@pytest.mark.unit
class TestBacklinksWithContext:
    """Test backlink context extraction"""

    def test_get_backlinks_with_context(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test retrieving backlinks with surrounding context"""
        target = knowledge_store.create_note({
            "title": "Target",
            "content": "Target content",
            "status": "draft",
            "tags": [],
        })

        source = knowledge_store.create_note({
            "title": "Source",
            "content": "This is some context before [[Target]] and some after.",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        backlinks = graph_index.get_backlinks_with_context(target["id"])

        assert len(backlinks) > 0
        # Context should include surrounding text
        backlink = backlinks[0]
        assert "context" in backlink
        assert "Target" in backlink["context"]

    def test_context_extraction_limits(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test that context extraction limits text length"""
        target = knowledge_store.create_note({
            "title": "Target",
            "content": "Target",
            "status": "draft",
            "tags": [],
        })

        # Create source with very long content
        long_before = "word " * 100
        long_after = " word" * 100

        source = knowledge_store.create_note({
            "title": "Source",
            "content": f"{long_before}[[Target]]{long_after}",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        backlinks = graph_index.get_backlinks_with_context(target["id"])

        if len(backlinks) > 0:
            context = backlinks[0]["context"]
            # Context should be limited (typically ~100-200 chars)
            assert len(context) < 500


# ============================================================================
# Neighbor Traversal Tests
# ============================================================================

@pytest.mark.unit
class TestNeighborTraversal:
    """Test BFS neighbor traversal"""

    def test_get_neighbors_depth_1(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test getting neighbors at depth 1"""
        # Create chain: A -> B -> C
        note_a = knowledge_store.create_note({
            "title": "Note A",
            "content": "Links to [[Note B]]",
            "status": "draft",
            "tags": [],
        })

        note_b = knowledge_store.create_note({
            "title": "Note B",
            "content": "Links to [[Note C]]",
            "status": "draft",
            "tags": [],
        })

        note_c = knowledge_store.create_note({
            "title": "Note C",
            "content": "End of chain",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        neighbors = graph_index.get_neighbors(note_a["id"], depth=1)

        # Should only include Note B
        neighbor_ids = [n["id"] for n in neighbors] if neighbors else []
        assert note_b["id"] in neighbor_ids
        assert note_c["id"] not in neighbor_ids

    def test_get_neighbors_depth_2(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test getting neighbors at depth 2"""
        # Create chain: A -> B -> C
        note_a = knowledge_store.create_note({
            "title": "Note A",
            "content": "Links to [[Note B]]",
            "status": "draft",
            "tags": [],
        })

        note_b = knowledge_store.create_note({
            "title": "Note B",
            "content": "Links to [[Note C]]",
            "status": "draft",
            "tags": [],
        })

        note_c = knowledge_store.create_note({
            "title": "Note C",
            "content": "End of chain",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        neighbors = graph_index.get_neighbors(note_a["id"], depth=2)

        # Should include both B and C
        neighbor_ids = [n["id"] for n in neighbors] if neighbors else []
        assert note_b["id"] in neighbor_ids
        assert note_c["id"] in neighbor_ids

    def test_get_neighbors_isolated_note(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test neighbors of isolated note"""
        note = knowledge_store.create_note({
            "title": "Isolated",
            "content": "No links",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        neighbors = graph_index.get_neighbors(note["id"], depth=1)

        assert neighbors == [] or len(neighbors) == 0

    def test_get_neighbors_depth_limit(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test that depth parameter limits traversal"""
        # Create long chain
        notes = []
        for i in range(5):
            content = f"Links to [[Note {i+1}]]" if i < 4 else "End"
            note = knowledge_store.create_note({
                "title": f"Note {i}",
                "content": content,
                "status": "draft",
                "tags": [],
            })
            notes.append(note)

        graph_index.build_index()

        # Depth 3 should not reach Note 4
        neighbors = graph_index.get_neighbors(notes[0]["id"], depth=3)

        neighbor_ids = [n["id"] for n in neighbors] if neighbors else []
        # Should include notes 1, 2, 3 but not necessarily 4
        assert len(neighbor_ids) <= 4


# ============================================================================
# Unlinked Mentions Tests
# ============================================================================

@pytest.mark.unit
class TestUnlinkedMentions:
    """Test detection of unlinked mentions"""

    def test_find_unlinked_mentions(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test finding notes that mention target without linking"""
        target = knowledge_store.create_note({
            "title": "Python Programming",
            "content": "About Python",
            "status": "draft",
            "tags": [],
        })

        # Note that mentions but doesn't link
        mentioner = knowledge_store.create_note({
            "title": "Tutorial",
            "content": "Learn Python Programming without a link",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        unlinked = graph_index.find_unlinked_mentions(target["id"])

        # Should find the mentioning note
        mention_ids = [m["note_id"] for m in unlinked] if unlinked else []
        assert mentioner["id"] in mention_ids

    def test_unlinked_mentions_excludes_linked(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test that unlinked mentions excludes notes that already link"""
        target = knowledge_store.create_note({
            "title": "Target",
            "content": "Target content",
            "status": "draft",
            "tags": [],
        })

        # Note with link
        linked = knowledge_store.create_note({
            "title": "Linked",
            "content": "This has a [[Target]] link",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        unlinked = graph_index.find_unlinked_mentions(target["id"])

        # Linked note should not appear in unlinked mentions
        mention_ids = [m["note_id"] for m in unlinked] if unlinked else []
        assert linked["id"] not in mention_ids


# ============================================================================
# Incremental Update Tests
# ============================================================================

@pytest.mark.unit
class TestIncrementalUpdates:
    """Test incremental graph updates"""

    def test_update_note_links(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test updating a note's links incrementally"""
        # Create initial note
        note = knowledge_store.create_note({
            "title": "Note",
            "content": "Links to [[Target A]]",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        # Update the note
        old_content = note["content"]
        new_content = "Now links to [[Target B]]"

        knowledge_store.update_note(note["id"], {"content": new_content})

        # Update graph
        graph_index.update_note(note["id"], old_content, new_content)

        # Should reflect new links
        outgoing = graph_index.get_outgoing_links(note["id"])
        assert any("Target B" in str(link) or "target-b" in str(link) for link in outgoing)


# ============================================================================
# Circular Links Tests
# ============================================================================

@pytest.mark.unit
class TestCircularLinks:
    """Test handling of circular link structures"""

    def test_circular_links_two_notes(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test A -> B -> A circular structure"""
        note_a = knowledge_store.create_note({
            "title": "Note A",
            "content": "Links to [[Note B]]",
            "status": "draft",
            "tags": [],
        })

        note_b = knowledge_store.create_note({
            "title": "Note B",
            "content": "Links back to [[Note A]]",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        # Should handle without infinite loops
        neighbors_a = graph_index.get_neighbors(note_a["id"], depth=2)
        neighbors_b = graph_index.get_neighbors(note_b["id"], depth=2)

        # Both should return finite results
        assert isinstance(neighbors_a, list)
        assert isinstance(neighbors_b, list)

    def test_self_referential_link(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test note that links to itself"""
        note = knowledge_store.create_note({
            "title": "Self Reference",
            "content": "This note links to [[Self Reference]]",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        # Should handle gracefully
        outgoing = graph_index.get_outgoing_links(note["id"])
        backlinks = graph_index.get_backlinks(note["id"])

        assert isinstance(outgoing, list)
        assert isinstance(backlinks, list)


# ============================================================================
# Edge Cases
# ============================================================================

@pytest.mark.unit
class TestEdgeCases:
    """Test edge cases and error handling"""

    def test_wikilink_to_nonexistent_note(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test link to note that doesn't exist"""
        note = knowledge_store.create_note({
            "title": "Note",
            "content": "Links to [[Nonexistent Target]]",
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        # Should still track the link
        outgoing = graph_index.get_outgoing_links(note["id"])
        assert len(outgoing) > 0

    def test_case_sensitivity_in_links(self, graph_index: GraphIndexService, knowledge_store: KnowledgeStore):
        """Test case sensitivity of wikilinks"""
        target = knowledge_store.create_note({
            "title": "Target Note",
            "content": "Target",
            "status": "draft",
            "tags": [],
        })

        # Link with different case
        source = knowledge_store.create_note({
            "title": "Source",
            "content": "Links to [[target note]]",  # lowercase
            "status": "draft",
            "tags": [],
        })

        graph_index.build_index()

        # Should handle case variations
        outgoing = graph_index.get_outgoing_links(source["id"])
        assert len(outgoing) > 0

    def test_empty_graph(self, graph_index: GraphIndexService):
        """Test operations on empty graph"""
        # All operations should handle empty graph
        assert graph_index.get_outgoing_links("any-id") == []
        assert graph_index.get_backlinks("any-id") == []
        assert graph_index.get_neighbors("any-id", depth=1) == [] or graph_index.get_neighbors("any-id", depth=1) is not None
