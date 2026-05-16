use std::path::{Path, PathBuf};

use clap::{CommandFactory, FromArgMatches, parser::ValueSource};
use serde::Deserialize;

const DEFAULT_RUNTIME_CONFIG_FILE: &str = "agena-studio.toml";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RuntimeConfig {
    backend: BackendRuntimeConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BackendRuntimeConfig {
    host: Option<String>,
    port: Option<u16>,
    ui_password: Option<String>,
    ui_dir: Option<String>,
    cors_origins: Option<Vec<String>>,
    cors_allow_all: Option<bool>,
    ui_cookie_samesite: Option<String>,
}

pub(crate) fn parse_args_with_runtime_config() -> Result<crate::Args, String> {
    let matches = crate::Args::command().get_matches();
    let mut args = crate::Args::from_arg_matches(&matches).map_err(|e| e.to_string())?;
    let explicit_config_path = matches.value_source("config").is_some();

    let config_path = args
        .config
        .as_deref()
        .map(PathBuf::from)
        .or_else(default_runtime_config_path);

    let Some(config_path) = config_path else {
        return Ok(args);
    };

    if explicit_config_path && !config_path.exists() {
        return Err(format!(
            "runtime config file not found: {}",
            config_path.display()
        ));
    }
    if !config_path.exists() {
        return Ok(args);
    }

    let runtime_config = read_runtime_config(&config_path)?;
    apply_runtime_overrides(&mut args, &matches, &runtime_config)?;

    Ok(args)
}

fn read_runtime_config(path: &Path) -> Result<RuntimeConfig, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read runtime config {}: {err}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(RuntimeConfig::default());
    }

    toml::from_str::<RuntimeConfig>(&raw)
        .map_err(|err| format!("failed to parse runtime config {}: {err}", path.display()))
}

fn default_runtime_config_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let current = dir.join(DEFAULT_RUNTIME_CONFIG_FILE);
    Some(current)
}

fn allow_file_override(matches: &clap::ArgMatches, id: &str) -> bool {
    matches
        .value_source(id)
        .map(|source| source == ValueSource::DefaultValue)
        .unwrap_or(true)
}

fn apply_runtime_overrides(
    args: &mut crate::Args,
    matches: &clap::ArgMatches,
    cfg: &RuntimeConfig,
) -> Result<(), String> {
    if allow_file_override(matches, "host")
        && let Some(host) = cfg.backend.host.as_deref().map(str::trim)
        && !host.is_empty()
    {
        args.host = host.to_string();
    }

    if allow_file_override(matches, "port")
        && let Some(port) = cfg.backend.port
    {
        args.port = port;
    }

    if allow_file_override(matches, "ui_password") {
        args.ui_password = cfg.backend.ui_password.clone();
    }

    if allow_file_override(matches, "ui_dir") {
        args.ui_dir = cfg
            .backend
            .ui_dir
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned);
    }

    if allow_file_override(matches, "cors_origin")
        && let Some(origins) = cfg.backend.cors_origins.clone()
    {
        args.cors_origin = origins;
    }

    if allow_file_override(matches, "cors_allow_all")
        && let Some(allow_all) = cfg.backend.cors_allow_all
    {
        args.cors_allow_all = allow_all;
    }

    if allow_file_override(matches, "ui_cookie_samesite") {
        args.ui_cookie_samesite = match cfg.backend.ui_cookie_samesite.as_deref().map(str::trim) {
            Some("") | None => crate::UiCookieSameSite::Auto,
            Some(value) => parse_ui_cookie_samesite(value)?,
        };
    }

    Ok(())
}

fn parse_ui_cookie_samesite(value: &str) -> Result<crate::UiCookieSameSite, String> {
    if value.eq_ignore_ascii_case("auto") {
        Ok(crate::UiCookieSameSite::Auto)
    } else if value.eq_ignore_ascii_case("strict") {
        Ok(crate::UiCookieSameSite::Strict)
    } else if value.eq_ignore_ascii_case("lax") {
        Ok(crate::UiCookieSameSite::Lax)
    } else if value.eq_ignore_ascii_case("none") {
        Ok(crate::UiCookieSameSite::None)
    } else {
        Err(format!(
            "invalid backend.ui_cookie_samesite in runtime config: {value}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_args_and_matches() -> (crate::Args, clap::ArgMatches) {
        let matches = crate::Args::command().get_matches_from(["agena-studio"]);
        let args = crate::Args::from_arg_matches(&matches).expect("default args should parse");
        (args, matches)
    }

    #[test]
    fn runtime_config_applies_backend_fields_from_file() {
        let (mut args, matches) = default_args_and_matches();
        let cfg = RuntimeConfig {
            backend: BackendRuntimeConfig {
                host: Some("0.0.0.0".to_string()),
                port: Some(4317),
                ui_password: Some("secret".to_string()),
                ui_dir: Some("/tmp/agena-studio-dist".to_string()),
                cors_origins: Some(vec!["http://localhost:5173".to_string()]),
                cors_allow_all: Some(true),
                ui_cookie_samesite: Some("strict".to_string()),
                ..BackendRuntimeConfig::default()
            },
        };

        apply_runtime_overrides(&mut args, &matches, &cfg)
            .expect("runtime config overrides should apply");

        assert_eq!(args.host, "0.0.0.0");
        assert_eq!(args.port, 4317);
        assert_eq!(args.ui_password.as_deref(), Some("secret"));
        assert_eq!(args.ui_dir.as_deref(), Some("/tmp/agena-studio-dist"));
        assert_eq!(args.cors_origin, vec!["http://localhost:5173"]);
        assert!(args.cors_allow_all);
        assert!(matches!(
            args.ui_cookie_samesite,
            crate::UiCookieSameSite::Strict
        ));
    }
}
