use std::path::PathBuf;

pub fn normalize_pasted_path(pasted: &str) -> Option<PathBuf> {
    let pasted = pasted.trim();
    if pasted.is_empty() {
        return None;
    }

    if let Ok(url) = url::Url::parse(pasted)
        && url.scheme() == "file"
    {
        return url.to_file_path().ok();
    }

    let looks_like_windows_path = {
        let drive = pasted
            .chars()
            .next()
            .map(|char| char.is_ascii_alphabetic())
            .unwrap_or(false)
            && pasted.get(1..2) == Some(":")
            && pasted
                .get(2..3)
                .map(|component| component == "\\" || component == "/")
                .unwrap_or(false);
        let unc = pasted.starts_with("\\\\");
        drive || unc
    };
    if looks_like_windows_path {
        #[cfg(target_os = "linux")]
        {
            if is_probably_wsl()
                && let Some(converted) = convert_windows_path_to_wsl(pasted)
            {
                return Some(converted);
            }
        }
        return Some(PathBuf::from(pasted));
    }

    let parts: Vec<String> = shlex::Shlex::new(pasted).collect();
    if parts.len() == 1 {
        return parts.into_iter().next().map(PathBuf::from);
    }

    let trimmed_quotes = pasted
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            pasted
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        });
    trimmed_quotes.map(PathBuf::from)
}

#[cfg(target_os = "linux")]
pub(super) fn is_probably_wsl() -> bool {
    if let Ok(version) = std::fs::read_to_string("/proc/version") {
        let lower = version.to_ascii_lowercase();
        if lower.contains("microsoft") || lower.contains("wsl") {
            return true;
        }
    }

    std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some()
}

#[cfg(target_os = "linux")]
pub(super) fn convert_windows_path_to_wsl(path: &str) -> Option<PathBuf> {
    if path.starts_with("\\\\") {
        return None;
    }
    let drive = path.chars().next()?.to_ascii_lowercase();
    if !drive.is_ascii_lowercase() || path.get(1..2) != Some(":") {
        return None;
    }

    let mut out = PathBuf::from(format!("/mnt/{drive}"));
    for component in path
        .get(2..)?
        .trim_start_matches(['\\', '/'])
        .split(['\\', '/'])
        .filter(|component| !component.is_empty())
    {
        out.push(component);
    }
    Some(out)
}

#[cfg(test)]
mod media_input_path_tests {
    #[test]
    fn quoted_unicode_clipboard_paths_are_parsed_as_paths_not_commands() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("图片 file.png");
        std::fs::write(&file, b"fixture").unwrap();
        let text = format!("\"{}\"", file.display());
        let parsed = super::normalize_pasted_path(&text);
        assert_eq!(parsed, Some(file.clone()));
        assert_eq!(
            super::normalize_pasted_path(url::Url::from_file_path(&file).unwrap().as_str()),
            Some(file)
        );
        assert!(super::normalize_pasted_path("/tmp/a.png ; echo do-not-execute").is_none());
    }
}
