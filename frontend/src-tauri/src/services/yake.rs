//! YAKE (Yet Another Keyword Extractor) — pure-Rust unsupervised keyphrase extraction.
//!
//! Uses 5 statistical features per word: casing, position, frequency, relatedness
//! (co-occurrence context), and dispersion (sentence spread). No external dependencies.

use std::collections::{HashMap, HashSet};

/// Extracted keyphrase with score (lower = more relevant)
#[derive(Debug, Clone)]
pub struct Keyphrase {
    pub text: String,
    pub score: f64,
    pub word_count: usize,
}

/// Configuration for YAKE extraction
pub struct YakeConfig {
    /// Maximum n-gram size (1=unigrams, 2=bigrams, 3=trigrams)
    pub max_ngram_size: usize,
    /// Jaccard similarity threshold for deduplicating similar keyphrases
    pub dedup_threshold: f64,
    /// Maximum number of keyphrases to return
    pub top_k: usize,
    /// Co-occurrence window size for relatedness feature
    pub window_size: usize,
}

impl Default for YakeConfig {
    fn default() -> Self {
        Self {
            max_ngram_size: 3,
            dedup_threshold: 0.9,
            top_k: 10,
            window_size: 2,
        }
    }
}

/// Canonical stopword list — shared across YAKE and zettelkasten modules.
pub const STOPWORDS: &[&str] = &[
    "a", "about", "above", "after", "again", "against", "all", "also", "am", "an", "and",
    "any", "are", "aren't", "as", "at", "back", "be", "because", "been", "before", "being",
    "below", "between", "both", "but", "by", "can", "can't", "cannot", "could", "couldn't",
    "did", "didn't", "do", "does", "doesn't", "doing", "don't", "down", "during", "each",
    "even", "every", "few", "for", "from", "further", "get", "got", "had", "hadn't", "has",
    "hasn't", "have", "haven't", "having", "he", "her", "here", "hers", "herself", "him",
    "himself", "his", "how", "i", "if", "in", "into", "is", "isn't", "it", "it's", "its",
    "itself", "just", "let", "like", "ll", "made", "make", "many", "may", "me", "might",
    "more", "most", "much", "must", "mustn't", "my", "myself", "no", "nor", "not", "now",
    "of", "off", "on", "once", "only", "or", "other", "ought", "our", "ours", "ourselves",
    "out", "over", "own", "re", "s", "same", "shall", "shan't", "she", "should", "shouldn't",
    "so", "some", "such", "t", "than", "that", "the", "their", "theirs", "them", "themselves",
    "then", "there", "these", "they", "this", "those", "through", "to", "too", "under",
    "until", "up", "ve", "very", "was", "wasn't", "we", "well", "were", "weren't", "what",
    "when", "where", "which", "while", "who", "whom", "why", "will", "with", "won't",
    "would", "wouldn't", "you", "your", "yours", "yourself", "yourselves",
    // Domain-specific
    "note", "notes",
];

// ── Internal types ───────────────────────────────────────────────────────

/// Per-word statistical features collected during a single pass over sentences.
struct WordStats {
    frequency: usize,
    uppercase_count: usize,
    capitalized_count: usize,
    sentence_positions: Vec<usize>,
    first_position: usize,
    left_context: HashSet<String>,
    right_context: HashSet<String>,
}

// ── Tokenization ─────────────────────────────────────────────────────────

/// Split text into sentences (heuristic: split on `.!?\n`).
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for c in text.chars() {
        current.push(c);
        if c == '.' || c == '!' || c == '?' || c == '\n' {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() && trimmed.split_whitespace().count() >= 2 {
                sentences.push(trimmed);
            }
            current.clear();
        }
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() && trimmed.split_whitespace().count() >= 2 {
        sentences.push(trimmed);
    }

    // Guarantee at least one sentence for non-empty input
    if sentences.is_empty() && !text.trim().is_empty() {
        sentences.push(text.trim().to_string());
    }

    sentences
}

/// Tokenize into words, keeping original casing for stats.
fn tokenize_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
        .filter(|w| !w.is_empty() && w.len() >= 2)
        .map(|w| w.to_string())
        .collect()
}

fn is_stopword(word: &str) -> bool {
    STOPWORDS.contains(&word.to_lowercase().as_str())
}

// ── Feature computation ──────────────────────────────────────────────────

