//! Agent Skills management tools. The same manager serves the settings clients;
//! tool calls additionally authorize their concrete filesystem and network effects.

use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock},
};

use agena_domain::PathKind;
use agena_plugin_host::sdk::{
    HookSubscription, HostClient, InitContext, InitOutcome, Plugin, PluginError, PluginManifest,
    Result as SdkResult, ToolContract, ToolDefinition, ToolDocs, ToolInvokeInput, ToolInvokeOutput,
    ToolRuntimePolicy, ToolStreamingMode, ToolTag, macro_support,
};
use agena_skills::source::{
    SkillCreateRequest, SkillDeleteResult, SkillInstallRequest, SkillResourceWriteRequest,
    SkillSourceAccess, SkillValidationRequest, load_install_source,
};
use agena_skills::{DeclaredSkill, SkillError, SkillManager, SkillScope};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

pub(crate) const SKILLS_PLUGIN_ID: &str = "agena.skills";

#[derive(Default)]
pub(crate) struct SkillsPlugin {
    workspace: OnceLock<PathBuf>,
    initial_host: OnceLock<Arc<dyn HostClient>>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct ListInput {
    query: Option<String>,
    scope: Option<SkillScope>,
    /// Include disabled, invalid, and shadowed entries for management.
    include_inactive: bool,
    offset: usize,
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct IdInput {
    id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateInput {
    id: String,
    document: String,
    expected_revision: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveInput {
    id: String,
    expected_revision: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EnabledInput {
    id: String,
    enabled: bool,
    expected_revision: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ResourceReadInput {
    id: String,
    path: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ResourceWriteInput {
    id: String,
    path: String,
    content: String,
    #[serde(default)]
    expected_revision: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ResourceRemoveInput {
    id: String,
    path: String,
    expected_revision: String,
}

fn definition<T: JsonSchema>(
    name: &str,
    summary: &str,
    help: &str,
    tags: &[ToolTag],
) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        contract: ToolContract {
            input_schema: macro_support::json_schema_for::<T>(),
            output_schema: Value::Null,
        },
        docs: ToolDocs {
            summary: Some(summary.into()),
            help: Some(help.into()),
            ..Default::default()
        },
        runtime: ToolRuntimePolicy {
            streaming: ToolStreamingMode::Buffered,
        },
        tags: tags.to_vec(),
    }
}

#[async_trait::async_trait]
impl Plugin for SkillsPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("agena", "skills", env!("CARGO_PKG_VERSION"));
        manifest.summary =
            Some("Discover, install, author, and manage complete Agent Skills packages.".into());
        manifest.hooks = HookSubscription::INIT | HookSubscription::TOOL_INVOKE;
        let read = &[ToolTag::Query, ToolTag::Discovery, ToolTag::ReadOnly];
        let write = &[ToolTag::Mutate, ToolTag::Filesystem];
        manifest.tools = vec![
            definition::<ListInput>(
                "list",
                "Find available skills or inspect the skill library.",
                "Search by name or description. By default return enabled, valid, effective entries. Set include_inactive to inspect disabled, invalid, and shadowed packages. IDs identify exact package copies; scope is global or workspace. Use get to load instructions only when relevant.",
                read,
            ),
            definition::<IdInput>(
                "get",
                "Read a skill's full instructions and resource inventory.",
                "Use an ID returned by list. Relative references resolve against the skill directory shown in path. Supporting resources are loaded separately with read_resource. Read-only plugin and compatibility packages can be installed into a managed scope for editing. allowed-tools is metadata and does not change existing tool permissions.",
                read,
            ),
            definition::<SkillCreateRequest>(
                "create",
                "Create a skill in the workspace or global library.",
                "Provide a complete SKILL.md with YAML name and description plus Markdown instructions. Validates the Agent Skills specification and refuses to replace an existing package. Add scripts, references, or assets with write_resource or install a complete package.",
                write,
            ),
            definition::<UpdateInput>(
                "update",
                "Edit an installed SKILL.md without discarding bundled resources.",
                "Read get first and supply its current revision. Preserve the directory-matching name and any metadata you want to keep. A stale revision fails without changing the package. Resource files remain in place.",
                write,
            ),
            definition::<RemoveInput>(
                "remove",
                "Remove a managed skill and all of its bundled files.",
                "Use the exact ID and current revision from get. Removes only that workspace or global package; other copies and read-only sources remain. Disable with set_enabled when you want to keep the package installed.",
                write,
            ),
            definition::<EnabledInput>(
                "set_enabled",
                "Enable or disable an installed skill.",
                "Use its current revision. Disabled packages remain available for management but are excluded from automatic skill discovery. Project packages override global copies; the library reports shadowed copies explicitly.",
                write,
            ),
            definition::<ResourceReadInput>(
                "read_resource",
                "Read a skill's UTF-8 reference, script, or other text resource.",
                "The path is relative to the skill directory. Use get for SKILL.md. Binary assets are retained by package installation; this tool returns text resources only.",
                read,
            ),
            definition::<ResourceWriteInput>(
                "write_resource",
                "Create or update a skill's UTF-8 resource.",
                "Use a relative path such as references/guide.md or scripts/check.py. Omit expected_revision only for a new file; use read_resource's revision to replace an existing file. Edit SKILL.md with update. This operation never executes scripts.",
                write,
            ),
            definition::<ResourceRemoveInput>(
                "remove_resource",
                "Delete one resource from a managed skill.",
                "Supply the current package revision from get, or a text resource revision from read_resource. Package revisions also support binary and large resources without reading their contents. SKILL.md and installation metadata cannot be removed through this operation; use remove for the complete skill.",
                write,
            ),
            definition::<SkillValidationRequest>(
                "validate",
                "Validate a SKILL.md document before saving or installing.",
                "Checks complete YAML frontmatter, required and optional standard fields, and name rules. Supply name to check the containing directory name. Returns actionable errors and warnings; no files are changed.",
                &[ToolTag::Query, ToolTag::ReadOnly],
            ),
            definition::<SkillInstallRequest>(
                "preview_install",
                "Preview skills in a GitHub repository, archive, or folder.",
                "Sources: github URL (optional reference/subpath), folder path, archive path/URL/uploaded base64, or uploaded files. Supports ZIP, tar, and tar.gz. Preview lists candidates and name conflicts and returns a source revision. It does not write the library or run bundled code.",
                read,
            ),
            definition::<SkillInstallRequest>(
                "install",
                "Install complete skill packages into the workspace or global library.",
                "Preview first, then send selected candidate paths and expected_source_revision. Explicit replace=true is required for existing names. An empty selection installs all valid candidates. All resources and executable bits are preserved; installation never executes code. Global packages are available across projects.",
                write,
            ),
        ];
        manifest
    }

    async fn init(
        &self,
        context: InitContext,
        host: Arc<dyn HostClient>,
    ) -> SdkResult<InitOutcome> {
        self.workspace
            .set(context.workspace_root)
            .map_err(|_| PluginError::internal("skills plugin initialized twice"))?;
        self.initial_host
            .set(host)
            .map_err(|_| PluginError::internal("skills permission host initialized twice"))?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let value = input.input;
        match input.tool_name.as_str() {
            "list" => {
                let input: ListInput = parse(value)?;
                let catalog = self.run(|manager| manager.list()).await?;
                let query = input.query.unwrap_or_default().to_lowercase();
                let mut skills = catalog
                    .skills
                    .into_iter()
                    .filter(|skill| {
                        input.scope.is_none_or(|scope| scope == skill.scope)
                            && (input.include_inactive
                                || (skill.enabled && skill.valid && !skill.shadowed))
                            && (query.is_empty()
                                || format!("{} {}", skill.name, skill.description)
                                    .to_lowercase()
                                    .contains(&query))
                    })
                    .collect::<Vec<_>>();
                let total = skills.len();
                let limit = input.limit.unwrap_or(100).clamp(1, 200);
                let skills = skills
                    .drain(..)
                    .skip(input.offset)
                    .take(limit)
                    .collect::<Vec<_>>();
                output(
                    "Skill library",
                    json!({"skills":skills,"total":total,"offset":input.offset,"limit":limit,"diagnostics":catalog.diagnostics,"roots":catalog.roots}),
                    false,
                )
            }
            "get" => {
                let input: IdInput = parse(value)?;
                self.authorize_id(&input.id, false).await?;
                let detail = self.run(move |manager| manager.get(&input.id)).await?;
                let directory = detail
                    .summary
                    .path
                    .strip_suffix("/SKILL.md")
                    .unwrap_or(&detail.summary.path);
                let text = format!(
                    "Skill directory: {directory}\nResolve relative references against this directory.\n\n{}\n\nResources:\n{}",
                    detail.document,
                    detail
                        .files
                        .iter()
                        .map(|file| format!("- {} ({} bytes)", file.path, file.size))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                Ok(ToolInvokeOutput::from_parts(
                    format!("Skill: {}", detail.summary.name),
                    "Skill instructions",
                    text,
                    Some(
                        serde_json::to_value(detail)
                            .map_err(|error| PluginError::internal_error(&error))?,
                    ),
                    BTreeMap::new(),
                    Vec::new(),
                ))
            }
            "create" => {
                let input: SkillCreateRequest = parse(value)?;
                let validation = agena_skills::validate_document(&input.document, None);
                if !validation.valid {
                    return Err(PluginError::invalid_params(validation.errors.join("\n")));
                }
                let name = validation
                    .metadata
                    .ok_or_else(|| PluginError::invalid_params("Skill metadata is required."))?
                    .name;
                let library = self.manager()?.managed_root(input.scope);
                let root = library.join(name);
                self.require_path(&root, PathKind::Read).await?;
                self.require_path(&root.join("SKILL.md"), PathKind::Read)
                    .await?;
                self.require_path(&root.join(".agena-skill.json"), PathKind::Read)
                    .await?;
                self.require_path(&library, PathKind::Write).await?;
                self.require_path(&root, PathKind::Write).await?;
                self.require_path(&root.join("SKILL.md"), PathKind::Write)
                    .await?;
                self.require_path(&root.join(".agena-skill.json"), PathKind::Write)
                    .await?;
                output(
                    "Skill created",
                    self.run(move |manager| manager.create(input.scope, &input.document))
                        .await?,
                    true,
                )
            }
            "update" => {
                let input: UpdateInput = parse(value)?;
                revision(&input.expected_revision)?;
                self.authorize_id(&input.id, true).await?;
                output(
                    "Skill updated",
                    self.run(move |manager| {
                        manager.update(&input.id, &input.document, Some(&input.expected_revision))
                    })
                    .await?,
                    true,
                )
            }
            "remove" => {
                let input: RemoveInput = parse(value)?;
                revision(&input.expected_revision)?;
                let root = self.authorize_id(&input.id, true).await?;
                if let Some(library) = root.parent().and_then(Path::parent) {
                    self.require_path(library, PathKind::Write).await?;
                }
                self.authorize_tree_write(
                    root.parent()
                        .ok_or_else(|| PluginError::invalid_params("Invalid package path"))?,
                )
                .await?;
                let skill = self
                    .run(move |manager| manager.remove(&input.id, Some(&input.expected_revision)))
                    .await?;
                output(
                    "Skill removed",
                    SkillDeleteResult {
                        id: skill.id,
                        removed: true,
                    },
                    true,
                )
            }
            "set_enabled" => {
                let input: EnabledInput = parse(value)?;
                revision(&input.expected_revision)?;
                self.authorize_id(&input.id, true).await?;
                output(
                    "Skill availability updated",
                    self.run(move |manager| {
                        manager.set_enabled(
                            &input.id,
                            input.enabled,
                            Some(&input.expected_revision),
                        )
                    })
                    .await?,
                    true,
                )
            }
            "read_resource" => {
                let input: ResourceReadInput = parse(value)?;
                let document = self.authorize_id(&input.id, false).await?;
                let resource = resource_path(&document, &input.path)?;
                self.require_path(&resource, PathKind::Read).await?;
                output(
                    "Skill resource",
                    self.run(move |manager| manager.read_resource(&input.id, &input.path))
                        .await?,
                    false,
                )
            }
            "write_resource" => {
                let input: ResourceWriteInput = parse(value)?;
                let document = self.authorize_id(&input.id, true).await?;
                let resource = resource_path(&document, &input.path)?;
                self.require_path(&resource, PathKind::Read).await?;
                self.require_path(&resource, PathKind::Write).await?;
                let request = SkillResourceWriteRequest {
                    path: input.path,
                    content: input.content,
                    expected_revision: input.expected_revision,
                };
                output(
                    "Skill resource saved",
                    self.run(move |manager| {
                        manager.write_resource(
                            &input.id,
                            &request.path,
                            &request.content,
                            Some(request.expected_revision.as_deref().unwrap_or("")),
                        )
                    })
                    .await?,
                    true,
                )
            }
            "remove_resource" => {
                let input: ResourceRemoveInput = parse(value)?;
                revision(&input.expected_revision)?;
                let document = self.authorize_id(&input.id, true).await?;
                let resource = resource_path(&document, &input.path)?;
                self.require_path(&resource, PathKind::Read).await?;
                self.require_path(&resource, PathKind::Write).await?;
                output(
                    "Skill resource removed",
                    self.run(move |manager| {
                        manager.remove_resource(
                            &input.id,
                            &input.path,
                            Some(&input.expected_revision),
                        )
                    })
                    .await?,
                    true,
                )
            }
            "validate" => {
                let input: SkillValidationRequest = parse(value)?;
                output(
                    "Skill validation",
                    agena_skills::validate_document(&input.document, input.name.as_deref()),
                    false,
                )
            }
            "preview_install" | "install" => {
                let preview_only = input.tool_name == "preview_install";
                let request: SkillInstallRequest = parse(value)?;
                let host = self.permission_host()?;
                let loaded = load_install_source(
                    request.source,
                    self.workspace_root()?,
                    move |access| {
                        let host = host.clone();
                        async move {
                            match access {
                                SkillSourceAccess::Read(path) => {
                                    crate::permissions::require_path(
                                        host.as_ref(),
                                        PathKind::Read,
                                        &path.to_string_lossy(),
                                    )
                                    .await
                                }
                                SkillSourceAccess::Network(url) => {
                                    host.require_network_permission(url).await
                                }
                            }
                        }
                    },
                    skill_error,
                )
                .await?;
                loaded
                    .verify_revision(request.expected_source_revision.as_deref())
                    .map_err(skill_error)?;
                let (preview, loaded) = self
                    .run(move |manager| {
                        manager
                            .preview_install(&loaded.package, request.scope)
                            .map(|preview| (preview, loaded))
                    })
                    .await?;
                if preview_only {
                    return output("Skill installation preview", preview, false);
                }
                let root = self.manager()?.managed_root(request.scope);
                self.require_path(&root, PathKind::Write).await?;
                for candidate in &preview.candidates {
                    let selected = if request.selected.is_empty() {
                        candidate.valid
                    } else {
                        request.selected.contains(&candidate.path)
                    };
                    if !selected || !candidate.valid {
                        continue;
                    }
                    let target = root.join(&candidate.name);
                    self.require_path(&target, PathKind::Read).await?;
                    self.require_path(&target.join("SKILL.md"), PathKind::Read)
                        .await?;
                    self.require_path(&target.join(".agena-skill.json"), PathKind::Read)
                        .await?;
                    self.require_path(&target, PathKind::Write).await?;
                    self.require_path(&target.join(".agena-skill.json"), PathKind::Write)
                        .await?;
                    if target.exists() {
                        self.require_path(&target, PathKind::Read).await?;
                        self.authorize_tree_write(&target).await?;
                    }
                    for file in loaded.package.files() {
                        let relative = if candidate.path == "." {
                            Some(file.path.as_str())
                        } else {
                            file.path.strip_prefix(&format!("{}/", candidate.path))
                        };
                        if let Some(relative) = relative {
                            self.require_path(&target.join(relative), PathKind::Write)
                                .await?;
                        }
                    }
                }
                output(
                    "Skills installed",
                    self.run(move |manager| {
                        manager.install(
                            &loaded.package,
                            request.scope,
                            &request.selected,
                            request.replace,
                            Some(loaded.source),
                        )
                    })
                    .await?,
                    true,
                )
            }
            name => Err(PluginError::not_implemented(format!(
                "Unknown skills operation: {name}"
            ))),
        }
    }
}

impl SkillsPlugin {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn workspace_root(&self) -> SdkResult<PathBuf> {
        agena_plugin_host::sdk::host_api::current_host_callback_context()
            .and_then(|context| context.workspace_root)
            .filter(|root| !root.is_empty())
            .map(PathBuf::from)
            .or_else(|| self.workspace.get().cloned())
            .ok_or_else(|| PluginError::internal("skills plugin invoked before initialization"))
    }

    fn permission_host(&self) -> SdkResult<Arc<dyn HostClient>> {
        if let Some(host) = agena_runtime_plugins::plugin_slot::current_plugin_host()
            && let Some(plugin) = host
                .plugins()
                .iter()
                .find(|plugin| plugin.key().to_string() == SKILLS_PLUGIN_ID)
        {
            return Ok(host
                .host_handle()
                .scoped_host_client_for_scope(SKILLS_PLUGIN_ID.to_owned(), plugin.effect_scope()));
        }
        self.initial_host
            .get()
            .cloned()
            .ok_or_else(|| PluginError::internal("skills permission host is unavailable"))
    }

    fn manager(&self) -> SdkResult<SkillManager> {
        let mut declared = Vec::new();
        let mut additional = Vec::new();
        let mut disabled = Vec::new();
        if let Some(host) = agena_runtime_plugins::plugin_slot::current_plugin_host() {
            for plugin in host.plugins() {
                let id = plugin.key().to_string();
                if id == "agena.commands" {
                    let settings = plugin.configured_plugin.settings();
                    additional = settings
                        .get("additional_roots")
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok())
                        .unwrap_or_default();
                    disabled = settings
                        .get("disabled")
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok())
                        .unwrap_or_default();
                }
                for skill in &plugin.manifest.skills {
                    declared.push(DeclaredSkill {
                        plugin_id: id.clone(),
                        version: plugin.manifest.version.clone(),
                        name: skill.name.clone(),
                        description: skill.description.clone(),
                        instructions: skill.instructions.clone(),
                        aliases: skill.aliases.clone(),
                    });
                }
            }
        }
        Ok(SkillManager::new(
            self.workspace_root()?,
            agena_storage_layout::StorageLayout::current()
                .root()
                .join("skills"),
        )
        .with_additional_roots(additional)
        .with_declared_skills(declared)
        .with_disabled_names(disabled))
    }

    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(SkillManager) -> Result<T, SkillError> + Send + 'static,
    ) -> SdkResult<T> {
        let filter = crate::permissions::read_filter(self.permission_host()?, true)?;
        let predicate = filter.predicate();
        let manager = self
            .manager()?
            .with_read_filter(Arc::new(move |path| predicate.allows(path)));
        let result = worker(move || operation(manager)).await;
        filter.finish(result)
    }

    async fn require_path(&self, path: &Path, kind: PathKind) -> SdkResult<()> {
        crate::permissions::require_path(
            self.permission_host()?.as_ref(),
            kind,
            &path.to_string_lossy(),
        )
        .await
    }

    async fn authorize_id(&self, id: &str, write: bool) -> SdkResult<PathBuf> {
        let manager = self.manager()?;
        let id = id.to_owned();
        let path = worker(move || manager.resolve_path(&id)).await?;
        if path.to_string_lossy().starts_with("plugin://") {
            if write {
                return Err(PluginError::invalid_params(
                    "Plugin-provided skills are read-only. Create or install a managed copy to edit them.",
                ));
            }
            return Ok(path);
        }
        self.require_path(&path, PathKind::Read).await?;
        if write {
            self.require_path(&path, PathKind::Write).await?;
            if let Some(parent) = path.parent() {
                self.require_path(parent, PathKind::Write).await?;
                self.require_path(&parent.join(".agena-skill.json"), PathKind::Write)
                    .await?;
            }
        }
        Ok(path)
    }

    async fn authorize_tree_write(&self, root: &Path) -> SdkResult<()> {
        let root = root.to_owned();
        let paths = worker(move || {
            let mut builder = ignore::WalkBuilder::new(root);
            builder
                .hidden(false)
                .ignore(false)
                .git_ignore(false)
                .parents(false)
                .follow_links(false)
                .max_depth(Some(32));
            let mut paths = Vec::new();
            for (index, entry) in builder.build().enumerate() {
                if index > 20000 {
                    return Err(SkillError::invalid(
                        "The installed package is too large to remove safely.",
                    ));
                }
                let entry = entry.map_err(|error| SkillError::invalid(error.to_string()))?;
                if entry.depth() >= 32 {
                    return Err(SkillError::invalid(
                        "The installed package exceeds the supported directory depth.",
                    ));
                }
                paths.push(entry.path().to_owned());
            }
            Ok(paths)
        })
        .await?;
        for path in paths {
            self.require_path(&path, PathKind::Write).await?;
        }
        Ok(())
    }
}

async fn worker<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, SkillError> + Send + 'static,
) -> SdkResult<T> {
    let permit = crate::BLOCKING_PLUGIN_WORKERS
        .acquire()
        .await
        .map_err(|error| PluginError::internal_error(&error))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation()
    })
    .await
    .map_err(|error| PluginError::internal_error(&error))?
    .map_err(skill_error)
}

