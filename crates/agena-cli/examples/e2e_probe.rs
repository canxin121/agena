//! Real end-to-end probe against the live runtime and a real provider.
//!
//! Boots the same runtime the CLI boots (config from `~/agena/agena.json`,
//! database from `AGENA_DATABASE_PATH` / `AGENA_SCHEDULER_DATABASE_PATH`),
//! submits a short user message to a fresh session, waits for the run to
//! reach quiescence, and prints the full projected tool-call sequence.
//!
//! This is the harness used to verify model behavior end-to-end (e.g. that
//! the model goes straight to `tools_help`/`tools_call` for `session.model`
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
    let execution_control = services
        .execution_control
        .clone()
        .expect("execution control present");
    let session_store = services
        .session_store
        .clone()
        .expect("session store present");

    let model = providers
        .default_model()?
        .ok_or("no default model configured")?;
    eprintln!("probe: model = {model:?}");

    // Optional deterministic mode: AGENA_PROBE_TEMPERATURE=0 pins sampling to
    // the model's lowest temperature, which suppresses sampling variance and
    // lets a few runs distinguish prompt behavior from model flakiness.
    let temperature = std::env::var("AGENA_PROBE_TEMPERATURE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok());
    eprintln!("probe: temperature = {temperature:?}");

    let options = SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature,
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

    // Wait for actual quiescence, not for one arbitrarily selected assistant
    // marker. A Runtime notification creates a fresh assistant response run;
    // treating the first completed assistant run as "the session finished"
    // races runtime shutdown against that response. Conversely, a freshly
    // submitted command is momentarily inactive before its spawned execution
    // registers, so require evidence that execution began before accepting a
    // quiescent snapshot.
    let started = std::time::Instant::now();
    let mut observed_execution = false;
    let mut last_snapshot = None;
    loop {
        let runs = queries.list_projected_runs(session_id, true).await?;
        let assistant_seen = runs
            .iter()
            .any(|run| run.role == agena_domain::Role::Assistant);
        let all_runs_terminal = !runs.is_empty() && runs.iter().all(|run| run.state.is_terminal());
        let active_execution = execution_control.active_execution(session_id).await;
        observed_execution |= active_execution.is_some() || assistant_seen;
        let active_background = session_store
            .active_background_operations(None, 1_024)
            .await?
            .into_iter()
            .filter(|operation| operation.session_id == session_id)
            .count();
        let pending_deliveries = session_store
            .pending_background_deliveries(1_024)
            .await?
            .into_iter()
            .filter(|delivery| delivery.session_id == session_id)
            .count();
        let session_state = session_store.session_state(session_id).await?;
        let snapshot = format!(
            "active={} runs={} all_terminal={} background={} deliveries={} session={:?}",
            active_execution.is_some(),
            runs.len(),
            all_runs_terminal,
            active_background,
            pending_deliveries,
            session_state.state,
        );
        if last_snapshot.as_deref() != Some(snapshot.as_str()) {
            eprintln!("probe: {snapshot}");
            last_snapshot = Some(snapshot);
        }
        if observed_execution
            && active_execution.is_none()
            && all_runs_terminal
            && active_background == 0
            && pending_deliveries == 0
            && session_state.state == agena_storage::store::SessionState::Ready
        {
            break;
        }
        if started.elapsed() > Duration::from_secs(240) {
            eprintln!(
                "probe: TIMEOUT waiting for true quiescence (last={})",
                last_snapshot.as_deref().unwrap_or("no snapshot")
            );
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
