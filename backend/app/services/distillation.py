"""Distillation service for Container → Atomic → Hub workflow"""
import logging
import os
import re
import tempfile
import shutil
import uuid
from pathlib import Path
from typing import List, Optional, Tuple, Callable
from datetime import datetime, timezone
from difflib import SequenceMatcher

import frontmatter

try:
    from filelock import FileLock
    HAS_FILELOCK = True
except ImportError:
    HAS_FILELOCK = False

from app.models.distillation import (
    AtomicNoteCandidate,
    CandidateAction,
    CandidateDecision,
    DistillMode,
    DistillRequest,
    DistillResponse,
    DuplicateMatch,
    ExtractionMethod,
    HubPolicy,
    HubUpdate,
    LinkMode,
    LinkType,
    ZettelLinkCandidate,
    ZettelNoteCandidate,
    ZettelType,
)
from app.models.note import Note, NoteCreate, NoteUpdate
from app.services.knowledge_store import KnowledgeStore
from app.services.vector_search import VectorSearchService
from app.services.graph_index import GraphIndexService
from app.config import get_settings

logger = logging.getLogger(__name__)
settings = get_settings()

# ============================================================================
# TAG UTILITIES
# ============================================================================

# Tags: # followed immediately by letter/number (not space = heading)
INLINE_TAG_PATTERN = re.compile(r'(?<![`#])#([a-zA-Z0-9][a-zA-Z0-9_/\-]*)')


def normalize_tag(tag: str) -> str:
    """Normalize tag: lowercase, strip '#', spaces → hyphens."""
    return tag.lstrip('#').lower().strip().replace(' ', '-')


def parse_inline_tags(content: str) -> List[str]:
    """
    Extract #tags from markdown, ignoring code blocks and inline code.
    Also ignores headings (# followed by space).
    """
    # 1. Remove fenced code blocks (```...```)
    clean = re.sub(r'```[\s\S]*?```', '', content)
    # 2. Remove inline code (`...`)
    clean = re.sub(r'`[^`]+`', '', clean)
    # 3. Find tags (# followed by letter/number, not whitespace)
    tags = INLINE_TAG_PATTERN.findall(clean)
    return list(set(normalize_tag(t) for t in tags))


def merge_tags(yaml_tags: List[str], inline_tags: List[str]) -> List[str]:
    """
    Merge inline tags into YAML tags (merge-only, no removal).
    Deleting a #tag from body does NOT remove it from YAML.
    """
    normalized = set(normalize_tag(t) for t in yaml_tags)
    normalized.update(normalize_tag(t) for t in inline_tags)
    return sorted(normalized)


def normalize_all_tags(tags: List[str]) -> List[str]:
    """Normalize and deduplicate a list of tags."""
    return sorted(set(normalize_tag(t) for t in tags))


# ============================================================================
# PROTECTED SECTION LOGIC (Canvas Export)
# ============================================================================

CANVAS_START = "<!-- SEEDREAM:CANVAS_SNAPSHOT:START -->"
CANVAS_END = "<!-- SEEDREAM:CANVAS_SNAPSHOT:END -->"


def update_protected_section(existing_content: str, new_snapshot: str) -> str:
    """
    Replace snapshot content OR safely append if markers are missing.
    This ensures existing canvas-export notes don't lose user edits.
    """
    if CANVAS_START in existing_content:
        # Replace content between markers
        pattern = f"{re.escape(CANVAS_START)}[\\s\\S]*?{re.escape(CANVAS_END)}"
        replacement = f"{CANVAS_START}\n{new_snapshot}\n{CANVAS_END}"
        return re.sub(pattern, replacement, existing_content)
    else:
        # SAFE MIGRATION: Append new section, don't overwrite user content
        return (
            f"{existing_content}\n\n"
            f"---\n\n"
            f"## Canvas Snapshot (auto)\n\n"
            f"{CANVAS_START}\n{new_snapshot}\n{CANVAS_END}"
        )


def extract_protected_section(content: str) -> Optional[str]:
    """Extract content between protected markers, if present."""
    if CANVAS_START not in content:
        return None
    pattern = f"{re.escape(CANVAS_START)}([\\s\\S]*?){re.escape(CANVAS_END)}"
    match = re.search(pattern, content)
    return match.group(1).strip() if match else None


# ============================================================================
# ATOMIC FILE WRITES
# ============================================================================


def atomic_write_file(path: Path, content: str) -> None:
    """
    Write to temp file, then rename (atomic on POSIX/Windows).
    Uses file locking if filelock is available.
    """
    path = Path(path)
    
    if HAS_FILELOCK:
        lock_path = path.with_suffix('.lock')
        lock = FileLock(lock_path, timeout=10)
    else:
        lock = None
    
    try:
        if lock:
            lock.acquire()
        
        fd, tmp_path = tempfile.mkstemp(
            dir=path.parent, 
            suffix='.tmp',
            prefix=path.stem
        )
        try:
            with os.fdopen(fd, 'w', encoding='utf-8') as f:
                f.write(content)
            shutil.move(tmp_path, path)
        except Exception:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)
            raise
    finally:
        if lock:
            lock.release()


# ============================================================================
# ZETTELKASTEN TEMPLATES AND PROMPTS
# ============================================================================

