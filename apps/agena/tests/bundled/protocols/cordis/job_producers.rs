use std::time::Duration;

use agena_api::resource::SessionResource;
use agena_domain::{AgentStatus, ContentId, ContentKind, ContentRef, ContentState};
use agena_plugin_sdk::CommandStatus;
use agena_testkit::{Reply, RequestMatch, TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

const PLUGIN: &str = "deepseek.fixture-job-producer";

async fn install(case: TestCase) -> TestResult {
    install_job_fixture(case, false, false).await
}

async fn install_with_custom_controller(case: TestCase) -> TestResult {
    install_job_fixture(case, true, false).await
}

async fn install_child_jobs(case: TestCase) -> TestResult {
    install_job_fixture(case, true, true).await
}

async fn install_job_fixture(
    case: TestCase,
    custom_controller: bool,
    workflow: bool,
) -> TestResult {
    let root = case.paths().root.join("cordis-job-producer");
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::write(root.join("package.json"),
        br#"{"name":"@fixture/job-producer","engines":{"dsh":">=0.2.2-rc.5"},"dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    ).await?;
    let mut patch = r#"
- insert:
    - name: ./producer.mjs
    - name: ./job-observer.mjs
"#
    .to_owned();
    if custom_controller {
        patch.push_str("    - name: ./controller.mjs\n");
        tokio::fs::write(
            root.join("controller.mjs"),
            r#"
export const inject = ['jobs', 'commands'];
export function apply(ctx) {
    const attach = () => ctx.jobs.attachController('fixture');
    const first = attach();
    const second = attach();
    ctx.commands.register({
        name: 'fixture_controller', description: 'Toggle the custom controller registrations',
        handler({rawInput}) {
            if (rawInput === 'first') first();
            if (rawInput === 'second') second();
            if (rawInput === 'restore') attach();
            return {kind: 'success', text: JSON.stringify({action: rawInput})};
        },
    });
}
"#,
        )
        .await?;
    }
    if workflow {
        patch.push_str("    - name: '@deepseek-ai/dsh-tool-workflow'\n");
    }
    tokio::fs::write(root.join("bundle.yml"), &patch).await?;
    tokio::fs::write(root.join("producer.mjs"), r#"
import { JobId } from '@deepseek-ai/dsh-jobs/brand';
import { defineTool } from '@deepseek-ai/dsh-tools';
import { setTimeout as delay } from 'node:timers/promises';

export const inject = ['jobs', 'commands', 'tools', 'mcpManager'];
export function apply(ctx) {
    let started = 0;
    let release;
    let stopReason = null;
    let cancellationStatus = null;
    let openOverlap;
    let observeOverlap;
    let packetJob;
    let spillPhase;
    ctx.commands.register({
        name: 'producer_state', description: 'Read or release the background producer',
        async handler({rawInput, agent, signal}) {
            if (rawInput.startsWith('wait ')) {
                const id = rawInput.slice(5);
                const pending = ctx.jobs.wait(id, 10000, agent.id, signal);
                release();
                const job = await pending;
                const terminal = await ctx.jobs.wait(id, 1, agent.id, AbortSignal.abort(new Error('old wait')));
                return {kind: 'success', text: JSON.stringify({job, terminal})};
            }
            if (rawInput === 'release') release();
            if (rawInput === 'withdraw-spill') spillPhase = 'withdrawn';
            if (rawInput.startsWith('kill ')) {
                const [, id, reason] = rawInput.split(' ');
                const outcome = ctx.jobs.kill(id, agent.id, reason);
                return {kind: 'success', text: JSON.stringify({outcome, cancellationStatus, stopReason})};
            }
            if (rawInput === 'append-packets') {
                packetJob.append('next', {channel: 'stdout'});
                packetJob.append('private-only', {channel: 'log'});
                return {kind: 'success', text: JSON.stringify(ctx.jobs.readAt(packetJob.id, 20, agent.id))};
            }
            if (rawInput === 'append-tail') {
                packetJob.append('x'.repeat(200) + '😀'.repeat(256) + 'z', {channel: 'stdout'});
                return {kind: 'success', text: JSON.stringify(ctx.jobs.readAt(packetJob.id, 0, agent.id))};
            }
            return {kind: 'success', text: JSON.stringify({started, stopReason, cancellationStatus})};
        },
    });
    ctx.tools.register(defineTool({
        name: 'produce', description: 'Start a Cordis JobSpec fixture.',
        parameters: {mode: {type: 'string', required: true}, jobId: {type: 'string'}, spillRoot: {type: 'string'}},
        timeoutMs: 20000,
        output: {
            schema: {type: 'json'},
            render(_args, value) { return [{type: 'text', text: JSON.stringify(value)}]; },
        },
        async execute({mode, jobId, spillRoot}, exec) {
            if (mode === 'spill') {
                spillPhase = 'live';
                const path = name => spillRoot + '/' + name + '.log';
                const readStdout = from => ({
                    text: '', nextOffset: from, lossy: false,
                    spillPath: spillPhase === 'live' ? path('stdout') : undefined,
                });
                const owner = exec.agent.id;
                const id = ctx.jobs.start({
                    kind: 'fixture:worker', label: 'Spill references', owner,
                    output: [
                        {
                            channel: 'stdout',
                            read(from) {
                                const result = readStdout(from);
                                if (from === 0) {
                                    result.text = 'x'.repeat(1300) + 'tail\n';
                                    result.nextOffset = result.text.length;
                                }
                                return result;
                            },
                        },
                        {read: readStdout},
                        {
                            channel: 'stderr',
                            read: from => ({text: '', nextOffset: from, lossy: false,
                                spillPath: path(spillPhase === 'settled' ? 'final' : 'stderr')}),
                        },
                        {read: from => ({text: '', nextOffset: from, lossy: false, spillPath: 'bad\nfile'})},
                    ],
                    run() {
                        const done = new Promise(resolve => {
                            release = () => {
                                spillPhase = 'settled';
                                resolve({status: 'completed', result: 'spill terminal'});
                            };
                        });
                        return {done, cancel() { release(); }};
                    },
                });
                return {id, output: ctx.jobs.get(id, owner).output};
            }
            if (mode === 'child-scope') {
                const id = ctx.jobs.start({
                    kind: 'fixture:worker', label: 'Child job', owner: exec.agent.id,
                    run(job) {
                        job.append('child job started\n');
                        const done = new Promise(resolve => {
                            release = () => resolve({status: 'killed', detail: 'child released its work'});
                        });
                        return {
                            done,
                            cancel() {
                                setTimeout(() => {
                                    job.append('child job released\n');
                                    release();
                                }, 250);
                            },
                        };
                    },
                });
                return {id};
            }
            if (mode === 'packets') {
                const owner = exec.agent.id;
                const id = ctx.jobs.start({
                    kind: 'fixture:worker', label: 'Packets', owner,
                    run(job) {
                        packetJob = job;
                        job.append('甲😀', {channel: 'stdout'});
                        job.append('beta', {channel: 'stderr', gapBefore: true});
                        job.append('notice', {channel: 'log'});
                        const done = new Promise(resolve => {
                            release = () => resolve({status: 'completed', result: 'packet result'});
                        });
                        return {done, cancel() { release(); }};
                    },
                });
                const staged = ctx.jobs.get(id, owner).output;
                packetJob.append('Ω!');
                return {id, staged, snapshot: ctx.jobs.readAt(id, 2, owner)};
            }
            if (mode === 'observe-overlap') {
                const owner = exec.agent.id;
                const before = ctx.jobs.get(jobId, owner);
                const advanced = new Promise(resolve => { observeOverlap = resolve; });
                openOverlap();
                const observation = await advanced;
                const timed = await ctx.jobs.wait(jobId, 5, owner);
                let cancellation = '';
                try {
                    await ctx.jobs.wait(jobId, 5000, owner, AbortSignal.timeout(10));
                } catch (error) {
                    cancellation = error.message;
                }
                return {before, observation, timed, cancellation, visible: ctx.jobs.list(owner)};
            }
            if (mode === 'tool-error') throw new Error('fixture unrelated tool error');
            let source = '';
            const id = ctx.jobs.start({
                kind: 'fixture:worker', label: 'Producer: ' + mode,
                owner: mode === 'forged-owner' ? 'not-this-session' : exec.agent.id,
                toolCall: {callId: exec.callId, rootCallId: exec.rootCallId},
                outputLimitBytes: 512,
                output: [{
                    channel: 'stderr',
                    read(fromByte) {
                        const text = source.slice(fromByte);
                        return {text, nextOffset: source.length, lossy: fromByte > 0 && text.length > 0};
                    },
                }],
                run(job) {
                    started += 1;
                    if (mode === 'starter-error') throw new Error('fixture starter failed');
                    if (job.id !== JobId(job.id)) throw new Error('brand did not preserve id');
                    job.append('producer staged output\n', {channel: 'stdout'});
                    job.updateProgress('starting');
                    source += 'first source chunk\n';
                    const released = new Promise(resolve => { release = resolve; });
                    const overlapping = mode === 'overlap'
                        ? new Promise(resolve => { openOverlap = resolve; }) : null;
                    let stopped = false;
                    const done = (async () => {
                        await delay(40);
                        if (!exec.signal.aborted) throw new Error('creating tool has not ended');
                        job.append('producer resumed after the originating call\n');
                        job.append('private job narration\n', {channel: 'log'});
                        job.updateProgress('waiting');
                        if (overlapping !== null) {
                            await overlapping;
                            if (!stopped) {
                                await delay(25);
                                const projection = ctx.jobs.get(job.id, exec.agent.id);
                                let ambientError;
                                try { await ctx.mcpManager.restart('missing-fixture-peer'); }
                                catch (error) { ambientError = error.message; }
                                job.append('progress while a sibling invocation waits\n');
                                job.updateProgress('advanced');
                                observeOverlap({projection, ambientError});
                            }
                        }
                        await released;
                        if (mode === 'done-error') throw new Error('fixture asynchronous failure');
                        await delay(mode === 'overlap' ? 100 : 10);
                        source += 'final source chunk\n';
                        return stopped
                            ? {status: 'killed', detail: 'producer cleaned up'}
                            : {status: 'completed', detail: 'producer complete', result: 'producer terminal result'};
                    })();
                    return {
                        done,
                        cancel(reason) {
                            if (mode === 'manual-cancel') {
                                cancellationStatus = ctx.jobs.get(job.id, exec.agent.id).status;
                                if (reason === 'reject') throw new Error('producer refused cancellation');
                                if (reason !== undefined) stopReason = reason;
                                stopped = true;
                                return;
                            }
                            stopReason = reason ?? 'session ended';
                            stopped = true;
                            if (overlapping !== null) openOverlap();
                            job.append('cancel hook observed: ' + stopReason + '\n');
                            release();
                        },
                    };
                },
            });
            if (typeof id !== 'string') throw new Error('jobs.start did not return synchronously');
            return {id};
        },
    }));
}
"#).await?;
    tokio::fs::write(
        root.join("job-observer.mjs"),
        r#"
export const inject = ['jobs', 'commands'];
export function apply(ctx) {
    const events = [];
    let otherOwner = 0;
    const dispose = ctx.jobs.events.subscribe({owners: 'scope'}, event => {
        const {type, job, id, owner, total, cause, awaited} = event;
        events.push({type, id: job?.id ?? id, owner: job?.owner ?? owner,
            status: job?.status, progress: job?.progress, total, cause, awaited,
            frozen: Object.isFrozen(event) && (job === undefined || Object.isFrozen(job.output))});
    });
    ctx.jobs.events.subscribe({owner: '999999999'}, () => { otherOwner += 1; });
    ctx.commands.register({
        name: 'job_observer', description: 'Read the job registry and its events',
        handler({rawInput, agent}) {
            const [operation, id, ...reason] = rawInput.split(' ');
            if (['get', 'kill', 'remove'].includes(operation)) {
                let response;
                try {
                    if (operation === 'get') response = ctx.jobs.get(id, agent.id);
                    if (operation === 'kill') response = ctx.jobs.kill(id, agent.id,
                        reason.length ? reason.join(' ') : undefined);
                    if (operation === 'remove') {
                        const result = ctx.jobs.remove(id, agent.id);
                        response = {removed: result === undefined};
                    }
                } catch (error) {
                    response = {error: error.message};
                }
                return {kind: 'success', text: JSON.stringify(response)};
            }
            if (rawInput.startsWith('read ')) {
                return {kind: 'success', text: JSON.stringify(ctx.jobs.read(rawInput.slice(5), agent.id))};
            }
            if (rawInput.startsWith('at ')) {
                const [, id, offset] = rawInput.split(' ');
                try {
                    return {kind: 'success', text: JSON.stringify(ctx.jobs.readAt(id, Number(offset), agent.id))};
                } catch (error) {
                    return {kind: 'success', text: JSON.stringify({error: error.message})};
                }
            }
            if (rawInput === 'dispose') dispose();
            return {kind: 'success', text: JSON.stringify({
                events, otherOwner, jobs: ctx.jobs.list(agent.id), unowned: ctx.jobs.list(),
                foreign: ctx.jobs.list('999999999'),
            })};
        },
    });
}
"#,
    )
    .await?;
    let path = root.to_str().context("Cordis JobSpec package path")?;
    let preview = case
        .run_cli(["plugin", "validate", path, "--strict"])
        .await?;
    let report: Value = serde_json::from_str(&preview.require_success()?.stdout)?;
    let expected_tools = if custom_controller { 1 } else { 4 } + usize::from(workflow);
    ensure!(
        report["compatibility"]["components"]["Cordis tools"] == expected_tools,
        "{report}"
    );
    patch.push_str(
        r#"
    - name: '@deepseek-ai/dsh-jobs-local'
      config: {maxConcurrentJobsPerOwner: 1, pumpPollMs: 10, retainBytes: 1024, settledRetainBytes: 16}
"#,
    );
    if !custom_controller {
        patch.push_str(
            r#"
    - name: '@deepseek-ai/dsh-tool-jobs'
      config: {completionDelivery: quiet, inlineOutputBytes: 1}
"#,
        );
    }
    tokio::fs::write(root.join("bundle.yml"), patch).await?;
    case.run_cli(["plugin", "install", path])
        .await?
        .require_success()?;
    Ok(())
}

async fn call(user: &UserSession, name: &str, args: Value) -> anyhow::Result<Value> {
    let call = user.call_tool(format!("{PLUGIN}.{name}"), args).await?;
    call.assert_succeeded()?;
    Ok(call.data()?.clone())
}

async fn command(user: &UserSession, text: &str) -> anyhow::Result<Value> {
    invoke(user, "producer_state", text).await
}

async fn invoke(user: &UserSession, name: &str, text: &str) -> anyhow::Result<Value> {
    let reply = user
        .client()
        .invoke_plugin_command(PLUGIN, name, json!({}), Some(user.id()), None, text)
        .await?
        .result;
    ensure!(reply.status == CommandStatus::Succeeded, "{reply:?}");
    Ok(serde_json::from_str(&reply.summary)?)
}

async fn observed_settlement(user: &UserSession, id: &str) -> anyhow::Result<Value> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = invoke(user, "job_observer", "").await?;
            if snapshot["events"]
                .as_array()
                .context("job events")?
                .windows(2)
                .any(|pair| {
                    pair[0]["type"] == "settled"
                        && pair[0]["id"] == id
                        && pair[1]["type"] == "output"
                        && pair[1]["id"] == id
                })
            {
                return Ok(snapshot);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .context("job settlement was never observed")?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_job_registry_observes_and_waits_without_blocking_its_module() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            let unrelated = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            super::super::super::allow_permissions(&unrelated).await?;
            ensure!(invoke(&user, "job_observer", "").await?["jobs"] == json!([]));
            ensure!(invoke(&unrelated, "job_observer", "").await?["events"] == json!([]));
            let (id, output) = start(&user, "overlap").await?;
            let overlap = call(
                &user,
                "produce",
                json!({"mode":"observe-overlap","jobId":id}),
            )
            .await?;
            ensure!(
                overlap["before"]["id"] == id && overlap["before"]["status"] == "running",
                "{overlap}"
            );
            ensure!(
                overlap["observation"]["projection"]["owner"] == user.id().to_string(),
                "{overlap}"
            );
            ensure!(
                overlap["observation"]["ambientError"]
                    .as_str()
                    .is_some_and(|error| error.contains("requires a running command or tool")
                        || error.contains("require a running command or tool")),
                "{overlap}"
            );
            ensure!(
                overlap["timed"]["status"] == "running"
                    && overlap["cancellation"] == "The operation timed out",
                "{overlap}"
            );
            ensure!(overlap["visible"][0]["progress"] == "advanced", "{overlap}");
            let result = command(&user, &format!("wait {id}")).await?;
            ensure!(
                result["job"]["status"] == "completed" && result["terminal"] == result["job"],
                "{result}"
            );
            wait_for_settlement(&user, output).await?;
            let observed = observed_settlement(&user, &id).await?;
            ensure!(observed["jobs"] == json!([result["job"]]), "{observed}");
            ensure!(
                observed["unowned"] == json!([])
                    && observed["foreign"] == json!([])
                    && observed["otherOwner"] == 0,
                "{observed}"
            );
            let events = observed["events"]
                .as_array()
                .context("observed Cordis jobs")?;
            ensure!(
                events
                    .first()
                    .is_some_and(|event| event["type"] == "registered"),
                "{observed}"
            );
            ensure!(
                events
                    .iter()
                    .all(|event| event["id"] == id && event["frozen"] == true),
                "{observed}"
            );
            ensure!(
                events
                    .iter()
                    .any(|event| event["type"] == "progress" && event["progress"] == "advanced"),
                "{observed}"
            );
            ensure!(
                events.iter().any(|event| event["type"] == "output"
                    && event["total"].as_u64().is_some_and(|n| n > 0)),
                "{observed}"
            );
            let [.., settlement, final_output] = events.as_slice() else {
                anyhow::bail!("missing terminal Cordis job events: {events:?}");
            };
            ensure!(
                settlement["type"] == "settled"
                    && settlement["status"] == "completed"
                    && settlement["cause"] == "producer"
                    && settlement["awaited"] == true
                    && final_output["type"] == "output"
                    && final_output["total"] == result["job"]["output"]["total"],
                "{observed}"
            );
            let private = invoke(&unrelated, "job_observer", "").await?;
            ensure!(
                private["jobs"] == json!([]) && private["events"] == json!([]),
                "{private}"
            );
            Ok(())
        })
        .await
}

async fn start(user: &UserSession, mode: &str) -> anyhow::Result<(String, ContentId)> {
    let started = user
        .call_tool(format!("{PLUGIN}.produce"), json!({"mode": mode}))
        .await?;
    started.assert_succeeded()?;
    ensure!(
        started.output["sources"] == json!([]),
        "the model saw the producer's capture"
    );
    let value = started.data()?;
    let id = value["id"].as_str().context("synchronous Cordis job ID")?;
    ensure!(
        id.strip_prefix("fixture:worker-").is_some_and(
            |suffix| suffix.len() == 16 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
        ),
        "{value}"
    );
    let outputs: Vec<ContentRef> =
        serde_json::from_value(started.part.content["resources"].clone())?;
    let text = outputs
        .iter()
        .find(|source| source.kind == ContentKind::Text)
        .context("Cordis producer text output")?;
    Ok((id.to_owned(), text.resource_id))
}

async fn wait_for_settlement(user: &UserSession, output: ContentId) -> TestResult {
    // Quiet delivery leaves a notice pending on an idle session. Settle the
    // job before the next scripted model round so that round receives it once.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let page = user
                .client()
                .read_content(user.id(), output, None, 1)
                .await?;
            if page.resource.state != ContentState::Active {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .context("Cordis job did not settle its output capture")?
}

#[tokio::test]
async fn cordis_custom_controllers_serve_only_live_module_registrations() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install_with_custom_controller)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            super::super::super::allow_permissions(&other).await?;

            let (_, first_output) = start(&user, "complete").await?;
            invoke(&user, "fixture_controller", "first").await?;
            invoke(&user, "fixture_controller", "first").await?;
            command(&user, "release").await?;
            wait_for_settlement(&user, first_output).await?;

            let (active, second_output) = start(&user, "cancel").await?;
            invoke(&user, "fixture_controller", "second").await?;
            let running = invoke(&user, "job_observer", &format!("get {active}")).await?;
            ensure!(running["status"] == "running", "{running}");
            ensure!(
                invoke(&user, "job_observer", &format!("kill {active} detached")).await?
                    == "requested"
            );
            wait_for_settlement(&user, second_output).await?;

            let rejected = user
                .call_tool(format!("{PLUGIN}.produce"), json!({"mode": "complete"}))
                .await?;
            rejected.assert_failed()?;
            ensure!(
                rejected.output.to_string().contains("no job controller"),
                "{}",
                rejected.output
            );
            ensure!(command(&user, "").await?["started"] == 2);

            let (_, other_output) = start(&other, "complete").await?;
            command(&other, "release").await?;
            wait_for_settlement(&other, other_output).await?;
            invoke(&user, "fixture_controller", "restore").await?;
            let (_, restored_output) = start(&user, "complete").await?;
            command(&user, "release").await?;
            wait_for_settlement(&user, restored_output).await?;
            ensure!(command(&user, "").await?["started"] == 3);
            Ok(())
        })
        .await
}

