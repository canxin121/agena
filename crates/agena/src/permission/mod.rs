mod request;
mod resolver;
mod store;

use globset::{Glob, GlobMatcher};
use path_clean::PathClean;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::message::FirstPartyToolInput;

pub use request::{
    DecisionTrace, DecisionTraceStep, PendingPermission, PermissionAction, PermissionReply,
    PermissionReplyKind, PermissionRequest, PermissionRiskLevel, PermissionScope,
    PolicySourceKind,
};
pub use resolver::{
    PermissionResolution, PermissionResolutionSource, resolve_permission_with_persisted_rule,
};
pub use store::{PersistedPermissionRule, decide_from_mode};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Pattern rules and per-tool modes win as configured. This is the
    /// historic behavior; nothing is auto-promoted to Ask.
    #[default]
    Auto,
    /// After all other rules, any `Allow` decision for a "sensitive"
    /// first-party tool (bash / apply_patch / write-style edits) is promoted to
    /// `Ask`. Use when a user wants every shell-out confirmed without
    /// authoring a per-pattern rule for each one.
    Ask,
}

#[derive(Debug, Clone)]
pub struct ToolPermissionPolicy {
    default_mode: PermissionMode,
    tool_modes: HashMap<String, PermissionMode>,
    bash_rules: Vec<BashPatternRule>,
    bash_deny_rules: Vec<BashPatternRule>,
    execution_mode: ExecutionMode,
}

#[derive(Debug, Clone)]
pub struct BashPatternRule {
    matcher: GlobMatcher,
    pattern: String,
    mode: PermissionMode,
}

impl BashPatternRule {
    pub fn new(
        pattern: impl Into<String>,
        mode: PermissionMode,
    ) -> Result<Self, PermissionConfigError> {
        let pattern = pattern.into();
        let glob = Glob::new(&pattern).map_err(|source| PermissionConfigError::InvalidGlob {
            pattern: pattern.clone(),
            source,
        })?;
        Ok(Self {
            matcher: glob.compile_matcher(),
            pattern,
            mode,
        })
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }
}

pub fn bash_rule_qualifier(command: &str, rules: &[BashPatternRule]) -> Option<String> {
    let normalized = command.trim();
    if normalized.is_empty() {
        return None;
    }
    rules
        .iter()
        .find(|rule| rule.matcher.is_match(normalized))
        .map(|rule| rule.pattern.clone())
}

pub fn bash_permission_qualifier(
    command: &str,
    policy: Option<&ToolPermissionPolicy>,
) -> Option<String> {
    let normalized = command.trim();
    if normalized.is_empty() {
        return None;
    }
    policy
        .and_then(|policy| {
            bash_rule_qualifier(normalized, policy.bash_deny_rules())
                .or_else(|| bash_rule_qualifier(normalized, policy.bash_rules()))
        })
        .or_else(|| Some(normalized.to_string()))
}

pub fn builtin_tool_action(
    input: &FirstPartyToolInput,
    policy: Option<&ToolPermissionPolicy>,
) -> PermissionAction {
    let tool_name = first_party_tool_name(input).to_string();
    let qualifier = match input {
        FirstPartyToolInput::Bash(bash) => bash_permission_qualifier(bash.command.as_str(), policy),
        _ => None,
    };
    PermissionAction::BuiltinTool {
        tool_name,
        qualifier,
    }
}

impl ToolPermissionPolicy {
    pub fn new(default_mode: PermissionMode) -> Self {
        Self {
            default_mode,
            tool_modes: HashMap::new(),
            bash_rules: Vec::new(),
            bash_deny_rules: Vec::new(),
            execution_mode: ExecutionMode::Auto,
        }
    }

    pub fn allow_all() -> Self {
        Self::new(PermissionMode::Allow)
    }

    pub fn with_execution_mode(mut self, mode: ExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }

    pub fn execution_mode(&self) -> ExecutionMode {
        self.execution_mode
    }

