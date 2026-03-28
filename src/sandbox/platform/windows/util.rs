use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::sandbox::{SandboxError, cap_fs};

pub(super) fn to_wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

pub(super) fn format_last_error(code: i32) -> String {
    std::io::Error::from_raw_os_error(code).to_string()
}

pub(super) fn normalize_null_device_env(env_map: &mut HashMap<String, String>) {
    let keys: Vec<String> = env_map.keys().cloned().collect();
    for key in keys {
        if let Some(value) = env_map.get(&key).cloned() {
            let lowered = value.trim().to_ascii_lowercase();
            if lowered == "/dev/null" || lowered == "\\\\dev\\null" {
                env_map.insert(key, "NUL".to_string());
            }
        }
    }
}

pub(super) fn ensure_non_interactive_pager(env_map: &mut HashMap<String, String>) {
    env_map
        .entry("GIT_PAGER".to_string())
        .or_insert_with(|| "more.com".to_string());
    env_map
        .entry("PAGER".to_string())
        .or_insert_with(|| "more.com".to_string());
    env_map
        .entry("LESS".to_string())
        .or_insert_with(String::new);
}

pub(super) fn ensure_safe_allow_path(
    path: &Path,
    reject_reparse_points: bool,
) -> Result<PathBuf, SandboxError> {
    if !cap_fs::path_exists(path) {
        return Err(SandboxError::InvalidRequest(format!(
            "allow path does not exist: {}",
            path.display()
        )));
    }

    if cap_fs::is_symlink(path)? {
        return Err(SandboxError::Denied(format!(
            "allow path cannot be a symlink: {}",
            path.display()
        )));
    }

    if reject_reparse_points && is_reparse_point(path)? {
        return Err(SandboxError::Denied(format!(
            "allow path cannot be a reparse point: {}",
            path.display()
        )));
    }

    cap_fs::canonicalize_path(path).map_err(SandboxError::Io)
}

pub(super) fn is_reparse_point(path: &Path) -> Result<bool, SandboxError> {
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    use windows_sys::Win32::Storage::FileSystem::GetFileAttributesW;

    let wide = to_wide(path.as_os_str());
    let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if attributes == u32::MAX {
        return Err(SandboxError::Windows(format!(
            "GetFileAttributesW failed for {}",
            path.display()
        )));
    }
    Ok((attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0)
}
