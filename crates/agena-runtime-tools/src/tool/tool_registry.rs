use agena_domain::ToolInvocation;
use agena_tool::ToolPermissionCheck;

/// A registered plugin execution tool. Runtime protocol functions have an
/// independent registry and are never registered as plugin tools.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionTool {
    registered: RegisteredTool,
}

impl ExecutionTool {
    pub fn from_registered_tool(registered: RegisteredTool) -> Self {
        Self { registered }
    }

    pub fn registered(&self) -> &RegisteredTool {
        &self.registered
    }

    pub fn into_registered(self) -> RegisteredTool {
        self.registered
    }
}

impl std::ops::Deref for ExecutionTool {
    type Target = RegisteredTool;

    fn deref(&self) -> &Self::Target {
        self.registered()
    }
}

/// Convert an internal tool key such as `agena.session.rename` into the compact
/// execution-tool name used by `tools_help` and `tools_call`, such as
/// `session.rename`. Names reserved for runtime protocol or control functions
/// retain the canonical execution identity.
pub fn compact_tool_call_name(name: &str) -> String {
    let mut parts = name.splitn(3, '.');
    let _namespace = parts.next();
    let plugin = parts.next();
    let tool = parts.next();
    let compact = match (plugin, tool) {
        (Some(plugin), Some(tool)) if plugin == tool => plugin.to_string(),
        (Some(plugin), Some(tool)) => format!("{plugin}.{tool}"),
        _ => name.to_string(),
    };
    if agena_domain::ToolApiFunction::from_function_name(&compact).is_some()
        || agena_domain::is_control_function_name(&compact)
    {
        name.to_owned()
    } else {
        compact
    }
}

/// Produce the callable Tool API target name for each ordinary execution tool.
/// Compact `plugin.tool` names are preferred, but collisions retain the full
/// registry key so every advertised name resolves to exactly one tool.
pub fn execution_tool_names(tools: &[ExecutionTool]) -> Vec<String> {
    let mut compact_name_counts = std::collections::HashMap::<String, usize>::new();
    for tool in tools {
        *compact_name_counts
            .entry(compact_tool_call_name(tool.canonical_name().as_str()))
            .or_default() += 1;
    }
    tools
        .iter()
        .map(|tool| {
            let canonical = tool.canonical_name();
            let compact = compact_tool_call_name(canonical.as_str());
            if compact_name_counts
                .get(compact.as_str())
                .copied()
                .unwrap_or_default()
                > 1
            {
                canonical
            } else {
                compact
            }
        })
        .collect()
}

pub fn suggest_tool_names<I, T>(requested: &str, candidates: I, limit: usize) -> Vec<String>
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let requested = requested.trim();
    let requested_lower = requested.to_ascii_lowercase();
    let mut ranked: Vec<(usize, String)> = Vec::new();

    for candidate in candidates {
        let name = candidate.as_ref().trim();
        if name.is_empty() {
            continue;
        }
        let score = normalized_tool_name_distance(requested, name);
        if score == 0 {
            continue;
        }
        let name_lower = name.to_ascii_lowercase();
        if score <= 4
            || name_lower.contains(requested_lower.as_str())
            || requested_lower.contains(name_lower.as_str())
        {
            ranked.push((score, name.to_string()));
        }
    }

    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    let mut suggestions = Vec::new();
    for (_, name) in ranked {
        if !suggestions.contains(&name) {
            suggestions.push(name);
        }
        if suggestions.len() >= limit {
            break;
        }
    }
    suggestions
}

