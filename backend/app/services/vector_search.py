"""Vector search service using LanceDB for semantic search"""
import re
import lancedb
from typing import List, Optional, Dict, Any
from pydantic import BaseModel
import pyarrow as pa

from backend.app.services.embedding import EmbeddingService
from backend.app.config import get_settings

settings = get_settings()


class ParsedQuery(BaseModel):
    """Parsed search query with operators extracted"""
    clean_query: str  # Query text with operators removed
    tags: List[str] = []  # tag:#research -> ["research"]
    exclude_tags: List[str] = []  # -tag:#draft -> ["draft"]
    paths: List[str] = []  # path:Canvas -> ["Canvas"]
    status: Optional[str] = None  # status:draft
    note_type: Optional[str] = None  # type:atomic
    exclude_terms: List[str] = []  # -word -> ["word"]
    has_filters: List[str] = []  # has:image -> ["image"]


class NoteEmbedding(BaseModel):
    """LanceDB schema for note embeddings"""
    note_id: str
    title: str
    text: str
    vector: List[float]
    content_type: Optional[str] = "general"  # For priority scoring
    modified: Optional[str] = None  # ISO format datetime for recency
    tags: List[str] = []  # Tags for relevance matching


class TileEmbedding(BaseModel):
    """LanceDB schema for canvas tile embeddings"""
    tile_id: str
    session_id: str
    prompt: str
    response: str
    model_id: str
    vector: List[float]


