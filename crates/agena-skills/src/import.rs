use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{
    MAX_ARCHIVE_BYTES, MAX_DOCUMENT_BYTES, MAX_PACKAGE_BYTES, MAX_PACKAGE_DEPTH, MAX_PACKAGE_FILES,
    Result, STATE_FILE, SkillError, SkillInstallCandidate, validate_document,
};

#[derive(Debug, Clone)]
pub struct SkillImportFile {
    pub path: String,
    pub content: Vec<u8>,
    pub executable: bool,
}

/// A bounded, immutable source snapshot. Preview and install consume the same
/// normalized contents; nothing from the source is executed.
#[derive(Debug, Clone)]
pub struct SkillImport {
    pub(crate) files: BTreeMap<String, SkillImportFile>,
}

impl SkillImport {
    pub fn from_files(files: Vec<SkillImportFile>) -> Result<Self> {
        if files.len() > MAX_PACKAGE_FILES {
            return Err(SkillError::too_large("Skill source exceeds 10,000 files"));
        }
        let mut collected = BTreeMap::new();
        let mut total = 0_usize;
        for mut file in files {
            let path = safe_relative(&file.path)?;
            if ignored_path(&path) {
                continue;
            }
            total = total
                .checked_add(file.content.len())
                .ok_or_else(|| SkillError::too_large("Skill source size overflow"))?;
            if total > MAX_PACKAGE_BYTES {
                return Err(SkillError::too_large(
                    "Expanded skill source exceeds 128 MiB",
                ));
            }
            if path.rsplit('/').next() == Some("SKILL.md")
                && file.content.len() > MAX_DOCUMENT_BYTES
            {
                return Err(SkillError::too_large("SKILL.md exceeds 1 MiB"));
            }
            file.path.clone_from(&path);
            if collected.insert(path.clone(), file).is_some() {
                return Err(SkillError::invalid(format!(
                    "Duplicate source path '{path}'"
                )));
            }
        }
        for path in collected.keys() {
            let mut parent = Path::new(path).parent();
            while let Some(candidate) = parent {
                if let Some(candidate) = candidate.to_str()
                    && collected.contains_key(candidate)
                {
                    return Err(SkillError::invalid(format!(
                        "Source path '{candidate}' is both a file and a directory"
                    )));
                }
                parent = candidate.parent();
            }
        }
        Ok(Self { files: collected })
    }

