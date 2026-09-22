//! Minimal, bounded terminal query replies. These are generated exclusively
//! from the virtual screen, never from the host terminal or system clipboard.

const MAX_REPLIES: usize = 4096;

#[derive(Default)]
pub(super) struct Protocol {
    replies: Vec<u8>,
    overflowed: bool,
}

impl Protocol {
    fn reply(&mut self, bytes: &[u8]) {
        if self.replies.len() + bytes.len() <= MAX_REPLIES {
            self.replies.extend_from_slice(bytes);
        } else {
            self.overflowed = true;
        }
    }

    pub fn take(&mut self) -> Result<Vec<u8>, &'static str> {
        if self.overflowed {
            return Err("terminal query flood exceeded the reply buffer");
        }
        Ok(std::mem::take(&mut self.replies))
    }
}

impl vt100::Callbacks for Protocol {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        if i2.is_some() {
            return;
        }
        match (i1, first, c) {
            (None, 5, 'n') => self.reply(b"\x1b[0n"),
            (None | Some(b'?'), 6, 'n') => {
                let (row, col) = screen.cursor_position();
                let private = if i1.is_some() { "?" } else { "" };
                self.reply(format!("\x1b[{private}{};{}R", row + 1, col + 1).as_bytes());
            }
            (None, 0, 'c') => self.reply(b"\x1b[?1;2c"),
            (Some(b'>'), 0, 'c') => self.reply(b"\x1b[>0;1;0c"),
            (None, 18, 't') => {
                let (rows, cols) = screen.size();
                self.reply(format!("\x1b[8;{rows};{cols}t").as_bytes());
            }
            _ => {}
        }
    }

    fn unhandled_escape(&mut self, _: &mut vt100::Screen, i1: Option<u8>, i2: Option<u8>, b: u8) {
        if (i1, i2, b) == (None, None, b'Z') {
            self.reply(b"\x1b[?1;2c");
        }
    }

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        // Virtual terminal colors only. Never forward arbitrary OSC requests,
        // hyperlinks, titles or clipboard actions to the host.
        match params {
            [b"10", b"?"] => self.reply(b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\"),
            [b"11", b"?"] => self.reply(b"\x1b]11;rgb:0000/0000/0000\x1b\\"),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_queries_reply_from_screen_and_ignore_clipboard() {
        let mut parser = vt100::Parser::new_with_callbacks(24, 80, 0, Protocol::default());
        parser.process(b"\x1b[3;9H\x1b[");
        parser.process(b"6n\x1b[5n\x1b[c\x1b[18t");
        assert_eq!(
            parser.callbacks_mut().take().unwrap(),
            b"\x1b[3;9R\x1b[0n\x1b[?1;2c\x1b[8;24;80t"
        );
        parser.process(b"\x1b]52;c;?\x07\x1b]52;c;c2VjcmV0\x07");
        assert!(parser.callbacks_mut().take().unwrap().is_empty());
    }

    #[test]
    fn queries_cannot_allocate_unbounded_replies() {
        let mut parser = vt100::Parser::new_with_callbacks(24, 80, 0, Protocol::default());
        for _ in 0..2000 {
            parser.process(b"\x1b[6n");
        }
        assert!(parser.callbacks_mut().take().is_err());
    }
}
