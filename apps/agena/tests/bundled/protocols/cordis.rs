use std::time::Duration;

use agena_api::{
    management::PluginCommandComposer,
    resource::{
        CommandRequestContext, PluginCommandInvokeRequest, RunOptions, RuntimeBackgroundTaskKind,
        RuntimeBackgroundTaskStatus, WorkspaceFileUploadResource,
    },
};
use agena_plugin_sdk::{CommandHostEffect, CommandStatus};
use agena_testkit::{Reply, RequestMatch, TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

mod background_workflows;
mod command_jobs;
mod fork_workflows;
mod job_producers;
mod services;
mod skills;
mod storage;
mod subagents;

const PLUGIN: &str = "deepseek.fixture-cordis";
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DA8J8BAAf/Af8Bf4mnAAAAAElFTkSuQmCC";

async fn install(case: TestCase) -> TestResult {
    install_with_hooks(case, false).await
}

async fn install_with_hooks(case: TestCase, hooks: bool) -> TestResult {
    let root = case.paths().root.join("cordis-plugin");
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::write(
        root.join("package.json"),
        br#"{"name":"@fixture/cordis","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    )
    .await?;
    tokio::fs::write(
        root.join("bundle.yml"),
        "- insert:\n    - name: ./commands.mjs\n",
    )
    .await?;
    tokio::fs::write(
        root.join("commands.mjs"),
        r#"
import { createUserMessage } from '@deepseek-ai/dsh-llm';
import { CommandDefinitionId } from '@deepseek-ai/dsh-commands/brand';
import { defineTool } from '@deepseek-ai/dsh-tools';

export const inject = ['commands', 'tools', 'systemPrompt'];
export function apply(ctx) {
  let count = 0;
  const order = ctx.systemPrompt.getSectionOrder('FILE_REFERENCE');
  ctx.systemPrompt.section({name: 'cordis:late', order: order + 1,
    text: 'Cordis literal {{missing}}', interpolate: false});
  ctx.systemPrompt.section({name: 'cordis:early', order,
    text: ({signal}) => {
      signal.throwIfAborted();
      return ctx.tools.get('count') ? 'Cordis count {{fixture_count}}' : '';
    },
  });
  ctx.systemPrompt.variable('fixture_count', () => String(count));
  const dispose = ctx.systemPrompt.section({name: 'cordis:retired', order: order + 2, text: 'Cordis retired fixture'});
  dispose();
  ctx.tools.register(defineTool({
    name: 'count', description: 'Count text passed to this Cordis module.',
    parameters: { item: { type: 'object', required: true, additionalProperties: false,
      properties: { text: { type: 'string', required: true } } } },
    output: {
      schema: { type: 'object', additionalProperties: false, properties: {
        text: { type: 'string', required: true }, count: { type: 'integer', required: true },
      } },
      render: (_args, value) => [{type: 'text', text: `Cordis: ${value.text} (${value.count})`}],
    },
    async execute(args, exec) {
      exec.signal.throwIfAborted();
      if (exec.name !== 'count' || !exec.callId || !Object.isFrozen(args.item)) {
        throw new Error('Unexpected Cordis execution context');
      }
      return { text: args.item.text, count: ++count };
    },
  }));
  ctx.commands.register({name: 'status', description: 'Read the Cordis tool count',
    handler() { return {kind: 'success', text: String(count)}; },
  });
  ctx.commands.register({
    name: 'plain', description: 'Legacy text command', input: {hint: '<text>'},
    handler({ attachments, agent, rawInput }) {
      if (attachments.length !== 0) return {kind: 'error', text: 'Unexpected attachments'};
      agent.followup(createUserMessage({source: {kind: 'user'}, content: [{type: 'text', text: rawInput}]}));
      return {kind: 'success', text: 'Legacy command accepted'};
    },
  });
  ctx.commands.register({
    definitionId: CommandDefinitionId('@fixture/cordis/compose'),
    name: 'compose', description: 'Inspect or forward attachments', input: {hint: '<mode>', attachments: true},
    handler({ attachments, agent, rawInput }) {
      const followup = content => agent.followup(createUserMessage({source: {kind: 'user'}, content}));
      if (rawInput === 'fail') {
        followup([attachments[0]]);
        return {kind: 'error', text: 'Keep this draft'};
      }
      if (rawInput === 'forge') {
        const block = attachments[0];
        followup([{...block, attachment: {...block.attachment, attachmentId: 'sha256:' + '0'.repeat(64)}}]);
        return {kind: 'success'};
      }
      if (rawInput === 'image-only') followup([attachments[0]]);
      if (rawInput === 'forward') followup([
        attachments[2], {type: 'text', text: 'selected Cordis attachments'}, attachments[1],
      ]);
      return {kind: 'success', text: JSON.stringify(attachments)};
    },
  });
}
"#,
    )
    .await?;
    let mut install = vec!["plugin", "install", root.to_str().context("plugin path")?];
    if hooks {
        install.push("--enable-hooks");
    }
    case.run_cli(install).await?.require_success()?;
    Ok(())
}

pub(super) async fn wait_for_startup_catalog(case: &TestCase) -> TestResult {
    // The first catalog refresh publishes a new runtime snapshot before its
    // task completes. The HTTP lifecycle assertions need a stable generation.
    let client = case.client().await;
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if client
                .runtime_status()
                .await?
                .background_tasks
                .iter()
                .all(|task| {
                    task.kind != RuntimeBackgroundTaskKind::ModelCatalogRefresh
                        || task.status != RuntimeBackgroundTaskStatus::Running
                })
            {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .context("startup model catalog refresh did not settle")?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_cordis_tools_share_command_state_in_the_owning_session() -> TestResult {
    async fn count(user: &UserSession) -> anyhow::Result<String> {
        let result = user
            .client()
            .invoke_plugin_command(PLUGIN, "status", json!({}), Some(user.id()), None, "")
            .await?
            .result;
        ensure!(result.status == CommandStatus::Succeeded, "{result:?}");
        Ok(result.summary)
    }
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            super::super::allow_permissions(&user).await?;
            super::super::allow_permissions(&other).await?;
            wait_for_startup_catalog(&case).await?;
            ensure!(count(&user).await? == "0");
            let tool = format!("{PLUGIN}.count");
            user.call_tool(&tool, json!({"item":{}}))
                .await?
                .assert_failed()?;
            for (number, text) in [(1, "alpha"), (2, "beta")] {
                let result = user.call_tool(&tool, json!({"item":{"text":text}})).await?;
                result.assert_succeeded()?;
                ensure!(result.data()? == &json!({"text":text,"count":number}));
                ensure!(count(&user).await? == number.to_string());
            }
            ensure!(count(&other).await? == "0");
            user.client().clear_session(user.id(), false).await?;
            ensure!(count(&user).await? == "0");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_cordis_prompt_sections_use_live_state_and_session_policy() -> TestResult {
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(|case| install_with_hooks(case, true))
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            super::super::allow_permissions(&user).await?;
            super::super::allow_permissions(&other).await?;
            other
                .client()
                .set_session_permission(
                    other.id(),
                    serde_json::from_value(json!({
                        "plugins": { (PLUGIN): false }, "tools": {"default":"allow"},
                    }))?,
                )
                .await?;
            wait_for_startup_catalog(&case).await?;
            user.call_tool(format!("{PLUGIN}.count"), json!({"item":{"text":"ready"}}))
                .await?
                .assert_succeeded()?;
            case.model()
                .expect(RequestMatch::user_text("enabled cordis prompt"))
                .check(|request| {
                    let instructions = request.body["instructions"]
                        .as_str()
                        .context("model system prompt")?;
                    ensure!(
                        instructions.contains("Cordis count 1\n\nCordis literal {{missing}}"),
                        "{instructions}"
                    );
                    ensure!(!instructions.contains("Cordis retired fixture"));
                    Ok(())
                })
                .reply(Reply::text("visible"));
            user.send("enabled cordis prompt")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("visible")?;
            case.model()
                .expect(RequestMatch::user_text("disabled cordis prompt"))
                .check(|request| {
                    let instructions = request.body["instructions"]
                        .as_str()
                        .context("model system prompt")?;
                    ensure!(
                        !instructions.contains("Cordis count")
                            && !instructions.contains("Cordis literal")
                    );
                    Ok(())
                })
                .reply(Reply::text("hidden"));
            other
                .send("disabled cordis prompt")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("hidden")?;
            Ok(())
        })
        .await
}

fn selected(file: &WorkspaceFileUploadResource, name: &str, kind: &str) -> Value {
    json!({
        "type": "activity",
        "activity": {
            "id": agena_domain::ActivityId::new(),
            "provenance": {"content_hash": file.sha256},
            "payload": {
                "activity_type": "resource",
                "kind": kind,
                "delivery": if kind == "image" { "model_input" } else { "reference" },
                "reference": {"reference_type": "workspace_path", "path": file.path},
                "name": name, "size_bytes": 999,
            }
        }
    })
}

fn invocation(
    user: &UserSession,
    mode: &str,
    nodes: Vec<Value>,
    options: RunOptions,
) -> anyhow::Result<PluginCommandInvokeRequest> {
    Ok(PluginCommandInvokeRequest {
        context: CommandRequestContext {
            input: json!({}),
            session_id: Some(user.id()),
            slash: None,
            raw: mode.to_owned(),
        },
        composer: Some(PluginCommandComposer {
            document: serde_json::from_value(json!(nodes))?,
            options,
        }),
    })
}

fn user_content(request: &agena_testkit::model::ModelRequest) -> Option<&[Value]> {
    request.body["input"]
        .as_array()?
        .iter()
        .rev()
        .find(|entry| entry["role"] == "user")?["content"]
        .as_array()
        .map(Vec::as_slice)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_cordis_attachments_work_through_the_session_command_endpoint() -> TestResult {
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .configure_global(|config| {
            config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["fake-model"]
                ["input"] = json!(["image", "text"]);
        })
        .before_start(install)
        .run(|case| async move {
            let user = case.user().new_session().await?;
            super::super::allow_permissions(&user).await?;
            let client = user.client();
            let catalog = client.plugin_surface_catalog_for_session(user.id()).await?;
            for (command, accepts) in [("compose", true), ("plain", false)] {
                ensure!(
                    catalog.catalog.commands.iter().any(|entry| {
                        entry.plugin_id.to_string() == PLUGIN
                            && entry.command.id == command
                            && entry.accepts_attachments == accepts
                    }),
                    "{command} attachment support missing from catalog"
                );
            }
            let ignored = client
                .upload_session_file(user.id(), "ignored.bin", "AAH/", None)
                .await?;
            let image = client
                .upload_session_file(user.id(), "screen.png", PNG, Some("image/png"))
                .await?;
            let file = client
                .upload_session_file(user.id(), "keep.bin", "AAj/", None)
                .await?;
            let nodes = vec![
                selected(&ignored, "ignored.bin", "file"),
                selected(&image, "screen.png", "image"),
                selected(&file, "../keep.bin", "file"),
            ];
            let request = |mode, options| invocation(&user, mode, nodes.clone(), options);
            let missing_model: RunOptions = serde_json::from_value(json!({
                "model":{"provider_id":"missing","model_id":"missing"}
            }))?;
            let inspected = client
                .invoke_plugin_command_request(PLUGIN, "compose", request("inspect", missing_model)?)
                .await?
                .result;
            ensure!(inspected.status == CommandStatus::Succeeded, "{inspected:?}");
            let attachments: Value = serde_json::from_str(&inspected.summary)?;
            ensure!(attachments == json!([
                {"type":"file","attachment":{
                    "attachmentId": format!("sha256:{}", ignored.sha256.as_deref().context("upload checksum")?),
                    "name":"ignored.bin","bytes":3,
                }},
                {"type":"image","attachment":{
                    "attachmentId": format!("sha256:{}", image.sha256.as_deref().context("upload checksum")?),
                    "name":"screen.png","bytes":image.size_bytes,
                    "mediaType":"image/png","width":2,"height":1,
                }},
                {"type":"file","attachment":{
                    "attachmentId": format!("sha256:{}", file.sha256.as_deref().context("upload checksum")?),
                    "name":"keep.bin","bytes":3,
                }},
            ]));
            for (command, mode) in [("plain", "inspect"), ("compose", "forge")] {
                ensure!(
                    client
                        .invoke_plugin_command_request(PLUGIN, command, request(mode, RunOptions::default())?)
                        .await
                        .is_err()
                );
            }
            let failed = client
                .invoke_plugin_command_request(PLUGIN, "compose", request("fail", RunOptions::default())?)
                .await?
                .result;
            ensure!(failed.status == CommandStatus::Failed && failed.summary == "Keep this draft");
            let other = case.user().new_session().await?;
            super::super::allow_permissions(&other).await?;
            ensure!(
                client
                    .invoke_plugin_command_request(
                        PLUGIN,
                        "compose",
                        invocation(&other, "inspect", nodes.clone(), RunOptions::default())?,
                    )
                    .await
                    .is_err(),
                "a different session admitted the uploaded file"
            );
            ensure!(user.parts().await?.is_empty());

            let stored = case.paths().data_dir
                .join(format!("session-assets/{}/artifacts/cordis-files", user.id()))
                .join(file.sha256.context("upload checksum")?)
                .join("keep.bin");
            let model_file = stored.clone();
            case.model()
                .expect(RequestMatch::user_text("selected Cordis attachments"))
                .check(move |request| {
                    let content = user_content(request).context("followup has no user content")?;
                    ensure!(content.len() == 3, "{content:?}");
                    let file = content[0]["text"].as_str().context("file reference was omitted")?;
                    ensure!(file.contains("keep.bin") && file.contains(&*model_file.to_string_lossy()));
                    ensure!(!request.user_text().contains("ignored.bin"));
                    ensure!(content[1]["text"] == "selected Cordis attachments");
                    ensure!(content[2]["image_url"] == format!("data:image/png;base64,{PNG}"));
                    Ok(())
                })
                .reply(Reply::text("Cordis media received"));
            let followed = client
                .invoke_plugin_command_request(PLUGIN, "compose", request("forward", RunOptions::default())?)
                .await?
                .result;
            ensure!(followed.status == CommandStatus::Succeeded);
            ensure!(followed.effects.is_empty() && followed.output.is_none());
            user.wait_text_contains("Cordis media received").await?;
            let stored = stored.to_str().context("snapshot path is not UTF-8")?;
            let (_, bytes) = client.download_session_media_file(user.id(), stored).await?;
            ensure!(bytes == [0, 8, 255]);

            let image = client
                .upload_session_file(other.id(), "screen.png", PNG, Some("image/png"))
                .await?;
            case.model()
                .expect(RequestMatch::new("Cordis image-only followup", |request| {
                    user_content(request).is_some_and(|content| {
                        content.len() == 1 && content[0]["image_url"] == format!("data:image/png;base64,{PNG}")
                    })
                }))
                .reply(Reply::text("Cordis image-only received"));
            let image_only = client
                .invoke_plugin_command_request(
                    PLUGIN,
                    "compose",
                    invocation(
                        &other, "image-only", vec![selected(&image, "screen.png", "image")], RunOptions::default(),
                    )?,
                )
                .await?
                .result;
            ensure!(image_only.status == CommandStatus::Succeeded);
            other.wait_text_contains("Cordis image-only received").await?;

            let plain = client
                .invoke_plugin_command(PLUGIN, "plain", json!({}), Some(user.id()), None, "legacy text")
                .await?
                .result;
            ensure!(plain.effects == [CommandHostEffect::InsertPrompt { prompt: "legacy text".into() }]);
            Ok(())
        })
        .await
}

const COMPLETE_PROMPT_PLUGIN: &str = "deepseek.fixture-complete-prompt";

async fn install_complete_prompt(case: TestCase) -> TestResult {
    let root = case.paths().root.join("cordis-complete-prompt");
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::create_dir_all(&root).await?;
    tokio::fs::write(
        root.join("package.json"),
        br#"{"name":"@fixture/complete-prompt","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    )
    .await?;
    tokio::fs::write(
        root.join("bundle.yml"),
        "- insert:\n    - name: ./prompt.mjs\n",
    )
    .await?;
    tokio::fs::write(
        root.join("prompt.mjs"),
        r#"
export const inject = ['commands', 'systemPrompt'];
export function apply(ctx) {
  let mode = 'normal';
  ctx.systemPrompt.variable('mode_label', () => mode);
  ctx.systemPrompt.section({
    name: 'cordis:normal',
    order: 100,
    text: () => 'Normal Cordis section {{mode_label}}',
  });
  ctx.systemPrompt.section({
    name: 'cordis:complete-a',
    order: 200,
    complete: true,
    text: () => (mode === 'single' || mode === 'conflict' ? 'Complete replacement prompt ({{mode_label}})' : ''),
  });
  ctx.systemPrompt.section({
    name: 'cordis:complete-b',
    order: 300,
    complete: true,
    text: () => (mode === 'conflict' ? 'Second complete prompt' : ''),
  });
  ctx.commands.register({
    name: 'mode',
    description: 'Switch complete prompt mode',
    handler({ rawInput }) {
      mode = rawInput.trim() || 'normal';
      return { kind: 'success', text: mode };
    },
  });
}
"#,
    )
    .await?;
    case.run_cli([
        "plugin",
        "install",
        root.to_str().context("complete prompt path")?,
        "--enable-hooks",
    ])
    .await?
    .require_success()?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_cordis_complete_prompt_replaces_system_prompt_and_rejects_conflicts() -> TestResult
{
    async fn set_mode(user: &UserSession, mode: &str) -> anyhow::Result<()> {
        let reply = user
            .client()
            .invoke_plugin_command(
                COMPLETE_PROMPT_PLUGIN,
                "mode",
                json!({}),
                Some(user.id()),
                None,
                mode,
            )
            .await?
            .result;
        ensure!(reply.status == CommandStatus::Succeeded && reply.summary == mode, "{reply:?}");
        Ok(())
    }

    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(COMPLETE_PROMPT_PLUGIN)
        .before_start(install_complete_prompt)
        .run(|case| async move {
            let user = case.user().new_session().await?;
            super::super::allow_permissions(&user).await?;
            wait_for_startup_catalog(&case).await?;

            // Inactive complete sections fall back to normal appended sections.
            case.model()
                .expect(RequestMatch::user_text("normal mode prompt"))
                .check(|request| {
                    let instructions = request.body["instructions"]
                        .as_str()
                        .context("model system prompt")?;
                    ensure!(
                        instructions.contains("Normal Cordis section normal")
                            && instructions.len() > "Normal Cordis section normal".len(),
                        "{instructions}"
                    );
                    Ok(())
                })
                .reply(Reply::text("normal ok"));
            user.send("normal mode prompt")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("normal ok")?;

            // Single active complete section replaces the entire system prompt.
            set_mode(&user, "single").await?;
            case.model()
                .expect(RequestMatch::user_text("complete mode prompt"))
                .check(|request| {
                    let instructions = request.body["instructions"]
                        .as_str()
                        .context("model system prompt")?;
                    ensure!(
                        instructions == "Complete replacement prompt (single)",
                        "{instructions}"
                    );
                    Ok(())
                })
                .reply(Reply::text("complete ok"));
            user.send("complete mode prompt")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("complete ok")?;

            // Multiple active complete sections fail prompt assembly.
            set_mode(&user, "conflict").await?;
            user.send("conflict mode prompt")
                .await?
                .wait_settled()
                .await?
                .assert_failed()?;
            Ok(())
        })
        .await
}

const MEDIA_NODE_PLUGIN: &str = "deepseek.fixture-media-node";

async fn install_media_node(case: TestCase) -> TestResult {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    let root = case.paths().root.join("cordis-media-node");
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::create_dir_all(root.join("assets")).await?;
    let png_bytes = STANDARD.decode(PNG)?;
    tokio::fs::write(root.join("assets/badge.png"), &png_bytes).await?;
    tokio::fs::write(
        root.join("package.json"),
        br#"{"name":"@fixture/media-node","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
    )
    .await?;
    tokio::fs::write(
        root.join("bundle.yml"),
        "- insert:\n    - name: ./media.mjs\n",
    )
    .await?;
    tokio::fs::write(
        root.join("media.mjs"),
        r#"
import path from 'node:path';
import crypto from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { fileURLToPath, pathToFileURL, URLSearchParams } from 'node:url';
import { Buffer } from 'node:buffer';
import { defineTool } from '@deepseek-ai/dsh-tools';

export const inject = ['tools'];
export function apply(ctx) {
  ctx.tools.register(defineTool({
    name: 'render_badge',
    description: 'Read a bundled PNG with Node APIs and return an image block.',
    parameters: {
      spoof: { type: 'boolean' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          sha256: { type: 'string', required: true },
          joined: { type: 'string', required: true },
          params: { type: 'string', required: true },
          roundTrip: { type: 'string', required: true },
          same: { type: 'boolean', required: true },
          uuidLen: { type: 'integer', required: true },
          randLen: { type: 'integer', required: true },
          pngBase64: { type: 'string', required: true },
          spoof: { type: 'boolean', required: true },
        },
      },
      render: (_args, value) => [
        { type: 'text', text: `Badge digest ${value.sha256}` },
        value.spoof
          ? { type: 'image', mediaType: 'image/png', data: Buffer.from('not-a-png').toString('base64') }
          : { type: 'image', mediaType: 'image/png', data: Buffer.from(value.pngBase64, 'base64'), filename: 'badge.png' },
      ],
    },
    async execute(args) {
      const dir = path.dirname(fileURLToPath(import.meta.url));
      const assetPath = path.join(dir, 'assets', 'badge.png');
      const assetUrl = pathToFileURL(assetPath);
      const bytes = await readFile(assetUrl);
      if (!Buffer.isBuffer(bytes)) throw new Error('Expected Buffer from binary readFile');
      const sha256 = crypto.createHash('sha256').update(bytes).digest('hex');
      const encoded = new TextEncoder().encode('hello-cordis');
      const roundTrip = new TextDecoder().decode(encoded);
      const same = crypto.timingSafeEqual(Buffer.from('abc'), Buffer.from('abc'));
      const params = new URLSearchParams({ a: '1', b: 'two words' }).toString();
      return {
        sha256,
        joined: path.relative(dir, assetPath),
        params,
        roundTrip,
        same,
        uuidLen: crypto.randomUUID().length,
        randLen: crypto.randomBytes(16).length,
        pngBase64: bytes.toString('base64'),
        spoof: args.spoof === true,
      };
    },
  }));
}
"#,
    )
    .await?;
    case.run_cli([
        "plugin",
        "install",
        root.to_str().context("media-node path")?,
    ])
    .await?
    .require_success()?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_cordis_multimedia_tool_results_and_node_builtins() -> TestResult {
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(MEDIA_NODE_PLUGIN)
        .configure_global(|config| {
            config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["fake-model"]
                ["input"] = json!(["image", "text"]);
        })
        .before_start(install_media_node)
        .run(|case| async move {
            let user = case.user().new_session().await?;
            super::super::allow_permissions(&user).await?;
            wait_for_startup_catalog(&case).await?;

            let tool = format!("{MEDIA_NODE_PLUGIN}.render_badge");
            // Spoofed PNG payload must be rejected by host media validation.
            user.call_tool(&tool, json!({"spoof": true}))
                .await?
                .assert_failed()?;

            // Direct tool call verifies structured Node builtin outputs.
            let direct = user.call_tool(&tool, json!({"spoof": false})).await?;
            direct.assert_succeeded()?;
            let data = direct.data()?;
            ensure!(
                data["sha256"]
                    == "78253200c249d4d84293b2a1e6fc803adedeba8645f38d6bc2f6734a2dfacec9",
                "{data}"
            );
            ensure!(data["joined"] == "assets/badge.png", "{data}");
            ensure!(data["roundTrip"] == "hello-cordis", "{data}");
            ensure!(data["params"] == "a=1&b=two+words", "{data}");
            ensure!(data["same"] == true && data["uuidLen"] == 36 && data["randLen"] == 16, "{data}");

            // Valid multimedia tool result delivers the rendered PNG attachment to the model.
            case.model()
                .expect(RequestMatch::user_text("inspect bundled badge"))
                .reply(Reply::tool_call("badge-call", &tool, json!({"spoof": false})));
            case.model()
                .expect(RequestMatch::tool_output("badge-call"))
                .check(|request| {
                    let wire = request.body.to_string();
                    ensure!(wire.contains("assets/badge.png"), "{wire}");
                    ensure!(wire.contains("hello-cordis"), "{wire}");
                    ensure!(wire.contains("a=1&b=two+words"), "{wire}");
                    ensure!(
                        wire.contains(&format!("data:image/png;base64,{PNG}")),
                        "expected rendered PNG attachment in model input: {wire}"
                    );
                    Ok(())
                })
                .reply(Reply::text("badge inspected"));
            user.send("inspect bundled badge")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("badge inspected")?;
            Ok(())
        })
        .await
}
