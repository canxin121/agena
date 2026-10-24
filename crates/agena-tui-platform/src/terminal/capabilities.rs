use std::{fmt, io::IsTerminal};

use crate::{iterm2, kitty};

use agena_tui::i18n::I18n;
use agena_tui::terminal::TerminalEnvironment;
use agena_tui::terminal::TerminalFamily;
use agena_tui::terminal::identity::{IdentitySource, TerminalIdentity};
use agena_tui::terminal::profile::{self as profiles, ProfileSupport};
use agena_tui::terminal::transport::{TransportEvidence, TransportHop, detect as detect_transport};
use agena_tui::terminal_capabilities::{
    Capability, CapabilityPath, CapabilitySource, ProviderReadiness, Support, TerminalCapabilities,
    TerminalDiagnostic,
};
use agena_tui::terminal_color::TerminalColorDetection;
use agena_tui::terminal_overrides as overrides;

#[derive(Debug, Clone)]
/// Context of the terminal.
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
        let mouse_capture = if minimal || identity.family == TerminalFamily::LinuxConsole {
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
        let profile_keyboard_all_keys =
            profile_capability(profiles::keyboard_all_keys(identity.family));
        let all_keys_override = overrides::boolean(
            environment,
            "AGENA_TUI_KEYBOARD_REPORT_ALL_KEYS",
            &mut override_diagnostics,
        );
        let keyboard_all_keys = match all_keys_override {
            Some(true) => Capability::forced(user),
            Some(false) => Capability::unsupported(user),
            None if protocol_barrier && profile_keyboard_all_keys.is_supported() => {
                profile_keyboard_all_keys.with_path(protocol_path)
            }
            None => profile_keyboard_all_keys,
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
        let iterm2_helper_ready = iterm2::upload_utility().is_some();
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

        // Window/tab titles via OSC 0/2 are consumed locally even inside a
        // multiplexer, so they are never path-gated. Notifications use the
        // universal BEL signal as the baseline; the family-specific OSC 9
        // method is selected at emission time. Only the capability gate lives
        // here.
        let window_title_override = overrides::boolean(
            environment,
            "AGENA_TUI_WINDOW_TITLE",
            &mut override_diagnostics,
        );
        let window_title = match window_title_override {
            Some(true) => Capability::forced(user),
            Some(false) => Capability::unsupported(user),
            None => profile_capability(profiles::window_title(identity.family)),
        };
        let notifications_override = overrides::boolean(
            environment,
            "AGENA_TUI_NOTIFICATIONS",
            &mut override_diagnostics,
        );
        let terminal_notifications = match notifications_override {
            Some(true) => Capability::forced(user),
            Some(false) => Capability::unsupported(user),
            None if minimal || !profiles::notifications_bel(identity.family) => {
                Capability::unsupported(profile)
            }
            None => Capability::profiled(profile),
        };
        let progress_override =
            overrides::boolean(environment, "AGENA_TUI_PROGRESS", &mut override_diagnostics);
        let terminal_progress = match progress_override {
            Some(true) => Capability::forced(user),
            Some(false) => Capability::unsupported(user),
            None => profile_capability(profiles::progress(identity.family)),
        };

        let capabilities = TerminalCapabilities {
            alternate_screen: base_screen,
            bracketed_paste: base_screen,
            focus_reporting,
            mouse_capture,
            keyboard_disambiguation: keyboard,
            keyboard_alternate_keys: keyboard,
            keyboard_event_types: Capability::unsupported(conservative),
            keyboard_report_all_keys: keyboard_all_keys,
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
            window_title,
            terminal_notifications,
            terminal_progress,
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

    pub fn diagnostic_summary(&self, i18n: &I18n) -> String {
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
            "{} | confidence={} | layers={layers} (order unknown) | remote={} | multiplexer={} | color={color}/generation:{} | keyboard={} | mouse={} | native-clipboard={} | osc52={}/read:{} | rich-clipboard={} | file-transfer={} | title={} | notifications={} | progress={} | warnings={}",
            self.identity.display_name(),
            self.identity.confidence.label(),
            self.is_remote(),
            self.in_multiplexer(),
            self.color_generation,
            self.capabilities
                .keyboard_disambiguation
                .diagnostic_label(i18n),
            self.capabilities.mouse_capture.diagnostic_label(i18n),
            self.capabilities
                .clipboard_write_native
                .diagnostic_label(i18n),
            self.capabilities
                .clipboard_write_osc52
                .diagnostic_label(i18n),
            self.capabilities
                .clipboard_read_osc52
                .diagnostic_label(i18n),
            self.capabilities
                .kitty_rich_clipboard
                .diagnostic_label(i18n),
            file_transfer_label(&self.capabilities),
            self.capabilities.window_title.diagnostic_label(i18n),
            self.capabilities
                .terminal_notifications
                .diagnostic_label(i18n),
            self.capabilities.terminal_progress.diagnostic_label(i18n),
            self.diagnostics.len(),
        )
    }
}

impl fmt::Display for TerminalContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.diagnostic_summary(&I18n::english()))
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
        && identity.source != IdentitySource::UserOverride
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
