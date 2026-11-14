use super::*;
use agena_plugin_host::sdk::Plugin;

fn bot() -> BotIdentity {
    BotIdentity {
        id: 4242,
        username: "agena_agent_bot".into(),
    }
}

fn config() -> TelegramConfig {
    TelegramConfig {
        account: "agena_agent_bot".into(),
        ..Default::default()
    }
}

fn request() -> AdapterPollRequest {
    AdapterPollRequest {
        binding_id: "binding".into(),
        // A binding chooses rooms; the connection itself is plugin settings.
        config: json!({}),
        cursor: Value::Null,
        limit: 32,
        now_ms: 1_700_000_000_000,
        not_before_ms: 1_600_000_000_000,
        channel_identity: None,
        channel_kinds: Vec::new(),
    }
}

fn group_message(text: &str) -> Value {
    json!({
        "message_id": 11,
        "date": 1_699_000_000,
        "chat": {"id": -1001234567890_i64, "type": "supergroup", "title": "Team"},
        "from": {"id": 77, "is_bot": false, "first_name": "Ann", "last_name": "Lee"},
        "text": text,
        "entities": [{"type": "mention", "offset": 0, "length": 16}],
    })
}

#[tokio::test]
async fn manifest_export_publishes_the_integration_service_and_its_settings() {
    let plugin = TelegramPlugin::new();
    let manifest = plugin.manifest();
    assert_eq!(manifest.namespace, "agena");
    assert_eq!(manifest.name, "telegram");
    let export = manifest
        .services
        .exports
        .iter()
        .find(|export| export.id == "agena.integration")
        .expect("telegram must export agena.integration");
    assert_eq!(export.api_version, 1);
    let methods = export
        .methods
        .iter()
        .map(|method| method.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        methods,
        ["describe", "poll", "send"].into_iter().collect(),
        "a channel advertises all three methods"
    );
    assert!(
        manifest
            .commands
            .iter()
            .any(|command| command.id == "check_connection"),
        "Telegram must provide a live connection check for the plugin workbench"
    );
    let contract = manifest
        .settings
        .as_ref()
        .expect("typed settings compile into the manifest contract");
    let encoded = serde_json::to_string(contract).unwrap();
    for field in ["bot_token_env", "account", "api_base", "long_poll_seconds"] {
        assert!(encoded.contains(field), "missing settings field {field}");
    }
}

#[tokio::test]
async fn describe_reports_a_channel_with_verified_context_and_no_replay_promise() {
    let description = TelegramPlugin::new().describe().await.unwrap();
    assert_eq!(description.kind, AdapterKind::Channel);
    assert_eq!(description.channel_context_version, Some(1));
    // Telegram has no idempotency key, so a lost acknowledgement must stay
    // uncertain instead of becoming an automatic replay.
    assert!(!description.idempotent_send);
    assert!(description.minimum_poll_seconds >= 1);
    // Room selection is the binding's job; the connection lives in plugin
    // settings, so the adapter advertises no per-grant fields at all.
    let contract =
        agena_plugin_sdk::macro_support::settings_contract_from_schema(description.config_schema)
            .expect("the advertised binding config must be a valid closed contract");
    assert!(contract.validate_value(&json!({})).is_ok());
    assert!(
        contract
            .validate_value(&json!({"api_base": "https://attacker.example"}))
            .is_err(),
        "a grant must not be able to carry connection settings"
    );
}

#[test]
fn settings_reject_a_token_where_a_variable_name_belongs_and_plain_http_remotely() {
    for api_base in [
        "http://api.telegram.org",
        "https://user:secret@api.telegram.org",
        "ftp://api.telegram.org",
    ] {
        let config = TelegramConfig {
            account: "agena_agent_bot".into(),
            api_base: api_base.into(),
            ..Default::default()
        };
        assert!(
            config.validate().is_err(),
            "endpoint must be rejected: {api_base}"
        );
    }
    let loopback = TelegramConfig {
        account: "agena_agent_bot".into(),
        api_base: "http://127.0.0.1:8081".into(),
        allow_loopback_http: true,
        ..Default::default()
    };
    assert!(loopback.validate().is_ok());
    let missing = TelegramConfig {
        account: "agena_agent_bot".into(),
        bot_token_env: String::new(),
        ..Default::default()
    };
    assert!(missing.validate().is_err());
    let bad_account = TelegramConfig {
        account: "@agena_agent_bot".into(),
        ..Default::default()
    };
    assert!(bad_account.validate().is_err());
}

