//! Attach the shared use-case dispatcher to Runtime without an ownership cycle.

use std::sync::Arc;

use agena_session_api::bridge::{SessionApiBackend, SessionApiFuture};

use crate::{Application, ApplicationError, dispatch};

struct ApplicationBackend {
    application: Application,
}

impl SessionApiBackend for ApplicationBackend {
    fn query(&self, query: agena_session_api::Query) -> SessionApiFuture<'_> {
        Box::pin(async move {
            let (result, tasks) = agena_runtime::capture_deferred_reloads(
                dispatch::dispatch_query(&self.application, query),
            )
            .await;
            encode_result(result.map_err(|error| *error.failure)?, tasks)
        })
    }

    fn command(&self, command: agena_session_api::Command) -> SessionApiFuture<'_> {
        Box::pin(async move {
            let (result, tasks) = agena_runtime::capture_deferred_reloads(
                dispatch::dispatch_command(&self.application, command),
            )
            .await;
            encode_result(result.map_err(|error| *error.failure)?, tasks)
        })
    }
}

impl Application {
    /// Constructed before the owning Application stores its backend. The copy
    /// shares every runtime port, notification store and cache, but does not
    /// own this backend itself. Runtime retains only its weak slot reference.
    pub(crate) fn create_session_api_backend(&self) -> Arc<dyn SessionApiBackend> {
        Arc::new(ApplicationBackend {
            application: self.clone(),
        })
    }

    /// Compose a complete in-process application without starting HTTP.
    /// CLI/embedded callers receive the same attached HostClient backend as
    /// server processes and retain the runtime's explicit shutdown owner.
    pub async fn bootstrap(
        request: agena_runtime::RuntimeBootstrapRequest,
    ) -> Result<(Self, agena_runtime::RuntimeBootstrapResult), ApplicationError> {
        let runtime = agena_runtime::bootstrap_application_services(request)
            .await
            .map_err(|error| ApplicationError::internal_error(&error))?;
        match Self::from_composed_runtime_services(runtime.application_services()) {
            Ok(application) => {
                if let Err(error) = application.runtime_control().application_ready().await {
                    runtime.shutdown();
                    return Err(ApplicationError::from(error));
                }
                Ok((application, runtime))
            }
            Err(error) => {
                runtime.shutdown();
                Err(error)
            }
        }
    }
}

fn encode_result(
    result: impl serde::Serialize,
    tasks: Vec<String>,
) -> Result<serde_json::Value, agena_failure::Failure> {
    let mut value = serde_json::to_value(result)
        .map_err(|error| *ApplicationError::internal_error(&error).failure)?;
    if !tasks.is_empty() {
        value
            .as_object_mut()
            .expect("tagged command/query result")
            .insert("deferred_reload_task_ids".into(), serde_json::json!(tasks));
    }
    Ok(value)
}

#[cfg(all(test, unix))]
mod tests;
