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
fn omitted_model_lists_from_older_payloads_deserialize_as_empty() {
    let resource: ProviderAdapterModelsResource = serde_json::from_value(serde_json::json!({
        "adapter_id": "openai_responses",
        "enabled": true
    }))
    .expect("deserialize adapter models");

    assert!(resource.models.is_empty());
}
