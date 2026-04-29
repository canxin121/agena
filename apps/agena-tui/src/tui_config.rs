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
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawTuiConfig {
    keybindings: RawKeybindings,
    double_esc_window_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawKeybindings {
    composer: RawComposerKeyBindings,
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
            return Self {
                keybindings: ComposerKeyBindings::default(),
                double_esc_window_ms: 600,
            };
        };

        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => {
                return Self {
                    keybindings: ComposerKeyBindings::default(),
                    double_esc_window_ms: 600,
                };
            }
        };

        let raw: RawTuiConfig = match toml::from_str(&text) {
            Ok(r) => r,
            Err(err) => {
                eprintln!("[agena-tui] failed to parse {}: {err}", path.display());
                return Self {
                    keybindings: ComposerKeyBindings::default(),
                    double_esc_window_ms: 600,
                };
            }
        };

        let keybindings = match ComposerKeyBindings::from_raw(&raw.keybindings.composer) {
            Ok(kb) => kb,
            Err(err) => {
                eprintln!("[agena-tui] invalid keybinding in {}: {err}", path.display());
                ComposerKeyBindings::default()
            }
        };

        Self {
            keybindings,
            double_esc_window_ms: raw.double_esc_window_ms.unwrap_or(600),
        }
    }
}
