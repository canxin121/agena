use std::{path::PathBuf, sync::Arc};

use sea_orm::DatabaseConnection;

use crate::{LoadConfigRequest, RuntimeBootstrapPreflight, TracingFilterReloadHandle};

/// Concrete-process inputs consumed by Runtime composition.
///
/// The input is Runtime-owned so callers never construct an implementation
/// configuration value or receive concrete composition state.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeCompositionConfig {
    pub(crate) load_request: LoadConfigRequest,
    pub(crate) workspace_root: Option<PathBuf>,
    /// Optional Runtime-only tracing/workspace preflight. When present, the
    /// concrete adapter can avoid a duplicate full schema load before the
    /// first snapshot is built.
    pub(crate) bootstrap_preflight: Option<RuntimeBootstrapPreflight>,
    pub(crate) database_connection: Option<Arc<DatabaseConnection>>,
    pub(crate) database_url: Option<String>,
    pub(crate) database_path: Option<PathBuf>,
    pub(crate) initialize_schema: bool,
    pub(crate) tracing_reload_handle: Option<TracingFilterReloadHandle>,
}

impl RuntimeCompositionConfig {
    /// Resolve the process workspace root once and mirror it into the generic
    /// load request when callers did not provide one explicitly.
    pub(crate) fn resolve_workspace_root(&mut self) -> Result<PathBuf, std::io::Error> {
        let workspace_root = match self.workspace_root.clone() {
            Some(path) => path,
            None => match self.bootstrap_preflight.as_ref() {
                Some(preflight) => preflight.workspace_root.clone(),
                None => std::env::current_dir()?,
            },
        };
        self.workspace_root = Some(workspace_root.clone());
        if self.load_request.workspace_root.is_none() {
            self.load_request.workspace_root = Some(workspace_root.clone());
        }
        Ok(workspace_root)
    }
}
