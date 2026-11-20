//! Stable settings-editing contracts for application transports.
//!
//! The concrete runtime remains responsible for validating a document against
//! its configuration schema and writing it to the selected configuration path.
//! These values and the transport-facing service port deliberately stay free of
//! that concrete schema so HTTP, CLI, and other presentation layers do not
//! depend on concrete Runtime configuration internals.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Map as JsonMap, Value as JsonValue};

pub use agena_api::management::settings::*;
pub use agena_api::resource::{ConfigSettingsEditResponse, ConfigSettingsReloadResponse};

/// Select the persisted path for a settings layer.
pub fn config_settings_layer_path<'a>(
    layer: ConfigSettingsLayer,
    global_path: &'a Path,
    workspace_path: &'a Path,
) -> &'a Path {
    match layer {
        ConfigSettingsLayer::Global => global_path,
        ConfigSettingsLayer::Workspace => workspace_path,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Kind of a settings service error.
pub enum RuntimeConfigSettingsErrorKind {
    InvalidInput,
    Internal,
}

#[derive(Debug, Clone)]
/// Error returned by the settings service.
pub struct RuntimeConfigSettingsError {
    kind: RuntimeConfigSettingsErrorKind,
    failure: Box<agena_failure::Failure>,
    diagnostic: String,
}

impl RuntimeConfigSettingsError {
    pub fn invalid_input(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind: RuntimeConfigSettingsErrorKind::InvalidInput,
            diagnostic: message.clone(),
            failure: Box::new(agena_failure::Failure::new(
                agena_failure::FailureCode::new("settings.invalid_input"),
                agena_failure::FailureCategory::InvalidInput,
                agena_failure::FailureResponsibility::Caller,
                agena_failure::RetryDirective::CorrectInput,
                agena_failure::RecoveryDirective::None,
                agena_failure::FailureImpact::RequestRejected,
                agena_failure::UserPresentation::validated("settings-invalid-input", &message),
            )),
        }
    }

    pub fn invalid_input_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::invalid_input(agena_failure::diagnostic::format_error_chain(error))
    }

    pub fn invalid_input_with_diagnostic(
        message: impl AsRef<str>,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        Self {
            kind: RuntimeConfigSettingsErrorKind::InvalidInput,
            diagnostic: diagnostic.to_string(),
            failure: Box::new(agena_failure::Failure::new(
                agena_failure::FailureCode::new("settings.invalid_input"),
                agena_failure::FailureCategory::InvalidInput,
                agena_failure::FailureResponsibility::Caller,
                agena_failure::RetryDirective::CorrectInput,
                agena_failure::RecoveryDirective::None,
                agena_failure::FailureImpact::RequestRejected,
                agena_failure::UserPresentation::validated("settings-invalid-input", message),
            )),
        }
    }

    pub fn internal(diagnostic: impl Into<String>) -> Self {
        let diagnostic = diagnostic.into();
        let presentation = agena_failure::UserPresentation::new(
            "settings-internal",
            "Couldn’t update the settings.",
        );
        Self {
            kind: RuntimeConfigSettingsErrorKind::Internal,
            failure: Box::new(agena_failure::Failure::new(
                agena_failure::FailureCode::new("settings.internal"),
                agena_failure::FailureCategory::Internal,
                agena_failure::FailureResponsibility::System,
                agena_failure::RetryDirective::Unknown,
                agena_failure::RecoveryDirective::Retry,
                agena_failure::FailureImpact::OperationFailed,
                presentation,
            )),
            diagnostic,
        }
    }

    pub fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::internal(agena_failure::diagnostic::format_error_chain(error))
    }

    pub fn internal_error_with_context(
        context: impl AsRef<str>,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::internal(agena_failure::diagnostic::format_error_chain_with_context(
            context, error,
        ))
    }

    pub fn kind(&self) -> RuntimeConfigSettingsErrorKind {
        self.kind
    }

    pub fn failure(&self) -> &agena_failure::Failure {
        &self.failure
    }

    pub fn diagnostic(&self) -> &str {
        self.diagnostic.as_str()
    }
}

impl std::fmt::Display for RuntimeConfigSettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.failure.user.fallback.as_str())
    }
}

impl std::error::Error for RuntimeConfigSettingsError {}

