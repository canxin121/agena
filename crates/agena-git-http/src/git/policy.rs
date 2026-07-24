pub(crate) async fn git_allow_force_push<S: crate::GitHttpState>(state: &S) -> bool {
    state.git_allow_force_push().await
}

pub(crate) async fn git_allow_no_verify_commit<S: crate::GitHttpState>(state: &S) -> bool {
    state.git_allow_no_verify_commit().await
}

pub(crate) async fn git_enforce_branch_protection<S: crate::GitHttpState>(state: &S) -> bool {
    state.git_enforce_branch_protection().await
}

pub(crate) async fn git_strict_patch_validation<S: crate::GitHttpState>(state: &S) -> bool {
    state.git_strict_patch_validation().await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitBranchProtectionPrompt {
    Commit,
    CommitToNewBranch,
    Prompt,
}

impl GitBranchProtectionPrompt {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Commit => "alwaysCommit",
            Self::CommitToNewBranch => "alwaysCommitToNewBranch",
            Self::Prompt => "alwaysPrompt",
        }
    }
}

pub(crate) async fn git_branch_protection_for_branch<S: crate::GitHttpState>(
    state: &S,
    branch: &str,
) -> Option<GitBranchProtectionPrompt> {
    state
        .git_branch_protection_prompt(branch.to_string())
        .await
        .map(|value| match value.as_str() {
            "alwaysCommit" => GitBranchProtectionPrompt::Commit,
            "alwaysCommitToNewBranch" => GitBranchProtectionPrompt::CommitToNewBranch,
            _ => GitBranchProtectionPrompt::Prompt,
        })
}
