//! Bound OSC payloads before passing them to vte's std-backed OSC buffer.
//! A producer can omit the terminator indefinitely; bounding just our output
//! ring would not bound the parser's allocation. Raw capture remains separate.

const MAX_OSC_BYTES: usize = 4096;

#[derive(Default)]
pub(super) struct OscGuard {
    escape: bool,
    length: Option<usize>,
    discarding: bool,
    pub discarded: u64,
}

impl OscGuard {
    pub fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            if let Some(length) = self.length.as_mut() {
                match byte {
                    0x07 | 0x18 | 0x1a | 0x1b => {
                        self.length = None;
                        self.discarding = false;
                        self.escape = byte == 0x1b;
                        output.push(byte);
                    }
                    _ if self.discarding => {}
                    _ => {
                        *length += 1;
                        if *length > MAX_OSC_BYTES {
                            // CAN resets the parser without interpreting the
                            // rest of an oversized title/clipboard as text.
                            output.push(0x18);
                            self.discarding = true;
                            self.discarded = self.discarded.saturating_add(1);
                        } else {
                            output.push(byte);
                        }
                    }
                }
            } else {
                if self.escape && byte == b']' {
                    self.length = Some(0);
                }
                self.escape = byte == 0x1b;
                output.push(byte);
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unterminated_osc_is_bounded_and_screen_recovers_after_terminator() {
        let mut guard = OscGuard::default();
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(&guard.filter(b"before\x1b]0;"));
        let mut forwarded = 0;
        for _ in 0..4096 {
            let filtered = guard.filter(&[b'x'; 4096]);
            forwarded += filtered.len();
            parser.process(&filtered);
        }
        assert!(forwarded <= MAX_OSC_BYTES + 1);
        assert_eq!(guard.discarded, 1);
        parser.process(&guard.filter(b"\x1b\\after"));
        assert_eq!(parser.screen().contents(), "beforeafter");
    }

    #[test]
    fn ordinary_split_osc_is_preserved_exactly() {
        let mut guard = OscGuard::default();
        let first = b"\x1b";
        let second = b"]11;?\x1b";
        let third = b"\\text\x1b[2J";
        for bytes in [first.as_slice(), second.as_slice(), third.as_slice()] {
            assert_eq!(guard.filter(bytes), bytes);
        }
        assert_eq!(guard.discarded, 0);
    }
}
