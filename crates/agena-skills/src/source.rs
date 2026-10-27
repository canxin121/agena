//! Shared installation inputs and source loading. Callers authorize every read
//! and every outbound request before this module performs the corresponding I/O.

use std::{
    future::Future,
    path::{Path, PathBuf},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{SkillError, SkillErrorKind, SkillImport, SkillImportFile, SkillScope, SkillSource};

pub const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SOURCE_FILES: usize = 10_000;
const MAX_BASE64_BYTES: usize = MAX_SOURCE_BYTES.div_ceil(3) * 4;
static SOURCE_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
static SOURCE_IO_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillInstallSource {
    Github {
        url: String,
        #[serde(default)]
        reference: Option<String>,
        #[serde(default)]
        subpath: Option<String>,
    },
    Folder {
        path: String,
    },
    Archive {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        filename: Option<String>,
        #[serde(default)]
        data_base64: Option<String>,
    },
    Files {
        files: Vec<SkillUploadFile>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillUploadFile {
    pub path: String,
    pub content_base64: String,
    #[serde(default)]
    pub executable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillInstallRequest {
    #[serde(default)]
    pub scope: SkillScope,
    pub source: SkillInstallSource,
    #[serde(default)]
    pub selected: Vec<String>,
    #[serde(default)]
    pub replace: bool,
    #[serde(default)]
    pub expected_source_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillCreateRequest {
    #[serde(default)]
    pub scope: SkillScope,
    pub document: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillUpdateRequest {
    pub document: String,
    pub expected_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillEnabledRequest {
    pub enabled: bool,
    pub expected_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillResourceWriteRequest {
    pub path: String,
    pub content: String,
    /// Omit when creating a new resource; existing resources require a revision.
    #[serde(default)]
    pub expected_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillValidationRequest {
    pub document: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SkillDeleteResult {
    pub id: String,
    pub removed: bool,
}

#[derive(Debug, Clone)]
pub enum SkillSourceAccess {
    Read(PathBuf),
    Network(String),
}

pub struct LoadedSkillImport {
    pub package: SkillImport,
    pub source: SkillSource,
}

impl LoadedSkillImport {
    pub fn verify_revision(&self, expected: Option<&str>) -> Result<(), SkillError> {
        if expected.is_some_and(|expected| expected != self.package.revision()) {
            return Err(SkillError::new(
                SkillErrorKind::Conflict,
                "The installation source changed after preview. Preview it again before installing.",
            ));
        }
        Ok(())
    }
}

/// A resolved GitHub source, including links to a repository subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubSource {
    pub repository: String,
    pub download_url: String,
    pub reference: String,
    pub subpath: Option<String>,
}

pub fn resolve_github_source(
    input: &str,
    reference: Option<&str>,
    subpath: Option<&str>,
) -> Result<GithubSource, SkillError> {
    let input = input.trim();
    let normalized = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://github.com/{input}")
    };
    let url = http_url(&normalized)?;
    if url.host_str() != Some("github.com") || url.port().is_some() {
        return Err(SkillError::invalid(
            "Use a GitHub repository URL, a /tree/<ref>/<path> URL, or owner/repository.",
        ));
    }
    let segments: Vec<_> = url
        .path()
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .map(|part| {
            urlencoding::decode(part)
                .map(|part| part.into_owned())
                .map_err(|_| SkillError::invalid("The GitHub URL contains invalid UTF-8 escapes."))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if segments.len() < 2
        || segments[..2].iter().any(|part| {
            !part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        })
    {
        return Err(SkillError::invalid(
            "A GitHub source must identify an owner and repository.",
        ));
    }
    let owner = segments[0].as_str();
    let repository = segments[1].strip_suffix(".git").unwrap_or(&segments[1]);
    if owner == "."
        || owner == ".."
        || repository.is_empty()
        || repository == "."
        || repository == ".."
    {
        return Err(SkillError::invalid("Invalid GitHub owner or repository."));
    }
    let mut selected_ref = reference
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);
    let mut selected_path = subpath
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);
    if segments.len() > 2 {
        if !matches!(segments[2].as_str(), "tree" | "blob") || segments.len() < 4 {
            return Err(SkillError::invalid(
                "Use a repository or tree URL. Release archive links can be installed as an archive URL.",
            ));
        }
        let tail = segments[3..].join("/");
        if let Some(explicit_ref) = selected_ref.as_ref() {
            if selected_path.is_none() {
                if let Some(path) = tail.strip_prefix(&format!("{explicit_ref}/")) {
                    selected_path = Some(path.to_owned());
                } else if segments.len() > 4 {
                    selected_path = Some(segments[4..].join("/"));
                }
            }
        } else {
            selected_ref = Some(segments[3].to_owned());
            if selected_path.is_none() && segments.len() > 4 {
                selected_path = Some(segments[4..].join("/"));
            }
        }
        if segments[2] == "blob" && segments.last().is_none_or(|segment| segment != "SKILL.md") {
            return Err(SkillError::invalid(
                "A GitHub blob URL must point to SKILL.md.",
            ));
        }
        if segments[2] == "blob"
            && subpath.is_none_or(|path| path.trim().is_empty())
            && let Some(path) = selected_path.as_mut()
        {
            if path == "SKILL.md" {
                selected_path = None;
            } else if let Some(parent) = path.strip_suffix("/SKILL.md") {
                *path = parent.to_owned();
            } else {
                return Err(SkillError::invalid(
                    "A GitHub blob URL must point to SKILL.md.",
                ));
            }
        }
    }
    let reference = selected_ref.unwrap_or_else(|| "HEAD".to_owned());
    if reference.trim().is_empty() || reference.chars().any(char::is_control) {
        return Err(SkillError::invalid(
            "The GitHub reference cannot be blank or contain control characters.",
        ));
    }
    if let Some(path) = &selected_path {
        validate_relative_source_path(path)?;
    }
    let repository_url = format!("https://github.com/{owner}/{repository}");
    let mut download = Url::parse("https://codeload.github.com/").expect("static URL");
    download
        .path_segments_mut()
        .expect("static HTTP URL")
        .extend([owner, repository, "tar.gz", &reference]);
    Ok(GithubSource {
        repository: repository_url,
        download_url: download.to_string(),
        reference,
        subpath: selected_path,
    })
}

/// Authorizers retain their own error type, including pending permission
/// requests; a denial must not be flattened into a generic import error.
pub async fn load_install_source<E, F, Fut, M>(
    source: SkillInstallSource,
    workspace: PathBuf,
    mut authorize: F,
    map_error: M,
) -> Result<LoadedSkillImport, E>
where
    F: FnMut(SkillSourceAccess) -> Fut,
    Fut: Future<Output = Result<(), E>>,
    M: Fn(SkillError) -> E,
{
    let _permit = SOURCE_WORKERS
        .acquire()
        .await
        .map_err(|error| map_error(SkillError::new(SkillErrorKind::Io, error.to_string())))?;
    let (package, mut provenance) = match source {
        SkillInstallSource::Github {
            url,
            reference,
            subpath,
        } => {
            let github = resolve_github_source(&url, reference.as_deref(), subpath.as_deref())
                .map_err(&map_error)?;
            let bytes = download(&github.download_url, &mut authorize, &map_error).await?;
            let selected_path = github.subpath.clone();
            let package = blocking(move || {
                let package = SkillImport::from_archive("github.tar.gz", &bytes)?;
                match selected_path {
                    Some(path) => package.with_subpath(&path),
                    None => Ok(package),
                }
            })
            .await
            .map_err(&map_error)?;
            (
                package,
                SkillSource {
                    kind: "github".into(),
                    location: github.repository,
                    reference: Some(github.reference),
                    subpath: github.subpath,
                    revision: None,
                },
            )
        }
        SkillInstallSource::Folder { path } => {
            let path = authorized_local_path(&workspace, &path, &mut authorize, &map_error).await?;
            let enumerate = path.clone();
            let paths = blocking(move || folder_entries(&enumerate))
                .await
                .map_err(&map_error)?;
            let mut files = Vec::with_capacity(paths.len());
            let mut total = 0usize;
            for (relative, path, executable) in paths {
                authorize(SkillSourceAccess::Read(path.clone())).await?;
                let remaining = MAX_SOURCE_BYTES - total;
                let content = blocking(move || read_bounded(&path, remaining))
                    .await
                    .map_err(&map_error)?;
                total += content.len();
                files.push(SkillImportFile {
                    path: relative,
                    content,
                    executable,
                });
            }
            let package = blocking(move || SkillImport::from_files(files))
                .await
                .map_err(&map_error)?;
            (
                package,
                SkillSource {
                    kind: "folder".into(),
                    location: path.to_string_lossy().into_owned(),
                    reference: None,
                    subpath: None,
                    revision: None,
                },
            )
        }
        SkillInstallSource::Archive {
            path,
            url,
            filename,
            data_base64,
        } => {
            let uploaded = data_base64.is_some();
            if usize::from(path.is_some())
                + usize::from(url.is_some())
                + usize::from(data_base64.is_some())
                != 1
            {
                return Err(map_error(SkillError::invalid(
                    "Provide exactly one archive path, URL, or uploaded data.",
                )));
            }
            let (bytes, archive_name, location) = if let Some(path) = path {
                let path =
                    authorized_local_path(&workspace, &path, &mut authorize, &map_error).await?;
                let location = path.to_string_lossy().into_owned();
                let archive_name = filename.unwrap_or_else(|| {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
                let bytes = blocking(move || read_bounded(&path, MAX_SOURCE_BYTES))
                    .await
                    .map_err(&map_error)?;
                (bytes, archive_name, location)
            } else if let Some(url) = url {
                let parsed = http_url(&url).map_err(&map_error)?;
                let archive_name = filename.unwrap_or_else(|| {
                    parsed
                        .path_segments()
                        .and_then(|mut parts| parts.next_back())
                        .unwrap_or("archive.zip")
                        .to_owned()
                });
                let bytes = download(parsed.as_str(), &mut authorize, &map_error).await?;
                let mut location = parsed;
                location.set_query(None);
                location.set_fragment(None);
                (bytes, archive_name, location.to_string())
            } else {
                let archive_name = filename.filter(|name| !name.trim().is_empty()).ok_or_else(|| map_error(SkillError::invalid("Uploaded archives need a filename such as skills.zip or skills.tar.gz.")))?;
                let encoded = data_base64.expect("checked exclusive source");
                let bytes = decode_upload(&encoded, MAX_SOURCE_BYTES).map_err(&map_error)?;
                (bytes, archive_name.clone(), archive_name)
            };
            let package = blocking(move || SkillImport::from_archive(&archive_name, &bytes))
                .await
                .map_err(&map_error)?;
            (
                package,
                SkillSource {
                    kind: if uploaded { "upload" } else { "archive" }.into(),
                    location,
                    reference: None,
                    subpath: None,
                    revision: None,
                },
            )
        }
        SkillInstallSource::Files { files } => {
            if files.is_empty() || files.len() > MAX_SOURCE_FILES {
                return Err(map_error(SkillError::invalid(
                    "A folder upload must contain between 1 and 10000 files.",
                )));
            }
            let package = blocking(move || {
                let mut total = 0usize;
                let mut decoded = Vec::with_capacity(files.len());
                for file in files {
                    let content = decode_upload(&file.content_base64, MAX_SOURCE_BYTES - total)?;
                    total += content.len();
                    decoded.push(SkillImportFile {
                        path: file.path,
                        content,
                        executable: file.executable,
                    });
                }
                SkillImport::from_files(decoded)
            })
            .await
            .map_err(&map_error)?;
            (
                package,
                SkillSource {
                    kind: "upload".into(),
                    location: "Uploaded folder".into(),
                    reference: None,
                    subpath: None,
                    revision: None,
                },
            )
        }
    };
    provenance.revision = Some(package.revision());
    Ok(LoadedSkillImport {
        package,
        source: provenance,
    })
}

fn http_url(value: &str) -> Result<Url, SkillError> {
    let parsed =
        Url::parse(value).map_err(|_| SkillError::invalid("The download URL is invalid."))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(SkillError::invalid(
            "Use an HTTP or HTTPS URL without embedded credentials.",
        ));
    }
    Ok(parsed)
}

fn validate_relative_source_path(path: &str) -> Result<(), SkillError> {
    if path.is_empty()
        || path.contains('\\')
        || path.starts_with('/')
        || path.split('/').any(|part| part == ".." || part.is_empty())
        || path.chars().any(char::is_control)
    {
        return Err(SkillError::invalid(
            "The repository subdirectory must be a relative path without '..'.",
        ));
    }
    Ok(())
}

fn local_path(workspace: &Path, input: &str) -> Result<PathBuf, SkillError> {
    if input.trim().is_empty() {
        return Err(SkillError::invalid("A source path is required."));
    }
    let path = if let Some(relative) = input.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or_else(|| {
                SkillError::invalid(
                    "The user home directory is unavailable; provide an absolute source path.",
                )
            })?;
        PathBuf::from(home).join(relative)
    } else {
        PathBuf::from(input)
    };
    let path = if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    };
    Ok(path)
}

async fn authorized_local_path<E, F, Fut, M>(
    workspace: &Path,
    input: &str,
    authorize: &mut F,
    map_error: &M,
) -> Result<PathBuf, E>
where
    F: FnMut(SkillSourceAccess) -> Fut,
    Fut: Future<Output = Result<(), E>>,
    M: Fn(SkillError) -> E,
{
    let requested = local_path(workspace, input).map_err(map_error)?;
    authorize(SkillSourceAccess::Read(requested.clone())).await?;
    let candidate = requested.clone();
    let canonical = blocking(move || std::fs::canonicalize(candidate).map_err(SkillError::from))
        .await
        .map_err(map_error)?;
    if canonical != requested {
        authorize(SkillSourceAccess::Read(canonical.clone())).await?;
    }
    Ok(canonical)
}

fn decode_upload(encoded: &str, remaining: usize) -> Result<Vec<u8>, SkillError> {
    if encoded.len() > MAX_BASE64_BYTES
        || encoded.len() > remaining.div_ceil(3).saturating_mul(4).saturating_add(4)
    {
        return Err(SkillError::new(
            SkillErrorKind::TooLarge,
            "Installation sources are limited to 64 MiB.",
        ));
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| SkillError::invalid("Uploaded content is not valid base64."))?;
    if bytes.len() > remaining {
        return Err(SkillError::new(
            SkillErrorKind::TooLarge,
            "Installation sources are limited to 64 MiB.",
        ));
    }
    Ok(bytes)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, SkillError> {
    use std::io::Read as _;
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(SkillError::invalid(
            "Install sources must contain regular files, not symbolic links or devices.",
        ));
    }
    if metadata.len() > limit as u64 {
        return Err(SkillError::new(
            SkillErrorKind::TooLarge,
            "Installation sources are limited to 64 MiB.",
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(SkillError::new(
            SkillErrorKind::TooLarge,
            "Installation sources are limited to 64 MiB.",
        ));
    }
    Ok(bytes)
}

fn folder_entries(root: &Path) -> Result<Vec<(String, PathBuf, bool)>, SkillError> {
    if !root.is_dir() {
        return Err(SkillError::invalid("The folder source is not a directory."));
    }
    let mut builder = ignore::WalkBuilder::new(root);
    builder
        .hidden(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .parents(false)
        .follow_links(false)
        .max_depth(Some(32))
        .filter_entry(|entry| {
            entry.depth() == 0
                || !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | ".hg" | ".svn" | ".DS_Store")
                )
        });
    let mut files = Vec::new();
    for (index, entry) in builder.build().enumerate() {
        if index > MAX_SOURCE_FILES * 2 {
            return Err(SkillError::new(
                SkillErrorKind::TooLarge,
                "The source folder contains too many entries.",
            ));
        }
        let entry =
            entry.map_err(|error| SkillError::new(SkillErrorKind::Io, error.to_string()))?;
        if entry.depth() >= 32 {
            return Err(SkillError::new(
                SkillErrorKind::TooLarge,
                "The source folder exceeds 32 directory levels.",
            ));
        }
        let Some(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            return Err(SkillError::invalid(format!(
                "The source contains a symbolic link: {}",
                entry.path().display()
            )));
        }
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err(SkillError::invalid(
                "The source contains a non-regular file.",
            ));
        }
        if entry.file_name() == ".agena-skill.json" {
            continue;
        }
        let path = entry.path().to_path_buf();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| SkillError::invalid("The source path escaped its folder."))?
            .to_string_lossy()
            .replace('\\', "/");
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::metadata(&path)?.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        files.push((relative, path, executable));
        if files.len() > MAX_SOURCE_FILES {
            return Err(SkillError::new(
                SkillErrorKind::TooLarge,
                "A source may contain at most 10000 files.",
            ));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, SkillError> + Send + 'static,
) -> Result<T, SkillError> {
    let permit = SOURCE_IO_WORKERS
        .acquire()
        .await
        .map_err(|error| SkillError::new(SkillErrorKind::Io, error.to_string()))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation()
    })
    .await
    .map_err(|error| {
        SkillError::new(
            SkillErrorKind::Io,
            format!("Skill source worker failed: {error}"),
        )
    })?
}

async fn download<E, F, Fut, M>(
    initial: &str,
    authorize: &mut F,
    map_error: &M,
) -> Result<Vec<u8>, E>
where
    F: FnMut(SkillSourceAccess) -> Fut,
    Fut: Future<Output = Result<(), E>>,
    M: Fn(SkillError) -> E,
{
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(20))
        .user_agent("Agena-Skills/0.1")
        .build()
        .map_err(|error| map_error(SkillError::new(SkillErrorKind::Io, error.to_string())))?;
    let mut url = http_url(initial).map_err(map_error)?;
    for hop in 0..=5 {
        authorize(SkillSourceAccess::Network(url.to_string())).await?;
        let response = client.get(url.clone()).send().await.map_err(|error| {
            map_error(SkillError::new(
                SkillErrorKind::Io,
                format!("Could not download the skill archive: {error}"),
            ))
        })?;
        if response.status().is_redirection() {
            if hop == 5 {
                return Err(map_error(SkillError::invalid(
                    "The archive URL redirected too many times.",
                )));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    map_error(SkillError::invalid(
                        "The archive redirect has no valid Location header.",
                    ))
                })?;
            let next = url.join(location).map_err(|_| {
                map_error(SkillError::invalid("The archive redirect URL is invalid."))
            })?;
            url = http_url(next.as_str()).map_err(map_error)?;
            continue;
        }
        if !response.status().is_success() {
            return Err(map_error(SkillError::invalid(format!(
                "The archive server returned HTTP {}. Check the repository, reference, and URL.",
                response.status()
            ))));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_SOURCE_BYTES as u64)
        {
            return Err(map_error(SkillError::new(
                SkillErrorKind::TooLarge,
                "The archive download exceeds 64 MiB.",
            )));
        }
        let mut chunks = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|error| {
                map_error(SkillError::new(SkillErrorKind::Io, error.to_string()))
            })?;
            if bytes.len().saturating_add(chunk.len()) > MAX_SOURCE_BYTES {
                return Err(map_error(SkillError::new(
                    SkillErrorKind::TooLarge,
                    "The archive download exceeds 64 MiB.",
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(bytes);
    }
    unreachable!("bounded redirect loop returns on final hop")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_repository_tree_and_blob_sources_preserve_selection() {
        let repository = resolve_github_source("example/skills.git", None, None).unwrap();
        assert_eq!(
            repository.download_url,
            "https://codeload.github.com/example/skills/tar.gz/HEAD"
        );
        let tree = resolve_github_source(
            "https://github.com/example/skills/tree/main/skills/review",
            None,
            None,
        )
        .unwrap();
        assert_eq!(tree.reference, "main");
        assert_eq!(tree.subpath.as_deref(), Some("skills/review"));
        let branch = resolve_github_source(
            "https://github.com/example/skills/tree/feature/docs/skills/review",
            Some("feature/docs"),
            None,
        )
        .unwrap();
        assert_eq!(branch.subpath.as_deref(), Some("skills/review"));
        assert!(branch.download_url.ends_with("feature%2Fdocs"));
        let blob = resolve_github_source(
            "https://github.com/example/skills/blob/main/skills/review/SKILL.md",
            None,
            None,
        )
        .unwrap();
        assert_eq!(blob.subpath.as_deref(), Some("skills/review"));
        assert!(
            resolve_github_source("https://github.com.evil.test/example/skills", None, None)
                .is_err()
        );
        assert!(
            resolve_github_source(
                "https://github.com/example/skills",
                None,
                Some("../secrets")
            )
            .is_err()
        );
    }

    #[test]
    fn uploads_reject_invalid_or_oversized_payloads() {
        assert_eq!(decode_upload("YWJj", 3).unwrap(), b"abc");
        assert!(decode_upload("YWJj", 2).is_err());
        assert!(decode_upload("not base64!", 32).is_err());
    }

    #[test]
    fn github_ref_overrides_and_escaped_paths_keep_the_selected_directory() {
        let source = resolve_github_source(
            "https://github.com/example/skills/tree/main/docs%20and%20examples/review?tab=readme-ov-file",
            Some("release/v2"), None,
        ).unwrap();
        assert_eq!(source.subpath.as_deref(), Some("docs and examples/review"));
        assert!(source.download_url.ends_with("release%2Fv2"));
        let source = resolve_github_source(
            "https://github.com/example/skills/tree/feature%2Fdocs/skills/review",
            None,
            None,
        )
        .unwrap();
        assert_eq!(source.reference, "feature/docs");
        assert_eq!(source.subpath.as_deref(), Some("skills/review"));
        let source = resolve_github_source(
            "https://github.com/example/skills/blob/main/skills/review/SKILL.md",
            Some("v2"),
            Some("other/review"),
        )
        .unwrap();
        assert_eq!(source.subpath.as_deref(), Some("other/review"));
    }

    #[tokio::test]
    async fn uploaded_archives_are_not_recorded_as_server_paths() {
        let document =
            b"---\nname: sample\ndescription: Sample instructions.\n---\nUse this skill.\n";
        let mut archive = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(document.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "sample/SKILL.md", &document[..])
            .unwrap();
        let bytes = archive.into_inner().unwrap();
        let result = load_install_source(
            SkillInstallSource::Archive {
                path: None,
                url: None,
                filename: Some("skills.tar".into()),
                data_base64: Some(STANDARD.encode(bytes)),
            },
            PathBuf::from("."),
            |_| async { Ok::<(), SkillError>(()) },
            |error| error,
        )
        .await
        .unwrap();
        assert_eq!(result.source.kind, "upload");
        assert_eq!(result.source.location, "skills.tar");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn source_links_authorize_the_resolved_target_before_reading() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let requested = directory.path().join("linked-source");
        std::os::unix::fs::symlink(&target, &requested).unwrap();
        let canonical = target.canonicalize().unwrap();
        let checked = std::sync::Mutex::new(Vec::new());
        let result = load_install_source(
            SkillInstallSource::Folder {
                path: requested.to_string_lossy().into_owned(),
            },
            directory.path().to_owned(),
            |access| {
                let SkillSourceAccess::Read(path) = access else {
                    panic!("unexpected network request")
                };
                let denied = path == canonical;
                checked.lock().unwrap().push(path);
                async move {
                    if denied {
                        Err("resolved target denied".to_owned())
                    } else {
                        Ok(())
                    }
                }
            },
            |error| error.to_string(),
        )
        .await;
        assert_eq!(result.err().as_deref(), Some("resolved target denied"));
        assert_eq!(*checked.lock().unwrap(), [requested, canonical]);
    }

    #[tokio::test]
    async fn source_authorization_happens_before_content_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("broken.zip"), b"not an archive").unwrap();
        let result = load_install_source(
            SkillInstallSource::Archive {
                path: Some("broken.zip".into()),
                url: None,
                filename: None,
                data_base64: None,
            },
            dir.path().to_owned(),
            |_| async { Err::<(), _>("denied".to_owned()) },
            |error| error.to_string(),
        )
        .await;
        assert!(matches!(result, Err(error) if error == "denied"));
    }
}
