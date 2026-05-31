//! TUI-local defaults. Persistent runtime settings live in `~/agena/agena.json`.

use crate::keybindings::ComposerKeyBindings;

#[derive(Debug, Clone, Default)]
pub struct TuiConfig {
    pub keybindings: ComposerKeyBindings,
    pub double_esc_window_ms: u64,
    pub status_line: TuiStatusLineConfig,
    pub theme: Option<String>,
    pub transcript: TuiTranscriptConfig,
}

#[derive(Debug, Clone, Default)]
pub struct TuiStatusLineConfig {
    pub command: Option<String>,
    pub refresh_interval_ms: u64,
}

#[derive(Debug, Clone)]
pub struct TuiTranscriptConfig {
    pub tool_output_default_expanded: bool,
    pub thinking_default_expanded: bool,
}

impl Default for TuiTranscriptConfig {
    fn default() -> Self {
        Self {
            tool_output_default_expanded: true,
            thinking_default_expanded: false,
        }
    }
}

impl TuiConfig {
    pub fn load() -> Self {
        Self::default_config()
    }

    fn default_config() -> Self {
        Self {
            keybindings: ComposerKeyBindings::default(),
            double_esc_window_ms: 600,
            status_line: TuiStatusLineConfig::default(),
            theme: None,
            transcript: TuiTranscriptConfig::default(),
        }
    }
}
