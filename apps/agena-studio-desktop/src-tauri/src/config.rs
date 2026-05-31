use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::AppHandle;

const SHARED_CONFIG_DIR_NAME: &str = "agena";
const SHARED_CONFIG_FILE_NAME: &str = "agena.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopConfig {
    #[serde(default = "default_autostart_on_boot")]
    pub autostart_on_boot: bool,
    pub backend: BackendConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackendConfig {
    pub host: String,
    pub port: u16,
    pub ui_dir: Option<String>,
    pub cors_origins: Vec<String>,
    pub cors_allow_all: bool,
    pub backend_log_level: Option<String>,
    pub ui_password: Option<String>,
    pub ui_cookie_samesite: Option<String>,
    pub workspace_root: Option<String>,
    pub database_path: Option<String>,
    pub database_url: Option<String>,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            autostart_on_boot: default_autostart_on_boot(),
            backend: BackendConfig::default(),
        }
    }
}

fn default_autostart_on_boot() -> bool {
    true
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3210,
            ui_dir: None,
            cors_origins: Vec::new(),
            cors_allow_all: false,
            backend_log_level: None,
            ui_password: Some(String::new()),
            ui_cookie_samesite: None,
            workspace_root: None,
            database_path: None,
            database_url: None,
        }
    }
}

pub fn runtime_config_path(app: &AppHandle) -> Option<PathBuf> {
    let _ = app;
    Some(default_config_path())
}

pub fn load_or_create(app: &AppHandle) -> Result<DesktopConfig, String> {
    let path = runtime_config_path(app)
        .ok_or_else(|| "unable to resolve shared config path".to_string())?;
    ensure_parent_dir(&path)?;
    let mut doc = read_shared_config_doc(&path)?;
    let current = doc.get("desktop").cloned();
    let normalized = normalize_config(match current.as_ref() {
        Some(value) => parse_desktop_config(value)?,
        None => DesktopConfig::default(),
    });
    let next = serde_json::to_value(&normalized)
        .map_err(|e| format!("serialize shared desktop config: {e}"))?;

    if current.as_ref() != Some(&next) || !path.exists() {
        doc.insert("desktop".to_string(), next);
        write_shared_config_doc(&path, JsonValue::Object(doc))?;
    }

    Ok(normalized)
}

pub fn save(app: &AppHandle, cfg: DesktopConfig) -> Result<DesktopConfig, String> {
    let path = runtime_config_path(app)
        .ok_or_else(|| "unable to resolve shared config path".to_string())?;
    ensure_parent_dir(&path)?;

    let mut doc = read_shared_config_doc(&path)?;
    let normalized = normalize_config(cfg);
    let next = serde_json::to_value(&normalized)
        .map_err(|e| format!("serialize shared desktop config: {e}"))?;
    doc.insert("desktop".to_string(), next);
    write_shared_config_doc(&path, JsonValue::Object(doc))?;
    Ok(normalized)
}

pub fn open_runtime_config_file(app: &AppHandle) -> Result<(), String> {
    let path = runtime_config_path(app)
        .ok_or_else(|| "unable to resolve shared config path".to_string())?;
    ensure_parent_dir(&path)?;

    let _ = load_or_create(app)?;

    use tauri_plugin_opener::OpenerExt;
    let _ = app
        .opener()
        .open_path(path.to_string_lossy().as_ref(), None::<&str>);
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {parent:?}: {e}"))?;
    }
    Ok(())
}

fn read_shared_config_doc(path: &Path) -> Result<JsonMap<String, JsonValue>, String> {
    if !path.exists() {
        return Ok(JsonMap::new());
    }

    let txt = fs::read_to_string(path).map_err(|e| format!("read shared config: {e}"))?;
    if txt.trim().is_empty() {
        return Ok(JsonMap::new());
    }

    let value =
        serde_json::from_str::<JsonValue>(&txt).map_err(|e| format!("parse shared config: {e}"))?;
    match value {
        JsonValue::Object(map) => Ok(map),
        _ => Err("shared config root must be a JSON object".to_string()),
    }
}

fn write_shared_config_doc(path: &Path, doc: JsonValue) -> Result<(), String> {
    let txt =
        serde_json::to_string_pretty(&doc).map_err(|e| format!("serialize shared config: {e}"))?;
    fs::write(path, format!("{txt}\n")).map_err(|e| format!("write shared config: {e}"))
}

fn parse_desktop_config(value: &JsonValue) -> Result<DesktopConfig, String> {
    if value.is_null() {
        return Ok(DesktopConfig::default());
    }
    serde_json::from_value::<DesktopConfig>(value.clone())
        .map_err(|e| format!("parse shared config.desktop: {e}"))
}

fn default_config_path() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(SHARED_CONFIG_DIR_NAME)
        .join(SHARED_CONFIG_FILE_NAME)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .map(PathBuf::from)
}

fn normalize_config(mut cfg: DesktopConfig) -> DesktopConfig {
    cfg.backend.host = normalize_host(&cfg.backend.host);
    cfg.backend.port = normalize_port(cfg.backend.port);
    cfg.backend.ui_dir = normalize_optional_text(cfg.backend.ui_dir.take());
    if cfg.backend.ui_password.is_none() {
        cfg.backend.ui_password = Some(String::new());
    }
    cfg.backend.ui_password = Some(
        cfg.backend
            .ui_password
            .take()
            .unwrap_or_default()
            .trim()
            .to_string(),
    );
    cfg.backend.cors_origins = normalize_cors_origins(cfg.backend.cors_origins);
    cfg.backend.backend_log_level = normalize_log_level(cfg.backend.backend_log_level.take());
    cfg.backend.ui_cookie_samesite =
        normalize_ui_cookie_samesite(cfg.backend.ui_cookie_samesite.take());
    cfg.backend.workspace_root = normalize_optional_text(cfg.backend.workspace_root.take());
    cfg.backend.database_path = normalize_optional_text(cfg.backend.database_path.take());
    cfg.backend.database_url = normalize_optional_text(cfg.backend.database_url.take());
    cfg
}

fn normalize_optional_text(raw: Option<String>) -> Option<String> {
    let value = raw?.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn normalize_port(raw: u16) -> u16 {
    if raw == 0 { 3210 } else { raw }
}

fn normalize_host(raw: &str) -> String {
    let v = raw.trim();
    if v.is_empty() {
        "127.0.0.1".to_string()
    } else {
        v.to_string()
    }
}

fn normalize_cors_origins(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::<String>::new();
    for raw in values {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.iter().any(|v| v == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn normalize_log_level(raw: Option<String>) -> Option<String> {
    let level = raw?.trim().to_ascii_uppercase();
    match level.as_str() {
        "DEBUG" | "INFO" | "WARN" | "ERROR" => Some(level),
        _ => None,
    }
}

fn normalize_ui_cookie_samesite(raw: Option<String>) -> Option<String> {
    let value = raw?.trim().to_ascii_lowercase();
    match value.as_str() {
        "auto" | "strict" | "lax" | "none" => Some(value),
        _ => None,
    }
}
