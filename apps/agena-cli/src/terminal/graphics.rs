use std::path::Path;

use agena::config::TuiGraphicsModeConfig;

use crate::math_render::GraphicsProtocolHint;

use super::{
    TerminalContext,
    identity::{IdentityConfidence, TerminalFamily},
    transport::TransportHop,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GraphicsPreference {
    Auto,
    Native,
    Unicode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TmuxGraphicsPath {
    Verified,
    Nested,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GraphicsTransportPolicy {
    pub(super) probe_native: bool,
    pub(super) through_tmux: bool,
    pub(super) reason: &'static str,
    pub(super) protocol_hint: Option<GraphicsProtocolHint>,
}

impl GraphicsTransportPolicy {
    pub(super) fn detect(
        context: &TerminalContext,
        configured_mode: TuiGraphicsModeConfig,
    ) -> Self {
        let preference = match configured_mode {
            TuiGraphicsModeConfig::Auto => GraphicsPreference::Auto,
            TuiGraphicsModeConfig::Native => GraphicsPreference::Native,
            TuiGraphicsModeConfig::Unicode => GraphicsPreference::Unicode,
        };
        let tmux_path = context.in_tmux().then(probe_tmux_graphics_path);
        let mut policy = policy_from_evidence(context, preference, tmux_path);
        policy.protocol_hint = if policy.probe_native {
            protocol_hint_from_identity(context)
        } else {
            None
        };
        policy
    }
}

fn protocol_hint_from_identity(context: &TerminalContext) -> Option<GraphicsProtocolHint> {
    // Kitty and Sixel advertise themselves through the bounded terminal
    // query. iTerm2 has no reliable equivalent capability response, so a
    // strong product identity is the only standards-compatible fallback.
    // Apply it only after the transport policy has admitted the path. A user
    // override is authoritative; automatically inferred product evidence must
    // be strong and internally consistent.
    let trusted = context.identity.confidence == IdentityConfidence::Explicit
        || (context.identity.confidence == IdentityConfidence::Strong
            && context.identity.conflicts().is_empty());
    if !trusted {
        return None;
    }
    match context.identity.family {
        TerminalFamily::Iterm2 | TerminalFamily::WezTerm => Some(GraphicsProtocolHint::Iterm2),
        TerminalFamily::Kitty | TerminalFamily::Ghostty => Some(GraphicsProtocolHint::Kitty),
        _ => None,
    }
}

fn policy_from_evidence(
    context: &TerminalContext,
    preference: GraphicsPreference,
    tmux_path: Option<TmuxGraphicsPath>,
) -> GraphicsTransportPolicy {
    let through_tmux = context.in_tmux();
    if preference == GraphicsPreference::Unicode {
        return GraphicsTransportPolicy {
            probe_native: false,
            through_tmux,
            reason: "disabled by ui.tui.graphics=unicode",
            protocol_hint: None,
        };
    }
    if preference == GraphicsPreference::Native {
        return GraphicsTransportPolicy {
            probe_native: true,
            through_tmux,
            reason: "enabled by ui.tui.graphics=native",
            protocol_hint: None,
        };
    }

    if context.transport.contains(&TransportHop::Mosh) {
        return GraphicsTransportPolicy {
            probe_native: false,
            through_tmux,
            reason: "Mosh does not provide a transparent graphics-protocol path",
            protocol_hint: None,
        };
    }
    if context
        .transport
        .iter()
        .any(|hop| matches!(hop, TransportHop::Screen | TransportHop::Zellij))
    {
        return GraphicsTransportPolicy {
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
            return GraphicsTransportPolicy {
                probe_native: false,
                through_tmux: true,
                reason,
                protocol_hint: None,
            };
        }
    }

    GraphicsTransportPolicy {
        // SSH is a byte-transparent transport and is deliberately not a
        // blocker. The endpoint query, optionally wrapped for tmux, remains
        // authoritative for protocol selection.
        probe_native: true,
        through_tmux,
        reason: if through_tmux {
            "tmux passthrough is enabled"
        } else if context.is_remote() {
            "SSH path will be verified by the endpoint query"
        } else {
            "direct terminal path"
        },
        protocol_hint: None,
    }
}

fn probe_tmux_graphics_path() -> TmuxGraphicsPath {
    let (passthrough, client_term) = std::thread::scope(|scope| {
        let passthrough = scope.spawn(|| {
            crate::helper_runner::run_probe(
                Path::new("tmux"),
                ["show-options", "-p", "-qv", "allow-passthrough"],
            )
        });
        let client_term = scope.spawn(|| {
            crate::helper_runner::run_probe(
                Path::new("tmux"),
                ["display-message", "-p", "#{client_termname}"],
            )
        });
        (passthrough.join().ok(), client_term.join().ok())
    });
    let passthrough = passthrough
        .and_then(Result::ok)
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|value| passthrough_value_is_enabled(value.trim()));
    let client_term = client_term
        .and_then(Result::ok)
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if client_term
        .as_deref()
        .is_some_and(client_term_implies_nested_mux)
    {
        TmuxGraphicsPath::Nested
    } else if passthrough && client_term.is_some() {
        TmuxGraphicsPath::Verified
    } else {
        TmuxGraphicsPath::Unverified
    }
}

fn passthrough_value_is_enabled(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "on" | "all")
}

fn client_term_implies_nested_mux(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value == "screen"
        || value.starts_with("screen-")
        || value == "tmux"
        || value.starts_with("tmux-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{TerminalContext, identity::TerminalEnvironment};

    fn context(pairs: &[(&str, &str)]) -> TerminalContext {
        TerminalContext::detect_from(&TerminalEnvironment::from_pairs(pairs), true)
    }

    #[test]
    fn ssh_is_not_treated_as_a_graphics_protocol_barrier() {
        let context = context(&[("TERM", "xterm-kitty"), ("SSH_CONNECTION", "a b c d")]);
        let policy = policy_from_evidence(&context, GraphicsPreference::Auto, None);
        assert!(policy.probe_native);
        assert!(!policy.through_tmux);
    }

    #[test]
    fn strong_iterm_identity_supplies_the_unqueryable_protocol_hint_over_ssh() {
        let context = context(&[
            ("TERM", "xterm-256color"),
            ("LC_TERMINAL", "iTerm2"),
            ("SSH_CONNECTION", "a b c d"),
        ]);
        let policy = GraphicsTransportPolicy::detect(&context, TuiGraphicsModeConfig::Auto);
        assert!(policy.probe_native);
        assert_eq!(policy.protocol_hint, Some(GraphicsProtocolHint::Iterm2));
    }

    #[test]
    fn conflicting_terminal_identity_never_forces_a_protocol_hint() {
        let context = context(&[
            ("TERM", "xterm-kitty"),
            ("LC_TERMINAL", "iTerm2"),
            ("KITTY_WINDOW_ID", "1"),
        ]);
        assert_eq!(protocol_hint_from_identity(&context), None);
    }

    #[test]
    fn explicit_terminal_override_remains_authoritative() {
        let context = context(&[
            ("AGENA_TUI_TERMINAL", "iterm2"),
            ("TERM", "xterm-kitty"),
            ("KITTY_WINDOW_ID", "1"),
            ("SSH_CONNECTION", "a b c d"),
        ]);
        let policy = GraphicsTransportPolicy::detect(&context, TuiGraphicsModeConfig::Auto);
        assert_eq!(policy.protocol_hint, Some(GraphicsProtocolHint::Iterm2));
    }

    #[test]
    fn tmux_requires_observed_passthrough_in_auto_mode() {
        let context = context(&[("TERM", "tmux-256color"), ("TMUX", "/tmp/tmux")]);
        assert!(
            policy_from_evidence(
                &context,
                GraphicsPreference::Auto,
                Some(TmuxGraphicsPath::Verified)
            )
            .probe_native
        );
        assert!(
            !policy_from_evidence(
                &context,
                GraphicsPreference::Auto,
                Some(TmuxGraphicsPath::Unverified)
            )
            .probe_native
        );
        assert!(
            !policy_from_evidence(
                &context,
                GraphicsPreference::Auto,
                Some(TmuxGraphicsPath::Nested)
            )
            .probe_native
        );
    }

    #[test]
    fn mosh_is_conservative_unless_the_user_forces_native_graphics() {
        let context = context(&[("TERM", "xterm-256color"), ("MOSH_CONNECTION", "a b")]);
        assert!(!policy_from_evidence(&context, GraphicsPreference::Auto, None).probe_native);
        assert!(policy_from_evidence(&context, GraphicsPreference::Native, None).probe_native);
    }

    #[test]
    fn parses_all_tmux_passthrough_modes() {
        assert!(passthrough_value_is_enabled("on"));
        assert!(passthrough_value_is_enabled("ALL"));
        assert!(!passthrough_value_is_enabled("off"));
        assert!(!passthrough_value_is_enabled(""));
        assert!(client_term_implies_nested_mux("tmux-256color"));
        assert!(client_term_implies_nested_mux("screen-256color"));
        assert!(!client_term_implies_nested_mux("xterm-256color"));
    }
}
