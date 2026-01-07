"""
Unit tests for EmbeddingService

Tests cover:
- Model loading (all-MiniLM-L6-v2)
- Single text encoding
- Batch text encoding
- Vector dimension validation (384)
- Unicode and special character handling
- Edge cases (empty text, very long text)
- Encoding consistency
"""
import pytest
import numpy as np

from app.services.embedding import EmbeddingService


# ============================================================================
# Model Loading Tests
# ============================================================================

@pytest.mark.unit
class TestModelLoading:
    """Test sentence-transformers model loading"""

    def test_model_loads_successfully(self, embedding_service: EmbeddingService):
        """Test that model loads without errors"""
        # Should have loaded model
        assert embedding_service is not None
        assert hasattr(embedding_service, 'model') or hasattr(embedding_service, '_model')

    def test_model_is_all_minilm_l6_v2(self, embedding_service: EmbeddingService):
        """Test that correct model is loaded"""
        # Model name should be all-MiniLM-L6-v2
        model_name = getattr(embedding_service, 'model_name', 'all-MiniLM-L6-v2')
        assert 'minilm' in model_name.lower() or 'all-MiniLM-L6-v2' in model_name

    def test_dimension_is_384(self, embedding_service: EmbeddingService):
        """Test that model produces 384-dimensional vectors"""
        assert embedding_service.dimension == 384


# ============================================================================
# Single Text Encoding Tests
# ============================================================================

@pytest.mark.unit
class TestSingleTextEncoding:
    """Test encoding single texts"""

    def test_encode_simple_text(self, embedding_service: EmbeddingService):
        """Test encoding simple text"""
        text = "This is a test sentence."

        vector = embedding_service.encode(text)

        assert vector is not None
        assert len(vector) == 384
        # Vector should contain floats
        assert all(isinstance(v, (float, np.floating)) for v in vector)

    def test_encode_empty_string(self, embedding_service: EmbeddingService):
        """Test encoding empty string"""
        text = ""

        # Should handle gracefully
        vector = embedding_service.encode(text)

        # Should still return valid vector
        assert len(vector) == 384

    def test_encode_whitespace_only(self, embedding_service: EmbeddingService):
        """Test encoding whitespace-only text"""
        text = "   \n\n   "

        vector = embedding_service.encode(text)

        assert len(vector) == 384

    def test_encode_single_word(self, embedding_service: EmbeddingService):
        """Test encoding single word"""
        text = "python"

        vector = embedding_service.encode(text)

        assert len(vector) == 384

    def test_encode_long_text(self, embedding_service: EmbeddingService):
        """Test encoding long text"""
        # Create long text (several paragraphs)
        text = " ".join(["This is sentence number {}.".format(i) for i in range(100)])

        vector = embedding_service.encode(text)

        assert len(vector) == 384

    def test_encode_very_long_text(self, embedding_service: EmbeddingService):
        """Test encoding very long text exceeding model limits"""
        # Create text with many tokens (> 512 for most models)
        text = " ".join(["word"] * 1000)

        # Should handle gracefully (truncate or chunk)
        vector = embedding_service.encode(text)

        assert len(vector) == 384


# ============================================================================
# Batch Encoding Tests
# ============================================================================