    pub fn from_folder(path: &Path) -> Result<Self> {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(SkillError::invalid(
                "Skill folder must be a regular directory",
            ));
        }
        let root = path.canonicalize()?;
        let mut files = Vec::new();
        let mut total = 0_usize;
        let mut entries = 0_usize;
        collect_folder(&root, &root, &mut files, &mut total, &mut entries, 0)?;
        Self::from_files(files)
    }

    pub fn from_archive(filename: &str, bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_ARCHIVE_BYTES {
            return Err(SkillError::too_large("Skill archive exceeds 64 MiB"));
        }
        let lower = filename.to_ascii_lowercase();
        let mut files = Vec::new();
        if bytes.starts_with(b"PK\x03\x04")
            || bytes.starts_with(b"PK\x05\x06")
            || lower.ends_with(".zip")
        {
            let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
                .map_err(|error| SkillError::invalid(format!("Invalid ZIP archive: {error}")))?;
            if archive.len() > MAX_PACKAGE_FILES {
                return Err(SkillError::too_large(
                    "Skill archive exceeds 10,000 entries",
                ));
            }
            let mut total = 0_usize;
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index).map_err(|error| {
                    SkillError::invalid(format!("Cannot read ZIP entry: {error}"))
                })?;
                let mode = entry.unix_mode().unwrap_or(0);
                let kind = mode & 0o170000;
                if !matches!(kind, 0 | 0o100000 | 0o040000) {
                    return Err(SkillError::invalid(
                        "Skill archives cannot contain symbolic links or special files",
                    ));
                }
                let name = entry.name().trim_end_matches('/');
                if name.is_empty() {
                    continue;
                }
                let path = safe_relative(name)?;
                if entry.is_dir() || ignored_path(&path) {
                    continue;
                }
                let size = usize::try_from(entry.size())
                    .map_err(|_| SkillError::too_large("ZIP entry is too large"))?;
                if size > MAX_PACKAGE_BYTES.saturating_sub(total) {
                    return Err(SkillError::too_large(
                        "Expanded skill archive exceeds 128 MiB",
                    ));
                }
                let content = read_bounded(&mut entry, MAX_PACKAGE_BYTES - total)?;
                if content.len() != size {
                    return Err(SkillError::invalid("ZIP entry size changed while reading"));
                }
                total += content.len();
                files.push(SkillImportFile {
                    path,
                    content,
                    executable: mode & 0o111 != 0,
                });
            }
        } else {
            let gzip = bytes.starts_with(&[0x1f, 0x8b])
                || lower.ends_with(".tar.gz")
                || lower.ends_with(".tgz");
            if gzip {
                let decoder = flate2::read::GzDecoder::new(bytes);
                read_tar(decoder, &mut files)?;
            } else if lower.ends_with(".tar") {
                read_tar(Cursor::new(bytes), &mut files)?;
            } else {
                return Err(SkillError::invalid(
                    "Supported archives are .zip, .tar, .tar.gz, and .tgz",
                ));
            }
        }
        let mut imported = Self::from_files(files)?;
        imported.strip_archive_wrapper();
        Ok(imported)
    }

    pub fn with_subpath(self, subpath: &str) -> Result<Self> {
        if subpath.trim().is_empty() || subpath == "." {
            return Ok(self);
        }
        let subpath = safe_relative(subpath.trim_end_matches('/'))?;
        let prefix = format!("{subpath}/");
        let files = self
            .files
            .into_values()
            .filter_map(|mut file| {
                let path = file.path.strip_prefix(&prefix)?.to_owned();
                file.path = path;
                Some(file)
            })
            .collect::<Vec<_>>();
        if files.is_empty() {
            return Err(SkillError::invalid(format!(
                "Source contains no files under '{subpath}'"
            )));
        }
        Self::from_files(files)
    }

    pub fn files(&self) -> impl Iterator<Item = &SkillImportFile> {
        self.files.values()
    }

    pub fn revision(&self) -> String {
        let mut digest = Sha256::new();
        for (path, file) in &self.files {
            digest.update(path.as_bytes());
            digest.update([0, u8::from(file.executable)]);
            digest.update((file.content.len() as u64).to_le_bytes());
            digest.update(&file.content);
        }
        hex::encode(digest.finalize())
    }

    pub(crate) fn candidates(&self) -> Vec<SkillInstallCandidate> {
        self.files
            .iter()
            .filter_map(|(path, file)| {
                if path.rsplit('/').next() != Some("SKILL.md") {
                    return None;
                }
                let directory = path
                    .strip_suffix("SKILL.md")
                    .unwrap_or("")
                    .trim_end_matches('/');
                let candidate_path = if directory.is_empty() { "." } else { directory };
                let expected_name = if directory.is_empty() {
                    None
                } else {
                    directory.rsplit('/').next()
                };
                let document = match std::str::from_utf8(&file.content) {
                    Ok(document) => document,
                    Err(_) => {
                        return Some(SkillInstallCandidate {
                            path: candidate_path.into(),
                            name: expected_name.unwrap_or("invalid-skill").into(),
                            description: String::new(),
                            valid: false,
                            diagnostics: vec!["SKILL.md is not UTF-8 text".into()],
                            conflict: false,
                        });
                    }
                };
                let validation = validate_document(document, expected_name);
                let name = validation
                    .metadata
                    .as_ref()
                    .map(|metadata| metadata.name.clone())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| expected_name.unwrap_or("invalid-skill").into());
                Some(SkillInstallCandidate {
                    path: candidate_path.into(),
                    name,
                    description: validation
                        .metadata
                        .as_ref()
                        .map(|metadata| metadata.description.clone())
                        .unwrap_or_default(),
                    valid: validation.valid,
                    diagnostics: validation.errors,
                    conflict: false,
                })
            })
            .collect()
    }

    pub(crate) fn package_files(&self, candidate: &str) -> Vec<&SkillImportFile> {
        let prefix = if candidate == "." {
            String::new()
        } else {
            format!("{candidate}/")
        };
        self.files
            .values()
            .filter(|file| file.path.starts_with(&prefix))
            .collect()
    }

    fn strip_archive_wrapper(&mut self) {
        let Some(first) = self.files.keys().next() else {
            return;
        };
        let Some((wrapper, _)) = first.split_once('/') else {
            return;
        };
        let prefix = format!("{wrapper}/");
        if !self.files.keys().all(|path| path.starts_with(&prefix)) {
            return;
        }
        self.files = std::mem::take(&mut self.files)
            .into_values()
            .map(|mut file| {
                file.path = file.path[prefix.len()..].to_owned();
                (file.path.clone(), file)
            })
            .collect();
    }
}

pub(crate) fn safe_relative(path: &str) -> Result<String> {
    let path = path.trim_start_matches("./");
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', '\0', ':'])
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path.split('/').count() > MAX_PACKAGE_DEPTH
    {
        return Err(SkillError::invalid(format!(
            "Invalid package-relative path '{path}'"
        )));
    }
    Ok(path.to_owned())
}

pub(crate) fn ignored_path(path: &str) -> bool {
    path.split('/')
        .any(|part| matches!(part, ".git" | ".hg" | ".svn" | STATE_FILE))
}

pub(crate) fn read_bounded(reader: &mut impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut content = Vec::new();
    reader
        .take((limit as u64).saturating_add(1))
        .read_to_end(&mut content)?;
    if content.len() > limit {
        return Err(SkillError::too_large(
            "File exceeds the skill source size limit",
        ));
    }
    Ok(content)
}

