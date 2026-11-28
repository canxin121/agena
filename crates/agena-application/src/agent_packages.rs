//! Shared installation and registration of external agent plugins.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use agena_api::management::marketplace::{
    MarketplaceIdentityResource, MarketplacePluginResource, MarketplaceSearchResponse,
};
use agena_plugin_host::{
    AgentPluginFormat, ConfiguredPlugin, PluginPackage,
    agent_plugin::{
        AgentPluginCompatibility, agent_plugin_identity, load_agent_plugin_for_ecosystem,
        native_installation_path, resolve_agent_plugin_path,
    },
    sdk::{
        PluginKey,
        configuration::{AgentPluginBridgeConfig, AgentPluginRuntime},
    },
};
use agena_plugin_marketplace::{
    MarketplaceError, ReqwestFetcher,
    agent_packages::{
        AgentEcosystem as Ecosystem, AgentPackageInstaller, AgentPackageRequest, AgentPluginSource,
        ClaudeDependencyPlan, ClaudeDependencyVersion, InstalledClaudePackage, npm_selector,
        recover_install_source, split_selector, valid_segment,
    },
};
use agena_runtime_config::{with_config_file_write_lock, write_config_file_atomically};
use serde_json::{Map, Value, json};

mod node_dependencies;

#[derive(Debug, Default)]
pub struct AgentInstallRequest {
    pub spec: String,
    pub registry: Option<String>,
    pub package_source: Option<String>,
    pub force: bool,
    pub dry_run: bool,
    pub refresh: bool,
    pub id: Option<String>,
    pub native_home: Option<PathBuf>,
    pub profile: Option<String>,
    pub installed: bool,
    pub enable_hooks: bool,
    pub environment: Vec<String>,
    pub option_env: BTreeMap<String, String>,
    pub accept_command: Option<String>,
}

pub enum AgentInstallOutcome {
    Preview(AgentInstallPreview),
    Package {
        id: PluginKey,
        format: AgentPluginFormat,
        path: PathBuf,
        version: Option<String>,
        diagnostics: Vec<String>,
    },
}

pub struct AgentInstallPreview {
    pub id: PluginKey,
    pub compatibility: AgentPluginCompatibility,
    pub diagnostics: Vec<String>,
    dependencies: BTreeMap<PluginKey, BTreeMap<PluginKey, Option<String>>>,
}

impl AgentInstallPreview {
    pub fn summary(&self) -> String {
        let format = self.compatibility.format;
        let version = self
            .compatibility
            .version
            .as_ref()
            .map_or_else(String::new, |version| format!(" v{version}"));
        let mut report = format!(
            "Would install {}{version} ({format}).\n{}",
            self.id,
            self.compatibility.component_summary()
        );
        for (plugin, requirements) in &self.dependencies {
            for (required, version) in requirements {
                let version = version
                    .as_ref()
                    .map_or_else(String::new, |version| format!(" {version}"));
                report.push_str(&format!("\n{plugin} requires {required}{version}."));
            }
        }
        report
    }
}

pub struct AgentUpgradeOutcome {
    pub previous_version: Option<String>,
    pub installed_version: Option<String>,
    pub upgraded: bool,
    pub diagnostics: Vec<String>,
}

pub fn is_agent_source(spec: &str) -> bool {
    Ecosystem::parse_selector(spec).is_some()
        || spec == "."
        || spec == ".."
        || ["./", "../", ".\\", "..\\"]
            .iter()
            .any(|prefix| spec.starts_with(prefix))
        || Path::new(spec).is_absolute()
        || Path::new(spec).is_dir()
}

pub fn search(
    config_path: &Path,
    cwd: &Path,
    ecosystem: &str,
    marketplace: Option<&str>,
    query: &str,
) -> Result<MarketplaceSearchResponse, MarketplaceError> {
    let spec = format!("{ecosystem}:");
    let (ecosystem, _) = Ecosystem::parse_selector(&spec)
        .ok_or_else(|| config_error("select codex, claude, or deepseek"))?;
    let format = match ecosystem {
        Ecosystem::Codex => AgentPluginFormat::Codex,
        Ecosystem::Claude => AgentPluginFormat::ClaudeCode,
        Ecosystem::DeepSeek => AgentPluginFormat::DeepSeekHarness,
    };
    let installer = AgentPackageInstaller {
        root: &install_root(config_path),
        fetcher: &ReqwestFetcher::new(),
        command: &run_install_command,
    };
    let catalogue = installer.search(ecosystem, cwd, marketplace, query)?;
    let mut entries = Vec::new();
    for item in catalogue.entries {
        let identity = if ecosystem == Ecosystem::DeepSeek {
            npm_selector(&item.identity)?.0
        } else {
            &item.identity
        };
        let id = default_id(format, identity, Some(&(ecosystem, identity.to_owned())))?;
        entries.push(MarketplacePluginResource {
            plugin_id: id.to_string(),
            install_spec: Some(format!("{}:{}", ecosystem.as_str(), item.identity)),
            install_registry: item.registry,
            package_source: item.package_source,
            name: item.name,
            description: item.description,
            homepage: item.homepage,
            repository: None,
            license: None,
            category: None,
            tags: Vec::new(),
            version_count: item.version_count,
            latest_version: item.version,
            latest_kind: Some("agent_plugin".into()),
            latest_platform: Some("any".into()),
            latest_source_repository: None,
            latest_source_tag: None,
            latest_source_commit: None,
            review_tier: "community".into(),
            featured: false,
        });
    }
    Ok(MarketplaceSearchResponse {
        registry_id: ecosystem.as_str().to_owned(),
        registry_url: catalogue.source,
        marketplace: MarketplaceIdentityResource {
            name: catalogue.name,
            description: catalogue.description,
            homepage: None,
            repository: None,
            owner_name: None,
            owner_url: None,
        },
        entries,
        renames: BTreeMap::new(),
    })
}

