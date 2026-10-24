use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Map as JsonMap, Value as JsonValue};

pub use crate::config_settings_layer_path as settings_layer_path;
use crate::{ConfigEnvironment, ConfigError, ProcessEnvironment};
pub use crate::{
    ConfigSettingsDeleteInput, ConfigSettingsEditOptions, ConfigSettingsEditResponse,
    ConfigSettingsGetInput, ConfigSettingsLayer, ConfigSettingsListInput, ConfigSettingsListItem,
    ConfigSettingsListResponse, ConfigSettingsPatchInput, ConfigSettingsPathInput,
    ConfigSettingsReadResponse, ConfigSettingsSetInput, ConfigSettingsSource,
    ConfigSettingsValidateResponse,
};

#[derive(Debug, Clone)]
struct LayeredValidation {
    global_path: PathBuf,
    workspace_path: PathBuf,
    edited_layer: ConfigSettingsLayer,
}

pub fn parse_settings_path(path: &str) -> Result<Vec<String>, ConfigError> {
    agena_domain::parse_json_path(path).map_err(|error| ConfigError::validation_error(&error))
}

pub fn get_json_path(value: &JsonValue, path: Option<&str>) -> Result<JsonValue, ConfigError> {
    agena_domain::get_json_path(value, path).map_err(|error| ConfigError::validation_error(&error))
}

pub fn list_json_path(
    value: &JsonValue,
    path: Option<&str>,
    recursive: bool,
) -> Result<Vec<ConfigSettingsListItem>, ConfigError> {
    crate::runtime_config_settings_service::list_json_path(value, path, recursive)
        .map_err(crate::config_error::settings_error_to_config_error)
}

pub fn format_settings_path(segments: &[String]) -> String {
    agena_domain::format_json_path(segments)
}

pub fn read_file_setting(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsGetInput,
) -> Result<ConfigSettingsReadResponse, ConfigError> {
    let config_path = config_path.into();
    let (config_found, file_value) = read_or_create_doc(&config_path)?;
    let value = get_json_path(&file_value, input.target.path.as_deref())?;
    Ok(ConfigSettingsReadResponse {
        revision: Some(crate::runtime_config_settings_service::document_revision(
            &file_value,
        )),
        config_path,
        config_found,
        source: ConfigSettingsSource::File,
        path: input.target.path,
        value,
    })
}

pub fn list_file_settings(
    config_path: impl Into<PathBuf>,
    input: ConfigSettingsListInput,
) -> Result<ConfigSettingsListResponse, ConfigError> {
    let config_path = config_path.into();
    let (config_found, file_value) = read_or_create_doc(&config_path)?;
    let items = list_json_path(&file_value, input.target.path.as_deref(), input.recursive)?;
    Ok(ConfigSettingsListResponse {
        revision: Some(crate::runtime_config_settings_service::document_revision(
            &file_value,
        )),
        config_path,
        config_found,
        source: ConfigSettingsSource::File,
        path: input.target.path,
        items,
    })
}

pub fn set_layered_file_setting(
    global_path: impl Into<PathBuf>,
    workspace_path: impl Into<PathBuf>,
    layer: ConfigSettingsLayer,
    input: ConfigSettingsSetInput,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    set_layered_file_setting_with_env(
        global_path,
        workspace_path,
        layer,
        input,
        &ProcessEnvironment,
    )
}

pub fn set_layered_file_setting_with_env(
    global_path: impl Into<PathBuf>,
    workspace_path: impl Into<PathBuf>,
    layer: ConfigSettingsLayer,
    input: ConfigSettingsSetInput,
    env: &dyn ConfigEnvironment,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    let validation = LayeredValidation {
        global_path: global_path.into(),
        workspace_path: workspace_path.into(),
        edited_layer: layer,
    };
    let config_path =
        settings_layer_path(layer, &validation.global_path, &validation.workspace_path)
            .to_path_buf();
    set_file_setting_impl(config_path, input, env, Some(&validation))
}

