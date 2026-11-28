//! MCPB/DTX manifests and their bundle-relative process configuration.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde_json::{Map, Value};

use super::{
    MAX_MANIFEST_BYTES, mcp, package_path, read_package_bytes, regular_file, required_name,
};

const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Bundle {
    archive: PathBuf,
    extracted: Arc<Mutex<Option<PathBuf>>>,
    pub(super) options: BTreeMap<String, String>,
}

pub(super) fn is_bundle_path(path: &str) -> bool {
    [".mcpb", ".dxt"]
        .iter()
        .any(|ending| path.ends_with(ending))
}

impl Bundle {
    pub(super) fn read(root: &Path, relative: &Path) -> Result<(String, Value, Self), String> {
        let path = package_path(root, relative)?;
        if !regular_file(&path) || fs::metadata(&path).map_err(display)?.len() > MAX_ARCHIVE_BYTES {
            return Err("MCP bundle is missing or exceeds 256 MiB".into());
        }
        let mut archive =
            zip::ZipArchive::new(File::open(&path).map_err(display)?).map_err(display)?;
        let entry = archive.by_name("manifest.json").map_err(display)?;
        if entry.is_symlink() || entry.size() > MAX_MANIFEST_BYTES {
            return Err("invalid MCP bundle manifest entry".into());
        }
        let mut bytes = Vec::new();
        entry
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(display)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err("MCP bundle manifest exceeds its byte budget".into());
        }
        let manifest: Map<String, Value> = serde_json::from_slice(&bytes).map_err(display)?;
        let name = required_name(&manifest)?.to_owned();
        let server = manifest
            .get("server")
            .and_then(Value::as_object)
            .ok_or("MCP bundle requires a server declaration")?;
        if !matches!(
            server.get("type").and_then(Value::as_str),
            Some("node" | "python" | "binary" | "uv")
        ) {
            return Err("unknown MCP bundle server type".into());
        }
        let mut config = server
            .get("mcp_config")
            .and_then(Value::as_object)
            .cloned()
            .ok_or("MCP bundle server requires mcp_config")?;
        let platform = if cfg!(target_os = "windows") {
            "win32"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        };
        if manifest
            .get("compatibility")
            .and_then(|compatibility| compatibility.get("platforms"))
            .and_then(Value::as_array)
            .is_some_and(|platforms| {
                platforms
                    .iter()
                    .all(|value| value.as_str() != Some(platform))
            })
        {
            return Err(format!("MCP bundle does not support {platform}"));
        }
        if let Some(overrides) = config
            .remove("platform_overrides")
            .as_ref()
            .and_then(|platforms| platforms.get(platform))
            .and_then(Value::as_object)
        {
            for (key, value) in overrides {
                if key == "env"
                    && let Some(extra) = value.as_object()
                    && let Some(env) = config.get_mut("env").and_then(Value::as_object_mut)
                {
                    env.extend(extra.clone());
                } else {
                    config.insert(key.clone(), value.clone());
                }
            }
        }
        config.insert("type".into(), Value::String("stdio".into()));
        if let Some(command) = config.get_mut("command").and_then(|command| match command {
            Value::String(command) => Some(command),
            _ => None,
        }) && !command.starts_with("./")
            && !command.starts_with("${")
            && !Path::new(command.as_str()).is_absolute()
            && (command.contains('/')
                || server.get("type").and_then(Value::as_str) == Some("binary"))
        {
            command.insert_str(0, "./");
        }
        let mut config = Value::Object(config);
        replace_variables(&mut config);
        let mut options = BTreeMap::new();
        mcp::add_user_defaults(manifest.get("user_config"), &mut options);
        Ok((
            name,
            config,
            Self {
                archive: relative.to_owned(),
                extracted: Arc::new(Mutex::new(None)),
                options,
            },
        ))
    }

    pub(super) fn materialize(&self, root: &Path) -> Result<PathBuf, String> {
        let mut extracted = self
            .extracted
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(path) = extracted
            .as_ref()
            .filter(|path| path.is_dir() && !path.is_symlink())
        {
            return Ok(path.clone());
        }
        let bytes = read_package_bytes(root, &self.archive, MAX_ARCHIVE_BYTES)?
            .ok_or("MCP bundle archive disappeared")?;
        let cache = agena_plugin_marketplace::default_cache_root().join("mcp-bundles");
        agena_storage_layout::secure_directory(&cache).map_err(display)?;
        let target = cache.join(blake3::hash(&bytes).to_hex().as_str());
        if target.is_symlink() {
            return Err("invalid MCP bundle cache directory".into());
        }
        if !target.is_dir() {
            let staging = tempfile::Builder::new()
                .prefix(".mcpb-")
                .tempdir_in(&cache)
                .map_err(display)?;
            agena_plugin_marketplace::agent_packages::extract_package_zip(&bytes, staging.path())
                .map_err(display)?;
            match fs::rename(staging.path(), &target) {
                Ok(()) => {}
                Err(_) if target.is_dir() && !target.is_symlink() => {}
                Err(error) => return Err(display(error)),
            }
        }
        *extracted = Some(target.clone());
        Ok(target)
    }
}

fn replace_variables(value: &mut Value) {
    match value {
        Value::String(value) => {
            *value = value
                .replace("${__dirname}", "${CLAUDE_PLUGIN_ROOT}")
                .replace("${pathSeparator}", std::path::MAIN_SEPARATOR_STR)
                .replace("${/}", std::path::MAIN_SEPARATOR_STR);
        }
        Value::Array(values) => values.iter_mut().for_each(replace_variables),
        Value::Object(values) => values.values_mut().for_each(replace_variables),
        _ => {}
    }
}

fn display(error: impl std::fmt::Display) -> String {
    format!("MCP bundle: {error}")
}

#[cfg(test)]
#[test]
fn manifest_reads_platform_overrides_and_translates_bundle_variables() {
    use std::io::Write;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("probe.mcpb");
    let mut archive = zip::ZipWriter::new(File::create(&path).unwrap());
    archive
        .start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(br#"{
        "name":"probe","manifest_version":"0.3",
        "server":{"type":"node","mcp_config":{
            "command":"node","args":["${__dirname}${/}server.js"],"env":{"MODE":"default","BASE":"true"},
            "platform_overrides":{
                "linux":{"env":{"MODE":"linux"}},
                "darwin":{"env":{"MODE":"darwin"}},
                "win32":{"env":{"MODE":"win32"}}
            }
        }},
        "user_config":{"directory":{"type":"string","default":"sample"}}
    }"#).unwrap();
    archive.finish().unwrap();
    let (name, config, bundle) = Bundle::read(directory.path(), Path::new("probe.mcpb")).unwrap();
    assert_eq!(name, "probe");
    assert_eq!(bundle.options["directory"], "sample");
    assert_eq!(
        config["args"][0],
        format!(
            "${{CLAUDE_PLUGIN_ROOT}}{}server.js",
            std::path::MAIN_SEPARATOR
        )
    );
    assert_eq!(config["env"]["BASE"], "true");
    let platform = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "win32",
        _ => "default",
    };
    assert_eq!(config["env"]["MODE"], platform);
}
