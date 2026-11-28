//! Read a pinned GitHub repository subtree for a marketplace package.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use git2::{ObjectType, Oid};
use serde::Deserialize;
use url::Url;

use super::{
    HttpFetcher, MAX_ENTRIES, MAX_MANIFEST, MAX_PACKAGE_BYTES, MarketplaceError, invalid,
    safe_path, set_mode, valid_segment,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);
const DOWNLOAD_WORKERS: usize = 6;

pub(super) struct Snapshot {
    repository: String,
    pub(super) commit: Oid,
    tree: Oid,
}

#[derive(Deserialize)]
struct BranchHead {
    sha: String,
    commit: CommitTree,
}

#[derive(Deserialize)]
struct CommitTree {
    tree: ObjectId,
}

#[derive(Deserialize)]
struct PinnedCommit {
    sha: String,
    tree: ObjectId,
}

#[derive(Deserialize)]
struct ObjectId {
    sha: String,
}

#[derive(Deserialize)]
struct RepositoryTree {
    sha: String,
    truncated: bool,
    tree: Vec<TreeEntry>,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
    size: Option<u64>,
}

struct PackageFile {
    path: PathBuf,
    oid: Oid,
    size: u64,
    executable: bool,
}

impl Snapshot {
    pub(super) fn default_branch(
        fetcher: &dyn HttpFetcher,
        repository: &str,
    ) -> Result<Self, MarketplaceError> {
        let url = format!("https://api.github.com/repos/{repository}/commits?per_page=1");
        let heads: Vec<BranchHead> = serde_json::from_slice(&fetch(fetcher, &url, MAX_MANIFEST)?)?;
        let head = heads
            .first()
            .ok_or_else(|| invalid("GitHub repository has no default branch commit"))?;
        Ok(Self {
            repository: repository.to_owned(),
            commit: oid(&head.sha)?,
            tree: oid(&head.commit.tree.sha)?,
        })
    }

    pub(super) fn at_commit(
        fetcher: &dyn HttpFetcher,
        repository: &str,
        commit: Oid,
    ) -> Result<Self, MarketplaceError> {
        let url = format!("https://api.github.com/repos/{repository}/git/commits/{commit}");
        let metadata: PinnedCommit = serde_json::from_slice(&fetch(fetcher, &url, MAX_MANIFEST)?)?;
        if oid(&metadata.sha)? != commit {
            return Err(invalid("GitHub returned a different commit"));
        }
        Ok(Self {
            repository: repository.to_owned(),
            commit,
            tree: oid(&metadata.tree.sha)?,
        })
    }

    pub(super) fn manifest(
        &self,
        fetcher: &dyn HttpFetcher,
        paths: &[&str],
    ) -> Result<Vec<u8>, MarketplaceError> {
        let mut error = invalid("marketplace has no known catalog path");
        for path in paths {
            match fetch(
                fetcher,
                self.raw_url(Path::new(path)).as_str(),
                MAX_MANIFEST,
            ) {
                Ok(bytes) => return Ok(bytes),
                Err(failure) => error = failure,
            }
        }
        Err(error)
    }

    pub(super) fn stage_package(
        &self,
        fetcher: &dyn HttpFetcher,
        relative: &Path,
        dest: &Path,
    ) -> Result<PathBuf, MarketplaceError> {
        if !safe_path(relative) {
            return Err(invalid("invalid relative package path"));
        }
        let mut directory = self.tree;
        for segment in relative.iter().filter(|part| *part != ".") {
            let contents = self.read_tree(fetcher, directory, false)?;
            let segment = segment
                .to_str()
                .ok_or_else(|| invalid("invalid GitHub directory name"))?;
            let child = contents
                .into_iter()
                .find(|entry| {
                    entry.path == segment && entry.kind == "tree" && entry.mode == "040000"
                })
                .ok_or_else(|| {
                    invalid("package directory is missing from the selected GitHub commit")
                })?;
            directory = oid(&child.sha)?;
        }

        let entries = self.read_tree(fetcher, directory, true)?;
        if entries.len() > MAX_ENTRIES {
            return Err(invalid("package has too many files"));
        }
        let mut files = Vec::new();
        let mut bytes = 0;
        for entry in entries {
            let path = Path::new(&entry.path);
            if entry.path.contains('\\')
                || !safe_path(path)
                || path.iter().any(|part| part == "." || part == ".git")
            {
                return Err(invalid("unsafe path in GitHub package tree"));
            }
            if entry.kind == "tree" && entry.mode == "040000" {
                continue;
            }
            if entry.kind != "blob" || !matches!(entry.mode.as_str(), "100644" | "100755") {
                return Err(invalid(format!(
                    "package contains a link or special file at {}",
                    path.display()
                )));
            }
            let size = entry
                .size
                .filter(|size| *size <= MAX_PACKAGE_BYTES - bytes)
                .ok_or_else(|| {
                    invalid("GitHub package file size is missing or exceeds the package limit")
                })?;
            bytes += size;
            files.push(PackageFile {
                path: path.to_owned(),
                oid: oid(&entry.sha)?,
                size,
                executable: entry.mode == "100755",
            });
        }

        fs::create_dir(dest)?;
        let chunk_size = files.len().div_ceil(DOWNLOAD_WORKERS).max(1);
        let stopped = AtomicBool::new(false);
        thread::scope(|scope| {
            let stopped = &stopped;
            let workers = files
                .chunks(chunk_size)
                .map(|chunk| {
                    scope.spawn(move || {
                        let result = chunk
                            .iter()
                            .take_while(|_| !stopped.load(Ordering::Relaxed))
                            .try_for_each(|file| self.stage_file(fetcher, relative, dest, file));
                        if result.is_err() {
                            stopped.store(true, Ordering::Relaxed);
                        }
                        result
                    })
                })
                .collect::<Vec<_>>();
            for worker in workers {
                worker
                    .join()
                    .map_err(|_| invalid("GitHub package download worker failed"))??;
            }
            Ok::<_, MarketplaceError>(())
        })?;
        Ok(dest.to_owned())
    }

