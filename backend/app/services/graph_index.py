"""Graph index service for wikilink parsing and backlink tracking"""
from typing import List, Dict, Set, Optional
from collections import deque

from app.services.knowledge_store import KnowledgeStore


class GraphIndexService:
    """Service for managing knowledge graph with wikilinks and backlinks"""
    
    def __init__(self):
        """Initialize graph index"""
        self._outgoing: Dict[str, Set[str]] = {}  # note_id -> linked note IDs
        self._incoming: Dict[str, Set[str]] = {}  # note_id -> notes linking to it
        self._knowledge_store = KnowledgeStore()
        self._build_index()
    
    def _build_index(self):
        """Build the graph index from all notes"""
        # Clear existing index
        self._outgoing.clear()
        self._incoming.clear()
        
        # Get all notes
        notes = self._knowledge_store.list_notes()
        
        # Build adjacency lists
        for note in notes:
            note_id = note.id
            full_note = self._knowledge_store.get_note(note_id)
            
            if full_note:
                # Initialize outgoing links
                self._outgoing[note_id] = set(full_note.outgoing_links)
                
                # Update incoming links for each target
                for linked_id in full_note.outgoing_links:
                    if linked_id not in self._incoming:
                        self._incoming[linked_id] = set()
                    self._incoming[linked_id].add(note_id)
    
    def build_index(self):
        """Rebuild the entire graph index"""
        self._build_index()
    
    def get_outgoing_links(self, note_id: str) -> List[str]:
        """Get all notes that the specified note links to"""
        return list(self._outgoing.get(note_id, set()))
    
    def get_backlinks(self, note_id: str) -> List[str]:
        """Get all notes that link to the specified note"""
        return list(self._incoming.get(note_id, set()))
    
    def get_backlinks_with_context(self, note_id: str) -> List[dict]:
        """Get backlinks with surrounding text context"""
        backlinks = self.get_backlinks(note_id)
        results = []
        
        for source_id in backlinks:
            source_note = self._knowledge_store.get_note(source_id)
            if source_note:
                # Find context around the wikilink
                context = self._extract_context(source_note.content, note_id)
                results.append({
                    'source_id': source_id,
                    'source_title': source_note.title,
                    'context': context
                })
        
        return results
    
    def _extract_context(self, content: str, target_id: str) -> str:
        """Extract context around a wikilink"""
        # Find the wikilink in content
        import re
        pattern = re.compile(r'\[\[' + re.escape(target_id) + r'(?:\|[^\]]+)?\]\]')
        match = pattern.search(content)
        
        if match:
            # Get ±100 characters around the match
            start = max(0, match.start() - 100)
            end = min(len(content), match.end() + 100)
            return content[start:end]
        
        return ""
    
    def get_neighbors(self, note_id: str, depth: int = 1) -> Dict[str, List[str]]:
        """Get neighboring notes within a specified depth using BFS"""
        neighbors = {}
        visited = set()
        queue = deque([(note_id, 0)])
        
        while queue:
            current_id, current_depth = queue.popleft()
            
            if current_depth > depth:
                continue
            
            if current_id in visited:
                continue
            
            visited.add(current_id)
            
            # Get outgoing links
            outgoing = self.get_outgoing_links(current_id)
            if current_depth < depth:
                neighbors[current_id] = outgoing
                for linked_id in outgoing:
                    if linked_id not in visited:
                        queue.append((linked_id, current_depth + 1))
        
        return neighbors
    
    def find_unlinked_mentions(self, note_id: str) -> List[dict]:
        """Find notes that mention the note title but don't link to it"""
        # Get the target note
        target_note = self._knowledge_store.get_note(note_id)
        if not target_note:
            return []
        
        # Search for mentions of the title in other notes
        mentions = []
        all_notes = self._knowledge_store.list_notes()
        
        for note in all_notes:
            if note.id == note_id:
                continue
            
            # Check if title is mentioned in content
            if target_note.title.lower() in note.title.lower():
                mentions.append({
                    'note_id': note.id,
                    'title': note.title
                })
        
        return mentions
    
    def update_note(self, note_id: str, old_content: str, new_content: str):
        """Incrementally update graph index for a modified note"""
        # Extract old and new wikilinks
        ks = KnowledgeStore()
        old_links = ks._extract_wikilinks(old_content)
        new_links = ks._extract_wikilinks(new_content)
        
        # Remove old links
        for linked_id in old_links:
            if linked_id in self._outgoing.get(note_id, set()):
                self._outgoing[note_id].remove(linked_id)
            if linked_id in self._incoming:
                self._incoming[linked_id].discard(note_id)
        
        # Add new links
        for linked_id in new_links:
            if note_id not in self._outgoing:
                self._outgoing[note_id] = set()
            self._outgoing[note_id].add(linked_id)
            
            if linked_id not in self._incoming:
                self._incoming[linked_id] = set()
            self._incoming[linked_id].add(note_id)
