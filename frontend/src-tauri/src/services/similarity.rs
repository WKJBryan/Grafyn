//! Similarity provider traits and TF-based implementation.
//!
//! Defines trait interfaces so TextTiling can be upgraded to DeepTiling later
//! without changing callers. Currently uses sparse TF vectors with cosine similarity.
//!
//! Future upgrade path (not implemented now):
//! - `DenseVector { values: Vec<f32> }` implementing `TextVector` with SIMD cosine
//! - `EmbeddingProvider` implementing `SimilarityProvider` via ONNX model or API
//! - TextTiling accepts `&dyn SimilarityProvider` → swap TfIdf for embeddings = DeepTiling

use crate::services::yake::STOPWORDS;
use std::collections::{HashMap, HashSet};

// ── Trait definitions ────────────────────────────────────────────────────

/// Trait for vector representations that support cosine similarity.
pub trait TextVector {
    fn cosine_similarity(&self, other: &Self) -> f64;
    fn dimension(&self) -> usize;
}

/// Trait for encoding text into vector representations.
pub trait SimilarityProvider {
    type Vector: TextVector;

    fn encode(&self, text: &str) -> Self::Vector;

    fn encode_batch(&self, texts: &[&str]) -> Vec<Self::Vector> {
        texts.iter().map(|t| self.encode(t)).collect()
    }

    fn similarity(&self, a: &str, b: &str) -> f64 {
        let va = self.encode(a);
        let vb = self.encode(b);
        va.cosine_similarity(&vb)
    }
}

// ── Sparse TF vector ─────────────────────────────────────────────────────

/// Sparse term-frequency vector using a hashmap of term → weight.
#[derive(Debug, Clone)]
pub struct SparseTfVector {
    pub terms: HashMap<String, f64>,
}

impl SparseTfVector {
    pub fn new() -> Self {
        Self {
            terms: HashMap::new(),
        }
    }

    pub fn from_terms(terms: HashMap<String, f64>) -> Self {
        Self { terms }
    }
}

impl Default for SparseTfVector {
    fn default() -> Self {
        Self::new()
    }
}

impl TextVector for SparseTfVector {
    fn cosine_similarity(&self, other: &Self) -> f64 {
        if self.terms.is_empty() || other.terms.is_empty() {
            return 0.0;
        }

        let mut dot_product = 0.0;
        let mut norm_a = 0.0;

        for (term, &weight_a) in &self.terms {
            norm_a += weight_a * weight_a;
            if let Some(&weight_b) = other.terms.get(term) {
                dot_product += weight_a * weight_b;
            }
        }

        let norm_b: f64 = other.terms.values().map(|w| w * w).sum();
        let magnitude = (norm_a * norm_b).sqrt();

        if magnitude < 1e-10 {
            return 0.0;
        }

        dot_product / magnitude
    }

    fn dimension(&self) -> usize {
        self.terms.len()
    }
}

// ── TF-IDF provider ──────────────────────────────────────────────────────

/// TF-based similarity provider. Encodes text as term-frequency vectors
/// after filtering stopwords and short tokens.
pub struct TfIdfProvider {
    stopwords: HashSet<&'static str>,
    min_word_len: usize,
}

impl TfIdfProvider {
    pub fn new() -> Self {
        Self {
            stopwords: STOPWORDS.iter().copied().collect(),
            min_word_len: 3,
        }
    }
}

impl Default for TfIdfProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SimilarityProvider for TfIdfProvider {
    type Vector = SparseTfVector;

    fn encode(&self, text: &str) -> SparseTfVector {
        let mut term_counts: HashMap<String, f64> = HashMap::new();

        for word in text.split(|c: char| !c.is_alphanumeric()) {
            let lower = word.to_lowercase();
            if lower.len() >= self.min_word_len && !self.stopwords.contains(lower.as_str()) {
                *term_counts.entry(lower).or_default() += 1.0;
            }
        }

        SparseTfVector::from_terms(term_counts)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sparse_tf_vector_cosine_identical() {
        let terms = HashMap::from([("rust".to_string(), 2.0), ("language".to_string(), 1.0)]);
        let v = SparseTfVector::from_terms(terms);

        let sim = v.cosine_similarity(&v);
        assert!((sim - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_sparse_tf_vector_cosine_orthogonal() {
        let v1 = SparseTfVector::from_terms([("rust".to_string(), 1.0)].into_iter().collect());
        let v2 = SparseTfVector::from_terms([("python".to_string(), 1.0)].into_iter().collect());

        let sim = v1.cosine_similarity(&v2);
        assert!((sim).abs() < 1e-10);
    }

    #[test]
    fn test_sparse_tf_vector_cosine_empty() {
        let v1 = SparseTfVector::new();
        let v2 = SparseTfVector::from_terms([("rust".to_string(), 1.0)].into_iter().collect());

        assert_eq!(v1.cosine_similarity(&v2), 0.0);
        assert_eq!(v2.cosine_similarity(&v1), 0.0);
    }

    #[test]
    fn test_tfidf_provider_encode() {
        let provider = TfIdfProvider::new();
        let v = provider.encode("Rust programming language is great for systems programming");

        assert!(v.terms.contains_key("rust"));
        assert!(v.terms.contains_key("programming"));
        // Stopwords filtered out
        assert!(!v.terms.contains_key("is"));
        assert!(!v.terms.contains_key("for"));
        // "programming" appears twice
        assert_eq!(v.terms.get("programming"), Some(&2.0));
    }

    #[test]
    fn test_tfidf_provider_similarity() {
        let provider = TfIdfProvider::new();
        let sim = provider.similarity(
            "Machine learning algorithms for data science",
            "Data science uses machine learning models",
        );
        assert!(sim > 0.0);
        assert!(sim <= 1.0);
    }

    #[test]
    fn test_dimension() {
        let v = SparseTfVector::from_terms(
            [("aaa".to_string(), 1.0), ("bbb".to_string(), 2.0)]
                .into_iter()
                .collect(),
        );
        assert_eq!(v.dimension(), 2);
    }
}
