use super::identity::TerminalFamily;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProfileSupport {
    Available,
    Unsupported,
    Unknown,
}

pub(super) const fn keyboard(family: TerminalFamily) -> ProfileSupport {
    match family {
        TerminalFamily::Kitty | TerminalFamily::Ghostty | TerminalFamily::Foot => {
            ProfileSupport::Available
        }
        TerminalFamily::Dumb | TerminalFamily::LinuxConsole | TerminalFamily::AppleTerminal => {
            ProfileSupport::Unsupported
        }
        _ => ProfileSupport::Unknown,
    }
}

pub(super) const fn osc52_write(family: TerminalFamily) -> ProfileSupport {
    match family {
        TerminalFamily::Iterm2
        | TerminalFamily::Kitty
        | TerminalFamily::WezTerm
        | TerminalFamily::Ghostty
        | TerminalFamily::WindowsTerminal
        | TerminalFamily::VsCode
        | TerminalFamily::Alacritty
        | TerminalFamily::Vte
        | TerminalFamily::Konsole
        | TerminalFamily::Foot
        | TerminalFamily::Warp
        | TerminalFamily::Rio
        | TerminalFamily::Contour => ProfileSupport::Available,
        TerminalFamily::Dumb | TerminalFamily::LinuxConsole => ProfileSupport::Unsupported,
        _ => ProfileSupport::Unknown,
    }
}

pub(super) const fn osc52_read(family: TerminalFamily) -> ProfileSupport {
    match family {
        TerminalFamily::Kitty | TerminalFamily::Ghostty => ProfileSupport::Available,
        TerminalFamily::WezTerm | TerminalFamily::Dumb | TerminalFamily::LinuxConsole => {
            ProfileSupport::Unsupported
        }
        _ => ProfileSupport::Unknown,
    }
}

pub(super) const fn inline_images(family: TerminalFamily) -> bool {
    matches!(
        family,
        TerminalFamily::Iterm2
            | TerminalFamily::Kitty
            | TerminalFamily::WezTerm
            | TerminalFamily::Ghostty
    )
}

pub(super) const fn synchronized_output(family: TerminalFamily) -> bool {
    matches!(
        family,
        TerminalFamily::Kitty
            | TerminalFamily::WezTerm
            | TerminalFamily::Ghostty
            | TerminalFamily::Foot
    )
}

pub(super) const fn hyperlinks(family: TerminalFamily) -> ProfileSupport {
    match family {
        TerminalFamily::Dumb | TerminalFamily::LinuxConsole => ProfileSupport::Unsupported,
        TerminalFamily::Unknown | TerminalFamily::XtermCompatible => ProfileSupport::Unknown,
        _ => ProfileSupport::Available,
    }
}
