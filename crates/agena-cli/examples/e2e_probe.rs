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
//!   AGENA_PROBE_MODEL=provider/explicit-model \
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
        config_path: None,
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

    let model_target = std::env::var("AGENA_PROBE_MODEL").map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "AGENA_PROBE_MODEL is required and must name an explicit provider/model route",
        )
    })?;
    let model = providers.resolve_model_target(model_target.as_str(), None)?;
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
        let page = queries.read_part_window(session_id, None, 256).await?;
        let runs = page
            .parts
            .iter()
            .filter(|part| part.kind == "run")
            .collect::<Vec<_>>();
        let assistant_seen = runs
            .iter()
            .any(|run| run.role == agena_storage::store::PartRole::Assistant);
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

    // Read canonical factual windows and resolve source-backed bodies explicitly.
    let mut before = None;
    let mut parts = Vec::new();
    loop {
        let page = queries.read_part_window(session_id, before, 256).await?;
        before = page
            .parts
            .last()
            .map(|part| agena_storage::store::PartCursor {
                created_at_ms: part.created_at_ms,
                part_id: part.part_id,
            });
        parts.extend(page.parts);
        if !page.has_more {
            break;
        }
    }
    parts.sort_by_key(|part| (part.created_at_ms, part.part_id));
    println!("TRANSCRIPT (session {session_id})");
    for part in parts {
        println!(
            "-- part {} run={:?} role={:?} state={:?} kind={}",
            part.part_id, part.run_id, part.role, part.state, part.kind
        );
        println!("   facts: {}", part.content);
        if let Some(resources) = part
            .content
            .get("resources")
            .and_then(serde_json::Value::as_array)
        {
            for reference in resources {
                let reference: agena_domain::ContentRef =
                    serde_json::from_value(reference.clone())?;
                let descriptor = session_store
                    .contents()
                    .describe(reference.resource_id)
                    .await?;
                let mut cursor = agena_domain::ContentCursor {
                    sequence: 0,
                    ..descriptor.cursor
                };
                loop {
                    let page = session_store
                        .contents()
                        .read(reference.resource_id, Some(cursor), 64 * 1024)
                        .await?;
                    if page.gap {
                        println!("   [content retention gap]");
                    }
                    for chunk in page.chunks {
                        println!("   content: {:?}", chunk.payload);
                    }
                    cursor = page.next_cursor;
                    if !page.has_more {
                        break;
                    }
                }
            }
        }
    }

    runtime.shutdown();
    Ok(())
}
