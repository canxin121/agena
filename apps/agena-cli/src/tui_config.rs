//! TUI-local defaults. Persistent runtime settings live in `~/agena/agena.json`.

use agena::config::{TuiColorSchemeConfig, UiConfig};
use agena_tui_components::{ColorScheme, TerminalRgb, ThemePalette};

use crate::tui_keymap::ComposerKeyBindings;

#[derive(Debug, Clone, Default)]
pub struct TuiConfig {
    pub keybindings: ComposerKeyBindings,
    pub double_esc_window_ms: u64,
    pub status_line: TuiStatusLineConfig,
    pub theme: Option<String>,
    pub color_scheme: TuiColorSchemeConfig,
    pub transcript: TuiTranscriptConfig,
}

#[derive(Debug, Clone, Default)]
pub struct TuiStatusLineConfig {
    pub command: Option<String>,
    pub refresh_interval_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TuiTranscriptConfig {
    pub activity_default_expanded: bool,
}

impl TuiConfig {
    pub fn load(ui: Option<&UiConfig>) -> Self {
        let mut config = Self::default_config();
        if let Some(ui) = ui {
            config.theme = ui.tui.theme.clone();
            config.color_scheme = ui.tui.color_scheme;
        }
        config
    }

    pub fn palette(&self, detected_background: Option<TerminalRgb>) -> ThemePalette {
        match self.color_scheme {
            TuiColorSchemeConfig::Dark => ThemePalette::for_scheme(ColorScheme::Dark),
            TuiColorSchemeConfig::Light => ThemePalette::for_scheme(ColorScheme::Light),
            TuiColorSchemeConfig::Auto => detected_background
                .map(ThemePalette::for_background)
                .unwrap_or_else(|| ThemePalette::for_scheme(ColorScheme::Dark)),
        }
    }

    fn default_config() -> Self {
        Self {
            keybindings: ComposerKeyBindings::default(),
            double_esc_window_ms: 600,
            status_line: TuiStatusLineConfig::default(),
            theme: None,
            color_scheme: TuiColorSchemeConfig::Auto,
            transcript: TuiTranscriptConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_scheme_uses_detected_terminal_background() {
        let config = TuiConfig::default_config();
        assert_eq!(
            config.palette(Some(TerminalRgb::new(248, 248, 248))).scheme,
            ColorScheme::Light
        );
        assert_eq!(
            config.palette(Some(TerminalRgb::new(20, 20, 20))).scheme,
            ColorScheme::Dark
        );
    }

    #[test]
    fn explicit_scheme_overrides_terminal_detection() {
        let mut config = TuiConfig::default_config();
        config.color_scheme = TuiColorSchemeConfig::Dark;
        assert_eq!(
            config.palette(Some(TerminalRgb::new(255, 255, 255))).scheme,
            ColorScheme::Dark
        );
    }
}
