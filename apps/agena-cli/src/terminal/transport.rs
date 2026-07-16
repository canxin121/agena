use super::identity::TerminalEnvironment;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

    pub(super) const fn is_remote(self) -> bool {
        matches!(self, Self::Ssh | Self::Mosh)
    }

    pub(super) const fn is_multiplexer(self) -> bool {
        matches!(self, Self::Tmux | Self::Screen | Self::Zellij)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportEvidence {
    pub layer: TransportHop,
    pub source_key: &'static str,
}

pub(super) fn detect_transport(environment: &TerminalEnvironment) -> Vec<TransportEvidence> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_evidence_without_claiming_nesting_order() {
        let environment = TerminalEnvironment::from_pairs(&[
            ("SSH_CONNECTION", "local remote"),
            ("TMUX", "/tmp/tmux"),
        ]);
        let evidence = detect_transport(&environment);
        assert_eq!(evidence[0].source_key, "SSH_CONNECTION");
        assert_eq!(evidence[1].source_key, "TMUX");
    }

    #[test]
    fn ssh_client_alone_still_marks_a_remote_transport() {
        let environment = TerminalEnvironment::from_pairs(&[("SSH_CLIENT", "a b c")]);
        let evidence = detect_transport(&environment);

        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].layer, TransportHop::Ssh);
        assert_eq!(evidence[0].source_key, "SSH_CLIENT");
    }
}