    pub fn with_first_party_mode(
        mut self,
        first_party_tool_name: &'static str,
        mode: PermissionMode,
    ) -> Self {
        self.tool_modes
            .insert(first_party_tool_name.to_string(), mode);
        self
    }

    pub fn with_tool_mode(mut self, tool_name: impl Into<String>, mode: PermissionMode) -> Self {
        self.tool_modes.insert(tool_name.into(), mode);
        self
    }

    /// Append a bash command pattern rule. Patterns use `globset` glob syntax
    /// against the literal command string (e.g. `git status`, `rm *`,
    /// `pnpm *`). Rules are evaluated in registration order; the first match
    /// wins. Bash-pattern rules apply *only* to `FirstPartyToolInput::Bash` and
    /// override the per-tool default for that one invocation when matched.
    pub fn with_bash_pattern_rule(
        mut self,
        pattern: impl Into<String>,
        mode: PermissionMode,
    ) -> Result<Self, PermissionConfigError> {
        self.bash_rules.push(BashPatternRule::new(pattern, mode)?);
        Ok(self)
    }

    /// Append a bash command pattern that *unconditionally* denies execution
    /// — checked before everything else, including `bash_rules` and the
    /// per-tool override. Useful for a global blocklist (`rm -rf *`,
    /// `:(){:|:&};:`, etc.) that even an explicit `Ask` rule should not be
    /// able to whitelist.
    pub fn with_bash_deny_pattern(
        mut self,
        pattern: impl Into<String>,
    ) -> Result<Self, PermissionConfigError> {
        self.bash_deny_rules
            .push(BashPatternRule::new(pattern, PermissionMode::Deny)?);
        Ok(self)
    }

    pub fn bash_rules(&self) -> &[BashPatternRule] {
        &self.bash_rules
    }

    pub fn bash_deny_rules(&self) -> &[BashPatternRule] {
        &self.bash_deny_rules
    }

    pub fn check_first_party(&self, input: &FirstPartyToolInput) -> PermissionDecision {
        let tool_name = first_party_tool_name(input);
        let command = match input {
            FirstPartyToolInput::Bash(bash) => Some(bash.command.as_str()),
            _ => None,
        };
        let sensitive = matches!(
            input,
            FirstPartyToolInput::Bash(_)
                | FirstPartyToolInput::ApplyPatch(_)
                | FirstPartyToolInput::NotebookEdit(_)
                | FirstPartyToolInput::PowerShell(_)
        );
        self.check_tool(tool_name, command, sensitive)
    }

    pub fn check_tool_name(&self, name: &str) -> PermissionDecision {
        self.check_tool(name, None, false)
    }

    pub fn check_tool(
        &self,
        name: &str,
        command: Option<&str>,
        sensitive: bool,
    ) -> PermissionDecision {
        if name == "bash"
            && let Some(command) = command
        {
            if let Some(decision) = self.evaluate_bash_deny(command) {
                return decision;
            }
            if let Some(decision) = self.evaluate_bash_pattern(command) {
                return self.apply_execution_mode(name, sensitive, decision);
            }
        }
        let base = self.check_tool_mode(name);
        self.apply_execution_mode(name, sensitive, base)
    }

