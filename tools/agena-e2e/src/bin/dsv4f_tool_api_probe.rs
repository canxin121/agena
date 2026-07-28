//! Real-provider Tool API regression probe for Cline dsv4f.
//!
//! This intentionally drives the public session path instead of invoking a
//! plugin body directly. It gives the model only the Tool API functions, waits
//! for an inner-tool permission request, replies from the same runtime, and
//! then verifies the original model run completes.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use agena_domain::{
    ModelRef, PermissionAction, PermissionConfig, PermissionMode, PermissionReply,
    PermissionReplyKind, PermissionRequest, ToolPermissionConfig,
};
use agena_runtime::{
    RuntimeBootstrapRequest, SessionCreateRequest, SessionPermissionReplyRequest,
    SessionProjectedPartDetail, SessionQueryService, SessionRunOptions, SessionUserMessagePart,
    SessionUserMessageRequest,
};
use anyhow::{Context, bail};
use clap::Parser;

const DEFAULT_MODEL: &str = "cline/cline-pass/deepseek-v4-flash";
const WEB_FETCH_TOOL_KEY: &str = "agena.web.fetch";

#[derive(Debug, Parser)]
#[command(about = "Run the real Cline dsv4f Tool API permission regression probe")]
struct Args {
    /// Provider/model reference accepted by `agena exec`.
    #[arg(long, default_value = DEFAULT_MODEL)]
    model: String,

    /// Workspace used for the session. The database is always in memory.
    #[arg(long, default_value = ".")]
    workspace: PathBuf,

    /// Maximum wait for the model to create its inner permission request.
    #[arg(long, default_value_t = 60)]
    permission_timeout_secs: u64,
}

fn main() -> anyhow::Result<()> {
    agena_runtime::build_app_runtime()
        .context("build probe Tokio runtime")?
        .block_on(async_main())
}