/// Service for reading and editing runtime config settings.
pub trait RuntimeConfigSettingsService: Send + Sync {
    fn read_file_settings(
        &self,
        input: ConfigSettingsGetInput,
    ) -> Result<ConfigSettingsReadResponse, RuntimeConfigSettingsError>;
    /// Read the composed workspace or a workspace root resolved by the application.
    fn read_project_file_settings(
        &self,
        input: ConfigSettingsGetInput,
        workspace_root: Option<&Path>,
    ) -> Result<ConfigSettingsReadResponse, RuntimeConfigSettingsError>;
    fn list_file_settings(
        &self,
        input: ConfigSettingsListInput,
    ) -> Result<ConfigSettingsListResponse, RuntimeConfigSettingsError>;
    fn set_file_setting(
        &self,
        input: ConfigSettingsSetInput,
    ) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError>;
    /// Write the composed workspace or a workspace root resolved by the application.
    fn set_project_file_setting(
        &self,
        input: ConfigSettingsSetInput,
        workspace_root: Option<&Path>,
    ) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError>;
    fn patch_file_settings(
        &self,
        input: ConfigSettingsPatchInput,
    ) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError>;
    fn delete_file_setting(
        &self,
        input: ConfigSettingsDeleteInput,
    ) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError>;
    fn delete_project_file_setting(
        &self,
        input: ConfigSettingsDeleteInput,
        workspace_root: Option<&Path>,
    ) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError>;
    fn validate_file_settings(
        &self,
        input: ConfigSettingsValidateInput,
    ) -> Result<ConfigSettingsValidateResponse, RuntimeConfigSettingsError>;
}

/// Read one value from a configuration document without requiring the concrete
/// configuration schema. A missing document is represented as an empty object,
/// matching the settings editor's create-on-first-write behavior.
pub fn read_runtime_file_setting(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsGetInput,
) -> Result<ConfigSettingsReadResponse, RuntimeConfigSettingsError> {
    read_runtime_file_setting_document(config_path.into(), input).map(|(response, _)| response)
}

fn read_runtime_file_setting_document(
    config_path: PathBuf,
    input: ConfigSettingsGetInput,
) -> Result<(ConfigSettingsReadResponse, JsonValue), RuntimeConfigSettingsError> {
    let (config_found, mut document) = read_runtime_settings_document(&config_path)?;
    let revision = document_revision(&document);
    crate::normalize_settings_document(&mut document)
        .map_err(RuntimeConfigSettingsError::invalid_input)?;
    let value = get_json_path(&document, input.target.path.as_deref())?;
    let override_path = if config_found {
        agena_domain::config_setting_override_path(&document, input.target.path.as_deref())
            .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))?
    } else {
        None
    };
    let requires_group_reset = agena_domain::config_setting_requires_group_reset(
        &document,
        input.target.path.as_deref(),
        override_path.as_deref(),
    )
    .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))?;
    let response = ConfigSettingsReadResponse {
        revision: Some(revision),
        config_path,
        config_found,
        source: ConfigSettingsSource::File,
        path: input.target.path,
        value,
        has_override: Some(override_path.is_some()),
        override_path,
        requires_group_reset: Some(requires_group_reset),
        default_value: None,
        layer_value: None,
        inherited_value: None,
    };
    Ok((response, document))
}