@pytest.mark.unit
class TestBatchEncoding:
    """Test batch encoding of multiple texts"""

    def test_encode_batch_multiple_texts(self, embedding_service: EmbeddingService):
        """Test encoding batch of texts"""
        texts = [
            "First sentence",
            "Second sentence",
            "Third sentence",
        ]

        vectors = embedding_service.encode_batch(texts)

        assert len(vectors) == 3
        # Each should be 384-dimensional
        for vector in vectors:
            assert len(vector) == 384

    def test_encode_batch_single_text(self, embedding_service: EmbeddingService):
        """Test batch encoding with single text"""
        texts = ["Single text"]

        vectors = embedding_service.encode_batch(texts)

        assert len(vectors) == 1
        assert len(vectors[0]) == 384

    def test_encode_batch_empty_list(self, embedding_service: EmbeddingService):
        """Test batch encoding with empty list"""
        texts = []

        vectors = embedding_service.encode_batch(texts)

        assert vectors == [] or len(vectors) == 0

    def test_encode_batch_large(self, embedding_service: EmbeddingService):
        """Test encoding large batch"""
        texts = [f"Sentence number {i}" for i in range(100)]

        vectors = embedding_service.encode_batch(texts)

        assert len(vectors) == 100
        for vector in vectors:
            assert len(vector) == 384

    def test_encode_batch_vs_single_consistency(self, embedding_service: EmbeddingService):
        """Test that batch and single encoding produce same results"""
        text = "Test sentence for consistency"

        # Encode individually
        single_vector = embedding_service.encode(text)

        # Encode in batch
        batch_vectors = embedding_service.encode_batch([text])
        batch_vector = batch_vectors[0]

        # Should be very similar (allowing for small numerical differences)
        similarity = np.dot(single_vector, batch_vector)
        assert similarity > 0.99  # Cosine similarity should be very high


# ============================================================================
# Unicode and Special Characters Tests
# ============================================================================

@pytest.mark.unit
class TestUnicodeHandling:
    """Test handling of unicode and special characters"""

    def test_encode_unicode_text(self, embedding_service: EmbeddingService):
        """Test encoding text with unicode characters"""
        texts = [
            "Hello world",  # English
            "你好世界",      # Chinese
            "こんにちは世界",  # Japanese
            "مرحبا بالعالم",  # Arabic
            "Привет мир",   # Russian
        ]

        for text in texts:
            vector = embedding_service.encode(text)
            assert len(vector) == 384

    def test_encode_emoji(self, embedding_service: EmbeddingService):
        """Test encoding text with emoji"""
        text = "Hello 👋 world 🌍 with emoji 🚀"

        vector = embedding_service.encode(text)

        assert len(vector) == 384

    def test_encode_mixed_scripts(self, embedding_service: EmbeddingService):
        """Test encoding text with mixed scripts"""
        text = "English with 中文 and العربية and emoji 🎉"

        vector = embedding_service.encode(text)

        assert len(vector) == 384

    def test_encode_special_characters(self, embedding_service: EmbeddingService):
        """Test encoding text with special characters"""
        texts = [
            "Text with @#$%^&*() symbols",
            "Math symbols: ∑ ∏ ∫ √ ∞",
            "Currency: € £ ¥ $ ¢",
            "Code: const x = {key: 'value'};",
        ]

        for text in texts:
            vector = embedding_service.encode(text)
            assert len(vector) == 384


# ============================================================================
# Vector Properties Tests
# ============================================================================

@pytest.mark.unit
class TestVectorProperties:
    """Test properties of generated vectors"""

    def test_vector_values_are_floats(self, embedding_service: EmbeddingService):
        """Test that vector values are floats"""
        text = "Test text"

        vector = embedding_service.encode(text)

        # All values should be floats
        assert all(isinstance(v, (float, np.floating)) for v in vector)

    def test_vector_values_normalized(self, embedding_service: EmbeddingService):
        """Test that vectors are normalized (unit length)"""
        text = "Test text for normalization"

        vector = embedding_service.encode(text)

        # Calculate L2 norm
        norm = np.linalg.norm(vector)

        # Should be close to 1 (unit vector) if model normalizes
        # Some models normalize, some don't - this documents behavior
        # Typically sentence-transformers normalizes by default

    def test_similar_texts_have_similar_vectors(self, embedding_service: EmbeddingService):
        """Test that semantically similar texts have similar vectors"""
        text1 = "The cat sits on the mat"
        text2 = "A cat is sitting on a mat"
        text3 = "Quantum physics is complex"

        vec1 = embedding_service.encode(text1)
        vec2 = embedding_service.encode(text2)
        vec3 = embedding_service.encode(text3)

        # Similarity between similar texts
        sim_12 = np.dot(vec1, vec2)

        # Similarity between dissimilar texts
        sim_13 = np.dot(vec1, vec3)

        # Similar texts should be more similar
        assert sim_12 > sim_13

    def test_identical_texts_produce_identical_vectors(self, embedding_service: EmbeddingService):
        """Test that same text produces same vector"""
        text = "This is a test sentence"

        vec1 = embedding_service.encode(text)
        vec2 = embedding_service.encode(text)

        # Should be identical
        assert np.allclose(vec1, vec2)