fn set_file_setting_impl(
    config_path: PathBuf,
    input: ConfigSettingsSetInput,
    env: &dyn ConfigEnvironment,
    layered: Option<&LayeredValidation>,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    crate::with_config_file_write_lock(|| {
        let segments = required_path_segments(&input.path)?;
        let (config_found, mut doc) = read_or_create_doc(&config_path)?;
        if input
            .options
            .expected_revision
            .as_deref()
            .is_some_and(|expected| {
                expected != crate::runtime_config_settings_service::document_revision(&doc)
            })
        {
            return Err(ConfigError::Validation(
                "settings revision changed; read source=file before retrying".into(),
            ));
        }
        let before = doc.clone();
        let previous = get_json_path(&before, Some(input.path.as_str()))?;
        let created = previous.is_null();
        set_json_path(&mut doc, &segments, input.value)?;
        finish_edit(
            config_path,
            config_found,
            doc,
            before,
            Some(input.path),
            "set",
            input.options.dry_run,
            input.options.validate,
            input.options.reload,
            created,
            false,
            env,
            layered,
        )
    })
}

pub fn delete_layered_file_setting(
    global_path: impl Into<PathBuf>,
    workspace_path: impl Into<PathBuf>,
    layer: ConfigSettingsLayer,
    input: ConfigSettingsDeleteInput,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    delete_layered_file_setting_with_env(
        global_path,
        workspace_path,
        layer,
        input,
        &ProcessEnvironment,
    )
}

pub fn delete_layered_file_setting_with_env(
    global_path: impl Into<PathBuf>,
    workspace_path: impl Into<PathBuf>,
    layer: ConfigSettingsLayer,
    input: ConfigSettingsDeleteInput,
    env: &dyn ConfigEnvironment,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    let validation = LayeredValidation {
        global_path: global_path.into(),
        workspace_path: workspace_path.into(),
        edited_layer: layer,
    };
    let config_path =
        settings_layer_path(layer, &validation.global_path, &validation.workspace_path)
            .to_path_buf();
    delete_file_setting_impl(config_path, input, env, Some(&validation))
}

fn delete_file_setting_impl(
    config_path: PathBuf,
    input: ConfigSettingsDeleteInput,
    env: &dyn ConfigEnvironment,
    layered: Option<&LayeredValidation>,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    crate::with_config_file_write_lock(|| {
        let segments = required_path_segments(&input.path)?;
        let (config_found, mut doc) = read_or_create_doc(&config_path)?;
        if input
            .options
            .expected_revision
            .as_deref()
            .is_some_and(|expected| {
                expected != crate::runtime_config_settings_service::document_revision(&doc)
            })
        {
            return Err(ConfigError::Validation(
                "settings revision changed; read source=file before retrying".into(),
            ));
        }
        let before = doc.clone();
        let deleted = remove_json_path(&mut doc, &segments)?;
        finish_edit(
            config_path,
            config_found,
            doc,
            before,
            Some(input.path),
            "delete",
            input.options.dry_run,
            input.options.validate,
            input.options.reload,
            false,
            deleted,
            env,
            layered,
        )
    })
}

pub fn patch_layered_file_settings(
    global_path: impl Into<PathBuf>,
    workspace_path: impl Into<PathBuf>,
    layer: ConfigSettingsLayer,
    input: ConfigSettingsPatchInput,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    patch_layered_file_settings_with_env(
        global_path,
        workspace_path,
        layer,
        input,
        &ProcessEnvironment,
    )
}

pub fn patch_layered_file_settings_with_env(
    global_path: impl Into<PathBuf>,
    workspace_path: impl Into<PathBuf>,
    layer: ConfigSettingsLayer,
    input: ConfigSettingsPatchInput,
    env: &dyn ConfigEnvironment,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    let validation = LayeredValidation {
        global_path: global_path.into(),
        workspace_path: workspace_path.into(),
        edited_layer: layer,
    };
    let config_path =
        settings_layer_path(layer, &validation.global_path, &validation.workspace_path)
            .to_path_buf();
    patch_file_settings_impl(config_path, input, env, Some(&validation))
}

