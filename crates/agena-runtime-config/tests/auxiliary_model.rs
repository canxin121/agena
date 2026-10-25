use agena_runtime_config::RawConfig;
use serde_json::{Value, json};

fn config() -> Value {
    json!({"providers": {"local": {
        "auth":{"mode":"api","subtype":"custom","base_url":"http://localhost:1234","api_key":{"kind":"env","value":"AGENA_TITLE_TEST_API_KEY"}},"adapters":{"openai_chat_completions":{"enabled":true}}
    }}})
}

#[test]
fn auxiliary_selection_is_validated_and_does_not_change_the_default() {
    let mut value = config();
    value["runtime"] = json!({"auxiliary_model":{"provider":"local","adapter":"openai_chat_completions","model":"small","thinking_mode":"low"}});
    let resolved = serde_json::from_value::<RawConfig>(value)
        .unwrap()
        .resolve()
        .unwrap();
    assert!(resolved.default_selection.is_empty());
    assert_eq!(
        resolved.runtime.auxiliary_model.unwrap().model.as_deref(),
        Some("small")
    );
}

#[test]
fn partial_unknown_and_disabled_auxiliary_routes_are_rejected() {
    for selection in [
        json!({"model":"small"}),
        json!({"provider":"local"}),
        json!({"provider":"missing","model":"small"}),
        json!({"provider":"local","adapter":"missing","model":"small"}),
    ] {
        let mut value = config();
        value["runtime"] = json!({"auxiliary_model":selection});
        assert!(
            serde_json::from_value::<RawConfig>(value)
                .unwrap()
                .resolve()
                .is_err()
        );
    }
    for disabled_provider in [false, true] {
        let mut value = config();
        value["runtime"] = json!({"auxiliary_model":{"provider":"local","adapter":"openai_chat_completions","model":"small"}});
        if disabled_provider {
            value["providers"]["local"]["enabled"] = json!(false);
        } else {
            value["providers"]["local"]["adapters"]["openai_chat_completions"]["enabled"] =
                json!(false);
        }
        assert!(
            serde_json::from_value::<RawConfig>(value)
                .unwrap()
                .resolve()
                .is_err()
        );
    }
}

#[test]
fn workspace_overrides_replace_auxiliary_model_as_a_unit_and_merge_title_policy() {
    let mut global = config();
    global["runtime"] =
        json!({"auxiliary_model":{"provider":"local","model":"global","thinking_mode":"high"}});
    global["session"] = json!({"auto_title":{"message_interval":7,"cooldown_seconds":60}});
    let mut global = serde_json::from_value::<RawConfig>(global).unwrap();
    global.merge_from(
        serde_json::from_value(json!({
            "runtime":{"auxiliary_model":{"provider":"local","model":"workspace"}},
            "session":{"auto_title":{"enabled":false}}
        }))
        .unwrap(),
    );
    let resolved = global.resolve().unwrap();
    let model = resolved.runtime.auxiliary_model.unwrap();
    assert_eq!(model.model.as_deref(), Some("workspace"));
    assert!(model.thinking_mode.is_none());
    assert!(!resolved.session.auto_title.enabled);
    assert_eq!(resolved.session.auto_title.message_interval, 7);
    assert_eq!(resolved.session.auto_title.cooldown_seconds, 60);
}

#[test]
fn invalid_title_interval_is_rejected_and_unset_model_uses_fallback() {
    let value = json!({"session":{"auto_title":{"message_interval":0}}});
    assert!(
        serde_json::from_value::<RawConfig>(value)
            .unwrap()
            .resolve()
            .is_err()
    );
    assert!(
        RawConfig::default()
            .resolve()
            .unwrap()
            .runtime
            .auxiliary_model
            .is_none()
    );
}
