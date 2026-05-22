//! TUI-local configuration (separate from the shared `agena::config`).
//!
//! Currently holds the configurable key bindings for the composer.
//! Loaded from (in order of precedence):
//!
//!   1. `--tui-config <path>` CLI flag
//!   2. `$AGENA_TUI_CONFIG`
//!   3. `$XDG_CONFIG_HOME/agena/tui.toml`
//!   4. `$HOME/.agena/tui.toml`
//!
//! Missing file is **not** an error; defaults apply.

use std::{env, fs, path::PathBuf};

use serde::Deserialize;

use crate::keybindings::{ComposerKeyBindings, RawComposerKeyBindings};

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

#[derive(Debug, Clone, Default)]
pub struct TuiTranscriptConfig {
    pub tool_output_default_expanded: bool,
    pub thinking_default_expanded: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawTuiConfig {
    keybindings: RawKeybindings,
    double_esc_window_ms: Option<u64>,
    status_line: RawStatusLineConfig,
    theme: Option<String>,
    composer_mode: Option<String>,
    transcript: RawTranscriptConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawKeybindings {
    composer: RawComposerKeyBindings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawStatusLineConfig {
    command: Option<String>,
    refresh_interval_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawTranscriptConfig {
    tool_output_default_expanded: Option<bool>,
    thinking_default_expanded: Option<bool>,
}

impl TuiConfig {
    pub fn load(explicit: Option<PathBuf>) -> Self {
        let path = explicit
            .or_else(|| env::var_os("AGENA_TUI_CONFIG").map(PathBuf::from))
            .or_else(|| {
                env::var_os("XDG_CONFIG_HOME").map(|base| {
                    let mut p = PathBuf::from(base);
                    p.push("agena/tui.toml");
                    p
                })
            })
            .or_else(|| {
                env::var_os("HOME").map(|base| {
                    let mut p = PathBuf::from(base);
                    p.push(".agena/tui.toml");
                    p
                })
            });

        let Some(path) = path else {
            return Self::default_config();
        };

        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => {
                return Self::default_config();
            }
        };

        let raw: RawTuiConfig = match toml::from_str(&text) {
            Ok(r) => r,
            Err(err) => {
                eprintln!("[agena-tui] failed to parse {}: {err}", path.display());
                return Self::default_config();
            }
        };

        let keybindings = match ComposerKeyBindings::from_raw(&raw.keybindings.composer) {
            Ok(kb) => kb,
            Err(err) => {
                eprintln!(
                    "[agena-tui] invalid keybinding in {}: {err}",
                    path.display()
                );
                ComposerKeyBindings::default()
            }
        };

        Self::from_raw(raw, keybindings)
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

    fn from_raw(raw: RawTuiConfig, keybindings: ComposerKeyBindings) -> Self {
        let command = raw
            .status_line
            .command
            .map(|command| command.trim().to_string())
            .filter(|command| !command.is_empty());
        let theme = raw
            .theme
            .map(|theme| theme.trim().to_string())
            .filter(|theme| !theme.is_empty());
        if let Some(mode) = raw.composer_mode.as_deref() {
            match mode.trim().to_ascii_lowercase().as_str() {
                "" | "default" | "vim" | "emacs" => {}
                other => eprintln!("[agena-tui] invalid composer_mode: expected `vim`, got `{other}`"),
            }
        }
        Self {
            keybindings,
            double_esc_window_ms: raw.double_esc_window_ms.unwrap_or(600),
            status_line: TuiStatusLineConfig {
                command,
                refresh_interval_ms: raw
                    .status_line
                    .refresh_interval_ms
                    .unwrap_or(1_000)
                    .max(250),
            },
            theme,
            transcript: TuiTranscriptConfig {
                tool_output_default_expanded: raw
                    .transcript
                    .tool_output_default_expanded
                    .unwrap_or(false),
                thinking_default_expanded: raw
                    .transcript
                    .thinking_default_expanded
                    .unwrap_or(false),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_raw_reads_transcript_defaults() {
        let config = TuiConfig::from_raw(
            RawTuiConfig {
                transcript: RawTranscriptConfig {
                    tool_output_default_expanded: Some(true),
                    thinking_default_expanded: Some(true),
                },
                ..RawTuiConfig::default()
            },
            ComposerKeyBindings::default(),
        );

        assert!(config.transcript.tool_output_default_expanded);
        assert!(config.transcript.thinking_default_expanded);
    }

    #[test]
    fn from_raw_accepts_legacy_composer_mode_but_keeps_defaults() {
        let config = TuiConfig::from_raw(
            RawTuiConfig {
                composer_mode: Some("emacs".to_string()),
                ..RawTuiConfig::default()
            },
            ComposerKeyBindings::default(),
        );

        assert!(!config.transcript.tool_output_default_expanded);
        assert!(!config.transcript.thinking_default_expanded);
    }
}
