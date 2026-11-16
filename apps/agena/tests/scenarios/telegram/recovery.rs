//! Restore an upgraded, faulted private chat through the real HTTP surface.
use super::*;
use sha2::{Digest, Sha256};

fn private_message(message_id: i64, text: &str) -> Value {
    json!({
        "message_id": message_id,
        // Telegram timestamps have second precision; place this fixture event
        // after the explicit binding activation, as the group fixture does.
        "date": Utc::now().timestamp() + 1,
        "chat": {"id": MEMBER_ID, "type": "private", "first_name": "Channel member"},
        "from": {"id": MEMBER_ID, "is_bot": false, "first_name": "Channel member", "username": "channel_member"},
        "text": text
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_restores_existing_private_history_and_local_replies_without_replaying_old_output()
-> TestResult {
    let api = Arc::new(BotApi::default());
    let observed = api.clone();
    telegram_fixture(api)
        .case_timeout(Duration::from_secs(240))
        .run(move |case| async move {
            let client = case.client().await;
            let bot = client.create_bot(&BotCreate {
                request_key: "review-private-bot".into(),
                name: "Private review fixture".into(),
                persona: None,
                avatar: None,
            }).await?;
            let source = client.ensure_named_bot_session(bot.id).await?;
            let config: Value = serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            let saved = client.invoke_plugin_command("agena.telegram", "save_bot", json!({
                "expected_revision": 0,
                "bot": {
                    "id": "default", "name": "Private recovery fixture", "enabled": true,
                    "connection": config["plugins"]["list"]["agena.telegram"]["settings"],
                    "credential_kind": "environment", "ai_bot_id": bot.id,
                    "chat_access": {
                        "mode": "whitelist", "private_enabled": true,
                        "group_enabled": false, "channel_enabled": false,
                        "chat_ids": [MEMBER_ID.to_string()]
                    }
                }, "token": null
            }), None, None, "").await?;
            let saved = saved.result.output.context("saved private Telegram bot")?;
            ensure!(saved["source_session_id"] == json!(source.session_id), "{saved}");
            let mut rule = spec();
            rule.resource = MEMBER_ID.to_string().into();
            rule.channel_policy.as_mut().unwrap().kinds = [ConversationKind::Direct].into_iter().collect();
            rule.channel_policy.as_mut().unwrap().thread_mode = ChannelThreadMode::Exact;
            rule.channel_policy.as_mut().unwrap().session_mode = ChannelSessionMode::PerActor;
            let created = client.create_integration(source.session_id, &CreateIntegration {
                request_key: "private-history-review".into(), spec: rule,
            }).await?;
            let active = client.control_integration(source.session_id, created.id, &IntegrationControl {
                not_before_ms: None, expected_revision: created.revision,
                action: IntegrationControlAction::Enable,
            }).await?;
            let model = case.model();
            let first = model.expect(RequestMatch::new("original private message", |request| {
                request.body.to_string().contains("private-history-before-upgrade")
            })).reply(Reply::text("private-history-preserved-response"));
            observed.updates.lock().unwrap().push(update(1, private_message(101, "private-history-before-upgrade")));
            client.control_integration(source.session_id, active.id, &IntegrationControl {
                not_before_ms: None, expected_revision: active.revision,
                action: IntegrationControlAction::PollNow,
            }).await?;
            first.finished().await?;
            let original = wait_for(|| async {
                let detail = client.integration_detail(source.session_id, active.id).await?;
                Ok((detail.channel_sessions.len() == 1 && detail.deliveries.iter().any(|d| d.state == "sent")).then_some(detail))
            }, "original private chat and delivery").await?;
            let original_scope = original.channel_sessions.first().context("original private scope")?.clone();
            let session_id = original_scope.session_id;
            ensure!(original_scope.authority.epoch == 1);
            ensure!(original_scope.authority.role == ChannelRole::Member);
            let http = case.http().await;
            let inspection: Value = http.get("/api/v1/plugins/agena.telegram").await?.require_success()?.json()?;
            let legacy_manifest: Value = serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/agena-runtime/src/runtime/assistant/integrations/legacy/telegram-885d63f24.json"
            )))?;
            let legacy_identity = json!({
                "manifest": legacy_manifest,
                "configuration": inspection["plugin"]["configured_plugin"],
            });
            let legacy_fingerprint: String = Sha256::digest(serde_json::to_vec(&legacy_identity)?)
                .iter().map(|byte| format!("{byte:02x}")).collect();
            let paused = client.control_integration(source.session_id, active.id, &IntegrationControl {
                not_before_ms: None, expected_revision: original.binding.revision,
                action: IntegrationControlAction::Pause,
            }).await?;
            ensure!(paused.epoch == 2);
            // Reconstruct the persisted upgrade failure in this isolated fixture.
            // The absent outbox row represents a completed old execution that
            // crashed before export; recovery must not send it a second time.
            let injected = tokio::process::Command::new("python3")
                .arg("-c")
                .arg(r#"import json,sqlite3,sys
c=sqlite3.connect(sys.argv[1])
with c:
    row=c.execute('SELECT binding_json FROM agena_integration_bindings WHERE id=?',(sys.argv[2],)).fetchone()
    b=json.loads(row[0])
    assert b['state']=='paused' and b['epoch']==2
    b['state']='faulted'
    b['provider_fingerprint']=sys.argv[3]
    b['last_error']='channel provider or source permissions changed; explicit review is required'
    c.execute('UPDATE agena_integration_bindings SET state=?,binding_json=? WHERE id=?',('faulted',json.dumps(b),sys.argv[2]))
    c.execute('DELETE FROM agena_integration_outbox WHERE binding_id=?',(sys.argv[2],))
"#)
                .arg(case.data_dir().join("agena.db"))
                .arg(active.id.to_string())
                .arg(legacy_fingerprint)
                .output().await?;
            ensure!(injected.status.success(), "fixture fault injection: {}", String::from_utf8_lossy(&injected.stderr));
            let mut policy = paused.spec.channel_policy.clone().context("private policy")?;
            policy.members.iter_mut().find(|member| member.actor == MEMBER_ID.to_string()).unwrap().role = ChannelRole::Owner;
            let changed = client.update_channel_access(source.session_id, active.id, &ChannelPolicyUpdate {
                expected_revision: paused.revision,
                request_key: "promote-without-capability-change".into(), policy,
            }).await?;
            ensure!(changed.state == BindingState::Faulted && changed.epoch == 3);
            let preview_path = format!("/api/v1/sessions/{session_id}/prompts/preview");
            ensure!(!http.get(&preview_path).await?.status.is_success(), "old authority must remain blocked until review");
            let recovered = client.control_integration(source.session_id, active.id, &IntegrationControl {
                not_before_ms: None, expected_revision: changed.revision,
                action: IntegrationControlAction::Review,
            }).await?;
            ensure!(recovered.id == active.id && recovered.state == BindingState::Enabled);
            ensure!(recovered.epoch > changed.epoch && recovered.provider_fingerprint.starts_with("integration-provider:v2:"));
            http.get(&preview_path).await?.require_success()?;
            let restored = client.integration_detail(source.session_id, active.id).await?;
            ensure!(restored.channel_sessions.len() == 1);
            let restored_scope = &restored.channel_sessions[0];
            ensure!(restored_scope.session_id == session_id);
            ensure!(restored_scope.authority.role == ChannelRole::Owner);
            ensure!(restored_scope.authority.epoch == recovered.epoch);
            ensure!(restored_scope.authority.grant == original_scope.authority.grant);
            ensure!(restored_scope.authority.memory_workspace_root == original_scope.authority.memory_workspace_root);
            ensure!(restored.deliveries.iter().any(|delivery| delivery.state == "skipped"));
            let local = model.expect(RequestMatch::user_text("local-message-after-review"))
                .check(|request| {
                    ensure!(request.body.to_string().contains("private-history-preserved-response"), "restored local turn lost its original history");
                    Ok(())
                }).reply(Reply::text("local-private-reply-after-review"));
            let chat = case.user().attach_session(session_id).await?;
            chat.send("local-message-after-review").await?.wait_terminal().await?;
            local.finished().await?;
            wait_for(|| async {
                Ok(observed.sent.lock().unwrap().iter().any(|message| message["text"].as_str().is_some_and(|text| text.contains("local-private-reply-after-review"))).then_some(()))
            }, "local reply delivered through restored private chat").await?;
            let remote = model.expect(RequestMatch::new("new private message after review", |request| {
                request.body.to_string().contains("telegram-message-after-review")
            })).reply(Reply::text("telegram-private-reply-after-review"));
            observed.updates.lock().unwrap().push(update(2, private_message(102, "telegram-message-after-review")));
            let latest = client.integration_detail(source.session_id, active.id).await?;
            client.control_integration(source.session_id, active.id, &IntegrationControl {
                not_before_ms: None, expected_revision: latest.binding.revision,
                action: IntegrationControlAction::PollNow,
            }).await?;
            remote.finished().await?;
            wait_for(|| async {
                Ok(observed.sent.lock().unwrap().iter().any(|message| message["text"].as_str().is_some_and(|text| text.contains("telegram-private-reply-after-review"))).then_some(()))
            }, "incoming Telegram message uses restored private chat").await?;
            let final_detail = client.integration_detail(source.session_id, active.id).await?;
            ensure!(final_detail.channel_sessions.len() == 1 && final_detail.channel_sessions[0].session_id == session_id);
            let sent = observed.sent.lock().unwrap();
            ensure!(sent.len() == 3, "recovery replayed an old response: {sent:?}");
            ensure!(sent.iter().filter(|message| message["text"].as_str().is_some_and(|text| text.contains("private-history-preserved-response"))).count() == 1);
            Ok(())
        }).await
}
