//! Pure terminal-runtime values shared by TUI capability detection.

use std::{collections::BTreeMap, env, fmt};

/// The terminal product family inferred from stable process environment
/// evidence. This is presentation/runtime metadata rather than app state.
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

    pub fn parse_override(value: &str) -> Option<Self> {
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

/// A stable snapshot of terminal-related process environment variables.
#[derive(Debug, Clone, Default)]
pub struct TerminalEnvironment {
    values: BTreeMap<String, String>,
}

impl TerminalEnvironment {
    pub fn from_process() -> Self {
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

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn has(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        Self {
            values: pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }
}

/// Conservative capability profiles for known terminal families.
pub mod profile {
    use super::TerminalFamily;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    /// Terminal profile support level.
    pub enum ProfileSupport {
        Available,
        Unsupported,
        Unknown,
    }

    /// Whether the terminal family supports the Kitty keyboard protocol
    /// (`CSI u`, aka keyboard disambiguation / enhancement).
    ///
    /// The composer's Enter-family bindings (Ctrl+Enter = submit, Shift+Enter /
    /// Ctrl+J = newline) depend on the terminal reporting modifier+Enter chords
    /// distinctly. Without the protocol, Ctrl+Enter and Shift+Enter are
    /// byte-identical to plain Enter (CR), so those chords can never be
    /// distinguished. Modern terminal emulators ignore the push sequence when
    /// they do not implement it (they keep sending legacy bytes), so enabling
    /// the protocol for the well-known CSI-u families is safe and strictly
    /// improves key disambiguation. xterm-compatible and Apple Terminal do not
    /// implement the protocol and stay disabled.
    pub const fn keyboard(f: TerminalFamily) -> ProfileSupport {
        match f {
            TerminalFamily::Kitty
            | TerminalFamily::Ghostty
            | TerminalFamily::Foot
            | TerminalFamily::Iterm2
            | TerminalFamily::WezTerm
            | TerminalFamily::WindowsTerminal
            | TerminalFamily::VsCode
            | TerminalFamily::JetBrains
            | TerminalFamily::Alacritty
            | TerminalFamily::Vte
            | TerminalFamily::Konsole
            | TerminalFamily::Warp
            | TerminalFamily::Rio
            | TerminalFamily::Contour => ProfileSupport::Available,
            TerminalFamily::Dumb
            | TerminalFamily::LinuxConsole
            | TerminalFamily::AppleTerminal
            | TerminalFamily::XtermCompatible => ProfileSupport::Unsupported,
            _ => ProfileSupport::Unknown,
        }
    }
    /// Whether the family should push the Kitty protocol flag that reports
    /// every key through CSI u (`REPORT_ALL_KEYS_AS_ESCAPE_CODES`).
    ///
    /// Defaults to off for every family. The flag is what makes iTerm2 treat
    /// Option as Alt (so Option+Backspace can word-delete), but it also makes
    /// iTerm2 report every key through CSI u, which breaks text entry from
    /// input methods (IME): composing and committing Chinese/Japanese text is
    /// no longer delivered as text. Users who want Option word-deletion in
    /// iTerm2 and do not rely on an IME can opt in explicitly with
    /// `AGENA_TUI_KEYBOARD_REPORT_ALL_KEYS=1`; the safer alternative is the
    /// iTerm2 profile setting "Option Key Sends = Esc+", which sends ESC DEL
    /// and is handled by the existing ALT word-delete path without the flag.
    pub const fn keyboard_all_keys(f: TerminalFamily) -> ProfileSupport {
        match f {
            TerminalFamily::Dumb
            | TerminalFamily::LinuxConsole
            | TerminalFamily::AppleTerminal
            | TerminalFamily::XtermCompatible => ProfileSupport::Unsupported,
            _ => ProfileSupport::Unknown,
        }
    }
    pub const fn osc52_write(f: TerminalFamily) -> ProfileSupport {
        match f {
            TerminalFamily::Iterm2
            | TerminalFamily::Kitty
            | TerminalFamily::WezTerm
            | TerminalFamily::Ghostty
            | TerminalFamily::WindowsTerminal
            | TerminalFamily::VsCode
            | TerminalFamily::Alacritty
            | TerminalFamily::Vte
            | TerminalFamily::Konsole
            | TerminalFamily::Foot
            | TerminalFamily::Warp
            | TerminalFamily::Rio
            | TerminalFamily::Contour => ProfileSupport::Available,
            TerminalFamily::Dumb | TerminalFamily::LinuxConsole => ProfileSupport::Unsupported,
            _ => ProfileSupport::Unknown,
        }
    }
    pub const fn osc52_read(f: TerminalFamily) -> ProfileSupport {
        match f {
            TerminalFamily::Kitty | TerminalFamily::Ghostty => ProfileSupport::Available,
            TerminalFamily::WezTerm | TerminalFamily::Dumb | TerminalFamily::LinuxConsole => {
                ProfileSupport::Unsupported
            }
            _ => ProfileSupport::Unknown,
        }
    }
    pub const fn inline_images(f: TerminalFamily) -> bool {
        matches!(
            f,
            TerminalFamily::Iterm2
                | TerminalFamily::Kitty
                | TerminalFamily::WezTerm
                | TerminalFamily::Ghostty
        )
    }
    pub const fn synchronized_output(f: TerminalFamily) -> bool {
        matches!(
            f,
            TerminalFamily::Kitty
                | TerminalFamily::WezTerm
                | TerminalFamily::Ghostty
                | TerminalFamily::Foot
        )
    }
    pub const fn hyperlinks(f: TerminalFamily) -> ProfileSupport {
        match f {
            TerminalFamily::Dumb | TerminalFamily::LinuxConsole => ProfileSupport::Unsupported,
            TerminalFamily::Unknown | TerminalFamily::XtermCompatible => ProfileSupport::Unknown,
            _ => ProfileSupport::Available,
        }
    }
    /// OSC 0/2 window/tab title setting. The escape is consumed locally by a
    /// multiplexer (becoming the pane title) rather than being forwarded, so
    /// title support is never path-gated. The Linux console is excluded
    /// because its title handling is not a native notification surface.
    pub const fn window_title(f: TerminalFamily) -> ProfileSupport {
        match f {
            TerminalFamily::Dumb | TerminalFamily::LinuxConsole => ProfileSupport::Unsupported,
            _ => ProfileSupport::Available,
        }
    }
    /// OSC 9;4 (ConEmu) native progress reporting. The terminal renders an
    /// indeterminate/pulsing or determinate progress indicator in its tab,
    /// titlebar, or taskbar chrome. Unlike window titles, the sequence is not
    /// consumed locally by a multiplexer and an unsupported endpoint may
    /// interpret `OSC 9;4;*` as an OSC 9 notification, so only families with
    /// verified progress support are enabled; everything else stays
    /// conservative (`Unsupported`).
    pub const fn progress(f: TerminalFamily) -> ProfileSupport {
        match f {
            TerminalFamily::Iterm2
            | TerminalFamily::WindowsTerminal
            | TerminalFamily::WezTerm
            | TerminalFamily::Ghostty
            | TerminalFamily::VsCode
            | TerminalFamily::Konsole
            | TerminalFamily::Warp => ProfileSupport::Available,
            _ => ProfileSupport::Unsupported,
        }
    }
    /// BEL is the universal VT attention signal and therefore the base
    /// notification method for every interactive family.
    pub const fn notifications_bel(f: TerminalFamily) -> bool {
        !matches!(f, TerminalFamily::Dumb | TerminalFamily::LinuxConsole)
    }
}

/// Terminal transport evidence inferred from the environment, without making
/// topology claims beyond the observed hops.
pub mod transport {
    use super::TerminalEnvironment;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    /// A hop in the terminal transport chain.
    pub enum TransportHop {
        Ssh,
        Mosh,
        Tmux,
        Screen,
        Zellij,
        Wsl,
    }

    impl TransportHop {
        pub const fn label(self) -> &'static str {
            match self {
                Self::Ssh => "SSH",
                Self::Mosh => "Mosh",
                Self::Tmux => "tmux",
                Self::Screen => "screen",
                Self::Zellij => "Zellij",
                Self::Wsl => "WSL",
            }
        }
        pub const fn is_remote(self) -> bool {
            matches!(self, Self::Ssh | Self::Mosh)
        }
        pub const fn is_multiplexer(self) -> bool {
            matches!(self, Self::Tmux | Self::Screen | Self::Zellij)
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    /// Evidence about the terminal transport.
    pub struct TransportEvidence {
        pub layer: TransportHop,
        pub source_key: &'static str,
    }

    pub fn detect(environment: &TerminalEnvironment) -> Vec<TransportEvidence> {
        let mut evidence = Vec::new();
        let mut push = |layer, source_key| evidence.push(TransportEvidence { layer, source_key });
        if environment.has("SSH_TTY") {
            push(TransportHop::Ssh, "SSH_TTY");
        } else if environment.has("SSH_CONNECTION") {
            push(TransportHop::Ssh, "SSH_CONNECTION");
        } else if environment.has("SSH_CLIENT") {
            push(TransportHop::Ssh, "SSH_CLIENT");
        }
        if environment.has("MOSH_CONNECTION") {
            push(TransportHop::Mosh, "MOSH_CONNECTION");
        }
        if environment.has("TMUX") {
            push(TransportHop::Tmux, "TMUX");
        }
        if environment.has("STY") {
            push(TransportHop::Screen, "STY");
        }
        if environment.has("ZELLIJ") {
            push(TransportHop::Zellij, "ZELLIJ");
        } else if environment.has("ZELLIJ_SESSION_NAME") {
            push(TransportHop::Zellij, "ZELLIJ_SESSION_NAME");
        }
        if environment.has("WSL_INTEROP") {
            push(TransportHop::Wsl, "WSL_INTEROP");
        } else if environment.has("WSL_DISTRO_NAME") {
            push(TransportHop::Wsl, "WSL_DISTRO_NAME");
        }
        evidence
    }
}

/// Pure identity evidence values used by terminal detection and diagnostics.
pub mod identity {
    use super::{TerminalEnvironment, TerminalFamily, TerminalVersion};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    /// Source of a terminal identity claim.
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
    /// Confidence of a terminal identity claim.
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
    /// Evidence of terminal identity.
    pub struct IdentityEvidence {
        pub key: &'static str,
        pub value: String,
        pub candidate: TerminalFamily,
        pub source: IdentitySource,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    /// Detected identity of the terminal.
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
        pub fn detect(e: &TerminalEnvironment) -> Self {
            let term = e.get("TERM").map(ToOwned::to_owned);
            let override_value = e.get("AGENA_TUI_TERMINAL");
            if let Some(value) = override_value
                && let Some(family) = TerminalFamily::parse_override(value)
            {
                let version = e.get("AGENA_TUI_TERMINAL_VERSION").map(ToOwned::to_owned);
                let mut evidence = collect(e, term.as_deref());
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
            let override_error = override_value.map(|value| {
                format!("invalid AGENA_TUI_TERMINAL value `{value}`; automatic detection was used")
            });
            let p = e.get("TERM_PROGRAM").unwrap_or_default();
            let emulator = e.get("TERMINAL_EMULATOR").unwrap_or_default();
            let family = if p.eq_ignore_ascii_case("vscode") {
                TerminalFamily::VsCode
            } else if p.eq_ignore_ascii_case("iTerm.app") {
                TerminalFamily::Iterm2
            } else if p.eq_ignore_ascii_case("WezTerm") {
                TerminalFamily::WezTerm
            } else if p.eq_ignore_ascii_case("ghostty") {
                TerminalFamily::Ghostty
            } else if p.eq_ignore_ascii_case("Apple_Terminal") {
                TerminalFamily::AppleTerminal
            } else if p.eq_ignore_ascii_case("Alacritty") {
                TerminalFamily::Alacritty
            } else if p.eq_ignore_ascii_case("WarpTerminal") {
                TerminalFamily::Warp
            } else if p.eq_ignore_ascii_case("Rio") {
                TerminalFamily::Rio
            } else if p.eq_ignore_ascii_case("Contour") {
                TerminalFamily::Contour
            } else if emulator.to_ascii_lowercase().contains("jetbrains")
                || emulator.to_ascii_lowercase().contains("jediterm")
            {
                TerminalFamily::JetBrains
            } else if e.has("WT_SESSION")
                && p.is_empty()
                && !e.has("KITTY_WINDOW_ID")
                && !e.has("KITTY_PID")
                && !e.has("GHOSTTY_RESOURCES_DIR")
                && !e.has("WEZTERM_PANE")
                && !matches!(term.as_deref(), Some("xterm-kitty" | "xterm-ghostty"))
            {
                TerminalFamily::WindowsTerminal
            } else if e.get("LC_TERMINAL").is_some_and(|v| v == "iTerm2")
                || e.has("ITERM_SESSION_ID")
            {
                TerminalFamily::Iterm2
            } else if e.has("WEZTERM_PANE") {
                TerminalFamily::WezTerm
            } else if e.has("GHOSTTY_RESOURCES_DIR") || term.as_deref() == Some("xterm-ghostty") {
                TerminalFamily::Ghostty
            } else if e.has("KITTY_WINDOW_ID")
                || e.has("KITTY_PID")
                || term.as_deref() == Some("xterm-kitty")
            {
                TerminalFamily::Kitty
            } else if e.has("ALACRITTY_SOCKET") {
                TerminalFamily::Alacritty
            } else if e.has("KONSOLE_VERSION") || e.has("KONSOLE_DBUS_SERVICE") {
                TerminalFamily::Konsole
            } else if e.has("VTE_VERSION") {
                TerminalFamily::Vte
            } else if term
                .as_deref()
                .is_some_and(|v| v == "foot" || v == "foot-extra" || v.starts_with("foot-"))
            {
                TerminalFamily::Foot
            } else if e.has("WARP_IS_LOCAL_SHELL_SESSION") {
                TerminalFamily::Warp
            } else if term.as_deref() == Some("linux") {
                TerminalFamily::LinuxConsole
            } else if term.as_deref() == Some("dumb") {
                TerminalFamily::Dumb
            } else if term
                .as_deref()
                .is_some_and(|v| v == "xterm" || v.starts_with("xterm-"))
            {
                TerminalFamily::XtermCompatible
            } else {
                TerminalFamily::Unknown
            };
            let version = match family {
                TerminalFamily::Iterm2 => e
                    .get("TERM_PROGRAM_VERSION")
                    .or_else(|| e.get("LC_TERMINAL_VERSION")),
                TerminalFamily::WezTerm
                | TerminalFamily::Ghostty
                | TerminalFamily::VsCode
                | TerminalFamily::AppleTerminal
                | TerminalFamily::Alacritty
                | TerminalFamily::Warp
                | TerminalFamily::Rio
                | TerminalFamily::Contour => e.get("TERM_PROGRAM_VERSION"),
                TerminalFamily::Konsole => e.get("KONSOLE_VERSION"),
                TerminalFamily::Vte => e.get("VTE_VERSION"),
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
            ) && term.as_deref().is_some_and(|v| {
                matches!(
                    (family, v),
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
                term: term.clone(),
                source,
                confidence,
                evidence: collect(e, term.as_deref()),
                override_error,
            }
        }
        pub fn display_name(&self) -> String {
            self.version
                .as_deref()
                .filter(|v| !v.is_empty())
                .map(|v| format!("{} {v}", self.family))
                .unwrap_or_else(|| self.family.to_string())
        }
        pub fn conflicts(&self) -> Vec<TerminalFamily> {
            let mut values = self
                .evidence
                .iter()
                .map(|e| e.candidate)
                .filter(|candidate| {
                    *candidate != self.family && *candidate != TerminalFamily::XtermCompatible
                })
                .collect::<Vec<_>>();
            values.sort_by_key(|family| family.label());
            values.dedup();
            values
        }
    }

    fn collect(e: &TerminalEnvironment, term: Option<&str>) -> Vec<IdentityEvidence> {
        let mut values = Vec::new();
        let mut push = |key, candidate| {
            if let Some(value) = e.get(key) {
                values.push(IdentityEvidence {
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
        let p = e.get("TERM_PROGRAM").unwrap_or_default();
        let program = if p.eq_ignore_ascii_case("vscode") {
            Some(TerminalFamily::VsCode)
        } else if p.eq_ignore_ascii_case("iTerm.app") {
            Some(TerminalFamily::Iterm2)
        } else if p.eq_ignore_ascii_case("WezTerm") {
            Some(TerminalFamily::WezTerm)
        } else if p.eq_ignore_ascii_case("ghostty") {
            Some(TerminalFamily::Ghostty)
        } else if p.eq_ignore_ascii_case("Apple_Terminal") {
            Some(TerminalFamily::AppleTerminal)
        } else if p.eq_ignore_ascii_case("Alacritty") {
            Some(TerminalFamily::Alacritty)
        } else if p.eq_ignore_ascii_case("WarpTerminal") {
            Some(TerminalFamily::Warp)
        } else if p.eq_ignore_ascii_case("Rio") {
            Some(TerminalFamily::Rio)
        } else if p.eq_ignore_ascii_case("Contour") {
            Some(TerminalFamily::Contour)
        } else {
            None
        };
        if let Some(f) = program {
            push("TERM_PROGRAM", f);
        }
        for (key, family) in [
            ("WT_SESSION", TerminalFamily::WindowsTerminal),
            ("KITTY_WINDOW_ID", TerminalFamily::Kitty),
            ("KITTY_PID", TerminalFamily::Kitty),
            ("WEZTERM_PANE", TerminalFamily::WezTerm),
            ("GHOSTTY_RESOURCES_DIR", TerminalFamily::Ghostty),
            ("ALACRITTY_SOCKET", TerminalFamily::Alacritty),
            ("KONSOLE_VERSION", TerminalFamily::Konsole),
            ("VTE_VERSION", TerminalFamily::Vte),
        ] {
            if e.has(key) {
                push(key, family);
            }
        }
        let candidate = match term {
            Some("xterm-kitty") => Some(TerminalFamily::Kitty),
            Some("xterm-ghostty") => Some(TerminalFamily::Ghostty),
            Some("linux") => Some(TerminalFamily::LinuxConsole),
            Some("dumb") => Some(TerminalFamily::Dumb),
            Some(v) if v == "xterm" || v.starts_with("xterm-") => {
                Some(TerminalFamily::XtermCompatible)
            }
            Some(v) if v == "foot" || v == "foot-extra" || v.starts_with("foot-") => {
                Some(TerminalFamily::Foot)
            }
            _ => None,
        };
        if let Some(f) = candidate {
            push("TERM", f);
        }
        values
    }
}

/// A dotted terminal version without vendor-specific interpretation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TerminalVersion {
    components: Vec<u64>,
}

impl TerminalVersion {
    pub fn parse(value: &str) -> Option<Self> {
        let components = value
            .trim()
            .split('.')
            .map(|component| component.parse::<u64>())
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        (!components.is_empty()).then_some(Self { components })
    }
}

impl fmt::Display for TerminalVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self
            .components
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(".");
        formatter.write_str(value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TerminalEnvironment, TerminalFamily, TerminalVersion, identity, profile, transport,
    };

    fn identify(pairs: &[(&str, &str)]) -> identity::TerminalIdentity {
        identity::TerminalIdentity::detect(&TerminalEnvironment::from_pairs(pairs))
    }

    #[test]
    fn parses_dotted_versions_without_guessing_vendor_encodings() {
        assert_eq!(
            TerminalVersion::parse("1.2.30").unwrap().to_string(),
            "1.2.30"
        );
        assert!(TerminalVersion::parse("240800").is_some());
        assert!(TerminalVersion::parse("nightly").is_none());
    }

    #[test]
    fn keyboard_disambiguation_covers_modern_csi_u_families() {
        use profile::ProfileSupport;
        for family in [
            TerminalFamily::Kitty,
            TerminalFamily::Ghostty,
            TerminalFamily::Foot,
            TerminalFamily::Iterm2,
            TerminalFamily::WezTerm,
            TerminalFamily::WindowsTerminal,
            TerminalFamily::VsCode,
            TerminalFamily::JetBrains,
            TerminalFamily::Alacritty,
            TerminalFamily::Vte,
            TerminalFamily::Konsole,
            TerminalFamily::Warp,
            TerminalFamily::Rio,
            TerminalFamily::Contour,
        ] {
            assert_eq!(
                profile::keyboard(family),
                ProfileSupport::Available,
                "{} should enable keyboard disambiguation",
                family.label()
            );
        }
        for family in [
            TerminalFamily::AppleTerminal,
            TerminalFamily::XtermCompatible,
            TerminalFamily::LinuxConsole,
            TerminalFamily::Dumb,
        ] {
            assert_eq!(
                profile::keyboard(family),
                ProfileSupport::Unsupported,
                "{} cannot disambiguate Enter chords",
                family.label()
            );
        }
        assert_eq!(
            profile::keyboard(TerminalFamily::Unknown),
            ProfileSupport::Unknown
        );
    }

    #[test]
    fn keyboard_all_keys_is_opt_in_never_auto_available() {
        use profile::ProfileSupport;
        for family in [
            TerminalFamily::Iterm2,
            TerminalFamily::Kitty,
            TerminalFamily::Ghostty,
            TerminalFamily::Foot,
            TerminalFamily::WezTerm,
            TerminalFamily::WindowsTerminal,
            TerminalFamily::VsCode,
            TerminalFamily::JetBrains,
            TerminalFamily::Alacritty,
            TerminalFamily::Vte,
            TerminalFamily::Konsole,
            TerminalFamily::Warp,
            TerminalFamily::Rio,
            TerminalFamily::Contour,
            TerminalFamily::Unknown,
        ] {
            assert_ne!(
                profile::keyboard_all_keys(family),
                ProfileSupport::Available,
                "{} must not report every key through CSI u by default",
                family.label()
            );
        }
    }

    #[test]
    fn transport_records_evidence_without_claiming_nesting_order() {
        let environment = TerminalEnvironment::from_pairs(&[
            ("SSH_CONNECTION", "local remote"),
            ("TMUX", "/tmp/tmux"),
        ]);
        let evidence = transport::detect(&environment);
        assert_eq!(evidence[0].source_key, "SSH_CONNECTION");
        assert_eq!(evidence[1].source_key, "TMUX");
    }

    #[test]
    fn ssh_client_alone_still_marks_a_remote_transport() {
        let environment = TerminalEnvironment::from_pairs(&[("SSH_CLIENT", "a b c")]);
        let evidence = transport::detect(&environment);
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].layer, transport::TransportHop::Ssh);
        assert_eq!(evidence[0].source_key, "SSH_CLIENT");
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
        assert_eq!(identity.source, identity::IdentitySource::UserOverride);
    }

    #[test]
    fn terminfo_identity_is_compatibility_evidence_and_conflicts_are_retained() {
        let identity = identify(&[
            ("TERM_PROGRAM", "vscode"),
            ("KITTY_WINDOW_ID", "1"),
            ("TERM", "xterm-kitty"),
        ]);
        assert_eq!(identity.family, TerminalFamily::VsCode);
        assert_eq!(identity.conflicts(), vec![TerminalFamily::Kitty]);

        let xterm = identify(&[("TERM", "xterm-256color")]);
        assert_eq!(
            xterm.confidence,
            identity::IdentityConfidence::CompatibilityOnly
        );
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
