//! Resolve published package roots using the upstream managers' installed state.

use std::path::{Path, PathBuf};

use agena_plugin_sdk::configuration::{AgentPluginBridgeConfig, AgentPluginRuntime};
use serde_json::Value;

use super::{MAX_MANIFEST_BYTES, read_package_object};

const CLAUDE_INSTALLED: &str = "plugins/installed_plugins.json";
const MAX_INSTALLATION_BYTES: u64 = 8 * 1024 * 1024;

pub fn resolve_agent_plugin_path(
    path: &Path,
    bridge: Option<&AgentPluginBridgeConfig>,
) -> Result<PathBuf, String> {
    match bridge {
        Some(bridge) if bridge.follow_installation => native_installation_path(bridge),
        _ => Ok(path.to_owned()),
    }
}

pub fn native_installation_path(bridge: &AgentPluginBridgeConfig) -> Result<PathBuf, String> {
    let home = &bridge.config_dir;
    if !home.is_absolute() {
        return Err("native installation requires an absolute config directory".into());
    }
    let path = match &bridge.runtime {
        AgentPluginRuntime::Codex { plugin } => {
            let (name, market) = marketplace_plugin_parts(plugin)?;
            let cache = home.join("plugins/cache").join(market).join(name);
            active_codex_installation(&cache).ok_or_else(|| {
                format!("Codex plugin {plugin:?} is missing from the selected installation")
            })?
        }
        AgentPluginRuntime::ClaudeCode {
            plugin: Some(plugin),
        } => {
            marketplace_plugin_parts(plugin)?;
            let installed =
                read_package_object(home, Path::new(CLAUDE_INSTALLED), MAX_INSTALLATION_BYTES)?
                    .ok_or("Claude Code installed plugin inventory is missing")?;
            let records = installed
                .get("plugins")
                .and_then(|plugins| plugins.get(plugin))
                .and_then(Value::as_array)
                .ok_or_else(|| format!("Claude Code plugin {plugin:?} is not installed"))?;
            let mut user_installations = records
                .iter()
                .filter(|record| record.get("scope").and_then(Value::as_str) == Some("user"));
            let record = user_installations.next().ok_or_else(|| {
                format!("Claude Code plugin {plugin:?} has no user-scope installation")
            })?;
            if user_installations.next().is_some() {
                return Err(format!(
                    "Claude Code lists multiple user installations for {plugin:?}"
                ));
            }
            record
                .get("installPath")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .ok_or("Claude Code installation is missing its installPath")?
        }
        AgentPluginRuntime::DeepseekHarness {
            profile,
            bundle: Some(bundle),
        } => deepseek_installation_path(home, profile, bundle, bridge.executable.as_deref())?,
        AgentPluginRuntime::ClaudeCode { plugin: None } => {
            return Err(
                "following a Claude Code installation requires its plugin@marketplace id".into(),
            );
        }
        AgentPluginRuntime::DeepseekHarness { bundle: None, .. } => {
            return Err("following a DeepSeek Harness installation requires a bundle name".into());
        }
    };
    if !path.is_absolute() {
        return Err("native plugin manager returned a relative installation path".into());
    }
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve native plugin installation: {error}"))?;
    if !path.is_dir() {
        return Err("native plugin installation must be a directory".into());
    }
    Ok(path)
}

pub fn native_installation_watch_paths(bridge: &AgentPluginBridgeConfig) -> Vec<PathBuf> {
    let home = &bridge.config_dir;
    if !home.is_absolute() {
        return Vec::new();
    }
    match &bridge.runtime {
        AgentPluginRuntime::Codex { plugin } => marketplace_plugin_parts(plugin)
            .ok()
            .map(|(name, market)| home.join("plugins/cache").join(market).join(name))
            .into_iter()
            .collect(),
        AgentPluginRuntime::ClaudeCode { plugin: Some(_) } => vec![home.join(CLAUDE_INSTALLED)],
        AgentPluginRuntime::DeepseekHarness {
            profile,
            bundle: Some(bundle),
        } if safe_segment(profile) && valid_npm_package_name(bundle) => {
            let profile = home.join("profiles").join(profile);
            vec![
                profile.join("package.json"),
                profile.join("node_modules").join(bundle),
            ]
        }
        _ => Vec::new(),
    }
}

