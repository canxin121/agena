mod request;
mod runtime;
mod store;

use globset::{Glob, GlobMatcher};
use path_clean::PathClean;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::message::BuiltinToolInput;

pub use request::{
    PendingPermission, PermissionAction, PermissionReply, PermissionReplyKind, PermissionRequest,
    PermissionScope,
};
pub use runtime::{PermissionRuntime, PermissionRuntimeDecision, PermissionRuntimeError};
pub use store::{
    InMemoryPermissionRuleStore, PermissionRuleStore, PermissionStoreError, decide_from_mode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    Write,
    ExternalDirectory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessSelector {
    Read,
    Write,
    ExternalDirectory,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Clone)]
pub struct ToolPermissionPolicy {
    default_mode: PermissionMode,
    tool_modes: HashMap<String, PermissionMode>,
}

impl ToolPermissionPolicy {
    pub fn new(default_mode: PermissionMode) -> Self {
        Self {
            default_mode,
            tool_modes: HashMap::new(),
        }
    }

    pub fn allow_all() -> Self {
        Self::new(PermissionMode::Allow)
    }

    pub fn with_builtin_mode(mut self, builtin_name: &'static str, mode: PermissionMode) -> Self {
        self.tool_modes.insert(builtin_name.to_string(), mode);
        self
    }

    pub fn with_tool_mode(mut self, tool_name: impl Into<String>, mode: PermissionMode) -> Self {
        self.tool_modes.insert(tool_name.into(), mode);
        self
    }

    pub fn check_builtin(&self, input: &BuiltinToolInput) -> PermissionDecision {
        self.check_tool_name(builtin_name(input))
    }

    pub fn check_tool_name(&self, name: &str) -> PermissionDecision {
        let mode = self
            .tool_modes
            .get(name)
            .copied()
            .unwrap_or(self.default_mode);
        match mode {
            PermissionMode::Allow => PermissionDecision::Allow,
            PermissionMode::Ask => PermissionDecision::Ask {
                reason: format!("tool '{name}' requires confirmation by policy"),
            },
            PermissionMode::Deny => PermissionDecision::Deny {
                reason: format!("tool '{name}' denied by policy"),
            },
        }
    }
}

pub fn builtin_name(input: &BuiltinToolInput) -> &'static str {
    match input {
        BuiltinToolInput::Bash(_) => "bash",
        BuiltinToolInput::Read(_) => "read",
        BuiltinToolInput::Write(_) => "write",
        BuiltinToolInput::Edit(_) => "edit",
        BuiltinToolInput::ApplyPatch(_) => "apply_patch",
        BuiltinToolInput::Glob(_) => "glob",
        BuiltinToolInput::Grep(_) => "grep",
        BuiltinToolInput::Task(_) => "task",
        BuiltinToolInput::ToolSearch(_) => "tool_search",
        BuiltinToolInput::TodoWrite(_) => "todo_write",
    }
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
    default_external_directory: PermissionMode,
    rules: Vec<PermissionRule>,
}

impl PermissionPolicy {
    pub fn new(default_read: PermissionMode, default_write: PermissionMode) -> Self {
        Self {
            default_read,
            default_write,
            default_external_directory: PermissionMode::Allow,
            rules: Vec::new(),
        }
    }

    pub fn with_external_directory_default(mut self, mode: PermissionMode) -> Self {
        self.default_external_directory = mode;
        self
    }

    pub fn allow_all() -> Self {
        Self::new(PermissionMode::Allow, PermissionMode::Allow)
    }

