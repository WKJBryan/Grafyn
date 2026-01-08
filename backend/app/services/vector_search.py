"""Vector search service using LanceDB for semantic search"""
import lancedb
from typing import List, Optional
from pydantic import BaseModel
import pyarrow as pa

from backend.app.services.embedding import EmbeddingService
from backend.app.config import get_settings

settings = get_settings()


class NoteEmbedding(BaseModel):
    """LanceDB schema for note embeddings"""
    note_id: str
    title: str
    text: str
    vector: List[float]


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
