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

from backend.app.models.distillation import (
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
)
from backend.app.models.note import Note, NoteCreate, NoteUpdate
from backend.app.services.knowledge_store import KnowledgeStore
from backend.app.services.vector_search import VectorSearchService
from backend.app.services.graph_index import GraphIndexService
from backend.app.config import get_settings

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
    normalized.update(inline_tags)
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
