use std::{path::Path, process::Command};

/// Start a fixture server with its own home/configuration and no inherited
/// Agena settings, provider credentials, or proxy configuration. Keep only
/// process execution/temp-directory settings needed by local fixture tools.
pub fn isolated_server_command(test_home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agena"));
    command.env_clear();
    for name in [
        "PATH",
        "TMPDIR",
        "TMP",
        "TEMP",
        "SYSTEMROOT",
        "RUST_MIN_STACK",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.env("HOME", test_home);
    command
}