/// Read a raw file layer alongside defaults, its actual parent, and the result
/// through that layer. Later file, environment, and command-line overrides never
/// leak into the values shown while editing an earlier layer.
pub fn read_runtime_layer_setting(
    global_path: &Path,
    workspace_path: &Path,
    layer: ConfigSettingsLayer,
    input: ConfigSettingsGetInput,
) -> Result<ConfigSettingsReadResponse, RuntimeConfigSettingsError> {
    let path = config_settings_layer_path(layer, global_path, workspace_path);
    let (mut response, document) = read_runtime_file_setting_document(path.to_path_buf(), input)?;
    if layer == ConfigSettingsLayer::Workspace && response.config_found {
        response.override_path = agena_domain::config_setting_inheritance_override_path(
            &document,
            response.path.as_deref(),
        )
        .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))?;
        response.has_override = Some(response.override_path.is_some());
        response.requires_group_reset = Some(
            agena_domain::config_setting_requires_group_reset(
                &document,
                response.path.as_deref(),
                response.override_path.as_deref(),
            )
            .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))?,
        );
    }
    response.default_value = default_settings_document()
        .ok()
        .map(|defaults| get_json_path(defaults, response.path.as_deref()))
        .transpose()?;

    // Invalid files stay available for raw repair. An unavailable projection is
    // omitted, never replaced with final effective values from later layers.
    let selected = crate::RawConfigFile::from_document(path, document, response.config_found);
    let (global, workspace) = match layer {
        ConfigSettingsLayer::Global => (selected, None),
        ConfigSettingsLayer::Workspace => (crate::RawConfigFile::read(global_path), Some(selected)),
    };
    if let Ok(global) = global {
        if layer == ConfigSettingsLayer::Workspace {
            response.inherited_value = resolved_settings_layer_document(
                global.config.clone(),
                None,
                global_path,
                workspace_path,
            )
            .ok()
            .map(|value| get_json_path(&value, response.path.as_deref()))
            .transpose()?;
        }
        let resolved = match workspace {
            None => {
                resolved_settings_layer_document(global.config, None, global_path, workspace_path)
            }
            Some(workspace) => workspace
                .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))
                .and_then(|workspace| {
                    resolved_settings_layer_document(
                        global.config,
                        Some(workspace),
                        global_path,
                        workspace_path,
                    )
                }),
        };
        response.layer_value = resolved
            .ok()
            .map(|value| get_json_path(&value, response.path.as_deref()))
            .transpose()?;
    }
    Ok(response)
}

fn default_settings_document() -> Result<&'static JsonValue, RuntimeConfigSettingsError> {
    // Program defaults do not depend on a file or runtime environment. Cache the
    // root once; both leaf and batch reads slice this same schema projection.
    static DEFAULTS: std::sync::LazyLock<Result<JsonValue, RuntimeConfigSettingsError>> =
        std::sync::LazyLock::new(|| {
            let config = crate::RawConfig::default()
                .resolve()
                .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))?;
            crate::resolved_config_json_value(&config)
                .map_err(|error| RuntimeConfigSettingsError::internal_error(&error))
        });
    DEFAULTS.as_ref().map_err(Clone::clone)
}

fn resolved_settings_layer_document(
    mut global: crate::RawConfig,
    workspace: Option<crate::RawConfigFile>,
    global_path: &Path,
    workspace_path: &Path,
) -> Result<JsonValue, RuntimeConfigSettingsError> {
    let global_prompts = global.prompts.clone().unwrap_or_default();
    let mut workspace_prompts = Default::default();
    let mut workspace_dir = Default::default();
    if let Some(workspace) = workspace {
        if workspace.config.access_control.is_some() {
            return Err(RuntimeConfigSettingsError::invalid_input(
                "access_control is instance-owned; edit it in global settings, not a workspace file",
            ));
        }
        workspace_prompts = workspace.config.prompts.clone().unwrap_or_default();
        if !workspace_path.as_os_str().is_empty() {
            workspace_dir = workspace_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("prompts");
        }
        global.merge_project_from_with_keys(workspace.config, workspace.merge_keys);
    }
    // Use Runtime's schema rules: scalar fields merge, while keyed entries,
    // model selections, and per-tool command rule groups retain their boundaries.
    let mut resolved = global
        .resolve()
        .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))?;
    resolved.prompts = crate::PromptLayersConfig {
        global: global_prompts,
        workspace: workspace_prompts,
        global_dir: global_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("prompts"),
        workspace_dir,
    };
    crate::resolved_config_json_value(&resolved)
        .map_err(|error| RuntimeConfigSettingsError::internal_error(&error))
}

/// List values from a configuration document using Runtime's stable path
/// grammar and transport projection.
pub fn list_runtime_file_settings(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsListInput,
) -> Result<ConfigSettingsListResponse, RuntimeConfigSettingsError> {
    let config_path = config_path.into();
    let (config_found, mut document) = read_runtime_settings_document(&config_path)?;
    let revision = document_revision(&document);
    crate::normalize_settings_document(&mut document)
        .map_err(RuntimeConfigSettingsError::invalid_input)?;
    let items = list_json_path(&document, input.target.path.as_deref(), input.recursive)?;
    Ok(ConfigSettingsListResponse {
        revision: Some(revision),
        config_path,
        config_found,
        source: ConfigSettingsSource::File,
        path: input.target.path,
        items,
    })
}

