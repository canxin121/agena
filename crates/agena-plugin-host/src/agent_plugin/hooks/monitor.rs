use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Stdio,
};

use agena_plugin_sdk::host_api::ImportedPluginNotices;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Map, Value};
use tokio_util::codec::{FramedRead, LinesCodec};

use crate::agent_plugin::{
    MAX_COMPONENT_ENTRIES, MAX_MANIFEST_BYTES, manifest_path, read_package_bytes,
};

#[derive(Clone, Debug)]
pub(in crate::agent_plugin) struct Monitor {
    pub(super) name: String,
    pub(super) command: String,
    pub(super) description: String,
    pub(super) skill: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Declaration {
    name: String,
    command: String,
    description: String,
    when: Option<String>,
}

pub(in crate::agent_plugin) fn discover(
    root: &Path,
    manifest: &Map<String, Value>,
    watch: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<String>,
) -> Vec<Monitor> {
    let declared = manifest
        .get("experimental")
        .and_then(|value| value.get("monitors"))
        .or_else(|| manifest.get("monitors"));
    let declarations = match declared {
        Some(Value::Array(entries)) => Ok(entries.clone()),
        Some(Value::String(relative)) => monitor_file(root, relative, true, watch),
        None => monitor_file(root, "./monitors/monitors.json", false, watch),
        _ => Err("monitors must declare a local path or an array".into()),
    };
    let declarations = match declarations {
        Ok(declarations) => declarations,
        Err(error) => {
            diagnostics.push(format!("monitors: {error}"));
            return Vec::new();
        }
    };
    let mut names = BTreeSet::new();
    declarations
        .into_iter()
        .take(MAX_COMPONENT_ENTRIES)
        .filter_map(|value| {
            let result = (|| {
                let row = serde_json::from_value::<Declaration>(value)
                    .map_err(|error| format!("invalid monitor declaration: {error}"))?;
                if row.name.is_empty()
                    || row.name.len() > 128
                    || !row
                        .name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
                {
                    return Err("invalid monitor name".into());
                }
                if row.command.trim().is_empty()
                    || row.command.len() > 64 * 1024
                    || row.command.contains('\0')
                {
                    return Err(format!("monitor {:?} needs a shell command", row.name));
                }
                if row.description.trim().is_empty() || row.description.len() > 1024 {
                    return Err(format!("monitor {:?} needs a description", row.name));
                }
                let skill = match row.when.as_deref() {
                    None | Some("always") => None,
                    Some(when) => Some(
                        when.strip_prefix("on-skill-invoke:")
                            .filter(|skill| !skill.is_empty())
                            .ok_or_else(|| format!("invalid monitor trigger {when:?}"))?
                            .to_owned(),
                    ),
                };
                if !names.insert(row.name.clone()) {
                    return Err(format!("duplicate monitor {:?}", row.name));
                }
                Ok(Monitor {
                    name: row.name,
                    command: row.command,
                    description: row.description,
                    skill,
                })
            })();
            match result {
                Ok(monitor) => Some(monitor),
                Err(error) => {
                    diagnostics.push(format!("monitors: {error}"));
                    None
                }
            }
        })
        .collect()
}

fn monitor_file(
    root: &Path,
    relative: &str,
    explicit: bool,
    watch: &mut BTreeSet<PathBuf>,
) -> Result<Vec<Value>, String> {
    let path = manifest_path(root, relative)?;
    watch.insert(path.clone());
    let path = path
        .strip_prefix(root)
        .expect("monitor path is under package root");
    match read_package_bytes(root, path, MAX_MANIFEST_BYTES)? {
        Some(data) => serde_json::from_slice(&data)
            .map_err(|error| format!("invalid monitor file {relative:?}: {error}")),
        None if explicit => Err(format!("monitor file {relative:?} is missing")),
        None => Ok(Vec::new()),
    }
}

pub(super) async fn run(
    mut command: tokio::process::Command,
    monitor: Monitor,
    notices: &dyn ImportedPluginNotices,
) -> Result<(), String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child =
        agena_process::spawn(command).map_err(|error| format!("monitor start failed: {error}"))?;
    let stdout = child
        .stdout()
        .take()
        .ok_or("monitor stdout is unavailable")?;
    let mut lines = FramedRead::new(stdout, LinesCodec::new_with_max_length(16 * 1024));
    while let Some(line) = lines.next().await {
        let line = line.map_err(|error| format!("invalid monitor output: {error}"))?;
        if !line.trim().is_empty() {
            notices
                .deliver(format!("{}: {line}", monitor.description), true)
                .await
                .map_err(|error| error.diagnostic_message().to_owned())?;
        }
    }
    let status = child
        .wait()
        .await
        .map_err(|error| format!("monitor wait failed: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("monitor exited with {status}"))
    }
}