pub fn install(
    config_path: &Path,
    cwd: &Path,
    args: AgentInstallRequest,
) -> Result<AgentInstallOutcome, MarketplaceError> {
    install_package(config_path, cwd, args, None)
}

pub fn upgrade(
    config_path: &Path,
    id: &str,
    registry_override: Option<&str>,
) -> Result<Option<AgentUpgradeOutcome>, MarketplaceError> {
    let config = read_config(config_path)?;
    let Some(entry) = agent_entries(&config).and_then(|entries| entries.get(id)) else {
        return Ok(None);
    };
    if !is_registered_agent(&config, id) {
        return Ok(None);
    }
    let PluginPackage::AgentPlugin {
        path,
        source,
        environment,
        user_config_env,
        enable_hooks,
        bridge,
    } = configured_entry(entry)?.package
    else {
        unreachable!("agent package entry was checked");
    };
    if bridge.is_some() {
        return Err(config_error(format!(
            "{id} follows an existing host installation"
        )));
    }
    let cwd = config_path.parent().unwrap_or_else(|| Path::new("."));
    let resolved = cwd.join(&path);
    let (name, previous_version) =
        agent_plugin_identity(&resolved, source.as_ref().map(|source| source.ecosystem))
            .map_err(config_error)?;
    let source = match source {
        Some(source) => Some(*source),
        None => recover_install_source(&install_root(config_path), &resolved, &name)?,
    }
    .ok_or_else(|| {
        config_error(format!(
            "{id} has no recorded install source; install it again from its original source"
        ))
    })?;
    if registry_override.is_some() && source.package_source.is_some() {
        return Err(config_error(
            "the plugin was installed from a package source",
        ));
    }
    let request = AgentInstallRequest {
        spec: format!("{}:{}", source.ecosystem.as_str(), source.selector),
        id: Some(id.to_owned()),
        registry: registry_override.map(str::to_owned).or(source.registry),
        package_source: source.package_source,
        environment,
        option_env: user_config_env,
        enable_hooks,
        ..AgentInstallRequest::default()
    };
    let AgentInstallOutcome::Package {
        version,
        path: new_path,
        diagnostics,
        ..
    } = install_package(config_path, cwd, request, Some(path))?
    else {
        unreachable!("agent upgrade resolves and installs a package");
    };
    Ok(Some(AgentUpgradeOutcome {
        previous_version,
        installed_version: version,
        upgraded: resolved != new_path,
        diagnostics,
    }))
}

