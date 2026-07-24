use std::path::PathBuf;

fn decode_url_encoded_path_component(input: &str) -> String {
    if !input.as_bytes().contains(&b'%') {
        return input.to_string();
    }
    urlencoding::decode(input)
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| input.to_string())
}

fn home_dir_env() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            let drive = std::env::var("HOMEDRIVE").ok().unwrap_or_default();
            let path = std::env::var("HOMEPATH").ok().unwrap_or_default();
            let joined = format!("{}{}", drive.trim(), path.trim())
                .trim()
                .to_string();
            (!joined.is_empty()).then_some(joined)
        })
}

pub(crate) fn home_dir_path() -> Option<PathBuf> {
    home_dir_env().map(PathBuf::from)
}

pub(crate) fn normalize_directory_path(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let trimmed = decode_url_encoded_path_component(trimmed);
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed == "~" {
        return home_dir_env().unwrap_or_else(|| trimmed.to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("~/")
        && let Some(home) = home_dir_env()
    {
        return PathBuf::from(home)
            .join(rest.replace('\\', "/"))
            .to_string_lossy()
            .replace('\\', "/");
    }
    if let Some(rest) = trimmed.strip_prefix("~\\")
        && let Some(home) = home_dir_env()
    {
        return PathBuf::from(home)
            .join(rest.replace('\\', "/"))
            .to_string_lossy()
            .replace('\\', "/");
    }
    trimmed.to_string()
}
