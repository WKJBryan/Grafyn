"""Embedding service for text to vector encoding using sentence-transformers"""
from sentence_transformers import SentenceTransformer
from typing import List

from backend.app.config import get_settings

settings = get_settings()


class EmbeddingService:
    """Service for encoding text to vectors using sentence-transformers"""
    
    def __init__(self, model_name: str = None):
        """Initialize embedding service with specified model"""
        self.model_name = model_name or settings.embedding_model
        self._model = None
        self._load_model()
    
    def _load_model(self):
        """Load the sentence transformer model"""
        self._model = SentenceTransformer(self.model_name)
    
    def encode(self, text: str) -> List[float]:
        """Encode a single text to vector"""
        embedding = self._model.encode(text)
        return embedding.tolist()
    
    def encode_batch(self, texts: List[str]) -> List[List[float]]:
        """Encode multiple texts to vectors"""
        embeddings = self._model.encode(texts)
        return [e.tolist() for e in embeddings]
    
    @property
    def dimension(self) -> int:
        """Get the dimension of the embedding vectors"""
        return self._model.get_sentence_embedding_dimension()
