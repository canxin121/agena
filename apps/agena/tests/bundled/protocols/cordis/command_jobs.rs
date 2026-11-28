use std::time::Duration;

use agena_domain::{ContentKind, ContentRef, ContentState};
use agena_plugin_sdk::CommandStatus;
use agena_testkit::{Reply, RequestMatch, TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

const PLUGIN: &str = "deepseek.fixture-command-jobs";
const COMMAND: &str = "command_jobs";

async fn install(case: TestCase, delivery: &str) -> TestResult {
    let root = case.paths().root.join("cordis-command-jobs");
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::write(
        root.join("package.json"),
        br#"{"name":"@fixture/command-jobs","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    )
    .await?;
    tokio::fs::write(
        root.join("bundle.yml"),
        format!(
            r#"
- insert:
    - name: '@deepseek-ai/dsh-jobs-local'
    - name: '@deepseek-ai/dsh-tool-jobs'
      config: {{completionDelivery: {delivery}}}
    - name: ./commands.mjs
"#,
        ),
    )
    .await?;
    tokio::fs::write(
        root.join("commands.mjs"),
        r#"
export const inject = ['commands', 'jobs'];

export function apply(ctx) {
    const complete = new Map();
    const response = value => ({kind: 'success', text: JSON.stringify(value)});
    ctx.commands.register({
        name: 'command_jobs', description: 'Exercise command-owned jobs',
        handler({rawInput, agent}) {
            const [action, id] = rawInput.split(' ');
            if (action === 'inspect') {
                return response(ctx.jobs.get(id, agent.id));
            }
            if (action === 'list') {
                return response(ctx.jobs.list(agent.id));
            }
            if (action === 'read-at') {
                return response(ctx.jobs.readAt(id, 0, agent.id));
            }
            if (action === 'finish') {
                complete.get(id)();
                return response({finished: id});
            }
            if (action === 'throw') {
                try {
                    ctx.jobs.start({
                        kind: 'fixture:command', label: 'Throwing starter', owner: agent.id,
                        run(job) {
                            job.append('uncommitted starter output');
                            throw new Error('fixture starter failed');
                        },
                    });
                } catch (error) {
                    return response({error: String(error)});
                }
                return response({error: null});
            }
            function start(label, owner) {
                return ctx.jobs.start({
                    kind: 'fixture:command', label, owner,
                    run(job) {
                        job.append(label + ' stdout\n', {channel: 'stdout'});
                        job.append(label + ' log\n', {channel: 'log'});
                        let settle;
                        const done = new Promise(resolve => { settle = resolve; });
                        complete.set(job.id, () => settle({status: 'completed', result: label + ' result'}));
                        return {done, cancel() { settle({status: 'killed'}); }};
                    },
                });
            }
            const first = start('Command first', agent.id);
            return response(action === 'two' ? [first, start('Command second')] : [first]);
        },
    });
}
"#,
    )
    .await?;
    case.run_cli(["plugin", "install", root.to_str().context("package path")?])
        .await?
        .require_success()?;
    Ok(())
}

async fn command(user: &UserSession, text: &str) -> anyhow::Result<Value> {
    let reply = user
        .client()
        .invoke_plugin_command(PLUGIN, COMMAND, json!({}), Some(user.id()), None, text)
        .await?
        .result;
    ensure!(reply.status == CommandStatus::Succeeded, "{reply:?}");
    Ok(serde_json::from_str(&reply.summary)?)
}

async fn tool(user: &UserSession, name: &str, input: Value) -> anyhow::Result<Value> {
    let result = user.call_tool(format!("{PLUGIN}.{name}"), input).await?;
    result.assert_succeeded()?;
    Ok(result.data()?.clone())
}

async fn command_captures(user: &UserSession) -> anyhow::Result<(i64, Vec<ContentRef>)> {
    let parts = user.parts().await?;
    let matching: Vec<_> = parts
        .iter()
        .filter(|part| part.content["command_id"] == COMMAND && part.content["plugin_id"] == PLUGIN)
        .collect();
    ensure!(matching.len() == 1, "{parts:?}");
    let part = matching[0];
    Ok((
        part.part_id,
        serde_json::from_value(part.content["resources"].clone())?,
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_command_jobs_share_a_durable_origin_and_use_the_session_registry() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(|case| install(case, "quiet"))
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            super::super::super::allow_permissions(&other).await?;

            let thrown = command(&user, "throw").await?;
            ensure!(
                thrown["error"]
                    .as_str()
                    .unwrap_or("")
                    .contains("fixture starter failed"),
                "{thrown}"
            );
            ensure!(user.parts().await?.is_empty());
            let [first, second]: [String; 2] =
                serde_json::from_value(command(&user, "two").await?)?;
            ensure!(
                first != second
                    && first.starts_with("fixture:command-")
                    && second.starts_with("fixture:command-")
            );
            let (anchor, captures) = command_captures(&user).await?;
            ensure!(captures.len() == 4, "{captures:?}");

            let own = command(&user, &format!("inspect {first}")).await?;
            ensure!(
                own["id"] == first && own["owner"] == user.id().to_string(),
                "{own}"
            );
            let listing = command(&user, "list").await?;
            ensure!(
                listing.as_array().is_some_and(|jobs| jobs.len() == 2),
                "{listing}"
            );
            let foreign = command(&other, &format!("inspect {first}"))
                .await
                .expect_err("a foreign session read the command job");
            ensure!(
                foreign
                    .to_string()
                    .contains("Unknown Cordis job in this session")
            );
            ensure!(command(&other, "list").await? == json!([]));
            other
                .call_tool(format!("{PLUGIN}.job_kill"), json!({"job_id": first}))
                .await?
                .assert_failed()?;

            let running = tool(&user, "job_list", json!({})).await?;
            ensure!(
                running["jobs"].as_array().is_some_and(|jobs| {
                    jobs.len() == 2
                        && jobs
                            .iter()
                            .any(|job| job["id"] == first && job["status"] == "running")
                        && jobs
                            .iter()
                            .any(|job| job["id"] == second && job["status"] == "running")
                }),
                "{running}"
            );
            command(&user, &format!("finish {first}")).await?;
            tool(
                &user,
                "job_kill",
                json!({"job_id": second, "reason":"fixture cancel"}),
            )
            .await?;
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let completed = command(&user, &format!("inspect {first}")).await?;
                    let killed = command(&user, &format!("inspect {second}")).await?;
                    if completed["status"] == "completed" && killed["status"] == "killed" {
                        ensure!(killed["detail"] == "fixture cancel", "{killed}");
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .context("command jobs did not settle")??;

            case.crash().await?;
            case.restart().await?;
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().attach_session(user.id()).await?;
            let (restored_anchor, restored_captures) = command_captures(&user).await?;
            ensure!(restored_anchor == anchor && restored_captures == captures);
            let mut captured = Vec::new();
            for capture in captures {
                let page = user
                    .client()
                    .read_content(user.id(), capture.resource_id, None, 4096)
                    .await?;
                ensure!(
                    page.resource.part_id == anchor
                        && page.resource.state == ContentState::Complete
                );
                captured.push((capture.kind, serde_json::to_string(&page.chunks)?));
            }
            for label in ["Command first", "Command second"] {
                for (kind, suffix) in [(ContentKind::Text, "stdout"), (ContentKind::Log, "log")] {
                    ensure!(
                        captured.iter().any(|(actual, body)| {
                            *actual == kind && body.contains(&format!("{label} {suffix}"))
                        }),
                        "{captured:?}"
                    );
                }
            }
            let retained = command(&user, &format!("read-at {first}")).await?;
            ensure!(
                retained["chunks"].as_array().is_some_and(|chunks| {
                    chunks
                        .iter()
                        .any(|chunk| chunk["text"] == "Command first stdout\n")
                }),
                "{retained}"
            );
            let completed = tool(&user, "job_output", json!({"job_id": first})).await?;
            ensure!(
                completed["job"]["id"] == first && completed["job"]["status"] == "completed",
                "{completed}"
            );
            let killed = tool(&user, "job_output", json!({"job_id": second})).await?;
            ensure!(
                killed["job"]["id"] == second && killed["job"]["status"] == "killed",
                "{killed}"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_command_completion_wakes_a_session_without_a_model_turn() -> TestResult {
    super::super::super::fixture_without_memory()
        .case_timeout(Duration::from_secs(45))
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(|case| install(case, "wakeup"))
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            let [id]: [String; 1] = serde_json::from_value(command(&user, "one").await?)?;
            let wake = case
                .model()
                .expect(RequestMatch::new(
                    "command job completion with only system input",
                    |request| {
                        let body = request.body.to_string();
                        body.contains("Cordis job Command first completed")
                            && body.contains("Command first stdout")
                            && body.contains("Command first result")
                            && !body.contains("Command first log")
                            && request.body["input"].as_array().is_some_and(|items| {
                                items.iter().all(|item| {
                                    item["role"] != "user"
                                        && !matches!(
                                            item["type"].as_str(),
                                            Some("function_call" | "function_call_output")
                                        )
                                })
                            })
                    },
                ))
                .reply(Reply::text("Acknowledged the command result"));
            command(&user, &format!("finish {id}")).await?;
            wake.observed().await?;
            wake.finished().await?;
            let output = tool(&user, "job_output", json!({"job_id": id})).await?;
            ensure!(
                output["job"]["status"] == "completed" && output["text"] == "",
                "{output}"
            );
            Ok(())
        })
        .await
}
