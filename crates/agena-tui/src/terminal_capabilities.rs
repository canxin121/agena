//! Terminal capability evidence and lifecycle projection.
//!
//! These values describe presentation endpoint support, transport policy, and
//! local helper availability.  They deliberately contain no process probing:
//! the final application collects that evidence at its process boundary and
//! supplies it to TUI feature code.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Supported,
    Forced,
    Profiled,
    Unsupported,
    Unknown,
}

impl Support {
    pub const fn diagnostic_label(self) -> &'static str {
        match self {
            Self::Supported => "confirmed",
            Self::Forced => "forced",
            Self::Profiled => "profiled",
            Self::Unsupported => "no",
            Self::Unknown => "unknown",
        }
    }

    pub const fn localization_key(self) -> &'static str {
        match self {
            Self::Supported => "terminal-diagnostics-status-confirmed",
            Self::Forced => "terminal-diagnostics-status-forced",
            Self::Profiled => "terminal-diagnostics-status-profiled",
            Self::Unsupported => "terminal-diagnostics-status-unsupported",
            Self::Unknown => "terminal-diagnostics-status-unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilitySource {
    UserOverride,
    Environment,
    TerminalQuery,
    TerminalProfile,
    PlatformDefault,
    ConservativeDefault,
}

impl CapabilitySource {
    pub const fn localization_key(self) -> &'static str {
        match self {
            Self::UserOverride => "terminal-diagnostics-source-user",
            Self::Environment => "terminal-diagnostics-source-environment",
            Self::TerminalQuery => "terminal-diagnostics-source-terminal-query",
            Self::TerminalProfile => "terminal-diagnostics-source-profile",
            Self::PlatformDefault => "terminal-diagnostics-source-platform",
            Self::ConservativeDefault => "terminal-diagnostics-source-conservative",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityEvidence {
    /// Endpoint support is intentionally distinct from complete transport-path
    /// and local-provider availability.
    pub support: Support,
    pub source: CapabilitySource,
    pub path: CapabilityPath,
    pub provider: ProviderReadiness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityPath {
    Clear,
    UserForced,
    Unverified,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderReadiness {
    NotRequired,
    Ready,
    Missing,
}

impl CapabilityEvidence {
    pub const fn supported(source: CapabilitySource) -> Self {
        Self {
            support: Support::Supported,
            source,
            path: CapabilityPath::Clear,
            provider: ProviderReadiness::NotRequired,
        }
    }

    pub const fn profiled(source: CapabilitySource) -> Self {
        Self {
            support: Support::Profiled,
            source,
            path: CapabilityPath::Clear,
            provider: ProviderReadiness::NotRequired,
        }
    }

    pub const fn forced(source: CapabilitySource) -> Self {
        Self {
            support: Support::Forced,
            source,
            path: CapabilityPath::UserForced,
            provider: ProviderReadiness::NotRequired,
        }
    }

    pub const fn unsupported(source: CapabilitySource) -> Self {
        Self {
            support: Support::Unsupported,
            source,
            path: CapabilityPath::Clear,
            provider: ProviderReadiness::NotRequired,
        }
    }

    pub const fn unknown(source: CapabilitySource) -> Self {
        Self {
            support: Support::Unknown,
            source,
            path: CapabilityPath::Clear,
            provider: ProviderReadiness::NotRequired,
        }
    }

    pub const fn with_provider(self, ready: bool) -> Self {
        Self {
            provider: if ready {
                ProviderReadiness::Ready
            } else {
                ProviderReadiness::Missing
            },
            ..self
        }
    }

    pub const fn with_path(self, path: CapabilityPath) -> Self {
        Self { path, ..self }
    }

    pub const fn is_operational(self) -> bool {
        !matches!(self.provider, ProviderReadiness::Missing)
            && matches!(
                self.path,
                CapabilityPath::Clear | CapabilityPath::UserForced
            )
            && matches!(
                self.support,
                Support::Supported | Support::Forced | Support::Profiled
            )
    }

    /// Whether the endpoint is known or profiled to support the feature,
    /// independent of transport policy and provider installation.
    pub const fn is_supported(self) -> bool {
        matches!(
            self.support,
            Support::Supported | Support::Forced | Support::Profiled
        )
    }

    pub fn diagnostic_label(self) -> String {
        let mut qualifiers = Vec::new();
        match self.path {
            CapabilityPath::Unverified => qualifiers.push("path unverified"),
            CapabilityPath::Blocked => qualifiers.push("path blocked"),
            CapabilityPath::Clear | CapabilityPath::UserForced => {}
        }
        if self.provider == ProviderReadiness::Missing
            && matches!(
                self.support,
                Support::Supported | Support::Forced | Support::Profiled
            )
        {
            qualifiers.push("provider unavailable");
        }
        if qualifiers.is_empty() {
            self.support.diagnostic_label().to_owned()
        } else {
            format!(
                "{} ({})",
                self.support.diagnostic_label(),
                qualifiers.join("; ")
            )
        }
    }
}

pub type Capability = CapabilityEvidence;

#[derive(Debug, Clone)]
pub struct TerminalCapabilities {
    pub alternate_screen: Capability,
    pub bracketed_paste: Capability,
    pub focus_reporting: Capability,
    pub mouse_capture: Capability,
    pub keyboard_disambiguation: Capability,
    pub keyboard_alternate_keys: Capability,
    pub keyboard_event_types: Capability,
    /// Kitty protocol flag 4 (`REPORT_ALL_KEYS_AS_ESCAPE_CODES`). iTerm2 only
    /// reports Option as Alt when every key is reported through CSI u.
    pub keyboard_report_all_keys: Capability,
    /// iTerm2-only, per-session Option-as-Alt takeover: agena emits
    /// OSC 1337 SetProfile at startup to switch its own session to a dynamic
    /// profile with "Option Key Sends = Esc+", so Option+Backspace is
    /// reported as ALT+Backspace (CSI u 127;3u) and word-deletes. The
    /// profile is restored on exit; other sessions are unaffected.
    pub iterm2_option_alt: Capability,
    pub default_color_query: Capability,
    pub clipboard_write_native: Capability,
    pub clipboard_write_osc52: Capability,
    pub clipboard_read_native: Capability,
    pub clipboard_read_osc52: Capability,
    pub kitty_rich_clipboard: Capability,
    pub iterm2_file_transfer: Capability,
    pub kitty_file_transfer: Capability,
    pub inline_images: Capability,
    pub hyperlinks: Capability,
    pub synchronized_output: Capability,
    pub window_title: Capability,
    pub terminal_notifications: Capability,
    /// OSC 9;4 (ConEmu) native progress reporting. The terminal renders a
    /// native indeterminate/determinate progress indicator in tab/taskbar
    /// chrome while the session is active.
    pub terminal_progress: Capability,
}

/// A presentation diagnostic derived from terminal evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalDiagnostic {
    pub code: &'static str,
    pub message: String,
}

impl TerminalCapabilities {
    /// Project detailed evidence into the compact terminal-mode contract used
    /// by the lifecycle state machine.
    pub fn lifecycle_capabilities(&self) -> crate::terminal_lifecycle::LifecycleCapabilities {
        crate::terminal_lifecycle::LifecycleCapabilities {
            alternate_screen: self.alternate_screen.is_operational(),
            bracketed_paste: self.bracketed_paste.is_operational(),
            focus_reporting: self.focus_reporting.is_operational(),
            mouse_capture: self.mouse_capture.is_operational(),
            keyboard_disambiguation: self.keyboard_disambiguation.is_operational(),
            keyboard_alternate_keys: self.keyboard_alternate_keys.is_operational(),
            keyboard_event_types: self.keyboard_event_types.is_operational(),
            keyboard_report_all_keys: self.keyboard_report_all_keys.is_operational(),
            iterm2_option_alt: self.iterm2_option_alt.is_operational(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Capability, CapabilityPath, CapabilitySource, ProviderReadiness, Support,
        TerminalCapabilities,
    };

    fn supported_capabilities() -> TerminalCapabilities {
        let capability = Capability::supported(CapabilitySource::TerminalQuery);
        TerminalCapabilities {
            alternate_screen: capability,
            bracketed_paste: capability,
            focus_reporting: capability,
            mouse_capture: capability,
            keyboard_disambiguation: capability,
            keyboard_alternate_keys: capability,
            keyboard_event_types: capability,
            keyboard_report_all_keys: capability,
            iterm2_option_alt: capability,
            default_color_query: capability,
            clipboard_write_native: capability,
            clipboard_write_osc52: capability,
            clipboard_read_native: capability,
            clipboard_read_osc52: capability,
            kitty_rich_clipboard: capability,
            iterm2_file_transfer: capability,
            kitty_file_transfer: capability,
            inline_images: capability,
            hyperlinks: capability,
            synchronized_output: capability,
            window_title: capability,
            terminal_notifications: capability,
            terminal_progress: capability,
        }
    }

    #[test]
    fn endpoint_support_path_and_provider_readiness_remain_independent() {
        let missing_provider =
            Capability::profiled(CapabilitySource::TerminalProfile).with_provider(false);
        assert!(missing_provider.is_supported());
        assert!(!missing_provider.is_operational());
        assert_eq!(missing_provider.path, CapabilityPath::Clear);
        assert_eq!(missing_provider.provider, ProviderReadiness::Missing);

        let unverified_path = Capability::profiled(CapabilitySource::TerminalProfile)
            .with_path(CapabilityPath::Unverified)
            .with_provider(true);
        assert!(unverified_path.is_supported());
        assert!(!unverified_path.is_operational());
        assert_eq!(unverified_path.provider, ProviderReadiness::Ready);
    }

    #[test]
    fn lifecycle_projection_uses_only_operational_capabilities() {
        let mut capabilities = supported_capabilities();
        capabilities.keyboard_event_types =
            Capability::unsupported(CapabilitySource::TerminalProfile);
        capabilities.keyboard_report_all_keys =
            Capability::unsupported(CapabilitySource::TerminalProfile);
        capabilities.mouse_capture =
            Capability::profiled(CapabilitySource::TerminalProfile).with_provider(false);

        let lifecycle = capabilities.lifecycle_capabilities();
        assert!(lifecycle.alternate_screen);
        assert!(!lifecycle.keyboard_event_types);
        assert!(!lifecycle.keyboard_report_all_keys);
        assert!(!lifecycle.mouse_capture);
        assert_eq!(capabilities.alternate_screen.support, Support::Supported);
    }
}
