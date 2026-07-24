use std::path::Path;

use agena_tui::terminal_graphics::{GraphicsMode, GraphicsTransportPolicy, TmuxGraphicsPath};

use super::TerminalContext;

/// The application host is responsible only for collecting bounded tmux
/// evidence. The graphics policy itself lives in `agena-tui`.
pub(super) fn detect(context: &TerminalContext, mode: GraphicsMode) -> GraphicsTransportPolicy {
    let tmux_path = context.in_tmux().then(probe_tmux_graphics_path);
    GraphicsTransportPolicy::detect(&context.identity, &context.transport, mode, tmux_path)
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
    use super::{client_term_implies_nested_mux, passthrough_value_is_enabled};

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
