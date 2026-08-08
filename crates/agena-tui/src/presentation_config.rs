//! In-memory preferences that control TUI presentation.
//!
//! Persistent configuration belongs to the application host.  It maps its
//! serialized settings into these runtime values before constructing the TUI.

use agena_tui_components::{ColorScheme, TerminalRgb, ThemePalette};

use crate::{input::ComposerKeyBindings, terminal_graphics::GraphicsMode};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
/// Preferred color scheme.
pub enum ColorSchemePreference {
    #[default]
    Auto,
    Dark,
    Light,
}

#[derive(Debug, Clone)]
/// Configuration of the TUI.
pub struct TuiConfig {
    pub keybindings: ComposerKeyBindings,
    pub double_esc_window_ms: u64,
    pub status_line: TuiStatusLineConfig,
    pub theme: Option<String>,
    pub color_scheme: ColorSchemePreference,
    pub graphics: GraphicsMode,
    pub transcript: TuiTranscriptConfig,
    /// Whether the terminal window/tab title tracks the session name and
    /// activity.
    pub terminal_title: TerminalIntegrationMode,
    /// Whether terminal-native attention notifications are raised for
    /// permission requests, user-input requests, errors, and completion.
    pub terminal_notifications: TerminalIntegrationMode,
    /// Whether the terminal renders an OSC 9;4 native progress indicator
    /// (indeterminate while running, paused while awaiting, error when
    /// blocked) instead of relying on the title suffix alone.
    pub terminal_progress: TerminalIntegrationMode,
}

/// User-selectable mode for a terminal-integration feature. `Auto` follows the
/// detected capability evidence; `Enabled` and `Disabled` override it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TerminalIntegrationMode {
    #[default]
    Auto,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Default)]
/// Status line configuration.
pub struct TuiStatusLineConfig {
    pub command: Option<String>,
    pub refresh_interval_ms: u64,
}

#[derive(Debug, Clone, Default)]
/// Transcript configuration.
pub struct TuiTranscriptConfig {
    pub activity_default_expanded: bool,
}

impl TuiConfig {
    pub fn palette(&self, detected_background: Option<TerminalRgb>) -> ThemePalette {
        match self.color_scheme {
            ColorSchemePreference::Dark => ThemePalette::for_scheme(ColorScheme::Dark),
            ColorSchemePreference::Light => ThemePalette::for_scheme(ColorScheme::Light),
            ColorSchemePreference::Auto => detected_background
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
            ColorSchemePreference::Dark => ColorScheme::Dark.reference_background(),
            ColorSchemePreference::Light => ColorScheme::Light.reference_background(),
            ColorSchemePreference::Auto => {
                detected_background.unwrap_or_else(|| ColorScheme::Dark.reference_background())
            }
        }
    }
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            keybindings: ComposerKeyBindings::default(),
            double_esc_window_ms: 600,
            status_line: TuiStatusLineConfig::default(),
            theme: None,
            color_scheme: ColorSchemePreference::Auto,
            graphics: GraphicsMode::Auto,
            transcript: TuiTranscriptConfig::default(),
            terminal_title: TerminalIntegrationMode::default(),
            terminal_notifications: TerminalIntegrationMode::default(),
            terminal_progress: TerminalIntegrationMode::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_scheme_uses_detected_terminal_background() {
        let config = TuiConfig::default();
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
        let config = TuiConfig {
            color_scheme: ColorSchemePreference::Dark,
            ..TuiConfig::default()
        };
        assert_eq!(
            config.palette(Some(TerminalRgb::new(255, 255, 255))).scheme,
            ColorScheme::Dark
        );
        assert_eq!(
            config.graphics_background(Some(TerminalRgb::new(255, 255, 255))),
            ColorScheme::Dark.reference_background()
        );

        let config = TuiConfig {
            color_scheme: ColorSchemePreference::Light,
            ..TuiConfig::default()
        };
        assert_eq!(
            config.graphics_background(Some(TerminalRgb::new(0, 0, 0))),
            ColorScheme::Light.reference_background()
        );
    }

    #[test]
    fn auto_graphics_canvas_preserves_detection_and_has_a_safe_fallback() {
        let config = TuiConfig::default();
        let detected = TerminalRgb::new(37, 42, 49);
        assert_eq!(config.graphics_background(Some(detected)), detected);
        assert_eq!(
            config.graphics_background(None),
            ColorScheme::Dark.reference_background()
        );
    }

    #[test]
    fn auto_scheme_keeps_terminal_native_code_surface_when_detection_fails() {
        let palette = TuiConfig::default().palette(None);
        assert_eq!(palette.code_fg, ratatui::style::Color::Reset);
        assert_eq!(palette.code_bg, ratatui::style::Color::Reset);
    }
}