class VectorSearchService:
    """Service for vector-based semantic search using LanceDB"""
    
    def __init__(self, data_path: Optional[str] = None):
        """Initialize vector search service"""
        self.data_path = data_path or settings.data_path
        self.embedding_service = EmbeddingService()
        self._db = None
        self._table = None
        self._canvas_table = None
        self._initialize_db()
    
    def _initialize_db(self):
        """Initialize LanceDB connection and tables"""
        import os
        db_path = os.path.join(self.data_path, "lancedb")
        os.makedirs(db_path, exist_ok=True)

        self._db = lancedb.connect(db_path)

        # Create notes table if it doesn't exist
        if "notes" not in self._db.table_names():
            # Define PyArrow schema for LanceDB
            schema = pa.schema([
                pa.field("note_id", pa.string()),
                pa.field("title", pa.string()),
                pa.field("text", pa.string()),
                pa.field("vector", pa.list_(pa.float32(), 384)),  # 384 for all-MiniLM-L6-v2
                pa.field("content_type", pa.string()),  # For priority scoring
                pa.field("modified", pa.string()),  # ISO format datetime for recency
                pa.field("tags", pa.list_(pa.string())),  # Tags for relevance matching
            ])
            self._table = self._db.create_table("notes", schema=schema)
        else:
            self._table = self._db.open_table("notes")

        # Create canvas_tiles table if it doesn't exist
        if "canvas_tiles" not in self._db.table_names():
            schema = pa.schema([
                pa.field("tile_id", pa.string()),
                pa.field("session_id", pa.string()),
                pa.field("prompt", pa.string()),
                pa.field("response", pa.string()),
                pa.field("model_id", pa.string()),
                pa.field("vector", pa.list_(pa.float32(), 384)),
            ])
            self._canvas_table = self._db.create_table("canvas_tiles", schema=schema)
        else:
            self._canvas_table = self._db.open_table("canvas_tiles")
    
    def index_note(
        self,
        note_id: str,
        title: str,
        content: str,
        content_type: str = "general",
        modified: Optional[str] = None,
        tags: Optional[List[str]] = None,
    ):
        """Index a single note with priority metadata"""
        # Generate embedding
        text = f"{title}\n\n{content}"
        vector = self.embedding_service.encode(text)
        
        # Store in LanceDB
        embedding = NoteEmbedding(
            note_id=note_id,
            title=title,
            text=content[:1000],  # First 1000 chars for snippet
            vector=vector,
            content_type=content_type,
            modified=modified,
            tags=tags or [],
        )
        
        # Use merge_insert for upsert behavior
        try:
            self._table.merge_insert(
                "note_id"
            ).when_matched_update_all().when_not_matched_insert_all().execute(
                [embedding.model_dump()]
            )
        except Exception:
            # Fallback: just add (for empty tables)
            self._table.add([embedding.model_dump()])
    
    def index_all(self, notes: List[dict]):
        """Batch index all notes with priority metadata"""
        embeddings = []
        for note in notes:
            text = f"{note['title']}\n\n{note['content']}"
            vector = self.embedding_service.encode(text)
            
            embeddings.append(NoteEmbedding(
                note_id=note['id'],
                title=note['title'],
                text=note['content'][:1000],
                vector=vector,
                content_type=note.get('content_type', 'general'),
                modified=note.get('modified'),
                tags=note.get('tags', []),
            ))
        
        # Clear and add all embeddings for bulk reindex
        if embeddings:
            # For bulk indexing, clear and repopulate to avoid duplicates
            if self._table.count_rows() > 0:
                self._db.drop_table("notes")
                schema = pa.schema([
                    pa.field("note_id", pa.string()),
                    pa.field("title", pa.string()),
                    pa.field("text", pa.string()),
                    pa.field("vector", pa.list_(pa.float32(), 384)),
                    pa.field("content_type", pa.string()),
                    pa.field("modified", pa.string()),
                    pa.field("tags", pa.list_(pa.string())),
                ])
                self._table = self._db.create_table("notes", schema=schema)
            self._table.add([e.model_dump() for e in embeddings])
    
    def search(self, query: str, limit: int = 10) -> List[dict]:
        """Semantic search for notes"""
        # Generate query embedding
        query_vector = self.embedding_service.encode(query)
        
        # Search in LanceDB
        results = self._table.search(query_vector).limit(limit).to_list()
        
        # Format results - normalize distance to 0-1 score
        formatted_results = []
        for result in results:
            # LanceDB returns _distance - convert to similarity score (0-1)
            # Lower distance = higher similarity, so we invert it
            distance = float(result.get('_distance', 0.0))
            # Use 1/(1+distance) to normalize to 0-1 range
            score = 1.0 / (1.0 + distance) if distance >= 0 else 1.0
            
            formatted_results.append({
                'note_id': result['note_id'],
                'title': result['title'],
                'snippet': result['text'],
                'score': score,
                'content_type': result.get('content_type', 'general'),
                'modified': result.get('modified'),
                'tags': result.get('tags', []),
            })
        
        return formatted_results
    
    def parse_search_query(self, query: str) -> ParsedQuery:
        """
        Parse a search query to extract structured operators.
        
        Supported operators:
        - tag:#research or #research -> filter by tag
        - -tag:#draft or -#draft -> exclude tag
        - path:Canvas -> filter by path prefix
        - status:draft -> filter by status
        - type:atomic -> filter by note_type
        - has:image -> filter notes containing images
        - -word -> exclude notes containing "word"
        """
        tags = []
        exclude_tags = []
        paths = []
        status = None
        note_type = None
        exclude_terms = []
        has_filters = []
        
        # Pattern for operators
        patterns = {
            'tag': re.compile(r'(?:tag:)?#([\w/]+)', re.IGNORECASE),
            'exclude_tag': re.compile(r'-(?:tag:)?#([\w/]+)', re.IGNORECASE),
            'path': re.compile(r'path:([\w/\-_]+)', re.IGNORECASE),
            'status': re.compile(r'status:(\w+)', re.IGNORECASE),
            'type': re.compile(r'type:(\w+)', re.IGNORECASE),
            'has': re.compile(r'has:(\w+)', re.IGNORECASE),
            'exclude': re.compile(r'-(\w+)(?!\S)'),  # Standalone -word
        }
        
        clean_query = query
        
        # Extract exclude tags first (before regular tags)
        for match in patterns['exclude_tag'].finditer(query):
            exclude_tags.append(match.group(1))
        clean_query = patterns['exclude_tag'].sub('', clean_query)
        
        # Extract regular tags
        for match in patterns['tag'].finditer(clean_query):
            tags.append(match.group(1))
        clean_query = patterns['tag'].sub('', clean_query)
        
        # Extract paths
        for match in patterns['path'].finditer(clean_query):
            paths.append(match.group(1))
        clean_query = patterns['path'].sub('', clean_query)
        
        # Extract status
        match = patterns['status'].search(clean_query)
        if match:
            status = match.group(1)
        clean_query = patterns['status'].sub('', clean_query)
        
        # Extract note_type
        match = patterns['type'].search(clean_query)
        if match:
            note_type = match.group(1)
        clean_query = patterns['type'].sub('', clean_query)
        
        # Extract has filters
        for match in patterns['has'].finditer(clean_query):
            has_filters.append(match.group(1))
        clean_query = patterns['has'].sub('', clean_query)
        
        # Extract exclude terms (standalone -word)
        for match in patterns['exclude'].finditer(clean_query):
            exclude_terms.append(match.group(1))
        clean_query = patterns['exclude'].sub('', clean_query)
        
        # Clean up whitespace
        clean_query = ' '.join(clean_query.split())
        
        return ParsedQuery(
            clean_query=clean_query,
            tags=tags,
            exclude_tags=exclude_tags,
            paths=paths,
            status=status,
            note_type=note_type,
            exclude_terms=exclude_terms,
            has_filters=has_filters
        )
    
    def power_search(
        self, 
        query: str, 
        limit: int = 10,
        knowledge_store = None
    ) -> List[dict]:
        """
        Enhanced search with operator support.
        
        Uses semantic search + post-filtering based on parsed operators.
        """
        parsed = self.parse_search_query(query)
        
        # If no clean query, use original for embedding
        search_query = parsed.clean_query if parsed.clean_query.strip() else query
        
        # Get initial semantic results (fetch more to allow for filtering)
        initial_limit = limit * 3 if (parsed.tags or parsed.status or parsed.note_type) else limit
        semantic_results = self.search(search_query, initial_limit)
        
        # If no filters, return semantic results directly
        if not any([parsed.tags, parsed.exclude_tags, parsed.paths, 
                    parsed.status, parsed.note_type, parsed.exclude_terms]):
            return semantic_results[:limit]
        
        # Apply filters (requires knowledge_store for metadata)
        if not knowledge_store:
            return semantic_results[:limit]
        
        filtered_results = []
        for result in semantic_results:
            note = knowledge_store.get_note(result['note_id'])
            if not note:
                continue
            
            # Apply tag filters
            note_tags = note.frontmatter.tags if note.frontmatter else []
            note_tags_lower = [t.lower() for t in note_tags]
            
            if parsed.tags:
                # Check if note has any of the required tags (including hierarchical)
                has_required_tag = False
                for required_tag in parsed.tags:
                    for note_tag in note_tags_lower:
                        if note_tag == required_tag.lower() or note_tag.startswith(required_tag.lower() + '/'):
                            has_required_tag = True
                            break
                    if has_required_tag:
                        break
                if not has_required_tag:
                    continue
            
            if parsed.exclude_tags:
                # Check if note has any excluded tags
                has_excluded_tag = False
                for excluded_tag in parsed.exclude_tags:
                    for note_tag in note_tags_lower:
                        if note_tag == excluded_tag.lower() or note_tag.startswith(excluded_tag.lower() + '/'):
                            has_excluded_tag = True
                            break
                    if has_excluded_tag:
                        break
                if has_excluded_tag:
                    continue
            
            # Apply status filter
            if parsed.status:
                note_status = note.frontmatter.status if note.frontmatter else 'draft'
                if note_status.lower() != parsed.status.lower():
                    continue
            
            # Apply path filter
            if parsed.paths:
                matches_path = False
                for path in parsed.paths:
                    if result['note_id'].lower().startswith(path.lower()) or \
                       result['title'].lower().startswith(path.lower()):
                        matches_path = True
                        break
                if not matches_path:
                    continue
            
            # Apply exclude terms (check title and content)
            if parsed.exclude_terms:
                excluded = False
                content_lower = note.content.lower() if note.content else ''
                title_lower = note.title.lower() if note.title else ''
                for term in parsed.exclude_terms:
                    if term.lower() in content_lower or term.lower() in title_lower:
                        excluded = True
                        break
                if excluded:
                    continue
            
            # Add tags to result for display
            result['tags'] = note_tags
            filtered_results.append(result)
            
            if len(filtered_results) >= limit:
                break
        
        return filtered_results
    
    def delete_note(self, note_id: str):
        """Remove a note from the index"""
        self._table.delete("note_id = ?", [note_id])
    
    def clear_all(self):
        """Clear all indexed notes"""
        # Drop and recreate table
        self._db.drop_table("notes")
        # Define PyArrow schema for LanceDB
        schema = pa.schema([
            pa.field("note_id", pa.string()),
            pa.field("title", pa.string()),
            pa.field("text", pa.string()),
            pa.field("vector", pa.list_(pa.float32(), 384)),  # 384 for all-MiniLM-L6-v2
        ])
        self._table = self._db.create_table("notes", schema=schema)

    # ============================================================
    # Canvas Tile Methods
    # ============================================================

    def index_tile(
        self,
        tile_id: str,
        session_id: str,
        prompt: str,
        response: str,
        model_id: str,
    ):
        """Index a canvas tile response for semantic search"""
        # Generate embedding from prompt + response
        text = f"Q: {prompt}\n\nA: {response}"
        vector = self.embedding_service.encode(text)

        # Store in LanceDB
        embedding = TileEmbedding(
            tile_id=tile_id,
            session_id=session_id,
            prompt=prompt[:1000],  # Truncate for storage
            response=response[:2000],  # Truncate for storage
            model_id=model_id,
            vector=vector,
        )

        # Use merge_insert for upsert behavior (tile_id + model_id as key)
        # Create composite key for uniqueness
        key = f"{tile_id}:{model_id}"
        try:
            self._canvas_table.merge_insert(
                "tile_id"
            ).when_matched_update_all().when_not_matched_insert_all().execute(
                [embedding.model_dump()]
            )
        except Exception:
            # Fallback: just add (for empty tables)
            self._canvas_table.add([embedding.model_dump()])

    def delete_tile(self, tile_id: str):
        """Remove a tile from the index"""
        self._canvas_table.delete(f"tile_id = '{tile_id}'")

    def search_all(self, query: str, limit: int = 5) -> List[dict]:
        """
        Search both notes and canvas tiles for relevant content.
        Returns merged results sorted by score.
        """
        query_vector = self.embedding_service.encode(query)

        all_results = []

        # Search notes
        try:
            note_results = self._table.search(query_vector).limit(limit).to_list()
            for result in note_results:
                distance = float(result.get("_distance", 0.0))
                score = 1.0 / (1.0 + distance) if distance >= 0 else 1.0
                all_results.append({
                    "note_id": result["note_id"],
                    "title": result["title"],
                    "snippet": result["text"],
                    "score": score,
                    "type": "note",
                    "content_type": result.get("content_type", "general"),
                    "modified": result.get("modified"),
                    "tags": result.get("tags", []),
                })
        except Exception:
            pass  # Table might be empty

        # Search canvas tiles
        try:
            tile_results = self._canvas_table.search(query_vector).limit(limit).to_list()
            for result in tile_results:
                distance = float(result.get("_distance", 0.0))
                score = 1.0 / (1.0 + distance) if distance >= 0 else 1.0
                all_results.append({
                    "tile_id": result["tile_id"],
                    "session_id": result["session_id"],
                    "prompt": result["prompt"],
                    "response": result["response"],
                    "model_id": result["model_id"],
                    "score": score,
                    "type": "tile",
                })
        except Exception:
            pass  # Table might be empty

        # Sort by score descending and return top results
        all_results.sort(key=lambda x: x["score"], reverse=True)
        return all_results[:limit]
