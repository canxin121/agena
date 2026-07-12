use std::env;

use agena_tui_components::TerminalRgb;

use super::TerminalContext;

/// Resolve background hints without reading stdin. The terminal runtime gives
/// the bounded pre-EventStream graphics/OSC query authority over these hints.
pub(super) fn detect_terminal_background(_context: &TerminalContext) -> Option<TerminalRgb> {
    background_from_environment()
}

fn background_from_environment() -> Option<TerminalRgb> {
    if let Ok(value) = env::var("COLORFGBG")
        && let Some(color) = parse_colorfgbg(&value)
    {
        return Some(color);
    }
    for key in ["TERM_BACKGROUND", "VSCODE_THEME_KIND"] {
        let Ok(value) = env::var(key) else {
            continue;
        };
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" | "highcontrastdark" => return Some(TerminalRgb::new(24, 24, 27)),
            "light" | "highcontrastlight" => return Some(TerminalRgb::new(250, 250, 250)),
            _ => {}
        }
    }
    None
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
}
