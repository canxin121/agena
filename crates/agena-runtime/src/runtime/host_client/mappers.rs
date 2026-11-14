use agena_domain::PermissionDecision;
use agena_plugin_host::sdk::host_api::PermissionQuery;

pub(super) fn host_unavailable(message: impl Into<String>) -> PluginError {
    PluginError::from_kind(
        agena_plugin_host::sdk::PluginErrorKind::HostUnavailable,
        message.into(),
    )
}

pub(super) fn tool_execution_to_invoke_output(
    execution: crate::tool::ToolInvocationExecution,
) -> ToolInvokeOutput {
    let summary = execution.summary();
    ToolInvokeOutput {
        title: summary.title,
        summary: summary.summary,
        output_text: summary.output_text,
        payload: summary.payload,
        metadata: summary.metadata.into_iter().collect(),
        attachments: execution.view.attachments,
    }
}

pub(super) fn map_storage_error(err: PluginStorageError) -> PluginError {
    use agena_plugin_host::sdk::PluginErrorKind;
    match err {
        PluginStorageError::MissingSessionId
        | PluginStorageError::MissingWorkspaceRoot
        | PluginStorageError::EmptyNamespace
        | PluginStorageError::EmptyKey
        | PluginStorageError::Data(_) => PluginError::invalid_params_error(&err),
        PluginStorageError::SecretUnavailable(_) => {
            PluginError::from_kind(PluginErrorKind::HostUnavailable, err)
        }
        PluginStorageError::Io(_) | PluginStorageError::Secret(_) => {
            PluginError::internal_error(&err)
        }
    }
}

pub(super) fn render_tool_descriptor(
    tool: agena_plugin_host::registry::RegisteredTool,
) -> ToolDescriptor {
    let brief_summary = tool.summary_text().map(ToString::to_string);
    let mut help_parts = Vec::new();
    if let Some(before_help) = tool.before_help_text() {
        help_parts.push(before_help.to_string());
    }
    if let Some(help) = tool.help_text() {
        help_parts.push(help.to_string());
    }
    if let Some(after_help) = tool.after_help_text() {
        help_parts.push(after_help.to_string());
    }
    let help = (!help_parts.is_empty()).then(|| help_parts.join("\n\n"));
    // Fixed presentation: list/search always show the brief summary and
    // tools_help always returns the full help text.
    let summary = brief_summary;
    let input_schema = Some(tool.input_schema());
    ToolDescriptor {
        name: crate::tool::compact_tool_call_name(tool.canonical_name().as_str()),
        plugin_id: Some(tool.plugin_key().to_string()),
        summary,
        help,
        input_schema,
    }
}

pub(super) fn join_monitor_command(command: &[String]) -> Result<String, PluginError> {
    if command.is_empty() {
        return Err(PluginError::invalid_params(
            "monitor_start requires at least one command token",
        ));
    }
    Ok(command.join(" "))
}

pub(super) fn ask_user_tool_input(req: AskUserRequest) -> Result<AskUserToolInput, PluginError> {
    if !req.questions.is_empty() {
        let questions = req
            .questions
            .into_iter()
            .map(|question| UserInputQuestion {
                header: question.header,
                question: question.question,
                options: question
                    .options
                    .into_iter()
                    .map(|option| UserInputOption {
                        custom: option.custom,
                        label: option.label,
                        description: option.description,
                    })
                    .collect(),
                multiple: question.multiple,
                allow_custom: question.allow_custom,
            })
            .collect();
        let input = AskUserToolInput {
            title: req.title,
            kind: req.kind,
            body_markdown: req.body_markdown,
            auto_resolution_ms: req.auto_resolution_ms,
            questions,
        };
        return AskUserToolInput::parse_input(
            serde_json::to_value(input)
                .map_err(|error| PluginError::invalid_params_error(&error))?,
        );
    }

    let options = req
        .options
        .into_iter()
        .map(|label| UserInputOption {
            custom: false,
            label,
            description: String::new(),
        })
        .collect();
    let input = AskUserToolInput {
        title: req.title,
        kind: req.kind,
        body_markdown: req.body_markdown,
        auto_resolution_ms: req.auto_resolution_ms,
        questions: vec![UserInputQuestion {
            header: String::new(),
            question: req.prompt,
            options,
            multiple: false,
            allow_custom: req.allow_free_text,
        }],
    };
    AskUserToolInput::parse_input(
        serde_json::to_value(input).map_err(|error| PluginError::invalid_params_error(&error))?,
    )
}

/// Project one live session onto the shared wire shape.
///
/// This is the same `SessionResource` the HTTP surface returns, so a plugin
/// and a browser client read one session identically.
pub(super) fn session_resource_from_session(
    session: &crate::session::Session,
) -> agena_session_api::resource::SessionResource {
    agena_session_api::resource::SessionResource {
        id: session.id,
        parent_id: session.parent_id,
        depth: session.depth,
        root_id: session.root_id,
        workspace_id: session.workspace_id,
        title: session.title.clone(),
        owner_bot_id: None,
        owner_surface: None,
        favorite: session.favorite,
        pinned: session.pinned,
        version: session.version,
        relation_kind: session.relation_kind,
        lifecycle_state: session.lifecycle_state,
        state: agena_session_api::resource::SessionState::default(),
        source_cutoff_seq_global: None,
        source_message_id: None,
        is_subagent: session.relation_kind.is_subagent(),
        agent_name: session.agent_name.clone(),
        agent_status: session.agent_status,
        created_at: session.created_at,
        updated_at: session.updated_at,
        message_count: session.message_count,
        child_session_count: 0,
        last_message_at: None,
    }
}

pub(super) fn host_status_to_sdk(
    status: agena_plugin_host::status::PluginStatus,
) -> HostPluginStatus {
    HostPluginStatus {
        plugin_id: status.plugin_id,
        kind: status.kind.to_string(),
        state: status.state.to_string(),
        pid: status.pid,
        restart_count: status.restart_count,
        last_exit_code: status.last_exit_code,
        last_restart_at_ms: status.last_restart_at_ms,
        last_failure: status.last_failure,
    }
}

use super::{
    AskUserRequest, AskUserToolInput, HostPluginStatus, PluginError, PluginStorageError,
    ToolDescriptor, ToolInvokeOutput, UserInputOption, UserInputQuestion,
};
use agena_domain::{ProcessStatus, ProcessStream};
use agena_plugin_sdk::ToolInput;

/// Project a policy decision onto the read-only answer a permission query
/// returns. `Auto` reads as `Ask`: automatic approval is not an approval, and
/// a query cannot wait for the classifier or for a human.
pub(super) fn permission_query(decision: PermissionDecision) -> PermissionQuery {
    match decision {
        PermissionDecision::Allow => PermissionQuery::Allow {},
        PermissionDecision::Deny { reason } => PermissionQuery::Deny { reason },
        PermissionDecision::Ask { reason } | PermissionDecision::Auto { reason } => {
            PermissionQuery::Ask { reason }
        }
    }
}
