"""Import service for LLM conversations"""
import asyncio
import json
import logging
import uuid
import os
import re
from pathlib import Path
from typing import List, Optional, Dict, Any, Callable
from datetime import datetime, timezone
from difflib import SequenceMatcher

from app.services.parsers import PARSERS
from app.models.import_models import (
    ParsedConversation,
    ImportJob,
    ImportDecision,
    ImportSummary,
    ImportErrorDetail,
    PreviewResult,
    DuplicateCheck,
    ConversationQuality,
    SummarizationSettings
)
from app.models.note import Note, NoteCreate, NoteUpdate
from app.services.knowledge_store import KnowledgeStore
from app.services.vector_search import VectorSearchService
from app.services.graph_index import GraphIndexService
from app.services.distillation import DistillationService
from app.services.openrouter import OpenRouterService
from app.config import get_settings

logger = logging.getLogger(__name__)
settings = get_settings()


class ImportService:
    """Service for importing LLM conversations into Grafyn"""
    
    # Default model for summarization if none specified
    DEFAULT_SUMMARY_MODEL = "anthropic/claude-3.5-sonnet"
    
    def __init__(
        self,
        knowledge_store: KnowledgeStore,
        vector_search: Optional[VectorSearchService] = None,
        graph_index: Optional[GraphIndexService] = None,
        distillation_service: Optional[DistillationService] = None,
        openrouter_service: Optional[OpenRouterService] = None,
        temp_dir: Optional[str] = None
    ):
        self.knowledge_store = knowledge_store
        self.vector_search = vector_search
        self.graph_index = graph_index
        self.distillation_service = distillation_service
        self.openrouter_service = openrouter_service
        
        # Use configured temp directory or default
        if temp_dir is None:
            temp_dir = os.path.join(settings.data_path, 'import', 'temp')
        
        self.temp_dir = Path(temp_dir)
        self.temp_dir.mkdir(parents=True, exist_ok=True)
        
        # In-memory job storage
        self.jobs: Dict[str, ImportJob] = {}
        # Track created note IDs per job for revert functionality
        self.job_created_notes: Dict[str, List[str]] = {}
    
    async def upload_file(self, file_content: bytes, file_name: str) -> ImportJob:
        """
        Upload file and create import job.
        
        Args:
            file_content: Raw file bytes
            file_name: Name of uploaded file
            
        Returns:
            Created import job
        """
        job_id = str(uuid.uuid4())
        
        # Save to temp directory
        file_path = self.temp_dir / f"{job_id}_{file_name}"
        with open(file_path, 'wb') as f:
            f.write(file_content)
        
        # Create job
        job = ImportJob(
            id=job_id,
            status="uploaded",
            file_path=str(file_path),
            file_name=file_name,
            created_at=datetime.now(timezone.utc),
            updated_at=datetime.now(timezone.utc)
        )
        
        self.jobs[job_id] = job
        logger.info(f"Created import job {job_id} for file {file_name}")
        
        return job
    
    async def parse_file(self, job_id: str) -> ImportJob:
        """
        Parse uploaded file using appropriate parser.
        
        Args:
            job_id: Import job ID
            
        Returns:
            Updated import job with parsed conversations
            
        Raises:
            ValueError: If no parser matches the file format
        """
        job = self.jobs.get(job_id)
        if not job:
            raise ValueError(f"Job not found: {job_id}")
        
        # Update status
        job.status = "parsing"
        job.updated_at = datetime.now(timezone.utc)
        
        # Detect platform and parse
        parser = self._detect_parser(job.file_path)
        if not parser:
            error = ImportErrorDetail(
                type="parse_error",
                message="No compatible parser found for this file format",
                severity="error",
                context={'file_name': job.file_name}
            )
            job.errors.append(error)
            job.status = "failed"
            return job
        
        job.platform = parser.platform
        
        # Parse conversations
        try:
            conversations = await parser.parse(job.file_path)
            job.parsed_conversations = conversations
            job.total_conversations = len(conversations)
            job.status = "parsed"
            job.updated_at = datetime.now(timezone.utc)
            
            logger.info(f"Parsed {len(conversations)} conversations for job {job_id}")
        except Exception as e:
            error = ImportErrorDetail(
                type="parse_error",
                message=f"Failed to parse file: {str(e)}",
                severity="error",
                context={'file_name': job.file_name}
            )
            job.errors.append(error)
            job.status = "failed"
            logger.error(f"Parse failed for job {job_id}: {e}")
        
        return job
    
    async def get_preview(self, job_id: str) -> PreviewResult:
        """
        Get preview of parsed conversations.
        
        Args:
            job_id: Import job ID
            
        Returns:
            Preview result with conversations and stats
            
        Raises:
            ValueError: If job not found or not parsed
        """
        job = self.jobs.get(job_id)
        if not job:
            raise ValueError(f"Job not found: {job_id}")
        
        if job.status not in ("parsed", "reviewing"):
            raise ValueError(f"Job not ready for preview: {job.status}")
        
        if not job.parsed_conversations:
            raise ValueError("No conversations parsed")
        
        # Find duplicates for each conversation
        duplicates_found = 0
        for conv in job.parsed_conversations:
            dupes = await self._find_duplicates(conv)
            conv.duplicate_candidates = dupes
            duplicates_found += len(dupes)
        
        # Estimate notes to create
        estimated_notes = job.total_conversations  # At least container notes
        
        # Calculate estimated atomics (rough estimate: ~3 atomics per container)
        estimated_notes += job.total_conversations * 3
        
        return PreviewResult(
            job_id=job_id,
            total_conversations=job.total_conversations,
            conversations=job.parsed_conversations,
            platform=job.platform,
            estimated_notes_to_create=estimated_notes,
            duplicates_found=duplicates_found
        )
    
    async def apply_import(
        self,
        job_id: str,
        decisions: List[ImportDecision],
        progress_callback: Optional[Callable[[str], None]] = None
    ) -> ImportSummary:
        """
        Apply import with user decisions.
        
        Args:
            job_id: Import job ID
            decisions: User's import decisions
            progress_callback: Optional progress callback
            
        Returns:
            Import summary with results
        """
        job = self.jobs.get(job_id)
        if not job:
            raise ValueError(f"Job not found: {job_id}")
        
        # Update status
        job.status = "applying"
        job.updated_at = datetime.now(timezone.utc)
        job.decisions = decisions
        
        # Build decision lookup
        decision_map = {d.conversation_id: d for d in decisions}
        
        # Track stats
        imported = 0
        skipped = 0
        merged = 0
        failed = 0
        container_notes = 0
        atomic_notes = 0
        
        created_ids: List[str] = []
        errors: List[ImportErrorDetail] = []
        
        if progress_callback:
            progress_callback(f"Processing {len(decisions)} conversations...")

        # Process each decision — graph rebuild is deferred to one batch call
        try:
            for i, decision in enumerate(decisions):
                if progress_callback:
                    progress_callback(f"Processing {i + 1}/{len(decisions)}...")

                logger.debug("Processing decision %d: conversation_id=%s, action=%s", i+1, decision.conversation_id, decision.action)

                logger.debug("Job has parsed_conversations: %s", job.parsed_conversations is not None)
                if job.parsed_conversations:
                    logger.debug("parsed_conversations count: %d", len(job.parsed_conversations))
                    logger.debug("parsed_conversations IDs: %s", [c.id for c in job.parsed_conversations])

                conv = self._find_conversation(job, decision.conversation_id)
                logger.debug("_find_conversation result: %s", conv is not None)
                if not conv:
                    logger.warning("Conversation not found: %s", decision.conversation_id)
                    skipped += 1
                    continue

                logger.debug("Found conversation: %s", conv.title)

                try:
                    if decision.action == "skip":
                        skipped += 1
                        continue

                    elif decision.action == "accept":
                        # Create container note
                        logger.info(f"Creating container note for: {conv.title}")
                        container_id = await self._create_container_note(conv)
                        logger.info(f"Container note result: {container_id}")
                        if container_id:
                            imported += 1
                            container_notes += 1
                            created_ids.append(container_id)

                            # Distill if requested (defer_graph_rebuild since we batch at end)
                            if decision.distill_option != "container_only":
                                atomics = await self._distill_container(
                                    container_id,
                                    conv,
                                    decision
                                )
                                atomic_notes += len(atomics)
                                created_ids.extend(atomics)

                    elif decision.action == "merge":
                        # Merge with existing note
                        if decision.target_note_id:
                            success = await self._merge_conversation(
                                decision.target_note_id,
                                conv
                            )
                            if success:
                                merged += 1
                                created_ids.append(decision.target_note_id)
                            else:
                                failed += 1
                                errors.append(ImportErrorDetail(
                                    type="system",
                                    message=f"Failed to merge conversation {conv.id}",
                                    severity="error",
                                    conversation_id=conv.id
                                ))

                except Exception as e:
                    failed += 1
                    errors.append(ImportErrorDetail(
                        type="system",
                        message=f"Failed to import conversation {conv.id}: {str(e)}",
                        severity="error",
                        conversation_id=conv.id
                    ))
                    logger.error(f"Import failed for conversation {conv.id}: {e}")
        finally:
            # Single graph rebuild after all notes are created — avoids O(N²)
            # rebuilds that previously happened once per note creation
            if self.graph_index:
                try:
                    logger.info("Rebuilding graph index after import batch")
                    self.graph_index.build_index()
                except Exception as e:
                    logger.error(f"Failed to rebuild graph index after import: {e}")

        # Update job status
        job.status = "completed"
        job.updated_at = datetime.now(timezone.utc)
        
        # Store created note IDs for potential revert
        self.job_created_notes[job_id] = created_ids
        
        if progress_callback:
            progress_callback(f"Import completed: {imported} imported, {skipped} skipped")
        
        return ImportSummary(
            total_conversations=len(decisions),
            imported=imported,
            skipped=skipped,
            merged=merged,
            failed=failed,
            notes_created=len(created_ids),
            container_notes=container_notes,
            atomic_notes=atomic_notes,
            errors=errors
        )
    
    async def revert_import(self, job_id: str, knowledge_store: KnowledgeStore) -> dict:
        """
        Revert an import by deleting all notes created during that import.
        
        Args:
            job_id: Import job ID
            knowledge_store: Knowledge store to delete notes from
            
        Returns:
            Dictionary with count of deleted notes
        """
        created_ids = self.job_created_notes.get(job_id)
        if not created_ids:
            raise ValueError(f"No import to revert for job: {job_id}")
        
        deleted_count = 0
        failed_count = 0
        
        for note_id in created_ids:
            try:
                # Delete the note
                success = knowledge_store.delete_note(note_id)
                if success:
                    deleted_count += 1
                    
                    # Remove from vector search if available
                    if self.vector_search:
                        try:
                            self.vector_search.delete_from_index(note_id)
                        except Exception:
                            pass  # Ignore vector search errors
                else:
                    failed_count += 1
            except Exception as e:
                logger.error(f"Failed to delete note {note_id}: {e}")
                failed_count += 1
        
        # Rebuild graph index if available
        if self.graph_index:
            try:
                self.graph_index.build_index()
            except Exception as e:
                logger.warning(f"Failed to rebuild graph index: {e}")
        
        # Clear the tracking for this job
        del self.job_created_notes[job_id]
        
        logger.info(f"Reverted import job {job_id}: deleted {deleted_count} notes, failed {failed_count}")
        
        return {
            "job_id": job_id,
            "deleted": deleted_count,
            "failed": failed_count,
            "message": f"Reverted import: deleted {deleted_count} notes"
        }
    
    def cancel_job(self, job_id: str) -> bool:
        """
        Cancel and cleanup import job.
        
        Args:
            job_id: Import job ID
            
        Returns:
            True if job was cancelled
        """
        job = self.jobs.get(job_id)
        if not job:
            return False
        
        # Delete uploaded file
        try:
            os.unlink(job.file_path)
        except Exception:
            pass
        
        # Remove from jobs
        del self.jobs[job_id]
        
        logger.info(f"Cancelled import job {job_id}")
        return True
    
    # =========================================================================
    # LLM-BASED QUALITY ASSESSMENT AND KNOWLEDGE LINKING
    # =========================================================================
    
    async def assess_conversations_quality(
        self,
        job_id: str,
        summarization_settings: Optional[SummarizationSettings] = None,
        progress_callback: Optional[Callable[[str], None]] = None
    ) -> ImportJob:
        """
        Assess quality of all parsed conversations using LLM.
        
        Args:
            job_id: Import job ID
            summarization_settings: User's LLM preferences
            progress_callback: Optional progress callback
            
        Returns:
            Updated job with quality assessments
        """
        job = self.jobs.get(job_id)
        if not job:
            raise ValueError(f"Job not found: {job_id}")
        
        if not job.parsed_conversations:
            raise ValueError("No conversations to assess")
        
        if not self.openrouter_service:
            logger.warning("OpenRouter not configured, skipping quality assessment")
            return job
        
        # Use default settings if none provided
        if not summarization_settings:
            summarization_settings = SummarizationSettings(
                model_id=self.DEFAULT_SUMMARY_MODEL,
                detail_level="detailed",
                max_tokens=4096
            )
        
        total = len(job.parsed_conversations)
        
        for i, conv in enumerate(job.parsed_conversations):
            if progress_callback:
                progress_callback(f"Assessing conversation {i + 1}/{total}...")
            
            try:
                quality = await self._assess_conversation_quality(
                    conv,
                    summarization_settings
                )
                conv.quality = quality
                
                # Find suggested links
                if quality.suggested_action != "skip":
                    suggested_links = await self._find_knowledge_overlaps(conv)
                    conv.suggested_links = suggested_links
                    
            except Exception as e:
                logger.error(f"Failed to assess conversation {conv.id}: {e}")
                # Set a default quality if assessment fails
                conv.quality = ConversationQuality(
                    relevance_score=0.5,
                    informativeness=0.5,
                    suggested_action="import_only"
                )
        
        job.updated_at = datetime.now(timezone.utc)
        return job
    
    async def _assess_conversation_quality(
        self,
        conversation: ParsedConversation,
        settings: SummarizationSettings
    ) -> ConversationQuality:
        """
        Use LLM to evaluate conversation quality and generate detailed summary.
        
        Args:
            conversation: Parsed conversation to assess
            settings: User's LLM preferences
            
        Returns:
            Quality assessment with detailed summary
        """
        if not self.openrouter_service:
            return ConversationQuality(
                relevance_score=0.5,
                informativeness=0.5,
                suggested_action="import_only"
            )
        
        # Get existing knowledge context (sample of note titles/tags)
        knowledge_context = self._get_knowledge_context_sample()
        
        # Build conversation excerpt (truncated for prompt)
        conversation_excerpt = self._build_conversation_excerpt(conversation, max_chars=8000)
        
        # Build detailed prompt based on detail level
        detail_instructions = {
            "brief": "Provide a 2-3 sentence summary.",
            "standard": "Provide a summary with 3-5 key points.",
            "detailed": """Provide an in-depth summary that includes:
- All key concepts with full explanations
- Important context and nuance from the discussion
- Actionable insights and conclusions reached
- Open questions or areas that need further exploration
- Any decisions made or recommendations given"""
        }
        
        prompt = f"""Analyze this AI conversation and provide a quality assessment.

## Your Existing Knowledge Base Topics
{knowledge_context}

## Conversation to Assess
Title: {conversation.title}
Platform: {conversation.platform}
Messages: {conversation.metadata.message_count}

{conversation_excerpt}

## Instructions
Assess this conversation and provide:
1. **relevance_score** (0.0-1.0): How relevant is this to the existing knowledge base topics?
2. **informativeness** (0.0-1.0): How much extractable, valuable information does this contain?
3. **suggested_action**: One of:
   - "import_and_distill": High value, should be imported and broken into atomic notes
   - "import_only": Moderate value, worth keeping as reference
   - "skip": Low value, not worth importing
4. **skip_reason**: If suggesting skip, explain why
5. **key_topics**: List of 3-7 main topics/concepts discussed
6. **detailed_summary**: {detail_instructions.get(settings.detail_level, detail_instructions["detailed"])}

Respond in this exact JSON format:
```json
{{
  "relevance_score": 0.8,
  "informativeness": 0.7,
  "suggested_action": "import_and_distill",
  "skip_reason": null,
  "key_topics": ["topic1", "topic2", "topic3"],
  "detailed_summary": "Your detailed summary here..."
}}
```"""

        messages = [
            {"role": "system", "content": "You are a knowledge management assistant that analyzes conversations for their value and relevance."},
            {"role": "user", "content": prompt}
        ]
        
        try:
            response = await self.openrouter_service.complete(
                model_id=settings.model_id,
                messages=messages,
                temperature=settings.temperature,
                max_tokens=settings.max_tokens
            )
            
            # Parse JSON response
            quality = self._parse_quality_response(response)
            return quality
            
        except Exception as e:
            logger.error(f"LLM quality assessment failed: {e}")
            return ConversationQuality(
                relevance_score=0.5,
                informativeness=0.5,
                suggested_action="import_only"
            )
    
    def _get_knowledge_context_sample(self, max_notes: int = 30) -> str:
        """Get a sample of existing knowledge for context."""
        notes = self.knowledge_store.list_notes()
        
        if not notes:
            return "No existing notes in knowledge base."
        
        # Get sample of note titles and tags
        sample_notes = notes[:max_notes]
        context_lines = []
        
        all_tags = set()
        for note in sample_notes:
            context_lines.append(f"- {note.title}")
            all_tags.update(note.tags)
        
        # Add unique tags
        if all_tags:
            context_lines.append(f"\nCommon tags: {', '.join(sorted(all_tags)[:20])}")
        
        return "\n".join(context_lines)
    
    def _build_conversation_excerpt(
        self,
        conversation: ParsedConversation,
        max_chars: int = 8000
    ) -> str:
        """Build a truncated excerpt of the conversation."""
        parts = []
        total_chars = 0
        
        for msg in conversation.messages:
            role = msg.role.upper()
            content = msg.content
            
            # Truncate individual message if too long
            if len(content) > 2000:
                content = content[:2000] + "... [truncated]"
            
            msg_text = f"**{role}**: {content}\n"
            
            if total_chars + len(msg_text) > max_chars:
                parts.append("... [conversation truncated for analysis]")
                break
            
            parts.append(msg_text)
            total_chars += len(msg_text)
        
        return "\n".join(parts)
    
    def _parse_quality_response(self, response: str) -> ConversationQuality:
        """Parse LLM response into ConversationQuality model."""
        try:
            # Extract JSON from response (handle markdown code blocks)
            json_match = re.search(r'```json\s*(.*?)\s*```', response, re.DOTALL)
            if json_match:
                json_str = json_match.group(1)
            else:
                # Try to find raw JSON
                json_match = re.search(r'\{.*\}', response, re.DOTALL)
                if json_match:
                    json_str = json_match.group(0)
                else:
                    raise ValueError("No JSON found in response")
            
            data = json.loads(json_str)
            
            return ConversationQuality(
                relevance_score=max(0.0, min(1.0, float(data.get("relevance_score", 0.5)))),
                informativeness=max(0.0, min(1.0, float(data.get("informativeness", 0.5)))),
                suggested_action=data.get("suggested_action", "import_only"),
                skip_reason=data.get("skip_reason"),
                key_topics=data.get("key_topics", []),
                detailed_summary=data.get("detailed_summary")
            )
            
        except Exception as e:
            logger.warning(f"Failed to parse quality response: {e}")
            return ConversationQuality(
                relevance_score=0.5,
                informativeness=0.5,
                suggested_action="import_only"
            )
    
    async def _find_knowledge_overlaps(
        self,
        conversation: ParsedConversation,
        min_score: float = 0.7,
        max_results: int = 10
    ) -> List[str]:
        """
        Find existing notes that overlap with conversation topics.
        
        Args:
            conversation: Conversation to find overlaps for
            min_score: Minimum similarity score
            max_results: Maximum number of links to suggest
            
        Returns:
            List of note IDs to potentially link to
        """
        if not self.vector_search:
            return []
        
        suggested_links: List[str] = []
        
        # Build search query from conversation
        query_parts = [conversation.title]
        
        # Add key topics if available from quality assessment
        if conversation.quality and conversation.quality.key_topics:
            query_parts.extend(conversation.quality.key_topics)
        
        # Add first assistant response summary
        for msg in conversation.messages:
            if msg.role == "assistant" and msg.content:
                query_parts.append(msg.content[:500])
                break
        
        query = " ".join(query_parts)
        
        try:
            results = self.vector_search.search(query, limit=max_results * 2)
            
            seen_ids = set()
            for r in results:
                note_id = r.get('note_id')
                score = r.get('score', 0)
                
                if not note_id or note_id in seen_ids:
                    continue
                    
                if score >= min_score:
                    # Verify note still exists and is not an evidence note
                    note = self.knowledge_store.get_note(note_id)
                    if note and note.frontmatter.status != "evidence":
                        suggested_links.append(note_id)
                        seen_ids.add(note_id)
                        
                        if len(suggested_links) >= max_results:
                            break
                            
        except Exception as e:
            logger.error(f"Failed to find knowledge overlaps: {e}")
        
        return suggested_links
    
    def _detect_parser(self, file_path: str):
        """Detect appropriate parser for file."""
        for parser in PARSERS:
            try:
                if parser.detect_format(file_path):
                    logger.info(f"Detected {parser.platform} parser for {file_path}")
                    return parser
            except Exception as e:
                logger.warning(f"Parser detection failed for {parser.platform}: {e}")
                continue
        return None
    
    async def _find_duplicates(
        self,
        conversation: ParsedConversation
    ) -> List[DuplicateCheck]:
        """Find duplicate notes using vector search."""
        if not self.vector_search:
            return []
        
        duplicates: List[DuplicateCheck] = []
        
        # Search by title + first message
        query = conversation.title
        if conversation.messages:
            query += " " + conversation.messages[0].content[:200]
        
        results = self.vector_search.search(query, limit=5)
        
        for r in results:
            note_id = r['note_id']
            note = self.knowledge_store.get_note(note_id)
            if not note:
                continue
            
            # Only compare against draft/canonical (not evidence/hubs)
            if note.frontmatter.status not in ("draft", "canonical"):
                continue
            
            # Check title similarity
            title_sim = SequenceMatcher(
                None,
                conversation.title.lower(),
                note.title.lower()
            ).ratio()
            
            score = r.get('score', 0)
            
            if score >= 0.85 and title_sim >= 0.5:
                duplicates.append(DuplicateCheck(
                    note_id=note_id,
                    title=note.title,
                    similarity_score=max(score, title_sim),
                    content_type=note.frontmatter.note_type if hasattr(note.frontmatter, 'note_type') else None,
                    tags=note.frontmatter.tags
                ))
        
        return duplicates
    
    def _find_conversation(
        self,
        job: ImportJob,
        conversation_id: str
    ) -> Optional[ParsedConversation]:
        """Find conversation in job by ID."""
        if not job.parsed_conversations:
            return None
        for conv in job.parsed_conversations:
            if conv.id == conversation_id:
                return conv
        return None
    
    # Messages per chunk for container note structure.  Each chunk gets
    # its own H2 heading so rules-based distillation can extract
    # candidates from H2 sections instead of one giant blob.
    MESSAGES_PER_CHUNK = 8

    async def _create_container_note(
        self,
        conversation: ParsedConversation
    ) -> Optional[str]:
        """Create a container note from conversation.

        Messages are grouped into chunks of ~MESSAGES_PER_CHUNK under
        separate H2 headings (e.g. "## Part 1", "## Part 2") so that
        rules-based distillation can split on H2 and produce meaningful
        candidates. Previously all messages lived under a single
        "## Conversation History" H2, causing rules-based extraction
        to produce only 1 giant candidate.
        """
        # Build metadata table
        metadata_table = f"""| Property | Value |
|---------|-------|
| Platform | {conversation.platform.upper()} |
| Original Date | {conversation.metadata.created_at.strftime('%Y-%m-%d')} |
| Messages | {conversation.metadata.message_count} |
| Models Used | {', '.join(conversation.metadata.model_info)} |
| Source ID | {conversation.id} |"""

        if conversation.metadata.source_url:
            metadata_table += f"\n| Source URL | [{conversation.id}]({conversation.metadata.source_url}) |"

        # Build quality summary section if available
        quality_section = ""
        if conversation.quality and conversation.quality.detailed_summary:
            quality_section = f"""
## Summary
{conversation.quality.detailed_summary}

### Key Topics
{chr(10).join(f"- {t}" for t in conversation.quality.key_topics) if conversation.quality.key_topics else "No key topics identified."}
"""

        # Build related notes section with wikilinks
        related_section = ""
        if conversation.suggested_links:
            wikilinks = []
            for link_id in conversation.suggested_links[:10]:  # Limit to 10 links
                linked_note = self.knowledge_store.get_note(link_id)
                if linked_note:
                    wikilinks.append(f"- [[{linked_note.title}]]")

            if wikilinks:
                related_section = f"""
## Related Notes
{chr(10).join(wikilinks)}
"""

        # Build conversation history as chunked H2 sections
        # Each chunk of ~MESSAGES_PER_CHUNK messages gets its own H2 heading
        # so rules-based distillation can extract per-chunk candidates
        messages = conversation.messages
        chunk_size = self.MESSAGES_PER_CHUNK
        history_sections = []

        if len(messages) <= chunk_size:
            # Small conversation — single section is fine
            parts = []
            for msg in messages:
                role_header = f"### Message {msg.index + 1} - {msg.role.upper()}"
                if msg.timestamp:
                    role_header += f" - {msg.timestamp.strftime('%Y-%m-%d %H:%M')}"
                if msg.model:
                    role_header += f" ({msg.model})"
                parts.append(f"{role_header}\n\n{msg.content}\n")
            history_sections.append(f"## Conversation History\n{chr(10).join(parts)}")
        else:
            # Large conversation — split into numbered parts
            for chunk_idx in range(0, len(messages), chunk_size):
                chunk = messages[chunk_idx:chunk_idx + chunk_size]
                chunk_num = chunk_idx // chunk_size + 1

                # Use first assistant message snippet as topic hint
                topic_hint = ""
                for msg in chunk:
                    if msg.role == "assistant" and msg.content:
                        hint_text = msg.content[:80].replace('\n', ' ').strip()
                        if len(hint_text) > 60:
                            hint_text = hint_text[:60] + "..."
                        topic_hint = f": {hint_text}"
                        break

                parts = []
                for msg in chunk:
                    role_header = f"### Message {msg.index + 1} - {msg.role.upper()}"
                    if msg.timestamp:
                        role_header += f" - {msg.timestamp.strftime('%Y-%m-%d %H:%M')}"
                    if msg.model:
                        role_header += f" ({msg.model})"
                    parts.append(f"{role_header}\n\n{msg.content}\n")

                history_sections.append(
                    f"## Part {chunk_num}{topic_hint}\n{chr(10).join(parts)}"
                )

        history = "\n".join(history_sections)

        # Build full note content
        content = f"""# Conversation: {conversation.title}
{quality_section}
## Metadata
{metadata_table}
{related_section}
{history}

## Sources
- [Original Conversation]({conversation.metadata.source_url or '#'})
"""
        
        try:
            note_data = NoteCreate(
                title=f"Conversation: {conversation.title}",
                content=content,
                tags=conversation.suggested_tags + ["evidence", conversation.platform],
                status="evidence"
            )
            note = self.knowledge_store.create_note(note_data)
            
            # Add source metadata to frontmatter
            from app.models.note import NoteFrontmatter
            update = NoteUpdate(
                tags=note_data.tags + ["import"],
                custom_fields={
                    'source': 'import',
                    'source_id': f"{conversation.platform}:{conversation.id}",
                    'container_of': []
                }
            )
            self.knowledge_store.update_note(note.id, update)
            
            # Index for vector search
            if self.vector_search and note:
                self.vector_search.index_note(
                    note.id,
                    note.title,
                    note.content
                )

            # Graph rebuild deferred to apply_import() for batch efficiency

            return note.id if note else None
        except Exception as e:
            logger.error(f"Failed to create container note: {e}")
            return None
    
    async def _distill_container(
        self,
        container_id: str,
        conversation: ParsedConversation,
        decision: ImportDecision
    ) -> List[str]:
        """Distill container into atomic notes."""
        if not self.distillation_service:
            return []
        
        # Use existing distillation service
        from app.models.distillation import DistillMode, DistillRequest, ExtractionMethod
        
        if decision.distill_option == "auto_distill":
            # Auto distill using LLM — defer graph rebuild since apply_import batches it
            request = DistillRequest(
                mode=DistillMode.AUTO,
                extraction_method=ExtractionMethod.AUTO
            )
            response = await self.distillation_service.distill(
                container_id, request, defer_graph_rebuild=True
            )
            return response.created_note_ids or []
        
        elif decision.distill_option == "custom" and decision.custom_atoms:
            # Create custom atomics
            created_ids = []
            for atom in decision.custom_atoms:
                note_id = await self._create_atomic_note(container_id, atom)
                if note_id:
                    created_ids.append(note_id)
            return created_ids
        
        return []
    
    async def _create_atomic_note(
        self,
        container_id: str,
        atom_template: Any
    ) -> Optional[str]:
        """Create an atomic note from template."""
        # Build content
        content = f"""# {atom_template.title}

## TL;DR
{chr(10).join(f"- {s}" for s in atom_template.summary) if atom_template.summary else ""}

## Details
<!-- Expand on key points here -->

## Sources
- Container: [[{container_id}]]

## Updates
<!-- Future updates appended here with date headers -->
"""
        
        try:
            note_data = NoteCreate(
                title=atom_template.title,
                content=content,
                tags=atom_template.tags + ["draft"],
                status="draft"
            )
            note = self.knowledge_store.create_note(note_data)
            
            if note and self.vector_search:
                self.vector_search.index_note(note.id, note.title, note.content)

            # Graph rebuild deferred to apply_import() for batch efficiency

            return note.id if note else None
        except Exception as e:
            logger.error(f"Failed to create atomic note: {e}")
            return None

    async def _merge_conversation(
        self,
        target_note_id: str,
        conversation: ParsedConversation
    ) -> bool:
        """Merge conversation into existing note."""
        note = self.knowledge_store.get_note(target_note_id)
        if not note:
            return False
        
        # Add conversation as update
        timestamp = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M")
        
        update_block = f"""
### Update from {conversation.platform.title()} ({timestamp})
*Source ID: {conversation.id}*

{conversation.messages[0].content if conversation.messages else ""}
"""
        
        # Append to content
        new_content = note.content + f"\n\n{update_block}"
        
        # Update tags
        new_tags = list(set(note.frontmatter.tags + conversation.suggested_tags))
        
        try:
            update_data = NoteUpdate(content=new_content, tags=new_tags)
            self.knowledge_store.update_note(target_note_id, update_data)
            return True
        except Exception as e:
            logger.error(f"Failed to merge conversation: {e}")
            return False
