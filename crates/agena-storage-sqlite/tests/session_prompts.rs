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

fn database_url(directory: &tempfile::TempDir, name: &str) -> String {
    format!(
        "sqlite://{}?mode=rwc",
        directory.path().join(name).display()
    )
}

async fn seed_legacy_session(
    directory: &tempfile::TempDir,
    name: &str,
    retired: serde_json::Value,
) -> (String, i64) {
    let url = database_url(directory, name);
    let runtime = open_kernel_runtime(&url).await.unwrap();
    let workspace = runtime
        .workspaces
        .ensure_id("/legacy-prompt-tests")
        .await
        .unwrap();
    let session = runtime
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Legacy prompt".into(),
            SessionConfiguration::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let raw: String =
        sea_orm::sqlx::query_scalar("SELECT configuration_json FROM agena_sessions WHERE id=?")
            .bind(session.get())
            .fetch_one(runtime.database.get_sqlite_connection_pool())
            .await
            .unwrap();
    let mut configuration: serde_json::Value = serde_json::from_str(&raw).unwrap();
    configuration
        .as_object_mut()
        .unwrap()
        .insert("additional_system_prompt".into(), retired);
    sea_orm::sqlx::query("UPDATE agena_sessions SET configuration_json=? WHERE id=?")
        .bind(serde_json::to_string(&configuration).unwrap())
        .bind(session.get())
        .execute(runtime.database.get_sqlite_connection_pool())
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
    drop(runtime);
    (url, session.get())
}

async fn stored_configuration(url: &str, session: i64) -> SessionConfiguration {
    let runtime = open_kernel_runtime(url).await.unwrap();
    let configuration = runtime
        .ports
        .reader
        .configuration(SessionId::try_from(session).unwrap())
        .await
        .unwrap();
    runtime.ports.clone().shutdown().await.unwrap();
    (*configuration).clone()
}

#[tokio::test]
async fn legacy_text_prompt_becomes_a_session_fragment() {
    let directory = tempfile::tempdir().unwrap();
    let (url, session) = seed_legacy_session(
        &directory,
        "legacy-text.db",
        serde_json::json!("Keep examples concise."),
    )
    .await;

    let configuration = stored_configuration(&url, session).await;
    assert_eq!(
        configuration.prompts.mode,
        agena_prompt_layers::LayerMode::Append
    );
    assert_eq!(configuration.prompts.order.len(), 1);
    assert_eq!(
        configuration.prompts.order[0].as_str(),
        "legacy-session-prompt"
    );
    assert_eq!(configuration.prompts.inline.len(), 1);
    assert_eq!(
        configuration.prompts.inline[0].body,
        "Keep examples concise."
    );

    // A second open is a no-op: the retired key is gone and the migrated layer
    // is not duplicated or overwritten.
    let reopened = stored_configuration(&url, session).await;
    assert_eq!(reopened, configuration);
}

#[tokio::test]
async fn legacy_null_prompt_is_removed_and_the_session_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let (url, session) =
        seed_legacy_session(&directory, "legacy-null.db", serde_json::Value::Null).await;

    let configuration = stored_configuration(&url, session).await;
    assert!(configuration.prompts.is_empty());

    let runtime = open_kernel_runtime(&url).await.unwrap();
    let raw: String =
        sea_orm::sqlx::query_scalar("SELECT configuration_json FROM agena_sessions WHERE id=?")
            .bind(session)
            .fetch_one(runtime.database.get_sqlite_connection_pool())
            .await
            .unwrap();
    assert!(!raw.contains("additional_system_prompt"), "{raw}");
    runtime.ports.clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn legacy_empty_prompt_disables_broader_additional_instructions() {
    let directory = tempfile::tempdir().unwrap();
    let (url, session) =
        seed_legacy_session(&directory, "legacy-empty.db", serde_json::json!("  \n")).await;

    let configuration = stored_configuration(&url, session).await;
    assert_eq!(
        configuration.prompts.mode,
        agena_prompt_layers::LayerMode::Replace
    );
    assert!(configuration.prompts.order.is_empty());
    assert!(configuration.prompts.inline.is_empty());
}

#[tokio::test]
async fn unsupported_legacy_prompt_type_fails_with_a_diagnostic() {
    let directory = tempfile::tempdir().unwrap();
    let (url, _session) =
        seed_legacy_session(&directory, "legacy-invalid.db", serde_json::json!(42)).await;

    let error = open_kernel_runtime(&url).await.err().unwrap();
    let message = error.to_string();
    assert!(
        message.contains("unsupported retired additional_system_prompt"),
        "{message}"
    );
    assert!(message.contains("number"), "{message}");
}
