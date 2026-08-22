use std::path::Path;

use git2::{ErrorCode, Repository};

#[derive(Debug, Clone)]
/// Error opening a git repository with libgit2.
pub enum Git2OpenError {
    NotARepository,
    Other(String),
}

impl Git2OpenError {
    pub fn code(&self) -> &'static str {
        match self {
            Git2OpenError::NotARepository => "not_git_repo",
            Git2OpenError::Other(_) => "git2_error",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Git2OpenError::NotARepository => "Not a git repository".to_string(),
            Git2OpenError::Other(e) => e.clone(),
        }
    }
}

fn map_git2_error(e: git2::Error) -> Git2OpenError {
    if e.code() == ErrorCode::NotFound {
        return Git2OpenError::NotARepository;
    }
    Git2OpenError::Other(git2_error_diagnostic(
        "failed to discover Git repository",
        &e,
    ))
}

pub(crate) fn git2_error_diagnostic(context: &str, error: &git2::Error) -> String {
    format!(
        "{context}: {} (class={:?}, code={:?})",
        error.message(),
        error.class(),
        error.code()
    )
}

pub fn open_repo_discover(dir: &Path) -> Result<Repository, Git2OpenError> {
    Repository::discover(dir).map_err(map_git2_error)
}
