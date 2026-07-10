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
    ///
    /// Truly-malformed bytes (e.g. 0xFF, or a broken continuation sequence,
    /// where `Utf8Error::error_len()` is `Some(n)`) are replaced with a
    /// single U+FFFD each and decoding continues past them — otherwise a bad
    /// byte at the front of the buffer would never be consumed and would pin
    /// the buffer forever, stalling all subsequent output while the raw
    /// bytes accumulate unbounded until end-of-stream.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);

        let mut output = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    output.push_str(text);
                    self.pending.clear();
                    return output;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    // Bytes [0, valid_up_to) are guaranteed valid UTF-8 by
                    // the error itself, so this cannot panic.
                    output.push_str(
                        std::str::from_utf8(&self.pending[..valid_up_to])
                            .expect("bytes up to valid_up_to are valid UTF-8"),
                    );

                    match error.error_len() {
                        // Definitively malformed sequence of `n` bytes:
                        // replace it and keep decoding the remainder.
                        Some(invalid_len) => {
                            output.push('\u{FFFD}');
                            self.pending.drain(..valid_up_to + invalid_len);
                        }
                        // Incomplete trailing sequence: might be completed
                        // by the next network chunk, so carry it over.
                        None => {
                            self.pending.drain(..valid_up_to);
                            return output;
                        }
                    }
                }
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

    #[test]
    fn malformed_byte_mid_stream_yields_replacement_and_decoding_continues() {
        // E6 97 = truncated "日" (invalid: interrupted by a non-continuation
        // byte), FF = never valid in UTF-8, E5 A5 BD = "好". The malformed
        // prefix must become replacement char(s) and decoding must continue
        // within the same push — the valid "好" must come through.
        let mut buf = Utf8ChunkBuffer::new();
        let out = buf.push(&[0xE6, 0x97, 0xFF, 0xE5, 0xA5, 0xBD]);
        assert!(
            out.contains('好'),
            "decoding must continue past malformed bytes, got {:?}",
            out
        );
        assert!(
            out.contains('\u{FFFD}'),
            "malformed bytes must yield a replacement char, got {:?}",
            out
        );
        assert_eq!(
            buf.flush(),
            "",
            "nothing should remain pinned in the buffer"
        );
    }

    #[test]
    fn lone_invalid_byte_does_not_stall_subsequent_pushes() {
        // A truly-malformed byte (0xFF) at the front of the buffer must not
        // pin it: later valid pushes must flow through immediately instead of
        // accumulating unbounded until flush.
        let mut buf = Utf8ChunkBuffer::new();
        let first = buf.push(&[0xFF]);
        assert_eq!(first, "\u{FFFD}", "invalid byte must be consumed as U+FFFD");

        let second = buf.push(b"hello");
        assert_eq!(
            second, "hello",
            "valid ASCII must not be held hostage by a prior bad byte"
        );

        let third = buf.push(b" world");
        assert_eq!(third, " world");
        assert_eq!(buf.flush(), "");
    }
}