#[tokio::test]
async fn cordis_cancel_callbacks_and_removal_follow_the_registry_lifecycle() -> TestResult {
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
            invoke(&user, "job_observer", "").await?;
            let (id, output) = start(&user, "manual-cancel").await?;

            for operation in ["kill", "remove"] {
                let rejected = invoke(&other, "job_observer", &format!("{operation} {id}")).await?;
                ensure!(rejected["error"].as_str().is_some_and(|e| e.contains("Unknown Cordis job")), "{rejected}");
            }
            let remove_live = invoke(&user, "job_observer", &format!("remove {id}")).await?;
            ensure!(remove_live["error"].as_str().is_some_and(|e| e.contains("still running")), "{remove_live}");
            let rejected = invoke(&user, "job_observer", &format!("kill {id} reject")).await?;
            ensure!(rejected["error"].as_str().is_some_and(|e| e.contains("producer refused cancellation")), "{rejected}");
            let running = invoke(&user, "job_observer", &format!("get {id}")).await?;
            ensure!(running["status"] == "running", "{running}");

            let local = command(&user, &format!("kill {id} local")).await?;
            ensure!(local == json!({"outcome":"requested", "cancellationStatus":"running", "stopReason":"local"}), "{local}");
            let remote = invoke(&user, "job_observer", &format!("kill {id} latest reason")).await?;
            ensure!(remote == "requested", "{remote}");
            let without_reason = invoke(&user, "job_observer", &format!("kill {id}")).await?;
            ensure!(without_reason == "requested", "{without_reason}");
            let producer = command(&user, "").await?;
            ensure!(producer["cancellationStatus"] == "stopping" && producer["stopReason"] == "latest reason", "{producer}");
            command(&user, "release").await?;
            wait_for_settlement(&user, output).await?;
            let terminal = invoke(&user, "job_observer", &format!("get {id}")).await?;
            ensure!(terminal["status"] == "killed" && terminal["detail"] == "producer cleaned up; latest reason", "{terminal}");
            let finished = command(&user, &format!("kill {id} ignored")).await?;
            ensure!(finished["outcome"] == "already-finished" && finished["stopReason"] == "latest reason", "{finished}");
            observed_settlement(&user, &id).await?;

            let removed = invoke(&user, "job_observer", &format!("remove {id}")).await?;
            ensure!(removed["removed"] == true, "{removed}");
            for operation in ["get", "kill", "remove"] {
                let reply = invoke(&user, "job_observer", &format!("{operation} {id}")).await?;
                ensure!(reply["error"].as_str().is_some_and(|e| e.contains("Unknown Cordis job")), "{reply}");
            }
            let listing = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let listing = invoke(&user, "job_observer", "").await?;
                    let events = listing["events"].as_array().context("registry events")?;
                    if events.iter().any(|event| event["id"] == id && event["type"] == "removed") {
                        return Ok::<_, anyhow::Error>(listing);
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await??;
            ensure!(listing["jobs"] == json!([]), "{listing}");
            let changes = listing["events"].as_array().context("lifecycle events")?
                .iter()
                .filter(|event| event["id"] == id)
                .filter_map(|event| match event["type"].as_str()? {
                    "stopping" | "settled" | "removed" => event["type"].as_str(),
                    _ => None,
                })
                .collect::<Vec<_>>();
            ensure!(changes == ["stopping", "stopping", "stopping", "settled", "removed"], "{listing}");
            ensure!(call(&user, "job_list", json!({})).await?["jobs"] == json!([]));
            user.call_tool(format!("{PLUGIN}.job_output"), json!({"job_id": id}))
                .await?
                .assert_failed()?;
            case.crash().await?;
            case.restart().await?;
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().attach_session(user.id()).await?;
            let listing = invoke(&user, "job_observer", "").await?;
            ensure!(listing["jobs"] == json!([]), "{listing}");
            ensure!(call(&user, "job_list", json!({})).await?["jobs"] == json!([]));
            let stale = invoke(&user, "job_observer", &format!("get {id}")).await?;
            ensure!(stale["error"].as_str().is_some_and(|e| e.contains("Unknown Cordis job")), "{stale}");
            let contents = user.client().read_content(user.id(), output, None, 4096).await?;
            ensure!(serde_json::to_string(&contents.chunks)?.contains("producer staged output"));
            Ok(())
        })
        .await
}