fn patch_file_settings_impl(
    config_path: PathBuf,
    input: ConfigSettingsPatchInput,
    env: &dyn ConfigEnvironment,
    layered: Option<&LayeredValidation>,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    crate::with_config_file_write_lock(|| {
        let changes = input.changes.as_object().ok_or_else(|| {
            ConfigError::Validation("settings_patch changes must be a JSON object".to_owned())
        })?;
        let (config_found, mut doc) = read_or_create_doc(&config_path)?;
        if input
            .options
            .expected_revision
            .as_deref()
            .is_some_and(|expected| {
                expected != crate::runtime_config_settings_service::document_revision(&doc)
            })
        {
            return Err(ConfigError::Validation(
                "settings revision changed; read source=file before retrying".into(),
            ));
        }
        let before = doc.clone();
        let created = match input.target.path.as_deref() {
            Some(path) => get_json_path(&before, Some(path))?.is_null(),
            None => false,
        };
        let target = ensure_object_path(&mut doc, input.target.path.as_deref())?;
        merge_json_object(target, changes)?;
        finish_edit(
            config_path,
            config_found,
            doc,
            before,
            input.target.path,
            "patch",
            input.options.dry_run,
            input.options.validate,
            input.options.reload,
            created,
            false,
            env,
            layered,
        )
    })
}

pub fn validate_layered_file_settings(
    global_path: impl Into<PathBuf>,
    workspace_path: impl Into<PathBuf>,
    layer: ConfigSettingsLayer,
) -> Result<ConfigSettingsValidateResponse, ConfigError> {
    validate_layered_file_settings_with_env(global_path, workspace_path, layer, &ProcessEnvironment)
}