ZETTELKASTEN_EXTRACTION_PROMPT = """You are a Zettelkasten knowledge extraction expert. Extract atomic notes from this conversation following these principles:

## Zettelkasten Principles
1. **Atomicity**: Each note = ONE clear, self-contained idea
2. **Linkability**: Notes should naturally connect to other concepts
3. **Provenance**: Always track the source of each idea

## Note Types to Extract

### Concept Notes
- Definitions and explanations of ideas
- Format: Clear definition + characteristics + examples
- Title format: Use descriptive terms that reference the concept

### Claim Notes
- Assertions, hypotheses, or arguments
- Format: Claim statement + reasoning + implications
- Title format: "Claim: ..." when making assertions

### Evidence Notes
- Data, research, examples supporting claims
- Format: Evidence description + source + what it supports
- Title format: "Evidence: ..." for supporting data

### Question Notes
- Open questions driving inquiry
- Format: Question + context + potential approaches
- Title format: "Question: ..." for open questions

### Fleche (Structure) Notes
- Connections between multiple ideas
- Format: Argument chain or conceptual framework
- Title format: "Structure: ..." for connecting ideas

## Output Format

For each atomic note, output:

```json
{{
  "title": "Descriptive note title",
  "zettel_type": "concept|claim|evidence|question|fleche",
  "content": "Full markdown content with proper structure",
  "summary": ["bullet", "points", "for", "TL;DR"],
  "key_claims": ["any", "claims", "made"],
  "open_questions": ["questions", "raised"],
  "recommended_tags": ["tag1", "tag2"],
  "suggested_links": [
    {{"target": "Related concept name", "type": "related|expands|supports", "reason": "why they connect"}}
  ]
}}
```

## Conversation to Extract
---
{conversation_content}
---

Extract 5-15 atomic notes depending on content density. Focus on quality over quantity.

IMPORTANT: Respond ONLY with a valid JSON array. No markdown, no code blocks, just the raw JSON array."""


def render_zettel_note(
    candidate: ZettelNoteCandidate,
    container: Note
) -> str:
    """
    Render a Zettelkasten note with proper structure based on its type.

    Args:
        candidate: Zettel note candidate with all metadata
        container: Source container note for provenance

    Returns:
        Full markdown content for the Zettel note
    """
    templates = {
        ZettelType.CONCEPT: _render_concept_note,
        ZettelType.CLAIM: _render_claim_note,
        ZettelType.EVIDENCE: _render_evidence_note,
        ZettelType.QUESTION: _render_question_note,
        ZettelType.FLECHE: _render_fleche_note,
        ZettelType.FLEETING: _render_generic_note,
    }

    renderer = templates.get(candidate.zettel_type, _render_generic_note)
    return renderer(candidate, container)


def _render_concept_note(candidate: ZettelNoteCandidate, container: Note) -> str:
    """Render a Zettelkasten concept note."""
    summary_bullets = "\n".join(f"- {s}" for s in candidate.summary)

    # Build suggested links section
    links = ""
    if candidate.suggested_links:
        links = "\n".join(
            f"- [[{l.target_title}]] ({l.link_type.value})"
            for l in candidate.suggested_links[:5]
        )
    else:
        links = "<!-- Add related concepts -->"

    # Build questions section
    questions = ""
    if candidate.open_questions:
        questions = "\n".join(f"- {q}" for q in candidate.open_questions)

    # Generate Zettel ID
    zettel_id = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")

    return f"""# {candidate.title}

## Definition
<!-- Core definition of this concept -->

{summary_bullets}

## Key Characteristics
<!-- Essential properties and features -->

## Related Concepts
{links}

## Examples
<!-- Concrete instances or applications -->

## Questions
<!-- Open questions about this concept -->
{questions}

## Sources
- [[{container.title}]]

---
*Zettel ID: {zettel_id}*
*Type: {candidate.zettel_type.value}*
*Tags: {', '.join(candidate.recommended_tags) if candidate.recommended_tags else 'none'}*
"""


def _render_claim_note(candidate: ZettelNoteCandidate, container: Note) -> str:
    """Render a Zettelkasten claim note."""
    claims = "\n".join(f"- {c}" for s in candidate.key_claims)

    # Build suggested links section
    links = ""
    if candidate.suggested_links:
        links = "\n".join(
            f"- [[{l.target_title}]] ({l.link_type.value})"
            for l in candidate.suggested_links[:5]
        )
    else:
        links = "<!-- Add supporting evidence -->"

    zettel_id = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")

    return f"""# {candidate.title}

## Claim Statement
<!-- The core assertion -->

{claims}

## Reasoning
<!-- Why this claim is being made -->

## Evidence
<!-- Supporting data or reasoning -->
{links}

## Implications
<!-- What follows if this is true -->

## Counterarguments
<!-- Potential objections or alternative views -->

## Status
- [ ] Unverified
- [ ] Supported
- [ ] Established
- [ ] Refuted

## Sources
- [[{container.title}]]

---
*Zettel ID: {zettel_id}*
*Type: {candidate.zettel_type.value}*
"""


def _render_evidence_note(candidate: ZettelNoteCandidate, container: Note) -> str:
    """Render a Zettelkasten evidence note."""
    # Build supported claims section
    supported = ""
    if candidate.suggested_links:
        supported = "\n".join(
            f"- [[{l.target_title}]]"
            for l in candidate.suggested_links[:5]
        )

    zettel_id = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")

    return f"""# {candidate.title}

## Evidence
<!-- The data, research, or example -->

{chr(10).join(f"- {s}" for s in candidate.summary)}

## Source
- Original: [[{container.title}]]

## Supports
<!-- Claims or concepts this evidence supports -->
{supported}

## Reliability
<!-- Assessment of evidence quality -->

## Context
<!-- When, where, how this evidence was gathered -->

---
*Zettel ID: {zettel_id}*
*Type: {candidate.zettel_type.value}*
"""


