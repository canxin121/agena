//! Agent Skills management tools. The same manager serves the settings clients;
//! tool calls additionally authorize their concrete filesystem and network effects.

use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock},
};

use agena_domain::PathKind;
use agena_plugin_host::sdk::host_api::HostProcessResources;
use agena_plugin_host::sdk::{
    ApplicableSessions, HookSubscription, HostClient, ImportedSkillRender, InitContext,
    InitOutcome, Plugin, PluginError, PluginManifest, Result as SdkResult, ToolContract,
    ToolDefinition, ToolDocs, ToolInvokeInput, ToolInvokeOutput, ToolRuntimePolicy,
    ToolStreamingMode, ToolTag, macro_support,
};
use agena_skills::source::{
    SkillCreateRequest, SkillDeleteResult, SkillInstallRequest, SkillResourceWriteRequest,
    SkillSourceAccess, SkillSourceOperation, SkillSourceResult, SkillValidationRequest,
    load_install_source_with_transports,
};
use agena_skills::worker::{SkillWorkerOperation, SkillWorkerRequest};
use agena_skills::{
    DeclaredSkill, SkillCatalog, SkillDetail, SkillError, SkillInstallPreview, SkillManager,
    SkillResourceBase, SkillScope, SkillSummary,
};
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
struct GetInput {
    id: String,
    #[serde(default)]
    arguments: String,
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
        applies_to: ApplicableSessions::all(),
    }
}

