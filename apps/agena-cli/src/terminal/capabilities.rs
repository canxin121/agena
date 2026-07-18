use std::{fmt, io::IsTerminal};

use agena_tui_components::TerminalRgb;

use crate::{iterm2, kitty};

use super::{
    identity::{TerminalEnvironment, TerminalFamily, TerminalIdentity},
    overrides,
    profiles::{self, ProfileSupport},
    transport::{TransportEvidence, TransportHop, detect_transport},
};

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
    /// Evidence about endpoint support. This is intentionally distinct from
    /// whether the complete transport path and local provider are usable.
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
    pub keyboard_disambiguation: Capability,
    pub keyboard_alternate_keys: Capability,
    pub keyboard_event_types: Capability,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalDiagnostic {
    pub code: &'static str,
    pub message: String,
}

/// Exact evidence used to classify the terminal's light/dark appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalColorSource {
    Osc11,
    Iterm2Osc4,
    ColorFgBg,
    TermBackground,
    VsCodeThemeKind,
    Unavailable,
}

impl TerminalColorSource {
    pub const fn localization_key(self) -> &'static str {
        match self {
            Self::Osc11 => "terminal-diagnostics-color-source-osc11",
            Self::Iterm2Osc4 => "terminal-diagnostics-color-source-iterm-osc4",
            Self::ColorFgBg => "terminal-diagnostics-color-source-colorfgbg",
            Self::TermBackground => "terminal-diagnostics-color-source-term-background",
            Self::VsCodeThemeKind => "terminal-diagnostics-color-source-vscode-theme",
            Self::Unavailable => "terminal-diagnostics-color-source-unavailable",
        }
    }

    const fn diagnostic_label(self) -> &'static str {
        match self {
            Self::Osc11 => "OSC 11",
            Self::Iterm2Osc4 => "iTerm2 OSC 4;-2",
            Self::ColorFgBg => "COLORFGBG",
            Self::TermBackground => "TERM_BACKGROUND",
            Self::VsCodeThemeKind => "VSCODE_THEME_KIND",
            Self::Unavailable => "unavailable",
        }
    }

    pub const fn supports_live_refresh(self) -> bool {
        matches!(self, Self::Osc11 | Self::Iterm2Osc4)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalColorDetection {
    pub background: Option<TerminalRgb>,
    pub source: TerminalColorSource,
}

impl Default for TerminalColorDetection {
    fn default() -> Self {
        Self {
            background: None,
            source: TerminalColorSource::Unavailable,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TerminalContext {
    pub identity: TerminalIdentity,
    pub capabilities: TerminalCapabilities,
    pub transport: Vec<TransportHop>,
    pub transport_evidence: Vec<TransportEvidence>,
    pub color: TerminalColorDetection,
    /// Starts at one after startup detection and advances only when the
    /// detected background or its evidence source actually changes.
    pub color_generation: u64,
    diagnostics: Vec<TerminalDiagnostic>,
}

impl TerminalContext {
    pub fn detect() -> Self {
        let environment = TerminalEnvironment::from_process();
        let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
        Self::detect_from(&environment, interactive)
    }

    pub(super) fn detect_from(environment: &TerminalEnvironment, interactive: bool) -> Self {
        let identity = TerminalIdentity::detect(environment);
        let transport_evidence = detect_transport(environment);
        let transport = transport_evidence
            .iter()
            .map(|evidence| evidence.layer)
            .collect::<Vec<_>>();
        let remote = transport.iter().copied().any(TransportHop::is_remote);
        let multiplexer = transport.iter().copied().any(TransportHop::is_multiplexer);
        let protocol_path = if transport.contains(&TransportHop::Mosh) {
            CapabilityPath::Blocked
        } else if multiplexer {
            CapabilityPath::Unverified
        } else {
            CapabilityPath::Clear
        };
        let protocol_barrier = protocol_path != CapabilityPath::Clear;
        let conservative = CapabilitySource::ConservativeDefault;
        let platform = CapabilitySource::PlatformDefault;
        let profile = CapabilitySource::TerminalProfile;
        let user = CapabilitySource::UserOverride;
        let mut override_diagnostics = Vec::new();
        if let Some(value) = environment.get("AGENA_TUI_HELPER_TIMEOUT_SECS") {
            let valid = value
                .trim()
                .parse::<u64>()
                .is_ok_and(|seconds| (15..=3_600).contains(&seconds));
            if !valid {
                override_diagnostics.push(format!(
                    "invalid AGENA_TUI_HELPER_TIMEOUT_SECS value `{value}`; expected 15..3600"
                ));
            }
        }

        let minimal = identity.family == TerminalFamily::Dumb;
        let base_screen = if minimal {
            Capability::unsupported(profile)
        } else {
            Capability::supported(platform)
        };
        let focus_reporting = if minimal || identity.family == TerminalFamily::LinuxConsole {
            Capability::unsupported(profile)
        } else {
            Capability::supported(platform)
        };

        let profile_capability = |support| match support {
            ProfileSupport::Available => Capability::profiled(profile),
            ProfileSupport::Unsupported => Capability::unsupported(profile),
            ProfileSupport::Unknown => Capability::unknown(conservative),
        };
        let profile_keyboard = profile_capability(profiles::keyboard(identity.family));
        let keyboard_override =
            overrides::keyboard_protocol(environment, &mut override_diagnostics);
        let keyboard = match keyboard_override {
            Some(true) => Capability::forced(user),
            Some(false) => Capability::unsupported(user),
            None if protocol_barrier && profile_keyboard.is_supported() => {
                profile_keyboard.with_path(protocol_path)
            }
            None => profile_keyboard,
        };

        // TerminalRuntime owns a dedicated bounded color transaction before
        // runtime input starts, then promotes this evidence only when an OSC
        // response is actually received.
        let default_color_query = if interactive {
            Capability::unknown(conservative).with_provider(false)
        } else {
            Capability::unsupported(conservative)
        };

        let clipboard_native_override = overrides::boolean(
            environment,
            "AGENA_TUI_NATIVE_CLIPBOARD",
            &mut override_diagnostics,
        );
        let clipboard_write_native = match clipboard_native_override {
            Some(true) => Capability::forced(user),
            Some(false) => Capability::unsupported(user),
            None if remote => Capability::unknown(conservative),
            None => Capability::profiled(platform),
        };
        let clipboard_read_native = clipboard_write_native;

        let osc52_profile = profile_capability(profiles::osc52_write(identity.family));
        let clipboard_write_osc52 =
            match overrides::boolean(environment, "AGENA_TUI_OSC52", &mut override_diagnostics) {
                Some(true) => Capability::forced(user),
                Some(false) => Capability::unsupported(user),
                None if protocol_barrier && osc52_profile.is_supported() => {
                    osc52_profile.with_path(protocol_path)
                }
                None => osc52_profile,
            };
        let clipboard_read_osc52 = if protocol_barrier
            && profiles::osc52_read(identity.family) == ProfileSupport::Available
        {
            profile_capability(profiles::osc52_read(identity.family)).with_path(protocol_path)
        } else {
            profile_capability(profiles::osc52_read(identity.family))
        };

        let direct_profile = |families: &[TerminalFamily], integrated: bool| {
            if !families.contains(&identity.family) {
                Capability::unsupported(profile)
            } else if protocol_barrier {
                Capability::profiled(profile)
                    .with_path(protocol_path)
                    .with_provider(integrated)
            } else {
                Capability::profiled(profile).with_provider(integrated)
            }
        };
        let iterm2_helper_ready =
            iterm2::upload_utility().is_some() || iterm2::download_utility().is_some();
        let iterm2_transfer_override = overrides::boolean(
            environment,
            "AGENA_TUI_ITERM2_FILE_TRANSFER",
            &mut override_diagnostics,
        );
        let iterm2_file_transfer = match iterm2_transfer_override {
            Some(true) => Capability::forced(user).with_provider(iterm2_helper_ready),
            Some(false) => Capability::unsupported(user),
            None if identity.family == TerminalFamily::Iterm2 && !protocol_barrier => {
                Capability::profiled(CapabilitySource::Environment)
                    .with_provider(iterm2_helper_ready)
            }
            None if identity.family == TerminalFamily::Iterm2 => Capability::profiled(profile)
                .with_path(protocol_path)
                .with_provider(iterm2_helper_ready),
            None => Capability::unsupported(profile),
        };

        let kitty_transfer_override = overrides::boolean(
            environment,
            "AGENA_TUI_KITTY_FILE_TRANSFER",
            &mut override_diagnostics,
        );
        let should_probe_kitty =
            identity.family == TerminalFamily::Kitty || kitty_transfer_override == Some(true);
        let kitty_helper = should_probe_kitty.then(kitty::helper).flatten();
        let kitty_transfer_ready = kitty_helper.is_some_and(|helper| helper.transfer);
        let kitty_clipboard_ready = kitty_helper.is_some_and(|helper| helper.clipboard);
        let kitty_file_transfer = match kitty_transfer_override {
            Some(false) => Capability::unsupported(user),
            Some(true) => Capability::forced(user).with_provider(kitty_transfer_ready),
            None => direct_profile(&[TerminalFamily::Kitty], kitty_transfer_ready),
        };
        let kitty_rich_clipboard = direct_profile(&[TerminalFamily::Kitty], kitty_clipboard_ready);

        // Inline-image profiles are provisional here. TerminalRuntime performs
        // the authoritative graphics query only after entering the alternate
        // screen, then marks the provider ready when it selects Kitty, Sixel,
        // or iTerm2. Synchronized output remains diagnostic-only.
        let inline_images = if profiles::inline_images(identity.family) {
            direct_profile(&[identity.family], false)
        } else {
            Capability::unsupported(profile)
        };
        let synchronized_output = if profiles::synchronized_output(identity.family) {
            direct_profile(&[identity.family], false)
        } else {
            Capability::unsupported(profile)
        };
        let hyperlinks =
            profile_capability(profiles::hyperlinks(identity.family)).with_provider(false);

        let capabilities = TerminalCapabilities {
            alternate_screen: base_screen,
            bracketed_paste: base_screen,
            focus_reporting,
            keyboard_disambiguation: keyboard,
            keyboard_alternate_keys: keyboard,
            keyboard_event_types: Capability::unsupported(conservative),
            default_color_query,
            clipboard_write_native,
            clipboard_write_osc52,
            clipboard_read_native,
            clipboard_read_osc52,
            kitty_rich_clipboard,
            iterm2_file_transfer,
            kitty_file_transfer,
            inline_images,
            hyperlinks,
            synchronized_output,
        };
        let diagnostics = build_diagnostics(
            &identity,
            &transport,
            &capabilities,
            environment,
            override_diagnostics,
        );
        Self {
            identity,
            capabilities,
            transport,
            transport_evidence,
            color: TerminalColorDetection::default(),
            color_generation: 0,
            diagnostics,
        }
    }

    pub fn in_tmux(&self) -> bool {
        self.transport.contains(&TransportHop::Tmux)
    }

    pub fn in_multiplexer(&self) -> bool {
        self.transport
            .iter()
            .copied()
            .any(TransportHop::is_multiplexer)
    }

    pub fn is_remote(&self) -> bool {
        self.transport.iter().copied().any(TransportHop::is_remote)
    }

    pub fn diagnostics(&self) -> &[TerminalDiagnostic] {
        &self.diagnostics
    }

    pub(super) fn record_runtime_diagnostic(&mut self, code: &'static str, message: &str) {
        self.diagnostics.push(TerminalDiagnostic {
            code,
            message: message.to_owned(),
        });
    }

    pub(super) fn record_color_detection(&mut self, detection: TerminalColorDetection) {
        if self.color_generation == 0 || self.color != detection {
            self.color = detection;
            self.color_generation = self.color_generation.saturating_add(1);
        }
    }

    pub fn diagnostic_summary(&self) -> String {
        let layers = if self.transport.is_empty() {
            "direct".to_owned()
        } else {
            self.transport
                .iter()
                .map(|hop| hop.label())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let color = self.color.background.map_or_else(
            || "unknown".to_owned(),
            |background| {
                format!(
                    "#{:02X}{:02X}{:02X}/{} ({})",
                    background.red,
                    background.green,
                    background.blue,
                    if background.is_light() {
                        "light"
                    } else {
                        "dark"
                    },
                    self.color.source.diagnostic_label(),
                )
            },
        );
        format!(
            "{} | confidence={} | layers={layers} (order unknown) | remote={} | multiplexer={} | color={color}/generation:{} | keyboard={} | native-clipboard={} | osc52={}/read:{} | rich-clipboard={} | file-transfer={} | warnings={}",
            self.identity.display_name(),
            self.identity.confidence.label(),
            self.is_remote(),
            self.in_multiplexer(),
            self.color_generation,
            self.capabilities.keyboard_disambiguation.diagnostic_label(),
            self.capabilities.clipboard_write_native.diagnostic_label(),
            self.capabilities.clipboard_write_osc52.diagnostic_label(),
            self.capabilities.clipboard_read_osc52.diagnostic_label(),
            self.capabilities.kitty_rich_clipboard.diagnostic_label(),
            file_transfer_label(&self.capabilities),
            self.diagnostics.len(),
        )
    }
}

impl fmt::Display for TerminalContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.diagnostic_summary())
    }
}

fn file_transfer_label(capabilities: &TerminalCapabilities) -> String {
    if capabilities.iterm2_file_transfer.is_operational() {
        "iTerm2".to_owned()
    } else if capabilities.kitty_file_transfer.is_operational() {
        "Kitty".to_owned()
    } else if matches!(
        capabilities.iterm2_file_transfer.support,
        Support::Forced | Support::Profiled
    ) || matches!(
        capabilities.kitty_file_transfer.support,
        Support::Forced | Support::Profiled
    ) {
        "profiled (provider unavailable or policy-blocked)".to_owned()
    } else {
        "none".to_owned()
    }
}

fn build_diagnostics(
    identity: &TerminalIdentity,
    transport: &[TransportHop],
    capabilities: &TerminalCapabilities,
    environment: &TerminalEnvironment,
    override_diagnostics: Vec<String>,
) -> Vec<TerminalDiagnostic> {
    let mut diagnostics = Vec::new();
    if let Some(message) = identity.override_error.as_ref() {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.override.invalid",
            message: message.clone(),
        });
    }
    diagnostics.extend(
        override_diagnostics
            .into_iter()
            .map(|message| TerminalDiagnostic {
                code: "terminal.override.invalid",
                message,
            }),
    );
    if identity.family == TerminalFamily::Unknown {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.identity.unknown",
            message:
                "terminal product could not be identified; conservative VT defaults are active"
                    .to_owned(),
        });
    }
    if identity.family == TerminalFamily::XtermCompatible {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.identity.compatibility-only",
            message:
                "TERM only proves xterm compatibility; the endpoint terminal product is unknown"
                    .to_owned(),
        });
    }
    if transport.iter().copied().any(TransportHop::is_remote)
        && identity.source != super::identity::IdentitySource::UserOverride
    {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.identity.remote-unverified",
            message: "remote environment evidence cannot prove the actual endpoint terminal; capability profiles remain best effort"
                .to_owned(),
        });
    }
    let conflicts = identity.conflicts();
    if !conflicts.is_empty() {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.identity.conflict",
            message: format!(
                "conflicting terminal evidence was detected: {}",
                conflicts
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
    if identity.family == TerminalFamily::Dumb {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.capability.dumb",
            message: "TERM=dumb cannot provide the interactive capabilities required by the TUI"
                .to_owned(),
        });
    }
    if identity.family == TerminalFamily::Ghostty
        && identity.term.as_deref() != Some("xterm-ghostty")
    {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.terminfo.ghostty",
            message: "Ghostty is not advertising xterm-ghostty; advanced keys may be degraded"
                .to_owned(),
        });
    }
    if identity.family == TerminalFamily::Kitty && identity.term.as_deref() != Some("xterm-kitty") {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.terminfo.kitty",
            message: "Kitty is not advertising xterm-kitty; check remote terminfo installation"
                .to_owned(),
        });
    }
    if transport
        .iter()
        .copied()
        .any(|hop| hop.is_multiplexer() || hop == TransportHop::Mosh)
        && matches!(
            capabilities.keyboard_disambiguation.path,
            CapabilityPath::Unverified | CapabilityPath::Blocked
        )
    {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.keyboard.transport",
            message:
                "enhanced keyboard reporting is disabled because passthrough policy is unverified"
                    .to_owned(),
        });
    }
    if identity.family == TerminalFamily::WezTerm
        && capabilities.keyboard_disambiguation.support == Support::Unknown
    {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.keyboard.wezterm",
            message: "WezTerm Kitty keyboard mode is not assumed without an explicit override"
                .to_owned(),
        });
    }
    if identity.family == TerminalFamily::Kitty
        && capabilities.kitty_file_transfer.provider == ProviderReadiness::Missing
    {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.helper.kitty-transfer",
            message: "Kitty was detected, but a compatible executable `kitten transfer` helper was not found"
                .to_owned(),
        });
    }
    if identity.family == TerminalFamily::Kitty
        && capabilities.kitty_rich_clipboard.provider == ProviderReadiness::Missing
    {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.helper.kitty-clipboard",
            message: "Kitty was detected, but a compatible executable `kitten clipboard` helper was not found"
                .to_owned(),
        });
    }
    if identity.family == TerminalFamily::VsCode && environment.has("SSH_CONNECTION") {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.vscode.remote-shell",
            message: "VS Code is reached through a regular SSH shell; local attachment picking requires a transfer provider"
                .to_owned(),
        });
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(pairs: &[(&str, &str)]) -> TerminalContext {
        TerminalContext::detect_from(&TerminalEnvironment::from_pairs(pairs), true)
    }

    #[test]
    fn kitty_profile_separates_terminal_support_from_provider_availability() {
        let context = detect(&[("TERM", "xterm-kitty"), ("KITTY_WINDOW_ID", "1")]);
        assert_eq!(context.identity.family, TerminalFamily::Kitty);
        assert_eq!(
            context.capabilities.keyboard_disambiguation.support,
            Support::Profiled
        );
        assert_eq!(
            context.capabilities.inline_images.support,
            Support::Profiled
        );
        assert_eq!(
            context.capabilities.inline_images.provider,
            ProviderReadiness::Missing
        );
    }

    #[test]
    fn multiplexer_preserves_endpoint_support_but_marks_the_path_unverified() {
        let context = detect(&[
            ("TERM", "xterm-kitty"),
            ("KITTY_WINDOW_ID", "1"),
            ("TMUX", "/tmp/tmux"),
        ]);
        assert_eq!(
            context.capabilities.keyboard_disambiguation.support,
            Support::Profiled
        );
        assert_eq!(
            context.capabilities.keyboard_disambiguation.path,
            CapabilityPath::Unverified
        );
        assert_eq!(
            context.capabilities.kitty_file_transfer.support,
            Support::Profiled
        );
        assert_eq!(
            context.capabilities.kitty_file_transfer.path,
            CapabilityPath::Unverified
        );
        assert!(context.in_multiplexer());
    }

    #[test]
    fn transport_is_evidence_not_claimed_topology() {
        let context = detect(&[("TERM_PROGRAM", "WezTerm"), ("ZELLIJ", "0")]);
        assert!(context.transport.contains(&TransportHop::Zellij));
        assert!(context.diagnostic_summary().contains("order unknown"));
        assert!(!context.diagnostic_summary().contains(" > "));
    }

    #[test]
    fn diagnostic_summary_records_the_exact_detected_color_and_source() {
        let mut context = detect(&[("TERM_PROGRAM", "iTerm.app")]);
        context.record_color_detection(TerminalColorDetection {
            background: Some(TerminalRgb::new(250, 251, 252)),
            source: TerminalColorSource::Iterm2Osc4,
        });

        let summary = context.diagnostic_summary();
        assert!(summary.contains("color=#FAFBFC/light (iTerm2 OSC 4;-2)/generation:1"));
    }

    #[test]
    fn color_generation_advances_only_when_live_evidence_changes() {
        let mut context = detect(&[("TERM_PROGRAM", "iTerm.app")]);
        let light = TerminalColorDetection {
            background: Some(TerminalRgb::new(250, 251, 252)),
            source: TerminalColorSource::Iterm2Osc4,
        };
        context.record_color_detection(light);
        assert_eq!(context.color_generation, 1);

        context.record_color_detection(light);
        assert_eq!(context.color_generation, 1);

        context.record_color_detection(TerminalColorDetection {
            background: Some(TerminalRgb::new(18, 19, 20)),
            source: TerminalColorSource::Iterm2Osc4,
        });
        assert_eq!(context.color_generation, 2);
    }

    #[test]
    fn invalid_boolean_override_produces_a_diagnostic() {
        let context = detect(&[("TERM", "xterm-kitty"), ("AGENA_TUI_OSC52", "perhaps")]);
        assert!(
            context
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message.contains("AGENA_TUI_OSC52"))
        );
    }

    #[test]
    fn explicit_protocol_override_is_forced_not_reported_as_confirmed() {
        let context = detect(&[
            ("TERM_PROGRAM", "WezTerm"),
            ("AGENA_TUI_KEYBOARD_PROTOCOL", "kitty"),
        ]);
        assert_eq!(
            context.capabilities.keyboard_disambiguation.support,
            Support::Forced
        );
        assert!(
            context
                .capabilities
                .keyboard_disambiguation
                .is_operational()
        );
    }

    #[test]
    fn event_type_reporting_is_always_conservatively_disabled() {
        let context = detect(&[("TERM", "xterm-kitty")]);
        assert_eq!(
            context.capabilities.keyboard_event_types,
            Capability::unsupported(CapabilitySource::ConservativeDefault)
        );
    }

    #[test]
    fn endpoint_support_path_policy_and_provider_readiness_are_independent() {
        let missing_provider =
            Capability::profiled(CapabilitySource::TerminalProfile).with_provider(false);
        assert!(missing_provider.is_supported());
        assert!(!missing_provider.is_operational());
        assert_eq!(missing_provider.path, CapabilityPath::Clear);
        assert_eq!(missing_provider.provider, ProviderReadiness::Missing);

        let blocked_path = Capability::profiled(CapabilitySource::TerminalProfile)
            .with_path(CapabilityPath::Unverified)
            .with_provider(true);
        assert!(blocked_path.is_supported());
        assert!(!blocked_path.is_operational());
        assert_eq!(blocked_path.path, CapabilityPath::Unverified);
        assert_eq!(blocked_path.provider, ProviderReadiness::Ready);
    }

    #[test]
    fn multiplexer_does_not_create_false_terminal_family_support() {
        let context = detect(&[("TERM_PROGRAM", "iTerm.app"), ("TMUX", "/tmp/tmux")]);
        assert_eq!(
            context.capabilities.kitty_rich_clipboard.support,
            Support::Unsupported
        );
        assert_eq!(
            context.capabilities.kitty_file_transfer.support,
            Support::Unsupported
        );
    }

    #[test]
    fn remote_native_clipboard_is_not_mistaken_for_the_endpoint_clipboard() {
        let context = detect(&[("TERM", "xterm-256color"), ("SSH_CONNECTION", "a b c d")]);
        assert_eq!(
            context.capabilities.clipboard_read_native.support,
            Support::Unknown
        );
        assert!(!context.capabilities.clipboard_read_native.is_operational());
    }

    #[test]
    fn ssh_client_fallback_also_blocks_remote_native_clipboard_assumptions() {
        let context = detect(&[("TERM", "xterm-256color"), ("SSH_CLIENT", "a b c")]);
        assert!(context.is_remote());
        assert_eq!(
            context.capabilities.clipboard_read_native.support,
            Support::Unknown
        );
        assert!(!context.capabilities.clipboard_read_native.is_operational());
    }

    #[test]
    fn regular_tmux_does_not_imply_iterm_transfer_passthrough() {
        let context = detect(&[
            ("TERM_PROGRAM", "iTerm.app"),
            ("TMUX", "/tmp/tmux"),
            ("ITERM_ENABLE_SHELL_INTEGRATION_WITH_TMUX", "1"),
        ]);
        assert_eq!(
            context.capabilities.iterm2_file_transfer.path,
            CapabilityPath::Unverified
        );
        assert!(!context.capabilities.iterm2_file_transfer.is_operational());
    }

    #[test]
    fn mosh_is_a_known_protocol_blocker_not_an_unknown_mux_policy() {
        let context = detect(&[("TERM", "xterm-kitty"), ("MOSH_CONNECTION", "a b")]);
        assert_eq!(
            context.capabilities.keyboard_disambiguation.path,
            CapabilityPath::Blocked
        );
        assert!(
            !context
                .capabilities
                .keyboard_disambiguation
                .is_operational()
        );
    }
}