/// Schema validation is supplied by the concrete composition layer. Runtime
/// owns JSON document operations and persistence mechanics while the composed
/// validator owns schema-specific policy.
pub type RuntimeSettingsDocumentValidator =
    dyn Fn(&Path, &str) -> Result<(), RuntimeConfigSettingsError> + Send + Sync;

pub fn set_runtime_file_setting(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsSetInput,
    validator: Option<&RuntimeSettingsDocumentValidator>,
) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError> {
    let config_path = config_path.into();
    crate::with_config_file_write_lock(|| {
        let segments = required_runtime_settings_path_segments(&input.path)?;
        let (config_found, mut document) = read_runtime_settings_document(&config_path)?;
        verify_document_revision(&document, input.options.expected_revision.as_deref())?;
        let before = document.clone();
        crate::normalize_settings_document(&mut document)
            .map_err(RuntimeConfigSettingsError::invalid_input)?;
        let previous = get_json_path(&document, Some(input.path.as_str()))?;
        let created = previous.is_null();
        set_runtime_json_path(&mut document, &segments, input.value)?;
        finish_runtime_settings_edit(
            config_path,
            config_found,
            document,
            before,
            Some(input.path),
            "set",
            input.options,
            created,
            false,
            validator,
        )
    })
}

pub fn delete_runtime_file_setting(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsDeleteInput,
    validator: Option<&RuntimeSettingsDocumentValidator>,
) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError> {
    let config_path = config_path.into();
    crate::with_config_file_write_lock(|| {
        let segments = required_runtime_settings_path_segments(&input.path)?;
        let (config_found, mut document) = read_runtime_settings_document(&config_path)?;
        verify_document_revision(&document, input.options.expected_revision.as_deref())?;
        let before = document.clone();
        crate::normalize_settings_document(&mut document)
            .map_err(RuntimeConfigSettingsError::invalid_input)?;
        let deleted = remove_runtime_json_path(&mut document, &segments)?;
        finish_runtime_settings_edit(
            config_path,
            config_found,
            document,
            before,
            Some(input.path),
            "delete",
            input.options,
            false,
            deleted,
            validator,
        )
    })
}

pub fn patch_runtime_file_settings(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsPatchInput,
    validator: Option<&RuntimeSettingsDocumentValidator>,
) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError> {
    let config_path = config_path.into();
    crate::with_config_file_write_lock(|| {
        let changes = input.changes.as_object().ok_or_else(|| {
            RuntimeConfigSettingsError::invalid_input(
                "settings_patch changes must be a JSON object",
            )
        })?;
        let (config_found, mut document) = read_runtime_settings_document(&config_path)?;
        verify_document_revision(&document, input.options.expected_revision.as_deref())?;
        let before = document.clone();
        crate::normalize_settings_document(&mut document)
            .map_err(RuntimeConfigSettingsError::invalid_input)?;
        let created = input
            .target
            .path
            .as_deref()
            .map(|path| get_json_path(&document, Some(path)).map(|value| value.is_null()))
            .transpose()?
            .unwrap_or(false);
        let target = ensure_runtime_object_path(&mut document, input.target.path.as_deref())?;
        merge_runtime_json_object(target, changes)?;
        finish_runtime_settings_edit(
            config_path,
            config_found,
            document,
            before,
            input.target.path,
            "patch",
            input.options,
            created,
            false,
            validator,
        )
    })
}

