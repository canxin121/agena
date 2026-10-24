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
