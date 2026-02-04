"""Knowledge store service for Markdown note CRUD operations"""
import logging
import re
import frontmatter
from pathlib import Path
from typing import List, Optional, Union
from datetime import datetime, timezone

from app.models.note import (
    Note, NoteCreate, NoteUpdate, NoteListItem, NoteFrontmatter,
    TypedProperty, PropertyType, ContentType, NoteType
)
from app.config import get_settings

logger = logging.getLogger(__name__)
settings = get_settings()

# Wikilink pattern: [[Note Title]] or [[Note Title|Display Text]]
WIKILINK_PATTERN = re.compile(r'\[\[([^\]|]+)(?:\|[^\]]+)?\]\]')

# Wikilink pattern with heading/block anchors: [[Note#Heading]] or [[Note#^block-id]]
WIKILINK_WITH_ANCHOR_PATTERN = re.compile(
    r'\[\[([^\]|#]+)(?:#([^\]|]+))?(?:\|[^\]]+)?\]\]'
)

# Heading pattern for extracting headings from content
HEADING_PATTERN = re.compile(r'^(#{1,6})\s+(.+)$', re.MULTILINE)

# Block ID pattern for extracting block references
BLOCK_ID_PATTERN = re.compile(r'\^([a-zA-Z0-9-]+)\s*$', re.MULTILINE)


