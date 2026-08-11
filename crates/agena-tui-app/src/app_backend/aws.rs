//! AWS profile name presentation for provider/credential choices.

use std::{env, fs, path::PathBuf};

/// List AWS profile names from `~/.aws/credentials` and `~/.aws/config`
/// (honoring `AWS_SHARED_CREDENTIALS_FILE` / `AWS_CONFIG_FILE`).
pub(crate) fn list_aws_profile_names() -> Vec<String> {
    let credentials_path = env::var("AWS_SHARED_CREDENTIALS_FILE")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            env::var("HOME")
                .ok()
                .map(|home| PathBuf::from(home).join(".aws/credentials"))
        });
    let config_path = env::var("AWS_CONFIG_FILE")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            env::var("HOME")
                .ok()
                .map(|home| PathBuf::from(home).join(".aws/config"))
        });
    let mut profiles = std::collections::BTreeSet::new();
    for path in [credentials_path, config_path].into_iter().flatten() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        profiles.extend(parse_aws_profile_names(text.as_str()));
    }
    profiles.into_iter().collect()
}

fn parse_aws_profile_names(text: &str) -> Vec<String> {
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