def _render_question_note(candidate: ZettelNoteCandidate, container: Note) -> str:
    """Render a Zettelkasten question note."""
    # Build approaches section
    approaches = "\n".join(f"- {s}" for s in candidate.summary) if candidate.summary else ""

    # Build related questions
    related = ""
    if candidate.suggested_links:
        related = "\n".join(
            f"- [[{l.target_title}]]"
            for l in candidate.suggested_links[:5]
        )

    zettel_id = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")

    return f"""# {candidate.title}

## Question
<!-- The core question -->

## Context
<!-- Why this question matters -->

{candidate.summary[0] if candidate.summary else ''}

## Approaches
<!-- Potential ways to explore or answer -->
{approaches}

## Related Questions
{related}

## Partial Answers
<!-- Any leads or partial insights -->

## Sources
- [[{container.title}]]

---
*Zettel ID: {zettel_id}*
*Type: {candidate.zettel_type.value}*
"""


def _render_fleche_note(candidate: ZettelNoteCandidate, container: Note) -> str:
    """Render a Zettelkasten fleche (structure) note."""
    # Build connected concepts
    links = ""
    if candidate.suggested_links:
        links = "\n".join(
            f"- [[{l.target_title}]] ({l.link_type.value})"
            for l in candidate.suggested_links[:10]
        )

    # Build argument structure
    structure = "\n".join(f"- {s}" for s in candidate.summary) if candidate.summary else ""

    zettel_id = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")

    return f"""# {candidate.title}

## Argument Structure
<!-- How these ideas connect -->

{structure}

## Connected Concepts
{links if links else "<!-- Link related concepts -->"}

## Key Insights
<!-- What emerges from these connections -->

## Sources
- [[{container.title}]]

---
*Zettel ID: {zettel_id}*
*Type: {candidate.zettel_type.value}*
"""


def _render_generic_note(candidate: ZettelNoteCandidate, container: Note) -> str:
    """Render a generic Zettelkasten note (fallback)."""
    summary_bullets = "\n".join(f"- {s}" for s in candidate.summary)
    zettel_id = datetime.now(timezone.utc).strftime("%Y%m%d%H%M%S")

    return f"""# {candidate.title}

## TL;DR
{summary_bullets}

## Details
<!-- Expand on the key points here -->

## Sources
- [[{container.title}]]

---
*Zettel ID: {zettel_id}*
*Type: {candidate.zettel_type.value}*
*Tags: {', '.join(candidate.recommended_tags) if candidate.recommended_tags else 'none'}*
"""


# ============================================================================
# DISTILLATION SERVICE
# ============================================================================