pub(crate) fn unknown_tool_message(requested: &str, suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        return format!("unknown tool '{requested}'");
    }
    format!(
        "unknown tool '{requested}'. Did you mean {}?",
        suggestions
            .iter()
            .map(|tool| format!("`{tool}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(crate) fn unknown_tool_hint(requested: &str, suggestions: Vec<String>) -> ToolError {
    let reason = unknown_tool_message(requested, &suggestions);
    ToolError::ToolUnavailable(Box::new(agena_domain::ToolUnavailableResult {
        tool_name: requested.to_string(),
        reason,
        suggestions,
        source: "tool_registry".to_string(),
        retryable: false,
    }))
}

pub(super) fn normalized_tool_name_distance(left: &str, right: &str) -> usize {
    let left = left.trim().to_ascii_lowercase();
    let right = right.trim().to_ascii_lowercase();
    if left == right {
        return 0;
    }
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut prev = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut curr = vec![0; right_chars.len() + 1];
    for (i, left_ch) in left_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, right_ch) in right_chars.iter().enumerate() {
            let replace = prev[j] + usize::from(left_ch != right_ch);
            let insert = curr[j] + 1;
            let delete = prev[j + 1] + 1;
            curr[j + 1] = replace.min(insert.min(delete));
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[right_chars.len()]
}

pub fn registered_tool_matches_name(registered_tool: &RegisteredTool, name: &str) -> bool {
    registered_tool.canonical_name() == name
        || compact_tool_call_name(registered_tool.canonical_name().as_str()) == name
}

pub(crate) fn unique_registered_tool_match(
    tools: impl IntoIterator<Item = RegisteredTool>,
    name: &str,
) -> Option<RegisteredTool> {
    let tools = tools.into_iter().collect::<Vec<_>>();
    if let Some(exact) = tools.iter().find(|tool| tool.canonical_name() == name) {
        return Some(exact.clone());
    }
    let mut aliases = tools
        .into_iter()
        .filter(|tool| registered_tool_matches_name(tool, name));
    let first = aliases.next()?;
    aliases.next().is_none().then_some(first)
}

#[derive(Clone, Default)]
/// Runtime context of a tool execution.
pub struct ToolRuntimeContext {
    pub session_id: Option<i64>,
    pub call_id: Option<i64>,
    pub prepared_shell_command: Option<PreparedShellCommand>,
    /// Exact assistant tool receipt for durable child work created by this
    /// invocation. Application/host calls leave this empty.
    pub launch_provenance: Option<agena_scheduler::ScheduledJobLaunchProvenance>,
    /// Typed sources owned by this tool. Tool completion commits their tails.
    pub output: Option<super::ToolOutputStreams>,
    /// The committed tool authority used to reserve independent activities.
    pub tool: Option<agena_storage::kernel::ToolHandle>,
}

impl std::fmt::Debug for ToolRuntimeContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRuntimeContext")
            .field("session_id", &self.session_id)
            .field("call_id", &self.call_id)
            .field("prepared_shell_command", &self.prepared_shell_command)
            .field("launch_provenance", &self.launch_provenance)
            .field("output", &self.output)
            .field(
                "tool",
                &self
                    .tool
                    .as_ref()
                    .map(agena_storage::kernel::ToolHandle::id),
            )
            .finish()
    }
}

impl ToolRuntimeContext {
    pub(crate) async fn begin_activity_output(
        &self,
        kind: agena_storage::kernel::BackgroundActivityKind,
        label: String,
        content: agena_domain::ContentKind,
    ) -> Result<Option<super::ToolOutputStreams>, ToolError> {
        match &self.tool {
            Some(tool) => super::ToolOutputStreams::begin_activity(tool, kind, label, content)
                .await
                .map(Some)
                .map_err(|error| {
                    ToolError::plugin(format!("commit background launch authority: {error}"))
                }),
            // Stateless host calls can carry a plugin session correlation ID
            // without being a kernel execution. They create no transcript
            // source or durable authority. Session orchestration supplies tool.
            None => Ok(None),
        }
    }
}

/// Internal-only diagnostic carried to the failure projection boundary.
/// The inner text cannot be constructed or inspected outside this crate;
/// consumers must use `ToolError`'s semantic constructors and projections.
#[derive(Debug)]
pub struct ToolDiagnostic(String);