fn compute_word_stats(sentences: &[String], window_size: usize) -> HashMap<String, WordStats> {
    let mut stats: HashMap<String, WordStats> = HashMap::new();
    let mut global_position = 0usize;

    for (sent_idx, sentence) in sentences.iter().enumerate() {
        let words = tokenize_words(sentence);

        for (i, word) in words.iter().enumerate() {
            let lower = word.to_lowercase();
            if lower.len() < 2 {
                continue;
            }

            let entry = stats.entry(lower.clone()).or_insert_with(|| WordStats {
                frequency: 0,
                uppercase_count: 0,
                capitalized_count: 0,
                sentence_positions: Vec::new(),
                first_position: global_position + i,
                left_context: HashSet::new(),
                right_context: HashSet::new(),
            });

            entry.frequency += 1;

            // Casing features
            if word.chars().all(|c| c.is_uppercase() || !c.is_alphabetic())
                && word.chars().any(|c| c.is_uppercase())
            {
                entry.uppercase_count += 1;
            }
            if word.chars().next().map_or(false, |c| c.is_uppercase()) {
                entry.capitalized_count += 1;
            }

            // Sentence position tracking
            if entry.sentence_positions.last() != Some(&sent_idx) {
                entry.sentence_positions.push(sent_idx);
            }

            // Co-occurrence context (relatedness feature)
            for j in i.saturating_sub(window_size)..i {
                let ctx = words[j].to_lowercase();
                if ctx != lower {
                    entry.left_context.insert(ctx);
                }
            }
            for j in (i + 1)..=(i + window_size).min(words.len().saturating_sub(1)) {
                let ctx = words[j].to_lowercase();
                if ctx != lower {
                    entry.right_context.insert(ctx);
                }
            }
        }

        global_position += words.len();
    }

    stats
}

/// Compute per-word importance scores from statistical features.
/// Lower score = more important.
fn compute_word_scores(
    stats: &HashMap<String, WordStats>,
    total_sentences: usize,
) -> HashMap<String, f64> {
    let max_freq = stats.values().map(|s| s.frequency).max().unwrap_or(1) as f64;
    let n_sentences = total_sentences.max(1) as f64;

    let mut scores = HashMap::new();

    for (word, s) in stats {
        if is_stopword(word) || word.len() < 2 {
            continue;
        }

        let tf = s.frequency as f64;

        // T_case: casing significance (acronyms / proper nouns score higher)
        let t_case = {
            let upper_ratio = s.uppercase_count as f64 / tf.max(1.0);
            let cap_ratio = s.capitalized_count as f64 / tf.max(1.0);
            upper_ratio.max(cap_ratio)
        };

        // T_pos: positional significance (earlier = smaller = better)
        let t_pos = (s.first_position as f64 + 1.0).ln_1p() / (n_sentences + 1.0).ln_1p();
        let t_pos = t_pos.max(0.01);

        // T_freq: normalized frequency
        let t_freq = tf / max_freq;

        // T_rel: relatedness — distinct co-occurring words relative to frequency
        let context_count = (s.left_context.len() + s.right_context.len()) as f64;
        let t_rel = 1.0 + context_count / (tf + 1.0);

        // T_dis: dispersion — spread across sentences
        let t_dis = s.sentence_positions.len() as f64 / n_sentences;

        // S(w) = T_pos / (T_case + T_freq/T_rel + T_dis/T_rel)
        let denominator = (t_case + 1e-6) + (t_freq / t_rel) + (t_dis / t_rel);
        let score = t_pos / denominator.max(1e-6);

        scores.insert(word.clone(), score);
    }

    scores
}

// ── N-gram extraction and scoring ────────────────────────────────────────

fn extract_ngrams(
    sentences: &[String],
    max_n: usize,
) -> Vec<(String, usize, Vec<String>)> {
    let mut ngram_counts: HashMap<String, (usize, Vec<String>)> = HashMap::new();

    for sentence in sentences {
        let words: Vec<String> = tokenize_words(sentence)
            .into_iter()
            .map(|w| w.to_lowercase())
            .collect();

        for n in 1..=max_n {
            for window in words.windows(n) {
                // Skip n-grams starting or ending with a stopword (for n > 1)
                if n > 1 && (is_stopword(&window[0]) || is_stopword(&window[n - 1])) {
                    continue;
                }
                // Skip if every word is a stopword
                if window.iter().all(|w| is_stopword(w)) {
                    continue;
                }

                let ngram = window.join(" ");
                let entry = ngram_counts
                    .entry(ngram)
                    .or_insert_with(|| (0, window.to_vec()));
                entry.0 += 1;
            }
        }
    }

    ngram_counts
        .into_iter()
        .map(|(text, (count, words))| (text, count, words))
        .collect()
}