pub fn validate_runtime_file_settings(
    config_path: impl Into<PathBuf>,
    validator: &RuntimeSettingsDocumentValidator,
) -> Result<ConfigSettingsValidateResponse, RuntimeConfigSettingsError> {
    let config_path = config_path.into();
    let (config_found, document) = read_runtime_settings_document(&config_path)?;
    let text = serde_json::to_string_pretty(&document)
        .map_err(|error| RuntimeConfigSettingsError::internal_error(&error))?;
    validator(&config_path, &text)?;
    Ok(ConfigSettingsValidateResponse {
        config_path,
        config_found,
        valid: true,
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_runtime_settings_edit(
    config_path: PathBuf,
    config_found: bool,
    mut document: JsonValue,
    before: JsonValue,
    path: Option<String>,
    operation: &'static str,
    options: ConfigSettingsEditOptions,
    created: bool,
    deleted: bool,
    validator: Option<&RuntimeSettingsDocumentValidator>,
) -> Result<ConfigSettingsEditResponse, RuntimeConfigSettingsError> {
    // Security-sensitive configuration is never persisted without validation.
    // This also canonicalizes safe legacy path rules before writing them.
    normalize_settings_document(&mut document)?;
    let text = serde_json::to_string_pretty(&document)
        .map_err(|error| RuntimeConfigSettingsError::internal_error(&error))?;
    let validated = if let Some(validator) = validator {
        // Public settings services always supply the composed-layer validator.
        // validate=false may not bypass it, including through raw JSON edits.
        validator(&config_path, &text)?;
        true
    } else if options.validate {
        return Err(RuntimeConfigSettingsError::internal(
            "runtime settings edit requested validation without a schema validator",
        ));
    } else {
        false
    };
    let before_revision = document_revision(&before);
    let mut previous_document = before;
    crate::normalize_settings_document(&mut previous_document)
        .map_err(RuntimeConfigSettingsError::invalid_input)?;
    let previous = get_json_path(&previous_document, path.as_deref())?;
    let current = get_json_path(&document, path.as_deref())?;
    let after_revision = document_revision(&document);
    // Object equality ignores insertion order, but network and command rule
    // order changes the authorization decision and must be persisted.
    let changed = before_revision != after_revision;
    if changed && !options.dry_run {
        write_runtime_settings_document(&config_path, &text)?;
    }
    Ok(ConfigSettingsEditResponse {
        before_revision,
        after_revision,
        config_path,
        config_found,
        operation: operation.to_string(),
        path,
        dry_run: options.dry_run,
        changed,
        created,
        deleted,
        validated,
        reload_requested: options.reload,
        // A previous save may have reached disk while its runtime reload failed.
        // Retrying the same value must still activate it in the running service.
        reload_required: !options.dry_run && options.reload,
        reload: None,
        previous,
        current,
    })
}

/// Normalize security settings before any settings document reaches disk.
/// The complete layer validator subsequently checks inherited rules as well.
pub(crate) fn normalize_settings_document(
    document: &mut JsonValue,
) -> Result<(), RuntimeConfigSettingsError> {
    crate::normalize_settings_document(document)
        .map_err(RuntimeConfigSettingsError::invalid_input)?;
    if let Some(value) = document.get("permission").filter(|value| !value.is_null()) {
        let mut permission: agena_domain::PermissionConfig =
            serde_path_to_error::deserialize(value.clone()).map_err(|error| {
                RuntimeConfigSettingsError::invalid_input(format!(
                    "permission.{}: {}",
                    error.path(),
                    error.inner()
                ))
            })?;
        normalize_permission_config(&mut permission, "permission")?;
        document["permission"] = serde_json::to_value(permission)
            .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))?;
    }
    if let Some(value) = document
        .get("access_control")
        .filter(|value| !value.is_null())
    {
        let mut access_control: agena_domain::AccessControlConfig =
            serde_path_to_error::deserialize(value.clone()).map_err(|error| {
                RuntimeConfigSettingsError::invalid_input(format!(
                    "access_control.{}: {}",
                    error.path(),
                    error.inner()
                ))
            })?;
        for (id, profile) in &mut access_control.profiles {
            if profile.complete {
                profile.permission = profile.permission.standalone();
            }
            normalize_permission_config(
                &mut profile.permission,
                &format!("access_control.profiles.{id}.permission"),
            )?;
        }
        for (index, user) in access_control.users.iter_mut().enumerate() {
            normalize_permission_config(
                &mut user.overrides,
                &format!("access_control.users.{index}.overrides"),
            )?;
        }
        access_control
            .validate()
            .map_err(RuntimeConfigSettingsError::invalid_input)?;
        document["access_control"] = serde_json::to_value(access_control)
            .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))?;
    }
    Ok(())
}

fn normalize_permission_config(
    permission: &mut agena_domain::PermissionConfig,
    field: &str,
) -> Result<(), RuntimeConfigSettingsError> {
    if let Some(paths) = &mut permission.path {
        paths.normalize_rules().map_err(|error| {
            RuntimeConfigSettingsError::invalid_input(format!("{field}: {error}"))
        })?;
    }
    agena_runtime_contracts::authorization::validate_permission_config(permission)
        .map_err(|error| RuntimeConfigSettingsError::invalid_input(format!("{field}: {error}")))?;
    Ok(())
}