impl std::fmt::Display for ToolDiagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0.as_str())
    }
}

/// A plugin-originated failure after its safe public contract has been split
/// from the diagnostic that is useful only to operators. This is deliberately
/// not a string: callers must persist `public`, never reconstruct a user error
/// from an untrusted plugin message.
#[derive(Debug)]
pub struct PluginToolFailure {
    pub public: agena_failure::Failure,
    diagnostic: ToolDiagnostic,
}

impl std::fmt::Display for PluginToolFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.diagnostic.fmt(formatter)
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Error)]
/// Error from tool execution.
pub enum ToolError {
    #[error("tool execution cancelled")]
    Cancelled,
    #[error("operation blocked by permission policy: {}", .0.reason)]
    PolicyDenied(Box<agena_domain::PolicyDeniedResult>),
    #[error("permission request declined by user")]
    UserDeclined(Box<agena_domain::UserDeclinedResult>),
    #[error("required execution capability is unavailable: {}", .0.reason)]
    CapabilityUnavailable(Box<agena_domain::CapabilityUnavailableResult>),
    #[error("tool is unavailable: {}", .0.reason)]
    ToolUnavailable(Box<agena_domain::ToolUnavailableResult>),
    #[error("user input required")]
    UserInputRequired(Box<AskUserToolInput>),
    #[error("invalid patch: {0}")]
    InvalidPatch(ToolDiagnostic),
    #[error("invalid tool input: {diagnostic}")]
    InvalidInput {
        diagnostic: ToolDiagnostic,
        fields: Vec<agena_failure::FieldIssue>,
    },
    #[error("invalid glob pattern: {0}")]
    InvalidGlobPattern(#[from] globset::Error),
    #[error("invalid regex pattern: {0}")]
    InvalidRegexPattern(#[from] regex::Error),
    #[error("shell error: {0}")]
    Shell(#[from] ShellError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("plugin error: {0}")]
    Plugin(Box<PluginToolFailure>),
}

impl ToolError {
    /// Structured field issues for invalid-input failures, if any.
    pub fn field_issues(&self) -> &[agena_failure::FieldIssue] {
        match self {
            Self::InvalidInput { fields, .. } => fields.as_slice(),
            _ => &[],
        }
    }

    pub fn actionable_message(&self) -> Option<String> {
        match self {
            Self::InvalidPatch(diagnostic) => Some(diagnostic.to_string()),
            Self::InvalidInput { diagnostic, .. } => Some(diagnostic.to_string()),
            Self::InvalidGlobPattern(error) => Some(error.to_string()),
            Self::InvalidRegexPattern(error) => Some(error.to_string()),
            Self::Shell(error) => Some(agena_failure::diagnostic::format_error_chain(error)),
            Self::Io(error) => Some(agena_failure::diagnostic::format_error_chain(error)),
            Self::Plugin(problem) => Some(problem.public.user.fallback.clone()),
            Self::Cancelled
            | Self::PolicyDenied(_)
            | Self::UserDeclined(_)
            | Self::CapabilityUnavailable(_)
            | Self::ToolUnavailable(_)
            | Self::UserInputRequired(_) => None,
        }
    }

    pub fn invalid_patch(diagnostic: impl std::fmt::Display) -> Self {
        Self::InvalidPatch(ToolDiagnostic(diagnostic.to_string()))
    }

    pub fn invalid_input(diagnostic: impl std::fmt::Display) -> Self {
        Self::InvalidInput {
            diagnostic: ToolDiagnostic(diagnostic.to_string()),
            fields: Vec::new(),
        }
    }

    pub fn invalid_input_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::invalid_input(agena_failure::diagnostic::format_error_chain(error))
    }

    pub fn invalid_field(
        field: impl AsRef<str>,
        kind: agena_failure::FieldIssueKind,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        Self::InvalidInput {
            diagnostic: ToolDiagnostic(diagnostic.to_string()),
            fields: vec![agena_failure::FieldIssue::new(field, kind)],
        }
    }

    pub fn plugin(diagnostic: impl std::fmt::Display) -> Self {
        Self::from_plugin_error(agena_plugin_host::sdk::PluginError::internal(diagnostic))
    }

    pub fn plugin_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::from_plugin_error(agena_plugin_host::sdk::PluginError::internal_error(error))
    }

    pub fn from_plugin_error(error: agena_plugin_host::sdk::PluginError) -> Self {
        let error_kind = error.kind;
        let public_problem = error
            .diagnostic
            .data
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .and_then(|data| data.get("agena_public_problem"))
            .and_then(serde_json::Value::as_str);
        let proposed_public_detail = error.failure.user.fallback.clone();
        let diagnostic = bounded_plugin_diagnostic(error.diagnostic.message);
        let mut public = if public_problem
            == Some(agena_plugin_host::schema::OUTPUT_CONTRACT_VIOLATION_MARKER)
        {
            agena_plugin_host::schema::output_contract_failure()
        } else if public_problem == Some(agena_plugin_host::sdk::CONFIGURATION_REQUIRED_MARKER) {
            configuration_required_failure()
        } else {
            // Plugin transports are not trusted to choose Failure semantics
            // or model feedback. Rebuild the envelope from the host-owned SDK
            // template and carry across only bounded, sanitised public prose.
            // This retains actionable producer details without allowing a
            // plugin to smuggle arbitrary categories, retry policy, or prompt
            // instructions into the transcript.
            let mut failure =
                *agena_plugin_host::sdk::PluginError::from_kind(error_kind, "").failure;
            if !proposed_public_detail.trim().is_empty() {
                failure.user = agena_failure::UserPresentation::validated(
                    format!("{}-detail", failure.user.key),
                    proposed_public_detail.clone(),
                );
            }
            failure
        };
        public.model = Some(match error_kind {
            agena_plugin_host::sdk::PluginErrorKind::InvalidParams => {
                agena_failure::ModelFeedback::invalid_input()
            }
            agena_plugin_host::sdk::PluginErrorKind::PolicyDenied => {
                agena_failure::ModelFeedback::permission_denied()
            }
            agena_plugin_host::sdk::PluginErrorKind::UserDeclined => {
                agena_failure::ModelFeedback::user_declined()
            }
            agena_plugin_host::sdk::PluginErrorKind::ToolUnavailable
            | agena_plugin_host::sdk::PluginErrorKind::CapabilityUnavailable => {
                agena_failure::ModelFeedback::tool_unavailable()
            }
            _ => agena_failure::ModelFeedback::plugin_failure(),
        });
        // Give the model the scrubbed root cause so it can correct its
        // approach rather than retrying a blank "plugin failed".
        if public.model.is_some() {
            public.model = public
                .model
                .map(|model| model.with_text(proposed_public_detail.clone()));
        }
        Self::Plugin(Box::new(PluginToolFailure {
            public,
            diagnostic: ToolDiagnostic(diagnostic),
        }))
    }

    /// Preserve a known execution problem across a host/plugin gateway. The
    /// marker carries no user text; it selects a fixed safe public Failure at
    /// the receiving boundary while `Display` remains log-only diagnostic.
    pub fn into_plugin_error(self) -> agena_plugin_host::sdk::PluginError {
        use agena_plugin_host::sdk::{CONFIGURATION_REQUIRED_MARKER, PluginError, PluginErrorKind};
        let marker = match &self {
            Self::Plugin(problem)
                if problem.public.code.as_str() == "tool.configuration_required" =>
            {
                Some(CONFIGURATION_REQUIRED_MARKER)
            }
            Self::Plugin(problem)
                if problem.public.code.as_str() == "tool.output_contract_violation" =>
            {
                Some(agena_plugin_host::schema::OUTPUT_CONTRACT_VIOLATION_MARKER)
            }
            _ => None,
        };
        let kind = match self {
            Self::InvalidPatch(_) | Self::InvalidInput { .. } => PluginErrorKind::InvalidParams,
            _ => PluginErrorKind::Internal,
        };
        let error = PluginError::from_kind(kind, self.to_string());
        marker.map_or(error.clone(), |marker| error.with_public_problem(marker))
    }
}

fn configuration_required_failure() -> agena_failure::Failure {
    agena_failure::Failure::new(
        agena_failure::FailureCode::new("tool.configuration_required"),
        agena_failure::FailureCategory::InvalidInput,
        agena_failure::FailureResponsibility::Caller,
        agena_failure::RetryDirective::CorrectInput,
        agena_failure::RecoveryDirective::OpenSettings,
        agena_failure::FailureImpact::RequestRejected,
        agena_failure::UserPresentation::new(
            "tool.configuration_required",
            "This tool needs a model configuration. Provide input.model or configure the tool before retrying.",
        ),
    )
}

fn bounded_plugin_diagnostic(message: String) -> String {
    let mut output = String::with_capacity(message.len().min(16_384));
    for character in message.chars() {
        if output.len() >= 16_384 {
            output.push('…');
            break;
        }
        if character == '\n' || character == '\t' || !character.is_control() {
            output.push(character);
        } else {
            output.push(' ');
        }
    }
    output
}

pub(super) fn tool_summary(registered_tool: &RegisteredTool) -> String {
    if let Some(summary) = registered_tool.summary_text() {
        return summary.to_string();
    }
    format!(
        "Tool `{}`.",
        compact_tool_call_name(registered_tool.canonical_name().as_str())
    )
}

#[derive(Clone)]
pub(super) struct CloudToolAdapterGate {
    pub adapter: Option<agena_domain::AdapterId>,
}

#[derive(Clone)]
/// Executor that runs tools with permissions.
pub struct ToolExecutor {
    pub(super) read_path_authorizer: Option<Arc<ReadPathAuthorizer>>,
    pub(crate) conversation: Option<agena_domain::SessionConversation>,
    pub(super) workspace_root: PathBuf,
    pub(super) principal: ExecutionPrincipal,
    pub(super) allowed_tool_names: Option<std::collections::HashSet<String>>,
    pub(super) model_id: Option<String>,
    /// AI-call execution guard only. Never used to filter tool discovery.
    pub(super) cloud_tool_adapter_gate: Option<CloudToolAdapterGate>,
    /// Trusted session-owned validation and destination binding for media
    /// returned to the current model. This never comes from tool arguments.
    pub(super) media_input_binder: Option<Arc<MediaInputBinder>>,
    /// Opaque plugin capability scope selected from the runtime-only session
    /// carrier. All catalog/permission/execution lookups for this executor use
    /// this exact scope snapshot.
    pub(super) plugin_scope: Option<agena_plugin_host::PluginScopeKey>,
    /// Definition-hook output captured for one scoped request/turn.
    ///
    /// Tool identity, permission, availability, and execution must all see
    /// the same registry snapshot. Without this cache each lookup rebuilt the
    /// catalog and synchronously re-entered async `tool.definition` hooks,
    /// multiplying one slow plugin into many blocking waits per tool call.
    pub(super) definition_catalog: Option<Arc<Vec<RegisteredTool>>>,
    pub(super) monitor_registry: Option<Arc<dyn MonitorService>>,
    pub(super) plugins: Arc<PluginHost>,
    pub(super) scheduler: Option<Arc<agena_scheduler::Scheduler>>,
    pub(super) lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
    pub(super) cancellation_token: Option<tokio_util::sync::CancellationToken>,
    pub(super) permission_inspector: Option<Arc<dyn ExecutionPermissionInspector>>,
}

pub type ReadPathAuthorizer = dyn Fn(&std::path::Path) -> Result<(), ToolError> + Send + Sync;

pub type MediaInputBinder = dyn Fn(agena_domain::AttachmentItem) -> Result<agena_domain::AttachmentItem, ToolError>
    + Send
    + Sync;

/// Runtime-owned extension point for adding execution-time permission checks
/// from trusted state that cannot live in model tool input. Inspectors only
/// add checks; they never remove the static tool/path/network checks.
pub trait ExecutionPermissionInspector: Send + Sync {
    fn additional_checks(
        &self,
        invocation: &ToolInvocation,
        principal: &ExecutionPrincipal,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError>;
}

use super::{
    Arc, AskUserToolInput, Error, ExecutionPrincipal, MonitorService, PathBuf, PluginHost,
    RegisteredTool, ShellError,
};
use agena_tool::PreparedShellCommand;

#[cfg(test)]
mod execution_namespace_tests {
    use super::*;
    use agena_plugin_host::sdk::{PluginKey, ToolContract, ToolDefinition};

