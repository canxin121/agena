//! Stateful normalization for bytes written to a Windows pseudoconsole.
//!
//! ConPTY accepts UTF-8 input, but its key encoding differs from the byte a
//! caller naturally sends. Three rules, all matching `conhost` expectations:
//!
//! - An Enter key is a carriage return, so a line feed becomes one. A CRLF pair
//!   arrives as one newline even when the two bytes are split across writes,
//!   which is why the state is kept between calls.
//! - Backspace is encoded as DEL (`0x7f`), which ConPTY translates to `VK_BACK`;
//!   the C0 backspace byte is passed through as DEL instead.
//! - Every other byte, including UTF-8 sequences and terminal control bytes,
//!   passes through unchanged.
//!
//! The type is compiled on every platform so its behavior is unit-testable here;
//! only the Windows PTY driver uses it.

/// Normalizes input bytes for a Windows pseudoconsole.
#[derive(Debug, Default, Clone, Copy)]
pub struct TtyInputNormalizer {
    previous_was_cr: bool,
}

impl TtyInputNormalizer {
    /// Normalize a complete write, keeping CR/LF state across calls.
    pub fn normalize(&mut self, bytes: &[u8]) -> Vec<u8> {
        let (_, normalized) = self.normalize_bounded(bytes, usize::MAX);
        normalized
    }

    /// Normalize at most `limit` output bytes.
    ///
    /// Returns the number of *input* bytes consumed and their normalized form.
    /// The state machine only advances for consumed bytes, so a caller that
    /// queues bounded chunks still collapses a CRLF split across chunk edges.
    pub fn normalize_bounded(&mut self, bytes: &[u8], limit: usize) -> (usize, Vec<u8>) {
        let mut normalized = Vec::with_capacity(bytes.len().min(limit));
        let mut consumed = 0;
        for &byte in bytes {
            if normalized.len() >= limit {
                break;
            }
            consumed += 1;
            match byte {
                b'\x08' => normalized.push(b'\x7f'),
                b'\n' => {
                    if !self.previous_was_cr {
                        normalized.push(b'\r');
                    }
                }
                _ => normalized.push(byte),
            }
            self.previous_was_cr = byte == b'\r';
        }
        (consumed, normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::TtyInputNormalizer;

    #[test]
    fn line_feeds_become_carriage_returns() {
        let mut normalizer = TtyInputNormalizer::default();
        assert_eq!(normalizer.normalize(b"echo hi\n"), b"echo hi\r".to_vec());
    }

    #[test]
    fn crlf_collapses_including_across_writes() {
        let mut normalizer = TtyInputNormalizer::default();
        assert_eq!(normalizer.normalize(b"a\r\nb"), b"a\rb".to_vec());

        let mut split = TtyInputNormalizer::default();
        let first = split.normalize(b"a\r");
        let second = split.normalize(b"\nb");
        assert_eq!(first, b"a\r".to_vec());
        assert_eq!(second, b"b".to_vec());
    }

    #[test]
    fn backspace_is_sent_as_del() {
        let mut normalizer = TtyInputNormalizer::default();
        assert_eq!(normalizer.normalize(b"ab\x08c"), b"ab\x7fc".to_vec());
    }

    #[test]
    fn utf8_and_control_bytes_pass_through() {
        let mut normalizer = TtyInputNormalizer::default();
        let input = "你好\x03\x04\t\x1b[A".as_bytes();
        assert_eq!(normalizer.normalize(input), input.to_vec());
    }

    #[test]
    fn bounded_normalization_reports_consumed_input_bytes() {
        let mut normalizer = TtyInputNormalizer::default();
        let (consumed, normalized) = normalizer.normalize_bounded(b"abcd\nef", 4);
        assert_eq!(consumed, 4);
        assert_eq!(normalized, b"abcd".to_vec());

        // The caller retries with its remaining input, exactly as the Windows
        // PTY driver does after a WouldBlock acknowledgement.
        let (consumed, normalized) = normalizer.normalize_bounded(&b"abcd\nef"[4..], 4);
        assert_eq!(consumed, 3);
        assert_eq!(normalized, b"\ref".to_vec());
    }

    #[test]
    fn bounded_normalization_keeps_crlf_state_at_the_boundary() {
        let mut normalizer = TtyInputNormalizer::default();
        let (consumed, normalized) = normalizer.normalize_bounded(b"ab\r\ncd", 3);
        assert_eq!(consumed, 3);
        assert_eq!(normalized, b"ab\r".to_vec());

        let (consumed, normalized) = normalizer.normalize_bounded(b"\ncd", 8);
        assert_eq!(consumed, 3);
        assert_eq!(normalized, b"cd".to_vec());
    }

    #[test]
    fn empty_input_consumes_nothing() {
        let mut normalizer = TtyInputNormalizer::default();
        assert_eq!(normalizer.normalize_bounded(b"", 16), (0, Vec::new()));
    }
}