pub fn validate_layered_file_settings_with_env(
    global_path: impl Into<PathBuf>,
    workspace_path: impl Into<PathBuf>,
    layer: ConfigSettingsLayer,
    env: &dyn ConfigEnvironment,
) -> Result<ConfigSettingsValidateResponse, ConfigError> {
    let global_path = global_path.into();
    let workspace_path = workspace_path.into();
    let config_path = settings_layer_path(layer, &global_path, &workspace_path).to_path_buf();
    let (config_found, doc) = read_or_create_doc(&config_path)?;
    let text = serde_json::to_string_pretty(&doc)?;
    super::raw::validate_layered_config_text(
        &global_path,
        &workspace_path,
        layer,
        text.as_str(),
        env,
    )?;
    Ok(ConfigSettingsValidateResponse {
        config_path,
        config_found,
        valid: true,
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_edit(
    config_path: PathBuf,
    config_found: bool,
    doc: JsonValue,
    before: JsonValue,
    path: Option<String>,
    operation: &'static str,
    dry_run: bool,
    validate: bool,
    reload_requested: bool,
    created: bool,
    deleted: bool,
    env: &dyn ConfigEnvironment,
    layered: Option<&LayeredValidation>,
) -> Result<ConfigSettingsEditResponse, ConfigError> {
    let text = serde_json::to_string_pretty(&doc)?;
    if validate {
        match layered {
            Some(layered) => super::raw::validate_layered_config_text(
                &layered.global_path,
                &layered.workspace_path,
                layered.edited_layer,
                text.as_str(),
                env,
            )?,
            None => super::raw::validate_config_text(&config_path, text.as_str())?,
        }
    }
    let previous = get_json_path(&before, path.as_deref())?;
    let current = get_json_path(&doc, path.as_deref())?;
    let changed = before != doc;
    if changed && !dry_run {
        write_doc(&config_path, text.as_str())?;
    }

    Ok(ConfigSettingsEditResponse {
        before_revision: crate::runtime_config_settings_service::document_revision(&before),
        after_revision: crate::runtime_config_settings_service::document_revision(&doc),
        config_path,
        config_found,
        operation: operation.to_string(),
        path,
        dry_run,
        changed,
        created,
        deleted,
        validated: validate,
        reload_requested,
        reload_required: changed && !dry_run && reload_requested,
        reload: None,
        previous,
        current,
    })
}

fn read_or_create_doc(path: &Path) -> Result<(bool, JsonValue), ConfigError> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let value = serde_json::from_str::<JsonValue>(text.as_str()).map_err(|source| {
                ConfigError::ParseFile {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
            Ok((true, normalize_root_object(value)))
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok((false, JsonValue::Object(JsonMap::new())))
        }
        Err(source) => Err(ConfigError::ReadFile {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn normalize_root_object(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(_) => value,
        JsonValue::Null => JsonValue::Object(JsonMap::new()),
        other => other,
    }
}

fn write_doc(path: &Path, text: &str) -> Result<(), ConfigError> {
    crate::write_config_file_atomically(path, text.as_bytes()).map_err(|source| {
        ConfigError::WriteFile {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn required_path_segments(path: &str) -> Result<Vec<String>, ConfigError> {
    let segments = parse_settings_path(path)?;
    if segments.is_empty() {
        return Err(ConfigError::Validation(
            "settings path must not be empty".to_owned(),
        ));
    }
    Ok(segments)
}

fn ensure_object_path<'a>(
    root: &'a mut JsonValue,
    path: Option<&str>,
) -> Result<&'a mut JsonMap<String, JsonValue>, ConfigError> {
    if !root.is_object() {
        *root = JsonValue::Object(JsonMap::new());
    }
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return root
            .as_object_mut()
            .ok_or_else(|| ConfigError::Validation("settings root must be an object".to_owned()));
    };
    let segments = parse_settings_path(path)?;
    let mut cursor = root;
    for segment in segments {
        let object = cursor.as_object_mut().ok_or_else(|| {
            ConfigError::Validation(format!(
                "settings path `{path}` crosses non-object segment `{segment}`"
            ))
        })?;
        let child = object
            .entry(segment)
            .or_insert_with(|| JsonValue::Object(JsonMap::new()));
        if child.is_null() {
            *child = JsonValue::Object(JsonMap::new());
        }
        if !child.is_object() {
            return Err(ConfigError::Validation(format!(
                "settings path `{path}` crosses non-object segment"
            )));
        }
        cursor = child;
    }
    cursor
        .as_object_mut()
        .ok_or_else(|| ConfigError::Validation("settings target must be an object".to_owned()))
}

fn set_json_path(
    root: &mut JsonValue,
    segments: &[String],
    value: JsonValue,
) -> Result<(), ConfigError> {
    let key = segments
        .last()
        .ok_or_else(|| ConfigError::Validation("settings path must not be empty".to_owned()))?
        .clone();
    let parent_path = if segments.len() > 1 {
        Some(format_settings_path(&segments[..segments.len() - 1]))
    } else {
        None
    };
    let object = ensure_object_path(root, parent_path.as_deref())?;
    object.insert(key, value);
    Ok(())
}

fn remove_json_path(root: &mut JsonValue, segments: &[String]) -> Result<bool, ConfigError> {
    let key = segments
        .last()
        .ok_or_else(|| ConfigError::Validation("settings path must not be empty".to_owned()))?;
    if segments.len() == 1 {
        let Some(object) = root.as_object_mut() else {
            return Ok(false);
        };
        return Ok(object.remove(key.as_str()).is_some());
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
    let Some(object) = cursor.as_object_mut() else {
        return Ok(false);
    };
    Ok(object.remove(key.as_str()).is_some())
}

fn merge_json_object(
    target: &mut JsonMap<String, JsonValue>,
    changes: &JsonMap<String, JsonValue>,
) -> Result<(), ConfigError> {
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
                ConfigError::Validation(format!(
                    "settings_patch cannot merge object into non-object key `{key}`"
                ))
            })?;
            merge_json_object(child, object_patch)?;
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}
