//! Public discovery and inline UI contracts observed from a running Agena.
//! Guide schemas and UI programs are consumed from real tool/API results.

use std::collections::BTreeSet;

use agena_domain::{InlineBlock, InlinePhase, UiState};
use agena_failure::ModelFeedbackKind;
use agena_plugin_sdk::CommandTarget;
use agena_testkit::{TestResult, ToolResult};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::{call, core_permissions::failure, fixture, string, text_payload_json, user};

fn text(result: &ToolResult) -> Result<String> {
    string(result.data()?, "/text")
}

fn first_json_block(text: &str) -> Result<Value> {
    let (_, block) = text
        .split_once("\x60\x60\x60json\n")
        .context("guide omitted its JSON example")?;
    let (block, _) = block
        .split_once("\n\x60\x60\x60")
        .context("guide has an unterminated JSON block")?;
    Ok(serde_json::from_str(block)?)
}

fn field_feedback(result: &ToolResult) -> Result<Value> {
    let failure = failure(result, ModelFeedbackKind::InvalidInput)?;
    serde_json::to_value(
        failure
            .model
            .context("invalid input omitted model feedback")?,
    )
    .context("serialize public model feedback")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inline_ui_guide_is_complete_bounded_and_serves_only_requested_schemas() -> TestResult {
    fixture()
        .expect_plugin("agena.inline_ui")
        .run(|case| async move {
            let user = user(&case).await?;
            let core = text(&call(&user, "inline_ui.guide", json!({})).await?)?;
            ensure!(core.len() <= 24 * 1024, "default guide grew beyond its advertised budget");
            for required in [
                "prepare", "update", "expected_revision", "revision_conflict", "phase",
                "document", "blocks", "program", "root", "children", "computed", "data",
                "state", "actions", "bind", "$get", "$op", "visible", "sum", "map",
                "filter", "reset", "copy", "set", "toggle", "Web/TUI", "local",
                "illustrative", "REFERENCE_FROM_PREPARE", "Agent-supplied baseline",
            ] {
                ensure!(core.contains(required), "default guide omitted {required:?}");
            }
            for optional in [
                "Full document JSON Schema", "Large dashboard reference", "validate(document",
                "{COMPONENT_CATALOG}", "{EXAMPLE}", "agentProgress",
            ] {
                ensure!(!core.contains(optional), "default guide contains optional/unexpanded {optional:?}");
            }

            let selected = text(&call(
                &user, "inline_ui.guide", json!({"sections":["chart","expressions","chart"]}),
            ).await?)?;
            assert_eq!(selected.matches("# chart: static block schema and reference").count(), 1);
            ensure!(selected.contains("filter") && selected.contains("map") && selected.contains("take"));
            ensure!(!selected.contains("Small complete panel"));

            let schema = text(&call(
                &user, "inline_ui.guide", json!({"sections":["schema"]}),
            ).await?)?;
            ensure!(schema.contains("Full document JSON Schema"));
            ensure!(!schema.contains("Agent-supplied baseline"));
            let full = first_json_block(&schema)?;
            let full_validator = jsonschema::validator_for(&full)
                .map_err(|error| anyhow::anyhow!("published document schema is invalid: {error}"))?;
            let panel: Value = serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"), "/../../examples/inline-ui/quick-start.json"
            )))?;
            ensure!(full_validator.is_valid(&panel));

            let dashboard = text(&call(
                &user, "inline_ui.guide", json!({"sections":["dashboard"]}),
            ).await?)?;
            ensure!(dashboard.contains("Large dashboard reference") && dashboard.contains("agentProgress"));

            // Validate the schema received by the real model, not the private
            // generator: its reachable definitions must be restricted to steps.
            let steps = text(&call(
                &user, "inline_ui.guide", json!({"sections":["steps"]}),
            ).await?)?;
            let narrowed = first_json_block(&steps)?;
            let validator = jsonschema::validator_for(&narrowed)
                .map_err(|error| anyhow::anyhow!("published steps schema is invalid: {error}"))?;
            let samples: Value = serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"), "/../../examples/inline-ui/reply.json"
            )))?;
            let blocks = samples["blocks"].as_array().context("reference blocks")?;
            for kind in ["steps", "chart"] {
                let block = blocks.iter().find(|block| block["type"] == kind)
                    .with_context(|| format!("missing {kind} reference"))?;
                assert_eq!(validator.is_valid(&json!({"version":1,"blocks":[block]})), kind == "steps");
            }
            let definitions = serde_json::to_string(&narrowed)?;
            ensure!(definitions.contains("InlineStep"));
            ensure!(!definitions.contains("InlineSeries"));

            let unknown = user.call_tool("inline_ui.guide", json!({"sections":["invented"]})).await?;
            failure(&unknown, ModelFeedbackKind::InvalidInput)?;
            let too_many = user.call_tool(
                "inline_ui.guide",
                json!({"sections":["schema","dashboard","expressions","updates","chart","paragraph","steps"]}),
            ).await?;
            failure(&too_many, ModelFeedbackKind::InvalidInput)?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inline_ui_live_panel_repairs_errors_without_corrupting_the_published_head() -> TestResult {
    fixture()
        .expect_plugin("agena.inline_ui")
        .run(|case| async move {
            let user = user(&case).await?;
            let document: Value = serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../examples/inline-ui/quick-start.json"
            )))?;
            let mut bad_prepare = document.clone();
            bad_prepare["blocks"][0]["program"]["nodes"][2]["props"]["style"] = json!("color:red");
            let rejected = user
                .call_tool("inline_ui.prepare", json!({"document":bad_prepare}))
                .await?;
            let hint = field_feedback(&rejected)?;
            ensure!(hint.to_string().contains("props.style"));
            ensure!(hint.to_string().contains("not applied"));
            ensure!(
                !rejected
                    .output
                    .to_string()
                    .contains("agena_inline_ui_error")
            );

            // A corrected request succeeds directly; the model needs no separate
            // validate endpoint, and an omitted phase defaults to complete.
            let corrected = call(&user, "inline_ui.prepare", json!({"document":document})).await?;
            assert_eq!(corrected.data()?["revision"], 1);
            assert_eq!(corrected.data()?["phase"], "complete");
            let complete_ref = string(corrected.data()?, "/inline_ui_prepared/reference")?;
            let complete = user
                .client()
                .read_inline_ui(user.id(), &complete_ref)
                .await?;
            assert_eq!(complete.phase, InlinePhase::Complete);
            assert_eq!(complete.revision, 1);

            let prepared = call(
                &user,
                "inline_ui.prepare",
                json!({"document":document,"phase":"working"}),
            )
            .await?;
            let reference = string(prepared.data()?, "/inline_ui_prepared/reference")?;
            let mut live = user
                .client()
                .stream_inline_ui(user.id(), &reference)
                .await?;
            let first = tokio::time::timeout(case.timeout(), live.recv())
                .await?
                .context("inline UI stream ended before its initial snapshot")??;
            assert_eq!(first.revision, 1);
            assert_eq!(first.phase, InlinePhase::Working);

            // Render the document fetched from the product as a client would.
            // This does not construct or invoke a plugin directly.
            let InlineBlock::Panel { program, .. } = &first.document.blocks[0] else {
                anyhow::bail!("prepared quick-start document is not a panel");
            };
            assert_eq!(program.nodes.len(), 7);
            let initial = program
                .render(&UiState::new())
                .map_err(anyhow::Error::msg)?;
            assert_eq!(
                initial
                    .root
                    .find("summary")
                    .context("summary metric")?
                    .props["value"],
                json!(150.0)
            );
            ensure!(initial.root.find("warning").is_none());
            let local = program
                .input(&initial.state, "slider", json!(50))
                .map_err(anyhow::Error::msg)?
                .state;
            let adjusted = program.render(&local).map_err(anyhow::Error::msg)?;
            assert_eq!(
                adjusted
                    .root
                    .find("summary")
                    .context("summary metric")?
                    .props["value"],
                json!(180.0)
            );
            ensure!(adjusted.root.find("warning").is_some());
            assert_eq!(
                program
                    .activate(&local, "reset")
                    .map_err(anyhow::Error::msg)?
                    .state["percent"],
                25
            );

            let update = json!({
                "reference":reference,"expected_revision":1,
                "patch":{"block_id":"scenario","data":{"base":160}}
            });
            let updated = call(&user, "inline_ui.update", json!({"update":update})).await?;
            assert_eq!(text_payload_json(&updated)?["revision"], 2);
            let second = tokio::time::timeout(case.timeout(), live.recv())
                .await?
                .context("inline UI stream ended before the committed update")??;
            assert_eq!(second.revision, 2);
            let InlineBlock::Panel { program: next, .. } = &second.document.blocks[0] else {
                anyhow::bail!("updated panel changed its block kind");
            };
            let view = next.render(&local).map_err(anyhow::Error::msg)?;
            assert_eq!(
                view.state["percent"], 50,
                "agent update erased client slider state"
            );
            assert_eq!(
                view.root.find("summary").context("updated summary")?.props["value"],
                json!(240.0)
            );

            let stale = user
                .call_tool("inline_ui.update", json!({"update":update}))
                .await?;
            let hint = field_feedback(&stale)?.to_string();
            ensure!(hint.contains("revision_conflict") && hint.contains("expected 1, current 2"));

            for (patch, expected_path) in [
                (
                    json!({"block_id":"scenario","data":{"base":"bad-number"}}),
                    "update.patch.data",
                ),
                (
                    json!({"block_id":"scenario","nodes":[{
                        "id":"summary","kind":"metric",
                        "props":{"label":"Adjusted total","value":0,"style":"unsupported"}
                    }]}),
                    "update.patch.nodes[0].props.style",
                ),
            ] {
                let rejected = user.call_tool(
                    "inline_ui.update",
                    json!({"update":{"reference":reference,"expected_revision":2,"patch":patch}}),
                ).await?;
                let feedback = field_feedback(&rejected)?;
                let fields = feedback["fields"]
                    .as_array()
                    .context("repair omitted field issues")?;
                ensure!(
                    fields.iter().any(|field| field["field"] == expected_path),
                    "incorrect repair path: {feedback}"
                );
                assert_eq!(
                    user.client().read_inline_ui(user.id(), &reference).await?,
                    second
                );
            }
            let read = text_payload_json(
                &call(&user, "inline_ui.read", json!({"reference":reference})).await?,
            )?;
            assert_eq!(read["revision"], 2);
            assert_eq!(
                read["document"]["blocks"][0]["program"]["data"]["base"],
                160
            );
            call(
                &user,
                "inline_ui.update",
                json!({"update":{
                    "reference":reference,"expected_revision":2,"phase":"complete",
                    "patch":{"block_id":"scenario","data":{"base":180}}
                }}),
            )
            .await?;
            let final_head = tokio::time::timeout(case.timeout(), live.recv())
                .await?
                .context("inline UI stream ended before completion")??;
            assert_eq!(final_head.revision, 3);
            assert_eq!(final_head.phase, InlinePhase::Complete);
            assert_eq!(
                serde_json::to_value(&final_head.document)?["blocks"][0]["program"]["data"]["base"],
                180
            );
            drop(live);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inline_ui_invalid_blocks_return_bounded_safe_model_feedback() -> TestResult {
    fixture()
        .expect_plugin("agena.inline_ui")
        .run(|case| async move {
            let user = user(&case).await?;
            let blocks = (0..12)
                .map(|index| {
                    json!({
                        "type":"paragraph","id":format!("block{index}"),
                        "text":"private-input-marker-not-error-copy","unknown_property":true
                    })
                })
                .collect::<Vec<_>>();
            let rejected = user
                .call_tool(
                    "inline_ui.prepare",
                    json!({"document":{"version":1,"blocks":blocks}}),
                )
                .await?;
            let feedback = field_feedback(&rejected)?;
            let fields = feedback["fields"]
                .as_array()
                .context("invalid blocks omitted repair fields")?;
            assert_eq!(fields.len(), 8);
            let paths = fields
                .iter()
                .map(|field| field["field"].as_str().context("repair field path"))
                .collect::<Result<BTreeSet<_>>>()?;
            assert_eq!(
                paths.len(),
                8,
                "different invalid blocks collapsed to one repair path"
            );
            ensure!(
                paths
                    .iter()
                    .all(|path| path.starts_with("document.blocks["))
            );
            // The envelope intentionally identifies the original invocation,
            // including the model's own input. Error presentation must not copy
            // that document into either the user problem or model feedback.
            let error_presentation = json!({
                "error":rejected.output.pointer("/result/error")
                    .context("invalid blocks omitted their public problem")?,
                "failure":rejected.failure()?,
            })
            .to_string();
            ensure!(!error_presentation.contains("private-input-marker-not-error-copy"));
            ensure!(
                !rejected
                    .output
                    .to_string()
                    .contains("agena_inline_ui_error")
            );
            ensure!(fields.iter().all(|field| {
                field["hint"]
                    .as_str()
                    .is_some_and(|hint| hint.contains("not applied"))
            }));
            // No private diagnostic-only truncation flag is promised by the
            // public error contract. The real model receives at most eight
            // actionable fields, even when the submitted document has twelve.
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn command_catalog_and_discovery_preserve_public_authoring_contracts() -> TestResult {
    const LEGACY: &str = "---\r\nname: legacy_name\r\ndescription: >-\r\n  Use this skill\r\n  for source reviews.\r\nlicense: MIT\r\ncompatibility: Requires Python\r\nmetadata:\r\n  author: Example\r\nallowed-tools: Read Bash(python:*)\r\naliases: [review-sources]\r\ncustom-extension: true\r\n---\r\n# Instructions\r\nlegacy-command-body-marker\r\n";
    fixture()
        .expect_plugin("agena.commands")
        .configure_global(|config| {
            config["plugins"]["list"]["agena.commands"] = json!({
                "package":{"kind":"static"},
                "settings":{"additional_roots":["legacy-skills"]}
            });
        })
        .seed_file("workspace/legacy-skills/legacy_name/SKILL.md", LEGACY.as_bytes().to_vec())
        .seed_file("workspace/.agena/commands/standup.md", b"Plain command marker.\n".to_vec())
        .seed_file(
            "workspace/legacy-skills/enabled-example/SKILL.md",
            b"---\nname: enabled-example\ndescription: Example.\n---\nEnabled marker.\n".to_vec(),
        )
        .seed_file(
            "workspace/legacy-skills/disabled-example/SKILL.md",
            b"---\nname: disabled-example\ndescription: Example.\n---\nDisabled marker.\n".to_vec(),
        )
        .seed_file("workspace/legacy-skills/disabled-example/.agena-skill.json", br#"{"enabled":false}"#.to_vec())
        .seed_file(
            "workspace/legacy-skills/.agena-skill-stage-example/SKILL.md",
            b"---\nname: staging-example\ndescription: Example.\n---\nStaging marker.\n".to_vec(),
        )
        .run(|case| async move {
            let user = user(&case).await?;
            let catalog = user.client().plugin_surface_catalog().await?.catalog;
            let commands = catalog.commands.iter().filter(|entry| entry.plugin_id.to_string() == "agena.commands")
                .collect::<Vec<_>>();
            ensure!(!commands.is_empty(), "running command plugin published no commands");
            let mut ids = BTreeSet::new();
            for entry in &commands {
                ensure!(ids.insert(&entry.command.id), "duplicate public command id");
                entry.command.validate()?;
            }
            let ui = commands.iter().find(|entry| entry.command.slash.as_deref() == Some("/ui"))
                .context("running catalog lost /ui")?;
            assert_eq!(ui.command.id, "ui");
            ensure!(ui.command.aliases.iter().any(|alias| alias == "inline-ui"));
            ensure!(matches!(&ui.command.target, CommandTarget::Client { action } if action == "inline-ui"));
            let compact = commands.iter().find(|entry| entry.command.slash.as_deref() == Some("/compact"))
                .context("running catalog lost /compact")?;
            ensure!(matches!(&compact.command.target, CommandTarget::Client { action } if action == "compact"));
            ensure!(compact.command.docs.usage.is_none());
            ensure!(compact.accepts_empty_input);
            assert_eq!(compact.default_input, json!({}));
            compact.command.input.validate_value(&json!({}))?;
            assert_eq!(compact.command.input.default_value()?, json!({}));

            let legacy = call(&user, "commands.get", json!({"name":"review-sources"})).await?;
            assert_eq!(legacy.data()?["name"], "legacy_name");
            assert_eq!(legacy.data()?["summary"], "Use this skill for source reviews.");
            assert_eq!(legacy.data()?["aliases"], json!(["review-sources"]));
            legacy.assert_text_contains("# Instructions")?;
            legacy.assert_text_contains("legacy-command-body-marker")?;
            let bare = call(&user, "commands.get", json!({"name":"standup"})).await?;
            assert_eq!(bare.data()?["name"], "standup");
            bare.assert_text_contains("Plain command marker.")?;

            let listed = call(&user, "commands.list", json!({"verbose":true})).await?;
            let names = listed.data()?["packages"].as_array().context("command package catalog")?
                .iter().filter_map(|entry| entry["name"].as_str()).collect::<BTreeSet<_>>();
            ensure!(names.contains("enabled-example") && names.contains("legacy_name") && names.contains("standup"));
            ensure!(!names.contains("disabled-example"));
            ensure!(!names.contains("staging-example") && !names.contains(".agena-skill-stage-example"));
            for absent in ["disabled-example", "staging-example"] {
                user.call_tool("commands.get", json!({"name":absent})).await?.assert_failed()?;
            }
            Ok(())
        })
        .await
}