    fn registered(namespace: &str, plugin: &str, tool: &str) -> RegisteredTool {
        RegisteredTool::new(
            PluginKey::new(namespace, plugin).unwrap(),
            ToolDefinition {
                name: tool.into(),
                contract: ToolContract {
                    input_schema: serde_json::json!({"type": "object"}),
                    ..Default::default()
                },
                docs: Default::default(),
                runtime: Default::default(),
                tags: vec![],
            },
        )
        .unwrap()
    }

    #[test]
    fn execution_aliases_cannot_claim_runtime_function_names() {
        let reserved = agena_domain::ToolApiFunction::ALL
            .map(|function| function.function_name())
            .into_iter()
            .chain(agena_domain::CONTROL_FUNCTION_NAMES);
        for name in reserved {
            let registered = registered("custom", name, name);
            let canonical = registered.canonical_name();
            let tools = vec![ExecutionTool::from_registered_tool(registered.clone())];
            assert_eq!(execution_tool_names(&tools), [canonical.clone()]);
            assert!(!registered_tool_matches_name(&registered, name));
            assert!(unique_registered_tool_match([registered.clone()], name).is_none());
            assert_eq!(
                unique_registered_tool_match([registered.clone()], &canonical),
                Some(registered),
            );
        }
    }

    #[test]
    fn execution_alias_collisions_keep_distinct_canonical_targets() {
        let tools = vec![
            ExecutionTool::from_registered_tool(registered("one", "fs", "read")),
            ExecutionTool::from_registered_tool(registered("two", "fs", "read")),
            ExecutionTool::from_registered_tool(registered("agena", "shell", "exec")),
        ];
        assert_eq!(
            execution_tool_names(&tools),
            ["one.fs.read", "two.fs.read", "shell.exec"],
        );
        assert!(
            unique_registered_tool_match(
                tools.iter().map(|tool| tool.registered().clone()),
                "fs.read",
            )
            .is_none(),
        );
    }

    #[test]
    fn output_contract_failure_never_becomes_an_automatic_retry() {
        let error = ToolError::from_plugin_error(agena_plugin_host::schema::output_contract_error(
            "$/ok: expected a boolean",
        ));
        let ToolError::Plugin(problem) = &error else {
            panic!("expected a plugin failure");
        };
        assert_eq!(problem.public.retry, agena_failure::RetryDirective::Never);
        assert_eq!(
            problem.public.impact,
            agena_failure::FailureImpact::PartialSuccess
        );
        let roundtrip = ToolError::from_plugin_error(error.into_plugin_error());
        let ToolError::Plugin(problem) = roundtrip else {
            panic!("expected a plugin failure");
        };
        assert_eq!(
            problem.public.code.as_str(),
            "tool.output_contract_violation"
        );
        assert_eq!(problem.public.retry, agena_failure::RetryDirective::Never);
        assert_eq!(
            problem.public.impact,
            agena_failure::FailureImpact::PartialSuccess
        );
    }
}
