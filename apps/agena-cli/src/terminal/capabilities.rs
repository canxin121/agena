use std::{fmt, io::IsTerminal};

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
    PolicyDependent,
    Unsupported,
    Unknown,
}

impl Support {
    pub const fn diagnostic_label(self) -> &'static str {
        match self {
            Self::Supported => "confirmed",
            Self::Forced => "forced",
            Self::Profiled => "profiled",
            Self::PolicyDependent => "policy-dependent",
            Self::Unsupported => "no",
            Self::Unknown => "unknown",
        }
    }

    pub const fn localization_key(self) -> &'static str {
        match self {
            Self::Supported => "terminal-diagnostics-status-confirmed",
            Self::Forced => "terminal-diagnostics-status-forced",
            Self::Profiled => "terminal-diagnostics-status-profiled",
            Self::PolicyDependent => "terminal-diagnostics-status-policy-dependent",
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
    HelperProbe,
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
            Self::HelperProbe => "terminal-diagnostics-source-helper",
            Self::TerminalProfile => "terminal-diagnostics-source-profile",
            Self::PlatformDefault => "terminal-diagnostics-source-platform",
            Self::ConservativeDefault => "terminal-diagnostics-source-conservative",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityEvidence {
    pub support: Support,
    pub source: CapabilitySource,
    pub integration_ready: bool,
}

impl CapabilityEvidence {
    pub const fn supported(source: CapabilitySource) -> Self {
        Self {
            support: Support::Supported,
            source,
            integration_ready: true,
        }
    }

    pub const fn profiled(source: CapabilitySource) -> Self {
        Self {
            support: Support::Profiled,
            source,
            integration_ready: true,
        }
    }

    pub const fn forced(source: CapabilitySource) -> Self {
        Self {
            support: Support::Forced,
            source,
            integration_ready: true,
        }
    }

    pub const fn policy_dependent(source: CapabilitySource) -> Self {
        Self {
            support: Support::PolicyDependent,
            source,
            integration_ready: true,
        }
    }

    pub const fn unsupported(source: CapabilitySource) -> Self {
        Self {
            support: Support::Unsupported,
            source,
            integration_ready: false,
        }
    }

    pub const fn unknown(source: CapabilitySource) -> Self {
        Self {
            support: Support::Unknown,
            source,
            integration_ready: true,
        }
    }

    pub const fn with_integration(self, ready: bool) -> Self {
        Self {
            integration_ready: ready,
            ..self
        }
    }

    pub const fn with_source(self, source: CapabilitySource) -> Self {
        Self { source, ..self }
    }

    pub const fn is_available(self) -> bool {
        self.integration_ready
            && matches!(
                self.support,
                Support::Supported | Support::Forced | Support::Profiled
            )
    }

    pub const fn is_available_or_unknown(self) -> bool {
        self.integration_ready
            && !matches!(
                self.support,
                Support::Unsupported | Support::PolicyDependent
            )
    }

    pub const fn is_supported(self) -> bool {
        self.is_available()
    }

    pub fn diagnostic_label(self) -> String {
        if self.integration_ready {
            self.support.diagnostic_label().to_owned()
        } else if matches!(
            self.support,
            Support::Supported | Support::Forced | Support::Profiled
        ) {
            format!("{} (provider unavailable)", self.support.diagnostic_label())
        } else {
            self.support.diagnostic_label().to_owned()
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

#[derive(Debug, Clone)]
pub struct TerminalContext {
    pub identity: TerminalIdentity,
    pub capabilities: TerminalCapabilities,
    pub transport: Vec<TransportHop>,
    pub transport_evidence: Vec<TransportEvidence>,
    diagnostics: Vec<TerminalDiagnostic>,
}

impl TerminalContext {
    pub fn detect() -> Self {
        let environment = TerminalEnvironment::from_process();
        let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
        Self::detect_from(&environment, interactive)
    }

    fn detect_from(environment: &TerminalEnvironment, interactive: bool) -> Self {
        let identity = TerminalIdentity::detect(environment);
        let transport_evidence = detect_transport(environment);
        let transport = transport_evidence
            .iter()
            .map(|evidence| evidence.layer)
            .collect::<Vec<_>>();
        let remote = transport.iter().copied().any(TransportHop::is_remote);
        let multiplexer = transport.iter().copied().any(TransportHop::is_multiplexer);
        let protocol_barrier = multiplexer || transport.contains(&TransportHop::Mosh);
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
            None if protocol_barrier && profile_keyboard.is_available() => {
                Capability::policy_dependent(conservative)
            }
            None => profile_keyboard,
        };

        // TerminalRuntime folds OSC 11 into the single bounded negotiation
        // before EventStream is created, then promotes this evidence when a
        // response is received.
        let default_color_query = if interactive {
            Capability::unknown(conservative).with_integration(false)
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
                None if protocol_barrier => Capability::policy_dependent(conservative),
                None => osc52_profile,
            };
        let clipboard_read_osc52 = if protocol_barrier
            && profiles::osc52_read(identity.family) == ProfileSupport::Available
        {
            Capability::policy_dependent(conservative)
        } else {
            profile_capability(profiles::osc52_read(identity.family))
        };

        let direct_profile = |families: &[TerminalFamily], integrated: bool| {
            if protocol_barrier {
                Capability::policy_dependent(conservative).with_integration(integrated)
            } else if families.contains(&identity.family) {
                Capability::profiled(profile).with_integration(integrated)
            } else {
                Capability::unsupported(profile)
            }
        };
        let iterm2_through_mux = !transport.contains(&TransportHop::Mosh)
            && (!multiplexer || environment.has("ITERM_ENABLE_SHELL_INTEGRATION_WITH_TMUX"));
        let iterm2_helper_ready =
            iterm2::upload_utility().is_some() || iterm2::download_utility().is_some();
        let iterm2_file_transfer =
            if identity.family == TerminalFamily::Iterm2 && iterm2_through_mux {
                Capability::profiled(CapabilitySource::Environment)
                    .with_integration(iterm2_helper_ready)
            } else if identity.family == TerminalFamily::Iterm2 {
                Capability::policy_dependent(conservative).with_integration(iterm2_helper_ready)
            } else {
                Capability::unsupported(profile)
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
            Some(true) if protocol_barrier => {
                Capability::forced(user).with_integration(kitty_transfer_ready)
            }
            Some(true) => Capability::forced(user).with_integration(kitty_transfer_ready),
            None => direct_profile(&[TerminalFamily::Kitty], kitty_transfer_ready),
        };
        let kitty_file_transfer = if kitty_transfer_ready && kitty_file_transfer.is_available() {
            kitty_file_transfer.with_source(CapabilitySource::HelperProbe)
        } else {
            kitty_file_transfer
        };
        let kitty_rich_clipboard = direct_profile(&[TerminalFamily::Kitty], kitty_clipboard_ready);
        let kitty_rich_clipboard = if kitty_clipboard_ready && kitty_rich_clipboard.is_available() {
            kitty_rich_clipboard.with_source(CapabilitySource::HelperProbe)
        } else {
            kitty_rich_clipboard
        };

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
            profile_capability(profiles::hyperlinks(identity.family)).with_integration(false);

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
        format!(
            "{} | confidence={} | layers={layers} (order unknown) | remote={} | multiplexer={} | keyboard={} | native-clipboard={} | osc52={}/read:{} | rich-clipboard={} | file-transfer={} | warnings={}",
            self.identity.display_name(),
            self.identity.confidence.label(),
            self.is_remote(),
            self.in_multiplexer(),
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
    if capabilities.iterm2_file_transfer.is_available() {
        "iTerm2".to_owned()
    } else if capabilities.kitty_file_transfer.is_available() {
        "Kitty".to_owned()
    } else if matches!(
        capabilities.iterm2_file_transfer.support,
        Support::Forced | Support::Profiled | Support::PolicyDependent
    ) || matches!(
        capabilities.kitty_file_transfer.support,
        Support::Forced | Support::Profiled | Support::PolicyDependent
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
        && capabilities.keyboard_disambiguation.support == Support::PolicyDependent
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
        && !capabilities.kitty_file_transfer.integration_ready
    {
        diagnostics.push(TerminalDiagnostic {
            code: "terminal.helper.kitty-transfer",
            message: "Kitty was detected, but a compatible executable `kitten transfer` helper was not found"
                .to_owned(),
        });
    }
    if identity.family == TerminalFamily::Kitty
        && !capabilities.kitty_rich_clipboard.integration_ready
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
        assert!(!context.capabilities.inline_images.integration_ready);
    }

    #[test]
    fn multiplexer_marks_passthrough_protocols_policy_dependent() {
        let context = detect(&[
            ("TERM", "xterm-kitty"),
            ("KITTY_WINDOW_ID", "1"),
            ("TMUX", "/tmp/tmux"),
        ]);
        assert_eq!(
            context.capabilities.keyboard_disambiguation.support,
            Support::PolicyDependent
        );
        assert_eq!(
            context.capabilities.kitty_file_transfer.support,
            Support::PolicyDependent
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
        assert!(context.capabilities.keyboard_disambiguation.is_available());
    }

    #[test]
    fn event_type_reporting_is_always_conservatively_disabled() {
        let context = detect(&[("TERM", "xterm-kitty")]);
        assert_eq!(
            context.capabilities.keyboard_event_types,
            Capability::unsupported(CapabilitySource::ConservativeDefault)
        );
    }
}
