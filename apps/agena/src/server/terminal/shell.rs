use portable_pty::CommandBuilder;

pub(super) fn shell_command(cwd: &str) -> CommandBuilder {
    #[cfg(windows)]
    let shell = "powershell.exe".to_owned();
    #[cfg(unix)]
    // portable-pty resolves $SHELL, then the user's password-database shell.
    // GUI/service launches often have no $SHELL; do not silently replace zsh
    // (and its startup files) with bash in that case.
    let shell = CommandBuilder::new_default_prog().get_shell();

    interactive_shell_command(&shell, cwd)
}

fn interactive_shell_command(shell: &str, cwd: &str) -> CommandBuilder {
    let mut command = CommandBuilder::new(shell);
    let name = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    match name.trim_end_matches(".exe") {
        "pwsh" | "powershell" => command.arg("-NoLogo"),
        "cmd" => {}
        _ => {
            // Login startup loads PATH/profile exports; interactive startup
            // also loads .zshrc/.bashrc, aliases and version managers.
            command.args(["-l", "-i"]);
        }
    }
    command.cwd(cwd);
    command.env("SHELL", shell);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command
}

pub(super) fn tmux_command(
    session_name: &str,
    cwd: &str,
    shell: &CommandBuilder,
) -> CommandBuilder {
    let mut command = CommandBuilder::new("tmux");
    command.args(["new-session", "-A", "-s", session_name, "-c", cwd]);
    // An existing tmux server retains the environment from its own launch.
    // Forward the startup context for this terminal so it reads the current
    // user's profiles, including a configured ZDOTDIR or XDG_CONFIG_HOME.
    #[cfg(unix)]
    let mut unset = Vec::new();
    for key in [
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "PATH",
        "ZDOTDIR",
        "XDG_CONFIG_HOME",
        "COLORTERM",
    ] {
        if let Some(value) = shell.get_env(key) {
            let mut assignment = std::ffi::OsString::from(key);
            assignment.push("=");
            assignment.push(value);
            command.arg("-e");
            command.arg(assignment);
        } else {
            #[cfg(unix)]
            unset.push(key);
        }
    }
    // Supply argv directly; tmux's default-shell may differ from the user's
    // selected shell. Separate arguments also preserve spaces in shell paths.
    command.arg("--");
    #[cfg(unix)]
    if !unset.is_empty() {
        // new-session -e only sets values. Remove startup variables absent
        // from this launch so the tmux server cannot restore stale values.
        command.arg("/usr/bin/env");
        for key in unset {
            command.args(["-u", key]);
        }
    }
    command.args(shell.get_argv());
    command.cwd(cwd);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env_remove("TMUX");
    command.env_remove("TMUX_PANE");
    command
}

#[cfg(test)]
mod tests;
