use globset::{Glob, GlobMatcher};
use path_clean::PathClean;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessSelector {
    Read,
    Write,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Error)]
pub enum PermissionConfigError {
    #[error("invalid permission glob pattern '{pattern}': {source}")]
    InvalidGlob {
        pattern: String,
        source: globset::Error,
    },
}

#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    default_read: PermissionMode,
    default_write: PermissionMode,
    rules: Vec<PermissionRule>,
}

impl PermissionPolicy {
    pub fn new(default_read: PermissionMode, default_write: PermissionMode) -> Self {
        Self {
            default_read,
            default_write,
            rules: Vec::new(),
        }
    }

    pub fn allow_all() -> Self {
        Self::new(PermissionMode::Allow, PermissionMode::Allow)
    }

    pub fn read_all_write_workspace_only() -> Self {
        Self {
            default_read: PermissionMode::Allow,
            default_write: PermissionMode::Deny,
            rules: vec![PermissionRule {
                selector: AccessSelector::Write,
                mode: PermissionMode::Allow,
                matcher: RuleMatcher::WorkspaceOnly,
                description: "allow write inside workspace".to_string(),
            }],
        }
    }

    pub fn with_absolute_glob_rule(
        mut self,
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        self.rules
            .push(PermissionRule::absolute_glob(selector, mode, pattern)?);
        Ok(self)
    }

    pub fn with_workspace_glob_rule(
        mut self,
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        self.rules
            .push(PermissionRule::workspace_glob(selector, mode, pattern)?);
        Ok(self)
    }

    pub fn with_rule(mut self, rule: PermissionRule) -> Self {
        self.rules.push(rule);
        self
    }

    pub fn check_access(
        &self,
        access: AccessKind,
        workspace_root: &Path,
        target_path: &Path,
    ) -> PermissionDecision {
        let context = MatchContext::new(workspace_root, target_path);

        for rule in &self.rules {
            if !rule.matches_selector(access) {
                continue;
            }
            if rule.matcher.matches(&context) {
                return mode_decision(rule.mode, &rule.description);
            }
        }

        match access {
            AccessKind::Read => mode_decision(self.default_read, "matched default read permission"),
            AccessKind::Write => {
                mode_decision(self.default_write, "matched default write permission")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PermissionRule {
    selector: AccessSelector,
    mode: PermissionMode,
    matcher: RuleMatcher,
    description: String,
}

impl PermissionRule {
    pub fn workspace_only(selector: AccessSelector, mode: PermissionMode) -> Self {
        Self {
            selector,
            mode,
            matcher: RuleMatcher::WorkspaceOnly,
            description: "matched workspace-only rule".to_string(),
        }
    }

    pub fn absolute_glob(
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        let pattern = pattern.into();
        let matcher = compile_glob(&pattern)?;
        Ok(Self {
            selector,
            mode,
            matcher: RuleMatcher::AbsoluteGlob(matcher),
            description: format!("matched absolute path glob: {pattern}"),
        })
    }

    pub fn workspace_glob(
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        let pattern = pattern.into();
        let matcher = compile_glob(&pattern)?;
        Ok(Self {
            selector,
            mode,
            matcher: RuleMatcher::WorkspaceGlob(matcher),
            description: format!("matched workspace-relative glob: {pattern}"),
        })
    }

    fn matches_selector(&self, access: AccessKind) -> bool {
        matches!(
            (self.selector, access),
            (AccessSelector::Any, _)
                | (AccessSelector::Read, AccessKind::Read)
                | (AccessSelector::Write, AccessKind::Write)
        )
    }
}

#[derive(Debug, Clone)]
enum RuleMatcher {
    WorkspaceOnly,
    AbsoluteGlob(GlobMatcher),
    WorkspaceGlob(GlobMatcher),
}

impl RuleMatcher {
    fn matches(&self, ctx: &MatchContext) -> bool {
        match self {
            Self::WorkspaceOnly => ctx.in_workspace,
            Self::AbsoluteGlob(glob) => glob.is_match(&ctx.absolute_norm),
            Self::WorkspaceGlob(glob) => ctx
                .workspace_relative_norm
                .as_ref()
                .is_some_and(|relative| glob.is_match(relative)),
        }
    }
}

#[derive(Debug, Clone)]
struct MatchContext {
    absolute_norm: String,
    workspace_relative_norm: Option<String>,
    in_workspace: bool,
}

impl MatchContext {
    fn new(workspace_root: &Path, target_path: &Path) -> Self {
        let root_absolute = if workspace_root.is_absolute() {
            workspace_root.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(workspace_root)
        };
        let root_norm = normalize_path_string(&root_absolute);

        let absolute_target = if target_path.is_absolute() {
            target_path.to_path_buf()
        } else {
            root_absolute.join(target_path)
        };
        let absolute_norm = normalize_path_string(&absolute_target);

        let in_workspace =
            absolute_norm == root_norm || absolute_norm.starts_with(&format!("{root_norm}/"));

        let workspace_relative_norm = if in_workspace {
            if absolute_norm == root_norm {
                Some(".".to_string())
            } else {
                Some(
                    absolute_norm
                        .trim_start_matches(&format!("{root_norm}/"))
                        .to_string(),
                )
            }
        } else {
            None
        };

        Self {
            absolute_norm,
            workspace_relative_norm,
            in_workspace,
        }
    }
}

fn mode_decision(mode: PermissionMode, reason: &str) -> PermissionDecision {
    match mode {
        PermissionMode::Allow => PermissionDecision::Allow,
        PermissionMode::Ask => PermissionDecision::Ask {
            reason: reason.to_string(),
        },
        PermissionMode::Deny => PermissionDecision::Deny {
            reason: reason.to_string(),
        },
    }
}

fn compile_glob(pattern: &str) -> Result<GlobMatcher, PermissionConfigError> {
    let compiled = Glob::new(pattern).map_err(|source| PermissionConfigError::InvalidGlob {
        pattern: pattern.to_string(),
        source,
    })?;
    Ok(compiled.compile_matcher())
}

fn normalize_path_string(path: &Path) -> String {
    let cleaned = path.clean();
    let mut out = cleaned.to_string_lossy().replace('\\', "/");
    while out.ends_with('/') && out.len() > 1 {
        out.pop();
    }
    if cfg!(windows) {
        out.make_ascii_lowercase();
    }
    out
}
