use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(super) struct ResolvedCommand {
    pub executable: PathBuf,
    pub cwd: PathBuf,
}

/// Resolve once using the child's environment, then keep this absolute path
/// for every verified restart. Relative PATH entries belong to the child's
/// working directory, and must not be searched from the host's directory.
pub(super) fn resolve(
    command: &str,
    env: &HashMap<String, String>,
    cwd: Option<&Path>,
) -> Result<ResolvedCommand, String> {
    let cwd = std::path::absolute(cwd.unwrap_or_else(|| Path::new(".")))
        .map_err(|error| format!("resolve stdio working directory: {error}"))?;
    let paths = env
        .iter()
        .filter(|(key, _)| {
            if cfg!(windows) {
                key.eq_ignore_ascii_case("PATH")
            } else {
                *key == "PATH"
            }
        })
        // Command::env is applied in this same iteration order. On Windows,
        // differently cased PATH entries refer to one variable; the last wins.
        .last()
        .map(|(_, value)| OsString::from(value))
        .or_else(|| std::env::var_os("PATH"));
    let paths = paths
        .map(|paths| std::env::join_paths(std::env::split_paths(&paths).map(|path| cwd.join(path))))
        .transpose()
        .map_err(|error| format!("resolve stdio search path: {error}"))?;
    let executable = which::which_in(command, paths, &cwd)
        .map_err(|error| format!("resolve checksummed stdio executable `{command}`: {error}"))?;
    Ok(ResolvedCommand { executable, cwd })
}
