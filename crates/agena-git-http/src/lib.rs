//! Git HTTP handlers and Git operation helpers used by Agena.

use std::sync::Arc;
use std::{future::Future, path::PathBuf, pin::Pin};

mod git;
mod git2_utils;
mod path_utils;

/// Settings access required by stateful Git handlers. The Git package is
/// independent from Studio's concrete `AppState` and persistence layer.
pub trait GitHttpState: Send + Sync {
    fn git_allow_force_push(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
    fn git_allow_no_verify_commit(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
    fn git_enforce_branch_protection(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
    fn git_strict_patch_validation(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
    fn git_branch_protection_prompt(
        &self,
        branch: String,
    ) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>>;
}

impl<T: GitHttpState + ?Sized> GitHttpState for Arc<T> {
    fn git_allow_force_push(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        (**self).git_allow_force_push()
    }

    fn git_allow_no_verify_commit(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        (**self).git_allow_no_verify_commit()
    }

    fn git_enforce_branch_protection(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        (**self).git_enforce_branch_protection()
    }

    fn git_strict_patch_validation(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        (**self).git_strict_patch_validation()
    }

    fn git_branch_protection_prompt(
        &self,
        branch: String,
    ) -> Pin<Box<dyn Future<Output = Option<String>> + Send + '_>> {
        (**self).git_branch_protection_prompt(branch)
    }
}

pub fn normalize_directory_path(input: &str) -> String {
    path_utils::normalize_directory_path(input)
}

pub fn home_dir_path() -> Option<PathBuf> {
    path_utils::home_dir_path()
}

pub use git::*;
pub use git2_utils::{Git2OpenError, open_repo_discover};
