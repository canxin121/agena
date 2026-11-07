//! Canonical attention and host queue contracts through production HTTP surfaces.
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::super::{allow_permissions, fixture};
use super::{activity, reload_snapshot, wait_catalog};
use agena_api::{
    commands::{Command, CommandResult, ResolveWorkspaceParams, SubmitRunParams},
    resource::RunOptions,
};
use agena_domain::{ComposerDocument, ComposerNode, session_kernel::ExecutionPhase};
use agena_plugin_sdk::prelude::*;
use agena_testkit::{EditOptions, Reply, RequestMatch, SettingsLayer, TestResult};
use anyhow::{Context, ensure};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_completions_and_failures_publish_without_a_connected_client() -> TestResult {
    fixture().builtin_plugin("agena.notifications", Value::Null)
        .expect_plugin("agena.notifications")
        .run(|case| async move {
            // Do not use UserSession here: it attaches a live session stream.
            // These are ordinary short REST requests, with no browser or SSE observer.
            let client = case.client().await;
            let CommandResult::Workspace(workspace) = client.command(
                Command::ResolveWorkspace(ResolveWorkspaceParams {
                    path: case.workspace().to_string_lossy().into_owned(),
                    create_if_missing: true,
                })
            ).await? else { anyhow::bail!("workspace response"); };
            let session = client.create_session(workspace.id, "Unobserved attention", None).await?;
            let mut expected = Vec::new();
            for (marker, phase, kind) in [
                ("unobserved-completed-marker", ExecutionPhase::Succeeded, AttentionKind::Completed),
                ("unobserved-failed-marker", ExecutionPhase::Failed, AttentionKind::Failed),
                ("unobserved-cancelled-marker", ExecutionPhase::Cancelled, AttentionKind::Cancelled),
            ] {
                let reply = match phase {
                    ExecutionPhase::Succeeded => Reply::text("unobserved-completed-answer").held(),
                    ExecutionPhase::Failed => Reply::raw_json(400, json!({
                        "error": {"message": "unobserved model failure", "type": "invalid_request_error"}
                    })),
                    ExecutionPhase::Cancelled => Reply::text_chunks([
                        "unobserved-visible-prefix", "unreleased-unobserved-suffix"
                    ]).hold_after_chunks(1),
                    _ => unreachable!(),
                };
                let step = case.model().expect(RequestMatch::user_text(marker)).reply(reply);
                let accepted = client.submit_message(SubmitRunParams {
                    session_id: session.id,
                    document: ComposerDocument(vec![ComposerNode::Text { text: marker.into() }]),
                    options: RunOptions::default(),
                }).await?;
                let receipt = accepted.receipt.context("accepted attention run has no receipt")?;
                step.observed().await?;
                if phase == ExecutionPhase::Cancelled {
                    tokio::time::timeout(Duration::from_secs(20), async {
                        loop {
                            let parts = client.session_all_parts(session.id).await?;
                            let mut visible = Vec::new();
                            for part in parts.iter().filter(|part|
                                part.role == "assistant" && part.kind == "text"
                                    && part.message_id == Some(receipt.message_id)) {
                                // Parts contain content references, not necessarily
                                // inline text. Read the public content endpoint while
                                // keeping this scenario free of live/SSE subscribers.
                                client.write_part_text(session.id, part, &mut visible).await?;
                            }
                            if String::from_utf8(visible)?.contains("unobserved-visible-prefix") {
                                return Ok::<_, anyhow::Error>(());
                            }
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                    }).await.context("the streamed prefix was not durably committed")??;
                    client.cancel_execution_attempt(session.id, receipt.execution_id, receipt.attempt_generation).await?;
                    step.disconnected().await?;
                } else {
                    step.release();
                }
                let settled = client.wait_execution_attempt_settled(
                    session.id, receipt.execution_id, receipt.attempt_generation, Duration::from_secs(20)
                ).await?;
                ensure!(settled.phase == phase, "{settled:?}");
                let id = format!("execution:{}:{}", receipt.execution_id, receipt.attempt_generation);
                expected.push((id.clone(), kind));
                let catalog = wait_catalog(&client, "canonical notification without a live client", |surface| {
                    surface.notifications.iter().any(|item| item.plugin_id == "agena.notifications"
                        && item.session_id == Some(session.id)
                        && item.attention.as_ref().is_some_and(|attention| attention.id == id && attention.kind == kind))
                }).await?;
                for (id, kind) in &expected {
                    let matching: Vec<_> = catalog.notifications.iter().filter_map(|item| {
                        item.attention.as_ref().filter(|attention| attention.id == *id)
                    }).collect();
                    ensure!(matching.len() == 1 && matching[0].kind == *kind, "{matching:?}");
                }
                ensure!(!catalog.notifications.iter().any(|item| item.session_id == Some(session.id)
                    && item.attention.as_ref().is_some_and(|attention| attention.kind == AttentionKind::Question)));
            }
            ensure!(case.model().requests().len() == 3);
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_pending_requests_recover_without_browser_state_and_keep_resolution() -> TestResult
{
    fixture().builtin_plugin("agena.notifications", Value::Null)
        .configure_global(|config| config["plugins"]["list"]["agena.notifications"]["enabled"] = json!(false))
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let client = case.client().await;
            ensure!(activity(&client.plugin_surface_catalog().await?).is_none());
            case.model().expect(RequestMatch::user_text("unobserved-old-reply-marker"))
                .reply(Reply::text("old-reply-must-not-be-replayed"));
            let old = user.send("unobserved-old-reply-marker").await?;
            old.wait_settled().await?.assert_succeeded()?;
            let old_id = format!("execution:{}:{}", old.receipt().execution_id, old.receipt().attempt_generation);

            // A real path approval is committed while no notifications instance
            // is loaded. Enabling it must recover the canonical pending request.
            let target = case.workspace().join("attention-denied.txt");
            client.set_session_permission(user.id(), serde_json::from_value(json!({
                "tools": {"default": "allow", "names": {"fs.write": "allow", "agena.fs.write": "allow"}},
                "path": {"rules": {(target.to_string_lossy().as_ref()): {"read": "allow", "write": "ask"}}}
            }))?).await?;
            let write = user.start_tool("fs.write", json!({
                "path": "attention-denied.txt", "content": "must-not-be-written"
            })).await?;
            let permission = write.run().wait_permission().await?;
            ensure!(!target.exists(), "approval was bypassed");
            case.settings().set(SettingsLayer::Global, "plugins.list.\"agena.notifications\".enabled",
                json!(true), EditOptions::default()).await?.require_success()?;
            let pending = wait_catalog(&client, "pending permission recovered by the enabled plugin", |surface| {
                activity(surface) == Some("blocked") && surface.notifications.iter().any(|item|
                    item.plugin_id == "agena.notifications" && item.session_id == Some(user.id())
                    && item.attention.as_ref().is_some_and(|attention| attention.kind == AttentionKind::Permission && !attention.resolved))
            }).await?;
            let permission_id = pending.notifications.iter().find_map(|item| {
                item.attention.as_ref().filter(|attention| item.session_id == Some(user.id())
                    && attention.kind == AttentionKind::Permission && !attention.resolved)
                    .map(|attention| attention.id.clone())
            }).context("recovered permission identity")?;
            ensure!(pending.notifications.iter().filter(|item| item.attention.as_ref()
                .is_some_and(|attention| attention.id == permission_id)).count() == 1);
            ensure!(pending.notifications.iter().all(|item| !item.attention.as_ref()
                .is_some_and(|attention| attention.id == old_id)), "enable replayed an unobserved historical completion");

            reload_snapshot(&case).await?;
            let reloaded = wait_catalog(&client, "pending permission survives a second real host", |surface| {
                surface.notifications.iter().any(|item| item.attention.as_ref()
                    .is_some_and(|attention| attention.id == permission_id && !attention.resolved))
            }).await?;
            ensure!(reloaded.notifications.iter().filter(|item| item.attention.as_ref()
                .is_some_and(|attention| attention.id == permission_id)).count() == 1);
            permission.mark_presented().await?;
            permission.deny_once().await?;
            write.wait_settled().await?.assert_failure_kind(agena_testkit::ModelFeedbackKind::UserDeclined)?;
            ensure!(!target.exists(), "denied permission wrote a file");
            let resolved = wait_catalog(&client, "permission resolution replaces its pending intent", |surface| {
                surface.notifications.iter().any(|item| item.attention.as_ref()
                    .is_some_and(|attention| attention.id == permission_id && attention.resolved))
                    && !surface.notifications.iter().any(|item| item.attention.as_ref()
                        .is_some_and(|attention| attention.id == permission_id && !attention.resolved))
            }).await?;
            ensure!(resolved.notifications.iter().filter(|item| item.attention.as_ref()
                .is_some_and(|attention| attention.id == permission_id)).count() == 1);
            case.restart().await?;
            let restarted = case.client().await;
            let recovered = wait_catalog(&restarted, "resolved permission after process restart", |surface| {
                surface.notifications.iter().any(|item| item.attention.as_ref()
                    .is_some_and(|attention| attention.id == permission_id && attention.resolved))
            }).await?;
            ensure!(recovered.notifications.iter().filter(|item| item.attention.as_ref()
                .is_some_and(|attention| attention.id == permission_id)).count() == 1);
            ensure!(recovered.notifications.iter().all(|item| !item.attention.as_ref()
                .is_some_and(|attention| attention.id == permission_id && !attention.resolved)));
            ensure!(recovered.notifications.iter().all(|item| !item.attention.as_ref()
                .is_some_and(|attention| attention.id == old_id)));
            Ok(())
        }).await
}

struct ReplayProbe {
    host: OnceLock<Arc<dyn HostClient>>,
}

#[agena_plugin(
    namespace = "testkit",
    name = "attention-probe",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Publish controlled attention events through the real host callback."
)]
impl ReplayProbe {
    #[hook(
        init,
        summary = "Retain the callback client",
        help = "Requires the real HTTP host callback transport."
    )]
    async fn initialize(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        if ctx.host_callback_url.is_none() || ctx.host_callback_token.is_none() {
            return Err(PluginError::internal("missing HTTP callback transport"));
        }
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("initialized twice"))?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    #[tool(
        name = "publish",
        summary = "Publish an attention replay fixture.",
        tags(read_only)
    )]
    async fn publish(
        &self,
        #[arg] session_id: i64,
        #[arg(trim, non_empty)] attention_id: String,
        #[arg] resolved: bool,
    ) -> Result<ToolInvokeOutput> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| PluginError::internal(error.to_string()))?
            .as_millis() as i64;
        self.host
            .get()
            .ok_or_else(|| PluginError::internal("missing callback client"))?
            .notify(PluginNotifyRequest {
                attention: Some(AttentionNotification {
                    id: attention_id,
                    kind: AttentionKind::Question,
                    created_at_ms: now,
                    resolved,
                }),
                title: "Attention replay fixture".into(),
                body: "A real plugin callback tests queue monotonicity.".into(),
                severity: "info".into(),
                session_id: Some(session_id),
                ..Default::default()
            })
            .await?;
        Ok(ToolInvokeOutput::text("attention callback accepted"))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_queue_does_not_resurrect_a_resolved_request_during_replay() -> TestResult {
    fixture()
        .before_start(|case| async move {
            let router = agena_plugin_sdk::drivers::http::router(
                || ReplayProbe {
                    host: OnceLock::new(),
                },
                Arc::new(NoopHostClient),
            );
            let service = case.serve_http(router).await?;
            let mut config: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            config["plugins"]["list"]["testkit.attention-probe"] = json!({
                "enabled": true, "package": {"kind": "http", "url": service.url("/rpc")}
            });
            case.write_config(SettingsLayer::Global, config).await
        })
        .expect_plugin("testkit.attention-probe")
        .run(|case| async move {
            let user = case.user().new_session().await?;
            allow_permissions(&user).await?;
            let id = format!("request:{}:http-replay", user.id());
            for (sent_resolved, expected_resolved) in [(false, false), (true, true), (false, true)]
            {
                user.call_tool(
                    "testkit.attention-probe.publish",
                    json!({
                        "session_id": user.id(), "attention_id": id, "resolved": sent_resolved
                    }),
                )
                .await?
                .assert_succeeded()?;
                let catalog = case.client().await.plugin_surface_catalog().await?;
                let matching: Vec<_> = catalog
                    .notifications
                    .iter()
                    .filter(|item| {
                        item.plugin_id == "testkit.attention-probe"
                            && item.session_id == Some(user.id())
                            && item
                                .attention
                                .as_ref()
                                .is_some_and(|attention| attention.id == id)
                    })
                    .collect();
                ensure!(
                    matching.len() == 1,
                    "host callback queue duplicated the identity: {matching:?}"
                );
                ensure!(
                    matching[0].attention.as_ref().unwrap().resolved == expected_resolved,
                    "a late pending replay resurrected resolved attention: {matching:?}"
                );
            }
            Ok(())
        })
        .await
}
