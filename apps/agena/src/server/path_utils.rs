use std::path::PathBuf;

fn decode_url_encoded_path_component(input: &str) -> String {
    if !input.as_bytes().contains(&b'%') {
        return input.to_string();
    }
    urlencoding::decode(input)
        .map(|v| v.into_owned())
        .unwrap_or_else(|_| input.to_string())
}

pub(crate) fn home_dir_env() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
        .or_else(|| {
            let drive = std::env::var("HOMEDRIVE").ok().unwrap_or_default();
            let path = std::env::var("HOMEPATH").ok().unwrap_or_default();
            let joined = format!("{}{}", drive.trim(), path.trim())
                .trim()
                .to_string();
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        })
}

pub(crate) fn home_dir_path() -> Option<PathBuf> {
    home_dir_env().map(PathBuf::from)
}

pub(crate) fn config_home_dir() -> PathBuf {
    if cfg!(windows)
        && let Ok(dir) = std::env::var("APPDATA")
    {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    home_dir_path()
        .map(|v| v.join(".config"))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(crate) fn normalize_directory_path(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "".to_string();
    }

    let trimmed = decode_url_encoded_path_component(trimmed);
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        return "".to_string();
    }

    if trimmed == "~" {
        return home_dir_env().unwrap_or_else(|| trimmed.to_string());
    }

    if let Some(rest) = trimmed.strip_prefix("~/")
        && let Some(home) = home_dir_env()
    {
        let rest = rest.replace('\\', "/");
        return PathBuf::from(home)
            .join(rest)
            .to_string_lossy()
            .replace('\\', "/");
    }

    if let Some(rest) = trimmed.strip_prefix("~\\")
        && let Some(home) = home_dir_env()
    {
        let rest = rest.replace('\\', "/");
        return PathBuf::from(home)
            .join(rest)
            .to_string_lossy()
            .replace('\\', "/");
    }

    trimmed.to_string()
}
