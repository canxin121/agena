use std::{env, time::Duration};

use agena_tui_components::TerminalRgb;
use ratatui_image::picker::{Picker, cap_parser::BackgroundColorQuery};

use super::{TerminalColorDetection, TerminalColorSource, TerminalContext, TerminalFamily};

const COLOR_QUERY_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(350);
const COLOR_REFRESH_TIMEOUT: Duration = Duration::from_millis(150);

/// Query color independently from graphics negotiation. iTerm2's documented
/// OSC 4 extension is tried first for an iTerm endpoint, then OSC 11; other
/// terminals receive two bounded OSC 11 attempts. Every reply is matched to
/// the selector that produced it, so a delayed response cannot be mislabeled
/// in diagnostics or confused with an unrelated foreground/palette response.
pub(super) fn query_terminal_background(
    context: &TerminalContext,
    through_tmux: bool,
) -> Option<TerminalColorDetection> {
    let queries = if context.identity.family == TerminalFamily::Iterm2 {
        [
            BackgroundColorQuery::Iterm2Osc4,
            BackgroundColorQuery::Osc11,
        ]
    } else {
        [BackgroundColorQuery::Osc11, BackgroundColorQuery::Osc11]
    };
    for query in queries {
        let Ok((red, green, blue)) =
            Picker::query_background_color_stdio(query, through_tmux, COLOR_QUERY_ATTEMPT_TIMEOUT)
        else {
            continue;
        };
        let source = match query {
            BackgroundColorQuery::Osc11 => TerminalColorSource::Osc11,
            BackgroundColorQuery::Iterm2Osc4 => TerminalColorSource::Iterm2Osc4,
        };
        return Some(TerminalColorDetection {
            background: Some(TerminalRgb::new(red, green, blue)),
            source,
        });
    }
    None
}

/// Re-query only a protocol that already succeeded during startup. This keeps
/// focus/resume refreshes short and prevents an unsupported terminal from
/// pausing input on every focus transition. A failed refresh preserves the
/// last known-good color instead of falling back and flipping appearance.
pub(super) fn refresh_terminal_background(
    source: TerminalColorSource,
    through_tmux: bool,
) -> Option<TerminalColorDetection> {
    let query = refresh_query_for_source(source)?;
    let (red, green, blue) =
        Picker::query_background_color_stdio(query, through_tmux, COLOR_REFRESH_TIMEOUT).ok()?;
    Some(TerminalColorDetection {
        background: Some(TerminalRgb::new(red, green, blue)),
        source,
    })
}

const fn refresh_query_for_source(source: TerminalColorSource) -> Option<BackgroundColorQuery> {
    match source {
        TerminalColorSource::Osc11 => Some(BackgroundColorQuery::Osc11),
        TerminalColorSource::Iterm2Osc4 => Some(BackgroundColorQuery::Iterm2Osc4),
        TerminalColorSource::ColorFgBg
        | TerminalColorSource::TermBackground
        | TerminalColorSource::VsCodeThemeKind
        | TerminalColorSource::Unavailable => None,
    }
}

/// Resolve background hints without reading stdin. The terminal runtime gives
/// the dedicated bounded pre-input color query authority over these hints.
pub(super) fn detect_terminal_background() -> TerminalColorDetection {
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
            Some(BackgroundColorQuery::Osc11)
        );
        assert_eq!(
            refresh_query_for_source(TerminalColorSource::Iterm2Osc4),
            Some(BackgroundColorQuery::Iterm2Osc4)
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
}
