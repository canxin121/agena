//! Terminal stdio query parser module.
use std::{fmt::Write, time::Duration};

use crate::picker::{ProtocolType, STDIN_READ_TIMEOUT_MILLIS};

pub struct Parser {
    data: String,
    sequence: ResponseParseState,
}

#[derive(Debug, PartialEq)]
pub enum ResponseParseState {
    Unknown,
    CSIResponse,
    OSCResponse,
    KittyResponse,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Response {
    Kitty,
    Sixel,
    RectangularOps,
    CellSize(Option<(u16, u16)>),
    CursorPositionReport(u16, u16),
    Background(BackgroundColorQuery, u8, u8, u8),
    Status,
}

/// Protocol used to request the terminal's default background color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundColorQuery {
    /// The xterm-compatible dynamic background query (`OSC 11`).
    Osc11,
    /// iTerm2's documented default-background query (`OSC 4; -2`).
    Iterm2Osc4,
}

/// Extra query options
pub struct QueryStdioOptions {
    /// Timeout for the stdio query.
    pub timeout: Duration,
    /// Query for [Text Sizing Protocol]. The result can be checked by searching for
    /// [crate::picker::Capability::TextSizingProtocol] in [crate::picker::Picker::capabilities].
    ///
    /// [Text Sizing Protocol] <https://sw.kovidgoyal.net/kitty/text-sizing-protocol//>
    pub text_sizing_protocol: bool,
    /// Blacklist protocols from the detection query. Currently only kitty can be detected, so that
    /// is the only ProtocolType that can have any effect here.
    /// [`crate::picker::Picker`] currently sets ProtocolType::Kitty for WezTerm and Konsole.
    pub blacklist_protocols: Vec<ProtocolType>,
}

impl Default for QueryStdioOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_millis(STDIN_READ_TIMEOUT_MILLIS),
            text_sizing_protocol: false,
            blacklist_protocols: Vec::new(),
        }
    }
}

impl Default for Parser {
    fn default() -> Self {
        Parser {
            data: String::new(),
            sequence: ResponseParseState::Unknown,
        }
    }
}

impl Parser {
    pub fn new() -> Self {
        Parser {
            data: String::new(),
            sequence: ResponseParseState::Unknown,
        }
    }

    /// Tmux requires escapes to be escaped, and some special start/end sequences.
    ///
    /// Returns start, escape, and end for tmux wrapping.
    pub fn tmux_start_escape_end(is_tmux: bool) -> (&'static str, &'static str, &'static str) {
        match is_tmux {
            false => ("", "\x1b", ""),
            true => ("\x1bPtmux;", "\x1b\x1b", "\x1b\\"),
        }
    }

    pub fn query(is_tmux: bool, options: QueryStdioOptions) -> String {
        let (start, escape, end) = Parser::tmux_start_escape_end(is_tmux);

        let mut buf = String::with_capacity(100);
        buf.push_str(start);

        if !options.blacklist_protocols.contains(&ProtocolType::Kitty) {
            // Kitty graphics
            write!(buf, "{escape}_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA{escape}\\").unwrap();
        }

        if !options.blacklist_protocols.contains(&ProtocolType::Sixel) {
            // Device Attributes Report 1 (sixel support)
            write!(buf, "{escape}[c").unwrap();
        }

        // Font size in pixels
        write!(buf, "{escape}[16t").unwrap();

        // iTerm2 proprietary, unknown response, untested so far.
        //write!(buf, "{escape}[1337n").unwrap();

        const BEL: &str = "\u{7}";

        if options.text_sizing_protocol {
            // Send CPR (Cursor Position Report) and Text Sizing Protocol commands.
            // https://sw.kovidgoyal.net/kitty/text-sizing-protocol/#detecting-if-the-terminal-supports-this-protocol
            // We need to write a CPR, a resized space, and CPR again, to see if it moved the cursor
            // correctly with extra width.
            // Do it again for the scaling part of the protocol.
            // See [Picker::interpret_parser_responses] for how the responses are interpreted - it
            // differs slightly from the spec!
            write!(
                buf,
                "{escape}[6n{escape}]66;w=2; {BEL}{escape}[6n{escape}]66;s=2; {BEL}{escape}[6n"
            )
            .unwrap();
        }

        // End with Device Status Report, implemented by all terminals, ensure that there is some
        // response and we don't hang reading forever.
        write!(buf, "{escape}[5n").unwrap();

        write!(buf, "{end}").unwrap();
        buf
    }

