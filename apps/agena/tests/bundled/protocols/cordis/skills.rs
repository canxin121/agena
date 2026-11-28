use std::path::Path;

use agena_api::skills::SkillSummary;
use agena_domain::{
    ActivityId, ActivityPayload, ActivityProvenance, CommandReferenceActivity, ComposerActivity,
    ComposerDocument, ComposerNode,
};
use agena_plugin_sdk::CommandStatus;
use agena_testkit::{Reply, RequestMatch, TestCase, TestResult, UserSession};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

const PLUGIN: &str = "deepseek.fixture-provider-bundle";

async fn install(case: TestCase) -> TestResult {
    let root = case.paths().root.join("provider-bundle");
    let dependency = root.join("node_modules/@fixture/provider");
    let names = root.join("node_modules/fixture-names");
    tokio::fs::create_dir_all(dependency.join("assets")).await?;
    tokio::fs::create_dir_all(names.join("lib")).await?;
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::write(
        root.join("package.json"),
        br#"{
      "name":"@fixture/provider-bundle","version":"1.2.3",
      "dsh":{"bundle":{"patch":"bundle.yml"}},
      "dependencies":{"@fixture/provider":"^1.0.0", "fixture-names":"1.0.0"}
    }"#,
    )
    .await?;
    tokio::fs::write(
        root.join("bundle.yml"),
        "- insert:\n    - name: '@fixture/provider/register'\n",
    )
    .await?;
    tokio::fs::write(
        dependency.join("package.json"),
        br#"{
      "name":"@fixture/provider","version":"1.0.1",
      "exports":{"./register":{"import":"./register.mjs"}},
      "dependencies":{"fixture-names":"^1.0.0"}
    }"#,
    )
    .await?;
    tokio::fs::write(
        names.join("package.json"),
        br#"{
      "name":"fixture-names","version":"1.0.0","exports":{"./names":"./lib/names.mjs"}
    }"#,
    )
    .await?;
    tokio::fs::write(
        names.join("lib/names.mjs"),
        "export const title = 'Fixture provider';\n",
    )
    .await?;
    tokio::fs::write(
        dependency.join("assets/body.md"),
        "Use the instructions from the provider.\n",
    )
    .await?;
    tokio::fs::write(
        dependency.join("assets/reference.txt"),
        "Provider resource\n",
    )
    .await?;
    tokio::fs::write(dependency.join("register.mjs"), r#"
import {readFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {BUNDLED_SKILL_RANK} from '@deepseek-ai/dsh-skill';
import {title} from 'fixture-names/names';

export const inject = ['skills', 'commands', 'mcpManager'];
const dir = {kind: 'directory', path: fileURLToPath(new URL('./assets/', import.meta.url))};
const file = new URL('./assets/body.md', import.meta.url);
const candidate = {
    name: 'provider-demo', description: title, source: 'bundled', provider: 'fixture',
    invocation: {modelInvocable: true, userInvocable: true},
    resourceBase: dir, rank: BUNDLED_SKILL_RANK, locator: file,
};
export function apply(ctx) {
    let edition = 0;
    let disposeManual;
    const registerManual = () => {
        disposeManual?.();
        disposeManual = ctx.skills.register({name: 'manual-only', description: 'Human invocation only',
            content: 'Manual skill edition ' + edition, invocation: {modelInvocable: false, userInvocable: true}});
    };
    registerManual();
    ctx.skills.register({name: 'model-only', description: 'Model invocation only',
        content: 'Model skill instructions', invocation: {modelInvocable: true, userInvocable: false}});
    ctx.commands.register({name: 'advance', description: 'Advance the provider edition',
        handler() {
            edition += 1;
            registerManual();
            return {kind: 'success', text: String(edition)};
        },
    });
    ctx.commands.register({name: 'mcp-status', description: 'Read MCP availability',
        handler() {
            let text = 'available';
            try { ctx.mcpManager.list(); } catch { text = 'denied'; }
            return {kind: 'success', text};
        },
    });
    const dispose = ctx.skills.register({name: 'retired-skill', description: 'Removed', content: 'Removed skill'});
    dispose();
    ctx.skills.registerProvider(({signal}) => ({
        name: 'fixture',
        async list({signal: lookup}) {
            signal.throwIfAborted();
            lookup.throwIfAborted();
            return {candidates: [candidate], complete: true};
        },
        async get(selected, {cwd}) {
            if (selected !== candidate) throw new Error('Provider candidate was not retained');
            let confined = false;
            try { await readFile(new URL('../../../../outside-cordis-package', import.meta.url), 'utf8'); }
            catch (error) { confined = String(error).includes('outside its package'); }
            if (!confined) throw new Error('Expected a package-local read');
            return {...selected, content: (await readFile(selected.locator, 'utf8'))
                + '\nLookup: ' + cwd + '\nProvider edition ' + edition};
        },
    }));
}
"#).await?;
    let path = root.to_str().context("Cordis package directory")?;
    let command = [
        "plugin",
        "install",
        "deepseek:@fixture/provider-bundle",
        "--package-source",
        path,
    ];
    let preview = case
        .run_cli(command.into_iter().chain(["--dry-run"]))
        .await?;
    ensure!(
        preview
            .require_success()?
            .stdout
            .contains("Cordis skills (3)"),
        "{preview:?}"
    );
    case.run_cli(command).await?.require_success()?;
    Ok(())
}

async fn command_text_for(user: &UserSession, plugin: &str, command: &str, text: &str) -> TestResult {
    let result = user
        .client()
        .invoke_plugin_command(plugin, command, json!({}), Some(user.id()), None, "")
        .await?
        .result;
    ensure!(
        result.status == CommandStatus::Succeeded && result.summary == text,
        "{result:?}"
    );
    Ok(())
}

async fn command_text(user: &UserSession, command: &str, text: &str) -> TestResult {
    command_text_for(user, PLUGIN, command, text).await
}

async fn advance(user: &UserSession, edition: &str) -> TestResult {
    command_text(user, "advance", edition).await
}

fn selected(text: &str, skill: &SkillSummary, provider: &str) -> ComposerDocument {
    ComposerDocument(vec![
        ComposerNode::Text {
            text: text.to_owned(),
        },
        ComposerNode::Activity {
            activity: Box::new(ComposerActivity {
                id: ActivityId::new(),
                provenance: ActivityProvenance::default(),
                payload: ActivityPayload::CommandReference(CommandReferenceActivity {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    content_hash: skill.revision.clone(),
                    source: provider.to_owned(),
                    skill_id: Some(skill.id.clone()),
                    aliases: Vec::new(),
                }),
            }),
        },
    ])
}

async fn session_skill(user: &UserSession, name: &str) -> anyhow::Result<SkillSummary> {
    user.client()
        .list_session_skills(user.id())
        .await?
        .skills
        .into_iter()
        .find(|skill| skill.name == name)
        .with_context(|| format!("missing session skill {name}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_cordis_provider_projects_installed_skills_and_assets() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            let http = case.http().await;
            let catalog: Value = http
                .get("/api/v1/skills")
                .await?
                .require_success()?
                .json()?;
            let items = catalog["skills"].as_array().context("skills catalog")?;
            let demo = items
                .iter()
                .find(|item| item["name"] == "provider-demo")
                .context("provider skill")?;
            let id = demo["id"].as_str().context("provider skill ID")?;
            ensure!(
                demo["source"]["location"] == PLUGIN && demo["editable"] == false,
                "{demo:#}"
            );
            let manual = items
                .iter()
                .find(|item| item["name"] == "manual-only")
                .context("user-only skill")?;
            ensure!(manual["invocation"]["modelInvocable"] == false);
            ensure!(items.iter().all(|item| item["name"] != "retired-skill"));

            let listing = user
                .call_tool("agena.skills.list", json!({"query":"provider"}))
                .await?;
            listing.assert_succeeded()?;
            ensure!(
                listing.data()?["skills"]
                    .as_array()
                    .context("tool skill catalog")?
                    .iter()
                    .any(|item| item["id"] == id)
            );
            user.call_tool("agena.skills.get", json!({"id": manual["id"]}))
                .await?
                .assert_failed()?;

            let result = user.call_tool("agena.skills.get", json!({"id":id})).await?;
            result.assert_succeeded()?;
            result.assert_text_contains("Use the instructions from the provider.")?;
            let detail = result.data()?;
            let cwd = case.paths().workspace.display().to_string();
            ensure!(
                detail["document"]
                    .as_str()
                    .context("skill body")?
                    .contains(&format!("Lookup: {cwd}")),
                "{detail:#}"
            );
            let base = detail["resource_base"]["path"]
                .as_str()
                .context("skill resource base")?;
            ensure!(
                Path::new(base).ends_with("node_modules/@fixture/provider/assets")
                    && !base.contains(".install-"),
                "{base}"
            );
            ensure!(
                detail["files"]
                    .as_array()
                    .context("skill assets")?
                    .iter()
                    .any(|file| file["path"] == "reference.txt")
            );

            let resource = user
                .call_tool(
                    "agena.skills.read_resource",
                    json!({"id":id,"path":"reference.txt"}),
                )
                .await?;
            resource.assert_succeeded()?;
            ensure!(resource.data()?["content"] == "Provider resource\n");
            user.call_tool(
                "agena.skills.read_resource",
                json!({"id":id,"path":"../register.mjs"}),
            )
            .await?
            .assert_failed()?;

            let initial = session_skill(&user, "provider-demo").await?;
            ensure!(initial.revision == demo["revision"]);
            advance(&user, "1").await?;
            let changed = session_skill(&user, "provider-demo").await?;
            ensure!(changed.id == initial.id && changed.revision != initial.revision);
            let result = user.call_tool("agena.skills.get", json!({"id":id})).await?;
            result.assert_succeeded()?;
            result.assert_text_contains("Provider edition 1")?;

            let other = case.user().new_session().await?;
            other.client().set_session_permission(
                other.id(),
                serde_json::from_value(json!({
                    "path":{"workspace":"readwrite","external":"readwrite"},
                    "tools":{"default":"allow","names":{"agena.mcp.servers.status":"deny","mcp.servers.status":"deny"}},
                }))?,
            ).await?;
            ensure!(session_skill(&other, "provider-demo").await?.revision == initial.revision);
            command_text(&other, "mcp-status", "denied").await?;
            let result = other
                .call_tool("agena.skills.get", json!({"id":id}))
                .await?;
            result.assert_succeeded()?;
            result.assert_text_contains("Provider edition 0")?;
            advance(&other, "1").await?;
            super::super::super::allow_permissions(&other).await?;
            command_text(&other, "mcp-status", "available").await?;
            ensure!(session_skill(&other, "provider-demo").await?.revision == changed.revision);
            let workspace: Value = http
                .get("/api/v1/skills")
                .await?
                .require_success()?
                .json()?;
            ensure!(
                workspace["skills"]
                    .as_array()
                    .context("workspace skills")?
                    .iter()
                    .any(|skill| skill["id"] == id && skill["revision"] == initial.revision)
            );

            other
                .client()
                .set_session_permission(
                    other.id(),
                    serde_json::from_value(json!({
                        "plugins":{(PLUGIN):false}, "tools":{"default":"allow"},
                    }))?,
                )
                .await?;
            ensure!(session_skill(&other, "provider-demo").await.is_err());
            other
                .call_tool("agena.skills.get", json!({"id":id}))
                .await?
                .assert_failed()?;

            user.client().clear_session(user.id(), false).await?;
            ensure!(session_skill(&user, "provider-demo").await?.revision == initial.revision);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_cordis_user_skill_selection_is_pinned_through_queue_edits() -> TestResult {
    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .before_start(install)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;
            let initial = session_skill(&user, "manual-only").await?;
            advance(&user, "1").await?;
            let manual = session_skill(&user, "manual-only").await?;
            ensure!(manual.id == initial.id && manual.revision != initial.revision);
            let model_only = session_skill(&user, "model-only").await?;
            for (entry, origin, error) in [
                (&initial, PLUGIN, "changed"),
                (&manual, "deepseek.unrelated-owner", "unavailable"),
                (&model_only, PLUGIN, "unavailable"),
            ] {
                let rejected = user
                    .send_document(
                        selected("reject invalid skill reference", entry, origin),
                        Default::default(),
                    )
                    .await
                    .err()
                    .context("invalid skill selection was accepted")?;
                ensure!(format!("{rejected:#}").contains(error), "{rejected:#}");
            }

            let client = user.client();
            client.set_message_queue_paused(user.id(), true).await?;
            let removed = user
                .send_document(
                    selected("remove original selection", &manual, PLUGIN),
                    Default::default(),
                )
                .await?;
            let direct = user
                .send_document(
                    selected("keep the submitted selection", &manual, PLUGIN),
                    Default::default(),
                )
                .await?;
            let kept = user.send("replace original draft").await?;
            let removed_id = removed.receipt().execution_id.into();
            let kept_id = kept.receipt().execution_id.into();
            let removed_revision = client
                .queued_message_for_edit(user.id(), removed_id)
                .await?
                .revision;
            let kept_revision = client
                .queued_message_for_edit(user.id(), kept_id)
                .await?
                .revision;
            client
                .edit_queued_message(
                    user.id(),
                    removed_id,
                    removed_revision,
                    ComposerDocument(vec![ComposerNode::Text {
                        text: "selection removed from queued message".into(),
                    }]),
                )
                .await?;
            advance(&user, "2").await?;
            let revised = session_skill(&user, "manual-only").await?;
            client
                .edit_queued_message(
                    user.id(),
                    kept_id,
                    kept_revision,
                    selected(
                        "selection added to queued message",
                        &revised,
                        "agena.skills",
                    ),
                )
                .await?;
            advance(&user, "3").await?;

            let first = case
                .model()
                .expect(RequestMatch::user_text(
                    "selection removed from queued message",
                ))
                .check(|request| {
                    ensure!(!request.user_text().contains("Manual skill edition"));
                    Ok(())
                })
                .reply(Reply::text("selection removed"));
            let second = case
                .model()
                .expect(RequestMatch::user_text("keep the submitted selection"))
                .after(&first)
                .check(|request| {
                    let text = request.user_text();
                    ensure!(text.contains("Manual skill edition 1"), "{text}");
                    ensure!(
                        !text.contains("Manual skill edition 2")
                            && !text.contains("Manual skill edition 3"),
                        "{text}"
                    );
                    Ok(())
                })
                .reply(Reply::text("selected at submission"));
            case.model()
                .expect(RequestMatch::user_text("selection added to queued message"))
                .after(&second)
                .check(|request| {
                    let text = request.user_text();
                    ensure!(text.contains("Manual skill edition 2"), "{text}");
                    ensure!(
                        !text.contains("Manual skill edition 3")
                            && !text.contains("Read this skill through"),
                        "{text}"
                    );
                    Ok(())
                })
                .reply(Reply::text("selection loaded"));
            client.set_message_queue_paused(user.id(), false).await?;
            removed
                .wait_settled()
                .await?
                .assert_assistant_text("selection removed")?;
            direct
                .wait_settled()
                .await?
                .assert_assistant_text("selected at submission")?;
            kept.wait_settled()
                .await?
                .assert_assistant_text("selection loaded")?;
            user.call_tool("agena.skills.get", json!({"id":manual.id}))
                .await?
                .assert_failed()?;
            Ok(())
        })
        .await
}

const SLASH_PLUGIN: &str = "deepseek.fixture-slash-skills";

async fn install_slash_bundle(case: TestCase) -> TestResult {
    let root = case.paths().root.join("slash-skills-bundle");
    let skills = root.join("bundled/skills");
    for (dir, doc) in [
        (
            "disk-skill",
            "---\nname: disk-skill\ndescription: Bundled disk skill\ndisable-model-invocation: true\n---\nFollow bundled disk instructions.\n",
        ),
        (
            "disk-hidden",
            "---\nname: disk-hidden\ndescription: Hidden disk skill\nuser-invocable: false\n---\nNever invoke disk-hidden from slash.\n",
        ),
        (
            "override-allow",
            "---\nname: override-allow\ndescription: Disk version\n---\nShadowed disk override body.\n",
        ),
        (
            "override-deny",
            "---\nname: override-deny\ndescription: Disk version\nuser-invocable: true\n---\nShadowed disk fallback should not run.\n",
        ),
    ] {
        tokio::fs::create_dir_all(skills.join(dir)).await?;
        tokio::fs::write(skills.join(dir).join("SKILL.md"), doc).await?;
    }
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::write(
        root.join("package.json"),
        br#"{
      "name":"@fixture/slash-skills","version":"1.0.0",
      "exports":{"./register":{"import":"./register.mjs"}},
      "dsh":{"bundle":{"patch":"bundle.yml"}}
    }"#,
    )
    .await?;
    tokio::fs::write(
        root.join("bundle.yml"),
        r#"- insert:
    - name: '@fixture/slash-skills/register'
- id: skill-filesystem
  config:
    customSkillDirs: ["bundled/skills"]
"#,
    )
    .await?;
    tokio::fs::write(
        root.join("register.mjs"),
        r#"
export const inject = ['skills', 'commands'];
export function apply(ctx) {
    let edition = 0;
    let disposeManual;
    const registerManual = () => {
        disposeManual?.();
        disposeManual = ctx.skills.register({
            name: 'manual-only',
            description: 'Human invocation only',
            content: 'Manual skill edition ' + edition,
            invocation: {modelInvocable: false, userInvocable: true},
        });
    };
    registerManual();
    ctx.skills.register({
        name: 'model-only',
        description: 'Model invocation only',
        content: 'Model skill instructions',
        invocation: {modelInvocable: true, userInvocable: false},
    });
    ctx.skills.register({
        name: 'override-allow',
        description: 'Cordis wins over disk',
        content: 'Cordis override body',
        invocation: {modelInvocable: true, userInvocable: true},
    });
    ctx.skills.register({
        name: 'override-deny',
        description: 'Cordis blocks shadowed disk skill',
        content: 'Cordis hidden override',
        invocation: {modelInvocable: true, userInvocable: false},
    });
    ctx.commands.register({
        name: 'advance',
        description: 'Advance edition',
        handler() {
            edition += 1;
            registerManual();
            return {kind: 'success', text: String(edition)};
        },
    });
}
"#,
    )
    .await?;
    let path = root.to_str().context("slash bundle directory")?;
    case.run_cli([
        "plugin",
        "install",
        "deepseek:@fixture/slash-skills",
        "--package-source",
        path,
    ])
    .await?
    .require_success()?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_deepseek_slash_skills_resolve_cordis_and_disk_with_permissions_and_escaping()
-> TestResult {
    fn turn_text<'a>(full: &'a str, marker: &str) -> anyhow::Result<&'a str> {
        let index = full
            .rfind(marker)
            .with_context(|| format!("missing turn marker {marker:?} in {full}"))?;
        Ok(&full[index..])
    }

    super::super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(SLASH_PLUGIN)
        .before_start(install_slash_bundle)
        .run(|case| async move {
            super::wait_for_startup_catalog(&case).await?;
            let user = case.user().new_session().await?;
            super::super::super::allow_permissions(&user).await?;

            let disk_hidden = session_skill(&user, "disk-hidden").await?;
            ensure!(
                disk_hidden
                    .invocation
                    .is_some_and(|policy| !policy.user_invocable && policy.model_invocable),
                "{disk_hidden:?}"
            );
            let disk_skill = session_skill(&user, "disk-skill").await?;
            ensure!(
                disk_skill
                    .invocation
                    .is_some_and(|policy| policy.user_invocable && !policy.model_invocable),
                "{disk_skill:?}"
            );

            // 1. Normal message resolves Cordis + bundled disk skills, respects
            //    user-invocable, Cordis precedence over disk, and ignores paths/escapes.
            case.model()
                .expect(RequestMatch::user_text("resolve all slash cases"))
                .check(|request| {
                    let full = request.user_text();
                    let text = turn_text(&full, "resolve all slash cases")?;
                    ensure!(text.contains("Manual skill edition 0"), "{text}");
                    ensure!(text.contains("Follow bundled disk instructions."), "{text}");
                    ensure!(
                        text.contains("Resolve relative resource paths against")
                            && text.contains("bundled/skills/disk-skill"),
                        "{text}"
                    );
                    ensure!(text.contains("Cordis override body"), "{text}");
                    ensure!(!text.contains("Model skill instructions"), "{text}");
                    ensure!(!text.contains("Never invoke disk-hidden"), "{text}");
                    ensure!(!text.contains("Shadowed disk override body"), "{text}");
                    ensure!(!text.contains("Shadowed disk fallback"), "{text}");
                    ensure!(!text.contains("Cordis hidden override"), "{text}");
                    Ok(())
                })
                .reply(Reply::text("resolved slash skills"));
            user.send(
                "resolve all slash cases /manual-only /disk-skill /override-allow /model-only /disk-hidden /override-deny /usr/bin https://example.test/manual-only //manual-only",
            )
            .await?
            .wait_settled()
            .await?
            .assert_assistant_text("resolved slash skills")?;

            // 2. Selecting a Cordis skill chip and typing its /name deduplicates.
            let manual = session_skill(&user, "manual-only").await?;
            case.model()
                .expect(RequestMatch::user_text("dedup chip and typed"))
                .check(|request| {
                    let full = request.user_text();
                    let text = turn_text(&full, "dedup chip and typed")?;
                    ensure!(text.matches("Manual skill edition 0").count() == 1, "{text}");
                    Ok(())
                })
                .reply(Reply::text("deduplicated"));
            user.send_document(
                selected("dedup chip and typed /manual-only /manual-only", &manual, SLASH_PLUGIN),
                Default::default(),
            )
            .await?
            .wait_settled()
            .await?
            .assert_assistant_text("deduplicated")?;

            // 3. TUI // leading-slash escape (literal_leading_slash: true) suppresses
            //    only the leading token while still resolving later /tokens; literal //
            //    (Web) is also ignored.
            case.model()
                .expect(RequestMatch::user_text("and /disk-skill escaped"))
                .check(|request| {
                    let full = request.user_text();
                    let text = turn_text(&full, "/manual-only and /disk-skill escaped")?;
                    ensure!(!text.contains("Manual skill edition"), "{text}");
                    ensure!(text.contains("Follow bundled disk instructions."), "{text}");
                    Ok(())
                })
                .reply(Reply::text("escaped leading"));
            let response = user
                .client()
                .submit_message(agena_api::commands::SubmitRunParams {
                    session_id: user.id(),
                    options: Default::default(),
                    document: ComposerDocument(vec![ComposerNode::Text {
                        text: "/manual-only and /disk-skill escaped".into(),
                    }]),
                    prompt_command: None,
                    literal_leading_slash: true,
                })
                .await?;
            user.attach_run(response.receipt.context("receipt")?)
                .wait_settled()
                .await?
                .assert_assistant_text("escaped leading")?;

            case.model()
                .expect(RequestMatch::user_text("//disk-skill literal"))
                .check(|request| {
                    let full = request.user_text();
                    let text = turn_text(&full, "//disk-skill literal")?;
                    ensure!(!text.contains("Follow bundled disk instructions."), "{text}");
                    Ok(())
                })
                .reply(Reply::text("double slash literal"));
            user.send("//disk-skill literal")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("double slash literal")?;

            // 4. Session path permission denies external plugin disk skill while
            //    keeping in-process Cordis skill; disabling the plugin blocks both.
            user.client()
                .set_session_permission(
                    user.id(),
                    serde_json::from_value(json!({
                        "path": {"workspace": "readwrite", "external": "deny"},
                        "tools": {"default": "allow"},
                    }))?,
                )
                .await?;
            case.model()
                .expect(RequestMatch::user_text("external read denied"))
                .check(|request| {
                    let full = request.user_text();
                    let text = turn_text(&full, "external read denied")?;
                    ensure!(text.contains("Manual skill edition 0"), "{text}");
                    ensure!(!text.contains("Follow bundled disk instructions."), "{text}");
                    Ok(())
                })
                .reply(Reply::text("path denied"));
            user.send("external read denied /manual-only /disk-skill")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("path denied")?;

            user.client()
                .set_session_permission(
                    user.id(),
                    serde_json::from_value(json!({
                        "path": {"workspace": "readwrite", "external": "readwrite"},
                        "plugins": {(SLASH_PLUGIN): false},
                        "tools": {"default": "allow"},
                    }))?,
                )
                .await?;
            case.model()
                .expect(RequestMatch::user_text("plugin disabled"))
                .check(|request| {
                    let full = request.user_text();
                    let text = turn_text(&full, "plugin disabled")?;
                    ensure!(!text.contains("Manual skill edition"), "{text}");
                    ensure!(!text.contains("Follow bundled disk instructions."), "{text}");
                    Ok(())
                })
                .reply(Reply::text("plugin disabled"));
            user.send("plugin disabled /manual-only /disk-skill")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("plugin disabled")?;

            super::super::super::allow_permissions(&user).await?;

            // 5. Queue editing re-resolves typed /skills at edit time and clears
            //    removed /skills.
            let client = user.client();
            client.set_message_queue_paused(user.id(), true).await?;
            let cleared = user.send("queued drop /manual-only /disk-skill").await?;
            let added = user.send("queued add plain").await?;
            let cleared_id = cleared.receipt().execution_id.into();
            let added_id = added.receipt().execution_id.into();
            let cleared_rev = client
                .queued_message_for_edit(user.id(), cleared_id)
                .await?
                .revision;
            let added_rev = client
                .queued_message_for_edit(user.id(), added_id)
                .await?
                .revision;

            command_text_for(&user, SLASH_PLUGIN, "advance", "1").await?;
            client
                .edit_queued_message(
                    user.id(),
                    cleared_id,
                    cleared_rev,
                    ComposerDocument(vec![ComposerNode::Text {
                        text: "queued edit cleared skills".into(),
                    }]),
                )
                .await?;
            client
                .edit_queued_message(
                    user.id(),
                    added_id,
                    added_rev,
                    ComposerDocument(vec![ComposerNode::Text {
                        text: "queued edit added /manual-only /disk-skill //override-allow".into(),
                    }]),
                )
                .await?;
            command_text_for(&user, SLASH_PLUGIN, "advance", "2").await?;

            let first = case
                .model()
                .expect(RequestMatch::user_text("queued edit cleared skills"))
                .check(|request| {
                    let full = request.user_text();
                    let text = turn_text(&full, "queued edit cleared skills")?;
                    ensure!(!text.contains("Manual skill edition"), "{text}");
                    ensure!(!text.contains("Follow bundled disk instructions."), "{text}");
                    Ok(())
                })
                .reply(Reply::text("cleared ok"));
            case.model()
                .expect(RequestMatch::user_text("queued edit added"))
                .after(&first)
                .check(|request| {
                    let full = request.user_text();
                    let text = turn_text(&full, "queued edit added")?;
                    ensure!(text.contains("Manual skill edition 1"), "{text}");
                    ensure!(!text.contains("Manual skill edition 2"), "{text}");
                    ensure!(text.contains("Follow bundled disk instructions."), "{text}");
                    ensure!(!text.contains("Cordis override body"), "{text}");
                    Ok(())
                })
                .reply(Reply::text("added ok"));

            client.set_message_queue_paused(user.id(), false).await?;
            cleared
                .wait_settled()
                .await?
                .assert_assistant_text("cleared ok")?;
            added
                .wait_settled()
                .await?
                .assert_assistant_text("added ok")?;
            Ok(())
        })
        .await
}
