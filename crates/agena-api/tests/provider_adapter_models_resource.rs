use agena_api::resource::ProviderAdapterModelsResource;

#[test]
fn empty_model_lists_are_serialized_for_browser_consumers() {
    let resource = ProviderAdapterModelsResource {
        adapter_id: "openai_responses".to_owned(),
        enabled: true,
        resolved_base_url: None,
        models: Vec::new(),
        failure: None,
    };

    assert_eq!(
        serde_json::to_value(resource).expect("serialize adapter models"),
        serde_json::json!({
            "adapter_id": "openai_responses",
            "enabled": true,
            "models": []
        })
    );
}

#[test]
fn current_adapter_model_payload_requires_models_and_rejects_unknown_fields() {
    let missing_models =
        serde_json::from_value::<ProviderAdapterModelsResource>(serde_json::json!({
            "adapter_id": "openai_responses",
            "enabled": true
        }))
        .expect_err("current adapter-model payload must include models");
    assert!(
        missing_models
            .to_string()
            .contains("missing field `models`")
    );

    let unknown = serde_json::from_value::<ProviderAdapterModelsResource>(serde_json::json!({
        "adapter_id": "openai_responses",
        "enabled": true,
        "models": [],
        "obsolete": true
    }))
    .expect_err("unknown adapter-model fields must be rejected");
    assert!(unknown.to_string().contains("unknown field `obsolete`"));
}