async fn async_main() -> anyhow::Result<()> {
    let args = Args::parse();
    let workspace = args
        .workspace
        .canonicalize()
        .with_context(|| format!("resolve workspace {}", args.workspace.display()))?;
    let model = args
        .model
        .parse::<ModelRef>()
        .with_context(|| format!("parse model reference `{}`", args.model))?;

    let runtime = agena_runtime::bootstrap_application_services(RuntimeBootstrapRequest {
        workspace_root: Some(workspace),
        config_override_expressions: Vec::new(),
        database_url: Some("sqlite::memory:".to_string()),
        initialize_schema: true,
        tracing_reload_handle: None,
        ..Default::default()
    })
    .await
    .context("start isolated probe runtime")?;
    let services = runtime.application_services();
    let commands = services
        .execution_commands
        .context("runtime does not provide session commands")?;
    let queries = services
        .session_queries
        .context("runtime does not provide session queries")?;

    let session = commands
        .create_session(SessionCreateRequest {
            title: "Cline dsv4f Tool API permission probe".to_string(),
            parent_session_id: None,
        })
        .await
        .context("create probe session")?;
    commands
        .set_session_allowed_tools(
            session.session_id,
            vec![
                "agena.tools.help".to_string(),
                "agena.tools.call".to_string(),
                WEB_FETCH_TOOL_KEY.to_string(),
            ],
        )
        .await
        .context("set Tool API allowlist")?;
    commands
        .set_session_permission(
            session.session_id,
            PermissionConfig {
                tools: Some(ToolPermissionConfig {
                    names: BTreeMap::from([
                        ("agena.tools.help".to_string(), PermissionMode::Allow),
                        ("agena.tools.call".to_string(), PermissionMode::Allow),
                    ]),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .context("allow the outer Tool API operations while preserving inner Ask policy")?;

    let options = SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: Some(0.0),
        max_output_tokens: Some(768),
        agent_profile: None,
    };
    let prompt = concat!(
        "This is a strict Tool API permission regression test. ",
        "Call tools_help exactly once with {\"tool\":\"web.fetch\"}. ",
        "Then call tools_call exactly once with ",
        "{\"tool\":\"web.fetch\",\"input\":{\"url\":\"https://example.com\",\"use_cache\":false}}. ",
        "Do not call any other tool. Once the tool succeeds, reply exactly WEB_FETCH_PERMISSION_OK."
    );

    let run_commands = Arc::clone(&commands);
    let run_options = options.clone();
    let run = tokio::spawn(async move {
        run_commands
            .submit_user_message(SessionUserMessageRequest::new(
                session.session_id,
                run_options,
                vec![SessionUserMessagePart::Text(agena_domain::TextPart {
                    text: prompt.to_owned(),
                    synthetic: false,
                })],
            ))
            .await
    });

    let permission = wait_for_permission(
        queries.as_ref(),
        session.session_id,
        Duration::from_secs(args.permission_timeout_secs),
    )
    .await?;
    if !permission.request_id.starts_with("host-permission:") {
        bail!(
            "expected a host-invoked permission request, received `{}`",
            permission.request_id
        );
    }
    if !permission
        .requested_actions
        .iter()
        .any(|action| matches!(action, PermissionAction::NetworkAccess { .. }))
    {
        bail!(
            "web.fetch host permission did not include its requested network action: {:?}",
            permission.requested_actions
        );
    }
    commands
        .reply_permission(SessionPermissionReplyRequest::new(
            session.session_id,
            options,
            PermissionReply {
                request_id: permission.request_id,
                kind: PermissionReplyKind::AllowOnce,
                reason: Some("dsv4f Tool API probe approval".to_string()),
                scope: None,
            },
            Some("dsv4f_tool_api_probe".to_string()),
        ))
        .await
        .context("approve inner web.fetch permission")?;

    tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .context("model run did not finish after permission approval")?
        .context("join model run")?
        .context("complete model run")?;
    let presentation = queries
        .session_presentation(session.session_id)
        .await
        .context("load completed probe session")?;
    if matches!(
        presentation.workflow_state,
        agena_domain::WorkflowState::Blocked
    ) {
        bail!("session remained blocked after AllowOnce");
    }
    let messages = queries
        .list_projected_messages(session.session_id, true)
        .await
        .context("load completed probe transcript")?;
    assert_tool_api_trace(&messages, "web.fetch")?;
    let transcript = messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.detail.as_ref() {
            Some(SessionProjectedPartDetail::Text { text, .. }) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !transcript.contains("WEB_FETCH_PERMISSION_OK") {
        bail!("model did not report the expected terminal marker:\n{transcript}");
    }

    println!(
        "{{\"ok\":true,\"session_id\":{},\"execution_tool\":\"web.fetch\",\"approval\":\"allow_once\"}}",
        session.session_id
    );
    runtime.shutdown();
    Ok(())
}

fn assert_tool_api_trace(
    messages: &[agena_runtime::SessionProjectedMessage],
    tool_name: &str,
) -> anyhow::Result<()> {
    let operations = messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.detail.as_ref() {
            Some(SessionProjectedPartDetail::Operation(operation)) => Some((
                operation.invocation.name.as_str(),
                Some(serde_json::Value::from(operation.invocation.input.clone())),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    let help_count = operations
        .iter()
        .filter(|(name, _)| *name == "agena.tools.help")
        .count();
    if help_count != 1 {
        bail!("expected exactly one agena.tools.help operation, found {help_count}");
    }
    let call_inputs = operations
        .iter()
        .filter(|(name, _)| *name == "agena.tools.call")
        .filter_map(|(_, input)| input.as_ref())
        .collect::<Vec<_>>();
    if call_inputs.len() != 1 {
        bail!(
            "expected exactly one agena.tools.call operation, found {}",
            call_inputs.len()
        );
    }
    if call_inputs[0]
        .get("tool")
        .and_then(serde_json::Value::as_str)
        != Some(tool_name)
    {
        bail!(
            "Tool API call selected execution tool `{}` instead of `{tool_name}`",
            call_inputs[0]
                .get("tool")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<missing>")
        );
    }
    Ok(())
}

async fn wait_for_permission(
    queries: &dyn SessionQueryService,
    session_id: i64,
    timeout: Duration,
) -> anyhow::Result<PermissionRequest> {
    tokio::time::timeout(timeout, async {
        loop {
            let pending = queries
                .pending_interactive_requests(session_id)
                .await
                .context("load session while waiting for permission")?;
            if let Some(request) = pending
                .into_iter()
                .find_map(|context| context.request.as_permission().cloned())
            {
                return Ok(request);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("timed out waiting for a permission request")?
}