fn score_ngrams(
    ngrams: &[(String, usize, Vec<String>)],
    word_scores: &HashMap<String, f64>,
) -> Vec<Keyphrase> {
    let mut keyphrases = Vec::new();

    for (text, count, words) in ngrams {
        let word_count = words.len();

        let constituent_scores: Vec<f64> = words
            .iter()
            .filter_map(|w| {
                if is_stopword(w) {
                    None
                } else {
                    Some(*word_scores.get(w).unwrap_or(&1.0))
                }
            })
            .collect();

        if constituent_scores.is_empty() {
            continue;
        }

        let score = if word_count == 1 {
            constituent_scores[0]
        } else {
            // S(kw) = product(S(wi)) / (TF(kw) * (1 + sum(S(wi))))
            let product: f64 = constituent_scores.iter().product();
            let sum: f64 = constituent_scores.iter().sum();
            let tf = *count as f64;
            product / (tf * (1.0 + sum))
        };

        keyphrases.push(Keyphrase {
            text: text.clone(),
            score,
            word_count,
        });
    }

    keyphrases
}

/// Jaccard-based deduplication: among overlapping keyphrases, keep the better-scored one.
fn deduplicate_keyphrases(keyphrases: &mut Vec<Keyphrase>, threshold: f64) {
    let mut to_remove = HashSet::new();

    for i in 0..keyphrases.len() {
        if to_remove.contains(&i) {
            continue;
        }
        for j in (i + 1)..keyphrases.len() {
            if to_remove.contains(&j) {
                continue;
            }
            let a_words: HashSet<&str> = keyphrases[i].text.split_whitespace().collect();
            let b_words: HashSet<&str> = keyphrases[j].text.split_whitespace().collect();

            let intersection = a_words.intersection(&b_words).count() as f64;
            let union = a_words.union(&b_words).count() as f64;

            if union > 0.0 && (intersection / union) >= threshold {
                if keyphrases[i].score <= keyphrases[j].score {
                    to_remove.insert(j);
                } else {
                    to_remove.insert(i);
                }
            }
        }
    }

    let mut idx = 0;
    keyphrases.retain(|_| {
        let keep = !to_remove.contains(&idx);
        idx += 1;
        keep
    });
}

// ── Public API ───────────────────────────────────────────────────────────

/// Extract keyphrases from text using YAKE algorithm.
/// Returns keyphrases sorted by score (lower = more relevant).
pub fn extract_keyphrases(text: &str, config: &YakeConfig) -> Vec<Keyphrase> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }

    let sentences = split_sentences(text);
    if sentences.is_empty() {
        return Vec::new();
    }

    let word_stats = compute_word_stats(&sentences, config.window_size);
    let word_scores = compute_word_scores(&word_stats, sentences.len());
    let ngrams = extract_ngrams(&sentences, config.max_ngram_size);
    let mut keyphrases = score_ngrams(&ngrams, &word_scores);

    // Sort by score ascending (lower = better)
    keyphrases.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    deduplicate_keyphrases(&mut keyphrases, config.dedup_threshold);
    keyphrases.truncate(config.top_k);

    keyphrases
}

