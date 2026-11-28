use std::time::Duration;

use agena_plugin_sdk::CommandStatus;
use agena_testkit::{Reply, RequestMatch, TestAgenaBuilder, TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

const PLUGIN: &str = "deepseek.fixture-subagent";
const TOOL: &str = "deepseek.fixture-subagent.delegate";
const CHILD_TEXT: &str = "Cordis child result marker";

fn fixture() -> TestAgenaBuilder {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
}

async fn install(case: TestCase) -> TestResult {
    let root = case.paths().root.join("cordis-subagent");
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::create_dir_all(case.workspace().join("subdir")).await?;
    tokio::fs::write(
        case.workspace().join("subdir/fixture.txt"),
        "Cordis cwd file",
    )
    .await?;
    tokio::fs::write(
        case.workspace().join("subdir/private.txt"),
        "protected fixture",
    )
    .await?;
    tokio::fs::write(
        root.join("package.json"),
        br#"{"name":"@fixture/subagent","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    )
    .await?;
    tokio::fs::write(
        root.join("bundle.yml"),
        r#"
- id: subagent
  config: {maxDepth: 3, maxActiveSubagents: 1}
- id: subagent-fork-in-process
  config: {providerName: with-history}
- id: tool-subagent
  config: {modelSelection: true, modelDiscovery: true}
- id: tool-subagent-fork
  config: {provider: with-history}
- id: tool-subagent-control
- insert:
    - name: ./main.mjs
    - name: ./observer.mjs
"#,
    )
    .await?;
    tokio::fs::write(
        root.join("main.mjs"),
        r#"
import { defineTool } from '@deepseek-ai/dsh-tools';
import { ROOT_AGENT_NAME } from '@deepseek-ai/dsh-subagent';
export const inject = ['commands', 'tools', 'subagents'];
export function apply(ctx) {
    let handle, last;
    const edges = [];
    ctx.on('subagent/start', info => edges.push(['start', info]));
    ctx.on('subagent/end', info => edges.push(['end', info]));
    const start = (parent, signal, { task, ...options }) => ctx.subagents.startActivation({
        parent, signal, provider: 'spawn', label: 'Cordis test delegation',
        prompt: [{type: 'text', text: task}], delivery: 'caller', ...options,
    });
    function receipt({childId, messageId, name}) { return {childId, messageId, name}; }
    const ok = value => ({kind: 'success', text: JSON.stringify(value)});
    ctx.tools.register(defineTool({
        name: 'delegate', description: 'Delegate a task and read the child outcome',
        parameters: {request: {type: 'json', required: true}},
        output: {schema: {type: 'json'}, render: (_, value) => [{type: 'text', text: JSON.stringify(value)}]},
        async execute({request}, {agent, signal}) {
            const run = await start(agent, signal, request);
            const result = await run.result;
            await run.dispose();
            const reply = {...receipt(run), result: {...result}, edges: edges.slice(-2)};
            if (request.delivery === 'parent') delete reply.result.output;
            return reply;
        },
    }));
    ctx.commands.register({
        name: 'delegate', description: 'Launch, observe or dispose a Cordis child',
        async handler({agent, signal, rawInput}) {
            const [action, task, message] = rawInput.split(':');
            if (action === 'inspect') return ok({last, edges, child: handle?.childId});
            if (action === 'providers') return ok({
                spawn: ctx.subagents.getProvider('spawn'),
                fork: ctx.subagents.getProvider('with-history'),
                absent: ctx.subagents.getProvider('fork') === undefined,
                maxDepth: ctx.subagents.resolveMaxDepth(),
                maxActiveSubagents: ctx.subagents.maxActiveSubagents,
                root: ROOT_AGENT_NAME,
            });
            if (action === 'dispose') {
                if (!handle) return ok({missing: true});
                await handle.dispose();
                return ok(await handle.result);
            }
            try {
                if (action === 'join-tree') return ok({joined: await ctx.subagents.waitForTreeWork(agent)});
                if (action === 'tree') {
                    const root = ctx.subagents.rootOf(agent);
                    return ok({
                        names: ctx.subagents.namesOf(agent),
                        capacity: ctx.subagents.capacityOf(root),
                        entries: await ctx.subagents.listDescendants(task || root.id, signal),
                    });
                }
                if (action === 'message') {
                    const id = await ctx.subagents.resolveName(agent, task, signal);
                    return ok({messageId: await ctx.subagents.sendMessage(
                        agent, id, [{type: 'text', text: message}], {signal},
                    )});
                }
                if (action === 'abort') await start(agent, AbortSignal.abort(new Error('fixture aborted')), {task});
                if (action === 'depth') await start(agent, signal, {task, maxDepth: 0});
                if (action === 'depth-one') await start(agent, signal, {task, maxDepth: 1});
                if (action === 'forged') await start({...agent}, signal, {task});
                if (action === 'bad-cwd') await start(agent, signal, {task, cwd: '..'});
                if (action === 'start') {
                    handle = await start(agent, signal, {task});
                    handle.result.then(value => { last = value; });
                    return ok(receipt(handle));
                }
            } catch (error) { return ok({error: String(error)}); }
            return ok({unexpected: true});
        },
    });
}
"#,
    )
    .await?;
    tokio::fs::write(
        root.join("observer.mjs"),
        r#"
export const inject = ['commands'];
export function apply(ctx) {
    const edges = [];
    ctx.on('subagent/start', info => edges.push(['start', info]));
    ctx.on('subagent/end', info => edges.push(['end', info]));
    ctx.commands.register({
        name: 'subagent-events', description: 'Read lifecycle events from other modules',
        handler: () => ({kind: 'success', text: JSON.stringify(edges)}),
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

async fn command(user: &UserSession, argument: &str) -> anyhow::Result<Value> {
    invoke_command(user, "delegate", argument).await
}

async fn invoke_command(user: &UserSession, name: &str, argument: &str) -> anyhow::Result<Value> {
    let reply = user
        .client()
        .invoke_plugin_command(PLUGIN, name, json!({}), Some(user.id()), None, argument)
        .await?
        .result;
    ensure!(reply.status == CommandStatus::Succeeded, "{reply:?}");
    Ok(serde_json::from_str(&reply.summary)?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_subagents_delegate_with_scoped_cwd_history_and_delivery() -> TestResult {
    fixture().run(|case| async move {
        super::wait_for_startup_catalog(&case).await?;
        let user = case.user().new_session().await?;
        super::super::super::allow_permissions(&user).await?;
        user.client().set_session_permission(user.id(), serde_json::from_value(json!({
            "path":{"workspace":"readwrite","external":"readwrite",
                "rules":{"<workspace>/subdir/private.txt":"deny"}},
            "tools":{"default":"allow"},
        }))?).await?;
        let providers = command(&user, "providers").await?;
        ensure!(providers["spawn"]["capabilities"]["agentPreset"] == true, "{providers}");
        ensure!(providers["fork"]["inheritsParentContext"] == true);
        ensure!(providers["fork"]["capabilities"]["agentPreset"] == false);
        ensure!(providers["maxDepth"] == 3 && providers["maxActiveSubagents"] == 1
            && providers["absent"] == true && providers["root"] == "lead");

        const HISTORY: &str = "Cordis completed parent turn";
        case.model().expect(RequestMatch::user_text(HISTORY)).reply(Reply::text("Saved parent answer"));
        user.send(HISTORY).await?.wait_settled().await?.assert_succeeded()?;

        let model = case.model().model_id().to_owned();
        case.model().expect(RequestMatch::user_text("Spawn only this Cordis child"))
            .check(|request| {
                ensure!(!request.user_text().contains(HISTORY));
                ensure!(request.body["max_output_tokens"] == 123, "{:?}", request.body["max_output_tokens"]);
                Ok(())
            })
            .reply(Reply::tool_call("check-cwd", "fs.read", json!({"file_path": "fixture.txt", "mode": "text"})));
        case.model().expect(RequestMatch::tool_output("check-cwd")).check(|request| {
            ensure!(request.tool_output_text("check-cwd").unwrap_or_default().contains("Cordis cwd file"));
            Ok(())
        }).reply(Reply::tool_call("check-parent-rule", "fs.read", json!({"file_path":"private.txt","mode":"text"})));
        case.model().expect(RequestMatch::tool_output("check-parent-rule")).check(|request| {
            let output = request.tool_output_text("check-parent-rule").context("protected file result")?;
            let output: Value = serde_json::from_str(&output)?;
            ensure!(output["phase"] == "failed" && !output.to_string().contains("protected fixture"), "{output}");
            Ok(())
        }).reply(Reply::text(CHILD_TEXT));
        let spawn = user.call_tool(TOOL, json!({"request":{
            "task":"Spawn only this Cordis child", "cwd":"subdir",
            "agentOptions":{"provider":"testkit","model":model,"maxTokens":123}
        }})).await?;
        spawn.assert_succeeded()?;
        let data = spawn.data()?;
        ensure!(data["name"] == "agent-1" && data["messageId"].as_str().is_some(), "{data}");
        ensure!(data["result"] == json!({"stopReason":"completed","output":[{"type":"text","text":CHILD_TEXT}]}));
        ensure!(data["edges"][0][0] == "start" && data["edges"][1][0] == "end", "{data}");
        ensure!(data["edges"][0][1]["runId"] == data["edges"][1][1]["runId"]);

        case.model().expect(RequestMatch::user_text("Caller-only mail check")).check(|request| {
            let encoded = request.body.to_string();
            ensure!(!encoded.contains("Agent agent-1"), "{encoded}");
            Ok(())
        }).reply(Reply::text("No duplicate child notification"));
        user.send("Caller-only mail check").await?.wait_settled().await?.assert_succeeded()?;

        const FORK: &str = "Run child with finished Cordis history";
        const ACTIVE: &str = "Delegate a historical Cordis task";
        const STRUCTURED: &str = r#"{"answer":"forked structured result"}"#;
        case.model().expect(RequestMatch::user_text(ACTIVE)).reply(Reply::tool_call(
            "corded-fork", TOOL, json!({"request":{
                "task":FORK, "provider":"with-history", "name":"history-reader", "delivery":"parent",
                "outputSchema":{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]}
            }}),
        ));
        case.model().expect(RequestMatch::user_text(FORK)).check(|request| {
            ensure!(request.user_text().contains(HISTORY));
            ensure!(!request.user_text().contains(ACTIVE));
            ensure!(request.user_text().contains("matching this schema"));
            Ok(())
        }).reply(Reply::text(STRUCTURED));
        case.model().expect(RequestMatch::tool_output("corded-fork")).check(|request| {
            let wire = request.body.to_string();
            ensure!(wire.contains("Agent history-reader"), "{wire}");
            let envelope: Value = serde_json::from_str(&request.tool_output_text("corded-fork").context("fork tool output")?)?;
            ensure!(envelope["phase"] == "succeeded", "{envelope}");
            let output: Value = serde_json::from_str(envelope["result"]["model"].as_str().context("fork result")?)?;
            ensure!(output["name"] == "history-reader");
            ensure!(output["result"]["structured"] == json!({"answer":"forked structured result"}));
            Ok(())
        }).reply(Reply::text("Parent received the fork result"));
        user.send(ACTIVE).await?.wait_settled().await?.assert_assistant_text("Parent received the fork result")?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_subagents_retain_command_handles_and_dispose_only_one_attempt() -> TestResult {
    fixture()
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            super::super::super::allow_permissions(&other).await?;
            for (request, reason) in [
                ("abort:No child prompt", "fixture aborted"),
                ("depth:No child prompt", "maxDepth"),
                ("forged:No child prompt", "exact live parent"),
                ("bad-cwd:No child prompt", "inside the parent"),
            ] {
                let rejected = command(&user, request).await?;
                ensure!(
                    rejected["error"]
                        .as_str()
                        .is_some_and(|text| text.contains(reason)),
                    "{rejected}"
                );
            }
            ensure!(command(&user, "inspect").await? == json!({"edges":[]}));
            ensure!(invoke_command(&user, "subagent-events", "").await? == json!([]));
            ensure!(invoke_command(&other, "subagent-events", "").await? == json!([]));

            let child_reply = case
                .model()
                .expect(RequestMatch::user_text("Held command child"))
                .reply(
                    Reply::text_chunks(["partial child reply", " late token"])
                        .hold_after_text_chunks(1),
                );
            let started = command(&user, "start:Held command child").await?;
            ensure!(started["name"] == "agent-1", "{started}");
            let child_id = started["childId"]
                .as_str()
                .context("child ID")?
                .parse::<i64>()?;
            child_reply.observed().await?;
            ensure!(command(&other, "dispose").await? == json!({"missing":true}));
            let refused = command(&user, "start:A second child would exceed the limit").await?;
            ensure!(
                refused["error"]
                    .as_str()
                    .is_some_and(|text| text.contains("concurrency limit reached (1 per tree)")),
                "{refused}"
            );
            let stopped = command(&user, "dispose").await?;
            ensure!(stopped["stopReason"] == "aborted", "{stopped}");
            child_reply.disconnected().await?;
            child_reply.release();

            let child = case.user().attach_session(child_id).await?;
            let resumed = case
                .model()
                .expect(RequestMatch::user_text("New attempt in existing child"))
                .reply(
                    Reply::text_chunks(["resumed child reply", " is intact"])
                        .hold_after_text_chunks(1),
                );
            let resumed_run = child.send("New attempt in existing child").await?;
            resumed.observed().await?;
            ensure!(command(&user, "dispose").await? == stopped);
            case.model()
                .expect(RequestMatch::new(
                    "completion notice from the resumed Cordis child",
                    |request| {
                        let wire = request.body.to_string();
                        wire.contains("Agent agent-1 (")
                            && wire.contains("resumed child reply is intact")
                    },
                ))
                .reply(Reply::text("Root acknowledged the resumed child"));
            resumed.release();
            resumed_run
                .wait_settled()
                .await?
                .assert_assistant_text("resumed child reply is intact")?;
            user.wait_text_contains("Root acknowledged the resumed child")
                .await?;

            case.model()
                .expect(RequestMatch::user_text(
                    "Child finishes after launch command",
                ))
                .reply(Reply::text("retained result"));
            let second = command(&user, "start:Child finishes after launch command").await?;
            ensure!(second["name"] == "agent-2", "{second}");
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let state = command(&user, "inspect").await?;
                    let observed = invoke_command(&user, "subagent-events", "").await?;
                    if state["last"]["output"][0]["text"] == "retained result"
                        && observed.as_array().is_some_and(|edges| edges.len() >= 4)
                    {
                        ensure!(
                            state["edges"]
                                .as_array()
                                .is_some_and(|edges| edges.len() == 4),
                            "{state}"
                        );
                        ensure!(observed == state["edges"], "{observed}");
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
            })
            .await??;
            ensure!(invoke_command(&other, "subagent-events", "").await? == json!([]));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_subagent_controls_keep_nested_catalog_and_inbox_in_one_tree() -> TestResult {
    fixture()
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let root = case.user().new_session().await?;
            let outsider = case.user().new_session().await?;
            root.client()
                .set_session_permission(
                    root.id(),
                    serde_json::from_value(json!({
                        "path": {"workspace":"readwrite", "external":"readwrite"},
                        "tools": {"default":"allow", "names":{"agena.agent.wait":"ask"}}
                    }))?,
                )
                .await?;
            super::super::super::allow_permissions(&outsider).await?;

            let outer = case
                .model()
                .expect(RequestMatch::user_text("First Cordis tree member"))
                .reply(Reply::text_chunks(["Outer member", " is ready"]).hold_after_text_chunks(1));
            let started = root
                .call_tool(
                    format!("{PLUGIN}.subagent"),
                    json!({
                        "description": "First Cordis tree member",
                        "prompt": "First Cordis tree member",
                        "provider": "testkit", "model": case.model().model_id(),
                    }),
                )
                .await?;
            started.assert_succeeded()?;
            ensure!(started.data()? == &json!({"kind":"activation","name":"agent-1"}));
            let starting_tree = command(&root, "tree").await?;
            let child_id = starting_tree["entries"][0]["id"]
                .as_str()
                .context("child ID")?;
            let child = case.user().attach_session(child_id.parse()?).await?;
            outer.observed().await?;
            case.model()
                .expect(RequestMatch::new(
                    "completion notice from the first Cordis member",
                    |request| {
                        let wire = request.body.to_string();
                        wire.contains("Agent agent-1 (") && wire.contains("Outer member is ready")
                    },
                ))
                .reply(Reply::text("Root acknowledged the first Cordis member"));
            outer.release();
            root.wait_text_contains("Root acknowledged the first Cordis member")
                .await?;

            let inner = case
                .model()
                .expect(RequestMatch::user_text("Held nested Cordis member"))
                .reply(Reply::text_chunks(["Nested", " held"]).hold_after_text_chunks(1));
            let too_deep = command(&child, "depth-one:Depth must include ancestors").await?;
            ensure!(
                too_deep["error"]
                    .to_string()
                    .contains("depth 2 exceeds maxDepth"),
                "{too_deep}"
            );
            let nested = command(&child, "start:Held nested Cordis member").await?;
            ensure!(nested["name"] == "agent-1/agent-1", "{nested}");
            let nested_id = nested["childId"].as_str().context("nested child ID")?;
            inner.observed().await?;
            assert_eq!(command(&child, "join-tree").await?, json!({"joined":false}));

            let tree = command(&child, "tree").await?;
            ensure!(tree["names"] == json!({"self": "agent-1", "parent": "lead"}));
            ensure!(
                tree["capacity"] == json!({"running": 1, "limit": 1}),
                "{tree}"
            );
            let full = command(&root, "start:Shared tree slot is occupied").await?;
            ensure!(
                full["error"]
                    .to_string()
                    .contains("concurrency limit reached (1 per tree)"),
                "{full}"
            );
            let rows = tree["entries"].as_array().context("agent catalog")?;
            ensure!(rows.len() == 2, "{tree}");
            ensure!(
                rows[0]["path"] == "agent-1"
                    && rows[0]["hasChildren"] == true
                    && rows[1]["path"] == "agent-1/agent-1"
                    && rows[1]["parentId"] == child_id
                    && rows[1]["id"] == nested_id
                    && rows[1]["depth"] == 2,
                "{tree}"
            );
            let outside = command(&outsider, &format!("tree:{}", root.id())).await?;
            ensure!(
                outside["error"]
                    .to_string()
                    .contains("outside this Cordis agent tree"),
                "{outside}"
            );
            let outside =
                command(&outsider, &format!("message:{nested_id}:Outsider message")).await?;
            ensure!(
                outside["error"]
                    .to_string()
                    .contains("outside this Cordis agent tree"),
                "{outside}"
            );

            let catalog = root
                .call_tool(format!("{PLUGIN}.list_agents"), json!({}))
                .await?;
            catalog.assert_succeeded()?;
            let catalog = catalog.data()?;
            ensure!(
                catalog[0]["name"] == "lead"
                    && catalog[1]["status"] == "inactive"
                    && catalog[2]["name"] == "agent-1/agent-1"
                    && catalog[2]["status"] == "running",
                "{catalog}"
            );
            let route = root
                .call_tool(
                    format!("{PLUGIN}.list_subagent_models"),
                    json!({
                        "provider": "testkit", "model": case.model().model_id(),
                    }),
                )
                .await?;
            route.assert_succeeded()?;
            route.assert_text_contains("Reasoning efforts:")?;

            let wait = root
                .start_tool(
                    format!("{PLUGIN}.wait_agent"),
                    json!({"timeout_ms": 10_000}),
                )
                .await?;
            let permission = wait.wait_permission().await?;
            let sent = command(&child, "message:lead:Message to the waiting root").await?;
            ensure!(sent["messageId"].as_str().is_some(), "{sent}");
            permission.allow_once().await?;
            let result = wait.wait_settled().await?;
            result.assert_succeeded()?;
            ensure!(result.data()?["timedOut"] == false, "{}", result.data()?);

            let refused = root
                .call_tool(
                    format!("{PLUGIN}.interrupt_agent"),
                    json!({
                        "agent": "lead",
                    }),
                )
                .await?;
            refused.assert_failed()?;

            let stopped = root
                .call_tool(
                    format!("{PLUGIN}.interrupt_agent"),
                    json!({
                        "agent": "agent-1/agent-1",
                    }),
                )
                .await?;
            stopped.assert_succeeded()?;
            inner.disconnected().await?;
            inner.release();
            ensure!(command(&child, "dispose").await?["stopReason"] == "aborted");

            let wake = case
                .model()
                .expect(RequestMatch::new(
                    "agent message to an idle child",
                    |request| {
                        let wire = request.body.to_string();
                        wire.contains("Message from lead (")
                            && wire.contains("Wake the idle Cordis child")
                    },
                ))
                .reply(Reply::text_chunks(["Idle child", " resumed"]).hold_after_text_chunks(1));
            let sent = root
                .call_tool(
                    format!("{PLUGIN}.send_message"),
                    json!({
                        "agent": "agent-1", "message": "Wake the idle Cordis child",
                    }),
                )
                .await?;
            sent.assert_succeeded()?;
            wake.observed().await?;
            case.model()
                .expect(RequestMatch::new(
                    "completion notice after an idle Cordis child resumed",
                    |request| {
                        let wire = request.body.to_string();
                        wire.contains("Agent agent-1 (") && wire.contains("Idle child resumed")
                    },
                ))
                .reply(Reply::text("Root acknowledged the inbox continuation"));
            let mut joining = std::pin::pin!(command(&root, "join-tree"));
            assert!(
                tokio::time::timeout(Duration::from_millis(250), &mut joining)
                    .await
                    .is_err()
            );
            wake.release();
            assert_eq!(joining.await?, json!({"joined":true}));
            root.wait_text_contains("Root acknowledged the inbox continuation")
                .await?;
            assert_eq!(command(&root, "join-tree").await?, json!({"joined":false}));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_subagent_tree_quota_defers_idle_wake_and_discovers_multiple_models() -> TestResult
{
    const SECONDARY: &str = "secondary-child-model";

    fixture()
        .configure_global(|config| {
            let models =
                &mut config["providers"]["testkit"]["adapters"]["openai_responses"]["models"];
            let mut extra = models["fake-model"].clone();
            extra["description"] = json!("Secondary child route");
            models[SECONDARY] = extra;
        })
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let root = case.user().new_session().await?;
            super::super::super::allow_permissions(&root).await?;

            // Multi-route discovery lists providers, all configured models, and per-model efforts.
            let providers = root
                .call_tool(format!("{PLUGIN}.list_subagent_models"), json!({}))
                .await?;
            providers.assert_succeeded()?;
            providers.assert_text_contains("testkit")?;

            let models = root
                .call_tool(
                    format!("{PLUGIN}.list_subagent_models"),
                    json!({"provider": "testkit"}),
                )
                .await?;
            models.assert_succeeded()?;
            models.assert_text_contains("testkit/fake-model")?;
            models.assert_text_contains(&format!("testkit/{SECONDARY}"))?;

            let secondary_detail = root
                .call_tool(
                    format!("{PLUGIN}.list_subagent_models"),
                    json!({"provider": "testkit", "model": SECONDARY}),
                )
                .await?;
            secondary_detail.assert_succeeded()?;
            secondary_detail.assert_text_contains("Reasoning efforts:")?;

            // Start agent-1 explicitly on the secondary model route and wait for it to finish.
            case.model()
                .expect(RequestMatch::user_text("Initial secondary model task"))
                .check(|request| {
                    ensure!(request.body["model"] == SECONDARY, "{}", request.body["model"]);
                    Ok(())
                })
                .reply(Reply::text("agent-1 done on secondary"));
            let first = root
                .call_tool(
                    format!("{PLUGIN}.subagent"),
                    json!({
                        "description": "Secondary model child",
                        "prompt": "Initial secondary model task",
                        "provider": "testkit",
                        "model": SECONDARY,
                    }),
                )
                .await?;
            first.assert_succeeded()?;
            ensure!(first.data()? == &json!({"kind": "activation", "name": "agent-1"}));
            case.model()
                .expect(RequestMatch::new("agent-1 initial completion", |request| {
                    let wire = request.body.to_string();
                    wire.contains("Agent agent-1 (")
                        && wire.contains("agent-1 done on secondary")
                }))
                .reply(Reply::text("root saw agent-1 initial completion"));
            root.wait_text_contains("root saw agent-1 initial completion")
                .await?;
            assert_eq!(command(&root, "join-tree").await?, json!({"joined": false}));

            // Launch agent-2 held mid-stream, occupying the tree's single active slot (maxActiveSubagents: 1).
            let held = case
                .model()
                .expect(RequestMatch::user_text("Held active slot holder"))
                .reply(
                    Reply::text_chunks(["Holding slot", " now finished"])
                        .hold_after_text_chunks(1),
                );
            let second = command(&root, "start:Held active slot holder").await?;
            ensure!(second["name"] == "agent-2", "{second}");
            held.observed().await?;

            // Prepare the model expectation for agent-1's deferred inbox wake BEFORE sending the message.
            let deferred_wake = case
                .model()
                .expect(RequestMatch::new(
                    "deferred wake for idle agent-1",
                    |request| {
                        let wire = request.body.to_string();
                        wire.contains("Message from lead (")
                            && wire.contains("Deferred wake after slot frees")
                    },
                ))
                .reply(
                    Reply::text_chunks(["Deferred agent-1", " completed"])
                        .hold_after_text_chunks(1),
                );

            // Send a message to idle agent-1 while agent-2 holds the only tree slot.
            let sent = root
                .call_tool(
                    format!("{PLUGIN}.send_message"),
                    json!({
                        "agent": "agent-1",
                        "message": "Deferred wake after slot frees",
                    }),
                )
                .await?;
            sent.assert_succeeded()?;

            // While agent-2 is still running, agent-1 must stay inactive and NOT start its turn.
            assert!(
                tokio::time::timeout(Duration::from_millis(300), deferred_wake.observed())
                    .await
                    .is_err(),
                "idle Cordis child woke while tree quota was full"
            );
            let catalog = root
                .call_tool(format!("{PLUGIN}.list_agents"), json!({}))
                .await?;
            catalog.assert_succeeded()?;
            let rows = catalog.data()?.as_array().context("agent rows")?;
            let agent1 = rows
                .iter()
                .find(|row| row["name"] == "agent-1")
                .context("agent-1 row")?;
            let agent2 = rows
                .iter()
                .find(|row| row["name"] == "agent-2")
                .context("agent-2 row")?;
            ensure!(agent1["status"] == "inactive", "{rows:?}");
            ensure!(agent2["status"] == "running", "{rows:?}");

            // Start waitForTreeWork while agent-2 is running, then release agent-2.
            // agent-1 must wake automatically once agent-2 settles, and join-tree must wait for BOTH!
            let mut joining = std::pin::pin!(command(&root, "join-tree"));
            held.release();
            deferred_wake.observed().await?;
            assert!(
                tokio::time::timeout(Duration::from_millis(250), &mut joining)
                    .await
                    .is_err(),
                "waitForTreeWork returned before deferred child completed"
            );

            case.model()
                .expect(RequestMatch::new(
                    "deferred agent-1 completion notice",
                    |request| {
                        let wire = request.body.to_string();
                        wire.contains("Agent agent-1 (")
                            && wire.contains("Deferred agent-1 completed")
                    },
                ))
                .reply(Reply::text("root saw deferred agent-1 completion"));
            deferred_wake.release();
            assert_eq!(joining.await?, json!({"joined": true}));
            root.wait_text_contains("root saw deferred agent-1 completion")
                .await?;
            assert_eq!(command(&root, "join-tree").await?, json!({"joined": false}));
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_reserved_child_id_media_messages_cross_session_events_when_tools_and_llm()
-> TestResult {
    const WAVE2_PLUGIN: &str = "deepseek.fixture-wave2";
    const PNG_2X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DA8J8BAAf/Af8Bf4mnAAAAAElFTkSuQmCC";

    async fn install_wave2(case: TestCase) -> TestResult {
        let root = case.paths().root.join("cordis-wave2");
        tokio::fs::create_dir_all(&root).await?;
        tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
        tokio::fs::write(
            root.join("package.json"),
            br#"{"name":"@fixture/wave2","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
        )
        .await?;
        tokio::fs::write(
            root.join("bundle.yml"),
            r#"
- id: subagent
  config: {maxDepth: 3, maxActiveSubagents: 3}
- id: tool-subagent
- id: tool-subagent-control
- insert:
    - name: ./main.mjs
    - name: ./observer.mjs
"#,
        )
        .await?;
        tokio::fs::write(
            root.join("main.mjs"),
            format!(
                r#"
import {{ defineTool }} from '@deepseek-ai/dsh-tools';
import EventEmitter from 'node:events';
import util from 'node:util';
import assert from 'node:assert/strict';
import qs from 'node:querystring';
import os from 'node:os';

export const inject = ['commands', 'tools', 'subagents', 'llm'];
export function apply(ctx) {{
    let unlocked = false;
    let alphaHandle = null;
    const localEvents = [];
    ctx.on('subagent/start', info => localEvents.push(['start', info]));
    ctx.on('subagent/end', info => localEvents.push(['end', info]));

    ctx.subagents.registerProvider({{
        name: 'custom-js',
        capabilities: {{ custom: true }},
        async startActivation(request) {{
            return {{
                childId: request.childId ?? 'custom-auto',
                name: request.name ?? 'custom-worker',
                result: Promise.resolve({{
                    stopReason: 'completed',
                    output: [{{ type: 'text', text: `custom-ok:${{request.prompt.length}}` }}],
                }}),
                async dispose() {{}},
            }};
        }},
    }});

    ctx.tools.register(
        defineTool({{
            name: 'gated_tool',
            description: 'Step-activated Cordis tool',
            deferLoading: true,
            parameters: {{ value: {{ type: 'string', required: true }} }},
            output: {{
                schema: {{ type: 'object' }},
                render: (_, value) => [{{ type: 'text', text: JSON.stringify(value) }}],
            }},
            async execute({{ value }}) {{
                return {{ ok: true, value }};
            }},
        }}),
        {{ when: () => unlocked }},
    );

    ctx.tools.register(defineTool({{
        name: 'delegate',
        description: 'Delegate a Cordis child with optional reserved childId',
        parameters: {{ request: {{ type: 'json', required: true }} }},
        output: {{
            schema: {{ type: 'json' }},
            render: (_, value) => [{{ type: 'text', text: JSON.stringify(value) }}],
        }},
        async execute({{ request }}, {{ agent, signal }}) {{
            const run = await ctx.subagents.startActivation({{
                parent: agent,
                signal,
                provider: 'spawn',
                label: 'Wave2 delegation',
                prompt: [{{ type: 'text', text: request.task }}],
                delivery: 'caller',
                ...request,
            }});
            const result = await run.result;
            await run.dispose();
            return {{ childId: run.childId, name: run.name, result }};
        }},
    }}));

    const ok = value => ({{ kind: 'success', text: JSON.stringify(value) }});
    ctx.commands.register({{
        name: 'wave2',
        description: 'Drive Wave 2 Cordis features',
        async handler({{ agent, signal, rawInput }}) {{
            try {{
                if (rawInput === 'unlock') {{
                    unlocked = true;
                    return ok({{ unlocked: true }});
                }}
                if (rawInput === 'ask-llm') {{
                    const reply = await ctx.llm.complete({{
                        system: 'helper',
                        prompt: 'summarize-wave2',
                        maxTokens: 64,
                    }});
                    return ok(reply);
                }}
                if (rawInput === 'spawn-alpha') {{
                    alphaHandle = await ctx.subagents.startActivation({{
                        parent: agent,
                        signal,
                        provider: 'spawn',
                        name: 'alpha',
                        childId: 'worker-custom-1',
                        label: 'Alpha reserved child',
                        prompt: [
                            {{ type: 'text', text: 'alpha task' }},
                            {{ type: 'image', mediaType: 'image/png', data: '{PNG_2X1}', filename: 'alpha.png' }},
                        ],
                        delivery: 'caller',
                    }});
                    return ok({{ childId: alphaHandle.childId, name: alphaHandle.name }});
                }}
                if (rawInput === 'spawn-dup') {{
                    await ctx.subagents.startActivation({{
                        parent: agent,
                        signal,
                        provider: 'spawn',
                        name: 'dup',
                        childId: 'worker-custom-1',
                        label: 'Duplicate reserved child',
                        prompt: [{{ type: 'text', text: 'dup' }}],
                        delivery: 'caller',
                    }});
                    return ok({{ unexpected: true }});
                }}
                if (rawInput === 'wait-alpha') {{
                    const result = await alphaHandle.result;
                    await alphaHandle.dispose();
                    return ok(result);
                }}
                if (rawInput === 'tree') {{
                    const root = ctx.subagents.rootOf(agent);
                    return ok({{
                        alphaId: await ctx.subagents.resolveName(agent, 'alpha', signal),
                        betaId: await ctx.subagents.resolveName(agent, 'alpha/beta', signal),
                        entries: await ctx.subagents.listDescendants(root.id, signal),
                    }});
                }}
                if (rawInput === 'send-media') {{
                    const messageId = await ctx.subagents.sendMessage(
                        agent,
                        'worker-custom-1',
                        [
                            {{ type: 'text', text: 'inspect dot' }},
                            {{ type: 'image', mediaType: 'image/png', data: '{PNG_2X1}', filename: 'dot.png' }},
                        ],
                        {{ signal }},
                    );
                    return ok({{ messageId }});
                }}
                if (rawInput === 'join-tree') {{
                    return ok({{ joined: await ctx.subagents.waitForTreeWork(agent) }});
                }}
                if (rawInput === 'custom-and-node') {{
                    const emitter = new EventEmitter();
                    let seen = '';
                    emitter.once('ev', v => {{ seen = v; }});
                    emitter.emit('ev', 'ok');
                    assert.strictEqual(seen, 'ok');
                    assert.deepStrictEqual({{ ...qs.parse(qs.stringify({{ x: '1', y: 'two words' }})) }}, {{ x: '1', y: 'two words' }});
                    const cloned = structuredClone({{ a: [1, 2], bytes: new Uint8Array([3, 4]) }});
                    assert.ok(util.isDeepStrictEqual(cloned.a, [1, 2]));
                    assert.ok(util.types.isUint8Array(cloned.bytes));
                    const customRun = await ctx.subagents.startActivation({{
                        parent: agent,
                        signal,
                        provider: 'custom-js',
                        childId: 'custom-child-9',
                        label: 'Custom JS run',
                        prompt: [{{ type: 'text', text: 'hello custom' }}],
                        delivery: 'caller',
                    }});
                    const customResult = await customRun.result;
                    await customRun.dispose();
                    return ok({{
                        formatted: util.format('%s=%d', 'wave', 2),
                        eol: os.EOL,
                        provider: ctx.subagents.getProvider('custom-js'),
                        childId: customRun.childId,
                        customResult,
                        customEvents: localEvents.filter(([, info]) => info.id === 'custom-child-9'),
                    }});
                }}
                return ok({{ unexpected: rawInput }});
            }} catch (error) {{
                return ok({{ error: String(error) }});
            }}
        }},
    }});
}}
"#
            ),
        )
        .await?;
        tokio::fs::write(
            root.join("observer.mjs"),
            r#"
export const inject = ['commands'];
export function apply(ctx) {
    const edges = [];
    ctx.on('subagent/start', info => edges.push(['start', info]));
    ctx.on('subagent/end', info => edges.push(['end', info]));
    ctx.commands.register({
        name: 'events',
        description: 'Read observed subagent events',
        handler: () => ({ kind: 'success', text: JSON.stringify(edges) }),
    });
}
"#,
        )
        .await?;
        case.run_cli([
            "plugin",
            "install",
            root.to_str().context("package path")?,
            "--enable-hooks",
        ])
        .await?
        .require_success()?;
        Ok(())
    }

    async fn wave2_cmd(user: &UserSession, name: &str, arg: &str) -> anyhow::Result<Value> {
        let reply = user
            .client()
            .invoke_plugin_command(WAVE2_PLUGIN, name, json!({}), Some(user.id()), None, arg)
            .await?
            .result;
        ensure!(reply.status == CommandStatus::Succeeded, "{reply:?}");
        Ok(serde_json::from_str(&reply.summary)?)
    }

    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(WAVE2_PLUGIN)
        .before_start(install_wave2)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let root = case.user().new_session().await?;
            super::super::super::allow_permissions(&root).await?;

            // 1. Step-activated tool (options.when + deferLoading: true): hidden before unlock, visible after unlock.
            let list_hidden = case
                .model()
                .expect(RequestMatch::user_text("check gated hidden"))
                .reply(Reply::function_call(
                    "list-hidden",
                    "tools_list",
                    json!({"plugin": WAVE2_PLUGIN}),
                ));
            case.model()
                .expect(RequestMatch::tool_output("list-hidden"))
                .after(&list_hidden)
                .check(|request| {
                    let output = request
                        .tool_output_text("list-hidden")
                        .context("tools_list output")?;
                    ensure!(
                        !output.contains("gated_tool"),
                        "expected gated_tool to be hidden before unlock: {output}"
                    );
                    Ok(())
                })
                .reply(Reply::text("gated hidden ok"));
            root.send("check gated hidden")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;

            assert_eq!(wave2_cmd(&root, "wave2", "unlock").await?, json!({"unlocked": true}));

            let list_visible = case
                .model()
                .expect(RequestMatch::user_text("check gated visible"))
                .reply(Reply::function_call(
                    "list-vis",
                    "tools_list",
                    json!({"plugin": WAVE2_PLUGIN}),
                ));
            let help_gated = case
                .model()
                .expect(RequestMatch::tool_output("list-vis"))
                .after(&list_visible)
                .check(|request| {
                    let output = request
                        .tool_output_text("list-vis")
                        .context("tools_list output")?;
                    ensure!(
                        output.contains("gated_tool"),
                        "expected gated_tool to appear after unlock: {output}"
                    );
                    Ok(())
                })
                .reply(Reply::function_call(
                    "help-gated",
                    "tools_help",
                    json!({"tool": format!("{WAVE2_PLUGIN}.gated_tool")}),
                ));
            let call_gated = case
                .model()
                .expect(RequestMatch::tool_output("help-gated"))
                .after(&help_gated)
                .reply(Reply::function_call(
                    "call-gated",
                    "tools_call",
                    json!({"tool": format!("{WAVE2_PLUGIN}.gated_tool"), "input": {"value": "unlocked-live"}}),
                ));
            case.model()
                .expect(RequestMatch::tool_output("call-gated"))
                .after(&call_gated)
                .check(|request| {
                    let output = request
                        .tool_output_text("call-gated")
                        .context("gated_tool output")?;
                    ensure!(output.contains("unlocked-live"), "{output}");
                    Ok(())
                })
                .reply(Reply::text("gated visible ok"));
            root.send("check gated visible")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;

            // 2. Direct Cordis `inject: ['llm']` auxiliary completion.
            case.model()
                .expect(RequestMatch::user_text("summarize-wave2"))
                .check(|request| {
                    let instructions = request
                        .body
                        .get("instructions")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    ensure!(instructions.contains("helper"), "{instructions}");
                    Ok(())
                })
                .reply(Reply::text("auxiliary-summary-ok"));
            let llm = wave2_cmd(&root, "wave2", "ask-llm").await?;
            ensure!(llm["text"] == "auxiliary-summary-ok", "{llm}");
            ensure!(llm["content"][0]["text"] == "auxiliary-summary-ok", "{llm}");
            ensure!(llm["provider"] == "testkit", "{llm}");
            ensure!(llm["model"] == "fake-model", "{llm}");

            // 3. Reserved childId, non-text initial prompt, nested grandchild with reserved childId,
            //    and cross-session subagent/start & subagent/end broadcast to root observer.
            let alpha_delegate = case
                .model()
                .expect(RequestMatch::new("alpha initial media prompt", |request| {
                    let wire = request.body.to_string();
                    wire.contains("alpha task") && wire.contains("alpha.png") && wire.contains("image/png")
                }))
                .reply(Reply::tool_call(
                    "alpha-help",
                    format!("{WAVE2_PLUGIN}.delegate"),
                    json!({
                        "request": {
                            "task": "grandchild task",
                            "name": "beta",
                            "childId": "grand-custom-2",
                            "delivery": "caller"
                        }
                    }),
                ));
            case.model()
                .expect(RequestMatch::user_text("grandchild task"))
                .reply(Reply::text("beta finished"));
            case.model()
                .expect(RequestMatch::tool_output("alpha-help"))
                .after(&alpha_delegate)
                .check(|request| {
                    let output = request
                        .tool_output_text("alpha-help")
                        .context("alpha delegate output")?;
                    ensure!(output.contains("grand-custom-2"), "{output}");
                    ensure!(output.contains("beta finished"), "{output}");
                    Ok(())
                })
                .reply(Reply::text("alpha finished"));

            let spawned = wave2_cmd(&root, "wave2", "spawn-alpha").await?;
            ensure!(spawned["childId"] == "worker-custom-1", "{spawned}");
            ensure!(spawned["name"] == "alpha", "{spawned}");

            let dup = wave2_cmd(&root, "wave2", "spawn-dup").await?;
            ensure!(
                dup["error"]
                    .as_str()
                    .is_some_and(|err| err.contains("already in use")),
                "expected duplicate reserved childId rejection: {dup}"
            );

            let alpha_done = wave2_cmd(&root, "wave2", "wait-alpha").await?;
            ensure!(alpha_done["stopReason"] == "completed", "{alpha_done}");
            ensure!(alpha_done["output"][0]["text"] == "alpha finished", "{alpha_done}");

            let tree = wave2_cmd(&root, "wave2", "tree").await?;
            ensure!(tree["alphaId"] == "worker-custom-1", "{tree}");
            ensure!(tree["betaId"] == "grand-custom-2", "{tree}");
            let entries = tree["entries"].as_array().context("tree entries")?;
            let alpha_row = entries
                .iter()
                .find(|entry| entry["name"] == "alpha")
                .context("alpha row")?;
            let beta_row = entries
                .iter()
                .find(|entry| entry["name"] == "beta")
                .context("beta row")?;
            ensure!(alpha_row["id"] == "worker-custom-1", "{alpha_row}");
            ensure!(beta_row["id"] == "grand-custom-2", "{beta_row}");
            ensure!(beta_row["parentId"] == "worker-custom-1", "{beta_row}");

            let events = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let observed = wave2_cmd(&root, "events", "").await?;
                    if observed
                        .as_array()
                        .is_some_and(|list| list.len() >= 4)
                    {
                        return Ok::<_, anyhow::Error>(observed);
                    }
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
            })
            .await
            .context("timed out waiting for cross-session subagent events on root observer")??;
            let list = events.as_array().context("observer events")?;
            ensure!(
                list.iter().any(|ev| ev[0] == "start" && ev[1]["id"] == "worker-custom-1"),
                "expected alpha start event on root observer: {events}"
            );
            ensure!(
                list.iter().any(|ev| ev[0] == "end" && ev[1]["id"] == "worker-custom-1"),
                "expected alpha end event on root observer: {events}"
            );
            ensure!(
                list.iter().any(|ev| {
                    ev[0] == "start"
                        && ev[1]["id"] == "grand-custom-2"
                        && ev[1]["parentSessionId"] == "worker-custom-1"
                }),
                "expected cross-session grandchild start event on root observer: {events}"
            );
            ensure!(
                list.iter().any(|ev| {
                    ev[0] == "end"
                        && ev[1]["id"] == "grand-custom-2"
                        && ev[1]["stopReason"] == "completed"
                }),
                "expected cross-session grandchild end event on root observer: {events}"
            );

            // 4. Non-text sendMessage waking idle reserved-ID child with image attachment.
            case.model()
                .expect(RequestMatch::new("alpha media inbox wake", |request| {
                    let wire = request.body.to_string();
                    wire.contains("inspect dot")
                        && wire.contains("dot.png")
                        && wire.contains("image/png")
                }))
                .reply(Reply::text("media message handled"));
            let sent = wave2_cmd(&root, "wave2", "send-media").await?;
            ensure!(sent["messageId"].as_str().is_some(), "{sent}");
            assert_eq!(
                wave2_cmd(&root, "wave2", "join-tree").await?,
                json!({"joined": true})
            );

            // 5. Custom JS provider + additional Node.js built-ins.
            let custom = wave2_cmd(&root, "wave2", "custom-and-node").await?;
            ensure!(custom["formatted"] == "wave=2", "{custom}");
            ensure!(custom["eol"] == "\n", "{custom}");
            ensure!(custom["provider"]["name"] == "custom-js", "{custom}");
            ensure!(custom["childId"] == "custom-child-9", "{custom}");
            ensure!(
                custom["customResult"]["output"][0]["text"] == "custom-ok:1",
                "{custom}"
            );
            ensure!(
                custom["customEvents"].as_array().is_some_and(|evs| {
                    evs.iter().any(|ev| ev[0] == "start" && ev[1]["local"] == false)
                        && evs.iter().any(|ev| ev[0] == "end" && ev[1]["local"] == false)
                }),
                "expected custom JS provider start/end events: {custom}"
            );
            Ok(())
        })
        .await
}