fn write_runtime_settings_document(
    config_path: &Path,
    text: &str,
) -> Result<(), RuntimeConfigSettingsError> {
    crate::write_config_file_atomically(config_path, text.as_bytes()).map_err(|error| {
        RuntimeConfigSettingsError::internal(format!(
            "failed to write configuration file {}: {error}",
            config_path.display()
        ))
    })
}

fn required_runtime_settings_path_segments(
    path: &str,
) -> Result<Vec<String>, RuntimeConfigSettingsError> {
    let segments = parse_runtime_settings_path(path)?;
    if segments.is_empty() {
        return Err(RuntimeConfigSettingsError::invalid_input(
            "settings path must not be empty",
        ));
    }
    Ok(segments)
}

fn ensure_runtime_object_path<'a>(
    root: &'a mut JsonValue,
    path: Option<&str>,
) -> Result<&'a mut JsonMap<String, JsonValue>, RuntimeConfigSettingsError> {
    if !root.is_object() {
        *root = JsonValue::Object(JsonMap::new());
    }
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return root.as_object_mut().ok_or_else(|| {
            RuntimeConfigSettingsError::invalid_input("settings root must be an object")
        });
    };
    let segments = parse_runtime_settings_path(path)?;
    let mut cursor = root;
    for (index, segment) in segments.iter().enumerate() {
        let object = cursor.as_object_mut().ok_or_else(|| {
            RuntimeConfigSettingsError::invalid_input(format!(
                "settings path `{path}` crosses non-object segment `{segment}`"
            ))
        })?;
        let child = object
            .entry(segment.clone())
            .or_insert_with(|| JsonValue::Object(JsonMap::new()));
        if child.is_null() {
            *child = JsonValue::Object(JsonMap::new());
        }
        if !child.is_object() {
            return Err(RuntimeConfigSettingsError::invalid_input(format!(
                "settings path `{path}` crosses non-object segment"
            )));
        }
        crate::network_config_migration::initialize_network_field_object(
            child,
            &segments[..=index],
        );
        cursor = child;
    }
    cursor.as_object_mut().ok_or_else(|| {
        RuntimeConfigSettingsError::invalid_input("settings target must be an object")
    })
}

fn set_runtime_json_path(
    root: &mut JsonValue,
    segments: &[String],
    value: JsonValue,
) -> Result<(), RuntimeConfigSettingsError> {
    let key = segments
        .last()
        .ok_or_else(|| {
            RuntimeConfigSettingsError::invalid_input("settings path must not be empty")
        })?
        .clone();
    let parent_path =
        (segments.len() > 1).then(|| format_settings_path(&segments[..segments.len() - 1]));
    let object = ensure_runtime_object_path(root, parent_path.as_deref())?;
    object.insert(key, value);
    Ok(())
}

fn remove_runtime_json_path(
    root: &mut JsonValue,
    segments: &[String],
) -> Result<bool, RuntimeConfigSettingsError> {
    let key = segments.last().ok_or_else(|| {
        RuntimeConfigSettingsError::invalid_input("settings path must not be empty")
    })?;
    if segments.len() == 1 {
        return Ok(root
            .as_object_mut()
            .is_some_and(|object| object.remove(key.as_str()).is_some()));
    }
    let mut cursor = root;
    for segment in &segments[..segments.len() - 1] {
        let Some(object) = cursor.as_object_mut() else {
            return Ok(false);
        };
        let Some(child) = object.get_mut(segment.as_str()) else {
            return Ok(false);
        };
        cursor = child;
    }
    Ok(cursor
        .as_object_mut()
        .is_some_and(|object| object.remove(key.as_str()).is_some()))
}