    pub fn background_query(is_tmux: bool, query: BackgroundColorQuery) -> String {
        let (start, escape, end) = Parser::tmux_start_escape_end(is_tmux);
        let body = match query {
            BackgroundColorQuery::Osc11 => "11;?",
            BackgroundColorQuery::Iterm2Osc4 => "4;-2;?",
        };
        format!("{start}{escape}]{body}\u{7}{end}")
    }

    pub fn push(&mut self, next: char) -> Vec<Response> {
        match self.sequence {
            ResponseParseState::Unknown => {
                match (&self.data[..], next) {
                    (_, '\x1b') => {
                        // If the current sequence hasn't been identified yet, start a new one on Esc.
                        return self.restart();
                    }
                    ("_Gi=31", ';') => {
                        self.sequence = ResponseParseState::KittyResponse;
                    }

                    ("[", _) => {
                        self.sequence = ResponseParseState::CSIResponse;
                    }
                    ("]", _) => {
                        self.sequence = ResponseParseState::OSCResponse;
                    }
                    _ => {}
                };
                self.data.push(next);
            }
            ResponseParseState::CSIResponse => {
                if self.data == "[0" && next == 'n' {
                    self.restart();
                    return vec![Response::Status];
                }
                match next {
                    'c' if self.data.starts_with("[?") => {
                        let mut caps = vec![];
                        let inner: Vec<&str> = (self.data[2..]).split(';').collect();
                        for cap in inner {
                            match cap {
                                "4" => caps.push(Response::Sixel),
                                "28" => caps.push(Response::RectangularOps),
                                _ => {}
                            }
                        }
                        self.restart();
                        return caps;
                    }
                    't' => {
                        let mut cell_size = None;
                        let inner: Vec<&str> = self.data.split(';').collect();
                        if let [_, h, w] = inner[..] {
                            if let (Ok(h), Ok(w)) = (h.parse::<u16>(), w.parse::<u16>()) {
                                if w > 0 && h > 0 {
                                    cell_size = Some((w, h));
                                }
                            }
                        }
                        self.restart();
                        return vec![Response::CellSize(cell_size)];
                    }
                    'R' => {
                        let mut cursor_pos = None;
                        let inner: Vec<&str> = self.data[1..].split(';').collect();
                        if let [x, w] = inner[..] {
                            if let (Ok(x), Ok(y)) = (x.parse::<u16>(), w.parse::<u16>()) {
                                cursor_pos = Some((y, x));
                            }
                        }
                        if let Some((x, y)) = cursor_pos {
                            self.restart();
                            return vec![Response::CursorPositionReport(x, y)];
                        } else {
                            self.restart();
                            return vec![];
                        }
                    }
                    '\x1b' => {
                        // Give up?
                        return self.restart();
                    }
                    _ => {
                        self.data.push(next);
                    }
                };
            }
            ResponseParseState::OSCResponse => {
                self.data.push(next);
                if next == '\u{7}' || self.data.ends_with("\x1b\\") {
                    let Some((selector, rgb)) = self.data.split_once("rgb:") else {
                        return self.restart();
                    };
                    let query = match selector {
                        "]11;" => BackgroundColorQuery::Osc11,
                        "]4;-2;" => BackgroundColorQuery::Iterm2Osc4,
                        // Other OSC color replies (for example OSC 10's
                        // foreground) must never be mistaken for the default
                        // background merely because they also contain rgb:.
                        _ => return self.restart(),
                    };
                    let rgb = rgb.trim_matches(|c| c == '\x07' || c == '\x1b' || c == '\\');
                    let parts: Vec<&str> = rgb.split('/').collect();
                    if parts.len() != 3 {
                        return self.restart();
                    }
                    let (Some(r), Some(g), Some(b)) = (
                        parse_x11_color_component(parts[0]),
                        parse_x11_color_component(parts[1]),
                        parse_x11_color_component(parts[2]),
                    ) else {
                        return self.restart();
                    };
                    self.restart();
                    return vec![Response::Background(query, r, g, b)];
                }
            }
            ResponseParseState::KittyResponse => match next {
                '\\' => {
                    let caps = match &self.data[..] {
                        "_Gi=31;OK\x1b" => vec![Response::Kitty],
                        _ => vec![],
                    };
                    self.restart();
                    return caps;
                }
                _ => {
                    self.data.push(next);
                }
            },
        };
        vec![]
    }
    fn restart(&mut self) -> Vec<Response> {
        self.data = String::new();
        self.sequence = ResponseParseState::Unknown;
        vec![]
    }
}

