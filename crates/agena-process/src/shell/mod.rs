//! User shell resolution and shell-environment policy for managed processes.
//!
//! Agena runs model-authored command strings, and those strings must behave the
//! way they do in the user's own terminal. This module owns the *trusted*
//! configuration that decides which shell program runs them and whether login
//! startup and an interactive startup snapshot are used:
//!
//! - [`SHELL_ENV`] (`AGENA_SHELL`) pins the shell program (path or bare name).
//! - [`SHELL_LOGIN_ENV`] (`AGENA_SHELL_LOGIN`) selects `-lc` (login) or `-c`.
//! - [`SHELL_SNAPSHOT_ENV`] (`AGENA_SHELL_SNAPSHOT`) selects snapshot behavior.
//!
//! Trusted process configuration selects the mode; model input cannot change it,
//! matching the existing `AGENA_SHELL_SANDBOX` policy surface. Command-line
//! construction stays in `agena-tool::shell`; interactive startup capture lives
//! in [`snapshot`].

mod detect;
pub mod env_expand;
pub mod snapshot;

pub use detect::DetectedShell;
pub use detect::ShellKind;
pub use detect::ShellSource;

/// Environment variable that pins the shell program (path or bare name).
pub const SHELL_ENV: &str = "AGENA_SHELL";
/// Environment variable that selects login (`-lc`) versus plain (`-c`) startup.
pub const SHELL_LOGIN_ENV: &str = "AGENA_SHELL_LOGIN";
/// Environment variable that selects interactive-snapshot behavior.
pub const SHELL_SNAPSHOT_ENV: &str = "AGENA_SHELL_SNAPSHOT";

/// Which shell dialect family a caller asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellPreference {
    /// POSIX-shaped command strings (`shell.run`'s default `bash` dialect).
    Posix,
    /// PowerShell-shaped command strings.
    Powershell,
}

/// Policy for the interactive startup snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SnapshotMode {
    /// Never capture; commands keep the historical `-lc` startup only.
    Disabled,
    /// Capture when possible; failures fall back to `-lc` and are reported.
    #[default]
    Auto,
    /// Capture is required; a failed capture refuses the command.
    Required,
}

impl SnapshotMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Auto => "auto",
            Self::Required => "required",
        }
    }
}

/// Parsed, trusted shell configuration for one process tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSettings {
    shell_override: Option<String>,
    login: bool,
    snapshot: SnapshotMode,
    diagnostics: Vec<String>,
}

impl Default for ShellSettings {
    fn default() -> Self {
        Self {
            shell_override: None,
            login: true,
            snapshot: SnapshotMode::default(),
            diagnostics: Vec::new(),
        }
    }
}

impl ShellSettings {
    /// Read the trusted configuration from the process environment.
    pub fn from_env() -> Self {
        Self::from_lookup(&|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
    }

    /// Read the trusted configuration through an injected lookup (for tests).
    pub fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> Self {
        let mut settings = Self::default();
        let raw_override = lookup(SHELL_ENV);
        settings.shell_override = raw_override
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if settings.shell_override.is_none() && raw_override.is_some() {
            settings.diagnostics.push(format!(
                "{SHELL_ENV} is empty; ignoring it and using the detected shell"
            ));
        }

        if let Some(raw) = lookup(SHELL_LOGIN_ENV) {
            match parse_bool(raw.trim()) {
                Some(login) => settings.login = login,
                None => settings.diagnostics.push(format!(
                    "{SHELL_LOGIN_ENV} value `{}` is not a boolean; keeping the default (login shell enabled)",
                    raw.trim()
                )),
            }
        }

        if let Some(raw) = lookup(SHELL_SNAPSHOT_ENV) {
            match parse_snapshot_mode(raw.trim()) {
                Some(mode) => settings.snapshot = mode,
                None => settings.diagnostics.push(format!(
                    "{SHELL_SNAPSHOT_ENV} value `{}` is not disabled, auto, or required; keeping the default (auto)",
                    raw.trim()
                )),
            }
        }

        settings
    }

    /// Explicit shell program from [`SHELL_ENV`], when set and non-empty.
    pub fn shell_override(&self) -> Option<&str> {
        self.shell_override.as_deref()
    }

    /// Whether commands run a login shell (`-lc`).
    pub fn login(&self) -> bool {
        self.login
    }

    /// Configured snapshot policy.
    pub fn snapshot(&self) -> SnapshotMode {
        self.snapshot
    }

    /// Configuration problems worth surfacing in tool metadata.
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    /// Resolve the shell program for one dialect family.
    pub fn resolve(&self, preference: ShellPreference) -> DetectedShell {
        detect::resolve_with(self, preference, &detect::RealShellHost)
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_snapshot_mode(value: &str) -> Option<SnapshotMode> {
    match value.to_ascii_lowercase().as_str() {
        "disabled" | "off" | "false" | "0" | "none" => Some(SnapshotMode::Disabled),
        "auto" => Some(SnapshotMode::Auto),
        "required" => Some(SnapshotMode::Required),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect::<HashMap<_, _>>();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn defaults_keep_login_startup_and_auto_snapshot() {
        let settings = ShellSettings::from_lookup(&lookup_from(&[]));
        assert_eq!(settings.shell_override(), None);
        assert!(settings.login());
        assert_eq!(settings.snapshot(), SnapshotMode::Auto);
        assert!(settings.diagnostics().is_empty());
    }

    #[test]
    fn explicit_values_are_parsed_with_whitespace_and_case_tolerance() {
        let settings = ShellSettings::from_lookup(&lookup_from(&[
            (SHELL_ENV, "  /bin/bash  "),
            (SHELL_LOGIN_ENV, "OFF"),
            (SHELL_SNAPSHOT_ENV, " Required "),
        ]));
        assert_eq!(settings.shell_override(), Some("/bin/bash"));
        assert!(!settings.login());
        assert_eq!(settings.snapshot(), SnapshotMode::Required);
        assert!(settings.diagnostics().is_empty());
    }

    #[test]
    fn invalid_values_are_reported_and_keep_the_defaults() {
        let settings = ShellSettings::from_lookup(&lookup_from(&[
            (SHELL_LOGIN_ENV, "sometimes"),
            (SHELL_SNAPSHOT_ENV, "always"),
        ]));
        assert!(settings.login());
        assert_eq!(settings.snapshot(), SnapshotMode::Auto);
        assert_eq!(settings.diagnostics().len(), 2);
        assert!(settings.diagnostics()[0].contains("not a boolean"));
        assert!(settings.diagnostics()[1].contains("not disabled, auto, or required"));
    }

    #[test]
    fn empty_override_is_reported_and_ignored() {
        let settings = ShellSettings::from_lookup(&lookup_from(&[(SHELL_ENV, "   ")]));
        assert_eq!(settings.shell_override(), None);
        assert_eq!(settings.diagnostics().len(), 1);
        assert!(settings.diagnostics()[0].contains("empty"));
    }

    #[test]
    fn snapshot_modes_round_trip_through_their_names() {
        for mode in [
            SnapshotMode::Disabled,
            SnapshotMode::Auto,
            SnapshotMode::Required,
        ] {
            assert_eq!(parse_snapshot_mode(mode.as_str()), Some(mode));
        }
    }
}
