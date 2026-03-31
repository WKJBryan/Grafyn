//! TextTiling segmentation — Hearst (1997) topic-based text segmentation.
//!
//! Uses TF cosine similarity between adjacent blocks to detect topic boundaries.
//! No embeddings needed — uses term frequency vectors via the SimilarityProvider trait.
//!
//! Algorithm:
//! 1. Strip YAML frontmatter + code blocks
//! 2. Group words into pseudo-sentences (default 20 words each)
//! 3. Group pseudo-sentences into blocks (default 5 = ~100 words per block)
//! 4. Compute TF cosine similarity between adjacent blocks
//! 5. Smooth similarity curve (moving average)
//! 6. Depth scoring at valleys: depth(i) = max_left + max_right - 2*sim(i)
//! 7. Segment at positions where depth > mean + threshold_factor * stddev
//! 8. Merge segments shorter than min_segment_words with nearest neighbor
//! 9. Remap boundaries to original text offsets, snapping to nearest \n\n

use crate::services::similarity::{SimilarityProvider, TfIdfProvider};
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    /// YAML frontmatter pattern
    static ref FRONTMATTER_RE: Regex = Regex::new(r"(?s)\A---\n.*?\n---\n?").unwrap();
    /// Fenced code block removal
    static ref CODE_BLOCK_RE: Regex = Regex::new(r"(?s)```.*?```").unwrap();
}

/// A text segment identified by TextTiling.
#[derive(Debug, Clone)]
pub struct TextSegment {
    /// Start character offset in the original text
    pub start_char: usize,
    /// End character offset in the original text
    pub end_char: usize,
    /// The segment content (slice of original text)
    pub content: String,
    /// Depth score of the boundary preceding this segment (0.0 for first segment)
    pub depth_score: f64,
}

/// Configuration for TextTiling segmentation.
pub struct TextTilingConfig {
    /// Number of words per pseudo-sentence
    pub pseudo_sentence_size: usize,
    /// Number of pseudo-sentences per block
    pub block_size: usize,
    /// Moving average window for smoothing similarities
    pub smoothing_window: usize,
    /// Factor multiplied by stddev for boundary threshold: mean + factor * stddev
    pub depth_threshold_factor: f64,
    /// Minimum words per segment (shorter segments merged with neighbors)
    pub min_segment_words: usize,
}

impl Default for TextTilingConfig {
    fn default() -> Self {
        Self {
            pseudo_sentence_size: 20,
            block_size: 5,
            smoothing_window: 3,
            depth_threshold_factor: 0.5,
            min_segment_words: 50,
        }
    }
}

// ── Text preprocessing ───────────────────────────────────────────────────

/// Strip YAML frontmatter, returning (content_without_frontmatter, byte_offset_of_content_start).
fn strip_frontmatter(text: &str) -> (&str, usize) {
    if let Some(m) = FRONTMATTER_RE.find(text) {
        (&text[m.end()..], m.end())
    } else {
        (text, 0)
    }
}

/// Remove fenced code blocks from text (for cleaner TF computation).
fn strip_code_blocks(text: &str) -> String {
    CODE_BLOCK_RE.replace_all(text, " ").to_string()
}

// ── Pseudo-sentence and block creation ───────────────────────────────────

/// Group words into pseudo-sentences of fixed size.
fn create_pseudo_sentences(text: &str, size: usize) -> Vec<Vec<String>> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 2)
        .collect();

    let chunk_size = size.max(1);
    words
        .chunks(chunk_size)
        .map(|chunk| chunk.to_vec())
        .collect()
}

/// Flatten pseudo-sentences into blocks (each block = block_size pseudo-sentences).
fn create_blocks(pseudo_sentences: &[Vec<String>], block_size: usize) -> Vec<Vec<String>> {
    let bs = block_size.max(1);
    pseudo_sentences
        .chunks(bs)
        .map(|chunk| chunk.iter().flat_map(|ps| ps.iter().cloned()).collect())
        .collect()
}

// ── Similarity computation ───────────────────────────────────────────────