#[tokio::test]
async fn cordis_spill_paths_follow_sources_and_persist_with_settlement() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            let spill_root = case.workspace();
            for name in ["stdout", "stderr", "final"] {
                tokio::fs::write(
                    spill_root.join(format!("{name}.log")),
                    "complete fixture output",
                )
                .await?;
            }
            let stdout = format!("{}/stdout.log", spill_root.display());
            let stderr = format!("{}/stderr.log", spill_root.display());
            let final_path = format!("{}/final.log", spill_root.display());
            invoke(&user, "job_observer", "").await?;
            let started = call(
                &user,
                "produce",
                json!({"mode":"spill", "spillRoot":spill_root}),
            )
            .await?;
            let id = started["id"].as_str().context("spill-producing job ID")?;
            ensure!(
                started["output"]
                    == json!({"total":1305,"earliest":281,"spillPaths":[stdout,stderr]}),
                "{started}"
            );
            let observed = invoke(&user, "job_observer", &format!("at {id} 0")).await?;
            ensure!(
                observed["lossy"] == true && observed["next"] == 1305,
                "{observed}"
            );
            let output = call(&user, "job_output", json!({"job_id":id})).await?;
            let text = output["text"]
                .as_str()
                .context("model spill reference hint")?;
            ensure!(
                text.contains(&format!("full output: {stdout}, {stderr}")),
                "{output}"
            );
            command(&user, "withdraw-spill").await?;
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let job = invoke(&user, "job_observer", &format!("get {id}")).await?;
                    if job["output"]["spillPaths"] == json!([stderr]) {
                        ensure!(job["output"]["total"] == 1305, "{job}");
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await??;
            command(&user, "release").await?;
            observed_settlement(&user, id).await?;
            let settled = invoke(&user, "job_observer", &format!("get {id}")).await?;
            ensure!(
                settled["status"] == "completed"
                    && settled["output"]["spillPaths"] == json!([final_path]),
                "{settled}"
            );
            case.crash().await?;
            case.restart().await?;
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().attach_session(user.id()).await?;
            let restored = invoke(&user, "job_observer", &format!("get {id}")).await?;
            ensure!(restored == settled, "{restored}");
            let consumed = invoke(&user, "job_observer", &format!("read {id}")).await?;
            ensure!(
                consumed["result"] == "spill terminal" && consumed["chunks"] == json!([]),
                "{consumed}"
            );
            Ok(())
        })
        .await
}

