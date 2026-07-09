//! Incremental UTF-8 decoding across network chunk boundaries.
//!
//! Streaming HTTP responses (OpenRouter SSE, Ollama NDJSON) arrive as
//! arbitrary byte slices from the underlying TCP stream. A multibyte UTF-8
//! character (CJK, emoji, etc.) can be split across two `bytes_stream()`
//! chunks. Decoding each chunk independently with `String::from_utf8_lossy`
//! replaces the dangling partial sequence at the end of the first chunk with
//! U+FFFD before it ever gets a chance to be joined with the continuation
//! bytes in the next chunk — permanently corrupting the stored response.
//!
//! `Utf8ChunkBuffer` fixes this by buffering raw bytes and only decoding the
//! prefix that is guaranteed valid UTF-8, carrying over any incomplete
//! trailing sequence to be completed by the next push.

/// Buffers raw bytes across pushes and decodes as much valid UTF-8 as
/// possible on each call, retaining any incomplete trailing sequence
/// internally instead of lossily replacing it.
#[derive(Debug, Default)]
pub struct Utf8ChunkBuffer {
    pending: Vec<u8>,
}

impl Utf8ChunkBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push newly received bytes and return as much valid UTF-8 text as can
    /// be decoded so far. Any trailing incomplete multi-byte sequence is
    /// retained and prepended to the next `push` call.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);

        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                let text = text.to_string();
                self.pending.clear();
                text
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                // Bytes [0, valid_up_to) are guaranteed valid UTF-8 by the
                // error itself, so this cannot panic.
                let text = std::str::from_utf8(&self.pending[..valid_up_to])
                    .expect("bytes up to valid_up_to are valid UTF-8")
                    .to_string();
                self.pending.drain(..valid_up_to);
                text
            }
        }
    }

    /// Flush any remaining buffered bytes at end-of-stream. Falls back to
    /// lossy decoding — by this point there is no further chunk to complete
    /// a dangling sequence, so any bytes still pending are genuinely
    /// truncated/invalid rather than merely chunk-split.
    pub fn flush(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_ascii_in_one_shot() {
        let mut buf = Utf8ChunkBuffer::new();
        assert_eq!(buf.push(b"hello world"), "hello world");
        assert_eq!(buf.flush(), "");
    }

    #[test]
    fn reassembles_multibyte_char_split_across_two_chunks() {
        // "日" (U+65E5) encodes to 3 bytes: E6 97 A5. Split mid-character.
        let full = "日本語".as_bytes().to_vec();
        let (first, second) = full.split_at(2);

        let mut buf = Utf8ChunkBuffer::new();
        let out1 = buf.push(first);
        let out2 = buf.push(second);
        let combined = format!("{}{}", out1, out2);

        assert_eq!(combined, "日本語");
        assert!(!combined.contains('\u{FFFD}'));
    }

    #[test]
    fn reassembles_emoji_split_across_chunks_at_every_byte_offset() {
        // Emoji commonly encode as 4-byte UTF-8 sequences.
        let full = "🎉party".as_bytes().to_vec();
        for split_at in 1..=3 {
            let (first, second) = full.split_at(split_at);
            let mut buf = Utf8ChunkBuffer::new();
            let out1 = buf.push(first);
            let out2 = buf.push(second);
            let combined = format!("{}{}", out1, out2);
            assert_eq!(combined, "🎉party", "split at byte {split_at}");
            assert!(!combined.contains('\u{FFFD}'), "split at byte {split_at}");
        }
    }

    #[test]
    fn handles_byte_by_byte_splits_across_a_longer_mixed_string() {
        let text = "Hello 世界! 🌍 emoji test with 日本語 mixed in.";
        let bytes = text.as_bytes();
        let mut buf = Utf8ChunkBuffer::new();
        let mut out = String::new();
        for byte in bytes {
            out.push_str(&buf.push(std::slice::from_ref(byte)));
        }
        out.push_str(&buf.flush());
        assert_eq!(out, text);
    }

    #[test]
    fn flush_lossily_decodes_genuinely_truncated_trailing_bytes() {
        // A lone continuation byte with no valid completion, e.g. stream cut
        // off mid-sequence at true end-of-stream.
        let mut buf = Utf8ChunkBuffer::new();
        buf.push(&[0xE6, 0x97]); // first two bytes of "日", never completed
        let flushed = buf.flush();
        assert!(flushed.contains('\u{FFFD}'));
    }
}