class DistillationService:
    """Service for distilling container notes into atomic notes."""
    
    def __init__(
        self,
        knowledge_store: KnowledgeStore,
        vector_search: Optional[VectorSearchService] = None,
        graph_index: Optional[GraphIndexService] = None,
        openrouter_service = None
    ):
        self.knowledge_store = knowledge_store
        self.vector_search = vector_search
        self.graph_index = graph_index
        self.openrouter_service = openrouter_service
    
    async def distill(
        self,
        note_id: str,
        request: DistillRequest,
        progress_callback: Optional[Callable[[str], None]] = None
    ) -> DistillResponse:
        """
        Main distillation entry point.

        SUGGEST mode: Extract candidates, find duplicates, return for review.
        APPLY mode: Process user decisions, create/update notes, update container.
        AUTO mode: Summarize with LLM, auto-create drafts (no review needed).

        Args:
            note_id: ID of the container note to distill
            request: Distillation request with mode and options
            progress_callback: Optional callback for progress updates
        """
        note = self.knowledge_store.get_note(note_id)
        if not note:
            return DistillResponse(message=f"Note not found: {note_id}")

        if request.mode == DistillMode.SUGGEST:
            return self._suggest(note, request)
        elif request.mode == DistillMode.AUTO:
            return await self._auto(note, request, progress_callback)
        else:
            return self._apply(note, request)

    async def distill_zettelkasten(
        self,
        note_id: str,
        link_mode: LinkMode = LinkMode.AUTOMATIC,
        progress_callback: Optional[Callable[[str], None]] = None
    ) -> DistillResponse:
        """
        Extract Zettelkasten-formatted atomic notes from container note.

        This is the main entry point for Zettelkasten distillation, which:
        1. Extracts atomic notes using LLM with proper Zettelkasten types
        2. Creates notes with type-specific templates (concept, claim, evidence, question, fleche)
        3. Discovers and applies links between notes based on link_mode

        Args:
            note_id: ID of the container note to distill
            link_mode: AUTOMATIC (apply all links), SUGGESTED (show candidates), MANUAL (none)
            progress_callback: Optional callback for progress updates

        Returns:
            DistillResponse with created note IDs and status
        """
        from app.services.link_discovery import LinkDiscoveryService

        note = self.knowledge_store.get_note(note_id)
        if not note:
            return DistillResponse(message=f"Note not found: {note_id}")

        if progress_callback:
            progress_callback("Extracting atomic notes with AI...")

        # Step 1: Extract atomic notes using LLM
        candidates = await self._extract_zettel_notes(note)

        if not candidates:
            return DistillResponse(
                message="No atomic notes extracted from this conversation",
                extraction_method_used="zettelkasten_llm",
                status="completed"
            )

        if progress_callback:
            progress_callback(f"Extracted {len(candidates)} atomic notes")

        # Step 2: Initialize link discovery service
        link_service = LinkDiscoveryService(
            self.knowledge_store,
            self.vector_search,
            self.graph_index,
            self.openrouter_service
        )

        # Step 3: Create atomic notes and discover links
        created_ids = []

        for i, candidate in enumerate(candidates):
            if progress_callback:
                progress_callback(f"Creating note {i + 1}/{len(candidates)}...")

            # Create the atomic note with Zettelkasten formatting
            note_id = self._create_zettel_note(candidate, note)
            if note_id:
                created_ids.append(note_id)

                # Discover links for the newly created note
                if progress_callback:
                    progress_callback(f"Finding links for note {i + 1}...")

                try:
                    created_note = self.knowledge_store.get_note(note_id)
                    if created_note:
                        links = await link_service.discover_links(
                            created_note,
                            mode=link_mode,
                            max_links=8
                        )

                        # Store discovered links (for SUGGESTED mode)
                        candidate.suggested_links = links
                except Exception as e:
                    logger.error(f"Link discovery failed for note {note_id}: {e}")

        # Step 4: Create cross-links between new atomic notes
        if created_ids and link_mode == LinkMode.AUTOMATIC:
            if progress_callback:
                progress_callback("Creating cross-links between new notes...")

            try:
                cross_links = await link_service.create_cross_links_between_atomics(created_ids)
                if progress_callback:
                    progress_callback(f"Created {cross_links} cross-links")
            except Exception as e:
                logger.error(f"Cross-link creation failed: {e}")

        # Step 5: Update container with extracted atomic notes
        container_updated = False
        if created_ids:
            container_updated = self._update_container(note.id, created_ids)

        if progress_callback:
            progress_callback(f"Completed: Created {len(created_ids)} Zettelkasten atomic notes")

        return DistillResponse(
            candidates=candidates,
            created_note_ids=created_ids,
            updated_note_ids=[],
            hub_updates=[],
            container_updated=container_updated,
            message=f"Created {len(created_ids)} Zettelkasten atomic notes",
            extraction_method_used="zettelkasten_llm",
            status="completed"
        )

    async def _extract_zettel_notes(
        self,
        container: Note
    ) -> List[ZettelNoteCandidate]:
        """
        Use LLM to extract Zettelkasten-formatted atomic notes from container.

        Args:
            container: Container note to extract from

        Returns:
            List of ZettelNoteCandidate objects
        """
        if not self.openrouter_service:
            logger.warning("OpenRouter not available, using rule-based extraction")
            return self._extract_zettel_rules(container)

        # Prepare conversation content (truncate if needed)
        content = container.content[:12000]  # Limit for LLM context

        prompt = ZETTELKASTEN_EXTRACTION_PROMPT.format(conversation_content=content)

        messages = [
            {
                "role": "system",
                "content": "You are a Zettelkasten knowledge extraction expert. Always respond with valid JSON arrays."
            },
            {"role": "user", "content": prompt}
        ]

        try:
            response = await self.openrouter_service.complete(
                model_id="anthropic/claude-3.5-sonnet",
                messages=messages,
                temperature=0.3,
                max_tokens=4096
            )

            return self._parse_zettel_response(response)
        except Exception as e:
            logger.error(f"Zettelkasten LLM extraction failed: {e}")
            return self._extract_zettel_rules(container)

    def _parse_zettel_response(self, response: str) -> List[ZettelNoteCandidate]:
        """
        Parse LLM response into ZettelNoteCandidate objects.

        Args:
            response: Raw LLM response string

        Returns:
            List of ZettelNoteCandidate objects
        """
        import json

        # Try to extract JSON array from response
        json_match = re.search(r'\[.*\]', response, re.DOTALL)
        if not json_match:
            logger.warning("No JSON array found in LLM response")
            return []

        try:
            data = json.loads(json_match.group(0))
        except json.JSONDecodeError as e:
            logger.warning(f"Failed to parse LLM response as JSON: {e}")
            return []

        candidates = []
        for item in data:
            try:
                zettel_type_str = item.get("zettel_type", "concept")
                try:
                    zettel_type = ZettelType(zettel_type_str)
                except ValueError:
                    zettel_type = ZettelType.CONCEPT

                # Parse suggested links
                suggested_links = []
                for link_item in item.get("suggested_links", []):
                    try:
                        link_type = LinkType(link_item.get("type", "related"))
                    except ValueError:
                        link_type = LinkType.RELATED

                    suggested_links.append(ZettelLinkCandidate(
                        target_id="",  # Will be resolved when creating links
                        target_title=link_item.get("target", ""),
                        link_type=link_type,
                        reason=link_item.get("reason", "")
                    ))

                candidates.append(ZettelNoteCandidate(
                    id=str(uuid.uuid4()),
                    title=item.get("title", "Untitled Note"),
                    zettel_type=zettel_type,
                    content=item.get("content", ""),
                    summary=item.get("summary", [])[:6],
                    key_claims=item.get("key_claims", [])[:5],
                    open_questions=item.get("open_questions", [])[:5],
                    recommended_tags=item.get("recommended_tags", [])[:8],
                    suggested_links=suggested_links[:5],
                    confidence=0.8
                ))
            except Exception as e:
                logger.error(f"Failed to parse candidate: {e}")
                continue

        return candidates

    def _extract_zettel_rules(self, container: Note) -> List[ZettelNoteCandidate]:
        """
        Rule-based extraction as fallback when LLM is unavailable.

        Splits by H2 sections and categorizes by heuristics.
        """
        candidates = []
        content = container.content

        # Find H2 sections as potential atomics
        sections = re.split(r'\n## ', content)

        for i, section in enumerate(sections[1:], 1):
            lines = section.strip().split('\n')
            if not lines:
                continue

            title = lines[0].strip()
            body = '\n'.join(lines[1:]).strip()

            # Skip meta sections
            if title.lower() in ('metadata', 'extracted atomic notes', 'sources', 'updates'):
                continue

            # Skip if too short
            if len(body) < 30:
                continue

            # Infer note type from title and content
            zettel_type = self._infer_zettel_type(title, body)

            # Extract summary bullets
            summary = []
            for line in body.split('\n'):
                line = line.strip()
                if line.startswith(('- ', '* ')):
                    summary.append(line[2:])
                    if len(summary) >= 5:
                        break

            # Extract inline tags
            section_tags = parse_inline_tags(body)

            # Inherit tags from container
            container_tags = [
                t for t in container.frontmatter.tags
                if t not in ('chat', 'canvas-export', 'evidence')
            ]

            recommended_tags = list(set(section_tags + container_tags))[:6]

            candidates.append(ZettelNoteCandidate(
                id=str(uuid.uuid4()),
                title=title,
                zettel_type=zettel_type,
                content=body[:500],
                summary=summary or [body[:200] + "..." if len(body) > 200 else body],
                key_claims=[],
                open_questions=[],
                recommended_tags=recommended_tags,
                confidence=0.6,
                suggested_links=[],
                source_section=title
            ))

        return candidates

    def _infer_zettel_type(self, title: str, content: str) -> ZettelType:
        """
        Infer Zettelkasten note type from title and content using heuristics.
        """
        title_lower = title.lower()
        content_lower = content.lower()

        # Question indicators
        if any(title_lower.startswith(w) for w in ('what', 'why', 'how', 'when', 'where', 'who', 'which', 'is', 'are')):
            if '?' in title or '?' in content[:200]:
                return ZettelType.QUESTION

        # Claim indicators
        claim_indicators = ['claim', 'assert', 'argue', 'believe', 'thesis', 'hypothesis']
        if any(indicator in title_lower or indicator in content_lower[:300] for indicator in claim_indicators):
            return ZettelType.CLAIM

        # Evidence indicators
        evidence_indicators = ['evidence', 'study', 'research', 'data', 'results', 'finding', 'shown']
        if any(indicator in title_lower or indicator in content_lower[:300] for indicator in evidence_indicators):
            return ZettelType.EVIDENCE

        # Structure/connection indicators
        structure_indicators = ['relationship', 'connection', 'between', 'versus', 'vs', 'compare']
        if any(indicator in title_lower for indicator in structure_indicators):
            return ZettelType.FLECHE

        # Default to concept
        return ZettelType.CONCEPT

    def _create_zettel_note(
        self,
        candidate: ZettelNoteCandidate,
        container: Note
    ) -> Optional[str]:
        """
        Create a Zettelkasten atomic note with proper type-specific structure.

        Args:
            candidate: Zettel note candidate with all metadata
            container: Source container note for provenance

        Returns:
            Created note ID or None if failed
        """
        # Generate note content using type-specific template
        content = render_zettel_note(candidate, container)

        # Prepare tags
        tags = normalize_all_tags(candidate.recommended_tags + ["draft"])

        # Add Zettel type as tag
        tags.append(f"zettel-{candidate.zettel_type.value}")

        try:
            note_data = NoteCreate(
                title=candidate.title,
                content=content,
                tags=tags,
                status="draft"
            )
            new_note = self.knowledge_store.create_note(note_data)

            if not new_note:
                return None

            # Index for vector search
            if self.vector_search and new_note:
                try:
                    self.vector_search.index_note(
                        new_note.id,
                        new_note.title,
                        new_note.content
                    )
                except Exception as e:
                    logger.error(f"Failed to index note {new_note.id}: {e}")

            # Update knowledge graph
            if self.graph_index:
                try:
                    self.graph_index.build_index()
                except Exception as e:
                    logger.error(f"Failed to update graph index: {e}")

            return new_note.id if new_note else None

        except Exception as e:
            logger.error(f"Failed to create Zettel note: {e}")
            return None

    async def _summarize_with_llm(self, note: Note) -> str:
        """
        Use LLM to generate a structured summary of the container note.
        Returns clean markdown with key insights.
        """
        if not self.openrouter_service:
            logger.warning("No OpenRouter service available for summarization")
            return note.content  # Fallback to original content
        
        prompt = f"""Summarize this note into atomic knowledge units. For each distinct concept or insight, create a section with:
- A clear descriptive title (not "Prompt 1" or generic names)
- 3-5 bullet point summary
- Key claims or insights
- Any open questions

Note content:
---
{note.content}
---

Format your response as markdown with ## headings for each atomic unit."""

        try:
            # Use a fast model for summarization
            response = await self.openrouter_service.chat_completion(
                messages=[{"role": "user", "content": prompt}],
                model="anthropic/claude-3-haiku",  # Fast and good at summarization
                stream=False
            )
            return response.get("content", note.content)
        except Exception as e:
            logger.error(f"LLM summarization failed: {e}")
            return note.content  # Fallback to original
    
    async def _auto(
        self,
        note: Note,
        request: DistillRequest,
        progress_callback: Optional[Callable[[str], None]] = None
    ) -> DistillResponse:
        """
        AUTO mode: Summarize with LLM, then auto-create all atomics as drafts.
        No user review needed - drafts can be reviewed/promoted later.
        
        Args:
            note: Container note to distill
            request: Distillation request with extraction method preference
            progress_callback: Optional callback for progress updates
        """
        # Determine extraction method
        extraction_method = request.extraction_method
        
        # If AUTO, prefer LLM but fallback to rules if not available
        if extraction_method == ExtractionMethod.AUTO:
            if not self.openrouter_service:
                logger.info("LLM service not available, using rule-based extraction")
                extraction_method = ExtractionMethod.RULES
            else:
                extraction_method = ExtractionMethod.LLM
        
        candidates: List[AtomicNoteCandidate] = []
        summary: Optional[str] = None
        extraction_method_used = extraction_method.value
        
        try:
            if extraction_method == ExtractionMethod.LLM:
                # Use LLM summarization
                if progress_callback:
                    progress_callback("Summarizing with LLM...")
                
                summary = await self._summarize_with_llm(note)
                
                if progress_callback:
                    progress_callback("Extracting atomic notes from LLM summary...")
                
                # Extract candidates from LLM summary
                candidates = self._extract_candidates_from_summary(summary, note)
                
                if progress_callback:
                    progress_callback(f"Extracted {len(candidates)} atomic notes from LLM summary")
                
            else:
                # Use rule-based extraction
                if progress_callback:
                    progress_callback("Extracting atomic notes using rules...")
                
                candidates = self._extract_candidates(note)
                
                if progress_callback:
                    progress_callback(f"Extracted {len(candidates)} atomic notes using rules")
        
        except Exception as e:
            logger.error(f"Extraction failed with {extraction_method.value}: {e}")
            
            # Fallback to rule-based extraction if LLM fails
            if extraction_method == ExtractionMethod.LLM:
                logger.info("Falling back to rule-based extraction")
                if progress_callback:
                    progress_callback("LLM extraction failed, falling back to rules...")
                
                try:
                    candidates = self._extract_candidates(note)
                    extraction_method_used = ExtractionMethod.RULES.value
                    
                    if progress_callback:
                        progress_callback(f"Extracted {len(candidates)} atomic notes using rules (fallback)")
                except Exception as fallback_error:
                    logger.error(f"Rule-based extraction also failed: {fallback_error}")
                    return DistillResponse(
                        message=f"Extraction failed: {str(e)}",
                        extraction_method_used=extraction_method_used,
                        status="error"
                    )
            else:
                return DistillResponse(
                    message=f"Extraction failed: {str(e)}",
                    extraction_method_used=extraction_method_used,
                    status="error"
                )
        
        if not candidates:
            return DistillResponse(
                message="No atomic note candidates found in this note",
                extraction_method_used=extraction_method_used,
                status="completed"
            )
        
        # Auto-create all candidates as drafts
        created_ids: List[str] = []
        hub_updates: List[HubUpdate] = []
        
        if progress_callback:
            progress_callback(f"Creating {len(candidates)} draft atomic notes...")
        
        for i, candidate in enumerate(candidates):
            # Use the extracted title directly
            title = candidate.title
            tags = normalize_all_tags(candidate.recommended_tags + ["draft"])
            
            new_id = self._create_atomic_note(
                candidate,
                note,  # container for provenance
                title=title,
                tags=tags
            )
            
            if new_id:
                created_ids.append(new_id)
                
                # Auto-create/update hub if suggested
                if candidate.suggested_hub and request.hub_policy == HubPolicy.AUTO:
                    hub_update = self._update_hub(candidate.suggested_hub, new_id)
                    if hub_update:
                        hub_updates.append(hub_update)
            
            # Update progress for each note created
            if progress_callback and i < len(candidates) - 1:
                progress_callback(f"Created {i + 1}/{len(candidates)} draft notes...")
        
        # Update container note with extracted links
        container_updated = False
        if created_ids:
            container_updated = self._update_container(note.id, created_ids)
        
        if progress_callback:
            progress_callback(f"Completed: Created {len(created_ids)} draft atomic notes")
        
        return DistillResponse(
            summary=summary,
            candidates=[],  # Not needed for AUTO mode
            created_note_ids=created_ids,
            updated_note_ids=[],
            hub_updates=hub_updates,
            container_updated=container_updated,
            message=f"Auto-created {len(created_ids)} draft atomic notes using {extraction_method_used}",
            extraction_method_used=extraction_method_used,
            status="completed"
        )
    
    def _suggest(self, note: Note, request: DistillRequest) -> DistillResponse:
        """Extract candidates and find potential duplicates."""
        candidates = self._extract_candidates(note)
        
        # Find duplicates for each candidate
        for candidate in candidates:
            dup = self._find_duplicate(candidate, request.min_score)
            if dup:
                candidate.duplicate_match = dup
        
        return DistillResponse(
            candidates=candidates,
            message=f"Found {len(candidates)} potential atomic notes"
        )
    

    def _apply(self, note: Note, request: DistillRequest) -> DistillResponse:
        """Process user decisions and create/update notes."""
        created_ids: List[str] = []
        updated_ids: List[str] = []
        hub_updates: List[HubUpdate] = []
        
        # Build candidate lookup
        candidate_map = {c.id: c for c in request.candidates}
        
        for decision in request.decisions:
            if decision.action == CandidateAction.SKIP:
                continue
            
            candidate = candidate_map.get(decision.candidate_id)
            if not candidate:
                logger.warning(f"Candidate not found: {decision.candidate_id}")
                continue
            
            # Use custom overrides if provided
            title = decision.custom_title or candidate.title
            tags = decision.custom_tags or candidate.recommended_tags
            tags = normalize_all_tags(tags)
            
            if decision.action == CandidateAction.CREATE:
                new_id = self._create_atomic_note(
                    candidate, 
                    note,  # container for provenance
                    title=title,
                    tags=tags
                )
                if new_id:
                    created_ids.append(new_id)
            
            elif decision.action == CandidateAction.APPEND:
                if candidate.duplicate_match:
                    success = self._append_to_note(
                        candidate.duplicate_match.note_id,
                        candidate,
                        note  # container for provenance
                    )
                    if success:
                        updated_ids.append(candidate.duplicate_match.note_id)
            
            # Handle hub updates
            hub_title = decision.hub_title or candidate.suggested_hub
            if hub_title and request.hub_policy == HubPolicy.AUTO:
                atomic_id = created_ids[-1] if created_ids else (
                    updated_ids[-1] if updated_ids else None
                )
                if atomic_id:
                    hub_update = self._update_hub(hub_title, atomic_id)
                    if hub_update:
                        hub_updates.append(hub_update)
        
        # Update container note with extracted links
        container_updated = False
        if created_ids or updated_ids:
            all_atomic_ids = created_ids + updated_ids
            container_updated = self._update_container(note.id, all_atomic_ids)
        
        return DistillResponse(
            candidates=[],
            created_note_ids=created_ids,
            updated_note_ids=updated_ids,
            hub_updates=hub_updates,
            container_updated=container_updated,
            message=f"Created {len(created_ids)}, updated {len(updated_ids)} notes"
        )
    
    def _extract_candidates(self, note: Note) -> List[AtomicNoteCandidate]:
        """
        Extract atomic note candidates from container content.
        This is a rule-based extraction - can be enhanced with LLM later.
        """
        candidates = []
        content = note.content
        
        # Find H2 sections as potential atomics
        sections = re.split(r'\n## ', content)
        
        for i, section in enumerate(sections[1:], 1):  # Skip content before first ##
            lines = section.strip().split('\n')
            if not lines:
                continue
            
            title = lines[0].strip()
            body = '\n'.join(lines[1:]).strip()
            
            # Skip meta sections
            if title.lower() in ('metadata', 'extracted atomic notes', 'sources', 'updates'):
                continue
            
            # Skip if too short
            if len(body) < 50:
                continue
            
            # Extract summary bullets
            summary = []
            for line in body.split('\n'):
                line = line.strip()
                if line.startswith('- ') or line.startswith('* '):
                    summary.append(line[2:])
                    if len(summary) >= 5:
                        break
            
            # Extract inline tags from section
            section_tags = parse_inline_tags(body)
            
            # Inherit some tags from container
            container_tags = [t for t in note.frontmatter.tags 
                           if t not in ('chat', 'canvas-export', 'evidence')]
            
            recommended_tags = list(set(section_tags + container_tags))[:5]
            
            candidate = AtomicNoteCandidate(
                id=str(uuid.uuid4()),
                title=f"Atomic: {title}",
                summary=summary or [body[:200] + "..."] if len(body) > 200 else [body],
                key_claims=[],
                open_questions=[],
                recommended_tags=recommended_tags,
                confidence=0.7,
                suggested_hub=self._suggest_hub(title, recommended_tags),
                source_section=title
            )
            candidates.append(candidate)
        
        return candidates
    
    def _extract_candidates_from_summary(
        self,
        summary: str,
        container: Note
    ) -> List[AtomicNoteCandidate]:
        """
        Extract atomic note candidates from LLM-generated summary.
        
        The summary should be formatted with ## headings for each atomic unit.
        """
        candidates = []
        
        # Find H2 sections as potential atomics
        sections = re.split(r'\n## ', summary)
        
        for i, section in enumerate(sections[1:], 1):  # Skip content before first ##
            lines = section.strip().split('\n')
            if not lines:
                continue
            
            title = lines[0].strip()
            body = '\n'.join(lines[1:]).strip()
            
            # Skip meta sections
            if title.lower() in ('metadata', 'extracted atomic notes', 'sources', 'updates'):
                continue
            
            # Skip if too short
            if len(body) < 30:
                continue
            
            # Extract summary bullets
            summary_bullets = []
            key_claims = []
            open_questions = []
            
            current_section = None
            for line in body.split('\n'):
                line = line.strip()
                
                # Check for section headers
                if line.startswith('###'):
                    current_section = line[3:].strip().lower()
                    continue
                
                # Extract bullet points
                if line.startswith('- ') or line.startswith('* '):
                    content = line[2:].strip()
                    
                    if current_section in ('key claims', 'claims'):
                        key_claims.append(content)
                    elif current_section in ('open questions', 'questions'):
                        open_questions.append(content)
                    else:
                        summary_bullets.append(content)
            
            # If no bullets found, treat first paragraph as summary
            if not summary_bullets:
                first_para = body.split('\n\n')[0].replace('\n', ' ').strip()
                if first_para:
                    summary_bullets.append(first_para)
            
            # Extract inline tags from section
            section_tags = parse_inline_tags(body)
            
            # Inherit some tags from container
            container_tags = [t for t in container.frontmatter.tags
                           if t not in ('chat', 'canvas-export', 'evidence')]
            
            recommended_tags = list(set(section_tags + container_tags))[:5]
            
            candidate = AtomicNoteCandidate(
                id=str(uuid.uuid4()),
                title=title,
                summary=summary_bullets[:6] if summary_bullets else [body[:200] + "..."] if len(body) > 200 else [body],
                key_claims=key_claims[:5],
                open_questions=open_questions[:3],
                recommended_tags=recommended_tags,
                confidence=0.85,  # Higher confidence for LLM-extracted
                suggested_hub=self._suggest_hub(title, recommended_tags),
                source_section=f"LLM Summary Section {i}"
            )
            candidates.append(candidate)
        
        return candidates
    
    def _find_duplicate(
        self, 
        candidate: AtomicNoteCandidate, 
        min_score: float
    ) -> Optional[DuplicateMatch]:
        """
        Find duplicate only among draft/canonical notes.
        Excludes evidence and hub notes.
        """
        if not self.vector_search:
            return None
        
        # Search by title + summary
        query = candidate.title + " " + " ".join(candidate.summary)
        results = self.vector_search.search(query, limit=5)
        
        for r in results:
            # Get full note to check status
            note = self.knowledge_store.get_note(r['note_id'])
            if not note:
                continue
            
            # FILTER: only compare against draft/canonical
            status = note.frontmatter.status
            if status not in ("draft", "canonical"):
                continue
            
            # Skip hub notes
            if "hub" in note.frontmatter.tags:
                continue
            
            # Check title similarity
            title_sim = SequenceMatcher(
                None, 
                candidate.title.lower(), 
                note.title.lower()
            ).ratio()
            
            score = r.get('score', 0)
            
            if score >= min_score and title_sim >= 0.5:
                return DuplicateMatch(
                    note_id=r['note_id'],
                    title=note.title,
                    score=score,
                    title_similarity=title_sim,
                    snippet=r.get('snippet', note.content[:200])
                )
        
        return None
    
    def _create_atomic_note(
        self,
        candidate: AtomicNoteCandidate,
        container: Note,
        title: str,
        tags: List[str]
    ) -> Optional[str]:
        """Create a new atomic note with proper structure."""
        # Build content
        summary_list = "\n".join(f"- {s}" for s in candidate.summary)
        
        content = f"""# {title}

## TL;DR
{summary_list}

## Details
<!-- Expand on the key points here -->

## Sources
- [[{container.title}]]

## Updates
<!-- Future updates appended here with date headers -->
"""
        
        try:
            note_data = NoteCreate(
                title=title,
                content=content,
                tags=tags,
                status="draft"
            )
            new_note = self.knowledge_store.create_note(note_data)
            
            # Index for vector search
            if self.vector_search and new_note:
                self.vector_search.index_note(
                    new_note.id, 
                    new_note.title, 
                    new_note.content
                )
            
            # Update knowledge graph
            if self.graph_index and new_note:
                self.graph_index.build_index()
            
            return new_note.id if new_note else None
        except Exception as e:
            logger.error(f"Failed to create atomic note: {e}")
            return None
    
    def _append_to_note(
        self,
        note_id: str,
        candidate: AtomicNoteCandidate,
        container: Note
    ) -> bool:
        """Append update to existing atomic note."""
        note = self.knowledge_store.get_note(note_id)
        if not note:
            return False
        
        timestamp = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M")
        summary_list = "\n".join(f"  - {s}" for s in candidate.summary)
        
        update_block = f"""
### Update ({timestamp})
*From: [[{container.title}]]*

{summary_list}
"""
        
        # Find ## Updates section and append
        if "## Updates" in note.content:
            new_content = note.content.replace(
                "## Updates",
                f"## Updates\n{update_block}"
            )
        else:
            new_content = note.content + f"\n\n## Updates\n{update_block}"
        
        # Merge tags
        existing_tags = note.frontmatter.tags
        new_tags = merge_tags(existing_tags, candidate.recommended_tags)
        
        try:
            update_data = NoteUpdate(content=new_content, tags=new_tags)
            self.knowledge_store.update_note(note_id, update_data)
            return True
        except Exception as e:
            logger.error(f"Failed to update note: {e}")
            return False
    
    def _suggest_hub(self, title: str, tags: List[str]) -> Optional[str]:
        """Suggest a hub based on title and tags."""
        # Simple heuristic: use first significant tag as hub
        for tag in tags:
            if tag not in ('seedream', 'draft'):
                return f"Hub: {tag.replace('-', ' ').title()}"
        return None
    
    def _update_hub(self, hub_title: str, atomic_id: str) -> Optional[HubUpdate]:
        """Get or create hub and add atomic link."""
        # Generate hub ID
        hub_id = hub_title.replace(' ', '_').replace(':', '')
        
        hub = self.knowledge_store.get_note(hub_id)
        action = "updated"
        
        if not hub:
            # Create new hub
            content = f"""# {hub_title}

## Stance / Current Summary
<!-- Brief overview of this topic -->

## Atomic Notes
- [[{atomic_id}]]

## Open Questions
<!-- Unanswered questions -->

## Related Hubs
<!-- Links to adjacent topic hubs -->
"""
            try:
                hub_data = NoteCreate(
                    title=hub_title,
                    content=content,
                    tags=["hub", "seedream"],
                    status="draft"
                )
                hub = self.knowledge_store.create_note(hub_data)
                action = "created"
            except Exception as e:
                logger.error(f"Failed to create hub: {e}")
                return None
        else:
            # Update existing hub - add link if not present
            link = f"[[{atomic_id}]]"
            if link not in hub.content:
                # Find ## Atomic Notes section
                if "## Atomic Notes" in hub.content:
                    new_content = hub.content.replace(
                        "## Atomic Notes",
                        f"## Atomic Notes\n- {link}"
                    )
                else:
                    new_content = hub.content + f"\n\n## Atomic Notes\n- {link}"
                
                try:
                    update_data = NoteUpdate(content=new_content)
                    self.knowledge_store.update_note(hub.id, update_data)
                except Exception as e:
                    logger.error(f"Failed to update hub: {e}")
                    return None
        
        return HubUpdate(
            hub_id=hub_id,
            hub_title=hub_title,
            action=action,
            atomic_ids_added=[atomic_id]
        )
    
    def _update_container(self, container_id: str, atomic_ids: List[str]) -> bool:
        """Update container note with extracted atomic notes list."""
        note = self.knowledge_store.get_note(container_id)
        if not note:
            return False
        
        timestamp = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M")
        links = "\n".join(f"- [[{aid}]]" for aid in atomic_ids)
        
        section_content = f"""
*Last distilled: {timestamp}*

{links}
"""
        
        section_header = "## Extracted Atomic Notes"
        
        if section_header in note.content:
            # Replace section content
            pattern = f"{section_header}[\\s\\S]*?(?=\\n## |$)"
            replacement = f"{section_header}\n{section_content}\n"
            new_content = re.sub(pattern, replacement, note.content)
        else:
            # Append new section
            new_content = note.content + f"\n\n{section_header}\n{section_content}"
        
        try:
            update_data = NoteUpdate(content=new_content)
            self.knowledge_store.update_note(container_id, update_data)
            return True
        except Exception as e:
            logger.error(f"Failed to update container: {e}")
            return False
    
    def normalize_note_tags(self, note_id: str) -> Optional[Note]:
        """Normalize YAML tags and merge inline #tags for a note."""
        note = self.knowledge_store.get_note(note_id)
        if not note:
            return None
        
        # Parse inline tags
        inline_tags = parse_inline_tags(note.content)
        
        # Merge with existing YAML tags
        merged = merge_tags(note.frontmatter.tags, inline_tags)
        
        # Update if changed
        if set(merged) != set(note.frontmatter.tags):
            update_data = NoteUpdate(tags=merged)
            return self.knowledge_store.update_note(note_id, update_data)
        
        return note