fn install_package(
    config_path: &Path,
    cwd: &Path,
    args: AgentInstallRequest,
    previous_path: Option<PathBuf>,
) -> Result<AgentInstallOutcome, MarketplaceError> {
    let spec = Ecosystem::parse_selector(&args.spec);
    validate_install_options(spec, &args)?;
    let override_id = args.id.as_deref().map(valid_key).transpose()?;
    let options = args.option_env;
    let environment: Vec<_> = args
        .environment
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    agena_plugin_host::agent_plugin::validate_agent_plugin_environment(&environment, &options)
        .map_err(config_error)?;
    let mut current = read_config(config_path)?;
    let current_plugins = plugins_mut(&mut current)?;
    let mut prepared = None;
    let mut bridge = None;
    let mut dependency_plan = ClaudeDependencyPlan::default();
    let (path, identity) = match spec {
        Some((ecosystem, selector)) if args.installed => {
            let home = native_home(ecosystem, args.native_home.as_deref(), cwd)?;
            let config_dir = home.canonicalize().map_err(|error| config_error(&error))?;
            let binding = AgentPluginBridgeConfig {
                runtime: installed_runtime(ecosystem, selector, args.profile.as_deref())?,
                config_dir,
                executable: None,
                follow_installation: true,
            };
            let path = native_installation_path(&binding).map_err(config_error)?;
            bridge = Some(Box::new(binding));
            (path, Some((ecosystem, selector.to_owned())))
        }
        Some((ecosystem, selector)) => {
            let install_root = install_root(config_path);
            let fetcher = ReqwestFetcher::new();
            let installer = AgentPackageInstaller {
                root: &install_root,
                fetcher: &fetcher,
                command: &run_install_command,
            };
            let package = installer
                .prepare(AgentPackageRequest {
                    ecosystem,
                    selector,
                    marketplace: args.registry.as_deref(),
                    package_source: args.package_source.as_deref(),
                    cwd,
                    accept_command: args.accept_command.as_deref().filter(|_| !args.dry_run),
                })
                .map_err(|error| config_error(&error))?;
            if ecosystem == Ecosystem::Claude {
                dependency_plan = installer
                    .prepare_claude_dependencies(&package, &|identity| {
                        let key = claude_dependency_id(identity)
                            .map_err(|error| MarketplaceError::Config(error.to_string()))?;
                        let Some(value) = current_plugins.get(&key.to_string()) else {
                            return Ok(None);
                        };
                        let installed = configured_entry(value)
                            .map_err(|error| MarketplaceError::Config(error.to_string()))?;
                        let path = installed_agent_package_path(config_path, &installed.package)
                            .map_err(MarketplaceError::Config)?;
                        Ok(path.map(|path| InstalledClaudePackage {
                            path,
                            enabled: installed.enabled,
                        }))
                    })
                    .map_err(|error| config_error(&error))?;
            }
            let path = package.path.clone();
            let id = package.identity.clone();
            prepared = Some(package);
            (path, Some((ecosystem, id)))
        }
        None => (cwd.join(&args.spec), None),
    };
    let ecosystem = identity.as_ref().map(|(ecosystem, _)| *ecosystem);
    let package =
        load_agent_plugin_for_ecosystem(&path, &environment, ecosystem).map_err(config_error)?;
    if let Some((ecosystem, _)) = &identity {
        let matches = match ecosystem {
            Ecosystem::Codex => matches!(
                package.format,
                AgentPluginFormat::AgentPluginsV1
                    | AgentPluginFormat::Codex
                    | AgentPluginFormat::ClaudeCode
            ),
            Ecosystem::Claude => package.format == AgentPluginFormat::ClaudeCode,
            Ecosystem::DeepSeek => package.format == AgentPluginFormat::DeepSeekHarness,
        };
        if !matches {
            return Err(config_error(format!(
                "the package does not declare the expected {} format",
                ecosystem.as_str()
            )));
        }
    }
    if let Some(prepared) = &prepared
        && prepared.source.package_source.is_some()
        && prepared.source.ecosystem != Ecosystem::DeepSeek
        && package.name != prepared.identity
    {
        return Err(config_error(format!(
            "package manifest name {:?} does not match {:?}",
            package.name, prepared.identity
        )));
    }
    let id = override_id.map_or_else(
        || default_id(package.format, &package.name, identity.as_ref()),
        Ok,
    )?;
    let compatibility = args.dry_run.then(|| package.compatibility());
    let mut diagnostics = package.diagnostics;
    if let Some(prepared) = &prepared {
        diagnostics.extend(prepared.diagnostics.iter().cloned());
    }
    let source = prepared
        .as_ref()
        .map(|package| Box::new(package.source.clone()));
    if let Some(prepared) = &prepared
        && package.format == AgentPluginFormat::ClaudeCode
        && let Err(error) = node_dependencies::prepare(&prepared.path, args.dry_run)
    {
        diagnostics.push(format!("Node.js dependencies: {error}"));
    }
    let version = package.version;
    let preview_path = if args.dry_run {
        prepared
            .as_ref()
            .map(|package| package.destination())
            .transpose()?
    } else {
        None
    };
    let published = if args.dry_run {
        None
    } else {
        prepared
            .map(|package| package.publish())
            .transpose()
            .map_err(|error| config_error(&error))?
    };
    let path = published
        .as_ref()
        .map(|package| package.path.clone())
        .or(preview_path)
        .unwrap_or(package.root);
    let configured = ConfiguredPlugin {
        enabled: package.default_enabled,
        package: PluginPackage::AgentPlugin {
            path: path.clone(),
            environment,
            user_config_env: options,
            enable_hooks: args.enable_hooks,
            bridge,
            source,
        },
        ..ConfiguredPlugin::static_settings(Value::Null)
    };
    let mut registrations = vec![Registration {
        id: id.clone(),
        entry: configured,
        force: args.force,
        version: version.clone(),
        previous_path,
    }];
    let mut publications = Vec::new();
    for dependency in dependency_plan.packages {
        let id = claude_dependency_id(&dependency.identity)?;
        let descriptor =
            load_agent_plugin_for_ecosystem(&dependency.path, &[], Some(Ecosystem::Claude))
                .map_err(config_error)?;
        if descriptor.format != AgentPluginFormat::ClaudeCode {
            return Err(config_error(format!(
                "dependency {id} has the wrong plugin format"
            )));
        }
        diagnostics.extend(
            descriptor
                .diagnostics
                .into_iter()
                .map(|message| format!("{id}: {message}")),
        );
        if let Err(error) = node_dependencies::prepare(&dependency.path, args.dry_run) {
            diagnostics.push(format!("{id} Node.js dependencies: {error}"));
        }
        let source = Some(Box::new(dependency.source.clone()));
        let path = if args.dry_run {
            dependency.destination()?
        } else {
            let published = dependency.publish().map_err(|error| config_error(&error))?;
            let path = published.path.clone();
            publications.push(published);
            path
        };
        registrations.push(Registration {
            id,
            force: false,
            version: descriptor.version,
            previous_path: None,
            entry: ConfiguredPlugin {
                package: PluginPackage::AgentPlugin {
                    path,
                    environment: Vec::new(),
                    user_config_env: BTreeMap::new(),
                    enable_hooks: false,
                    bridge: None,
                    source,
                },
                ..ConfiguredPlugin::static_settings(Value::Null)
            },
        });
    }
    let primary_identity = identity.as_ref().map(|(_, key)| key.as_str());
    let key_for = |identity: &str| {
        if Some(identity) == primary_identity {
            Ok(id.clone())
        } else {
            claude_dependency_id(identity)
        }
    };
    let bindings = dependency_plan
        .bindings
        .into_iter()
        .map(|(consumer, requirements)| {
            let required = requirements
                .into_iter()
                .map(|(provider, version)| key_for(&provider).map(|key| (key, version)))
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            Ok((key_for(&consumer)?, required))
        })
        .collect::<Result<BTreeMap<_, _>, MarketplaceError>>()?;
    let reused = dependency_plan
        .reused
        .into_iter()
        .map(|(identity, path)| key_for(&identity).map(|key| (key, path)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    if let Some(compatibility) = compatibility {
        register_packages(config_path, registrations, bindings.clone(), reused, true)?;
        return Ok(AgentInstallOutcome::Preview(AgentInstallPreview {
            id,
            compatibility,
            diagnostics,
            dependencies: bindings,
        }));
    }
    for dependency in publications {
        dependency
            .save_catalog()
            .map_err(|error| config_error(&error))?;
    }
    if let Some(published) = published {
        published
            .save_catalog()
            .map_err(|error| config_error(&error))?;
    }
    register_packages(config_path, registrations, bindings, reused, false)?;
    Ok(AgentInstallOutcome::Package {
        id,
        format: package.format,
        path,
        version,
        diagnostics,
    })
}

fn install_root(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(|directory| {
            agena_storage_layout::StorageLayout::new(directory.to_path_buf()).marketplace_dir()
        })
        .unwrap_or_else(agena_plugin_marketplace::default_cache_root)
        .join("agent-plugins")
}

fn validate_install_options(
    source: Option<(Ecosystem, &str)>,
    args: &AgentInstallRequest,
) -> Result<(), MarketplaceError> {
    if args.installed {
        if source.is_none()
            || args.registry.is_some()
            || args.package_source.is_some()
            || args.refresh
            || args.accept_command.is_some()
        {
            return Err(config_error(
                "--installed requires an ecosystem selector and reads an existing installation",
            ));
        }
    } else if args.native_home.is_some() || args.profile.is_some() {
        return Err(config_error(
            "--native-home and --profile select an existing installation with --installed",
        ));
    }
    let Some((ecosystem, selector)) = source else {
        if args.registry.is_some()
            || args.package_source.is_some()
            || args.accept_command.is_some()
            || args.refresh
        {
            return Err(config_error(
                "marketplace, package source and installer options require an ecosystem selector",
            ));
        }
        return Ok(());
    };
    if args
        .profile
        .as_deref()
        .is_some_and(|name| ecosystem != Ecosystem::DeepSeek || !valid_segment(name))
    {
        return Err(config_error(
            "--profile requires a valid DeepSeek Harness profile name",
        ));
    }
    if args.accept_command.as_ref().is_some_and(|hash| {
        ecosystem != Ecosystem::Claude
            || hash.len() != 64
            || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(config_error(
            "--accept-command requires a Claude marketplace command SHA-256 hash",
        ));
    }
    if args.package_source.is_some() && (args.registry.is_some() || args.accept_command.is_some()) {
        return Err(config_error(
            "--package-source selects a package independently of marketplace options",
        ));
    }
    match ecosystem {
        Ecosystem::DeepSeek => {
            let (_, version) = npm_selector(selector).map_err(|error| config_error(&error))?;
            if version.is_some() && (args.installed || args.package_source.is_some()) {
                return Err(config_error(
                    "provide a bare npm package name with --installed or --package-source",
                ));
            }
        }
        Ecosystem::Codex | Ecosystem::Claude => {
            let (_, market) = split_selector(selector).map_err(|error| config_error(&error))?;
            if args.installed && market.is_none() {
                return Err(config_error("--installed requires plugin@marketplace"));
            }
            if args.package_source.is_some() && market.is_some() {
                return Err(config_error(
                    "provide a bare plugin name with --package-source",
                ));
            }
        }
    }
    Ok(())
}

fn run_install_command(
    script: &str,
    cwd: &Path,
    timeout: Duration,
) -> Result<PathBuf, MarketplaceError> {
    let mut command = if cfg!(windows) {
        let mut command = tokio::process::Command::new("cmd.exe");
        command.args(["/D", "/S", "/C", script]);
        command
    } else {
        let mut command = tokio::process::Command::new("sh");
        command.args(["-c", script]);
        command
    };
    command.current_dir(cwd).stdin(Stdio::null());
    let output = tokio::runtime::Handle::current().block_on(agena_process::output(
        command,
        timeout,
        64 * 1024,
    ))?;
    if !output.status.success() {
        return Err(MarketplaceError::Config(format!(
            "approved install command exited with {}. {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
        )));
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| {
        MarketplaceError::Config("install command must print a UTF-8 package path".into())
    })?;
    let text = text.trim_end_matches(['\r', '\n']);
    if text.contains(['\r', '\n', '\0']) {
        return Err(MarketplaceError::Config(
            "install command must print exactly one package path".into(),
        ));
    }
    Ok(PathBuf::from(text))
}

fn native_home(
    ecosystem: Ecosystem,
    explicit: Option<&Path>,
    cwd: &Path,
) -> Result<PathBuf, MarketplaceError> {
    let (variable, fallback) = match ecosystem {
        Ecosystem::Codex => ("CODEX_HOME", ".codex"),
        Ecosystem::Claude => ("CLAUDE_CONFIG_DIR", ".claude"),
        Ecosystem::DeepSeek => ("DSH_HOME", ".dsh"),
    };
    let path = explicit
        .map(Path::to_owned)
        .or_else(|| {
            std::env::var_os(variable)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| std::env::home_dir().map(|home| home.join(fallback)))
        .ok_or_else(|| config_error(format!("set --native-home or {variable}")))?;
    Ok(cwd.join(path))
}

fn installed_runtime(
    ecosystem: Ecosystem,
    selector: &str,
    profile: Option<&str>,
) -> Result<AgentPluginRuntime, MarketplaceError> {
    Ok(match ecosystem {
        Ecosystem::Codex => AgentPluginRuntime::Codex {
            plugin: selector.into(),
        },
        Ecosystem::Claude => AgentPluginRuntime::ClaudeCode {
            plugin: Some(selector.into()),
        },
        Ecosystem::DeepSeek => AgentPluginRuntime::DeepseekHarness {
            profile: profile.unwrap_or("headless").into(),
            bundle: Some(selector.into()),
        },
    })
}

fn default_id(
    format: AgentPluginFormat,
    name: &str,
    identity: Option<&(Ecosystem, String)>,
) -> Result<PluginKey, MarketplaceError> {
    let namespace = match format {
        AgentPluginFormat::AgentPluginsV1 => "agent",
        AgentPluginFormat::Codex => "codex",
        AgentPluginFormat::ClaudeCode => "claude",
        AgentPluginFormat::DeepSeekHarness => "deepseek",
    };
    let (namespace, name) = identity.map_or((namespace, name), |(ecosystem, id)| {
        (ecosystem.as_str(), id.as_str())
    });
    let name = name
        .trim_start_matches('@')
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || matches!(char, '-' | '_') {
                char.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    valid_key(&format!("{namespace}.{name}"))
}

fn claude_dependency_id(identity: &str) -> Result<PluginKey, MarketplaceError> {
    default_id(
        AgentPluginFormat::ClaudeCode,
        identity,
        Some(&(Ecosystem::Claude, identity.to_owned())),
    )
}

fn valid_key(value: &str) -> Result<PluginKey, MarketplaceError> {
    let key = value
        .parse::<PluginKey>()
        .map_err(|error| config_error(&error))?;
    if key.namespace().eq_ignore_ascii_case("agena")
        || !valid_segment(key.namespace())
        || !valid_segment(key.name())
    {
        return Err(config_error(
            "choose a third-party plugin ID with letters, digits, hyphens, and underscores",
        ));
    }
    Ok(key)
}

fn config_error(error: impl std::fmt::Display) -> MarketplaceError {
    MarketplaceError::Config(error.to_string())
}

fn read_config(path: &Path) -> Result<Value, MarketplaceError> {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(json!({})),
        Ok(text) => {
            let value: Value = serde_json::from_str(&text).map_err(|error| config_error(&error))?;
            if !value.is_object() {
                return Err(config_error("Agena configuration must be an object"));
            }
            Ok(value)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(config_error(&error)),
    }
}

fn plugins_mut(config: &mut Value) -> Result<&mut Map<String, Value>, MarketplaceError> {
    let root = config
        .as_object_mut()
        .ok_or_else(|| config_error("Agena configuration must be an object"))?;
    let plugins = root.entry("plugins").or_insert_with(|| json!({}));
    let list = plugins
        .as_object_mut()
        .ok_or_else(|| config_error("plugins must be an object"))?
        .entry("list")
        .or_insert_with(|| json!({}));
    list.as_object_mut()
        .ok_or_else(|| config_error("plugins.list must be an object"))
}

fn write_config(path: &Path, config: &Value) -> Result<(), MarketplaceError> {
    let mut bytes = serde_json::to_vec_pretty(config).map_err(|error| config_error(&error))?;
    bytes.push(b'\n');
    write_config_file_atomically(path, &bytes).map_err(|error| config_error(&error))
}

fn config_transaction<T>(
    path: &Path,
    operation: impl FnOnce() -> Result<T, MarketplaceError>,
) -> Result<T, MarketplaceError> {
    let lock_path = path.with_extension("plugin-install.lock");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| config_error(&error))?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|error| config_error(&error))?;
    lock.lock().map_err(|error| config_error(&error))?;
    with_config_file_write_lock(operation)
}

struct Registration {
    id: PluginKey,
    entry: ConfiguredPlugin,
    force: bool,
    version: Option<String>,
    previous_path: Option<PathBuf>,
}

fn register_packages(
    path: &Path,
    registrations: Vec<Registration>,
    bindings: BTreeMap<PluginKey, BTreeMap<PluginKey, Option<String>>>,
    reused: BTreeMap<PluginKey, PathBuf>,
    dry_run: bool,
) -> Result<(), MarketplaceError> {
    let write = move || {
        let mut config = read_config(path)?;
        let entries = plugins_mut(&mut config)?;
        let mut versions = BTreeMap::new();
        for Registration {
            id,
            entry: mut new,
            force,
            version,
            previous_path,
        } in registrations
        {
            if versions.insert(id.clone(), version).is_some() {
                return Err(config_error(format!(
                    "multiple packages resolve to plugin ID {id}"
                )));
            }
            if let Some(expected) = &previous_path {
                let current = entries
                    .get(&id.to_string())
                    .and_then(|entry| entry.pointer("/package/path"));
                if current != Some(&json!(expected)) {
                    return Err(config_error(format!(
                        "{id} changed while it was being upgraded"
                    )));
                }
            }
            if let Some(previous) = entries.get(&id.to_string()) {
                let previous = configured_entry(previous)?;
                if previous_path.is_some() || same_source(path, &previous.package, &new.package) {
                    merge_options(&previous.package, &mut new.package);
                    new = ConfiguredPlugin {
                        package: new.package,
                        ..previous
                    };
                } else if !force {
                    return Err(config_error(format!(
                        "{id} already exists; select --id or use --force"
                    )));
                }
            }
            write_entry(entries, &id, new)?;
        }
        for (id, expected) in reused {
            let entry = entries
                .get(&id.to_string())
                .map(configured_entry)
                .transpose()?
                .ok_or_else(|| {
                    config_error(format!("dependency {id} changed during installation"))
                })?;
            let valid = installed_agent_package_path(path, &entry.package)
                .is_ok_and(|current| current.as_ref() == Some(&expected));
            if !valid || !entry.enabled {
                return Err(config_error(format!(
                    "dependency {id} changed during installation"
                )));
            }
        }
        for (consumer, requires) in bindings {
            let current = entries.get(&consumer.to_string()).ok_or_else(|| {
                config_error(format!("plugin {consumer} changed during installation"))
            })?;
            let mut entry = configured_entry(current)?;
            entry.activation.imported_requires = requires;
            write_entry(entries, &consumer, entry)?;
        }
        for (consumer, entry) in entries.iter() {
            if entry.pointer("/activation/imported_requires").is_none() {
                continue;
            }
            let entry = configured_entry(entry)?;
            if !entry.enabled {
                continue;
            }
            for (provider, range) in &entry.activation.imported_requires {
                if let Some(version) = versions.get(provider) {
                    if let Some(range) = range
                        && !version.as_deref().is_some_and(|version| {
                            ClaudeDependencyVersion::parse(range)
                                .is_ok_and(|range| range.matches(version))
                        })
                    {
                        return Err(config_error(format!(
                            "{consumer} requires {provider} {range}; installation would select {}",
                            version.as_deref().unwrap_or("an unspecified version")
                        )));
                    }
                    if entries[&provider.to_string()].get("enabled") == Some(&Value::Bool(false)) {
                        return Err(config_error(format!(
                            "{consumer} requires enabled dependency {provider}"
                        )));
                    }
                }
            }
        }
        if dry_run {
            Ok(())
        } else {
            write_config(path, &config)
        }
    };
    if dry_run {
        write()
    } else {
        config_transaction(path, write)
    }
}

fn configured_entry(value: &Value) -> Result<ConfiguredPlugin, MarketplaceError> {
    serde_json::from_value(value.clone()).map_err(|error| config_error(&error))
}

fn installed_agent_package_path(
    config_path: &Path,
    package: &PluginPackage,
) -> Result<Option<PathBuf>, String> {
    let PluginPackage::AgentPlugin { path, bridge, .. } = package else {
        return Ok(None);
    };
    let path = config_path.parent().unwrap_or(Path::new(".")).join(path);
    resolve_agent_plugin_path(&path, bridge.as_deref()).map(Some)
}

fn write_entry(
    entries: &mut Map<String, Value>,
    id: &PluginKey,
    value: ConfiguredPlugin,
) -> Result<(), MarketplaceError> {
    entries.insert(
        id.to_string(),
        serde_json::to_value(value).map_err(|error| config_error(&error))?,
    );
    Ok(())
}

fn same_source(config_path: &Path, previous: &PluginPackage, new: &PluginPackage) -> bool {
    match (previous, new) {
        (
            PluginPackage::AgentPlugin {
                path: left,
                bridge: old,
                ..
            },
            PluginPackage::AgentPlugin {
                path: right,
                bridge: new,
                ..
            },
        ) if old == new => {
            left == right
                || (left.parent() == right.parent()
                    && left.starts_with(install_root(config_path).join("packages")))
                || old
                    .as_deref()
                    .is_some_and(|bridge| bridge.follow_installation)
        }
        _ => false,
    }
}

fn merge_options(previous: &PluginPackage, new: &mut PluginPackage) {
    if let (
        PluginPackage::AgentPlugin {
            environment: old_env,
            user_config_env: old_mapping,
            enable_hooks: old_hooks,
            ..
        },
        PluginPackage::AgentPlugin {
            environment,
            user_config_env,
            enable_hooks,
            ..
        },
    ) = (previous, new)
    {
        *environment = old_env
            .iter()
            .chain(environment.iter())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut mapping = old_mapping.clone();
        mapping.append(user_config_env);
        *user_config_env = mapping;
        *enable_hooks |= *old_hooks;
    }
}

pub fn uninstall(
    path: &Path,
    plugin_id: &str,
    cascade: bool,
) -> Result<Vec<String>, MarketplaceError> {
    let config = read_config(path)?;
    if !is_registered_agent(&config, plugin_id) {
        return Ok(Vec::new());
    }
    config_transaction(path, || {
        let mut config = read_config(path)?;
        if !is_registered_agent(&config, plugin_id) {
            return Ok(Vec::new());
        }
        let entries = plugins_mut(&mut config)?;
        let mut removed = BTreeSet::from([plugin_id.to_owned()]);
        loop {
            let mut dependents = Vec::new();
            for (id, entry) in entries.iter() {
                if removed.contains(id)
                    || entry.pointer("/package/kind").and_then(Value::as_str)
                        != Some("agent_plugin")
                {
                    continue;
                }
                if entry
                    .pointer("/activation/imported_requires")
                    .and_then(Value::as_object)
                    .is_some_and(|requires| {
                        requires.keys().any(|provider| removed.contains(provider))
                    })
                {
                    dependents.push(id.clone());
                }
            }
            if dependents.is_empty() {
                break;
            }
            if !cascade {
                return Err(config_error(format!(
                    "{plugin_id} is required by {}. Use --cascade to remove its dependents.",
                    dependents.join(", ")
                )));
            }
            removed.extend(dependents);
        }
        for id in &removed {
            entries.remove(id);
        }
        write_config(path, &config)?;
        Ok(removed.into_iter().collect())
    })
}

fn is_registered_agent(config: &Value, plugin_id: &str) -> bool {
    agent_entries(config)
        .and_then(|entries| entries.get(plugin_id))
        .and_then(|entry| entry.pointer("/package/kind"))
        .and_then(Value::as_str)
        == Some("agent_plugin")
}

fn agent_entries(config: &Value) -> Option<&Map<String, Value>> {
    config.pointer("/plugins/list").and_then(Value::as_object)
}

pub struct RegisteredAgentPackage {
    pub id: String,
    pub root: PathBuf,
    pub version: Option<String>,
    pub source: Option<AgentPluginSource>,
}

pub fn list(path: &Path) -> Result<Vec<RegisteredAgentPackage>, MarketplaceError> {
    let config = read_config(path)?;
    let Some(entries) = agent_entries(&config) else {
        return Ok(Vec::new());
    };
    Ok(entries
        .iter()
        .filter_map(|(id, entry)| {
            if !is_registered_agent(&config, id) {
                return None;
            }
            let entry = serde_json::from_value::<ConfiguredPlugin>(entry.clone()).ok()?;
            let PluginPackage::AgentPlugin {
                path: root,
                bridge,
                source,
                ..
            } = entry.package
            else {
                return None;
            };
            let root = path.parent().unwrap_or(Path::new(".")).join(root);
            let root = resolve_agent_plugin_path(&root, bridge.as_deref()).unwrap_or(root);
            let identity =
                agent_plugin_identity(&root, source.as_ref().map(|source| source.ecosystem)).ok();
            let source = source.map(|source| *source).or_else(|| {
                let (name, _) = identity.as_ref()?;
                recover_install_source(&install_root(path), &root, name)
                    .ok()
                    .flatten()
            });
            Some(RegisteredAgentPackage {
                id: id.clone(),
                root,
                version: identity.and_then(|(_, version)| version),
                source,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marketplace_installs_claude_components_from_codex_and_upgrades_the_same_registration() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("instance/agena.json");
        let market = temp.path().join("catalogue");
        let package = market.join("plugin");
        fs::create_dir_all(market.join(".agents/plugins")).unwrap();
        fs::create_dir_all(package.join(".claude-plugin")).unwrap();
        fs::write(
            market.join(".agents/plugins/marketplace.json"),
            r#"{"name":"team","plugins":[{"name":"review","source":"./plugin","defaultEnabled":false}]}"#,
        ).unwrap();
        let manifest = package.join(".claude-plugin/plugin.json");
        let write_version = |version| {
            fs::write(
                &manifest,
                json!({
                    "name": "review",
                    "version": version,
                    "mcpServers": {"review": {"command": "review-server"}}
                })
                .to_string(),
            )
            .unwrap();
        };
        write_version("1.0.0");
        let AgentInstallOutcome::Package {
            id, path, format, ..
        } = install(
            &config,
            temp.path(),
            AgentInstallRequest {
                spec: "codex:review".into(),
                registry: Some(market.to_string_lossy().into_owned()),
                ..AgentInstallRequest::default()
            },
        )
        .unwrap()
        else {
            panic!("expected an installed package");
        };
        assert_eq!(format, AgentPluginFormat::ClaudeCode);
        assert!(!path.join(".codex-plugin/plugin.json").exists());
        assert_eq!(
            load_agent_plugin_for_ecosystem(&path, &[], Some(Ecosystem::Codex))
                .unwrap()
                .mcp_servers[0]
                .name,
            "review"
        );
        let mut current = read_config(&config).unwrap();
        let id = id.to_string();
        assert_eq!(current["plugins"]["list"][&id]["enabled"], false);
        current["plugins"]["list"][&id]["settings"] = json!({"theme":"custom"});
        // An install from the preceding version stored only the immutable path.
        current["plugins"]["list"][&id]["package"]
            .as_object_mut()
            .unwrap()
            .remove("source");
        write_config(&config, &current).unwrap();

        write_version("2.0.0");
        let updated = upgrade(&config, &id, None).unwrap().unwrap();
        assert!(updated.upgraded);
        assert_eq!(updated.previous_version.as_deref(), Some("1.0.0"));
        assert_eq!(updated.installed_version.as_deref(), Some("2.0.0"));
        assert!(!upgrade(&config, &id, None).unwrap().unwrap().upgraded);
        let current = read_config(&config).unwrap();
        let entry = &current["plugins"]["list"][&id];
        assert_eq!(entry["enabled"], false);
        assert_eq!(entry["settings"], json!({"theme":"custom"}));
        assert_eq!(entry["package"]["source"]["selector"], "review@team");
    }

    #[test]
    fn reinstall_follows_native_versions_and_keeps_agena_settings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agena.json");
        let id = valid_key("codex.review-team").unwrap();
        let plugin = |version, env, enable_hooks| ConfiguredPlugin {
            package: PluginPackage::AgentPlugin {
                path: temp.path().join(version),
                bridge: Some(Box::new(AgentPluginBridgeConfig {
                    runtime: AgentPluginRuntime::Codex {
                        plugin: "review@team".into(),
                    },
                    config_dir: temp.path().into(),
                    executable: None,
                    follow_installation: true,
                })),
                source: None,
                environment: vec![env],
                user_config_env: BTreeMap::new(),
                enable_hooks,
            },
            ..ConfiguredPlugin::static_settings(Value::Null)
        };
        let mut initial = plugin("1.0.0", "API_TOKEN".into(), true);
        initial.enabled = false;
        initial.settings = json!({"custom": true});
        let register = |entry, dry_run| {
            register_packages(
                &path,
                vec![Registration {
                    id: id.clone(),
                    entry,
                    force: false,
                    version: None,
                    previous_path: None,
                }],
                BTreeMap::new(),
                BTreeMap::new(),
                dry_run,
            )
            .unwrap();
        };
        register(initial.clone(), true);
        assert!(!path.exists());
        assert!(!path.with_extension("plugin-install.lock").exists());
        register(initial, false);
        register(plugin("2.0.0", "EXTRA_ENV".into(), false), false);
        let config = read_config(&path).unwrap();
        let stored = &config["plugins"]["list"][id.to_string()];
        assert_eq!(stored["settings"]["custom"], true);
        assert_eq!(stored["enabled"], false);
        assert_eq!(stored["package"]["enable_hooks"], true);
        assert_eq!(
            stored["package"]["environment"],
            json!(["API_TOKEN", "EXTRA_ENV"])
        );
        assert_eq!(
            stored["package"]["path"],
            temp.path().join("2.0.0").to_str().unwrap()
        );
        assert_eq!(
            uninstall(&path, &id.to_string(), false).unwrap(),
            [id.to_string()]
        );
        assert!(list(&path).unwrap().is_empty());
    }

    #[test]
    fn conflicted_dependency_registration_preserves_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agena.json");
        let consumer = valid_key("claude.consumer-team").unwrap();
        let dependency = valid_key("claude.shared-team").unwrap();
        let another = valid_key("claude.another-team").unwrap();
        let registration = |id: &PluginKey, version: &str| Registration {
            id: id.clone(),
            entry: ConfiguredPlugin {
                package: PluginPackage::AgentPlugin {
                    path: temp.path().join(id.to_string()).join(version),
                    bridge: None,
                    source: None,
                    environment: Vec::new(),
                    user_config_env: BTreeMap::new(),
                    enable_hooks: false,
                },
                ..ConfiguredPlugin::static_settings(Value::Null)
            },
            version: Some(version.into()),
            previous_path: None,
            force: true,
        };
        register_packages(
            &path,
            vec![
                registration(&consumer, "1.0.0"),
                registration(&dependency, "1.2.0"),
            ],
            BTreeMap::from([(
                consumer.clone(),
                BTreeMap::from([(dependency.clone(), Some("^1.0".into()))]),
            )]),
            BTreeMap::new(),
            false,
        )
        .unwrap();
        let original = fs::read(&path).unwrap();
        let failure = register_packages(
            &path,
            vec![
                registration(&another, "1.0.0"),
                registration(&dependency, "2.0.0"),
            ],
            BTreeMap::new(),
            BTreeMap::new(),
            false,
        )
        .unwrap_err();
        assert!(
            failure
                .to_string()
                .contains("claude.consumer-team requires claude.shared-team ^1.0")
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}
