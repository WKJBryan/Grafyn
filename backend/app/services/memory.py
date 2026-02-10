"""AI Memory Layer service for contextual recall, contradiction detection, and extraction."""
import logging
import re
from typing import List, Optional

from app.services.knowledge_store import KnowledgeStore
from app.services.vector_search import VectorSearchService
from app.services.graph_index import GraphIndexService
from app.services.distillation import normalize_tag

logger = logging.getLogger(__name__)


class MemoryService:
    """Service that combines semantic search and graph traversal for intelligent memory recall."""

    def __init__(
        self,
        knowledge_store: Optional[KnowledgeStore] = None,
        vector_search: Optional[VectorSearchService] = None,
        graph_index: Optional[GraphIndexService] = None,
    ):
        self._knowledge_store = knowledge_store or KnowledgeStore()
        self._vector_search = vector_search or VectorSearchService()
        self._graph_index = graph_index or GraphIndexService()

    def recall_relevant(
        self,
        query: str,
        context_note_ids: Optional[List[str]] = None,
        limit: int = 5,
    ) -> List[dict]:
        """Recall relevant notes by combining semantic search with graph neighbors.

        Args:
            query: Natural-language query string.
            context_note_ids: Optional note IDs whose graph neighbors get a score boost.
            limit: Maximum results to return.

        Returns:
            List of result dicts sorted by descending relevance_score.
        """
        context_note_ids = context_note_ids or []

        # Semantic search — fetch extra candidates so we have room after merging
        semantic_results = self._vector_search.search(query, limit * 2)

        # Build a dict keyed by note_id for easy merging
        results_map: dict[str, dict] = {}
        for r in semantic_results:
            note_id = r["note_id"]
            results_map[note_id] = {
                "note_id": note_id,
                "title": r["title"],
                "content": r.get("snippet", ""),
                "relevance_score": r["score"],
                "connection_type": "semantic",
            }

        # Collect graph neighbor IDs from context notes
        graph_neighbor_ids: set[str] = set()
        for ctx_id in context_note_ids:
            neighbors = self._graph_index.get_neighbors(ctx_id, depth=1)
            for linked_ids in neighbors.values():
                graph_neighbor_ids.update(linked_ids)

        # Boost or add graph neighbors
        boost_factor = 1.25
        for neighbor_id in graph_neighbor_ids:
            if neighbor_id in results_map:
                # Appears in both — boost and mark as "both"
                results_map[neighbor_id]["relevance_score"] *= boost_factor
                results_map[neighbor_id]["connection_type"] = "both"
            else:
                # Graph-only neighbor — add with a base score
                note = self._knowledge_store.get_note(neighbor_id)
                if note:
                    results_map[neighbor_id] = {
                        "note_id": neighbor_id,
                        "title": note.title,
                        "content": note.content[:500] if note.content else "",
                        "relevance_score": 0.3,
                        "connection_type": "graph",
                    }

        # Sort by relevance and return top `limit`
        sorted_results = sorted(
            results_map.values(),
            key=lambda x: x["relevance_score"],
            reverse=True,
        )
        return sorted_results[:limit]

    def find_contradictions(self, note_id: str) -> List[dict]:
        """Find notes that may contradict the given note based on metadata mismatches.

        Flags contradictions when:
        - Two notes are semantically similar (>0.8) but have disjoint tag sets
        - Two notes cover the same topic but differ in status

        Args:
            note_id: The reference note to check against.

        Returns:
            List of contradiction dicts.
        """
        note = self._knowledge_store.get_note(note_id)
        if not note:
            return []

        note_status = note.frontmatter.status if note.frontmatter else "draft"
        note_tags = set(note.frontmatter.tags) if note.frontmatter else set()

        # Find semantically similar notes
        query_text = f"{note.title}\n\n{note.content[:500] if note.content else ''}"
        similar = self._vector_search.search(query_text, limit=10)

        contradictions: List[dict] = []
        for result in similar:
            if result["note_id"] == note_id:
                continue

            other_note = self._knowledge_store.get_note(result["note_id"])
            if not other_note:
                continue

            other_status = other_note.frontmatter.status if other_note.frontmatter else "draft"
            other_tags = set(other_note.frontmatter.tags) if other_note.frontmatter else set()
            similarity = result["score"]

            # Status contradiction: same topic (high similarity) but different status
            if similarity > 0.8 and str(note_status) != str(other_status):
                contradictions.append({
                    "note_id": result["note_id"],
                    "title": result["title"],
                    "conflicting_field": "status",
                    "this_value": str(note_status),
                    "other_value": str(other_status),
                    "similarity_score": similarity,
                })

            # Tag contradiction: high similarity but completely disjoint tags
            if similarity > 0.8 and note_tags and other_tags and note_tags.isdisjoint(other_tags):
                contradictions.append({
                    "note_id": result["note_id"],
                    "title": result["title"],
                    "conflicting_field": "tags",
                    "this_value": ", ".join(sorted(note_tags)),
                    "other_value": ", ".join(sorted(other_tags)),
                    "similarity_score": similarity,
                })

        return contradictions

    def extract_from_conversation(
        self,
        messages: List[dict],
        source: str = "conversation",
    ) -> List[dict]:
        """Extract note suggestions from a list of chat messages.

        For each substantive assistant response, creates a draft note suggestion
        with title, content, tags, and status.

        Args:
            messages: List of dicts with 'role' and 'content' keys.
            source: Label for provenance tracking.

        Returns:
            List of note suggestion dicts.
        """
        suggestions: List[dict] = []

        for msg in messages:
            if msg.get("role") != "assistant":
                continue

            content = msg.get("content", "").strip()
            if len(content) < 50:
                continue  # Skip trivial responses

            title = self._extract_title(content)
            tags = self._extract_tags(content)

            suggestions.append({
                "title": title,
                "content": content,
                "tags": tags,
                "status": "draft",
                "source": source,
            })

        return suggestions

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    @staticmethod
    def _extract_title(content: str) -> str:
        """Derive a short title from the first meaningful line of content."""
        for line in content.split("\n"):
            line = line.strip().lstrip("#").strip()
            if line:
                # Truncate long lines
                return line[:80] if len(line) > 80 else line
        return "Untitled Note"

    @staticmethod
    def _extract_tags(content: str) -> List[str]:
        """Extract and normalize hashtag-style tags from content."""
        # Match inline #tags but skip markdown headings (lines starting with #)
        tag_pattern = re.compile(r'(?<!\w)#([\w\-/]+)', re.UNICODE)
        raw_tags: set[str] = set()
        for line in content.split("\n"):
            stripped = line.lstrip()
            # Skip markdown headings
            if stripped.startswith("#") and " " in stripped[:7]:
                match_start = stripped.index(" ")
                remainder = stripped[match_start:]
            else:
                remainder = line
            for m in tag_pattern.finditer(remainder):
                raw_tags.add(normalize_tag(m.group(1)))
        # Return sorted, deduplicated
        return sorted(t for t in raw_tags if t)
