//! Native workers for bundled operations that need catalog or business state.
//! Requests contain an explicit operation and minimal serializable data. No
//! application configuration, database, plugin host or model is bootstrapped.
use agena_plugin_sdk::host_api::{HostPreparedProcessSandbox, HostProcessResources};
use agena_plugin_sdk::{HostClient, PluginError, PluginErrorKind, Result as SdkResult};
use agena_process::sandbox::{SandboxNetwork, SandboxPolicy};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{Read, Write},
    time::Duration,
};

const ARGUMENT: &str = "--agena-plugin-worker";
const MAX_TRANSFER: usize = 256 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operation: String,
    input: Value,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Response {
    Ok { result: Value },
    Error { error: PluginError },
}

fn network_operation(operation: &str) -> SdkResult<bool> {
    match operation {
        "skills.manager" | "skills.source" | "memory.store" => Ok(false),
        "provider.realtime" => Ok(true),
        _ => Err(PluginError::invalid_params(format!(
            "unsupported bundled worker operation: {operation}"
        ))),
    }
}

pub async fn execute_bundled_worker(
    host: &dyn HostClient,
    operation: &str,
    input: Value,
    resources: HostProcessResources,
) -> SdkResult<Value> {
    network_operation(operation)?;
    let prepared = prepare_bundled_worker(host, resources).await?;
    execute_prepared_bundled_worker(prepared, operation, input).await
}

pub(crate) struct PreparedBundledWorker {
    prepared: HostPreparedProcessSandbox,
    cancellation: agena_process::sandbox_http::WorkerCancellation,
}

impl PreparedBundledWorker {
    pub(crate) fn cancellation_token(&self) -> agena_process::sandbox_http::WorkerCancellation {
        self.cancellation.clone()
    }
}

pub(crate) async fn prepare_bundled_worker(
    host: &dyn HostClient,
    resources: HostProcessResources,
) -> SdkResult<PreparedBundledWorker> {
    let completion = agena_plugin_host::current_plugin_call_completion();
    let prepared = host.prepare_worker_sandbox(resources).await?;
    if prepared.permission_snapshot.is_none() {
        return Err(PluginError::internal(
            "native worker preparation did not supply a permission snapshot",
        ));
    }
    if prepared.call_bound && completion.is_none() {
        return Err(PluginError::from_kind(
            PluginErrorKind::PolicyDenied,
            "call-bound worker requires its originating call lifetime",
        ));
    }
    let cancellation = completion
        .as_ref()
        .map(|completion| completion.cancellation_token())
        .unwrap_or_default();
    Ok(PreparedBundledWorker {
        prepared,
        cancellation,
    })
}

/// Preparation may occur before acquiring a host-side blocking mutation lock;
/// only the trusted result of HostClient::prepare_worker_sandbox is accepted.
pub(crate) async fn execute_prepared_bundled_worker(
    worker: PreparedBundledWorker,
    operation: &str,
    mut input: Value,
) -> SdkResult<Value> {
    let PreparedBundledWorker {
        prepared,
        cancellation,
    } = worker;
    let uses_network = network_operation(operation)?;
    let mut policy: SandboxPolicy = serde_json::from_value(prepared.policy)
        .map_err(|error| PluginError::internal_error(&error))?;
    if !uses_network {
        policy.network = SandboxNetwork::Disabled;
    }
    let object = input
        .as_object_mut()
        .ok_or_else(|| PluginError::invalid_params("worker input must be an object"))?;
    // Never accept a permission snapshot supplied in operation arguments.
    object.insert(
        "_agena_permission_snapshot".into(),
        prepared.permission_snapshot.unwrap_or(Value::Null),
    );
    let request = serde_json::to_vec(&Request {
        operation: operation.to_owned(),
        input,
    })
    .map_err(|error| PluginError::internal_error(&error))?;
    if request.len() > MAX_TRANSFER {
        return Err(PluginError::invalid_params(
            "bundled worker request exceeds 256 MiB",
        ));
    }
    let (_scratch, command) = agena_process::worker::command(ARGUMENT, &policy)
        .map_err(|error| PluginError::internal_error(&error))?;
    let output = agena_process::output_with_input_cancellable(
        command,
        &request,
        Duration::from_secs(180),
        MAX_TRANSFER,
        Some(cancellation),
    )
    .await
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::Interrupted {
            PluginError::from_kind(PluginErrorKind::Cancelled, error.to_string())
        } else {
            PluginError::internal_error(&error)
        }
    })?;
    if !output.status.success() {
        return Err(PluginError::internal(format!(
            "MXC bundled worker failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    match serde_json::from_slice::<Response>(&output.stdout).map_err(|error| {
        PluginError::internal(format!("invalid bundled worker response: {error}"))
    })? {
        Response::Ok { result } => Ok(result),
        Response::Error { error } => Err(error),
    }
}

async fn dispatch(operation: &str, input: Value) -> SdkResult<Value> {
    network_operation(operation)?;
    match operation {
        "skills.manager" | "skills.source" => {
            crate::plugins::provided::skills::execute_worker(operation, input)
        }
        "memory.store" => crate::memory::execute_worker(operation, input),
        "provider.realtime" => {
            crate::plugins::provided::provider_api::execute_realtime_worker(input).await
        }
        _ => unreachable!(),
    }
}

pub fn try_run_worker() -> Option<i32> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(ARGUMENT)) {
        return None;
    }
    let result = (|| -> SdkResult<Value> {
        if std::env::args_os().count() != 2 {
            return Err(PluginError::invalid_params(
                "invalid bundled worker arguments",
            ));
        }
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(MAX_TRANSFER as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| PluginError::internal_error(&error))?;
        if bytes.len() > MAX_TRANSFER {
            return Err(PluginError::invalid_params(
                "bundled worker request exceeds 256 MiB",
            ));
        }
        let request: Request = serde_json::from_slice(&bytes)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| PluginError::internal_error(&error))?;
        runtime.block_on(dispatch(&request.operation, request.input))
    })();
    let response = match result {
        Ok(result) => Response::Ok { result },
        Err(error) => Response::Error { error },
    };
    let result = (|| -> std::io::Result<()> {
        let bytes = serde_json::to_vec(&response).map_err(std::io::Error::other)?;
        if bytes.len() > MAX_TRANSFER {
            return Err(std::io::Error::other(
                "bundled worker response exceeds 256 MiB",
            ));
        }
        let mut output = std::io::stdout().lock();
        output.write_all(&bytes)?;
        output.flush()
    })();
    Some(match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("bundled worker response failed: {error}");
            1
        }
    })
}
