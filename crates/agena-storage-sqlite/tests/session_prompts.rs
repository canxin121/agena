use agena_prompt_layers::{FragmentId, PromptFragment, PromptLayer};
use agena_storage::kernel::*;
use agena_storage_sqlite::open_kernel_runtime;

fn layer(body: &str) -> PromptLayer {
    PromptLayer {
        mode: agena_prompt_layers::LayerMode::Append,
        order: vec![FragmentId::parse("session-note").unwrap()],
        disabled: Vec::new(),
        inline: vec![PromptFragment::new(
            FragmentId::parse("session-note").unwrap(),
            body,
        )],
    }
}

#[tokio::test]
async fn session_prompt_layer_round_trips_and_updates() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime.workspaces.ensure_id("/prompt-tests").await.unwrap();
    let session = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Prompt test".into(),
            SessionConfiguration {
                prompts: layer("initial"),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    let stored = runtime
        .ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange::default(),
        )
        .await
        .unwrap()
        .value;
    assert_eq!(stored.prompts.inline[0].body, "initial");

    runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                prompts: Some(layer("updated")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let stored = runtime
        .ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange::default(),
        )
        .await
        .unwrap()
        .value;
    assert_eq!(stored.prompts.inline[0].body, "updated");
}

#[tokio::test]
async fn empty_prompt_layer_clears_the_session_layer() {
    let runtime = open_kernel_runtime("sqlite::memory:").await.unwrap();
    let workspace = runtime.workspaces.ensure_id("/prompt-tests").await.unwrap();
    let session = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Prompt clear test".into(),
            SessionConfiguration {
                prompts: layer("initial"),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;

    runtime
        .ports
        .sessions
        .update_metadata(
            RequestId::new(),
            session,
            MetadataChange {
                prompts: Some(PromptLayer::default()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Omitting the field on a later update keeps the cleared state.
    let stored = runtime
        .ports
        .sessions
        .configure(
            RequestId::new(),
            session,
            SessionConfigurationChange::default(),
        )
        .await
        .unwrap()
        .value;
    assert!(stored.prompts.is_empty());
}

#[test]
fn retired_configuration_fields_no_longer_deserialize() {
    let error = serde_json::from_str::<SessionConfiguration>(
        r#"{"additional_system_prompt":"legacy text"}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("additional_system_prompt"));
}
