//! Schema-neutral input for concrete runtime bootstrap.

use std::path::PathBuf;

use crate::ConfigEnvironment;

/// Schema-neutral bootstrap information needed before the full runtime is
/// composed. Concrete configuration adapters project their early resolution
/// into this value so entrypoints never need a concrete configuration result.
#[derive(Debug, Clone)]
pub struct RuntimeBootstrapPreflight {
    pub workspace_root: PathBuf,
    pub tracing: crate::RuntimeTracingConfiguration,
}

/// Resolve the narrow process configuration required before full bootstrap.
/// This deliberately reads only Runtime-owned tracing fields; full schema
/// validation remains at the concrete configuration adapter boundary.
pub fn resolve_runtime_bootstrap_preflight(
    request: &RuntimeBootstrapRequest,
) -> Result<RuntimeBootstrapPreflight, crate::RuntimeBootstrapError> {
    let workspace_root = request
        .workspace_root
        .clone()
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)
        .map_err(|error| crate::RuntimeBootstrapError::io(error.to_string()))?;
    let environment = crate::ProcessEnvironment;
    let mut tracing = crate::RuntimeTracingConfiguration::default();
    apply_tracing_file(crate::default_config_path(&environment), &mut tracing)?;
    apply_tracing_file(
        crate::project_config_path(workspace_root.as_path()),
        &mut tracing,
    )?;
    for (name, target) in [
        ("AGENA_LOG", &mut tracing.filter),
        ("AGENA_DATABASE_LOG", &mut tracing.database),
        ("AGENA_ADAPTER_LOG", &mut tracing.adapter),
    ] {
        if let Some(value) = environment.var(name) {
            *target = value;
        }
    }
    for expression in &request.config_override_expressions {
        let Some((key, value)) = expression.split_once('=') else {
            continue;
        };
        match key.trim() {
            "tracing.filter" => tracing.filter = value.trim().to_owned(),
            "tracing.database" => tracing.database = value.trim().to_owned(),
            "tracing.adapter" => tracing.adapter = value.trim().to_owned(),
            _ => {}
        }
    }
    Ok(RuntimeBootstrapPreflight {
        workspace_root,
        tracing,
    })
}

fn apply_tracing_file(
    path: PathBuf,
    tracing: &mut crate::RuntimeTracingConfiguration,
) -> Result<(), crate::RuntimeBootstrapError> {
    let Some(value) = crate::read_config_json(&path).map_err(|error| {
        let message = error.to_string();
        if matches!(error, crate::ConfigError::ReadFile { .. }) {
            crate::RuntimeBootstrapError::io(message)
        } else {
            crate::RuntimeBootstrapError::configuration(message)
        }
    })?
    else {
        return Ok(());
    };
    let Some(config) = value.get("tracing").and_then(serde_json::Value::as_object) else {
        return Ok(());
    };
    for (name, target) in [
        ("filter", &mut tracing.filter),
        ("database", &mut tracing.database),
        ("adapter", &mut tracing.adapter),
    ] {
        if let Some(value) = config.get(name).and_then(serde_json::Value::as_str) {
            *target = value.to_owned();
        }
    }
    Ok(())
}

/// Inputs supplied by a process entrypoint before configuration schema
/// resolution. Override expressions intentionally remain raw text at this
/// process boundary; Runtime parses them into its generic `ConfigOverride`
/// values, while applying them to the resolved Runtime configuration remains
/// inside Runtime composition.
#[derive(Debug, Clone, Default)]
pub struct RuntimeBootstrapRequest {
    pub workspace_root: Option<PathBuf>,
    pub config_override_expressions: Vec<String>,
    pub database_url: Option<String>,
    pub database_path: Option<PathBuf>,
    /// Optional dedicated scheduler database URL. When unset the scheduler
    /// uses `AGENA_SCHEDULER_DATABASE_URL`/`AGENA_SCHEDULER_DATABASE_PATH`, or
    /// a conventional `~/.agena/scheduler.db` sibling of the chat database
    /// when the chat database is file-backed. In-memory chat databases (tests,
    /// ephemeral deployments) default to the in-memory scheduler store.
    pub scheduler_database_url: Option<String>,
    pub scheduler_database_path: Option<PathBuf>,
    pub initialize_schema: bool,
    pub tracing_reload_handle: Option<crate::TracingFilterReloadHandle>,
}

impl RuntimeBootstrapRequest {
    /// Materialize the Runtime-owned composition input used by the concrete
    /// snapshot adapter.  Process entrypoints therefore pass one stable
    /// request value, while preflight, override parsing, and field wiring stay
    /// inside Runtime.
    pub(crate) fn into_composition_config(
        self,
    ) -> Result<crate::RuntimeCompositionConfig, crate::RuntimeBootstrapError> {
        let bootstrap_preflight = resolve_runtime_bootstrap_preflight(&self)?;
        let load_request = load_config_request_from_bootstrap(&self)
            .map_err(|error| crate::RuntimeBootstrapError::configuration(error.to_string()))?;
        let Self {
            workspace_root,
            database_url,
            database_path,
            scheduler_database_url,
            scheduler_database_path,
            initialize_schema,
            tracing_reload_handle,
            ..
        } = self;
        Ok(crate::RuntimeCompositionConfig {
            load_request,
            workspace_root,
            bootstrap_preflight: Some(bootstrap_preflight),
            database_connection: None,
            database_url,
            database_path,
            scheduler_database_connection: None,
            scheduler_database_url,
            scheduler_database_path,
            initialize_schema,
            tracing_reload_handle,
        })
    }
}

/// Materialize the Runtime-owned loader request without exposing raw
/// override parsing or request construction to a process composition root.
pub(crate) fn load_config_request_from_bootstrap(
    request: &RuntimeBootstrapRequest,
) -> Result<crate::LoadConfigRequest, crate::RuntimeConfigOverrideError> {
    Ok(crate::LoadConfigRequest {
        overrides: crate::parse_config_override_expressions(&request.config_override_expressions)?,
        workspace_root: request.workspace_root.clone(),
    })
}