/// Generate a title from text by extracting top keyphrases.
/// Returns a title-cased string of top 2-3 keyphrases joined with " — ".
/// Falls back to first non-trivial sentence for very short text (<10 words).
pub fn generate_title(text: &str) -> String {
    let word_count = text.split_whitespace().count();

    if word_count < 10 {
        let first_line = text
            .lines()
            .find(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .unwrap_or(text);
        let truncated: String = first_line.chars().take(60).collect();
        return title_case(&truncated);
    }

    let config = YakeConfig {
        max_ngram_size: 3,
        top_k: 5,
        ..Default::default()
    };

    let keyphrases = extract_keyphrases(text, &config);

    if keyphrases.is_empty() {
        let first_line = text
            .lines()
            .find(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .unwrap_or(text);
        let truncated: String = first_line.chars().take(60).collect();
        return title_case(&truncated);
    }

    // Take top 2-3 keyphrases, preferring multi-word, avoiding redundancy
    let mut title_parts: Vec<&str> = Vec::new();
    let mut total_words = 0;

    for kp in &keyphrases {
        if total_words + kp.word_count > 6 {
            break;
        }
        let already_covered = title_parts.iter().any(|p| p.contains(&kp.text));
        if !already_covered {
            title_parts.push(&kp.text);
            total_words += kp.word_count;
        }
        if title_parts.len() >= 3 {
            break;
        }
    }

    if title_parts.is_empty() {
        return "Untitled".to_string();
    }

    title_case(&title_parts.join(" — "))
}

/// Extract tags from text using YAKE keyphrases.
/// Returns lowercase-hyphenated tags (1-2 word keyphrases only).
pub fn extract_tags(text: &str, max_tags: usize) -> Vec<String> {
    let config = YakeConfig {
        max_ngram_size: 2,
        top_k: max_tags * 2,
        ..Default::default()
    };

    let keyphrases = extract_keyphrases(text, &config);

    keyphrases
        .into_iter()
        .filter(|kp| kp.word_count <= 2)
        .take(max_tags)
        .map(|kp| kp.text.replace(' ', "-"))
        .collect()
}

/// Convert a string to title case, leaving small connector words lowercase.
fn title_case(s: &str) -> String {
    const SMALL_WORDS: &[&str] = &[
        "a", "an", "the", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with",
        "by", "\u{2014}", // em-dash
    ];

    s.split_whitespace()
        .map(|word| {
            let lower = word.to_lowercase();
            if SMALL_WORDS.contains(&lower.as_str()) {
                return lower;
            }
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_sentences() {
        let text = "First sentence here. Second sentence here. Third one!";
        let sentences = split_sentences(text);
        assert_eq!(sentences.len(), 3);
    }

    #[test]
    fn test_tokenize_words() {
        let text = "Hello, world! This is a test.";
        let words = tokenize_words(text);
        assert!(words.contains(&"Hello".to_string()));
        assert!(words.contains(&"world".to_string()));
        assert!(words.contains(&"test".to_string()));
    }

    #[test]
    fn test_stopword_detection() {
        assert!(is_stopword("the"));
        assert!(is_stopword("The"));
        assert!(is_stopword("and"));
        assert!(!is_stopword("algorithm"));
        assert!(!is_stopword("rust"));
    }

    #[test]
    fn test_extract_keyphrases_basic() {
        let text = "Machine learning algorithms are used for natural language processing. \
                    Deep learning models improve natural language understanding. \
                    Neural networks power modern machine learning systems.";
        let config = YakeConfig::default();
        let keyphrases = extract_keyphrases(text, &config);

        assert!(!keyphrases.is_empty());
        let texts: Vec<&str> = keyphrases.iter().map(|k| k.text.as_str()).collect();
        assert!(texts
            .iter()
            .any(|t| t.contains("learning") || t.contains("language") || t.contains("neural")));
    }

    #[test]
    fn test_extract_keyphrases_empty() {
        let config = YakeConfig::default();
        let keyphrases = extract_keyphrases("", &config);
        assert!(keyphrases.is_empty());
    }

    #[test]
    fn test_generate_title() {
        let text = "Rust programming language provides memory safety guarantees. \
                    The borrow checker ensures memory safety at compile time. \
                    Rust ownership model prevents data races and memory leaks.";
        let title = generate_title(text);
        assert!(!title.is_empty());
        assert!(title.len() <= 100);
    }

    #[test]
    fn test_generate_title_short_text() {
        let title = generate_title("Short text");
        assert_eq!(title, "Short Text");
    }

    #[test]
    fn test_extract_tags() {
        let text = "Quantum computing uses quantum mechanics principles. \
                    Quantum bits or qubits enable quantum parallelism. \
                    Quantum entanglement allows quantum communication.";
        let tags = extract_tags(text, 5);
        assert!(!tags.is_empty());
        for tag in &tags {
            assert_eq!(*tag, tag.to_lowercase());
            assert!(!tag.contains(' '));
        }
    }

    #[test]
    fn test_title_case() {
        assert_eq!(title_case("hello world"), "Hello World");
        assert_eq!(title_case("the art of war"), "the Art of War");
    }

    #[test]
    fn test_keyphrase_scores_ordered() {
        let text = "Database indexing improves query performance significantly. \
                    B-tree indexes support range queries efficiently. \
                    Hash indexes provide constant-time lookups for exact matches. \
                    Database query optimization relies on proper indexing strategies.";
        let config = YakeConfig::default();
        let keyphrases = extract_keyphrases(text, &config);

        // Scores should be in ascending order (lower = better)
        for i in 1..keyphrases.len() {
            assert!(
                keyphrases[i].score >= keyphrases[i - 1].score,
                "Keyphrases not sorted: {:?} (score {}) came after {:?} (score {})",
                keyphrases[i].text,
                keyphrases[i].score,
                keyphrases[i - 1].text,
                keyphrases[i - 1].score,
            );
        }
    }

    #[test]
    fn test_deduplication() {
        let mut keyphrases = vec![
            Keyphrase { text: "machine learning".into(), score: 0.1, word_count: 2 },
            Keyphrase { text: "machine learning algorithms".into(), score: 0.2, word_count: 3 },
            Keyphrase { text: "deep learning".into(), score: 0.15, word_count: 2 },
        ];
        // "machine learning" and "machine learning algorithms" share 2/3 words = 0.67 Jaccard
        // With threshold 0.5, they should be deduplicated
        deduplicate_keyphrases(&mut keyphrases, 0.5);
        assert!(keyphrases.len() <= 2);
    }
}
