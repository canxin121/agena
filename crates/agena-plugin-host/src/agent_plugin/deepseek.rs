//! Compose a DeepSeek Harness bundle and translate supported Cordis entries.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use rquickjs::Ctx;
use serde_json::{Map, Value};
use serde_yaml::Value as Yaml;
use url::Url;

use super::{MAX_MANIFEST_BYTES, package_path, read_package_bytes, regular_file};

const MCP_MODULE: &str = "@deepseek-ai/dsh-mcp-client";
const MCP_MANAGER: &str = "@deepseek-ai/dsh-mcp-manager";
const MCP_MANAGER_ENTRY: &str = "mcp-manager";
const SKILL_FILESYSTEM: &str = "@deepseek-ai/dsh-skill-filesystem";
const SKILL_FILESYSTEM_ENTRY: &str = "skill-filesystem";
const SKILL_REGISTRY: &str = "@deepseek-ai/dsh-skill";
const SKILL_TOOL: &str = "@deepseek-ai/dsh-tool-skill";
const MAX_PATCH_ROWS: usize = 4_096;
const DSH_REFERENCE_RELEASES: &[&str] = &["0.2.2-rc.5", "0.2.3-alpha.2"];
pub(super) const CLAUDE_HOOK_MODULE: &str = "@deepseek-ai/dsh-hooks-claude-code";
pub(super) const CODEX_HOOK_MODULE: &str = "@deepseek-ai/dsh-hooks-codex";
pub(super) const LSP_MODULE: &str = "@deepseek-ai/dsh-lsp-stdio";

pub(super) fn diagnose_engine_requirements(
    document: &Map<String, Value>,
    diagnostics: &mut Vec<String>,
) {
    use agena_plugin_marketplace::agent_packages::ClaudeDependencyVersion;

    let engine = document
        .get("engines")
        .and_then(|engines| engines.get("dsh"));
    let requirements: Vec<_> = if let Some(engine) = engine {
        vec![("engines.dsh", engine)]
    } else {
        document
            .get("peerDependencies")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|peers| peers.iter())
            .filter(|(name, _)| name.starts_with("@deepseek-ai/dsh"))
            .map(|(name, version)| (name.as_str(), version))
            .collect()
    };
    for (field, requirement) in requirements {
        let Some(requirement) = requirement.as_str() else {
            diagnostics.push(format!("DeepSeek Harness {field} must be a version range"));
            continue;
        };
        let Ok(range) = ClaudeDependencyVersion::parse(requirement) else {
            diagnostics.push(format!(
                "DeepSeek Harness {field} has an invalid version range"
            ));
            continue;
        };
        if !DSH_REFERENCE_RELEASES
            .iter()
            .any(|version| range.matches(version))
        {
            diagnostics.push(format!(
                "DeepSeek Harness {field} requires {requirement:?}. Agena's adapter targets DSH {}.",
                DSH_REFERENCE_RELEASES.join(" and ")
            ));
        }
    }
}

pub(super) fn is_native_module(name: &str) -> bool {
    matches!(
        name,
        MCP_MODULE
            | MCP_MANAGER
            | CLAUDE_HOOK_MODULE
            | CODEX_HOOK_MODULE
            | LSP_MODULE
            | SKILL_FILESYSTEM
            | super::memvault::MODULE
            | super::deepseek_commands::storage::HUB
            | super::deepseek_commands::storage::JSON_BACKEND
            | super::deepseek_commands::storage::SQLITE_BACKEND
            | super::deepseek_commands::storage::DOMAIN
            | super::deepseek_commands::WORKFLOW_PTC
            | super::deepseek_commands::SUBAGENT_SPAWN
            | super::deepseek_commands::SUBAGENT_FORK
            | super::deepseek_commands::SUBAGENT
            | super::deepseek_commands::JOBS_LOCAL
    )
}

