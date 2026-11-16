//! Shared sandbox dispatch for resource operations owned by bundled plugins.
use std::path::Path;

use agena_plugin_sdk::{
    PluginError, Result,
    host_api::{HostClient, HostProcessResources, current_host_callback_context},
};
use agena_process::sandbox::{SandboxNetwork, SandboxPolicy};

pub(crate) fn enabled() -> bool {
    agena_process::sandbox::enabled()
        && current_host_callback_context().and_then(|context| context.sandbox_enabled)
            != Some(false)
}

pub(crate) async fn policy(
    host: &dyn HostClient,
    resources: HostProcessResources,
) -> Result<SandboxPolicy> {
    if !enabled() {
        return Ok(SandboxPolicy {
            disabled: true,
            ..Default::default()
        });
    }
    let prepared = host.prepare_worker_sandbox(resources).await?;
    serde_json::from_value(prepared.policy)
        .map_err(|error| PluginError::internal(format!("invalid host sandbox policy: {error}")))
}

pub(crate) async fn read_file(
    host: &dyn HostClient,
    path: &Path,
    maximum: u64,
    expected: Option<&str>,
) -> Result<Vec<u8>> {
    crate::permissions::require_path(host, agena_domain::PathKind::Read, &path.to_string_lossy())
        .await?;
    let mut policy = policy(
        host,
        HostProcessResources {
            reads: vec![path.to_string_lossy().into_owned()],
            ..Default::default()
        },
    )
    .await?;
    policy.network = SandboxNetwork::Disabled;
    agena_process::sandbox_file::read(path, maximum, expected, &policy)
        .await
        .map_err(|error| PluginError::invalid_params(format!("input file read failed: {error}")))
}

pub(crate) async fn read_media(
    host: &dyn HostClient,
    path: &Path,
    expected: Option<&str>,
) -> Result<agena_runtime_tools::media_input::PreparedMedia> {
    crate::permissions::require_path(host, agena_domain::PathKind::Read, &path.to_string_lossy())
        .await?;
    let mut policy = policy(
        host,
        HostProcessResources {
            reads: vec![path.to_string_lossy().into_owned()],
            ..Default::default()
        },
    )
    .await?;
    policy.network = SandboxNetwork::Disabled;
    agena_runtime_tools::sandbox_worker::read_media_with_policy(path, expected, &policy)
        .await
        .map_err(|error| error.into_plugin_error())
}

pub(crate) async fn inspect_media(
    host: &dyn HostClient,
    filename: &str,
    bytes: Vec<u8>,
) -> Result<Option<agena_runtime_tools::media_input::PreparedMedia>> {
    let mut policy = policy(host, HostProcessResources::default()).await?;
    policy.network = SandboxNetwork::Disabled;
    agena_runtime_tools::sandbox_worker::inspect_media_bytes_with_policy(filename, bytes, &policy)
        .await
        .map_err(|error| error.into_plugin_error())
}

pub(crate) async fn http_route(
    host: &dyn HostClient,
    url: &url::Url,
    route: agena_web::WebHttpRoute,
) -> Result<agena_web::WebHttpRoute> {
    let cancellation = agena_plugin_host::current_plugin_call_completion()
        .map(|completion| completion.cancellation_token());
    let policy = policy(
        host,
        HostProcessResources {
            network: vec![url.origin().ascii_serialization()],
            ..Default::default()
        },
    )
    .await?;
    route
        .with_sandbox_policy(url, policy)
        .map(|route| route.with_cancellation(cancellation))
        .map_err(|error| PluginError::invalid_params(error.to_string()))
}