pub(super) fn deepseek_installation_path(
    home: &Path,
    profile: &str,
    bundle: &str,
    executable: Option<&Path>,
) -> Result<PathBuf, String> {
    if !safe_segment(profile) || !valid_npm_package_name(bundle) {
        return Err("invalid DeepSeek Harness profile or bundle name".into());
    }
    let root = home.join("profiles").join(profile);
    let bundles = deepseek_profile_bundles(&root)?
        .ok_or_else(|| format!("DeepSeek Harness profile {profile:?} is missing"))?;
    if !bundles.iter().any(|value| value == bundle) {
        return Err(format!(
            "DeepSeek Harness bundle {bundle:?} is not selected in profile {profile:?}"
        ));
    }
    let installed = executable
        .map(Path::to_path_buf)
        .or_else(|| which::which("dsh").ok());
    if let Some(path) = installed.and_then(|path| shipped_deepseek_bundle(&path, bundle)) {
        return Ok(path);
    }
    Ok(root.join("node_modules").join(bundle))
}

fn shipped_deepseek_bundle(executable: &Path, bundle: &str) -> Option<PathBuf> {
    let executable = executable.canonicalize().ok()?;
    let package = executable.ancestors().skip(1).find(|dir| {
        read_package_object(dir, Path::new("package.json"), MAX_MANIFEST_BYTES)
            .ok()
            .flatten()
            .is_some_and(|manifest| {
                manifest.get("name").and_then(Value::as_str) == Some("@deepseek-ai/dsh")
            })
    })?;
    package
        .ancestors()
        .filter(|dir| dir.file_name().is_none_or(|name| name != "node_modules"))
        .map(|dir| dir.join("node_modules").join(bundle))
        .find(|dir| dir.join("package.json").is_file())
}

pub(super) fn deepseek_profile_bundles(root: &Path) -> Result<Option<Vec<String>>, String> {
    let Some(manifest) = read_package_object(root, Path::new("package.json"), MAX_MANIFEST_BYTES)?
    else {
        return Ok(None);
    };
    let bundles = manifest
        .get("dsh")
        .and_then(|dsh| dsh.pointer("/profile/bundles"))
        .and_then(Value::as_array)
        .ok_or("DeepSeek Harness profile needs a dsh.profile.bundles array")?;
    bundles
        .iter()
        .map(|bundle| {
            bundle
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "invalid DeepSeek Harness bundle selection".into())
        })
        .collect::<Result<_, _>>()
        .map(Some)
}

pub(super) fn marketplace_plugin_parts(value: &str) -> Result<(&str, &str), String> {
    value
        .split_once('@')
        .filter(|(plugin, market)| safe_segment(plugin) && safe_segment(market))
        .ok_or_else(|| "expected a native plugin id of the form plugin@marketplace".to_owned())
}

pub(super) fn valid_npm_package_name(name: &str) -> bool {
    if let Some(scoped) = name.strip_prefix('@') {
        scoped
            .split_once('/')
            .is_some_and(|(scope, name)| safe_segment(scope) && safe_segment(name))
    } else {
        safe_segment(name)
    }
}

pub(super) fn safe_segment(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn active_codex_installation(root: &Path) -> Option<PathBuf> {
    let mut versions = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|version| {
            !matches!(version.as_str(), "" | "." | "..")
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte))
        })
        .collect::<Vec<_>>();
    if versions.iter().any(|version| version == "local") {
        return Some(root.join("local"));
    }
    versions.sort_unstable_by(|left, right| {
        match (semver::Version::parse(left), semver::Version::parse(right)) {
            (Ok(left), Ok(right)) => left.cmp(&right),
            _ => left.cmp(right),
        }
    });
    versions.pop().map(|version| root.join(version))
}