pub(super) fn patch_paths(
    root: &Path,
    manifest: &Map<String, Value>,
) -> Result<Option<Vec<PathBuf>>, String> {
    let Some(patch) = manifest
        .get("dsh")
        .and_then(|dsh| dsh.pointer("/bundle/patch"))
    else {
        return Ok(None);
    };
    let patches = patch
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(patch));
    patches
        .iter()
        .map(|patch| {
            let relative = patch.as_str().ok_or(
                "DeepSeek Harness dsh.bundle.patch must be a local path or a list of paths",
            )?;
            let path = package_path(root, Path::new(relative))?;
            if !regular_file(&path) {
                return Err("DeepSeek Harness bundle patch does not exist".into());
            }
            Ok(path)
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

pub(super) struct McpEntries {
    pub configs: BTreeMap<String, Value>,
    pub managed: Vec<super::mcp::DeepSeekManagedMcpServer>,
}

pub(super) fn literal_mcp_servers(
    rows: &[(String, Yaml)],
    registered: &[(String, Vec<(String, Yaml)>)],
    diagnostics: &mut Vec<String>,
) -> McpEntries {
    let mut servers = BTreeMap::new();
    let mut declared = BTreeSet::new();
    let mut conflicts = BTreeSet::new();
    let mut managed = Vec::new();
    let mut managed_names = BTreeSet::new();
    for (id, row) in rows {
        let mut add = |name: &str, result: Result<Value, String>| {
            if !declared.insert(name.to_owned()) {
                conflicts.insert(name.to_owned());
            }
            match result {
                Ok(config) => {
                    servers.insert(name.to_owned(), config);
                }
                Err(error) => diagnostics.push(format!(
                    "DeepSeek Harness MCP entry {id:?}, server {name:?}: {error}"
                )),
            }
        };
        let config = row.get("config").unwrap_or(&Yaml::Null);
        match module_name(row) {
            Some(MCP_MODULE) => {
                let name = config
                    .get("serverName")
                    .and_then(Yaml::as_str)
                    .unwrap_or_default();
                add(name, translate_mcp_server(name, config, false));
            }
            Some(MCP_MANAGER) => {
                let Some(entries) = config.get("servers") else {
                    continue;
                };
                let Some(entries) = entries.as_mapping() else {
                    diagnostics.push(format!(
                        "DeepSeek Harness MCP manager {id:?}: servers must be a mapping"
                    ));
                    continue;
                };
                for (name, server) in entries {
                    let name = name.as_str().unwrap_or_default();
                    let flag = server.get("enabled");
                    let enabled = flag.and_then(Yaml::as_bool) != Some(false);
                    if managed_names.insert(name.to_owned()) {
                        managed.push(super::mcp::DeepSeekManagedMcpServer {
                            name: name.to_owned(),
                            enabled,
                            registered: false,
                            server: None,
                        });
                    }
                    let config = match flag {
                        None | Some(Yaml::Bool(_)) => translate_mcp_server(name, server, true),
                        Some(_) => Err("enabled must be a boolean".into()),
                    };
                    add(name, config);
                }
            }
            _ => {}
        }
    }
    for name in &conflicts {
        servers.remove(name);
        diagnostics.push(format!(
            "DeepSeek Harness MCP server {name:?} is declared more than once"
        ));
    }
    for (id, entries) in registered {
        for (name, config) in entries {
            if servers.contains_key(name)
                || conflicts.contains(name)
                || managed_names.contains(name)
            {
                diagnostics.push(format!(
                    "DeepSeek Harness module {id:?}: MCP server {name:?} is already declared"
                ));
                continue;
            }
            match translate_mcp_server(name, config, true) {
                Ok(config) => {
                    servers.insert(name.clone(), config);
                    managed.push(super::mcp::DeepSeekManagedMcpServer {
                        name: name.clone(),
                        enabled: true,
                        registered: true,
                        server: None,
                    });
                }
                Err(error) => diagnostics.push(format!(
                    "DeepSeek Harness module {id:?}, MCP server {name:?}: {error}"
                )),
            }
        }
    }
    McpEntries {
        configs: servers,
        managed,
    }
}

pub(super) fn literal_rows(
    root: &Path,
    manifest: &Map<String, Value>,
    environment: &[String],
    watch: &mut BTreeSet<PathBuf>,
    diagnostics: &mut Vec<String>,
) -> Vec<(String, Yaml)> {
    let paths = match patch_paths(root, manifest) {
        Ok(Some(paths)) => paths,
        Ok(None) => return Vec::new(),
        Err(error) => {
            diagnostics.push(error);
            return Vec::new();
        }
    };
    watch.extend(paths.iter().cloned());

    let mut entries = Vec::new();
    let mut patches_seen = 0;
    for path in paths {
        let relative = path
            .strip_prefix(root)
            .expect("bundle patch under package root");
        let patches = match read_patches(root, relative) {
            Ok(patches) => patches,
            Err(error) => {
                diagnostics.push(format!("DeepSeek Harness patch {relative:?}: {error}"));
                return Vec::new();
            }
        };
        patches_seen += patches.len();
        if patches_seen > MAX_PATCH_ROWS {
            diagnostics.push("too many DeepSeek Harness patches".into());
            return Vec::new();
        }
        for patch in patches {
            apply_patch(&mut entries, patch, relative, diagnostics);
            if entry_count(&entries, 0).is_none_or(|count| count > MAX_PATCH_ROWS) {
                diagnostics.push(
                    "DeepSeek Harness Cordis composition exceeds its size or depth limit".into(),
                );
                return Vec::new();
            }
        }
    }

    let mut rows = Vec::new();
    let mut skipped = BTreeSet::new();
    if let Err(error) = super::deepseek_js::with_loader_environment(root, environment, |ctx| {
        collect_rows(ctx, manifest, entries, &mut rows, &mut skipped, diagnostics);
        Ok(())
    }) {
        diagnostics.push(error);
    }
    for name in skipped {
        diagnostics.push(format!(
            "DeepSeek Harness module {name:?} has no Agena adapter"
        ));
    }
    rows
}

fn children(entry: &Yaml) -> Option<&[Yaml]> {
    if entry.get("group").and_then(Yaml::as_bool) == Some(true) {
        entry
            .get("config")
            .and_then(Yaml::as_sequence)
            .map(Vec::as_slice)
    } else {
        None
    }
}

fn entry_count(entries: &[Yaml], depth: usize) -> Option<usize> {
    if depth > 64 {
        return None;
    }
    let mut count = entries.len();
    for entry in entries.iter().filter_map(children) {
        count += entry_count(entry, depth + 1)?;
    }
    Some(count)
}

fn find_entry<'a>(entries: &'a mut [Yaml], id: &str) -> Option<&'a mut Yaml> {
    for entry in entries.iter_mut().rev() {
        if entry.get("id").and_then(Yaml::as_str) == Some(id) {
            return Some(entry);
        }
        if entry.get("group").and_then(Yaml::as_bool) == Some(true)
            && let Some(children) = entry.get_mut("config").and_then(Yaml::as_sequence_mut)
            && let Some(found) = find_entry(children, id)
        {
            return Some(found);
        }
    }
    None
}

