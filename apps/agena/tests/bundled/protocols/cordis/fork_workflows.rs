use agena_api::resource::SessionResource;
use agena_domain::{AgentStatus, RawOutput};
use agena_testkit::{Reply, RequestMatch, TestCase, TestResult};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

const PLUGIN: &str = "deepseek.fixture-fork-workflows";
const WORKFLOW: &str = "deepseek.fixture-fork-workflows.workflow";
const ROUTED: &str = "deepseek.fixture-fork-workflows.routed_workflow";

async fn install(case: TestCase) -> TestResult {
    let root = case.paths().root.join("cordis-fork-workflows");
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::write(
        root.join("package.json"),
        br#"{"name":"@fixture/fork-workflows","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    )
    .await?;
    tokio::fs::write(
        root.join("bundle.yml"),
        r#"
- id: subagent-fork-in-process
  config: {providerName: with-history}
- id: workflow-ptc
  config: {provider: with-history}
- insert:
    - id: extra-spawn
      name: '@deepseek-ai/dsh-subagent-spawn-in-process'
      config: {providerName: isolated}
    - name: '@deepseek-ai/dsh-tool-workflow'
    - name: '@deepseek-ai/dsh-tool-jobs'
      config: {completionDelivery: quiet, inlineOutputBytes: 0}
    - name: ./route.mjs
"#,
    )
    .await?;
    tokio::fs::write(
        root.join("route.mjs"),
        r#"
import { defineTool } from '@deepseek-ai/dsh-tools';
export const inject = ['tools', 'workflowEngine'];
export function apply(ctx) {
    ctx.tools.register(defineTool({
        name: 'routed_workflow',
        description: 'Choose the standard per-run subagent provider.',
        parameters: {
            provider: { type: 'string', required: true },
        },
        output: {
            schema: { type: 'json' },
            render(_args, value) { return [{ type: 'text', text: JSON.stringify(value) }]; },
        },
        async execute({ provider }, exec) {
            const run = ctx.workflowEngine.start({
                parent: exec.agent, signal: exec.signal, subagentProvider: provider,
                meta: { name: 'routed', description: 'Use an explicit subagent provider' },
                script: "return await agent('Child request using an explicit provider');",
            });
            try { return await run.result; } finally { await run.dispose(); }
        },
    }));
}
"#,
    )
    .await?;
    case.run_cli([
        "plugin",
        "install",
        root.to_str().context("Cordis fork package path")?,
    ])
    .await?
    .require_success()?;
    Ok(())
}

fn workflow(name: &str, script: &str) -> Value {
    json!({"script": script, "meta": {"name": name, "description": "Exercise the fork provider"}})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_fork_uses_completed_context_and_only_child_output() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;

            const FIRST: &str = "First parent fork turn marker";
            const ANSWER: &str = "Saved parent answer marker";
            case.model().expect(RequestMatch::user_text(FIRST)).reply(Reply::tool_call(
                "fresh-fork", WORKFLOW, workflow("fresh-fork", "return await agent('First child prompt marker');"),
            ));
            case.model().expect(RequestMatch::user_text("First child prompt marker"))
                .check(|request| {
                    ensure!(!request.user_text().contains(FIRST), "a first-turn fork received the active turn");
                    Ok(())
                })
                .reply(Reply::text("Fresh child result"));
            case.model().expect(RequestMatch::tool_output("fresh-fork"))
                .check(|request| {
                    ensure!(request.tool_output_text("fresh-fork").unwrap_or_default().contains("Fresh child result"));
                    Ok(())
                })
                .reply(Reply::text(ANSWER));
            user.send(FIRST).await?.wait_settled().await?.assert_assistant_text(ANSWER)?;

            const ACTIVE: &str = "Second parent active turn marker";
            const QUEUED: &str = "Parent queued marker";
            let active = case.model().expect(RequestMatch::user_text(ACTIVE))
                .reply(
                    Reply::text_chunks(["Active streaming marker", " before tool"])
                        .with_tool_call("seeded-fork", WORKFLOW, workflow(
                            "seeded-fork",
                            "return await agent('Structured child prompt marker', {schema:{type:'object',properties:{value:{type:'string'}},required:['value']}});",
                        ))
                        .hold_after_text_chunks(1),
                );
            case.model().expect(RequestMatch::user_text("Structured child prompt marker"))
                .check(|request| {
                    let text = request.user_text();
                    let wire = request.body.to_string();
                    ensure!(text.contains(FIRST) && wire.contains(ANSWER), "the child missed completed context");
                    ensure!(!text.contains(ACTIVE) && !text.contains(QUEUED));
                    ensure!(!wire.contains("Active streaming marker"));
                    ensure!(text.contains("matching this schema"));
                    Ok(())
                })
                .reply(Reply::text(r#"{"value":"Fork structured result"}"#));
            case.model().expect(RequestMatch::tool_output("seeded-fork"))
                .check(|request| {
                    let output = request.tool_output_text("seeded-fork").context("workflow output")?;
                    let output: Value = serde_json::from_str(&output)?;
                    ensure!(output["phase"] == "succeeded");
                    ensure!(output["result"]["model"].as_str().unwrap_or_default().contains("Fork structured result"), "{output}");
                    Ok(())
                })
                .reply(Reply::text("Second parent turn settled"));
            case.model().expect(RequestMatch::user_text(QUEUED))
                .reply(Reply::text("Queued parent turn settled"));
            let run = user.send(ACTIVE).await?;
            active.observed().await?;
            let queued = user.send(QUEUED).await?;
            active.release();
            let settled = run.wait_settled().await?;
            settled.assert_succeeded()?;
            let tool = settled.parts.iter().find(|part| {
                part.kind == "tool_call" && part.content["name"] == WORKFLOW
            }).context("the parent run is missing the workflow operation")?;
            let detail = user.client().session_tool_detail(
                user.id(), tool.part_id, agena_api::live::ToolDetailSection::Output,
            ).await?;
            let raw: RawOutput = serde_json::from_value(detail.value)?;
            ensure!(raw.payload.context("structured workflow payload")?["result"] == json!({"value":"Fork structured result"}));
            queued.wait_settled().await?.assert_assistant_text("Queued parent turn settled")?;

            case.model().expect(RequestMatch::user_text("Child request using an explicit provider"))
                .check(|request| {
                    ensure!(!request.user_text().contains(FIRST), "spawn override received fork context");
                    Ok(())
                })
                .reply(Reply::text("Explicit spawn result"));
            let override_result = user.call_tool(ROUTED, json!({"provider":"spawn"})).await?;
            override_result.assert_succeeded()?;
            ensure!(override_result.data()?["value"] == "Explicit spawn result");
            let unknown = user.call_tool(ROUTED, json!({"provider":"unregistered-provider"})).await?;
            unknown.assert_failed()?;
            unknown.assert_text_contains("unregistered-provider")?;

            case.model().expect(RequestMatch::user_text("Silent fork child prompt marker"))
                .check(|request| {
                    ensure!(request.body.to_string().contains(ANSWER));
                    Ok(())
                })
                .reply(
                    Reply::text_chunks(std::iter::empty::<&str>())
                        .with_thinking_chunks(["Child reasoning without a final answer"]),
                );
            let silent = user.call_tool(WORKFLOW, workflow(
                "silent-fork", "return await agent('Silent fork child prompt marker');",
            )).await?;
            silent.assert_succeeded()?;
            ensure!(silent.data()?["agentsStarted"] == 1);
            ensure!(silent.data()?["result"].is_null(), "the child returned inherited text");
            let rows: Vec<SessionResource> = case.http().await
                .get(&format!("/api/v1/sessions/tree/{}", user.id())).await?
                .require_success()?.json()?;
            let children: Vec<_> = rows.iter().filter(|session| session.parent_id == Some(user.id())).collect();
            ensure!(children.len() == 4 && children.iter().all(|session| {
                session.is_subagent && session.agent_status == Some(AgentStatus::Completed)
            }), "{children:?}");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detached_cordis_forks_seed_each_child_when_it_starts() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            const EARLY: &str = "Parent completed before detached launch";
            const LATE: &str = "Parent completed during detached workflow";
            case.model().expect(RequestMatch::user_text(EARLY)).reply(Reply::text("Initial parent answer"));
            user.send(EARLY).await?.wait_settled().await?.assert_succeeded()?;
            let first = case.model().expect(RequestMatch::user_text("Hold first detached child"))
                .check(|request| {
                    ensure!(request.user_text().contains(EARLY) && !request.user_text().contains(LATE));
                    Ok(())
                })
                .reply(Reply::text_chunks(["First", " child"]).hold_after_text_chunks(1));
            let mut request = workflow(
                "detached-forks",
                "await agent('Hold first detached child'); return await agent('Check later detached child');",
            );
            request["run_in_background"] = true.into();
            let job = user.call_tool(WORKFLOW, request).await?;
            job.assert_succeeded()?;
            let id = job.data()?["jobId"].as_str().context("workflow job ID")?;
            first.observed().await?;
            case.model().expect(RequestMatch::user_text(LATE)).reply(Reply::text("Later parent answer"));
            user.send(LATE).await?.wait_settled().await?.assert_succeeded()?;
            case.model().expect(RequestMatch::user_text("Check later detached child"))
                .check(|request| {
                    let text = request.user_text();
                    ensure!(text.contains(EARLY) && text.contains(LATE), "the second child missed a newly completed turn");
                    ensure!(request.body.to_string().contains("Later parent answer"));
                    ensure!(!text.contains("Hold first detached child"));
                    Ok(())
                })
                .reply(Reply::text("Later child result"));
            first.release();
            let result = user.call_tool(format!("{PLUGIN}.job_output"), json!({"job_id": id, "wait":true})).await?;
            result.assert_succeeded()?;
            ensure!(result.data()?["job"]["status"] == "completed");
            result.assert_text_contains("Later child result")?;
            Ok(())
        })
        .await
}
