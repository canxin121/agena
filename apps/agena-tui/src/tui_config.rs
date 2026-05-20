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
    pub composer_mode: TuiComposerMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TuiComposerMode {
    #[default]
    Emacs,
    Vim,
}

#[derive(Debug, Clone, Default)]
pub struct TuiStatusLineConfig {
    pub command: Option<String>,
    pub refresh_interval_ms: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawTuiConfig {
    keybindings: RawKeybindings,
    double_esc_window_ms: Option<u64>,
    status_line: RawStatusLineConfig,
    theme: Option<String>,
    composer_mode: Option<String>,
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
            composer_mode: TuiComposerMode::Emacs,
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
        let composer_mode = raw
            .composer_mode
            .as_deref()
            .map(parse_composer_mode)
            .transpose()
            .unwrap_or_else(|err| {
                eprintln!("[agena-tui] invalid composer_mode: {err}");
                None
            })
            .unwrap_or(TuiComposerMode::Emacs);
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
            composer_mode,
        }
    }
}

fn parse_composer_mode(raw: &str) -> Result<TuiComposerMode, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "emacs" | "default" => Ok(TuiComposerMode::Emacs),
        "vim" => Ok(TuiComposerMode::Vim),
        other => Err(format!("expected `emacs` or `vim`, got `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_line_command() {
        let raw: RawTuiConfig = toml::from_str(
            r#"
theme = "solarized"
composer_mode = "vim"

[status_line]
command = "printf ready"
refresh_interval_ms = 500
"#,
        )
        .unwrap();

        let config = TuiConfig::from_raw(raw, ComposerKeyBindings::default());

        assert_eq!(config.status_line.command.as_deref(), Some("printf ready"));
        assert_eq!(config.status_line.refresh_interval_ms, 500);
        assert_eq!(config.theme.as_deref(), Some("solarized"));
        assert_eq!(config.composer_mode, TuiComposerMode::Vim);
    }

    #[test]
    fn trims_empty_status_line_command() {
        let raw: RawTuiConfig = toml::from_str(
            r#"
[status_line]
command = "   "
refresh_interval_ms = 10
"#,
        )
        .unwrap();

        let config = TuiConfig::from_raw(raw, ComposerKeyBindings::default());

        assert!(config.status_line.command.is_none());
        assert_eq!(config.status_line.refresh_interval_ms, 250);
    }

    #[test]
    fn trims_empty_theme() {
        let raw: RawTuiConfig = toml::from_str(
            r#"
theme = "   "
"#,
        )
        .unwrap();

        let config = TuiConfig::from_raw(raw, ComposerKeyBindings::default());

        assert!(config.theme.is_none());
    }

    #[test]
    fn parse_composer_mode_accepts_emacs_and_vim() {
        assert_eq!(
            parse_composer_mode("emacs").unwrap(),
            TuiComposerMode::Emacs
        );
        assert_eq!(parse_composer_mode("vim").unwrap(), TuiComposerMode::Vim);
    }
}