class KnowledgeStore:
    """Service for managing Markdown notes with YAML frontmatter"""
    
    def __init__(self, vault_path: Optional[str] = None):
        """Initialize knowledge store with vault path"""
        self.vault_path = Path(vault_path or settings.vault_path)
        self.vault_path.mkdir(parents=True, exist_ok=True)
    
    def _get_note_path(self, note_id: str) -> Path:
        """Get the file path for a note ID with path traversal protection"""
        # Sanitize note_id to prevent path traversal
        sanitized_id = re.sub(r'[^\w\s-]', '', note_id).strip().replace(' ', '_')
        
        # Construct path and resolve to absolute path
        note_path = (self.vault_path / f"{sanitized_id}.md").resolve()
        
        # Ensure the resolved path is within vault_path
        try:
            note_path.relative_to(self.vault_path.resolve())
        except ValueError:
            logger.warning(f"Path traversal attempt detected: {note_id}")
            raise ValueError(f"Invalid note ID: {note_id}")
        
        return note_path
    
    def _generate_note_id(self, title: str) -> str:
        """Generate a note ID from title"""
        # Replace spaces with underscores, remove special characters
        return re.sub(r'[^\w\s-]', '', title).strip().replace(' ', '_')
    
    def _extract_wikilinks(self, content: str) -> List[str]:
        """Extract wikilinks from content"""
        matches = WIKILINK_PATTERN.findall(content)
        return matches
    
    def _infer_note_type(self, title: str, stored_type: Optional[str] = None) -> NoteType:
        """
        Infer note type from title or stored frontmatter value.
        
        Priority:
        1. Explicit stored type in frontmatter
        2. Title prefix inference (Atomic:, Hub:)
        3. Default to GENERAL
        """
        # If explicitly stored, use that
        if stored_type:
            try:
                return NoteType(stored_type)
            except ValueError:
                pass
        
        # Infer from title prefix for backward compatibility
        if title.startswith('Atomic:') or title.startswith('Atomic '):
            return NoteType.ATOMIC
        elif title.startswith('Hub:') or title.startswith('Hub '):
            return NoteType.HUB
        elif title.startswith('Canvas:') or title.startswith('Evidence:'):
            return NoteType.CONTAINER
        
        return NoteType.GENERAL
    
    def extract_wikilinks(self, content: str) -> List[str]:
        """
        Extract wikilink targets from content.

        Args:
            content: Markdown content to parse

        Returns:
            List of wikilink target titles (preserves duplicates)
        """
        return [match.group(1) for match in WIKILINK_PATTERN.finditer(content)]

    def extract_wikilinks_with_anchors(self, content: str) -> List[dict]:
        """
        Extract wikilinks with optional heading/block anchors.

        Returns list of dicts with 'target' (note title) and 'anchor' (heading/block-id)
        """
        results = []
        for match in WIKILINK_WITH_ANCHOR_PATTERN.finditer(content):
            target = match.group(1)
            anchor = match.group(2) if match.group(2) else None
            results.append({
                'target': target,
                'anchor': anchor,
                'is_block': anchor and anchor.startswith('^') if anchor else False
            })
        return results
    
    def extract_headings(self, content: str) -> List[dict]:
        """
        Extract all headings from markdown content.
        
        Returns list of dicts with 'level', 'text', and 'slug' (URL-friendly ID)
        """
        headings = []
        for match in HEADING_PATTERN.finditer(content):
            level = len(match.group(1))  # Number of # characters
            text = match.group(2).strip()
            # Create URL-friendly slug
            slug = re.sub(r'[^\w\s-]', '', text.lower()).strip().replace(' ', '-')
            headings.append({
                'level': level,
                'text': text,
                'slug': slug
            })
        return headings
    
    def extract_block_ids(self, content: str) -> List[str]:
        """Extract all block IDs (^block-id) from content"""
        return BLOCK_ID_PATTERN.findall(content)
    
    def update_links_on_rename(self, old_title: str, new_title: str) -> int:
        """
        Update all wikilinks when a note is renamed.
        
        Finds all notes that link to old_title and updates them to new_title.
        Returns the number of notes updated.
        """
        updated_count = 0
        
        # Pattern to match [[Old Title]] or [[Old Title|Display]]
        old_link_pattern = re.compile(
            r'\[\[' + re.escape(old_title) + r'(\|[^\]]+)?\]\]'
        )
        
        for md_file in self.vault_path.glob("*.md"):
            try:
                post = frontmatter.load(md_file)
                original_content = post.content
                
                # Replace old links with new
                def replace_link(match):
                    display = match.group(1) if match.group(1) else ''
                    return f'[[{new_title}{display}]]'
                
                new_content = old_link_pattern.sub(replace_link, original_content)
                
                if new_content != original_content:
                    post.content = new_content
                    post['modified'] = datetime.now(timezone.utc)
                    
                    with open(md_file, 'w', encoding='utf-8') as f:
                        f.write(frontmatter.dumps(post))
                    
                    updated_count += 1
                    logger.info(f"Updated links in {md_file.name}: '{old_title}' -> '{new_title}'")
                    
            except Exception as e:
                logger.error(f"Error updating links in {md_file}: {e}")
                continue
        
        return updated_count
    
    def list_notes(self) -> List[NoteListItem]:
        """List all notes with metadata"""
        notes = []
        for md_file in self.vault_path.glob("*.md"):
            try:
                post = frontmatter.load(md_file)
                note_id = md_file.stem
                
                # Extract wikilinks for link count and outgoing links
                wikilinks = self._extract_wikilinks(post.content)
                
                # Infer note_type from title or frontmatter
                note_type = self._infer_note_type(
                    post.get('title', note_id),
                    post.get('note_type')
                )
                
                notes.append(NoteListItem(
                    id=note_id,
                    title=post.get('title', note_id),
                    status=post.get('status', 'draft'),
                    tags=post.get('tags', []),
                    created=post.get('created'),
                    modified=post.get('modified'),
                    link_count=len(wikilinks),
                    note_type=note_type,
                    outgoing_links=wikilinks,
                    source=post.get('source'),
                    container_of=post.get('container_of', [])
                ))
            except Exception as e:
                logger.error(f"Error loading note {md_file}: {e}")
                continue
        
        return notes
    
    def get_note(self, note_id: str) -> Optional[Note]:
        """Get a specific note by ID"""
        note_path = self._get_note_path(note_id)
        if not note_path.exists():
            return None
        
        post = frontmatter.load(note_path)
        
        # Extract wikilinks
        outgoing_links = self._extract_wikilinks(post.content)
        
        # Load properties from frontmatter
        properties = {}
        raw_properties = post.get('properties', {})
        if isinstance(raw_properties, dict):
            for prop_name, prop_data in raw_properties.items():
                try:
                    if isinstance(prop_data, dict):
                        properties[prop_name] = TypedProperty(**prop_data)
                except Exception as e:
                    logger.warning(f"Failed to load property '{prop_name}' from note {note_id}: {e}")
        
        # Infer note_type from title or frontmatter
        note_type = self._infer_note_type(
            post.get('title', note_id),
            post.get('note_type')
        )
        
        # Build frontmatter
        frontmatter_data = NoteFrontmatter(
            title=post.get('title', note_id),
            created=post.get('created'),
            modified=post.get('modified'),
            tags=post.get('tags', []),
            status=post.get('status', 'draft'),
            aliases=post.get('aliases', []),
            content_type=ContentType(post.get('content_type', 'general')),
            note_type=note_type,
            properties=properties
        )
        
        return Note(
            id=note_id,
            title=frontmatter_data.title,
            content=post.content,
            frontmatter=frontmatter_data,
            outgoing_links=outgoing_links,
            backlinks=[]  # Will be populated by graph index
        )
    
    def create_note(self, note_data: Union[NoteCreate, dict]) -> Note:
        """Create a new note

        Args:
            note_data: Note data as NoteCreate model or dict
        """
        # Accept both dict and NoteCreate for flexibility
        if isinstance(note_data, dict):
            note_data = NoteCreate(**note_data)

        note_id = self._generate_note_id(note_data.title)
        note_path = self._get_note_path(note_id)
        
        if note_path.exists():
            raise FileExistsError(f"Note {note_id} already exists")
        
        # Create frontmatter
        now = datetime.now(timezone.utc)
        frontmatter_data = {
            'title': note_data.title,
            'created': now,
            'modified': now,
            'tags': note_data.tags,
            'status': note_data.status,
            'content_type': note_data.content_type.value,
            'note_type': note_data.note_type.value,
            'properties': {name: prop.model_dump(mode='json') for name, prop in note_data.properties.items()}
        }
        
        # Write note file
        post = frontmatter.Post(note_data.content, **frontmatter_data)
        with open(note_path, 'w', encoding='utf-8') as f:
            f.write(frontmatter.dumps(post))
        
        return self.get_note(note_id)
    
    def update_note(self, note_id: str, note_data: Union[NoteUpdate, dict]) -> Optional[Note]:
        """Update an existing note

        Args:
            note_id: ID of the note to update
            note_data: Update data as NoteUpdate model or dict
        """
        note_path = self._get_note_path(note_id)
        if not note_path.exists():
            return None

        # Accept both dict and NoteUpdate for flexibility
        if isinstance(note_data, dict):
            note_data = NoteUpdate(**note_data)
        
        # Load existing note
        post = frontmatter.load(note_path)
        old_title = post.get('title', note_id)
        
        # Update fields if provided
        if note_data.title is not None:
            post['title'] = note_data.title
        if note_data.content is not None:
            post.content = note_data.content
        if note_data.tags is not None:
            post['tags'] = note_data.tags
        if note_data.status is not None:
            post['status'] = note_data.status
        if note_data.content_type is not None:
            post['content_type'] = note_data.content_type.value
        if note_data.properties is not None:
            post['properties'] = {name: prop.model_dump(mode='json') for name, prop in note_data.properties.items()}
        
        # Update modified timestamp
        post['modified'] = datetime.now(timezone.utc)
        
        # Write updated note
        with open(note_path, 'w', encoding='utf-8') as f:
            f.write(frontmatter.dumps(post))
        
        # If title changed, update links in other notes
        if note_data.title is not None and note_data.title != old_title:
            updated_count = self.update_links_on_rename(old_title, note_data.title)
            logger.info(f"Updated {updated_count} notes with new link: '{old_title}' -> '{note_data.title}'")
        
        return self.get_note(note_id)
    
    def delete_note(self, note_id: str) -> bool:
        """Delete a note"""
        note_path = self._get_note_path(note_id)
        if not note_path.exists():
            return False
        
        note_path.unlink()
        return True
    
    def get_all_content(self) -> List[dict]:
        """Get all notes for bulk indexing"""
        notes = []
        for md_file in self.vault_path.glob("*.md"):
            try:
                post = frontmatter.load(md_file)
                note_id = md_file.stem
                notes.append({
                    'id': note_id,
                    'title': post.get('title', note_id),
                    'content': post.content,
                    'tags': post.get('tags', [])
                })
            except Exception as e:
                logger.error(f"Error loading note {md_file}: {e}")
                continue
        
        return notes
