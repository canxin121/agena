//! Project-selected local files for native Git checkouts.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::{fs, io};

use agena_runtime_tools::{
    copy_regular_file_nofollow, fingerprint_regular_file_nofollow, read_regular_file_nofollow,
};
use git2::{
    Commit, ErrorCode, ObjectType, Repository, Status, StatusEntry, StatusOptions, StatusShow, Tree,
};
use ignore::{
    Match,
    gitignore::{Gitignore, GitignoreBuilder},
};

pub(super) const MANIFEST_NAME: &str = ".worktreeinclude";
const MAX_MANIFEST_BYTES: usize = 256 * 1024;

pub(super) fn is_regular_file(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Cannot inspect {}: {error}", path.display())),
    }
}

pub(super) fn select_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let manifest = root.join(MANIFEST_NAME);
    let bytes = read_regular_file_nofollow(&manifest, MAX_MANIFEST_BYTES)
        .map_err(|error| format!("Cannot read {}: {error}", manifest.display()))?;
    let source = std::str::from_utf8(&bytes)
        .map_err(|error| format!("Invalid UTF-8 in {}: {error}", manifest.display()))?;
    let mut builder = GitignoreBuilder::new(root);
    for line in source.trim_start_matches('\u{feff}').lines() {
        builder
            .add_line(None, line)
            .map_err(|error| format!("Invalid pattern in {}: {error}", manifest.display()))?;
    }
    let matcher = builder.build().map_err(|error| error.to_string())?;
    if matcher.num_ignores() == 0 {
        return Ok(Vec::new());
    }

    let repo = Repository::open(root).map_err(|error| error.to_string())?;
    let mut options = StatusOptions::new();
    options
        .show(StatusShow::Workdir)
        .include_untracked(false)
        .include_ignored(true)
        .recurse_ignored_dirs(true)
        .exclude_submodules(true);
    let statuses = repo
        .statuses(Some(&mut options))
        .map_err(|error| error.to_string())?;
    let mut files = Vec::new();
    for status in statuses
        .iter()
        .filter(|status| status.status() == Status::IGNORED)
    {
        let Some(path) = relative_status_path(&status) else {
            continue;
        };
        let Match::Ignore(pattern) = matcher.matched_path_or_any_parents(&path, false) else {
            continue;
        };
        if let Some(tail) = pattern.original().strip_prefix("**/")
            && !glob_reaches_ignored_parents(&repo, &matcher, &path, tail)?
        {
            continue;
        }
        if is_regular_file(&root.join(&path))? {
            files.push(path);
        }
    }
    Ok(files)
}

fn glob_reaches_ignored_parents(
    repo: &Repository,
    matcher: &Gitignore,
    path: &Path,
    tail: &str,
) -> Result<bool, String> {
    let name = tail.split('/').next().unwrap_or_default();
    for parent in path
        .ancestors()
        .skip(1)
        .take_while(|path| !path.as_os_str().is_empty())
    {
        if repo
            .status_should_ignore(parent)
            .map_err(|error| error.to_string())?
            && !matcher.matched(parent, true).is_ignore()
            && !parent.components().any(|part| part.as_os_str() == name)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn copy_files(
    repo: &Repository,
    start: &Commit<'_>,
    source: &Path,
    checkout: &Path,
    paths: &[PathBuf],
) -> Result<BTreeMap<PathBuf, String>, String> {
    let mut copied = BTreeMap::new();
    if paths.is_empty() {
        return Ok(copied);
    }
    let base = start.tree().map_err(|error| error.to_string())?;
    for path in paths {
        match repo.status_file(path) {
            Ok(status) if status == Status::IGNORED => {}
            Ok(_) => continue,
            Err(error) if error.code() == ErrorCode::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        }
        if !can_add_file(&base, path).map_err(|error| error.to_string())? {
            continue;
        }
        let fingerprint = copy_regular_file_nofollow(&source.join(path), &checkout.join(path))
            .map_err(|error| {
                format!(
                    "Cannot copy {MANIFEST_NAME} file {}: {error}",
                    path.display()
                )
            })?;
        copied.insert(path.clone(), fingerprint);
    }
    Ok(copied)
}

fn can_add_file(base: &Tree<'_>, path: &Path) -> Result<bool, git2::Error> {
    let ancestors: Vec<_> = path
        .ancestors()
        .take_while(|path| !path.as_os_str().is_empty())
        .collect();
    for ancestor in ancestors.into_iter().rev() {
        match base.get_path(ancestor) {
            Ok(entry) if ancestor == path || entry.kind() != Some(ObjectType::Tree) => {
                return Ok(false);
            }
            Ok(_) => {}
            Err(error) if error.code() == ErrorCode::NotFound => return Ok(true),
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

pub(super) fn is_unchanged(
    checkout: &Path,
    files: &BTreeMap<PathBuf, String>,
    status: &StatusEntry<'_>,
) -> bool {
    if !matches!(status.status(), Status::IGNORED | Status::WT_NEW) {
        return false;
    }
    let Some(path) = relative_status_path(status) else {
        return false;
    };
    files.get(&path).is_some_and(|expected| {
        fingerprint_regular_file_nofollow(&checkout.join(path))
            .is_ok_and(|current| &current == expected)
    })
}

fn relative_status_path(status: &StatusEntry<'_>) -> Option<PathBuf> {
    let path = Path::new(status.path().ok()?);
    path.components()
        .all(|part| matches!(part, Component::Normal(_)))
        .then(|| path.components().collect())
}
