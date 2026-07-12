use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilitySource {
    UserOverride,
    Environment,
    PlatformDefault,
    ConservativeDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityEvidence {
    pub support: Support,
    pub source: CapabilitySource,
}

impl CapabilityEvidence {
    pub const fn supported(source: CapabilitySource) -> Self {
        Self {
            support: Support::Supported,
            source,
        }
    }

    pub const fn unknown(source: CapabilitySource) -> Self {
        Self {
            support: Support::Unknown,
            source,
        }
    }

    pub const fn is_supported(self) -> bool {
        matches!(self.support, Support::Supported)
    }

    pub const fn is_supported_or_unknown(self) -> bool {
        !matches!(self.support, Support::Unsupported)
    }
}

pub type Capability = CapabilityEvidence;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportHop {
    Ssh,
    Mosh,
    Tmux,
    Screen,
    Wsl,
}

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
    pub iterm2_file_transfer: Capability,
}

#[derive(Debug, Clone)]
pub struct TerminalContext {
    pub capabilities: TerminalCapabilities,
    pub transport: Vec<TransportHop>,
}

impl TerminalContext {
    pub fn detect() -> Self {
        let mut transport = Vec::new();
        if env::var_os("SSH_TTY").is_some() || env::var_os("SSH_CONNECTION").is_some() {
            transport.push(TransportHop::Ssh);
        }
        if env::var_os("MOSH_CONNECTION").is_some() {
            transport.push(TransportHop::Mosh);
        }
        if env::var_os("TMUX").is_some() {
            transport.push(TransportHop::Tmux);
        }
        if env::var_os("STY").is_some() {
            transport.push(TransportHop::Screen);
        }
        if env::var_os("WSL_DISTRO_NAME").is_some() || env::var_os("WSL_INTEROP").is_some() {
            transport.push(TransportHop::Wsl);
        }

        let term_program = env::var("TERM_PROGRAM").ok();
        let iterm2 = term_program.as_deref() == Some("iTerm.app")
            || env::var("LC_TERMINAL").is_ok_and(|value| value == "iTerm2")
            || env::var_os("ITERM_SESSION_ID").is_some();
        let remote = transport
            .iter()
            .any(|hop| matches!(hop, TransportHop::Ssh | TransportHop::Mosh));
        let multiplexer = transport
            .iter()
            .any(|hop| matches!(hop, TransportHop::Tmux | TransportHop::Screen));
        let iterm2_through_mux =
            !multiplexer || env::var_os("ITERM_ENABLE_SHELL_INTEGRATION_WITH_TMUX").is_some();
        let interactive_terminal = std::io::IsTerminal::is_terminal(&std::io::stdin())
            && std::io::IsTerminal::is_terminal(&std::io::stdout());

        let platform = CapabilitySource::PlatformDefault;
        let conservative = CapabilitySource::ConservativeDefault;
        let capabilities = TerminalCapabilities {
            alternate_screen: Capability::supported(platform),
            bracketed_paste: Capability::supported(platform),
            focus_reporting: Capability::supported(platform),
            keyboard_disambiguation: Capability::supported(platform),
            keyboard_alternate_keys: Capability::supported(platform),
            // Agena has no UI behavior that needs key-release events. Keeping
            // this unsupported prevents late CSI-u releases escaping on exit.
            keyboard_event_types: Capability {
                support: Support::Unsupported,
                source: conservative,
            },
            // Active queries are conservative under state-sync transports and
            // old multiplexers. Environment palette evidence remains usable.
            default_color_query: if interactive_terminal
                && transport.is_empty()
                && env::var("AGENA_TUI_QUERY_BACKGROUND")
                    .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes"))
            {
                Capability::supported(CapabilitySource::UserOverride)
            } else {
                Capability {
                    support: Support::Unsupported,
                    source: conservative,
                }
            },
            clipboard_write_native: if remote {
                Capability::unknown(conservative)
            } else {
                Capability::supported(platform)
            },
            clipboard_write_osc52: if multiplexer {
                Capability::unknown(conservative)
            } else {
                Capability::supported(platform)
            },
            clipboard_read_native: if remote {
                Capability::unknown(conservative)
            } else {
                Capability::supported(platform)
            },
            iterm2_file_transfer: if iterm2 && iterm2_through_mux {
                Capability::supported(CapabilitySource::Environment)
            } else {
                Capability::unknown(conservative)
            },
        };

        Self {
            capabilities,
            transport,
        }
    }

    pub fn in_tmux(&self) -> bool {
        self.transport.contains(&TransportHop::Tmux)
    }

    pub fn in_multiplexer(&self) -> bool {
        self.transport
            .iter()
            .any(|hop| matches!(hop, TransportHop::Tmux | TransportHop::Screen))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_reporting_is_conservatively_disabled() {
        let context = TerminalContext::detect();
        assert_eq!(
            context.capabilities.keyboard_event_types.support,
            Support::Unsupported
        );
    }
}
