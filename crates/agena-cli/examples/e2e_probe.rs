//! Real end-to-end probe against the live runtime and a real provider.
//!
//! Boots the same runtime the CLI boots (config from `~/agena/agena.json`,
//! database from `AGENA_DATABASE_PATH` / `AGENA_SCHEDULER_DATABASE_PATH`),
//! submits a short user message to a fresh session, waits for the run to
//! reach quiescence, and prints the full projected tool-call sequence.
//!
//! This is the harness used to verify model behavior end-to-end (e.g. that
//! the model goes straight to `tools_help`/`tools_call` for `context.status`
//! instead of calling `tools_search`), which CLI-only unit assertions cannot
//! prove.
//!
//! Usage:
//!   AGENA_DATABASE_PATH=/tmp/agena-e2e/agena.db \
//!   AGENA_SCHEDULER_DATABASE_PATH=/tmp/agena-e2e/scheduler.db \
//!   cargo run -p agena-cli --example e2e_probe -- "你是谁？"

use std::time::Duration;

use agena_domain::{ComposerDocument, ComposerNode};
use agena_runtime::{
    RuntimeBootstrapRequest, SessionCreateRequest, SessionRunOptions, SessionUserRunRequest,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "你是谁？".to_owned());

    let request = RuntimeBootstrapRequest {
        workspace_root: None,
        config_override_expressions: Vec::new(),
        database_url: None,
        database_path: std::env::var("AGENA_DATABASE_PATH")
            .ok()
            .map(std::path::PathBuf::from),
        scheduler_database_url: None,
        scheduler_database_path: std::env::var("AGENA_SCHEDULER_DATABASE_PATH")
            .ok()
            .map(std::path::PathBuf::from),
        initialize_schema: true,
        tracing_reload_handle: None,
    };
    let runtime = agena_runtime::bootstrap_application_services(request).await?;
    let services = runtime.application_services();
    let providers = services.provider_catalog.clone();
    let queries = services
        .session_queries
        .clone()
        .expect("session queries present");
    let commands = services
        .execution_commands
        .clone()
        .expect("execution commands present");

    let model = providers
        .default_model()?
        .ok_or("no default model configured")?;
    eprintln!("probe: model = {model:?}");

    let options = SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: None,
        max_output_tokens: None,
    };

    let created = commands
        .create_session(SessionCreateRequest {
            title: format!("e2e probe: {prompt}"),
            parent_session_id: None,
        })
        .await?;
    let session_id = created.session_id;
    eprintln!("probe: session {session_id} created");

    commands
        .submit_user_run(SessionUserRunRequest::new(
            session_id,
            options,
            ComposerDocument(vec![ComposerNode::Text { text: prompt }]),
        ))
        .await?;
    eprintln!("probe: user run submitted; waiting for quiescence...");

    // Poll until the assistant run marker reaches a terminal state. A freshly
    // submitted session is momentarily Quiescent before the spawned execution
    // task persists anything, and workflow_state does not reflect an in-flight
    // model call, so we wait on the run marker itself.
    let started = std::time::Instant::now();
    loop {
        let runs = queries.list_projected_runs(session_id, true).await?;
        let assistant_run = runs
            .iter()
            .find(|run| run.role == agena_domain::Role::Assistant);
        let state = assistant_run.map(|run| format!("{:?}", run.state));
        eprintln!("probe: run state -> {state:?}");
        if let Some(ref state) = state {
            let terminal = state == "Completed"
                || state == "Cancelled"
                || state == "Failed"
                || state == "TimedOut"
                || state.ends_with("Denied")
                || state.ends_with("Unavailable")
                || state.ends_with("Aborted")
                || state.ends_with("Interrupted");
            if terminal {
                break;
            }
        }
        if started.elapsed() > Duration::from_secs(240) {
            eprintln!("probe: TIMEOUT waiting for terminal run state (last={state:?})");
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // Dump the projected transcript: every run, its parts, and the tool
    // names + payloads for tool calls.
    let runs = queries.list_projected_runs(session_id, true).await?;
    println!();
    println!("==== TRANSCRIPT (session {session_id}) ====");
    for run in &runs {
        println!(
            "-- run {} role={:?} state={:?} parts={}",
            run.id,
            run.role,
            run.state,
            run.parts.len()
        );
        for part in &run.parts {
            match part.detail.as_ref() {
                Some(agena_runtime::SessionProjectedPartDetail::Text { text, .. }) => {
                    let text = text.trim();
                    if !text.is_empty() {
                        println!("   text: {text}");
                    }
                }
                Some(agena_runtime::SessionProjectedPartDetail::Reasoning { summary, .. }) => {
                    println!("   think: {}", summary.join(" "));
                }
                _ => {}
            }
            let name = part.name.as_deref().unwrap_or("");
            match part.kind.as_str() {
                "tool_call" => {
                    let payload = part
                        .content
                        .as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    println!("   tool_call name={name} content={payload}");
                }
                "tool_result" => {
                    let summary = part.summary.as_deref().unwrap_or("");
                    println!("   tool_result name={name} summary={summary}");
                }
                "run" => {
                    let meta = part
                        .content
                        .as_ref()
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    println!("   run-marker name={name} meta={meta}");
                }
                other => {
                    println!("   {other} name={name}");
                }
            }
        }
    }

    runtime.shutdown();
    Ok(())
}
