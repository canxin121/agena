use std::{env, time::Duration};

use agena_tui_components::TerminalRgb;
use crossterm::event::BackgroundColorQuery as RuntimeBackgroundColorQuery;
use ratatui_image::picker::{
    Picker, cap_parser::BackgroundColorQuery as PickerBackgroundColorQuery,
};

use crate::terminal::TerminalFamily;

/// Exact evidence used to classify the terminal's light/dark appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalColorSource {
    Osc11,
    Iterm2Osc4,
    ColorFgBg,
    TermBackground,
    VsCodeThemeKind,
    Unavailable,
}

impl TerminalColorSource {
    pub const fn localization_key(self) -> &'static str {
        match self {
            Self::Osc11 => "terminal-diagnostics-color-source-osc11",
            Self::Iterm2Osc4 => "terminal-diagnostics-color-source-iterm-osc4",
            Self::ColorFgBg => "terminal-diagnostics-color-source-colorfgbg",
            Self::TermBackground => "terminal-diagnostics-color-source-term-background",
            Self::VsCodeThemeKind => "terminal-diagnostics-color-source-vscode-theme",
            Self::Unavailable => "terminal-diagnostics-color-source-unavailable",
        }
    }

    pub const fn diagnostic_label(self) -> &'static str {
        match self {
            Self::Osc11 => "OSC 11",
            Self::Iterm2Osc4 => "iTerm2 OSC 4;-2",
            Self::ColorFgBg => "COLORFGBG",
            Self::TermBackground => "TERM_BACKGROUND",
            Self::VsCodeThemeKind => "VSCODE_THEME_KIND",
            Self::Unavailable => "unavailable",
        }
    }

    pub const fn supports_live_refresh(self) -> bool {
        matches!(self, Self::Osc11 | Self::Iterm2Osc4)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalColorDetection {
    pub background: Option<TerminalRgb>,
    pub source: TerminalColorSource,
}

impl Default for TerminalColorDetection {
    fn default() -> Self {
        Self {
            background: None,
            source: TerminalColorSource::Unavailable,
        }
    }
}

const COLOR_QUERY_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(350);

/// Query color independently from graphics negotiation. iTerm2's documented
/// OSC 4 extension is tried first for an iTerm endpoint, then OSC 11; other
/// terminals receive two bounded OSC 11 attempts. Every reply is matched to
/// the selector that produced it, so a delayed response cannot be mislabeled
/// in diagnostics or confused with an unrelated foreground/palette response.
pub fn query_terminal_background(
    family: TerminalFamily,
    through_tmux: bool,
) -> Option<TerminalColorDetection> {
    let queries = if family == TerminalFamily::Iterm2 {
        [
            PickerBackgroundColorQuery::Iterm2Osc4,
            PickerBackgroundColorQuery::Osc11,
        ]
    } else {
        [
            PickerBackgroundColorQuery::Osc11,
            PickerBackgroundColorQuery::Osc11,
        ]
    };
    for query in queries {
        let Ok((red, green, blue)) = Picker::query_background_color_stdio_in_raw_mode(
            query,
            through_tmux,
            COLOR_QUERY_ATTEMPT_TIMEOUT,
        ) else {
            continue;
        };
        let source = match query {
            PickerBackgroundColorQuery::Osc11 => TerminalColorSource::Osc11,
            PickerBackgroundColorQuery::Iterm2Osc4 => TerminalColorSource::Iterm2Osc4,
        };
        return Some(TerminalColorDetection {
            background: Some(TerminalRgb::new(red, green, blue)),
            source,
        });
    }
    None
}

/// Complete wire frames for one response-bearing transaction. The request is
/// followed by a barrier on the same ordered terminal output stream.
pub struct QueryTransaction {
    request: Vec<u8>,
    barrier: Vec<u8>,
}

impl QueryTransaction {
    pub fn frames(&self) -> [&[u8]; 2] {
        [&self.request, &self.barrier]
    }
}

/// Build a live color query only for a protocol that succeeded at startup.
/// Unlike the startup compatibility probe, this function performs no stdin
/// reads: `TerminalRuntime` owns the response transaction through Crossterm's
/// typed event stream.
pub fn color_refresh_transaction(
    source: TerminalColorSource,
    through_tmux: bool,
) -> Option<(RuntimeBackgroundColorQuery, QueryTransaction)> {
    let query = refresh_query_for_source(source)?;
    let request = match query {
        RuntimeBackgroundColorQuery::Osc11 => b"\x1b]11;?\x07".as_slice(),
        RuntimeBackgroundColorQuery::Iterm2Osc4 => b"\x1b]4;-2;?\x07".as_slice(),
    };
    Some((
        query,
        QueryTransaction {
            request: transport_frame(request, through_tmux),
            barrier: transport_frame(b"\x1b[5n", through_tmux),
        },
    ))
}

/// A cursor-position response is distinct from every completion marker used by
/// the synchronous startup probes, so it forms an unambiguous final barrier
/// before the normal event loop begins.
pub fn startup_barrier(through_tmux: bool) -> Vec<u8> {
    transport_frame(b"\x1b[6n", through_tmux)
}

fn transport_frame(frame: &[u8], through_tmux: bool) -> Vec<u8> {
    if !through_tmux {
        return frame.to_vec();
    }

    let mut wrapped = Vec::with_capacity(frame.len() + 10);
    wrapped.extend_from_slice(b"\x1bPtmux;");
    for byte in frame {
        if *byte == b'\x1b' {
            wrapped.push(b'\x1b');
        }
        wrapped.push(*byte);
    }
    wrapped.extend_from_slice(b"\x1b\\");
    wrapped
}

const fn refresh_query_for_source(
    source: TerminalColorSource,
) -> Option<RuntimeBackgroundColorQuery> {
    match source {
        TerminalColorSource::Osc11 => Some(RuntimeBackgroundColorQuery::Osc11),
        TerminalColorSource::Iterm2Osc4 => Some(RuntimeBackgroundColorQuery::Iterm2Osc4),
        TerminalColorSource::ColorFgBg
        | TerminalColorSource::TermBackground
        | TerminalColorSource::VsCodeThemeKind
        | TerminalColorSource::Unavailable => None,
    }
}

/// Resolve background hints without reading stdin. The terminal runtime gives
/// the dedicated bounded pre-input color query authority over these hints.
pub fn detect_terminal_background() -> TerminalColorDetection {
    background_from_environment()
}

fn background_from_environment() -> TerminalColorDetection {
    let colorfgbg = env::var("COLORFGBG").ok();
    let term_background = env::var("TERM_BACKGROUND").ok();
    let vscode_theme_kind = env::var("VSCODE_THEME_KIND").ok();
    background_from_values(
        colorfgbg.as_deref(),
        term_background.as_deref(),
        vscode_theme_kind.as_deref(),
    )
}

fn background_from_values(
    colorfgbg: Option<&str>,
    term_background: Option<&str>,
    vscode_theme_kind: Option<&str>,
) -> TerminalColorDetection {
    if let Some(value) = colorfgbg
        && let Some(color) = parse_colorfgbg(value)
    {
        return TerminalColorDetection {
            background: Some(color),
            source: TerminalColorSource::ColorFgBg,
        };
    }
    for (value, source) in [
        (term_background, TerminalColorSource::TermBackground),
        (vscode_theme_kind, TerminalColorSource::VsCodeThemeKind),
    ] {
        let Some(value) = value else { continue };
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" | "highcontrastdark" => {
                return TerminalColorDetection {
                    background: Some(TerminalRgb::new(24, 24, 27)),
                    source,
                };
            }
            "light" | "highcontrastlight" => {
                return TerminalColorDetection {
                    background: Some(TerminalRgb::new(250, 250, 250)),
                    source,
                };
            }
            _ => {}
        }
    }
    TerminalColorDetection::default()
}

