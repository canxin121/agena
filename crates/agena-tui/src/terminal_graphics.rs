//! Terminal-native graphics policy values.

use crate::{
    terminal::identity::{IdentityConfidence, TerminalIdentity},
    terminal::transport::TransportHop,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsMode {
    Auto,
    Native,
    Unicode,
}

/// The observed tmux passthrough path, collected by the process host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxGraphicsPath {
    Verified,
    Nested,
    Unverified,
}

/// A conservative protocol hint derived from trusted terminal identity.
///
/// The actual image renderer remains free to reject this hint when its live
/// capability query discovers a stronger result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsProtocolHint {
    Iterm2,
    Kitty,
}

/// A pure graphics-probe decision derived from terminal identity and observed
/// transport evidence. The host supplies tmux evidence; this module never
/// spawns helpers or reads application configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsTransportPolicy {
    pub probe_native: bool,
    pub through_tmux: bool,
    pub reason: &'static str,
    pub protocol_hint: Option<GraphicsProtocolHint>,
}

impl GraphicsTransportPolicy {
    pub fn detect(
        identity: &TerminalIdentity,
        transport: &[TransportHop],
        mode: GraphicsMode,
        tmux_path: Option<TmuxGraphicsPath>,
    ) -> Self {
        let through_tmux = transport.contains(&TransportHop::Tmux);
        let mut policy = Self::from_evidence(transport, mode, tmux_path);
        policy.protocol_hint = policy
            .probe_native
            .then(|| protocol_hint_from_identity(identity))
            .flatten();
        debug_assert_eq!(policy.through_tmux, through_tmux);
        policy
    }

    fn from_evidence(
        transport: &[TransportHop],
        mode: GraphicsMode,
        tmux_path: Option<TmuxGraphicsPath>,
    ) -> Self {
        let through_tmux = transport.contains(&TransportHop::Tmux);
        if mode == GraphicsMode::Unicode {
            return Self {
                probe_native: false,
                through_tmux,
                reason: "disabled by ui.tui.graphics=unicode",
                protocol_hint: None,
            };
        }
        if mode == GraphicsMode::Native {
            return Self {
                probe_native: true,
                through_tmux,
                reason: "enabled by ui.tui.graphics=native",
                protocol_hint: None,
            };
        }
        if transport.contains(&TransportHop::Mosh) {
            return Self {
                probe_native: false,
                through_tmux,
                reason: "Mosh does not provide a transparent graphics-protocol path",
                protocol_hint: None,
            };
        }
        if transport
            .iter()
            .any(|hop| matches!(hop, TransportHop::Screen | TransportHop::Zellij))
        {
            return Self {
                probe_native: false,
                through_tmux,
                reason: "screen/Zellij graphics passthrough is not verifiable",
                protocol_hint: None,
            };
        }
        if through_tmux {
            let reason = match tmux_path {
                Some(TmuxGraphicsPath::Verified) => None,
                Some(TmuxGraphicsPath::Nested) => Some(
                    "tmux is nested inside another multiplexer whose graphics path cannot be verified",
                ),
                Some(TmuxGraphicsPath::Unverified) | None => {
                    Some("tmux allow-passthrough is not enabled for the current pane")
                }
            };
            if let Some(reason) = reason {
                return Self {
                    probe_native: false,
                    through_tmux: true,
                    reason,
                    protocol_hint: None,
                };
            }
        }
        Self {
            probe_native: true,
            through_tmux,
            reason: if through_tmux {
                "tmux passthrough is enabled"
            } else if transport.iter().copied().any(TransportHop::is_remote) {
                "SSH path will be verified by the endpoint query"
            } else {
                "direct terminal path"
            },
            protocol_hint: None,
        }
    }
}

fn protocol_hint_from_identity(identity: &TerminalIdentity) -> Option<GraphicsProtocolHint> {
    let trusted = identity.confidence == IdentityConfidence::Explicit
        || (identity.confidence == IdentityConfidence::Strong && identity.conflicts().is_empty());
    if !trusted {
        return None;
    }
    match identity.family {
        crate::terminal::TerminalFamily::Iterm2 | crate::terminal::TerminalFamily::WezTerm => {
            Some(GraphicsProtocolHint::Iterm2)
        }
        crate::terminal::TerminalFamily::Kitty | crate::terminal::TerminalFamily::Ghostty => {
            Some(GraphicsProtocolHint::Kitty)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TerminalEnvironment;

    fn identity(pairs: &[(&str, &str)]) -> TerminalIdentity {
        TerminalIdentity::detect(&TerminalEnvironment::from_pairs(pairs))
    }

    #[test]
    fn ssh_is_not_a_graphics_barrier_and_strong_iterm_supplies_a_hint() {
        let identity = identity(&[("LC_TERMINAL", "iTerm2"), ("TERM", "xterm-256color")]);
        let policy = GraphicsTransportPolicy::detect(
            &identity,
            &[TransportHop::Ssh],
            GraphicsMode::Auto,
            None,
        );
        assert!(policy.probe_native);
        assert!(!policy.through_tmux);
        assert_eq!(policy.protocol_hint, Some(GraphicsProtocolHint::Iterm2));
    }

    #[test]
    fn conflicting_identity_never_forces_a_protocol_hint() {
        let identity = identity(&[
            ("TERM", "xterm-kitty"),
            ("LC_TERMINAL", "iTerm2"),
            ("KITTY_WINDOW_ID", "1"),
        ]);
        let policy = GraphicsTransportPolicy::detect(&identity, &[], GraphicsMode::Auto, None);
        assert_eq!(policy.protocol_hint, None);
    }

    #[test]
    fn tmux_requires_verified_passthrough_and_mosh_is_conservative() {
        let identity = identity(&[("TERM", "tmux-256color")]);
        let transport = [TransportHop::Tmux];
        assert!(
            GraphicsTransportPolicy::detect(
                &identity,
                &transport,
                GraphicsMode::Auto,
                Some(TmuxGraphicsPath::Verified),
            )
            .probe_native
        );
        assert!(
            !GraphicsTransportPolicy::detect(
                &identity,
                &transport,
                GraphicsMode::Auto,
                Some(TmuxGraphicsPath::Unverified),
            )
            .probe_native
        );
        assert!(
            !GraphicsTransportPolicy::detect(
                &identity,
                &[TransportHop::Mosh],
                GraphicsMode::Auto,
                None,
            )
            .probe_native
        );
        assert!(
            GraphicsTransportPolicy::detect(
                &identity,
                &[TransportHop::Mosh],
                GraphicsMode::Native,
                None,
            )
            .probe_native
        );
    }
}