fn merge_runtime_json_object(
    target: &mut JsonMap<String, JsonValue>,
    changes: &JsonMap<String, JsonValue>,
) -> Result<(), RuntimeConfigSettingsError> {
    for (key, value) in changes {
        if value.is_null() {
            target.remove(key.as_str());
            continue;
        }
        if let Some(object_patch) = value.as_object() {
            let entry = target
                .entry(key.clone())
                .or_insert_with(|| JsonValue::Object(JsonMap::new()));
            if entry.is_null() {
                *entry = JsonValue::Object(JsonMap::new());
            }
            let child = entry.as_object_mut().ok_or_else(|| {
                RuntimeConfigSettingsError::invalid_input(format!(
                    "settings_patch cannot merge object into non-object key `{key}`"
                ))
            })?;
            merge_runtime_json_object(child, object_patch)?;
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}

fn read_runtime_settings_document(
    config_path: &Path,
) -> Result<(bool, JsonValue), RuntimeConfigSettingsError> {
    match fs::read_to_string(config_path) {
        Ok(text) => {
            let value = serde_json::from_str::<JsonValue>(&text).map_err(|error| {
                RuntimeConfigSettingsError::invalid_input_with_diagnostic(
                    format!(
                        "The configuration JSON is invalid at line {}, column {}.",
                        error.line(),
                        error.column()
                    ),
                    format_args!(
                        "failed to parse configuration file {}: {error}",
                        config_path.display()
                    ),
                )
            })?;
            let value = match value {
                JsonValue::Null => JsonValue::Object(Default::default()),
                other => other,
            };
            Ok((true, value))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((false, JsonValue::Object(Default::default())))
        }
        Err(error) => Err(RuntimeConfigSettingsError::internal(format!(
            "failed to read configuration file {}: {error}",
            config_path.display()
        ))),
    }
}

pub fn get_json_path(
    value: &JsonValue,
    path: Option<&str>,
) -> Result<JsonValue, RuntimeConfigSettingsError> {
    agena_domain::get_json_path(value, path)
        .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))
}

pub fn list_json_path(
    value: &JsonValue,
    path: Option<&str>,
    recursive: bool,
) -> Result<Vec<ConfigSettingsListItem>, RuntimeConfigSettingsError> {
    let base = path.map(str::trim).filter(|path| !path.is_empty());
    let target = get_json_path(value, base)?;
    let base_segments = base
        .map(parse_runtime_settings_path)
        .transpose()?
        .unwrap_or_default();
    let mut entries = Vec::new();
    collect_list_entries(&mut entries, &base_segments, &target, recursive);
    Ok(entries)
}

/// Parse a dotted settings path with quoted segments without exposing the
/// concrete Runtime configuration schema.
pub fn parse_runtime_settings_path(path: &str) -> Result<Vec<String>, RuntimeConfigSettingsError> {
    agena_domain::parse_json_path(path)
        .map_err(|error| RuntimeConfigSettingsError::invalid_input_error(&error))
}

fn collect_list_entries(
    entries: &mut Vec<ConfigSettingsListItem>,
    base: &[String],
    value: &JsonValue,
    recursive: bool,
) {
    match value {
        JsonValue::Object(object) => {
            for (key, child) in object {
                let mut child_path = base.to_vec();
                child_path.push(key.clone());
                entries.push(ConfigSettingsListItem {
                    path: format_settings_path(&child_path),
                    kind: json_kind(child).to_owned(),
                    value: scalar_json_value(child),
                });
                if recursive {
                    collect_list_entries(entries, &child_path, child, recursive);
                }
            }
        }
        JsonValue::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let mut child_path = base.to_vec();
                child_path.push(index.to_string());
                entries.push(ConfigSettingsListItem {
                    path: format_settings_path(&child_path),
                    kind: json_kind(child).to_owned(),
                    value: scalar_json_value(child),
                });
                if recursive {
                    collect_list_entries(entries, &child_path, child, recursive);
                }
            }
        }
        other => entries.push(ConfigSettingsListItem {
            path: format_settings_path(base),
            kind: json_kind(other).to_owned(),
            value: Some(other.clone()),
        }),
    }
}

pub fn format_settings_path(segments: &[String]) -> String {
    agena_domain::format_json_path(segments)
}

fn scalar_json_value(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Object(_) | JsonValue::Array(_) => None,
        other => Some(other.clone()),
    }
}

fn json_kind(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "bool",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

/// Stable revision for the complete parsed persisted document. Never a hash of redacted output.
pub fn document_revision(value: &JsonValue) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(
        serde_json::to_vec(value).expect("JSON value is serializable"),
    ))
}
fn verify_document_revision(
    value: &JsonValue,
    expected: Option<&str>,
) -> Result<(), RuntimeConfigSettingsError> {
    if expected.is_some_and(|expected| expected != document_revision(value)) {
        return Err(RuntimeConfigSettingsError::invalid_input(
            "settings revision changed; read source=file and retry with the returned revision",
        ));
    }
    Ok(())
}
