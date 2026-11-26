//! Runtime composition for the provider-neutral LSP configuration values.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use agena_plugin_host::PluginsConfig;
use agena_runtime_config::{
    LSP_PLUGIN_ID, LspConfig, LspServerConfig, LspServerDefaultsConfig, LspServerProcessConfig,
    LspServerRoutingConfig, lsp_config_from_plugins,
};

/// Compose the optional LSP registry and its background maintenance guard.
pub fn compose_lsp_services(
    plugins: &PluginsConfig,
    workspace_root: &Path,
    originator: impl Into<String>,
    package_version: impl Into<String>,
) -> Result<
    (
        Option<Arc<agena_lsp::LspRegistry>>,
        Option<Arc<crate::AbortOnDrop>>,
    ),
    String,
> {
    let config = lsp_config_from_plugins(plugins)?;
    let enabled = plugins
        .list
        .get(LSP_PLUGIN_ID)
        .is_some_and(|entry| !entry.disabled());
    if !enabled {
        return Ok((None, None));
    }

    let specs = server_specs(config)?;
    let registry = Arc::new(agena_lsp::LspRegistry::with_servers(
        workspace_root.to_path_buf(),
        originator,
        package_version,
        specs,
    ));
    let maintenance =
        crate::spawn_abortable(agena_lsp::LspRegistry::maintain(Arc::downgrade(&registry)));
    Ok((Some(registry), Some(Arc::new(maintenance))))
}

fn server_specs(config: LspConfig) -> Result<BTreeMap<String, agena_lsp::LspServerSpec>, String> {
    let mut specs = BTreeMap::new();
    for (name, entry) in built_in_servers() {
        if !config.servers.contains_key(name) {
            let mut spec = lsp_runtime_spec(name.to_owned(), &entry, &config.defaults)?;
            spec.built_in = true;
            specs.insert(name.to_owned(), spec);
        }
    }
    for (name, entry) in config.servers {
        if !entry.disabled {
            let spec = lsp_runtime_spec(name.clone(), &entry, &config.defaults)?;
            specs.insert(name, spec);
        }
    }
    Ok(specs)
}

fn lsp_runtime_spec(
    name: String,
    config: &LspServerConfig,
    defaults: &LspServerDefaultsConfig,
) -> Result<agena_lsp::LspServerSpec, String> {
    let invalid = |message| format!("agena.lsp server `{name}`: {message}");
    if name.trim().is_empty() || config.process.command.trim().is_empty() {
        return Err(invalid("server name and command must be nonempty"));
    }
    let mut env = defaults.env.clone();
    env.extend(config.process.env.clone());
    let root_markers = if config.routing.root_markers.is_empty() {
        defaults.root_markers.clone()
    } else {
        config.routing.root_markers.clone()
    };
    if root_markers.iter().any(|marker| {
        marker.is_empty()
            || !Path::new(marker)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
    }) {
        return Err(invalid(
            "root markers must be nonempty project-relative paths",
        ));
    }
    let mut language_ids = std::collections::BTreeMap::new();
    for (extension, language_id) in &config.routing.language_ids {
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        if extension.is_empty()
            || language_id.trim().is_empty()
            || language_ids
                .insert(extension, language_id.trim().to_owned())
                .is_some()
        {
            return Err(invalid(
                "language_ids must have unique nonempty extensions and language identifiers",
            ));
        }
    }
    Ok(agena_lsp::LspServerSpec {
        name,
        built_in: false,
        command: config.process.command.clone(),
        args: config.process.args.clone(),
        env,
        file_extensions: config.routing.file_extensions.clone(),
        root_markers,
        language_ids,
        prefer_outermost_root: config.routing.prefer_outermost_root,
        configuration: config.session.configuration.clone().unwrap_or_default(),
        initialization_options: config
            .session
            .initialization_options
            .clone()
            .or_else(|| defaults.initialization_options.clone()),
    })
}

struct BuiltInServer {
    name: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    extensions: &'static [&'static str],
    root_markers: &'static [&'static str],
    prefer_outermost_root: bool,
}

const BUILT_IN_SERVERS: [BuiltInServer; 5] = [
    BuiltInServer {
        name: "rust-analyzer",
        command: "rust-analyzer",
        args: &[],
        extensions: &["rs"],
        root_markers: &["Cargo.toml", "rust-project.json"],
        prefer_outermost_root: true,
    },
    BuiltInServer {
        name: "gopls",
        command: "gopls",
        args: &["serve"],
        extensions: &["go"],
        root_markers: &["go.work", "go.mod"],
        prefer_outermost_root: true,
    },
    BuiltInServer {
        name: "pyright",
        command: "pyright-langserver",
        args: &["--stdio"],
        extensions: &["py", "pyi"],
        root_markers: &["pyrightconfig.json", "pyproject.toml"],
        prefer_outermost_root: false,
    },
    BuiltInServer {
        name: "typescript",
        command: "typescript-language-server",
        args: &["--stdio"],
        extensions: &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"],
        root_markers: &["tsconfig.json", "jsconfig.json", "package.json"],
        prefer_outermost_root: false,
    },
    BuiltInServer {
        name: "clangd",
        command: "clangd",
        args: &[],
        extensions: &["c", "cc", "cpp", "cxx", "h", "hh", "hpp", "hxx"],
        root_markers: &[".clangd", "compile_commands.json"],
        prefer_outermost_root: false,
    },
];

fn built_in_servers() -> impl Iterator<Item = (&'static str, LspServerConfig)> {
    let strings = |values: &[&str]| values.iter().map(|value| (*value).to_owned()).collect();
    BUILT_IN_SERVERS.iter().map(move |server| {
        (
            server.name,
            LspServerConfig {
                process: LspServerProcessConfig {
                    command: server.command.into(),
                    args: strings(server.args),
                    ..Default::default()
                },
                routing: LspServerRoutingConfig {
                    file_extensions: strings(server.extensions),
                    root_markers: strings(server.root_markers),
                    prefer_outermost_root: server.prefer_outermost_root,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_settings_register_every_builtin_route() {
        let specs = server_specs(LspConfig::default()).unwrap();
        let names: Vec<_> = specs.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            ["clangd", "gopls", "pyright", "rust-analyzer", "typescript"]
        );
        assert!(specs.values().all(|spec| spec.built_in));
        assert_eq!(specs["rust-analyzer"].file_extensions, ["rs"]);
        assert_eq!(specs["pyright"].command, "pyright-langserver");
        assert!(specs["typescript"].handles_extension("tsx"));
    }

    #[test]
    fn configured_servers_can_replace_or_disable_a_builtin() {
        let config: LspConfig = serde_json::from_value(serde_json::json!({
            "defaults": { "env": { "SHARED": "value" } },
            "servers": {
                "gopls": { "disabled": true },
                "rust-analyzer": {
                    "process": { "command": "rust-analyzer-custom" },
                    "routing": { "file_extensions": ["rs"] }
                },
                "lua": {
                    "process": { "command": "lua-language-server" },
                    "routing": { "file_extensions": ["lua"] }
                }
            }
        }))
        .unwrap();
        let specs = server_specs(config).unwrap();
        assert!(!specs.contains_key("gopls"));
        assert!(!specs["rust-analyzer"].built_in);
        assert_eq!(specs["rust-analyzer"].command, "rust-analyzer-custom");
        assert_eq!(specs["lua"].file_extensions, ["lua"]);
        assert_eq!(specs["clangd"].env["SHARED"], "value");
    }
}
