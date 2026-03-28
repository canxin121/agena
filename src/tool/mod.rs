mod apply_patch;
mod bash;
mod edit;
mod glob;
mod grep;
mod orchestrator;
mod read;
mod result;
mod task;
mod write;

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::agent::Agent;
use crate::message::{BuiltinToolInput, BuiltinToolOutput};
use crate::permission::{AccessKind, PermissionDecision};
use procwarden::{
    SandboxCommandRequest, SandboxError, SandboxExecOutput, SandboxManager, SandboxPolicy,
};

pub use apply_patch::{AppliedFileChange, ApplyPatchExecution};
pub use result::{BuiltinExecution, ToolExecutionView};

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("permission confirmation required: {0}")]
    PermissionAsk(String),
    #[error("invalid patch: {0}")]
    InvalidPatch(String),
    #[error("invalid tool input: {0}")]
    InvalidInput(String),
    #[error("invalid glob pattern: {0}")]
    InvalidGlobPattern(#[from] globset::Error),
    #[error("invalid regex pattern: {0}")]
    InvalidRegexPattern(#[from] regex::Error),
    #[error("sandbox error: {0}")]
    Sandbox(#[from] SandboxError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported builtin tool in executor: {0}")]
    UnsupportedBuiltin(&'static str),
}

pub struct ToolExecutor {
    workspace_root: PathBuf,
    agent: Agent,
    sandbox_policy: SandboxPolicy,
    sandbox_manager: SandboxManager,
}

impl ToolExecutor {
    pub fn new(workspace_root: impl Into<PathBuf>, agent: Agent) -> Self {
        Self::with_sandbox_policy(
            workspace_root,
            agent,
            SandboxPolicy::new_workspace_write_policy(),
        )
    }

    pub fn with_sandbox_policy(
        workspace_root: impl Into<PathBuf>,
        agent: Agent,
        sandbox_policy: SandboxPolicy,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            agent,
            sandbox_policy,
            sandbox_manager: SandboxManager::new(),
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn agent(&self) -> &Agent {
        &self.agent
    }

    pub fn sandbox_policy(&self) -> &SandboxPolicy {
        &self.sandbox_policy
    }

    pub fn execute_builtin_detailed(
        &self,
        input: &BuiltinToolInput,
    ) -> Result<BuiltinExecution, ToolError> {
        match self.agent.authorize_builtin_tool(input) {
            PermissionDecision::Allow => {}
            PermissionDecision::Ask { reason } => return Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => {
                return Err(ToolError::PermissionDenied(reason));
            }
        }

        orchestrator::execute_builtin(self, input)
    }

    pub fn execute_builtin(
        &self,
        input: &BuiltinToolInput,
    ) -> Result<(BuiltinToolOutput, Option<ApplyPatchExecution>), ToolError> {
        let execution = self.execute_builtin_detailed(input)?;
        Ok((execution.output, execution.apply_patch))
    }

    pub(crate) fn resolve_target_path(&self, raw_path: &str) -> PathBuf {
        let candidate = PathBuf::from(raw_path);
        if candidate.is_absolute() {
            candidate
        } else {
            self.workspace_root.join(candidate)
        }
    }

    pub(crate) fn execute_sandboxed_command(
        &self,
        request: &SandboxCommandRequest,
    ) -> Result<SandboxExecOutput, ToolError> {
        self.sandbox_manager
            .execute(request, self.sandbox_policy(), self.workspace_root())
            .map_err(ToolError::from)
    }

    pub(crate) fn display_path(&self, path: &Path) -> String {
        if let Ok(relative) = path.strip_prefix(&self.workspace_root) {
            let normalized = normalize_path_for_display(relative);
            if normalized.is_empty() {
                return ".".to_string();
            }
            return normalized;
        }
        normalize_path_for_display(path)
    }

    pub(crate) fn ensure_read_permission(&self, target_path: &Path) -> Result<(), ToolError> {
        self.ensure_access_permission(AccessKind::Read, target_path)
    }

    pub(crate) fn ensure_edit_permission(&self, target_path: &Path) -> Result<(), ToolError> {
        self.ensure_access_permission(AccessKind::Write, target_path)
    }

    fn ensure_access_permission(
        &self,
        access: AccessKind,
        target_path: &Path,
    ) -> Result<(), ToolError> {
        match self.agent.authorize_path_access(
            AccessKind::ExternalDirectory,
            self.workspace_root(),
            target_path,
        ) {
            PermissionDecision::Allow => {}
            PermissionDecision::Ask { reason } => return Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => return Err(ToolError::PermissionDenied(reason)),
        }

        match self
            .agent
            .authorize_path_access(access, self.workspace_root(), target_path)
        {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Ask { reason } => Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => Err(ToolError::PermissionDenied(reason)),
        }
    }
}

pub(crate) fn normalize_path_for_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    use crate::message::{
        BashToolInput, BuiltinToolInput, BuiltinToolOutput, EditToolInput, GlobToolInput,
        GrepToolInput, ReadToolInput, TaskToolInput, WriteToolInput,
    };
    use crate::permission::PermissionPolicy;
    use procwarden::SandboxPolicy;

    use super::ToolExecutor;

    #[derive(Debug)]
    struct TempWorkspace {
        root: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("agena-tool-tests-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).expect("failed to create temp workspace");
            Self { root }
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn build_executor(root: &Path) -> ToolExecutor {
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all());
        ToolExecutor::new(root, agent)
    }

    fn build_executor_with_policy(root: &Path, policy: SandboxPolicy) -> ToolExecutor {
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all());
        ToolExecutor::with_sandbox_policy(root, agent, policy)
    }

    #[test]
    fn read_builtin_returns_line_numbered_preview() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("notes.txt");
        fs::write(&file_path, "one\ntwo\nthree\n").expect("failed to seed file");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Read(ReadToolInput {
                file_path: "notes.txt".to_string(),
                offset: Some(2),
                limit: Some(2),
            }))
            .expect("read builtin should succeed");

        match result.output {
            BuiltinToolOutput::Read {
                preview,
                truncated,
                loaded_paths,
            } => {
                let preview = preview.expect("preview must exist");
                assert!(preview.contains("2: two"));
                assert!(preview.contains("3: three"));
                assert_eq!(truncated, Some(false));
                assert_eq!(loaded_paths, vec!["notes.txt".to_string()]);
            }
            other => panic!("expected read output, got {other:?}"),
        }
    }

    #[test]
    fn write_and_edit_builtins_update_file_content() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        executor
            .execute_builtin_detailed(&BuiltinToolInput::Write(WriteToolInput {
                file_path: "src/app.txt".to_string(),
                content: "hello world\n".to_string(),
            }))
            .expect("write builtin should succeed");

        executor
            .execute_builtin_detailed(&BuiltinToolInput::Edit(EditToolInput {
                file_path: "src/app.txt".to_string(),
                old_string: "world".to_string(),
                new_string: "agena".to_string(),
                replace_all: false,
            }))
            .expect("edit builtin should succeed");

        let current = fs::read_to_string(workspace.root.join("src/app.txt"))
            .expect("failed to read edited file");
        assert_eq!(current, "hello agena\n");
    }

    #[test]
    fn glob_and_grep_report_match_counts() {
        let workspace = TempWorkspace::new();
        fs::create_dir_all(workspace.root.join("src/nested")).expect("failed to create tree");
        fs::write(
            workspace.root.join("src/main.rs"),
            "fn main() { println!(\"hello\"); }\n",
        )
        .expect("failed to write main.rs");
        fs::write(
            workspace.root.join("src/nested/lib.rs"),
            "pub fn value() -> i32 { 7 }\n",
        )
        .expect("failed to write lib.rs");

        let executor = build_executor(&workspace.root);

        let glob_result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Glob(GlobToolInput {
                pattern: "**/*.rs".to_string(),
                path: Some("src".to_string()),
            }))
            .expect("glob should succeed");

        match glob_result.output {
            BuiltinToolOutput::Glob { count } => {
                assert_eq!(count, Some(2));
            }
            other => panic!("expected glob output, got {other:?}"),
        }

        let grep_result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Grep(GrepToolInput {
                pattern: "hello".to_string(),
                path: Some("src".to_string()),
                include: Some("**/*.rs".to_string()),
            }))
            .expect("grep should succeed");

        match grep_result.output {
            BuiltinToolOutput::Grep { matches } => {
                assert_eq!(matches, Some(1));
            }
            other => panic!("expected grep output, got {other:?}"),
        }
    }

    #[test]
    fn task_builtin_generates_session_id() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Task(TaskToolInput {
                description: "inspect code".to_string(),
                prompt: "find modules".to_string(),
                subagent_type: "explore".to_string(),
                task_id: None,
                command: None,
            }))
            .expect("task should succeed");

        match result.output {
            BuiltinToolOutput::Task { session_id, .. } => {
                assert!(session_id.is_some());
            }
            other => panic!("expected task output, got {other:?}"),
        }
    }

    #[test]
    fn bash_builtin_runs_command_with_read_only_policy() {
        if cfg!(windows) {
            // Windows host environments can include PATH entries whose ACL cannot be audited
            // in sandbox preflight, which makes this smoke test flaky/non-portable.
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor_with_policy(
            &workspace.root,
            SandboxPolicy::new_read_only_policy().with_world_writable_audit(false),
        );

        let result = executor
            .execute_builtin_detailed(&BuiltinToolInput::Bash(BashToolInput {
                command: "echo hello_agena".to_string(),
                description: "smoke bash".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
            }))
            .expect("bash builtin should succeed");

        match result.output {
            BuiltinToolOutput::Bash {
                output,
                description,
            } => {
                let output = output.expect("output should exist").to_ascii_lowercase();
                assert!(output.contains("hello_agena"));
                assert!(description.is_some());
            }
            other => panic!("expected bash output, got {other:?}"),
        }
    }
}