/// X11 color responses use one to four hexadecimal digits per component.
/// iTerm2 specifically documents both two- and four-digit replies. Normalize
/// the complete component range instead of assuming 16-bit values: shifting a
/// two-digit `ff` by eight bits incorrectly turns white into black.
fn parse_x11_color_component(component: &str) -> Option<u8> {
    if component.is_empty() || component.len() > 4 {
        return None;
    }
    let value = u32::from_str_radix(component, 16).ok()?;
    let maximum = (1_u32 << (component.len() * 4)) - 1;
    Some(((value * 255 + maximum / 2) / maximum) as u8)
}

#[cfg(test)]
mod tests {
    use std::assert_eq;

    use super::{BackgroundColorQuery, Parser, Response, parse_x11_color_component};

    fn parse(response: &str) -> Vec<Response> {
        let mut parser = Parser::new();
        let mut caps: Vec<Response> = vec![];
        for ch in response.chars() {
            let mut more_caps = parser.push(ch);
            caps.append(&mut more_caps)
        }
        caps
    }

    #[test]
    fn test_parse_all() {
        let caps =
            parse("\x1b_Gi=31;OK\x1b\\\x1b[?64;4c\x1b[6;7;14t\x1b[6;6R\x1b[7;7R\x1b[6;6R\x1b[0n");
        assert_eq!(
            caps,
            vec![
                Response::Kitty,
                Response::Sixel,
                Response::CellSize(Some((14, 7))),
                Response::CursorPositionReport(6, 6),
                Response::CursorPositionReport(7, 7),
                Response::CursorPositionReport(6, 6),
                Response::Status,
            ],
        );
    }

    #[test]
    fn test_parse_only_garbage() {
        let caps = parse("\x1bhonkey\x1btonkey\x1b[42\x1b\\");
        assert_eq!(caps, vec![]);
    }

    #[test]
    fn test_parse_preceding_garbage() {
        let caps = parse("\x1bgarbage...\x1b[?64;5c\x1b[0n");
        assert_eq!(caps, vec![Response::Status]);
    }

    #[test]
    fn test_parse_inner_garbage() {
        let caps = parse("\x1b[6;7;14t\x1bgarbage...\x1b[?64;5c\x1b[0n");
        assert_eq!(
            caps,
            vec![Response::CellSize(Some((14, 7))), Response::Status]
        );
    }

    #[test]
    fn background_queries_use_the_protocol_selected_by_the_terminal_owner() {
        assert_eq!(
            Parser::background_query(false, BackgroundColorQuery::Osc11),
            "\x1b]11;?\x07"
        );
        assert_eq!(
            Parser::background_query(false, BackgroundColorQuery::Iterm2Osc4),
            "\x1b]4;-2;?\x07"
        );
    }

    #[test]
    fn parses_two_and_four_digit_iterm_color_responses_without_theme_inversion() {
        assert_eq!(
            parse("\x1b]4;-2;rgb:ff/80/00\x07"),
            vec![Response::Background(
                BackgroundColorQuery::Iterm2Osc4,
                255,
                128,
                0
            )]
        );
        assert_eq!(
            parse("\x1b]4;-2;rgb:ffff/8080/0000\x1b\\"),
            vec![Response::Background(
                BackgroundColorQuery::Iterm2Osc4,
                255,
                128,
                0
            )]
        );
        assert_eq!(parse_x11_color_component("fff"), Some(255));
        assert_eq!(parse_x11_color_component(""), None);
        assert_eq!(parse_x11_color_component("00000"), None);
    }

    #[test]
    fn color_parser_rejects_foreground_and_unrelated_palette_replies() {
        assert!(parse("\x1b]10;rgb:ffff/ffff/ffff\x07").is_empty());
        assert!(parse("\x1b]4;7;rgb:ffff/ffff/ffff\x07").is_empty());
        assert_eq!(
            parse("\x1b]11;rgb:00/00/00\x07"),
            vec![Response::Background(
                BackgroundColorQuery::Osc11,
                0,
                0,
                0
            )]
        );
    }

    // #[test]
    // fn test_parse_incomplete_support_in_text_sizing_protocol() {
    // let caps = parse("\x1b[6;7;14t\x1b[6;6R\x1b[7;7R\x1b[6;6R\x1b[0n");
    // assert_eq!(
    // caps,
    // vec![
    // Response::CellSize(Some((14, 7))),
    // Response::CursorPositionReport(6, 6),
    // Response::CursorPositionReport(7, 7),
    // Response::CursorPositionReport(6, 6),
    // Response::Status,
    // ],
    // );
    // }
}
