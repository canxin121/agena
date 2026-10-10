//! Live gateway regression probe. All session/scheduler data is temporary.
//! Example: cargo run --locked -p agena --example model_tool_mode_probe --
//! --config /path/to/agena.json --provider gateway --adapter openai_responses
//! --model deepseek-v4.1-flash --thinking max --scenario tool-loop

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use agena_domain::{
    ComposerDocument, ComposerNode, ExecutionLifecycle, ExecutionOutcome, ModelRef,
    PermissionConfig, PermissionMode, ToolPermissionConfig,
};
use agena_runtime::{
    RuntimeBootstrapRequest, SessionCreateRequest, SessionRunOptions, SessionUserRunRequest,
    bootstrap_application_services,
};
use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Scenario {
    Greeting,
    ToolLoop,
    Disabled,
}

#[derive(Parser)]
struct Args {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    #[arg(long)]
    provider: String,
    #[arg(long)]
    adapter: String,
    #[arg(long)]
    model: String,
    #[arg(long)]
    thinking: Option<String>,
    #[arg(long, value_enum, default_value = "tool-loop")]
    scenario: Scenario,
    #[arg(long, default_value_t = 180)]
    timeout_secs: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let probe_root = tempfile::tempdir()?;
    let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
        workspace_root: Some(args.workspace.canonicalize()?),
        config_path: args.config,
        database_path: Some(probe_root.path().join("sessions.db")),
        scheduler_database_path: Some(probe_root.path().join("scheduler.db")),
        initialize_schema: true,
        ..Default::default()
    })
    .await?;
    let result = async {
        let services = runtime.application_services();
        let commands = services.execution_commands.as_ref().ok_or_else(|| anyhow::anyhow!("no execution commands"))?;
        let control = services.execution_control.as_ref().ok_or_else(|| anyhow::anyhow!("no execution control"))?;
        let store = services.session_store.as_ref().ok_or_else(|| anyhow::anyhow!("no session store"))?;
        let session_id = commands.create_session(SessionCreateRequest {
            title: "model tool mode probe".to_owned(),
            parent_session_id: None,
        }).await?.session_id;
        commands.set_session_permission(session_id, PermissionConfig {
            tools: Some(ToolPermissionConfig {
                names: BTreeMap::from([
                    ("agena.tools.help".to_owned(), PermissionMode::Allow),
                    ("agena.session.environment".to_owned(), PermissionMode::Allow),
                    ("agena.session.model".to_owned(), PermissionMode::Allow),
                    ("agena.session.rename".to_owned(), PermissionMode::Allow),
                ]),
                ..Default::default()
            }),
            ..Default::default()
        }).await?;
        let prompt = match args.scenario {
            Scenario::Greeting => "你好",
            Scenario::ToolLoop => concat!(
                "This is a live Tool API regression test. First call tools_help with ",
                "{\"tool\":[\"session.environment\",\"session.model\",\"session.rename\"]}. ",
                "After receiving that help, use tools_call to execute session.environment and session.model ",
                "with empty input objects, then session.rename with title MODEL_TOOL_LOOP_OK. ",
                "Only after these calls succeed, answer MODEL_TOOL_LOOP_OK and include the actual model ",
                "name and operating system returned by the tools. Do not use any other execution tool."
            ),
            Scenario::Disabled => concat!(
                "Read session.environment if tools are available. If Agena tools are disabled for ",
                "this model route, answer TOOLS_DISABLED_OK and explain their unavailability."
            ),
        };
        let accepted = commands.submit_user_run(SessionUserRunRequest::new(
            session_id,
            SessionRunOptions {
                model: ModelRef::try_new_with_adapter(args.provider, args.adapter, args.model.clone())?,
                thinking_mode: args.thinking,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: Some(4096),
            },
            ComposerDocument(vec![ComposerNode::Text { text: prompt.to_owned() }]),
        )).await?;
        anyhow::ensure!(accepted.receipt.is_some(), "execution not accepted");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(args.timeout_secs);
        let mut observed_active = false;
        let mut last_phase = String::new();
        loop {
            match control.active_execution(session_id).await {
                Some(ExecutionLifecycle::Active { phase, .. }) => {
                    observed_active = true;
                    let phase = format!("{phase:?}");
                    if phase != last_phase {
                        eprintln!("phase={phase}");
                        last_phase = phase;
                    }
                }
                Some(ExecutionLifecycle::Terminal { outcome, .. }) => {
                    anyhow::ensure!(outcome == ExecutionOutcome::Completed, "execution outcome: {outcome:?}");
                    break;
                }
                None if observed_active => break,
                None => {}
            }
            anyhow::ensure!(tokio::time::Instant::now() < deadline, "probe timed out");
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let view = store.load(session_id).await?;
        let tool_parts = view.parts.iter().filter(|part| part.kind == "tool_call").collect::<Vec<_>>();
        let tool_names = tool_parts.iter().filter_map(|part| part.content.get("name").and_then(serde_json::Value::as_str)).collect::<Vec<_>>();
        let mut final_text = String::new();
        for part in view.parts.iter().filter(|part| part.kind == "text" && part.role.as_str() == "assistant") {
            if let Some(refs) = part.content.get("resources").and_then(serde_json::Value::as_array) {
                for value in refs {
                    let reference: agena_domain::ContentRef = serde_json::from_value(value.clone())?;
                    final_text.push_str(&store.contents().read_text(reference.resource_id, 64 * 1024).await?.text);
                }
            }
            if let Some(text) = part.content.get("text").and_then(serde_json::Value::as_str) {
                final_text.push_str(text);
            }
        }
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({
            "scenario": format!("{:?}", args.scenario),
            "session_id": session_id,
            "session_title": view.meta.title,
            "tool_names": tool_names,
            "tool_states": tool_parts.iter().map(|part| part.state.as_str()).collect::<Vec<_>>(),
            "native_tool_calls": tool_parts.iter().filter_map(|part| part.content.get("tool_api_call")).collect::<Vec<_>>(),
            "final_text": final_text,
            "error_parts": view.parts.iter().filter(|part| part.kind == "error").map(|part| &part.content).collect::<Vec<_>>(),
        }))?);
        anyhow::ensure!(!final_text.trim().is_empty(), "no final answer");
        anyhow::ensure!(!final_text.contains("DSML"), "tool markup leaked into the answer");
        anyhow::ensure!(view.parts.iter().all(|part| part.kind != "error" && part.state.as_str() == "completed"), "incomplete or failed transcript parts");
        match args.scenario {
            Scenario::Greeting => {}
            Scenario::ToolLoop => {
                for name in ["tools_help", "session.environment", "session.model", "session.rename"] {
                    anyhow::ensure!(tool_names.contains(&name), "missing execution: {name}");
                }
                anyhow::ensure!(tool_parts.iter().all(|part| part.content.get("tool_api_call").is_some_and(|value| !value.is_null())), "missing native Tool API provenance");
                anyhow::ensure!(view.meta.title == "MODEL_TOOL_LOOP_OK", "rename did not persist");
                anyhow::ensure!(final_text.contains("MODEL_TOOL_LOOP_OK") && final_text.contains(&args.model), "tool results did not reach final answer");
            }
            Scenario::Disabled => {
                anyhow::ensure!(tool_parts.is_empty(), "disabled route executed a tool");
                anyhow::ensure!(final_text.contains("TOOLS_DISABLED_OK"), "disabled route did not explain availability");
            }
        }
        Ok(())
    }.await;
    runtime.shutdown();
    result
}
