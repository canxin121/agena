use std::{path::PathBuf, time::Duration};

use agena_domain::{ComposerDocument, ComposerNode, ExecutionLifecycle, ModelRef};
use agena_runtime::{
    RuntimeBootstrapRequest, SessionCreateRequest, SessionRunOptions, SessionUserRunRequest,
    bootstrap_application_services,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "error,agena::adapter=trace,agena_storage=trace,agena::session::lease=debug",
        ))
        .init();
    let probe_root = PathBuf::from("/tmp/agena-tool-e2e.zrCBcK");
    let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
        workspace_root: Some(PathBuf::from("/Volumes/Rc20/Projects/agena")),
        config_path: None,
        config_override_expressions: Vec::new(),
        database_url: None,
        database_path: Some(probe_root.join("probe-trace.db")),
        scheduler_database_url: None,
        scheduler_database_path: Some(probe_root.join("probe-trace-scheduler.db")),
        initialize_schema: true,
        tracing_reload_handle: None,
    })
    .await?;
    let services = runtime.application_services();
    let commands = services
        .execution_commands
        .clone()
        .ok_or_else(|| anyhow::anyhow!("execution commands unavailable"))?;
    let control = services
        .execution_control
        .clone()
        .ok_or_else(|| anyhow::anyhow!("execution control unavailable"))?;
    let store = services
        .session_store
        .clone()
        .ok_or_else(|| anyhow::anyhow!("session store unavailable"))?;
    let queries = services
        .session_queries
        .clone()
        .ok_or_else(|| anyhow::anyhow!("session queries unavailable"))?;

    let created = commands
        .create_session(SessionCreateRequest {
            title: "real 3-round tools loop probe".to_owned(),
            parent_session_id: None,
        })
        .await?;
    let session_id = created.session_id;
    let accepted = commands
        .submit_user_run(SessionUserRunRequest::new(
            session_id,
            SessionRunOptions {
                model: ModelRef::new("cpa", "deepseek-v4-flash"),
                thinking_mode: Some("max".to_owned()),
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: Some(
                    "Execute this exact 3-round tool sequence using the Tool API.\n\
                     ROUND 1: call tools_search twice in parallel (query \"session model\" and query \"session rename\").\n\
                     ROUND 2: after both ROUND 1 results, call tools_help twice in parallel (tool \"session.model\" and tool \"session.rename\").\n\
                     ROUND 3: after both ROUND 2 results, call no further tool and answer exactly REAL_TOOL_LOOP_OK session.model session.rename"
                        .to_owned(),
                ),
                temperature: Some(0.0),
                max_output_tokens: Some(1024),
            },
            ComposerDocument(vec![ComposerNode::Text {
                text: "Start the 3-round sequence now."
                    .to_owned(),
            }]),
        ))
        .await?;
    anyhow::ensure!(accepted.receipt.is_some(), "execution was not accepted");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    let mut observed_active = false;
    let poll = std::env::var("POLL_LIVE").is_ok();
    let mut last_key = String::new();
    loop {
        if poll {
            // Poll the EXACT query surface the TUI refresh path uses
            // (`list_projected_runs` behind `get_session_state`), plus the
            // durable watermark, plus the raw facade view. Report how the
            // run-marker set and streaming deltas evolve over the run.
            let t0 = tokio::time::Instant::now();
            if let Ok(projected) = queries.list_projected_runs(session_id).await {
                let elapsed_ms = t0.elapsed().as_millis();
                let runs = projected
                    .iter()
                    .map(|run| {
                        let think_units = run
                            .parts
                            .iter()
                            .filter(|part| part.kind == "think")
                            .filter_map(|part| part.content.as_ref())
                            .filter_map(|c| c.get("summary"))
                            .filter_map(|s| s.as_array())
                            .map(|a| a.len())
                            .sum::<usize>();
                        let text_len = run
                            .parts
                            .iter()
                            .filter(|part| part.kind == "text")
                            .filter_map(|part| part.content.as_ref())
                            .filter_map(|c| c.get("text"))
                            .filter_map(serde_json::Value::as_str)
                            .map(str::len)
                            .sum::<usize>();
                        let tool_calls = run
                            .parts
                            .iter()
                            .filter(|part| part.kind == "tool_call")
                            .count();
                        let results = run
                            .parts
                            .iter()
                            .filter(|part| part.kind == "tool_result")
                            .count();
                        format!(
                            "run{}:{:?}:{:?} t={} h={} c={} r={}",
                            run.id, run.role, run.state, think_units, text_len, tool_calls, results
                        )
                    })
                    .collect::<Vec<_>>();
                let watermark = queries.latest_event_seq(session_id).await.ok().flatten();
                let live_view = store.load(session_id).await.ok();
                let overlay_think = live_view
                    .map(|v| {
                        v.parts
                            .iter()
                            .filter(|part| part.kind == "think")
                            .filter_map(|part| part.content.get("summary"))
                            .filter_map(|s| s.as_array())
                            .map(|a| a.len())
                            .sum::<usize>()
                    })
                    .unwrap_or(0);
                let key = format!(
                    "runs=[{}] wm={:?} overlay_think={} load_ms={}",
                    runs.join(" | "),
                    watermark,
                    overlay_think,
                    elapsed_ms
                );
                if key != last_key {
                    eprintln!("[tui] {key}");
                    last_key = key;
                }
            }
        }
        match control.active_execution(session_id).await {
            Some(ExecutionLifecycle::Active { phase, .. }) => {
                observed_active = true;
                eprintln!("phase={phase:?}");
            }
            Some(ExecutionLifecycle::Terminal { outcome, .. }) => {
                eprintln!("terminal={outcome:?}");
                break;
            }
            None if observed_active => break,
            None => {}
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "execution did not finish within 120 seconds"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let view = store.load(session_id).await?;
    let tool_parts = view
        .parts
        .iter()
        .filter(|part| part.kind == "tool_call")
        .collect::<Vec<_>>();
    let final_text = view
        .parts
        .iter()
        .rev()
        .find(|part| part.kind == "text" && format!("{:?}", part.role) == "Assistant")
        .and_then(|part| part.content.get("text"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let error_parts = view
        .parts
        .iter()
        .filter(|part| part.kind == "error")
        .collect::<Vec<_>>();
    let run_markers = view
        .parts
        .iter()
        .filter(|part| part.kind == "run")
        .map(|part| {
            serde_json::json!({
                "part_id": part.part_id,
                "role": format!("{:?}", part.role),
                "state": format!("{:?}", part.state),
            })
        })
        .collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "session_id": session_id,
            "tool_count": tool_parts.len(),
            "tool_names": tool_parts.iter().filter_map(|p| p.content.get("name")).collect::<Vec<_>>(),
            "final_text": final_text,
            "error_parts": error_parts.iter().map(|part| &part.content).collect::<Vec<_>>(),
            "run_markers": run_markers,
        }))?
    );

    anyhow::ensure!(tool_parts.len() == 4, "expected four tool calls");
    anyhow::ensure!(
        final_text.contains("REAL_TOOL_LOOP_OK session.model session.rename"),
        "final model response missing success marker"
    );
    anyhow::ensure!(error_parts.is_empty(), "execution persisted an error part");
    // One user message == one run marker (turn-scoped runs): the whole 3-round
    // tool loop must persist under a single ASSISTANT run marker (the user's
    // message is its own marker), not one assistant marker per provider
    // round-trip.
    let assistant_markers = run_markers
        .iter()
        .filter(|marker| marker["role"] == "Assistant")
        .collect::<Vec<_>>();
    anyhow::ensure!(
        assistant_markers.len() == 1,
        "expected exactly one assistant run marker, got {}: {run_markers:?}",
        assistant_markers.len()
    );
    anyhow::ensure!(
        run_markers.len() == 2,
        "expected the user message + one assistant reply (2 markers), got {}: {run_markers:?}",
        run_markers.len()
    );
    runtime.shutdown();
    Ok(())
}