fn apply_patch(entries: &mut Vec<Yaml>, patch: Yaml, source: &Path, diagnostics: &mut Vec<String>) {
    let report = |diagnostics: &mut Vec<String>, reason: &str| {
        diagnostics.push(format!("DeepSeek Harness patch {source:?}: {reason}"));
    };
    let id = patch
        .get("id")
        .and_then(Yaml::as_str)
        .filter(|id| !id.is_empty());
    if let Some(insert) = patch.get("insert") {
        let Some(rows) = insert.as_sequence() else {
            report(diagnostics, "insert must be a list of entries");
            return;
        };
        let Some(id) = id else {
            entries.extend_from_slice(rows);
            return;
        };
        if let Some(group) = find_entry(entries, id)
            && group.get("group").and_then(Yaml::as_bool) == Some(true)
        {
            let config = group
                .as_mapping_mut()
                .expect("group is a mapping")
                .entry(Yaml::String("config".into()))
                .or_insert(Yaml::Null);
            if !config.is_sequence() {
                *config = Yaml::Sequence(Vec::new());
            }
            config
                .as_sequence_mut()
                .expect("group config is a sequence")
                .extend_from_slice(rows);
        } else {
            report(
                diagnostics,
                &format!("insert target {id:?} is not a group in this bundle"),
            );
        }
        return;
    }
    let Some(id) = id else {
        report(diagnostics, "override needs an entry id");
        return;
    };
    let base_module = match id {
        SKILL_FILESYSTEM_ENTRY => Some(SKILL_FILESYSTEM),
        MCP_MANAGER_ENTRY => Some(MCP_MANAGER),
        "workflow-ptc" => Some(super::deepseek_commands::WORKFLOW_PTC),
        "subagent-spawn-in-process" => Some(super::deepseek_commands::SUBAGENT_SPAWN),
        "subagent-fork-in-process" => Some(super::deepseek_commands::SUBAGENT_FORK),
        "subagent" => Some(super::deepseek_commands::SUBAGENT),
        "tool-subagent" | "tool-subagent-fork" => Some(super::deepseek_commands::SUBAGENT_TOOL),
        "tool-subagent-control" => Some(super::deepseek_commands::SUBAGENT_CONTROL_TOOL),
        _ => None,
    };
    if let Some(module) = base_module
        && find_entry(entries, id).is_none()
    {
        // Agena supplies the profile's built-in provider.
        entries.push(Yaml::Mapping(serde_yaml::Mapping::from_iter([
            (Yaml::from("id"), Yaml::from(id)),
            (Yaml::from("name"), Yaml::from(module)),
        ])));
    }
    let Some(entry) = find_entry(entries, id) else {
        report(
            diagnostics,
            &format!("override target {id:?} was not found in this bundle"),
        );
        return;
    };
    if patch
        .get("name")
        .and_then(Yaml::as_str)
        .is_some_and(|name| module_name(entry) != Some(name))
    {
        report(
            diagnostics,
            &format!("override target {id:?} has a different module name"),
        );
        return;
    }
    if let (Some(target), Yaml::Mapping(overrides)) = (entry.as_mapping_mut(), patch) {
        target.extend(
            overrides
                .into_iter()
                .filter(|(key, _)| !matches!(key.as_str(), Some("id" | "name"))),
        );
    }
}