fn collect_folder(
    root: &Path,
    directory: &Path,
    files: &mut Vec<SkillImportFile>,
    total: &mut usize,
    entries: &mut usize,
    depth: usize,
) -> Result<()> {
    if depth > MAX_PACKAGE_DEPTH {
        return Err(SkillError::too_large(
            "Skill source exceeds the directory depth limit",
        ));
    }
    let mut children = std::fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        *entries += 1;
        if *entries > MAX_PACKAGE_FILES {
            return Err(SkillError::too_large("Skill source exceeds 10,000 entries"));
        }
        let child_path = child.path();
        let relative = child_path
            .strip_prefix(root)
            .map_err(|_| SkillError::invalid("Folder entry escapes its source"))?;
        let relative = path_string(relative)?;
        if ignored_path(&relative) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&child_path)?;
        if metadata.file_type().is_symlink() {
            return Err(SkillError::invalid(format!(
                "Skill source contains a symbolic link: {relative}"
            )));
        }
        if metadata.is_dir() {
            collect_folder(root, &child_path, files, total, entries, depth + 1)?;
        } else if metadata.is_file() {
            if metadata.len() > (MAX_PACKAGE_BYTES - *total) as u64 {
                return Err(SkillError::too_large(
                    "Expanded skill source exceeds 128 MiB",
                ));
            }
            let mut file = std::fs::File::open(&child_path)?;
            let content = read_bounded(&mut file, MAX_PACKAGE_BYTES - *total)?;
            let after = file.metadata()?;
            if after.len() != metadata.len()
                || after.modified().ok() != metadata.modified().ok()
                || after.len() != content.len() as u64
            {
                return Err(SkillError::conflict(format!(
                    "Source file changed while reading: {relative}"
                )));
            }
            *total += content.len();
            #[cfg(unix)]
            let executable = {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let executable = false;
            files.push(SkillImportFile {
                path: relative,
                content,
                executable,
            });
        } else {
            return Err(SkillError::invalid(format!(
                "Skill source contains a special file: {relative}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn path_string(path: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| SkillError::invalid("Package paths must be UTF-8"))?,
            ),
            _ => {
                return Err(SkillError::invalid(
                    "Package path must be relative without traversal",
                ));
            }
        }
    }
    safe_relative(&parts.join("/"))
}

fn read_tar(reader: impl Read, files: &mut Vec<SkillImportFile>) -> Result<()> {
    let mut archive =
        tar::Archive::new(reader.take((MAX_PACKAGE_BYTES + MAX_PACKAGE_FILES * 1024) as u64));
    let mut total = 0_usize;
    let entries = archive
        .entries()
        .map_err(|error| SkillError::invalid(format!("Invalid TAR archive: {error}")))?;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_PACKAGE_FILES {
            return Err(SkillError::too_large(
                "Skill archive exceeds 10,000 entries",
            ));
        }
        let mut entry = entry
            .map_err(|error| SkillError::invalid(format!("Cannot read TAR entry: {error}")))?;
        let kind = entry.header().entry_type();
        // Git archives carry the commit ID in a global PAX metadata entry.
        // It is archive bookkeeping, never a package file or extraction path.
        if kind.is_pax_global_extensions() {
            let limit = MAX_DOCUMENT_BYTES.min(MAX_PACKAGE_BYTES - total);
            if entry.size() > limit as u64 {
                return Err(SkillError::too_large(
                    "TAR global metadata exceeds its byte limit",
                ));
            }
            let metadata = read_bounded(&mut entry, limit)?;
            if metadata.len() as u64 != entry.size() {
                return Err(SkillError::invalid(
                    "TAR metadata size changed while reading",
                ));
            }
            total += metadata.len();
            continue;
        }
        if !kind.is_file() && !kind.is_dir() {
            return Err(SkillError::invalid(
                "Skill archives cannot contain links or special entries",
            ));
        }
        let raw_path = entry
            .path()
            .map_err(|error| SkillError::invalid(format!("Invalid TAR path: {error}")))?
            .into_owned();
        let path_text = raw_path
            .to_str()
            .ok_or_else(|| SkillError::invalid("Archive paths must be UTF-8"))?;
        let path_text = path_text.trim_end_matches('/');
        if path_text.is_empty() || path_text == "." || path_text == "./" {
            continue;
        }
        let path = safe_relative(path_text)?;
        if kind.is_dir() || ignored_path(&path) {
            continue;
        }
        let size = entry.size();
        if size > (MAX_PACKAGE_BYTES - total) as u64 {
            return Err(SkillError::too_large(
                "Expanded skill archive exceeds 128 MiB",
            ));
        }
        let executable = entry.header().mode().unwrap_or(0) & 0o111 != 0;
        let content = read_bounded(&mut entry, MAX_PACKAGE_BYTES - total)?;
        if content.len() as u64 != size {
            return Err(SkillError::invalid("TAR entry size changed while reading"));
        }
        total += content.len();
        files.push(SkillImportFile {
            path,
            content,
            executable,
        });
    }
    Ok(())
}

pub(crate) fn join_relative(root: &Path, path: &str) -> PathBuf {
    path.split('/')
        .fold(root.to_path_buf(), |parent, part| parent.join(part))
}