#[tokio::test]
async fn the_connection_comes_from_plugin_settings_so_no_grant_can_retarget_it() {
    // Either an uninitialized plugin or a binding that tries to override the
    // endpoint must fail rather than quietly reach the public Bot API.
    let plugin = TelegramPlugin::new();
    assert!(
        plugin.config().is_err(),
        "a plugin with no initialized settings must not fall back to a default endpoint"
    );
    plugin
        .settings
        .set(
            TelegramConfig {
                account: "agena_agent_bot".into(),
                api_base: "http://127.0.0.1:9".into(),
                allow_loopback_http: true,
                ..Default::default()
            },
            "init once",
        )
        .unwrap();
    let request = AdapterPollRequest {
        config: json!({"api_base": "https://attacker.example"}),
        ..request()
    };
    // The binding's config is ignored: without the token in the environment the
    // poll fails on the connection it was actually configured with.
    let error = plugin.poll(&request).await.unwrap_err();
    assert!(
        error.to_string().contains("bot_id"),
        "the binding must not override Telegram connection configuration: {error}"
    );
}

#[test]
fn a_token_for_another_bot_is_refused_rather_than_serving_its_rooms() {
    let error = verify_account(
        &config(),
        BotIdentity {
            id: 1,
            username: "someone_else_bot".into(),
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("someone_else_bot"));
    assert!(verify_account(&config(), bot()).is_ok());
}

#[test]
fn a_room_shape_maps_to_the_kind_the_host_policy_speaks() {
    assert_eq!(
        conversation_kind(Some("private")),
        Some(ConversationKind::Direct)
    );
    assert_eq!(
        conversation_kind(Some("group")),
        Some(ConversationKind::Group)
    );
    assert_eq!(
        conversation_kind(Some("supergroup")),
        Some(ConversationKind::Group)
    );
    // A channel is one-to-many: the bot may have no voice there at all.
    assert_eq!(
        conversation_kind(Some("channel")),
        Some(ConversationKind::Broadcast)
    );
    assert_eq!(conversation_kind(Some("unknown")), None);
}

#[test]
fn a_group_message_requiring_a_mention_is_reported_with_verified_evidence_only() {
    let plugin = TelegramPlugin::new();
    let event = plugin
        .message_event(
            &group_message("@agena_agent_bot ship it"),
            &config(),
            &bot(),
            &request(),
            false,
        )
        .expect("a mentioned group message is an event");
    assert_eq!(event.id, "telegram:-1001234567890:11");
    assert_eq!(event.resource, "-1001234567890");
    assert_eq!(event.actor, "77");
    assert_eq!(event.event_type, "message");
    assert_eq!(event.thread, None);
    let channel = event.channel.expect("identity evidence is mandatory");
    assert_eq!(channel.identity.tenant, "telegram");
    assert_eq!(channel.identity.account, "agena_agent_bot");
    assert_eq!(channel.kind, ConversationKind::Group);
    assert!(channel.actor_verified);
    assert!(channel.is_member);
    assert!(channel.addressed_to_bot);
}

#[test]
fn a_mention_found_only_inside_a_link_preview_does_not_address_the_bot() {
    let plugin = TelegramPlugin::new();
    let mut message = group_message("look at this");
    // The text never names the bot; only an entity claims to.
    message["text"] = json!("read https://example.com/@agena_agent_bot");
    message["entities"] = json!([{"type": "url", "offset": 5, "length": 31}]);
    message["url"] = json!("https://example.com/@agena_agent_bot");
    let event = plugin
        .message_event(&message, &config(), &bot(), &request(), false)
        .expect("the message is still an event");
    assert!(!event.channel.unwrap().addressed_to_bot);
}

#[test]
fn a_reply_to_the_bot_addresses_it_without_any_mention() {
    let plugin = TelegramPlugin::new();
    let mut message = group_message("and then?");
    message["entities"] = json!([]);
    message["reply_to_message"] = json!({"from": {"id": 4242, "is_bot": true}});
    let event = plugin
        .message_event(&message, &config(), &bot(), &request(), false)
        .expect("a reply is an event");
    assert!(event.channel.unwrap().addressed_to_bot);
}

#[test]
fn a_command_addressed_to_this_bot_counts_but_one_for_another_bot_does_not() {
    let plugin = TelegramPlugin::new();
    let mine = json!({
        "message_id": 5, "date": 1_699_000_000,
        "chat": {"id": -9, "type": "group"},
        "from": {"id": 3, "first_name": "Bo"},
        "text": "/help@agena_agent_bot",
        "entities": [{"type": "bot_command", "offset": 0, "length": 21}],
    });
    assert!(
        plugin
            .message_event(&mine, &config(), &bot(), &request(), false)
            .unwrap()
            .channel
            .unwrap()
            .addressed_to_bot
    );
    let other = json!({
        "message_id": 6, "date": 1_699_000_000,
        "chat": {"id": -9, "type": "group"},
        "from": {"id": 3, "first_name": "Bo"},
        "text": "/help@different_bot",
        "entities": [{"type": "bot_command", "offset": 0, "length": 18}],
    });
    assert!(
        !plugin
            .message_event(&other, &config(), &bot(), &request(), false)
            .unwrap()
            .channel
            .unwrap()
            .addressed_to_bot
    );
}

#[test]
fn a_private_chat_never_carries_a_thread_because_the_host_refuses_one() {
    let plugin = TelegramPlugin::new();
    let message = json!({
        "message_id": 3, "date": 1_699_000_000,
        "chat": {"id": 900, "type": "private"},
        "from": {"id": 900, "first_name": "Ann"},
        "text": "hello",
        "message_thread_id": 4,
    });
    let event = plugin
        .message_event(&message, &config(), &bot(), &request(), false)
        .expect("a direct message is an event");
    let channel = event.channel.expect("identity evidence is mandatory");
    assert_eq!(channel.kind, ConversationKind::Direct);
    assert!(channel.addressed_to_bot);
    assert_eq!(event.thread, None);
}

#[test]
fn a_channel_post_is_broadcast_and_attributed_to_the_channel_itself() {
    let plugin = TelegramPlugin::new();
    let post = json!({
        "message_id": 42, "date": 1_699_000_000,
        "chat": {"id": -1009, "type": "channel", "title": "Announcements"},
        "text": "release notes",
    });
    let event = plugin
        .message_event(&post, &config(), &bot(), &request(), true)
        .expect("a channel post is an event");
    assert_eq!(
        event.channel.as_ref().unwrap().kind,
        ConversationKind::Broadcast
    );
    assert_eq!(event.actor, "channel:-1009");
    // A broadcast surface never addresses the bot.
    assert!(!event.channel.unwrap().addressed_to_bot);
}

#[test]
fn a_message_without_text_keeps_its_place_in_the_conversation() {
    let plugin = TelegramPlugin::new();
    let photo = json!({
        "message_id": 7, "date": 1_699_000_000,
        "chat": {"id": 900, "type": "private"},
        "from": {"id": 900, "first_name": "Ann"},
        "photo": [{"file_id": "abc"}],
    });
    let event = plugin
        .message_event(&photo, &config(), &bot(), &request(), false)
        .expect("an attachment is still a message");
    assert!(event.text.contains("photo omitted"));
    assert_eq!(event.data["telegram"]["media"], json!("photo"));
}

#[test]
fn history_from_before_the_grant_is_checkpointed_instead_of_answered() {
    let plugin = TelegramPlugin::new();
    let mut request = request();
    request.not_before_ms = 1_699_000_000_000 + 1;
    assert!(
        plugin
            .message_event(&group_message("old"), &config(), &bot(), &request, false)
            .is_none()
    );
}

#[test]
fn a_room_shape_outside_the_binding_is_dropped_before_it_crosses_the_wire() {
    let plugin = TelegramPlugin::new();
    let mut request = request();
    request.channel_kinds = vec![ConversationKind::Direct];
    assert!(
        plugin
            .message_event(
                &group_message("@agena_agent_bot hi"),
                &config(),
                &bot(),
                &request,
                false
            )
            .is_none()
    );
}

#[test]
fn leaving_a_chat_becomes_a_verified_revocation_not_a_message() {
    let change = json!({
        "chat": {"id": -1001234567890_i64, "type": "supergroup"},
        "date": 1_699_000_100,
        "old_chat_member": {"status": "member", "user": {"id": 77}},
        "new_chat_member": {"status": "left", "user": {"id": 77}},
    });
    let event = revocation_event(&change, &config(), &request()).expect("a departure is revocable");
    assert_eq!(event.event_type, "membership.revoked");
    assert_eq!(event.actor, "77");
    let channel = event.channel.unwrap();
    assert!(channel.actor_verified);
    assert!(!channel.is_member, "the host revokes on this evidence");

    // An update that leaves someone still in the chat grants nothing.
    let still = json!({
        "chat": {"id": -1001234567890_i64, "type": "supergroup"},
        "date": 1_699_000_100,
        "old_chat_member": {"status": "member", "user": {"id": 77}},
        "new_chat_member": {"status": "member", "user": {"id": 77}},
    });
    assert!(revocation_event(&still, &config(), &request()).is_none());
}

#[test]
fn an_api_refusal_maps_to_the_receipt_vocabulary() {
    let limited = ApiFailure {
        transport: false,
        status: Some(429),
        code: Some(429),
        description: "Too Many Requests".into(),
        retry_after: Some(7),
    };
    assert_eq!(limited.receipt_disposition(), SendDisposition::Retryable);
    assert!(limited.transient());

    let broken_text = ApiFailure {
        transport: false,
        status: Some(400),
        code: Some(400),
        description: "Bad Request: can't parse entities".into(),
        retry_after: None,
    };
    assert_eq!(broken_text.receipt_disposition(), SendDisposition::Rejected);
    assert!(broken_text.mentions_entity_parsing());

    let offline = ApiFailure {
        transport: true,
        status: None,
        code: None,
        description: "unreachable".into(),
        retry_after: None,
    };
    assert_eq!(offline.receipt_disposition(), SendDisposition::Retryable);
}

#[test]
fn markdown_becomes_the_tag_subset_telegram_renders() {
    assert_eq!(
        markdown_to_html("**bold** and `code` and ~~gone~~"),
        "<b>bold</b> and <code>code</code> and <s>gone</s>"
    );
    assert_eq!(
        markdown_to_html("[docs](https://example.com/a)"),
        "<a href=\"https://example.com/a\">docs</a>"
    );
    // Angle brackets the model wrote stay literal, so they can never become
    // tags that unbalance the message.
    assert_eq!(markdown_to_html("a < b & c > d"), "a &lt; b &amp; c &gt; d");
    assert_eq!(
        markdown_to_html("```rust\nlet x = 1 < 2;\n```"),
        "<pre><code class=\"language-rust\">let x = 1 &lt; 2;</code></pre>\n"
    );
    assert_eq!(
        markdown_to_html("> quoted"),
        "<blockquote>quoted</blockquote>"
    );
    // A URL Telegram would not accept stays as written rather than becoming a
    // broken link tag.
    assert_eq!(
        markdown_to_html("[x](javascript:alert(1))"),
        "[x](javascript:alert(1))"
    );
}

#[test]
fn every_opening_tag_has_its_closing_tag() {
    for source in [
        "**unclosed bold",
        "`unclosed code",
        "~~unclosed",
        "**a *b** c*",
        "> quote **with** code",
    ] {
        let html = markdown_to_html(source);
        for tag in ["b", "i", "s", "code"] {
            let opens = html.matches(&format!("<{tag}>")).count();
            let closes = html.matches(&format!("</{tag}>")).count();
            assert_eq!(opens, closes, "unbalanced <{tag}> in {source:?}: {html}");
        }
    }
}

#[test]
fn short_text_is_one_chunk() {
    assert_eq!(split_markdown("hello", 100), vec!["hello".to_owned()]);
}

#[test]
fn chunks_respect_the_limit_and_keep_every_character() {
    let text = (0..80)
        .map(|index| format!("line {index} with some words\n"))
        .collect::<String>();
    let chunks = split_markdown(&text, 200);
    assert!(chunks.len() > 1);
    for chunk in &chunks {
        assert!(
            utf16_len(chunk) <= 200,
            "chunk exceeds the limit: {chunk:?}"
        );
    }
    assert_eq!(chunks.concat(), text);
}

#[test]
fn a_split_inside_a_fence_reopens_it_so_no_chunk_is_unterminated() {
    let body = (0..40)
        .map(|index| format!("    let value_{index} = {index};\n"))
        .collect::<String>();
    let text = format!("Here is the code:\n```rust\n{body}```\nDone.\n");
    let chunks = split_markdown(&text, 200);
    assert!(chunks.len() > 1);
    for chunk in &chunks {
        assert!(utf16_len(chunk) <= 200);
        let fences = chunk.matches("```").count();
        assert_eq!(
            fences % 2,
            0,
            "a chunk left a code fence open, which Telegram renders as broken:\n{chunk}"
        );
    }
}

#[test]
fn chunk_bounds_are_counted_in_utf16_units_so_emoji_do_not_overflow() {
    // A surrogate pair costs two UTF-16 units but four bytes, so a byte-based
    // limit would let this overflow Telegram's own bound.
    let text = "🚀".repeat(60);
    let chunks = split_markdown(&text, 64);
    for chunk in &chunks {
        assert!(utf16_len(chunk) <= 64, "overflowing chunk: {chunk:?}");
    }
    assert_eq!(chunks.concat(), text);
}

#[test]
fn utf16_offsets_slice_a_mention_out_of_a_plane_one_message() {
    let message = json!({
        "message_id": 1, "date": 1_699_000_000,
        "chat": {"id": 2, "type": "group"},
        "from": {"id": 3, "first_name": "Bo"},
        "text": "🚀 @agena_agent_bot go",
        // The rocket is one surrogate pair, so the mention starts at unit 3.
        "entities": [{"type": "mention", "offset": 3, "length": 16}],
    });
    let plugin = TelegramPlugin::new();
    let event = plugin
        .message_event(&message, &config(), &bot(), &request(), false)
        .expect("an event");
    assert!(event.channel.unwrap().addressed_to_bot);
}