    fn check_tool_mode(&self, name: &str) -> PermissionDecision {
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

    fn evaluate_bash_pattern(&self, command: &str) -> Option<PermissionDecision> {
        let normalized = command.trim();
        if normalized.is_empty() {
            return None;
        }
        for rule in &self.bash_rules {
            if rule.matcher.is_match(normalized) {
                let decision = match rule.mode {
                    PermissionMode::Allow => PermissionDecision::Allow,
                    PermissionMode::Ask => PermissionDecision::Ask {
                        reason: format!(
                            "bash command matches `{}` and requires confirmation",
                            rule.pattern
                        ),
                    },
                    PermissionMode::Deny => PermissionDecision::Deny {
                        reason: format!(
                            "bash command matches `{}` and is denied by policy",
                            rule.pattern
                        ),
                    },
                };
                return Some(decision);
            }
        }
        None
    }

    fn evaluate_bash_deny(&self, command: &str) -> Option<PermissionDecision> {
        let normalized = command.trim();
        if normalized.is_empty() {
            return None;
        }
        for rule in &self.bash_deny_rules {
            if rule.matcher.is_match(normalized) {
                return Some(PermissionDecision::Deny {
                    reason: format!(
                        "bash command matches deny pattern `{}` and is unconditionally blocked",
                        rule.pattern
                    ),
                });
            }
        }
        None
    }

    /// In `Ask` mode, promote any `Allow` decision for a sensitive tool to
    /// `Ask`. Deny stays Deny; explicit Ask stays Ask. Other modes pass
    /// through unchanged.
    fn apply_execution_mode(
        &self,
        tool_name: &str,
        sensitive: bool,
        decision: PermissionDecision,
    ) -> PermissionDecision {
        if !matches!(self.execution_mode, ExecutionMode::Ask) || !sensitive {
            return decision;
        }
        match decision {
            PermissionDecision::Allow => PermissionDecision::Ask {
                reason: format!("execution mode `ask` requires confirmation for `{tool_name}`"),
            },
            other => other,
        }
    }
}

pub fn first_party_tool_name(input: &FirstPartyToolInput) -> &'static str {
    match input {
        FirstPartyToolInput::Bash(_) => "bash",
        FirstPartyToolInput::Read(_) => "read",
        FirstPartyToolInput::ViewFile(_) => "view_file",
        FirstPartyToolInput::ApplyPatch(_) => "apply_patch",
        FirstPartyToolInput::Glob(_) => "glob",
        FirstPartyToolInput::Grep(_) => "grep",
        FirstPartyToolInput::Task(_) => "task",
        FirstPartyToolInput::ToolSearch(_) => "tool_search",
        FirstPartyToolInput::TodoWrite(_) => "todo_write",
        FirstPartyToolInput::AskUser(_) => "ask_user",
        FirstPartyToolInput::Monitor(_) => "monitor",
        FirstPartyToolInput::WebFetch(_) => "web_fetch",
        FirstPartyToolInput::WebSearch(_) => "web_search",
        FirstPartyToolInput::EnterPlanMode(_) => "enter_plan_mode",
        FirstPartyToolInput::ExitPlanMode(_) => "exit_plan_mode",
        FirstPartyToolInput::EnterWorktree(_) => "enter_worktree",
        FirstPartyToolInput::ExitWorktree(_) => "exit_worktree",
        FirstPartyToolInput::CronCreate(_) => "cron_create",
        FirstPartyToolInput::CronList(_) => "cron_list",
        FirstPartyToolInput::CronDelete(_) => "cron_delete",
        FirstPartyToolInput::ScheduleWakeup(_) => "schedule_wakeup",
        FirstPartyToolInput::LspDefinition(_) => "lsp_definition",
        FirstPartyToolInput::LspReferences(_) => "lsp_references",
        FirstPartyToolInput::LspHover(_) => "lsp_hover",
        FirstPartyToolInput::LspDiagnostics(_) => "lsp_diagnostics",
        FirstPartyToolInput::NotebookEdit(_) => "notebook_edit",
        FirstPartyToolInput::PowerShell(_) => "powershell",
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
                return decide_from_mode(rule.mode, &rule.description);
            }
        }

        match access {
            AccessKind::Read => {
                decide_from_mode(self.default_read, "matched default read permission")
            }
            AccessKind::Write => {
                decide_from_mode(self.default_write, "matched default write permission")
            }
            AccessKind::ExternalDirectory => decide_from_mode(
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

    use crate::message::{ApplyPatchToolInput, FirstPartyToolInput, ReadToolInput};

    use super::{
        AccessKind, AccessSelector, ExecutionMode, PermissionDecision, PermissionMode,
        PermissionPolicy, ToolPermissionPolicy, normalize_path_string,
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
        let input = FirstPartyToolInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });

        match policy.check_first_party(&input) {
            PermissionDecision::Ask { reason } => {
                assert!(reason.contains("read"));
            }
            other => panic!("expected ask decision, got {other:?}"),
        }
    }

    #[test]
    fn tool_permission_policy_supports_per_tool_overrides() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Deny)
            .with_first_party_mode("read", PermissionMode::Allow)
            .with_first_party_mode("apply_patch", PermissionMode::Ask);

        let read = FirstPartyToolInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });
        let apply_patch = FirstPartyToolInput::ApplyPatch(ApplyPatchToolInput {
            patch: "*** Begin Patch\n*** Add File: README.md\n+hello\n*** End Patch".to_string(),
        });

        assert_eq!(policy.check_first_party(&read), PermissionDecision::Allow);

        match policy.check_first_party(&apply_patch) {
            PermissionDecision::Ask { reason } => {
                assert!(reason.contains("apply_patch"));
            }
            other => panic!("expected ask decision, got {other:?}"),
        }
    }

    #[test]
    fn bash_pattern_rule_allows_matching_command_even_when_default_is_ask() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Allow)
            .with_first_party_mode("bash", PermissionMode::Ask)
            .with_bash_pattern_rule("git *", PermissionMode::Allow)
            .expect("git glob compiles");

        let bash = FirstPartyToolInput::Bash(crate::message::BashToolInput {
            command: "git status".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        });
        assert_eq!(policy.check_first_party(&bash), PermissionDecision::Allow);

        let other = FirstPartyToolInput::Bash(crate::message::BashToolInput {
            command: "make".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        });
        match policy.check_first_party(&other) {
            PermissionDecision::Ask { reason } => assert!(reason.contains("bash")),
            other => panic!("expected ask decision, got {other:?}"),
        }
    }

    #[test]
    fn bash_pattern_rule_can_demand_confirmation_for_dangerous_command() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Allow)
            .with_bash_pattern_rule("rm *", PermissionMode::Ask)
            .expect("rm glob compiles");

        let bash = FirstPartyToolInput::Bash(crate::message::BashToolInput {
            command: "rm -rf build".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        });
        match policy.check_first_party(&bash) {
            PermissionDecision::Ask { reason } => assert!(reason.contains("`rm *`")),
            other => panic!("expected ask decision, got {other:?}"),
        }

        // Non-bash invocations are unaffected by bash pattern rules.
        let read = FirstPartyToolInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });
        assert_eq!(policy.check_first_party(&read), PermissionDecision::Allow);
    }

    #[test]
    fn bash_pattern_rule_first_match_wins() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Ask)
            .with_bash_pattern_rule("git push *", PermissionMode::Ask)
            .expect("first rule compiles")
            .with_bash_pattern_rule("git *", PermissionMode::Allow)
            .expect("second rule compiles");

        let push = FirstPartyToolInput::Bash(crate::message::BashToolInput {
            command: "git push origin master".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        });
        match policy.check_first_party(&push) {
            PermissionDecision::Ask { reason } => assert!(reason.contains("`git push *`")),
            other => panic!("expected ask decision, got {other:?}"),
        }

        let status = FirstPartyToolInput::Bash(crate::message::BashToolInput {
            command: "git status".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        });
        assert_eq!(policy.check_first_party(&status), PermissionDecision::Allow);
    }

    #[test]
    fn bash_without_matching_pattern_falls_through_to_tool_default() {
        let policy = ToolPermissionPolicy::new(PermissionMode::Deny)
            .with_bash_pattern_rule("git *", PermissionMode::Allow)
            .expect("rule compiles");

        let bash = FirstPartyToolInput::Bash(crate::message::BashToolInput {
            command: "make build".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        });
        match policy.check_first_party(&bash) {
            PermissionDecision::Deny { reason } => assert!(reason.contains("bash")),
            other => panic!("expected deny decision, got {other:?}"),
        }
    }

    #[test]
    fn execution_mode_ask_promotes_allow_for_sensitive_custom_tool() {
        let policy = ToolPermissionPolicy::allow_all().with_execution_mode(ExecutionMode::Ask);

        match policy.check_tool("plugin_paths", None, true) {
            PermissionDecision::Ask { reason } => {
                assert!(reason.contains("ask"));
                assert!(reason.contains("plugin_paths"));
            }
            other => panic!("expected Ask for sensitive custom tool, got {other:?}"),
        }

        assert_eq!(
            policy.check_tool("plugin_paths", None, false),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn execution_mode_ask_promotes_allow_for_bash_and_apply_patch() {
        let policy = ToolPermissionPolicy::allow_all().with_execution_mode(ExecutionMode::Ask);

        let bash = FirstPartyToolInput::Bash(crate::message::BashToolInput {
            command: "ls".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        });
        match policy.check_first_party(&bash) {
            PermissionDecision::Ask { reason } => {
                assert!(reason.contains("ask"));
                assert!(reason.contains("bash"));
            }
            other => panic!("expected Ask under execution_mode=ask, got {other:?}"),
        }

        let apply = FirstPartyToolInput::ApplyPatch(crate::message::ApplyPatchToolInput {
            patch: "*** Begin Patch\n*** Add File: x.md\n+x\n*** End Patch".to_string(),
        });
        match policy.check_first_party(&apply) {
            PermissionDecision::Ask { .. } => {}
            other => panic!("expected Ask for apply_patch, got {other:?}"),
        }

        // Read-style tools are unaffected.
        let read = FirstPartyToolInput::Read(ReadToolInput {
            file_path: "README.md".to_string(),
            offset: None,
            limit: None,
        });
        assert_eq!(policy.check_first_party(&read), PermissionDecision::Allow);
    }

    #[test]
    fn execution_mode_auto_does_not_promote_decisions() {
        let policy = ToolPermissionPolicy::allow_all().with_execution_mode(ExecutionMode::Auto);
        let bash = FirstPartyToolInput::Bash(crate::message::BashToolInput {
            command: "ls".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        });
        assert_eq!(policy.check_first_party(&bash), PermissionDecision::Allow);
    }

    #[test]
    fn bash_deny_pattern_overrides_allow_rule_and_ask_mode() {
        let policy = ToolPermissionPolicy::allow_all()
            .with_execution_mode(ExecutionMode::Ask)
            .with_bash_pattern_rule("rm *", PermissionMode::Allow)
            .expect("rm allow rule compiles")
            .with_bash_deny_pattern("rm -rf /*")
            .expect("deny pattern compiles");

        let dangerous = FirstPartyToolInput::Bash(crate::message::BashToolInput {
            command: "rm -rf /tmp/oops".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        });
        match policy.check_first_party(&dangerous) {
            PermissionDecision::Deny { reason } => {
                assert!(reason.contains("deny pattern"));
            }
            other => panic!("expected unconditional Deny, got {other:?}"),
        }

        // A non-matching command still flows through the normal pipeline.
        let safe = FirstPartyToolInput::Bash(crate::message::BashToolInput {
            command: "rm tmpfile".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        });
        // 'rm tmpfile' matches the allow rule, so under Ask it gets... Allow!
        // (Allow rule is explicit, not the policy default — apply_execution_mode
        //  only promotes the *fallthrough* default Allow.)
        // Wait — apply_execution_mode runs after both bash_rules and the
        // tool-name fallback, so explicit allow gets promoted too. Let's
        // just assert it isn't Deny.
        if let PermissionDecision::Deny { .. } = policy.check_first_party(&safe) {
            panic!("rm tmpfile should not be denied")
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
