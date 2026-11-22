//! Restore a faulted private chat through the real HTTP surface.
use super::*;

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
            let first = model.expect(RequestMatch::user_text("private-history-before-review"))
                .reply(Reply::function_call("unassigned-tools", "tools_list", json!({"plugin":"agena.fs"})));
            model.expect(RequestMatch::tool_output("unassigned-tools")).after(&first)
                .check_tool_output_contains("unassigned-tools", "returned 0 of 0")
                .reply(Reply::text("private-history-preserved-response"));
            observed.updates.lock().unwrap().push(update(1, private_message(101, "private-history-before-review")));
            client.control_integration(source.session_id, active.id, &IntegrationControl {
                not_before_ms: None, expected_revision: active.revision,
                action: IntegrationControlAction::PollNow,
            }).await?;
            first.finished().await?;
            let original = wait_for(|| async {
                let detail = client.integration_detail(source.session_id, active.id).await?;
                Ok((detail.channel_sessions.len() == 1 && detail.deliveries.iter().any(|d| d.state == "sent")).then_some(detail))
            }, "original private chat and delivery").await?;
            let delivered = original
                .deliveries
                .iter()
                .find(|delivery| delivery.state == "sent")
                .context("original execution delivery")?;
            let original_scope = original.channel_sessions.first().context("original private scope")?.clone();
            let session_id = original_scope.session_id;
            ensure!(original_scope.authority.epoch == 1);
            ensure!(original_scope.authority.role == ChannelRole::Member);
            let http = case.http().await;
            let paused = client.control_integration(source.session_id, active.id, &IntegrationControl {
                not_before_ms: None, expected_revision: original.binding.revision,
                action: IntegrationControlAction::Pause,
            }).await?;
            ensure!(paused.epoch == 2);
            // Restore the durable state of a faulted binding with a completed
            // execution interrupted before its outbox receipt was enqueued.
            let injected = tokio::process::Command::new("python3")
                .arg("-c")
                .arg(r#"import json
import sqlite3
import sys

connection = sqlite3.connect(sys.argv[1])
connection.execute('PRAGMA foreign_keys=ON')
delivery_id = sys.argv[2]
binding_id, execution_id, generation = delivery_id.split(':')
with connection:
    row = connection.execute(
        'SELECT binding_json FROM agena_integration_bindings WHERE id=?', (binding_id,)
    ).fetchone()
    binding = json.loads(row[0])
    assert binding['state'] == 'paused' and binding['epoch'] == 2
    binding['state'] = 'faulted'
    binding['last_error'] = 'local review is required'
    connection.execute(
        'UPDATE agena_integration_bindings SET state=?,binding_json=? WHERE id=?',
        ('faulted', json.dumps(binding), binding_id),
    )
    deleted = connection.execute('DELETE FROM agena_integration_outbox WHERE id=?', (delivery_id,))
    assert deleted.rowcount == 1
    pending = connection.execute('''
        INSERT INTO agena_channel_pending_outputs(session_id,finished_at_ms,execution_id,generation,delivery_id)
        SELECT session_id,finished_at_ms,execution_id,generation,? FROM agena_attempts
        WHERE execution_id=? AND generation=? AND finished_at_ms IS NOT NULL
    ''', (delivery_id, execution_id, int(generation)))
    assert pending.rowcount == 1
"#)
                .arg(case.data_dir().join("agena.db"))
                .arg(&delivered.id)
                .output().await?;
            ensure!(injected.status.success(), "fixture fault injection: {}", String::from_utf8_lossy(&injected.stderr));
            let mut policy = paused.spec.channel_policy.clone().context("private policy")?;
            policy.members.iter_mut().find(|member| member.actor == MEMBER_ID.to_string()).unwrap().role = ChannelRole::Owner;
            let denied = client.update_channel_access(source.session_id, active.id, &ChannelPolicyUpdate {
                expected_revision: paused.revision,
                request_key: "retired-channel-policy-write".into(), policy,
            }).await;
            ensure!(denied.is_err(), "Telegram permission writes require named assignments");
            let changed = client.integration_detail(source.session_id, active.id).await?.binding;
            ensure!(changed.state == BindingState::Faulted && changed.epoch == paused.epoch);
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
            ensure!(restored_scope.authority.role == ChannelRole::Member);
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
            let mut assignments = json!({
                "users": [{"platform":"agena.telegram", "tenant":"telegram", "account":BOT_USERNAME,
                    "resource":MEMBER_ID.to_string(), "actor":MEMBER_ID.to_string(), "profile_id":"@default"}]
            });
            case.settings().set(SettingsLayer::Global, "access_control", assignments.clone(), agena_testkit::EditOptions::default()).await?.require_success()?;
            let remote = model.expect(RequestMatch::user_text("telegram-message-after-review"))
                .reply(Reply::function_call("profile-plugins", "plugins_list", json!({"plugin":"agena.fs"})));
            let list = model.expect(RequestMatch::tool_output("profile-plugins")).after(&remote)
                .check(|request| {
                    let output = request.tool_output_text("profile-plugins").context("plugin catalog response")?;
                    ensure!(output.contains("- agena.fs [") && !output.contains(" · 0 tool(s)"), "default profile should offer filesystem tools: {output}");
                    Ok(())
                }).reply(Reply::function_call("profile-tools", "tools_list", json!({"plugin":"agena.fs"})));
            let help = model.expect(RequestMatch::tool_output("profile-tools")).after(&list)
                .check_tool_output_contains("profile-tools", "- fs.read [")
                .reply(Reply::function_call("profile-help", "tools_help", json!({"tool":["fs.read", "plan.get"]})));
            let read = model.expect(RequestMatch::tool_output("profile-help")).after(&help)
                .check_tool_output_contains("profile-help", "file_path")
                .reply(Reply::tool_call("profile-plan", "plan.get", json!({})));
            model.expect(RequestMatch::tool_output("profile-plan")).after(&read)
                .check(|request| {
                    let output: Value = serde_json::from_str(&request.tool_output_text("profile-plan").context("plan response")?)?;
                    ensure!(output["phase"] == "succeeded", "plan.get did not execute: {output}");
                    let result: Value = serde_json::from_str(output["result"]["model"].as_str().context("plan payload")?)?;
                    ensure!(result.get("plan") == Some(&Value::Null), "unexpected plan: {result}");
                    Ok(())
                })
                .reply(Reply::text("telegram-private-reply-after-review"));
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
            // The same @default profile is also the source Bot's active policy.
            // Editing a switch must take effect on the same Telegram connection.
            let mut permission = serde_json::to_value(agena_domain::PermissionConfig::global_default())?;
            for (update_id, enabled) in [(3, false), (4, true)] {
                permission["plugins"] = json!({"agena.plan":enabled});
                assignments["profiles"] = json!({"@default": {
                    "name":"Default permissions", "complete":true, "revision":update_id, "permission":permission,
                }});
                case.settings().set(SettingsLayer::Global, "access_control", assignments.clone(), agena_testkit::EditOptions::default())
                    .await?.require_success()?;
                let user_text = format!("telegram-plugin-switch-{enabled}");
                let done_text = format!("telegram-plugin-checked-{enabled}");
                let plugins_id = format!("plugins-{enabled}");
                let tools_id = format!("tools-{enabled}");
                let call_id = format!("plan-{enabled}");
                let plugins = model.expect(RequestMatch::user_text(user_text.clone()))
                    .reply(Reply::function_call(plugins_id.clone(), "plugins_list", json!({"plugin":"agena.plan"})));
                let tools = model.expect(RequestMatch::tool_output(plugins_id.clone())).after(&plugins)
                    .check(move |request| {
                        let output = request.tool_output_text(&plugins_id).context("plugin catalog")?;
                        ensure!(output.contains("- agena.plan [") == enabled, "unexpected plugin catalog: {output}");
                        Ok(())
                    }).reply(Reply::function_call(tools_id.clone(), "tools_list", json!({"plugin":"agena.plan"})));
                let call = model.expect(RequestMatch::tool_output(tools_id.clone())).after(&tools)
                    .check(move |request| {
                        let output = request.tool_output_text(&tools_id).context("tool catalog")?;
                        ensure!(output.contains("- plan.get [") == enabled, "unexpected tool catalog: {output}");
                        Ok(())
                    }).reply(Reply::tool_call(call_id.clone(), "plan.get", json!({})));
                model.expect(RequestMatch::tool_output(call_id.clone())).after(&call)
                    .check(move |request| {
                        let output: Value = serde_json::from_str(&request.tool_output_text(&call_id).context("plan response")?)?;
                        ensure!(output["phase"] == if enabled { "succeeded" } else { "failed" }, "unexpected tool result: {output}");
                        Ok(())
                    }).reply(Reply::text(done_text.clone()));
                observed.updates.lock().unwrap().push(update(update_id, private_message(100 + update_id, &user_text)));
                let latest = client.integration_detail(source.session_id, active.id).await?;
                client.control_integration(source.session_id, active.id, &IntegrationControl {
                    not_before_ms: None, expected_revision: latest.binding.revision,
                    action: IntegrationControlAction::PollNow,
                }).await?;
                wait_for(|| async {
                    Ok(observed.sent.lock().unwrap().iter().any(|message| message["text"].as_str().is_some_and(|text| text.contains(&done_text))).then_some(()))
                }, "Telegram reply after plugin permission change").await?;
            }
            let final_detail = client.integration_detail(source.session_id, active.id).await?;
            ensure!(final_detail.channel_sessions.len() == 1 && final_detail.channel_sessions[0].session_id == session_id);
            ensure!(final_detail.binding.state == BindingState::Enabled && final_detail.binding.epoch == recovered.epoch);
            let sent = observed.sent.lock().unwrap();
            ensure!(sent.len() == 5, "unexpected deliveries: {sent:?}");
            ensure!(sent.iter().filter(|message| message["text"].as_str().is_some_and(|text| text.contains("private-history-preserved-response"))).count() == 1);
            Ok(())
        }).await
}
