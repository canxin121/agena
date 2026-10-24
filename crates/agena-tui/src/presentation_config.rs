//! In-memory preferences that control TUI presentation.
//!
//! Persistent configuration belongs to the application host.  It maps its
//! serialized settings into these runtime values before constructing the TUI.

use crate::{input::ComposerKeyBindings, terminal_graphics::GraphicsMode};
use agena_tui_components::{ColorScheme, TerminalRgb, ThemePalette};

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
            terminal_title: TerminalIntegrationMode::default(),
            terminal_notifications: TerminalIntegrationMode::default(),
            terminal_progress: TerminalIntegrationMode::default(),
        }
    }
}
