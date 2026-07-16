//! TUI-local defaults. Persistent runtime settings live in `~/agena/agena.json`.

use agena::config::{TuiColorSchemeConfig, TuiGraphicsModeConfig, UiConfig};
use agena_tui_components::{ColorScheme, TerminalRgb, ThemePalette};

use crate::tui_keymap::ComposerKeyBindings;

#[derive(Debug, Clone, Default)]
pub struct TuiConfig {
    pub keybindings: ComposerKeyBindings,
    pub double_esc_window_ms: u64,
    pub status_line: TuiStatusLineConfig,
    pub theme: Option<String>,
    pub color_scheme: TuiColorSchemeConfig,
    pub graphics: TuiGraphicsModeConfig,
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
            config.graphics = ui.tui.graphics;
        }
        config
    }

    pub fn palette(&self, detected_background: Option<TerminalRgb>) -> ThemePalette {
        match self.color_scheme {
            TuiColorSchemeConfig::Dark => ThemePalette::for_scheme(ColorScheme::Dark),
            TuiColorSchemeConfig::Light => ThemePalette::for_scheme(ColorScheme::Light),
            TuiColorSchemeConfig::Auto => detected_background
                .map(ThemePalette::for_background)
                .unwrap_or_else(ThemePalette::for_unknown_background),
        }
    }

    /// Resolve the concrete canvas used by generated terminal graphics.
    /// Explicit appearance settings take precedence over terminal detection,
    /// just as they do for the semantic text palette. Auto mode preserves an
    /// exact OSC 11 color when available and otherwise uses the dark reference
    /// canvas so generated content remains self-contrasting.
    pub fn graphics_background(&self, detected_background: Option<TerminalRgb>) -> TerminalRgb {
        match self.color_scheme {
            TuiColorSchemeConfig::Dark => ColorScheme::Dark.reference_background(),
            TuiColorSchemeConfig::Light => ColorScheme::Light.reference_background(),
            TuiColorSchemeConfig::Auto => {
                detected_background.unwrap_or_else(|| ColorScheme::Dark.reference_background())
            }
        }
    }

    fn default_config() -> Self {
        Self {
            keybindings: ComposerKeyBindings::default(),
            double_esc_window_ms: 600,
            status_line: TuiStatusLineConfig::default(),
            theme: None,
            color_scheme: TuiColorSchemeConfig::Auto,
            graphics: TuiGraphicsModeConfig::Auto,
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
        assert_eq!(
            config.graphics_background(Some(TerminalRgb::new(255, 255, 255))),
            ColorScheme::Dark.reference_background()
        );

        config.color_scheme = TuiColorSchemeConfig::Light;
        assert_eq!(
            config.graphics_background(Some(TerminalRgb::new(0, 0, 0))),
            ColorScheme::Light.reference_background()
        );
    }

    #[test]
    fn auto_graphics_canvas_preserves_detection_and_has_a_safe_fallback() {
        let config = TuiConfig::default_config();
        let detected = TerminalRgb::new(37, 42, 49);
        assert_eq!(config.graphics_background(Some(detected)), detected);
        assert_eq!(
            config.graphics_background(None),
            ColorScheme::Dark.reference_background()
        );
    }

    #[test]
    fn auto_scheme_keeps_terminal_native_code_surface_when_detection_fails() {
        let palette = TuiConfig::default_config().palette(None);
        assert_eq!(palette.code_fg, ratatui::style::Color::Reset);
        assert_eq!(palette.code_bg, ratatui::style::Color::Reset);
    }
}