fn parse_colorfgbg(value: &str) -> Option<TerminalRgb> {
    let index = value
        .split([';', ':'])
        .next_back()?
        .trim()
        .parse::<u8>()
        .ok()?;
    let (red, green, blue) = match index {
        0 => (0, 0, 0),
        1 => (170, 0, 0),
        2 => (0, 170, 0),
        3 => (170, 85, 0),
        4 => (0, 0, 170),
        5 => (170, 0, 170),
        6 => (0, 170, 170),
        7 => (170, 170, 170),
        8 => (85, 85, 85),
        9 => (255, 85, 85),
        10 => (85, 255, 85),
        11 => (255, 255, 85),
        12 => (85, 85, 255),
        13 => (255, 85, 255),
        14 => (85, 255, 255),
        15 => (255, 255, 255),
        16..=231 => {
            let offset = index - 16;
            let component = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            (
                component(offset / 36),
                component((offset % 36) / 6),
                component(offset % 6),
            )
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    };
    Some(TerminalRgb::new(red, green, blue))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_colorfgbg_without_terminal_io() {
        assert_eq!(parse_colorfgbg("15;0"), Some(TerminalRgb::new(0, 0, 0)));
        assert_eq!(
            parse_colorfgbg("0;231"),
            Some(TerminalRgb::new(255, 255, 255))
        );
    }

    #[test]
    fn environment_color_fallback_records_the_exact_evidence_source() {
        assert_eq!(
            background_from_values(Some("0;231"), Some("dark"), Some("dark")),
            TerminalColorDetection {
                background: Some(TerminalRgb::new(255, 255, 255)),
                source: TerminalColorSource::ColorFgBg,
            }
        );
        assert_eq!(
            background_from_values(None, Some("light"), Some("dark")),
            TerminalColorDetection {
                background: Some(TerminalRgb::new(250, 250, 250)),
                source: TerminalColorSource::TermBackground,
            }
        );
        assert_eq!(
            background_from_values(None, None, Some("highContrastDark")),
            TerminalColorDetection {
                background: Some(TerminalRgb::new(24, 24, 27)),
                source: TerminalColorSource::VsCodeThemeKind,
            }
        );
        assert_eq!(
            background_from_values(None, None, None),
            TerminalColorDetection::default()
        );
    }

    #[test]
    fn live_refresh_reuses_only_a_query_that_succeeded_at_startup() {
        assert_eq!(
            refresh_query_for_source(TerminalColorSource::Osc11),
            Some(RuntimeBackgroundColorQuery::Osc11)
        );
        assert_eq!(
            refresh_query_for_source(TerminalColorSource::Iterm2Osc4),
            Some(RuntimeBackgroundColorQuery::Iterm2Osc4)
        );
        for source in [
            TerminalColorSource::ColorFgBg,
            TerminalColorSource::TermBackground,
            TerminalColorSource::VsCodeThemeKind,
            TerminalColorSource::Unavailable,
        ] {
            assert_eq!(refresh_query_for_source(source), None);
        }
    }

    #[test]
    fn response_transactions_are_complete_and_ordered_for_both_transports() {
        let (_, direct) = color_refresh_transaction(TerminalColorSource::Iterm2Osc4, false)
            .expect("query-backed source");
        assert_eq!(
            direct.frames(),
            [b"\x1b]4;-2;?\x07".as_slice(), b"\x1b[5n".as_slice()]
        );
        assert_eq!(startup_barrier(false), b"\x1b[6n");

        let (_, tmux) = color_refresh_transaction(TerminalColorSource::Osc11, true)
            .expect("query-backed source");
        assert_eq!(tmux.frames()[0], b"\x1bPtmux;\x1b\x1b]11;?\x07\x1b\\");
        assert_eq!(tmux.frames()[1], b"\x1bPtmux;\x1b\x1b[5n\x1b\\");
        assert_eq!(startup_barrier(true), b"\x1bPtmux;\x1b\x1b[6n\x1b\\");
    }
}