fn collect_rows(
    ctx: &Ctx<'_>,
    manifest: &Map<String, Value>,
    entries: Vec<Yaml>,
    rows: &mut Vec<(String, Yaml)>,
    skipped: &mut BTreeSet<String>,
    diagnostics: &mut Vec<String>,
) {
    for (index, mut entry) in entries.into_iter().enumerate() {
        let group = entry.get("group").and_then(Yaml::as_bool) == Some(true);
        let module = module_name(&entry).map(ToOwned::to_owned);
        let label = entry
            .get("id")
            .and_then(Yaml::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| module.clone())
            .unwrap_or_else(|| format!("entry {}", index + 1));
        let report = |diagnostics: &mut Vec<String>, error: String| {
            diagnostics.push(format!("DeepSeek Harness entry {label:?}: {error}"));
        };
        if let Some(disabled) = entry.get_mut("disabled") {
            if let Err(error) = super::deepseek_js::interpolate_value(ctx, disabled) {
                report(diagnostics, error);
                continue;
            }
            if !matches!(disabled, Yaml::Null | Yaml::Bool(false)) {
                if matches!(
                    module.as_deref(),
                    Some(super::deepseek_commands::SUBAGENT_SPAWN | super::deepseek_commands::SUBAGENT_FORK)
                ) {
                    rows.push((label, entry));
                }
                continue;
            }
        }
        if group {
            if let Some(Yaml::Sequence(children)) = entry
                .as_mapping_mut()
                .and_then(|mapping| mapping.remove("config"))
            {
                collect_rows(ctx, manifest, children, rows, skipped, diagnostics);
            }
            continue;
        }
        match module.as_deref() {
            Some(SKILL_REGISTRY | SKILL_TOOL) | None => continue,
            Some(name)
                if is_native_module(name)
                    || matches!(name, super::deepseek_commands::WORKFLOW_TOOL | super::deepseek_commands::JOBS_TOOL
                        | super::deepseek_commands::SUBAGENT_TOOL | super::deepseek_commands::SUBAGENT_CONTROL_TOOL)
                    || super::deepseek_commands::is_package_module(manifest, name) => {}
            Some(name) => {
                skipped.insert(name.to_owned());
                continue;
            }
        }
        if let Some(config) = entry.get_mut("config") {
            let interpolate = if matches!(
                module.as_deref(),
                Some(
                    super::deepseek_commands::storage::JSON_BACKEND
                        | super::deepseek_commands::storage::SQLITE_BACKEND
                )
            ) {
                super::deepseek_js::interpolate_storage_config
            } else {
                super::deepseek_js::interpolate_value
            };
            if let Err(error) = interpolate(ctx, config) {
                report(diagnostics, error);
                continue;
            }
        }
        rows.push((label, entry));
    }
}

pub(super) fn module_name(row: &Yaml) -> Option<&str> {
    row.get("name").and_then(Yaml::as_str)
}

pub(super) fn skill_roots<'a>(
    rows: &'a [(String, Yaml)],
    diagnostics: &mut Vec<String>,
) -> Vec<&'a str> {
    let mut roots = Vec::new();
    for (id, row) in rows
        .iter()
        .filter(|(_, row)| module_name(row) == Some(SKILL_FILESYSTEM))
    {
        let Some(config) = row.get("config") else {
            continue;
        };
        if let Some(directories) = config.get("customSkillDirs") {
            match directories.as_sequence() {
                Some(paths) if paths.iter().all(|path| path.as_str().is_some()) => {
                    roots.extend(paths.iter().filter_map(Yaml::as_str));
                }
                _ => diagnostics.push(format!(
                    "DeepSeek Harness skill entry {id:?}: customSkillDirs must be a list of paths"
                )),
            }
        }
        if let Some(directory) = config.get("bundledSkillDir") {
            match directory.as_str() {
                Some(path) => roots.push(path),
                None if directory.is_null() => {}
                None => diagnostics.push(format!(
                    "DeepSeek Harness skill entry {id:?}: bundledSkillDir must be a path"
                )),
            }
        }
    }
    roots
}

