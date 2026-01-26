"""Graph index service for wikilink parsing and backlink tracking"""
import re
from typing import List, Dict, Set, Optional
from collections import deque

from app.services.knowledge_store import KnowledgeStore


class GraphIndexService:
    """Service for managing knowledge graph with wikilinks and backlinks"""
    
    def __init__(self):
        """Initialize graph index"""
        self._outgoing: Dict[str, Set[str]] = {}  # note_id -> linked note IDs
        self._incoming: Dict[str, Set[str]] = {}  # note_id -> notes linking to it
        self._title_to_id: Dict[str, str] = {}    # title -> note_id (for resolving wikilinks)
        self._id_to_title: Dict[str, str] = {}    # note_id -> title (reverse lookup)
        self._knowledge_store = KnowledgeStore()
        self._build_index()
    
    def _build_index(self):
        """Build the graph index from all notes"""
        # Clear existing index
        self._outgoing.clear()
        self._incoming.clear()
        self._title_to_id.clear()
        self._id_to_title.clear()
        
        # Get all notes
        notes = self._knowledge_store.list_notes()
        
        # First pass: build title<->ID maps
        for note in notes:
            full_note = self._knowledge_store.get_note(note.id)
            if full_note:
                self._title_to_id[full_note.title] = note.id
                self._id_to_title[note.id] = full_note.title
                # Also map aliases if available
                for alias in full_note.frontmatter.aliases:
                    self._title_to_id[alias] = note.id
        
        # Second pass: build adjacency lists with resolved IDs
        for note in notes:
            note_id = note.id
            full_note = self._knowledge_store.get_note(note_id)
            
            if full_note:
                # Resolve wikilink titles to note IDs
                resolved_links = set()
                for linked_title in full_note.outgoing_links:
                    # Try to resolve title to ID
                    resolved_id = self._title_to_id.get(linked_title)
                    if resolved_id:
                        resolved_links.add(resolved_id)
                    else:
                        # Keep unresolved for forward links (note might not exist yet)
                        resolved_links.add(linked_title)
                
                self._outgoing[note_id] = resolved_links
                
                # Update incoming links for each target
                for linked_id in resolved_links:
                    if linked_id not in self._incoming:
                        self._incoming[linked_id] = set()
                    self._incoming[linked_id].add(note_id)
    
    def resolve_title_to_id(self, title: str) -> Optional[str]:
        """Resolve a wikilink title to a note ID"""
        return self._title_to_id.get(title)
    
    def resolve_id_to_title(self, note_id: str) -> Optional[str]:
        """Get the title for a note ID"""
        return self._id_to_title.get(note_id)
    
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
        
        # Get the target note's title for context extraction
        target_title = self._id_to_title.get(note_id, note_id)
        
        for source_id in backlinks:
            source_note = self._knowledge_store.get_note(source_id)
            if source_note:
                # Find context around the wikilink (by title, not ID)
                context = self._extract_context(source_note.content, target_title)
                results.append({
                    'note_id': source_id,  # Match BacklinkInfo model
                    'title': source_note.title,  # Match BacklinkInfo model
                    'context': context
                })
        
        return results
    
    def _extract_context(self, content: str, target_title: str) -> str:
        """Extract context around a wikilink"""
        # Find the wikilink in content (search by title)
        pattern = re.compile(r'\[\[' + re.escape(target_title) + r'(?:\|[^\]]+)?\]\]')
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
        
        # Get notes that already link to this note
        already_linked = set(self.get_backlinks(note_id))
        
        # Search for mentions of the title in other notes' CONTENT
        mentions = []
        all_notes = self._knowledge_store.list_notes()
        
        for note in all_notes:
            if note.id == note_id:
                continue
            
            # Skip if already linked
            if note.id in already_linked:
                continue
            
            # Get full note to check content
            full_note = self._knowledge_store.get_note(note.id)
            if not full_note:
                continue
            
            # Check if title is mentioned in content (case-insensitive)
            if target_note.title.lower() in full_note.content.lower():
                # Find the context of the mention
                pattern = re.compile(re.escape(target_note.title), re.IGNORECASE)
                match = pattern.search(full_note.content)
                context = ""
                if match:
                    start = max(0, match.start() - 50)
                    end = min(len(full_note.content), match.end() + 50)
                    context = full_note.content[start:end]
                
                mentions.append({
                    'note_id': note.id,
                    'title': note.title,
                    'context': context
                })
        
        return mentions
    
    def update_note(self, note_id: str, old_content: str, new_content: str):
        """Incrementally update graph index for a modified note"""
        # Extract old and new wikilinks
        ks = KnowledgeStore()
        old_links_raw = ks._extract_wikilinks(old_content)
        new_links_raw = ks._extract_wikilinks(new_content)
        
        # Resolve to IDs
        old_links = set(self._title_to_id.get(t, t) for t in old_links_raw)
        new_links = set(self._title_to_id.get(t, t) for t in new_links_raw)
        
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

    def get_unlinked_notes(self) -> List[dict]:
        """Get all notes that have no incoming or outgoing links (orphan notes)"""
        all_notes = self._knowledge_store.list_notes()
        unlinked = []

        for note in all_notes:
            note_id = note.id
            has_outgoing = bool(self._outgoing.get(note_id))
            has_incoming = bool(self._incoming.get(note_id))

            if not has_outgoing and not has_incoming:
                full_note = self._knowledge_store.get_note(note_id)
                if full_note:
                    unlinked.append({
                        'note_id': note_id,
                        'title': full_note.title,
                        'status': full_note.frontmatter.status.value if full_note.frontmatter.status else 'draft',
                        'tags': full_note.frontmatter.tags or [],
                    })

        return unlinked

    def get_full_graph(self) -> Dict:
        """Get the complete graph structure (nodes and edges) with hub-based coloring"""
        nodes = []
        edges = []
        
        # Color palette for hubs
        hub_colors = ['#8b5cf6', '#06b6d4', '#10b981', '#f59e0b', '#ef4444', '#ec4899', '#6366f1', '#14b8a6']
        
        # Get all involved notes (both source and target)
        all_ids = set(self._outgoing.keys())
        for targets in self._outgoing.values():
            all_ids.update(targets)
        
        # First pass: identify all hub notes and assign colors
        hub_to_color: Dict[str, str] = {}
        hub_index = 0
        for note_id in all_ids:
            note = self._knowledge_store.get_note(note_id)
            if note and note.frontmatter.note_type.value == "hub":
                hub_to_color[note_id] = hub_colors[hub_index % len(hub_colors)]
                hub_index += 1
        
        # Second pass: assign colors to non-hub notes based on hub linkage
        note_to_group: Dict[str, str] = {}
        for hub_id, color in hub_to_color.items():
            # Notes that the hub links TO inherit the hub's color
            for target_id in self._outgoing.get(hub_id, set()):
                if target_id not in hub_to_color:  # Don't override other hubs
                    note_to_group[target_id] = color
            # Notes that link TO the hub also get the color
            for source_id in self._incoming.get(hub_id, set()):
                if source_id not in hub_to_color:
                    note_to_group[source_id] = color
            
        # Create node list with extended metadata
        for note_id in all_ids:
            note = self._knowledge_store.get_note(note_id)
            title = note.title if note else note_id
            note_type = note.frontmatter.note_type.value if note else "general"
            tags = note.frontmatter.tags if note else []
            
            # Determine group color
            if note_id in hub_to_color:
                group_color = hub_to_color[note_id]
            elif note_id in note_to_group:
                group_color = note_to_group[note_id]
            else:
                group_color = "#6b7280"  # Gray for orphans/unlinked
            
            nodes.append({
                "id": note_id,
                "label": title,
                "val": len(self._incoming.get(note_id, set())) + 1,  # Size based on backlinks
                "note_type": note_type,
                "tags": tags,
                "group": group_color
            })
            
        # Create edge list
        for source, targets in self._outgoing.items():
            for target in targets:
                edges.append({
                    "source": source,
                    "target": target
                })
                
        return {
            "nodes": nodes,
            "links": edges
        }