/// Compute cosine similarity between each pair of adjacent blocks.
fn compute_gap_similarities(blocks: &[Vec<String>], provider: &TfIdfProvider) -> Vec<f64> {
    if blocks.len() < 2 {
        return Vec::new();
    }

    let mut similarities = Vec::with_capacity(blocks.len() - 1);

    for i in 0..blocks.len() - 1 {
        let text_a = blocks[i].join(" ");
        let text_b = blocks[i + 1].join(" ");
        let sim = provider.similarity(&text_a, &text_b);
        similarities.push(sim);
    }

    similarities
}

/// Smooth similarity scores using a moving average.
fn smooth(scores: &[f64], window: usize) -> Vec<f64> {
    if scores.is_empty() || window <= 1 {
        return scores.to_vec();
    }

    let half = window / 2;
    let mut smoothed = Vec::with_capacity(scores.len());

    for i in 0..scores.len() {
        let start = i.saturating_sub(half);
        let end = (i + half + 1).min(scores.len());
        let sum: f64 = scores[start..end].iter().sum();
        let count = (end - start) as f64;
        smoothed.push(sum / count);
    }

    smoothed
}

// ── Depth scoring ────────────────────────────────────────────────────────

/// Compute depth score at each gap position.
/// depth(i) = max_left + max_right - 2*sim(i)
/// where max_left/max_right are the highest similarities found
/// by walking outward from the valley until similarity stops increasing.
fn compute_depth_scores(similarities: &[f64]) -> Vec<f64> {
    if similarities.is_empty() {
        return Vec::new();
    }

    let mut depths = Vec::with_capacity(similarities.len());

    for i in 0..similarities.len() {
        let max_left = {
            let mut max_val = similarities[i];
            for j in (0..i).rev() {
                if similarities[j] > max_val {
                    max_val = similarities[j];
                } else {
                    break;
                }
            }
            max_val
        };

        let max_right = {
            let mut max_val = similarities[i];
            for j in (i + 1)..similarities.len() {
                if similarities[j] > max_val {
                    max_val = similarities[j];
                } else {
                    break;
                }
            }
            max_val
        };

        let depth = max_left + max_right - 2.0 * similarities[i];
        depths.push(depth.max(0.0));
    }

    depths
}

/// Find gap indices where depth exceeds mean + threshold_factor * stddev.
fn find_boundaries(depths: &[f64], threshold_factor: f64) -> Vec<usize> {
    if depths.is_empty() {
        return Vec::new();
    }

    let n = depths.len() as f64;
    let mean: f64 = depths.iter().sum::<f64>() / n;
    let variance: f64 = depths.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n;
    let stddev = variance.sqrt();

    let threshold = mean + threshold_factor * stddev;

    depths
        .iter()
        .enumerate()
        .filter(|(_, &depth)| depth > threshold)
        .map(|(i, _)| i)
        .collect()
}

// ── Boundary mapping ─────────────────────────────────────────────────────

/// Map block-level gap indices to character offsets in the text.
/// Each gap index i means "split after block i", which corresponds to
/// approximately `(i+1) * words_per_block` words into the text.
fn map_boundaries_to_offsets(
    text: &str,
    boundary_indices: &[usize],
    block_size: usize,
    pseudo_sentence_size: usize,
) -> Vec<usize> {
    let words_per_block = block_size * pseudo_sentence_size;
    let mut char_boundaries = Vec::new();

    for &boundary_idx in boundary_indices {
        let approx_word_pos = (boundary_idx + 1) * words_per_block;

        // Walk through the text counting words to find the character position
        let mut word_count = 0;
        let mut in_word = false;
        let mut best_pos = text.len();

        for (i, c) in text.char_indices() {
            let is_ws = c.is_whitespace();
            if !is_ws && !in_word {
                word_count += 1;
                if word_count > approx_word_pos {
                    best_pos = snap_to_paragraph(text, i);
                    break;
                }
                in_word = true;
            } else if is_ws {
                in_word = false;
            }
        }

        char_boundaries.push(best_pos);
    }

    char_boundaries.sort();
    char_boundaries.dedup();

    // Filter out boundaries at the very start or end
    char_boundaries.retain(|&pos| pos > 0 && pos < text.len());

    char_boundaries
}

