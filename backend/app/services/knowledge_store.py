"""Knowledge store service for Markdown note CRUD operations"""
import re
import frontmatter
from pathlib import Path
from typing import List, Optional
from datetime import datetime

from app.models.note import Note, NoteCreate, NoteUpdate, NoteListItem
from app.config import get_settings

settings = get_settings()

# Wikilink pattern: [[Note Title]] or [[Note Title|Display Text]]
WIKILINK_PATTERN = re.compile(r'\[\[([^\]|]+)(?:\|[^\]]+)?\]\]')


class KnowledgeStore:
    """Service for managing Markdown notes with YAML frontmatter"""
    
    def __init__(self, vault_path: Optional[str] = None):
        """Initialize knowledge store with vault path"""
        self.vault_path = Path(vault_path or settings.vault_path)
        self.vault_path.mkdir(parents=True, exist_ok=True)
    
    def _get_note_path(self, note_id: str) -> Path:
        """Get the file path for a note ID"""
        return self.vault_path / f"{note_id}.md"
    
    def _generate_note_id(self, title: str) -> str:
        """Generate a note ID from title"""
        # Replace spaces with underscores, remove special characters
        return re.sub(r'[^\w\s-]', '', title).strip().replace(' ', '_')
    
    def _extract_wikilinks(self, content: str) -> List[str]:
        """Extract wikilinks from content"""
        matches = WIKILINK_PATTERN.findall(content)
        return matches
    
    def list_notes(self) -> List[NoteListItem]:
        """List all notes with metadata"""
        notes = []
        for md_file in self.vault_path.glob("*.md"):
            try:
                post = frontmatter.load(md_file)
                note_id = md_file.stem
                
                # Extract wikilinks for link count
                wikilinks = self._extract_wikilinks(post.content)
                
                notes.append(NoteListItem(
                    id=note_id,
                    title=post.get('title', note_id),
                    status=post.get('status', 'draft'),
                    tags=post.get('tags', []),
                    created=post.get('created'),
                    modified=post.get('modified'),
                    link_count=len(wikilinks)
                ))
            except Exception as e:
                print(f"Error loading note {md_file}: {e}")
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
        
        # Build frontmatter
        frontmatter_data = NoteFrontmatter(
            title=post.get('title', note_id),
            created=post.get('created'),
            modified=post.get('modified'),
            tags=post.get('tags', []),
            status=post.get('status', 'draft'),
            aliases=post.get('aliases', [])
        )
        
        return Note(
            id=note_id,
            title=frontmatter_data.title,
            content=post.content,
            frontmatter=frontmatter_data,
            outgoing_links=outgoing_links,
            backlinks=[]  # Will be populated by graph index
        )
    
    def create_note(self, note_data: NoteCreate) -> Note:
        """Create a new note"""
        note_id = self._generate_note_id(note_data.title)
        note_path = self._get_note_path(note_id)
        
        if note_path.exists():
            raise FileExistsError(f"Note {note_id} already exists")
        
        # Create frontmatter
        now = datetime.utcnow()
        frontmatter_data = {
            'title': note_data.title,
            'created': now,
            'modified': now,
            'tags': note_data.tags,
            'status': note_data.status
        }
        
        # Write note file
        post = frontmatter.Post(note_data.content, **frontmatter_data)
        with open(note_path, 'w', encoding='utf-8') as f:
            f.write(frontmatter.dumps(post))
        
        return self.get_note(note_id)
    
    def update_note(self, note_id: str, note_data: NoteUpdate) -> Optional[Note]:
        """Update an existing note"""
        note_path = self._get_note_path(note_id)
        if not note_path.exists():
            return None
        
        # Load existing note
        post = frontmatter.load(note_path)
        
        # Update fields if provided
        if note_data.title is not None:
            post['title'] = note_data.title
        if note_data.content is not None:
            post.content = note_data.content
        if note_data.tags is not None:
            post['tags'] = note_data.tags
        if note_data.status is not None:
            post['status'] = note_data.status
        
        # Update modified timestamp
        post['modified'] = datetime.utcnow()
        
        # Write updated note
        with open(note_path, 'w', encoding='utf-8') as f:
            f.write(frontmatter.dumps(post))
        
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
                print(f"Error loading note {md_file}: {e}")
                continue
        
        return notes