fn resource_path(document: &Path, relative: &str) -> SdkResult<PathBuf> {
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.contains('\\')
        || path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(PluginError::invalid_params(
            "Resource paths must stay inside the skill directory.",
        ));
    }
    document
        .parent()
        .map(|parent| parent.join(path))
        .ok_or_else(|| PluginError::invalid_params("The skill has no resource directory."))
}

fn revision(value: &str) -> SdkResult<()> {
    if value.is_empty() {
        return Err(PluginError::invalid_params(
            "Read the current skill or resource and supply its revision.",
        ));
    }
    Ok(())
}

fn parse<T: DeserializeOwned>(value: Value) -> SdkResult<T> {
    serde_json::from_value(if value.is_null() { json!({}) } else { value })
        .map_err(|error| PluginError::invalid_params_error(&error))
}

fn skill_error(error: SkillError) -> PluginError {
    match error.kind {
        agena_skills::SkillErrorKind::Io => PluginError::internal_error(&error),
        _ => PluginError::invalid_params_error(&error),
    }
}

fn output(title: &str, value: impl Serialize, changed: bool) -> SdkResult<ToolInvokeOutput> {
    let payload =
        serde_json::to_value(value).map_err(|error| PluginError::internal_error(&error))?;
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|error| PluginError::internal_error(&error))?;
    let metadata = if changed {
        BTreeMap::from([("agena.effect".to_owned(), "command_catalog".to_owned())])
    } else {
        BTreeMap::new()
    };
    Ok(ToolInvokeOutput::from_parts(
        title,
        title,
        text,
        Some(payload),
        metadata,
        Vec::new(),
    ))
}