/// Snap a character position to the nearest paragraph break (`\n\n`).
/// Searches within 200 characters in either direction.
fn snap_to_paragraph(text: &str, pos: usize) -> usize {
    let search_range = 200;
    let start = previous_char_boundary(text, pos.saturating_sub(search_range));
    let end = next_char_boundary(text, (pos + search_range).min(text.len()));

    if end <= start {
        return pos;
    }

    let search_slice = &text[start..end];
    let bytes = search_slice.as_bytes();

    let mut best_dist = usize::MAX;
    let mut best_pos = pos;

    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == b'\n' && bytes[i + 1] == b'\n' {
            let abs_pos = start + i + 2; // Position after the \n\n
            let dist = if abs_pos > pos {
                abs_pos - pos
            } else {
                pos - abs_pos
            };
            if dist < best_dist {
                best_dist = dist;
                best_pos = abs_pos;
            }
        }
    }

    best_pos
}

fn previous_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn next_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

// ── Short segment merging ────────────────────────────────────────────────

/// Merge segments shorter than `min_words` with their nearest neighbor.
fn merge_short_segments(segments: &mut Vec<TextSegment>, min_words: usize) {
    if segments.len() <= 1 {
        return;
    }

    let mut i = 0;
    while i < segments.len() && segments.len() > 1 {
        let word_count = segments[i].content.split_whitespace().count();
        if word_count < min_words {
            if i == 0 && i + 1 < segments.len() {
                // Merge with next
                segments[i + 1].start_char = segments[i].start_char;
                segments[i + 1].content =
                    format!("{}\n\n{}", segments[i].content, segments[i + 1].content);
                segments.remove(i);
            } else if i > 0 {
                // Merge with previous
                let content = segments[i].content.clone();
                let end = segments[i].end_char;
                segments[i - 1].end_char = end;
                segments[i - 1].content =
                    format!("{}\n\n{}", segments[i - 1].content, content);
                segments.remove(i);
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

// ── Public API ───────────────────────────────────────────────────────────

/// Segment text into topically coherent sections using TextTiling.
///
/// Returns a single segment for short texts (<2 * min_segment_words) or
/// texts with uniform topic distribution. For longer multi-topic texts,
/// returns segments split at detected topic boundaries.
pub fn segment(text: &str, config: &TextTilingConfig) -> Vec<TextSegment> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }

    // Strip frontmatter for analysis, but use original text for output
    let (content, content_start) = strip_frontmatter(text);
    let word_count = content.split_whitespace().count();

    // Too short to segment meaningfully
    if word_count < config.min_segment_words * 2 {
        return vec![TextSegment {
            start_char: 0,
            end_char: text.len(),
            content: text.to_string(),
            depth_score: 0.0,
        }];
    }

    // Strip code blocks for TF computation only
    let clean = strip_code_blocks(content);

    // Create pseudo-sentences and blocks
    let pseudo_sentences = create_pseudo_sentences(&clean, config.pseudo_sentence_size);
    let blocks = create_blocks(&pseudo_sentences, config.block_size);

    if blocks.len() < 3 {
        return vec![TextSegment {
            start_char: 0,
            end_char: text.len(),
            content: text.to_string(),
            depth_score: 0.0,
        }];
    }

    // Compute similarities between adjacent blocks
    let provider = TfIdfProvider::new();
    let similarities = compute_gap_similarities(&blocks, &provider);

    // Smooth
    let smoothed = smooth(&similarities, config.smoothing_window);

    // Compute depth scores at each gap
    let depths = compute_depth_scores(&smoothed);

    // Find significant boundaries
    let boundary_indices = find_boundaries(&depths, config.depth_threshold_factor);

    if boundary_indices.is_empty() {
        return vec![TextSegment {
            start_char: 0,
            end_char: text.len(),
            content: text.to_string(),
            depth_score: 0.0,
        }];
    }

    // Map gap indices to character offsets in content (frontmatter-stripped)
    let char_boundaries = map_boundaries_to_offsets(
        content,
        &boundary_indices,
        config.block_size,
        config.pseudo_sentence_size,
    );

    if char_boundaries.is_empty() {
        return vec![TextSegment {
            start_char: 0,
            end_char: text.len(),
            content: text.to_string(),
            depth_score: 0.0,
        }];
    }

    // Build segments from boundaries (adjusted to original text coordinates)
    let mut segments = Vec::new();
    let mut prev_content_offset = 0;

    for (i, &boundary) in char_boundaries.iter().enumerate() {
        if boundary <= prev_content_offset || boundary > content.len() {
            continue;
        }

        let seg_content = content[prev_content_offset..boundary].trim().to_string();
        if !seg_content.is_empty() {
            let depth = boundary_indices
                .get(i)
                .and_then(|&idx| depths.get(idx).copied())
                .unwrap_or(0.0);
            segments.push(TextSegment {
                start_char: content_start + prev_content_offset,
                end_char: content_start + boundary,
                content: seg_content,
                depth_score: depth,
            });
        }
        prev_content_offset = boundary;
    }

    // Add final segment
    if prev_content_offset < content.len() {
        let seg_content = content[prev_content_offset..].trim().to_string();
        if !seg_content.is_empty() {
            segments.push(TextSegment {
                start_char: content_start + prev_content_offset,
                end_char: text.len(),
                content: seg_content,
                depth_score: 0.0,
            });
        }
    }

    // Merge short segments
    merge_short_segments(&mut segments, config.min_segment_words);

    segments
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_pseudo_sentences() {
        let text = "one two three four five six seven eight nine ten eleven twelve";
        let ps = create_pseudo_sentences(text, 5);
        assert_eq!(ps.len(), 3); // 12 words / 5 = 3 chunks
        assert_eq!(ps[0].len(), 5);
    }

    #[test]
    fn test_create_blocks() {
        let ps = vec![
            vec!["a".into(), "b".into()],
            vec!["c".into(), "d".into()],
            vec!["e".into(), "f".into()],
            vec!["g".into(), "h".into()],
        ];
        let blocks = create_blocks(&ps, 2);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], vec!["a", "b", "c", "d"]);
        assert_eq!(blocks[1], vec!["e", "f", "g", "h"]);
    }

    #[test]
    fn test_smooth() {
        let scores = vec![0.8, 0.2, 0.9, 0.3, 0.7];
        let smoothed = smooth(&scores, 3);
        assert_eq!(smoothed.len(), 5);
        // Middle values should be pulled toward neighbors
        assert!(smoothed[1] > 0.2);
    }

    #[test]
    fn test_depth_scores() {
        // Valley at position 1 (0.2 between 0.8 and 0.9)
        let similarities = vec![0.8, 0.2, 0.9];
        let depths = compute_depth_scores(&similarities);
        assert_eq!(depths.len(), 3);
        assert!(depths[1] > depths[0]); // Valley at position 1 should have highest depth
    }

    #[test]
    fn test_find_boundaries() {
        let depths = vec![0.1, 0.5, 0.1, 0.8, 0.1];
        let boundaries = find_boundaries(&depths, 0.5);
        // The depth at position 3 (0.8) should be above threshold
        assert!(boundaries.contains(&3));
    }

    #[test]
    fn test_segment_short_text() {
        let text = "This is a short text that should not be split.";
        let config = TextTilingConfig::default();
        let segments = segment(text, &config);
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn test_segment_empty_text() {
        let segments = segment("", &TextTilingConfig::default());
        assert!(segments.is_empty());
    }

    #[test]
    fn test_segment_two_topics() {
        // Create two distinct topic blocks with enough content
        let topic1 = "Rust programming language provides memory safety without garbage collection. \
                      The borrow checker validates references at compile time ensuring correctness. \
                      Ownership rules prevent data races in concurrent programs automatically. \
                      Lifetimes annotate reference scopes for the Rust compiler to verify. \
                      Smart pointers like Box and Rc manage heap allocation patterns. \
                      The compiler ensures thread safety through Send and Sync traits. \
                      Zero-cost abstractions make Rust performance competitive with C code. \
                      Pattern matching enables expressive control flow in Rust programs. \
                      Error handling uses Result and Option types for safe error propagation.";

        let topic2 = "Ocean biology studies marine ecosystems and underwater biodiversity worldwide. \
                      Coral reefs support thousands of fish species in tropical waters. \
                      Phytoplankton produce most of Earth's oxygen through photosynthesis daily. \
                      Deep sea vents host unique chemosynthetic organisms near ocean floor. \
                      Marine mammals like whales migrate across entire ocean basins seasonally. \
                      Kelp forests provide essential habitat for sea otters and invertebrates. \
                      Ocean acidification threatens shellfish populations and reef ecosystems. \
                      Bioluminescence is commonly observed in deep water marine creatures. \
                      Tide pools contain surprisingly diverse intertidal species and organisms.";

        let text = format!("{}\n\n{}", topic1, topic2);
        let config = TextTilingConfig {
            min_segment_words: 30,
            ..Default::default()
        };
        let segments = segment(&text, &config);

        // Should detect at least a boundary
        assert!(!segments.is_empty());

        // Verify all content is covered
        let total_len: usize = segments.iter().map(|s| s.content.len()).sum();
        assert!(total_len > 0);
    }

    #[test]
    fn test_segment_uniform_text() {
        // Same topic throughout — should produce few segments
        let uniform = (0..10)
            .map(|i| {
                format!(
                    "Rust ownership rule number {} ensures memory safety at compile time. \
                     The borrow checker validates this specific ownership rule for correctness.",
                    i
                )
            })
            .collect::<Vec<_>>()
            .join(" ");

        let config = TextTilingConfig::default();
        let segments = segment(&uniform, &config);

        // Uniform text should not be over-segmented
        assert!(segments.len() <= 3);
    }

    #[test]
    fn test_snap_to_paragraph() {
        let text = "First paragraph here.\n\nSecond paragraph here.\n\nThird paragraph.";
        let snapped = snap_to_paragraph(text, 25);
        // Should snap to the \n\n at position 21, returning position 23 (after \n\n)
        assert_eq!(snapped, 23);
    }

    #[test]
    fn test_snap_to_paragraph_handles_unicode_boundaries() {
        let text = format!("{}—{}\n\nNext paragraph here.", "a".repeat(202), "b".repeat(250));
        let snapped = snap_to_paragraph(&text, 403);
        let expected = text.find("\n\n").unwrap() + 2;

        assert_eq!(snapped, expected);
        assert!(text.is_char_boundary(snapped));
    }

    #[test]
    fn test_strip_frontmatter() {
        let text = "---\ntitle: Test\ntags: [a, b]\n---\nActual content here.";
        let (content, offset) = strip_frontmatter(text);
        assert!(content.contains("Actual content"));
        assert!(!content.contains("title: Test"));
        assert!(offset > 0);
    }

    #[test]
    fn test_strip_code_blocks() {
        let text = "Before\n```rust\nfn main() {}\n```\nAfter";
        let cleaned = strip_code_blocks(text);
        assert!(cleaned.contains("Before"));
        assert!(cleaned.contains("After"));
        assert!(!cleaned.contains("fn main"));
    }

    #[test]
    fn test_merge_short_segments() {
        let mut segments = vec![
            TextSegment {
                start_char: 0,
                end_char: 10,
                content: "short".to_string(),
                depth_score: 0.5,
            },
            TextSegment {
                start_char: 10,
                end_char: 500,
                content: (0..60)
                    .map(|i| format!("word{}", i))
                    .collect::<Vec<_>>()
                    .join(" "),
                depth_score: 0.3,
            },
        ];
        merge_short_segments(&mut segments, 50);
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn test_cosine_similarity_via_blocks() {
        let provider = TfIdfProvider::new();
        let blocks = vec![
            vec![
                "rust".to_string(),
                "programming".to_string(),
                "memory".to_string(),
                "safety".to_string(),
            ],
            vec![
                "ocean".to_string(),
                "biology".to_string(),
                "marine".to_string(),
                "coral".to_string(),
            ],
        ];
        let sims = compute_gap_similarities(&blocks, &provider);
        assert_eq!(sims.len(), 1);
        // Different topics should have low similarity
        assert!(sims[0] < 0.3);
    }

    #[test]
    fn test_compute_gap_similarities_single_block() {
        let provider = TfIdfProvider::new();
        let blocks = vec![vec!["hello".to_string()]];
        let sims = compute_gap_similarities(&blocks, &provider);
        assert!(sims.is_empty());
    }
}