#[tokio::test]
async fn cordis_jobs_are_owned_by_the_child_attempt_and_finish_before_the_agent() -> TestResult {
    const TASK: &str = "Exercise child-owned Cordis jobs";
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install_child_jobs)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            let (parent_job, parent_output) = start(&user, "complete").await?;
            case.model().expect(RequestMatch::user_text(TASK)).reply(Reply::tool_call(
                "child-producer", format!("{PLUGIN}.produce"), json!({"mode":"child-scope"}),
            ));
            case.model().expect(RequestMatch::tool_output("child-producer")).reply(Reply::tool_call(
                "child-workflow", format!("{PLUGIN}.workflow"), json!({
                    "run_in_background":true,
                    "script":"log('local child workflow'); return 42;",
                    "meta":{"name":"child-workflow", "description":"Run a local child workflow"},
                }),
            ));
            case.model().expect(RequestMatch::tool_output("child-workflow")).reply(Reply::tool_call(
                "nested-workflow", format!("{PLUGIN}.workflow"), json!({
                    "run_in_background":true,
                    "script":"return await agent('One more level');",
                    "meta":{"name":"nested-workflow", "description":"Check nested delegation policy"},
                }),
            ));
            case.model().expect(RequestMatch::tool_output("nested-workflow"))
                .reply(Reply::text("Finished the child Cordis work"));
            let agent = user.call_tool("agena.agent.spawn", json!({"name":"cordis_owner","prompt":TASK})).await?;
            agent.assert_succeeded()?;
            let agent_id = agent.data()?["agent_id"].as_i64().context("child session ID")?;
            let http = case.http().await;
            let tree_endpoint = format!("/api/v1/sessions/tree/{}", user.id());
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let tree: Vec<SessionResource> = http.get(&tree_endpoint).await?.require_success()?.json()?;
                    if let Some(child) = tree.iter().find(|child| child.id == agent_id)
                        && let Some(status) = child.agent_status.filter(|status| status.is_terminal())
                    {
                        ensure!(status == AgentStatus::Completed, "{child:?}");
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }).await??;
            let child = case.user().attach_session(agent_id).await?;
            let listing = invoke(&child, "job_observer", "").await?;
            let jobs = listing["jobs"].as_array().context("child jobs")?;
            ensure!(jobs.len() == 3, "{listing}");
            let produced = jobs.iter().find(|job| job["label"] == "Child job").context("owned producer")?;
            ensure!(produced["status"] == "killed" && produced["detail"] == "child released its work", "{listing}");
            let producer_id = produced["id"].as_str().context("owned producer ID")?;
            let data = invoke(&child, "job_observer", &format!("read {producer_id}")).await?;
            ensure!(data["chunks"].to_string().contains("child job released"), "{data}");

            let workflow = jobs.iter().find(|job| job["label"] == "child-workflow").context("local workflow")?;
            ensure!(workflow["status"] == "completed", "{listing}");
            let nested = jobs.iter().find(|job| job["label"] == "nested-workflow").context("nested workflow record")?;
            ensure!(nested["status"] == "failed" && nested["detail"].to_string().contains("cannot create nested agents"), "{listing}");

            let foreign = invoke(&user, "job_observer", &format!("get {producer_id}")).await?;
            ensure!(foreign["error"].as_str().is_some_and(|error| error.contains("Unknown Cordis job")), "{foreign}");
            let parent = invoke(&user, "job_observer", &format!("get {parent_job}")).await?;
            ensure!(parent["status"] == "running", "{parent}");
            command(&user, "release").await?;
            wait_for_settlement(&user, parent_output).await
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_packet_reads_share_the_durable_model_cursor() -> TestResult {
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
            let started = call(&user, "produce", json!({"mode": "packets"})).await?;
            let id = started["id"].as_str().context("packet job ID")?;
            let chunks = json!([
                {"at":0, "text":"甲😀", "channel":"stdout"},
                {"at":7, "text":"beta", "channel":"stderr", "gapBefore":true},
                {"at":11, "text":"notice", "channel":"log"},
                {"at":17, "text":"Ω!"}
            ]);
            ensure!(started["staged"] == json!({"earliest":0, "total":17}), "{started}");
            ensure!(started["snapshot"] == json!({"chunks":chunks, "next":20, "lossy":false}), "{started}");
            let snapshot = invoke(&user, "job_observer", &format!("at {id} 2")).await?;
            ensure!(snapshot == started["snapshot"], "{snapshot}");
            let invalid = invoke(&user, "job_observer", &format!("at {id} 1.5")).await?;
            ensure!(invalid["error"].as_str().is_some_and(|s| s.contains("safe integer")), "{invalid}");
            let foreign = invoke(&other, "job_observer", &format!("at {id} 0")).await?;
            ensure!(foreign["error"].is_string() && foreign.get("chunks").is_none(), "{foreign}");

            let consumed = invoke(&user, "job_observer", &format!("read {id}")).await?;
            ensure!(consumed["chunks"] == chunks && consumed.get("result").is_none(), "{consumed}");
            ensure!(call(&user, "job_output", json!({"job_id":id})).await?["text"] == "");
            let next = command(&user, "append-packets").await?;
            ensure!(next["next"] == 36 && next["chunks"][1]["channel"] == "log", "{next}");
            let model = call(&user, "job_output", json!({"job_id":id})).await?;
            ensure!(model["text"] == "next", "{model}");
            let consumed = invoke(&user, "job_observer", &format!("read {id}")).await?;
            ensure!(consumed["chunks"] == json!([]), "{consumed}");

            let tail = command(&user, "append-tail").await?;
            ensure!(tail == json!({
                "chunks":[{"at":240,"text":format!("{}z", "😀".repeat(255)), "channel":"stdout", "gapBefore":true}],
                "next":1261,"lossy":true,
            }), "{tail}");
            let settled = command(&user, &format!("wait {id}")).await?;
            ensure!(settled["job"]["status"] == "completed", "{settled}");
            case.restart().await?;
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().attach_session(user.id()).await?;
            let retained = invoke(&user, "job_observer", &format!("at {id} 0")).await?;
            ensure!(retained == tail, "{retained}");

            let first = invoke(&user, "job_observer", &format!("read {id}")).await?;
            ensure!(first["chunks"] == tail["chunks"] && first["lossy"] == true
                && first["result"] == "packet result", "{first}");
            ensure!(first["job"]["output"] == json!({"earliest":1248,"total":1261}), "{first}");
            let repeated = invoke(&user, "job_observer", &format!("read {id}")).await?;
            ensure!(repeated["chunks"] == json!([]) && repeated.get("result").is_none(), "{repeated}");
            ensure!(call(&user, "job_output", json!({"job_id":id})).await?["text"] == "");
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cordis_jobs_starts_and_controls_installed_javascript_producers() -> TestResult {
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
            for mode in ["forged-owner", "starter-error"] {
                user.call_tool(format!("{PLUGIN}.produce"), json!({"mode": mode}))
                    .await?
                    .assert_failed()?;
            }
            ensure!(command(&user, "").await?["started"] == 1);
            ensure!(call(&user, "job_list", json!({})).await?["jobs"] == json!([]));

            let (id, output) = start(&user, "complete").await?;
            for mode in ["tool-error", "cap-exceeded"] {
                user.call_tool(format!("{PLUGIN}.produce"), json!({"mode": mode}))
                    .await?
                    .assert_failed()?;
            }
            ensure!(command(&user, "").await?["started"] == 2);
            ensure!(call(&other, "job_list", json!({})).await?["jobs"] == json!([]));
            other
                .call_tool(format!("{PLUGIN}.job_kill"), json!({"job_id": id}))
                .await?
                .assert_failed()?;

            let active = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let listing = call(&user, "job_list", json!({})).await?;
                    if listing["jobs"][0]["progress"] == "waiting" {
                        return Ok::<_, anyhow::Error>(listing["jobs"][0].clone());
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await??;
            ensure!(
                active["id"] == id && active["label"] == "Producer: complete",
                "{active}"
            );
            ensure!(
                active["kind"] == "fixture:worker" && active["owner"] == user.id().to_string(),
                "{active}"
            );
            let running = call(
                &user,
                "job_output",
                json!({"job_id": id, "wait": true, "timeout_ms": 5}),
            )
            .await?;
            let text = running["text"]
                .as_str()
                .context("running producer output")?;
            ensure!(running["job"]["status"] == "running", "{running}");
            for expected in [
                "producer staged output",
                "resumed after the originating call",
                "first source chunk",
            ] {
                ensure!(text.contains(expected), "{running}");
            }
            ensure!(!text.contains("private job narration"), "{running}");

            command(&user, "release").await?;
            wait_for_settlement(&user, output).await?;
            let page = user
                .client()
                .read_content(user.id(), output, None, 1)
                .await?;
            let parts = user.parts().await?;
            let part = parts
                .iter()
                .find(|part| part.part_id == page.resource.part_id)
                .context("producer launch in the transcript")?;
            let outputs: Vec<ContentRef> =
                serde_json::from_value(part.content["resources"].clone())?;
            let log = outputs
                .iter()
                .find(|source| source.kind == ContentKind::Log)
                .context("producer narration stream")?;
            let log = user
                .client()
                .read_content(user.id(), log.resource_id, None, 4096)
                .await?;
            ensure!(serde_json::to_string(&log.chunks)?.contains("private job narration"));
            let complete = call(&user, "job_output", json!({"job_id": id, "wait": true})).await?;
            let text = complete["text"]
                .as_str()
                .context("finished producer output")?;
            ensure!(
                complete["job"]["status"] == "completed"
                    && complete["job"]["detail"] == "producer complete",
                "{complete}"
            );
            ensure!(
                text.contains("[source output omitted]")
                    && text.contains("final source chunk")
                    && text.contains("producer terminal result"),
                "{complete}"
            );
            ensure!(call(&user, "job_output", json!({"job_id": id})).await?["text"] == "");

            let (failed, output) = start(&user, "done-error").await?;
            command(&user, "release").await?;
            wait_for_settlement(&user, output).await?;
            let result = call(&user, "job_output", json!({"job_id": failed, "wait": true})).await?;
            ensure!(result["job"]["status"] == "failed", "{result}");
            ensure!(
                result["job"]["detail"]
                    .as_str()
                    .context("producer rejection")?
                    .contains("fixture asynchronous failure"),
                "{result}"
            );

            ensure!(invoke(&user, "job_observer", "").await?["events"] == json!([]));
            let (killed, _) = start(&user, "cancel").await?;
            let kill = call(
                &user,
                "job_kill",
                json!({"job_id": killed, "reason": "fixture stop"}),
            )
            .await?;
            ensure!(kill["outcome"] == "cancellation-requested", "{kill}");
            let result = call(&user, "job_output", json!({"job_id": killed, "wait": true})).await?;
            ensure!(result["job"]["status"] == "killed", "{result}");
            ensure!(
                result["text"]
                    .as_str()
                    .context("cancelled producer output")?
                    .contains("cancel hook observed: fixture stop"),
                "{result}"
            );
            ensure!(command(&user, "").await?["stopReason"] == "fixture stop");
            let cancelled = observed_settlement(&user, &killed).await?;
            let events = cancelled["events"]
                .as_array()
                .context("cancelled job events")?;
            ensure!(
                events
                    .iter()
                    .any(|event| event["type"] == "stopping" && event["status"] == "stopping"),
                "{cancelled}"
            );
            ensure!(
                events.iter().any(|event| event["type"] == "settled"
                    && event["cause"] == "kill"
                    && event["status"] == "killed"),
                "{cancelled}"
            );
            invoke(&user, "job_observer", "dispose").await?;

            let (interrupted, _) = start(&user, "interrupt").await?;
            ensure!(invoke(&user, "job_observer", "").await?["events"] == cancelled["events"]);
            case.crash().await?;
            case.restart().await?;
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().attach_session(user.id()).await?;
            let result = call(&user, "job_output", json!({"job_id": interrupted})).await?;
            ensure!(
                result["job"]["id"] == interrupted && result["job"]["status"] == "failed",
                "{result}"
            );
            ensure!(result["job"]["finishedAt"].is_i64(), "{result}");
            let completed = call(&user, "job_output", json!({"job_id": id})).await?;
            ensure!(
                completed["job"]["status"] == "completed" && completed["text"] == "",
                "{completed}"
            );
            ensure!(
                case.model()
                    .requests()
                    .iter()
                    .all(|request| !request.body.to_string().contains("private job narration")),
                "the provider received observer-only job narration"
            );
            Ok(())
        })
        .await
}