fn read_patches(root: &Path, path: &Path) -> Result<Vec<Yaml>, String> {
    let bytes =
        read_package_bytes(root, path, MAX_MANIFEST_BYTES)?.ok_or("patch file is missing")?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "Cordis patch must be UTF-8")?;
    let yaml = if text.contains("!js") || text.contains("tag:yaml.org,2002:js") {
        std::borrow::Cow::Owned(super::deepseek_js::normalize_tags(text)?)
    } else {
        std::borrow::Cow::Borrowed(text)
    };
    let mut patches = serde_yaml::from_str::<Vec<Yaml>>(&yaml)
        .map_err(|_| "expected a YAML list of Cordis patches")?;
    let source = Url::from_file_path(root.join(path)).map_err(|()| "invalid patch path")?;
    for patch in &mut patches {
        if let Some(entries) = patch.get_mut("insert").and_then(Yaml::as_sequence_mut) {
            anchor_local_modules(entries, &source)?;
        }
    }
    Ok(patches)
}

fn anchor_local_modules(entries: &mut [Yaml], source: &Url) -> Result<(), String> {
    for entry in entries {
        if let Some(Yaml::String(module)) = entry.get_mut("name")
            && (module.starts_with("./") || module.starts_with("../"))
        {
            *module = source
                .join(module)
                .map_err(|error| error.to_string())?
                .into();
        }
        if entry.get("group").and_then(Yaml::as_bool) == Some(true)
            && let Some(children) = entry.get_mut("config").and_then(Yaml::as_sequence_mut)
        {
            anchor_local_modules(children, source)?;
        }
    }
    Ok(())
}

fn translate_mcp_server(name: &str, config: &Yaml, managed: bool) -> Result<Value, String> {
    if !(1..=32).contains(&name.len())
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err("invalid serverName".into());
    }
    let transport = config
        .get("transport")
        .and_then(Yaml::as_str)
        .ok_or("missing transport")?;
    let fields: &[&str] = match transport {
        "stdio" => &["command", "args", "env", "cwd", "expandTemplates"],
        "streamable-http" => &["url", "headers", "expandTemplates"],
        _ => return Err("unsupported MCP transport".into()),
    };
    if managed {
        // The manager's editable entry has the fields of both transports, with
        // the unused fields normally represented by an empty string or collection.
        let unused: &[&str] = match transport {
            "stdio" => &["url", "headers"],
            _ => &["command", "args", "env", "cwd"],
        };
        for field in unused {
            let Some(value) = config.get(*field) else {
                continue;
            };
            let empty = match *field {
                "args" => value.as_sequence().is_some_and(Vec::is_empty),
                "env" | "headers" => value
                    .as_mapping()
                    .is_some_and(serde_yaml::Mapping::is_empty),
                _ => value.as_str() == Some(""),
            };
            if !empty {
                return Err(format!("{field} is unused with {transport}"));
            }
        }
    }
    let mut server = Map::from_iter([("type".into(), Value::String(transport.to_owned()))]);
    for field in fields
        .iter()
        .chain(&[
            "startupTimeoutMs",
            "toolCallTimeoutMs",
            "closeTimeoutMs",
            "reconnect",
            "toolLoading",
        ])
        .chain((!managed).then_some(&"catalogDescriptionMaxLength"))
    {
        if let Some(value) = config.get(*field) {
            // The native stdio client defaults to the package root.
            if *field == "cwd" && value.as_str() == Some("") {
                continue;
            }
            if !is_literal(value) {
                return Err(format!("{field} needs a literal configuration value"));
            }
            let value = serde_json::to_value(value).map_err(|_| format!("invalid {field}"))?;
            server.insert((*field).into(), value);
        }
    }
    if managed {
        server.insert("expandTemplates".into(), Value::Bool(true));
        server
            .entry("startupTimeoutMs")
            .or_insert_with(|| Value::from(30_000));
        server
            .entry("closeTimeoutMs")
            .or_insert_with(|| Value::from(5_000));
    }
    Ok(Value::Object(server))
}

fn is_literal(value: &Yaml) -> bool {
    match value {
        Yaml::Tagged(_) => false,
        Yaml::Mapping(values) => values
            .iter()
            .all(|(key, value)| is_literal(key) && is_literal(value)),
        Yaml::Sequence(values) => values.iter().all(is_literal),
        _ => true,
    }
}