    fn read_tree(
        &self,
        fetcher: &dyn HttpFetcher,
        tree: Oid,
        recursive: bool,
    ) -> Result<Vec<TreeEntry>, MarketplaceError> {
        let suffix = if recursive { "?recursive=1" } else { "" };
        let repo = &self.repository;
        let url = format!("https://api.github.com/repos/{repo}/git/trees/{tree}{suffix}");
        let data: RepositoryTree = serde_json::from_slice(&fetch(fetcher, &url, MAX_MANIFEST)?)?;
        if oid(&data.sha)? != tree || data.truncated {
            return Err(invalid(
                "GitHub returned an incomplete or different package tree",
            ));
        }
        Ok(data.tree)
    }

    fn stage_file(
        &self,
        fetcher: &dyn HttpFetcher,
        directory: &Path,
        dest: &Path,
        file: &PackageFile,
    ) -> Result<(), MarketplaceError> {
        let url = self.raw_url(&directory.join(&file.path));
        let bytes = fetch(fetcher, url.as_str(), file.size)?;
        if bytes.len() as u64 != file.size
            || Oid::hash_object(ObjectType::Blob, &bytes)
                .map_err(|error| invalid(error.to_string()))?
                != file.oid
        {
            return Err(invalid(format!(
                "GitHub blob does not match the selected commit at {}",
                file.path.display()
            )));
        }
        let path = dest.join(&file.path);
        let parent = path
            .parent()
            .ok_or_else(|| invalid("invalid package file path"))?;
        fs::create_dir_all(parent)?;
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?
            .write_all(&bytes)?;
        set_mode(&path, if file.executable { 0o111 } else { 0 })?;
        Ok(())
    }

    fn raw_url(&self, path: &Path) -> Url {
        let mut url = Url::parse(&format!(
            "https://raw.githubusercontent.com/{}/{}",
            self.repository, self.commit
        ))
        .expect("validated GitHub repository");
        url.path_segments_mut()
            .expect("hierarchical GitHub URL")
            .extend(
                path.iter()
                    .filter(|part| *part != ".")
                    .map(|part| part.to_string_lossy()),
            );
        url
    }
}

pub(super) fn https_repository(location: &str) -> Option<String> {
    let url = Url::parse(location).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let (owner, repo) = url
        .path()
        .strip_prefix('/')?
        .trim_end_matches('/')
        .split_once('/')?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    (valid_segment(owner) && valid_segment(repo)).then(|| format!("{owner}/{repo}"))
}

pub(super) fn oid(value: &str) -> Result<Oid, MarketplaceError> {
    if value.len() != 40 {
        return Err(invalid("GitHub object ID must be a complete SHA-1"));
    }
    Oid::from_str(value).map_err(|_| invalid("invalid GitHub object ID"))
}

fn fetch(fetcher: &dyn HttpFetcher, url: &str, limit: u64) -> Result<Vec<u8>, MarketplaceError> {
    match fetcher.fetch_bounded(url, limit, REQUEST_TIMEOUT) {
        Err(MarketplaceError::Http(_)) => fetcher.fetch_bounded(url, limit, REQUEST_TIMEOUT),
        result => result,
    }
}

#[cfg(test)]
mod tests;
