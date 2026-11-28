use agena_plugin_sdk::CommandStatus;
use agena_testkit::{Reply, RequestMatch, TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use serde_json::{Value, json};
use std::time::Duration;

const PLUGIN: &str = "deepseek.fixture-services";

async fn install(case: TestCase) -> TestResult {
    let root = case.paths().root.join("cordis-services");
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::write(
        root.join("package.json"),
        br#"{"name":"@fixture/services","version":"1.2.3","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    ).await?;
    tokio::fs::write(
        root.join("bundle.yml"),
        r#"
- insert:
    - name: '@deepseek-ai/dsh-storage'
    - name: '@deepseek-ai/dsh-storage-json'
      config:
        root: !!js dshHomePath('fixture-data')
    - name: '@deepseek-ai/dsh-storage-domain'
      config: {backend: json}
    - name: '@deepseek-ai/dsh-workflow-ptc'
      config: {provider: spawn, maxConcurrentAgents: 1, maxTotalAgents: 2, maxItemsPerCall: 2}
    - name: '@deepseek-ai/dsh-tool-workflow'
    - id: counter
      name: ./counter.mjs
    - id: missing
      name: ./missing.mjs
"#,
    )
    .await?;
    tokio::fs::write(
        root.join("missing.mjs"),
        r#"
export const inject = ['agentTeams'];
export function apply() { throw new Error('unavailable module was activated'); }
"#,
    )
    .await?;
    tokio::fs::write(root.join("counter.mjs"), r#"
import { defineDomain, domainTable } from '@deepseek-ai/dsh-storage-domain';
export const inject = {commands: null, storageDomain: null, storage: null};
const integer = {parse(value) {
    if (!Number.isSafeInteger(value)) throw new Error('Not an integer');
    return value;
}};
export async function apply(ctx) {
    if (ctx.get('storage')?.form('domain') !== ctx.storageDomain) throw new Error('Missing domain');
    const domain = await ctx.storageDomain.open(defineDomain({
        name: 'counter', version: 1, tables: {counts: domainTable(integer)},
        global: {schema: integer, initial: 0},
    }));
    const counts = domain.table('counts');
    const changes = [];
    ctx.on('domain/changed', change => changes.push(change));
    ctx.commands.register({name: 'counter', description: 'Store counters', async handler({rawInput}) {
        if (rawInput === 'write') {
            await counts.put('__proto__', 1);
            await counts.update('__proto__', current => current + 1);
            await domain.global.set(3);
        }
        let invalid = false;
        if (rawInput === 'invalid') {
            try { await counts.put('__proto__', 'poison'); } catch (error) { invalid = error.code === 'invalid-record'; }
        }
        if (rawInput === 'delete') await counts.delete('__proto__');
        return {kind: 'success', text: JSON.stringify({
            value: counts.get('__proto__') ?? null, global: domain.global.get(), changes, invalid,
        })};
    }});
}
"#).await?;

    let path = root.to_str().context("Cordis package path")?;
    let checked = case
        .run_cli(["plugin", "validate", path, "--strict"])
        .await?;
    let report: Value = serde_json::from_str(&checked.require_failure()?.stdout)?;
    let compatibility = &report["compatibility"];
    ensure!(compatibility["components"]["commands"] == 1, "{report}");
    ensure!(compatibility["components"]["Cordis tools"] == 4, "{report}");
    ensure!(
        compatibility["cordis_dependencies"]
            .as_array()
            .is_some_and(|dependencies| {
                dependencies.iter().any(|dependency| {
                    dependency["entry"] == "missing"
                        && dependency["name"] == "agentTeams"
                        && dependency["status"] == "unavailable"
                })
            }),
        "{report}"
    );
    let before = tokio::fs::read(&case.paths().global_config).await?;
    let command = [
        "plugin",
        "install",
        "deepseek:@fixture/services",
        "--package-source",
        path,
    ];
    let preview = case
        .run_cli(command.into_iter().chain(["--dry-run"]))
        .await?;
    let preview = preview.require_success()?;
    ensure!(
        preview.stdout.contains("deepseek.fixture-services v1.2.3"),
        "{preview:?}"
    );
    ensure!(preview.stdout.contains("Cordis tools (4)"), "{preview:?}");
    ensure!(
        preview.stderr.contains("agentTeams is unavailable"),
        "{preview:?}"
    );
    ensure!(tokio::fs::read(&case.paths().global_config).await? == before);
    case.run_cli(command).await?.require_success()?;
    Ok(())
}

async fn counter(user: &UserSession, input: &str) -> anyhow::Result<Value> {
    let result = user
        .client()
        .invoke_plugin_command(PLUGIN, "counter", json!({}), Some(user.id()), None, input)
        .await?
        .result;
    ensure!(result.status == CommandStatus::Succeeded, "{result:?}");
    Ok(serde_json::from_str(&result.summary)?)
}

async fn web_preview(case: &TestCase) -> TestResult {
    let http = case.http().await;
    let response: Value = http
        .send_json(
            reqwest::Method::POST,
            "/api/v1/plugins/marketplace/install",
            &json!({
                "spec": "deepseek:@fixture/services",
                "dry_run": true,
                "agent_options": {"package_source": case.paths().root.join("cordis-services")},
            }),
        )
        .await?
        .require_success()?
        .json()?;
    let id = response["task"]["id"].as_str().context("preview task ID")?;
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let task: Value = http
                .get(&format!("/api/v1/runtime/tasks/{id}"))
                .await?
                .require_success()?
                .json()?;
            if task["status"] != "running" {
                return Ok::<_, anyhow::Error>(task);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??;
    ensure!(result["status"] == "succeeded", "{result}");
    let message = result["message"].as_str().context("preview message")?;
    ensure!(
        message.contains("Cordis tools (4)") && message.contains("agentTeams is unavailable"),
        "{message}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_cordis_storage_survives_session_and_server_restarts() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            web_preview(&case).await?;
            let user = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            let saved = counter(&user, "write").await?;
            ensure!(saved["value"] == 2 && saved["global"] == 3, "{saved}");
            ensure!(saved["changes"][1] == json!({
                "domain": "counter", "table": "counts", "key": "__proto__", "operation": "put", "value": 2,
            }), "{saved}");
            let invalid = counter(&other, "invalid").await?;
            ensure!(invalid["value"] == 2 && invalid["invalid"] == true, "{invalid}");
            case.restart().await?;
            super::wait_for_startup_catalog(&case).await?;
            let recovered = case.user().attach_session(user.id()).await?;
            let saved = counter(&recovered, "read").await?;
            ensure!(saved["value"] == 2 && saved["global"] == 3, "{saved}");
            let deleted = counter(&recovered, "delete").await?;
            ensure!(deleted["value"].is_null() && deleted["changes"][0]["operation"] == "deleted", "{deleted}");
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_cordis_workflow_delegates_through_the_project_session() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            let tool = format!("{PLUGIN}.workflow");
            case.model()
                .expect(RequestMatch::user_text("Summarize the Cordis fixture"))
                .reply(Reply::text("Checked the fixture"));
            let workflow = user
                .call_tool(
                    &tool,
                    json!({
                        "script": "return (await parallel([() => agent(args.prompt)]))[0];",
                        "meta": {"name":"test-workflow", "description":"Fixture workflow"},
                        "args": {"prompt":"Summarize the Cordis fixture"},
                    }),
                )
                .await?;
            workflow.assert_succeeded()?;
            ensure!(
                workflow.data()?["agentsStarted"] == 1,
                "{:?}",
                workflow.data()?
            );
            ensure!(
                workflow.data()?["result"] == "Checked the fixture",
                "{:?}",
                workflow.data()?
            );

            let too_many = user.call_tool(&tool, json!({
                "script": "return await parallel([() => 1, () => 2, () => 3]);",
                "meta": {"name":"batch-limit", "description":"Test the engine configuration"},
            })).await?;
            too_many.assert_failed()?;
            too_many.assert_text_contains("at most 2 items")?;
            let recovered_null = user.call_tool(&tool, json!({
                "script": "return await pipeline([null], x => x, x => x ?? 'continued');",
                "meta": {"name":"null-item", "description":"Run stages after a null value"},
            })).await?;
            recovered_null.assert_succeeded()?;
            ensure!(recovered_null.data()?["result"] == json!(["continued"]));
            Ok(())
        })
        .await
}