    pub fn read_all_write_workspace_only() -> Self {
        Self {
            default_read: PermissionMode::Allow,
            default_write: PermissionMode::Deny,
            default_external_directory: PermissionMode::Deny,
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

    pub fn with_external_absolute_glob_rule(
        mut self,
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        self.rules.push(PermissionRule::external_absolute_glob(
            selector, mode, pattern,
        )?);
        Ok(self)
    }

    pub fn check_external_directory(
        &self,
        workspace_root: &Path,
        target_path: &Path,
    ) -> PermissionDecision {
        let context = MatchContext::new(workspace_root, target_path);
        self.check_access_with_context(AccessKind::ExternalDirectory, &context)
    }

    pub fn check_access(
        &self,
        access: AccessKind,
        workspace_root: &Path,
        target_path: &Path,
    ) -> PermissionDecision {
        let context = MatchContext::new(workspace_root, target_path);

        if !context.in_workspace {
            match self.check_access_with_context(AccessKind::ExternalDirectory, &context) {
                PermissionDecision::Allow => {}
                decision => return decision,
            }
        }

        self.check_access_with_context(access, &context)
    }

    fn check_access_with_context(
        &self,
        access: AccessKind,
        context: &MatchContext,
    ) -> PermissionDecision {
        if matches!(access, AccessKind::ExternalDirectory) && context.in_workspace {
            return PermissionDecision::Allow;
        }

        for rule in self.rules.iter().rev() {
            if !rule.matches_selector(access) {
                continue;
            }
            if rule.matcher.matches(context) {
                return mode_decision(rule.mode, &rule.description);
            }
        }

        match access {
            AccessKind::Read => mode_decision(self.default_read, "matched default read permission"),
            AccessKind::Write => {
                mode_decision(self.default_write, "matched default write permission")
            }
            AccessKind::ExternalDirectory => mode_decision(
                self.default_external_directory,
                "matched default external_directory permission",
            ),
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

    pub fn external_only(selector: AccessSelector, mode: PermissionMode) -> Self {
        Self {
            selector,
            mode,
            matcher: RuleMatcher::ExternalOnly,
            description: "matched external-directory-only rule".to_string(),
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

    pub fn external_absolute_glob(
        selector: AccessSelector,
        mode: PermissionMode,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        let pattern = pattern.into();
        let matcher = compile_glob(&pattern)?;
        Ok(Self {
            selector,
            mode,
            matcher: RuleMatcher::ExternalAbsoluteGlob(matcher),
            description: format!("matched external absolute path glob: {pattern}"),
        })
    }

    fn matches_selector(&self, access: AccessKind) -> bool {
        matches!(
            (self.selector, access),
            (AccessSelector::Any, _)
                | (AccessSelector::Read, AccessKind::Read)
                | (AccessSelector::Write, AccessKind::Write)
                | (
                    AccessSelector::ExternalDirectory,
                    AccessKind::ExternalDirectory,
                )
        )
    }
}

#[derive(Debug, Clone)]
enum RuleMatcher {
    WorkspaceOnly,
    ExternalOnly,
    AbsoluteGlob(GlobMatcher),
    WorkspaceGlob(GlobMatcher),
    ExternalAbsoluteGlob(GlobMatcher),
}

impl RuleMatcher {
    fn matches(&self, ctx: &MatchContext) -> bool {
        match self {
            Self::WorkspaceOnly => ctx.in_workspace,
            Self::ExternalOnly => !ctx.in_workspace,
            Self::AbsoluteGlob(glob) => glob.is_match(&ctx.absolute_norm),
            Self::WorkspaceGlob(glob) => ctx
                .workspace_relative_norm
                .as_ref()
                .is_some_and(|relative| glob.is_match(relative)),
            Self::ExternalAbsoluteGlob(glob) => {
                !ctx.in_workspace && glob.is_match(&ctx.absolute_norm)
            }
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::message::{BuiltinToolInput, ReadToolInput, WriteToolInput};

    use super::{
        AccessKind, AccessSelector, PermissionDecision, PermissionMode, PermissionPolicy,
        ToolPermissionPolicy, normalize_path_string,
    };

    #[test]
    fn workspace_paths_bypass_external_directory_gate() {
        let root = workspace_root();
        let target = root.join("src/main.rs");

        let policy = PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Allow)
            .with_external_directory_default(PermissionMode::Deny);

        assert_eq!(
            policy.check_access(AccessKind::Write, &root, &target),
            PermissionDecision::Allow
        );
        assert_eq!(
            policy.check_external_directory(&root, &target),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn external_directory_default_applies_to_non_workspace_paths() {
        let root = workspace_root();
        let target = external_file("denied/file.txt");

        let policy = PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Allow)
            .with_external_directory_default(PermissionMode::Deny);

        match policy.check_access(AccessKind::Read, &root, &target) {
            PermissionDecision::Deny { reason } => {
                assert!(reason.contains("external_directory"));
            }
            other => panic!("expected deny decision, got {other:?}"),
        }
    }

    #[test]
    fn external_custom_glob_can_whitelist_specific_external_paths() {
        let root = workspace_root();
        let allowed_dir = external_dir("whitelist");
        let blocked_dir = external_dir("blocked");

        let policy = PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Allow)
            .with_external_directory_default(PermissionMode::Deny)
            .with_external_absolute_glob_rule(
                AccessSelector::ExternalDirectory,
                PermissionMode::Allow,
                format!("{}/**", normalize_path_string(&allowed_dir)),
            )
            .expect("external glob should compile");

        assert_eq!(
            policy.check_access(AccessKind::Write, &root, &allowed_dir.join("ok.txt")),
            PermissionDecision::Allow
        );

        match policy.check_access(AccessKind::Write, &root, &blocked_dir.join("no.txt")) {
            PermissionDecision::Deny { .. } => {}
            other => panic!("expected deny decision, got {other:?}"),
        }
    }

    #[test]
    fn last_matching_rule_wins_for_external_path_overrides() {
        let root = workspace_root();
        let common = external_dir("policy");
        let allowed = common.join("allowed");

        let policy = PermissionPolicy::new(PermissionMode::Allow, PermissionMode::Allow)
            .with_external_directory_default(PermissionMode::Deny)
            .with_external_absolute_glob_rule(
                AccessSelector::ExternalDirectory,
                PermissionMode::Deny,
                format!("{}/**", normalize_path_string(&common)),
            )
            .expect("deny glob should compile")
            .with_external_absolute_glob_rule(
                AccessSelector::ExternalDirectory,
                PermissionMode::Allow,
                format!("{}/**", normalize_path_string(&allowed)),
            )
            .expect("allow override glob should compile");

        assert_eq!(
            policy.check_access(AccessKind::Write, &root, &allowed.join("ok.txt")),
            PermissionDecision::Allow
        );

        match policy.check_access(AccessKind::Write, &root, &common.join("other/no.txt")) {
            PermissionDecision::Deny { .. } => {}
            other => panic!("expected deny decision, got {other:?}"),
        }
    }

    #[test]
    fn tool_permission_policy_uses_default_mode() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Ask);
        let input = BuiltinToolInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });

        match policy.check_builtin(&input) {
            PermissionDecision::Ask { reason } => {
                assert!(reason.contains("read"));
            }
            other => panic!("expected ask decision, got {other:?}"),
        }
    }

    #[test]
    fn tool_permission_policy_supports_per_tool_overrides() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Deny)
            .with_builtin_mode("read", PermissionMode::Allow)
            .with_builtin_mode("write", PermissionMode::Ask);

        let read = BuiltinToolInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });
        let write = BuiltinToolInput::Write(WriteToolInput {
            file_path: "README.md".to_string(),
            content: "hello".to_string(),
        });

        assert_eq!(policy.check_builtin(&read), PermissionDecision::Allow);

        match policy.check_builtin(&write) {
            PermissionDecision::Ask { reason } => {
                assert!(reason.contains("write"));
            }
            other => panic!("expected ask decision, got {other:?}"),
        }
    }

    fn workspace_root() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\workspace\repo")
        } else {
            PathBuf::from("/workspace/repo")
        }
    }

    fn external_dir(suffix: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"D:\external").join(suffix)
        } else {
            PathBuf::from("/external").join(suffix)
        }
    }

    fn external_file(suffix: &str) -> PathBuf {
        external_dir("").join(Path::new(suffix))
    }
}
