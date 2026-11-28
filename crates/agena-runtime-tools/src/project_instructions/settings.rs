use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

use agena_plugin_host::sdk::ImportedConfigChangeSource;
use cap_fs_ext::OpenOptionsSyncExt;
use cap_std::fs::{Dir, OpenOptions};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;
const PROJECT_SETTINGS: [(ImportedConfigChangeSource, &str); 2] = [
    (
        ImportedConfigChangeSource::ProjectSettings,
        ".claude/settings.json",
    ),
    (
        ImportedConfigChangeSource::LocalSettings,
        ".claude/settings.local.json",
    ),
];

pub fn claude_project_settings_paths(
    workspace: &Path,
) -> [(ImportedConfigChangeSource, PathBuf); 2] {
    PROJECT_SETTINGS.map(|(source, path)| (source, workspace.join(path)))
}

/// Immutable instruction filters from the two project settings files at publication time.
pub struct ClaudeInstructionSettings {
    root: PathBuf,
    excludes: GlobSet,
    digests: [Option<[u8; 32]>; PROJECT_SETTINGS.len()],
}

impl ClaudeInstructionSettings {
    pub fn load(workspace: &Path) -> Self {
        let root = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_owned());
        let mut patterns = GlobSetBuilder::new();
        let mut digests = [None; PROJECT_SETTINGS.len()];
        if let Ok(directory) = Dir::open_ambient_dir(&root, cap_std::ambient_authority()) {
            for ((_, path), digest) in PROJECT_SETTINGS.into_iter().zip(&mut digests) {
                let bytes = match read_settings(&directory, path) {
                    Ok(bytes) => bytes,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => {
                        tracing::warn!(settings = path, %error, "could not load Claude instruction settings");
                        continue;
                    }
                };
                *digest = Some(Sha256::digest(&bytes).into());
                let Ok(settings) = serde_json::from_slice::<Settings>(&bytes) else {
                    tracing::warn!(
                        settings = path,
                        "invalid Claude settings JSON or claudeMdExcludes"
                    );
                    continue;
                };
                for pattern in settings.excludes {
                    #[cfg(windows)]
                    let pattern = pattern.replace('\\', "/");
                    match GlobBuilder::new(&pattern)
                        .literal_separator(true)
                        .case_insensitive(cfg!(windows))
                        .build()
                    {
                        Ok(glob) => {
                            patterns.add(glob);
                        }
                        Err(_) => {
                            tracing::warn!(settings = path, "ignored invalid claudeMdExcludes glob")
                        }
                    }
                }
            }
        }
        let excludes = patterns.build().unwrap_or_else(|error| {
            tracing::warn!(%error, "could not compile Claude instruction exclusions");
            GlobSet::empty()
        });
        Self {
            root,
            excludes,
            digests,
        }
    }

    /// Compare the exact files used to build two snapshots, including changes
    /// made after the watcher sampled their modification times.
    pub fn changed_paths(&self, previous: &Self) -> Vec<PathBuf> {
        claude_project_settings_paths(&self.root)
            .into_iter()
            .zip(self.digests.iter().zip(&previous.digests))
            .filter_map(|((_, path), (current, previous))| (current != previous).then_some(path))
            .collect()
    }

    pub(super) fn applies_to(&self, workspace: &Path) -> bool {
        self.root == workspace && !self.excludes.is_empty()
    }

    pub(super) fn excludes(&self, path: &Path) -> bool {
        if self.excludes.is_match(path) {
            return true;
        }
        #[cfg(windows)]
        {
            let spelling = path.to_string_lossy();
            if let Some(unc) = spelling.strip_prefix(r"\\?\UNC\") {
                return self.excludes.is_match(format!("//{unc}"));
            }
            if let Some(path) = spelling.strip_prefix(r"\\?\") {
                return self.excludes.is_match(path);
            }
        }
        false
    }
}

#[derive(Deserialize)]
struct Settings {
    #[serde(default, rename = "claudeMdExcludes")]
    excludes: Vec<String>,
}

fn read_settings(workspace: &Dir, path: &str) -> io::Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true).nonblock(true);
    let file = workspace.open_with(path, &options)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expected a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "settings exceed 1 MiB",
        ));
    }
    Ok(bytes)
}
