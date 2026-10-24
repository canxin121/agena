//! Kitty integration through the official standalone `kitten` helper.

use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

use crate::{
    helper_runner::{
        helper_is_executable, interrupted_status, run_interactive, run_interactive_guarded,
        run_probe,
    },
    provider_error::ProviderError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Helper for the Kitty graphics protocol.
pub struct KittyHelper {
    pub path: PathBuf,
    pub version: Option<String>,
    pub clipboard: bool,
    pub transfer: bool,
}

pub fn helper() -> Option<&'static KittyHelper> {
    static HELPER: OnceLock<Option<KittyHelper>> = OnceLock::new();
    HELPER.get_or_init(detect_helper).as_ref()
}

pub fn transfer_utility() -> Option<PathBuf> {
    helper()
        .filter(|helper| helper.transfer)
        .map(|helper| helper.path.clone())
}

pub fn clipboard_utility() -> Option<PathBuf> {
    helper()
        .filter(|helper| helper.clipboard)
        .map(|helper| helper.path.clone())
}

fn detect_helper() -> Option<KittyHelper> {
    let path = helper_candidates()
        .into_iter()
        .find(|path| helper_is_executable(path))?;
    Some(probe_helper(path))
}

fn helper_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("AGENA_TUI_KITTEN")
        && !path.is_empty()
    {
        candidates.push(PathBuf::from(path));
    }
    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidates.push(home.join(".local/kitty.app/bin/kitten"));
        candidates.push(home.join(".local/bin/kitten"));
    }
    if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|directory| directory.join("kitten")));
    }
    candidates
}

fn probe_helper(path: PathBuf) -> KittyHelper {
    let (version, clipboard, transfer) = std::thread::scope(|scope| {
        let version = scope.spawn(|| {
            run_probe(path.as_path(), ["--version"])
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| {
                    let text = if output.stdout.is_empty() {
                        String::from_utf8_lossy(&output.stderr).into_owned()
                    } else {
                        String::from_utf8_lossy(&output.stdout).into_owned()
                    };
                    first_version_token(text.as_str())
                })
        });
        let clipboard = scope.spawn(|| {
            run_probe(path.as_path(), ["clipboard", "--help"])
                .is_ok_and(|output| output.status.success())
        });
        let transfer = scope.spawn(|| {
            run_probe(path.as_path(), ["transfer", "--help"])
                .is_ok_and(|output| output.status.success())
        });
        (
            version.join().unwrap_or(None),
            clipboard.join().unwrap_or(false),
            transfer.join().unwrap_or(false),
        )
    });
    KittyHelper {
        path,
        version,
        clipboard,
        transfer,
    }
}

fn first_version_token(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|token| token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.'))
        .find(|token| {
            !token.is_empty()
                && token.contains('.')
                && token
                    .split('.')
                    .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
        })
        .map(ToOwned::to_owned)
}

pub(crate) fn request_upload(
    local_sources: &[String],
    remote_destination: &Path,
    guard: &mut dyn FnMut() -> Result<(), ProviderError>,
) -> Result<(), ProviderError> {
    if local_sources.is_empty() {
        return Err(ProviderError::Unsupported(
            "no local Kitty upload paths were provided".to_owned(),
        ));
    }
    let utility = transfer_utility().ok_or_else(|| {
        ProviderError::DependencyMissing(
            "Kitty transfer helper is unavailable or lacks the `transfer` subcommand".to_owned(),
        )
    })?;
    let mut command = Command::new(utility);
    command.args(["transfer", "--direction=upload", "--confirm-paths"]);
    command.arg("--");
    command.args(local_sources);
    command.arg(path_with_directory_suffix(remote_destination));
    let status = run_interactive_guarded(&mut command, "Kitty file upload", guard)?;
    if status.success() {
        Ok(())
    } else if interrupted_status(status) {
        Err(ProviderError::Cancelled)
    } else {
        Err(ProviderError::Protocol(
            "Kitty file upload failed or was rejected by the terminal".to_owned(),
        ))
    }
}

pub(crate) fn request_clipboard_image(destination: &Path) -> Result<(), ProviderError> {
    let utility = clipboard_utility().ok_or_else(|| {
        ProviderError::DependencyMissing(
            "Kitty clipboard helper is unavailable or lacks the `clipboard` subcommand".to_owned(),
        )
    })?;
    let mut command = Command::new(utility);
    command
        .args(["clipboard", "--get-clipboard", "--"])
        .arg(destination);
    let status = run_interactive(&mut command, "Kitty clipboard read")?;
    if status.success() {
        Ok(())
    } else if interrupted_status(status) {
        Err(ProviderError::Cancelled)
    } else {
        Err(ProviderError::PermissionDenied(
            "Kitty clipboard access was denied or did not contain supported image data".to_owned(),
        ))
    }
}

fn path_with_directory_suffix(path: &Path) -> String {
    let mut value = path.to_string_lossy().into_owned();
    if !value.ends_with(std::path::MAIN_SEPARATOR) {
        value.push(std::path::MAIN_SEPARATOR);
    }
    value
}
