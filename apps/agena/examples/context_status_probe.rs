use std::{path::PathBuf, time::Duration};

use agena_domain::{ComposerDocument, ComposerNode, StructuredObject, ToolInvocation};
use agena_runtime::{
    RuntimeBootstrapRequest, SessionCreateRequest, SessionRunOptions, SessionUserRunRequest,
    bootstrap_application_services,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new("error"))
        .init();
    let probe_root = PathBuf::from("/tmp/agena-context-status-probe.rpHqxW");
    let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
        workspace_root: Some(PathBuf::from("/Volumes/Rc20/Projects/agena")),
        config_override_expressions: Vec::new(),
        database_url: None,
        database_path: Some(probe_root.join("probe.db")),
        scheduler_database_url: None,
        scheduler_database_path: Some(probe_root.join("probe-scheduler.db")),
        initialize_schema: true,
        tracing_reload_handle: None,
    })
    .await?;
    let services = runtime.application_services();

    // The context.status tool must be wired in by the bundled context plugin.
    let tool_service = services.tools.clone();
    let tool_names = tool_service
        .available_runtime_tools()
        .await
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        tool_names.iter().any(|name| name == "context.status"),
        "context.status not available; tools = {tool_names:?}"
    );

    let commands = services
        .execution_commands
        .clone()
        .ok_or_else(|| anyhow::anyhow!("execution commands unavailable"))?;
    let session_tools = services
        .tool_execution
        .clone()
        .ok_or_else(|| anyhow::anyhow!("session tool execution unavailable"))?;

    // Create a session with NO model override — the exact broken scenario
    // where `execution.selection` is empty and `context.status` used to
    // return null for every model field.
    let created = commands
        .create_session(SessionCreateRequest {
            title: "context.status default-model probe".to_owned(),
            parent_session_id: None,
        })
        .await?;
    let session_id = created.session_id;

    // Submitting a run is required so the session reaches an execution-ready
    // state (default model resolution) before the tool runs.
    let accepted = commands
        .submit_user_run(SessionUserRunRequest::new(
            session_id,
            SessionRunOptions {
                model: agena_domain::ModelRef::new("cpa", "deepseek-v4-flash"),
                thinking_mode: Some("max".to_owned()),
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: Some("do not run any tools; just reply OK".to_owned()),
                temperature: Some(0.0),
                max_output_tokens: Some(64),
            },
            ComposerDocument(vec![ComposerNode::Text {
                text: "Reply OK.".to_owned(),
            }]),
        ))
        .await?;
    anyhow::ensure!(accepted.receipt.is_some(), "execution was not accepted");

    let outcome = session_tools
        .execute_session_tool(
            session_id,
            ToolInvocation::new(
                "context.status",
                StructuredObject::try_from(serde_json::json!({})).unwrap(),
            ),
        )
        .await
        .map_err(|error| anyhow::anyhow!("tool execution failed: {error}"))?;
    let summary = outcome.into_summary();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "title": summary.title,
            "output_text": summary.output_text,
            "payload": summary.payload,
        }))?
    );

    let payload = summary
        .payload
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("context.status returned no payload"))?;
    let provider = payload
        .get("model_provider_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty());
    let model = payload
        .get("model_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty());
    anyhow::ensure!(
        provider.is_some() && model.is_some(),
        "context.status returned null/empty model identity; payload = {payload}"
    );
    println!(
        "OK effective model: {} / {}",
        provider.unwrap(),
        model.unwrap()
    );

    runtime.shutdown();
    let _ = Duration::ZERO;
    Ok(())
}
