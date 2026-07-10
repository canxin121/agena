//! Real-provider gateway regression probe for Cline dsv4f.
//!
//! This intentionally drives the public session path instead of invoking a
//! plugin body directly. It gives the model only the gateway functions, waits
//! for an inner-tool permission request, replies from the same runtime, and
//! then verifies the original model run completes.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use agena::{
    agent::{PermissionConfig, ToolPermissionConfig},
    config::LoadConfigRequest,
    message::{PartContent, PendingInteractiveRequest},
    model::ModelRef,
    permission::{PermissionMode, PermissionReply, PermissionReplyKind},
    runtime::{AgenaRuntime, AgenaRuntimeConfig},
    session::{
        SessionCreateRequest, SessionManager, SessionPermissionReplyRequest, SessionRunOptions,
        SessionUserMessageRequest,
    },
};
use anyhow::{Context, bail};
use clap::Parser;

const DEFAULT_MODEL: &str = "cline/cline-pass/deepseek-v4-flash";
const WEB_FETCH_TARGET: &str = "agena.web.fetch";

#[derive(Debug, Parser)]
#[command(about = "Run the real Cline dsv4f gateway permission regression probe")]
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
    agena::runtime::build_app_runtime()
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

    let runtime = AgenaRuntime::new(AgenaRuntimeConfig {
        load_request: LoadConfigRequest {
            overrides: Vec::new(),
            workspace_root: Some(workspace.clone()),
        },
        workspace_root: Some(workspace),
        database_connection: None,
        database_url: Some("sqlite::memory:".to_string()),
        auto_migrate: true,
        tracing_reload_handle: None,
    })
    .await
    .context("start isolated probe runtime")?;
    let manager = runtime
        .session_manager()
        .context("runtime does not provide a session manager")?;

    let session = manager
        .create_session(SessionCreateRequest {
            title: "Cline dsv4f gateway permission probe".to_string(),
            parent_session_id: None,
        })
        .await
        .context("create probe session")?;
    manager
        .set_session_allowed_tools(
            session.id,
            vec![
                "agena.tools.help".to_string(),
                "agena.tools.call".to_string(),
                WEB_FETCH_TARGET.to_string(),
            ],
        )
        .await
        .context("set gateway allowlist")?;
    manager
        .set_session_permission(
            session.id,
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
        .context("allow the outer gateway operations while preserving inner Ask policy")?;

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
        "This is a strict plugin-gateway permission regression test. ",
        "Call tools_help exactly once with {\"tool\":\"web.fetch\"}. ",
        "Then call tools_call exactly once with ",
        "{\"tool\":\"web.fetch\",\"input\":{\"url\":\"https://example.com\",\"use_cache\":false}}. ",
        "Do not call any other tool. Once the tool succeeds, reply exactly WEB_FETCH_PERMISSION_OK."
    );

    let run_manager = Arc::clone(&manager);
    let run_options = options.clone();
    let run = tokio::spawn(async move {
        run_manager
            .submit_user_message(SessionUserMessageRequest::new(
                session.id,
                run_options,
                vec![PartContent::text(prompt)],
            ))
            .await
    });

    let permission = wait_for_permission(
        manager.as_ref(),
        session.id,
        Duration::from_secs(args.permission_timeout_secs),
    )
    .await?;
    if !permission.request_id.starts_with("host-permission:") {
        bail!(
            "expected a host-invoked permission request, received `{}`",
            permission.request_id
        );
    }
    if !permission.requested_actions.iter().any(|action| {
        matches!(
            action,
            agena::permission::PermissionAction::NetworkAccess { .. }
        )
    }) {
        bail!(
            "web.fetch host permission did not include its requested network action: {:?}",
            permission.requested_actions
        );
    }
    manager
        .reply_permission(SessionPermissionReplyRequest::new(
            session.id,
            options,
            PermissionReply {
                request_id: permission.request_id,
                kind: PermissionReplyKind::AllowOnce,
                reason: Some("dsv4f gateway probe approval".to_string()),
                scope: None,
            },
            Some("dsv4f_gateway_probe".to_string()),
        ))
        .await
        .context("approve inner web.fetch permission")?;

    let completed = tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .context("model run did not finish after permission approval")?
        .context("join model run")?
        .context("complete model run")?;
    if completed.blocked() {
        bail!("session remained blocked after AllowOnce");
    }
    assert_gateway_trace(&completed, "web.fetch")?;
    let transcript = completed
        .messages
        .iter()
        .map(|message| message.as_text_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    if !transcript.contains("WEB_FETCH_PERMISSION_OK") {
        bail!("model did not report the expected terminal marker:\n{transcript}");
    }

    println!(
        "{{\"ok\":true,\"session_id\":{},\"target\":\"web.fetch\",\"approval\":\"allow_once\"}}",
        completed.id
    );
    runtime.shutdown();
    Ok(())
}

fn assert_gateway_trace(session: &agena::session::Session, target: &str) -> anyhow::Result<()> {
    let operations = session
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.content.as_ref() {
            Some(PartContent::Operation(operation)) => Some((
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
        != Some(target)
    {
        bail!(
            "gateway call targeted `{}` instead of `{target}`",
            call_inputs[0]
                .get("tool")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<missing>")
        );
    }
    Ok(())
}

async fn wait_for_permission(
    manager: &SessionManager,
    session_id: i64,
    timeout: Duration,
) -> anyhow::Result<agena::permission::PermissionRequest> {
    tokio::time::timeout(timeout, async {
        loop {
            let session = manager
                .get_session(session_id)
                .await
                .context("load session while waiting for permission")?;
            if let Some(PendingInteractiveRequest::Permission { request }) =
                session.pending_interactive_requests().into_iter().next()
            {
                return Ok(request);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("timed out waiting for a permission request")?
}
