"""Link discovery service for Zettelkasten note linking"""
import logging
import re
import uuid
from typing import List, Optional, Set, Tuple
from datetime import datetime, timezone

from app.models.distillation import (
    LinkMode,
    LinkType,
    ZettelLinkCandidate,
    ZettelType,
)
from app.models.note import Note, NoteUpdate
from app.services.knowledge_store import KnowledgeStore
from app.services.vector_search import VectorSearchService
from app.services.graph_index import GraphIndexService
from app.services.openrouter import OpenRouterService

logger = logging.getLogger(__name__)


# Key term extraction patterns
# Extract nouns, proper nouns, and important technical terms
KEY_TERM_PATTERNS = [
    r'\b[A-Z][a-z]+(?:\s+[A-Z][a-z]+)+\b',  # Proper nouns (multi-word)
    r'\b[A-Z]{2,}\b',  # Acronyms
    r'\b[a-z]{4,}\b',  # Long words (potential concepts)
]


class LinkDiscoveryService:
    """
    Service for discovering and creating links between Zettelkasten notes.

    Supports three modes:
    - AUTOMATIC: Creates all discovered links immediately
    - SUGGESTED: Returns candidates for user approval
    - MANUAL: Empty results (user triggers separately)
    """

    def __init__(
        self,
        knowledge_store: KnowledgeStore,
        vector_search: Optional[VectorSearchService] = None,
        graph_index: Optional[GraphIndexService] = None,
        openrouter_service: Optional[OpenRouterService] = None,
    ):
        self.knowledge_store = knowledge_store
        self.vector_search = vector_search
        self.graph_index = graph_index
        self.openrouter = openrouter_service

    async def discover_links(
        self,
        note: Note,
        mode: LinkMode = LinkMode.AUTOMATIC,
        max_links: int = 10,
    ) -> List[ZettelLinkCandidate]:
        """
        Discover links from note to existing notes.

        Args:
            note: The note to find links for
            mode: AUTOMATIC creates links, SUGGESTED returns candidates, MANUAL returns empty
            max_links: Maximum number of links to discover

        Returns:
            List of link candidates. Empty if mode=MANUAL.
        """
        if mode == LinkMode.MANUAL:
            return []

        candidates = []

        # 1. Semantic similarity links (vector search)
        if self.vector_search:
            semantic_links = await self._find_semantic_links(note, max_links)
            candidates.extend(semantic_links)

        # 2. Keyword/tag-based links
        keyword_links = self._find_keyword_links(note)
        candidates.extend(keyword_links)

        # 3. LLM-based conceptual links (if OpenRouter available)
        if self.openrouter:
            llm_links = await self._find_llm_links(note, max_links=5)
            candidates.extend(llm_links)

        # Deduplicate and score
        candidates = self._deduplicate_links(candidates)
        candidates = sorted(candidates, key=lambda x: x.confidence, reverse=True)[:max_links]

        # If AUTOMATIC mode, apply links immediately
        if mode == LinkMode.AUTOMATIC and candidates:
            await self._apply_links(note, candidates)

        return candidates

    async def _find_semantic_links(
        self,
        note: Note,
        max_links: int
    ) -> List[ZettelLinkCandidate]:
        """Find links using vector similarity."""
        links = []

        # Build query from title and content
        query_parts = [note.title]

        # Add first few summary lines if available
        summary_match = re.search(r'## TL;DR\s*\n(.+?)(?=##|\n\n|$)', note.content, re.DOTALL)
        if summary_match:
            summary_text = summary_match.group(1).strip()[:500]
            query_parts.append(summary_text)

        # Add first paragraph
        first_para = re.search(r'^#\s+.+?\n\n(.+?)(?=##|\n\n|$)', note.content, re.DOTALL)
        if first_para:
            para_text = first_para.group(1).strip()[:500]
            query_parts.append(para_text)

        query = " ".join(query_parts)

        try:
            results = self.vector_search.search(query, limit=max_links * 2)
        except Exception as e:
            logger.error(f"Vector search failed: {e}")
            return []

        for r in results:
            note_id = r.get('note_id')
            if note_id == note.id:
                continue

            target = self.knowledge_store.get_note(note_id)
            if not target:
                continue

            # Skip evidence notes (containers)
            if target.frontmatter.status == "evidence":
                continue

            score = r.get('score', 0)
            if score >= 0.70:  # Similarity threshold
                link_type = self._infer_link_type_from_similarity(note, target, score)
                links.append(ZettelLinkCandidate(
                    target_id=note_id,
                    target_title=target.title,
                    link_type=link_type,
                    confidence=score,
                    reason=f"Semantic similarity: {score:.2f}"
                ))

        return sorted(links, key=lambda x: x.confidence, reverse=True)[:max_links]

    def _find_keyword_links(self, note: Note) -> List[ZettelLinkCandidate]:
        """Find links based on shared keywords and tags."""
        links = []

        # Extract key terms from note
        note_terms = self._extract_key_terms(note.content)
        note_tags = set(note.frontmatter.tags)

        if not note_terms and not note_tags:
            return []

        # Search for notes with matching terms/tags
        all_notes = self.knowledge_store.list_notes()

        for other in all_notes:
            if other.id == note.id:
                continue

            # Skip evidence notes (containers)
            if other.frontmatter.status == "evidence":
                continue

            other_terms = self._extract_key_terms(other.content)
            other_tags = set(other.frontmatter.tags)

            # Calculate overlap
            term_overlap = len(note_terms & other_terms)
            tag_overlap = len(note_tags & other_tags)

            # Require significant overlap
            if term_overlap >= 2 or (tag_overlap >= 1 and term_overlap >= 1):
                confidence = min(0.9, (term_overlap * 0.15) + (tag_overlap * 0.25))
                links.append(ZettelLinkCandidate(
                    target_id=other.id,
                    target_title=other.title,
                    link_type=LinkType.RELATED,
                    confidence=confidence,
                    reason=f"Shared {term_overlap} terms, {tag_overlap} tags"
                ))

        return sorted(links, key=lambda x: x.confidence, reverse=True)[:5]

    async def _find_llm_links(
        self,
        note: Note,
        max_links: int = 5
    ) -> List[ZettelLinkCandidate]:
        """Use LLM to find conceptual links."""
        if not self.openrouter:
            return []

        # Get sample of existing notes for context
        existing_notes = self.knowledge_store.list_notes()[:25]

        # Filter out the current note and evidence notes
        context_notes = [
            n for n in existing_notes
            if n.id != note.id and n.frontmatter.status != "evidence"
        ][:15]

        if not context_notes:
            return []

        # Build note summary
        note_summary = f"TITLE: {note.title}\n"
        summary_match = re.search(r'## TL;DR\s*\n(.+?)(?=##|\n\n|$)', note.content, re.DOTALL)
        if summary_match:
            note_summary += f"SUMMARY: {summary_match.group(1).strip()}\n"

        # Build context of existing notes
        context = "\n".join(
            f"- {n.title}: {re.sub(r'^#+\s+', '', n.content.split('\\n')[0])[:100]}..."
            for n in context_notes
        )

        prompt = f"""Given this new Zettelkasten note:

{note_summary}

And these existing notes in the knowledge base:
{context}

Identify 2-4 existing notes that should link to this new note. For each:
- Return the EXACT title as shown
- Specify relationship type: related, expands, supports, contradicts, questions, answers, example
- Explain why in ONE short sentence

Respond ONLY as a JSON array:
[{{"title": "Exact Note Title", "type": "related", "reason": "why"}}]"""

        try:
            response = await self.openrouter.complete(
                model_id="anthropic/claude-3.5-haiku",
                messages=[{"role": "user", "content": prompt}],
                temperature=0.2,
                max_tokens=800
            )

            return self._parse_llm_links(response, context_notes)
        except Exception as e:
            logger.error(f"LLM link discovery failed: {e}")
            return []

    def _parse_llm_links(
        self,
        response: str,
        existing_notes: List[Note]
    ) -> List[ZettelLinkCandidate]:
        """Parse LLM response into ZettelLinkCandidate objects."""
        import json

        # Extract JSON from response
        json_match = re.search(r'\[.*\]', response, re.DOTALL)
        if not json_match:
            return []

        try:
            data = json.loads(json_match.group(0))
        except json.JSONDecodeError:
            return []

        # Build title->note mapping
        note_map = {n.title: n for n in existing_notes}

        links = []
        for item in data:
            title = item.get("title", "").strip()
            if not title or title not in note_map:
                continue

            target = note_map[title]

            try:
                link_type = LinkType(item.get("type", "related"))
            except ValueError:
                link_type = LinkType.RELATED

            links.append(ZettelLinkCandidate(
                target_id=target.id,
                target_title=title,
                link_type=link_type,
                confidence=0.75,  # Medium-high confidence for LLM suggestions
                reason=item.get("reason", "LLM suggested")[:100]
            ))

        return links

    def _extract_key_terms(self, content: str) -> Set[str]:
        """Extract key terms from note content."""
        # Remove code blocks and inline code
        clean = re.sub(r'```[\s\S]*?```', '', content)
        clean = re.sub(r'`[^`]+`', '', clean)

        # Remove wikilinks
        clean = re.sub(r'\[\[.+?\]\]', '', clean)

        terms = set()

        # Extract using patterns
        for pattern in KEY_TERM_PATTERNS:
            matches = re.findall(pattern, clean)
            for match in matches:
                # Normalize and add
                term = match.lower().strip()
                if len(term) >= 3 and term not in {
                    'this', 'that', 'with', 'from', 'have', 'been',
                    'will', 'would', 'should', 'could', 'might'
                }:
                    terms.add(term)

        return terms

    def _infer_link_type_from_similarity(
        self,
        source: Note,
        target: Note,
        score: float
    ) -> LinkType:
        """Infer link type based on note types and similarity."""
        # High similarity suggests RELATED or EXPANDS
        if score >= 0.85:
            return LinkType.EXPANDS
        return LinkType.RELATED

    def _deduplicate_links(self, links: List[ZettelLinkCandidate]) -> List[ZettelLinkCandidate]:
        """Remove duplicate links (same target, keep highest confidence)."""
        seen = {}
        for link in links:
            if link.target_id not in seen or link.confidence > seen[link.target_id].confidence:
                seen[link.target_id] = link
        return list(seen.values())

    async def _apply_links(
        self,
        note: Note,
        links: List[ZettelLinkCandidate]
    ) -> int:
        """Apply links to note content and create bidirectional links."""
        applied_count = 0

        for link in links:
            target = self.knowledge_store.get_note(link.target_id)
            if not target:
                continue

            # Add forward link (note -> target)
            if self._add_wikilink_to_note(note, target, link.link_type):
                applied_count += 1

                # Add reverse link (target -> note)
                reverse_type = self._get_reverse_link_type(link.link_type)
                self._add_wikilink_to_note(target, note, reverse_type)

        # Rebuild graph index after adding links
        if applied_count > 0 and self.graph_index:
            try:
                self.graph_index.build_index()
            except Exception as e:
                logger.error(f"Failed to rebuild graph index: {e}")

        return applied_count

    def _add_wikilink_to_note(
        self,
        note: Note,
        target: Note,
        link_type: LinkType
    ) -> bool:
        """Add a wikilink to note content in appropriate section."""
        # Check if link already exists
        if f"[[{target.title}]]" in note.content:
            return False

        link = f"- [[{target.title}]] ({link_type.value})"

        # Find or create "Related Concepts" section
        if "## Related Concepts" in note.content:
            new_content = note.content.replace(
                "## Related Concepts",
                f"## Related Concepts\n{link}"
            )
        elif "## See Also" in note.content:
            new_content = note.content.replace(
                "## See Also",
                f"## See Also\n{link}"
            )
        else:
            # Add section before Sources or at end
            if "## Sources" in note.content:
                new_content = note.content.replace(
                    "## Sources",
                    f"## Related Concepts\n{link}\n\n## Sources"
                )
            else:
                new_content = note.content.rstrip() + f"\n\n## Related Concepts\n{link}"

        try:
            update = NoteUpdate(content=new_content)
            self.knowledge_store.update_note(note.id, update)
            return True
        except Exception as e:
            logger.error(f"Failed to update note {note.id} with link: {e}")
            return False

    def _get_reverse_link_type(self, link_type: LinkType) -> LinkType:
        """Get the reverse link type for bidirectional linking."""
        reverse_map = {
            LinkType.SUPPORTS: LinkType.RELATED,
            LinkType.CONTRADICTS: LinkType.CONTRADICTS,
            LinkType.EXPANDS: LinkType.RELATED,
            LinkType.QUESTIONS: LinkType.ANSWERS,
            LinkType.ANSWERS: LinkType.QUESTIONS,
            LinkType.EXAMPLE: LinkType.RELATED,
            LinkType.PART_OF: LinkType.RELATED,
            LinkType.RELATED: LinkType.RELATED,
        }
        return reverse_map.get(link_type, LinkType.RELATED)

    async def create_bidirectional_links(
        self,
        source_id: str,
        target_id: str,
        link_type: LinkType = LinkType.RELATED
    ) -> bool:
        """
        Create bidirectional links between two notes.

        Args:
            source_id: Source note ID
            target_id: Target note ID
            link_type: Type of link from source to target

        Returns:
            True if links were created successfully
        """
        source = self.knowledge_store.get_note(source_id)
        target = self.knowledge_store.get_note(target_id)

        if not source or not target:
            logger.warning(f"Cannot create link: source={source_id}, target={target_id}")
            return False

        # Add forward link (source -> target)
        source_updated = self._add_wikilink_to_note(source, target, link_type)

        # Add reverse link (target -> source)
        reverse_type = self._get_reverse_link_type(link_type)
        target_updated = self._add_wikilink_to_note(target, source, reverse_type)

        # Rebuild graph index
        if (source_updated or target_updated) and self.graph_index:
            try:
                self.graph_index.build_index()
            except Exception as e:
                logger.error(f"Failed to rebuild graph index: {e}")

        return source_updated or target_updated

    async def create_cross_links_between_atomics(
        self,
        note_ids: List[str]
    ) -> int:
        """
        Create cross-links between a group of newly created atomic notes.

        This helps connect related concepts that were extracted from the same source.

        Args:
            note_ids: List of atomic note IDs to cross-link

        Returns:
            Number of links created
        """
        if not note_ids or len(note_ids) < 2:
            return 0

        links_created = 0

        # Get all notes
        notes = []
        for note_id in note_ids:
            note = self.knowledge_store.get_note(note_id)
            if note:
                notes.append(note)

        if not notes:
            return 0

        # For each pair, find potential connections
        for i, note_a in enumerate(notes):
            for note_b in notes[i + 1:]:
                # Check for tag overlap
                tags_a = set(note_a.frontmatter.tags)
                tags_b = set(note_b.frontmatter.tags)
                tag_overlap = tags_a & tags_b

                # Check for term overlap in titles
                title_a = set(note_a.title.lower().split())
                title_b = set(note_b.title.lower().split())
                title_overlap = title_a & title_b

                if tag_overlap or title_overlap:
                    # Create a bidirectional link
                    if await self.create_bidirectional_links(note_a.id, note_b.id):
                        links_created += 1

        return links_created
