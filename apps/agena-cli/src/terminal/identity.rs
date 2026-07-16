use std::{collections::BTreeMap, env, fmt};

use super::version::TerminalVersion;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalFamily {
    Iterm2,
    Kitty,
    WezTerm,
    Ghostty,
    WindowsTerminal,
    VsCode,
    AppleTerminal,
    Alacritty,
    Vte,
    Konsole,
    Foot,
    Warp,
    JetBrains,
    Rio,
    Contour,
    XtermCompatible,
    LinuxConsole,
    Dumb,
    Unknown,
}

impl TerminalFamily {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Iterm2 => "iTerm2",
            Self::Kitty => "Kitty",
            Self::WezTerm => "WezTerm",
            Self::Ghostty => "Ghostty",
            Self::WindowsTerminal => "Windows Terminal",
            Self::VsCode => "VS Code",
            Self::AppleTerminal => "Apple Terminal",
            Self::Alacritty => "Alacritty",
            Self::Vte => "VTE",
            Self::Konsole => "Konsole",
            Self::Foot => "foot",
            Self::Warp => "Warp",
            Self::JetBrains => "JetBrains Terminal",
            Self::Rio => "Rio",
            Self::Contour => "Contour",
            Self::XtermCompatible => "xterm-compatible",
            Self::LinuxConsole => "Linux console",
            Self::Dumb => "dumb terminal",
            Self::Unknown => "unknown terminal",
        }
    }

    fn parse_override(value: &str) -> Option<Self> {
        let normalized = value
            .trim()
            .to_ascii_lowercase()
            .replace(['-', '_', ' '], "");
        Some(match normalized.as_str() {
            "iterm" | "iterm2" => Self::Iterm2,
            "kitty" => Self::Kitty,
            "wezterm" => Self::WezTerm,
            "ghostty" => Self::Ghostty,
            "windowsterminal" | "wt" => Self::WindowsTerminal,
            "vscode" | "code" => Self::VsCode,
            "appleterminal" | "terminalapp" => Self::AppleTerminal,
            "alacritty" => Self::Alacritty,
            "vte" | "gnometerminal" | "ptyxis" | "tilix" => Self::Vte,
            "konsole" => Self::Konsole,
            "foot" => Self::Foot,
            "warp" => Self::Warp,
            "jetbrains" | "jediterm" => Self::JetBrains,
            "rio" => Self::Rio,
            "contour" => Self::Contour,
            "xterm" | "xtermcompatible" | "genericvt" => Self::XtermCompatible,
            "linux" | "linuxconsole" => Self::LinuxConsole,
            "dumb" => Self::Dumb,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

impl fmt::Display for TerminalFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentitySource {
    UserOverride,
    Environment,
    Terminfo,
    Unknown,
}

impl IdentitySource {
    pub const fn localization_key(self) -> &'static str {
        match self {
            Self::UserOverride => "terminal-diagnostics-source-user",
            Self::Environment => "terminal-diagnostics-source-environment",
            Self::Terminfo => "terminal-diagnostics-source-terminfo",
            Self::Unknown => "terminal-diagnostics-source-unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityConfidence {
    Explicit,
    Strong,
    CompatibilityOnly,
    Unknown,
}

impl IdentityConfidence {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Strong => "strong",
            Self::CompatibilityOnly => "compatibility-only",
            Self::Unknown => "unknown",
        }
    }

    pub const fn localization_key(self) -> &'static str {
        match self {
            Self::Explicit => "terminal-diagnostics-confidence-explicit",
            Self::Strong => "terminal-diagnostics-confidence-strong",
            Self::CompatibilityOnly => "terminal-diagnostics-confidence-compatibility",
            Self::Unknown => "terminal-diagnostics-confidence-unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityEvidence {
    pub key: &'static str,
    pub value: String,
    pub candidate: TerminalFamily,
    pub source: IdentitySource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalIdentity {
    pub family: TerminalFamily,
    pub version: Option<String>,
    pub parsed_version: Option<TerminalVersion>,
    pub term: Option<String>,
    pub source: IdentitySource,
    pub confidence: IdentityConfidence,
    pub evidence: Vec<IdentityEvidence>,
    pub override_error: Option<String>,
}

impl TerminalIdentity {
    pub(super) fn detect(environment: &TerminalEnvironment) -> Self {
        let term = environment.get("TERM").map(ToOwned::to_owned);
        let requested_override = environment.get("AGENA_TUI_TERMINAL");
        if let Some(value) = requested_override
            && let Some(family) = TerminalFamily::parse_override(value)
        {
            let version = environment
                .get("AGENA_TUI_TERMINAL_VERSION")
                .map(ToOwned::to_owned);
            let mut evidence = collect_identity_evidence(environment, term.as_deref());
            evidence.insert(
                0,
                IdentityEvidence {
                    key: "AGENA_TUI_TERMINAL",
                    value: value.to_owned(),
                    candidate: family,
                    source: IdentitySource::UserOverride,
                },
            );
            return Self {
                family,
                parsed_version: version.as_deref().and_then(TerminalVersion::parse),
                version,
                term,
                source: IdentitySource::UserOverride,
                confidence: IdentityConfidence::Explicit,
                evidence,
                override_error: None,
            };
        }
        let override_error = requested_override.map(|value| {
            format!("invalid AGENA_TUI_TERMINAL value `{value}`; automatic detection was used")
        });

        let term_program = environment.get("TERM_PROGRAM").unwrap_or_default();
        let terminal_emulator = environment.get("TERMINAL_EMULATOR").unwrap_or_default();
        // A current process-specific product marker is stronger than helper
        // variables inherited from a parent or forwarded through SSH. Resolve
        // every recognized TERM_PROGRAM value before considering those
        // secondary hints.
        let family = if term_program.eq_ignore_ascii_case("vscode") {
            TerminalFamily::VsCode
        } else if term_program.eq_ignore_ascii_case("iTerm.app") {
            TerminalFamily::Iterm2
        } else if term_program.eq_ignore_ascii_case("WezTerm") {
            TerminalFamily::WezTerm
        } else if term_program.eq_ignore_ascii_case("ghostty") {
            TerminalFamily::Ghostty
        } else if term_program.eq_ignore_ascii_case("Apple_Terminal") {
            TerminalFamily::AppleTerminal
        } else if term_program.eq_ignore_ascii_case("Alacritty") {
            TerminalFamily::Alacritty
        } else if term_program.eq_ignore_ascii_case("WarpTerminal") {
            TerminalFamily::Warp
        } else if term_program.eq_ignore_ascii_case("Rio") {
            TerminalFamily::Rio
        } else if term_program.eq_ignore_ascii_case("Contour") {
            TerminalFamily::Contour
        } else if terminal_emulator.to_ascii_lowercase().contains("jetbrains")
            || terminal_emulator.to_ascii_lowercase().contains("jediterm")
        {
            TerminalFamily::JetBrains
        } else if environment.has("WT_SESSION")
            && term_program.is_empty()
            && !environment.has("KITTY_WINDOW_ID")
            && !environment.has("KITTY_PID")
            && !environment.has("GHOSTTY_RESOURCES_DIR")
            && !environment.has("WEZTERM_PANE")
            && !matches!(term.as_deref(), Some("xterm-kitty" | "xterm-ghostty"))
        {
            TerminalFamily::WindowsTerminal
        } else if environment
            .get("LC_TERMINAL")
            .is_some_and(|value| value == "iTerm2")
            || environment.has("ITERM_SESSION_ID")
        {
            TerminalFamily::Iterm2
        } else if environment.has("WEZTERM_PANE") {
            TerminalFamily::WezTerm
        } else if environment.has("GHOSTTY_RESOURCES_DIR")
            || term.as_deref() == Some("xterm-ghostty")
        {
            TerminalFamily::Ghostty
        } else if environment.has("KITTY_WINDOW_ID")
            || environment.has("KITTY_PID")
            || term.as_deref() == Some("xterm-kitty")
        {
            TerminalFamily::Kitty
        } else if environment.has("ALACRITTY_SOCKET") {
            TerminalFamily::Alacritty
        } else if environment.has("KONSOLE_VERSION") || environment.has("KONSOLE_DBUS_SERVICE") {
            TerminalFamily::Konsole
        } else if environment.has("VTE_VERSION") {
            TerminalFamily::Vte
        } else if term.as_deref().is_some_and(|value| {
            value == "foot" || value == "foot-extra" || value.starts_with("foot-")
        }) {
            TerminalFamily::Foot
        } else if environment.has("WARP_IS_LOCAL_SHELL_SESSION") {
            TerminalFamily::Warp
        } else if term.as_deref() == Some("linux") {
            TerminalFamily::LinuxConsole
        } else if term.as_deref() == Some("dumb") {
            TerminalFamily::Dumb
        } else if term
            .as_deref()
            .is_some_and(|value| value == "xterm" || value.starts_with("xterm-"))
        {
            TerminalFamily::XtermCompatible
        } else {
            TerminalFamily::Unknown
        };

        let version = match family {
            TerminalFamily::Iterm2 => environment
                .get("TERM_PROGRAM_VERSION")
                .or_else(|| environment.get("LC_TERMINAL_VERSION")),
            TerminalFamily::WezTerm
            | TerminalFamily::Ghostty
            | TerminalFamily::VsCode
            | TerminalFamily::AppleTerminal
            | TerminalFamily::Alacritty
            | TerminalFamily::Warp
            | TerminalFamily::Rio
            | TerminalFamily::Contour => environment.get("TERM_PROGRAM_VERSION"),
            TerminalFamily::Konsole => environment.get("KONSOLE_VERSION"),
            TerminalFamily::Vte => environment.get("VTE_VERSION"),
            _ => None,
        }
        .map(ToOwned::to_owned);

        let source = if family == TerminalFamily::Unknown {
            IdentitySource::Unknown
        } else if matches!(
            family,
            TerminalFamily::Ghostty
                | TerminalFamily::Kitty
                | TerminalFamily::Foot
                | TerminalFamily::LinuxConsole
                | TerminalFamily::Dumb
                | TerminalFamily::XtermCompatible
        ) && term.as_deref().is_some_and(|value| {
            matches!(
                (family, value),
                (TerminalFamily::Ghostty, "xterm-ghostty")
                    | (TerminalFamily::Kitty, "xterm-kitty")
                    | (TerminalFamily::Foot, "foot" | "foot-extra")
                    | (TerminalFamily::LinuxConsole, "linux")
                    | (TerminalFamily::Dumb, "dumb")
                    | (TerminalFamily::XtermCompatible, "xterm" | "xterm-256color")
            )
        }) {
            IdentitySource::Terminfo
        } else {
            IdentitySource::Environment
        };

        let evidence = collect_identity_evidence(environment, term.as_deref());
        let confidence = match source {
            IdentitySource::UserOverride => IdentityConfidence::Explicit,
            IdentitySource::Environment => IdentityConfidence::Strong,
            IdentitySource::Terminfo => IdentityConfidence::CompatibilityOnly,
            IdentitySource::Unknown => IdentityConfidence::Unknown,
        };
        Self {
            family,
            parsed_version: version.as_deref().and_then(TerminalVersion::parse),
            version,
            term,
            source,
            confidence,
            evidence,
            override_error,
        }
    }

    pub fn display_name(&self) -> String {
        match self.version.as_deref() {
            Some(version) if !version.is_empty() => format!("{} {version}", self.family),
            _ => self.family.to_string(),
        }
    }

    pub fn conflicts(&self) -> Vec<TerminalFamily> {
        let mut conflicts = self
            .evidence
            .iter()
            .map(|evidence| evidence.candidate)
            .filter(|candidate| {
                *candidate != self.family && *candidate != TerminalFamily::XtermCompatible
            })
            .collect::<Vec<_>>();
        conflicts.sort_by_key(|family| family.label());
        conflicts.dedup();
        conflicts
    }
}

fn collect_identity_evidence(
    environment: &TerminalEnvironment,
    term: Option<&str>,
) -> Vec<IdentityEvidence> {
    let mut evidence = Vec::new();
    let mut push = |key: &'static str, candidate: TerminalFamily| {
        if let Some(value) = environment.get(key) {
            evidence.push(IdentityEvidence {
                key,
                value: value.to_owned(),
                candidate,
                source: if key == "TERM" {
                    IdentitySource::Terminfo
                } else {
                    IdentitySource::Environment
                },
            });
        }
    };
    let term_program = environment.get("TERM_PROGRAM").unwrap_or_default();
    if term_program.eq_ignore_ascii_case("vscode") {
        push("TERM_PROGRAM", TerminalFamily::VsCode);
    } else if term_program.eq_ignore_ascii_case("iTerm.app") {
        push("TERM_PROGRAM", TerminalFamily::Iterm2);
    } else if term_program.eq_ignore_ascii_case("WezTerm") {
        push("TERM_PROGRAM", TerminalFamily::WezTerm);
    } else if term_program.eq_ignore_ascii_case("ghostty") {
        push("TERM_PROGRAM", TerminalFamily::Ghostty);
    } else if term_program.eq_ignore_ascii_case("Apple_Terminal") {
        push("TERM_PROGRAM", TerminalFamily::AppleTerminal);
    } else if term_program.eq_ignore_ascii_case("Alacritty") {
        push("TERM_PROGRAM", TerminalFamily::Alacritty);
    } else if term_program.eq_ignore_ascii_case("WarpTerminal") {
        push("TERM_PROGRAM", TerminalFamily::Warp);
    } else if term_program.eq_ignore_ascii_case("Rio") {
        push("TERM_PROGRAM", TerminalFamily::Rio);
    } else if term_program.eq_ignore_ascii_case("Contour") {
        push("TERM_PROGRAM", TerminalFamily::Contour);
    }
    for (key, candidate) in [
        ("WT_SESSION", TerminalFamily::WindowsTerminal),
        ("KITTY_WINDOW_ID", TerminalFamily::Kitty),
        ("KITTY_PID", TerminalFamily::Kitty),
        ("WEZTERM_PANE", TerminalFamily::WezTerm),
        ("GHOSTTY_RESOURCES_DIR", TerminalFamily::Ghostty),
        ("ALACRITTY_SOCKET", TerminalFamily::Alacritty),
        ("KONSOLE_VERSION", TerminalFamily::Konsole),
        ("VTE_VERSION", TerminalFamily::Vte),
    ] {
        if environment.has(key) {
            push(key, candidate);
        }
    }
    let term_candidate = match term {
        Some("xterm-kitty") => Some(TerminalFamily::Kitty),
        Some("xterm-ghostty") => Some(TerminalFamily::Ghostty),
        Some("linux") => Some(TerminalFamily::LinuxConsole),
        Some("dumb") => Some(TerminalFamily::Dumb),
        Some(value) if value == "xterm" || value.starts_with("xterm-") => {
            Some(TerminalFamily::XtermCompatible)
        }
        Some(value) if value == "foot" || value == "foot-extra" || value.starts_with("foot-") => {
            Some(TerminalFamily::Foot)
        }
        _ => None,
    };
    if let Some(candidate) = term_candidate {
        push("TERM", candidate);
    }
    evidence
}

#[derive(Debug, Clone, Default)]
pub(super) struct TerminalEnvironment {
    values: BTreeMap<String, String>,
}

impl TerminalEnvironment {
    pub(super) fn from_process() -> Self {
        Self {
            values: env::vars_os()
                .filter_map(|(key, value)| {
                    key.into_string()
                        .ok()
                        .map(|key| (key, value.to_string_lossy().into_owned()))
                })
                .collect(),
        }
    }

    pub(super) fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub(super) fn has(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    #[cfg(test)]
    pub(super) fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        Self {
            values: pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identify(pairs: &[(&str, &str)]) -> TerminalIdentity {
        TerminalIdentity::detect(&TerminalEnvironment::from_pairs(pairs))
    }

    #[test]
    fn explicit_integrated_terminal_evidence_wins_over_terminfo() {
        let identity = identify(&[
            ("TERM_PROGRAM", "vscode"),
            ("TERM_PROGRAM_VERSION", "1.102.0"),
            ("TERM", "xterm-256color"),
        ]);
        assert_eq!(identity.family, TerminalFamily::VsCode);
        assert_eq!(identity.version.as_deref(), Some("1.102.0"));
    }

    #[test]
    fn nested_terminal_evidence_wins_over_inherited_windows_terminal_session() {
        let identity = identify(&[
            ("WT_SESSION", "inherited"),
            ("KITTY_WINDOW_ID", "1"),
            ("TERM", "xterm-kitty"),
        ]);
        assert_eq!(identity.family, TerminalFamily::Kitty);
    }

    #[test]
    fn explicit_term_program_wins_over_inherited_product_hints() {
        for (term_program, expected) in [
            ("WezTerm", TerminalFamily::WezTerm),
            ("ghostty", TerminalFamily::Ghostty),
            ("Alacritty", TerminalFamily::Alacritty),
        ] {
            let identity = identify(&[
                ("TERM_PROGRAM", term_program),
                ("ITERM_SESSION_ID", "inherited"),
                ("LC_TERMINAL", "iTerm2"),
            ]);
            assert_eq!(identity.family, expected);
        }
    }

    #[test]
    fn detects_modern_terminals_from_stable_environment_evidence() {
        let cases = [
            (&[("TERM_PROGRAM", "iTerm.app")][..], TerminalFamily::Iterm2),
            (&[("KITTY_WINDOW_ID", "1")][..], TerminalFamily::Kitty),
            (&[("TERM", "xterm-ghostty")][..], TerminalFamily::Ghostty),
            (&[("WEZTERM_PANE", "3")][..], TerminalFamily::WezTerm),
            (
                &[("WT_SESSION", "uuid")][..],
                TerminalFamily::WindowsTerminal,
            ),
            (&[("VTE_VERSION", "7600")][..], TerminalFamily::Vte),
            (
                &[("KONSOLE_VERSION", "240800")][..],
                TerminalFamily::Konsole,
            ),
            (&[("TERM", "foot-extra")][..], TerminalFamily::Foot),
            (
                &[("TERM_PROGRAM", "Apple_Terminal")][..],
                TerminalFamily::AppleTerminal,
            ),
            (
                &[("TERM_PROGRAM", "Alacritty")][..],
                TerminalFamily::Alacritty,
            ),
            (
                &[("TERM_PROGRAM", "WarpTerminal")][..],
                TerminalFamily::Warp,
            ),
            (
                &[("TERMINAL_EMULATOR", "JetBrains-JediTerm")][..],
                TerminalFamily::JetBrains,
            ),
            (&[("TERM_PROGRAM", "Rio")][..], TerminalFamily::Rio),
            (&[("TERM_PROGRAM", "Contour")][..], TerminalFamily::Contour),
            (&[("TERM", "linux")][..], TerminalFamily::LinuxConsole),
            (&[("TERM", "dumb")][..], TerminalFamily::Dumb),
            (
                &[("TERM", "xterm-256color")][..],
                TerminalFamily::XtermCompatible,
            ),
        ];
        for (environment, expected) in cases {
            assert_eq!(identify(environment).family, expected);
        }
    }

    #[test]
    fn user_override_is_explicit_and_versioned() {
        let identity = identify(&[
            ("AGENA_TUI_TERMINAL", "kitty"),
            ("AGENA_TUI_TERMINAL_VERSION", "0.47.4"),
            ("TERM_PROGRAM", "vscode"),
        ]);
        assert_eq!(identity.family, TerminalFamily::Kitty);
        assert_eq!(identity.version.as_deref(), Some("0.47.4"));
        assert_eq!(identity.source, IdentitySource::UserOverride);
    }

    #[test]
    fn xterm_terminfo_is_compatibility_evidence_not_product_identity() {
        let identity = identify(&[("TERM", "xterm-256color")]);
        assert_eq!(identity.family, TerminalFamily::XtermCompatible);
        assert_eq!(identity.confidence, IdentityConfidence::CompatibilityOnly);
    }

    #[test]
    fn retains_conflicting_identity_evidence() {
        let identity = identify(&[
            ("TERM_PROGRAM", "vscode"),
            ("KITTY_WINDOW_ID", "1"),
            ("TERM", "xterm-kitty"),
        ]);
        assert_eq!(identity.family, TerminalFamily::VsCode);
        assert_eq!(identity.conflicts(), vec![TerminalFamily::Kitty]);
    }

    #[test]
    fn invalid_override_is_not_silently_ignored() {
        let identity = identify(&[
            ("AGENA_TUI_TERMINAL", "made-up-terminal"),
            ("TERM_PROGRAM", "WezTerm"),
        ]);
        assert_eq!(identity.family, TerminalFamily::WezTerm);
        assert!(identity.override_error.is_some());
    }
}