#[async_trait::async_trait]
impl Plugin for SkillsPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("agena", "skills", env!("CARGO_PKG_VERSION"));
        manifest.display_name = Some("Skills".to_owned());
        manifest.icon = Some(crate::plugin_metadata::icon(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/plugin-icons/skills.svg"
        ))));
        manifest.authors = vec!["canxin121".to_owned()];
        manifest.help = Some("Discover, install, create and update complete Agent Skills packages with their scripts, references and assets. Preview installations and validate SKILL.md before writing.".to_owned());
        manifest.tags = vec![
            agena_plugin_host::sdk::ToolTag::Discovery,
            agena_plugin_host::sdk::ToolTag::Filesystem,
            agena_plugin_host::sdk::ToolTag::Custom("skills".to_owned()),
        ];
        manifest.summary =
            Some("Discover, install, author, and manage complete Agent Skills packages.".into());
        manifest.translations.insert("zh-CN".to_owned(), agena_plugin_host::sdk::PluginManifestTranslation {
            summary: Some("发现、安装、创作和管理完整的 Agent Skills 包。".to_owned()),
            help: Some("发现、安装、创建和更新包含脚本、参考资料及资源的完整 Agent Skills 包。支持安装预览和 SKILL.md 校验。".to_owned()),
            ..Default::default()
        });
        manifest.translations.insert(
            "zh-TW".to_owned(),
            agena_plugin_host::sdk::PluginManifestTranslation {
                summary: Some("探索、安裝、撰寫與管理完整的 Agent Skills 套件。".to_owned()),
                help: None,
                ..Default::default()
            },
        );
        manifest.translations.insert(
            "ja-JP".to_owned(),
            agena_plugin_host::sdk::PluginManifestTranslation {
                summary: Some(
                    "Agent Skills パッケージの検索、インストール、作成と管理を行います。"
                        .to_owned(),
                ),
                help: None,
                ..Default::default()
            },
        );
        manifest.translations.insert(
            "ko-KR".to_owned(),
            agena_plugin_host::sdk::PluginManifestTranslation {
                summary: Some(
                    "완전한 Agent Skills 패키지를 검색, 설치, 작성하고 관리합니다.".to_owned(),
                ),
                help: None,
                ..Default::default()
            },
        );
        manifest.translations.insert(
            "fr-FR".to_owned(),
            agena_plugin_host::sdk::PluginManifestTranslation {
                summary: Some(
                    "Découvrir, installer, créer et gérer des paquets Agent Skills complets."
                        .to_owned(),
                ),
                help: None,
                ..Default::default()
            },
        );
        manifest.translations.insert("de-DE".to_owned(), agena_plugin_host::sdk::PluginManifestTranslation {
            summary: Some("Vollständige Agent-Skills-Pakete finden, installieren, erstellen und verwalten.".to_owned()),
            help: None,
            ..Default::default()
        });
        manifest.translations.insert(
            "es-ES".to_owned(),
            agena_plugin_host::sdk::PluginManifestTranslation {
                summary: Some(
                    "Descubre, instala, crea y administra paquetes completos de Agent Skills."
                        .to_owned(),
                ),
                help: None,
                ..Default::default()
            },
        );
        manifest.translations.insert(
            "hi-IN".to_owned(),
            agena_plugin_host::sdk::PluginManifestTranslation {
                summary: Some("संपूर्ण Agent Skills पैकेज खोजें, इंस्टॉल करें, बनाएँ और प्रबंधित करें।".to_owned()),
                help: None,
                ..Default::default()
            },
        );
        manifest.translations.insert(
            "ar-SA".to_owned(),
            agena_plugin_host::sdk::PluginManifestTranslation {
                summary: Some(
                    "اكتشاف حزم Agent Skills الكاملة وتثبيتها وتأليفها وإدارتها.".to_owned(),
                ),
                help: None,
                ..Default::default()
            },
        );
        manifest.translations.insert(
            "pt-BR".to_owned(),
            agena_plugin_host::sdk::PluginManifestTranslation {
                summary: Some(
                    "Descubra, instale, crie e gerencie pacotes completos de Agent Skills."
                        .to_owned(),
                ),
                help: None,
                ..Default::default()
            },
        );
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
            definition::<GetInput>(
                "get",
                "Read a skill's full instructions and resource inventory.",
                "Use an ID returned by list. For an imported Claude skill, pass optional arguments to render its instructions. Its shell snippets use the session's shell permissions. Relative references resolve against the skill directory shown in path. Load supporting files with read_resource. allowed-tools is metadata and does not change existing tool permissions.",
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
        if Self::is_global_session()
            && matches!(
                input.tool_name.as_str(),
                "create"
                    | "update"
                    | "remove"
                    | "set_enabled"
                    | "write_resource"
                    | "remove_resource"
                    | "preview_install"
                    | "install"
            )
        {
            return Err(PluginError::invalid_params(
                "Select a project conversation to modify or install skills.",
            ));
        }
        match input.tool_name.as_str() {
            "list" => {
                let input: ListInput = parse(value)?;
                let catalog: SkillCatalog = parse(self.run(SkillWorkerOperation::List).await?)?;
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
                let input: GetInput = parse(value)?;
                let manager = self.manager_with_providers().await?;
                let path = self.authorize_id_with(manager.clone(), &input.id, false).await?;
                let mut detail: SkillDetail =
                    parse(self.run_with_manager(manager, SkillWorkerOperation::Get { id: input.id }).await?)?;
                let mut metadata = BTreeMap::new();
                if detail.summary.enabled
                    && detail.summary.valid
                    && let Some(rendered) = self
                        .permission_host()?
                        .render_imported_skill(&path, &detail.document, &input.arguments)
                        .await?
                {
                    match rendered {
                        ImportedSkillRender::Prompt(prompt) => detail.document = prompt,
                        ImportedSkillRender::ForkResult(result) => {
                            detail.document = result;
                            metadata.insert("skip_skill_activation".into(), "true".into());
                        }
                        ImportedSkillRender::Inspection => {
                            metadata.insert("skip_skill_activation".into(), "true".into());
                        }
                    }
                }
                let directory = detail
                    .summary
                    .path
                    .strip_suffix("/SKILL.md")
                    .unwrap_or(&detail.summary.path);
                let resource_hint = match &detail.resource_base {
                    Some(base) => base.hint(),
                    None => format!("Skill directory: {directory}\nResolve relative references against this directory."),
                };
                let text = format!(
                    "{resource_hint}\n\n{}\n\nResources:\n{}",
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
                    metadata,
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
                    self.run(SkillWorkerOperation::Create {
                        scope: input.scope,
                        document: input.document,
                    })
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
                    self.run(SkillWorkerOperation::Update {
                        id: input.id,
                        document: input.document,
                        expected_revision: input.expected_revision,
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
                let skill: SkillSummary = parse(
                    self.run(SkillWorkerOperation::Remove {
                        id: input.id,
                        expected_revision: input.expected_revision,
                    })
                    .await?,
                )?;
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
                    self.run(SkillWorkerOperation::SetEnabled {
                        id: input.id,
                        enabled: input.enabled,
                        expected_revision: input.expected_revision,
                    })
                    .await?,
                    true,
                )
            }
            "read_resource" => {
                let input: ResourceReadInput = parse(value)?;
                let manager = self.manager_with_providers().await?;
                let document = self.authorize_id_with(manager.clone(), &input.id, false).await?;
                let document = manager.declared_resource_base(&input.id)
                    .and_then(SkillResourceBase::directory)
                    .map_or(document, |directory| directory.join("SKILL.md"));
                let resource = resource_path(&document, &input.path)?;
                self.require_path(&resource, PathKind::Read).await?;
                output(
                    "Skill resource",
                    self.run_with_manager(manager, SkillWorkerOperation::ReadResource {
                        id: input.id,
                        path: input.path,
                    })
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
                    self.run(SkillWorkerOperation::WriteResource {
                        id: input.id,
                        path: request.path,
                        content: request.content,
                        expected_revision: request.expected_revision,
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
                    self.run(SkillWorkerOperation::RemoveResource {
                        id: input.id,
                        path: input.path,
                        expected_revision: input.expected_revision,
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
                let source_host = host.clone();
                let download_host = host.clone();
                let loaded = load_install_source_with_transports(
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
                                SkillSourceAccess::ResolvedNetwork { original, address } => {
                                    use agena_plugin_host::sdk::host_api::PermissionQuery;
                                    match host
                                        .check_resolved_network_permission(original, address.to_string())
                                        .await?
                                    {
                                        PermissionQuery::Allow {} => Ok(()),
                                        PermissionQuery::Deny { reason } | PermissionQuery::Ask { reason } => {
                                            Err(PluginError::from_kind(
                                                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                                                format!("skill source address {address} is not permitted: {reason}"),
                                            ))
                                        }
                                    }
                                }
                            }
                        }
                    },
                    skill_error,
                    move |operation| execute_source_worker(source_host.clone(), operation),
                    move |request| execute_source_download(download_host.clone(), request),
                )
                .await?;
                loaded
                    .verify_revision(request.expected_source_revision.as_deref())
                    .map_err(skill_error)?;
                let preview: SkillInstallPreview = parse(
                    self.run(SkillWorkerOperation::PreviewInstall {
                        package: loaded.package.clone(),
                        scope: request.scope,
                    })
                    .await?,
                )?;
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
                    self.run(SkillWorkerOperation::Install {
                        package: loaded.package,
                        scope: request.scope,
                        selected: request.selected,
                        replace: request.replace,
                        source: loaded.source,
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

    fn is_global_session() -> bool {
        agena_plugin_host::sdk::host_api::current_host_callback_context().is_some_and(|context| {
            context.session_id.is_some()
                && context.workspace_root.as_deref().is_none_or(str::is_empty)
        })
    }

    fn workspace_root(&self) -> SdkResult<PathBuf> {
        if Self::is_global_session() {
            return Err(PluginError::invalid_params(
                "Select a project conversation to access workspace skills.",
            ));
        }
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
        if agena_plugin_host::sdk::host_api::current_host_callback_context()
            .is_some_and(|context| context.session_id.is_some())
        {
            return Err(PluginError::internal(
                "skills host for this session is unavailable",
            ));
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
        let mut exact_skill_dirs = Vec::new();
        if let Some(host) = agena_runtime_plugins::plugin_slot::current_plugin_host() {
            exact_skill_dirs = host.agent_plugin_skill_dirs();
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
                        invocation: None,
                        resource_base: None,
                    });
                }
            }
        }
        let instance_root = agena_storage_layout::StorageLayout::current()
            .root()
            .to_path_buf();
        let workspace = if Self::is_global_session() {
            instance_root.join("global-skill-workspace")
        } else {
            self.workspace_root()?
        };
        let mut manager = SkillManager::new(workspace, instance_root.join("skills"))
            .with_additional_roots(additional)
            .with_exact_skill_dirs(exact_skill_dirs)
            .with_declared_skills(declared)
            .with_disabled_names(disabled);
        if Self::is_global_session() {
            manager = manager.with_instance_only_roots();
        }
        Ok(manager)
    }

    async fn run(&self, operation: SkillWorkerOperation) -> SdkResult<Value> {
        let manager = self.manager_with_providers().await?;
        self.run_with_manager(manager, operation).await
    }

    async fn manager_with_providers(&self) -> SdkResult<SkillManager> {
        let manager = self.manager()?;
        if Self::is_global_session() {
            return Ok(manager);
        }
        let provided = self.permission_host()?.imported_model_skills().await?;
        Ok(manager.with_provided_skills(provided))
    }

    async fn run_with_manager(&self, manager: SkillManager, operation: SkillWorkerOperation) -> SdkResult<Value> {
        let host = self.permission_host()?;
        if crate::sandbox_operations::enabled() {
            let mutation = operation.is_mutation();
            let snapshot = manager.worker_snapshot();
            let resources = HostProcessResources {
                reads: snapshot
                    .roots
                    .iter()
                    .map(|root| root.path.clone())
                    .chain(snapshot.declared.iter().filter_map(|skill| {
                        skill.resource_base.as_ref()?.directory().map(|path| path.to_string_lossy().into_owned())
                    }))
                    .collect(),
                writes: if mutation {
                    snapshot
                        .roots
                        .iter()
                        .filter(|root| root.editable)
                        .map(|root| root.path.clone())
                        .collect()
                } else {
                    Vec::new()
                },
                ..Default::default()
            };
            let input = serde_json::to_value(SkillWorkerRequest {
                manager: snapshot,
                operation,
            })
            .map_err(|error| PluginError::internal_error(&error))?;
            if mutation {
                return execute_mutation_worker(host, input, resources).await;
            }
            return crate::worker::execute_bundled_worker(
                host.as_ref(),
                "skills.manager",
                input,
                resources,
            )
            .await;
        }
        let filter = crate::permissions::read_filter(host, true)?;
        let predicate = filter.predicate();
        let manager = manager.with_read_filter(Arc::new(move |path| predicate.allows(path)));
        let result = worker(move || operation.execute(manager)).await;
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
        let manager = self.manager_with_providers().await?;
        self.authorize_id_with(manager, id, write).await
    }

    async fn authorize_id_with(&self, manager: SkillManager, id: &str, write: bool) -> SdkResult<PathBuf> {
        let path: PathBuf = parse(
            self.run_with_manager(manager, SkillWorkerOperation::ResolvePath { id: id.to_owned() })
                .await?,
        )?;
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
        let result = execute_source_worker(
            self.permission_host()?,
            SkillSourceOperation::MutationPaths {
                path: root.to_owned(),
            },
        )
        .await
        .map_err(skill_error)?;
        let SkillSourceResult::Paths(paths) = result else {
            return Err(PluginError::internal(
                "invalid skills mutation-path response",
            ));
        };
        for path in paths {
            self.require_path(&path, PathKind::Write).await?;
        }
        Ok(())
    }
}

async fn execute_source_download(
    host: Arc<dyn HostClient>,
    request: agena_skills::source::SkillDownloadRequest,
) -> SdkResult<reqwest::Response> {
    let url = request.request.url().clone();
    let route = crate::sandbox_operations::http_route(
        host.as_ref(),
        &url,
        agena_web::WebHttpRoute::default().with_addresses(request.addresses),
    )
    .await?;
    route
        .execute(
            &request.client,
            request.request,
            std::time::Duration::from_secs(20),
            Some(std::time::Duration::from_secs(120)),
            Some(agena_skills::MAX_ARCHIVE_BYTES as u64),
        )
        .await
        .map_err(|error| PluginError::internal(format!("skill archive transfer failed: {error}")))
}

async fn execute_source_worker(
    host: Arc<dyn HostClient>,
    operation: SkillSourceOperation,
) -> Result<SkillSourceResult, SkillError> {
    if !crate::sandbox_operations::enabled() {
        let permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|error| {
                SkillError::new(agena_skills::SkillErrorKind::Io, error.to_string())
            })?;
        return tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation.execute()
        })
        .await
        .map_err(|error| SkillError::new(agena_skills::SkillErrorKind::Io, error.to_string()))?;
    }
    let resources = HostProcessResources {
        reads: operation.read_paths(),
        ..Default::default()
    };
    let input =
        serde_json::to_value(operation).map_err(|error| SkillError::invalid(error.to_string()))?;
    let result =
        crate::worker::execute_bundled_worker(host.as_ref(), "skills.source", input, resources)
            .await
            .map_err(|error| {
                SkillError::new(agena_skills::SkillErrorKind::Io, error.to_string())
            })?;
    serde_json::from_value(result).map_err(|error| SkillError::invalid(error.to_string()))
}

/// Run the isolated mutation while retaining the same host-side lock used by
/// settings clients. Cancellation signals the runner; the blocking lock is
/// released only after that runner has terminated and reaped the child.
async fn execute_mutation_worker(
    host: Arc<dyn HostClient>,
    input: Value,
    resources: HostProcessResources,
) -> SdkResult<Value> {
    let prepared = crate::worker::prepare_bundled_worker(host.as_ref(), resources).await?;
    let cancel_on_drop = prepared.cancellation_token().drop_guard();
    let runtime = tokio::runtime::Handle::current();
    let result = tokio::task::spawn_blocking(move || {
        agena_skills::with_mutation_lock(|| {
            runtime.block_on(crate::worker::execute_prepared_bundled_worker(
                prepared,
                "skills.manager",
                input,
            ))
        })
        .map_err(skill_error)?
    })
    .await
    .map_err(|error| PluginError::internal_error(&error))?;
    drop(cancel_on_drop);
    result
}

/// Worker entry point: only catalog identity, public declarations and explicit
/// operation arguments cross the boundary. No plugin host is initialized.
pub(crate) fn execute_worker(operation: &str, mut input: Value) -> SdkResult<Value> {
    let snapshot = input
        .as_object_mut()
        .and_then(|object| object.remove("_agena_permission_snapshot"))
        .filter(|value| !value.is_null())
        .ok_or_else(|| {
            PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                "skills worker requires the host permission snapshot",
            )
        })?;
    let snapshot: agena_runtime_tools::authorization::SandboxPermissionSnapshot =
        serde_json::from_value(snapshot)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
    match operation {
        "skills.manager" => {
            let request: SkillWorkerRequest = parse(input)?;
            let manager = SkillManager::from_worker_snapshot(request.manager).with_read_filter(
                Arc::new(move |path| {
                    matches!(
                        snapshot.authorize_path_access(agena_domain::AccessKind::Read, path),
                        agena_domain::PermissionDecision::Allow
                    )
                }),
            );
            request.operation.execute(manager).map_err(skill_error)
        }
        "skills.source" => {
            let request: SkillSourceOperation = parse(input)?;
            serde_json::to_value(request.execute().map_err(skill_error)?)
                .map_err(|error| PluginError::internal_error(&error))
        }
        _ => Err(PluginError::invalid_params(format!(
            "unsupported skills worker operation: {operation}"
        ))),
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

#[cfg(test)]
mod global_scope_tests {
    use super::*;
    use agena_plugin_host::sdk::host_api::{
        HostCallbackContext, run_in_isolated_host_callback_context,
    };

    #[tokio::test]
    async fn global_session_has_only_instance_skill_root_and_no_workspace() {
        run_in_isolated_host_callback_context(
            HostCallbackContext {
                session_id: Some(1),
                ..Default::default()
            },
            async {
                let plugin = SkillsPlugin::new();
                assert!(plugin.workspace_root().is_err());
                let roots = plugin.manager().unwrap().roots();
                assert_eq!(roots.len(), 1);
                assert_eq!(
                    PathBuf::from(&roots[0].path),
                    agena_storage_layout::StorageLayout::current()
                        .root()
                        .join("skills")
                );
            },
        )
        .await;
    }
}
