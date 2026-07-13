use std::{env, path::Path};

use crate::math_render::GraphicsProtocolHint;

use super::{
    TerminalContext,
    identity::{IdentitySource, TerminalFamily},
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
    pub(super) diagnostic: Option<String>,
    pub(super) protocol_hint: Option<GraphicsProtocolHint>,
}

impl GraphicsTransportPolicy {
    pub(super) fn detect(context: &TerminalContext) -> Self {
        let (preference, diagnostic) = preference_from_env();
        let tmux_path = context.in_tmux().then(probe_tmux_graphics_path);
        let mut policy = policy_from_evidence(context, preference, tmux_path);
        policy.diagnostic = diagnostic;
        policy.protocol_hint = if context.identity.source == IdentitySource::UserOverride {
            match context.identity.family {
                TerminalFamily::Iterm2 => Some(GraphicsProtocolHint::Iterm2),
                TerminalFamily::Kitty => Some(GraphicsProtocolHint::Kitty),
                _ => None,
            }
        } else {
            None
        };
        policy
    }
}

fn preference_from_env() -> (GraphicsPreference, Option<String>) {
    let Ok(value) = env::var("AGENA_TUI_GRAPHICS") else {
        return (GraphicsPreference::Auto, None);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => (GraphicsPreference::Auto, None),
        "native" | "image" | "images" | "on" | "1" => (GraphicsPreference::Native, None),
        "unicode" | "text" | "halfblocks" | "off" | "0" => (GraphicsPreference::Unicode, None),
        _ => (
            GraphicsPreference::Auto,
            Some(format!(
                "invalid AGENA_TUI_GRAPHICS value `{value}`; expected auto, native, or unicode"
            )),
        ),
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
            reason: "disabled by AGENA_TUI_GRAPHICS=unicode",
            diagnostic: None,
            protocol_hint: None,
        };
    }
    if preference == GraphicsPreference::Native {
        return GraphicsTransportPolicy {
            probe_native: true,
            through_tmux,
            reason: "enabled by AGENA_TUI_GRAPHICS=native",
            diagnostic: None,
            protocol_hint: None,
        };
    }

    if context.transport.contains(&TransportHop::Mosh) {
        return GraphicsTransportPolicy {
            probe_native: false,
            through_tmux,
            reason: "Mosh does not provide a transparent graphics-protocol path",
            diagnostic: None,
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
            diagnostic: None,
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
                diagnostic: None,
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
        diagnostic: None,
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
