use std::time::Duration;

use agena_plugin_sdk::CommandStatus;
use agena_testkit::{Reply, RequestMatch, TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

const PLUGIN: &str = "deepseek.fixture-workflow-jobs";

async fn install(case: TestCase) -> TestResult {
    install_with_delivery(case, "quiet").await
}

async fn install_with_delivery(case: TestCase, delivery: &str) -> TestResult {
    let root = case.paths().root.join("cordis-workflow-jobs");
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::write(
        root.join("package.json"),
        br#"{"name":"@fixture/workflow-jobs","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    )
    .await?;
    tokio::fs::write(
        root.join("bundle.yml"),
        r#"
- insert:
    - name: '@deepseek-ai/dsh-workflow-ptc'
    - name: '@deepseek-ai/dsh-jobs-local'
    - name: '@deepseek-ai/dsh-tool-jobs'
      config: {completionDelivery: DELIVERY, maxWaitTimeoutMs: 20000, maxConsecutiveWakes: 1}
    - name: '@deepseek-ai/dsh-tool-workflow'
      config: {enableRunInBackground: true}
    - name: '@deepseek-ai/dsh-tool-workflow'
      config: {toolName: foreground_workflow, enableRunInBackground: false}
    - name: ./events.mjs
    - name: ./foreground-events.mjs
"#
        .replace("DELIVERY", delivery),
    )
    .await?;
    tokio::fs::write(root.join("events.mjs"), r#"
export const inject = ['commands', 'jobs'];
export function apply(ctx) {
    const events = [];
    const jobEvents = [];
    ctx.jobs.events.subscribe({owners: 'scope'}, event => jobEvents.push(event));
    for (const name of ['start', 'phase', 'log', 'agent-start', 'agent-end', 'end']) {
        ctx.on(`workflow/${name}`, (info, data) => {
            events.push({
                name, info, ...(data !== undefined && {data}),
                frozen: Object.isFrozen(info) && Object.isFrozen(info.meta) && Object.isFrozen(data),
            });
        });
    }
    ctx.commands.register({
        name: 'workflow_events', description: 'Read workflow event fixture',
        handler() { return {kind: 'success', text: JSON.stringify(events)}; }
    });
    ctx.commands.register({
        name: 'workflow_job_events', description: 'Read workflow job registry events',
        handler({rawInput, agent}) {
            const value = rawInput ? ctx.jobs.readAt(rawInput, 0, agent.id) : jobEvents;
            return {kind: 'success', text: JSON.stringify(value)};
        }
    });
}
"#).await?;
    tokio::fs::write(
        root.join("foreground-events.mjs"),
        r#"
import { defineTool } from '@deepseek-ai/dsh-tools';
export const inject = ['commands', 'tools', 'workflowEngine'];
const scripts = {
    completed: "phase('Review'); log('reading'); log('waiting'); return await agent('Hold the foreground event child', {label:'Check result'});",
    error: "await agent('Return before the workflow error'); throw new Error('fixture workflow error');",
    cancelled: "throw new Error('the pre-aborted script ran');",
    'listener-cancelled': "phase('Stop here'); await new Promise(() => {});",
};
export function apply(ctx) {
    const events = [];
    for (const name of ['start', 'phase', 'log', 'agent-start', 'agent-end', 'end']) {
        ctx.on(`workflow/${name}`, (info, data) => {
            events.push({
                name, info, ...(data !== undefined && {data}),
                frozen: Object.isFrozen(info) && Object.isFrozen(info.meta) && Object.isFrozen(data),
            });
        });
    }
    ctx.commands.register({
        name: 'foreground_origin_events', description: 'Read events in the workflow caller',
        handler() { return {kind: 'success', text: JSON.stringify(events)}; },
    });
    ctx.tools.register(defineTool({
        name: 'observe_foreground',
        description: 'Run a foreground workflow and read its local listener state.',
        parameters: {mode: {type: 'string', enum: Object.keys(scripts), required: true}},
        output: {
            schema: {type: 'json'},
            render(_args, value) { return [{type: 'text', text: JSON.stringify(value)}]; },
        },
        async execute({mode}, exec) {
            let once = 0;
            const remove = ctx.once('workflow/log', () => { once++; });
            let run;
            const disarm = mode === 'listener-cancelled'
                ? ctx.once('workflow/phase', () => run.cancel('fixture listener cancellation'))
                : () => {};
            const signal = mode === 'cancelled'
                ? AbortSignal.abort(new Error('fixture pre-abort reason'))
                : exec.signal;
            run = ctx.workflowEngine.start({
                parent: exec.agent, signal,
                meta: {name: mode, description: 'Exercise foreground workflow events'},
                script: scripts[mode],
            });
            const startWasSynchronous = events.some(event => event.info.id === run.id && event.name === 'start');
            try {
                const result = await run.result;
                return {
                    id: run.id, result, once, startWasSynchronous,
                    events: events.filter(event => event.info.id === run.id),
                };
            } finally {
                remove();
                disarm();
                await run.dispose();
            }
        },
    }));
}
"#,
    )
    .await?;
    case.run_cli([
        "plugin",
        "install",
        root.to_str().context("Cordis job package path")?,
    ])
    .await?
    .require_success()?;
    Ok(())
}

async fn call(user: &UserSession, tool: &str, input: Value) -> anyhow::Result<Value> {
    let result = user.call_tool(format!("{PLUGIN}.{tool}"), input).await?;
    result.assert_succeeded()?;
    Ok(result.data()?.clone())
}

async fn wait_until_job_output_runs(user: &UserSession) -> TestResult {
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            if user.parts().await?.iter().any(|part| {
                part.kind == "tool_call"
                    && part.state == "running"
                    && part.content["name"] == format!("{PLUGIN}.job_output")
            }) {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .context("job_output was never admitted")?
}

async fn observed_events(user: &UserSession) -> anyhow::Result<Vec<Value>> {
    read_workflow_events(user, "workflow_events").await
}

async fn read_workflow_events(user: &UserSession, command: &str) -> anyhow::Result<Vec<Value>> {
    let reply = user
        .client()
        .invoke_plugin_command(PLUGIN, command, json!({}), Some(user.id()), None, "")
        .await?
        .result;
    ensure!(reply.status == CommandStatus::Succeeded, "{reply:?}");
    Ok(serde_json::from_str(&reply.summary)?)
}

async fn wait_for_event(
    user: &UserSession,
    name: &str,
    event_name: &str,
) -> anyhow::Result<Vec<Value>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let events: Vec<_> = observed_events(user)
                .await?
                .into_iter()
                .filter(|event| event["info"]["meta"]["name"] == name)
                .collect();
            if events.iter().any(|event| event["name"] == event_name) {
                return Ok(events);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .with_context(|| format!("{name} workflow did not emit {event_name}"))?
}

async fn wait_for_job_event(
    user: &UserSession,
    id: &str,
    kind: &str,
) -> anyhow::Result<Vec<Value>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let events: Vec<_> = read_workflow_events(user, "workflow_job_events")
                .await?
                .into_iter()
                .filter(|event| event["job"]["id"] == id || event["id"] == id)
                .collect();
            let position = events.iter().position(|event| event["type"] == kind);
            if let Some(index) = position
                && (kind != "settled"
                    || events
                        .get(index + 1)
                        .is_some_and(|event| event["type"] == "output"))
            {
                return Ok(events);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .with_context(|| format!("workflow job {id} did not emit {kind}"))?
}

fn event_names(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event["name"].as_str().unwrap_or_default())
        .collect()
}

fn script(name: &str, body: &str) -> Value {
    json!({
        "script": body,
        "meta": {"name": name, "description": "Cordis workflow job fixture"},
        "run_in_background": true,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreground_cordis_events_reach_the_live_caller_and_other_modules() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            super::super::super::allow_permissions(&other).await?;
            ensure!(observed_events(&user).await?.is_empty());
            ensure!(observed_events(&other).await?.is_empty());
            let child = case
                .model()
                .expect(RequestMatch::user_text("Hold the foreground event child"))
                .reply(Reply::text_chunks(["Foreground", " result"]).hold_after_text_chunks(1));
            let running = user
                .start_tool(
                    format!("{PLUGIN}.observe_foreground"),
                    json!({"mode":"completed"}),
                )
                .await?;
            child.observed().await?;
            let progress = wait_for_event(&user, "completed", "agent-start").await?;
            ensure!(
                event_names(&progress) == ["start", "phase", "log", "log", "agent-start"],
                "{progress:?}"
            );
            ensure!(progress[1]["data"] == "Review" && progress[2]["data"] == "reading");
            ensure!(
                progress[4]["data"]["label"] == "Check result"
                    && progress[4]["data"]["phase"] == "Review"
            );
            ensure!(
                observed_events(&other).await?.is_empty(),
                "another session received workflow events"
            );
            child.release();

            let output = running.wait_settled().await?;
            output.assert_succeeded()?;
            let output = output.data()?;
            ensure!(output["result"]["value"] == "Foreground result");
            ensure!(output["startWasSynchronous"] == true && output["once"] == 1);
            let peer = wait_for_event(&user, "completed", "end").await?;
            ensure!(
                event_names(&peer)
                    == [
                        "start",
                        "phase",
                        "log",
                        "log",
                        "agent-start",
                        "agent-end",
                        "end"
                    ],
                "{peer:?}"
            );
            ensure!(
                output["events"] == json!(peer),
                "the invoking module saw a different lifecycle"
            );
            ensure!(
                peer.iter()
                    .all(|event| event["info"]["id"] == output["id"] && event["frozen"] == true)
            );
            ensure!(peer[4]["data"]["childId"] == peer[5]["data"]["childId"]);
            ensure!(peer[5]["data"]["seq"] == 1 && peer[5]["data"]["outcome"] == "completed");
            ensure!(peer[6]["data"] == json!({"stopReason":"completed","agentsStarted":1}));
            ensure!(read_workflow_events(&user, "foreground_origin_events").await? == peer);
            ensure!(observed_events(&other).await?.is_empty());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreground_cordis_event_settlement_reports_errors_and_cancellation_once() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            ensure!(observed_events(&user).await?.is_empty());
            case.model()
                .expect(RequestMatch::user_text("Return before the workflow error"))
                .reply(Reply::text("Child completed before the script failed"));
            for (mode, reason, expected, count, message) in [
                (
                    "error",
                    "error",
                    &["start", "agent-start", "agent-end", "end"][..],
                    1,
                    "fixture workflow error",
                ),
                (
                    "cancelled",
                    "cancelled",
                    &["start", "end"][..],
                    0,
                    "fixture pre-abort reason",
                ),
                (
                    "listener-cancelled",
                    "cancelled",
                    &["start", "phase", "end"][..],
                    0,
                    "fixture listener cancellation",
                ),
            ] {
                let output = call(&user, "observe_foreground", json!({"mode": mode})).await?;
                let events = wait_for_event(&user, mode, "end").await?;
                ensure!(event_names(&events) == expected, "{events:?}");
                ensure!(
                    output["events"] == json!(events) && output["startWasSynchronous"] == true,
                    "{output}"
                );
                let settled = &events.last().context("workflow/end event")?["data"];
                ensure!(
                    settled["stopReason"] == reason && settled["agentsStarted"] == count,
                    "{settled}"
                );
                ensure!(
                    settled["error"]
                        .as_str()
                        .unwrap_or_default()
                        .contains(message),
                    "{settled}"
                );
                ensure!(
                    output["result"]["error"] == settled["error"]
                        && output["result"]["value"].is_null()
                );
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_job_outlives_the_launch_turn_and_retains_its_result() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            super::super::super::allow_permissions(&other).await?;
            let child = case.model()
                .expect(RequestMatch::user_text("Summarize the detached Cordis fixture"))
                .reply(Reply::text_chunks(["detached", " result"]).hold_after_text_chunks(1));
            ensure!(observed_events(&user).await?.is_empty());
            ensure!(observed_events(&other).await?.is_empty());
            let mut args = script("detached-work", "phase('Collect'); log('starting fixture'); return await agent('Summarize the detached Cordis fixture', {label: 'Report', phase: 'Collect'});");
            args["meta"]["whenToUse"] = json!("Measure progress across modules");
            let started = tokio::time::timeout(Duration::from_secs(15), call(
                &user, "workflow", args,
            )).await.context("launch tool waited for the detached child")??;
            ensure!(started["kind"] == "background", "{started}");
            let id = started["jobId"].as_str().context("job ID")?;
            uuid::Uuid::parse_str(started["runId"].as_str().context("workflow run ID")?)?;
            child.observed().await?;

            let observed = wait_for_job_event(&user, id, "progress").await?;
            ensure!(observed[0]["type"] == "registered" && observed[0]["job"]["kind"] == "workflow", "{observed:?}");
            ensure!(observed[0]["job"]["owner"] == user.id().to_string(), "{observed:?}");
            ensure!(observed.iter().any(|event| event["type"] == "output" && event["total"].as_u64().is_some_and(|total| total > 0)), "{observed:?}");
            ensure!(observed.iter().any(|event| event["type"] == "progress" && event["job"]["progress"] == "Collect"), "{observed:?}");
            let packets = user.client().invoke_plugin_command(
                PLUGIN, "workflow_job_events", json!({}), Some(user.id()), None, id,
            ).await?.result;
            ensure!(packets.status == CommandStatus::Succeeded, "{packets:?}");
            let packets: Value = serde_json::from_str(&packets.summary)?;
            ensure!(packets["chunks"].as_array().is_some_and(|chunks| chunks.iter().any(|chunk|
                chunk["channel"] == "log" && chunk["text"] == "phase: Collect\n")), "{packets}");

            let job = call(&user, "job_list", json!({})).await?;
            ensure!(job["jobs"][0]["id"] == id && job["jobs"][0]["status"] == "running", "{job}");
            let running = call(&user, "job_output", json!({"job_id": id, "wait": true, "timeout_ms": 5})).await?;
            ensure!(running["job"]["status"] == "running", "{running}");
            let progress = running["text"].as_str().context("running progress")?;
            ensure!(progress.contains("phase: Collect") && progress.contains("log: starting fixture"), "{progress}");
            let repeat = call(&user, "job_output", json!({"job_id": id})).await?;
            ensure!(!repeat["text"].as_str().context("unread progress")?.contains("starting fixture"), "{repeat}");

            ensure!(call(&other, "job_list", json!({})).await?["jobs"] == json!([]));
            for tool in ["job_output", "job_kill"] {
                other.call_tool(format!("{PLUGIN}.{tool}"), json!({"job_id": id}))
                    .await?.assert_failed()?;
            }

            let waiting = user.start_tool(format!("{PLUGIN}.job_output"), json!({"job_id": id, "wait": true})).await?;
            wait_until_job_output_runs(&user).await?;
            child.release();
            let output = waiting.wait_settled().await?;
            output.assert_succeeded()?;
            ensure!(output.data()?["job"]["status"] == "completed", "{}", output.data()?);
            output.assert_text_contains("detached result")?;
            let events = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let events = observed_events(&user).await?;
                    if events.last().is_some_and(|event| event["name"] == "end") {
                        return Ok::<_, anyhow::Error>(events);
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }).await.context("workflow/end was not delivered")??;
            let names = events.iter().map(|event| event["name"].as_str().unwrap_or_default()).collect::<Vec<_>>();
            ensure!(names == ["start", "phase", "log", "agent-start", "agent-end", "end"], "{events:?}");
            ensure!(events.iter().all(|event| event["info"]["id"] == started["runId"] && event["frozen"] == true));
            ensure!(events[0]["info"]["meta"]["whenToUse"] == "Measure progress across modules");
            ensure!(events[3]["data"]["label"] == "Report" && events[3]["data"]["phase"] == "Collect");
            ensure!(events[3]["data"]["seq"] == events[4]["data"]["seq"] && events[4]["data"]["outcome"] == "completed");
            ensure!(events[5]["data"]["stopReason"] == "completed" && events[5]["data"]["agentsStarted"] == 1);
            ensure!(observed_events(&other).await?.is_empty());
            let observed = wait_for_job_event(&user, id, "settled").await?;
            let [.., settled, final_output] = observed.as_slice() else {
                anyhow::bail!("missing terminal workflow job events: {observed:?}");
            };
            ensure!(settled["type"] == "settled" && settled["job"]["status"] == "completed" && settled["cause"] == "producer" && settled["awaited"].is_boolean(), "{observed:?}");
            ensure!(final_output["type"] == "output" && final_output["total"] == settled["job"]["output"]["total"], "{observed:?}");
            let offsets: Vec<_> = observed.iter().filter_map(|event| {
                event["total"].as_u64().or_else(|| event["job"]["output"]["total"].as_u64())
            }).collect();
            ensure!(offsets.windows(2).all(|pair| pair[0] <= pair[1]), "{observed:?}");
            ensure!(read_workflow_events(&other, "workflow_job_events").await?.is_empty());
            let parts = user.parts().await?;
            ensure!(!parts.iter().any(|part| part.content.to_string().contains("Cordis workflow detached-work completed")));

            // A result above the inline budget stays unread even when the
            // quiet completion notice is already in the transcript.
            let unread_child = case.model()
                .expect(RequestMatch::user_text("Hold the unread workflow until its launcher ends"))
                .reply(Reply::text_chunks(["held", " result"]).hold_after_text_chunks(1));
            let unread = call(&user, "workflow", script(
                "unread-result", "await agent('Hold the unread workflow until its launcher ends'); return 'unread across restart '.repeat(140);",
            )).await?;
            let unread_id = unread["jobId"].as_str().context("unread job ID")?;
            unread_child.observed().await?;
            unread_child.release();
            let unread_events = wait_for_job_event(&user, unread_id, "settled").await?;
            ensure!(unread_events.iter().any(|event| event["type"] == "settled"
                && event["job"]["status"] == "completed"), "{unread_events:?}");

            case.restart().await?;
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().attach_session(user.id()).await?;
            let output = call(&user, "job_output", json!({"job_id": id})).await?;
            ensure!(output["job"]["status"] == "completed", "{output}");
            ensure!(output["text"] == "", "the consumed result was repeated after restart: {output}");
            let mut retained = false;
            for part in user.parts().await?.iter().filter(|part| {
                part.kind == "tool_call" && part.content["name"] == format!("{PLUGIN}.job_output")
            }) {
                let detail = user.client().session_tool_detail(
                    user.id(), part.part_id, agena_api::live::ToolDetailSection::Output,
                ).await?;
                if detail.value.to_string().contains("detached result") {
                    retained = true;
                    break;
                }
            }
            ensure!(retained, "the successful read lost its stored tool output");
            ensure!(call(&user, "job_kill", json!({"job_id": id})).await?["outcome"] == "already-finished");
            let unread_output = call(&user, "job_output", json!({"job_id": unread_id})).await?;
            ensure!(unread_output["text"].as_str().context("unread result")?.contains("unread across restart"));

            user.call_tool(
                format!("{PLUGIN}.foreground_workflow"),
                script("disabled-job", "return 1;"),
            ).await?.assert_failed()?;
            ensure!(call(&user, "job_list", json!({})).await?["jobs"].as_array().context("jobs")?.len() == 2);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_job_kill_and_session_clear_stop_owned_children() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            ensure!(
                read_workflow_events(&user, "workflow_job_events")
                    .await?
                    .is_empty()
            );
            for (name, prompt) in [
                ("kill-job", "Wait for Cordis job kill"),
                ("clear-job", "Wait for Cordis session clear"),
            ] {
                let child = case
                    .model()
                    .expect(RequestMatch::user_text(prompt))
                    .reply(Reply::text_chunks(["partial", " held"]).hold_after_text_chunks(1));
                let started = call(
                    &user,
                    "workflow",
                    script(name, &format!("return await agent({});", json!(prompt))),
                )
                .await?;
                let id = started["jobId"].as_str().context("job ID")?;
                child.observed().await?;
                if name == "kill-job" {
                    let killed = call(
                        &user,
                        "job_kill",
                        json!({"job_id": id, "reason": "Fixture no longer needs it"}),
                    )
                    .await?;
                    ensure!(killed["outcome"] == "cancellation-requested", "{killed}");
                    let output =
                        call(&user, "job_output", json!({"job_id": id, "wait": true})).await?;
                    ensure!(output["job"]["status"] == "killed", "{output}");
                    ensure!(
                        output["text"]
                            .as_str()
                            .context("cancellation output")?
                            .contains("Fixture no longer needs it"),
                        "{output}"
                    );
                    let observed = wait_for_job_event(&user, id, "settled").await?;
                    ensure!(
                        observed.iter().any(|event| event["type"] == "stopping"
                            && event["job"]["status"] == "stopping"),
                        "{observed:?}"
                    );
                    ensure!(
                        observed.iter().any(|event| event["type"] == "settled"
                            && event["job"]["status"] == "killed"
                            && event["cause"] == "kill"),
                        "{observed:?}"
                    );
                } else {
                    user.client().clear_session(user.id(), false).await?;
                }
                child.disconnected().await?;
            }
            let after_clear = call(&user, "job_list", json!({})).await?;
            ensure!(after_clear["jobs"] == json!([]), "{after_clear}");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupted_cordis_job_is_readable_after_service_restart() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            let child = case.model()
                .expect(RequestMatch::user_text("Hold for Cordis process restart"))
                .reply(Reply::text_chunks(["pre-crash", " held"]).hold_after_text_chunks(1));
            let started = call(&user, "workflow", script(
                "restart-job",
                "log('durable Cordis progress'); return await agent('Hold for Cordis process restart');",
            )).await?;
            let id = started["jobId"].as_str().context("job ID")?;
            child.observed().await?;
            let before = call(&user, "job_output", json!({"job_id": id})).await?;
            ensure!(before["job"]["status"] == "running", "{before}");
            ensure!(before["text"].as_str().context("pre-crash output")?.contains("durable Cordis progress"));

            case.crash().await?;
            child.disconnected().await?;
            case.restart().await?;
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().attach_session(user.id()).await?;
            let after = call(&user, "job_output", json!({"job_id": id})).await?;
            ensure!(after["job"]["status"] == "failed", "{after}");
            ensure!(after["job"]["finishedAt"].is_i64(), "{after}");
            ensure!(after["job"]["detail"].is_string(), "{after}");
            ensure!(!after["text"].as_str().context("recovered output")?.contains("durable Cordis progress"));

            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_job_delivers_completion_to_the_idle_session() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(|case| install_with_delivery(case, "wakeup"))
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            let model = case.model();
            let child = model
                .expect(RequestMatch::user_text("Complete the Cordis wake fixture"))
                .reply(
                    Reply::text_chunks(["Cordis", " result for idle owner"])
                        .hold_after_text_chunks(1),
                );
            let started = call(
                &user,
                "workflow",
                script(
                    "wake-job",
                    "return await agent('Complete the Cordis wake fixture');",
                ),
            )
            .await?;
            child.observed().await?;

            let wake = model
                .expect(RequestMatch::new(
                    "workflow completion wakes its owner",
                    |request| {
                        let body = request.body.to_string();
                        body.contains("Cordis workflow wake-job completed")
                            && body.contains("Cordis result for idle owner")
                    },
                ))
                .after(&child)
                .reply(Reply::text("Acknowledged the workflow result"));
            child.release();
            wake.observed().await?;
            wake.finished().await?;
            let output = call(&user, "job_output", json!({"job_id": started["jobId"]})).await?;
            ensure!(
                output["job"]["status"] == "completed" && output["text"] == "",
                "{output}"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_user_message_releases_cordis_job_wait_without_stopping_the_job() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            let child = case
                .model()
                .expect(RequestMatch::user_text(
                    "Hold until the steer fixture stops this job",
                ))
                .reply(Reply::text_chunks(["partial", " held"]).hold_after_text_chunks(1));
            let started = call(
                &user,
                "workflow",
                script(
                    "steered-job",
                    "return await agent('Hold until the steer fixture stops this job');",
                ),
            )
            .await?;
            let id = started["jobId"].as_str().context("job ID")?;
            child.observed().await?;
            let waiting = user
                .start_tool(
                    format!("{PLUGIN}.job_output"),
                    json!({
                        "job_id": id, "wait": true, "timeout_ms": 20_000,
                    }),
                )
                .await?;
            wait_until_job_output_runs(&user).await?;
            case.model()
                .expect(RequestMatch::user_text(
                    "Resume with the new user instruction",
                ))
                .reply(Reply::text("User input handled"));
            let next = user.send("Resume with the new user instruction").await?;
            let output = tokio::time::timeout(Duration::from_secs(8), waiting.wait_settled())
                .await
                .context("queued user input did not release job_output")??;
            output.assert_succeeded()?;
            ensure!(output.data()?["job"]["status"] == "running");
            output.assert_text_contains("user steered")?;
            next.wait_settled()
                .await?
                .assert_assistant_text("User input handled")?;
            ensure!(
                call(&user, "job_kill", json!({"job_id": id})).await?["outcome"]
                    == "cancellation-requested"
            );
            child.disconnected().await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_job_kill_can_run_in_the_same_batch_as_job_output_wait() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            let model = case.model();
            let child = model
                .expect(RequestMatch::user_text(
                    "Hold for parallel Cordis job controls",
                ))
                .reply(Reply::text_chunks(["partial", " held"]).hold_after_text_chunks(1));
            let started = call(
                &user,
                "workflow",
                script(
                    "parallel-controls",
                    "return await agent('Hold for parallel Cordis job controls');",
                ),
            )
            .await?;
            child.observed().await?;
            let id = started["jobId"].as_str().context("job ID")?;
            model
                .expect(RequestMatch::user_text("Call the parallel job controls"))
                .reply(
                    Reply::tool_call(
                        "parallel_job_wait",
                        format!("{PLUGIN}.job_output"),
                        json!({"job_id": id, "wait": true, "timeout_ms": 20_000}),
                    )
                    .with_tool_call(
                        "parallel_job_kill",
                        format!("{PLUGIN}.job_kill"),
                        json!({"job_id": id, "reason": "Cancelled in the same tool batch"}),
                    ),
                );
            model
                .expect(RequestMatch::tool_output("parallel_job_wait"))
                .check(|request| {
                    let payload = |call| {
                        let output = request
                            .tool_output_text(call)
                            .context("missing job tool result")?;
                        let envelope: Value = serde_json::from_str(&output)?;
                        ensure!(envelope["phase"] == "succeeded", "{envelope}");
                        Ok::<Value, anyhow::Error>(serde_json::from_str(
                            envelope["result"]["model"]
                                .as_str()
                                .context("model tool output")?,
                        )?)
                    };
                    let wait = payload("parallel_job_wait")?;
                    let kill = payload("parallel_job_kill")?;
                    ensure!(wait["job"]["status"] == "killed", "{wait}");
                    ensure!(kill["outcome"] == "cancellation-requested", "{kill}");
                    Ok(())
                })
                .reply(Reply::text("Parallel job controls completed"));
            tokio::time::timeout(Duration::from_secs(12), async {
                user.send("Call the parallel job controls")
                    .await?
                    .wait_settled()
                    .await?
                    .assert_assistant_text("Parallel job controls completed")
            })
            .await
            .context("job_kill waited behind job_output")??;
            child.disconnected().await?;
            Ok(())
        })
        .await
}
