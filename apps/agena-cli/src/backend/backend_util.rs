use anyhow::{Context, anyhow};

pub(super) fn parse_credential_issuer(value: &str) -> Result<CredentialIssuer> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai_chatgpt" => Ok(CredentialIssuer::OpenaiChatgpt),
        "github_copilot" => Ok(CredentialIssuer::GithubCopilot),
        "gitlab" => Ok(CredentialIssuer::Gitlab),
        "google_adc" => Ok(CredentialIssuer::GoogleAdc),
        "sap_ai_core" => Ok(CredentialIssuer::SapAiCore),
        _ => Err(anyhow!(
            "unsupported credential issuer `{}`; expected openai_chatgpt, github_copilot, gitlab, google_adc, or sap_ai_core",
            value.trim()
        )),
    }
}

pub(super) fn summarize_named_mode(
    display_name: Option<&str>,
    description: Option<&str>,
) -> String {
    match (
        display_name
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        description.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Some(display_name), Some(description)) => format!("{display_name} · {description}"),
        (Some(display_name), None) => display_name.to_owned(),
        (None, Some(description)) => description.to_owned(),
        (None, None) => "configured mode".to_owned(),
    }
}

pub(super) fn parse_aws_profile_names(text: &str) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if !line.starts_with('[') || !line.ends_with(']') {
            continue;
        }
        let section = line.trim_start_matches('[').trim_end_matches(']').trim();
        if section.eq_ignore_ascii_case("default") {
            names.insert("default".to_owned());
            continue;
        }
        if let Some(profile) = section.strip_prefix("profile ") {
            let profile = profile.trim();
            if !profile.is_empty() {
                names.insert(profile.to_owned());
            }
            continue;
        }
        if !section.contains(' ') && !section.contains('.') {
            names.insert(section.to_owned());
        }
    }
    names.into_iter().collect()
}

pub(super) fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(super) fn git_success<const N: usize>(workspace_root: &Path, args: [&str; N]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(super) fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

pub(super) fn trimmed_owned(value: &str) -> Option<String> {
    non_empty(Some(value)).map(ToOwned::to_owned)
}

pub(super) fn summarize_git_status(status: &str) -> (u64, u64, u64, u64) {
    let mut staged = 0_u64;
    let mut unstaged = 0_u64;
    let mut untracked = 0_u64;
    let mut changed = 0_u64;

    for line in status.lines().filter(|line| !line.is_empty()) {
        changed += 1;
        let bytes = line.as_bytes();
        let x = bytes.first().copied().unwrap_or(b' ');
        let y = bytes.get(1).copied().unwrap_or(b' ');
        if x == b'?' && y == b'?' {
            untracked += 1;
            continue;
        }
        if x != b' ' {
            staged += 1;
        }
        if y != b' ' {
            unstaged += 1;
        }
    }

    (staged, unstaged, untracked, changed)
}

pub(super) fn git_command_output<const N: usize>(
    workspace_root: &Path,
    args: [&str; N],
) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .context("failed to execute git command")?;
    if !output.status.success() {
        return Err(anyhow!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(super) fn detect_dimensions(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    match imagesize::blob_size(bytes) {
        Ok(size) => (
            u32::try_from(size.width).ok(),
            u32::try_from(size.height).ok(),
        ),
        Err(_) => (None, None),
    }
}

pub(super) fn detect_mime(path: &Path, bytes: &[u8]) -> String {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png".to_string();
    }
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff {
        return "image/jpeg".to_string();
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return "image/gif".to_string();
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp".to_string();
    }
    if bytes.starts_with(b"BM") {
        return "image/bmp".to_string();
    }
    if bytes.starts_with(b"%PDF-") {
        return "application/pdf".to_string();
    }
    if std::str::from_utf8(bytes).is_ok() {
        return MimeGuess::from_path(path)
            .first_raw()
            .filter(|mime| {
                mime.starts_with("text/")
                    || matches!(
                        *mime,
                        "application/json"
                            | "application/xml"
                            | "application/yaml"
                            | "application/x-yaml"
                            | "application/javascript"
                    )
            })
            .map(str::to_owned)
            .unwrap_or_else(|| "text/plain".to_string());
    }

    MimeGuess::from_path(path)
        .first_raw()
        .map(str::to_owned)
        .unwrap_or_else(|| "application/octet-stream".to_string())
}
use crate::backend::Result;
use crate::backend::{Command, CredentialIssuer, MimeGuess, Path};
