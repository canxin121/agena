use std::os::unix::fs::PermissionsExt as _;

use agena_api::commands::CreateSessionParams;
use agena_domain::ExternalAgent;
use agena_testkit::TestResult;
use anyhow::{Context as _, ensure};
use serde_json::json;

use super::fixture;

async fn run_provider_scenario(version: &'static str) -> TestResult {
    let host = tempfile::tempdir()?;
    let executable = host.path().join("claude-agent-acp");
    std::fs::write(&executable, include_str!("plugins/fake_acp_providers.py"))?;
    std::fs::set_permissions(executable, std::fs::Permissions::from_mode(0o755))?;
    let routes = host.path().join("providers.json");
    std::fs::write(
        &routes,
        serde_json::to_vec(&json!({"claude-code": [
            {"action":"set", "providerId":"main", "apiType":"_custom_provider",
             "baseUrl":"https://configured.example.invalid/llm", "headers":{
                 "Authorization":{"env":"AGENA_TEST_PROVIDER_HEADER"}, "X-Project":"routing-fixture"
             }, "_meta":{"test":{"mode":"full"}}},
            {"action":"disable", "providerId":"optional"}
        ]}))?,
    )?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(
        std::iter::once(host.path().to_owned()).chain(std::env::split_paths(&path)),
    )?;
    fixture()
        .env("PATH", path)
        .env("AGENA_ACP_PROVIDERS_FILE", routes.clone())
        .env("AGENA_TEST_PROVIDER_HEADER", "Bearer fixture-private-value")
        .configure_global(|config| config["sandbox"] = json!(false))
        .seed_file("workspace/.providers-version", version.as_bytes().to_vec())
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let workspace_id = ordinary.snapshot().await?.session.workspace_id.context("workspace")?;
            let session = ordinary.client().create_session_configured(CreateSessionParams {
                workspace_id,
                title: "ACP routing".into(),
                external_agent: Some(ExternalAgent::ClaudeCode),
                auto_title: false,
                parent_id: None,
                profile: None,
                prompts: None,
            }).await?;
            let user = case.user().attach_session(session.id).await?;
            let catalog = user.client().external_agent_providers(session.id).await?;
            ensure!(catalog.supported && catalog.providers.len() == 2);
            let main = &catalog.providers[0];
            let current = main.current.as_ref().context("effective route")?;
            ensure!(main.provider_id == "main" && main.required);
            ensure!(current.api_type == "_custom_provider" && current.base_url == "https://configured.example.invalid/llm");
            ensure!(catalog.providers[1].provider_id == "optional" && catalog.providers[1].current.is_none());
            ensure!(!serde_json::to_string(&catalog)?.contains("fixture-private-value"));

            user.client().external_agent_options(session.id, None).await?;
            for text in ["first routing prompt", "continue under configured route"] {
                let run = user.send(text).await?.wait_settled().await?;
                run.assert_succeeded()?;
                run.assert_assistant_text("provider routing applied")?;
            }
            let usage = user.client().session_cost_summary(session.id).await?;
            ensure!(usage.input_tokens == 26 && usage.output_tokens == 14 && usage.cache_read_tokens == 10 && usage.total_tokens == 52);
            let wire_path = case.workspace().join(".providers-wire");
            let wire = tokio::fs::read_to_string(&wire_path).await?;
            ensure!(wire.lines().filter(|line| *line == "providers/set").count() == 4, "{wire}");
            ensure!(wire.lines().filter(|line| *line == "providers/disable").count() == 4, "{wire}");
            ensure!(wire.lines().any(|line| line == "session/resume"), "{wire}");

            // Discovery rejects the complete invalid set before emitting a mutation.
            // Protocol version is already memoized from earlier requests on this peer.
            tokio::fs::write(&routes, b"{\"claude-code\":[{\"action\":\"disable\",\"providerId\":\"optional\"},{\"action\":\"disable\",\"providerId\":\"main\"}]}").await?;
            ensure!(user.client().external_agent_providers(session.id).await.is_err());
            let updated = tokio::fs::read_to_string(&wire_path).await?;
            ensure!(
                updated.strip_prefix(&wire) == Some("initialize\nproviders/list\n"),
                "{updated}"
            );

            tokio::fs::write(case.workspace().join(".providers-unsupported"), "").await?;
            let unsupported = user.client().external_agent_providers(session.id).await?;
            ensure!(!unsupported.supported && unsupported.providers.is_empty());
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_provider_configuration_applies_before_each_native_session() -> TestResult {
    run_provider_scenario("1").await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_provider_configuration_applies_before_each_native_session() -> TestResult {
    run_provider_scenario("2").await
}
