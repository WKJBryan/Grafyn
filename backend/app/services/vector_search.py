"""Vector search service using LanceDB for semantic search"""
import lancedb
from typing import List, Optional
from pydantic import BaseModel

from app.services.embedding import EmbeddingService
from app.config import get_settings

settings = get_settings()


class NoteEmbedding(BaseModel):
    """LanceDB schema for note embeddings"""
    note_id: str
    title: str
    text: str
    vector: List[float]


class VectorSearchService:
    """Service for vector-based semantic search using LanceDB"""
    
    def __init__(self, data_path: Optional[str] = None):
        """Initialize vector search service"""
        self.data_path = data_path or settings.data_path
        self.embedding_service = EmbeddingService()
        self._db = None
        self._table = None
        self._initialize_db()
    
    def _initialize_db(self):
        """Initialize LanceDB connection and table"""
        import os
        db_path = os.path.join(self.data_path, "lancedb")
        os.makedirs(db_path, exist_ok=True)
        
        self._db = lancedb.connect(db_path)
        
        # Create table if it doesn't exist
        if "notes" not in self._db.table_names():
            schema = NoteEmbedding.to_arrow_schema()
            self._table = self._db.create_table("notes", schema=schema)
        else:
            self._table = self._db.open_table("notes")
    
    def index_note(self, note_id: str, title: str, content: str):
        """Index a single note"""
        # Generate embedding
        text = f"{title}\n\n{content}"
        vector = self.embedding_service.encode(text)
        
        # Store in LanceDB
        embedding = NoteEmbedding(
            note_id=note_id,
            title=title,
            text=content[:1000],  # First 1000 chars for snippet
            vector=vector
        )
        
        # Upsert to handle updates
        self._table.add([embedding.model_dump()], mode="upsert")
    
    def index_all(self, notes: List[dict]):
        """Batch index all notes"""
        embeddings = []
        for note in notes:
            text = f"{note['title']}\n\n{note['content']}"
            vector = self.embedding_service.encode(text)
            
            embeddings.append(NoteEmbedding(
                note_id=note['id'],
                title=note['title'],
                text=note['content'][:1000],
                vector=vector
            ))
        
        # Upsert all embeddings
        if embeddings:
            self._table.add([e.model_dump() for e in embeddings], mode="upsert")
    
    def search(self, query: str, limit: int = 10) -> List[dict]:
        """Semantic search for notes"""
        # Generate query embedding
        query_vector = self.embedding_service.encode(query)
        
        # Search in LanceDB
        results = self._table.search(query_vector).limit(limit).to_list()
        
        # Format results
        formatted_results = []
        for result in results:
            formatted_results.append({
                'note_id': result['note_id'],
                'title': result['title'],
                'snippet': result['text'],
                'score': float(result.get('_distance', 0.0)),
                'tags': []  # Tags would need to be joined from knowledge store
            })
        
        return formatted_results
    
    def delete_note(self, note_id: str):
        """Remove a note from the index"""
        self._table.delete("note_id = ?", [note_id])
    
    def clear_all(self):
        """Clear all indexed notes"""
        # Drop and recreate table
        self._db.drop_table("notes")
        schema = NoteEmbedding.to_arrow_schema()
        self._table = self._db.create_table("notes", schema=schema)
