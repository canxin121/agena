//! iTerm2 shell-integration helpers for user-selected local file uploads.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const MAX_UPLOADED_FILES: usize = 32;

/// Locate the `it2ul` utility installed by iTerm2 shell integration.
///
/// The official installer stores utilities below `~/.iterm2` and generally
/// exposes them through shell aliases, which a process launched by Agena does
/// not inherit. Prefer that stable on-disk path and then fall back to PATH for
/// installations that provide a real executable.
pub(crate) fn upload_utility() -> Option<PathBuf> {
    utility_path("it2ul")
}

pub(crate) fn download_utility() -> Option<PathBuf> {
    utility_path("it2dl")
}

fn utility_path(name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".iterm2").join(name));
    }
    if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|directory| directory.join(name)));
    }
    candidates.into_iter().find(|path| path.is_file())
}

pub(crate) fn request_download(path: &Path) -> Result<(), String> {
    let utility = download_utility().ok_or_else(|| {
        "iTerm2 download utility `it2dl` is unavailable; install iTerm2 Shell Integration and Utilities on this remote account"
            .to_string()
    })?;
    let status = Command::new(utility)
        .arg(path)
        .status()
        .map_err(|error| format!("could not start iTerm2 file download: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("iTerm2 file download failed".to_string())
    }
}

/// Ask iTerm2 to show its local file picker and unpack the returned upload
/// into `destination`. `it2ul` owns the terminal request/response protocol;
/// it must run while Agena has temporarily released raw terminal input.
pub(crate) fn request_upload(destination: &Path) -> Result<(), String> {
    let utility = upload_utility().ok_or_else(|| {
        "iTerm2 upload utility `it2ul` is unavailable; install iTerm2 Shell Integration and Utilities on this remote account"
            .to_string()
    })?;
    let status = Command::new(utility)
        .arg(destination)
        .status()
        .map_err(|error| format!("could not start iTerm2 file upload: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("iTerm2 file selection was cancelled or the upload failed".to_string())
    }
}

/// Return only regular files produced by `it2ul`, rejecting symlinks and other
/// special files before they can be staged as conversation attachments.
pub(crate) fn uploaded_regular_files(destination: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    collect_regular_files(destination, &mut files)?;
    if files.is_empty() {
        return Err("no files were selected in iTerm2".to_string());
    }
    files.sort();
    Ok(files)
}

fn collect_regular_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("could not inspect iTerm2 upload directory: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("could not inspect iTerm2 upload entry: {error}"))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "could not inspect uploaded file {}: {error}",
                path.display()
            )
        })?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(format!(
                "iTerm2 upload contained a symbolic link, which is not allowed: {}",
                path.display()
            ));
        }
        if file_type.is_dir() {
            collect_regular_files(path.as_path(), files)?;
        } else if file_type.is_file() {
            if files.len() >= MAX_UPLOADED_FILES {
                return Err(format!(
                    "iTerm2 upload contains more than {MAX_UPLOADED_FILES} files"
                ));
            }
            files.push(path);
        } else {
            return Err(format!(
                "iTerm2 upload contained an unsupported special file: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::uploaded_regular_files;

    #[test]
    fn uploaded_regular_files_recurses_and_sorts() {
        let directory = tempfile::tempdir().expect("temporary upload directory");
        std::fs::write(directory.path().join("b.txt"), "b").expect("write first file");
        std::fs::create_dir(directory.path().join("nested")).expect("create nested directory");
        std::fs::write(directory.path().join("nested/a.txt"), "a").expect("write nested file");

        let files = uploaded_regular_files(directory.path()).expect("collect uploaded files");
        let relative = files
            .iter()
            .map(|path| {
                path.strip_prefix(directory.path())
                    .expect("relative upload path")
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(relative, ["b.txt", "nested/a.txt"]);
    }
}