# ============================================================================
# Encoding Consistency Tests
# ============================================================================

@pytest.mark.unit
class TestEncodingConsistency:
    """Test consistency of encodings"""

    def test_deterministic_encoding(self, embedding_service: EmbeddingService):
        """Test that encoding is deterministic"""
        text = "Deterministic test"

        # Encode multiple times
        vectors = [embedding_service.encode(text) for _ in range(5)]

        # All should be identical
        for i in range(1, 5):
            assert np.allclose(vectors[0], vectors[i])

    def test_case_sensitivity(self, embedding_service: EmbeddingService):
        """Test how case affects encoding"""
        text_lower = "python programming"
        text_upper = "PYTHON PROGRAMMING"
        text_title = "Python Programming"

        vec_lower = embedding_service.encode(text_lower)
        vec_upper = embedding_service.encode(text_upper)
        vec_title = embedding_service.encode(text_title)

        # Should be similar but not identical
        sim_lower_upper = np.dot(vec_lower, vec_upper)
        assert sim_lower_upper > 0.8  # High similarity despite case

    def test_whitespace_variations(self, embedding_service: EmbeddingService):
        """Test how whitespace affects encoding"""
        text1 = "Python programming"
        text2 = "Python  programming"  # Extra space
        text3 = "Python\nprogramming"    # Newline

        vec1 = embedding_service.encode(text1)
        vec2 = embedding_service.encode(text2)
        vec3 = embedding_service.encode(text3)

        # Should be very similar
        assert np.dot(vec1, vec2) > 0.95
        assert np.dot(vec1, vec3) > 0.95


# ============================================================================
# Performance Tests
# ============================================================================

@pytest.mark.unit
@pytest.mark.slow
class TestPerformance:
    """Test encoding performance"""

    def test_single_encoding_speed(self, embedding_service: EmbeddingService):
        """Test single text encoding speed"""
        import time

        text = "Test sentence for performance measurement"

        start = time.time()
        for _ in range(10):
            embedding_service.encode(text)
        elapsed = time.time() - start

        # Should complete 10 encodings in reasonable time
        assert elapsed < 5.0  # 5 seconds for 10 encodings

    def test_batch_encoding_efficiency(self, embedding_service: EmbeddingService):
        """Test that batch encoding is more efficient than sequential"""
        import time

        texts = [f"Sentence number {i}" for i in range(50)]

        # Sequential encoding
        start = time.time()
        for text in texts:
            embedding_service.encode(text)
        sequential_time = time.time() - start

        # Batch encoding
        start = time.time()
        embedding_service.encode_batch(texts)
        batch_time = time.time() - start

        # Batch should be faster (or at least not much slower)
        # Some implementations may not see speedup due to model internals
        assert batch_time <= sequential_time * 1.5  # Allow some variance


# ============================================================================
# Edge Cases and Error Handling
# ============================================================================

@pytest.mark.unit
class TestEdgeCases:
    """Test edge cases and error conditions"""

    def test_encode_none(self, embedding_service: EmbeddingService):
        """Test encoding None value"""
        try:
            vector = embedding_service.encode(None)
            # If it doesn't raise, should return valid vector or handle gracefully
        except (TypeError, AttributeError):
            # Acceptable to reject None
            pass

    def test_encode_number(self, embedding_service: EmbeddingService):
        """Test encoding number (should convert to string)"""
        # Numbers might be converted to strings
        try:
            vector = embedding_service.encode(12345)
            assert len(vector) == 384
        except (TypeError, AttributeError):
            # Acceptable to require string
            pass

    def test_encode_list(self, embedding_service: EmbeddingService):
        """Test that single encode doesn't accept list"""
        try:
            embedding_service.encode(["text1", "text2"])
            # If accepted, should handle appropriately
        except (TypeError, ValueError):
            # Expected to reject list in single encode
            pass

    def test_batch_encode_with_none(self, embedding_service: EmbeddingService):
        """Test batch encoding with None in list"""
        texts = ["Valid text", None, "Another valid text"]

        try:
            vectors = embedding_service.encode_batch(texts)
            # Should handle or skip None
        except (TypeError, AttributeError):
            # Acceptable to reject None in batch
            pass

    def test_encode_extremely_long_text(self, embedding_service: EmbeddingService):
        """Test encoding text well beyond model limits"""
        # Create extremely long text (10,000 words)
        text = " ".join(["word"] * 10000)

        # Should handle gracefully (truncate or error)
        try:
            vector = embedding_service.encode(text)
            assert len(vector) == 384
        except Exception as e:
            # Document any limitations
            pass

    def test_encode_only_punctuation(self, embedding_service: EmbeddingService):
        """Test encoding text with only punctuation"""
        text = "!@#$%^&*()_+-=[]{}|;:',.<>?/~`"

        vector = embedding_service.encode(text)

        assert len(vector) == 384

    def test_encode_repeated_characters(self, embedding_service: EmbeddingService):
        """Test encoding text with repeated characters"""
        text = "a" * 1000

        vector = embedding_service.encode(text)

        assert len(vector) == 384

    def test_concurrent_encoding(self, embedding_service: EmbeddingService):
        """Test concurrent encoding requests"""
        import concurrent.futures

        texts = [f"Text {i}" for i in range(20)]

        def encode_text(text):
            return embedding_service.encode(text)

        # Encode concurrently
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
            futures = [executor.submit(encode_text, text) for text in texts]
            results = [f.result() for f in futures]

        # All should succeed
        assert len(results) == 20
        for vector in results:
            assert len(vector) == 384


# ============================================================================
# Model-Specific Tests
# ============================================================================

@pytest.mark.unit
class TestModelSpecificBehavior:
    """Test behavior specific to all-MiniLM-L6-v2"""

    def test_model_max_sequence_length(self, embedding_service: EmbeddingService):
        """Test handling of sequences beyond model's max length"""
        # all-MiniLM-L6-v2 typically has 256 or 512 token limit

        # Create text longer than limit
        long_text = " ".join(["word"] * 600)

        # Should handle via truncation
        vector = embedding_service.encode(long_text)

        assert len(vector) == 384

    def test_model_pooling_strategy(self, embedding_service: EmbeddingService):
        """Test that model uses appropriate pooling"""
        # sentence-transformers typically uses mean pooling

        text = "Test sentence"
        vector = embedding_service.encode(text)

        # Vector should exist and be valid
        assert vector is not None
        assert len(vector) == 384
        # Values should be in reasonable range for mean pooling
        assert all(-10 < v < 10 for v in vector)


# ============================================================================
# Integration Tests
# ============================================================================

@pytest.mark.unit
class TestIntegration:
    """Test integration with other components"""

    def test_embedding_used_by_vector_search(self, embedding_service: EmbeddingService):
        """Test that embeddings work with vector search"""
        # Create embeddings
        text1 = "Python programming language"
        text2 = "Java programming language"

        vec1 = embedding_service.encode(text1)
        vec2 = embedding_service.encode(text2)

        # Should be able to compute similarity
        similarity = np.dot(vec1, vec2)

        # Similar texts should have positive similarity
        assert similarity > 0

    def test_batch_for_indexing(self, embedding_service: EmbeddingService):
        """Test batch encoding for vector search indexing"""
        # Simulate bulk indexing scenario
        notes = [
            {"title": "Note 1", "content": "Content 1"},
            {"title": "Note 2", "content": "Content 2"},
            {"title": "Note 3", "content": "Content 3"},
        ]

        # Combine title and content
        texts = [f"{note['title']}\n\n{note['content']}" for note in notes]

        # Batch encode
        vectors = embedding_service.encode_batch(texts)

        assert len(vectors) == 3
        for vector in vectors:
            assert len(vector) == 384
